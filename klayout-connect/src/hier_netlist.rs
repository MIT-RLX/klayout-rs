//! Hierarchical netlist preservation.
//!
//! `extract_flat` and `extract_hierarchical` (in `extract.rs` /
//! `hier.rs`) both flatten the hierarchy into a single `Netlist` —
//! every shape is composed up to the top frame and a single connected-
//! components pass yields nets. That's correct but discards the
//! hierarchy: a 1000-instance standard-cell design produces a 1000×
//! larger netlist than necessary, and downstream tools (SPICE export,
//! hierarchical LVS, partition-based routing) lose the cell boundaries.
//!
//! [`extract_hier_netlist`] preserves the hierarchy: each unique cell
//! gets its own [`CellNetlist`] describing its *local* nets and pin
//! mappings; per-instance pin-to-net bindings record how parent metal
//! routes between child cells.
//!
//! Algorithm (per cell, post-order traversal):
//! 1. Merge local conductor shapes on `layer` → local nets.
//! 2. Match `cell.ports()` against local nets by center-point
//!    containment → `pin_map`.
//! 3. For each instance, transform child port centers into parent
//!    coordinates and check which parent local net contains them →
//!    `pin_to_net`.
//!
//! Connectivity that crosses multiple cells (e.g. parent metal touching
//! grandchild port via through-routing) requires that intermediate
//! cells expose passing-through ports. That's the standard convention
//! in commercial flows and matches how OpenROAD / KLayout treat
//! hierarchical designs.

use klayout_core::{Bbox, CellId, Instance, LayerIndex, Library, Polygon, Shape, Trans};
use klayout_geom::{merge, Region};
use smol_str::SmolStr;
use std::collections::HashMap;

/// One net local to a cell — a merged region of conductor shapes plus
/// optional label-derived name.
#[derive(Clone, Debug)]
pub struct LocalNet {
    pub id: u32,
    pub name: SmolStr,
    pub bbox: Bbox,
    pub polygon: Polygon,
}

/// One placement of a child cell in a parent. `pin_to_net` lists, for
/// each child port that connects to a parent local net, the
/// `(child_port_index, parent_local_net_id)` pair. Child ports with
/// no parent containment are omitted — those connections are the
/// caller's responsibility one level up.
#[derive(Clone, Debug)]
pub struct NetlistInstance {
    pub instance_index: u32,
    pub child_cell: CellId,
    pub trans: Trans,
    pub pin_to_net: Vec<(u32, u32)>,
}

/// Per-cell netlist. `pin_map[i] = (port_index, local_net_id)` records
/// which local net each of *this cell's own* ports is bound to.
#[derive(Clone, Debug)]
pub struct CellNetlist {
    pub cell: CellId,
    pub local_nets: Vec<LocalNet>,
    pub pin_map: Vec<(u32, u32)>,
    pub instances: Vec<NetlistInstance>,
}

#[derive(Clone, Debug)]
pub struct HierNetlist {
    pub top: CellId,
    pub layer: LayerIndex,
    pub cells: HashMap<CellId, CellNetlist>,
}

/// Walk the hierarchy from `top` and build a [`HierNetlist`]. `layer`
/// is the conductor layer to extract on; `label_layer` provides
/// optional net naming labels.
pub fn extract_hier_netlist(
    lib: &Library,
    top: CellId,
    layer: LayerIndex,
    label_layer: LayerIndex,
) -> HierNetlist {
    let mut cells: HashMap<CellId, CellNetlist> = HashMap::new();
    let mut order: Vec<CellId> = Vec::new();
    let mut visited: HashMap<CellId, bool> = HashMap::new();
    visit(lib, top, &mut visited, &mut order);
    for cid in order {
        let cn = build_cell_netlist(lib, cid, layer, label_layer);
        cells.insert(cid, cn);
    }
    HierNetlist {
        top,
        layer,
        cells,
    }
}

fn visit(
    lib: &Library,
    cell_id: CellId,
    visited: &mut HashMap<CellId, bool>,
    order: &mut Vec<CellId>,
) {
    if visited.get(&cell_id).copied().unwrap_or(false) {
        return;
    }
    visited.insert(cell_id, true);
    let cell = lib.get(cell_id);
    for inst in cell.instances() {
        visit(lib, inst.cell, visited, order);
    }
    order.push(cell_id);
}

fn build_cell_netlist(
    lib: &Library,
    cell_id: CellId,
    layer: LayerIndex,
    label_layer: LayerIndex,
) -> CellNetlist {
    let cell = lib.get(cell_id);

    // Step 1: collect local conductor shapes (no instance descent).
    let mut polys: Vec<Polygon> = Vec::new();
    for shape in cell.shapes_on(layer) {
        match shape {
            Shape::Polygon(p) => polys.push(p.clone()),
            Shape::Box(r) => polys.push(Polygon::rect(r.bbox)),
            _ => {}
        }
    }
    let merged = if polys.is_empty() {
        Region::empty()
    } else {
        merge(&Region::from_polygons(polys))
    };

    // Step 2: collect labels on label_layer for net-name attribution.
    let labels: Vec<(SmolStr, klayout_core::Point)> = cell
        .shapes_on(label_layer)
        .filter_map(|s| match s {
            Shape::Text(t) => Some((t.string.clone(), t.anchor)),
            _ => None,
        })
        .collect();

    // Step 3: build LocalNets, attaching label names where they fall.
    let mut local_nets: Vec<LocalNet> = merged
        .polygons()
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let bbox = p.bbox();
            let name = labels
                .iter()
                .find(|(_, pt)| bbox.contains(*pt))
                .map(|(s, _)| s.clone())
                .unwrap_or_else(|| SmolStr::from(format!("net_{i}")));
            LocalNet {
                id: i as u32,
                name,
                bbox,
                polygon: p.clone(),
            }
        })
        .collect();
    let _ = &mut local_nets; // mut binding kept for future label-promotion

    // Step 4: match this cell's own ports against local nets.
    let mut pin_map: Vec<(u32, u32)> = Vec::new();
    for (port_idx, port) in cell.ports().iter().enumerate() {
        if port.layer != layer {
            continue;
        }
        if let Some(net) = local_nets
            .iter()
            .find(|n| n.bbox.contains(port.center))
        {
            pin_map.push((port_idx as u32, net.id));
        }
    }

    // Step 5: per-instance pin-to-net mapping.
    let mut instances: Vec<NetlistInstance> = Vec::new();
    for (inst_idx, inst) in cell.instances().iter().enumerate() {
        for placement in expand_inst(inst) {
            let pin_to_net = pin_to_net_for_instance(lib, inst.cell, placement, layer, &local_nets);
            instances.push(NetlistInstance {
                instance_index: inst_idx as u32,
                child_cell: inst.cell,
                trans: placement,
                pin_to_net,
            });
        }
    }

    CellNetlist {
        cell: cell_id,
        local_nets,
        pin_map,
        instances,
    }
}

/// For each port on `child_cell` that lies on `layer` once transformed
/// by `trans`, find the parent local net (if any) that contains it.
fn pin_to_net_for_instance(
    lib: &Library,
    child_cell: CellId,
    trans: Trans,
    layer: LayerIndex,
    parent_nets: &[LocalNet],
) -> Vec<(u32, u32)> {
    let child = lib.get(child_cell);
    let mut out = Vec::new();
    for (port_idx, port) in child.ports().iter().enumerate() {
        if port.layer != layer {
            continue;
        }
        let placed = trans.apply(port.center);
        if let Some(net) = parent_nets.iter().find(|n| n.bbox.contains(placed)) {
            out.push((port_idx as u32, net.id));
        }
    }
    out
}

fn expand_inst(inst: &Instance) -> Vec<Trans> {
    use klayout_core::Repetition;
    use klayout_core::Vec2;
    match &inst.repetition {
        None => vec![inst.trans],
        Some(Repetition::Regular {
            col,
            row,
            n_cols,
            n_rows,
        }) => {
            let mut out = Vec::with_capacity((*n_cols as usize) * (*n_rows as usize));
            for j in 0..*n_rows {
                for i in 0..*n_cols {
                    let extra = Trans::translate(Vec2::new(
                        col.x * i as i64 + row.x * j as i64,
                        col.y * i as i64 + row.y * j as i64,
                    ));
                    out.push(extra.compose(inst.trans));
                }
            }
            out
        }
        Some(Repetition::Irregular { offsets }) => offsets
            .iter()
            .map(|o| Trans::translate(*o).compose(inst.trans))
            .collect(),
    }
}

impl HierNetlist {
    pub fn cell(&self, id: CellId) -> Option<&CellNetlist> {
        self.cells.get(&id)
    }

    pub fn top_cell(&self) -> &CellNetlist {
        // Invariant: `self.top` is inserted into `self.cells` by
        // `HierNetlist::extract` before the value is returned to the
        // caller, so this lookup cannot fail unless that invariant is
        // violated.
        self.cells
            .get(&self.top)
            .expect("HierNetlist invariant: top CellId is always a key in cells")
    }

    /// Total number of nets across all cells (sum of local_nets.len()).
    /// Useful as a size metric vs. flat extraction.
    pub fn total_local_nets(&self) -> usize {
        self.cells.values().map(|c| c.local_nets.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::{
        Angle90, Bbox as Bb, CellBuilder, LayerInfo, Library, Point as P, Port, Rect, Trans,
        Vec2,
    };

    #[test]
    fn single_cell_two_local_nets() {
        let lib = Library::new("t", 1);
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));
        let mut cb = CellBuilder::new("top");
        cb.add_shape(m, Rect::new(Bb::new(P::new(0, 0), P::new(10, 10))));
        cb.add_shape(m, Rect::new(Bb::new(P::new(50, 0), P::new(60, 10))));
        let id = lib.insert(cb);

        let h = extract_hier_netlist(&lib, id, m, lbl);
        let top = h.top_cell();
        assert_eq!(top.local_nets.len(), 2);
        assert!(top.instances.is_empty());
    }

    #[test]
    fn instance_port_resolves_to_parent_net() {
        let lib = Library::new("t", 1);
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));

        // Child has one shape + a port on it.
        let mut child = CellBuilder::new("child");
        child.add_shape(m, Rect::new(Bb::new(P::new(0, 0), P::new(10, 10))));
        child.add_port(Port::new("A", m, P::new(5, 5), Angle90::E, 1));
        let child_id = lib.insert(child);

        // Parent places child twice and connects them with a metal strap.
        let mut parent = CellBuilder::new("top");
        parent.add_instance(Instance::new(child_id, Trans::IDENTITY));
        parent.add_instance(Instance::new(
            child_id,
            Trans::translate(Vec2::new(50, 0)),
        ));
        parent.add_shape(m, Rect::new(Bb::new(P::new(0, 0), P::new(60, 10))));
        let top_id = lib.insert(parent);

        let h = extract_hier_netlist(&lib, top_id, m, lbl);
        let top = h.top_cell();
        // Parent's strap merges with both child instances' shapes (they
        // overlap), so there's exactly one local net.
        assert_eq!(top.local_nets.len(), 1);
        // Both instances' port "A" maps to that net.
        assert_eq!(top.instances.len(), 2);
        for inst in &top.instances {
            assert_eq!(inst.pin_to_net.len(), 1);
            let (port_idx, net_id) = inst.pin_to_net[0];
            assert_eq!(port_idx, 0);
            assert_eq!(net_id, 0);
        }
    }

    #[test]
    fn unique_cell_processed_once_in_cells_map() {
        let lib = Library::new("t", 1);
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));
        let mut child = CellBuilder::new("c");
        child.add_shape(m, Rect::new(Bb::new(P::new(0, 0), P::new(5, 5))));
        let cid = lib.insert(child);
        let mut top = CellBuilder::new("t");
        for i in 0..5 {
            top.add_instance(Instance::new(
                cid,
                Trans::translate(Vec2::new(i * 100, 0)),
            ));
        }
        let tid = lib.insert(top);
        let h = extract_hier_netlist(&lib, tid, m, lbl);
        // Two unique cells (top + child); flat extraction would have 5
        // child instances' nets duplicated.
        assert_eq!(h.cells.len(), 2);
        assert_eq!(h.cell(cid).unwrap().local_nets.len(), 1);
    }

    #[test]
    fn label_attaches_name_to_local_net() {
        let lib = Library::new("t", 1);
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));
        let mut cb = CellBuilder::new("top");
        cb.add_shape(m, Rect::new(Bb::new(P::new(0, 0), P::new(10, 10))));
        cb.add_shape(
            lbl,
            klayout_core::Text::new("VDD", P::new(5, 5)),
        );
        let id = lib.insert(cb);
        let h = extract_hier_netlist(&lib, id, m, lbl);
        let top = h.top_cell();
        assert_eq!(top.local_nets.len(), 1);
        assert_eq!(top.local_nets[0].name.as_str(), "VDD");
    }

    #[test]
    fn pin_map_links_cell_port_to_net() {
        let lib = Library::new("t", 1);
        let m = lib.layer(LayerInfo::gds(1, 0));
        let lbl = lib.layer(LayerInfo::gds(2, 0));
        let mut cb = CellBuilder::new("c");
        cb.add_shape(m, Rect::new(Bb::new(P::new(0, 0), P::new(10, 10))));
        cb.add_port(Port::new("Y", m, P::new(5, 5), Angle90::E, 1));
        let id = lib.insert(cb);
        let h = extract_hier_netlist(&lib, id, m, lbl);
        let top = h.top_cell();
        assert_eq!(top.pin_map.len(), 1);
        assert_eq!(top.pin_map[0], (0, 0));
    }
}
