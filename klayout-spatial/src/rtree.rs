//! R-tree spatial index — drop-in alternative to `SpatialIndex`.
//!
//! The grid-bucket index in this crate is fast for layouts up to ~10⁶
//! shapes with reasonably uniform distribution. For sign-off scale or
//! highly non-uniform distributions (clusters of fine-pitch metal next
//! to large empty regions), the grid degrades — most query work goes
//! into iterating empty cells or, conversely, oversubscribed ones.
//!
//! [`RTreeIndex`] uses the `rstar` R*-tree, which gives O(log n)
//! average-case queries with bulk loading and tighter bounding-box
//! hierarchies. Its API mirrors [`crate::SpatialIndex`] so callers can
//! swap backends behind a generic.

use klayout_core::Bbox;
use rstar::{
    primitives::{GeomWithData, Rectangle},
    RTree, AABB,
};

// rstar requires `RTreeNum` for the point scalar; in 0.12 only i32/f32/f64
// implement it. Layout DBU coordinates are always within ±2^53, so f64
// preserves them exactly.
type Rect2 = Rectangle<[f64; 2]>;
type Entry<T> = GeomWithData<Rect2, EntryData<T>>;

struct EntryData<T> {
    bbox: Bbox,
    value: T,
}

pub struct RTreeIndex<T> {
    tree: RTree<Entry<T>>,
    bbox: Bbox,
    len: usize,
}

impl<T> RTreeIndex<T> {
    pub fn build(it: impl IntoIterator<Item = (Bbox, T)>) -> Self {
        let entries: Vec<Entry<T>> = it
            .into_iter()
            .map(|(b, v)| {
                let rect = bbox_to_rect(b);
                GeomWithData::new(
                    rect,
                    EntryData {
                        bbox: b,
                        value: v,
                    },
                )
            })
            .collect();
        let len = entries.len();
        let bbox = entries
            .iter()
            .map(|e| e.data.bbox)
            .fold(Bbox::EMPTY, |a, b| a.union(&b));
        let tree = RTree::bulk_load(entries);
        Self { tree, bbox, len }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn bbox(&self) -> Bbox {
        self.bbox
    }

    /// Items whose bbox intersects `query`. Yields `(bbox, &value)`.
    pub fn query(&self, query: Bbox) -> impl Iterator<Item = (Bbox, &T)> + '_ {
        if query.is_empty() {
            return Vec::new().into_iter();
        }
        let q = bbox_to_aabb(query);
        let hits: Vec<(Bbox, &T)> = self
            .tree
            .locate_in_envelope_intersecting(&q)
            .map(|e| (e.data.bbox, &e.data.value))
            .collect();
        hits.into_iter()
    }

    /// Items whose bbox is fully contained in `query`.
    pub fn query_contained(&self, query: Bbox) -> impl Iterator<Item = (Bbox, &T)> + '_ {
        self.query(query)
            .filter(move |(b, _)| query.contains(b.min) && query.contains(b.max))
    }
}

fn bbox_to_rect(b: Bbox) -> Rect2 {
    if b.is_empty() {
        return Rectangle::from_corners([0.0, 0.0], [0.0, 0.0]);
    }
    Rectangle::from_corners(
        [b.min.x as f64, b.min.y as f64],
        [b.max.x as f64, b.max.y as f64],
    )
}

fn bbox_to_aabb(b: Bbox) -> AABB<[f64; 2]> {
    if b.is_empty() {
        return AABB::from_corners([0.0, 0.0], [0.0, 0.0]);
    }
    AABB::from_corners(
        [b.min.x as f64, b.min.y as f64],
        [b.max.x as f64, b.max.y as f64],
    )
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
        let idx = RTreeIndex::build([
            (bb(0, 0, 10, 10), "a"),
            (bb(50, 50, 60, 60), "b"),
            (bb(20, 0, 30, 10), "c"),
        ]);
        let hits: Vec<&&str> = idx.query(bb(5, 5, 25, 5)).map(|(_, v)| v).collect();
        assert_eq!(hits.len(), 2);
        assert!(hits.contains(&&"a"));
        assert!(hits.contains(&&"c"));
    }

    #[test]
    fn empty_index() {
        let idx: RTreeIndex<&str> = RTreeIndex::build([]);
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
        let idx = RTreeIndex::build(items);
        assert_eq!(idx.len(), 1000);
        let hits: Vec<_> = idx.query(bb(0, 0, 50, 50)).collect();
        assert!(hits.len() <= 50);
        assert!(hits.len() >= 25);
    }

    #[test]
    fn contained_filter() {
        let idx = RTreeIndex::build([
            (bb(0, 0, 10, 10), "inside"),
            (bb(50, 50, 200, 200), "spans_edge"),
        ]);
        let q = bb(-100, -100, 100, 100);
        assert_eq!(idx.query(q).count(), 2);
        assert_eq!(idx.query_contained(q).count(), 1);
    }
}
