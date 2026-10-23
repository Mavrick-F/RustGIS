# Roadmap

_Last reviewed: 2026-10-23 (v0.3.0). Reviewed weekly; if this date is old, trust the linked issues
over this page._

This page says where RustGIS is heading and where help is most useful. Details live in the linked
issues; feature-by-feature comparison with GeoLibre is tracked in
[GEOLIBRE_PARITY.md](GEOLIBRE_PARITY.md).

## Goals
- **What exists works.** A feature that crashes, misreads data or is undocumented costs more
  than a missing one. Robustness and correct coordinates come before breadth.
- **Finish the everyday workflow first.** Add data, style it, edit it, analyse it, print or share
  it: depth in these beats a long list of first versions.
- **Foundations before features.** Build the shared piece once (an expression engine, a secret
  store, Z coordinates) and let several features stand on it.
- **Free and self-contained.** Pure Rust with nothing else to install; GDAL and DuckDB are
  optional extras that add formats, not requirements.
- **One program everywhere.** Desktop, browser, CLI, Python and AI agents share the same commands;
  nothing below `apps/` breaks the WebAssembly build.

## Contributing
Most items below have no issue yet: propose one and open a pull request. See
[CONTRIBUTING.md](CONTRIBUTING.md) for which items to pick freely, which to coordinate first, and
how to write a proposal. Themes under Next are not releases and not in order; where an item
needs another first, it says so.

## Underway
An item is underway when it has an open pull request, or someone said on its issue in the last
week that they are working on it. Check here and the
[open pull requests](https://github.com/opengeos/RustGIS/pulls) before starting; to take an item,
comment on its issue and open a draft pull request early.
- GPU map: vector tiles and labels on the GPU tile layer
  ([#635](https://github.com/opengeos/RustGIS/issues/635),
  [#596](https://github.com/opengeos/RustGIS/issues/596)).
- Labeling pane: label classes managed in the pane, query editor, more options
  ([#618](https://github.com/opengeos/RustGIS/issues/618),
  [#722](https://github.com/opengeos/RustGIS/issues/722)).
- Georeferencer: side-by-side window, transform picker, more formats
  ([#609](https://github.com/opengeos/RustGIS/issues/609)).
- Script pane: completion of values such as layer and field names
  ([#662](https://github.com/opengeos/RustGIS/issues/662)).
- Connection manager for databases and web services
  ([#699](https://github.com/opengeos/RustGIS/issues/699)).
- Zonal statistics as a built-in tool ([#718](https://github.com/opengeos/RustGIS/pull/718)).
- A "No fill" option for polygons ([#745](https://github.com/opengeos/RustGIS/pull/745), for
  [#644](https://github.com/opengeos/RustGIS/issues/644)).
- Editing PostGIS tables and saving the edits back
  ([#671](https://github.com/opengeos/RustGIS/pull/671), draft).

## Next

### Fixes and quick wins
The bugs users hit, the docs and a few quick wins: good first pull requests.
- Robustness: the Windows 11 start-up failure
  ([#375](https://github.com/opengeos/RustGIS/issues/375)), Control-click menus on macOS
  ([#707](https://github.com/opengeos/RustGIS/issues/707)), Load Off-Map Data
  ([#661](https://github.com/opengeos/RustGIS/issues/661)).
- A pass over the user guide, the comparison pages and the README against what the app does
  today (vertex editing, spatial join and kriging are listed as missing or undocumented; some
  pages still say 16 languages).
- Styling: standard deviation, geometric interval and manual breaks _(open)_.
- Selection by polygon and lasso, and a radius tool on the map (Neighborhood Explorer already
  selects by radius) _(open)_.

### Solid ground
Correct, robust and recoverable, with the foundations other themes stand on.
- Correct coordinates: grid datum shifts (NTv2 and GeoTIFF grids) for NAD27, NAD83(HARN),
  GDA94↔GDA2020 and OSGB36, and a notice whenever a layer is read without its shift; GDA94
  ([#730](https://github.com/opengeos/RustGIS/issues/730)) is one case. _Coordinate first._
- Reloading files changed on disk.
- Autosave and crash recovery: reopen unsaved work after a crash.
- Signed Windows builds (no SmartScreen warning), then in-place updates.
- A secret store in the OS keychain for database, S3 and service connections (they are kept for
  the session only today).
- Overlay tools beside Clip and Erase: intersect, union, identity and near _(open)_.
- Vector tools, each writing a new layer: join field (a one-off attribute join), merge (append
  layers), extract vertices, points along lines, check and fix geometry _(open)_.
- Batch runs: one tool over many layers or a folder of files.

### Expressions and cartography
One expression engine, a superset of today's SQL where clause so existing definition queries,
label-class queries and display filters keep working. It replaces the three parsers of today
(where clauses, Calculate Field, attribute rules) and serves Calculate Field, attribute rules,
labels, definition queries, display filters, pop-ups and symbol properties. On top of it:
- Calculate Field with functions, not only `field op value`.
- Expression labels, with offset, rotation, wrapping, case, priority, number formats and halo
  colour/width.
- Data-defined symbol size, colour and rotation.
- Rule-based renderer: ordered rules with an expression, symbol and scale range.
- Quick filters in the attribute table; pop-up templates (chosen fields, formats for dates, links
  and images, a title).

And alongside it:
- Kernel density (heat map) as a tool and as a renderer; point clustering.
- SVG and PNG markers, fill patterns and a symbol library.
- A legend and colour bar on the map.
- Style import: QGIS QML and SLD styles applied to layers.
- A histogram for raster stretches; raster attribute tables.

### Data at the source
- Write edits back to GeoPackage and PostGIS (edits stay in the project today; PostGIS is
  underway in [#671](https://github.com/opengeos/RustGIS/pull/671)). _Coordinate first._
- Editor tracking fields (created/edited by and at).
- Attribute table columns: rename, hide and reorder _(open)_.
- Persistent attribute joins and virtual fields. _Coordinate first_ (they change `Feature`).
- Attribute forms: widgets, constraints and conditional visibility, on top of domains and
  subtypes.
- Layer metadata, exported as STAC items.
- ArcGIS sign-in (OAuth) and a Portal browser, once the secret store (Solid ground) exists.

### Analysis
- Routing: isochrones, service areas and an origin–destination matrix (today shortest path over
  a line layer).
- Hot-spot statistics (Getis-Ord Gi\*), and emerging hot spots on the time slider.
- Charts linked to the selection: histogram, scatter, bar and box plots of a layer's fields.
- Reverse geocoding; a DEM downloader so viewshed works without a loaded elevation raster
  _(open)_.

### 3D scenes, first steps
- Phases 1–2 of [#726](https://github.com/opengeos/RustGIS/issues/726): Z coordinates kept from
  the data and written back; a scene with a free camera and each layer's elevation mode (on the
  ground, relative, absolute).
- LiDAR point layers (LAS/LAZ/COPC, read today as points) drawn at their heights in the scene.
- 3D objects and 3D Tiles follow.

### Self-contained, any time _(open)_
Built in means no runner download, the selection honoured, geodesic distances and attributes
kept. Before adding a tool, check whether the GeoLibre suite already has it (`gp.tools`); a
built-in version must justify itself on one of those.
- SVG layout export; STL export; copy a layout to the clipboard.
- UTM and MGRS grids and coordinate readouts.
- H3 grids built in, next to the A5 grids (the GeoLibre suite has H3 tools only when its runner
  is installed).
- An on-map right-click menu: copy the coordinate, run a buffer or viewshed from the point.
- More than two synced maps side by side.
- Auto-refresh for web service layers.
- Vector to PMTiles export: the PMTiles writer and MVT encoder in `rustgis-tiles` exist but are
  only used by tests ([#557](https://github.com/opengeos/RustGIS/issues/557)).
- A catalog of data cubes for the time slider
  ([#683](https://github.com/opengeos/RustGIS/issues/683)).
- A Dockerfile for self-hosting the web app.
- Rebindable keyboard shortcuts ([#562](https://github.com/opengeos/RustGIS/issues/562)).

## Exploratory
Open questions rather than plans; discuss before building.
- Layers kept in their own coordinate system rather than WGS 84.
- Integrating [opengeos/terrio](https://github.com/opengeos/terrio), the pure-Rust raster, vector and point cloud I/O library: which of its drivers, coordinate transformations and GDAL-style operations could back or replace RustGIS's own readers, or stand in for the GDAL program, and which of them build for the web app.
- Collaboration: shared projects, review comments, accounts and a server.
- Mobile and field work: touch layout, GPS, data collection, QFieldCloud
  ([#665](https://github.com/opengeos/RustGIS/issues/665)).
- Native NetCDF-4/HDF5 without GDAL ([#396](https://github.com/opengeos/RustGIS/issues/396)),
  kerchunk and Icechunk.
- An SQL engine in the web app.
- Planetary coordinate systems ([#223](https://github.com/opengeos/RustGIS/issues/223)).
- Opening QGIS and ArcGIS Pro projects.
- An in-app Python console; an R package.
- AI segmentation and detection (SAM, ONNX models).

## Done
- Layered workspace, command registry with undo, project files (`.rgis`), `rustgis-cli`, the
  web app (WebAssembly, installable with offline start, opened with a project or data by its
  web address), 17 interface languages; a ribbon laid out in the order of the work
  (Home, Data, Style, Edit, Analyze, View, Publish), start-up preferences, Check for Updates.
- Data: files by their web address; GeoJSON, CSV/TSV (x/y in any EPSG system, or WKT), Excel and OpenDocument spreadsheets,
  GeoRSS, LandXML, encoded polylines, Shapefile (zipped too, with the coordinate system in its
  `.prj`), GeoPackage, GeoParquet, FlatGeobuf, KML/KMZ (styles included), GPX, GML, MapInfo MIF, OSM PBF, TopoJSON, DXF and LiDAR (LAS/LAZ/COPC) in, all of them
  in the web app too (large files read in place, a view at a time); Esri File Geodatabases without GDAL (desktop; 3D multipatches
  as footprints with their heights); the vector formats GDAL reads when it is installed, PostGIS, MySQL, MariaDB and SQL Server tables read live (desktop), and SQL queries
  and DuckDB databases (DuckDB Spatial) when DuckDB is; GeoJSON, CSV, GeoPackage, GeoParquet, Shapefile, FlatGeobuf, KML/KMZ, GPX and WKT out;
  standalone tables; geotagged photos and KML ground overlays; geocoding (Find Place, address
  tables); a Data Sources pane (favourites, recent, saved services, a library) and an SQL pane
  (DuckDB Spatial); layer, map, style, template, task and package files.
- Open data: STAC search (Planetary Computer assets), NASA Earthdata (desktop), Hugging Face, Overture Maps, Source Cooperative,
  OpenStreetMap by area (Overpass), geoBoundaries, CKAN, Socrata and ArcGIS Hub portals, and
  S3 buckets with signed connections and a bucket browser.
- Rasters: GeoTIFF/COG (by web address, read in place by range; Zstandard too) and ASCII grids,
  Zarr 2 and 3 stores (a variable's step, by folder or web address), and JPEG 2000, NetCDF, HDF, ERDAS Imagine… through GDAL,
  EPSG reprojection, stretch/RGB/classified/shaded-relief
  symbology, raster functions, indices, change detection, classification, georeferencing;
  cloud-optimised GeoTIFFs out; data cubes on a time slider, with trends (Sen's slope, Mann–Kendall).
- Services: WMS, WMTS, WFS, WCS, OGC API Features and Tiles, and ArcGIS REST, listed from their
  capabilities; CSW catalogue search.
- Maps: multiple maps, a GPU renderer (beta) with smooth zoom and pan, tilt, rotation and terrain, a 3D globe with the flat
  map draped over it, projection presets and EPSG codes (features cut along the edge of world
  projections), a search box for layers, places and coordinates, bookmarks, web tile
  layers (XYZ/TMS/WMS, Mapbox Vector Tiles, MapLibre styles, PMTiles and MBTiles archives),
  a basemap gallery (OpenFreeMap, USGS Imagery), worldviews.
- Cartography: single, unique values, graduated colours and proportional symbols; layer groups;
  label classes with SQL queries, placement and annotation; custom colour ramps (and QGIS, GDAL
  and GMT palette files); masking, display filters, blend modes, aggregation, extrusion; graphics.
- Editing: create, move, rotate, scale, merge, split, explode, vertex editing; snapping, templates, elevation,
  topology; domains, subtypes, attribute rules, contingent values, relationships, archiving.
- Analysis: selection by attributes and location; overlay (clip, erase), proximity (buffer) and summary tools, spatial
  join, A5 grids, kriging; guided workflows (viewshed, line of sight, IDW, shortest path,
  suitability); ModelBuilder and scripts, and history copied as code; the GeoLibre and Whitebox
  suite (1,000+ tools) natively and in the browser.
- Layouts (map frames, surrounds, map series, publication sheets and figures in one command, PDF),
  reports, presentations and notebooks; the web app embeddable in a page (`?embed`).
- Sharing as open formats (web maps, OGC API Features collections, MapLibre styles, glTF
  scenes, locators), offline areas with sync, deep learning pixel classification, hosted
  imagery tiles.
- The AI Assistant (Anthropic, OpenAI, ChatGPT sign-in, Gemini, OpenRouter, Ollama, OpenAI-compatible) in the
  app, the web app and `rustgis-cli ask`; an MCP server (`rustgis-cli mcp`) for AI agents such
  as Claude Code, Cursor and VS Code, with the documentation built in.
- The AI Assistant's agent skill and loopback relay, which let external AI clients drive the app.
- Sandboxed WebAssembly plugins with a registry at plugins.rustgis.app; process plugins that run
  as programs (with Python environments RustGIS makes), in the background with progress and Cancel.
- The `rustgis` Python package on PyPI: the engine in-process, and `rustgis.Map` in Jupyter.
- Desktop packages: Windows installers (setup.exe, .msi), a portable zip and a Store MSIX; a
  signed and notarized macOS DMG; Linux .deb, .rpm, AppImage and tarballs; Homebrew, the AUR,
  Fedora COPR and winget; a download mirror at downloads.rustgis.app.
