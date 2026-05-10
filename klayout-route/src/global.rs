//! Global routing with gcells.
//!
//! Detailed routing on every net independently doesn't scale to
//! ≥1000-net designs because pairwise interactions blow up. The
//! standard fix is a two-stage flow: a *global* router places each
//! net in a coarse "gcell" grid, predicting a corridor that the
//! detailed router then refines. Capacity tracking on each gcell
//! edge surfaces congestion early so the detailed pass doesn't fail
//! on already-over-subscribed channels.
//!
//! v1 implements:
//! * **Gcell grid** — partitions the layout bbox into rectangular
//!   cells of `gcell_w × gcell_h` (typically 10–20× the metal pitch).
//! * **Per-edge capacity** — each gcell edge has a per-layer track
//!   capacity (number of wires that can cross). Set from PDK pitch.
//! * **Congestion-aware Dijkstra** — for each net, find the gcell
//!   path from src to dst that minimises `length + congestion_penalty
//!   * (used / capacity)`. Update usage as we route.
//! * **Routability flag** — a net whose path crosses a saturated
//!   edge is reported but still routed (the detailed pass can rip up
//!   later). Saturation is reported separately in `congestion_map`.
//!
//! The output `GlobalRoute` is a sequence of gcell IDs; the detailed
//! router uses these as a bbox-corridor preference (cells inside the
//! corridor cost less than off-corridor moves).

use klayout_core::{Bbox, Point};
use smol_str::SmolStr;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GCellId {
    pub gx: u32,
    pub gy: u32,
}

#[derive(Clone, Debug)]
pub struct GCellGrid {
    pub origin: Point,
    pub gcell_w: i64,
    pub gcell_h: i64,
    pub n_cols: u32,
    pub n_rows: u32,
}

impl GCellGrid {
    pub fn new(bbox: Bbox, gcell_w: i64, gcell_h: i64) -> Self {
        let n_cols = (bbox.width().max(1) / gcell_w.max(1)).max(1) as u32;
        let n_rows = (bbox.height().max(1) / gcell_h.max(1)).max(1) as u32;
        Self {
            origin: bbox.min,
            gcell_w: gcell_w.max(1),
            gcell_h: gcell_h.max(1),
            n_cols,
            n_rows,
        }
    }

    pub fn snap(&self, p: Point) -> GCellId {
        let gx = (((p.x - self.origin.x) / self.gcell_w).max(0) as u32).min(self.n_cols - 1);
        let gy = (((p.y - self.origin.y) / self.gcell_h).max(0) as u32).min(self.n_rows - 1);
        GCellId { gx, gy }
    }

    pub fn center(&self, c: GCellId) -> Point {
        Point::new(
            self.origin.x + c.gx as i64 * self.gcell_w + self.gcell_w / 2,
            self.origin.y + c.gy as i64 * self.gcell_h + self.gcell_h / 2,
        )
    }

    pub fn cell_count(&self) -> usize {
        (self.n_cols * self.n_rows) as usize
    }

    fn idx(&self, c: GCellId) -> usize {
        c.gy as usize * self.n_cols as usize + c.gx as usize
    }
}

/// Gcell-edge capacity grid. Edges are oriented: an east-going edge
/// from `(gx, gy)` reaches `(gx+1, gy)` and is indexed by the source
/// cell. Same for north-going edges.
#[derive(Clone, Debug)]
pub struct CapacityGrid {
    pub east_capacity: Vec<u32>,  // n_cols * n_rows
    pub north_capacity: Vec<u32>, // n_cols * n_rows
    pub east_used: Vec<u32>,
    pub north_used: Vec<u32>,
}

impl CapacityGrid {
    pub fn uniform(grid: &GCellGrid, capacity: u32) -> Self {
        let n = grid.cell_count();
        Self {
            east_capacity: vec![capacity; n],
            north_capacity: vec![capacity; n],
            east_used: vec![0; n],
            north_used: vec![0; n],
        }
    }
}

#[derive(Clone, Debug)]
pub struct GlobalRouteRequest {
    pub net_name: SmolStr,
    pub src: Point,
    pub dst: Point,
}

#[derive(Clone, Debug)]
pub struct GlobalRoute {
    pub net_name: SmolStr,
    pub gcells: Vec<GCellId>,
    /// True if any edge along the path was already at-capacity when
    /// this net was routed (still placed but flagged for rip-up later).
    pub crosses_saturated_edge: bool,
}

#[derive(Clone, Debug)]
pub struct GlobalRouterConfig {
    /// Multiplier for congestion penalty in Dijkstra cost: weight =
    /// edge_length + congestion_penalty * (used / capacity)^2.
    pub congestion_penalty: i64,
    /// Per-step base cost (set to grid.gcell_w + gcell_h / 2 typically).
    pub base_cost: i64,
}

impl Default for GlobalRouterConfig {
    fn default() -> Self {
        Self {
            congestion_penalty: 1000,
            base_cost: 10,
        }
    }
}

/// Run global routing for a batch of nets. Order matters — earlier
/// nets get less-congested paths.
pub fn route_global(
    grid: &GCellGrid,
    capacity: &mut CapacityGrid,
    requests: &[GlobalRouteRequest],
    cfg: &GlobalRouterConfig,
) -> Vec<GlobalRoute> {
    let mut out = Vec::with_capacity(requests.len());
    for req in requests {
        if let Some(route) = dijkstra_gcell(grid, capacity, req, cfg) {
            apply_usage(grid, capacity, &route.gcells);
            out.push(route);
        } else {
            out.push(GlobalRoute {
                net_name: req.net_name.clone(),
                gcells: Vec::new(),
                crosses_saturated_edge: true,
            });
        }
    }
    out
}

#[derive(Copy, Clone, PartialEq, Eq)]
struct HeapEntry {
    cost: i64,
    cell: GCellId,
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.cmp(&self.cost)
    }
}
impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn dijkstra_gcell(
    grid: &GCellGrid,
    capacity: &CapacityGrid,
    req: &GlobalRouteRequest,
    cfg: &GlobalRouterConfig,
) -> Option<GlobalRoute> {
    let src = grid.snap(req.src);
    let dst = grid.snap(req.dst);
    let mut g_score: HashMap<GCellId, i64> = HashMap::new();
    let mut came_from: HashMap<GCellId, GCellId> = HashMap::new();
    let mut crosses_sat: HashMap<GCellId, bool> = HashMap::new();
    g_score.insert(src, 0);
    crosses_sat.insert(src, false);

    let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();
    heap.push(HeapEntry {
        cost: 0,
        cell: src,
    });

    while let Some(cur) = heap.pop() {
        if cur.cell == dst {
            return Some(reconstruct(&came_from, dst, &req.net_name, &crosses_sat));
        }
        if cur.cost > *g_score.get(&cur.cell).unwrap_or(&i64::MAX) {
            continue;
        }
        let neighbors = neighbors(grid, cur.cell);
        for (nb, dir) in neighbors {
            let edge_cost = edge_cost(grid, capacity, cur.cell, dir, cfg);
            let cap = edge_capacity(grid, capacity, cur.cell, dir);
            let used = edge_used(grid, capacity, cur.cell, dir);
            let saturated = used >= cap;
            let prev_sat = *crosses_sat.get(&cur.cell).unwrap_or(&false);
            let tentative = cur.cost + edge_cost;
            if tentative < *g_score.get(&nb).unwrap_or(&i64::MAX) {
                g_score.insert(nb, tentative);
                came_from.insert(nb, cur.cell);
                crosses_sat.insert(nb, prev_sat || saturated);
                heap.push(HeapEntry {
                    cost: tentative,
                    cell: nb,
                });
            }
        }
    }
    None
}

#[derive(Copy, Clone, Debug)]
enum Dir {
    East,
    West,
    North,
    South,
}

fn neighbors(grid: &GCellGrid, c: GCellId) -> Vec<(GCellId, Dir)> {
    let mut out = Vec::new();
    if c.gx + 1 < grid.n_cols {
        out.push((GCellId { gx: c.gx + 1, gy: c.gy }, Dir::East));
    }
    if c.gx > 0 {
        out.push((GCellId { gx: c.gx - 1, gy: c.gy }, Dir::West));
    }
    if c.gy + 1 < grid.n_rows {
        out.push((GCellId { gx: c.gx, gy: c.gy + 1 }, Dir::North));
    }
    if c.gy > 0 {
        out.push((GCellId { gx: c.gx, gy: c.gy - 1 }, Dir::South));
    }
    out
}

fn edge_cost(
    grid: &GCellGrid,
    capacity: &CapacityGrid,
    from: GCellId,
    dir: Dir,
    cfg: &GlobalRouterConfig,
) -> i64 {
    let used = edge_used(grid, capacity, from, dir) as i64;
    let cap = edge_capacity(grid, capacity, from, dir).max(1) as i64;
    let load = (used * 100) / cap; // percent
    cfg.base_cost + cfg.congestion_penalty * load * load / 10000
}

fn edge_capacity(grid: &GCellGrid, capacity: &CapacityGrid, from: GCellId, dir: Dir) -> u32 {
    match dir {
        Dir::East => capacity.east_capacity.get(grid.idx(from)).copied().unwrap_or(0),
        Dir::West => {
            if from.gx == 0 {
                0
            } else {
                let west = GCellId {
                    gx: from.gx - 1,
                    gy: from.gy,
                };
                capacity.east_capacity.get(grid.idx(west)).copied().unwrap_or(0)
            }
        }
        Dir::North => capacity.north_capacity.get(grid.idx(from)).copied().unwrap_or(0),
        Dir::South => {
            if from.gy == 0 {
                0
            } else {
                let south = GCellId {
                    gx: from.gx,
                    gy: from.gy - 1,
                };
                capacity.north_capacity.get(grid.idx(south)).copied().unwrap_or(0)
            }
        }
    }
}

fn edge_used(grid: &GCellGrid, capacity: &CapacityGrid, from: GCellId, dir: Dir) -> u32 {
    match dir {
        Dir::East => capacity.east_used.get(grid.idx(from)).copied().unwrap_or(0),
        Dir::West => {
            if from.gx == 0 {
                0
            } else {
                let west = GCellId {
                    gx: from.gx - 1,
                    gy: from.gy,
                };
                capacity.east_used.get(grid.idx(west)).copied().unwrap_or(0)
            }
        }
        Dir::North => capacity.north_used.get(grid.idx(from)).copied().unwrap_or(0),
        Dir::South => {
            if from.gy == 0 {
                0
            } else {
                let south = GCellId {
                    gx: from.gx,
                    gy: from.gy - 1,
                };
                capacity.north_used.get(grid.idx(south)).copied().unwrap_or(0)
            }
        }
    }
}

fn apply_usage(grid: &GCellGrid, capacity: &mut CapacityGrid, path: &[GCellId]) {
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        if b.gx > a.gx {
            capacity.east_used[grid.idx(a)] += 1;
        } else if b.gx < a.gx {
            capacity.east_used[grid.idx(b)] += 1;
        } else if b.gy > a.gy {
            capacity.north_used[grid.idx(a)] += 1;
        } else if b.gy < a.gy {
            capacity.north_used[grid.idx(b)] += 1;
        }
    }
}

fn reconstruct(
    came_from: &HashMap<GCellId, GCellId>,
    dst: GCellId,
    net_name: &SmolStr,
    crosses_sat: &HashMap<GCellId, bool>,
) -> GlobalRoute {
    let mut path = vec![dst];
    let mut cur = dst;
    while let Some(&prev) = came_from.get(&cur) {
        path.push(prev);
        cur = prev;
    }
    path.reverse();
    GlobalRoute {
        net_name: net_name.clone(),
        gcells: path,
        crosses_saturated_edge: *crosses_sat.get(&dst).unwrap_or(&false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::{Bbox, Point};

    fn small_grid() -> GCellGrid {
        GCellGrid::new(
            Bbox::new(Point::new(0, 0), Point::new(100, 100)),
            10,
            10,
        )
    }

    #[test]
    fn snap_to_correct_cell() {
        let g = small_grid();
        let c = g.snap(Point::new(25, 35));
        assert_eq!(c, GCellId { gx: 2, gy: 3 });
    }

    #[test]
    fn dijkstra_finds_shortest_uncongested_path() {
        let grid = small_grid();
        let mut cap = CapacityGrid::uniform(&grid, 10);
        let cfg = GlobalRouterConfig::default();
        let routes = route_global(
            &grid,
            &mut cap,
            &[GlobalRouteRequest {
                net_name: "n0".into(),
                src: Point::new(5, 5),
                dst: Point::new(95, 95),
            }],
            &cfg,
        );
        assert_eq!(routes.len(), 1);
        assert!(!routes[0].gcells.is_empty());
        assert!(!routes[0].crosses_saturated_edge);
    }

    #[test]
    fn congestion_pushes_later_nets_to_alternate_paths() {
        let grid = small_grid();
        let mut cap = CapacityGrid::uniform(&grid, 1); // very tight
        let cfg = GlobalRouterConfig::default();
        let req = vec![
            GlobalRouteRequest {
                net_name: "n0".into(),
                src: Point::new(5, 5),
                dst: Point::new(95, 5),
            },
            GlobalRouteRequest {
                net_name: "n1".into(),
                src: Point::new(5, 5),
                dst: Point::new(95, 5),
            },
        ];
        let routes = route_global(&grid, &mut cap, &req, &cfg);
        // Both routed; second one is forced to detour or share saturated edges.
        assert_eq!(routes.len(), 2);
        // n0's path uses up edge capacity along y=0 row; n1 must
        // either detour or be flagged as crossing saturated edges.
        let n1 = &routes[1];
        let n1_y_excursion = n1.gcells.iter().any(|c| c.gy != 0);
        assert!(n1_y_excursion || n1.crosses_saturated_edge);
    }

    #[test]
    fn batch_preserves_order() {
        let grid = small_grid();
        let mut cap = CapacityGrid::uniform(&grid, 10);
        let cfg = GlobalRouterConfig::default();
        let names = ["a", "b", "c"];
        let reqs: Vec<GlobalRouteRequest> = names
            .iter()
            .map(|n| GlobalRouteRequest {
                net_name: SmolStr::from(*n),
                src: Point::new(5, 5),
                dst: Point::new(85, 85),
            })
            .collect();
        let routes = route_global(&grid, &mut cap, &reqs, &cfg);
        for (r, n) in routes.iter().zip(names.iter()) {
            assert_eq!(r.net_name.as_str(), *n);
        }
    }
}
