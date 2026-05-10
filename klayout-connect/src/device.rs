//! Device extraction — recognize circuit elements from layout geometry.
//!
//! v1 covers MOSFETs (the core layout pattern: poly stripe crossing diff
//! = transistor gate). Algorithm:
//!
//! 1. `gates = poly ∩ diff` — each connected piece is one transistor gate.
//! 2. `sd_region = diff − gates` — each connected piece is one S/D node.
//! 3. For each gate piece, find adjacent S/D pieces (bbox edge-touch).
//! 4. Look up net names by inspecting text labels whose anchor is inside
//!    each piece's bbox; un-labeled pieces get an auto-generated name.
//!
//! Devices reference nets by **name**, not `NetId` — names are stable
//! across separate netlist extractions and don't require a `Netlist`
//! argument to interpret. Hook them up with `Netlist::by_name(...)` if
//! needed downstream.

use klayout_core::{Bbox, CellId, LayerIndex, Library, Shape};
use klayout_geom::{difference, intersection, merge, Region};
use smol_str::SmolStr;
use std::collections::HashMap;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Nmos,
    Pmos,
    PolyResistor,
    Diffusion,
    MimCap,
}

#[derive(Clone, Debug)]
pub struct Device {
    pub kind: DeviceKind,
    pub name: SmolStr,
    pub bbox: Bbox,
    /// terminal name → net name. MOS terminals are `gate`, `source`, `drain`.
    pub terminals: HashMap<SmolStr, SmolStr>,
    pub params: HashMap<SmolStr, f64>,
}

impl Device {
    pub fn new(kind: DeviceKind, name: impl Into<SmolStr>, bbox: Bbox) -> Self {
        Self {
            kind,
            name: name.into(),
            bbox,
            terminals: HashMap::new(),
            params: HashMap::new(),
        }
    }
}

/// Layer assignments for MOS extraction.
#[derive(Copy, Clone, Debug)]
pub struct MosLayers {
    pub poly: LayerIndex,
    pub diff: LayerIndex,
    /// Optional implant — distinguishes NMOS from PMOS. If `None`, all
    /// MOS devices are reported as NMOS.
    pub nwell: Option<LayerIndex>,
    /// Layer where text labels live (for net naming).
    pub label: LayerIndex,
}

/// Extract MOSFETs from a flat layout. Each gate piece becomes one
/// `Device`; multi-finger transistors report each finger separately.
/// Net names come from labels whose anchor lies inside the relevant
/// region; un-labeled regions get `net_<n>` auto-names.
pub fn extract_mos(
    lib: &Library,
    cell: CellId,
    layers: MosLayers,
) -> Vec<Device> {
    let cell_arc = lib.get(cell);
    let poly = collect_polygons_on(&cell_arc, layers.poly);
    let diff = collect_polygons_on(&cell_arc, layers.diff);
    if poly.is_empty() || diff.is_empty() {
        return Vec::new();
    }
    let nwell = layers
        .nwell
        .map(|l| collect_polygons_on(&cell_arc, l))
        .unwrap_or_else(Region::empty);

    let gates = merge(&intersection(&poly, &diff));
    if gates.is_empty() {
        return Vec::new();
    }
    let sd_region = merge(&difference(&diff, &gates));
    let poly_nets = merge(&difference(&poly, &diff));

    // Collect labels from the layout for net naming.
    let labels = collect_labels(&cell_arc, layers.label);

    let mut sd_pieces: Vec<(Bbox, SmolStr)> = Vec::new();
    let mut auto_n = 0u32;
    for piece in sd_region.polygons() {
        let pb = piece.bbox();
        let name = labels
            .iter()
            .find(|(_, anchor)| pb.contains(*anchor))
            .map(|(s, _)| s.clone())
            .unwrap_or_else(|| {
                let n = auto_n;
                auto_n += 1;
                SmolStr::from(format!("sd_{n}"))
            });
        sd_pieces.push((pb, name));
    }

    // Poly nets (the parts of poly outside gates) — used for naming the
    // gate terminal of each device.
    let mut poly_pieces: Vec<(Bbox, SmolStr)> = Vec::new();
    let mut auto_pn = 0u32;
    for piece in poly_nets.polygons() {
        let pb = piece.bbox();
        let name = labels
            .iter()
            .find(|(_, anchor)| pb.contains(*anchor))
            .map(|(s, _)| s.clone())
            .unwrap_or_else(|| {
                let n = auto_pn;
                auto_pn += 1;
                SmolStr::from(format!("g_{n}"))
            });
        poly_pieces.push((pb, name));
    }

    let mut devices: Vec<Device> = Vec::new();
    for (i, gate) in gates.polygons().iter().enumerate() {
        let gate_bbox = gate.bbox();
        let length = gate_bbox.width().min(gate_bbox.height());
        let width = gate_bbox.width().max(gate_bbox.height());

        let kind = if !nwell.is_empty()
            && !intersection(
                &Region::from_polygons([gate.clone()]),
                &nwell,
            )
            .is_empty()
        {
            DeviceKind::Pmos
        } else {
            DeviceKind::Nmos
        };

        let mut device = Device::new(kind, format!("M{i}"), gate_bbox);
        device.params.insert(SmolStr::from("l"), length as f64);
        device.params.insert(SmolStr::from("w"), width as f64);

        // Gate net: find a poly piece whose bbox abuts (or contains) the gate.
        // The gate region is part of the poly; the abutting non-gate poly
        // piece is the same electrical net.
        let gate_net = poly_pieces
            .iter()
            .find(|(pb, _)| abuts(*pb, gate_bbox) || pb.intersects(&gate_bbox))
            .map(|(_, name)| name.clone());
        if let Some(name) = gate_net {
            device.terminals.insert(SmolStr::from("gate"), name);
        }

        // S/D: up to two adjacent SD pieces.
        let mut sd_seen: Vec<&SmolStr> = Vec::new();
        for (pb, name) in &sd_pieces {
            if abuts(*pb, gate_bbox) && !sd_seen.contains(&name) {
                let term = if sd_seen.is_empty() { "source" } else { "drain" };
                device
                    .terminals
                    .insert(SmolStr::from(term), name.clone());
                sd_seen.push(name);
                if sd_seen.len() == 2 {
                    break;
                }
            }
        }

        devices.push(device);
    }
    devices
}

fn collect_polygons_on(cell: &klayout_core::Cell, layer: LayerIndex) -> Region {
    let mut polys: Vec<klayout_core::Polygon> = Vec::new();
    for shape in cell.shapes_on(layer) {
        match shape {
            Shape::Polygon(p) => polys.push(p.clone()),
            Shape::Box(r) => polys.push(klayout_core::Polygon::rect(r.bbox)),
            _ => {}
        }
    }
    Region::from_polygons(polys)
}

fn collect_labels(
    cell: &klayout_core::Cell,
    layer: LayerIndex,
) -> Vec<(SmolStr, klayout_core::Point)> {
    let mut out = Vec::new();
    for shape in cell.shapes_on(layer) {
        if let Shape::Text(t) = shape {
            out.push((t.string.clone(), t.anchor));
        }
    }
    out
}

fn abuts(a: Bbox, b: Bbox) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let touches_x = a.max.x == b.min.x || b.max.x == a.min.x;
    let touches_y = a.max.y == b.min.y || b.max.y == a.min.y;
    let perp_x_overlap = a.min.x.max(b.min.x) < a.max.x.min(b.max.x);
    let perp_y_overlap = a.min.y.max(b.min.y) < a.max.y.min(b.max.y);
    (touches_x && perp_y_overlap) || (touches_y && perp_x_overlap)
}
