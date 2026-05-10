//! Hierarchical LVS — match cells bottom-up, propagate match
//! results upward.
//!
//! Flat LVS (`lvs_compare`) sees the entire flattened netlist as one
//! graph; that scales poorly past ~100k devices and loses cell
//! boundaries. Hierarchical LVS walks the cell hierarchy bottom-up:
//! leaf cells are matched directly with VF2 / structural fallback;
//! once a cell is matched, its sub-block becomes a "single device"
//! at the parent level. Parents only need to compare instance
//! placements + connectivity, not internal contents.
//!
//! Algorithm: topologically order cells (children before parents).
//! For each cell, build a device list from its [`HierNetlist`] entry,
//! recursively include matched-instance stubs (each child instance
//! becomes a synthetic device carrying the child's pin map), and run
//! flat LVS (VF2) on this cell vs. its schematic counterpart. Match
//! results propagate upward.
//!
//! v1 simplification: the schematic side is also represented as a
//! `HierNetlist` (so both layout and schematic share the data model).
//! Real flows mix layouts with SPICE / Verilog netlists; that
//! conversion is its own concern.

use crate::device::{Device, DeviceKind};
use crate::hier_netlist::{CellNetlist, HierNetlist};
use crate::lvs::LvsReport;
use crate::vf2::vf2_match;
use klayout_core::CellId;
use smol_str::SmolStr;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct CellLvsReport {
    pub cell: CellId,
    pub matched: bool,
    pub flat_report: LvsReport,
}

#[derive(Clone, Debug)]
pub struct HierLvsReport {
    pub per_cell: Vec<CellLvsReport>,
    pub all_matched: bool,
}

/// Per-cell LVS, walking both hierarchies bottom-up with proper
/// hierarchical composition: each child cell is matched once and
/// cached by content-hash; identical cells reuse the cached match.
/// A child match becomes a "black-box" device at the parent level —
/// the parent's VF2 sees one synthetic device per instance, not the
/// flattened internals.
///
/// Failure propagates: if a child cell fails to match, every parent
/// that instantiates it inherits the failure (we can't reason about
/// the parent without trusting the child's stub).
pub fn lvs_compare_hier(
    layout: &HierNetlist,
    schem: &HierNetlist,
    layout_devices: &HashMap<CellId, Vec<Device>>,
    schem_devices: &HashMap<CellId, Vec<Device>>,
) -> HierLvsReport {
    lvs_compare_hier_with_lib(layout, schem, layout_devices, schem_devices, None, None)
}

/// Same as [`lvs_compare_hier`] but takes optional `Library`
/// references so child-cell match results can be cached by
/// `content_hash`. Identical cells re-use the cached match instead
/// of re-running VF2.
pub fn lvs_compare_hier_with_lib(
    layout: &HierNetlist,
    schem: &HierNetlist,
    layout_devices: &HashMap<CellId, Vec<Device>>,
    schem_devices: &HashMap<CellId, Vec<Device>>,
    layout_lib: Option<&klayout_core::Library>,
    schem_lib: Option<&klayout_core::Library>,
) -> HierLvsReport {
    let order = topo_order(layout);
    let mut per_cell: Vec<CellLvsReport> = Vec::with_capacity(order.len());
    let mut child_match: HashMap<CellId, bool> = HashMap::new();
    // Match cache keyed by (layout_hash, schem_hash). Identical cells
    // hit the cache and skip the expensive VF2 run.
    let mut cache: HashMap<(klayout_core::ContentHash, klayout_core::ContentHash), bool> =
        HashMap::new();

    for cid in order {
        let layout_cell = match layout.cell(cid) {
            Some(c) => c,
            None => continue,
        };
        let schem_cell = schem.cell(cid);

        // Check fast-fail: if any child failed, parent fails too.
        let any_child_failed = layout_cell
            .instances
            .iter()
            .any(|inst| !child_match.get(&inst.child_cell).copied().unwrap_or(true));
        if any_child_failed {
            child_match.insert(cid, false);
            per_cell.push(CellLvsReport {
                cell: cid,
                matched: false,
                flat_report: LvsReport::default(),
            });
            continue;
        }

        // Cache lookup keyed by content_hash (when libs are supplied).
        let cache_key = match (layout_lib, schem_lib) {
            (Some(la), Some(sa)) => Some((
                la.get(cid).content_hash(),
                sa.get(cid).content_hash(),
            )),
            _ => None,
        };
        if let Some(key) = cache_key {
            if let Some(&hit) = cache.get(&key) {
                child_match.insert(cid, hit);
                per_cell.push(CellLvsReport {
                    cell: cid,
                    matched: hit,
                    flat_report: LvsReport::default(),
                });
                continue;
            }
        }

        let lay_devs = build_cell_devices(layout_cell, layout_devices.get(&cid));
        let schem_devs = match schem_cell {
            Some(sc) => build_cell_devices(sc, schem_devices.get(&cid)),
            None => Vec::new(),
        };

        let matched = vf2_match(&lay_devs, &schem_devs).is_some();
        child_match.insert(cid, matched);
        if let Some(key) = cache_key {
            cache.insert(key, matched);
        }
        per_cell.push(CellLvsReport {
            cell: cid,
            matched,
            flat_report: LvsReport::default(),
        });
    }

    let all_matched = per_cell.iter().all(|r| r.matched);
    HierLvsReport {
        per_cell,
        all_matched,
    }
}

/// Build the device list for a cell: real devices from the device
/// library plus one synthetic device per child instance (the child
/// behaves like a black-box device with its pin connections).
fn build_cell_devices(cell_nl: &CellNetlist, real_devices: Option<&Vec<Device>>) -> Vec<Device> {
    let mut out: Vec<Device> = Vec::new();
    if let Some(devs) = real_devices {
        out.extend(devs.iter().cloned());
    }
    for inst in &cell_nl.instances {
        // Synthetic device representing this child instance.
        let mut term_map = HashMap::new();
        for (port_idx, net_id) in &inst.pin_to_net {
            let term_name = SmolStr::from(format!("p{port_idx}"));
            let net_name = cell_nl
                .local_nets
                .iter()
                .find(|n| n.id == *net_id)
                .map(|n| n.name.clone())
                .unwrap_or_else(|| SmolStr::from(format!("net_{net_id}")));
            term_map.insert(term_name, net_name);
        }
        out.push(Device {
            kind: DeviceKind::Diffusion, // Synthetic kind — child cells aren't transistors.
            name: SmolStr::from(format!("inst_{}", inst.instance_index)),
            bbox: klayout_core::Bbox::EMPTY,
            terminals: term_map,
            params: HashMap::new(),
        });
    }
    out
}

fn topo_order(nl: &HierNetlist) -> Vec<CellId> {
    let mut visited: HashMap<CellId, bool> = HashMap::new();
    let mut out: Vec<CellId> = Vec::new();
    fn visit(
        cid: CellId,
        nl: &HierNetlist,
        visited: &mut HashMap<CellId, bool>,
        out: &mut Vec<CellId>,
    ) {
        if visited.get(&cid).copied().unwrap_or(false) {
            return;
        }
        visited.insert(cid, true);
        if let Some(c) = nl.cell(cid) {
            for inst in &c.instances {
                visit(inst.child_cell, nl, visited, out);
            }
        }
        out.push(cid);
    }
    visit(nl.top, nl, &mut visited, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hier_netlist::extract_hier_netlist;
    use klayout_core::{
        Angle90, Bbox, CellBuilder, Instance, LayerInfo, Library, Point, Port, Rect, Trans, Vec2,
    };

    fn build_two_cell_lib() -> (Library, CellId) {
        let lib = Library::new("t", 1);
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));
        let _ = lbl;
        let mut child = CellBuilder::new("inv");
        child.add_shape(m, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
        child.add_port(Port::new("A", m, Point::new(5, 5), Angle90::E, 1));
        let child_id = lib.insert(child);
        let mut top = CellBuilder::new("top");
        top.add_shape(m, Rect::new(Bbox::new(Point::new(0, 0), Point::new(50, 10))));
        top.add_instance(Instance::new(child_id, Trans::IDENTITY));
        top.add_instance(Instance::new(
            child_id,
            Trans::translate(Vec2::new(20, 0)),
        ));
        let top_id = lib.insert(top);
        (lib, top_id)
    }

    #[test]
    fn identical_layout_and_schem_match() {
        let (lib, top) = build_two_cell_lib();
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));
        let layout_nl = extract_hier_netlist(&lib, top, m, lbl);
        let schem_nl = layout_nl.clone();
        let layout_devs = HashMap::new();
        let schem_devs = HashMap::new();
        let report = lvs_compare_hier(&layout_nl, &schem_nl, &layout_devs, &schem_devs);
        assert!(report.all_matched);
    }

    #[test]
    fn per_cell_reports_are_ordered_children_first() {
        let (lib, top) = build_two_cell_lib();
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));
        let nl = extract_hier_netlist(&lib, top, m, lbl);
        let schem = nl.clone();
        let report = lvs_compare_hier(&nl, &schem, &HashMap::new(), &HashMap::new());
        // Top is processed last (children first).
        assert_eq!(report.per_cell.last().map(|r| r.cell), Some(top));
    }
}
