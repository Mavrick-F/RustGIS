//! Geodesic buffers: a distance measured on the Earth, wherever the feature is and however far it
//! spans (#394).
//!
//! A feature is buffered in an azimuthal equidistant projection centred on it, where distances
//! from the centre are true and the error elsewhere grows with the square of the distance from it.
//! So a small feature is buffered in one piece, and a long line or a large polygon's outline in
//! stretches of a few hundred kilometres, each in its own projection, and the pieces are joined.
//!
//! The outline keeps its distance between its vertices too, not only at them (#1159): an edge
//! drawn straight in the plane is split before it comes back to lon/lat wherever a straight edge
//! there would stray from it ([`OUTLINE_TOLERANCE`]), so up to 85° of latitude the sides are
//! within 0.012% of the distance (or 1 cm) wherever the feature is within 150 km of its
//! projection's centre: the projection's own error, under 0.0093% there, and the edges' 0.002%.
//! A winding line or outline buffered in stretches can have vertices up to [`STRETCH_M`] from a
//! stretch's centre, where the sides are within 0.05%. A ring gains at most [`MAX_ADDED`] points,
//! which only a ring of thousands of long edges uses up; past that, its edges are left whole.
//!
//! Arcs are drawn as `geo` draws them by default, close to GEOS and PostGIS's 8 edges to a quarter
//! circle: a point's circle with 32 edges, a line's caps and a polygon's corners with 15 to a half
//! circle. Between an arc's vertices the outline runs inside it, by up to 0.48% of the distance
//! round a point and 0.55% at a cap or corner.

use geo::{
    BooleanOps, Buffer, Contains, Coord as GC, LineString, MultiLineString, MultiPolygon, Orient, Point, Polygon, orient::Direction,
};
use rustgis_data::{Coord, Geometry};
use rustgis_geom::{Aeqd, geodesic_interpolate, haversine_m, lonlat_to_merc, merc_to_lonlat, spherical_centre};

/// A feature within this distance of its centre is buffered in one projection (the error is
/// under 0.01%).
const ONE_PIECE_M: f64 = 150_000.0;
/// A stretch stays within this distance of its first vertex (the error is under 0.05%).
const STRETCH_M: f64 = 300_000.0;
/// Edges are split to this length before buffering in stretches, so the pieces and the polygon
/// they join meet along the same path.
const DENSIFY_M: f64 = 10_000.0;
/// At most this many vertices are added to one line or ring (four times round the Earth at 10 km).
const MAX_ADDED: f64 = 16_000.0;
/// Back in lon/lat, an outline's straight edges stay within this fraction of the distance of the
/// edges drawn in the plane, or [`MIN_TOLERANCE_M`], whichever is more, on top of the projection's
/// own error (under 0.0093% at [`ONE_PIECE_M`]).
const OUTLINE_TOLERANCE: f64 = 2e-5;
/// The smallest tolerance (metres): finer only adds vertices.
const MIN_TOLERANCE_M: f64 = 0.01;
/// An edge is halved at most this many times on its way back to lon/lat (4096 pieces): enough for
/// the longest edges a buffer has (about 300 km) to come within 1 cm where they stray most.
const MAX_SPLITS: u32 = 12;
/// How far, per square metre of an edge's length, a straight edge in lon/lat can stray from the
/// same edge straight in the plane: measured at 2e-8 in mid-latitudes (the long side of a 2 km
/// buffer of a 96 km line strayed 190 m, 2e-8 × 96,000²) and 1.9e-7 at worst, along a 298 km
/// line at 84°N, with a margin. An edge too short to stray as far as the tolerance isn't
/// checked, so an arc's short edges cost nothing.
const STRAY_PER_M2: f64 = 3e-7;

/// A feature's buffer at `metres` (negative shrinks a polygon), in lon/lat; `None` when nothing
/// is left.
pub fn geodesic_buffer(g: &Geometry, metres: f64) -> Option<Geometry> {
    if !metres.is_finite() {
        return None;
    }
    let mp = match g {
        Geometry::Point(c) => point(*c, metres),
        Geometry::MultiPoint(v) => union(v.iter().map(|c| point(*c, metres)).collect()),
        Geometry::Line(parts) => lines(&finite(parts), metres),
        Geometry::Polygon(polys) => polygons(&polys.iter().map(|rings| finite(rings)).collect::<Vec<_>>(), metres),
    };
    to_geometry(&mp)
}

/// An azimuthal equidistant plane whose centre's longitude is within 180° of `reference`, a
/// longitude of the coordinates it buffers as written, so the buffer comes back in lon/lat next to
/// them and to the polygon it may be joined to, even at the antimeridian.
struct Plane(Aeqd);

impl Plane {
    fn new(centre: Coord, reference: f64) -> Self {
        let lon = reference + wrap(centre.x - reference);
        Plane(Aeqd::new(Coord::new(if lon.is_finite() { lon } else { centre.x }, centre.y)))
    }

    fn line(&self, pts: &[Coord]) -> LineString<f64> {
        LineString::new(pts.iter().map(|c| self.0.forward(*c)).map(|p| GC { x: p.x, y: p.y }).collect())
    }

    /// A buffer at `metres` drawn in the plane, in lon/lat degrees.
    fn back(&self, mp: &MultiPolygon<f64>, metres: f64) -> MultiPolygon<f64> {
        let tolerance = (metres.abs() * OUTLINE_TOLERANCE).max(MIN_TOLERANCE_M);
        let ring = |r: &LineString<f64>| self.ring(r, tolerance);
        MultiPolygon::new(mp.0.iter().map(|p| Polygon::new(ring(p.exterior()), p.interiors().iter().map(ring).collect())).collect())
    }

    /// A ring drawn in the plane, in lon/lat degrees, its edges split as [`Plane::split`] needs, by
    /// at most [`MAX_ADDED`] points in all, however many edges stray.
    fn ring(&self, r: &LineString<f64>, tolerance: f64) -> LineString<f64> {
        let mut dense = Vec::with_capacity(r.0.len());
        let mut budget = MAX_ADDED as usize;
        for w in r.0.windows(2) {
            let [a, b] = w else { continue };
            dense.push(*a);
            self.split(*a, *b, tolerance, 0, &mut budget, &mut dense);
        }
        dense.extend(r.0.last());
        let dense = &LineString::new(dense);
        let lonlat: Vec<Coord> = dense.0.iter().map(|p| self.0.inverse(Coord::new(p.x, p.y))).collect();
        let around = |lat: f64| {
            let pole = self.0.forward(Coord::new(0.0, lat));
            Polygon::new(dense.clone(), vec![]).contains(&Point::new(pole.x, pole.y))
        };
        if around(90.0) {
            return LineString::new(self.capped(dense, &lonlat, 90.0));
        }
        if around(-90.0) {
            return LineString::new(self.capped(dense, &lonlat, -90.0));
        }
        let mut out: Vec<GC<f64>> = Vec::with_capacity(lonlat.len());
        for c in lonlat {
            // Each longitude within 180° of the one before, so the ring doesn't jump at ±180°.
            let x = out.last().map_or(c.x, |prev| prev.x + wrap(c.x - prev.x));
            out.push(GC { x: if x.is_finite() { x } else { c.x }, y: c.y });
        }
        LineString::new(out)
    }

    /// The points to add between `a` and `b`, an edge straight in the plane, so that back in lon/lat
    /// straight edges between them stay within `tolerance` metres of it. The edge's quarter points
    /// and midpoint are compared with the same points of the straight edge in Web Mercator, the
    /// plane the tools draw edges in (one point alone misses a stray that changes side, as along a
    /// line crossing the equator); an edge that strays is halved, and each half checked again, up
    /// to [`MAX_SPLITS`] times, while the ring's `budget` of added points lasts.
    fn split(&self, a: GC<f64>, b: GC<f64>, tolerance: f64, depth: u32, budget: &mut usize, out: &mut Vec<GC<f64>>) {
        if *budget == 0 || depth >= MAX_SPLITS || STRAY_PER_M2 * ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)) <= tolerance {
            return;
        }
        let at = |t: f64| self.0.inverse(Coord::new(a.x + t * (b.x - a.x), a.y + t * (b.y - a.y)));
        let (la, lb) = (at(0.0), at(1.0));
        let beyond = |c: Coord| c.y.abs() >= rustgis_geom::MAX_LAT;
        let strays = match (beyond(la), beyond(lb)) {
            // Beyond Web Mercator's 85.05° there's no straight edge to compare with.
            (true, true) => false,
            // Across that edge: halved until each piece is on one side of it or too short to stray.
            (true, false) | (false, true) => true,
            (false, false) => {
                // `b` on `a`'s side of the antimeridian, so the straight edge is the short one.
                let near_b = Coord::new(la.x + wrap(lb.x - la.x), lb.y);
                let (ma, mb) = (lonlat_to_merc(la), lonlat_to_merc(near_b));
                [0.25, 0.5, 0.75].into_iter().any(|t| {
                    let p = at(t);
                    let chord = merc_to_lonlat(Coord::new(ma.x + t * (mb.x - ma.x), ma.y + t * (mb.y - ma.y)));
                    haversine_m(p, Coord::new(p.x + wrap(chord.x - p.x), chord.y)) > tolerance
                })
            }
        };
        if !strays {
            return;
        }
        let mid = GC { x: (a.x + b.x) / 2.0, y: (a.y + b.y) / 2.0 };
        self.split(a, mid, tolerance, depth + 1, budget, out);
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        out.push(mid);
        self.split(mid, b, tolerance, depth + 1, budget, out);
    }

    /// A ring round a pole, which goes once round the globe in longitude, as a lon/lat ring: cut at
    /// the antimeridian and closed along the pole, so in lon/lat it holds the cap between the ring and
    /// the top (or bottom) of the map.
    fn capped(&self, planar: &LineString<f64>, ring: &[Coord], pole: f64) -> Vec<GC<f64>> {
        let mut v: Vec<Coord> = ring.iter().map(|c| Coord::new(wrap(c.x), c.y)).collect();
        if v.len() > 1 && v.first() == v.last() {
            v.pop();
        }
        let n = v.len();
        // The edge that crosses the antimeridian: its ends are more than 180° apart.
        let seam = (0..n).find(|k| match (v.get((k + n - 1) % n), v.get(*k)) {
            (Some(a), Some(b)) => (b.x - a.x).abs() > 180.0,
            _ => false,
        });
        let found = seam.and_then(|k| Some((k, v.get((k + n - 1) % n)?, planar.0.get((k + n - 1) % n)?, planar.0.get(k)?)));
        let Some((k, a, pa, pb)) = found else {
            return ring.iter().map(|c| GC { x: c.x, y: c.y }).collect();
        };
        // Where the edge from `a` meets ±180° (`edge`, on `a`'s side), found along the edge in the
        // plane, where it's straight.
        let edge = if a.x > 0.0 { 180.0 } else { -180.0 };
        let at = |t: f64| self.0.inverse(Coord::new(pa.x + t * (pb.x - pa.x), pa.y + t * (pb.y - pa.y)));
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..50 {
            let mid = (lo + hi) / 2.0;
            // Before the crossing a longitude is short of `edge`, after it past (wrapped round).
            if wrap(at(mid).x - edge) * edge < 0.0 { lo = mid } else { hi = mid }
        }
        let y = at((lo + hi) / 2.0).y;
        let mut out = vec![GC { x: -edge, y }];
        out.extend((0..n).filter_map(|i| v.get((k + i) % n)).map(|c| GC { x: c.x, y: c.y }));
        out.extend([GC { x: edge, y }, GC { x: edge, y: pole }, GC { x: -edge, y: pole }, GC { x: -edge, y }]);
        out
    }
}

/// A longitude difference folded into −180..180.
fn wrap(d: f64) -> f64 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

/// A point's buffer: a circle drawn in a projection centred on the point.
fn point(c: Coord, metres: f64) -> MultiPolygon<f64> {
    if !c.is_finite() {
        return MultiPolygon::new(vec![]);
    }
    let plane = Plane::new(c, c.x);
    plane.back(&Point::new(0.0, 0.0).buffer(metres), metres)
}

/// A line's buffer: in one projection centred on it when it fits, else in stretches.
fn lines(parts: &[Vec<Coord>], metres: f64) -> MultiPolygon<f64> {
    let all: Vec<Coord> = parts.iter().flatten().copied().collect();
    let Some(centre) = spherical_centre(&all) else { return MultiPolygon::new(vec![]) };
    if fits(&all, centre) {
        let plane = Plane::new(centre, all.first().map_or(centre.x, |c| c.x));
        return plane.back(&MultiLineString::new(parts.iter().map(|p| plane.line(p)).collect()).buffer(metres), metres);
    }
    union(stretched(parts, metres))
}

/// A polygon's buffer: in one projection centred on it when it fits, else the polygon with its
/// outline's buffer (in stretches) added or, inward, taken away.
fn polygons(polys: &[Vec<Vec<Coord>>], metres: f64) -> MultiPolygon<f64> {
    let all: Vec<Coord> = polys.iter().flatten().flatten().copied().collect();
    let Some(centre) = spherical_centre(&all) else { return MultiPolygon::new(vec![]) };
    let shape = |ring: &dyn Fn(&[Coord]) -> LineString<f64>| {
        MultiPolygon::new(
            polys
                .iter()
                .filter_map(|rings| {
                    let mut it = rings.iter();
                    let outer = ring(it.next()?);
                    Some(Polygon::new(outer, it.map(|r| ring(r)).collect()).orient(Direction::Default))
                })
                .collect(),
        )
    };
    if fits(&all, centre) {
        let plane = Plane::new(centre, all.first().map_or(centre.x, |c| c.x));
        return plane.back(&shape(&|r| plane.line(r)).buffer(metres), metres);
    }
    // A buffer distributes over a union: the polygon's buffer is the polygon with its outline's
    // buffer added (or, inward, taken away). The polygon stays in lon/lat, along the same densified
    // edges as the outline's pieces.
    let lonlat = shape(&|r| LineString::new(densify(r).iter().map(|c| GC { x: c.x, y: c.y }).collect()));
    let rings: Vec<Vec<Coord>> = polys.iter().flatten().cloned().collect();
    let outline = union(stretched(&rings, metres.abs()));
    if metres >= 0.0 { lonlat.union(&outline) } else { lonlat.difference(&outline) }
}

/// Lines and rings without their non-finite coordinates.
fn finite(parts: &[Vec<Coord>]) -> Vec<Vec<Coord>> {
    parts.iter().map(|p| p.iter().copied().filter(|c| c.is_finite()).collect::<Vec<_>>()).filter(|p| !p.is_empty()).collect()
}

fn fits(pts: &[Coord], centre: Coord) -> bool {
    pts.iter().all(|c| haversine_m(centre, *c) <= ONE_PIECE_M)
}

/// The buffers of lines cut into stretches, each buffered in its own plane, in lon/lat.
fn stretched(parts: &[Vec<Coord>], metres: f64) -> Vec<MultiPolygon<f64>> {
    let mut out = Vec::new();
    for part in parts {
        let d = densify(part);
        let near = |a: usize, b: usize| d.get(a).zip(d.get(b)).is_some_and(|(a, b)| haversine_m(*a, *b) <= STRETCH_M);
        let mut start = 0;
        loop {
            let mut end = start + 1;
            while end + 1 < d.len() && near(start, end + 1) {
                end += 1;
            }
            let Some(stretch) = d.get(start..=end.min(d.len().saturating_sub(1))) else { break };
            if let (Some(centre), Some(first)) = (spherical_centre(stretch), stretch.first()) {
                let plane = Plane::new(centre, first.x);
                out.push(plane.back(&plane.line(stretch).buffer(metres), metres));
            }
            // Consecutive stretches share a vertex, so their buffers overlap.
            if end + 1 >= d.len() {
                break;
            }
            start = end;
        }
    }
    out
}

/// A line with every edge split along its geodesic on WGS 84 into pieces of at most [`DENSIFY_M`], or
/// longer ones for a line so long it would gain more than [`MAX_ADDED`] vertices.
fn densify(pts: &[Coord]) -> Vec<Coord> {
    let step = DENSIFY_M.max(rustgis_geom::geodesic_length_m(pts) / MAX_ADDED);
    let mut out: Vec<Coord> = pts.first().copied().into_iter().collect();
    for w in pts.windows(2) {
        let [a, b] = w else { continue };
        let n = (haversine_m(*a, *b) / step).ceil().clamp(1.0, MAX_ADDED) as usize;
        out.extend((1..n).map(|i| on_wgs84(*a, *b, i as f64 / n as f64)));
        out.push(*b);
    }
    out
}

/// The point a fraction `t` of the way from `a` to `b` along the geodesic between them on WGS 84,
/// the path the rest of the buffer follows (a great circle runs tens of metres from it over a
/// few hundred kilometres). Coordinates that aren't on the Earth (a latitude beyond ±90°, say)
/// have no geodesic, so they get the great circle, which stays finite.
fn on_wgs84(a: Coord, b: Coord, t: f64) -> Coord {
    let on_earth = |c: Coord| c.x.is_finite() && (-90.0..=90.0).contains(&c.y);
    if on_earth(a) && on_earth(b) {
        let p = geo::InterpolatePoint::point_at_ratio_between(&geo::Geodesic, geo::Point::new(a.x, a.y), geo::Point::new(b.x, b.y), t);
        let p = Coord::new(a.x + wrap(p.x() - a.x), p.y());
        if p.is_finite() {
            return p;
        }
    }
    geodesic_interpolate(a, b, t)
}

fn union(pieces: Vec<MultiPolygon<f64>>) -> MultiPolygon<f64> {
    if pieces.len() == 1 {
        return pieces.into_iter().next().unwrap_or_else(|| MultiPolygon::new(vec![]));
    }
    let oriented: Vec<Polygon<f64>> = pieces.iter().flat_map(|m| m.0.iter()).map(|p| p.orient(Direction::Default)).collect();
    geo::unary_union(oriented.iter())
}

fn to_geometry(mp: &MultiPolygon<f64>) -> Option<Geometry> {
    let ring = |r: &LineString<f64>| r.0.iter().map(|c| Coord::new(c.x, c.y)).collect::<Vec<_>>();
    let polys: Vec<Vec<Vec<Coord>>> =
        mp.0.iter().map(|p| std::iter::once(p.exterior()).chain(p.interiors().iter()).map(ring).collect()).collect();
    (!polys.is_empty()).then_some(Geometry::Polygon(polys))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ring_gains_at_most_max_added_points() {
        // 2,000 edges of about 100 km zigzagging across the plane, each needing dozens of splits.
        let plane = Plane::new(Coord::new(10.0, 46.0), 10.0);
        let zigzag: Vec<GC<f64>> =
            (0..=2000).map(|i| GC { x: i as f64 * 100.0 - 100_000.0, y: if i % 2 == 0 { -50_000.0 } else { 50_000.0 } }).collect();
        let n = zigzag.len();
        let ring = plane.ring(&LineString::new(zigzag), MIN_TOLERANCE_M);
        assert_eq!(ring.0.len(), n + MAX_ADDED as usize);
    }
}
