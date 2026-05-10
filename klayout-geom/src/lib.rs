//! `klayout-geom` — geometric algorithms over `klayout-core` types.
//!
//! `Region` is the central abstraction: a deterministic, merged set of
//! polygons on a single conceptual layer. Boolean ops and sizing produce
//! `Region`s; flattening a hierarchical `(Library, CellId, LayerIndex)` into
//! a `Region` is the bridge to verification and routing.

pub mod boolean;
pub mod convex;
pub mod edges;
pub mod fracture;
pub mod path;
pub mod region;
pub mod select;
pub mod simplify;
pub mod size;

pub use boolean::{difference, intersection, merge, union, xor};
pub use convex::{convex_pieces, decompose, ear_clip, Triangle};
pub use fracture::{fracture, Trapezoid};
pub use edges::{Edge, Edges};
pub use path::path_to_polygon;
pub use region::Region;
pub use select::{
    inside, interacting, not_interacting, not_overlapping, outside, overlapping,
    select_by_area, select_by_aspect_ratio, select_by_perimeter, with_text,
};
pub use simplify::{simplify, simplify_open, simplify_ring};
pub use size::{size, SizeJoin};
