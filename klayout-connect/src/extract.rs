//! Connectivity extraction.
//!
//! v1 covers the basic flow:
//! 1. Walk a cell's shapes on a single conducting layer.
//! 2. Group connected (touching/overlapping) polygons via `Region::merge`.
//! 3. Each merged piece becomes a `Net`.
//! 4. For each text label on the same (or paired) layer that lies on or
//!    inside a piece, attach the label as the net name.
//!
//! Out of scope for v1: cross-layer via stitching, hierarchical traversal
//! that descends into instances, multi-conductor antenna tracking. Those
//! sit on top of this primitive.

use crate::netlist::{Net, NetId, Netlist, ShapeRef};
use klayout_core::{Bbox, CellId, LayerIndex, Library, Shape};
use klayout_geom::Region;
use smol_str::SmolStr;

/// Extract a flat netlist from a cell's shapes on `conductor_layer`.
/// Labels on `label_layer` (often the same layer) name the nets they sit on.
/// Unnamed nets get an auto-generated name.
pub fn extract_flat(
    lib: &Library,
    cell: CellId,
    conductor_layer: LayerIndex,
    label_layer: LayerIndex,
) -> Netlist {
    let mut polys: Vec<klayout_core::Polygon> = Vec::new();
    let mut shape_refs: Vec<ShapeRef> = Vec::new();

    let cell_arc = lib.get(cell);
    for (shape_index, shape) in cell_arc.shapes_on(conductor_layer).enumerate() {
        match shape {
            Shape::Polygon(p) => {
                polys.push(p.clone());
                shape_refs.push(ShapeRef {
                    cell,
                    layer: conductor_layer,
                    index: shape_index as u32,
                });
            }
            Shape::Box(r) => {
                polys.push(klayout_core::Polygon::rect(r.bbox));
                shape_refs.push(ShapeRef {
                    cell,
                    layer: conductor_layer,
                    index: shape_index as u32,
                });
            }
            _ => {}
        }
    }

    if polys.is_empty() {
        return Netlist::new();
    }

    // Merge into connected components. After merge, each polygon in the
    // result represents one electrical net.
    let merged = klayout_geom::merge(&Region::from_polygons(polys.clone()));

    // Collect labels (text shapes) on label_layer.
    let labels: Vec<(SmolStr, klayout_core::Point)> = cell_arc
        .shapes_on(label_layer)
        .filter_map(|s| match s {
            Shape::Text(t) => Some((t.string.clone(), t.anchor)),
            _ => None,
        })
        .collect();

    let mut netlist = Netlist::new();
    for (i, merged_poly) in merged.polygons().iter().enumerate() {
        let bbox = merged_poly.bbox();
        // Find a label inside this net's bbox.
        let mut name: Option<SmolStr> = None;
        for (lbl, pt) in &labels {
            if bbox.contains(*pt) {
                name = Some(lbl.clone());
                break;
            }
        }
        let net_name = name.unwrap_or_else(|| SmolStr::from(format!("net_{i}")));
        let mut net = Net::new(NetId(0), net_name);
        net.bbox = bbox;
        net.shapes = shapes_in_bbox(&shape_refs, &polys, bbox);
        netlist.insert(net);
    }
    netlist
}

fn shapes_in_bbox(
    refs: &[ShapeRef],
    polys: &[klayout_core::Polygon],
    bbox: Bbox,
) -> Vec<ShapeRef> {
    polys
        .iter()
        .zip(refs)
        .filter_map(|(p, r)| {
            let pb = p.bbox();
            if bbox.intersects(&pb) {
                Some(*r)
            } else {
                None
            }
        })
        .collect()
}
