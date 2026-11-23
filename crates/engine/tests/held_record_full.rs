//! Opening a project that fills the record of addresses held to the internet says so: from then
//! on every address is held there, for the rest of the session.
//!
//! Its own test binary: it fills the process-wide record, which would hold every other test's
//! addresses to the internet too.

use rustgis_engine::map as rustgis_map;
use rustgis_engine::render::rustgis_tiles::{address::Reach, cache};
use rustgis_engine::{Project, Session};
use serde_json::json;

/// A project with one tile layer held to the internet, with `dates` dated files.
fn held(dates: usize) -> String {
    let mut s = Session::new(Project::default());
    let template = format!("https://held-full-{dates}.example/scene.tif");
    s.run("map.addTiles", &json!({"url": "https://held-full.example/{z}/{x}/{y}.png", "name": "Scene", "overlay": true})).unwrap();
    let mut v: serde_json::Value = serde_json::to_value(&s.project).unwrap();
    let tile = &mut v["maps"][0]["tiles"][0];
    tile["url"] = json!(template);
    tile["internetOnly"] = json!(true);
    tile["timeUrls"] =
        json!((0..dates).map(|i| json!([i as f64, format!("https://held-full-{dates}.example/{i}.tif")])).collect::<Vec<_>>());
    v["requires"] = json!(["internet-only"]);
    v.to_string()
}

#[test]
fn a_project_that_fills_the_record_of_held_addresses_says_so() {
    let said = |s: &Session| s.notices_since(0).iter().filter(|n| n.text.contains("filled the record")).count();
    // Within the record: nothing said, and the user's own addresses are theirs.
    let opened = Session::new(Project::from_json(&held(10)).unwrap());
    assert_eq!(said(&opened), 0);
    assert!(!cache::public_restrictions_full());
    assert_eq!(cache::ceiling_for("http://192.168.1.5/tiles/0/0/0.png"), Reach::Local);
    // Past it (a data cube with more dated files than the record holds): said once, and from then
    // on every address is held to the internet.
    let opened = Session::new(Project::from_json(&held(rustgis_map::tiles::MAX_TIME_URLS)).unwrap());
    assert_eq!(said(&opened), 1, "{:?}", opened.notices_since(0).iter().map(|n| &n.text).collect::<Vec<_>>());
    assert!(cache::public_restrictions_full());
    assert_eq!(cache::ceiling_for("http://192.168.1.5/tiles/0/0/0.png"), Reach::Public);
}
