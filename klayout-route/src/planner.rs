//! Path planning: given two ports + obstacles, return a centerline `Path`.

use klayout_core::{Angle90, Path, Point, Port};
use smallvec::SmallVec;

/// Obstacle environment — used by smarter planners. The v1 manhattan
/// planner ignores obstacles, so this is currently informational.
#[derive(Default, Clone)]
pub struct Obstacles {
    pub forbidden_bboxes: Vec<klayout_core::Bbox>,
}

pub trait Planner {
    fn plan(&self, src: &Port, dst: &Port, env: &Obstacles) -> Path;
}

/// 90°-manhattan planner: drops one bend at the elbow point.
///
/// Bend point is chosen so each leg leaves its port along the port's
/// outgoing direction. Width = max of the two port widths. Obstacles
/// are ignored in v1; a future obstacle-avoiding A* planner can replace
/// this without touching the rest of the routing pipeline.
pub struct ManhattanPlanner;

impl Planner for ManhattanPlanner {
    fn plan(&self, src: &Port, dst: &Port, _env: &Obstacles) -> Path {
        let mut pts: SmallVec<[Point; 4]> = SmallVec::new();
        pts.push(src.center);
        // Pick elbow based on port directions. If src faces E/W, go
        // horizontal first; if N/S, vertical first.
        let elbow = match src.angle {
            Angle90::E | Angle90::W => Point::new(dst.center.x, src.center.y),
            Angle90::N | Angle90::S => Point::new(src.center.x, dst.center.y),
        };
        if elbow != src.center && elbow != dst.center {
            pts.push(elbow);
        }
        pts.push(dst.center);
        let width = src.width.max(dst.width);
        Path {
            points: pts,
            width,
            begin_ext: 0,
            end_ext: 0,
            cap: klayout_core::PathCap::Flat,
        }
    }
}
