//! `klayout-liberty` — reader for the Liberty (.lib) timing-library format.
//!
//! Liberty files are nested `group_name (args) { ... }` blocks
//! containing simple key-value attributes (`key : value ;`) and
//! complex attributes (`key (v1, v2, ...) ;`). Cells, pins, timing
//! arcs, and lookup tables all use this same recursive structure.
//!
//! v1 covers the data that STA tools actually consume in 90% of
//! flows:
//! * `library`-level units (`time_unit`, `capacitive_load_unit`,
//!   `voltage_unit`, …)
//! * per-cell `area`, `cell_leakage_power`
//! * per-pin `direction`, `capacitance`, `function`, `clock`
//! * `timing` arcs with `related_pin`, `timing_sense`, `timing_type`,
//!   plus the raw text of `cell_rise` / `cell_fall` /
//!   `rise_transition` / `fall_transition` lookup tables
//!
//! The lookup-table values are kept as their original string for v1
//! — STA consumers want the table verbatim and will parse the index
//! grids themselves. v2 will decode them into `LookupTable` structs
//! with axes + values.

pub mod nldm;
pub mod parser;
pub mod tokenizer;
pub mod types;

pub use nldm::{parse_lookup_table, LookupTable};
pub use parser::{parse_liberty, parse_liberty_path};
pub use types::*;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LibertyError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("liberty: unexpected token at byte {pos}: {msg}")]
    UnexpectedToken { pos: usize, msg: String },

    #[error("liberty: unexpected end of input")]
    UnexpectedEof,

    #[error("liberty: invalid number: {0}")]
    InvalidNumber(String),

    #[error("liberty: invalid utf-8")]
    InvalidUtf8,
}

pub type Result<T> = std::result::Result<T, LibertyError>;
