//! Pathfinder global router (McMurchie & Ebeling 1995).
//!
//! Where the [`crate::global::route_global`] kernel routes each net
//! once in net order with a present-congestion penalty, **Pathfinder**
//! is iterative: every net is ripped up and re-routed every
//! iteration, with edge costs that grow over time on edges that have
//! been over-subscribed in *previous* iterations. The history term
//! is what gives Pathfinder its name — edges that are persistently
//! over-used become more expensive, eventually pushing nets onto
//! alternative paths.
//!
//! ## Cost function
//!
//! For each routing edge `e`, the per-iteration cost is
//!
//! ```text
//!     cost(e) = base(e) · (1 + h(e) · h_factor) · p_factor(e)
//! ```
//!
//! where
//!
//! * `base(e)` is the geometric cost (1 for a unit-length GCell hop).
//! * `h(e)` is the **history cost**, accumulated across iterations
//!   on overused edges. In iteration `i`, after routing all nets, we
//!   set `h(e) ← h(e) + h_increment` for every `e` with `used(e) >
//!   capacity(e)`.
//! * `p_factor(e)` is the **present congestion penalty**:
//!     - `1` if `used(e) ≤ capacity(e)`
//!     - `1 + p_slope · (used(e) − capacity(e) + 1) / capacity(e)`
//!       otherwise. `p_slope` ramps up across iterations so early
//!       iterations are tolerant (find the topology) and later ones
//!       are punitive (resolve overflow).
//!
//! ## Per-iteration loop
//!
//! ```text
//! for iter = 1..max_iter:
//!     reset_present(used := 0)
//!     for net in nets:
//!         path = a_star(net.src, net.sink, cost)
//!         apply_usage(path)
//!     overflow = count_overflowed_edges()
//!     if overflow == 0: done
//!     update_history()
//!     p_slope *= p_growth
//! ```
//!
//! Convergence isn't guaranteed in pathological cases (truly
//! over-subscribed channels), but on synthesizable designs Pathfinder
//! reduces overflow to zero in 5–30 iterations on the standard ISPD
//! global-routing benchmarks.
//!
//! ## What's not in this v1
//!
//! * **Multi-pin nets via Steiner trees.** Each net is currently a
//!   set of source-sink 2-pin pairs — the caller can pre-decompose a
//!   k-pin net via [`crate::rsmt::rsmt`] and submit each tree edge as
//!   a separate request, but routing a true k-pin tree as a unified
//!   Steiner shortest-path arborescence (Bingham/Madden 2013, Liu et
//!   al. 2018) is the obvious next upgrade.
//! * **Layer assignment.** v1 routes on a single 2-D GCell graph; a
//!   real flow needs per-layer GCell graphs with via-cost transitions
//!   between them.
//! * **NDR (non-default rules)** for shielded / wide / multi-track
//!   nets.

use crate::global::{CapacityGrid, GCellGrid, GCellId};
use klayout_core::Point;
use smol_str::SmolStr;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

#[derive(Clone, Debug)]
pub struct PathfinderRequest {
    pub net_name: SmolStr,
    pub src: Point,
    pub sink: Point,
}

#[derive(Clone, Debug)]
pub struct PathfinderRoute {
    pub net_name: SmolStr,
    pub gcells: Vec<GCellId>,
}

#[derive(Clone, Debug)]
pub struct PathfinderConfig {
    pub max_iterations: u32,
    pub h_factor: f64,
    pub h_increment: f64,
    pub p_initial_slope: f64,
    pub p_growth: f64,
    /// Maximum permissible per-edge overflow at iteration termination.
    /// Default `0` means "must be fully congestion-free".
    pub overflow_target: u32,
}

impl Default for PathfinderConfig {
    fn default() -> Self {
        Self {
            max_iterations: 30,
            h_factor: 0.4,
            h_increment: 1.0,
            p_initial_slope: 0.5,
            p_growth: 1.5,
            overflow_target: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PathfinderResult {
    pub routes: Vec<PathfinderRoute>,
    /// Iterations actually consumed before convergence (or
    /// `cfg.max_iterations` if the loop exhausted without reaching
    /// `overflow_target`).
    pub iterations: u32,
    /// Final residual overflow — number of (edge, used > capacity)
    /// occurrences. Zero means the result is congestion-free.
    pub residual_overflow: u32,
}

/// Per-edge history cost, indexed identically to `CapacityGrid`.
#[derive(Clone, Debug)]
struct History {
    east: Vec<f64>,
    north: Vec<f64>,
}

impl History {
    fn empty(grid: &GCellGrid) -> Self {
        let n = grid.cell_count();
        Self {
            east: vec![0.0; n],
            north: vec![0.0; n],
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum Dir {
    East,
    West,
    North,
    South,
}

pub fn route_pathfinder(
    grid: &GCellGrid,
    capacity: &mut CapacityGrid,
    requests: &[PathfinderRequest],
    cfg: &PathfinderConfig,
) -> PathfinderResult {
    let mut history = History::empty(grid);
    let mut p_slope = cfg.p_initial_slope;
    let mut routes: Vec<PathfinderRoute> = Vec::with_capacity(requests.len());

    for iter in 0..cfg.max_iterations {
        // Reset per-iteration usage and rebuild routes from scratch.
        capacity.east_used.fill(0);
        capacity.north_used.fill(0);
        routes.clear();

        for req in requests {
            let path = pathfinder_a_star(grid, capacity, &history, req, cfg, p_slope);
            if let Some(path) = path {
                apply_usage(grid, capacity, &path);
                routes.push(PathfinderRoute {
                    net_name: req.net_name.clone(),
                    gcells: path,
                });
            } else {
                // Unroutable — emit empty path; the iteration will
                // still count overflow on the rest.
                routes.push(PathfinderRoute {
                    net_name: req.net_name.clone(),
                    gcells: Vec::new(),
                });
            }
        }

        let overflow = count_overflow(grid, capacity);
        if overflow <= cfg.overflow_target {
            return PathfinderResult {
                routes,
                iterations: iter + 1,
                residual_overflow: overflow,
            };
        }

        update_history(grid, capacity, &mut history, cfg);
        p_slope *= cfg.p_growth;
    }

    let residual_overflow = count_overflow(grid, capacity);
    PathfinderResult {
        routes,
        iterations: cfg.max_iterations,
        residual_overflow,
    }
}

fn count_overflow(grid: &GCellGrid, cap: &CapacityGrid) -> u32 {
    let mut total = 0u32;
    for i in 0..grid.cell_count() {
        if cap.east_used[i] > cap.east_capacity[i] {
            total += cap.east_used[i] - cap.east_capacity[i];
        }
        if cap.north_used[i] > cap.north_capacity[i] {
            total += cap.north_used[i] - cap.north_capacity[i];
        }
    }
    total
}

fn update_history(grid: &GCellGrid, cap: &CapacityGrid, hist: &mut History, cfg: &PathfinderConfig) {
    for i in 0..grid.cell_count() {
        if cap.east_used[i] > cap.east_capacity[i] {
            hist.east[i] += cfg.h_increment;
        }
        if cap.north_used[i] > cap.north_capacity[i] {
            hist.north[i] += cfg.h_increment;
        }
    }
}

fn apply_usage(grid: &GCellGrid, cap: &mut CapacityGrid, path: &[GCellId]) {
    for w in path.windows(2) {
        let a = w[0];
        let b = w[1];
        if b.gx > a.gx {
            cap.east_used[grid_idx(grid, a)] += 1;
        } else if b.gx < a.gx {
            cap.east_used[grid_idx(grid, b)] += 1;
        } else if b.gy > a.gy {
            cap.north_used[grid_idx(grid, a)] += 1;
        } else if b.gy < a.gy {
            cap.north_used[grid_idx(grid, b)] += 1;
        }
    }
}

fn grid_idx(grid: &GCellGrid, c: GCellId) -> usize {
    c.gy as usize * grid.n_cols as usize + c.gx as usize
}

fn neighbors(grid: &GCellGrid, c: GCellId) -> Vec<(GCellId, Dir)> {
    let mut out = Vec::with_capacity(4);
    if c.gx + 1 < grid.n_cols {
        out.push((
            GCellId {
                gx: c.gx + 1,
                gy: c.gy,
            },
            Dir::East,
        ));
    }
    if c.gx > 0 {
        out.push((
            GCellId {
                gx: c.gx - 1,
                gy: c.gy,
            },
            Dir::West,
        ));
    }
    if c.gy + 1 < grid.n_rows {
        out.push((
            GCellId {
                gx: c.gx,
                gy: c.gy + 1,
            },
            Dir::North,
        ));
    }
    if c.gy > 0 {
        out.push((
            GCellId {
                gx: c.gx,
                gy: c.gy - 1,
            },
            Dir::South,
        ));
    }
    out
}

#[derive(Copy, Clone, PartialEq)]
struct HeapEntry {
    cost: f64,
    cell: GCellId,
}

impl Eq for HeapEntry {}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap on cost. NaN can't appear: all our cost ops are
        // well-defined finite arithmetic.
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn pathfinder_a_star(
    grid: &GCellGrid,
    cap: &CapacityGrid,
    hist: &History,
    req: &PathfinderRequest,
    cfg: &PathfinderConfig,
    p_slope: f64,
) -> Option<Vec<GCellId>> {
    let src = grid.snap(req.src);
    let sink = grid.snap(req.sink);
    if src == sink {
        return Some(vec![src]);
    }

    let h = |c: GCellId| -> f64 {
        // Manhattan in GCell space → admissible heuristic for A*.
        let dx = (c.gx as i64 - sink.gx as i64).abs();
        let dy = (c.gy as i64 - sink.gy as i64).abs();
        (dx + dy) as f64
    };

    let mut g_score: HashMap<GCellId, f64> = HashMap::new();
    let mut came_from: HashMap<GCellId, GCellId> = HashMap::new();
    g_score.insert(src, 0.0);
    let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();
    heap.push(HeapEntry {
        cost: h(src),
        cell: src,
    });

    while let Some(cur) = heap.pop() {
        if cur.cell == sink {
            return Some(reconstruct(&came_from, sink));
        }
        let cur_g = *g_score.get(&cur.cell).unwrap_or(&f64::INFINITY);
        if cur.cost > cur_g + h(cur.cell) + 1e-9 {
            continue; // stale entry
        }
        for (nb, dir) in neighbors(grid, cur.cell) {
            let edge = edge_cost(grid, cap, hist, cur.cell, dir, cfg, p_slope);
            let tentative = cur_g + edge;
            if tentative < *g_score.get(&nb).unwrap_or(&f64::INFINITY) {
                g_score.insert(nb, tentative);
                came_from.insert(nb, cur.cell);
                heap.push(HeapEntry {
                    cost: tentative + h(nb),
                    cell: nb,
                });
            }
        }
    }
    None
}

fn edge_cost(
    grid: &GCellGrid,
    cap: &CapacityGrid,
    hist: &History,
    from: GCellId,
    dir: Dir,
    cfg: &PathfinderConfig,
    p_slope: f64,
) -> f64 {
    let (used, capacity, h_e) = match dir {
        Dir::East => {
            let i = grid_idx(grid, from);
            (cap.east_used[i], cap.east_capacity[i], hist.east[i])
        }
        Dir::West => {
            let dst = GCellId {
                gx: from.gx - 1,
                gy: from.gy,
            };
            let i = grid_idx(grid, dst);
            (cap.east_used[i], cap.east_capacity[i], hist.east[i])
        }
        Dir::North => {
            let i = grid_idx(grid, from);
            (cap.north_used[i], cap.north_capacity[i], hist.north[i])
        }
        Dir::South => {
            let dst = GCellId {
                gx: from.gx,
                gy: from.gy - 1,
            };
            let i = grid_idx(grid, dst);
            (cap.north_used[i], cap.north_capacity[i], hist.north[i])
        }
    };

    let base = 1.0;
    let h_term = 1.0 + h_e * cfg.h_factor;
    let p_term = if used + 1 > capacity {
        1.0 + p_slope * (used as f64 + 1.0 - capacity as f64) / capacity.max(1) as f64
    } else {
        1.0
    };
    base * h_term * p_term
}

fn reconstruct(came_from: &HashMap<GCellId, GCellId>, sink: GCellId) -> Vec<GCellId> {
    let mut path = vec![sink];
    let mut cur = sink;
    while let Some(&prev) = came_from.get(&cur) {
        path.push(prev);
        cur = prev;
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::Bbox;

    fn grid_4x4() -> GCellGrid {
        GCellGrid::new(
            Bbox::new(Point::new(0, 0), Point::new(400, 400)),
            100,
            100,
        )
    }

    #[test]
    fn single_uncongested_net_routes_via_manhattan() {
        let grid = grid_4x4();
        let mut cap = CapacityGrid::uniform(&grid, 4);
        let req = PathfinderRequest {
            net_name: "n".into(),
            src: Point::new(50, 50),
            sink: Point::new(350, 350),
        };
        let result = route_pathfinder(&grid, &mut cap, &[req], &PathfinderConfig::default());
        assert_eq!(result.iterations, 1);
        assert_eq!(result.residual_overflow, 0);
        assert!(!result.routes[0].gcells.is_empty());
    }

    #[test]
    fn two_nets_share_when_capacity_allows() {
        let grid = grid_4x4();
        let mut cap = CapacityGrid::uniform(&grid, 4);
        let reqs = vec![
            PathfinderRequest {
                net_name: "a".into(),
                src: Point::new(50, 50),
                sink: Point::new(350, 50),
            },
            PathfinderRequest {
                net_name: "b".into(),
                src: Point::new(50, 50),
                sink: Point::new(350, 50),
            },
        ];
        let result = route_pathfinder(&grid, &mut cap, &reqs, &PathfinderConfig::default());
        assert_eq!(result.residual_overflow, 0);
    }

    #[test]
    fn over_capacity_resolves_via_history() {
        // Capacity of 1 per edge, 3 nets all going through the same
        // bottleneck → without history, the third would overflow;
        // Pathfinder should reroute over multiple iterations.
        let grid = GCellGrid::new(
            Bbox::new(Point::new(0, 0), Point::new(300, 300)),
            100,
            100,
        );
        let mut cap = CapacityGrid::uniform(&grid, 1);
        let reqs = (0..3)
            .map(|i| PathfinderRequest {
                net_name: SmolStr::from(format!("n{i}")),
                src: Point::new(50, 150),
                sink: Point::new(250, 150),
            })
            .collect::<Vec<_>>();
        let result = route_pathfinder(&grid, &mut cap, &reqs, &PathfinderConfig::default());
        // 3 nets through a 3-row × 3-col grid with capacity 1 per
        // edge: one routes straight across (uses east edges in row
        // 1), the others must detour through row 0 or row 2. The
        // total east-edge capacity along any horizontal traversal is
        // exactly 3 (one per row at each column), so the routing is
        // feasible — Pathfinder must find it.
        assert_eq!(
            result.residual_overflow, 0,
            "expected congestion-free after history-driven rerouting"
        );
    }

    #[test]
    fn unsolvable_terminates_at_max_iterations() {
        // 2 nets, capacity 0 everywhere → no feasible solution.
        let grid = GCellGrid::new(
            Bbox::new(Point::new(0, 0), Point::new(200, 200)),
            100,
            100,
        );
        let mut cap = CapacityGrid::uniform(&grid, 0);
        let reqs = vec![PathfinderRequest {
            net_name: "n".into(),
            src: Point::new(50, 50),
            sink: Point::new(150, 50),
        }];
        let cfg = PathfinderConfig {
            max_iterations: 3,
            ..Default::default()
        };
        let result = route_pathfinder(&grid, &mut cap, &reqs, &cfg);
        // With capacity 0, the routing is over-subscribed; the
        // present-penalty is finite so a path is still produced,
        // just at high cost. Either residual_overflow > 0 or the
        // route is empty; in both cases iter == max_iterations.
        assert_eq!(result.iterations, 3);
    }
}
