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

/// Compares the layer with the files it was read from. Runs on the UI thread: it reads the
/// files' sizes and times, hashing only a file of the same size whose time changed or was too
/// recent to record (a copy, a touch, a file just written), and finds edits by hashing the
/// layer's rows, as `gpkg.pendingEdits` does, for no more rows than a layer read whole holds.
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
        let changed = synced.files.changed(std::path::Path::new(path));
        let state = match (edited, !changed.is_empty()) {
            (false, false) => "inSync",
            (true, false) => "edited",
            (false, true) => "sourceChanged",
            (true, true) => "both",
        };
        Ok(serde_json::json!({"layer": id, "path": path, "state": state, "edited": edited, "changed": changed}))
    }
}
