//! Standalone table exports: optional stable ordering and a row limit, on the export worker.

use std::cmp::Ordering;
use std::path::Path;

use rustgis_data::{Dataset, Value as Cell, progress};
use serde_json::{Value, json};

use super::super::exports::{Work, Wrote};
use super::super::ps;
use crate::Session;

/// Validates the optional ordering against the stored field names before a file is touched.
/// No sort and no limit preserve the table's current row order and full contents.
fn options(ds: &Dataset, p: &Value) -> Result<(Vec<(usize, bool)>, Option<usize>), String> {
    let mut sort = Vec::new();
    if let Some(raw) = p.get("sort").filter(|v| !v.is_null()) {
        for key in raw.as_array().ok_or("sort must be an array of {field, descending?} objects")? {
            let name = key.get("field").and_then(Value::as_str).ok_or("each sort key needs a field name")?;
            let field = ds.field_index(name).ok_or_else(|| format!("no field named '{name}'"))?;
            let descending = match key.get("descending") {
                None => false,
                Some(v) => v.as_bool().ok_or("descending must be true or false")?,
            };
            sort.push((field, descending));
        }
    }
    let limit = p
        .get("limit")
        .filter(|v| !v.is_null())
        .map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| "limit must be a nonnegative integer".to_string()))
        .transpose()?;
    Ok((sort, limit))
}

/// Compares mixed integer/float cells without rounding whole numbers past 2^53. Bounds are
/// checked before the truncating cast; NaN follows all other numbers.
fn compare_number(a: &Cell, b: &Cell) -> Ordering {
    let integer_float = |i: i64, f: f64| {
        if f.is_nan() || f >= -(i64::MIN as f64) {
            Ordering::Less
        } else if f < i64::MIN as f64 {
            Ordering::Greater
        } else {
            i.cmp(&(f as i64)).then_with(|| (i as f64).partial_cmp(&f).unwrap_or(Ordering::Equal))
        }
    };
    match (a, b) {
        (Cell::Int(i), Cell::Float(f)) => integer_float(*i, *f),
        (Cell::Float(f), Cell::Int(i)) => integer_float(*i, *f).reverse(),
        _ => Ordering::Equal,
    }
}

/// Compares stored values without coercing text to numbers. Nulls stay last in both directions;
/// mixed stored types have a fixed order, so even hostile rows yield a consistent comparison.
fn compare(a: &Cell, b: &Cell, descending: bool) -> Ordering {
    let rank = |v: &Cell| match v {
        Cell::Int(_) | Cell::Float(_) => 0,
        Cell::Bool(_) => 1,
        Cell::Date(_) | Cell::DateTime(_) => 2,
        Cell::Text(_) => 3,
        Cell::Null => 4,
    };
    if matches!(a, Cell::Null) || matches!(b, Cell::Null) {
        return rank(a).cmp(&rank(b));
    }
    let order = match (a, b) {
        (Cell::Int(a), Cell::Int(b)) => a.cmp(b),
        (Cell::Float(a), Cell::Float(b)) => match (a.is_nan(), b.is_nan()) {
            (false, false) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (a, b) => a.cmp(&b),
        },
        (Cell::Int(_), Cell::Float(_)) | (Cell::Float(_), Cell::Int(_)) => compare_number(a, b),
        (Cell::Text(a), Cell::Text(b)) => a.cmp(b),
        (Cell::Bool(a), Cell::Bool(b)) => a.cmp(b),
        _ => rank(a).cmp(&rank(b)).then_with(|| a.partial_cmp(b).unwrap_or(Ordering::Equal)),
    };
    if descending { order.reverse() } else { order }
}

/// Sorts row indices by bottom-up stable merging, checking Cancel between every emitted index.
/// Equal keys take the left row first. Two index buffers avoid copying attributes during sorting;
/// only the rows kept by the limit are cloned. No change reaches the project's original table.
fn ordered(ds: &mut Dataset, sort: &[(usize, bool)], limit: Option<usize>) -> Result<usize, String> {
    let n = ds.features.len();
    let keep = limit.unwrap_or(n).min(n);
    let omitted = n - keep;
    progress::check().map_err(|e| e.to_string())?;
    if sort.is_empty() && omitted == 0 {
        return Ok(0);
    }
    let count = if sort.is_empty() { keep } else { n };
    let mut rows = Vec::new();
    rows.try_reserve_exact(count).map_err(|e| e.to_string())?;
    progress::begin(count);
    for r in 0..count {
        progress::step(1).map_err(|e| e.to_string())?;
        rows.push(r);
    }
    if !sort.is_empty() {
        let mut merged = Vec::new();
        merged.try_reserve_exact(n).map_err(|e| e.to_string())?;
        let cmp = |a: usize, b: usize| {
            sort.iter()
                .map(|&(field, descending)| compare(ds.value(a, field), ds.value(b, field), descending))
                .find(|o| *o != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        };
        let mut width = 1usize;
        while width < n {
            progress::begin(n);
            merged.clear();
            for pair in rows.chunks(width.saturating_mul(2)) {
                let (left, right) = pair.split_at(width.min(pair.len()));
                let (mut l, mut r) = (left.iter().peekable(), right.iter().peekable());
                while l.peek().is_some() || r.peek().is_some() {
                    progress::step(1).map_err(|e| e.to_string())?;
                    let next = match (l.peek(), r.peek()) {
                        (Some(a), Some(b)) if cmp(**a, **b) != Ordering::Greater => l.next(),
                        (Some(_), Some(_)) | (None, Some(_)) => r.next(),
                        _ => l.next(),
                    };
                    if let Some(&row) = next {
                        merged.push(row);
                    }
                }
            }
            std::mem::swap(&mut rows, &mut merged);
            width = width.saturating_mul(2);
        }
    }
    let mut features = Vec::new();
    features.try_reserve_exact(keep).map_err(|e| e.to_string())?;
    progress::begin(keep);
    for row in rows.into_iter().take(keep) {
        progress::step(1).map_err(|e| e.to_string())?;
        if let Some(feature) = ds.features.get(row) {
            features.push(feature.clone());
        }
    }
    ds.features = features.into();
    Ok(omitted)
}

/// Takes a cheap shared snapshot; ordering, loss accounting and writing happen on the worker.
/// The default uses the same writers and notices as an unsorted Export Table.
pub(in crate::cmd) fn table_work(s: &Session, p: &Value) -> Result<Work, String> {
    let path = ps(p, "path").ok_or("path is required")?.to_string();
    let ext = Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let gpkg = super::gpkg_table(p, &ext)?;
    let mut ds = s.project.tables.get(s.table_index(p)?).ok_or("no such table")?.dataset.clone();
    let (sort, limit) = options(&ds, p)?;
    Ok(Box::new(move || {
        let omitted = ordered(&mut ds, &sort, limit)?;
        let default = ds.name.clone();
        let null_numbers = rustgis_data::export::numbers_written_as_null(&ds.features, &ds.fields, &ext);
        let mut notes = rustgis_data::export_notes([(&ds, ds.extent())], &path, rustgis_data::raster::Crs::LonLat);
        if matches!(ext.as_str(), "shp" | "zip") {
            notes.extend(rustgis_data::io::shapefile_names_note(&ds.fields, &path));
            notes.extend(rustgis_data::io::shapefile_values_notes(&ds, &path));
            notes.extend(rustgis_data::io::shapefile_geometry_notes(&ds, &path, rustgis_data::raster::Crs::LonLat));
        }
        if matches!(ext.as_str(), "parquet" | "geoparquet") {
            notes.extend(rustgis_data::source::parquet::values_note(&ds, &path));
        }
        if ext == "gpkg" {
            notes.extend(rustgis_data::io::geopackage_names_note(&ds.fields, &path));
        }
        notes.extend(rustgis_data::io::dates_as_text_note(&ds.fields, &path));
        notes.extend(super::gpkg_dates_note(&ds, &ext, &path));
        let name_field = rustgis_data::io::name_field(&ds);
        notes.extend(rustgis_data::io::export_losses_notes(&ds, &ext, name_field, &path));
        let (left_out, table) =
            rustgis_data::io::write_path_to(&ds, Path::new(&path), rustgis_data::raster::Crs::LonLat, &gpkg).map_err(|e| e.to_string())?;
        if let Some(note) = rustgis_data::io::left_out_note(&path, &ext, left_out) {
            notes.insert(0, note);
        }
        let answer = json!({"path": path});
        let written_path = Some(path.clone());
        Ok(Wrote {
            answer,
            notes,
            written_path,
            then: Some(Box::new(move |s, out| {
                if omitted > 0 {
                    s.notify_text(
                        crate::notice_text::NoticeText::format(
                            "{path}: {count} row(s) left out by the requested limit",
                            [("path", path.clone().into()), ("count", omitted.to_string().into())],
                        ),
                        false,
                    );
                }
                super::notify_null_numbers(s, &path, null_numbers);
                super::report_table(s, &path, &gpkg, &default, table, out);
                Ok(())
            })),
            #[cfg(not(target_arch = "wasm32"))]
            owner: None,
            #[cfg(not(target_arch = "wasm32"))]
            crs_used: Vec::new(),
        })
    }))
}
