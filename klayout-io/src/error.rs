use std::io;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, IoError>;

#[derive(Debug, Error)]
pub enum IoError {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("gds: unexpected end of stream")]
    UnexpectedEof,

    #[error("gds: invalid record length {0}")]
    InvalidRecordLength(u16),

    #[error("gds: unexpected record {kind:#06x} in {context}")]
    UnexpectedRecord {
        kind: u16,
        context: &'static str,
    },

    #[error("gds: missing required record {0}")]
    MissingRecord(&'static str),

    #[error("gds: non-orthogonal angle {0} (only 0/90/180/270 supported)")]
    NonOrthogonalAngle(f64),

    #[error("gds: magnification {0} != 1.0 (mag is not yet supported)")]
    UnsupportedMagnification(f64),

    #[error("gds: cell '{0}' referenced but not defined")]
    CellNotFound(String),

    #[error("gds: cycle detected in cell hierarchy at '{0}'")]
    CycleDetected(String),

    #[error("gds: malformed UNITS record")]
    MalformedUnits,

    #[error("gds: bad XY record (got {got} bytes, expected multiple of 8)")]
    BadXy { got: usize },

    #[error("gds: AREF count zero")]
    ZeroArefCount,

    // ---------- OASIS ----------
    #[error("oasis: variable-length integer overflow (>64 bits)")]
    OasisVarIntOverflow,

    #[error("oasis: unsupported real-number type code {0}")]
    OasisUnsupportedReal(u8),

    #[error("oasis: invalid UTF-8 in string")]
    OasisInvalidUtf8,

    #[error("oasis: bad magic header")]
    OasisBadMagic,

    #[error("oasis: unsupported record type {0:#x}")]
    OasisUnsupportedRecord(u8),

    #[error("oasis: feature not yet implemented: {0}")]
    OasisNotImplemented(&'static str),
}
