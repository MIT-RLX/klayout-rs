# klayout-geom

Geometric algorithms over `klayout-core` types: boolean ops, sizing,
fracturing, edge enumeration, polygon simplification, convex
decomposition, and region selection. Part of the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

`Region` is the central abstraction: a deterministic, merged set of
polygons on a single conceptual layer. Boolean ops and sizing produce
`Region`s; flattening a hierarchical `(Library, CellId, LayerIndex)`
into a `Region` is the bridge to verification and routing.

## API

| Operation | Function |
|-----------|----------|
| Boolean | `union`, `intersection`, `difference`, `xor`, `merge` |
| Sizing | `size` (with `SizeJoin::Miter` / `Round` / `Bevel`) |
| Fracturing | `fracture` → `Trapezoid` slabs |
| Convex split | `decompose`, `convex_pieces`, `ear_clip` |
| Edges | `Edges`, `Edge` enumeration over a `Region` |
| Path → polygon | `path_to_polygon` |
| Simplification | `simplify`, `simplify_open`, `simplify_ring` |
| Selection | `inside`, `outside`, `interacting`, `overlapping`, `with_text`, `select_by_*` |

## Backend

Booleans dispatch through [`i_overlay`](https://crates.io/crates/i_overlay),
a Bentley–Ottmann sweep implementation. All ops are deterministic for
identical input.

## Example

```rust,ignore
use klayout_geom::{Region, size, SizeJoin, union};

let bigger = size(&region, 100, SizeJoin::Miter);
let merged = union(&[&a, &b]);
```

## License

Licensed under GPL-3.0-only.
