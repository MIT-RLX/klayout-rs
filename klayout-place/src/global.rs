//! Global placement via force-directed relaxation.
//!
//! Each net is a spring connecting its constituent cells: the spring
//! pulls cells toward the net's centroid with a force proportional to
//! the cell's distance from the centroid times the net weight. Fixed
//! cells don't move and act as anchors. Iterating with damping
//! converges to a low-HPWL placement (analogous to a quadratic
//! placement solution but expressed iteratively, no linear-algebra
//! dependency).
//!
//! The result is *not* legal — cells overlap, sit off the row grid.
//! Run [`legalize`](super::legalize) afterward.

use crate::types::{Cell, Placement};
use klayout_core::Point;

#[derive(Clone, Debug)]
pub struct GlobalConfig {
    pub iterations: u32,
    /// Damping factor in [0, 1]. Higher = slower convergence,
    /// less oscillation. 0.3 is a typical default.
    pub damping: f64,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            iterations: 200,
            damping: 0.3,
        }
    }
}

/// Iteratively move each non-fixed cell toward the centroid of every
/// net it's connected to. Each net contributes weight × (centroid -
/// cell_position) as the per-cell pull. Returns when iterations
/// elapse or HPWL improvement stalls below 1e-3.
pub fn global_place(p: &mut Placement, cfg: &GlobalConfig) {
    let n = p.cells.len();
    if n == 0 {
        return;
    }
    let mut last_hpwl = p.hpwl() as f64;
    for _ in 0..cfg.iterations {
        let mut tx = vec![0.0f64; n];
        let mut ty = vec![0.0f64; n];
        let mut weight_sum = vec![0.0f64; n];
        for net in &p.nets {
            if net.cell_indices.len() < 2 {
                continue;
            }
            // Centroid (cell-center weighted average).
            let mut cx = 0.0;
            let mut cy = 0.0;
            for &ci in &net.cell_indices {
                let c = &p.cells[ci];
                cx += (c.position.x + c.width / 2) as f64;
                cy += (c.position.y + c.height / 2) as f64;
            }
            let nc = net.cell_indices.len() as f64;
            cx /= nc;
            cy /= nc;
            for &ci in &net.cell_indices {
                tx[ci] += net.weight * cx;
                ty[ci] += net.weight * cy;
                weight_sum[ci] += net.weight;
            }
        }
        for i in 0..n {
            if p.cells[i].fixed || weight_sum[i] == 0.0 {
                continue;
            }
            let target_x = tx[i] / weight_sum[i] - p.cells[i].width as f64 / 2.0;
            let target_y = ty[i] / weight_sum[i] - p.cells[i].height as f64 / 2.0;
            let dx = (target_x - p.cells[i].position.x as f64) * (1.0 - cfg.damping);
            let dy = (target_y - p.cells[i].position.y as f64) * (1.0 - cfg.damping);
            let new_x = (p.cells[i].position.x as f64 + dx).round() as i64;
            let new_y = (p.cells[i].position.y as f64 + dy).round() as i64;
            p.cells[i].position = Point::new(new_x, new_y);
        }
        let now_hpwl = p.hpwl() as f64;
        if (last_hpwl - now_hpwl).abs() < 1.0 {
            break;
        }
        last_hpwl = now_hpwl;
    }
}

/// Pre-condition utility: if any cell starts at (0,0) and isn't fixed,
/// scatter it inside the row bbox so the first force-iteration has
/// gradients to work with. Optional — solver works without it but
/// converges faster from a non-degenerate start.
pub fn scatter_random(p: &mut Placement, seed: u64) {
    use std::cell::RefCell;
    thread_local! { static RNG: RefCell<u64> = const { RefCell::new(0) }; }
    RNG.with(|r| *r.borrow_mut() = seed);
    fn next() -> u64 {
        RNG.with(|r| {
            let mut s = r.borrow_mut();
            *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            *s
        })
    }
    // Compute the legal placement window. If any of these collapse
    // (rows is empty, or all rows degenerate to the same x/y), there
    // is nothing meaningful to randomize, so bail.
    let (Some(xmin), Some(xmax), Some(ymin), Some(ymax)) = (
        p.rows.iter().map(|r| r.origin.x).min(),
        p.rows.iter().map(|r| r.x_max()).max(),
        p.rows.iter().map(|r| r.origin.y).min(),
        p.rows.iter().map(|r| r.y_max()).max(),
    ) else {
        return;
    };
    let _ = next; // keep the function used
    for c in p.cells.iter_mut() {
        if c.fixed {
            continue;
        }
        let x = (next() as i64 % (xmax - xmin).max(1)) + xmin;
        let y = (next() as i64 % (ymax - ymin).max(1)) + ymin;
        c.position = Point::new(x, y);
    }
}

#[allow(dead_code)]
fn _silence_unused_cell(_c: &Cell) {}
