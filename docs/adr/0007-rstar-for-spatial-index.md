# ADR-0007 — `rstar` for the production spatial index

**Status:** Accepted

## Context

Spatial indexing shows up in two distinct hot paths:

1. **DRC cross-polygon rules** (`space`, `separation`, `enclosing`,
   `overlap`, `space_any`) — 10⁵ – 10⁸ polygon bboxes, queried
   millions of times per pass.
2. **DME closest-pair selection in CTS** — 10² – 10⁴ subtree taps
   re-queried every merge round.

Three implementations sit in `klayout-spatial`:

1. **`SpatialIndex` (grid bucket).** Uniform grid over the
   bounding-box extent. O(√n) average queries; cache-friendly;
   degrades on highly non-uniform distributions (clusters of
   fine-pitch metal next to large empty regions are common in real
   layouts).
2. **`RTreeIndex` (`rstar` R*-tree).** O(log n) average queries
   even on non-uniform distributions; bulk-loaded for fast
   construction; well-tested upstream library.
3. **kd-tree.** Worse than R*-tree for bbox queries (kd-trees
   index points; we want bbox-vs-bbox intersect tests).

## Decision

Both indices coexist behind a common interface:

- `SpatialIndex` is the default for hot inner loops on bounded n
  (our existing DRC kernel uses it because the grid bucket
  outperforms the R*-tree for `n ≲ 10⁴` due to lower constant
  factors).
- `RTreeIndex` (wrapping `rstar`) is what large-design / sign-off
  flows pick when the polygon distribution is genuinely
  non-uniform.

Internal code (DME's `closest_pair_rstar`, the LVS netlist matcher)
goes directly to `rstar` when the access pattern needs `nearest_neighbor`
queries that the grid bucket doesn't expose.

## Consequences

**Pro:**
- Two implementations, one API, swappable. Tests cover both.
- `rstar` is a well-maintained crate with a substantial proptest
  battery; we don't carry the maintenance burden of a hand-rolled
  R*-tree.
- DME got `O(n²)` → `O(n log n)` per merge round basically for
  free.

**Con:**
- Two implementations means twice the surface area to keep aligned
  on bug fixes.
- `rstar`'s API uses `[f64; 2]` for points (it needs `RTreeNum`,
  which is implemented for `i32 / f32 / f64` only). DBU-`i64`
  coordinates fit exactly in `f64` up to `2⁵³`, which is far
  beyond any reticle, so the cast is lossless — but it's an
  invariant we have to remember.

## Revisit if

- A real benchmark surfaces a workload where neither index is
  fast enough — at which point a hilbert-sort or zorder-curve
  approach over packed `(u32, u32)` keys may beat both.
