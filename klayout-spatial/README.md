# klayout-spatial

Bbox-keyed spatial index used by DRC and routing in the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

Two backends share the same query surface:

| Backend | Type | Best for |
|---------|------|----------|
| Uniform grid | `SpatialIndex<T>` | Layouts up to ~10⁶ shapes with uniform density. O(√n) query. |
| R*-tree (`rstar`) | `RTreeIndex<T>` | Sign-off-scale layouts (10⁹+ shapes), or non-uniform distributions. O(log n). |

Pick the grid index for hot inner loops on small layouts (no extra
dependency overhead, lower constant factor) and the R-tree when the
layout is large or the distribution is uneven.

## Example

```rust,ignore
use klayout_spatial::SpatialIndex;
use klayout_core::Bbox;

let idx = SpatialIndex::build(items.iter().map(|s| (s.bbox, s.id)));
for hit in idx.query(&query_bbox) {
    // hit: &T
}
```

## License

Licensed under GPL-3.0-only.
