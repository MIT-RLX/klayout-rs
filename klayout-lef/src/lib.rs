//! `klayout-lef` — LEF/DEF reader and writer.
//!
//! These two text formats are how digital flows hand layouts off:
//! * **LEF** — cell library abstracts (no internal geometry, just
//!   boundary + pin geometry).
//! * **DEF** — placed-and-routed netlist referencing LEF cells.
//!
//! Import reads into [`klayout_core::Library`] plus [`DefDesign`](types::DefDesign) metadata
//! (rows, tracks, vias, nets, special nets, blockages, regions, groups). OpenDB
//! differential parity for placement and regular-net routing lives in
//! `validation/klayout-validate/tests/def.rs`. Extra import combinations and edge
//! cases (special stripes, stack vias, `*` coordinates, `PIN` net taps) are in
//! `tests/def_import_coverage.rs`.

pub mod def_read;
pub mod def_write;
pub mod error;
pub mod lef_read;
pub mod lef_write;
pub mod tokenizer;
pub mod types;

pub use def_read::{read_def, read_def_full};
pub use def_write::{write_def, write_def_full};
pub use error::{LefError, Result};
pub use lef_read::{read_lef, read_lef_full};
pub use lef_write::{write_lef, write_lef_full};
pub use types::*;
