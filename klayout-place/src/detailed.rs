//! Detailed placement — pairwise swap moves to reduce HPWL.

use crate::types::Placement;

#[derive(Clone, Debug)]
pub struct DetailedConfig {
    pub iterations: u32,
}

impl Default for DetailedConfig {
    fn default() -> Self {
        Self { iterations: 5 }
    }
}

/// Try swapping every adjacent (in-row) cell pair; keep the swap iff
/// HPWL decreases. Repeat until no swap improves or iteration limit.
pub fn detailed_place(p: &mut Placement, cfg: &DetailedConfig) {
    for _ in 0..cfg.iterations {
        let initial_hpwl = p.hpwl();
        // Group cells by row (by integer y).
        let mut by_row: std::collections::HashMap<i64, Vec<usize>> =
            std::collections::HashMap::new();
        for (ci, c) in p.cells.iter().enumerate() {
            if c.fixed {
                continue;
            }
            by_row.entry(c.position.y).or_default().push(ci);
        }
        let mut any_swap = false;
        for (_, mut row_cells) in by_row {
            row_cells.sort_by_key(|&ci| p.cells[ci].position.x);
            for w in 0..row_cells.len().saturating_sub(1) {
                let a = row_cells[w];
                let b = row_cells[w + 1];
                if try_swap(p, a, b) {
                    any_swap = true;
                }
            }
        }
        if !any_swap || p.hpwl() == initial_hpwl {
            break;
        }
    }
}

fn try_swap(p: &mut Placement, a: usize, b: usize) -> bool {
    if p.cells[a].fixed || p.cells[b].fixed {
        return false;
    }
    let before = p.hpwl();
    let pa = p.cells[a].position;
    let pb = p.cells[b].position;
    p.cells[a].position = pb;
    p.cells[b].position = pa;
    let after = p.hpwl();
    if after >= before {
        // Revert.
        p.cells[a].position = pa;
        p.cells[b].position = pb;
        false
    } else {
        true
    }
}
