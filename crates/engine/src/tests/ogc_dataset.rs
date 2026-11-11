//! Dataset discovery is exercised through commands, with the same documents fetched natively
//! and replayed incrementally as the browser does. The fixture invents no endpoint or layer id.

use super::{Project, Session, ogc_server};
use ogc_server::{Route, Server};
use serde_json::{Value, json};

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn ogc_dataset_supplied_listing_reads_off_frame_with_its_login() {
    use crate::analysis::control::Control;
    use crate::cmd::reads::{read_ahead, read_land};
    let mut session = Session::new(Project::default());
    let connection = session
        .run("connection.new", &json!({"kind":"ogcapi","address":"https://maps.example.org","auth":"header","keyName":"X-Key"}))
        .unwrap();
    session.run("connection.login", &json!({"id":connection["id"],"apiKey":"synthetic-dataset-read-ahead"})).unwrap();
    for (text, filter, count) in [
        (json!({"links":[{"rel":"tilesets-map","href":"/tiles"}]}).to_string(), None, 0),
        (json!({"collections":[{"id":"roads"},{"id":"water"}]}).to_string(), Some("roads"), 1),
    ] {
        let mut params = json!({"url":"https://maps.example.org/dataset","type":"ogcapi","text":text,"connection":connection["id"]});
        if let Some(filter) = filter {
            params["filter"] = json!(filter);
        }
        let expected = session.run("service.layers", &params).unwrap();
        assert_eq!(expected["count"], count, "{expected}");
        let read = read_ahead(&mut session, "service.layers", &params).unwrap();
        assert!(read.reads());
        let read = read.run(&Control::new()).unwrap();
        assert_eq!(read_land(&mut session, read).unwrap(), expected, "the worker retains the initiating login binding and filter");
    }
}

#[test]
fn ogc_dataset_rejected_landing_links_never_guess_a_template() {
    let mut session = Session::new(Project::default());
    for (kind, href, reason) in [
        ("text/html", "/tiles".to_string(), "no usable JSON link"),
        ("application/json", format!("https://maps.example.org/{}", "a".repeat(8200)), "addresses exceed 8192 bytes"),
    ] {
        let text = json!({"links":[{"rel":"tilesets-map","type":kind,"href":href}]}).to_string();
        let listing =
            session.run("service.layers", &json!({"url":"https://maps.example.org/dataset","type":"ogcapi","text":text})).unwrap();
        assert_eq!(listing["count"], 0, "a rejected link cannot enable a guessed template");
        assert!(listing["next"].is_null());
        assert_eq!(listing["notes"].as_array().unwrap().len(), 1, "{listing}");
        assert!(listing["notes"][0].as_str().unwrap().starts_with("1 tileset documents were left out"));
        assert!(listing["notes"][0].as_str().unwrap().contains(reason), "{listing}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn ogc_dataset_prepared_add_keeps_error_path_losses() {
    use crate::cmd::reads::{read_ahead, read_land};
    for query in [false, true] {
        let url = if query { "https://maps.example.org/landing?token=synthetic-query" } else { "https://maps.example.org/landing" };
        let text = json!({"links":[{"rel":"data","type":"application/json","href":"https://other.example.org/collections"}]}).to_string();
        let mut params = json!({"url":url,"type":"ogcapi","layer":"map","text":text});
        let listing = Session::new(Project::default()).run("service.layers", &params).unwrap();
        params["documents"] = json!([{"url":listing["next"],"text":"not JSON"}]);
        let mut direct = Session::new(Project::default());
        assert!(direct.run("service.add", &params).is_err());
        let notes = |s: &Session| s.notices.iter().filter(|n| !n.error).map(|n| n.text.clone()).collect::<Vec<_>>();
        let expected = notes(&direct);
        assert_eq!(expected.len(), usize::from(query), "{expected:?}");
        let mut prepared = Session::new(Project::default());
        let read = read_ahead(&mut prepared, "service.add", &params).unwrap().run(&crate::analysis::control::Control::new()).unwrap();
        assert!(read_land(&mut prepared, read).is_err());
        assert_eq!(notes(&prepared), expected, "prepared errors must retain counted losses after rollback");
    }
}

/// A dataset offering two projections and styles whose ids deliberately misname their grid.
fn documents() -> Vec<(&'static str, Value)> {
    vec![
        (
            "/dataset",
            json!({"title":"Dataset", "links":[
                {"rel":"tilesets-vector","type":"application/json","href":"/tiles?f=json"},
                {"rel":"styles","type":"application/json","href":"/styles?f=json"}
            ]}),
        ),
        (
            "/tiles",
            json!({"tilesets":[
                {"tileMatrixSetId":"NetherlandsRDNewQuad","dataType":"vector","links":[
                    {"rel":"item","type":"application/vnd.mapbox-vector-tile","href":"/rd/{tileMatrix}/{tileRow}/{tileCol}?f=mvt"}]},
                {"tileMatrixSetId":"WebMercatorQuad","dataType":"vector","links":[
                    {"rel":"item","type":"application/vnd.mapbox-vector-tile","href":"/wm/{tileMatrix}/{tileRow}/{tileCol}?f=mvt"}]}
            ]}),
        ),
        (
            "/styles",
            json!({"styles":[
                {"id":"rd-name","title":"Supported","links":[{"rel":"stylesheet","type":"application/vnd.mapbox.style+json","href":"/good?f=mapbox"}]},
                {"id":"mercator-name","links":[{"rel":"stylesheet","type":"application/vnd.mapbox.style+json","href":"/wrong?f=mapbox"}]}
            ]}),
        ),
        (
            "/good",
            json!({"version":8,"sources":{"basemap":{"type":"vector","tiles":["/wm/{z}/{y}/{x}?f=mvt"]}},
            "layers":[{"id":"roads","type":"line","source":"basemap","source-layer":"roads"}]}),
        ),
        (
            "/wrong",
            json!({"version":8,"sources":{"basemap":{"type":"vector","tiles":["/rd/{z}/{y}/{x}?f=mvt"]}},
            "layers":[{"id":"roads","type":"line","source":"basemap","source-layer":"roads"}]}),
        ),
    ]
}

#[test]
fn ogc_dataset_styles_use_advertised_projection_and_native_web_parity() {
    let docs = documents();
    let server = Server::new(docs.iter().map(|(path, body)| Route::new(path, &body.to_string())).collect());
    let url = format!("{}/dataset", server.base);
    let mut session = Session::new(Project::default());
    let native = session.run("service.layers", &json!({"url":url,"type":"ogcapi"})).unwrap();
    assert_eq!(native["kind"], "OGC API Tiles");
    assert_eq!(native["count"], 1, "{native}");
    assert_eq!(native["layers"][0]["name"], "rd-name");
    assert_eq!(native["layers"][0]["offer"]["type"], "style");
    assert!(native["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().starts_with("1 styles were left out")));
    let requests = server.requests().len();
    assert_eq!(requests, docs.len());
    #[cfg(not(target_arch = "wasm32"))]
    {
        use crate::cmd::reads::{read_ahead, read_land};
        let read = read_ahead(&mut session, "service.layers", &json!({"url":url,"type":"ogcapi"})).unwrap();
        assert!(read.reads(), "URL-only discovery must fetch on the worker too");
        let read = read.run(&crate::analysis::control::Control::new()).unwrap();
        assert_eq!(read_land(&mut session, read).unwrap(), native);
        assert_eq!(server.requests().len(), requests + docs.len());
    }
    let requests = server.requests().len();
    let mut params = json!({"url":url,"type":"ogcapi","text":docs[0].1.to_string(),"documents":[]});
    loop {
        let listing = session.run("service.layers", &params).unwrap();
        let Some(next) = listing["next"].as_str() else {
            assert_eq!(listing, native);
            break;
        };
        let address = url::Url::parse(next).unwrap();
        let body = &docs.iter().find(|(path, _)| *path == address.path()).unwrap().1;
        params["documents"].as_array_mut().unwrap().push(json!({"url":next,"text":body.to_string()}));
    }
    params["layer"] = json!("rd-name");
    params["source_url"] = native["layers"][0]["source_url"].clone();
    session.run("service.add", &params).unwrap();
    assert_eq!(session.map().tiles.len(), 1);
    assert_eq!(session.map().tiles[0].url, format!("{}/good?f=mapbox", server.base));
    #[cfg(not(target_arch = "wasm32"))]
    {
        use crate::cmd::reads::{read_ahead, read_land};
        let mut copy = Session::new(Project::default());
        let connection =
            copy.run("connection.new", &json!({"kind":"ogcapi","address":server.base,"auth":"header","keyName":"X-Key"})).unwrap();
        copy.run("connection.login", &json!({"id":connection["id"],"apiKey":"synthetic-service-add","allowInsecure":true})).unwrap();
        params["connection"] = connection["id"].clone();
        let read = read_ahead(&mut copy, "service.add", &params).unwrap();
        assert!(read.reads());
        let done = read.run(&crate::analysis::control::Control::new()).unwrap();
        assert!(copy.map().tiles.is_empty(), "discovery cannot install a layer before landing");
        read_land(&mut copy, done).unwrap();
        let tile = &copy.map().tiles[0];
        assert_eq!(tile.connection, connection["id"].as_str().unwrap());
        assert!(crate::render::rustgis_tiles::logins::apply(&tile.url).unwrap().is_some());
        assert_eq!(crate::render::rustgis_tiles::logins::inner(&tile.url), format!("{}/good?f=mapbox", server.base));
    }
    assert_eq!(server.requests().len(), requests, "supplied discovery and add never refetch documents");
}

#[test]
fn ogc_dataset_styles_require_templates_allowed_from_the_tileset_document() {
    for (origin, template, allowed) in [
        ("https://maps.example.org", "http://127.0.0.1/{z}/{y}/{x}.pbf", false),
        ("https://maps.example.org", "https://tiles.example.org/{z}/{y}/{x}.pbf", true),
        ("http://127.0.0.1", "http://127.0.0.1/{z}/{y}/{x}.pbf", true),
    ] {
        let root = json!({"links":[{"rel":"tilesets-vector","href":"/tiles?f=json"},{"rel":"styles","href":"/styles?f=json"}]});
        let item = template.replace("{z}", "{tileMatrix}").replace("{y}", "{tileRow}").replace("{x}", "{tileCol}");
        let tiles = json!({"tilesets":[{"dataType":"vector","tileMatrixSetId":"WebMercatorQuad","links":[
            {"rel":"item","type":"application/vnd.mapbox-vector-tile","href":item}]}]});
        let styles = json!({"styles":[{"id":"roads","links":[
            {"rel":"stylesheet","type":"application/vnd.mapbox.style+json","href":"/style?f=mapbox"}]}]});
        let style = json!({"version":8,"sources":{"tiles":{"type":"vector","tiles":[template]}},
            "layers":[{"id":"roads","type":"line","source":"tiles","source-layer":"roads"}]});
        let mut params = json!({"url":format!("{origin}/dataset"),"type":"ogcapi","text":root.to_string(),
            "documents":[{"url":format!("{origin}/tiles?f=json"),"text":tiles.to_string()},
                {"url":format!("{origin}/styles?f=json"),"text":styles.to_string()},
                {"url":format!("{origin}/style?f=mapbox"),"text":style.to_string()}]});
        let mut session = Session::new(Project::default());
        let listing = session.run("service.layers", &params).unwrap();
        assert!(listing["next"].is_null(), "all fixture documents must be consumed: {listing}");
        assert_eq!(listing["count"], usize::from(allowed), "{listing}");
        let notes = listing["notes"].as_array().unwrap();
        if allowed {
            assert!(notes.is_empty(), "{listing}");
        } else {
            assert!(
                notes.iter().any(|n| n.as_str().unwrap().starts_with("1 tilesets were left out: disallowed template addresses")),
                "{listing}"
            );
            params["layer"] = json!("roads");
            assert!(session.run("service.add", &params).is_err());
            assert!(session.map().tiles.is_empty());
        }
    }
}

#[test]
fn ogc_dataset_shared_map_vector_list_is_read_once_for_both_offers() {
    let root = json!({"links":[
        {"rel":"tilesets-vector","href":"/tiles?f=json"},
        {"rel":"tilesets-map","href":"/tiles?f=json"}
    ]});
    let tiles = json!({"tilesets":[
        {"dataType":"vector","tileMatrixSetId":"WebMercatorQuad","vector_layers":[{"id":"roads"}],"links":[
            {"rel":"item","type":"application/vnd.mapbox-vector-tile","href":"/vector/{tileMatrix}/{tileRow}/{tileCol}.pbf"}]},
        {"dataType":"map","tileMatrixSetId":"WebMercatorQuad","links":[
            {"rel":"item","type":"image/png","href":"/map/{tileMatrix}/{tileRow}/{tileCol}.png"}]}
    ]});
    let server = Server::new(vec![Route::new("/dataset", &root.to_string()), Route::new("/tiles", &tiles.to_string())]);
    let mut session = Session::new(Project::default());
    let mut params = json!({"url":format!("{}/dataset", server.base),"type":"ogcapi"});
    let native = session.run("service.layers", &params).unwrap();
    assert_eq!(native["count"], 2, "{native}");
    let layers = native["layers"].as_array().unwrap();
    assert!(layers.iter().any(|l| l["offer"]["type"] == "vectorTiles"), "{native}");
    assert!(layers.iter().any(|l| l["offer"]["type"] == "tiles"), "{native}");
    assert!(native["notes"].as_array().unwrap().is_empty(), "{native}");
    assert_eq!(server.requests().len(), 2, "the shared list must be fetched only once");
    params["text"] = json!(root.to_string());
    params["documents"] = json!([{"url":format!("{}/tiles?f=json",server.base),"text":tiles.to_string()}]);
    assert_eq!(session.run("service.layers", &params).unwrap(), native);
    assert_eq!(server.requests().len(), 2, "supplied replay must not fetch anything");
}

#[test]
fn ogc_dataset_shared_documents_keep_independent_offers() {
    for metadata in [false, true] {
        let mut docs = documents();
        if metadata {
            docs[0].1["links"].as_array_mut().unwrap().truncate(1);
            docs[1].1 = json!({"tilesets":[
                {"tileMatrixSetId":"WebMercatorQuad","vector_layers":[{"id":"roads"}],"links":[{"rel":"self","type":"application/json","href":"/shared?f=json"}]},
                {"tileMatrixSetId":"WebMercatorQuad","vector_layers":[{"id":"roads"}],"links":[{"rel":"self","type":"application/json","href":"/shared?f=json"}]},
                {"tileMatrixSetId":"WebMercatorQuad","title":"Independent","vector_layers":[{"id":"roads"}],"links":[{"rel":"self","type":"application/json","href":"/later?f=json"}]}
            ]});
            let body = json!({"links":[{"rel":"item","type":"application/vnd.mapbox-vector-tile","href":"/wm/{tileMatrix}/{tileRow}/{tileCol}?f=mvt"}]});
            docs.extend([("/shared", body.clone()), ("/later", body)]);
        } else {
            docs[2].1 = json!({"styles":[
                {"id":"first","links":[{"rel":"stylesheet","type":"application/vnd.mapbox.style+json","href":"/good?f=mapbox"}]},
                {"id":"alias","links":[{"rel":"stylesheet","type":"application/vnd.mapbox.style+json","href":"/good?f=mapbox"}]},
                {"id":"later","links":[{"rel":"stylesheet","type":"application/vnd.mapbox.style+json","href":"/later?f=mapbox"}]}
            ]});
            docs.push(("/later", docs[3].1.clone()));
        }
        let server = Server::new(docs.iter().map(|(path, body)| Route::new(path, &body.to_string())).collect());
        let mut session = Session::new(Project::default());
        let mut params = json!({"url":format!("{}/dataset", server.base),"type":"ogcapi"});
        let native = session.run("service.layers", &params).unwrap();
        assert_eq!(native["count"], 2, "{native}");
        assert!(native["layers"].as_array().unwrap().iter().any(|l| l["name"] == "later" || l["title"] == "Independent"), "{native}");
        assert!(
            native["notes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n.as_str().unwrap().starts_with("1 dataset documents were left out: repeated addresses")),
            "{native}"
        );
        let requests = server.requests();
        assert_eq!(requests.iter().filter(|r| r.path.starts_with(if metadata { "/shared" } else { "/good" })).count(), 1);
        params["text"] = json!(docs[0].1.to_string());
        params["documents"] = json!(
            requests
                .iter()
                .skip(1)
                .map(|r| {
                    let body = &docs.iter().find(|(p, _)| *p == r.path).unwrap().1;
                    let mut address = url::Url::parse(&format!("{}{}", server.base, r.path)).unwrap();
                    address.query_pairs_mut().extend_pairs(r.query.iter().map(|(k, v)| (k, v)));
                    json!({"url":address.as_str(),"text":body.to_string()})
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(session.run("service.layers", &params).unwrap(), native);
    }
}

#[test]
fn ogc_dataset_metadata_and_tilejson_are_read_before_listing() {
    let mut docs = documents();
    docs[1].1 = json!({"tilesets":[{"tileMatrixSetId":"WebMercatorQuad","dataType":"vector","title":"Road tiles","vector_layers":[{"id":"roads"}],"links":[
        {"rel":"self","type":"application/json","href":"/metadata?f=json"}]}]});
    docs.push((
        "/metadata",
        json!({"links":[
        {"rel":"item","type":"application/vnd.mapbox-vector-tile","href":"/wm/{tileMatrix}/{tileRow}/{tileCol}?f=mvt"}]}),
    ));
    docs[2].1["styles"].as_array_mut().unwrap().truncate(1);
    docs[3].1["sources"]["basemap"] = json!({"type":"vector","url":"/tilejson"});
    docs.push(("/tilejson", json!({"tiles":["/wm/{z}/{y}/{x}?f=mvt"]})));
    let root = "https://maps.example.org/dataset";
    let mut params = json!({"url":root,"type":"ogcapi","text":docs[0].1.to_string(),"documents":[]});
    for (path, body) in docs.iter().skip(1) {
        let url = format!(
            "https://maps.example.org{path}{}",
            if ["/tiles", "/styles", "/metadata"].contains(path) {
                "?f=json"
            } else if *path == "/good" {
                "?f=mapbox"
            } else {
                ""
            }
        );
        params["documents"].as_array_mut().unwrap().push(json!({"url":url,"type":"ogcapi","text":body.to_string()}));
    }
    let mut session = Session::new(Project::default());
    let listing = session.run("service.layers", &params).unwrap();
    assert_eq!(listing["count"], 2, "{listing}");
    assert_eq!(listing["layers"][0]["title"], "Road tiles");
    assert_eq!(listing["layers"][0]["offer"]["layers"], json!(["roads"]));
    assert!(listing.get("next").is_none());
    assert!(listing["notes"].as_array().unwrap().is_empty(), "{listing}");
    params["layer"] = listing["layers"][0]["name"].clone();
    params["source_url"] = listing["layers"][0]["source_url"].clone();
    session.run("service.add", &params).unwrap();
    assert!(session.map().tiles[0].url.contains("/wm/{z}/{y}/{x}?f=mvt"));
    params["documents"][1]["text"] =
        json!(params["documents"][1]["text"].as_str().unwrap().replace("/good?f=mapbox", "/good?f=mapbox&key=fixture-key"));
    params["documents"][2]["url"] = json!("https://maps.example.org/good?f=mapbox&key=fixture-key");
    let listing = session.run("service.layers", &params).unwrap();
    assert_eq!(listing["count"], 2, "{listing}");
    assert!(listing["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("didn't get its keys")));
}

#[test]
fn ogc_dataset_reports_missing_names_and_rejects_mixed_style_sources() {
    let mut docs = documents();
    docs[3].1["sources"]["basemap"]["tiles"].as_array_mut().unwrap().push(json!("/rd/{z}/{y}/{x}?f=mvt"));
    let server = Server::new(docs.iter().map(|(path, body)| Route::new(path, &body.to_string())).collect());
    let mut session = Session::new(Project::default());
    let listing = session.run("service.layers", &json!({"url":format!("{}/dataset",server.base),"type":"ogcapi"})).unwrap();
    assert_eq!(listing["count"], 0);
    assert!(listing["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("neither named vector-layer metadata")));
}

#[test]
fn ogc_dataset_style_discovery_is_bounded_and_checks_source_addresses() {
    let mut docs = documents();
    docs[2].1["styles"] = json!(
        (0..17)
            .map(|i| json!({"id":format!("style-{i}"),"links":[
                {"rel":"stylesheet","type":"application/vnd.mapbox.style+json","href":format!("/style-{i}")}
            ]}))
            .collect::<Vec<_>>()
    );
    let good = docs[3].1.clone();
    let mut routes: Vec<Route> = docs.iter().map(|(path, body)| Route::new(path, &body.to_string())).collect();
    for i in 0..17 {
        routes.push(Route::new(&format!("/style-{i}"), &good.to_string()));
    }
    let server = Server::new(routes);
    let mut session = Session::new(Project::default());
    let listing = session.run("service.layers", &json!({"url":format!("{}/dataset",server.base),"type":"ogcapi"})).unwrap();
    assert_eq!(listing["count"], 16, "{listing}");
    assert!(listing["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("16-style discovery limit")));
    assert_eq!(server.requests().len(), 19);
    let root = "https://maps.example.org/dataset";
    let mut docs = documents();
    docs[2].1["styles"].as_array_mut().unwrap().truncate(1);
    docs[3].1["sources"]["basemap"] = json!({"type":"vector","url":"http://127.0.0.1/private"});
    let params = json!({"url":root,"type":"ogcapi","text":docs[0].1.to_string(),"documents":[
        {"url":"https://maps.example.org/tiles?f=json","text":docs[1].1.to_string()},
        {"url":"https://maps.example.org/styles?f=json","text":docs[2].1.to_string()},
        {"url":"https://maps.example.org/good?f=mapbox","text":docs[3].1.to_string()}
    ]});
    let mut params = params;
    params["documents"][1]["text"] = json!(
        json!({"styles":[
            {"id":"private", "links":[{"rel":"stylesheet","type":"application/vnd.mapbox.style+json","href":"/good?f=mapbox"}]},
            {"id":"public", "links":[{"rel":"stylesheet","type":"application/vnd.mapbox.style+json","href":"/public"}]}
        ]})
        .to_string()
    );
    params["documents"].as_array_mut().unwrap().push(json!({"url":"https://maps.example.org/public","text":good.to_string()}));
    let listing = session.run("service.layers", &params).unwrap();
    assert_eq!(listing["count"], 1, "{listing}");
    assert_eq!(listing["layers"][0]["name"], "public");
    assert!(listing["next"].is_null(), "no private source is fetched or requested");
    assert!(listing["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("disallowed addresses")));
}

#[test]
fn ogc_dataset_canonical_grid_allows_an_opaque_scheme_definition_link() {
    use rustgis_data::ogc::{TilesStep, resolve_tiles};
    let root = "https://maps.example.org/dataset";
    let text = json!({"links":[{"rel":"tilesets-map","type":"application/json","href":"/tiles"}]}).to_string();
    let scheme = json!({"rel":"tiling-scheme","href":"/definitions/smerc"});
    let set = json!({"tileMatrixSetURI":"http://www.opengis.net/def/tilematrixset/OGC/1.0/WebMercatorQuad", "links":[scheme,
        {"rel":"self","type":"application/json","href":"/metadata"}]});
    let metadata = json!({"links":[scheme,{"rel":"item","type":"image/png","href":"/map/{tileMatrix}/{tileRow}/{tileCol}.png"}]});
    let mut direct = set.clone();
    direct["links"] = metadata["links"].clone();
    let direct = json!({"tilesets":[direct]}).to_string();
    assert_eq!(
        resolve_tiles("https://maps.example.org/tiles", "https://maps.example.org/tiles", direct.as_bytes(), false).unwrap().step,
        TilesStep::Template("https://maps.example.org/map/{z}/{y}/{x}.png".into()),
        "the shared collection resolver accepts the canonical grid too"
    );
    let params = json!({"url":root,"type":"ogcapi","text":text,"documents":[
        {"url":"https://maps.example.org/tiles","text":json!({"tilesets":[set]}).to_string()},
        {"url":"https://maps.example.org/metadata?f=json","text":metadata.to_string()}
    ]});
    let listing = Session::new(Project::default()).run("service.layers", &params).unwrap();
    assert_eq!(listing["count"], 1, "{listing}");
    assert!(listing["notes"].as_array().unwrap().is_empty(), "{listing}");
}

#[test]
fn ogc_dataset_rejects_direct_and_contradictory_tileset_projections() {
    let root = "https://maps.example.org/dataset";
    let landing = json!({"links":[{"rel":"tilesets-map","type":"application/json","href":"/tiles"}]}).to_string();
    let mut session = Session::new(Project::default());
    for linked in [false, true] {
        for grid in ["NetherlandsRDNewQuad", "EuropeanETRS89_LAEAQuad"] {
            let metadata = json!({"tileMatrixSetId":grid,"links":[
                {"rel":"item","type":"image/png","href":"/wrong/{tileMatrix}/{tileRow}/{tileCol}.png"}
            ]})
            .to_string();
            let mut params =
                json!({"url":root,"type":"ogcapi","text":landing,"documents":[{"url":"https://maps.example.org/tiles","text":metadata}]});
            if linked {
                params["documents"][0]["text"] = json!(
                    json!({"tilesets":[{"tileMatrixSetId":"WebMercatorQuad","links":[
                    {"rel":"self","type":"application/json","href":"/metadata"}]}]})
                    .to_string()
                );
                params["documents"].as_array_mut().unwrap().push(json!({"url":"https://maps.example.org/metadata?f=json","text":metadata}));
            }
            let listing = session.run("service.layers", &params).unwrap();
            assert_eq!(listing["count"], 0, "{listing}");
            assert!(listing["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("tile matrix set")));
        }
    }
}

#[test]
fn ogc_dataset_tilesets_have_distinct_addable_names() {
    let root = "https://maps.example.org/dataset";
    let landing = json!({"links":[{"rel":"tilesets-map","type":"application/json","href":"/tiles"}]}).to_string();
    let sets = json!({"tilesets": (1..=2).map(|id| json!({"tileMatrixSetId":"WebMercatorQuad", "links":[
        {"rel":"item","type":"image/png","href":format!("/map-{id}/{{tileMatrix}}/{{tileRow}}/{{tileCol}}.png")}
    ]})).collect::<Vec<_>>()})
    .to_string();
    let mut params = json!({"url":root,"type":"ogcapi","text":landing,"documents":[{"url":"https://maps.example.org/tiles","text":sets}]});
    let mut session = Session::new(Project::default());
    let listing = session.run("service.layers", &params).unwrap();
    assert_eq!(listing["count"], 2, "{listing}");
    assert_eq!(listing["layers"][0]["name"], "map");
    assert_ne!(listing["layers"][0]["name"], listing["layers"][1]["name"]);
    for layer in listing["layers"].as_array().unwrap() {
        params["layer"] = layer["name"].clone();
        params["source_url"] = layer["source_url"].clone();
        let added = session.run("service.add", &params).unwrap();
        let index = added["index"].as_u64().unwrap() as usize;
        assert_eq!(session.map().tiles[index].url, layer["offer"]["url"].as_str().unwrap());
    }
    assert_ne!(session.map().tiles[0].url, session.map().tiles[1].url);
}

#[test]
fn ogc_dataset_linked_tilesets_keep_the_advertised_order() {
    let root = "https://maps.example.org/dataset";
    let text = json!({"links":[{"rel":"tilesets-map","href":"/tiles"}]}).to_string();
    let sets = json!({"tilesets": (1..=2).map(|id| json!({"tileMatrixSetId":"WebMercatorQuad", "links":[
        {"rel":"self","type":"application/json","href":format!("/map-{id}")}
    ]})).collect::<Vec<_>>()})
    .to_string();
    let mut documents = vec![json!({"url":"https://maps.example.org/tiles?f=json","text":sets})];
    for id in 1..=2 {
        let metadata =
            json!({"links":[{"rel":"item","type":"image/png","href":format!("/map-{id}/{{tileMatrix}}/{{tileRow}}/{{tileCol}}.png")}]})
                .to_string();
        documents.push(json!({"url":format!("https://maps.example.org/map-{id}?f=json"),"text":metadata}));
    }
    let mut session = Session::new(Project::default());
    let listing = session.run("service.layers", &json!({"url":root,"type":"ogcapi","text":text,"documents":documents})).unwrap();
    assert_eq!(listing["count"], 2, "{listing}");
    for (index, id) in [1, 2].into_iter().enumerate() {
        assert_eq!(listing["layers"][index]["offer"]["url"], format!("https://maps.example.org/map-{id}/{{z}}/{{y}}/{{x}}.png"));
    }
}

#[test]
fn ogc_dataset_service_discovery_accepts_an_explicit_map_template() {
    let root = "https://maps.example.org/dataset";
    let landing = json!({"title":"Map", "links":[{"rel":"tilesets-map","type":"application/json","href":"/tiles"}]}).to_string();
    let template = "https://maps.example.org/resolved/{z}/{y}/{x}.png";
    let mut params = json!({"url":root,"type":"ogcapi","text":landing,"layer":"map","tiles":template,
        "source_url":"https://maps.example.org/dataset?f=json"});
    let mut session = Session::new(Project::default());
    let added = session.run("service.add", &params).unwrap();
    assert_eq!(session.map().tiles[added["index"].as_u64().unwrap() as usize].url, template);
    params["tiles"] = json!("not-a-template");
    assert!(session.run("service.add", &params).unwrap_err().contains("not a tile template"));

    // A current listing's metadata source must still select the normal validated map offer.
    params["tiles"] = json!(template);
    params["source_url"] = json!("https://maps.example.org/tiles");
    params["documents"] = json!([{"url":"https://maps.example.org/tiles","text":json!({"tilesets":[{
        "tileMatrixSetId":"WebMercatorQuad","dataType":"map","links":[
            {"rel":"item","type":"image/png","href":"/resolved/{tileMatrix}/{tileRow}/{tileCol}.png"}]
    }]}).to_string()}]);
    let added = session.run("service.add", &params).unwrap();
    assert_eq!(session.map().tiles[added["index"].as_u64().unwrap() as usize].url, template);

    // Rejecting an invalid explicit template must not hide keys withheld during discovery.
    params["url"] = json!(format!("{root}?key=fixture-key"));
    params.as_object_mut().unwrap().remove("source_url");
    params["tiles"] = json!("not-a-template");
    session.notices.clear();
    assert!(session.run("service.add", &params).unwrap_err().contains("not a tile template"));
    assert_eq!(session.notices.iter().filter(|n| !n.error && n.text.contains("address(es)")).count(), 1);
}

#[test]
fn ogc_dataset_metadata_bytes_are_bounded_without_discarding_valid_tiles() {
    let mut docs = documents();
    docs[1].1["tilesets"] = json!([{
        "tileMatrixSetId":"WebMercatorQuad","dataType":"vector","vector_layers":[{"id":"roads"}],
        "links":[{"rel":"item","type":"application/vnd.mapbox-vector-tile","href":"/wm/{tileMatrix}/{tileRow}/{tileCol}?f=mvt"}]
    }]);
    docs[2].1["styles"].as_array_mut().unwrap().truncate(1);
    let mut params = json!({"url":"https://maps.example.org/dataset","type":"ogcapi","text":docs[0].1.to_string(),"documents":[
        {"url":"https://maps.example.org/tiles?f=json","text":docs[1].1.to_string()},
        {"url":"https://maps.example.org/styles?f=json","text":docs[2].1.to_string()},
        {"url":"https://maps.example.org/good?f=mapbox","text":docs[3].1.to_string()}
    ]});
    let mut session = Session::new(Project::default());
    let clean = session.run("service.layers", &params).unwrap();
    assert_eq!(clean["count"], 2, "{clean}");
    assert!(clean["notes"].as_array().unwrap().is_empty());
    params["documents"][2]["text"] = json!(format!("{}{}", docs[3].1, " ".repeat(crate::cmd::service::MAX_DATASET_BYTES)));
    let partial = session.run("service.layers", &params).unwrap();
    assert_eq!(partial["count"], 1, "{partial}");
    assert!(partial.get("next").is_none());
    assert!(partial["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("metadata budget")));
}

#[test]
fn ogc_dataset_failed_discovery_keeps_prior_omission_counts() {
    for omitted in [false, true] {
        let mut sets = vec![json!({"tileMatrixSetId":"WebMercatorQuad","links":[{"rel":"self","href":"/metadata?f=json"}]})];
        if omitted {
            sets.insert(0, json!({"tileMatrixSetId":"Unsupported"}));
        }
        let params = json!({"url":"https://maps.example.org/dataset","type":"ogcapi","layer":"map",
        "text":json!({"links":[{"rel":"tilesets-map","href":"/tiles?f=json"}]}).to_string(),
        "documents":[
            {"url":"https://maps.example.org/tiles?f=json","text":json!({"tilesets":sets}).to_string()},
            {"url":"https://maps.example.org/metadata?f=json","text":" ".repeat(crate::cmd::service::MAX_DATASET_BYTES)}
        ]});
        let mut session = Session::new(Project::default());
        for command in ["service.layers", "service.add"] {
            let error = session.run(command, &params).unwrap_err();
            assert!(error.contains("metadata budget"), "{error}");
            assert_eq!(error.contains("1 tilesets were left out"), omitted, "{error}");
            #[cfg(not(target_arch = "wasm32"))]
            {
                use crate::cmd::reads::{read_ahead, read_land};
                let read = read_ahead(&mut session, command, &params).unwrap().run(&crate::analysis::control::Control::new()).unwrap();
                assert_eq!(read_land(&mut session, read).unwrap_err(), error, "worker errors must retain the same counts after rollback");
            }
        }
    }
}

#[test]
fn ogc_dataset_map_tiles_reject_declared_coverage_and_unknown_types() {
    for listed in [false, true] {
        for kind in [None, Some("map"), Some("vector"), Some("coverage"), Some("unknown")] {
            let mut tileset = json!({"tileMatrixSetId":"WebMercatorQuad","links":[
                {"rel":"item","type":"image/png","href":"/tiles/{tileMatrix}/{tileRow}/{tileCol}.png"}
            ]});
            if let Some(kind) = kind {
                tileset["dataType"] = json!(kind);
            }
            let document = if listed { json!({"tilesets":[tileset]}) } else { tileset };
            let listing = Session::new(Project::default())
                .run(
                    "service.layers",
                    &json!({
                        "url":"https://maps.example.org/dataset","type":"ogcapi",
                        "text":json!({"links":[{"rel":"tilesets-map","href":"/tiles?f=json"}]}).to_string(),
                        "documents":[{"url":"https://maps.example.org/tiles?f=json","text":document.to_string()}]
                    }),
                )
                .unwrap();
            let supported = kind.is_none_or(|k| k == "map");
            assert_eq!(listing["count"], usize::from(supported), "{kind:?}: {listing}");
            let notes = listing["notes"].as_array().unwrap();
            if supported {
                assert!(notes.is_empty(), "{listing}");
            } else {
                assert_eq!(notes.len(), 1, "{listing}");
                assert!(notes[0].as_str().unwrap().starts_with("1 tilesets were left out"), "{listing}");
            }
        }
    }
}
