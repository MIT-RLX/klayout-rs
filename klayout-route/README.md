# klayout-route

Routing primitives for the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

Three orthogonal stages, expressed as traits so each can be swapped
independently:

| Trait | Role | v1 implementation |
|-------|------|-------------------|
| `Planner` | Two ports + obstacles → centerline `Path`. | 90°-manhattan one-bend planner. |
| `Stylizer` | Centerline `Path` → shapes / instances. | Single `Path` shape on the port's layer. |
| `Bundler` | Set of pairs → length-matched / non-crossing `Path`s. | Identity (no bundling). |

## Algorithms shipped

| Module | Algorithm | Reference |
|--------|-----------|-----------|
| `astar` | Multi-layer A* | classical |
| `multilayer` | RSMT (rectilinear Steiner minimum tree) | Iterated 1-Steiner — Kahng / Robins 1992 |
| `pathfinder` | Negotiated-congestion global routing | McMurchie / Ebeling 1995 |
| `antenna` | Antenna-rule check + jumper insertion | — |
| `crosstalk` | Aggressor-victim shielding | — |
| `em` | Per-segment current-density check (J = I / (W · T)) | Black 1969 MTTF |
| `bundler` | Bundled bus routing | — |
| `ordering` | Net-ordering heuristics | — |

## Example

```rust,ignore
use klayout_route::{Planner, ManhattanPlanner};

let planner = ManhattanPlanner::new(&obstacles);
let path = planner.plan(&port_a, &port_b)?;
```

## Limits (v1)

- No Pathfinder layer assignment (single-layer GCell graph).
- RSMT is `O(n⁵)` worst case — acceptable for n ≲ 200-pin nets.

## License

Licensed under GPL-3.0-only.
