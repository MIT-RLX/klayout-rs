//! Bundler: many port pairs → many planned paths, with bundle-level
//! constraints (length-matching, non-crossing).
//!
//! Three bundlers are provided:
//! * [`IdentityBundler`] — plans each pair independently. No
//!   bundle-level constraints. Cheap, fine for unrelated nets.
//! * [`LengthMatchedBundler`] — equalises path lengths within
//!   `tolerance` by injecting serpentine detours on the shorter
//!   paths. Used for clock-tree leaves, DDR address/data buses,
//!   high-speed parallel links.
//! * [`DiffPairBundler`] — paired routing with constant `gap` between
//!   the two paths. Each pair's `(p_port, n_port)` produces two paths
//!   that hug each other through the routing channel.

use crate::planner::{Obstacles, Planner};
use klayout_core::{Path, Point, Port};
use smallvec::SmallVec;

pub trait Bundler {
    fn bundle<P: Planner>(
        &self,
        planner: &P,
        pairs: &[(Port, Port)],
        env: &Obstacles,
    ) -> Vec<Path>;
}

pub struct IdentityBundler;

impl Bundler for IdentityBundler {
    fn bundle<P: Planner>(
        &self,
        planner: &P,
        pairs: &[(Port, Port)],
        env: &Obstacles,
    ) -> Vec<Path> {
        pairs.iter().map(|(s, d)| planner.plan(s, d, env)).collect()
    }
}

/// Length-match a bundle by inserting serpentine detours so every
/// path's length lands within `tolerance` of the longest path.
///
/// Strategy: plan each pair independently; measure path length; for
/// each path shorter than `target - tolerance`, add a U-shaped detour
/// (one extra bend pair) sized to bring it up to the target. The
/// detour is placed on the longest collinear segment of the path —
/// the U pokes perpendicular to the wire direction by enough to add
/// `(target - this_length)` to the total.
pub struct LengthMatchedBundler {
    pub tolerance: i64,
    /// Maximum perpendicular extent of a serpentine excursion. Caps
    /// the detour height to avoid ridiculous spikes.
    pub max_detour: i64,
}

impl Default for LengthMatchedBundler {
    fn default() -> Self {
        Self {
            tolerance: 1,
            max_detour: i64::MAX,
        }
    }
}

impl Bundler for LengthMatchedBundler {
    fn bundle<P: Planner>(
        &self,
        planner: &P,
        pairs: &[(Port, Port)],
        env: &Obstacles,
    ) -> Vec<Path> {
        let mut paths: Vec<Path> = pairs
            .iter()
            .map(|(s, d)| planner.plan(s, d, env))
            .collect();
        let max_len = paths.iter().map(path_length).max().unwrap_or(0);
        for p in paths.iter_mut() {
            let len = path_length(p);
            let deficit = max_len - len;
            if deficit > self.tolerance {
                inject_serpentine(p, deficit, self.max_detour);
            }
        }
        paths
    }
}

/// Differential-pair bundler. Each input pair `(p_port, n_port)` is
/// duplicated: the planner runs twice — once on the original pair,
/// once on a perpendicular-offset copy with `gap` between them. Both
/// resulting paths are emitted (interleaved: p0, n0, p1, n1, …).
pub struct DiffPairBundler {
    /// Perpendicular gap between the P and N legs (DBU).
    pub gap: i64,
}

impl Bundler for DiffPairBundler {
    fn bundle<P: Planner>(
        &self,
        planner: &P,
        pairs: &[(Port, Port)],
        env: &Obstacles,
    ) -> Vec<Path> {
        let mut out: Vec<Path> = Vec::with_capacity(pairs.len() * 2);
        for (p, n) in pairs {
            // P-path on the original ports.
            let p_path = planner.plan(p, n, env);
            out.push(p_path.clone());
            // N-path: shift both ports perpendicular to their angle by `gap`.
            let p_off = offset_port(p, self.gap);
            let n_off = offset_port(n, self.gap);
            let n_path = planner.plan(&p_off, &n_off, env);
            out.push(n_path);
        }
        out
    }
}

fn path_length(p: &Path) -> i64 {
    p.points
        .windows(2)
        .map(|w| (w[1].x - w[0].x).abs() + (w[1].y - w[0].y).abs())
        .sum()
}

fn inject_serpentine(p: &mut Path, deficit: i64, max_detour: i64) {
    if p.points.len() < 2 {
        return;
    }
    // Find the longest segment to put the detour on.
    let mut best_idx = 0;
    let mut best_len: i64 = -1;
    for (i, w) in p.points.windows(2).enumerate() {
        let len = (w[1].x - w[0].x).abs() + (w[1].y - w[0].y).abs();
        if len > best_len {
            best_len = len;
            best_idx = i;
        }
    }
    if best_len <= 0 {
        return;
    }
    let a = p.points[best_idx];
    let b = p.points[best_idx + 1];
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let mid_x = (a.x + b.x) / 2;
    let mid_y = (a.y + b.y) / 2;
    // Detour height = deficit/2 (round-trip), capped.
    let h = (deficit / 2).clamp(1, max_detour);
    // Perpendicular direction: rotate (dx, dy) by 90°.
    let (perp_x, perp_y) = if dx.abs() >= dy.abs() {
        (0, h * if dy >= 0 { 1 } else { -1 })
    } else {
        (h * if dx >= 0 { -1 } else { 1 }, 0)
    };
    let p1 = Point::new(mid_x, mid_y);
    let p2 = Point::new(mid_x + perp_x, mid_y + perp_y);
    let p3 = Point::new(mid_x + perp_x + dx.signum(), mid_y + perp_y + dy.signum());
    let p4 = Point::new(p3.x - perp_x, p3.y - perp_y);
    // Insert p1, p2, p3, p4 between a and b.
    let mut new_pts: SmallVec<[Point; 4]> = SmallVec::new();
    for (i, p) in p.points.iter().enumerate() {
        new_pts.push(*p);
        if i == best_idx {
            new_pts.push(p1);
            new_pts.push(p2);
            new_pts.push(p3);
            new_pts.push(p4);
        }
    }
    p.points = new_pts;
}

fn offset_port(port: &Port, gap: i64) -> Port {
    use klayout_core::Angle90;
    // Offset perpendicular to the port's outgoing direction.
    let (dx, dy) = match port.angle {
        Angle90::E | Angle90::W => (0, gap),
        Angle90::N | Angle90::S => (gap, 0),
    };
    Port {
        center: Point::new(port.center.x + dx, port.center.y + dy),
        ..port.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planner::ManhattanPlanner;
    use klayout_core::{Angle90, LayerInfo, Library, Point as P, Port};

    fn port(x: i64, y: i64, a: Angle90) -> Port {
        let lib = Library::new("t", 1);
        let layer = lib.layer(LayerInfo::gds(1, 0));
        Port::new("p", layer, P::new(x, y), a, 1)
    }

    #[test]
    fn identity_bundler_preserves_pair_count() {
        let b = IdentityBundler;
        let p = ManhattanPlanner;
        let env = Obstacles::default();
        let pairs = vec![
            (port(0, 0, Angle90::E), port(50, 0, Angle90::W)),
            (port(0, 10, Angle90::E), port(50, 10, Angle90::W)),
        ];
        let paths = b.bundle(&p, &pairs, &env);
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn length_matched_bundler_equalises_lengths() {
        let b = LengthMatchedBundler {
            tolerance: 0,
            max_detour: i64::MAX,
        };
        let p = ManhattanPlanner;
        let env = Obstacles::default();
        let pairs = vec![
            (port(0, 0, Angle90::E), port(20, 0, Angle90::W)), // len 20
            (port(0, 10, Angle90::E), port(60, 10, Angle90::W)), // len 60
        ];
        let paths = b.bundle(&p, &pairs, &env);
        assert_eq!(paths.len(), 2);
        let l0 = path_length(&paths[0]);
        let l1 = path_length(&paths[1]);
        // After length-matching, both paths should be ≈ 60.
        assert!((l0 - l1).abs() <= 4, "lengths {l0} vs {l1}");
        assert!(l0 >= 56);
    }

    #[test]
    fn length_matched_bundler_respects_max_detour() {
        let b = LengthMatchedBundler {
            tolerance: 0,
            max_detour: 2,
        };
        let p = ManhattanPlanner;
        let env = Obstacles::default();
        let pairs = vec![
            (port(0, 0, Angle90::E), port(20, 0, Angle90::W)),
            (port(0, 10, Angle90::E), port(200, 10, Angle90::W)),
        ];
        let paths = b.bundle(&p, &pairs, &env);
        // Capped detour can't fully length-match a huge gap.
        let l0 = path_length(&paths[0]);
        assert!(l0 < 200, "max_detour=2 should cap padding");
    }

    #[test]
    fn diff_pair_bundler_emits_two_paths_per_pair() {
        let b = DiffPairBundler { gap: 4 };
        let p = ManhattanPlanner;
        let env = Obstacles::default();
        let pairs = vec![(port(0, 0, Angle90::E), port(50, 0, Angle90::W))];
        let paths = b.bundle(&p, &pairs, &env);
        assert_eq!(paths.len(), 2);
        // The N-path should be offset by `gap` from the P-path.
        let p_first = paths[0].points[0];
        let n_first = paths[1].points[0];
        let dx = (p_first.x - n_first.x).abs();
        let dy = (p_first.y - n_first.y).abs();
        assert_eq!(dx + dy, 4);
    }
}
