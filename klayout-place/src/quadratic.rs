//! Analytic global placement via the bound-to-bound (B2B) net model
//! solved with conjugate gradient.
//!
//! ## Why this exists
//!
//! The pre-existing [`crate::engine::QuadraticPlacer`] uses a
//! clique-model Laplacian solved by Jacobi iteration. The clique
//! model penalizes net wire length quadratically in fanout —
//! a 10-pin net counts as `O(10²)` springs, so high-fanout nets
//! dominate the cost function and pull their cells into a tight
//! cluster, not what HPWL minimization wants. Jacobi converges
//! linearly and slowly on graph-Laplacian systems.
//!
//! This module implements the **bound-to-bound (B2B) net model**
//! used by FastPlace (Viswanathan/Chu 2005) and the basis of every
//! modern analytic placer (RePlAce, ePlace, Eh?Place):
//!
//! 1. For each net, identify the pin with extreme x (separately for
//!    `x_min` and `x_max`) given the *current* placement.
//! 2. Each non-extreme pin contributes a spring of weight
//!    `2 / ((k − 1) · |x_i − x_extreme|)` to **both** extreme pins
//!    (or 1 spring if the net has only 2 pins). This linearizes
//!    `HPWL_x = x_max − x_min` as a sum of weighted quadratic spring
//!    terms whose minimum coincides with the L1 minimum.
//! 3. Solve the resulting sparse symmetric-positive-definite system
//!    `A · x = b` via conjugate gradient.
//! 4. Repeat steps 1–3 a few times (the extremes may shift after the
//!    solve); B2B converges in 4–8 iterations on real designs.
//!
//! `x` and `y` are decoupled — two independent SPD solves per outer
//! iteration. Each CG inner solve is `O(nnz · √κ(A))` where `κ` is
//! the spectral condition number of the Laplacian; for sparse
//! placement matrices this lands at `O(n · log(1/ε))` empirically.
//!
//! ## Limit of applicability
//!
//! This solver minimizes wire length; it does **not** spread cells
//! across the layout. Without a density penalty, the analytic
//! optimum stacks cells on top of each other at the HPWL minimum.
//! Run [`crate::legalize`] after this engine to snap cells to the
//! row site grid and resolve overlaps. A full ePlace-style
//! electrostatic-density-aware solver — which adds a gradient term
//! `ρ · ∇φ` from the Poisson equation over the placement bin grid —
//! is the next algorithmic upgrade and is *not* implemented in v1.

use crate::types::Placement;
use klayout_core::Point;

#[derive(Clone, Debug)]
pub struct QuadraticConfig {
    /// Number of B2B outer iterations (re-determining extreme pins).
    /// 4–8 is typical; the model converges quickly.
    pub b2b_iterations: u32,
    /// Inner CG convergence tolerance — relative residual norm.
    pub cg_tolerance: f64,
    /// Maximum CG iterations per outer step. CG converges in at most
    /// `n` iterations exactly, so this is a safety bound.
    pub cg_max_iterations: u32,
    /// Avoid division-by-zero for cells exactly coincident with their
    /// extreme pin; clamp the inverse-distance weight to this.
    pub min_separation: f64,
}

impl Default for QuadraticConfig {
    fn default() -> Self {
        Self {
            b2b_iterations: 6,
            cg_tolerance: 1e-4,
            cg_max_iterations: 256,
            min_separation: 1.0,
        }
    }
}

/// Run B2B-quadratic global placement on `p`. Mutates each free
/// cell's `position`; fixed cells are anchors and are not moved.
pub fn quadratic_place(p: &mut Placement, cfg: &QuadraticConfig) {
    let n = p.cells.len();
    if n == 0 {
        return;
    }

    for _ in 0..cfg.b2b_iterations {
        place_axis(p, cfg, Axis::X);
        place_axis(p, cfg, Axis::Y);
    }
}

#[derive(Copy, Clone)]
enum Axis {
    X,
    Y,
}

fn place_axis(p: &mut Placement, cfg: &QuadraticConfig, axis: Axis) {
    let n = p.cells.len();
    // Map each cell to a free-cell index, or `None` if fixed.
    let mut free_idx: Vec<Option<usize>> = vec![None; n];
    let mut free_count = 0usize;
    for (i, c) in p.cells.iter().enumerate() {
        if !c.fixed {
            free_idx[i] = Some(free_count);
            free_count += 1;
        }
    }
    if free_count == 0 {
        return;
    }

    let coord = |i: usize| -> f64 {
        let pos = p.cells[i].position;
        match axis {
            Axis::X => pos.x as f64 + p.cells[i].width as f64 / 2.0,
            Axis::Y => pos.y as f64 + p.cells[i].height as f64 / 2.0,
        }
    };

    // Build the sparse Laplacian for this axis: A[i,j] = -w_ij,
    // A[i,i] = sum_j w_ij. Right-hand side b[i] absorbs the
    // contributions from edges to fixed cells.
    //
    // Stored as adjacency-list-of-(j, w) and separate diagonal/RHS.
    let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); free_count];
    let mut diag: Vec<f64> = vec![0.0; free_count];
    let mut rhs: Vec<f64> = vec![0.0; free_count];

    let add_edge = |i: usize, j: usize, w: f64,
                        free_idx: &[Option<usize>],
                        adj: &mut [Vec<(usize, f64)>],
                        diag: &mut [f64],
                        rhs: &mut [f64],
                        coord: &dyn Fn(usize) -> f64| {
        let fi = free_idx[i];
        let fj = free_idx[j];
        match (fi, fj) {
            (Some(a), Some(b)) => {
                adj[a].push((b, w));
                adj[b].push((a, w));
                diag[a] += w;
                diag[b] += w;
            }
            (Some(a), None) => {
                diag[a] += w;
                rhs[a] += w * coord(j);
            }
            (None, Some(b)) => {
                diag[b] += w;
                rhs[b] += w * coord(i);
            }
            (None, None) => {}
        }
    };

    // For each net, pick the pin with min coord and the pin with max
    // coord (under current positions). Add B2B springs:
    //   * 2-pin net: one spring of weight 1.
    //   * k-pin net (k > 2): for each non-extreme pin i, springs to
    //     both `min_pin` and `max_pin` of weight
    //     `2 / ((k − 1) · max(|coord_i − coord_extreme|, ε))`.
    //     Plus one spring `min_pin ↔ max_pin` of the same form.
    for net in &p.nets {
        let pins = &net.cell_indices;
        if pins.len() < 2 {
            continue;
        }
        if pins.len() == 2 {
            let w = net.weight;
            add_edge(pins[0], pins[1], w, &free_idx, &mut adj, &mut diag, &mut rhs, &coord);
            continue;
        }
        let k = pins.len() as f64;
        // Find min and max coord pins.
        let mut min_pin = pins[0];
        let mut max_pin = pins[0];
        for &q in &pins[1..] {
            if coord(q) < coord(min_pin) {
                min_pin = q;
            }
            if coord(q) > coord(max_pin) {
                max_pin = q;
            }
        }
        let coord_min = coord(min_pin);
        let coord_max = coord(max_pin);
        let inv_sep = |a: f64, b: f64| -> f64 {
            let d = (a - b).abs().max(cfg.min_separation);
            1.0 / d
        };
        // Springs from each non-extreme pin to both extremes.
        for &i in pins {
            if i == min_pin || i == max_pin {
                continue;
            }
            let w_min = 2.0 * net.weight / (k - 1.0) * inv_sep(coord(i), coord_min);
            let w_max = 2.0 * net.weight / (k - 1.0) * inv_sep(coord(i), coord_max);
            add_edge(i, min_pin, w_min, &free_idx, &mut adj, &mut diag, &mut rhs, &coord);
            add_edge(i, max_pin, w_max, &free_idx, &mut adj, &mut diag, &mut rhs, &coord);
        }
        // Spring between the two extremes.
        let w_pair = 2.0 * net.weight / (k - 1.0) * inv_sep(coord_min, coord_max);
        add_edge(min_pin, max_pin, w_pair, &free_idx, &mut adj, &mut diag, &mut rhs, &coord);
    }

    // CG initial guess: current positions of free cells.
    let mut x: Vec<f64> = (0..n)
        .filter(|i| !p.cells[*i].fixed)
        .map(&coord)
        .collect();

    conjugate_gradient(&adj, &diag, &rhs, &mut x, cfg.cg_tolerance, cfg.cg_max_iterations);

    // Write back. Convert each free cell's center back to lower-left.
    let mut k = 0usize;
    for i in 0..n {
        if p.cells[i].fixed {
            continue;
        }
        let center = x[k];
        let new = match axis {
            Axis::X => Point::new(
                (center - p.cells[i].width as f64 / 2.0).round() as i64,
                p.cells[i].position.y,
            ),
            Axis::Y => Point::new(
                p.cells[i].position.x,
                (center - p.cells[i].height as f64 / 2.0).round() as i64,
            ),
        };
        p.cells[i].position = new;
        k += 1;
    }
}

/// Conjugate gradient for `A · x = b` with `A` represented as a
/// symmetric sparse Laplacian: diagonal in `diag[i]`, off-diagonals
/// in `adj[i]` as `(j, w_ij)` with `A[i,j] = -w_ij`. Updates `x` in
/// place. Returns the iteration count for diagnostics; we ignore it.
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
            for &(j, w) in &adj[i] {
                s -= w * v[j];
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn cell(name: &str, x: i64, y: i64, w: i64, h: i64, fixed: bool) -> Cell {
        Cell {
            name: name.into(),
            width: w,
            height: h,
            position: Point::new(x, y),
            fixed,
        }
    }

    #[test]
    fn empty_placement_is_noop() {
        let mut p = Placement::new();
        quadratic_place(&mut p, &QuadraticConfig::default());
        assert_eq!(p.cells.len(), 0);
    }

    #[test]
    fn two_pin_net_pulls_free_to_anchor_center() {
        let mut p = Placement::new();
        p.add_row(Row {
            origin: Point::new(0, 0),
            site_width: 1,
            num_sites: 200,
            height: 1,
        });
        p.add_cell(cell("a", 100, 0, 2, 1, true)); // fixed; center @ x=101
        p.add_cell(cell("b", 0, 0, 2, 1, false));
        p.add_net(Net {
            name: "n".into(),
            cell_indices: vec![0, 1],
            weight: 1.0,
        });
        quadratic_place(&mut p, &QuadraticConfig::default());
        let b_center = p.cells[1].position.x + p.cells[1].width / 2;
        // Free cell's center should land at the anchor's center (101)
        // up to integer rounding (B2B for a single 2-pin net is just
        // a single spring; CG converges to the analytic optimum).
        assert!((b_center - 101).abs() <= 1, "got center {b_center}");
    }

    #[test]
    fn high_fanout_net_does_not_collapse_to_clique_centroid() {
        // 1 fixed pin at x=0 plus 4 free cells, all on one net. With
        // the (broken) clique model, free cells would converge to
        // their own centroid (close to each other but pulled toward
        // 0). With B2B the analytic optimum places them all at the
        // same x as the anchor (i.e., HPWL = 0 for x).
        let mut p = Placement::new();
        p.add_row(Row {
            origin: Point::new(0, 0),
            site_width: 1,
            num_sites: 1000,
            height: 1,
        });
        p.add_cell(cell("anchor", 500, 0, 2, 1, true)); // center @ 501
        for k in 0..4 {
            p.add_cell(cell(&format!("free{k}"), k * 100, 0, 2, 1, false));
        }
        let cell_indices: Vec<usize> = (0..5).collect();
        p.add_net(Net {
            name: "fanout".into(),
            cell_indices,
            weight: 1.0,
        });
        quadratic_place(&mut p, &QuadraticConfig::default());
        // All free cells should sit very close to x=500 — within a
        // few DBU is enough for a v1 assertion.
        for k in 1..5 {
            let center = p.cells[k].position.x + p.cells[k].width / 2;
            assert!(
                (center - 501).abs() <= 5,
                "free cell {k} center was {center}, expected ≈ 501",
            );
        }
    }

    #[test]
    fn cg_solves_diagonal_system_in_one_iter() {
        // A = I, b = [1, 2, 3]. CG converges in 1 iter.
        let adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); 3];
        let diag = vec![1.0; 3];
        let b = vec![1.0, 2.0, 3.0];
        let mut x = vec![0.0; 3];
        let iters = conjugate_gradient(&adj, &diag, &b, &mut x, 1e-10, 100);
        assert!(iters <= 2);
        for (xi, bi) in x.iter().zip(b.iter()) {
            assert!((xi - bi).abs() < 1e-6);
        }
    }

    #[test]
    fn fixed_cells_are_not_moved() {
        let mut p = Placement::new();
        p.add_row(Row {
            origin: Point::new(0, 0),
            site_width: 1,
            num_sites: 100,
            height: 1,
        });
        p.add_cell(cell("a", 10, 0, 1, 1, true));
        p.add_cell(cell("b", 90, 0, 1, 1, true));
        p.add_cell(cell("c", 0, 0, 1, 1, false));
        p.add_net(Net {
            name: "n".into(),
            cell_indices: vec![0, 1, 2],
            weight: 1.0,
        });
        quadratic_place(&mut p, &QuadraticConfig::default());
        assert_eq!(p.cells[0].position.x, 10);
        assert_eq!(p.cells[1].position.x, 90);
        // Free cell should land between 10 and 90.
        let c_x = p.cells[2].position.x;
        assert!((10..=90).contains(&c_x), "got x={c_x}");
    }
}
