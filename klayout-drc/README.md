# klayout-drc

Geometric design-rule check primitives for the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

Each rule returns a `Region` of violation areas, which is composable:
union violation results across rules, intersect with allowed-violation
areas (waivers), or feed into a reporting layer. All rules are
expressible via `klayout-geom` boolean and sizing ops.

## Rules implemented in v1

| Rule | Semantics |
|------|-----------|
| `width` | Find regions of a layer narrower than `min`. |
| `space` | Find pairs of polygons in a layer closer than `min`. |
| `separation` | Find pairs across two layers closer than `min`. |
| `enclosing` | Find inner-layer pieces not enclosed by ≥ `min` of outer. |
| `overlap` | Find overlap regions narrower than `min`. |
| `area_min` | Find polygons with area below `min`. |
| `density_window` | KLayout `without_density` / `with_density`: `padding_zero` / `padding_ignore`, `tile_boundary`, `tile_origin`, `tile_count`, `DensityWindowOutput`, merged result |
| `density_fill` | Insert filler tiles to satisfy a density floor. |

## Performance

Cross-polygon rules (`space`, `separation`, `enclosing`, `overlap`,
`space_any`) use a `klayout_spatial::SpatialIndex` of polygon bboxes
inflated by the rule distance, so the candidate-pair set is reduced
from `O(n²)` to `O(n · k)` where `k` is the average number of
neighbors within `min`.

Per-polygon edge enumeration uses orientation-bucketed bisection
(`width`, `width_any`) so the inner cost is `O(E · log E)` rather than
`O(E²)` for polygons with many edges.

The edge-pair kernel matches KLayout's `width_check` / `space_check`
semantics for axis-aligned input and is differentially validated
against the reference engine.

## Example

```rust,ignore
use klayout_drc::{width, space};

let m1_too_thin   = width(&m1, 100);
let m1_too_close  = space(&m1, 200);
let report = m1_too_thin.union(&m1_too_close);
```

## License

Licensed under GPL-3.0-only.
