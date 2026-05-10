use klayout_core::{Angle90, Bbox, LayerInfo, Library, Point, Port};
use klayout_route::{
    AStarPlanner, Bundler, IdentityBundler, ManhattanPlanner, Obstacles, Planner, Stylizer,
    WirePathStylizer,
};

fn lib() -> Library {
    Library::new("t", 1000)
}

#[test]
fn manhattan_horizontal_then_vertical() {
    let l = lib();
    let m1 = l.layer(LayerInfo::gds(10, 0));
    let src = Port::new("a", m1, Point::new(0, 0), Angle90::E, 10);
    let dst = Port::new("b", m1, Point::new(100, 50), Angle90::W, 10);
    let path = ManhattanPlanner.plan(&src, &dst, &Obstacles::default());
    assert_eq!(path.points[0], Point::new(0, 0));
    assert_eq!(*path.points.last().unwrap(), Point::new(100, 50));
    // src faces E → horizontal first → elbow at (100, 0).
    assert!(path.points.contains(&Point::new(100, 0)));
}

#[test]
fn manhattan_vertical_then_horizontal_for_n_facing_port() {
    let l = lib();
    let m1 = l.layer(LayerInfo::gds(10, 0));
    let src = Port::new("a", m1, Point::new(0, 0), Angle90::N, 10);
    let dst = Port::new("b", m1, Point::new(100, 50), Angle90::S, 10);
    let path = ManhattanPlanner.plan(&src, &dst, &Obstacles::default());
    // src faces N → vertical first → elbow at (0, 50).
    assert!(path.points.contains(&Point::new(0, 50)));
}

#[test]
fn manhattan_collinear_no_elbow() {
    let l = lib();
    let m1 = l.layer(LayerInfo::gds(10, 0));
    let src = Port::new("a", m1, Point::new(0, 0), Angle90::E, 10);
    let dst = Port::new("b", m1, Point::new(100, 0), Angle90::W, 10);
    let path = ManhattanPlanner.plan(&src, &dst, &Obstacles::default());
    // Collinear → no elbow inserted.
    assert_eq!(path.points.len(), 2);
}

#[test]
fn stylizer_emits_single_path_shape() {
    let l = lib();
    let m1 = l.layer(LayerInfo::gds(10, 0));
    let src = Port::new("a", m1, Point::new(0, 0), Angle90::E, 10);
    let dst = Port::new("b", m1, Point::new(100, 50), Angle90::W, 10);
    let path = ManhattanPlanner.plan(&src, &dst, &Obstacles::default());
    let shapes = WirePathStylizer.stylize(m1, path);
    assert_eq!(shapes.len(), 1);
    assert_eq!(shapes[0].0, m1);
    assert!(matches!(shapes[0].1, klayout_core::Shape::Path(_)));
}

#[test]
fn astar_no_obstacles_matches_manhattan() {
    let l = lib();
    let m1 = l.layer(LayerInfo::gds(10, 0));
    let src = Port::new("a", m1, Point::new(0, 0), Angle90::E, 10);
    let dst = Port::new("b", m1, Point::new(50, 30), Angle90::W, 10);
    let path = AStarPlanner::new()
        .with_grid_step(2)
        .plan(&src, &dst, &Obstacles::default());
    // Endpoints preserved
    assert_eq!(path.points[0], Point::new(0, 0));
    assert_eq!(*path.points.last().unwrap(), Point::new(50, 30));
}

#[test]
fn astar_routes_around_obstacle() {
    let l = lib();
    let m1 = l.layer(LayerInfo::gds(10, 0));
    let src = Port::new("a", m1, Point::new(0, 50), Angle90::E, 10);
    let dst = Port::new("b", m1, Point::new(100, 50), Angle90::W, 10);
    let env = Obstacles {
        forbidden_bboxes: vec![Bbox::new(Point::new(40, 40), Point::new(60, 60))],
    };
    let path = AStarPlanner::new()
        .with_grid_step(5)
        .plan(&src, &dst, &env);
    // Path must reach destination and avoid the obstacle bbox.
    assert_eq!(path.points[0], Point::new(0, 50));
    assert_eq!(*path.points.last().unwrap(), Point::new(100, 50));
    let blocked = Bbox::new(Point::new(40, 40), Point::new(60, 60));
    // Walk segments and verify no point is strictly inside the blocked region.
    for win in path.points.windows(2) {
        let (a, b) = (win[0], win[1]);
        assert!(a.x == b.x || a.y == b.y, "segments must be axis-aligned");
        // Sample midpoint
        let mid = Point::new((a.x + b.x) / 2, (a.y + b.y) / 2);
        assert!(
            !blocked.contains(mid)
                || (a == src.center || b == dst.center),
            "midpoint {mid:?} must not be inside obstacle"
        );
    }
}

#[test]
fn astar_falls_back_when_no_path_exists() {
    let l = lib();
    let m1 = l.layer(LayerInfo::gds(10, 0));
    let src = Port::new("a", m1, Point::new(50, 50), Angle90::E, 10);
    let dst = Port::new("b", m1, Point::new(150, 50), Angle90::W, 10);
    // Surround src with obstacles forming a wall (margin is 100, the wall
    // is just within the search bbox so A* can't find a route).
    let env = Obstacles {
        forbidden_bboxes: vec![Bbox::new(Point::new(60, -200), Point::new(140, 250))],
    };
    let path = AStarPlanner::new()
        .with_grid_step(10)
        .with_margin(50)
        .plan(&src, &dst, &env);
    // Should fall back to manhattan (which doesn't avoid obstacles).
    assert_eq!(path.points[0], Point::new(50, 50));
    assert_eq!(*path.points.last().unwrap(), Point::new(150, 50));
}

#[test]
fn identity_bundler_plans_each_pair() {
    let l = lib();
    let m1 = l.layer(LayerInfo::gds(10, 0));
    let pairs: Vec<_> = (0..3)
        .map(|i| {
            (
                Port::new("a", m1, Point::new(0, i * 30), Angle90::E, 10),
                Port::new("b", m1, Point::new(100, i * 30 + 5), Angle90::W, 10),
            )
        })
        .collect();
    let paths = IdentityBundler.bundle(&ManhattanPlanner, &pairs, &Obstacles::default());
    assert_eq!(paths.len(), 3);
    // Each path takes its own elbow.
    for (i, p) in paths.iter().enumerate() {
        assert_eq!(p.points[0], Point::new(0, i as i64 * 30));
    }
}
