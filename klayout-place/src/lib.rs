//! `klayout-place` — placement primitives for klayout-rs.
//!
//! Three stages, each separately invocable:
//!
//! * [`global_place`] — analytical placement using force-directed
//!   relaxation. Each net contributes a "spring" between its
//!   constituent cells; springs pull connected cells together while
//!   fixed pins (primary I/Os) anchor the boundary. Iterates until
//!   convergence.
//! * [`legalize`] — snap each cell to its row's site grid and
//!   resolve overlaps left-to-right. Tetris-style: scan rows
//!   sorted by x, place each cell at the leftmost free site that
//!   doesn't overlap a previously-placed cell.
//! * [`detailed_place`] — pairwise swap moves to reduce HPWL further.
//!   At each iteration, try swapping each adjacent cell pair in a
//!   row; keep the swap iff HPWL decreases.
//!
//! v1 is single-row-aware (cells sit on integer rows of fixed pitch)
//! and assumes cell heights match row height. Multi-height cells and
//! mixed-row constraints are a v2.

pub mod bookshelf;
pub mod detailed;
pub mod engine;
pub mod eplace;
pub mod fill;
pub mod global;
pub mod legalize;
pub mod pin_opt;
pub mod quadratic;
pub mod types;

pub use detailed::detailed_place;
pub use engine::{ForceDirectedPlacer, Placer, QuadraticPlacer};
pub use fill::{insert_decaps, insert_taps, TapKind};
pub use global::global_place;
pub use legalize::legalize;
pub use pin_opt::{pin_opt, PinOptConfig, PinPlacement, PinReq, PinSide};
pub use bookshelf::{read_aux, read_bookshelf, AuxManifest, BookshelfError};
pub use eplace::{eplace, EplaceConfig};
pub use quadratic::{quadratic_place, QuadraticConfig};
pub use types::*;
