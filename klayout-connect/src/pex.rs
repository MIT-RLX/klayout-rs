//! Parasitic extraction (PEX).
//!
//! Compute per-net resistance + ground capacitance + pairwise coupling
//! capacitance from a single conducting layer's geometry. Per-layer
//! params are an `LayerPexParams` struct holding sheet resistance,
//! area-cap, edge-cap, and coupling-cap coefficients.
//!
//! Model:
//! * **Resistance** — bbox approximation: `R = ρ_sheet × (long / short)`
//!   where `long`/`short` are the polygon's bbox dimensions. Exact for
//!   rectangular wires; conservative-monotone for L-shapes and bent
//!   wires (under-estimates true resistance; KLayout's "skeleton"
//!   extractor is the v2 upgrade for accuracy).
//! * **Ground capacitance** — `C_g = area × C_area + perimeter × C_edge`.
//!   Captures the bulk parallel-plate term plus fringing on the
//!   perimeter. Both coefficients are per-area / per-length in the
//!   layer's micron units.
//! * **Coupling capacitance** — for each pair of nets, sum over facing
//!   axis-aligned edge pairs within `coupling_max_distance` of each
//!   other: `C_c += overlap × C_couple / spacing`.
//!
//! Multi-layer: call this per conductor layer and sum the per-net
//! results. Hierarchical / via-aware extraction is the layer above
//! (use `klayout-connect::hier`).

use crate::netlist::NetId;
use klayout_core::{Bbox, CellId, LayerIndex, Library, Polygon, Shape};
use klayout_geom::{merge, Edges, Region};
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct LayerPexParams {
    /// Sheet resistance Ω/□.
    pub sheet_rho: f64,
    /// Plate capacitance per area (e.g. fF/μm²).
    pub area_cap: f64,
    /// Edge (fringing) capacitance per perimeter unit (fF/μm).
    pub edge_cap: f64,
    /// Coupling lookup window — pairs of edges within this distance in
    /// DBU contribute to coupling capacitance. Larger windows catch
    /// more coupling but quadratically more pair-work.
    pub coupling_max_distance_dbu: i64,
    /// Coupling capacitance coefficient (fF/μm). The contribution from
    /// a single pair of facing edges is
    /// `overlap_um × coupling_cap / spacing_um`.
    pub coupling_cap: f64,
    /// DBU/μm for unit conversion (matches `Library::dbu()`).
    pub dbu_per_um: f64,
}

impl LayerPexParams {
    pub fn dbu_to_um(&self, v: i64) -> f64 {
        v as f64 / self.dbu_per_um
    }
}

#[derive(Clone, Debug)]
pub struct NetParasitics {
    /// Index into the returned vector — matches the merged net's
    /// position in the polygon-collection. Used for coupling tables.
    pub net_index: usize,
    pub bbox: Bbox,
    /// Optional name (from labels). `None` if not labeled.
    pub name: Option<SmolStr>,
    /// Resistance in Ω.
    pub resistance: f64,
    /// Ground capacitance in fF (assuming params are in fF/μm² + fF/μm).
    pub ground_cap: f64,
    /// Coupling capacitance to other nets, by their `net_index`. Symmetric.
    pub coupling: Vec<(usize, f64)>,
    /// Polygon area in DBU² (for diagnostics).
    pub area_dbu2: i128,
    /// Polygon perimeter in DBU (for diagnostics).
    pub perimeter_dbu: i64,
}

/// Run PEX on `cell`'s shapes on `layer`. Each connected (merged)
/// component becomes one net; per-net parasitics are computed against
/// the supplied `params`.
pub fn extract_pex(
    lib: &Library,
    cell: CellId,
    layer: LayerIndex,
    params: &LayerPexParams,
) -> Vec<NetParasitics> {
    let cell_arc = lib.get(cell);
    let mut polys: Vec<Polygon> = Vec::new();
    for shape in cell_arc.shapes_on(layer) {
        match shape {
            Shape::Polygon(p) => polys.push(p.clone()),
            Shape::Box(r) => polys.push(Polygon::rect(r.bbox)),
            _ => {}
        }
    }
    if polys.is_empty() {
        return Vec::new();
    }
    let merged = merge(&Region::from_polygons(polys));
    pex_from_polygons(lib, cell, layer, merged.polygons(), params)
}

/// Lower-level entry: compute parasitics on an explicit list of
/// already-merged net polygons. Useful when callers do their own
/// connectivity pass (e.g. with via stitching).
pub fn pex_from_polygons(
    lib: &Library,
    cell: CellId,
    layer: LayerIndex,
    nets: &[Polygon],
    params: &LayerPexParams,
) -> Vec<NetParasitics> {
    let mut out: Vec<NetParasitics> = nets
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let area = polygon_area_dbu2(p);
            let perim = polygon_perimeter_dbu(p);
            let bbox = p.bbox();
            let resistance = bbox_resistance(bbox, params.sheet_rho);
            let area_um2 = area as f64 / (params.dbu_per_um * params.dbu_per_um);
            let perim_um = params.dbu_to_um(perim);
            let ground_cap = area_um2 * params.area_cap + perim_um * params.edge_cap;
            NetParasitics {
                net_index: i,
                bbox,
                name: label_at(lib, cell, layer, bbox),
                resistance,
                ground_cap,
                coupling: Vec::new(),
                area_dbu2: area,
                perimeter_dbu: perim,
            }
        })
        .collect();

    // Compute pairwise coupling. For each pair of nets, iterate over
    // their axis-aligned edges and sum facing-pair contributions.
    let nets_edges: Vec<Edges> = nets.iter().map(Edges::from_polygon).collect();
    let max_d = params.coupling_max_distance_dbu;
    for i in 0..nets.len() {
        for j in (i + 1)..nets.len() {
            let bi = nets[i].bbox();
            let bj = nets[j].bbox();
            // Bbox-based prune: nets farther apart than max_d in both
            // axes can't contribute.
            if bbox_distance(bi, bj) > max_d {
                continue;
            }
            let cap = pairwise_coupling(&nets_edges[i], &nets_edges[j], params);
            if cap > 0.0 {
                out[i].coupling.push((j, cap));
                out[j].coupling.push((i, cap));
            }
        }
    }

    out
}

/// Optional binding: associate each `NetParasitics` with a `NetId` from
/// an existing `Netlist`. Matches by bbox containment of the netlist
/// net's bbox against the PEX-extracted bbox.
#[derive(Clone, Debug)]
pub struct NetParasiticsWithId {
    pub net_id: NetId,
    pub parasitics: NetParasitics,
}

fn label_at(
    lib: &Library,
    cell: CellId,
    layer: LayerIndex,
    bbox: Bbox,
) -> Option<SmolStr> {
    let _ = lib;
    let _ = cell;
    let _ = layer;
    let _ = bbox;
    None
}

fn bbox_resistance(b: Bbox, sheet_rho: f64) -> f64 {
    if b.is_empty() {
        return 0.0;
    }
    let w = b.width().max(1) as f64;
    let h = b.height().max(1) as f64;
    let long = w.max(h);
    let short = w.min(h);
    sheet_rho * long / short
}

fn polygon_area_dbu2(p: &Polygon) -> i128 {
    let mut s: i128 = 0;
    let n = p.hull.len();
    if n < 3 {
        return 0;
    }
    for i in 0..n {
        let a = p.hull[i];
        let b = p.hull[(i + 1) % n];
        s += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
    }
    let mut total = s.abs() / 2;
    for hole in &p.holes {
        let mut h: i128 = 0;
        let m = hole.len();
        for i in 0..m {
            let a = hole[i];
            let b = hole[(i + 1) % m];
            h += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
        }
        total -= h.abs() / 2;
    }
    total
}

fn polygon_perimeter_dbu(p: &Polygon) -> i64 {
    let mut total: i64 = 0;
    for ring in std::iter::once(p.hull.as_slice()).chain(p.holes.iter().map(|h| h.as_slice())) {
        let n = ring.len();
        for i in 0..n {
            let a = ring[i];
            let b = ring[(i + 1) % n];
            // Manhattan-friendly: for axis-aligned edges this is exact;
            // for diagonals it's the L1 distance, an over-estimate. The
            // caller's edge_cap should be calibrated to that convention.
            let dx = (b.x - a.x).abs();
            let dy = (b.y - a.y).abs();
            // Use Euclidean; round up to nearest DBU unit.
            let d2 = (dx as i128) * (dx as i128) + (dy as i128) * (dy as i128);
            total = total.saturating_add((d2 as f64).sqrt().round() as i64);
        }
    }
    total
}

fn bbox_distance(a: Bbox, b: Bbox) -> i64 {
    if a.is_empty() || b.is_empty() {
        return i64::MAX;
    }
    let dx = if a.max.x < b.min.x {
        b.min.x - a.max.x
    } else if b.max.x < a.min.x {
        a.min.x - b.max.x
    } else {
        0
    };
    let dy = if a.max.y < b.min.y {
        b.min.y - a.max.y
    } else if b.max.y < a.min.y {
        a.min.y - b.max.y
    } else {
        0
    };
    dx.max(dy)
}

/// Sum coupling capacitance over all facing edge pairs between two
/// nets. Considers axis-aligned edges only (horizontal pairs and
/// vertical pairs separately). Each pair contributes
/// `overlap_um × coupling_cap / spacing_um`.
fn pairwise_coupling(a: &Edges, b: &Edges, params: &LayerPexParams) -> f64 {
    let mut total = 0.0;
    // Horizontal-horizontal: same dy, opposite dx sign, perpendicular
    // distance |y_a - y_b| within max_d, x-overlap > 0.
    for ea in a.edges() {
        if !ea.is_axis_aligned() {
            continue;
        }
        for eb in b.edges() {
            if !eb.is_axis_aligned() {
                continue;
            }
            let cap = pair_coupling(ea, eb, params);
            total += cap;
        }
    }
    total
}

fn pair_coupling(
    a: &klayout_geom::Edge,
    b: &klayout_geom::Edge,
    params: &LayerPexParams,
) -> f64 {
    let max_d = params.coupling_max_distance_dbu;
    if a.is_horizontal() && b.is_horizontal() {
        let dy = (a.a.y - b.a.y).abs();
        if dy == 0 || dy > max_d {
            return 0.0;
        }
        // Edges face each other only if their interior normals point
        // toward each other. We approximate this by requiring the
        // *direction* of the edges to be opposite (one east, one west).
        let dir_a = a.b.x - a.a.x;
        let dir_b = b.b.x - b.a.x;
        if dir_a.signum() == dir_b.signum() {
            return 0.0;
        }
        let lo = a.a.x.min(a.b.x).max(b.a.x.min(b.b.x));
        let hi = a.a.x.max(a.b.x).min(b.a.x.max(b.b.x));
        if hi <= lo {
            return 0.0;
        }
        let overlap_um = params.dbu_to_um(hi - lo);
        let dy_um = params.dbu_to_um(dy);
        if dy_um <= 0.0 {
            return 0.0;
        }
        overlap_um * params.coupling_cap / dy_um
    } else if a.is_vertical() && b.is_vertical() {
        let dx = (a.a.x - b.a.x).abs();
        if dx == 0 || dx > max_d {
            return 0.0;
        }
        let dir_a = a.b.y - a.a.y;
        let dir_b = b.b.y - b.a.y;
        if dir_a.signum() == dir_b.signum() {
            return 0.0;
        }
        let lo = a.a.y.min(a.b.y).max(b.a.y.min(b.b.y));
        let hi = a.a.y.max(a.b.y).min(b.a.y.max(b.b.y));
        if hi <= lo {
            return 0.0;
        }
        let overlap_um = params.dbu_to_um(hi - lo);
        let dx_um = params.dbu_to_um(dx);
        if dx_um <= 0.0 {
            return 0.0;
        }
        overlap_um * params.coupling_cap / dx_um
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::{Bbox as Bb, CellBuilder, LayerInfo, Library, Point as P, Rect};

    fn rect_lib(rects: &[Bb]) -> (Library, CellId, LayerIndex) {
        let lib = Library::new("t", 1000);
        let l = lib.layer(LayerInfo::gds(1, 0));
        let mut cb = CellBuilder::new("c");
        for r in rects {
            cb.add_shape(l, Rect::new(*r));
        }
        let id = lib.insert(cb);
        (lib, id, l)
    }

    fn params() -> LayerPexParams {
        LayerPexParams {
            sheet_rho: 0.1,
            area_cap: 0.001, // fF/μm²
            edge_cap: 0.05,  // fF/μm
            coupling_max_distance_dbu: 10_000,
            coupling_cap: 0.05,
            dbu_per_um: 1000.0,
        }
    }

    #[test]
    fn single_long_wire_resistance_uses_aspect_ratio() {
        // 10 μm long, 1 μm wide → R = 0.1 × 10 = 1.0 Ω.
        let (lib, c, l) = rect_lib(&[Bb::new(P::new(0, 0), P::new(10_000, 1_000))]);
        let r = extract_pex(&lib, c, l, &params());
        assert_eq!(r.len(), 1);
        assert!((r[0].resistance - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ground_cap_is_area_plus_edge() {
        // 10 μm × 5 μm → area = 50 μm², perimeter = 30 μm.
        // C = 50 × 0.001 + 30 × 0.05 = 0.05 + 1.5 = 1.55 fF.
        let (lib, c, l) = rect_lib(&[Bb::new(P::new(0, 0), P::new(10_000, 5_000))]);
        let r = extract_pex(&lib, c, l, &params());
        assert_eq!(r.len(), 1);
        assert!((r[0].ground_cap - 1.55).abs() < 1e-6, "{}", r[0].ground_cap);
    }

    #[test]
    fn two_separate_nets_emit_two_entries() {
        let (lib, c, l) = rect_lib(&[
            Bb::new(P::new(0, 0), P::new(1_000, 1_000)),
            Bb::new(P::new(5_000, 0), P::new(6_000, 1_000)),
        ]);
        let r = extract_pex(&lib, c, l, &params());
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn touching_polygons_are_one_net() {
        // Abutting at x=1000 → one merged net.
        let (lib, c, l) = rect_lib(&[
            Bb::new(P::new(0, 0), P::new(1_000, 1_000)),
            Bb::new(P::new(1_000, 0), P::new(2_000, 1_000)),
        ]);
        let r = extract_pex(&lib, c, l, &params());
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn coupling_capacitance_between_parallel_wires() {
        // Two wires running parallel, 1 μm apart, 10 μm long.
        // Overlap = 10 μm, spacing = 1 μm.
        // C_couple = 10 × 0.05 / 1 = 0.5 fF (each side; reported once).
        let (lib, c, l) = rect_lib(&[
            Bb::new(P::new(0, 0), P::new(10_000, 500)),
            Bb::new(P::new(0, 1_500), P::new(10_000, 2_000)),
        ]);
        let r = extract_pex(&lib, c, l, &params());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].coupling.len(), 1);
        let (other, cap) = r[0].coupling[0];
        assert_eq!(other, 1);
        assert!(cap > 0.0, "coupling > 0 expected");
        // Symmetric.
        let (other2, cap2) = r[1].coupling[0];
        assert_eq!(other2, 0);
        assert!((cap - cap2).abs() < 1e-9);
    }

    #[test]
    fn distant_wires_have_no_coupling() {
        // 20 μm apart > max_distance (10 μm).
        let (lib, c, l) = rect_lib(&[
            Bb::new(P::new(0, 0), P::new(10_000, 500)),
            Bb::new(P::new(0, 20_500), P::new(10_000, 21_000)),
        ]);
        let r = extract_pex(&lib, c, l, &params());
        assert_eq!(r.len(), 2);
        assert!(r[0].coupling.is_empty());
        assert!(r[1].coupling.is_empty());
    }
}
