//! GDSII format reader and writer.

pub mod codec;
pub mod read;
pub mod records;
pub mod stream;
pub mod stream_write;
pub mod validate;
pub mod write;

pub use read::{read_gds_bytes, read_gds_path};
pub use stream::{open_gds_bytes, GdsIndex, StreamEvent};
pub use stream_write::StreamingGdsWriter;
pub use validate::{repair, validate, Issue, IssueKind, ValidationReport};
pub use write::{write_gds_bytes, write_gds_path};
