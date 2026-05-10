//! `klayout` — prelude crate for the klayout-rs workspace.
//!
//! Pulls every sister crate under one roof so downstream code can write
//! a single dependency and a single `use klayout::prelude::*;` instead
//! of enumerating every crate by hand.
//!
//! Each member crate is re-exported under its short name (the
//! `klayout-` prefix is dropped):
//!
//! | Re-export | Source crate |
//! |-----------|--------------|
//! | `klayout::core` | `klayout-core` |
//! | `klayout::io` | `klayout-io` |
//! | `klayout::geom` | `klayout-geom` |
//! | `klayout::spatial` | `klayout-spatial` |
//! | `klayout::pdk` | `klayout-pdk` |
//! | `klayout::drc` | `klayout-drc` |
//! | `klayout::deck` | `klayout-deck` |
//! | `klayout::connect` | `klayout-connect` |
//! | `klayout::place` | `klayout-place` |
//! | `klayout::cts` | `klayout-cts` |
//! | `klayout::route` | `klayout-route` |
//! | `klayout::lef` | `klayout-lef` |
//! | `klayout::liberty` | `klayout-liberty` |
//! | `klayout::sta` | `klayout-sta` |
//!
//! [`prelude`] gathers the most-frequently used names — `Library`,
//! `Cell`, `Region`, `LayerIndex`, GDS read/write, etc. — for glob
//! import.

pub use klayout_connect as connect;
pub use klayout_core as core;
pub use klayout_cts as cts;
pub use klayout_deck as deck;
pub use klayout_drc as drc;
pub use klayout_geom as geom;
pub use klayout_io as io;
pub use klayout_lef as lef;
pub use klayout_liberty as liberty;
pub use klayout_pdk as pdk;
pub use klayout_place as place;
pub use klayout_route as route;
pub use klayout_spatial as spatial;
pub use klayout_sta as sta;

/// Most-used names across the workspace, intended for glob import:
///
/// ```ignore
/// use klayout::prelude::*;
/// ```
pub mod prelude {
    pub use klayout_core::{
        Bbox, Cell, CellId, LayerIndex, LayerInfo, Library, Path, Point, Polygon, Port, Shape,
        Trans,
    };
    pub use klayout_geom::Region;
    pub use klayout_io::{read_gds_path, write_gds_path};
    pub use klayout_spatial::SpatialIndex;
}
