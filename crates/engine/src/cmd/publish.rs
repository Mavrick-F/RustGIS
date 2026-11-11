//! A layer kept in the project and the file it was read from: whether its edits and the file are
//! in step (#1385).

use crate::{CmdResult, CommandSpec as C, Session};
use serde_json::Value;

pub fn commands() -> Vec<C> {
    vec![
        C::new("source.syncState", "Source File State", "Data › Data Source", sync_state).roles(&[]).effects(crate::policy::Effects::NONE.with(crate::policy::Effect::Files)).view().params(
            r#"{"layer"?: "name|id (default: the active layer): one read whole from a Shapefile or .geojson file"} — whether it has edits its file lacks ("edited"), the file changed since it was read ("sourceChanged"), both ("both") or neither ("inSync"), and which of the file's parts changed"#,
        ),
    ]
}

/// The key a check of layer `id`'s files made ahead is taken by (see [`super::reads`]).
#[cfg(not(target_arch = "wasm32"))]
fn ahead_key(id: u32) -> String {
    format!("syncState:{id}")
}

/// A check of a layer's files made ahead: the record and path checked, and what changed.
#[cfg(not(target_arch = "wasm32"))]
struct Checked {
    files: rustgis_data::sync::FileSync,
    path: String,
    changed: Result<Vec<String>, String>,
}

/// The check `source.syncState` makes before it runs in the desktop app: every part of the
/// file read and hashed on a thread of its own, with progress and Cancel.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sync_ahead(s: &Session, p: &Value) -> Option<super::reads::ReadWork> {
    let id = s.layer_id(p).ok()?;
    let l = s.map().layer(id)?;
    let (files, path) = (l.synced.as_ref()?.files.clone(), l.dataset.source.clone()?);
    Some(super::reads::ahead("Source File State", ahead_key(id), move || {
        let changed = files.changed(std::path::Path::new(&path)).map_err(|e| e.to_string());
        Checked { files, path, changed }
    }))
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn sync_ahead(_: &Session, _: &Value) -> Option<super::reads::ReadWork> {
    None
}

/// Compares the layer with the files it was read from: each part of the size recorded is read
/// and hashed, so the work grows with the file. The desktop app checks them on a thread of its
/// own first ([`sync_ahead`]), with progress and **Cancel**, and the command takes that check as
/// it lands; when the layer's record changed meanwhile it says so rather than check again here,
/// which would hold the window. The assistant, scripts and the command line check where they
/// ask. Edits are found by hashing the layer's rows here, as `gpkg.pendingEdits` does: no more
/// rows than a layer read whole from a file of at most [`rustgis_data::io::MAX_BYTES`] holds.
fn sync_state(s: &mut Session, p: &Value) -> CmdResult {
    let id = s.layer_id(p)?;
    let l = s.map().layer(id).ok_or("the layer no longer exists")?;
    let (Some(synced), Some(path)) = (&l.synced, &l.dataset.source) else {
        if l.dataset.source.as_deref().is_some_and(rustgis_data::sync::is_syncable) && !l.is_streamed() {
            return Err(format!(
                "{} has no record of its file as it was read (it was added before RustGIS kept one, the file changed while it was read, or an older RustGIS saved the project): add the file again to compare it",
                l.name
            ));
        }
        return Err(format!("{} wasn't read whole from a Shapefile or .geojson file, so there's no file to compare it with", l.name));
    };
    let edited = rustgis_map::source::rows_digest(&l.dataset) != synced.rows;
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (edited, path);
        Err("the web app can't read files on this computer".into())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (files, path) = (synced.files.clone(), path.clone());
        let changed = match super::reads::take::<Checked>(s, &ahead_key(id)) {
            Some(checked) if checked.files == files && checked.path == path => checked.changed?,
            // Checking again here would hold the window: the check made on its thread is dropped.
            Some(_) => return Err("the layer's file or its record changed while it was checked: check again".into()),
            None => files.changed(std::path::Path::new(&path)).map_err(|e| e.to_string())?,
        };
        let state = match (edited, !changed.is_empty()) {
            (false, false) => "inSync",
            (true, false) => "edited",
            (false, true) => "sourceChanged",
            (true, true) => "both",
        };
        Ok(serde_json::json!({"layer": id, "path": path, "state": state, "edited": edited, "changed": changed}))
    }
}
