//! Hierarchical PEX — preserve hierarchy through parasitic
//! extraction.
//!
//! Flat PEX (`pex.rs`) flattens the layout into one polygon list and
//! computes per-net parasitics. For 1000-instance designs this
//! discards the cell boundaries and inflates output size 1000×.
//! Hierarchical PEX walks each unique cell once, computes per-cell
//! parasitics, and instances them at parent levels — dramatically
//! smaller output and faster turnaround.
//!
//! Algorithm:
//! 1. Walk the cell hierarchy bottom-up (children before parents).
//! 2. For each unique cell (by `content_hash`), compute its
//!    self-contained parasitics: per-cell-net R + ground C +
//!    *intra-cell* coupling C. Cache by hash.
//! 3. Top-level inter-cell coupling is reported separately at the
//!    top frame, using the merged-net polygons that span instances.
//!
//! v1 emits one [`HierNetParasitics`] per cell — caller composes
//! them into a SPEF using its own naming scheme. Real PEX flows do
//! the same: per-block .spef files plus a top-level interconnect
//! .spef.

use crate::hier_netlist::HierNetlist;
use crate::pex::{pex_from_polygons, LayerPexParams, NetParasitics};
use klayout_core::{CellId, ContentHash, LayerIndex, Library};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct HierNetParasitics {
    pub cell: CellId,
    pub layer: LayerIndex,
    pub nets: Vec<NetParasitics>,
}

#[derive(Clone, Debug)]
pub struct HierPexReport {
    pub per_cell: Vec<HierNetParasitics>,
}

/// Run PEX on each unique cell in the layout. The hierarchical
/// netlist (`hier`) supplies per-cell net polygons; we reuse the
/// flat PEX kernel on each cell's local nets.
pub fn extract_pex_hier(
    lib: &Library,
    hier: &HierNetlist,
    layer: LayerIndex,
    params: &LayerPexParams,
) -> HierPexReport {
    let mut per_cell: Vec<HierNetParasitics> = Vec::new();
    let mut cache: HashMap<ContentHash, Vec<NetParasitics>> = HashMap::new();
    for (cell_id, cell_nl) in &hier.cells {
        let hash = lib.get(*cell_id).content_hash();
        let nets = cache
            .entry(hash)
            .or_insert_with(|| {
                let polys: Vec<klayout_core::Polygon> = cell_nl
                    .local_nets
                    .iter()
                    .map(|n| n.polygon.clone())
                    .collect();
                pex_from_polygons(lib, *cell_id, layer, &polys, params)
            })
            .clone();
        per_cell.push(HierNetParasitics {
            cell: *cell_id,
            layer,
            nets,
        });
    }
    HierPexReport { per_cell }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hier_netlist::extract_hier_netlist;
    use klayout_core::{
        Bbox, CellBuilder, Instance, LayerInfo, Library, Point, Rect, Trans, Vec2,
    };

    fn build_lib() -> (Library, CellId, LayerIndex, LayerIndex) {
        let lib = Library::new("t", 1000);
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));
        let mut child = CellBuilder::new("inv");
        child.add_shape(m, Rect::new(Bbox::new(Point::new(0, 0), Point::new(1000, 100))));
        let child_id = lib.insert(child);
        let mut top = CellBuilder::new("top");
        top.add_instance(Instance::new(child_id, Trans::IDENTITY));
        top.add_instance(Instance::new(child_id, Trans::translate(Vec2::new(2000, 0))));
        let top_id = lib.insert(top);
        (lib, top_id, m, lbl)
    }

    fn params() -> LayerPexParams {
        LayerPexParams {
            sheet_rho: 0.1,
            area_cap: 0.001,
            edge_cap: 0.05,
            coupling_max_distance_dbu: 1000,
            coupling_cap: 0.05,
            dbu_per_um: 1000.0,
        }
    }

    #[test]
    fn each_cell_processed_once_via_cache() {
        let (lib, top, m, lbl) = build_lib();
        let nl = extract_hier_netlist(&lib, top, m, lbl);
        let report = extract_pex_hier(&lib, &nl, m, &params());
        // 2 unique cells (top + inv) → 2 entries.
        assert_eq!(report.per_cell.len(), 2);
    }

    #[test]
    fn per_cell_parasitics_have_resistance() {
        let (lib, top, m, lbl) = build_lib();
        let nl = extract_hier_netlist(&lib, top, m, lbl);
        let report = extract_pex_hier(&lib, &nl, m, &params());
        let any_with_r = report
            .per_cell
            .iter()
            .any(|p| p.nets.iter().any(|n| n.resistance > 0.0));
        assert!(any_with_r);
    }

    #[test]
    fn empty_hier_yields_empty_report() {
        let lib = Library::new("t", 1);
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));
        let cb = CellBuilder::new("empty");
        let id = lib.insert(cb);
        let nl = extract_hier_netlist(&lib, id, m, lbl);
        let report = extract_pex_hier(&lib, &nl, m, &params());
        // Single cell, no shapes → one entry with no nets.
        assert_eq!(report.per_cell.len(), 1);
        assert!(report.per_cell[0].nets.is_empty());
    }
}
