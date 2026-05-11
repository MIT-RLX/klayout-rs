//! Grid A* obstacle-avoiding manhattan planner.
//!
//! Discretizes the bounding box of (src, dst) plus a margin onto a regular
//! grid (default 1 DBU spacing — coarser grids can be requested via
//! `AStarPlanner::with_grid_step`). Each grid cell either belongs to a
//! forbidden bbox or doesn't; A* over the remaining cells with manhattan
//! distance heuristic finds the shortest connecting path. Bends are
//! penalized lightly to prefer paths with fewer corners.
//!
//! Returns a centerline `Path` matching the [`Planner`] trait. Falls back
//! to one-bend manhattan if A* finds no route.
//!
//! For obstacle-critical callers (PNR), use [`AStarPlanner::plan_or_none`] /
//! [`AStarPlanner::plan_congested_or_none`] so routing fails closed instead of
//! silently using obstacle-ignorant Manhattan.

use crate::congestion::FractionalCongestionGrid;
use crate::planner::{ManhattanPlanner, Obstacles, Planner};
use klayout_core::{Bbox, Path as CorePath, PathCap, Point, Port};
use smallvec::SmallVec;
use std::collections::BinaryHeap;

pub struct AStarPlanner {
    pub grid_step: i64,
    pub margin: i64,
    pub bend_penalty: i64,
}

impl Default for AStarPlanner {
    fn default() -> Self {
        Self {
            grid_step: 1,
            margin: 100,
            bend_penalty: 1,
        }
    }
}

impl AStarPlanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_grid_step(mut self, step: i64) -> Self {
        self.grid_step = step.max(1);
        self
    }

    pub fn with_margin(mut self, margin: i64) -> Self {
        self.margin = margin.max(0);
        self
    }

    pub fn with_bend_penalty(mut self, penalty: i64) -> Self {
        self.bend_penalty = penalty.max(0);
        self
    }

    /// Obstacle-aware planning without the obstacle-ignorant [`Planner`]
    /// fallback. Use from routers that must not short through blockages when
    /// the grid search fails.
    pub fn plan_or_none(&self, src: &Port, dst: &Port, env: &Obstacles) -> Option<CorePath> {
        let pts = astar(src.center, dst.center, env, self, None, 0)?;
        let width = src.width.max(dst.width);
        Some(CorePath {
            points: compress_polyline(&pts),
            width,
            begin_ext: 0,
            end_ext: 0,
            cap: PathCap::Flat,
        })
    }

    /// Congestion-weighted variant of [`Self::plan_or_none`].
    pub fn plan_congested_or_none(
        &self,
        src: &Port,
        dst: &Port,
        env: &Obstacles,
        congestion: Option<&FractionalCongestionGrid>,
        congestion_weight: i64,
    ) -> Option<CorePath> {
        let pts = astar(
            src.center,
            dst.center,
            env,
            self,
            congestion,
            congestion_weight,
        )?;
        let width = src.width.max(dst.width);
        Some(CorePath {
            points: compress_polyline(&pts),
            width,
            begin_ext: 0,
            end_ext: 0,
            cap: PathCap::Flat,
        })
    }

    /// Same as [`Planner::plan`] but adds `congestion_weight ×
    /// grid.lookup_world(unsnap(neighbor))` to each grid move (integer-rounded).
    pub fn plan_congested(
        &self,
        src: &Port,
        dst: &Port,
        env: &Obstacles,
        congestion: Option<&FractionalCongestionGrid>,
        congestion_weight: i64,
    ) -> CorePath {
        self.plan_congested_or_none(src, dst, env, congestion, congestion_weight)
            .unwrap_or_else(|| ManhattanPlanner.plan(src, dst, env))
    }
}

impl Planner for AStarPlanner {
    fn plan(&self, src: &Port, dst: &Port, env: &Obstacles) -> CorePath {
        self.plan_or_none(src, dst, env)
            .unwrap_or_else(|| ManhattanPlanner.plan(src, dst, env))
    }
}

fn compress_polyline(pts: &[Point]) -> SmallVec<[Point; 4]> {
    let mut compressed: SmallVec<[Point; 4]> = SmallVec::new();
    for p in pts {
        if compressed.len() >= 2 {
            let a = compressed[compressed.len() - 2];
            let b = compressed[compressed.len() - 1];
            if collinear(a, b, *p) {
                let last_idx = compressed.len() - 1;
                compressed[last_idx] = *p;
                continue;
            }
        }
        compressed.push(*p);
    }
    compressed
}

fn collinear(a: Point, b: Point, c: Point) -> bool {
    (a.x == b.x && b.x == c.x) || (a.y == b.y && b.y == c.y)
}

#[derive(Copy, Clone, Eq, PartialEq)]
struct Node {
    f: i64,
    g: i64,
    pos: (i64, i64),
    came_dir: u8, // 0=none, 1=+x, 2=-x, 3=+y, 4=-y
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Min-heap on f.
        other.f.cmp(&self.f).then_with(|| other.g.cmp(&self.g))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

fn astar(
    src: Point,
    dst: Point,
    env: &Obstacles,
    cfg: &AStarPlanner,
    congestion: Option<&FractionalCongestionGrid>,
    congestion_weight: i64,
) -> Option<Vec<Point>> {
    let bbox = Bbox::new(src, dst);
    let bbox_norm = Bbox::new(
        Point::new(
            bbox.min.x.min(bbox.max.x) - cfg.margin,
            bbox.min.y.min(bbox.max.y) - cfg.margin,
        ),
        Point::new(
            bbox.min.x.max(bbox.max.x) + cfg.margin,
            bbox.min.y.max(bbox.max.y) + cfg.margin,
        ),
    );
    let step = cfg.grid_step;

    // Grid coordinates: snap src and dst to the grid.
    let snap = |p: Point| -> (i64, i64) {
        ((p.x - bbox_norm.min.x) / step, (p.y - bbox_norm.min.y) / step)
    };
    let unsnap = |g: (i64, i64)| -> Point {
        Point::new(bbox_norm.min.x + g.0 * step, bbox_norm.min.y + g.1 * step)
    };

    let start = snap(src);
    let goal = snap(dst);
    let max_x = (bbox_norm.max.x - bbox_norm.min.x) / step;
    let max_y = (bbox_norm.max.y - bbox_norm.min.y) / step;

    let blocked = |gx: i64, gy: i64| -> bool {
        if gx < 0 || gy < 0 || gx > max_x || gy > max_y {
            return true;
        }
        let p = unsnap((gx, gy));
        for fb in &env.forbidden_bboxes {
            if fb.contains(p) {
                return true;
            }
        }
        false
    };

    let h = |g: (i64, i64)| -> i64 { (g.0 - goal.0).abs() + (g.1 - goal.1).abs() };

    let mut open: BinaryHeap<Node> = BinaryHeap::new();
    open.push(Node {
        f: h(start),
        g: 0,
        pos: start,
        came_dir: 0,
    });

    use std::collections::HashMap;
    let mut came_from: HashMap<(i64, i64), ((i64, i64), u8)> = HashMap::new();
    let mut g_score: HashMap<(i64, i64), i64> = HashMap::new();
    g_score.insert(start, 0);

    while let Some(cur) = open.pop() {
        if cur.pos == goal {
            // Reconstruct path
            let mut path = vec![dst];
            let mut node = cur.pos;
            while let Some((prev, _)) = came_from.get(&node).copied() {
                path.push(unsnap(prev));
                node = prev;
            }
            path.reverse();
            // Replace endpoints with exact src/dst.
            if path.first() != Some(&src) {
                path.insert(0, src);
            }
            if path.last() != Some(&dst) {
                path.push(dst);
            }
            return Some(path);
        }

        let dirs: [(i64, i64, u8); 4] = [(1, 0, 1), (-1, 0, 2), (0, 1, 3), (0, -1, 4)];
        for (dx, dy, dir) in dirs {
            let n_pos = (cur.pos.0 + dx, cur.pos.1 + dy);
            if blocked(n_pos.0, n_pos.1) {
                continue;
            }
            let bend = if cur.came_dir != 0 && cur.came_dir != dir {
                cfg.bend_penalty
            } else {
                0
            };
            let soft = congestion
                .map(|cg| {
                    let p = unsnap(n_pos);
                    let d = cg.lookup_world(p);
                    (congestion_weight as f64 * d as f64).round() as i64
                })
                .unwrap_or(0);
            let tentative_g = cur.g + 1 + bend + soft;
            let prev = g_score.get(&n_pos).copied().unwrap_or(i64::MAX);
            if tentative_g < prev {
                g_score.insert(n_pos, tentative_g);
                came_from.insert(n_pos, (cur.pos, dir));
                open.push(Node {
                    f: tentative_g + h(n_pos),
                    g: tentative_g,
                    pos: n_pos,
                    came_dir: dir,
                });
            }
        }

        // Bail-out: very large search space. For obstacle-free cases the
        // expansion is bounded; pathological obstacles can blow this up.
        if g_score.len() > 12_000_000 {
            return None;
        }
    }

    None
}
