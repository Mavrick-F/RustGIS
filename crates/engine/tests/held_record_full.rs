//! Opening a project that fills the record of addresses held to the internet says so: from then
//! on every address is held there, for the rest of the session. With the record full, a layer read
//! marked keeps its mark, and one held only because the record is full is said when written.
//!
//! Its own test binary, with one test in order: it fills the process-wide record, which would hold
//! every other test's addresses to the internet too.

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
fn a_full_record_of_held_addresses_is_said_and_keeps_what_was_marked() {
    let full = |s: &Session| s.notices_since(0).iter().filter(|n| n.text.contains("filled the record")).count();
    let unmarked = |text: &str| text.contains("only because this session's record of such addresses is full");
    // Within the record: nothing said, and the user's own addresses are theirs.
    let opened = Session::new(Project::from_json(&held(10)).unwrap());
    assert_eq!(full(&opened), 0);
    let (_, report) = opened.project.clone().saved_text(None).unwrap();
    assert!(!report.notices().iter().any(|n| unmarked(n)), "nothing said with room in the record");
    assert!(!cache::public_restrictions_full());
    assert_eq!(cache::ceiling_for("http://192.168.1.5/tiles/0/0/0.png"), Reach::Local);
    // Past it (a data cube with more dated files than the record holds): said once, and from then
    // on every address is held to the internet.
    let opened = Session::new(Project::from_json(&held(rustgis_map::tiles::MAX_TIME_URLS)).unwrap());
    assert_eq!(full(&opened), 1, "{:?}", opened.notices_since(0).iter().map(|n| &n.text).collect::<Vec<_>>());
    assert!(cache::public_restrictions_full());
    assert_eq!(cache::ceiling_for("http://192.168.1.5/tiles/0/0/0.png"), Reach::Public);
    // A layer read marked keeps its mark, though the record has no room for its addresses; one
    // added since, held only because the record is full, is written unmarked, and that is said.
    let mut s = Session::new(Project::from_json(&held(7)).unwrap());
    s.run("map.addTiles", &json!({"url": "https://after-full.example/{z}/{x}/{y}.png", "name": "Later", "overlay": true})).unwrap();
    let (text, report) = s.project.saved_text(None).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let marked = |name: &str| v["maps"][0]["tiles"].as_array().unwrap().iter().any(|t| t["name"] == name && t["internetOnly"] == true);
    assert!(marked("Scene"), "read marked, kept marked");
    assert!(!marked("Later"));
    let said: Vec<String> = report.notices().into_iter().filter(|n| unmarked(n)).collect();
    assert_eq!(said.len(), 1, "{:?}", report.notices());
    assert!(said[0].starts_with("1 layer(s)"), "{said:?}");
}
