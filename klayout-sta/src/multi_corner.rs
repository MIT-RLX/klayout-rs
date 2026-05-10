//! Multi-corner / multi-mode STA + OCV derating.
//!
//! Production sign-off requires multiple PVT (process/voltage/temp)
//! corners — typically tt/ff/ss × low/high voltage × low/high temp.
//! Each corner has its own per-arc delay multiplier; the global
//! arrival/required propagation runs once per corner.
//!
//! On-chip variation (OCV) derating layers an extra per-corner
//! multiplier on each arc to bound timing variability across the
//! die. POCV (parametric OCV) is finer-grained — each cell + arc
//! gets its own variation; AOCV (advanced OCV) maps depth-of-logic
//! to a derate factor. v1 supports a flat per-corner derate.

use crate::graph::TimingGraph;
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct Corner {
    pub name: SmolStr,
    /// Multiplicative delay factor applied to every edge in the
    /// timing graph. tt corner = 1.0; ss = ~1.3; ff = ~0.7.
    pub delay_factor: f64,
    /// Late-path derate (added to delay_factor for setup paths).
    pub late_derate: f64,
    /// Early-path derate (subtracted for hold paths).
    pub early_derate: f64,
    /// Clock period for this corner.
    pub clock_period: f64,
}

impl Corner {
    pub fn typical(period: f64) -> Self {
        Self {
            name: "tt".into(),
            delay_factor: 1.0,
            late_derate: 1.05,
            early_derate: 0.95,
            clock_period: period,
        }
    }

    pub fn slow(period: f64) -> Self {
        Self {
            name: "ss".into(),
            delay_factor: 1.3,
            late_derate: 1.10,
            early_derate: 0.95,
            clock_period: period,
        }
    }

    pub fn fast(period: f64) -> Self {
        Self {
            name: "ff".into(),
            delay_factor: 0.7,
            late_derate: 1.05,
            early_derate: 0.90,
            clock_period: period,
        }
    }
}

/// Apply a corner's `delay_factor × late_derate` to every edge
/// delay in the graph. Returns a new `TimingGraph` (input is
/// unchanged).
pub fn apply_corner(graph: &TimingGraph, corner: &Corner) -> TimingGraph {
    let mut out = graph.clone();
    let scale = corner.delay_factor * corner.late_derate;
    for e in out.edges.iter_mut() {
        e.delay *= scale;
    }
    out
}

/// Apply early-derate (typically used for hold checks).
pub fn apply_corner_early(graph: &TimingGraph, corner: &Corner) -> TimingGraph {
    let mut out = graph.clone();
    let scale = corner.delay_factor * corner.early_derate;
    for e in out.edges.iter_mut() {
        e.delay *= scale;
    }
    out
}

#[derive(Clone, Debug)]
pub struct CornerReport {
    pub corner: SmolStr,
    pub worst_setup_slack: f64,
    pub worst_hold_slack: f64,
}

/// Run setup + hold across all corners and return the worst slack
/// at each corner.
pub fn corner_sweep(
    graph: &TimingGraph,
    corners: &[Corner],
) -> Vec<CornerReport> {
    use crate::arrival::{compute_arrivals, compute_required, compute_slacks};
    use crate::hold::compute_hold_slacks;
    let mut out = Vec::with_capacity(corners.len());
    for corner in corners {
        // Setup (late path).
        let g_late = apply_corner(graph, corner);
        let arr = compute_arrivals(&g_late, &[]).unwrap_or_default();
        let req = compute_required(&g_late, &[], corner.clock_period).unwrap_or_default();
        let setup_slacks = compute_slacks(&arr, &req);
        let worst_setup = setup_slacks
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min);

        // Hold (early path) — use 0.0 as default required.
        let g_early = apply_corner_early(graph, corner);
        let arr_early = compute_arrivals(&g_early, &[]).unwrap_or_default();
        let hold_req = vec![0.0; arr_early.len()];
        let hold_slacks = compute_hold_slacks(&arr_early, &hold_req);
        let worst_hold = hold_slacks
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min);

        out.push(CornerReport {
            corner: corner.name.clone(),
            worst_setup_slack: worst_setup,
            worst_hold_slack: worst_hold,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeKind, NodeKind, TimingGraph};

    fn linear_chain() -> TimingGraph {
        let mut g = TimingGraph::new();
        let pi = g.add_node("pi", NodeKind::PrimaryInput);
        let mid = g.add_node("mid", NodeKind::CellOutput);
        let po = g.add_node("po", NodeKind::PrimaryOutput);
        g.add_edge(pi, mid, EdgeKind::CellArc, 0.2);
        g.add_edge(mid, po, EdgeKind::Net, 0.1);
        g.finalize();
        g
    }

    #[test]
    fn slow_corner_scales_delays_up() {
        let g = linear_chain();
        let g_slow = apply_corner(&g, &Corner::slow(1.0));
        let original_total: f64 = g.edges.iter().map(|e| e.delay).sum();
        let slow_total: f64 = g_slow.edges.iter().map(|e| e.delay).sum();
        assert!(slow_total > original_total);
    }

    #[test]
    fn fast_corner_scales_delays_down() {
        let g = linear_chain();
        let g_fast = apply_corner(&g, &Corner::fast(1.0));
        let original_total: f64 = g.edges.iter().map(|e| e.delay).sum();
        let fast_total: f64 = g_fast.edges.iter().map(|e| e.delay).sum();
        assert!(fast_total < original_total);
    }

    #[test]
    fn corner_sweep_reports_per_corner() {
        let g = linear_chain();
        let corners = vec![
            Corner::typical(1.0),
            Corner::slow(1.0),
            Corner::fast(1.0),
        ];
        let reports = corner_sweep(&g, &corners);
        assert_eq!(reports.len(), 3);
        // Slow corner has worst setup slack; fast corner has worst hold.
        let slow = reports.iter().find(|r| r.corner == "ss").unwrap();
        let fast = reports.iter().find(|r| r.corner == "ff").unwrap();
        assert!(slow.worst_setup_slack < fast.worst_setup_slack);
    }
}
