//! Legalization — snap cells to row/site grid and resolve overlaps.
//!
//! Tetris-style algorithm:
//! 1. Assign each cell to its nearest row(s) — multi-row cells span
//!    `cell.height / row.height` consecutive rows.
//! 2. Within each row, sort cells by current x.
//! 3. Sweep left-to-right per row, ensuring multi-row cells reserve
//!    space on every row they span.
//! 4. If a cell overflows, mark `all_ok = false` but still place it.
//!
//! Returns `true` on full success, `false` if any cells couldn't fit.

use crate::types::Placement;
use klayout_core::Point;
use std::collections::HashMap;

pub fn legalize(p: &mut Placement) -> bool {
    if p.rows.is_empty() {
        return false;
    }
    // Step 1: assign each non-fixed cell to its bottom row, and
    // record how many rows tall it is.
    let row_count = p.rows.len();
    let row_ys: Vec<(usize, i64)> = p
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| (i, r.origin.y))
        .collect();
    // For each row, the cells whose *bottom row* lands on that row.
    let mut row_assignments: Vec<Vec<usize>> = vec![Vec::new(); row_count];
    // For each cell, how many rows it spans.
    let mut span: HashMap<usize, usize> = HashMap::new();
    for (ci, c) in p.cells.iter().enumerate() {
        if c.fixed {
            continue;
        }
        let cy = c.position.y;
        let mut best = 0;
        let mut best_d = i64::MAX;
        for &(i, y) in &row_ys {
            let dy = (cy - y).abs();
            if dy < best_d {
                best_d = dy;
                best = i;
            }
        }
        // How many rows does this cell occupy?
        let row_h = p.rows[best].height.max(1);
        let n_rows = ((c.height + row_h - 1) / row_h).max(1) as usize;
        span.insert(ci, n_rows);
        row_assignments[best].push(ci);
    }

    // Per-row frontier-x — cells in higher rows must respect lower
    // rows' frontiers when they span both.
    let mut frontier_x: Vec<i64> = p.rows.iter().map(|r| r.origin.x).collect();
    let mut all_ok = true;

    // Process rows bottom-up so multi-row cells claim space in
    // higher rows after their bottom row has been positioned.
    for ri in 0..row_count {
        let row = &p.rows[ri];
        let mut ci_list = row_assignments[ri].clone();
        ci_list.sort_by_key(|&ci| p.cells[ci].position.x);
        for &ci in ci_list.iter() {
            let n_rows = *span.get(&ci).unwrap_or(&1);
            let top_row = (ri + n_rows - 1).min(row_count - 1);
            // The cell's left edge must be ≥ max frontier across all
            // spanned rows.
            let max_front = frontier_x[ri..=top_row]
                .iter()
                .copied()
                .max()
                .unwrap_or(frontier_x[ri]);
            let mut x = p.cells[ci].position.x.max(max_front);
            x = row.snap_x(x);
            while x < max_front {
                x += row.site_width;
            }
            let cell_right = x + p.cells[ci].width;
            if cell_right > row.x_max() {
                all_ok = false;
                x = row.origin.x;
            }
            p.cells[ci].position = Point::new(x, row.origin.y);
            // Update every spanned row's frontier.
            let new_front = x + p.cells[ci].width;
            for f in &mut frontier_x[ri..=top_row] {
                *f = new_front.max(*f);
            }
        }
    }
    all_ok
}
