//! `klayout-drc` — geometric design-rule check primitives.
//!
//! Each rule returns a `Region` of violation areas, which is composable:
//! you can union violation results across rules, intersect with allowed-
//! violation areas (waivers), or feed into a reporting layer.
//!
//! All algorithms here are expressible via `klayout-geom` boolean and sizing
//! ops.
//!
//! Cross-polygon rules (`space`, `separation`, `enclosing`, `overlap`,
//! `space_any`) use a `klayout_spatial::SpatialIndex` of polygon bboxes
//! inflated by the rule distance, so the candidate-pair set is reduced
//! from `O(n²)` to `O(n · k)` where `k` is the average number of
//! neighbors within `min`. Per-polygon edge enumeration uses
//! orientation-bucketed bisection (`width`, `width_any`) so the
//! inner cost is `O(E · log E)` rather than `O(E²)` for polygons with
//! many edges.
//!
//! Rules implemented in v1:
//!
//! | Rule          | Semantics                                                     |
//! |---------------|---------------------------------------------------------------|
//! | `width`       | Find regions of a layer narrower than `min`.                  |
//! | `space`       | Find pairs of polygons in a layer closer than `min`.          |
//! | `separation`  | Find pairs across two layers closer than `min`.               |
//! | `enclosing`   | Find inner-layer pieces not enclosed by ≥ `min` of outer.     |
//! | `overlap`     | Find overlap regions narrower than `min`.                     |
//! | `area_min`    | Find polygons with area < `min`.                              |

pub mod density_fill;
mod edge;
mod edge_general;
pub mod edge_rules;
pub mod hier;
pub mod lfd;
pub mod opc;
pub mod rules;
pub mod tile;
pub mod violation;
pub mod waiver;

pub use density_fill::{generate_fill, FillConfig};
pub use edge_rules::{separation_edges, space_edges, width_edges};
pub use hier::hierarchical_check;
pub use lfd::{line_end_near_corner, small_jog, tight_u_shape};
pub use opc::{hammerhead, serif, sraf, OpcConfig};
pub use rules::{
    area_min, density, enclosing, overlap, separation, space, space_any, width, width_any,
};
pub use tile::{tile_binary, tile_unary, TileConfig};
pub use violation::{violations_from_region, Violation};
pub use waiver::{apply_waivers, apply_waivers_logged, Waiver, WaiverHit};
