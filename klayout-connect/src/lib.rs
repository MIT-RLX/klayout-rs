//! `klayout-connect` — connectivity extraction → Netlist IR.
//!
//! Bridges geometry to electrical structure: given a `Library` + `CellId` +
//! conductor/label layers, produces a `Netlist` where each `Net` aggregates
//! the shapes that form one electrically-connected piece, named by any
//! text labels that fall inside it.
//!
//! v1 is flat (no hierarchy descent) and single-layer (no via stitching).
//! Cross-layer stitching and hierarchical extraction are follow-ups.

pub mod antenna;
mod cdl;
pub mod device;
pub mod extract;
pub mod hier;
pub mod hier_netlist;
pub mod lvs;
pub mod lvs_hier;
pub mod netlist;
pub mod pex;
pub mod pex_hier;
pub mod pex_multilayer;
pub mod pex_skeleton;
pub mod spef;
pub mod spef_hier;
pub mod vf2;

pub use antenna::{antenna_check, metal_area_per_net, AntennaViolation};
pub use device::{extract_mos, Device, DeviceKind, MosLayers};
pub use extract::extract_flat;
pub use hier::{extract_hierarchical, Conductor, ExtractConfig, Via};
pub use hier_netlist::{
    extract_hier_netlist, CellNetlist, HierNetlist, LocalNet, NetlistInstance,
};
pub use lvs::{
    lvs_compare, lvs_compare_vf2, lvs_compare_with_tol, DeviceMismatch, LvsReport, ParamTolerance,
};
pub use lvs_hier::{
    lvs_compare_hier, lvs_compare_hier_with_lib, CellLvsReport, HierLvsReport,
};
pub use netlist::{Net, NetId, Netlist, PinRef, ShapeRef};
pub use pex::{extract_pex, pex_from_polygons, LayerPexParams, NetParasitics};
pub use pex_hier::{extract_pex_hier, HierNetParasitics, HierPexReport};
pub use pex_skeleton::{decompose_rects, skeleton_path, skeleton_resistance};
pub use spef::{nets_from_pex, read_spef, write_spef, SpefHeader, SpefNet, SpefUnit};
pub use spef_hier::{write_spef_hier, HierSpef};
pub use vf2::{vf2_match, Vf2Match};
