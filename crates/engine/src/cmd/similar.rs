//! Similarity ranks every resident row. Desktop adapters prepare the work off the window
//! thread; only validated completion may replace selection or deliver its notices.

use rustgis_data::{Dataset, query};
use rustgis_map::effects::DisplayFilter;
use serde_json::{Value, json};

use crate::notice_text::NoticeText;
use crate::{CmdResult, Session};

#[derive(PartialEq)]
struct Target {
    id: u32,
    row: usize,
    fields: Vec<usize>,
    count: usize,
    select: bool,
}

/// The map filters affecting output selection, independently of the ranking population.
#[derive(PartialEq)]
struct Visibility {
    query: Option<String>,
    filters: Vec<DisplayFilter>,
    time: Option<(String, Option<(f64, f64)>)>,
    scale: f64,
}

/// Captures only this layer's metadata and shared row storage, never a project/session copy.
struct Input {
    target: Target,
    opened: u64,
    map: u64,
    instance: u64,
    name: String,
    data: Dataset,
    visibility: Option<Visibility>,
}

impl Input {
    /// Resolves the reference and schema without traversing features. Selection supplies only
    /// its first row when no explicit reference is given.
    fn resolve(s: &Session, p: &Value) -> Result<Self, String> {
        let id = s.layer_id(p)?;
        let l = s.map().layer(id).ok_or("no such layer")?;
        let row = match p.get("row").and_then(Value::as_u64) {
            Some(r) => r as usize,
            None => *l.selection.first().ok_or("select a feature (or give row)")?,
        };
        if row >= l.dataset.features.len() {
            return Err("no such row".into());
        }
        let fields = super::workflows::numeric_fields(&l.dataset, p)?;
        if fields.is_empty() {
            return Err("the layer has no numeric fields to compare".into());
        }
        let count = p.get("count").and_then(Value::as_u64).unwrap_or(10).clamp(1, 1000) as usize;
        let select = super::b(p, "select").unwrap_or(true);
        let visibility = select.then(|| Visibility {
            query: l.definition_query.clone(),
            filters: l.display_filters.clone(),
            time: l.time.as_ref().map(|t| (t.field.clone(), t.window)),
            scale: s.scale(),
        });
        Ok(Self {
            target: Target { id, row, fields, count, select },
            opened: s.opened,
            map: s.map().instance_id(),
            instance: l.instance_id(),
            name: l.name.clone(),
            data: l.dataset.clone(),
            visibility,
        })
    }

    /// Refuses data/reference/filter changes while work ran. Comparing row allocations avoids
    /// copying or visiting features on the window thread.
    fn same(&self, now: &Self) -> bool {
        self.target == now.target
            && self.opened == now.opened
            && self.map == now.map
            && self.instance == now.instance
            && self.name == now.name
            && self.data.features.shares(&now.data.features)
            && self.data.fields == now.data.fields
            && self.data.coord_crs() == now.data.coord_crs()
            && self.visibility == now.visibility
    }
}

struct Done {
    answer: Value,
    selection: Option<Vec<usize>>,
    notices: Vec<NoticeText>,
}

struct Ahead {
    input: Input,
    done: Result<Done, String>,
}

/// Filters the bounded result/reference rows with cancellation, without a layer-wide mask.
fn keep(rows: Vec<usize>, mut matches: impl FnMut(usize) -> bool) -> Result<Vec<usize>, String> {
    rustgis_data::progress::begin(rows.len());
    let mut kept = Vec::with_capacity(rows.len());
    for row in rows {
        rustgis_data::progress::step(1).map_err(|e| e.to_string())?;
        if matches(row) {
            kept.push(row);
        }
    }
    Ok(kept)
}

/// Computes rankings and output intent without mutating the session. Ranking checks Cancel in
/// its loops; expression evaluation of one candidate and a time binding's initial mask fill are
/// indivisible library steps, checked before/after. A detached time cache cannot lock the live
/// window's cache. Only the at-most-1,001 result/reference rows need query/filter evaluation.
fn gather(input: &Input) -> Result<Done, String> {
    let t = &input.target;
    let found = rustgis_analysis::network::similar(&input.data, t.row, &t.fields, t.count)?;
    let answer = json!({"similar": found.iter().map(|(r, d)| json!({"row": r, "distance": d})).collect::<Vec<_>>()});
    let Some(v) = &input.visibility else { return Ok(Done { answer, selection: None, notices: Vec::new() }) };
    let mut rows: Vec<usize> = found.iter().map(|(r, _)| *r).collect();
    rows.push(t.row);
    let matched = rows.len();
    let mut failed = 0usize;
    let mut first = None;
    let mut error = |e| {
        failed += 1;
        if first.is_none() {
            first = Some(e);
        }
    };
    rustgis_data::progress::check().map_err(|e| e.to_string())?;
    match rustgis_data::rows::Query::compile(v.query.as_deref(), &input.data) {
        Ok(q) => rows = keep(rows, |r| q.matches(&input.data, r, &query::Env::default()))?,
        Err(e) => error(e),
    }
    if let Some((field, Some(window))) = &v.time {
        let mut time = rustgis_map::time::TimeBinding::new(field);
        time.window = Some(*window);
        let mask = time.keeps(&input.data);
        rustgis_data::progress::check().map_err(|e| e.to_string())?;
        if let Some(mask) = mask {
            rows = keep(rows, |r| mask.get(r) == Some(&true))?;
        }
    }
    for filter in v.filters.iter().filter(|f| f.applies(v.scale)) {
        rustgis_data::progress::check().map_err(|e| e.to_string())?;
        match query::parse_fields(&filter.where_, &input.data) {
            Ok(q) => rows = keep(rows, |r| query::matches(&q, &input.data, r, &query::Env::default()))?,
            Err(e) => error(e),
        }
    }
    let mut notices = Vec::new();
    if let Some(error) = first {
        notices.push(NoticeText::format(
            "{layer}: selecting without unreadable query/filter settings ({count}). First parser error: {error}",
            [("layer", input.name.clone().into()), ("count", failed.to_string().into()), ("error", error.into())],
        ));
    }
    let excluded = matched - rows.len();
    if excluded > 0 {
        notices.push(NoticeText::format(
            "{n} features aren't drawn (the layer's definition query, time window or display filters hide them), and aren't selected",
            [("n", excluded.to_string().into())],
        ));
    }
    rustgis_data::progress::check().map_err(|e| e.to_string())?;
    Ok(Done { answer, selection: Some(rows), notices })
}

/// Prepares desktop Similar work on shared rows. Invalid requests remain cheap synchronous
/// errors; successful work is taken only by the original admitted command at landing.
pub(super) fn ahead(s: &Session, p: &Value) -> Option<super::reads::ReadWork> {
    let input = Input::resolve(s, p).ok()?;
    Some(super::reads::ahead("Find Similar", "data.similar".into(), move || {
        let done = gather(&input);
        Ahead { input, done }
    }))
}

/// Desktop UI and direct MCP adapters use the prepared read queue, with progress and Cancel.
/// A borrowed synchronous Session cannot yield a deferred command reply; headless/script callers
/// run here on their caller's thread, and the browser has one thread. A registered window owner
/// refuses an unprepared run instead of repeating heavy work inline.
pub(super) fn run(s: &mut Session, p: &Value) -> CmdResult {
    let input = Input::resolve(s, p)?;
    let ahead = super::reads::take::<Ahead>(s, "data.similar");
    #[cfg(not(target_arch = "wasm32"))]
    if ahead.is_none() && s.statistics_window_thread == Some(std::thread::current().id()) {
        return Err("Similarity needs the desktop background read queue; run Find Similar or a direct MCP data.similar request".into());
    }
    let done = match ahead {
        Some(a) if a.input.same(&input) => a.done?,
        Some(_) => return Err("the layer, reference or map filters changed while similarity was computed: ask again".into()),
        None => gather(&input)?,
    };
    if let Some(rows) = done.selection {
        super::select::set(s, input.target.id, rows, super::select::Mode::New);
    }
    for notice in done.notices {
        s.notify_text(notice, false);
    }
    Ok(done.answer)
}
