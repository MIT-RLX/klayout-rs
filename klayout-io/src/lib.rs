//! `klayout-io` — GDSII reader and writer for klayout-rs.
//!
//! Layered above `klayout-core`. Reading produces a `Library`; writing
//! consumes one. OASIS and LEF/DEF will be added as separate modules.

pub mod cif;
pub mod dxf;
pub mod error;
pub mod gds;
pub mod mag;
pub mod oasis;

pub use cif::{read_cif_path, read_cif_str, write_cif_str};
pub use dxf::{read_dxf_path, read_dxf_str, write_dxf_str};
pub use mag::{read_mag_path, read_mag_str, write_mag_str};
pub use error::{IoError, Result};
pub use gds::{read_gds_bytes, read_gds_path, write_gds_bytes, write_gds_path};
pub use oasis::{read_oasis_bytes, read_oasis_path, write_oasis_bytes, write_oasis_path};
