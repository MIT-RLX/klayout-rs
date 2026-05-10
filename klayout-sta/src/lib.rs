//! `klayout-sta` — static timing analysis primitives.
//!
//! Closes the digital sign-off loop: with Liberty (cell timing) +
//! LEF/DEF (placement) + SPEF (parasitics) + hier-netlist
//! (connectivity) all produced by sister crates, this crate evaluates
//! per-pin arrival/required times and reports slack on every endpoint.
//!
//! Algorithm:
//! 1. **Timing graph** — every pin in the design becomes a node;
//!    cell timing arcs become intra-cell edges; net interconnect
//!    becomes inter-cell edges with optional RC delay from SPEF.
//! 2. **Forward propagation** (arrival times) — topo-sort the graph
//!    from primary inputs / clock pins, accumulate max-delay along
//!    each path.
//! 3. **Backward propagation** (required times) — start from primary
//!    outputs / endpoint setup constraints, walk backward, take min
//!    of (req - arc_delay).
//! 4. **Slack** — `required - arrival` per pin. Negative slack on a
//!    timing endpoint = violation.
//!
//! v1 covers combinational paths and setup constraints. Hold checks
//! and clock-domain crossings are a v2.
//!
//! The model is intentionally small: a single `TimingGraph` struct
//! with arrays for nodes/edges, no graph-library dependency. Lookup
//! tables are stored verbatim from Liberty; v1 evaluates them as a
//! single representative-corner constant (the table's first cell).
//! v2 will interpolate against (input slew, output cap) axes.

pub mod arrival;
pub mod ccs;
pub mod cdc;
pub mod cppr;
pub mod derate;
pub mod exceptions;
pub mod graph;
pub mod hier_eval;
pub mod hold;
pub mod mcmm;
pub mod multi_corner;
pub mod report;
pub mod sdf;
pub mod spef_back;
pub mod timing_model;

pub use arrival::{
    compute_arrivals, compute_arrivals_with_model, compute_arrivals_with_slew, compute_required,
    compute_slacks,
};
pub use cdc::{find_cdc_violations, CdcViolation};
pub use cppr::{
    clock_arrivals, common_path_delay, cppr_credit, lowest_common_clock_ancestor,
};
pub use derate::{apply_aocv, apply_pocv, depths, AocvTable, PocvNodeStat, PocvVariation};
pub use graph::{Edge, EdgeKind, Node, NodeId, TimingGraph};
pub use hier_eval::{
    compose_hier_timing, extract_abstract, parent_node_count, CellTimingAbstract, HierStaInputs,
    HierStaResult,
};
pub use hold::{compute_hold_slacks, hold_violations};
pub use multi_corner::{apply_corner, apply_corner_early, corner_sweep, Corner, CornerReport};
pub use report::{format_path_report, PathReport, SlackReport};
pub use sdf::{back_annotate, read_sdf, SdfCell, SdfError, SdfFile, SdfIoPath, Triplet};
pub use ccs::{ccs_delay, integrate_to_voltage, CcsTable};
pub use exceptions::{apply_exceptions, PathException};
pub use mcmm::{run_mcmm, McmmReport, Scenario, ScenarioReport};
pub use spef_back::{back_annotate_spef, NetParasitic};
pub use timing_model::{CcsDelayModel, DelayContext, LinearDelayModel, NldmDelayModel, TimingModel};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StaError {
    #[error("sta: cycle detected in timing graph at node {0}")]
    CycleDetected(usize),

    #[error("sta: missing endpoint for path search")]
    MissingEndpoint,
}

pub type Result<T> = std::result::Result<T, StaError>;
