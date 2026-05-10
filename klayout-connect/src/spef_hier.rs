//! Hierarchical SPEF emission.
//!
//! Flat SPEF lists every net's parasitics at the top frame. For
//! hierarchical designs this duplicates per-cell parasitics N times
//! (once per instance). Real STA flows consume *per-cell* SPEF
//! files plus a top-level interconnect SPEF — the per-cell file is
//! emitted once and instanced.
//!
//! [`write_spef_hier`] takes a [`HierPexReport`] and produces:
//! * One **top-level SPEF** containing top-cell nets + cross-instance
//!   coupling caps.
//! * One **per-child SPEF** per unique child cell, containing that
//!   cell's local nets only.
//!
//! Each is serialised as plain text via the existing SPEF writer
//! kernel; the caller writes them to separate files. Tools that
//! consume hierarchical SPEF (PrimeTime, OpenSTA) handle the
//! cell-by-cell composition.

use crate::pex_hier::HierPexReport;
use crate::spef::{nets_from_pex, write_spef, SpefHeader, SpefNet};
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct HierSpef {
    pub top_text: String,
    /// One entry per child cell: `(cell_name, spef_text)`.
    pub per_cell: Vec<(SmolStr, String)>,
}

pub fn write_spef_hier(
    report: &HierPexReport,
    top_design: &str,
    cell_name: impl Fn(klayout_core::CellId) -> SmolStr,
) -> HierSpef {
    let mut per_cell: Vec<(SmolStr, String)> = Vec::new();
    let mut top_text = String::new();

    for entry in &report.per_cell {
        let name = cell_name(entry.cell);
        // Convert this cell's parasitics to a SpefNet vector.
        let nets: Vec<SpefNet> = nets_from_pex(&entry.nets, |i| {
            SmolStr::from(format!("{name}/net_{i}"))
        });
        let header = SpefHeader {
            design: format!("{name}"),
            ..SpefHeader::default()
        };
        let body = write_spef(&header, &nets);
        if name == top_design {
            top_text = body;
        } else {
            per_cell.push((name, body));
        }
    }

    HierSpef {
        top_text,
        per_cell,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hier_netlist::extract_hier_netlist;
    use crate::pex::LayerPexParams;
    use crate::pex_hier::extract_pex_hier;
    use klayout_core::{
        Bbox, CellBuilder, Instance, LayerInfo, Library, Point, Rect, Trans, Vec2,
    };

    fn build_lib() -> (Library, klayout_core::CellId) {
        let lib = Library::new("t", 1000);
        let m = lib.layer(LayerInfo::gds(1, 0));
        let mut child = CellBuilder::new("inv");
        child.add_shape(m, Rect::new(Bbox::new(Point::new(0, 0), Point::new(1000, 100))));
        let child_id = lib.insert(child);
        let mut top = CellBuilder::new("top");
        top.add_instance(Instance::new(child_id, Trans::IDENTITY));
        top.add_instance(Instance::new(child_id, Trans::translate(Vec2::new(2000, 0))));
        let top_id = lib.insert(top);
        (lib, top_id)
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
    fn emits_one_top_and_per_child_block() {
        let (lib, top_id) = build_lib();
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));
        let nl = extract_hier_netlist(&lib, top_id, m, lbl);
        let report = extract_pex_hier(&lib, &nl, m, &params());
        let names: HashMapAdapter = HashMapAdapter::from(&lib);
        let hier_spef = write_spef_hier(&report, "top", |id| names.get(id));
        assert!(!hier_spef.top_text.is_empty());
        assert!(!hier_spef.per_cell.is_empty());
        // Every per-cell block has its own *DESIGN line.
        for (cell, text) in &hier_spef.per_cell {
            assert!(
                text.contains(&format!("\"{cell}\"")),
                "per-cell block for {cell} should reference its name"
            );
        }
    }

    #[test]
    fn each_unique_cell_emitted_once() {
        let (lib, top_id) = build_lib();
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));
        let nl = extract_hier_netlist(&lib, top_id, m, lbl);
        let report = extract_pex_hier(&lib, &nl, m, &params());
        let names = HashMapAdapter::from(&lib);
        let hier_spef = write_spef_hier(&report, "top", |id| names.get(id));
        // 2 unique cells (top + inv) → 1 per-child entry plus the top.
        assert_eq!(hier_spef.per_cell.len(), 1);
        assert_eq!(hier_spef.per_cell[0].0.as_str(), "inv");
    }

    /// Helper to build a `CellId → name` lookup the test caller can
    /// pass into write_spef_hier.
    struct HashMapAdapter {
        map: std::collections::HashMap<klayout_core::CellId, SmolStr>,
    }

    impl HashMapAdapter {
        fn from(lib: &Library) -> Self {
            let mut map = std::collections::HashMap::new();
            for (id, cell) in lib.all_cells() {
                map.insert(id, SmolStr::from(cell.name().as_str()));
            }
            Self { map }
        }
        fn get(&self, id: klayout_core::CellId) -> SmolStr {
            self.map.get(&id).cloned().unwrap_or_default()
        }
    }
}
