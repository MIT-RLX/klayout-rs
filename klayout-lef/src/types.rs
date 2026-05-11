//! Rich types for LEF/DEF metadata that doesn't fit cleanly into the
//! `klayout-core` data model.
//!
//! `Library` carries shapes and instances; LEF/DEF carry additional
//! metadata (layer rules, placement rows, routing tracks, top-level
//! pins, blockages, etc.) that doesn't have a natural home in core.
//! These types side-bind that metadata to a parsed file.

use klayout_core::{Bbox, LayerIndex, Point};
use smol_str::SmolStr;
use std::collections::HashMap;

// ====================================================================
// LEF: layer / via / site metadata
// ====================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LayerType {
    Routing,
    Cut,
    Masterslice,
    Overlap,
    Implant,
    Other,
}

#[derive(Clone, Debug, Default)]
pub struct LayerSpec {
    pub name: SmolStr,
    pub gds_layer: Option<u16>,
    pub gds_datatype: Option<u16>,
    pub layer_type: Option<LayerType>,
    pub direction: Option<RoutingDirection>,
    pub width: Option<f64>,       // micrometers
    pub pitch: Option<f64>,
    pub offset: Option<f64>,
    pub spacing: Vec<SpacingRule>,
    pub min_area: Option<f64>,
    pub min_step: Option<f64>,
    pub minimum_cut: Option<f64>,
    pub edge_capacitance: Option<f64>,
    pub resistance_per_sq: Option<f64>,
    pub capacitance_per_sq: Option<f64>,
    pub antenna_diff_area_factor: Option<f64>,
    pub antenna_metal_area_factor: Option<f64>,
    pub thickness: Option<f64>,
    pub max_via_stack: Option<u32>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RoutingDirection {
    Horizontal,
    Vertical,
    Diag45,
    Diag135,
}

#[derive(Clone, Debug)]
pub struct SpacingRule {
    pub min_spacing: f64,
    pub same_net: bool,
    pub range_min: Option<f64>,
    pub range_max: Option<f64>,
}

/// One LEF VIA definition. The via geometry is stored as `(layer, bbox)`
/// pairs. Generated/PARAM vias (via VIARULE) are not yet supported.
#[derive(Clone, Debug, Default)]
pub struct ViaSpec {
    pub name: SmolStr,
    pub default: bool,
    pub resistance: Option<f64>,
    pub shapes: Vec<ViaShape>,
    pub via_rule: Option<SmolStr>,
    pub cut_size: Option<(f64, f64)>,
    pub layers: Option<(SmolStr, SmolStr, SmolStr)>,
}

#[derive(Clone, Debug)]
pub struct ViaShape {
    pub layer: SmolStr,
    pub bbox: Bbox,
}

#[derive(Clone, Debug, Default)]
pub struct SiteSpec {
    pub name: SmolStr,
    pub class: Option<SmolStr>,
    pub size: Option<(f64, f64)>,
    pub symmetry: Vec<SmolStr>, // X, Y, R90
    pub row_pattern: Vec<(SmolStr, SmolStr)>, // (orientation, site)
}

// ====================================================================
// LEF: macro-level rich pin metadata
// ====================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PinDirection {
    Input,
    Output,
    Inout,
    Feedthru,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PinUse {
    Signal,
    Power,
    Ground,
    Clock,
    Analog,
    Reset,
    Tieoff,
    Scan,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PinShape {
    Abutment,
    Ring,
    Feedthru,
}

/// A pin port — one or more layered shapes that constitute a connection
/// region.
#[derive(Clone, Debug, Default)]
pub struct PinGeom {
    pub shapes: Vec<(SmolStr, PortShape)>,
}

#[derive(Clone, Debug)]
pub enum PortShape {
    Rect(Bbox),
    Polygon(Vec<Point>),
}

#[derive(Clone, Debug, Default)]
pub struct PinSpec {
    pub name: SmolStr,
    pub direction: Option<PinDirection>,
    pub use_: Option<PinUse>,
    pub shape: Option<PinShape>,
    pub geometry: PinGeom,
    pub antenna_gate_area: Option<f64>,
    pub antenna_diff_area: Option<f64>,
    /// If this pin is a bus member (e.g. "DATA[3]"), the original
    /// bus declaration. `None` for scalar pins.
    pub bus: Option<klayout_core::BusName>,
}

#[derive(Clone, Debug, Default)]
pub struct MacroSpec {
    pub name: SmolStr,
    pub class: Option<SmolStr>,
    pub size: Option<(f64, f64)>,
    pub origin: Option<(f64, f64)>,
    pub symmetry: Vec<SmolStr>,
    pub site: Option<SmolStr>,
    pub foreign: Option<(SmolStr, f64, f64)>,
    pub pins: Vec<PinSpec>,
    pub obs: Vec<(SmolStr, PortShape)>,
}

// ====================================================================
// Top-level read/write result types
// ====================================================================

pub struct LefLibrary {
    pub library: klayout_core::Library,
    pub version: Option<f64>,
    pub bus_bit_chars: Option<SmolStr>,
    pub divider_char: Option<SmolStr>,
    pub manufacturing_grid: Option<f64>,
    pub layers: Vec<LayerSpec>,
    pub vias: Vec<ViaSpec>,
    pub sites: Vec<SiteSpec>,
    pub macros: Vec<MacroSpec>,
}

impl LefLibrary {
    /// LEF `LAYER … WIDTH <µm>` as default routing width in DBU for NETS segments
    /// that omit an explicit width (matches KLayout / LEFDEF 5.8 default width).
    pub fn routing_width_dbu(&self, dbu_per_micron: i64) -> HashMap<SmolStr, i64> {
        let scale = dbu_per_micron as f64;
        let mut m = HashMap::new();
        for layer in &self.layers {
            if let Some(w_um) = layer.width {
                let dbu = (w_um * scale).round() as i64;
                m.insert(layer.name.clone(), dbu);
            }
        }
        m
    }
}

// ====================================================================
// DEF: design-level metadata
// ====================================================================

#[derive(Clone, Debug, Default)]
pub struct Row {
    pub name: SmolStr,
    pub site: SmolStr,
    pub origin: (i64, i64),
    pub orient: SmolStr,
    pub num_x: u32,
    pub num_y: u32,
    pub step_x: i64,
    pub step_y: i64,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub direction: TrackDirection,
    pub start: i64,
    pub num_tracks: u32,
    pub step: i64,
    pub layers: Vec<SmolStr>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TrackDirection {
    X,
    Y,
}

#[derive(Clone, Debug)]
pub struct GcellGrid {
    pub direction: TrackDirection,
    pub start: i64,
    pub num: u32,
    pub step: i64,
}

#[derive(Clone, Debug, Default)]
pub struct DesignPin {
    pub name: SmolStr,
    pub net: SmolStr,
    pub direction: Option<PinDirection>,
    pub use_: Option<PinUse>,
    pub layer: Option<SmolStr>,
    pub layer_bbox: Option<Bbox>,
    pub placed: Option<(i64, i64)>,
    pub orient: Option<SmolStr>,
    pub fixed: bool,
    /// Bus declaration if this pin name was a bus (e.g. `IN[3:0]`).
    pub bus: Option<klayout_core::BusName>,
}

#[derive(Clone, Debug, Default)]
pub struct RouteNet {
    pub name: SmolStr,
    pub use_: Option<PinUse>,
    pub connects: Vec<NetConnect>,
    pub segments: Vec<RouteSegment>,
    pub special: bool,
}

/// Connection point: an instance pin reference, or a top-level PIN.
#[derive(Clone, Debug)]
pub struct NetConnect {
    pub instance: Option<SmolStr>,
    pub pin: SmolStr,
}

#[derive(Clone, Debug)]
pub enum RouteSegment {
    /// Wire on a routing layer between two points (or a polyline).
    Wire {
        layer: SmolStr,
        points: Vec<Point>,
        width: Option<i64>,
    },
    /// A via at a point (named via from a LEF VIA def or a per-design
    /// via in DEF VIAS).
    Via {
        via_name: SmolStr,
        at: Point,
    },
    /// A power/ground stripe — `RECT` block in SPECIALNETS.
    Rect {
        layer: SmolStr,
        bbox: Bbox,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BlockageKind {
    /// PLACEMENT blockage — no cells may go here.
    Placement,
    /// ROUTING blockage on a specific layer.
    Routing,
}

#[derive(Clone, Debug)]
pub struct Blockage {
    pub kind: BlockageKind,
    pub layer: Option<SmolStr>,
    pub bboxes: Vec<Bbox>,
    pub component: Option<SmolStr>,
}

#[derive(Clone, Debug, Default)]
pub struct Region {
    pub name: SmolStr,
    pub bboxes: Vec<Bbox>,
    pub kind: Option<SmolStr>, // FENCE, GUIDE
}

#[derive(Clone, Debug, Default)]
pub struct Group {
    pub name: SmolStr,
    pub region: Option<SmolStr>,
    pub members: Vec<SmolStr>,
}

#[derive(Clone, Debug, Default)]
pub struct DefVia {
    pub name: SmolStr,
    pub via_rule: Option<SmolStr>,
    pub shapes: Vec<ViaShape>,
}

#[derive(Default)]
pub struct DefDesign {
    pub top: Option<klayout_core::CellId>,
    pub design_name: SmolStr,
    pub version: Option<f64>,
    pub bus_bit_chars: Option<SmolStr>,
    pub divider_char: Option<SmolStr>,
    pub units_dbu_per_micron: i64,
    pub diearea: Option<Bbox>,
    pub rows: Vec<Row>,
    pub tracks: Vec<Track>,
    pub gcell_grids: Vec<GcellGrid>,
    pub pins: Vec<DesignPin>,
    pub nets: Vec<RouteNet>,
    pub special_nets: Vec<RouteNet>,
    pub blockages: Vec<Blockage>,
    pub regions: Vec<Region>,
    pub groups: Vec<Group>,
    pub vias: Vec<DefVia>,
}

// Suppress unused-import warning if LayerIndex used only conditionally.
#[allow(dead_code)]
fn _layer_index_marker(_: LayerIndex) {}
