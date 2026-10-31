//! Publish tab: export the map as PNG and layers as GeoJSON, CSV, FlatGeobuf, GeoPackage,
//! GeoParquet, Shapefile, KML/KMZ or GPX.

use std::path::Path;

use rustgis_data::raster::Crs;
use rustgis_render::View;
use serde_json::{Value, json};

use super::{f, need_layer, ps};
use crate::{CmdResult, CommandSpec as C, Session};

pub fn commands() -> Vec<C> {
    vec![
        C::new("share.exportMap", "Export Map", "Publish › Output", export_map)
            .view()
            .params(r#"{"path": "*.png", "width"?: px, "height"?: px, "dpi"?: 96..600}"#),
        C::new("share.exportLayer", "Export Features", "Publish › Output", export_layer)
            .view()
            .when(need_layer)
            .params(r#"{"layer"?, "path": "*.geojson|*.csv|*.fgb|*.gpkg|*.parquet|*.shp|*.zip|*.kml|*.kmz|*.gpx", "crs"?: "EPSG:… (or WKT) for a GeoPackage, Shapefile or FlatGeobuf (default: the system the layer was read in, else EPSG:4326)"}"#),
        C::new("share.exportTable", "Export Table", "Publish › Output", |s, p| {
            let path = ps(p, "path").ok_or("path is required")?;
            let t = s.project.tables.get(s.table_index(p)?).ok_or("no such table")?;
            rustgis_data::io::write_path(&t.dataset, Path::new(path)).map_err(|e| e.to_string())?;
            Ok(json!({"path": path}))
        })
        .params(
            r#"{"table": name|id (a standalone table), "path": "*.geojson|*.csv|*.fgb|*.gpkg|*.parquet|*.shp|*.zip|*.kml|*.kmz|*.gpx"}"#,
        )
        .view(),
    ]
}

/// Renders the active map at the current view (or a requested size) to PNG bytes.
pub fn map_png(s: &mut Session, width: Option<f64>, height: Option<f64>, dpi: Option<f64>) -> Result<Vec<u8>, String> {
    let (vw, vh) = s.view_size;
    // NaN would pass through `clamp`; non-finite values fall back to the defaults.
    let fin = |v: Option<f64>| v.filter(|x| x.is_finite());
    let (width, height, dpi) = (fin(width), fin(height), fin(dpi));
    let w = width.unwrap_or(vw).clamp(16.0, 8000.0);
    let h = height.unwrap_or(vh).clamp(16.0, 8000.0);
    let ppp = (dpi.unwrap_or(96.0) / 96.0).clamp(0.5, 6.0);
    let mut camera = s.map().camera;
    // Keep the same ground extent when the size differs from the view.
    if width.is_some() || height.is_some() {
        camera.res *= (vw / w).max(vh / h);
        camera.clamp();
    }
    let view = View { width: w, height: h, pixels_per_point: ppp, camera, sky: false };
    rustgis_render::render_png(s.map_mut(), &view)
}

/// The map view on one PDF page (for printing): the page is the view's shape at 96 points per
/// inch of screen, rendered at `dpi`.
pub fn map_pdf(s: &mut Session, dpi: f64) -> Result<Vec<u8>, String> {
    let (vw, vh) = s.view_size;
    let ppp = (dpi / 96.0).clamp(0.5, 6.0);
    let view =
        View { width: vw.clamp(16.0, 8000.0), height: vh.clamp(16.0, 8000.0), pixels_per_point: ppp, camera: s.map().camera, sky: false };
    let png = rustgis_render::render_png(s.map_mut(), &view)?;
    let pm = rustgis_render::decode_png(&png).ok_or("could not draw the map")?;
    let mm = |pt: f64| pt * 25.4 / 96.0;
    Ok(rustgis_render::pdf::write(&[rustgis_render::pdf::Page::new(&pm, mm(view.width), mm(view.height))], &s.map().name))
}

fn export_map(s: &mut Session, p: &Value) -> CmdResult {
    let path = ps(p, "path").ok_or("path is required")?.to_string();
    // A style added with its own view: the export waits for it to load, as it does for tiles.
    super::tiles::follow_style_view(s, web_time::Duration::from_secs(30));
    let png = map_png(s, f(p, "width"), f(p, "height"), f(p, "dpi"))?;
    rustgis_data::io::write_atomic(std::path::Path::new(&path), &png).map_err(|e| format!("{path}: {e}"))?;
    s.notify(format!("Exported {path}"), false);
    Ok(json!({"path": path, "bytes": png.len()}))
}

/// Writes a layer (a streamed one's whole source) to `path`, saying what the file couldn't hold:
/// features without drawable geometry, a missing `.prj`, Z or M values dropped on import.
fn export_layer(s: &mut Session, p: &Value) -> CmdResult {
    let id = s.layer_id(p)?;
    let path = ps(p, "path").ok_or("path is required")?.to_string();
    let l = s.map().layer(id).ok_or("no layer")?;
    // A streamed layer exports its whole source, not the features read for the view.
    let whole = if l.is_streamed() { Some(l.load_all()?) } else { None };
    let ds = whole.as_ref().unwrap_or(&l.dataset);
    // GeoJSON, KML and GPX are longitude/latitude; the others keep the layer's own system unless
    // told another.
    let ext = Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let crs = match ps(p, "crs").map(str::trim).filter(|c| !c.is_empty()) {
        Some(text) => Crs::parse(text)?,
        None => rustgis_data::io::default_export_crs(ds, &ext),
    };
    let left_out = rustgis_data::io::write_path_in(ds, Path::new(&path), crs).map_err(|e| e.to_string())?;
    let written = ds.features.len().saturating_sub(left_out);
    let zm_note = rustgis_data::zm_export_note(ds, &path);
    // FlatGeobuf, KML, GPX and Shapefile hold features with geometry only (a Shapefile, only
    // geometry it can draw).
    if left_out > 0 {
        s.notify(format!("{path}: {left_out} features without drawable geometry were left out"), false);
    }
    if let Some(note) = zm_note {
        s.notify(note, false);
    }
    if matches!(ext.as_str(), "shp" | "zip") && rustgis_data::raster::wkt::to_wkt(crs).is_none() {
        s.notify(format!("{path}: no .prj written, as {} has no WKT form", crs.label()), false);
    }
    s.notify(format!("Exported {path}"), false);
    Ok(json!({"path": path, "features": written, "leftOut": left_out, "crs": crs.label()}))
}
