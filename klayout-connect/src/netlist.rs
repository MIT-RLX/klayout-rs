//! Netlist intermediate representation.
//!
//! A `Netlist` is a flat list of `Net`s, each holding its name, the cell
//! ports/pins it connects to, and the geometric shapes it claims. Built
//! from `klayout-core` types — no GDSII or SPICE awareness leaks in here.
//!
//! Higher-level concerns (LVS comparison, hierarchical netlists, devices)
//! sit on top in future crates.

use klayout_core::{Bbox, CellId, LayerIndex};
use smol_str::SmolStr;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetId(pub u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PinRef {
    pub cell: CellId,
    pub port_index: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShapeRef {
    pub cell: CellId,
    pub layer: LayerIndex,
    pub index: u32,
}

#[derive(Clone, Debug)]
pub struct Net {
    pub id: NetId,
    pub name: SmolStr,
    pub pins: Vec<PinRef>,
    pub shapes: Vec<ShapeRef>,
    pub bbox: Bbox,
    /// Merged polygons making up this net, in the top-cell frame
    /// (transforms already composed). Populated by
    /// `extract_hierarchical`; downstream consumers can do real
    /// point-in-polygon containment instead of bbox-only.
    /// Empty for callers that build `Net` directly without going
    /// through extraction.
    pub polygons: Vec<klayout_core::Polygon>,
}

impl Net {
    pub fn new(id: NetId, name: impl Into<SmolStr>) -> Self {
        Self {
            id,
            name: name.into(),
            pins: Vec::new(),
            shapes: Vec::new(),
            bbox: Bbox::EMPTY,
            polygons: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Netlist {
    nets: Vec<Net>,
    by_name: std::collections::HashMap<SmolStr, NetId>,
}

impl Netlist {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, mut net: Net) -> NetId {
        let id = NetId(self.nets.len() as u32);
        net.id = id;
        if !net.name.is_empty() {
            self.by_name.insert(net.name.clone(), id);
        }
        self.nets.push(net);
        id
    }

    pub fn get(&self, id: NetId) -> Option<&Net> {
        self.nets.get(id.0 as usize)
    }

    pub fn by_name(&self, name: &str) -> Option<NetId> {
        self.by_name.get(name).copied()
    }

    pub fn nets(&self) -> &[Net] {
        &self.nets
    }

    pub fn len(&self) -> usize {
        self.nets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nets.is_empty()
    }
}
