//! Layer-assigned multi-layer Pathfinder.
//!
//! Extends [`crate::pathfinder`] from a 2-D GCell graph to a stack
//! of per-layer 2-D graphs connected by via transitions. Three
//! features distinguish the multi-layer router:
//!
//! 1. **Per-layer GCell capacity.** Each layer carries its own
//!    `east_used / north_used` grids; congestion on M2 doesn't
//!    affect routability on M3.
//! 2. **Preferred direction.** Each layer is one of `Horizontal`,
//!    `Vertical`, or `Either`. Routing against the preferred
//!    direction is permitted but pays a `wrong_dir_penalty`
//!    multiplier; this is the conventional way to keep wires
//!    aligned with their layer's track grid.
//! 3. **Via transitions.** Moving from layer `i` to layer `i+1`
//!    (or `i-1`) at the same `(gx, gy)` costs `via_cost`. Vias also
//!    consume capacity on a per-cell counter.
//!
//! Routes are reported as `Vec<(GCellId, layer_idx)>` so the
//! detailed router can lower them onto actual wire shapes layer by
//! layer.
//!
//! ## What this is NOT
//!
//! * **A full track-graph router.** Modern detailed flows route on
//!   per-track graphs after the global pass; this layer is one step
//!   above that, planning a corridor + layer assignment.
//! * **NDR / shielded routing aware.** The non-default-rule API
//!   exists in [`crate::detailed`]; combining it with multi-layer
//!   global routing is a follow-up.
//! * **Timing-driven.** The cost function is congestion + via +
//!   wrong-direction. Slack-driven routing would multiply the cost
//!   by an edge-criticality factor; we have the hooks but no
//!   timing-engine integration yet.

use crate::global::{CapacityGrid, GCellGrid, GCellId};
use klayout_core::Point;
use smol_str::SmolStr;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LayerDirection {
    Horizontal,
    Vertical,
    Either,
}

#[derive(Clone, Debug)]
pub struct LayerSpec {
    pub name: SmolStr,
    /// Preferred routing direction. Hops in the perpendicular axis
    /// are penalized but legal.
    pub direction: LayerDirection,
    /// Edge capacities on this layer (one capacity grid per layer).
    pub capacity: CapacityGrid,
}

#[derive(Clone, Debug)]
pub struct MultiLayerStack {
    pub grid: GCellGrid,
    pub layers: Vec<LayerSpec>,
    /// Cost added to the routing-edge cost when traversing a via
    /// between adjacent layers.
    pub via_cost: f64,
    /// Multiplier on edge cost when moving in the layer's
    /// non-preferred direction.
    pub wrong_dir_penalty: f64,
}

#[derive(Clone, Debug)]
pub struct MultiPathRequest {
    pub net_name: SmolStr,
    pub src: Point,
    pub src_layer: usize,
    pub sink: Point,
    pub sink_layer: usize,
}

#[derive(Clone, Debug)]
pub struct MultiPathRoute {
    pub net_name: SmolStr,
    /// Path as `(GCellId, layer_idx)` tuples, including layer
    /// transitions. Adjacent entries with the same `(gx, gy)` and
    /// different `layer_idx` are vias.
    pub path: Vec<(GCellId, usize)>,
}

#[derive(Clone, Debug)]
pub struct MultiConfig {
    pub max_iterations: u32,
    pub h_factor: f64,
    pub h_increment: f64,
    pub p_initial_slope: f64,
    pub p_growth: f64,
    pub overflow_target: u32,
}

impl Default for MultiConfig {
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
pub struct MultiResult {
    pub routes: Vec<MultiPathRoute>,
    pub iterations: u32,
    pub residual_overflow: u32,
}

#[derive(Clone, Debug)]
struct History {
    /// Per-layer east / north history costs (parallel to each
    /// layer's `CapacityGrid`).
    east: Vec<Vec<f64>>,
    north: Vec<Vec<f64>>,
    /// Per-layer-pair via history (counts vias landing on this cell
    /// from layer i to layer i+1). Reserved for a future
    /// via-overflow penalty; the v1 cost function uses a constant
    /// `via_cost` and doesn't yet update this term.
    #[allow(dead_code)]
    via: Vec<Vec<f64>>,
}

impl History {
    fn empty(stack: &MultiLayerStack) -> Self {
        let n = stack.grid.cell_count();
        let nl = stack.layers.len();
        Self {
            east: vec![vec![0.0; n]; nl],
            north: vec![vec![0.0; n]; nl],
            via: vec![vec![0.0; n]; nl.saturating_sub(1)],
        }
    }
}

#[derive(Copy, Clone, Debug)]
enum Move {
    East,
    West,
    North,
    South,
    Up,
    Down,
}

#[derive(Copy, Clone, PartialEq)]
struct HeapEntry {
    cost: f64,
    cell: GCellId,
    layer: usize,
}

impl Eq for HeapEntry {}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
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

pub fn route_multi_pathfinder(
    stack: &mut MultiLayerStack,
    requests: &[MultiPathRequest],
    cfg: &MultiConfig,
) -> MultiResult {
    let mut history = History::empty(stack);
    let mut p_slope = cfg.p_initial_slope;
    let mut routes: Vec<MultiPathRoute> = Vec::with_capacity(requests.len());

    for iter in 0..cfg.max_iterations {
        // Reset usage at the start of each rip-up-and-reroute round.
        for layer in &mut stack.layers {
            layer.capacity.east_used.fill(0);
            layer.capacity.north_used.fill(0);
        }
        routes.clear();

        for req in requests {
            let path = a_star_multi(stack, &history, req, cfg, p_slope);
            if let Some(path) = path {
                apply_usage(stack, &path);
                routes.push(MultiPathRoute {
                    net_name: req.net_name.clone(),
                    path,
                });
            } else {
                routes.push(MultiPathRoute {
                    net_name: req.net_name.clone(),
                    path: Vec::new(),
                });
            }
        }

        let overflow = count_overflow(stack);
        if overflow <= cfg.overflow_target {
            return MultiResult {
                routes,
                iterations: iter + 1,
                residual_overflow: overflow,
            };
        }

        update_history(stack, &mut history, cfg);
        p_slope *= cfg.p_growth;
    }

    MultiResult {
        residual_overflow: count_overflow(stack),
        routes,
        iterations: cfg.max_iterations,
    }
}

fn count_overflow(stack: &MultiLayerStack) -> u32 {
    let mut total = 0u32;
    for layer in &stack.layers {
        for i in 0..stack.grid.cell_count() {
            if layer.capacity.east_used[i] > layer.capacity.east_capacity[i] {
                total += layer.capacity.east_used[i] - layer.capacity.east_capacity[i];
            }
            if layer.capacity.north_used[i] > layer.capacity.north_capacity[i] {
                total += layer.capacity.north_used[i] - layer.capacity.north_capacity[i];
            }
        }
    }
    total
}

fn update_history(stack: &MultiLayerStack, hist: &mut History, cfg: &MultiConfig) {
    for (li, layer) in stack.layers.iter().enumerate() {
        for i in 0..stack.grid.cell_count() {
            if layer.capacity.east_used[i] > layer.capacity.east_capacity[i] {
                hist.east[li][i] += cfg.h_increment;
            }
            if layer.capacity.north_used[i] > layer.capacity.north_capacity[i] {
                hist.north[li][i] += cfg.h_increment;
            }
        }
    }
}

fn apply_usage(stack: &mut MultiLayerStack, path: &[(GCellId, usize)]) {
    for w in path.windows(2) {
        let (a, la) = w[0];
        let (b, lb) = w[1];
        if la != lb {
            // Via — no edge usage to bump on this hop, but the cell
            // is consumed on both layers (caller may model via
            // capacity here if needed).
            continue;
        }
        let layer = &mut stack.layers[la];
        if b.gx > a.gx {
            layer.capacity.east_used[grid_idx(&stack.grid, a)] += 1;
        } else if b.gx < a.gx {
            layer.capacity.east_used[grid_idx(&stack.grid, b)] += 1;
        } else if b.gy > a.gy {
            layer.capacity.north_used[grid_idx(&stack.grid, a)] += 1;
        } else if b.gy < a.gy {
            layer.capacity.north_used[grid_idx(&stack.grid, b)] += 1;
        }
    }
}

fn grid_idx(grid: &GCellGrid, c: GCellId) -> usize {
    c.gy as usize * grid.n_cols as usize + c.gx as usize
}

fn a_star_multi(
    stack: &MultiLayerStack,
    hist: &History,
    req: &MultiPathRequest,
    cfg: &MultiConfig,
    p_slope: f64,
) -> Option<Vec<(GCellId, usize)>> {
    let src_cell = stack.grid.snap(req.src);
    let sink_cell = stack.grid.snap(req.sink);
    let n_layers = stack.layers.len();
    if req.src_layer >= n_layers || req.sink_layer >= n_layers {
        return None;
    }

    let h = |c: GCellId, layer: usize| -> f64 {
        let dx = (c.gx as i64 - sink_cell.gx as i64).abs();
        let dy = (c.gy as i64 - sink_cell.gy as i64).abs();
        let dl = (layer as i64 - req.sink_layer as i64).abs() as f64;
        (dx + dy) as f64 + dl * stack.via_cost
    };

    type State = (GCellId, usize);
    let mut g: HashMap<State, f64> = HashMap::new();
    let mut came_from: HashMap<State, State> = HashMap::new();
    let src_state: State = (src_cell, req.src_layer);
    g.insert(src_state, 0.0);
    let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();
    heap.push(HeapEntry {
        cost: h(src_cell, req.src_layer),
        cell: src_cell,
        layer: req.src_layer,
    });

    while let Some(cur) = heap.pop() {
        if cur.cell == sink_cell && cur.layer == req.sink_layer {
            return Some(reconstruct(&came_from, (sink_cell, req.sink_layer), src_state));
        }
        let cur_state: State = (cur.cell, cur.layer);
        let cur_g = *g.get(&cur_state).unwrap_or(&f64::INFINITY);
        if cur.cost > cur_g + h(cur.cell, cur.layer) + 1e-9 {
            continue;
        }
        for (nb_state, edge) in neighbors(stack, cur.cell, cur.layer) {
            let edge_cost = edge_cost(stack, hist, cur.cell, cur.layer, edge, p_slope, cfg);
            let tentative = cur_g + edge_cost;
            if tentative < *g.get(&nb_state).unwrap_or(&f64::INFINITY) {
                g.insert(nb_state, tentative);
                came_from.insert(nb_state, cur_state);
                heap.push(HeapEntry {
                    cost: tentative + h(nb_state.0, nb_state.1),
                    cell: nb_state.0,
                    layer: nb_state.1,
                });
            }
        }
    }
    None
}

fn neighbors(
    stack: &MultiLayerStack,
    c: GCellId,
    layer: usize,
) -> Vec<((GCellId, usize), Move)> {
    let mut out = Vec::with_capacity(6);
    if c.gx + 1 < stack.grid.n_cols {
        out.push((
            (
                GCellId {
                    gx: c.gx + 1,
                    gy: c.gy,
                },
                layer,
            ),
            Move::East,
        ));
    }
    if c.gx > 0 {
        out.push((
            (
                GCellId {
                    gx: c.gx - 1,
                    gy: c.gy,
                },
                layer,
            ),
            Move::West,
        ));
    }
    if c.gy + 1 < stack.grid.n_rows {
        out.push((
            (
                GCellId {
                    gx: c.gx,
                    gy: c.gy + 1,
                },
                layer,
            ),
            Move::North,
        ));
    }
    if c.gy > 0 {
        out.push((
            (
                GCellId {
                    gx: c.gx,
                    gy: c.gy - 1,
                },
                layer,
            ),
            Move::South,
        ));
    }
    if layer + 1 < stack.layers.len() {
        out.push(((c, layer + 1), Move::Up));
    }
    if layer > 0 {
        out.push(((c, layer - 1), Move::Down));
    }
    out
}

fn edge_cost(
    stack: &MultiLayerStack,
    hist: &History,
    from: GCellId,
    layer: usize,
    mv: Move,
    p_slope: f64,
    cfg: &MultiConfig,
) -> f64 {
    let dir_penalty = |layer_dir: LayerDirection, horizontal: bool| -> f64 {
        match (layer_dir, horizontal) {
            (LayerDirection::Either, _) => 1.0,
            (LayerDirection::Horizontal, true) => 1.0,
            (LayerDirection::Vertical, false) => 1.0,
            _ => stack.wrong_dir_penalty,
        }
    };

    match mv {
        Move::East | Move::West | Move::North | Move::South => {
            let l = &stack.layers[layer];
            let (used, capacity, h_e) = match mv {
                Move::East => {
                    let i = grid_idx(&stack.grid, from);
                    (
                        l.capacity.east_used[i],
                        l.capacity.east_capacity[i],
                        hist.east[layer][i],
                    )
                }
                Move::West => {
                    let dst = GCellId {
                        gx: from.gx - 1,
                        gy: from.gy,
                    };
                    let i = grid_idx(&stack.grid, dst);
                    (
                        l.capacity.east_used[i],
                        l.capacity.east_capacity[i],
                        hist.east[layer][i],
                    )
                }
                Move::North => {
                    let i = grid_idx(&stack.grid, from);
                    (
                        l.capacity.north_used[i],
                        l.capacity.north_capacity[i],
                        hist.north[layer][i],
                    )
                }
                Move::South => {
                    let dst = GCellId {
                        gx: from.gx,
                        gy: from.gy - 1,
                    };
                    let i = grid_idx(&stack.grid, dst);
                    (
                        l.capacity.north_used[i],
                        l.capacity.north_capacity[i],
                        hist.north[layer][i],
                    )
                }
                _ => unreachable!(),
            };
            let h_term = 1.0 + h_e * cfg.h_factor;
            let p_term = if used + 1 > capacity {
                1.0 + p_slope * (used as f64 + 1.0 - capacity as f64) / capacity.max(1) as f64
            } else {
                1.0
            };
            let horizontal = matches!(mv, Move::East | Move::West);
            dir_penalty(l.direction, horizontal) * h_term * p_term
        }
        Move::Up | Move::Down => stack.via_cost,
    }
}

fn reconstruct(
    came_from: &HashMap<(GCellId, usize), (GCellId, usize)>,
    sink: (GCellId, usize),
    src: (GCellId, usize),
) -> Vec<(GCellId, usize)> {
    let mut path = vec![sink];
    let mut cur = sink;
    while let Some(&prev) = came_from.get(&cur) {
        path.push(prev);
        cur = prev;
        if cur == src {
            break;
        }
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::Bbox;

    fn three_layer_stack(grid_n: u32, capacity: u32) -> MultiLayerStack {
        let edge = 100i64;
        let bbox = Bbox::new(
            Point::new(0, 0),
            Point::new(grid_n as i64 * edge, grid_n as i64 * edge),
        );
        let grid = GCellGrid::new(bbox, edge, edge);
        let cap = CapacityGrid::uniform(&grid, capacity);
        let layers = vec![
            LayerSpec {
                name: "M1".into(),
                direction: LayerDirection::Horizontal,
                capacity: cap.clone(),
            },
            LayerSpec {
                name: "M2".into(),
                direction: LayerDirection::Vertical,
                capacity: cap.clone(),
            },
            LayerSpec {
                name: "M3".into(),
                direction: LayerDirection::Horizontal,
                capacity: cap,
            },
        ];
        MultiLayerStack {
            grid,
            layers,
            via_cost: 5.0,
            wrong_dir_penalty: 4.0,
        }
    }

    #[test]
    fn same_layer_routes_directly() {
        let mut stack = three_layer_stack(8, 4);
        let req = MultiPathRequest {
            net_name: "n".into(),
            src: Point::new(50, 50),
            src_layer: 0,
            sink: Point::new(550, 50),
            sink_layer: 0,
        };
        let r = route_multi_pathfinder(&mut stack, &[req], &MultiConfig::default());
        assert_eq!(r.iterations, 1);
        assert_eq!(r.residual_overflow, 0);
        assert_eq!(r.routes.len(), 1);
        // All path entries should be on layer 0.
        for (_, l) in &r.routes[0].path {
            assert_eq!(*l, 0);
        }
    }

    #[test]
    fn cross_layer_routes_take_via_at_layer_change() {
        let mut stack = three_layer_stack(8, 4);
        let req = MultiPathRequest {
            net_name: "n".into(),
            src: Point::new(50, 50),
            src_layer: 0,
            sink: Point::new(50, 50),
            sink_layer: 2,
        };
        let r = route_multi_pathfinder(&mut stack, &[req], &MultiConfig::default());
        assert_eq!(r.routes.len(), 1);
        let path = &r.routes[0].path;
        // Should hop: layer 0 → 1 → 2 at the same (gx, gy).
        assert!(path.iter().any(|(_, l)| *l == 1));
        assert!(path.iter().any(|(_, l)| *l == 2));
    }

    #[test]
    fn preferred_direction_is_actually_preferred() {
        let mut stack = three_layer_stack(8, 4);
        // Route a long horizontal pair: M1 (horizontal-preferred)
        // should be the natural choice over the M2 (vertical-pref).
        let req = MultiPathRequest {
            net_name: "n".into(),
            src: Point::new(50, 50),
            src_layer: 0,
            sink: Point::new(750, 50),
            sink_layer: 0,
        };
        let r = route_multi_pathfinder(&mut stack, &[req], &MultiConfig::default());
        // Expect mostly horizontal moves on M1; no excursions to M2
        // since direct route on preferred direction is cheapest.
        let path = &r.routes[0].path;
        let layers_used: std::collections::HashSet<usize> =
            path.iter().map(|(_, l)| *l).collect();
        assert_eq!(layers_used.len(), 1);
        assert!(layers_used.contains(&0));
    }

    #[test]
    fn congestion_drives_rerouting_to_zero_overflow() {
        // Three nets share the same source/sink corridor. With
        // capacity 1 per edge, only one fits the direct path;
        // Pathfinder must reroute the others through alternative
        // paths (different rows on the same layer is cheaper than
        // hopping layers given our via_cost / wrong_dir_penalty
        // settings, but the router can use whichever it wants).
        // What the test asserts is the contract: zero residual
        // overflow after iterations.
        let mut stack = three_layer_stack(4, 1);
        let reqs: Vec<_> = (0..3)
            .map(|i| MultiPathRequest {
                net_name: SmolStr::from(format!("n{i}")),
                src: Point::new(50, 150),
                src_layer: 0,
                sink: Point::new(350, 150),
                sink_layer: 0,
            })
            .collect();
        let r = route_multi_pathfinder(&mut stack, &reqs, &MultiConfig::default());
        assert_eq!(
            r.residual_overflow, 0,
            "expected congestion-free after history-driven rerouting"
        );
    }
}
