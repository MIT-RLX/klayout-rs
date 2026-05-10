# klayout-core

Core data model for the [`klayout-rs`](https://github.com/MIT-RLX/klayout-rs)
workspace: coordinates, shapes, cells, libraries, layers, ports,
transformations, and the parameterized-component machinery every other
crate is built on.

## What's in here

| Module | Contents |
|--------|----------|
| `coord` | DBU integer coordinates, `Point`, `Vec2`, `Bbox`, `Trans`, `Rot4`. |
| `shape` | `Shape`, `Polygon`, `Path`, `Rect`, `Text` and friends — data only. |
| `cell` | Immutable `Cell`, `CellBuilder`, `ShapeBag`, content-hashed `CellId`. |
| `library` | `Library`: thread-safe interner of cells + layer table. |
| `layer` | `LayerIndex`, `LayerInfo`, `LayerTable`. |
| `port` | Typed `Port`, `PortKindId`, `Angle90`. |
| `instance` | `Instance`, `Repetition` arrays, `SourceTag` provenance. |
| `component` | Parameterized cell generation via `Component` + `BuildCtx`. |
| `edit` | `flatten`, `clip_cell`, `remap_layers`, `replace_instances`. |
| `query` | `LayoutQuery`, `HierarchyVisitor` for read-only traversal. |
| `hash` | Deterministic structural `ContentHash` and `ParamHash`. |
| `properties` | `Properties` map for user-defined per-shape/cell metadata. |
| `bus` | Bus-name parser/formatter for synthesis-style pin lists. |
| `error` | `CoreError` and `Result`. |

## Design contract

1. Frozen cells are immutable. All mutation goes through `CellBuilder`.
2. `ContentHash` is deterministic, structural, and byte-stable across
   runs.
3. All coordinates are `i64` DBU. Microns are a thin conversion layer.
4. `Library` is `Send + Sync`; concurrent `build` / `insert` is safe.
5. `CellId` / `LayerIndex` are meaningful only with their issuing
   library.
6. No global state. `BuildCtx` is passed.

## Example

```rust,ignore
use klayout_core::{Library, Polygon, Point, Shape};

let lib = Library::new("demo");
let m1 = lib.layer(klayout_core::LayerInfo::named("M1", 1, 0));
let mut cell = lib.builder("top");
cell.add(m1, Shape::Polygon(Polygon::rect(0, 0, 1000, 500)));
let top = cell.freeze();
```

## License

Licensed under GPL-3.0-only.
