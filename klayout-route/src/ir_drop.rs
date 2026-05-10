//! IR-drop solver for the power grid.
//!
//! Given a power grid (a set of horizontal stripes + vertical stripes
//! interconnected at intersections, plus pad locations and per-cell
//! current sinks), compute the steady-state DC voltage at every node
//! by solving the linear system
//!
//! ```text
//!     G · v = i
//! ```
//!
//! where `G` is the conductance (`1/R`) matrix of the resistive
//! network, `v` is the per-node voltage vector, and `i` is the
//! per-node current injection / withdrawal. Voltage at the pad
//! nodes is fixed to `V_dd`; the solver finds the voltage drop at
//! every interior node, and the per-cell IR drop is `V_dd −
//! V(nearest_node)`.
//!
//! ## Algorithm
//!
//! Conjugate-gradient on a sparse symmetric-positive-definite
//! Laplacian. The structure is identical to [`crate::pathfinder`]'s
//! shortest-path search at the topology level, but the math is a
//! linear solve, not a graph-search.
//!
//! 1. Discretize the power grid into a graph: nodes at every stripe
//!    intersection + every cell connection point + every pad.
//! 2. Edges = grid segments, with conductance `g = 1 / R`.
//! 3. Pads have fixed voltage `V_dd`. The system reduces to solving
//!    for the unknown interior voltages.
//! 4. Cell currents `I_cell` go on the right-hand side as injections.
//! 5. CG converges in `O(√κ)` iterations where κ is the condition
//!    number; for typical power grids that's ~50 iterations.
//!
//! ## What this is NOT
//!
//! * **Transient (AC) IR drop.** The solver is DC-steady-state only;
//!   `dI/dt` and decoupling caps are out of scope.
//! * **Self-heat coupled.** A real flow loops IR drop with thermal
//!   simulation (Joule heating raises local resistance, which raises
//!   IR drop, which raises local heating). One-shot DC is enough
//!   for sign-off-style worst-case bounds.
//! * **Parasitically-extracted.** We assume the caller supplies
//!   per-segment resistance directly; no field-solver coupling
//!   between adjacent power stripes.

use klayout_core::Point;
use smol_str::SmolStr;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct PowerNode {
    pub name: SmolStr,
    pub at: Point,
    /// Fixed voltage (`Some(V)`) for pads / supplies; `None` for
    /// solved nodes.
    pub fixed_v: Option<f64>,
    /// Steady-state current injection (positive) or withdrawal
    /// (negative) at this node, in amperes.
    pub current: f64,
}

#[derive(Clone, Debug)]
pub struct PowerSegment {
    pub from: usize,
    pub to: usize,
    /// Resistance in Ω.
    pub resistance: f64,
}

#[derive(Clone, Debug)]
pub struct PowerGridIr {
    pub nodes: Vec<PowerNode>,
    pub segments: Vec<PowerSegment>,
}

#[derive(Clone, Debug)]
pub struct IrDropConfig {
    pub cg_tolerance: f64,
    pub cg_max_iterations: u32,
}

impl Default for IrDropConfig {
    fn default() -> Self {
        Self {
            cg_tolerance: 1e-6,
            cg_max_iterations: 1024,
        }
    }
}

#[derive(Clone, Debug)]
pub struct IrDropResult {
    /// Voltage at every node, indexed by `PowerGridIr::nodes` order.
    pub voltages: Vec<f64>,
    /// Per-node IR drop (`V_supply − V_node`); positive means the
    /// node sees a sag below the supply.
    pub drops: Vec<f64>,
    /// CG iteration count for diagnostics.
    pub cg_iterations: u32,
}

/// Solve DC IR-drop. Treats every node with `fixed_v = Some(_)` as a
/// pad and solves for the rest.
pub fn solve_ir_drop(grid: &PowerGridIr, cfg: &IrDropConfig) -> IrDropResult {
    let n = grid.nodes.len();
    let mut voltages = vec![0.0f64; n];
    if n == 0 {
        return IrDropResult {
            voltages,
            drops: Vec::new(),
            cg_iterations: 0,
        };
    }

    // Index the unknown ("free") nodes 0..k. Fixed nodes don't enter
    // the linear system; they appear on the right-hand side as
    // boundary contributions.
    let mut free_idx: Vec<Option<usize>> = vec![None; n];
    let mut free_count = 0;
    for (i, node) in grid.nodes.iter().enumerate() {
        if node.fixed_v.is_none() {
            free_idx[i] = Some(free_count);
            free_count += 1;
        } else {
            voltages[i] = node.fixed_v.unwrap();
        }
    }
    if free_count == 0 {
        return IrDropResult {
            drops: drops_from(&voltages, grid),
            voltages,
            cg_iterations: 0,
        };
    }

    // Build sparse Laplacian: G[i,j] = -g_ij, G[i,i] = sum_j g_ij.
    // RHS b[i] = current_at_i + sum over fixed-neighbor j of
    // g_ij · v_j. Both sides indexed in the free-node space.
    let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); free_count];
    let mut diag: Vec<f64> = vec![0.0; free_count];
    let mut rhs: Vec<f64> = vec![0.0; free_count];

    for seg in &grid.segments {
        if seg.resistance <= 0.0 {
            continue;
        }
        let g = 1.0 / seg.resistance;
        let (i, j) = (seg.from, seg.to);
        match (free_idx[i], free_idx[j]) {
            (Some(a), Some(b)) => {
                adj[a].push((b, g));
                adj[b].push((a, g));
                diag[a] += g;
                diag[b] += g;
            }
            (Some(a), None) => {
                diag[a] += g;
                rhs[a] += g * voltages[j];
            }
            (None, Some(b)) => {
                diag[b] += g;
                rhs[b] += g * voltages[i];
            }
            (None, None) => {}
        }
    }
    // Add per-node current injections on the RHS.
    for (i, node) in grid.nodes.iter().enumerate() {
        if let Some(a) = free_idx[i] {
            rhs[a] += node.current;
        }
    }

    // CG initial guess: copy any Vdd from the nearest fixed node by
    // taking the global mean.
    let supply_estimate: f64 = grid
        .nodes
        .iter()
        .filter_map(|n| n.fixed_v)
        .next()
        .unwrap_or(0.0);
    let mut x: Vec<f64> = vec![supply_estimate; free_count];

    let cg_iters = conjugate_gradient(&adj, &diag, &rhs, &mut x, cfg.cg_tolerance, cfg.cg_max_iterations);

    for (i, _) in grid.nodes.iter().enumerate() {
        if let Some(a) = free_idx[i] {
            voltages[i] = x[a];
        }
    }
    IrDropResult {
        drops: drops_from(&voltages, grid),
        voltages,
        cg_iterations: cg_iters,
    }
}

fn drops_from(voltages: &[f64], grid: &PowerGridIr) -> Vec<f64> {
    let supply: f64 = grid
        .nodes
        .iter()
        .filter_map(|n| n.fixed_v)
        .fold(f64::MIN, f64::max);
    if supply == f64::MIN {
        return vec![0.0; voltages.len()];
    }
    voltages.iter().map(|v| supply - v).collect()
}

fn conjugate_gradient(
    adj: &[Vec<(usize, f64)>],
    diag: &[f64],
    b: &[f64],
    x: &mut [f64],
    tol: f64,
    max_iter: u32,
) -> u32 {
    let n = x.len();
    if n == 0 {
        return 0;
    }
    let matvec = |v: &[f64], out: &mut [f64]| {
        for i in 0..n {
            let mut s = diag[i] * v[i];
            for &(j, g) in &adj[i] {
                s -= g * v[j];
            }
            out[i] = s;
        }
    };
    let mut r = vec![0.0; n];
    let mut ax = vec![0.0; n];
    matvec(x, &mut ax);
    for i in 0..n {
        r[i] = b[i] - ax[i];
    }
    let mut p = r.clone();
    let mut rs_old: f64 = r.iter().map(|v| v * v).sum();
    let b_norm: f64 = b.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-30);
    let mut ap = vec![0.0; n];
    for k in 0..max_iter {
        matvec(&p, &mut ap);
        let pap: f64 = p.iter().zip(ap.iter()).map(|(a, b)| a * b).sum();
        if pap.abs() < 1e-30 {
            return k;
        }
        let alpha = rs_old / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        let rs_new: f64 = r.iter().map(|v| v * v).sum();
        if rs_new.sqrt() / b_norm < tol {
            return k + 1;
        }
        let beta = rs_new / rs_old;
        for i in 0..n {
            p[i] = r[i] + beta * p[i];
        }
        rs_old = rs_new;
    }
    max_iter
}

/// Build a [`PowerGridIr`] from horizontal + vertical stripes (the
/// shape [`crate::generate_power_grid`] produces) and a list of
/// per-cell current sinks. Each stripe-stripe intersection becomes a
/// node; each stripe segment between intersections becomes a
/// resistor; each cell connects to the nearest grid node via a small
/// "cell-to-rail" via resistance.
pub fn build_power_grid_ir(
    pads: &[(Point, f64)], // (location, fixed Vdd)
    h_stripes: &[(i64, i64, i64)], // (y, x_start, x_end)
    v_stripes: &[(i64, i64, i64)], // (x, y_start, y_end)
    sheet_resistance: f64, // Ω per unit length of stripe
    cell_currents: &[(Point, f64)], // (location, draw current)
    via_resistance: f64,   // cell tap resistance to nearest rail node
) -> PowerGridIr {
    // Collect intersection nodes.
    let mut nodes: Vec<PowerNode> = Vec::new();
    let mut node_at: HashMap<(i64, i64), usize> = HashMap::new();

    let intern = |p: Point,
                  fixed_v: Option<f64>,
                  current: f64,
                  nodes: &mut Vec<PowerNode>,
                  node_at: &mut HashMap<(i64, i64), usize>|
     -> usize {
        if let Some(&idx) = node_at.get(&(p.x, p.y)) {
            // Merge: a fixed-V or current value supersedes / sums.
            if let Some(v) = fixed_v {
                nodes[idx].fixed_v = Some(v);
            }
            nodes[idx].current += current;
            return idx;
        }
        let idx = nodes.len();
        nodes.push(PowerNode {
            name: SmolStr::from(format!("n{idx}")),
            at: p,
            fixed_v,
            current,
        });
        node_at.insert((p.x, p.y), idx);
        idx
    };

    // Pads first so their fixed_v wins on later merges.
    for (p, v) in pads {
        intern(*p, Some(*v), 0.0, &mut nodes, &mut node_at);
    }

    // Intersections of every (h_stripe, v_stripe) pair that meet.
    let mut intersections_per_h: Vec<Vec<i64>> = vec![Vec::new(); h_stripes.len()];
    let mut intersections_per_v: Vec<Vec<i64>> = vec![Vec::new(); v_stripes.len()];
    for (hi, &(y, xs, xe)) in h_stripes.iter().enumerate() {
        for (vi, &(x, ys, ye)) in v_stripes.iter().enumerate() {
            if x >= xs && x <= xe && y >= ys && y <= ye {
                let _ = intern(Point::new(x, y), None, 0.0, &mut nodes, &mut node_at);
                intersections_per_h[hi].push(x);
                intersections_per_v[vi].push(y);
            }
        }
    }
    // Pads might lie on a stripe — treat them as intersection points
    // along that stripe so segments through the pad get the pad node
    // as an endpoint.
    for (p, _) in pads {
        for (hi, &(y, xs, xe)) in h_stripes.iter().enumerate() {
            if p.y == y && p.x >= xs && p.x <= xe {
                intersections_per_h[hi].push(p.x);
            }
        }
        for (vi, &(x, ys, ye)) in v_stripes.iter().enumerate() {
            if p.x == x && p.y >= ys && p.y <= ye {
                intersections_per_v[vi].push(p.y);
            }
        }
    }

    let mut segments: Vec<PowerSegment> = Vec::new();

    // Horizontal stripe segments between consecutive intersections.
    for (hi, &(y, _, _)) in h_stripes.iter().enumerate() {
        let mut xs = intersections_per_h[hi].clone();
        xs.sort_unstable();
        xs.dedup();
        for w in xs.windows(2) {
            let a = node_at[&(w[0], y)];
            let b = node_at[&(w[1], y)];
            let r = sheet_resistance * (w[1] - w[0]) as f64;
            if r > 0.0 {
                segments.push(PowerSegment {
                    from: a,
                    to: b,
                    resistance: r,
                });
            }
        }
    }
    for (vi, &(x, _, _)) in v_stripes.iter().enumerate() {
        let mut ys = intersections_per_v[vi].clone();
        ys.sort_unstable();
        ys.dedup();
        for w in ys.windows(2) {
            let a = node_at[&(x, w[0])];
            let b = node_at[&(x, w[1])];
            let r = sheet_resistance * (w[1] - w[0]) as f64;
            if r > 0.0 {
                segments.push(PowerSegment {
                    from: a,
                    to: b,
                    resistance: r,
                });
            }
        }
    }

    // Cell taps: connect each cell to the nearest grid node via
    // `via_resistance`. The tap node carries the cell's current.
    let mut tap_nodes: Vec<usize> = Vec::new();
    for (cp, current) in cell_currents {
        // Nearest grid node by Manhattan distance.
        let mut best: Option<(usize, i64)> = None;
        for (i, n) in nodes.iter().enumerate() {
            let d = (cp.x - n.at.x).abs() + (cp.y - n.at.y).abs();
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        let nearest = best.map(|(i, _)| i).unwrap_or(0);
        let tap_idx = intern(*cp, None, *current, &mut nodes, &mut node_at);
        tap_nodes.push(tap_idx);
        if tap_idx != nearest && via_resistance > 0.0 {
            segments.push(PowerSegment {
                from: tap_idx,
                to: nearest,
                resistance: via_resistance,
            });
        }
    }

    PowerGridIr { nodes, segments }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_grid_zero_drop() {
        let grid = PowerGridIr {
            nodes: Vec::new(),
            segments: Vec::new(),
        };
        let r = solve_ir_drop(&grid, &IrDropConfig::default());
        assert_eq!(r.voltages.len(), 0);
        assert_eq!(r.drops.len(), 0);
    }

    #[test]
    fn series_resistor_chain_voltage_divider() {
        // Two resistors of equal R connecting a fixed pad (1.0 V) to
        // ground (0 V) via a middle node: Kirchhoff says the middle
        // sits at exactly 0.5 V. Tests CG correctness on a tiny
        // problem where the answer is analytic.
        let grid = PowerGridIr {
            nodes: vec![
                PowerNode {
                    name: "vdd".into(),
                    at: Point::new(0, 0),
                    fixed_v: Some(1.0),
                    current: 0.0,
                },
                PowerNode {
                    name: "mid".into(),
                    at: Point::new(50, 0),
                    fixed_v: None,
                    current: 0.0,
                },
                PowerNode {
                    name: "gnd".into(),
                    at: Point::new(100, 0),
                    fixed_v: Some(0.0),
                    current: 0.0,
                },
            ],
            segments: vec![
                PowerSegment {
                    from: 0,
                    to: 1,
                    resistance: 1.0,
                },
                PowerSegment {
                    from: 1,
                    to: 2,
                    resistance: 1.0,
                },
            ],
        };
        let r = solve_ir_drop(&grid, &IrDropConfig::default());
        assert!((r.voltages[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn current_sink_creates_local_drop() {
        // Pad at left at 1.0 V, single resistor (R=1Ω) to a sink
        // node drawing 0.1 A. Voltage at the sink = 1.0 − 0.1·1 =
        // 0.9 V; drop = 0.1 V.
        let grid = PowerGridIr {
            nodes: vec![
                PowerNode {
                    name: "pad".into(),
                    at: Point::new(0, 0),
                    fixed_v: Some(1.0),
                    current: 0.0,
                },
                PowerNode {
                    name: "load".into(),
                    at: Point::new(100, 0),
                    fixed_v: None,
                    current: -0.1, // negative = current OUT
                },
            ],
            segments: vec![PowerSegment {
                from: 0,
                to: 1,
                resistance: 1.0,
            }],
        };
        let r = solve_ir_drop(&grid, &IrDropConfig::default());
        assert!((r.voltages[1] - 0.9).abs() < 1e-6);
        assert!((r.drops[1] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn build_grid_then_solve_realistic_topology() {
        // 2 horizontal stripes × 2 vertical stripes → 4 intersection
        // nodes. One pad on a corner. One cell tap drawing current.
        // Sanity: solve completes, drops are non-negative, drop at
        // the pad is ≈ 0.
        let h_stripes = vec![(0, 0, 1000), (1000, 0, 1000)];
        let v_stripes = vec![(0, 0, 1000), (1000, 0, 1000)];
        let pads = vec![(Point::new(0, 0), 1.8)];
        let cells = vec![(Point::new(900, 900), -1e-3)];
        let grid = build_power_grid_ir(
            &pads,
            &h_stripes,
            &v_stripes,
            0.001, // 1 mΩ per DBU of stripe → 1 Ω across 1000 DBU
            &cells,
            0.05, // via R
        );
        assert!(!grid.nodes.is_empty());
        let r = solve_ir_drop(&grid, &IrDropConfig::default());
        assert!(r.cg_iterations < 1024);
        // Pad voltage stays exactly Vdd.
        let pad_idx = grid
            .nodes
            .iter()
            .position(|n| n.fixed_v == Some(1.8))
            .unwrap();
        assert!((r.voltages[pad_idx] - 1.8).abs() < 1e-9);
        // All drops ≥ 0 (pad delivers current downhill).
        for d in &r.drops {
            assert!(*d >= -1e-6, "negative drop {d} indicates a sign error");
        }
    }
}
