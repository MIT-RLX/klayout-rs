//! Error types. Every error carries the cell context where possible so that
//! sign-off-grade reports can be assembled without losing provenance.

use crate::cell::CellName;
use crate::coord::Point;
use crate::layer::LayerInfo;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("off-grid coordinate {coord:?} (manufacturing grid = {grid}) in cell {cell}")]
    OffGrid {
        coord: Point,
        grid: i64,
        cell: CellName,
    },

    #[error("unknown layer {0:?}")]
    UnknownLayer(LayerInfo),

    #[error("unknown cell id {0:?}")]
    UnknownCell(u32),

    #[error("port name collision: {name} already exists in cell {cell}")]
    PortCollision { name: String, cell: CellName },

    #[error("library mismatch: {detail}")]
    LibraryMismatch { detail: String },
}
