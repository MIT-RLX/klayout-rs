//! OASIS Stream Format reader / writer.
//!
//! **Status: scaffold (v1).** Variable-length integer codec is complete
//! and correctness-tested. Read+write of empty (header-only) layouts
//! works. Cell + shape encoding is laid out for the writer (RECTANGLE,
//! POLYGON, CELL records) but the reader currently rejects those records
//! with `OasisNotImplemented`. CBLOCK compression, modal variables,
//! TABLE-OFFSETS strict mode, and PROPERTY records are deferred.
//!
//! The codec module is the meaningful first deliverable — every higher
//! record type bottoms out in `unsigned-integer`, `signed-integer`,
//! `real`, and `string`, so the foundation is in place.

pub mod codec;
pub(crate) mod modal;
pub mod read;
pub mod records;
pub mod stream;
pub mod write;

pub use read::{read_oasis_bytes, read_oasis_path};
pub use stream::{open_oasis_bytes, OasisIndex, OasisStreamEvent};
pub use write::{write_oasis_bytes, write_oasis_bytes_with, write_oasis_path};
