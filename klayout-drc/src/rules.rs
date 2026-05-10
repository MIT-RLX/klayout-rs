//! Rule primitives. `width`, `space`, and `separation` use exact edge-pair
//! analysis (matches KLayout). `enclosing` and `overlap` use shrink-based
//! approximations and may differ from KLayout's edge-pair geometry — see
//! the per-function docs.

use crate::edge::{
    concave_corners, enclosing_pair, overlap_pair, polygon_edges, space_pair,
    width_pairs_indexed, AAEdge,
};
use crate::edge_general::{general_pair, polygon_dir_edges};
use klayout_core::{Bbox, Point, Polygon};
use klayout_geom::{merge, Region};
use klayout_spatial::SpatialIndex;
use rayon::prelude::*;

/// Find regions on `r` narrower than `min`.
///
/// Edge-pair-based: enumerates each polygon's axis-aligned edges and
/// flags pairs of opposing internal edges with distance < `min`. Matches
/// KLayout's `width_check` exactly for axis-aligned input.
///
/// Non-axis-aligned edges are skipped silently. A future pass will add
/// 45° / arbitrary-angle support.
pub fn width(r: &Region, min: i64) -> Region {
    if min <= 0 || r.is_empty() {
        return Region::empty();
    }
    // KLayout's width_check operates on the merged region so two abutting
    // polygons see each other's edges as one continuous boundary.
    let merged = merge(r);
    // Per-polygon work is independent — parallelize across polygons.
    let violations: Vec<Polygon> = merged
        .polygons()
        .par_iter()
        .flat_map_iter(|p| {
            let edges = polygon_edges(p);
            let concave = concave_corners(p);
            width_pairs_indexed(&edges, &concave, min)
        })
        .collect();
    polygons_to_region(violations)
}

/// Find pairs of polygons in `r` separated by less than `min`.
///
/// Edge-pair-based: pairs of polygons' edges are checked for "facing
/// through gap" — exterior sides oriented toward each other, parallel,
/// perpendicular overlap, distance < `min`. Matches KLayout's
/// `space_check` exactly for axis-aligned input.
pub fn space(r: &Region, min: i64) -> Region {
    if min <= 0 || r.is_empty() {
        return Region::empty();
    }
    let merged = merge(r);
    let polys = merged.polygons();
    if polys.len() < 2 {
        return Region::empty();
    }
    let edges: Vec<Vec<AAEdge>> = polys.iter().map(polygon_edges).collect();
    // Spatial index over per-polygon bboxes inflated by `min` so the
    // query directly returns candidate neighbors for the space check.
    let idx = SpatialIndex::build(
        polys
            .iter()
            .enumerate()
            .map(|(i, p)| (inflate(p.bbox(), min), i)),
    );
    let violations: Vec<Polygon> = (0..polys.len())
        .into_par_iter()
        .flat_map_iter(|i| {
            let p = &polys[i];
            let mut local: Vec<Polygon> = Vec::new();
            for (_, &j) in idx.query(p.bbox()) {
                if j <= i {
                    continue;
                }
                for ei in &edges[i] {
                    for ej in &edges[j] {
                        if let Some(poly) = space_pair(ei, ej, min) {
                            local.push(poly);
                        }
                    }
                }
            }
            local
        })
        .collect();
    polygons_to_region(violations)
}

/// Find regions where `a` and `b` are closer than `min` (between layers).
///
/// Edge-pair check across two regions. Same predicate as `space` but with
/// edges drawn from different inputs. Matches KLayout's
/// `separation_check` exactly for axis-aligned input.
pub fn separation(a: &Region, b: &Region, min: i64) -> Region {
    if min <= 0 || a.is_empty() || b.is_empty() {
        return Region::empty();
    }
    let a_merged = merge(a);
    let b_merged = merge(b);
    let polys_a = a_merged.polygons();
    let polys_b = b_merged.polygons();
    let edges_a: Vec<Vec<AAEdge>> = polys_a.iter().map(polygon_edges).collect();
    let edges_b: Vec<Vec<AAEdge>> = polys_b.iter().map(polygon_edges).collect();
    // Spatial index over `b`'s polygons so each `a` polygon's lookup
    // hits only the b's actually within `min` of it.
    let idx_b = SpatialIndex::build(
        polys_b
            .iter()
            .enumerate()
            .map(|(j, p)| (inflate(p.bbox(), min), j)),
    );
    let violations: Vec<Polygon> = (0..polys_a.len())
        .into_par_iter()
        .flat_map_iter(|i| {
            let p_a = &polys_a[i];
            let mut local: Vec<Polygon> = Vec::new();
            for (_, &j) in idx_b.query(p_a.bbox()) {
                for ea in &edges_a[i] {
                    for eb in &edges_b[j] {
                        if let Some(poly) = space_pair(ea, eb, min) {
                            local.push(poly);
                        }
                    }
                }
            }
            local
        })
        .collect();
    polygons_to_region(violations)
}

/// Find parts of `inner` that aren't enclosed by ≥ `min` of `outer`.
///
/// Edge-pair check: pairs of same-direction edges from outer × inner with
/// inner "more interior" than outer and within `min`. The violation
/// polygon is a parallelogram between the two edges, with outer's range
/// extended past inner's endpoints by `round(sqrt(min² - dist²))` —
/// matches KLayout's `enclosing_check` exactly.
pub fn enclosing(outer: &Region, inner: &Region, min: i64) -> Region {
    if min <= 0 || inner.is_empty() || outer.is_empty() {
        return Region::empty();
    }
    let outer_merged = merge(outer);
    let inner_merged = merge(inner);
    let polys_o = outer_merged.polygons();
    let polys_i = inner_merged.polygons();
    let edges_o: Vec<Vec<AAEdge>> = polys_o.iter().map(polygon_edges).collect();
    let edges_i: Vec<Vec<AAEdge>> = polys_i.iter().map(polygon_edges).collect();
    let idx_inner = SpatialIndex::build(
        polys_i
            .iter()
            .enumerate()
            .map(|(j, p)| (inflate(p.bbox(), min), j)),
    );
    let violations: Vec<Polygon> = (0..polys_o.len())
        .into_par_iter()
        .flat_map_iter(|i| {
            let p_o = &polys_o[i];
            let mut local: Vec<Polygon> = Vec::new();
            for (_, &j) in idx_inner.query(p_o.bbox()) {
                for eo in &edges_o[i] {
                    for ei in &edges_i[j] {
                        if let Some(poly) = enclosing_pair(eo, ei, min) {
                            local.push(poly);
                        }
                    }
                }
            }
            local
        })
        .collect();
    polygons_to_region(violations)
}

/// Find regions where `a` and `b` overlap by less than `min` width.
///
/// Edge-pair check: opposite-direction edges from a × b with chamfer-extended
/// ranges past the projection-overlap bounds (matching KLayout's
/// `overlap_check`).
pub fn overlap(a: &Region, b: &Region, min: i64) -> Region {
    if min <= 0 || a.is_empty() || b.is_empty() {
        return Region::empty();
    }
    let a_merged = merge(a);
    let b_merged = merge(b);
    let polys_a = a_merged.polygons();
    let polys_b = b_merged.polygons();
    let edges_a: Vec<Vec<AAEdge>> = polys_a.iter().map(polygon_edges).collect();
    let edges_b: Vec<Vec<AAEdge>> = polys_b.iter().map(polygon_edges).collect();
    let idx_b = SpatialIndex::build(
        polys_b
            .iter()
            .enumerate()
            .map(|(j, p)| (p.bbox(), j)),
    );
    let violations: Vec<Polygon> = (0..polys_a.len())
        .into_par_iter()
        .flat_map_iter(|i| {
            let p_a = &polys_a[i];
            let mut local: Vec<Polygon> = Vec::new();
            for (_, &j) in idx_b.query(p_a.bbox()) {
                for ea in &edges_a[i] {
                    for eb in &edges_b[j] {
                        if let Some(poly) = overlap_pair(ea, eb, min) {
                            local.push(poly);
                        }
                    }
                }
            }
            local
        })
        .collect();
    polygons_to_region(violations)
}

/// Density check: flag windows where `region_area / window_area` falls
/// outside `[min_density, max_density]`. Sliding window iteration over
/// the region's bbox using `(window_w, window_h)` step + size.
///
/// Densities are expressed as fractions in `[0.0, 1.0]`. Windows that
/// fully exit the layout's bbox are skipped (no degenerate-edge windows).
///
/// Returns a `Region` of violation rectangles — each one is a window
/// whose density was out of bounds. Compose with `merge` if you want
/// the merged-block view.
pub fn density(
    r: &Region,
    window: (i64, i64),
    step: (i64, i64),
    min_density: f64,
    max_density: f64,
) -> Region {
    if r.is_empty() || window.0 <= 0 || window.1 <= 0 {
        return Region::empty();
    }
    let bbox = r.bbox();
    if bbox.is_empty() {
        return Region::empty();
    }
    let merged = merge(r);
    let window_area = (window.0 as f64) * (window.1 as f64);
    let mut violations: Vec<Polygon> = Vec::new();

    let step_x = step.0.max(1);
    let step_y = step.1.max(1);
    let mut y = bbox.min.y;
    while y < bbox.max.y {
        let mut x = bbox.min.x;
        while x < bbox.max.x {
            let win_bbox = klayout_core::Bbox::new(
                klayout_core::Point::new(x, y),
                klayout_core::Point::new(x + window.0, y + window.1),
            );
            // Density = (area of region within window) / window area.
            let win_region =
                Region::from_polygons([Polygon::rect(win_bbox)]);
            let inter = klayout_geom::intersection(&merged, &win_region);
            let area_in: i128 = inter
                .polygons()
                .iter()
                .map(|p| polygon_area2(p) / 2)
                .sum();
            let density = (area_in as f64) / window_area;
            if density < min_density || density > max_density {
                violations.push(Polygon::rect(win_bbox));
            }
            x += step_x;
        }
        y += step_y;
    }
    // Don't auto-merge — each window is a distinct violation entry.
    Region::from_polygons(violations)
}

/// Width check that handles arbitrary-angle edges.
///
/// The axis-aligned [`width`] kernel skips diagonal edges silently —
/// fine for digital flows, miss-the-bug for analog/RF. `width_any`
/// uses a generalized vector-math edge-pair kernel that handles
/// arbitrary angles. Slower per-pair than the AA version, so call
/// this only when diagonals are expected.
pub fn width_any(r: &Region, min: i64) -> Region {
    if min <= 0 || r.is_empty() {
        return Region::empty();
    }
    let merged = merge(r);
    let violations: Vec<Polygon> = merged
        .polygons()
        .par_iter()
        .flat_map_iter(|p| {
            let edges = polygon_dir_edges(p);
            let mut local: Vec<Polygon> = Vec::new();
            for i in 0..edges.len() {
                for j in (i + 1)..edges.len() {
                    if let Some(poly) = general_pair(&edges[i], &edges[j], min, true) {
                        local.push(poly);
                    }
                }
            }
            local
        })
        .collect();
    polygons_to_region(violations)
}

/// Space check that handles arbitrary-angle edges. Cross-polygon
/// counterpart of [`width_any`].
pub fn space_any(r: &Region, min: i64) -> Region {
    if min <= 0 || r.is_empty() {
        return Region::empty();
    }
    let merged = merge(r);
    let polys = merged.polygons();
    if polys.len() < 2 {
        return Region::empty();
    }
    let edges: Vec<Vec<crate::edge_general::DirEdge>> =
        polys.iter().map(polygon_dir_edges).collect();
    let idx = SpatialIndex::build(
        polys
            .iter()
            .enumerate()
            .map(|(i, p)| (inflate(p.bbox(), min), i)),
    );
    let violations: Vec<Polygon> = (0..polys.len())
        .into_par_iter()
        .flat_map_iter(|i| {
            let p = &polys[i];
            let mut local: Vec<Polygon> = Vec::new();
            for (_, &j) in idx.query(p.bbox()) {
                if j <= i {
                    continue;
                }
                for ei in &edges[i] {
                    for ej in &edges[j] {
                        if let Some(poly) = general_pair(ei, ej, min, false) {
                            local.push(poly);
                        }
                    }
                }
            }
            local
        })
        .collect();
    polygons_to_region(violations)
}

/// Find polygons in `r` with area < `min` (in DBU²).
pub fn area_min(r: &Region, min: i128) -> Region {
    if min <= 0 || r.is_empty() {
        return Region::empty();
    }
    let small: Vec<Polygon> = r
        .polygons()
        .iter()
        .filter(|p| polygon_area2(p) / 2 < min)
        .cloned()
        .collect();
    Region::from_polygons(small)
}

fn polygon_area2(p: &Polygon) -> i128 {
    let mut s: i128 = 0;
    let n = p.hull.len();
    for i in 0..n {
        let a = p.hull[i];
        let b = p.hull[(i + 1) % n];
        s += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
    }
    let mut total = s.abs();
    for hole in &p.holes {
        let mut s: i128 = 0;
        let n = hole.len();
        for i in 0..n {
            let a = hole[i];
            let b = hole[(i + 1) % n];
            s += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
        }
        total -= s.abs();
    }
    total
}

/// Inflate a bbox by `delta` on every side. Used for spatial-index
/// queries: items whose inflated bbox overlaps `q.bbox()` are exactly
/// the candidates within `delta` of the query polygon.
fn inflate(b: Bbox, delta: i64) -> Bbox {
    if b.is_empty() {
        return b;
    }
    Bbox::new(
        Point::new(b.min.x - delta, b.min.y - delta),
        Point::new(b.max.x + delta, b.max.y + delta),
    )
}

fn polygons_to_region(polys: Vec<Polygon>) -> Region {
    if polys.is_empty() {
        return Region::empty();
    }
    // The edge-pair output may emit redundant overlapping rects; merge
    // them so callers see one disjoint set of violation areas.
    let r = Region::from_polygons(polys);
    klayout_geom::merge(&r)
}
