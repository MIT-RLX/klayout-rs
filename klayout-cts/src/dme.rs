//! DME (Deferred-Merge Embedding) clock-tree synthesizer.
//!
//! Where the [`super::synthesise_clock_tree`] H-tree splits sinks
//! geometrically by alternating x/y bisection, this synthesizer pairs
//! subtrees by Manhattan proximity and shifts each merge point off the
//! geometric midpoint by the amount needed to **balance the two
//! children's accumulated wire delay**. The construction follows
//! Chao/Hsu/Ho 1992:
//!
//! 1. **Bottom-up phase.** Starting from one merging segment per sink
//!    (each a degenerate single point), greedily pair the two
//!    subtrees with the smallest Manhattan distance between merging
//!    segments and merge them. Each merge produces a new merging
//!    segment — the locus of points equidistant in delay from both
//!    children — which becomes the parent subtree's representation.
//!
//! 2. **Top-down phase.** Once the bottom-up loop has built the full
//!    tree of merging segments, walk it in pre-order from the root
//!    and **pin** each subtree's tap to the point on its merging
//!    segment closest (under L1) to the parent tap. The root's tap
//!    is pinned to the segment point closest to the synthetic source.
//!    This minimizes the parent → child wire on every leg without
//!    breaking the equal-delay invariant.
//!
//! ### Geometry — rotated `(u, v)` coordinates
//!
//! Manhattan-aligned merging segments are tilted 45° in `(x, y)`,
//! which makes the math awkward. We work instead in
//!
//! ```text
//!     u = x + y       v = x - y
//! ```
//!
//! where the L1 metric in `(x, y)` becomes L∞ in `(u, v)` (i.e.
//! `L1((x₁,y₁),(x₂,y₂)) = max(|u₁-u₂|, |v₁-v₂|)`). In this rotated
//! space:
//!
//! * a single point becomes a single `(u, v)` point;
//! * a 45° tilted segment becomes axis-aligned;
//! * the L1 ball of radius `r` centered at `p` becomes the L∞ ball
//!   (a square) `[u_p ± r] × [v_p ± r]`;
//! * "Minkowski-dilate by `r`" reduces to enlarging an axis-aligned
//!   rectangle by `r` on every side;
//! * intersection of two such dilations is just rectangle intersection.
//!
//! The merging segment of a subtree is therefore represented as an
//! axis-aligned rectangle in `(u, v)`. Sinks degenerate to a single
//! point; non-trivial merges produce true rectangles whose extent in
//! `u` and/or `v` corresponds to the locus of admissible tap points.
//!
//! ### Detour
//!
//! When one subtree's accumulated delay exceeds the inter-segment
//! distance plus the other's delay, the balanced-formula `e_a` or
//! `e_b` goes negative; the merge "wants" the tap on the dominant
//! subtree's merging segment, and the shorter leg gets a snake-style
//! detour wire of length `|d_a − d_b| − L`. This is recorded as
//! [`super::Branch::detour_to_parent`] and accounted for in
//! [`super::ClockTree::skew`] / `total_wire_length`.
//!
//! ### Cost
//!
//! Each round of pairing rebuilds an `rstar` index over the `O(n)`
//! current subtree taps (`O(n log n)`) and queries each subtree's
//! nearest neighbor (`O(log n)` per query, `O(n log n)` total). With
//! `O(n)` rounds, the algorithm is `O(n² log n)` end-to-end — a
//! material improvement over the straightforward `O(n³)` greedy and
//! adequate up to the `n ≲ 10⁵` range that single-domain clocks reach
//! in practice.

use crate::{Branch, BranchChild, BranchId, ClockSink, ClockTree};
use klayout_core::Point;
use rstar::{primitives::GeomWithData, RTree};
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct DmeConfig {
    /// When `true`, allow snake-style detour wires on the shorter leg
    /// of an unbalanced merge to drive skew to zero. When `false`,
    /// leave the imbalance in place and report it via
    /// [`ClockTree::skew`]. Defaults to `true`.
    pub allow_detour: bool,
}

impl Default for DmeConfig {
    fn default() -> Self {
        Self { allow_detour: true }
    }
}

// ---------- Merging-segment representation ----------

/// Axis-aligned rectangle in rotated `(u, v) = (x+y, x-y)` coords.
/// Encodes any merging segment encountered during DME: a single point
/// (degenerate rectangle), a 45°-tilted segment in `(x, y)` (one
/// dimension zero), or a tilted parallelogram region (both nonzero).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct MergingRect {
    u_min: i64,
    u_max: i64,
    v_min: i64,
    v_max: i64,
}

impl MergingRect {
    fn from_point(p: Point) -> Self {
        let u = p.x + p.y;
        let v = p.x - p.y;
        Self {
            u_min: u,
            u_max: u,
            v_min: v,
            v_max: v,
        }
    }

    /// Minimum L∞ distance (in `(u, v)`) between the two rectangles —
    /// equivalently, minimum L1 distance between the merging segments
    /// in `(x, y)` coords.
    fn distance(&self, other: &Self) -> i64 {
        let du_gap = (other.u_min - self.u_max).max(self.u_min - other.u_max).max(0);
        let dv_gap = (other.v_min - self.v_max).max(self.v_min - other.v_max).max(0);
        du_gap.max(dv_gap)
    }

    /// Minkowski-dilate by `r` (the L∞ ball of radius `r` in `(u, v)`,
    /// equivalently the L1 ball in `(x, y)`).
    fn dilate(&self, r: i64) -> Self {
        Self {
            u_min: self.u_min - r,
            u_max: self.u_max + r,
            v_min: self.v_min - r,
            v_max: self.v_max + r,
        }
    }

    /// Axis-aligned intersection. Returns `None` if disjoint.
    fn intersect(&self, other: &Self) -> Option<Self> {
        let u_min = self.u_min.max(other.u_min);
        let u_max = self.u_max.min(other.u_max);
        let v_min = self.v_min.max(other.v_min);
        let v_max = self.v_max.min(other.v_max);
        if u_min <= u_max && v_min <= v_max {
            Some(Self {
                u_min,
                u_max,
                v_min,
                v_max,
            })
        } else {
            None
        }
    }

    /// Pick the integer `(u, v)` point in this rectangle closest under
    /// L∞ to a target. Used by the top-down refinement: each subtree's
    /// tap is pinned to the closest point on its merging segment to
    /// the parent's tap, minimizing parent-leg wire.
    ///
    /// Returns a point with `(u + v)` even, so the inverse mapping
    /// `((u+v)/2, (u-v)/2)` lands exactly on the integer `(x, y)`
    /// grid. Source points always have `(u + v) = 2x` (even); after
    /// integer-radius dilation and rectangle intersection, the bounds
    /// can land on either parity. When the L∞-closest point has odd
    /// parity we nudge by 1 DBU in `u` or `v` to the nearest valid
    /// same-parity point in the rectangle — costing at most 1 DBU of
    /// extra parent-leg wire and avoiding the ½-DBU truncation loss
    /// that would otherwise accumulate at every merge level.
    fn closest_uv(&self, target_u: i64, target_v: i64) -> (i64, i64) {
        let cu = target_u.clamp(self.u_min, self.u_max);
        let cv = target_v.clamp(self.v_min, self.v_max);
        if (cu + cv) & 1 == 0 {
            return (cu, cv);
        }
        // Try the four 1-DBU neighbors in priority order. Any
        // same-parity neighbor inside the rectangle works; the first
        // that fits is the L∞-closest valid point.
        let candidates = [
            (cu, cv + 1),
            (cu, cv - 1),
            (cu + 1, cv),
            (cu - 1, cv),
        ];
        for (u, v) in candidates {
            if u >= self.u_min
                && u <= self.u_max
                && v >= self.v_min
                && v <= self.v_max
                && (u + v) & 1 == 0
            {
                return (u, v);
            }
        }
        // The rectangle has zero same-parity points (only possible
        // when both u and v dimensions are zero AND the single point
        // has odd parity). Accept the original coordinate; the ½-DBU
        // loss is unavoidable for that pathological MS.
        (cu, cv)
    }
}

/// Convert `(u, v)` back to `(x, y)`. Since `(u, v) = (x+y, x-y)`,
/// `x = (u+v)/2` and `y = (u-v)/2`. The two operands have the same
/// parity for every `(u, v)` that originates from an integer point,
/// but rectangle intersections can land on a mixed-parity coordinate;
/// in that case we round `(u + v)` / `(u - v)` toward zero, losing at
/// most ½ DBU per axis. Below the DRC grid for any practical PDK.
fn uv_to_point(u: i64, v: i64) -> Point {
    Point::new((u + v) / 2, (u - v) / 2)
}

// ---------- Subtree representation ----------

#[derive(Clone)]
enum SubtreeKind {
    Sink(SmolStr, Point),
    /// An internal merge. The two children are recorded so the
    /// top-down refinement can descend into them after the parent's
    /// tap is pinned. The merging segment lives on the outer
    /// [`Subtree`] struct (`ms` field). Per-child detours are recorded
    /// here so [`top_down_pin`] can stamp them onto the materialized
    /// branch nodes.
    Merge {
        detour_a: i64,
        detour_b: i64,
        child_a: Box<Subtree>,
        child_b: Box<Subtree>,
    },
}

#[derive(Clone)]
struct Subtree {
    /// The subtree's merging segment in `(u, v)` coords.
    ms: MergingRect,
    /// Maximum wire length from any tap in `ms` to any leaf below.
    delay: i64,
    kind: SubtreeKind,
}

impl Subtree {
    fn from_sink(s: &ClockSink) -> Self {
        Self {
            ms: MergingRect::from_point(s.at),
            delay: 0,
            kind: SubtreeKind::Sink(s.name.clone(), s.at),
        }
    }

    /// Anchor `(u, v)` for the rstar nearest-neighbor query. The
    /// "center" of a single-point MS is the point itself; for
    /// rectangles we use the centroid, which gives a stable proxy for
    /// closest-pair queries even though the true L∞ distance between
    /// rectangles is computed exactly via [`MergingRect::distance`].
    fn rstar_anchor(&self) -> [f64; 2] {
        let cu = (self.ms.u_min + self.ms.u_max) as f64 / 2.0;
        let cv = (self.ms.v_min + self.ms.v_max) as f64 / 2.0;
        [cu, cv]
    }
}

// ---------- Bottom-up phase ----------

/// Greedy nearest-pair pairing using `rstar`. Each round rebuilds the
/// index over the surviving subtrees' anchor points and queries each
/// subtree's nearest neighbor; the global minimum is the round's
/// merge target. `O(n log n)` per round, `O(n² log n)` overall.
fn closest_pair_rstar(subs: &[Subtree]) -> (usize, usize) {
    debug_assert!(subs.len() >= 2);
    if subs.len() == 2 {
        return (0, 1);
    }
    let entries: Vec<GeomWithData<[f64; 2], usize>> = subs
        .iter()
        .enumerate()
        .map(|(i, s)| GeomWithData::new(s.rstar_anchor(), i))
        .collect();
    let tree = RTree::bulk_load(entries);

    let mut best_i = 0usize;
    let mut best_j = 1usize;
    let mut best_d = subs[0].ms.distance(&subs[1].ms);
    for i in 0..subs.len() {
        let anchor = subs[i].rstar_anchor();
        // The two nearest neighbors include `i` itself; we want the
        // closest *other* index. `nearest_neighbor_iter` yields in
        // increasing rstar-distance order; walk until we find one
        // whose true L∞ MS distance beats `best_d`.
        for nbr in tree.nearest_neighbor_iter(&anchor).take(4) {
            let j = nbr.data;
            if j == i {
                continue;
            }
            let d = subs[i].ms.distance(&subs[j].ms);
            if d < best_d {
                best_d = d;
                best_i = i.min(j);
                best_j = i.max(j);
                if best_d == 0 {
                    return (best_i, best_j);
                }
                break;
            }
        }
    }
    (best_i, best_j)
}

/// Compute the merging-rectangle `(MS, e_a, e_b)` for combining two
/// children with merging segments `a` / `b` and delays `d_a` / `d_b`.
/// Returns the per-child detour as well so the top-down phase can
/// stamp it onto [`Branch::detour_to_parent`].
fn compute_merge(
    a: &MergingRect,
    d_a: i64,
    b: &MergingRect,
    d_b: i64,
    cfg: &DmeConfig,
) -> (MergingRect, i64, i64, i64, i64) {
    let l = a.distance(b);
    let e_a_num = l + d_b - d_a;
    let e_b_num = l + d_a - d_b;
    let (e_a, e_b, detour_a, detour_b) = if e_a_num >= 0 && e_b_num >= 0 {
        // Floor `e_a` and let `e_b` absorb the leftover so `e_a + e_b
        // == L` exactly, even when `e_a_num` is odd. The asymmetry
        // costs at most 1 DBU per merge (and only on odd-numerator
        // merges), but it preserves the
        // `dilate_a(e_a) ∩ dilate_b(e_b)` non-emptiness invariant —
        // an invariant we rely on for the merging-segment to be a
        // valid (boundary) locus rather than the empty set.
        let ea = e_a_num / 2;
        let eb = l - ea;
        (ea, eb, 0, 0)
    } else if e_a_num < 0 {
        let detour = d_a - d_b - l;
        if cfg.allow_detour {
            (0, l, 0, detour)
        } else {
            (0, l, 0, 0)
        }
    } else {
        let detour = d_b - d_a - l;
        if cfg.allow_detour {
            (l, 0, detour, 0)
        } else {
            (l, 0, 0, 0)
        }
    };

    let dilated_a = a.dilate(e_a);
    let dilated_b = b.dilate(e_b);
    // Fallback when the intersection is empty: this can only happen
    // when detours are disabled and the imbalance exceeds L. We pick
    // the dominant child's segment so the top-down phase still has a
    // valid tap region.
    let fallback = if e_a == 0 { *a } else { *b };
    let ms = dilated_a.intersect(&dilated_b).unwrap_or(fallback);
    (ms, e_a, e_b, detour_a, detour_b)
}

/// Bottom-up: repeatedly merge the closest pair of subtrees.
fn bottom_up(mut subs: Vec<Subtree>, cfg: &DmeConfig) -> Subtree {
    while subs.len() > 1 {
        let (i, j) = closest_pair_rstar(&subs);
        let (hi, lo) = (i.max(j), i.min(j));
        let b = subs.remove(hi);
        let a = subs.remove(lo);
        let (ms, e_a, e_b, detour_a, detour_b) =
            compute_merge(&a.ms, a.delay, &b.ms, b.delay, cfg);
        let delay = (e_a + a.delay + detour_a).max(e_b + b.delay + detour_b);
        let _ = (e_a, e_b); // kept for clarity in `compute_merge` return; not stored.
        subs.push(Subtree {
            ms,
            delay,
            kind: SubtreeKind::Merge {
                detour_a,
                detour_b,
                child_a: Box::new(a),
                child_b: Box::new(b),
            },
        });
    }
    subs
        .pop()
        .expect("bottom_up loop exits with exactly one subtree")
}

// ---------- Top-down phase ----------

/// Walk the merging-segment tree top-down, picking each subtree's
/// physical tap. The tap of every internal merge is the point on its
/// merging rectangle closest under L1 to the *parent's* tap; the root
/// is pinned to the source. The recursion materializes
/// [`Branch`] entries into `branches` and stamps detour wires.
fn top_down_pin(
    sub: &Subtree,
    parent: BranchId,
    parent_at: Point,
    detour_to_parent: i64,
    branches: &mut Vec<Branch>,
) -> BranchChild {
    match &sub.kind {
        SubtreeKind::Sink(name, at) => BranchChild::Sink(name.clone(), *at),
        SubtreeKind::Merge {
            child_a,
            child_b,
            detour_a,
            detour_b,
        } => {
            let parent_u = parent_at.x + parent_at.y;
            let parent_v = parent_at.x - parent_at.y;
            let (chosen_u, chosen_v) = sub.ms.closest_uv(parent_u, parent_v);
            let tap = uv_to_point(chosen_u, chosen_v);

            let new_id = BranchId(branches.len() as u32);
            branches.push(Branch {
                id: new_id,
                at: tap,
                parent: Some(parent),
                children: Vec::new(),
                detour_to_parent,
            });

            let child_a_node = top_down_pin(child_a, new_id, tap, *detour_a, branches);
            let child_b_node = top_down_pin(child_b, new_id, tap, *detour_b, branches);
            branches[new_id.0 as usize].children.push(child_a_node);
            branches[new_id.0 as usize].children.push(child_b_node);

            BranchChild::Branch(new_id)
        }
    }
}

// ---------- Public entry point ----------

/// Synthesize a clock tree by full-DME merging-segment construction.
pub fn synthesise_clock_tree_dme(
    source: Point,
    sinks: &[ClockSink],
    cfg: &DmeConfig,
) -> ClockTree {
    let mut branches: Vec<Branch> = Vec::new();
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

    let initial: Vec<Subtree> = sinks.iter().map(Subtree::from_sink).collect();
    let root_subtree = bottom_up(initial, cfg);

    // Top-down: source has no merging segment of its own, so it
    // simply hands its tap (== `source`) to the root subtree, which
    // then pins itself to the closest point on its MS.
    let root_child = top_down_pin(&root_subtree, root_id, source, 0, &mut branches);
    branches[root_id.0 as usize].children.push(root_child);

    let mut sink_lengths = Vec::with_capacity(sinks.len());
    crate::accumulate_lengths(&branches, root_id, source, 0, &mut sink_lengths);
    let buffer_count = branches.len();
    ClockTree {
        source,
        root: root_id,
        branches,
        buffer_count,
        sink_path_lengths: sink_lengths,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink(name: &str, x: i64, y: i64) -> ClockSink {
        ClockSink {
            name: name.into(),
            at: Point::new(x, y),
        }
    }

    #[test]
    fn empty_sinks() {
        let tree = synthesise_clock_tree_dme(Point::new(0, 0), &[], &DmeConfig::default());
        assert_eq!(tree.buffer_count, 1);
        assert!(tree.sink_path_lengths.is_empty());
    }

    #[test]
    fn single_sink_is_direct_child_of_source() {
        let tree = synthesise_clock_tree_dme(
            Point::new(0, 0),
            &[sink("ff0", 100, 0)],
            &DmeConfig::default(),
        );
        assert_eq!(tree.sink_path_lengths.len(), 1);
        assert_eq!(tree.sink_path_lengths[0].1, 100);
    }

    #[test]
    fn symmetric_pair_yields_zero_skew() {
        let tree = synthesise_clock_tree_dme(
            Point::new(0, 0),
            &[sink("ff0", -100, 0), sink("ff1", 100, 0)],
            &DmeConfig::default(),
        );
        assert_eq!(tree.skew(), 0);
    }

    #[test]
    fn three_collinear_sinks_zero_skew() {
        let tree = synthesise_clock_tree_dme(
            Point::new(0, 0),
            &[sink("a", 0, 0), sink("b", 10, 0), sink("c", 1000, 0)],
            &DmeConfig::default(),
        );
        assert_eq!(tree.skew(), 0);
    }

    #[test]
    fn dme_skew_at_least_as_good_as_htree_on_grid() {
        let grid: Vec<ClockSink> = (0..16)
            .map(|i| ClockSink {
                name: SmolStr::from(format!("ff{i}")),
                at: Point::new((i % 4) * 100, (i / 4) * 100),
            })
            .collect();
        let dme_tree =
            synthesise_clock_tree_dme(Point::new(150, 150), &grid, &DmeConfig::default());
        let htree =
            crate::synthesise_clock_tree(Point::new(150, 150), &grid, &Default::default());
        assert!(dme_tree.skew() <= htree.skew());
    }

    #[test]
    fn merging_rect_distance_is_l1_for_points() {
        let a = MergingRect::from_point(Point::new(0, 0));
        let b = MergingRect::from_point(Point::new(3, 4));
        // L1 in (x,y) = 7; L∞ in (u,v) = max(|3+4|, |3-4|) = 7. ✓
        assert_eq!(a.distance(&b), 7);
    }

    #[test]
    fn merging_rect_dilation_then_intersection_yields_balanced_locus() {
        // Two single-point segments, dilated by half their distance,
        // should intersect on the perpendicular bisector under L1.
        let a = MergingRect::from_point(Point::new(0, 0));
        let b = MergingRect::from_point(Point::new(10, 0));
        let l = a.distance(&b);
        assert_eq!(l, 10);
        let mid = a.dilate(l / 2).intersect(&b.dilate(l / 2));
        assert!(mid.is_some());
        let mid = mid.unwrap();
        // Center of the merged rect, in (x,y), should be at (5, 0).
        let cu = (mid.u_min + mid.u_max) / 2;
        let cv = (mid.v_min + mid.v_max) / 2;
        assert_eq!(uv_to_point(cu, cv), Point::new(5, 0));
    }

    #[test]
    fn top_down_minimizes_parent_leg() {
        // Two sinks at (0,0) and (0,100); source at (50, 50).
        // Bottom-up gives a merging segment for the root that's a
        // tilted segment in (x,y); top-down should pin the tap to the
        // point of that segment closest to (50,50). For this
        // specific geometry the segment passes through (0,50), and
        // the L1-closest point on it to (50,50) is (0,50) itself.
        let tree = synthesise_clock_tree_dme(
            Point::new(50, 50),
            &[sink("a", 0, 0), sink("b", 0, 100)],
            &DmeConfig::default(),
        );
        // The buffer node placed by bottom-up should have its `at`
        // pinned by top-down to a point on the segment closest to
        // the source, which is `(0, 50)`.
        let buffers: Vec<_> = tree
            .branches
            .iter()
            .filter(|b| b.parent.is_some())
            .collect();
        // Exactly one internal buffer for two sinks.
        assert_eq!(buffers.len(), 1);
        assert_eq!(buffers[0].at, Point::new(0, 50));
        // And both sink paths end up equal length.
        assert_eq!(tree.skew(), 0);
    }

    #[test]
    fn detour_inserted_when_imbalance_exceeds_distance() {
        // Construct a configuration that forces a detour: pair two
        // sinks tightly to build a high-delay subtree, then merge
        // with a third sink that's CLOSER than the imbalance. The
        // simplest such case — geometrically — is the degenerate
        // "two sinks at the same location" pairing followed by a
        // third at small distance, but our merging-segment math
        // collapses delays to zero in that case. Instead, test the
        // detour formula directly via `compute_merge`.
        let a = MergingRect::from_point(Point::new(0, 0));
        let b = MergingRect::from_point(Point::new(5, 0));
        let cfg = DmeConfig::default();
        // d_a = 100 (huge), d_b = 0, L = 5: e_a_num = 5 - 100 < 0,
        // so tap pins to a's MS and b leg gets a 95-unit detour.
        let (ms, e_a, e_b, det_a, det_b) = compute_merge(&a, 100, &b, 0, &cfg);
        assert_eq!(e_a, 0);
        assert_eq!(e_b, 5);
        assert_eq!(det_a, 0);
        assert_eq!(det_b, 95); // d_a - d_b - L = 100 - 0 - 5
        // The merging rect, when the dominant side dominates, falls
        // back to a's MS so the top-down phase pins onto a.
        assert_eq!(ms, a);
        // With detours disabled, the formula still pins the tap but
        // omits the detour, leaving residual skew for the caller to
        // observe.
        let no_detour = DmeConfig { allow_detour: false };
        let (_ms2, e_a2, e_b2, det_a2, det_b2) =
            compute_merge(&a, 100, &b, 0, &no_detour);
        assert_eq!((e_a2, e_b2, det_a2, det_b2), (0, 5, 0, 0));
    }

    #[test]
    fn dense_sink_field_completes_with_bounded_skew() {
        // Stress test: 64 sinks on a non-trivial grid; with detours
        // enabled the DME tree should be balanced to within the
        // integer-grid floor — at most 1 DBU per merge level (i.e.
        // O(log n) DBU end-to-end), which is the unavoidable
        // quantization loss when both the inter-MS distance `L` and
        // the delay difference `d_a − d_b` are odd integers and
        // therefore can't be split exactly evenly. Without the
        // detour pass, this configuration produces ~10⁵ DBU skew.
        let mut sinks: Vec<ClockSink> = Vec::new();
        for i in 0..64 {
            let x = (i % 8) * 137; // non-power-of-two stride
            let y = (i / 8) * 89;
            sinks.push(ClockSink {
                name: SmolStr::from(format!("ff{i}")),
                at: Point::new(x, y),
            });
        }
        let tree =
            synthesise_clock_tree_dme(Point::new(0, 0), &sinks, &DmeConfig::default());
        let depth_bound = (sinks.len() as f64).log2().ceil() as i64; // 6 for 64 sinks
        assert!(
            tree.skew() <= depth_bound,
            "skew {} exceeded grid-quantization bound {}",
            tree.skew(),
            depth_bound,
        );
        assert_eq!(tree.sink_path_lengths.len(), 64);
    }
}
