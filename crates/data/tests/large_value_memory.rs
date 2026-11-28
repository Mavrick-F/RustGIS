//! The memory a download's writers take for one large value or shape: it is neither copied nor
//! built into a record before the limit refuses it. A test binary of its own, as the peak it
//! reads is the whole process's.
#![cfg(target_os = "linux")]

use rustgis_data::capped::Limit;
use rustgis_data::raster::Crs;
use rustgis_data::{Coord, Dataset, Field, FieldType, Geometry, Value};

/// The bytes of each large value or shape.
const ITEM: usize = 32 << 20;
/// The room its file is held to.
const ROOM: usize = 1 << 20;
/// The most the peak may grow while an item is written: a quarter of the item.
const MOST: u64 = 8 << 20;

/// The most memory this process has held since the peak was last reset, in bytes.
fn peak_bytes() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find_map(|l| l.strip_prefix("VmHWM:")).unwrap();
    line.trim().trim_end_matches("kB").trim().parse::<u64>().unwrap() * 1024
}

/// How far `run` takes the peak past what is held as it starts.
fn growth(run: impl FnOnce()) -> u64 {
    // Linux sets the peak back to what is held now.
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let before = peak_bytes();
    run();
    peak_bytes().saturating_sub(before)
}

/// A one-feature layer of `geometry` with the text `value` in a field.
fn layer(geometry: Geometry, field: &str, value: String) -> Dataset {
    let mut ds = Dataset::new("large", geometry.kind(), vec![Field::new(field, FieldType::Text)]);
    ds.push(Some(geometry), vec![Value::Text(value)]);
    ds
}

#[test]
fn a_large_value_or_shape_is_not_copied_for_a_download() {
    let point = || Geometry::Point(Coord::new(1.0, 2.0));
    let plain = || "x".repeat(ITEM);
    let formula = || format!("={}", "\"x".repeat(ITEM / 2));
    // Text holding numbers, as a JSON document in a cell does.
    let digits = || "{\"a\": 1.5} ".repeat(ITEM / 11);
    let vertices = || (0..ITEM / 16).map(|i| Coord::new((i % 360) as f64 - 180.0, 1.5)).collect::<Vec<_>>();
    let mut over = Vec::new();
    let mut check = |name: &str, grew: u64| {
        if grew >= MOST {
            over.push(format!("{name}: the peak grew {} MiB", grew >> 20));
        }
    };

    // Each text format stops a file at the room without a copy of the value.
    for (name, format, name_field, field, value) in [
        ("KML description", "kml", None, "description", plain()),
        ("KML attribute", "kml", None, "note", plain()),
        ("GPX name", "gpx", Some(0), "note", plain()),
        ("WKT cell", "wkt", None, "note", plain()),
        ("WKT guarded cell", "wkt", None, "note", formula()),
        ("CSV cell", "csv", None, "note", plain()),
        ("CSV guarded cell", "csv", None, "note", formula()),
    ] {
        let ds = layer(point(), field, value);
        let limit = Limit::new(ROOM);
        let grew = growth(|| assert!(rustgis_data::io::text_bytes_within(&ds, format, name_field, &limit).is_err(), "{name}"));
        assert!(limit.passed(), "{name}");
        check(name, grew);
    }

    // The counting of what a file loses runs before it is written.
    for (name, ext, value) in
        [("CSV notices", "csv", digits()), ("CSV guarded notices", "csv", formula()), ("KML notices", "kml", digits())]
    {
        let ds = layer(point(), "note", value);
        let grew = growth(|| {
            rustgis_data::io::export_losses_notes_in(&ds, ext, None, "t", Crs::LonLat).unwrap();
        });
        check(name, grew);
    }

    // A dBase column keeps 254 bytes of a value, so the Shapefile fits.
    let ds = layer(point(), "note", plain());
    let grew = growth(|| {
        rustgis_data::io::write_bytes_noted_within(&ds, "shp", Crs::LonLat, "t", &Limit::new(ROOM)).unwrap();
    });
    check("dBase value", grew);
    let grew = growth(|| assert_eq!(rustgis_data::io::shapefile_values_notes(&ds, "t").len(), 1));
    check("dBase notices", grew);

    // A shape whose record can't fit is refused before its points are copied.
    for (name, geometry) in [
        ("Shapefile line", Geometry::Line(vec![vertices()])),
        ("Shapefile polygon", Geometry::Polygon(vec![vec![vertices()]])),
        ("Shapefile multipoint", Geometry::MultiPoint(vertices())),
    ] {
        let ds = layer(geometry, "note", String::new());
        let limit = Limit::new(ROOM);
        let grew = growth(|| {
            assert!(rustgis_data::io::write_bytes_noted_within(&ds, "shp", Crs::LonLat, "t", &limit).is_err(), "{name}");
        });
        assert!(limit.passed(), "{name}");
        check(name, grew);
    }

    assert!(over.is_empty(), "{over:#?}");
}
