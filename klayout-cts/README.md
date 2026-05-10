# klayout-cts

Clock-tree synthesis for the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

Build a balanced clock distribution tree from one source to N sinks
(FF clock pins). Two algorithms:

| Function | Method | Reference |
|----------|--------|-----------|
| Recursive-bisection H-tree | Method-of-means partitioning, midpoint buffers | classical |
| `synthesise_clock_tree_dme` | Deferred-Merge Embedding with merging-segment construction in rotated `(u, v)` coordinates and `rstar`-backed nearest-pair selection | Chao / Hsu / Ho 1992 |

The output is a [`ClockTree`] of `Branch` nodes with explicit buffer
locations and parent / child relationships. A consumer can lower it
into routing requests (one per buffer-to-buffer or buffer-to-sink
leg) and feed those into the detailed router.

## Modules

- `dme` — Deferred-Merge Embedding (full Chao/Hsu/Ho).
- `buffer` — Buffer insertion: `buffer_insert`, `BufferPlacement`,
  `Candidate`, `InsertConfig`.

## Example

```rust,ignore
use klayout_cts::{synthesise_clock_tree_dme, DmeConfig};

let tree = synthesise_clock_tree_dme(
    &sinks,
    &source,
    &DmeConfig::default(),
);
for buf in tree.buffers() {
    println!("{:?} → {:?}", buf.location, buf.children);
}
```

## Limits (v1)

Single-corner DME with point-tap selection on each merging segment.
Full top-down refinement is implemented but per-arc POCV derate is
left to the caller.

## License

Licensed under GPL-3.0-only.
