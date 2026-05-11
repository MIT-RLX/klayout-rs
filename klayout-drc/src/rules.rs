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

/// Which tiles KLayout selects: `without_density` (outside the band) vs
/// `with_density` (inside the band). Matches the `inverse` flag in Ruby
/// [`_with_density`](https://www.klayout.de/doc-qt5/about/drc_ref_layer.html#with_density).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DensityWindowOutput {
    /// Ruby `without_density`: emit `bx` when density is **outside**
    /// `(min_density, max_density)` (ε-open interval).
    #[default]
    OutsideBand,
    /// Ruby `with_density`: emit `bx` when density is **inside** the band.
    InsideBand,
}

/// How the density denominator is chosen — matches KLayout DRC
/// `padding_zero` vs `padding_ignore` on [`with_density`] / [`without_density`].
///
/// [`with_density`]: https://www.klayout.de/doc-qt5/about/drc_ref_layer.html#with_density
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DensityPadding {
    /// `density = input.area(bx) / bx.area`
    #[default]
    Zero,
    /// `density = input.area(bx) / boundary.area(bx)` when `boundary.area(bx) > 0`
    Ignore,
}

/// Options matching KLayout `tile_boundary`, `tile_origin`, `tile_count`, and padding.
#[derive(Debug)]
pub struct DensityWindowConfig {
    pub padding: DensityPadding,
    /// `tile_boundary` in KLayout — restricts the boundary used for `padding_ignore`
    /// and contributes to the tiling frame. `None` uses a filled rectangle equal to
    /// the primary **layer** bbox (after [`merge`]).
    pub boundary: Option<Region>,
    pub tile_origin: Option<(i64, i64)>,
    pub tile_count: Option<(usize, usize)>,
    /// `false` reproduces bare KLayout `TilingProcessor` behavior: a 1×1 tile plan
    /// sets `_tile` unset, so the density script never runs → empty violations.
    /// `true` (default) approximates setting a non-empty frame so single tiles are
    /// still evaluated — needed for most PDK-scale decks and our checked-in corpus.
    pub evaluate_singleton_tiles: bool,
    /// `without_density` vs `with_density` result tiles.
    pub output: DensityWindowOutput,
}

impl Default for DensityWindowConfig {
    fn default() -> Self {
        Self {
            padding: DensityPadding::default(),
            boundary: None,
            tile_origin: None,
            tile_count: None,
            evaluate_singleton_tiles: true,
            output: DensityWindowOutput::default(),
        }
    }
}

impl DensityWindowConfig {
    /// Match bare KLayout for 1×1 tile grids (no explicit frame): no violations.
    pub fn klayout_strict_singleton() -> Self {
        Self {
            evaluate_singleton_tiles: false,
            ..Self::default()
        }
    }
}

/// Selected density windows: **violations** when [`DensityWindowOutput::OutsideBand`]
/// (default), or **in-band tiles** when [`DensityWindowOutput::InsideBand`]
/// (KLayout `with_density`).
///
/// The tile plan follows `db::TilingProcessor`: the layer∪boundary bounding
/// box is enlarged by the window/step **border** before tile counts and the
/// default centered origin are computed (see `dbTilingProcessor.cc`).
///
/// Returned geometry is **merged** (touching windows fuse), matching
/// `validation/oracle.py` `dump_region`.
pub fn density_window_with_config(
    r: &Region,
    window: (i64, i64),
    step: (i64, i64),
    min_density: f64,
    max_density: f64,
    config: &DensityWindowConfig,
) -> Region {
    if r.is_empty() || window.0 <= 0 || window.1 <= 0 || step.0 <= 0 || step.1 <= 0 {
        return Region::empty();
    }

    let merged = merge(r);
    let layer_bbox = merged.bbox();
    if layer_bbox.is_empty() {
        return Region::empty();
    }

    let boundary_merged = if let Some(ref b) = config.boundary {
        merge(b)
    } else {
        Region::from_polygons([Polygon::rect(layer_bbox)])
    };
    let boundary_bbox = boundary_merged.bbox();
    if boundary_bbox.is_empty() {
        return Region::empty();
    }

    let tot_bbox = layer_bbox.union(&boundary_bbox);
    if tot_bbox.is_empty() {
        return Region::empty();
    }

    const DBU: f64 = 1.0;
    const EPS: f64 = 1e-10;

    let left = tot_bbox.min.x as f64;
    let bottom = tot_bbox.min.y as f64;
    let right = tot_bbox.max.x as f64;
    let top = tot_bbox.max.y as f64;

    let m_tw = step.0 as f64;
    let m_th = step.1 as f64;

    let tile_w = DBU * (0.5 + m_tw / DBU + EPS).floor();
    let tile_h = DBU * (0.5 + m_th / DBU + EPS).floor();

    // Match `db::TilingProcessor`: enlarge the tiling box by the tile border
    // *before* computing tile counts and the centered origin (`dbTilingProcessor.cc`
    // enlarges `tot_box` by `(m_tile_bx, m_tile_by)`).
    let xb = (0.5 * (window.0 as f64 - m_tw)).max(0.0);
    let yb = (0.5 * (window.1 as f64 - m_th)).max(0.0);
    let xoverlap = (xb / DBU).round() as i64;
    let yoverlap = (yb / DBU).round() as i64;

    let left_e = left - xb;
    let right_e = right + xb;
    let bottom_e = bottom - yb;
    let top_e = top + yb;
    let tot_w_e = right_e - left_e;
    let tot_h_e = top_e - bottom_e;

    let (ntiles_w, ntiles_h) = if let Some((nx, ny)) = config.tile_count {
        (nx.max(1), ny.max(1))
    } else {
        (
            ((tot_w_e / m_tw - EPS).ceil() as usize).max(1),
            ((tot_h_e / m_th - EPS).ceil() as usize).max(1),
        )
    };

    let cx = 0.5 * (left_e + right_e);
    let cy = 0.5 * (bottom_e + top_e);

    let (l, b) = if let Some((ox, oy)) = config.tile_origin {
        (
            DBU * (0.5 + ox as f64 / DBU + EPS).floor(),
            DBU * (0.5 + oy as f64 / DBU + EPS).floor(),
        )
    } else {
        (
            DBU * (0.5 + (cx - ntiles_w as f64 * 0.5 * tile_w) / DBU + EPS).floor(),
            DBU * (0.5 + (cy - ntiles_h as f64 * 0.5 * tile_h) / DBU + EPS).floor(),
        )
    };

    let tw_i = tile_w as i64;
    let th_i = tile_h as i64;
    let l_i = l as i64;
    let b_i = b as i64;

    let klayout_has_tiles = ntiles_w > 1 || ntiles_h > 1;
    if !(klayout_has_tiles || config.evaluate_singleton_tiles) {
        return Region::empty();
    }

    let mut violations: Vec<Polygon> = Vec::new();
    let meas_rect = |meas: Bbox| Region::from_polygons([Polygon::rect(meas)]);

    for iy in 0..ntiles_h {
        for ix in 0..ntiles_w {
            let clip = Bbox::new(
                Point::new(l_i + ix as i64 * tw_i, b_i + iy as i64 * th_i),
                Point::new(l_i + (ix as i64 + 1) * tw_i, b_i + (iy as i64 + 1) * th_i),
            );
            let meas = Bbox::new(
                Point::new(clip.min.x - xoverlap, clip.min.y - yoverlap),
                Point::new(clip.max.x + xoverlap, clip.max.y + yoverlap),
            );
            let mw = (meas.max.x - meas.min.x) as f64;
            let mh = (meas.max.y - meas.min.y) as f64;
            let meas_area = mw * mh;

            let win_region = meas_rect(meas);
            let inter = klayout_geom::intersection(&merged, &win_region);
            let area_in: i128 = inter
                .polygons()
                .iter()
                .map(|p| polygon_area2(p) / 2)
                .sum();

            let density = match config.padding {
                DensityPadding::Zero => (area_in as f64) / meas_area,
                DensityPadding::Ignore => {
                    let binter = klayout_geom::intersection(&boundary_merged, &win_region);
                    let ba: i128 = binter
                        .polygons()
                        .iter()
                        .map(|p| polygon_area2(p) / 2)
                        .sum();
                    if ba <= 0 {
                        continue;
                    }
                    (area_in as f64) / (ba as f64)
                }
            };

            let in_range = density > min_density - EPS && density < max_density + EPS;
            let emit = match config.output {
                DensityWindowOutput::OutsideBand => !in_range,
                DensityWindowOutput::InsideBand => in_range,
            };
            if emit {
                violations.push(Polygon::rect(meas));
            }
        }
    }

    merge(&Region::from_polygons(violations))
}

/// Density-window DRC with default options (`padding_zero`, layer bbox as
/// boundary proxy, centered tiling, singleton tiles evaluated).
///
/// For full KLayout parity knobs see [`density_window_with_config`].
pub fn density_window(
    r: &Region,
    window: (i64, i64),
    step: (i64, i64),
    min_density: f64,
    max_density: f64,
) -> Region {
    density_window_with_config(
        r,
        window,
        step,
        min_density,
        max_density,
        &DensityWindowConfig::default(),
    )
}

/// Backward-compatible name for [`density_window`].
#[inline]
pub fn density(
    r: &Region,
    window: (i64, i64),
    step: (i64, i64),
    min_density: f64,
    max_density: f64,
) -> Region {
    density_window(r, window, step, min_density, max_density)
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
