//! `klayout-core` — the foundation data model for klayout-rs.
//!
//! This crate defines coordinates, geometry primitives (data only), layers,
//! cells, ports, instances, and the parameterized-component machinery.
//! Everything else in the workspace — IO, geometry algorithms, routing,
//! DRC/LVS — sits on top of these types and is expected to honor the
//! invariants documented per-module.
//!
//! Design contract:
//! 1. Frozen cells are immutable. All mutation goes through `CellBuilder`.
//! 2. `ContentHash` is deterministic, structural, byte-stable across runs.
//! 3. All coordinates are i64 DBU. Microns are a thin conversion layer.
//! 4. `Library` is `Send + Sync`; concurrent `build`/`insert` is safe.
//! 5. `CellId` / `LayerIndex` are meaningful only with their issuing library.
//! 6. No global state. `BuildCtx` is passed.

pub mod bus;
pub mod cell;
pub mod component;
pub mod coord;
pub mod edit;
pub mod error;
pub mod hash;
pub mod instance;
pub mod layer;
pub mod layermap;
pub mod library;
pub mod port;
pub mod properties;
pub mod query;
pub mod shape;

pub use bus::{format_bus_pin, parse_bus, Bus, BusBitChars, BusName};
pub use cell::{Cell, CellBuilder, CellId, CellName, ShapeBag};
pub use component::{BuildCtx, Component};
pub use edit::{clip_cell, flatten, local_bbox, remap_layers, replace_instances, LayerMap};
pub use coord::{Bbox, DTrans, Point, Rot4, Trans, Vec2};
pub use error::{CoreError, Result};
pub use hash::{ContentHash, ParamHash, ParamHasher};
pub use instance::{Instance, Repetition, SourceTag};
pub use layer::{LayerIndex, LayerInfo, LayerTable};
pub use layermap::{parse_layermap, write_layermap, LayerMapEntry, LayerMapError, LayerMapping};
pub use library::Library;
pub use port::{Angle90, Port, PortKindId};
pub use properties::{Properties, PropertyValue};
pub use query::{HierarchyVisitor, InstanceRef, LayoutQuery, ShapeRef, Visit};
pub use shape::{HAlign, Path, PathCap, Polygon, Rect, Shape, Text, VAlign};
