//! MCMM — Multi-Corner Multi-Mode orchestration.
//!
//! A real chip ships in multiple "corners" (PVT permutations: slow /
//! typical / fast process, low / high voltage, hot / cold
//! temperature) AND multiple "modes" (functional / scan-shift /
//! standby), each with its own timing model and constraints. STA
//! must verify the chip closes timing across **all** active corner ×
//! mode combinations — a problem typically called MCMM.
//!
//! v1 covers:
//!
//! * **Corner × mode matrix.** A `Scenario` captures one
//!   `(corner_name, mode_name, corner, exceptions)` tuple.
//! * **Per-scenario evaluation.** For each scenario, run
//!   forward+backward+slack with the scenario's [`Corner`] and apply
//!   its [`PathException`] list.
//! * **Cross-scenario aggregation.** Report the *worst* slack
//!   across scenarios per endpoint — the sign-off-relevant figure.
//!
//! What's not in v1:
//!
//! * **Per-scenario Liberty / SPEF binding.** A real flow loads a
//!   different `.lib` and `.spef` per corner; this v1 takes a
//!   single graph + per-scenario [`Corner`] derate. Wiring a
//!   per-scenario graph rebuild is the next step.
//! * **Mode-conditional cell-arc disabling.** Scan-shift mode
//!   typically disables certain combinational arcs; we don't have
//!   the data path for that.

use crate::arrival::{compute_arrivals, compute_required, compute_slacks};
use crate::exceptions::{apply_exceptions, PathException};
use crate::graph::{NodeId, TimingGraph};
use crate::multi_corner::{apply_corner, Corner};
use crate::Result;
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct Scenario {
    pub name: SmolStr,
    pub corner: Corner,
    pub mode: SmolStr,
    pub exceptions: Vec<PathException>,
    /// Per-endpoint required-time constraint. Keyed by node name
    /// (matching nodes are looked up at evaluation time).
    pub endpoint_required: Vec<(SmolStr, f64)>,
}

#[derive(Clone, Debug)]
pub struct ScenarioReport {
    pub scenario: SmolStr,
    pub mode: SmolStr,
    /// Slack vector parallel to `g.nodes`.
    pub slacks: Vec<f64>,
    /// Worst-slack endpoint name + value.
    pub worst_slack: f64,
    pub worst_endpoint: Option<SmolStr>,
}

#[derive(Clone, Debug)]
pub struct McmmReport {
    pub per_scenario: Vec<ScenarioReport>,
    /// Across-all-scenarios worst slack per endpoint (sign-off view).
    pub worst_overall: f64,
    pub worst_overall_scenario: Option<SmolStr>,
}

/// Run MCMM analysis: for each scenario, derate the graph, propagate
/// arrival/required, apply exceptions, record slacks. Aggregate by
/// taking the per-endpoint min slack across scenarios.
pub fn run_mcmm(g: &TimingGraph, scenarios: &[Scenario]) -> Result<McmmReport> {
    let mut per_scenario: Vec<ScenarioReport> = Vec::with_capacity(scenarios.len());
    let mut worst_overall = f64::INFINITY;
    let mut worst_overall_scenario: Option<SmolStr> = None;

    for sc in scenarios {
        let derated = apply_corner(g, &sc.corner);
        let arrivals = compute_arrivals(&derated, &[])?;
        let endpoints: Vec<(NodeId, f64)> = sc
            .endpoint_required
            .iter()
            .filter_map(|(name, t)| {
                derated
                    .nodes
                    .iter()
                    .position(|n| n.name == *name)
                    .map(|i| (NodeId(i as u32), *t))
            })
            .collect();
        let requireds = compute_required(&derated, &endpoints, sc.corner.clock_period)?;
        let slacks = compute_slacks(&arrivals, &requireds);
        let slacks_after_exc = apply_exceptions(
            &derated,
            &arrivals,
            &slacks,
            &sc.exceptions,
            sc.corner.clock_period,
        );

        // Worst-slack scan is restricted to **primary outputs** —
        // those are the timing endpoints where required-time
        // constraints have meaning. Internal nodes' slacks are
        // intermediate accounting; reporting them as the worst
        // would surface false alarms.
        let mut worst = f64::INFINITY;
        let mut worst_node: Option<SmolStr> = None;
        for (i, s) in slacks_after_exc.iter().enumerate() {
            if !matches!(
                derated.nodes[i].kind,
                crate::graph::NodeKind::PrimaryOutput
            ) {
                continue;
            }
            if s.is_finite() && *s < worst {
                worst = *s;
                worst_node = Some(derated.nodes[i].name.clone());
            }
        }
        if worst < worst_overall {
            worst_overall = worst;
            worst_overall_scenario = Some(sc.name.clone());
        }
        per_scenario.push(ScenarioReport {
            scenario: sc.name.clone(),
            mode: sc.mode.clone(),
            slacks: slacks_after_exc,
            worst_slack: worst,
            worst_endpoint: worst_node,
        });
    }

    Ok(McmmReport {
        per_scenario,
        worst_overall,
        worst_overall_scenario,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeKind, NodeKind};
    use std::collections::HashSet;

    fn small_graph() -> (TimingGraph, NodeId, NodeId) {
        let mut g = TimingGraph::new();
        let a = g.add_node("a", NodeKind::PrimaryInput);
        let b = g.add_node("b", NodeKind::CellInput);
        let c = g.add_node("c", NodeKind::CellOutput);
        let d = g.add_node("d", NodeKind::PrimaryOutput);
        g.add_edge(a, b, EdgeKind::Net, 0.2);
        g.add_edge(b, c, EdgeKind::CellArc, 0.3);
        g.add_edge(c, d, EdgeKind::Net, 0.1);
        g.finalize();
        (g, a, d)
    }

    #[test]
    fn slow_corner_gives_worse_slack_than_fast() {
        let (g, _a, d) = small_graph();
        let scenarios = vec![
            Scenario {
                name: "slow".into(),
                corner: Corner::slow(1.0),
                mode: "func".into(),
                exceptions: Vec::new(),
                endpoint_required: vec![("d".into(), 1.0)],
            },
            Scenario {
                name: "fast".into(),
                corner: Corner::fast(1.0),
                mode: "func".into(),
                exceptions: Vec::new(),
                endpoint_required: vec![("d".into(), 1.0)],
            },
        ];
        let r = run_mcmm(&g, &scenarios).unwrap();
        let slow = &r.per_scenario[0];
        let fast = &r.per_scenario[1];
        assert!(slow.worst_slack <= fast.worst_slack);
        // Worst-overall = slow corner's value.
        assert!((r.worst_overall - slow.worst_slack).abs() < 1e-9);
        let _ = d;
    }

    #[test]
    fn false_path_exception_relaxes_slack() {
        let (g, a, d) = small_graph();
        let scenarios = vec![Scenario {
            name: "tight".into(),
            corner: Corner::typical(0.4), // tight clock — would fail naturally
            mode: "func".into(),
            exceptions: vec![PathException::FalsePath {
                from: HashSet::from([a]),
                to: HashSet::from([d]),
            }],
            endpoint_required: vec![("d".into(), 0.4)],
        }];
        let r = run_mcmm(&g, &scenarios).unwrap();
        // worst_slack should be ∞ (no failing endpoints) since the
        // only timing-relevant path is set_false_path'd.
        assert!(r.per_scenario[0].worst_slack.is_infinite());
    }
}
