//! `klayout-lef` — LEF/DEF reader and writer.
//!
//! These two text formats are how digital flows hand layouts off:
//! * **LEF** — cell library abstracts (no internal geometry, just
//!   boundary + pin geometry).
//! * **DEF** — placed-and-routed netlist referencing LEF cells.
//!
//! v1 covers the basic shape — `MACRO`/`PIN`/`PORT` for LEF and
//! `DESIGN`/`COMPONENTS` for DEF. Full LEF/DEF spec compliance (vias,
//! routing, blockages, antenna properties, …) is a follow-up; the
//! parser scaffolding is structured to add records incrementally.

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
