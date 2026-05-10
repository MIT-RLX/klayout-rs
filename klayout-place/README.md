# klayout-place

Placement primitives for the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

Three stages, each separately invocable:

| Stage | Function | Algorithm |
|-------|----------|-----------|
| Global | `global_place` | Force-directed relaxation; bound-to-bound (B2B) quadratic CG solve. |
| Legalize | `legalize` | Tetris-style: scan rows by x, place each cell at the leftmost free site. |
| Detailed | `detailed_place` | Pairwise swap moves to reduce HPWL further. |
| Pin opt | `pin_optimize` | Reorder swappable input pins to shorten net length. |
| Fill | `decap_fill`, `tap_fill` | Insert decap and tap cells in unoccupied row gaps. |

Each stage operates on the same `PlaceCtx`, so callers can mix and
match (e.g., skip global, run only legalize after a custom initial
placement).

## Example

```rust,ignore
use klayout_place::{global_place, legalize, detailed_place, PlaceCtx};

let mut ctx = PlaceCtx::from_netlist(&netlist, &rows);
global_place(&mut ctx);
legalize(&mut ctx);
detailed_place(&mut ctx);
let final_hpwl = ctx.hpwl();
```

## Limits (v1)

- Single-row aware (cells on integer rows of fixed pitch).
- Cell heights match row height; multi-height cells are v2.
- No electrostatic-density penalty (full ePlace) — analytic optimum
  stacks cells, so `legalize` is mandatory.

## License

Licensed under GPL-3.0-only.
