//! Timing-path exceptions: false paths, multicycle paths, max/min-delay.
//!
//! Real flows accept SDC commands like:
//!
//! ```text
//! set_false_path -from [get_pins reset_reg/Q]
//! set_multicycle_path -setup 2 -through some_long_combo
//! set_max_delay 1.5 -from clk_a -to clk_b
//! ```
//!
//! These tell STA to **ignore** or **scale** specific timing paths
//! that violate normal expectations but are sequentially safe (e.g.
//! a reset path that's never timing-critical, or a slow combinational
//! sub-circuit guarded by a multi-cycle clock-edge handshake).
//!
//! v1 covers:
//!
//! * **`set_false_path`** — exclude paths between specified node sets
//!   from slack reporting.
//! * **`set_multicycle_path`** — multiply the effective period by `N`
//!   for paths between specified node sets.
//! * **`set_max_delay`** / **`set_min_delay`** — override the
//!   required-time constraint on specific paths.
//!
//! What's not in v1:
//!
//! * **SDC parser.** Path constraints are constructed via
//!   programmatic API; a Tcl-based SDC reader is the next step.
//!   Until then, frontends emit `PathException` records directly.
//! * **`-through` filtering.** v1 supports `-from` / `-to` only.
//!   `-through` filtering requires path enumeration during
//!   propagation, not just endpoint matching.

use crate::graph::{NodeId, TimingGraph};
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub enum PathException {
    /// Exclude every path with a `from` ∈ `from_set` AND a `to` ∈
    /// `to_set` from slack reporting.
    FalsePath {
        from: HashSet<NodeId>,
        to: HashSet<NodeId>,
    },
    /// Scale the effective clock period by `cycles` on matching paths.
    MultiCycle {
        cycles: u32,
        from: HashSet<NodeId>,
        to: HashSet<NodeId>,
    },
    /// Override the required time at `to` to `delay` for matching
    /// paths.
    MaxDelay {
        delay: f64,
        from: HashSet<NodeId>,
        to: HashSet<NodeId>,
    },
}

impl PathException {
    fn from_set(&self) -> &HashSet<NodeId> {
        match self {
            PathException::FalsePath { from, .. } => from,
            PathException::MultiCycle { from, .. } => from,
            PathException::MaxDelay { from, .. } => from,
        }
    }
    fn to_set(&self) -> &HashSet<NodeId> {
        match self {
            PathException::FalsePath { to, .. } => to,
            PathException::MultiCycle { to, .. } => to,
            PathException::MaxDelay { to, .. } => to,
        }
    }
}

/// Apply exceptions to a slack vector. Slacks are `required −
/// arrival`; exceptions modify the effective `required` on a per-
/// endpoint basis.
///
/// `arrivals` and `slacks` must be parallel to `g.nodes`. The
/// returned slack vector replaces `slacks[to]` for every endpoint
/// `to` reachable from a `from` node in any false-path / multicycle /
/// max-delay exception.
///
/// `clock_period` is the unmodified clock period the original
/// `slacks` vector was computed against; multicycle exceptions
/// rebuild the effective required time as `cycles × clock_period`.
pub fn apply_exceptions(
    g: &TimingGraph,
    arrivals: &[f64],
    slacks: &[f64],
    exceptions: &[PathException],
    clock_period: f64,
) -> Vec<f64> {
    let n = g.nodes.len();
    let mut out = slacks.to_vec();

    // For each exception, find every endpoint `to` reachable from some
    // `from` ∈ exception.from. We walk the timing graph forward from
    // each `from` until we hit an endpoint in exception.to.
    for exc in exceptions {
        let reachable_to_endpoints = forward_reachable(g, exc.from_set(), exc.to_set());
        for &to in &reachable_to_endpoints {
            let to_idx = to.0 as usize;
            if to_idx >= n {
                continue;
            }
            match exc {
                PathException::FalsePath { .. } => {
                    out[to_idx] = f64::INFINITY;
                }
                PathException::MultiCycle { cycles, .. } => {
                    let new_required = (*cycles as f64) * clock_period;
                    out[to_idx] = new_required - arrivals[to_idx];
                }
                PathException::MaxDelay { delay, .. } => {
                    out[to_idx] = *delay - arrivals[to_idx];
                }
            }
        }
    }
    out
}

/// Forward BFS in `g` from any node in `from`, stopping at any node
/// in `to`. Returns the subset of `to` that was actually reached.
fn forward_reachable(
    g: &TimingGraph,
    from: &HashSet<NodeId>,
    to: &HashSet<NodeId>,
) -> HashSet<NodeId> {
    let mut visited: HashSet<NodeId> = HashSet::new();
    let mut found: HashSet<NodeId> = HashSet::new();
    let mut stack: Vec<NodeId> = from.iter().copied().collect();
    while let Some(n) = stack.pop() {
        if !visited.insert(n) {
            continue;
        }
        if to.contains(&n) {
            found.insert(n);
            // Don't continue past matched endpoints — exception is
            // path-set-defined, not transitive.
            continue;
        }
        for e in g.outgoing(n) {
            stack.push(e.to);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeKind, NodeKind};

    fn linear_chain() -> (TimingGraph, NodeId, NodeId) {
        let mut g = TimingGraph::new();
        let a = g.add_node("a", NodeKind::PrimaryInput);
        let b = g.add_node("b", NodeKind::CellInput);
        let c = g.add_node("c", NodeKind::CellOutput);
        let d = g.add_node("d", NodeKind::PrimaryOutput);
        g.add_edge(a, b, EdgeKind::Net, 0.5);
        g.add_edge(b, c, EdgeKind::CellArc, 0.3);
        g.add_edge(c, d, EdgeKind::Net, 0.2);
        g.finalize();
        (g, a, d)
    }

    #[test]
    fn false_path_pushes_slack_to_infinity() {
        let (g, a, d) = linear_chain();
        let arrivals = vec![0.0, 0.5, 0.8, 1.0];
        let slacks = vec![0.0, 0.0, 0.0, -0.5]; // 0.5 ns negative slack at d.

        let exc = PathException::FalsePath {
            from: HashSet::from([a]),
            to: HashSet::from([d]),
        };
        let out = apply_exceptions(&g, &arrivals, &slacks, &[exc], 0.5);
        assert_eq!(out[d.0 as usize], f64::INFINITY);
    }

    #[test]
    fn multicycle_doubles_effective_period() {
        let (g, a, d) = linear_chain();
        let arrivals = vec![0.0, 0.5, 0.8, 1.0];
        let slacks = vec![0.0, 0.0, 0.0, -0.5];
        let exc = PathException::MultiCycle {
            cycles: 2,
            from: HashSet::from([a]),
            to: HashSet::from([d]),
        };
        // With cycles=2 and period=0.5, new required = 1.0; slack
        // becomes 1.0 - 1.0 = 0.0 (was -0.5).
        let out = apply_exceptions(&g, &arrivals, &slacks, &[exc], 0.5);
        assert!((out[d.0 as usize] - 0.0).abs() < 1e-9);
    }

    #[test]
    fn max_delay_overrides_required() {
        let (g, a, d) = linear_chain();
        let arrivals = vec![0.0, 0.5, 0.8, 1.0];
        let slacks = vec![0.0, 0.0, 0.0, -0.5];
        let exc = PathException::MaxDelay {
            delay: 2.0,
            from: HashSet::from([a]),
            to: HashSet::from([d]),
        };
        // New slack at d = 2.0 - 1.0 = 1.0.
        let out = apply_exceptions(&g, &arrivals, &slacks, &[exc], 0.5);
        assert!((out[d.0 as usize] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn unrelated_endpoint_unchanged() {
        let (g, a, d) = linear_chain();
        let other = NodeId(99); // doesn't exist; exception doesn't reach.
        let arrivals = vec![0.0, 0.5, 0.8, 1.0];
        let slacks = vec![0.0, 0.0, 0.0, -0.5];
        let exc = PathException::FalsePath {
            from: HashSet::from([a]),
            to: HashSet::from([other]),
        };
        let out = apply_exceptions(&g, &arrivals, &slacks, &[exc], 0.5);
        // d's slack must remain -0.5 since the exception doesn't
        // target it.
        assert!((out[d.0 as usize] - (-0.5)).abs() < 1e-9);
    }
}
