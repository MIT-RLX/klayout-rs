//! Pluggable placer engines.
//!
//! Pre-trait, callers wired `global_place(&mut p, &cfg)` directly.
//! That works but ties every flow to one algorithm. The [`Placer`]
//! trait abstracts the contract — "given a `Placement` with cells +
//! nets + rows, mutate `cell.position` to minimise HPWL" — and lets
//! callers swap implementations without touching call sites.
//!
//! v1 ships:
//! * [`ForceDirectedPlacer`] — wraps the existing
//!   [`crate::global_place`] kernel.
//! * [`QuadraticPlacer`] — solves the same minimum-HPWL objective
//!   in closed form via a small Jacobi iteration on the connectivity
//!   Laplacian. Faster convergence on highly-connected designs;
//!   doesn't honour density constraints (a v2 concern).

use crate::global::{global_place, GlobalConfig};
use crate::types::Placement;
use klayout_core::Point;

pub trait Placer: Send + Sync {
    fn place(&self, p: &mut Placement);
}

#[derive(Default)]
pub struct ForceDirectedPlacer {
    pub config: GlobalConfig,
}

impl Placer for ForceDirectedPlacer {
    fn place(&self, p: &mut Placement) {
        global_place(p, &self.config);
    }
}

/// Quadratic placer using Jacobi iteration on the connectivity
/// Laplacian. Each cell's position is updated as the weight-sum-
/// normalised average of its connected fixed-anchor positions and
/// neighbour positions. Iterates until the position delta drops
/// below `tolerance`.
pub struct QuadraticPlacer {
    pub iterations: u32,
    pub tolerance: f64,
}

impl Default for QuadraticPlacer {
    fn default() -> Self {
        Self {
            iterations: 100,
            tolerance: 0.5,
        }
    }
}

impl Placer for QuadraticPlacer {
    fn place(&self, p: &mut Placement) {
        let n = p.cells.len();
        if n == 0 {
            return;
        }
        // Build per-cell connectivity weights: each net contributes
        // its weight to every (i, j) member pair.
        let mut neighbour_weight: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
        let mut total_weight: Vec<f64> = vec![0.0; n];
        for net in &p.nets {
            let m = net.cell_indices.len() as f64;
            if m < 2.0 {
                continue;
            }
            // Clique model: each cell connects to all others in the
            // net with weight `net.weight / (m - 1)`.
            let w = net.weight / (m - 1.0);
            for &i in &net.cell_indices {
                for &j in &net.cell_indices {
                    if i == j {
                        continue;
                    }
                    neighbour_weight[i].push((j, w));
                    total_weight[i] += w;
                }
            }
        }
        for _ in 0..self.iterations {
            let mut max_delta = 0.0f64;
            let snapshot: Vec<Point> = p.cells.iter().map(|c| c.position).collect();
            for i in 0..n {
                if p.cells[i].fixed || total_weight[i] == 0.0 {
                    continue;
                }
                let mut tx = 0.0f64;
                let mut ty = 0.0f64;
                for &(j, w) in &neighbour_weight[i] {
                    tx += w * (snapshot[j].x as f64 + p.cells[j].width as f64 / 2.0);
                    ty += w * (snapshot[j].y as f64 + p.cells[j].height as f64 / 2.0);
                }
                let target_x = tx / total_weight[i] - p.cells[i].width as f64 / 2.0;
                let target_y = ty / total_weight[i] - p.cells[i].height as f64 / 2.0;
                let new_x = target_x.round() as i64;
                let new_y = target_y.round() as i64;
                let dx = (new_x - p.cells[i].position.x).abs() as f64;
                let dy = (new_y - p.cells[i].position.y).abs() as f64;
                if dx + dy > max_delta {
                    max_delta = dx + dy;
                }
                p.cells[i].position = Point::new(new_x, new_y);
            }
            if max_delta < self.tolerance {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn make() -> Placement {
        let mut p = Placement::new();
        p.add_row(Row {
            origin: Point::new(0, 0),
            site_width: 1,
            num_sites: 100,
            height: 1,
        });
        p.add_cell(Cell {
            name: "fixed".into(),
            width: 1,
            height: 1,
            position: Point::new(50, 0),
            fixed: true,
        });
        p.add_cell(Cell {
            name: "free".into(),
            width: 1,
            height: 1,
            position: Point::new(0, 0),
            fixed: false,
        });
        p.add_net(Net {
            name: "n".into(),
            cell_indices: vec![0, 1],
            weight: 1.0,
        });
        p
    }

    #[test]
    fn force_directed_engine_pulls_free_toward_fixed() {
        let mut p = make();
        let placer = ForceDirectedPlacer::default();
        placer.place(&mut p);
        assert!(p.cells[1].position.x > 0);
        assert_eq!(p.cells[0].position.x, 50);
    }

    #[test]
    fn quadratic_engine_converges_to_anchor() {
        let mut p = make();
        let placer = QuadraticPlacer::default();
        placer.place(&mut p);
        // Two-cell net with one fixed → free cell should sit at the
        // fixed cell's center (50).
        let free_center = p.cells[1].position.x + p.cells[1].width / 2;
        assert!((free_center - 50).abs() <= 1, "got center {free_center}");
        assert_eq!(p.cells[0].position.x, 50);
    }

    #[test]
    fn engines_are_swappable_via_trait_object() {
        let placers: Vec<Box<dyn Placer>> = vec![
            Box::<ForceDirectedPlacer>::default(),
            Box::<QuadraticPlacer>::default(),
        ];
        for placer in placers {
            let mut p = make();
            placer.place(&mut p);
            // Both should leave the fixed cell anchored.
            assert_eq!(p.cells[0].position.x, 50);
        }
    }
}
