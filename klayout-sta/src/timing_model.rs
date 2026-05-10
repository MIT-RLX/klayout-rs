//! Pluggable delay-model abstraction.
//!
//! Pre-trait, `TimingGraph::Edge.delay` was a fixed `f64`. That's
//! "linear delay" — the simplest model, fine for back-of-envelope
//! exploration. Real STA uses NLDM lookup tables (delay depends on
//! input slew + output capacitance) and increasingly CCS (current-
//! source models). Different corners of a flow want different
//! fidelity.
//!
//! [`TimingModel`] abstracts the per-edge delay computation: given
//! the edge, the upstream signal's slew, and the receiver's load
//! capacitance, return the propagation delay through the edge. The
//! arrival/required propagation in `arrival.rs` can run against any
//! impl of this trait — caller picks the trade-off.

use crate::graph::{Edge, EdgeKind};

/// Per-edge state the delay model can consult.
#[derive(Copy, Clone, Debug)]
pub struct DelayContext<'a> {
    pub edge: &'a Edge,
    /// Input slew (transition time) at the edge's source pin.
    pub input_slew: f64,
    /// Capacitive load at the edge's destination pin.
    pub output_load: f64,
}

pub trait TimingModel: Send + Sync {
    /// Propagation delay through the edge in time-units.
    fn delay(&self, ctx: DelayContext<'_>) -> f64;

    /// Output transition time given the same context. Default
    /// returns the input slew (i.e. signal transitions don't
    /// change). Models with proper transition tables override.
    fn transition(&self, ctx: DelayContext<'_>) -> f64 {
        ctx.input_slew
    }
}

/// Linear delay model: returns the edge's stored `delay` verbatim.
/// Equivalent to the original behaviour; useful for tests + flows
/// that don't have Liberty data yet.
pub struct LinearDelayModel;

impl TimingModel for LinearDelayModel {
    fn delay(&self, ctx: DelayContext<'_>) -> f64 {
        ctx.edge.delay
    }
}

/// NLDM delay model: each cell-arc edge looks up its delay from a
/// 2D `(slew, load)` table; net edges use a separate per-edge linear
/// model (typically Elmore RC). The mapping from `Edge` to its
/// lookup table is supplied by the caller (a Liberty cell library
/// would key by `(cell, arc_index)`).
pub struct NldmDelayModel<F: Fn(&Edge, f64, f64) -> f64 + Send + Sync> {
    pub lookup: F,
}

impl<F: Fn(&Edge, f64, f64) -> f64 + Send + Sync> TimingModel for NldmDelayModel<F> {
    fn delay(&self, ctx: DelayContext<'_>) -> f64 {
        match ctx.edge.kind {
            EdgeKind::CellArc => (self.lookup)(ctx.edge, ctx.input_slew, ctx.output_load),
            EdgeKind::Net => ctx.edge.delay,
        }
    }
}

/// CCS placeholder. Real CCS models drive a current waveform into
/// the receiver and compute delay from the resulting voltage curve;
/// implementing that is its own crate. v1 falls back to NLDM-style
/// table lookup if provided, else linear.
pub struct CcsDelayModel<F: Fn(&Edge, f64, f64) -> f64 + Send + Sync> {
    pub lookup: F,
}

impl<F: Fn(&Edge, f64, f64) -> f64 + Send + Sync> TimingModel for CcsDelayModel<F> {
    fn delay(&self, ctx: DelayContext<'_>) -> f64 {
        (self.lookup)(ctx.edge, ctx.input_slew, ctx.output_load)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{NodeKind, TimingGraph};

    #[test]
    fn linear_model_returns_edge_delay() {
        let mut g = TimingGraph::new();
        let a = g.add_node("a", NodeKind::PrimaryInput);
        let b = g.add_node("b", NodeKind::PrimaryOutput);
        g.add_edge(a, b, EdgeKind::Net, 0.123);
        g.finalize();
        let model = LinearDelayModel;
        let d = model.delay(DelayContext {
            edge: &g.edges[0],
            input_slew: 0.05,
            output_load: 0.01,
        });
        assert!((d - 0.123).abs() < 1e-9);
    }

    #[test]
    fn nldm_model_calls_lookup_for_cell_arcs() {
        let mut g = TimingGraph::new();
        let a = g.add_node("a", NodeKind::CellInput);
        let b = g.add_node("b", NodeKind::CellOutput);
        g.add_edge(a, b, EdgeKind::CellArc, 0.0);
        g.finalize();
        let model = NldmDelayModel {
            lookup: |_e: &Edge, slew: f64, load: f64| slew * 2.0 + load * 10.0,
        };
        let d = model.delay(DelayContext {
            edge: &g.edges[0],
            input_slew: 0.05,
            output_load: 0.01,
        });
        // 0.05 × 2 + 0.01 × 10 = 0.2.
        assert!((d - 0.2).abs() < 1e-9);
    }

    #[test]
    fn nldm_model_passes_through_net_edges() {
        let mut g = TimingGraph::new();
        let a = g.add_node("a", NodeKind::CellOutput);
        let b = g.add_node("b", NodeKind::CellInput);
        g.add_edge(a, b, EdgeKind::Net, 0.42);
        g.finalize();
        let model = NldmDelayModel {
            lookup: |_e: &Edge, _s: f64, _l: f64| 999.0,
        };
        let d = model.delay(DelayContext {
            edge: &g.edges[0],
            input_slew: 0.0,
            output_load: 0.0,
        });
        // Net edges fall back to edge.delay.
        assert!((d - 0.42).abs() < 1e-9);
    }

    #[test]
    fn default_transition_passes_through_slew() {
        let model = LinearDelayModel;
        let edge = Edge {
            from: crate::graph::NodeId(0),
            to: crate::graph::NodeId(1),
            kind: EdgeKind::Net,
            delay: 0.0,
        };
        let t = model.transition(DelayContext {
            edge: &edge,
            input_slew: 0.07,
            output_load: 0.0,
        });
        assert!((t - 0.07).abs() < 1e-9);
    }
}
