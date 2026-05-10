//! `klayout-spatial` — bbox-keyed spatial index.
//!
//! Two backends with the same query surface:
//! * [`SpatialIndex`] — uniform grid bucketing. Fast for layouts up to
//!   ~10⁶ shapes with reasonably uniform distribution. O(√n) average.
//! * [`RTreeIndex`] (in [`rtree`]) — `rstar` R*-tree with bulk loading.
//!   O(log n) average, robust to non-uniform distributions and
//!   sign-off-scale (10⁹-shape) layouts.
//!
//! Pick `SpatialIndex` for hot inner loops on small layouts (no dep
//! overhead, lower constant) and `RTreeIndex` when the layout is large
//! or distribution is uneven.

pub mod rtree;
pub use rtree::RTreeIndex;

use klayout_core::Bbox;
use std::collections::{HashMap, HashSet};

pub struct SpatialIndex<T> {
    items: Vec<(Bbox, T)>,
    bbox: Bbox,
    cell_size: i64,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

impl<T> SpatialIndex<T> {
    pub fn build(it: impl IntoIterator<Item = (Bbox, T)>) -> Self {
        let items: Vec<(Bbox, T)> = it.into_iter().collect();
        if items.is_empty() {
            return Self {
                items,
                bbox: Bbox::EMPTY,
                cell_size: 1,
                grid: HashMap::new(),
            };
        }
        let bbox = items
            .iter()
            .map(|(b, _)| *b)
            .fold(Bbox::EMPTY, |a, b| a.union(&b));
        let area = bbox.width().max(1) as i128 * bbox.height().max(1) as i128;
        let target_cell_area = (area / items.len() as i128).max(1);
        let cell_size = (target_cell_area as f64).sqrt().round().max(1.0) as i64;
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, (item_bbox, _)) in items.iter().enumerate() {
            for cell in cells_covering(*item_bbox, bbox, cell_size) {
                grid.entry(cell).or_default().push(i as u32);
            }
        }
        Self {
            items,
            bbox,
            cell_size,
            grid,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn bbox(&self) -> Bbox {
        self.bbox
    }

    /// Query for items whose bbox intersects `query`.
    /// Iterator yields `(bbox, value)` pairs in unspecified order.
    pub fn query(&self, query: Bbox) -> impl Iterator<Item = (Bbox, &T)> + '_ {
        let mut seen: HashSet<u32> = HashSet::new();
        let mut hits: Vec<(Bbox, &T)> = Vec::new();
        for cell in cells_covering(query, self.bbox, self.cell_size) {
            let Some(indices) = self.grid.get(&cell) else {
                continue;
            };
            for &i in indices {
                if !seen.insert(i) {
                    continue;
                }
                let (b, v) = &self.items[i as usize];
                if b.intersects(&query) {
                    hits.push((*b, v));
                }
            }
        }
        hits.into_iter()
    }

    /// All items whose bbox is fully contained in `query`. Stricter than
    /// `query` (which returns intersecting items); useful for region-of-
    /// interest queries that don't want clipped coverage.
    pub fn query_contained(&self, query: Bbox) -> impl Iterator<Item = (Bbox, &T)> + '_ {
        self.query(query)
            .filter(move |(b, _)| query.contains(b.min) && query.contains(b.max))
    }
}

fn cells_covering(b: Bbox, base: Bbox, cell_size: i64) -> Vec<(i32, i32)> {
    if b.is_empty() || base.is_empty() {
        return Vec::new();
    }
    let lo_x = ((b.min.x - base.min.x) / cell_size) as i32;
    let lo_y = ((b.min.y - base.min.y) / cell_size) as i32;
    let hi_x = ((b.max.x - base.min.x) / cell_size) as i32;
    let hi_y = ((b.max.y - base.min.y) / cell_size) as i32;
    let mut out = Vec::with_capacity(((hi_x - lo_x + 1) * (hi_y - lo_y + 1)) as usize);
    for gx in lo_x..=hi_x {
        for gy in lo_y..=hi_y {
            out.push((gx, gy));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::Point;

    fn bb(x0: i64, y0: i64, x1: i64, y1: i64) -> Bbox {
        Bbox::new(Point::new(x0, y0), Point::new(x1, y1))
    }

    #[test]
    fn finds_intersecting() {
        let idx = SpatialIndex::build([
            (bb(0, 0, 10, 10), "a"),
            (bb(50, 50, 60, 60), "b"),
            (bb(20, 0, 30, 10), "c"),
        ]);
        let hits: Vec<&&str> = idx.query(bb(5, 5, 25, 5)).map(|(_, v)| v).collect();
        // Only "a" and "c" overlap the query.
        assert_eq!(hits.len(), 2);
        assert!(hits.contains(&&"a"));
        assert!(hits.contains(&&"c"));
    }

    #[test]
    fn empty_index() {
        let idx: SpatialIndex<&str> = SpatialIndex::build([]);
        assert!(idx.is_empty());
        assert_eq!(idx.query(bb(0, 0, 100, 100)).count(), 0);
    }

    #[test]
    fn dense_grid() {
        let items: Vec<(Bbox, usize)> = (0i64..1000)
            .map(|i| {
                let x = i % 32;
                let y = i / 32;
                (bb(x * 10, y * 10, x * 10 + 5, y * 10 + 5), i as usize)
            })
            .collect();
        let idx = SpatialIndex::build(items);
        assert_eq!(idx.len(), 1000);
        // Query a small region should return only a few items.
        let hits: Vec<_> = idx.query(bb(0, 0, 50, 50)).collect();
        assert!(hits.len() <= 50);
        assert!(hits.len() >= 25); // 5x5 grid of items in the 50x50 region
    }

    #[test]
    fn contained_filter() {
        let idx = SpatialIndex::build([
            (bb(0, 0, 10, 10), "inside"),
            (bb(50, 50, 200, 200), "spans_edge"),
        ]);
        let q = bb(-100, -100, 100, 100);
        let intersecting: Vec<_> = idx.query(q).collect();
        let contained: Vec<_> = idx.query_contained(q).collect();
        assert_eq!(intersecting.len(), 2);
        assert_eq!(contained.len(), 1);
    }
}
