//! `klayout-cts` — clock tree synthesis.
//!
//! Build a balanced clock distribution tree from one source to N
//! sinks (FF clock pins). v1 implements a recursive-bisection (CRPR-
//! agnostic) method-of-means H-tree:
//!
//! 1. Recursively partition the sink set by the longer dimension of
//!    its bbox; place one buffer at the midpoint of each partition's
//!    bbox.
//! 2. Connect parent buffer → child buffers / sinks via straight
//!    routes; track per-leaf path length.
//! 3. Length-match leaves by computing each leaf's accumulated path
//!    length and either inserting a serpentine on shorter paths or
//!    moving the buffer toward the longer leaves (we use the simpler
//!    "report skew, leave caller to compensate" approach in v1).
//!
//! The result is a [`ClockTree`] of `Branch` nodes with explicit
//! buffer locations and parent/child relationships. A consumer can
//! convert it to routing requests (one per buffer-to-buffer or
//! buffer-to-sink leg) and feed those into the detailed router.

pub mod buffer;
pub mod dme;
pub use buffer::{
    buffer_insert, Buffer, BufferPlacement, Candidate, InsertConfig, ReceiverPin, RouteTreeChild,
    RouteTreeNode,
};
pub use dme::{synthesise_clock_tree_dme, DmeConfig};

use klayout_core::{Bbox, Point};
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct ClockSink {
    pub name: SmolStr,
    pub at: Point,
}

#[derive(Clone, Debug)]
pub struct ClockTree {
    pub source: Point,
    pub root: BranchId,
    pub branches: Vec<Branch>,
    pub buffer_count: usize,
    /// Per-sink path length from source. Used to compute skew.
    pub sink_path_lengths: Vec<(SmolStr, i64)>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct BranchId(pub u32);

#[derive(Clone, Debug)]
pub struct Branch {
    pub id: BranchId,
    /// Buffer location (None for the synthetic source / leaf node).
    pub at: Point,
    /// Parent branch (`None` only on root).
    pub parent: Option<BranchId>,
    pub children: Vec<BranchChild>,
    /// Extra wire length added to the parent → this branch leg beyond
    /// the Manhattan distance between the two points. Used by DME
    /// synthesis to insert a "snake" detour when one subtree's
    /// accumulated delay would otherwise exceed the merge tap point;
    /// always `0` for the recursive-bisection H-tree path.
    pub detour_to_parent: i64,
}

#[derive(Clone, Debug)]
pub enum BranchChild {
    Branch(BranchId),
    Sink(SmolStr, Point),
}

#[derive(Clone, Debug)]
pub struct CtsConfig {
    /// Recurse until each leaf branch has at most this many sinks.
    pub max_fanout: usize,
    /// Stop recursion when bbox dimension drops below this.
    pub min_bbox: i64,
}

impl Default for CtsConfig {
    fn default() -> Self {
        Self {
            max_fanout: 4,
            min_bbox: 0,
        }
    }
}

pub fn synthesise_clock_tree(
    source: Point,
    sinks: &[ClockSink],
    cfg: &CtsConfig,
) -> ClockTree {
    let mut branches: Vec<Branch> = Vec::new();
    // Root branch holds the source.
    let root_id = BranchId(0);
    branches.push(Branch {
        id: root_id,
        at: source,
        parent: None,
        children: Vec::new(),
        detour_to_parent: 0,
    });
    if sinks.is_empty() {
        return ClockTree {
            source,
            root: root_id,
            branches,
            buffer_count: 1,
            sink_path_lengths: Vec::new(),
        };
    }
    build_recursive(&mut branches, root_id, sinks, cfg);
    let mut sink_lengths = Vec::with_capacity(sinks.len());
    accumulate_lengths(&branches, root_id, source, 0, &mut sink_lengths);
    let buffer_count = branches.len();
    ClockTree {
        source,
        root: root_id,
        branches,
        buffer_count,
        sink_path_lengths: sink_lengths,
    }
}

impl ClockTree {
    /// Maximum minus minimum sink path length — the worst-case skew.
    pub fn skew(&self) -> i64 {
        if self.sink_path_lengths.len() < 2 {
            return 0;
        }
        let mn = self.sink_path_lengths.iter().map(|(_, l)| *l).min();
        let mx = self.sink_path_lengths.iter().map(|(_, l)| *l).max();
        match (mn, mx) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        }
    }

    pub fn total_wire_length(&self) -> i64 {
        let mut total = 0;
        for b in &self.branches {
            for child in &b.children {
                let (dst, detour) = match child {
                    BranchChild::Branch(id) => {
                        let cb = &self.branches[id.0 as usize];
                        (cb.at, cb.detour_to_parent)
                    }
                    BranchChild::Sink(_, p) => (*p, 0),
                };
                total += (dst.x - b.at.x).abs() + (dst.y - b.at.y).abs() + detour;
            }
        }
        total
    }
}

fn build_recursive(
    branches: &mut Vec<Branch>,
    parent: BranchId,
    sinks: &[ClockSink],
    cfg: &CtsConfig,
) {
    if sinks.len() <= cfg.max_fanout {
        for s in sinks {
            branches[parent.0 as usize]
                .children
                .push(BranchChild::Sink(s.name.clone(), s.at));
        }
        return;
    }
    // Bisect by longer dimension of sinks' bbox.
    let mut bb = Bbox::EMPTY;
    for s in sinks {
        bb = bb.union(&Bbox::new(s.at, s.at));
    }
    let horiz_split = bb.width() >= bb.height();
    let mid = if horiz_split {
        (bb.min.x + bb.max.x) / 2
    } else {
        (bb.min.y + bb.max.y) / 2
    };
    let (left, right): (Vec<_>, Vec<_>) = sinks.iter().cloned().partition(|s| {
        if horiz_split {
            s.at.x <= mid
        } else {
            s.at.y <= mid
        }
    });

    for partition in [left, right] {
        if partition.is_empty() {
            continue;
        }
        // Place a child buffer at the partition's bbox center.
        let mut pbb = Bbox::EMPTY;
        for s in &partition {
            pbb = pbb.union(&Bbox::new(s.at, s.at));
        }
        let buf_at = Point::new((pbb.min.x + pbb.max.x) / 2, (pbb.min.y + pbb.max.y) / 2);
        let child_id = BranchId(branches.len() as u32);
        branches.push(Branch {
            id: child_id,
            at: buf_at,
            parent: Some(parent),
            children: Vec::new(),
            detour_to_parent: 0,
        });
        branches[parent.0 as usize]
            .children
            .push(BranchChild::Branch(child_id));
        build_recursive(branches, child_id, &partition, cfg);
    }
}

pub(crate) fn accumulate_lengths(
    branches: &[Branch],
    cur: BranchId,
    cur_at: Point,
    cur_len: i64,
    out: &mut Vec<(SmolStr, i64)>,
) {
    for child in &branches[cur.0 as usize].children {
        match child {
            BranchChild::Branch(id) => {
                let next = &branches[id.0 as usize];
                let leg = (next.at.x - cur_at.x).abs()
                    + (next.at.y - cur_at.y).abs()
                    + next.detour_to_parent;
                accumulate_lengths(branches, *id, next.at, cur_len + leg, out);
            }
            BranchChild::Sink(name, p) => {
                let leg = (p.x - cur_at.x).abs() + (p.y - cur_at.y).abs();
                out.push((name.clone(), cur_len + leg));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_sink_no_branches() {
        let tree = synthesise_clock_tree(
            Point::new(0, 0),
            &[ClockSink {
                name: "ff0".into(),
                at: Point::new(100, 0),
            }],
            &CtsConfig::default(),
        );
        assert_eq!(tree.buffer_count, 1);
        assert_eq!(tree.sink_path_lengths.len(), 1);
        assert_eq!(tree.sink_path_lengths[0].1, 100);
    }

    #[test]
    fn many_sinks_partition_into_subtree() {
        let sinks: Vec<ClockSink> = (0..16)
            .map(|i| ClockSink {
                name: SmolStr::from(format!("ff{i}")),
                at: Point::new((i % 4) * 100, (i / 4) * 100),
            })
            .collect();
        let tree = synthesise_clock_tree(Point::new(0, 0), &sinks, &CtsConfig::default());
        // With max_fanout=4 and 16 sinks, we expect ≥ 5 buffers
        // (root + 4 leaf buffers each with 4 sinks).
        assert!(tree.buffer_count >= 5);
        assert_eq!(tree.sink_path_lengths.len(), 16);
    }

    #[test]
    fn skew_bounded_by_bbox_diameter() {
        let sinks: Vec<ClockSink> = (0..8)
            .map(|i| ClockSink {
                name: SmolStr::from(format!("ff{i}")),
                at: Point::new(i * 50, 0),
            })
            .collect();
        let tree = synthesise_clock_tree(Point::new(150, 0), &sinks, &CtsConfig::default());
        let bbox_diameter = 8 * 50;
        assert!(tree.skew() <= bbox_diameter);
    }

    #[test]
    fn total_wire_length_positive() {
        let sinks: Vec<ClockSink> = (0..4)
            .map(|i| ClockSink {
                name: SmolStr::from(format!("ff{i}")),
                at: Point::new(i * 100, i * 100),
            })
            .collect();
        let tree = synthesise_clock_tree(Point::new(0, 0), &sinks, &CtsConfig::default());
        assert!(tree.total_wire_length() > 0);
    }
}
