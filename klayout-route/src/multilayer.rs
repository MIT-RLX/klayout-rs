//! Multi-layer routing with vias.
//!
//! Single-layer A* (in `astar.rs`) finds an obstacle-avoiding manhattan
//! path on one metal. Real designs route across stacks: M1 horizontal,
//! M2 vertical, M3 horizontal, with vias between adjacent layers. This
//! planner generalizes the A* state space from `(x, y)` to
//! `(x, y, layer)`, with via transitions between layers.
//!
//! A move costs:
//! * One cardinal step along a layer's preferred direction → grid_step.
//! * One step against the layer's preferred direction → grid_step ×
//!   off_axis_penalty (default 4).
//! * One via UP or DOWN → via_cost (per stack entry).
//!
//! Each layer has its own obstacle set. A grid cell is forbidden on
//! layer `i` iff any obstacle bbox on that layer covers the cell.
//!
//! Output is a list of [`RouteSegment`]s — `Wire { layer, points }` for
//! straight-line legs and `Via { at, from_layer, to_layer }` markers
//! at every layer change.

use crate::planner::Obstacles;
use klayout_core::Point;
use smol_str::SmolStr;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PreferredDirection {
    Horizontal,
    Vertical,
    Both,
}

#[derive(Clone, Debug)]
pub struct RoutingLayer {
    pub name: SmolStr,
    pub direction: PreferredDirection,
}

#[derive(Default, Clone, Debug)]
pub struct LayerStack {
    pub layers: Vec<RoutingLayer>,
}

#[derive(Clone, Debug)]
pub struct MultiAStarConfig {
    pub grid_step: i64,
    pub margin: i64,
    /// Cost multiplier for moves against the layer's preferred direction.
    pub off_axis_penalty: i64,
    /// Cost of one via (one layer transition).
    pub via_cost: i64,
}

impl Default for MultiAStarConfig {
    fn default() -> Self {
        Self {
            grid_step: 1,
            margin: 100,
            off_axis_penalty: 4,
            via_cost: 5,
        }
    }
}

#[derive(Clone, Debug)]
pub enum RouteSegment {
    Wire {
        layer_idx: usize,
        points: Vec<Point>,
    },
    Via {
        at: Point,
        from_layer: usize,
        to_layer: usize,
        /// Cut count for via-array generation. Default `1`; the
        /// detailed router upgrades this based on adjacent wire width
        /// to ensure adequate current-handling. v1 multilayer A*
        /// emits `1` (single-cut via).
        cut_count: u32,
    },
}

/// Plan a multi-layer route from `(src_pt, src_layer)` to
/// `(dst_pt, dst_layer)`. Returns `None` if no path exists.
pub fn multilayer_route(
    src: (Point, usize),
    dst: (Point, usize),
    obstacles: &[Obstacles],
    stack: &LayerStack,
    cfg: &MultiAStarConfig,
) -> Option<Vec<RouteSegment>> {
    if stack.layers.is_empty() {
        return None;
    }
    if src.1 >= stack.layers.len() || dst.1 >= stack.layers.len() {
        return None;
    }
    let bbox = bounding_grid(src.0, dst.0, cfg.margin, cfg.grid_step);
    let g = Grid::new(bbox, cfg.grid_step, stack.layers.len());

    let src_cell = g.snap(src.0, src.1);
    let dst_cell = g.snap(dst.0, dst.1);
    if !g.in_bounds(src_cell) || !g.in_bounds(dst_cell) {
        return None;
    }

    let path_cells = astar_3d(&g, src_cell, dst_cell, obstacles, stack, cfg)?;
    Some(decompose_segments(&g, &path_cells))
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
struct Cell {
    x: i32,
    y: i32,
    layer: usize,
}

struct Grid {
    origin: Point,
    width: i32,
    height: i32,
    step: i64,
    layers: usize,
}

impl Grid {
    fn new(bbox: (Point, Point), step: i64, layers: usize) -> Self {
        let width = (((bbox.1.x - bbox.0.x) / step) as i32) + 1;
        let height = (((bbox.1.y - bbox.0.y) / step) as i32) + 1;
        Grid {
            origin: bbox.0,
            width,
            height,
            step,
            layers,
        }
    }
    fn snap(&self, p: Point, layer: usize) -> Cell {
        let x = ((p.x - self.origin.x) / self.step) as i32;
        let y = ((p.y - self.origin.y) / self.step) as i32;
        Cell { x, y, layer }
    }
    fn in_bounds(&self, c: Cell) -> bool {
        c.x >= 0 && c.x < self.width && c.y >= 0 && c.y < self.height && c.layer < self.layers
    }
    fn point(&self, c: Cell) -> Point {
        Point::new(
            self.origin.x + c.x as i64 * self.step,
            self.origin.y + c.y as i64 * self.step,
        )
    }
}

fn bounding_grid(a: Point, b: Point, margin: i64, step: i64) -> (Point, Point) {
    let lo_x = a.x.min(b.x) - margin;
    let lo_y = a.y.min(b.y) - margin;
    let hi_x = a.x.max(b.x) + margin;
    let hi_y = a.y.max(b.y) + margin;
    // Snap to step grid.
    let snap = |v: i64| (v / step) * step;
    (
        Point::new(snap(lo_x), snap(lo_y)),
        Point::new(snap(hi_x), snap(hi_y)),
    )
}

#[derive(Copy, Clone, PartialEq, Eq)]
struct HeapNode {
    f: i64,
    g: i64,
    cell: Cell,
}

impl Ord for HeapNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap on f.
        other.f.cmp(&self.f)
    }
}
impl PartialOrd for HeapNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn astar_3d(
    grid: &Grid,
    src: Cell,
    dst: Cell,
    obstacles: &[Obstacles],
    stack: &LayerStack,
    cfg: &MultiAStarConfig,
) -> Option<Vec<Cell>> {
    let mut g_score: HashMap<Cell, i64> = HashMap::new();
    let mut came_from: HashMap<Cell, Cell> = HashMap::new();
    let mut heap: BinaryHeap<HeapNode> = BinaryHeap::new();
    g_score.insert(src, 0);
    heap.push(HeapNode {
        f: heuristic(src, dst, cfg),
        g: 0,
        cell: src,
    });

    while let Some(cur) = heap.pop() {
        if cur.cell == dst {
            return Some(reconstruct(&came_from, dst));
        }
        if cur.g > *g_score.get(&cur.cell).unwrap_or(&i64::MAX) {
            continue;
        }

        // 4 cardinal neighbors on the same layer.
        let dirs: [(i32, i32, bool); 4] = [
            (1, 0, true),  // east
            (-1, 0, true), // west
            (0, 1, false), // north
            (0, -1, false),// south
        ];
        let layer = &stack.layers[cur.cell.layer];
        for (dx, dy, horiz) in dirs {
            let nb = Cell {
                x: cur.cell.x + dx,
                y: cur.cell.y + dy,
                layer: cur.cell.layer,
            };
            if !grid.in_bounds(nb) || cell_blocked(grid, nb, obstacles) {
                continue;
            }
            let preferred = match layer.direction {
                PreferredDirection::Horizontal => horiz,
                PreferredDirection::Vertical => !horiz,
                PreferredDirection::Both => true,
            };
            let step_cost = grid.step
                + if preferred {
                    0
                } else {
                    grid.step * (cfg.off_axis_penalty - 1).max(0)
                };
            let tentative = cur.g + step_cost;
            if tentative < *g_score.get(&nb).unwrap_or(&i64::MAX) {
                g_score.insert(nb, tentative);
                came_from.insert(nb, cur.cell);
                heap.push(HeapNode {
                    f: tentative + heuristic(nb, dst, cfg),
                    g: tentative,
                    cell: nb,
                });
            }
        }

        // Via UP / DOWN.
        for d in [-1i32, 1] {
            let l = cur.cell.layer as i32 + d;
            if l < 0 || l as usize >= stack.layers.len() {
                continue;
            }
            let nb = Cell {
                x: cur.cell.x,
                y: cur.cell.y,
                layer: l as usize,
            };
            if !grid.in_bounds(nb) || cell_blocked(grid, nb, obstacles) {
                continue;
            }
            let tentative = cur.g + cfg.via_cost;
            if tentative < *g_score.get(&nb).unwrap_or(&i64::MAX) {
                g_score.insert(nb, tentative);
                came_from.insert(nb, cur.cell);
                heap.push(HeapNode {
                    f: tentative + heuristic(nb, dst, cfg),
                    g: tentative,
                    cell: nb,
                });
            }
        }
    }
    None
}

fn cell_blocked(grid: &Grid, c: Cell, obstacles: &[Obstacles]) -> bool {
    if c.layer >= obstacles.len() {
        return false;
    }
    let p = grid.point(c);
    obstacles[c.layer]
        .forbidden_bboxes
        .iter()
        .any(|b| b.contains(p))
}

fn heuristic(a: Cell, b: Cell, cfg: &MultiAStarConfig) -> i64 {
    let dx = (a.x - b.x).abs() as i64;
    let dy = (a.y - b.y).abs() as i64;
    let dl = (a.layer as i64 - b.layer as i64).abs();
    (dx + dy) * cfg.grid_step + dl * cfg.via_cost
}

fn reconstruct(came_from: &HashMap<Cell, Cell>, dst: Cell) -> Vec<Cell> {
    let mut path = vec![dst];
    let mut cur = dst;
    while let Some(&prev) = came_from.get(&cur) {
        path.push(prev);
        cur = prev;
    }
    path.reverse();
    path
}

fn decompose_segments(grid: &Grid, cells: &[Cell]) -> Vec<RouteSegment> {
    let mut out = Vec::new();
    if cells.is_empty() {
        return out;
    }
    let mut current_layer = cells[0].layer;
    let mut current_pts: Vec<Point> = vec![grid.point(cells[0])];
    for w in cells.windows(2) {
        let prev = w[0];
        let cur = w[1];
        if cur.layer != prev.layer {
            // Emit current wire (compressed), then a Via, then start a new wire.
            if !current_pts.is_empty() {
                let pt = grid.point(prev);
                if Some(&pt) != current_pts.last() {
                    current_pts.push(pt);
                }
                out.push(RouteSegment::Wire {
                    layer_idx: current_layer,
                    points: compress_collinear(current_pts.clone()),
                });
            }
            out.push(RouteSegment::Via {
                at: grid.point(prev),
                from_layer: prev.layer,
                to_layer: cur.layer,
                cut_count: 1,
            });
            current_layer = cur.layer;
            current_pts = vec![grid.point(cur)];
        } else {
            current_pts.push(grid.point(cur));
        }
    }
    if !current_pts.is_empty() {
        out.push(RouteSegment::Wire {
            layer_idx: current_layer,
            points: compress_collinear(current_pts),
        });
    }
    out
}

fn compress_collinear(pts: Vec<Point>) -> Vec<Point> {
    if pts.len() <= 2 {
        return pts;
    }
    // pts.len() ≥ 3 here, so first/last index by .len() - 1 is in bounds.
    let last = pts[pts.len() - 1];
    let mut out = vec![pts[0]];
    for w in pts.windows(3) {
        let (a, b, c) = (w[0], w[1], w[2]);
        let collinear = (b.x - a.x) * (c.y - b.y) == (b.y - a.y) * (c.x - b.x);
        if !collinear {
            out.push(b);
        }
    }
    out.push(last);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::{Bbox, Point};

    fn stack_two() -> LayerStack {
        LayerStack {
            layers: vec![
                RoutingLayer {
                    name: "M1".into(),
                    direction: PreferredDirection::Horizontal,
                },
                RoutingLayer {
                    name: "M2".into(),
                    direction: PreferredDirection::Vertical,
                },
            ],
        }
    }

    #[test]
    fn same_layer_route_emits_one_wire() {
        let stack = stack_two();
        let cfg = MultiAStarConfig::default();
        let obs = vec![Obstacles::default(), Obstacles::default()];
        let result = multilayer_route(
            (Point::new(0, 0), 0),
            (Point::new(50, 0), 0),
            &obs,
            &stack,
            &cfg,
        )
        .unwrap();
        // Single wire on layer 0.
        assert_eq!(result.len(), 1);
        match &result[0] {
            RouteSegment::Wire { layer_idx, .. } => assert_eq!(*layer_idx, 0),
            _ => panic!(),
        }
    }

    #[test]
    fn cross_layer_route_emits_via() {
        let stack = stack_two();
        let cfg = MultiAStarConfig::default();
        let obs = vec![Obstacles::default(), Obstacles::default()];
        let result = multilayer_route(
            (Point::new(0, 0), 0),
            (Point::new(0, 50), 1),
            &obs,
            &stack,
            &cfg,
        )
        .unwrap();
        // Should contain at least one Via.
        let has_via = result.iter().any(|s| matches!(s, RouteSegment::Via { .. }));
        assert!(has_via, "cross-layer route should produce a via");
    }

    #[test]
    fn obstacle_forces_detour() {
        let stack = stack_two();
        let cfg = MultiAStarConfig {
            grid_step: 1,
            margin: 50,
            off_axis_penalty: 4,
            via_cost: 5,
        };
        let mut obs0 = Obstacles::default();
        // Blockage straight in the path.
        obs0.forbidden_bboxes.push(Bbox::new(
            Point::new(20, -5),
            Point::new(30, 5),
        ));
        let obs = vec![obs0, Obstacles::default()];
        let result = multilayer_route(
            (Point::new(0, 0), 0),
            (Point::new(50, 0), 0),
            &obs,
            &stack,
            &cfg,
        )
        .expect("should route around");
        // Route should include a layer hop or a y-detour.
        let has_via = result.iter().any(|s| matches!(s, RouteSegment::Via { .. }));
        let has_y_excursion = result.iter().any(|s| {
            if let RouteSegment::Wire { points, .. } = s {
                points.iter().any(|p| p.y != 0)
            } else {
                false
            }
        });
        assert!(has_via || has_y_excursion);
    }

    #[test]
    fn unrouted_returns_none() {
        let stack = stack_two();
        let cfg = MultiAStarConfig {
            grid_step: 1,
            margin: 5,
            off_axis_penalty: 4,
            via_cost: 5,
        };
        let mut obs0 = Obstacles::default();
        // Wall the destination off completely.
        obs0.forbidden_bboxes
            .push(Bbox::new(Point::new(45, -50), Point::new(55, 50)));
        let mut obs1 = Obstacles::default();
        obs1.forbidden_bboxes
            .push(Bbox::new(Point::new(45, -50), Point::new(55, 50)));
        let obs = vec![obs0, obs1];
        let result = multilayer_route(
            (Point::new(0, 0), 0),
            (Point::new(60, 0), 0),
            &obs,
            &stack,
            &cfg,
        );
        assert!(result.is_none());
    }
}
