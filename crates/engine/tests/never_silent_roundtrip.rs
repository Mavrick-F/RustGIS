//! Never silent (AGENTS.md), as a property: a generated layer exported with `share.exportLayer`
//! and added back with `layer.addData` comes back the same, or the export says something. What a
//! format can't hold and says nothing about is listed in [`LIMITS`], with the reason. Layers lean
//! on edges: names formats treat specially, extreme and non-finite numbers, odd text, nulls,
//! multipoints, the antimeridian and the poles.
//!
//! Pull requests run a fixed seed; the nightly a new one with more cases. A failure gives its seed
//! and the smallest layer found that changes without a notice. To run a seed again, or deeper:
//! `PROPERTY_SEED=7 PROPERTY_CASES=2000 cargo test -p rustgis-engine --test never_silent_roundtrip`.
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestCaseError, TestRng, TestRunner};
use rustgis_engine::{
    Project, Session,
    data::{Coord, Dataset, Feature, Field, FieldType, Geometry, GeometryType, Value},
};
use serde_json::json;

const FORMATS: [&str; 8] = ["geojson", "csv", "fgb", "gpkg", "parquet", "shp", "kml", "gpx"];

/// Changes a format makes without a notice, as (format, the start of a change from [`changes`],
/// why). Each is what the format can hold or how its reader works, not a loss of a value's
/// number or text, except trimming and blanks read as null, which are kept here until they are
/// said.
const LIMITS: &[(&str, &str, &str)] = &[
    // CSV holds text: reading trims each cell, types each column from its values (`text_type`)
    // and reads an empty cell as null.
    ("csv", "text trimmed", "reading trims the spaces around a cell"),
    ("csv", "blank text became null", "an empty cell is read as null"),
    ("csv", "field type changed", "a column's type comes from its values"),
    ("csv", "field of nulls type changed", "a column's type comes from its values"),
    ("csv", "boolean became text", "yes/no values are written as True/False, read back as text"),
    ("csv", "number became text", "a column with a value that isn't a number is text"),
    ("csv", "text became a number", "a column of numbers is a number column"),
    ("csv", "infinity or NaN became text", "inf and NaN aren't read as numbers"),
    ("csv", "not read back: nothing", "a layer without geometry or values is an empty file"),
    // GeoJSON types a property from its values.
    ("geojson", "field of nulls type changed", "a property that is null everywhere has no type"),
    ("geojson", "not read back: nothing", "a layer without geometry or values reads as nothing"),
    // dBase pads text with spaces and has no null text; reading makes a whole-number column an
    // integer one.
    ("shp", "text trimmed", "dBase pads text with spaces, and reading trims them"),
    ("shp", "blank text became null", "dBase has no null text: blank text reads as null"),
    ("shp", "field of nulls type changed", "a number column's type comes from its values"),
    ("shp", "field type changed Double → Integer", "a column of whole numbers reads as integers"),
];

/// KML and GPX hold text, read with types from the values as for CSV (`type_text_columns`); the
/// name field is written as the feature's name and read back as `name`, and a field that is
/// null everywhere isn't in the file.
const TEXT_FORMAT_LIMITS: &[&str] = &[
    "text trimmed",
    "blank text became null",
    "field type changed",
    "field of nulls type changed",
    "field of nulls missing",
    "field renamed",
    "boolean became text",
    "number became text",
    "text became a number",
    "infinity or NaN became text",
];

/// Whether `change` is a limit of `ext` listed above.
fn declared(ext: &str, change: &str) -> bool {
    LIMITS.iter().any(|(f, start, _)| *f == ext && change.starts_with(start))
        || matches!(ext, "kml" | "gpx") && TEXT_FORMAT_LIMITS.iter().any(|start| change.starts_with(start))
}

/// A number from the environment, else `default`.
fn env(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// A longitude/latitude, sometimes on an edge: ±180°, the poles, past Web Mercator's 85.05°.
fn coord() -> impl Strategy<Value = Coord> {
    let lon = prop_oneof![4 => -180.0..180.0f64, 1 => prop::sample::select(vec![-180.0, 180.0, 0.0, 1e-9, 179.999_999_999])];
    let lat = prop_oneof![4 => -89.0..89.0f64, 1 => prop::sample::select(vec![-90.0, 90.0, 0.0, 85.06, -85.06])];
    (lon, lat).prop_map(|(x, y)| Coord::new(x, y))
}

/// A closed ring of three to six points around a centre.
fn ring() -> impl Strategy<Value = Vec<Coord>> {
    (coord(), 0.01..5.0f64, 3usize..7).prop_map(|(c, r, n)| {
        let at = |i: usize| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            Coord::new((c.x * 0.9 + r * a.cos()).clamp(-180.0, 180.0), (c.y * 0.9 + r * a.sin()).clamp(-89.0, 89.0))
        };
        (0..=n).map(|i| at(i % n)).collect()
    })
}

/// A geometry of a layer of `kind`: a point layer's points and multipoints, lines of one or two
/// parts, polygons of one or two parts.
fn geometry(kind: GeometryType) -> BoxedStrategy<Geometry> {
    match kind {
        GeometryType::Point => prop_oneof![
            3 => coord().prop_map(Geometry::Point),
            1 => prop::collection::vec(coord(), 1..4).prop_map(Geometry::MultiPoint),
        ]
        .boxed(),
        GeometryType::Polyline => prop::collection::vec(prop::collection::vec(coord(), 2..5), 1..3).prop_map(Geometry::Line).boxed(),
        _ => prop::collection::vec(prop::collection::vec(ring(), 1..2), 1..3).prop_map(Geometry::Polygon).boxed(),
    }
}

/// Field names, leaning on ones that formats treat specially.
fn field_name() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "[a-z][a-z0-9_]{0,7}",
        1 => prop::sample::select(vec![
            "id", "fid", "name", "NAME", "geometry", "a_very_long_field_name", "a_very_long_field_other", "Ünïcode", "名前",
            "with space", "x", "y", "lat", "lon", "longitude", "WKT", "1st",
        ])
        .prop_map(String::from),
    ]
}

/// Values of a field's type, leaning on edges: extremes, NaN, long and odd text, nulls.
fn value(ty: FieldType) -> BoxedStrategy<Value> {
    let v = match ty {
        FieldType::Integer => prop_oneof![
            4 => (-1000i64..1000).prop_map(Value::Int),
            1 => prop::sample::select(vec![i64::MAX, i64::MIN, 2i64.pow(53) + 1, 2_147_483_648]).prop_map(Value::Int),
        ]
        .boxed(),
        FieldType::Double => prop_oneof![
            4 => (-1e6..1e6f64).prop_map(Value::Float),
            1 => prop::sample::select(vec![0.1, -0.0, 1e300, 5e-324, 0.1234567891, f64::NAN, f64::INFINITY]).prop_map(Value::Float),
        ]
        .boxed(),
        FieldType::Text => prop_oneof![
            4 => "[a-zA-Z0-9 ]{0,12}".prop_map(Value::Text),
            1 => prop::sample::select(vec![
                "", " lead", "comma, here", "quote \"x\"", "line\nbreak", "Ünïcødé 名前 😀", "NULL", "123", "true", "=1+1", "<b>&amp;</b>",
            ])
            .prop_map(|s| Value::Text(s.to_string())),
            1 => (1usize..400).prop_map(|n| Value::Text("é".repeat(n))),
        ]
        .boxed(),
        _ => any::<bool>().prop_map(Value::Bool).boxed(),
    };
    prop_oneof![6 => v, 1 => Just(Value::Null)].boxed()
}

/// A layer: a geometry kind, up to four fields of any type, up to four features (a few without
/// geometry).
fn layer() -> impl Strategy<Value = Dataset> {
    let kind = prop::sample::select(vec![GeometryType::Point, GeometryType::Polyline, GeometryType::Polygon]);
    let types = prop::sample::select(vec![FieldType::Integer, FieldType::Double, FieldType::Text, FieldType::Boolean]);
    let fields = prop::collection::vec((field_name(), types), 0..5).prop_map(|fs| {
        let mut seen = std::collections::HashSet::new();
        fs.into_iter().filter(|(n, _)| seen.insert(n.clone())).map(|(n, t)| Field::new(n, t)).collect::<Vec<_>>()
    });
    (kind, fields).prop_flat_map(|(kind, fields)| {
        let attrs: Vec<_> = fields.iter().map(|f| value(f.ty)).collect();
        let feature = (prop::option::weighted(0.9, geometry(kind)), attrs);
        prop::collection::vec(feature, 1..5).prop_map(move |features| {
            let mut ds = Dataset::new("layer", kind, fields.clone());
            for (g, attrs) in features {
                ds.push(g, attrs);
            }
            ds
        })
    })
}

/// Exports `ds` as `ext` and adds the file back: what the export said, besides naming the file,
/// and the layer read back, or why there is none. `None` when the export refused the layer, which
/// isn't silent.
fn trip(ds: &Dataset, ext: &str, dir: &std::path::Path) -> Option<(Vec<String>, Result<Dataset, String>)> {
    let path = dir.join(format!("layer.{ext}"));
    let mut s = Session::new(Project::default());
    s.map_mut().add_dataset(ds.clone());
    let mark = s.notice_mark();
    s.run("share.exportLayer", &json!({"layer": "layer", "path": path})).ok()?;
    let said: Vec<String> = s.notices_since(mark).iter().map(|n| n.text.clone()).filter(|t| !t.starts_with("Exported ")).collect();
    let before: Vec<u32> = s.map().layers.iter().map(|l| l.id).collect();
    if let Err(e) = s.run("layer.addData", &json!({"path": path})) {
        return Some((said, Err(e)));
    }
    // A format read as several layers (GPX: tracks and their points) gives back the one of the
    // layer's kind.
    let added: Vec<&Dataset> = s.map().layers.iter().filter(|l| !before.contains(&l.id)).map(|l| &l.dataset).collect();
    let back = added.iter().find(|d| d.geometry_type == ds.geometry_type).or(added.first()).map(|d| (*d).clone());
    Some((said, back.ok_or_else(|| "nothing".to_string())))
}

/// The value's type, to name a change.
fn tag(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Int(_) => "integer",
        Value::Float(_) => "number",
        Value::Text(_) => "text",
        _ => "boolean",
    }
}

/// A value as a number, if it is one.
fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        _ => None,
    }
}

/// How a value changed, if it did; the same number in another type isn't a change. The kinds
/// name what a format that holds text does to a value, so [`LIMITS`] can allow just that.
fn value_change(a: &Value, b: &Value) -> Option<String> {
    let same = match (a, b) {
        (Value::Float(x), Value::Float(y)) => x == y || x.is_nan() && y.is_nan(),
        _ => a == b || number(a).zip(number(b)).is_some_and(|(x, y)| x == y),
    };
    if same {
        return None;
    }
    let reads_as = |t: &str, v: &Value| number(v).is_some_and(|n| t.trim().parse::<f64>().is_ok_and(|m| m == n));
    Some(match (a, b) {
        (Value::Text(x), Value::Null) if x.trim().is_empty() => "blank text became null".into(),
        (Value::Text(x), Value::Text(y)) if x.trim() == y.trim() => "text trimmed".into(),
        (Value::Text(x), _) if reads_as(x, b) => "text became a number".into(),
        (Value::Float(x), Value::Text(_)) if !x.is_finite() => "infinity or NaN became text".into(),
        (_, Value::Text(y)) if reads_as(y, a) => "number became text".into(),
        (Value::Bool(x), Value::Text(y)) if y == if *x { "True" } else { "False" } => "boolean became text".into(),
        _ if tag(a) == tag(b) => format!("{} changed", tag(a)),
        _ => format!("{} became {}", tag(a), tag(b)),
    })
}

/// A geometry's parts, with coordinates to 1e-7°, each ring from its least point in one direction,
/// and polygon parts in order: what a format may rearrange without changing the shape.
fn shape(g: &Geometry) -> Vec<Vec<(i64, i64)>> {
    let q = |c: &Coord| ((c.x * 1e7).round() as i64, (c.y * 1e7).round() as i64);
    let ring = |r: &[Coord]| {
        let mut r: Vec<_> = r.iter().map(q).collect();
        if r.len() > 1 && r.first() == r.last() {
            r.pop();
        }
        let start = |r: &[(i64, i64)]| {
            let mut r = r.to_vec();
            let i = r.iter().enumerate().min_by_key(|(_, p)| **p).map_or(0, |(i, _)| i);
            r.rotate_left(i);
            r
        };
        let forward = start(&r);
        r.reverse();
        forward.min(start(&r))
    };
    match g {
        Geometry::Point(c) => vec![vec![q(c)]],
        Geometry::MultiPoint(cs) => cs.iter().map(|c| vec![q(c)]).collect(),
        Geometry::Line(ls) => ls.iter().map(|l| l.iter().map(q).collect()).collect(),
        Geometry::Polygon(ps) => {
            let mut rings: Vec<_> = ps.iter().flatten().map(|r| ring(r)).collect();
            rings.sort();
            rings
        }
    }
}

/// What changed in one feature: its geometry and the values of the fields in `map`.
fn feature_changes(x: &Feature, y: &Feature, map: &[(usize, Option<usize>)], fields: &[Field]) -> Vec<String> {
    let mut out = Vec::new();
    match (&x.geometry, &y.geometry) {
        (Some(g), Some(h)) if shape(g) != shape(h) => out.push("geometry changed".to_string()),
        (Some(_), None) => out.push("geometry dropped".into()),
        (None, Some(_)) => out.push("geometry made up".into()),
        _ => {}
    }
    for &(i, j) in map {
        let (Some(j), Some(f)) = (j, fields.get(i)) else { continue };
        let (a, b) = (x.attrs.get(i).unwrap_or(&Value::Null), y.attrs.get(j).unwrap_or(&Value::Null));
        if let Some(why) = value_change(a, b) {
            out.push(format!("{why} ({}: {a:?} → {b:?})", f.name));
        }
    }
    out
}

/// What changed between a layer and the layer read back. Features are matched by content, since
/// a format may reorder them (FlatGeobuf sorts them along its spatial index); fields by name, and
/// a format may add its own.
fn changes(a: &Dataset, b: &Dataset) -> Vec<String> {
    let mut out = Vec::new();
    let mut map = Vec::new();
    for (i, f) in a.fields.iter().enumerate() {
        // A field holding only nulls gives a reader that types fields from their values nothing
        // to go on.
        let nulls = if a.features.iter().all(|x| x.attrs.get(i).is_none_or(|v| *v == Value::Null)) { " of nulls" } else { "" };
        let exact = b.fields.iter().position(|g| g.name == f.name);
        let j = exact.or_else(|| b.fields.iter().position(|g| g.name.eq_ignore_ascii_case(&f.name)));
        match j.and_then(|j| b.fields.get(j)) {
            None => out.push(format!("field{nulls} missing ({})", f.name)),
            Some(g) if exact.is_none() => out.push(format!("field renamed ({} → {})", f.name, g.name)),
            Some(g) if g.ty != f.ty => out.push(format!("field{nulls} type changed {:?} → {:?} ({})", f.ty, g.ty, f.name)),
            Some(_) => {}
        }
        map.push((i, j));
    }
    let (fa, fb) = (a.features.iter().collect::<Vec<_>>(), b.features.iter().collect::<Vec<_>>());
    if fa.len() != fb.len() {
        out.push(format!("feature count changed ({} → {})", fa.len(), fb.len()));
        return out;
    }
    let mut used = vec![false; fb.len()];
    for x in fa {
        let best =
            (0..fb.len()).filter(|&j| !used[j]).map(|j| (feature_changes(x, fb[j], &map, &a.fields), j)).min_by_key(|(c, _)| c.len());
        if let Some((c, j)) = best {
            used[j] = true;
            out.extend(c);
        }
    }
    out
}

#[test]
fn an_export_that_changes_a_layer_says_so() {
    let dir = std::env::temp_dir().join(format!("rustgis-never-silent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let seed = env("PROPERTY_SEED", 1);
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(&seed.to_le_bytes());
    // About 6 ms a case.
    let cases = u32::try_from(env("PROPERTY_CASES", 256)).unwrap_or(256);
    let config = Config { cases, failure_persistence: None, ..Config::default() };
    let mut runner = TestRunner::new_with_rng(config, TestRng::from_seed(RngAlgorithm::ChaCha, &bytes));
    let result = runner.run(&layer(), |ds| {
        for ext in FORMATS {
            let sub = dir.join(ext);
            let _ = std::fs::remove_dir_all(&sub);
            std::fs::create_dir_all(&sub).unwrap();
            let Some((said, back)) = trip(&ds, ext, &sub) else { continue };
            let found = back.map_or_else(|e| vec![format!("not read back: {e}")], |back| changes(&ds, &back));
            let silent: Vec<String> = found.into_iter().filter(|c| !declared(ext, c)).collect();
            if said.is_empty() && !silent.is_empty() {
                return Err(TestCaseError::fail(format!("{ext}: changed without a notice: {silent:?}")));
            }
        }
        Ok(())
    });
    let _ = std::fs::remove_dir_all(&dir);
    if let Err(e) = result {
        panic!("PROPERTY_SEED={seed}: {e}");
    }
}
