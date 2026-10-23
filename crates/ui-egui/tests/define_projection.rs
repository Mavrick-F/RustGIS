//! Drive the real Define Projection dialog and its async import handoff.
use egui_kittest::kittest::Queryable;
use rustgis_engine::{
    Project, Session,
    data::{Coord, Dataset, Geometry, GeometryType},
};
use rustgis_ui_egui::{RustGisApp, Services};
use serde_json::json;

/// Two layers let a selection change expose accidental retargeting while a dialog is open.
fn harness() -> egui_kittest::Harness<'static, RustGisApp> {
    let mut s = Session::new(Project::default());
    for name in ["Parcels", "Other"] {
        let mut ds = Dataset::new(name, GeometryType::Point, vec![]);
        ds.push(Some(Geometry::Point(Coord::new(451_219.33, 5_411_550.20))), vec![]);
        s.map_mut().add_dataset(ds);
    }
    s.run("layer.select", &json!({"layer": "Parcels"})).unwrap();
    let app = RustGisApp::new(s);
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 1000.0)).with_max_steps(64).build_ui_state(
        |ui, app: &mut RustGisApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.state_mut().ui.command_search = Some("layer.defineProjection".into());
    h.run_steps(3);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().dialogs.open, Some("defineProjection"));
    h
}

/// Types through the labelled text widget after the dialog has settled, including replacing
/// its previous CRS. No form state is injected around the submission path.
fn enter_crs(h: &mut egui_kittest::Harness<'_, RustGisApp>, crs: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Source coordinate system").focus();
    h.step();
    h.event(egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND });
    h.event(egui::Event::Text(crs.into()));
    h.run_steps(3);
}

#[test]
fn dialog_applies_to_captured_layer_and_undo_restores_native_coordinates() {
    let mut h = harness();
    h.state_mut().run("layer.select", json!({"layer": "Other"})).unwrap();
    enter_crs(&mut h, "EPSG:32631");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply").click();
    h.step();
    assert!(h.state().dialogs.open.is_none());
    let point = |app: &RustGisApp, name: &str| {
        app.session.map().layer_by_name(name).unwrap().dataset.features[0].geometry.as_ref().unwrap().label_point().unwrap()
    };
    assert!((point(h.state(), "Parcels").x - 2.335).abs() < 1e-6);
    assert_eq!(point(h.state(), "Other").x, 451_219.33);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(point(h.state(), "Parcels").x, 451_219.33);
}

#[test]
fn import_dialog_passes_crs_to_browser_upload_before_wrong_prj_is_read() {
    use std::sync::{Arc, Mutex};
    let opened = Arc::new(Mutex::new(Vec::new()));
    let captured = opened.clone();
    let mut h = harness();
    h.state_mut().services =
        Services { open_async: Some(Box::new(move |kind| captured.lock().unwrap().push(kind.to_string()))), ..Services::default() };
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Import Original File…").click();
    h.run_steps(3);
    assert_eq!(h.state().dialogs.open, Some("addShapefile"));
    enter_crs(&mut h, "EPSG:32631");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Choose File…").click();
    h.step();
    assert_eq!(*opened.lock().unwrap(), ["shapefile"], "dialog error: {:?}", h.state().dialogs.error);
    let ds = h.state().session.map().layer_by_name("Parcels").unwrap().dataset.clone();
    let parts = rustgis_engine::data::io::shapefile_parts_in(&ds, rustgis_engine::data::raster::Crs::LonLat).unwrap().0;
    let entries = parts.into_iter().map(|(ext, bytes)| (format!("parcels.{ext}"), bytes)).collect::<Vec<_>>();
    let bytes = rustgis_engine::data::zip::write(&entries).unwrap();
    // A drop or another picker is unmarked and must not inherit this import's CRS.
    let ordinary = h.state_mut().open_picked("ordinary.zip", &bytes).unwrap();
    let ordinary_id = ordinary["layer"].as_u64().unwrap() as u32;
    assert_eq!(
        h.state().session.map().layer(ordinary_id).unwrap().dataset.features[0].geometry.as_ref().unwrap().label_point().unwrap().x,
        451_219.33
    );
    let result = h.state_mut().open_picked(&format!("{}parcels.zip", rustgis_ui_egui::SHAPEFILE_PICK), &bytes).unwrap();
    let id = result["layer"].as_u64().unwrap() as u32;
    let point = h.state().session.map().layer(id).unwrap().dataset.features[0].geometry.as_ref().unwrap().label_point().unwrap();
    assert!((point.x - 2.335).abs() < 1e-6, "{point:?}");
    // The browser can return several grouped Shapefiles from one pick.
    let second = h.state_mut().open_picked(&format!("{}another.zip", rustgis_ui_egui::SHAPEFILE_PICK), &bytes).unwrap();
    let second_id = second["layer"].as_u64().unwrap() as u32;
    assert_eq!(h.state().session.map().layer(second_id).unwrap().dataset.crs, h.state().session.map().layer(id).unwrap().dataset.crs);
}

#[test]
fn dialog_rejects_a_different_map_with_the_same_layer_id() {
    let mut h = harness();
    let original_id = h.state().session.map().layer_by_name("Parcels").unwrap().id;
    h.state_mut().run("map.new", json!({"name": "Second Map"})).unwrap();
    let mut ds = Dataset::new("Unrelated", GeometryType::Point, vec![]);
    ds.push(Some(Geometry::Point(Coord::new(451_219.33, 5_411_550.20))), vec![]);
    let id = h.state_mut().session.map_mut().add_dataset(ds);
    assert_eq!(id, original_id, "the collision is the regression's precondition");
    h.state_mut().session.active_layer = Some(id);
    h.run_steps(3);
    enter_crs(&mut h, "EPSG:32631");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply").click();
    h.run_steps(3);
    assert!(h.state().dialogs.error.as_deref().is_some_and(|e| e.contains("project or map changed")), "{:?}", h.state().dialogs.error);
    let current = &h.state().session.map().layer(id).unwrap().dataset;
    assert_eq!(current.features[0].geometry.as_ref().unwrap().label_point().unwrap().x, 451_219.33);
    let original = &h.state().session.project.maps[0].layer(original_id).unwrap().dataset;
    assert_eq!(original.features[0].geometry.as_ref().unwrap().label_point().unwrap().x, 451_219.33);
}

#[test]
fn dialog_rejects_a_replacement_project_with_the_same_map_and_layer() {
    let mut h = harness();
    let map = h.state().session.map().name.clone();
    let id = h.state().session.map().layer_by_name("Parcels").unwrap().id;
    h.state_mut().run("project.new", json!({})).unwrap();
    let mut ds = Dataset::new("Unrelated", GeometryType::Point, vec![]);
    ds.push(Some(Geometry::Point(Coord::new(451_219.33, 5_411_550.20))), vec![]);
    assert_eq!(h.state_mut().session.map_mut().add_dataset(ds), id);
    assert_eq!(h.state().session.map().name, map);
    h.state_mut().session.active_layer = Some(id);
    h.run_steps(3);
    enter_crs(&mut h, "EPSG:32631");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply").click();
    h.run_steps(3);
    assert!(h.state().dialogs.error.as_deref().is_some_and(|e| e.contains("project or map changed")), "{:?}", h.state().dialogs.error);
    let current = &h.state().session.map().layer(id).unwrap().dataset;
    assert_eq!(current.features[0].geometry.as_ref().unwrap().label_point().unwrap().x, 451_219.33);
}
