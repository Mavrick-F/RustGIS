//! Field packages (#1902): a map taken to a phone and brought back.
//!
//! A field package (`.rgisfield`) is a zip holding a project of one map, `<area> (offline)`, and
//! the files it reads beside it: the MBTiles and PMTiles archives its tile layers draw from, under
//! `basemaps/`, so the map has its basemap with no network. Packaging the active map makes an
//! offline area of it in this project first (as Download Map does), so the edits made in the
//! field can be brought back to it by feature, with the offline area's conflict handling.
//!
//! Opening a field package unpacks it into the app's own folder (`field/<name>`) and opens the
//! project there, unsaved, as a project package opens.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Session;
use crate::project::Project;

/// A field package's extension.
pub const EXTENSION: &str = "rgisfield";
/// The folder of a field package that holds its tile archives.
const BASEMAPS: &str = "basemaps";
/// What an offline area's map is called after: `<area> (offline)`.
pub(crate) const OFFLINE: &str = " (offline)";
/// The most bytes read from one field package's entries when it is opened.
const MOST_UNPACKED: usize = rustgis_data::zip::MAX_TOTAL;

/// Whether a file name or path is a field package's, ignoring case.
pub fn is_field_package(name: &str) -> bool {
    name.to_ascii_lowercase().strip_suffix(EXTENSION).is_some_and(|s| s.ends_with('.'))
}

/// A field package of the active map, as (name, bytes).
///
/// An offline area's map (`<area> (offline)` of one of this project's areas, or the map of a
/// project opened from a field package) is packaged as it is, to bring its edits back. Any other map is first made an offline area of this project (`extent`
/// and `name` as for `offline.download`, the view by default), whose map is packaged.
///
/// Never silent: the raster layers an offline area leaves behind, the tile layers that stay
/// online (a web address, not an archive), and an archive that couldn't be read are said.
///
/// It runs where it is called, on the window's thread from Save (`ui.saveItem`), as Package
/// Project does: each archive is one read and the zip one write, with no Cancel between them.
pub(crate) fn package(s: &mut Session, p: &Value) -> Result<(String, Vec<u8>), String> {
    // An offline area's map: named for one of this project's areas, or opened from a field
    // package (the phone's project holds no areas). A map only named so is packaged as any other.
    let opened_field = s.opened_package.as_deref().is_some_and(|p| is_field_package(&p.to_string_lossy()));
    let returning = s
        .map()
        .name
        .strip_suffix(OFFLINE)
        .map(str::to_string)
        .filter(|area| opened_field || s.project.offline_areas.iter().any(|a| a.name.eq_ignore_ascii_case(area)));
    let (area, mut map) = match returning {
        Some(area) => {
            // Its raster layers stay behind, as an offline area's do going out: said.
            let mut m = s.map().clone();
            let rasters: Vec<u32> = m.layers.iter().filter(|l| l.is_raster()).map(|l| l.id).collect();
            if !rasters.is_empty() {
                m.release_tiles(&rasters);
                m.layers.retain(|l| !l.is_raster());
                s.notify(format!("{} raster layer(s) were left out of the field package: it holds feature layers", rasters.len()), false);
            }
            (area, m)
        }
        None => {
            let rasters = s.map().layers.iter().filter(|l| l.is_raster()).count();
            let features =
                |m: &rustgis_map::Map| m.layers.iter().filter(|l| !l.is_raster()).map(|l| l.dataset.features.len()).sum::<usize>();
            let had = features(s.map());
            let made = super::workspace::offline_download(s, p)?;
            let name = made.get("name").and_then(Value::as_str).ok_or("the offline area wasn't made")?.to_string();
            let data =
                s.project.offline_areas.iter().find(|a| a.name == name).map(|a| a.data.clone()).ok_or("the offline area wasn't made")?;
            if rasters > 0 {
                s.notify(
                    format!("{rasters} raster layer(s) were left out of the field package: an offline area holds feature layers"),
                    false,
                );
            }
            s.notify(
                format!("Made the offline area '{name}' of this map for the field package: save this project to bring the field's edits back to it later"),
                false,
            );
            // The area holds the features its extent (the view, unless one is given) touches.
            let out = had.saturating_sub(features(&data));
            if out > 0 {
                s.notify(
                    format!("{out} feature(s) outside the offline area's extent, or without a shape, stayed out of the field package"),
                    false,
                );
            }
            let mut m = data;
            m.name = format!("{name}{OFFLINE}");
            (name, m)
        }
    };
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut budget = MOST_UNPACKED as u64 / 2;
    // Each local archive goes in beside the project, its layer reading it from there.
    let (mut online, mut unread, mut uploaded) = (Vec::new(), Vec::new(), Vec::new());
    let mut taken = std::collections::HashSet::new();
    for t in &mut map.tiles {
        let Some(file) = t.archive_file().map(str::to_string) else {
            // An archive added to the web app lives in its memory, not in a file to put in.
            if t.url.starts_with(rustgis_map::tiles::UPLOAD) {
                uploaded.push(t.name.clone());
            } else {
                online.push(t.name.clone());
            }
            continue;
        };
        match crate::project::read_file(Path::new(&file), budget) {
            Ok(bytes) => {
                budget = budget.saturating_sub(bytes.len() as u64);
                // A name every system's folders can hold as one file, the same in the entry and
                // the layer's address: letters, digits, `-`, `_`, `.` and spaces, anything else `_`
                // (`:`, `\`, `*`, `?`… are fine in a Linux name, refused or split elsewhere).
                let stem: String = Path::new(&file).file_name().map_or_else(
                    || "basemap".into(),
                    |n| {
                        n.to_string_lossy()
                            .chars()
                            .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') { c } else { '_' })
                            .collect()
                    },
                );
                let mut name = stem.clone();
                for n in 2.. {
                    if taken.insert(name.to_ascii_lowercase()) {
                        break;
                    }
                    name = format!("{n}-{stem}");
                }
                t.url = format!("{BASEMAPS}/{name}");
                entries.push((t.url.clone(), bytes));
            }
            Err(e) => unread.push(format!("{} ({e})", t.name)),
        }
    }
    if !online.is_empty() {
        s.notify(
            format!(
                "{} tile layer(s) in the field package draw from the web and show only online: {}. Add an MBTiles or PMTiles archive for a basemap with no network",
                online.len(),
                online.join(", ")
            ),
            false,
        );
    }
    if !uploaded.is_empty() {
        s.notify(
            format!(
                "{} tile archive(s) added to this browser couldn't be put in the field package, as they live only in its memory: {}. Add them to the field from the file",
                uploaded.len(),
                uploaded.join(", ")
            ),
            true,
        );
    }
    if !unread.is_empty() {
        s.notify(format!("{} tile archive(s) couldn't be put in the field package: {}", unread.len(), unread.join(", ")), true);
    }
    // A project of the one map: what else the project holds (tables, notebooks, models, shared
    // items and their paths…) stays at home. The custom colour ramps and styles go with it, as its
    // layers may draw with them.
    let mut project = crate::project::Project::new(area.clone());
    // Rasters were taken out above, so the map goes as it is.
    project.maps = vec![map];
    let home = &s.project;
    let stayed: Vec<String> = [
        (home.tables.len(), "table(s)"),
        (home.notebooks.len(), "notebook(s)"),
        (home.models.len(), "model(s)"),
        (home.layouts.len(), "layout(s)"),
        (home.reports.len(), "report(s)"),
        (home.presentations.len(), "presentation(s)"),
        (home.workspaces.len(), "imagery workspace(s)"),
        (home.dl_models.len(), "deep learning model(s)"),
        (home.maps.len().saturating_sub(1), "other map(s)"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{n} {what}"))
    .collect();
    if !stayed.is_empty() {
        s.notify(format!("The field package holds only the map '{area}': {} stayed in this project", stayed.join(", ")), false);
    }
    project.ramps = s.project.ramps.clone();
    project.styles = s.project.styles.clone();
    let readme = format!(
        "{area}: a RustGIS field package.\nOpen it with RustGIS on a phone or a computer (Open Project). Its basemaps are in {BASEMAPS}/.\n"
    );
    let room = crate::project::max_bytes().min(MOST_UNPACKED as u64).saturating_sub(entries.iter().map(|(_, b)| b.len() as u64).sum());
    let too_large = || "the field package would be too large: leave a tile archive or some data out".to_string();
    let (text, report) = crate::durable::output_owned(project, |p| crate::project::json_within(p, room)?.ok_or_else(too_large))?;
    crate::durable::notify(s, &report);
    entries.insert(0, (format!("{}.{}", super::items::safe(&area), crate::project::EXTENSION), text.into_bytes()));
    entries.push(("README.txt".into(), readme.into_bytes()));
    Ok((area, rustgis_data::zip::write(&entries)?))
}

/// The folder a field package named `stem` is unpacked into: the app's own `field/<stem>`.
fn unpack_dir(stem: &str) -> Result<PathBuf, String> {
    let base =
        rustgis_data::dirs::app_data_dir().ok_or("this computer gives RustGIS no folder of its own to unpack the field package into")?;
    Ok(fresh_dir(&base.join("field"), &super::items::safe(stem)))
}

/// A folder in `base` named `name`, or `name 2`, `name 3`… when one is there: a copy unpacked
/// before (another package of the same name, perhaps still open) is never written over.
fn fresh_dir(base: &Path, name: &str) -> PathBuf {
    let mut dir = base.join(name);
    for n in 2.. {
        if !dir.exists() {
            break;
        }
        dir = base.join(format!("{name} {n}"));
    }
    dir
}

/// Unpacks a field package's `bytes` into a folder of its own in the app's folder (`field/<name>`,
/// numbered when that is taken) and returns the folder and the project's text. An entry that
/// would land outside the folder is refused, and the package with it.
///
/// The project's own paths are trusted as a project file's are: a package from someone else can
/// name files outside the folder, as a `.rgis` can.
pub(crate) fn unpack(bytes: &[u8], stem: &str) -> Result<(PathBuf, String), String> {
    let dir = unpack_dir(stem)?;
    let text = unpack_into(bytes, &dir)?;
    Ok((dir, text))
}

/// Unpacks a field package's `bytes` into `dir`, a folder not there yet (see [`unpack`]), and
/// returns the project's text. A write that fails removes what was written.
fn unpack_into(bytes: &[u8], dir: &Path) -> Result<String, String> {
    let entries = rustgis_data::zip::read(bytes)?;
    let mut project = None;
    let mut files = Vec::new();
    for (name, data) in entries {
        let parts: Vec<&str> = name.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
        if parts.is_empty() || parts.iter().any(|p| *p == ".." || *p == "." || p.contains(':')) || name.starts_with(['/', '\\']) {
            return Err(format!("the field package holds an entry outside its folder: {name}"));
        }
        if parts.len() == 1 && crate::project::is_project_file(&name) {
            if project.is_some() {
                return Err("the field package holds more than one project".into());
            }
            project = Some(String::from_utf8(data).map_err(|_| "the field package's project is damaged".to_string())?);
        } else {
            files.push((parts.iter().collect::<PathBuf>(), data));
        }
    }
    let text = project.ok_or("the field package holds no project")?;
    // Read before anything is written: a project that can't be read leaves no folder behind.
    Project::from_import_json(&text, Some(dir))?;
    if dir.exists() {
        return Err(format!("{}: the folder is already there", dir.display()));
    }
    let written = (|| {
        for (rel, data) in files {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&path, data).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok::<(), String>(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(dir);
        return Err(e);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(text)
}

/// Reads a field package's project, its relative paths resolved against `dir`.
pub(crate) fn project_in(text: &str, dir: &Path) -> Result<Project, String> {
    Project::from_import_json(text, Some(dir))
}

/// Removes the offline areas past the first `had` of this project: those a field package just made,
/// when it wasn't saved after all (the save was cancelled or failed). Says so.
pub fn forget_new_areas(s: &mut Session, had: usize) {
    let made: Vec<String> = s.project.offline_areas.iter().skip(had).map(|a| a.name.clone()).collect();
    for name in made {
        s.project.offline_areas.retain(|a| a.name != name);
        s.notify(format!("The field package wasn't saved, so its offline area '{name}' was removed too"), false);
    }
}

/// Refuses `path` when it is the open project's own file (or the field package it was opened
/// from): written there, the package would replace the project, as an export there is refused.
pub(crate) fn not_the_project(s: &Session, path: &Path) -> Result<(), String> {
    let same = |a: &Path, b: &Path| a == b || a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(a, b)| a == b);
    if s.path.iter().chain(s.opened_package.iter()).any(|project| same(project, path)) {
        return Err("the field package can't go in the open project's own file: choose another".into());
    }
    Ok(())
}

/// What `offline.package` returns.
pub(crate) fn packaged(path: &str, area: &str, bytes: usize) -> Value {
    json!({"path": path, "area": area, "bytes": bytes})
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustgis_map::tiles::{TileKind, TileLayer};

    /// A folder of the test's own, removed when it ends.
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Temp {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            let dir = std::env::temp_dir().join(format!("rustgis-field-{name}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Temp(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sample() -> Session {
        Session::new(crate::sample::world_project())
    }

    #[test]
    fn a_map_packaged_for_the_field_opens_with_its_basemap_and_comes_from_an_offline_area() {
        let temp = Temp::new("package");
        let archive = temp.path().join("site.mbtiles");
        std::fs::write(&archive, b"stand-in archive bytes").unwrap();
        let mut s = sample();
        s.map_mut().tiles.push(TileLayer::new("Site", TileKind::MbTiles, archive.display().to_string()));
        s.map_mut().tiles.push(TileLayer::new("Streets", TileKind::Raster, "https://tiles.example.com/{z}/{x}/{y}.png"));
        let mark = s.notice_mark();
        let (area, bytes) = package(&mut s, &json!({"name": "Survey", "extent": [-20.0, -40.0, 60.0, 40.0]})).unwrap();
        assert_eq!(area, "Survey");
        // The offline area is in this project, to bring the edits back to.
        assert!(s.project.offline_areas.iter().any(|a| a.name == "Survey"));
        // The online tile layer is said; so is the area made.
        let said: Vec<String> = s.notices_since(mark).iter().map(|n| n.text.clone()).collect();
        assert!(said.iter().any(|t| t.contains("Streets") && t.contains("online")), "{said:?}");
        assert!(said.iter().any(|t| t.contains("offline area 'Survey'")), "{said:?}");
        // Clipped to the extent: the features left outside it are said.
        assert!(said.iter().any(|t| t.contains("outside the offline area's extent")), "{said:?}");
        // The package: the project, the archive under basemaps/, a README.
        let entries = rustgis_data::zip::read(&bytes).unwrap();
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Survey.rgis", "basemaps/site.mbtiles", "README.txt"]);
        assert_eq!(entries[1].1, b"stand-in archive bytes");
        // Unpacked and read: one map, the area's, its features in it, its basemap read from beside it.
        let into = temp.path().join("unpacked");
        let text = unpack_into(&bytes, &into).unwrap();
        let project = project_in(&text, &into).unwrap();
        assert_eq!(project.maps.len(), 1);
        let map = &project.maps[0];
        assert_eq!(map.name, "Survey (offline)");
        assert!(map.layers.iter().any(|l| !l.dataset.features.is_empty()));
        let base = map.tiles.iter().find(|t| t.name == "Site").unwrap();
        assert_eq!(Path::new(&base.url), into.join("basemaps/site.mbtiles"));
        assert!(into.join("basemaps/site.mbtiles").is_file());
        // Unpacked again: into a folder of its own, the first left as it is.
        assert!(unpack_into(&bytes, &into).unwrap_err().contains("already there"));
        let again = fresh_dir(temp.path(), "unpacked");
        assert_eq!(again, temp.path().join("unpacked 2"));
        unpack_into(&bytes, &again).unwrap();
        assert!(into.join("basemaps/site.mbtiles").is_file() && again.join("basemaps/site.mbtiles").is_file());
    }

    #[test]
    fn a_field_package_holds_only_its_map_and_a_basemap_of_any_name_is_read() {
        let temp = Temp::new("only");
        // A name a Linux file can have, which a package's folders couldn't hold as it is.
        let archive = temp.path().join("site:a\\b*?.mbtiles");
        std::fs::write(&archive, b"stand-in archive bytes").unwrap();
        let mut s = sample();
        s.map_mut().tiles.push(TileLayer::new("Site", TileKind::MbTiles, archive.display().to_string()));
        // What else the project holds stays at home.
        s.project
            .tables
            .push(crate::project::Table::new(7, rustgis_data::Dataset::new("Inventory", rustgis_data::GeometryType::None, Vec::new())));
        s.project.folders.push("/home/someone/data".into());
        let mark = s.notice_mark();
        let (_, bytes) = package(&mut s, &json!({"name": "Survey", "extent": [-20.0, -40.0, 60.0, 40.0]})).unwrap();
        // What stayed at home is said.
        let said: Vec<String> = s.notices_since(mark).iter().map(|n| n.text.clone()).collect();
        assert!(said.iter().any(|t| t.contains("stayed in this project") && t.contains("table(s)")), "{said:?}");
        let names: Vec<String> = rustgis_data::zip::read(&bytes).unwrap().into_iter().map(|(n, _)| n).collect();
        assert!(names.contains(&"basemaps/site_a_b__.mbtiles".to_string()), "{names:?}");
        let into = temp.path().join("unpacked");
        let project = project_in(&unpack_into(&bytes, &into).unwrap(), &into).unwrap();
        assert!(project.tables.is_empty() && project.folders.is_empty() && project.offline_areas.is_empty());
        assert_eq!(project.maps.len(), 1);
        let base = project.maps[0].tiles.iter().find(|t| t.name == "Site").unwrap();
        assert_eq!(Path::new(&base.url), into.join("basemaps/site_a_b__.mbtiles"));
        assert!(into.join("basemaps/site_a_b__.mbtiles").is_file());
    }

    #[test]
    fn a_field_package_not_saved_leaves_no_offline_area() {
        let mut s = sample();
        let had = s.project.offline_areas.len();
        // A place that can't be written: the command is refused, and the area made for it goes.
        let e = s.run("offline.package", &json!({"path": "/no/such/folder/site.rgisfield", "name": "Lost"})).unwrap_err();
        assert!(e.contains("site.rgisfield"), "{e}");
        assert_eq!(s.project.offline_areas.len(), had);
        // As the front end does when its Save is cancelled: the area goes, and that is said.
        package(&mut s, &json!({"name": "Cancelled"})).unwrap();
        assert_eq!(s.project.offline_areas.len(), had + 1);
        let mark = s.notice_mark();
        forget_new_areas(&mut s, had);
        assert_eq!(s.project.offline_areas.len(), had);
        assert!(s.notices_since(mark).iter().any(|n| n.text.contains("'Cancelled' was removed")));
        // Not over the open project's own file.
        let temp = Temp::new("own");
        let own = temp.path().join("home.rgis");
        std::fs::write(&own, b"{}").unwrap();
        s.path = Some(own.clone());
        let e = s.run("offline.package", &json!({"path": own.display().to_string(), "name": "Own"})).unwrap_err();
        assert!(e.contains("open project's own file"), "{e}");
        assert_eq!(std::fs::read(&own).unwrap(), b"{}");
        // A project of the one map and nothing else: nothing is said to have stayed.
        let home = &mut s.project;
        home.maps.truncate(1);
        home.active_map = 0;
        home.tables.clear();
        home.notebooks.clear();
        home.models.clear();
        home.layouts.clear();
        home.reports.clear();
        home.presentations.clear();
        home.workspaces.clear();
        home.dl_models.clear();
        let mark = s.notice_mark();
        package(&mut s, &json!({"name": "Alone"})).unwrap();
        assert!(s.notices_since(mark).iter().all(|n| !n.text.contains("stayed in this project")));
    }

    #[test]
    fn what_a_package_leaves_behind_is_said_and_a_clean_one_says_nothing() {
        let temp = Temp::new("notices");
        let archive = temp.path().join("base.pmtiles");
        std::fs::write(&archive, b"tiles").unwrap();
        // Clean: one local archive, no raster, no web layer: no notice of anything left behind.
        let mut s = sample();
        s.map_mut().layers.retain(|l| !l.is_raster());
        s.map_mut().tiles.push(TileLayer::new("Base", TileKind::PmTiles, archive.display().to_string()));
        let mark = s.notice_mark();
        package(&mut s, &json!({"name": "Clean"})).unwrap();
        let said: Vec<String> = s.notices_since(mark).iter().map(|n| n.text.clone()).collect();
        assert!(said.iter().all(|t| !t.contains("left out") && !t.contains("from the web") && !t.contains("couldn't be put")), "{said:?}");
        // An archive that can't be read, and a raster layer: each said.
        use rustgis_data::raster::{ColorHint, Crs, GeoTransform, Raster, geotiff};
        let gt = GeoTransform { origin_x: 10.0, origin_y: 10.0, pixel_width: 1.0, pixel_height: -1.0 };
        let r =
            Raster::new("relief", 2, 2, vec![vec![1.0; 4]], gt, Crs::from_epsg(4326).unwrap(), None, ColorHint::Gray, "Float32").unwrap();
        let tif = temp.path().join("relief.tif");
        std::fs::write(&tif, geotiff::write(&r, 0).unwrap()).unwrap();
        s.run("layer.addData", &json!({"path": tif.display().to_string()})).unwrap();
        assert!(s.map().layers.iter().any(|l| l.is_raster()));
        s.map_mut().tiles.push(TileLayer::new("Gone", TileKind::MbTiles, temp.path().join("gone.mbtiles").display().to_string()));
        let mark = s.notice_mark();
        package(&mut s, &json!({"name": "Lossy"})).unwrap();
        let said: Vec<String> = s.notices_since(mark).iter().map(|n| n.text.clone()).collect();
        assert!(said.iter().any(|t| t.contains("couldn't be put") && t.contains("Gone")), "{said:?}");
        assert!(said.iter().any(|t| t.contains("raster layer(s) were left out")), "{said:?}");
    }

    #[test]
    fn a_returning_maps_rasters_and_a_browsers_archives_are_said() {
        let temp = Temp::new("returning");
        let mut s = sample();
        s.run("offline.download", &json!({"name": "Back", "extent": [-20.0, -40.0, 60.0, 40.0]})).unwrap();
        s.run("offline.open", &json!({"name": "Back"})).unwrap();
        use rustgis_data::raster::{ColorHint, Crs, GeoTransform, Raster, geotiff};
        let gt = GeoTransform { origin_x: 10.0, origin_y: 10.0, pixel_width: 1.0, pixel_height: -1.0 };
        let r =
            Raster::new("relief", 2, 2, vec![vec![1.0; 4]], gt, Crs::from_epsg(4326).unwrap(), None, ColorHint::Gray, "Float32").unwrap();
        let tif = temp.path().join("relief.tif");
        std::fs::write(&tif, geotiff::write(&r, 0).unwrap()).unwrap();
        s.run("layer.addData", &json!({"path": tif.display().to_string()})).unwrap();
        s.map_mut().tiles.push(TileLayer::new("Uploaded", TileKind::MbTiles, format!("{}base.mbtiles", rustgis_map::tiles::UPLOAD)));
        let mark = s.notice_mark();
        let (_, bytes) = package(&mut s, &Value::Null).unwrap();
        let said: Vec<String> = s.notices_since(mark).iter().map(|n| n.text.clone()).collect();
        assert!(said.iter().any(|t| t.contains("raster layer(s) were left out")), "{said:?}");
        assert!(said.iter().any(|t| t.contains("Uploaded") && t.contains("memory")), "{said:?}");
        assert!(!said.iter().any(|t| t.contains("from the web")), "{said:?}");
        // The package holds no raster layer.
        let text = rustgis_data::zip::read(&bytes).unwrap().into_iter().find(|(n, _)| n.ends_with(".rgis")).unwrap().1;
        let project = Project::from_import_json(&String::from_utf8(text).unwrap(), None).unwrap();
        assert!(project.maps[0].layers.iter().all(|l| !l.is_raster()));
    }

    #[test]
    fn a_map_only_named_offline_is_packaged_as_any_other() {
        let mut s = sample();
        s.run("map.rename", &json!({"name": "Plan (offline)"})).unwrap();
        let had = s.project.offline_areas.len();
        // A new area, named past the map that already holds "Plan (offline)".
        let (area, _) = package(&mut s, &json!({"name": "Plan"})).unwrap();
        assert_eq!(area, "Plan 2");
        assert_eq!(s.project.offline_areas.len(), had + 1);
        // The whole world: every feature is in it, and nothing is said to have stayed out.
        let mark = s.notice_mark();
        package(&mut s, &json!({"name": "World", "extent": [-180.0, -90.0, 180.0, 90.0]})).unwrap();
        let said: Vec<String> = s.notices_since(mark).iter().map(|n| n.text.clone()).collect();
        assert!(said.iter().all(|t| !t.contains("outside the offline area's extent")), "{said:?}");
    }

    #[test]
    fn an_offline_map_is_packaged_as_it_is_to_go_back() {
        let mut s = sample();
        s.run("offline.download", &json!({"name": "Back", "extent": [-20.0, -40.0, 60.0, 40.0]})).unwrap();
        s.run("offline.open", &json!({"name": "Back"})).unwrap();
        assert_eq!(s.map().name, "Back (offline)");
        let areas = s.project.offline_areas.len();
        let (area, bytes) = package(&mut s, &Value::Null).unwrap();
        assert_eq!(area, "Back");
        // No new area: the map already is one.
        assert_eq!(s.project.offline_areas.len(), areas);
        let entries = rustgis_data::zip::read(&bytes).unwrap();
        assert_eq!(entries[0].0, "Back.rgis");
    }

    #[test]
    fn a_package_entry_outside_its_folder_is_refused() {
        let temp = Temp::new("refused");
        for bad in ["../escape.txt", "/abs.txt", "a/../../b.txt", "C:/x.txt"] {
            let bytes = rustgis_data::zip::write(&[("p.rgis".into(), b"{}".to_vec()), (bad.into(), b"x".to_vec())]).unwrap();
            let e = unpack_into(&bytes, &temp.path().join("in")).unwrap_err();
            assert!(e.contains("outside"), "{bad}: {e}");
        }
        // Two projects: refused. One that can't be read: refused, with nothing written.
        let two = rustgis_data::zip::write(&[("a.rgis".into(), b"{}".to_vec()), ("b.rgis".into(), b"{}".to_vec())]).unwrap();
        assert!(unpack_into(&two, &temp.path().join("two")).unwrap_err().contains("more than one project"));
        let damaged =
            rustgis_data::zip::write(&[("a.rgis".into(), b"not json".to_vec()), ("basemaps/b.mbtiles".into(), b"x".to_vec())]).unwrap();
        assert!(unpack_into(&damaged, &temp.path().join("damaged")).is_err());
        assert!(!temp.path().join("damaged").exists());
        let none = rustgis_data::zip::write(&[("a.txt".into(), b"x".to_vec())]).unwrap();
        assert!(unpack_into(&none, &temp.path().join("in")).unwrap_err().contains("no project"));
        assert!(is_field_package("Site.RGISFIELD") && !is_field_package("site.rgis"));
        assert!(crate::project::opens_as_project("site.rgisfield"));
    }
}
