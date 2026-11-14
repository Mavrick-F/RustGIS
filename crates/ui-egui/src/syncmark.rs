//! Edits not yet published (#1385): whether a layer read whole from a Shapefile or GeoJSON file
//! holds rows its file lacks, for the Layers pane. The layer's rows are hashed
//! (`rustgis_data::sync::rows_digest`) on a thread of their own and compared with the record of
//! the last sync; the answer is kept until the rows, fields, coordinate system or record change.
//! An edit copies the rows, so a changed layer is told by their allocation without a hash. The
//! rows hashed are held only while their hash runs. App state alone: nothing here dirties the
//! project or adds an undo step.

use std::collections::HashMap;
use std::sync::Weak;

use rustgis_engine::data::{Feature, Field, sync::Synced};
use rustgis_engine::map::Layer;

/// A layer's answer, or the hash under way for it.
struct Entry {
    rows: Weak<Vec<Feature>>,
    fields: Vec<Field>,
    crs: Option<rustgis_engine::data::raster::Crs>,
    synced: Synced,
    /// The rows' hash once it lands; `Err` when it couldn't be made.
    digest: Option<Result<u64, String>>,
    /// The one hash under way for the layer, and whether it hashes the layer as this entry has
    /// it. One for an earlier state runs out before another starts, so edits in quick succession
    /// never hash the layer more than once at a time.
    job: Option<(crate::Slot<Result<u64, String>>, bool)>,
}

impl Entry {
    /// Whether it answers for `l` as it is now.
    fn of(&self, l: &Layer) -> bool {
        Weak::ptr_eq(&self.rows, &l.dataset.features.allocation_key())
            && self.fields == l.dataset.fields
            && self.crs == l.dataset.crs
            && l.synced.as_ref() == Some(&self.synced)
    }
}

/// Each layer's answer, by its map's instance and its own.
#[derive(Default)]
pub(crate) struct Marks {
    entries: HashMap<(u64, u64), Entry>,
    /// The project revision the entries were last pruned at.
    revision: Option<u64>,
}

/// Whether `l`, a layer of the active map, holds edits its file lacks: `None` while that isn't
/// known yet (its hash is under way, or couldn't start) or for a layer with no record. Cheap on
/// the UI thread: a hash starts only when the layer changed, on a thread of its own, and the
/// pane is drawn again when it lands.
pub(crate) fn edited(app: &mut crate::RustGisApp, l: &Layer, ctx: &egui::Context) -> Option<bool> {
    let synced = l.synced.as_ref()?;
    let id = (app.session.map().instance_id(), l.instance_id());
    if !app.syncmark.entries.get(&id).is_some_and(|e| e.of(l)) {
        let earlier = app.syncmark.entries.remove(&id).and_then(|e| e.job).map(|(job, _)| (job, false));
        let entry = Entry {
            rows: l.dataset.features.allocation_key(),
            fields: l.dataset.fields.clone(),
            crs: l.dataset.crs,
            synced: synced.clone(),
            digest: None,
            job: earlier,
        };
        app.syncmark.entries.insert(id, entry);
    }
    let entry = app.syncmark.entries.get_mut(&id)?;
    if let Some((job, current)) = &entry.job {
        let landed = job.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
        let current = *current;
        if let Some(digest) = landed {
            entry.job = None;
            if current {
                entry.digest = Some(digest);
            }
        }
        // Looked at again soon: either it lands, or the next hash starts once it has.
        ctx.request_repaint_after(web_time::Duration::from_millis(100));
    } else if entry.digest.is_none() {
        let slot = crate::Slot::default();
        let out = slot.clone();
        let dataset = l.dataset.clone();
        // A thread the OS refuses leaves the layer unmarked: an unknown state isn't shown as edited.
        let spawned = std::thread::Builder::new().name("Edits not published".into()).spawn(move || {
            let digest = rustgis_engine::data::sync::rows_digest(&dataset).map_err(|e| e.to_string());
            *out.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(digest);
        });
        match spawned {
            Ok(_) => {
                entry.job = Some((slot, true));
                ctx.request_repaint_after(web_time::Duration::from_millis(100));
            }
            Err(e) => entry.digest = Some(Err(e.to_string())),
        }
    }
    match &entry.digest {
        Some(Ok(digest)) => Some(*digest != entry.synced.rows),
        _ => None,
    }
}

/// Forgets layers that are gone, so the answers kept follow the project; looked at once per
/// project revision.
pub(crate) fn retain(app: &mut crate::RustGisApp) {
    if app.syncmark.revision == Some(app.session.revision) {
        return;
    }
    app.syncmark.revision = Some(app.session.revision);
    let live: std::collections::HashSet<(u64, u64)> =
        app.session.project.maps.iter().flat_map(|m| m.layers.iter().map(move |l| (m.instance_id(), l.instance_id()))).collect();
    app.syncmark.entries.retain(|id, _| live.contains(id));
}
