//! Axis-aligned edge-pair DRC kernel.
//!
//! For axis-aligned polygons (the bulk of every real layout), KLayout's
//! `width_check`/`space_check` semantics are precise edge-pair distance
//! checks. This module replicates that exactly.
//!
//! Convention recap: our `Polygon` hulls are CW (clockwise), so walking
//! edges in order, the polygon's interior is on the **right** of each
//! edge. From this we derive each edge's "exterior side":
//!
//! * Vertical edge going up (`a.y < b.y`): interior +x → exterior -x → `faces_neg_x = true`.
//! * Vertical edge going down: exterior +x → `faces_neg_x = false`.
//! * Horizontal edge going right (`a.x < b.x`): interior -y → exterior +y → `faces_neg_y = false`.
//! * Horizontal edge going left: exterior -y → `faces_neg_y = true`.
//!
//! Each violation is emitted as a parallelogram spanning each edge's full
//! extent (matching KLayout's edge-pair output): the parallel sides are
//! the two edges in their entirety; the connecting ends are the 45°
//! diagonals between matching endpoints. For fully-overlapping edges
//! the parallelogram collapses to a rectangle; for staggered edges it
//! includes the corner extensions.

use klayout_core::{Point, Polygon};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum AAEdge {
    Horizontal {
        y: i64,
        x_lo: i64,
        x_hi: i64,
        faces_neg_y: bool,
    },
    Vertical {
        x: i64,
        y_lo: i64,
        y_hi: i64,
        faces_neg_x: bool,
    },
}

pub(crate) fn polygon_edges(p: &Polygon) -> Vec<AAEdge> {
    let mut edges = Vec::with_capacity(p.hull.len());
    let n = p.hull.len();
    for i in 0..n {
        let a = p.hull[i];
        let b = p.hull[(i + 1) % n];
        if a.y == b.y && a.x != b.x {
            let faces_neg_y = a.x > b.x;
            edges.push(AAEdge::Horizontal {
                y: a.y,
                x_lo: a.x.min(b.x),
                x_hi: a.x.max(b.x),
                faces_neg_y,
            });
        } else if a.x == b.x && a.y != b.y {
            let faces_neg_x = a.y < b.y;
            edges.push(AAEdge::Vertical {
                x: a.x,
                y_lo: a.y.min(b.y),
                y_hi: a.y.max(b.y),
                faces_neg_x,
            });
        }
    }
    edges
}

/// Width pair predicate (same-polygon, interiors face each other).
/// Plain version without concave-corner extension; kept as a primitive
/// for callers that don't need the extension. The corner-aware version
/// is `width_pair_with_corners`.
#[allow(dead_code)]
pub(crate) fn width_pair(e1: &AAEdge, e2: &AAEdge, min: i64) -> Option<Polygon> {
    pair_check(e1, e2, min, /* is_width */ true)
}

/// Space pair predicate (cross-polygon, exteriors face each other through gap).
pub(crate) fn space_pair(e1: &AAEdge, e2: &AAEdge, min: i64) -> Option<Polygon> {
    pair_check(e1, e2, min, /* is_width */ false)
}

/// Chamfer length for enclosing/overlap/concave-corner pairs: KLayout uses
/// `round(sqrt(min² - dist²))` so the chamfer endpoint is at perpendicular
/// distance exactly `min` from the inner edge endpoint.
pub(crate) fn chamfer_len(min: i64, dist: i64) -> i64 {
    let n = ((min as f64) * (min as f64) - (dist as f64) * (dist as f64)).max(0.0);
    n.sqrt().round() as i64
}

/// Find the polygon's concave corners (interior angle > 180°). For our CW
/// hull convention, a concave corner is a "left turn" — positive cross
/// product of incoming × outgoing edges.
pub(crate) fn concave_corners(p: &klayout_core::Polygon) -> std::collections::HashSet<klayout_core::Point> {
    use klayout_core::Point;
    let mut out: std::collections::HashSet<Point> = std::collections::HashSet::new();
    let n = p.hull.len();
    if n < 3 {
        return out;
    }
    for i in 0..n {
        let prev = p.hull[(i + n - 1) % n];
        let curr = p.hull[i];
        let next = p.hull[(i + 1) % n];
        let dx1 = curr.x - prev.x;
        let dy1 = curr.y - prev.y;
        let dx2 = next.x - curr.x;
        let dy2 = next.y - curr.y;
        // i64 multiplication can overflow at extreme coordinates; promote
        // to i128 to be safe.
        let cross = (dx1 as i128) * (dy2 as i128) - (dy1 as i128) * (dx2 as i128);
        if cross > 0 {
            out.insert(curr);
        }
    }
    out
}

/// Width pair with concave-corner extensions.
///
/// Same as `width_pair`, but if either edge's endpoint at the projection
/// boundary lies at a concave corner of the polygon, the OTHER edge's
/// range in the violation polygon is extended past the projection by
/// `round(sqrt(min² - dist²))` — matching KLayout's edge-pair output for
/// L-shapes and similar concave geometry.
pub(crate) fn width_pair_with_corners(
    e1: &AAEdge,
    e2: &AAEdge,
    min: i64,
    concave: &std::collections::HashSet<klayout_core::Point>,
) -> Option<Polygon> {
    use klayout_core::Point;
    match (e1, e2) {
        (
            AAEdge::Horizontal {
                y: y1,
                x_lo: l1,
                x_hi: h1,
                faces_neg_y: f1,
            },
            AAEdge::Horizontal {
                y: y2,
                x_lo: l2,
                x_hi: h2,
                faces_neg_y: f2,
            },
        ) => {
            if f1 == f2 || y1 == y2 {
                return None;
            }
            let (lo_y, hi_y, lo_l, lo_h, hi_l, hi_h, lo_faces_neg) = if y1 < y2 {
                (*y1, *y2, *l1, *h1, *l2, *h2, *f1)
            } else {
                (*y2, *y1, *l2, *h2, *l1, *h1, *f2)
            };
            if !lo_faces_neg {
                return None;
            }
            let dist = hi_y - lo_y;
            if dist >= min || dist == 0 {
                return None;
            }
            let proj_lo = lo_l.max(hi_l);
            let proj_hi = lo_h.min(hi_h);
            if proj_lo >= proj_hi {
                return None;
            }
            let cham = chamfer_len(min, dist);
            // Determine extensions on each side of the projection range.
            // If the lower-edge endpoint at proj_lo is a concave corner,
            // extend the upper edge by `cham` past proj_lo (clipped by its
            // own range). Symmetric for proj_hi.
            // Concave-corner extensions: when the OPPOSITE edge's endpoint
            // at proj_{lo,hi} is at a concave corner of the polygon, the
            // current edge's range in the violation extends past proj by
            // `cham`, clipped by the edge's own range bounds.
            let lo_l_corner = concave.contains(&Point::new(proj_lo, lo_y));
            let hi_l_corner = concave.contains(&Point::new(proj_lo, hi_y));
            let lo_h_corner = concave.contains(&Point::new(proj_hi, lo_y));
            let hi_h_corner = concave.contains(&Point::new(proj_hi, hi_y));
            let ext_lo_lower = if hi_l_corner { (proj_lo - cham).max(lo_l) } else { proj_lo };
            let ext_lo_upper = if lo_l_corner { (proj_lo - cham).max(hi_l) } else { proj_lo };
            let ext_hi_lower = if hi_h_corner { (proj_hi + cham).min(lo_h) } else { proj_hi };
            let ext_hi_upper = if lo_h_corner { (proj_hi + cham).min(hi_h) } else { proj_hi };
            Some(Polygon::from_hull(vec![
                Point::new(ext_lo_lower, lo_y),
                Point::new(ext_hi_lower, lo_y),
                Point::new(ext_hi_upper, hi_y),
                Point::new(ext_lo_upper, hi_y),
            ]))
        }
        (
            AAEdge::Vertical {
                x: x1,
                y_lo: l1,
                y_hi: h1,
                faces_neg_x: f1,
            },
            AAEdge::Vertical {
                x: x2,
                y_lo: l2,
                y_hi: h2,
                faces_neg_x: f2,
            },
        ) => {
            if f1 == f2 || x1 == x2 {
                return None;
            }
            let (lo_x, hi_x, lo_l, lo_h, hi_l, hi_h, lo_faces_neg) = if x1 < x2 {
                (*x1, *x2, *l1, *h1, *l2, *h2, *f1)
            } else {
                (*x2, *x1, *l2, *h2, *l1, *h1, *f2)
            };
            if !lo_faces_neg {
                return None;
            }
            let dist = hi_x - lo_x;
            if dist >= min || dist == 0 {
                return None;
            }
            let proj_lo = lo_l.max(hi_l);
            let proj_hi = lo_h.min(hi_h);
            if proj_lo >= proj_hi {
                return None;
            }
            let cham = chamfer_len(min, dist);
            let lo_l_corner = concave.contains(&Point::new(lo_x, proj_lo));
            let hi_l_corner = concave.contains(&Point::new(hi_x, proj_lo));
            let lo_h_corner = concave.contains(&Point::new(lo_x, proj_hi));
            let hi_h_corner = concave.contains(&Point::new(hi_x, proj_hi));
            let ext_lo_lower = if hi_l_corner { (proj_lo - cham).max(lo_l) } else { proj_lo };
            let ext_lo_upper = if lo_l_corner { (proj_lo - cham).max(hi_l) } else { proj_lo };
            let ext_hi_lower = if hi_h_corner { (proj_hi + cham).min(lo_h) } else { proj_hi };
            let ext_hi_upper = if lo_h_corner { (proj_hi + cham).min(hi_h) } else { proj_hi };
            Some(Polygon::from_hull(vec![
                Point::new(lo_x, ext_lo_lower),
                Point::new(lo_x, ext_hi_lower),
                Point::new(hi_x, ext_hi_upper),
                Point::new(hi_x, ext_lo_upper),
            ]))
        }
        _ => None,
    }
}

/// Enclosing pair: outer encloses inner by ≥ `min`. Both edges face the
/// same direction (both left edges, both bottom edges, etc.) with inner
/// "more interior" than outer. Polygon is the parallelogram between them
/// with outer's range extended by `chamfer_len` past inner's endpoints.
pub(crate) fn enclosing_pair(
    outer: &AAEdge,
    inner: &AAEdge,
    min: i64,
) -> Option<Polygon> {
    match (outer, inner) {
        (
            AAEdge::Horizontal {
                y: yo,
                x_lo: xo_lo,
                x_hi: xo_hi,
                faces_neg_y: fo,
            },
            AAEdge::Horizontal {
                y: yi,
                x_lo: xi_lo,
                x_hi: xi_hi,
                faces_neg_y: fi,
            },
        ) => {
            if fo != fi {
                return None;
            }
            // Top edges (faces_neg_y=false): inner.y < outer.y.
            // Bottom edges (faces_neg_y=true):  inner.y > outer.y.
            let dist = if !fo {
                if yi >= yo {
                    return None;
                }
                yo - yi
            } else {
                if yi <= yo {
                    return None;
                }
                yi - yo
            };
            if dist >= min || dist == 0 {
                return None;
            }
            let proj_lo = (*xi_lo).max(*xo_lo);
            let proj_hi = (*xi_hi).min(*xo_hi);
            if proj_lo >= proj_hi {
                return None;
            }
            let cham = chamfer_len(min, dist);
            let outer_lo = ((*xi_lo) - cham).max(*xo_lo);
            let outer_hi = ((*xi_hi) + cham).min(*xo_hi);
            // Walk in CW order: top edges → inner first; bottom edges → outer first.
            let pts = if !fo {
                vec![
                    Point::new(*xi_lo, *yi),
                    Point::new(*xi_hi, *yi),
                    Point::new(outer_hi, *yo),
                    Point::new(outer_lo, *yo),
                ]
            } else {
                vec![
                    Point::new(outer_lo, *yo),
                    Point::new(outer_hi, *yo),
                    Point::new(*xi_hi, *yi),
                    Point::new(*xi_lo, *yi),
                ]
            };
            Some(Polygon::from_hull(pts))
        }
        (
            AAEdge::Vertical {
                x: xo,
                y_lo: yo_lo,
                y_hi: yo_hi,
                faces_neg_x: fo,
            },
            AAEdge::Vertical {
                x: xi,
                y_lo: yi_lo,
                y_hi: yi_hi,
                faces_neg_x: fi,
            },
        ) => {
            if fo != fi {
                return None;
            }
            let dist = if !fo {
                if xi >= xo {
                    return None;
                }
                xo - xi
            } else {
                if xi <= xo {
                    return None;
                }
                xi - xo
            };
            if dist >= min || dist == 0 {
                return None;
            }
            let proj_lo = (*yi_lo).max(*yo_lo);
            let proj_hi = (*yi_hi).min(*yo_hi);
            if proj_lo >= proj_hi {
                return None;
            }
            let cham = chamfer_len(min, dist);
            let outer_lo = ((*yi_lo) - cham).max(*yo_lo);
            let outer_hi = ((*yi_hi) + cham).min(*yo_hi);
            let pts = if !fo {
                vec![
                    Point::new(*xi, *yi_lo),
                    Point::new(*xi, *yi_hi),
                    Point::new(*xo, outer_hi),
                    Point::new(*xo, outer_lo),
                ]
            } else {
                vec![
                    Point::new(*xo, outer_lo),
                    Point::new(*xo, outer_hi),
                    Point::new(*xi, *yi_hi),
                    Point::new(*xi, *yi_lo),
                ]
            };
            Some(Polygon::from_hull(pts))
        }
        _ => None,
    }
}

/// Overlap pair: a and b overlap with width < `min`. Edges face opposite
/// (width-style predicate); both edges' ranges are chamfer-extended past
/// the projection-overlap bounds (clipped by their own range).
pub(crate) fn overlap_pair(e1: &AAEdge, e2: &AAEdge, min: i64) -> Option<Polygon> {
    match (e1, e2) {
        (
            AAEdge::Horizontal {
                y: y1,
                x_lo: l1,
                x_hi: h1,
                faces_neg_y: f1,
            },
            AAEdge::Horizontal {
                y: y2,
                x_lo: l2,
                x_hi: h2,
                faces_neg_y: f2,
            },
        ) => {
            if f1 == f2 || y1 == y2 {
                return None;
            }
            let (lo_y, hi_y, lo_l, lo_h, hi_l, hi_h, lo_faces_neg) = if y1 < y2 {
                (*y1, *y2, *l1, *h1, *l2, *h2, *f1)
            } else {
                (*y2, *y1, *l2, *h2, *l1, *h1, *f2)
            };
            // Width-style: lower edge's exterior points away from upper
            // (lo_faces_neg = true).
            if !lo_faces_neg {
                return None;
            }
            let dist = hi_y - lo_y;
            if dist >= min || dist == 0 {
                return None;
            }
            let proj_lo = lo_l.max(hi_l);
            let proj_hi = lo_h.min(hi_h);
            if proj_lo >= proj_hi {
                return None;
            }
            let cham = chamfer_len(min, dist);
            let lo_l_ext = (proj_lo - cham).max(lo_l);
            let lo_h_ext = (proj_hi + cham).min(lo_h);
            let hi_l_ext = (proj_lo - cham).max(hi_l);
            let hi_h_ext = (proj_hi + cham).min(hi_h);
            Some(Polygon::from_hull(vec![
                Point::new(lo_l_ext, lo_y),
                Point::new(lo_h_ext, lo_y),
                Point::new(hi_h_ext, hi_y),
                Point::new(hi_l_ext, hi_y),
            ]))
        }
        (
            AAEdge::Vertical {
                x: x1,
                y_lo: l1,
                y_hi: h1,
                faces_neg_x: f1,
            },
            AAEdge::Vertical {
                x: x2,
                y_lo: l2,
                y_hi: h2,
                faces_neg_x: f2,
            },
        ) => {
            if f1 == f2 || x1 == x2 {
                return None;
            }
            let (lo_x, hi_x, lo_l, lo_h, hi_l, hi_h, lo_faces_neg) = if x1 < x2 {
                (*x1, *x2, *l1, *h1, *l2, *h2, *f1)
            } else {
                (*x2, *x1, *l2, *h2, *l1, *h1, *f2)
            };
            if !lo_faces_neg {
                return None;
            }
            let dist = hi_x - lo_x;
            if dist >= min || dist == 0 {
                return None;
            }
            let proj_lo = lo_l.max(hi_l);
            let proj_hi = lo_h.min(hi_h);
            if proj_lo >= proj_hi {
                return None;
            }
            let cham = chamfer_len(min, dist);
            let lo_l_ext = (proj_lo - cham).max(lo_l);
            let lo_h_ext = (proj_hi + cham).min(lo_h);
            let hi_l_ext = (proj_lo - cham).max(hi_l);
            let hi_h_ext = (proj_hi + cham).min(hi_h);
            Some(Polygon::from_hull(vec![
                Point::new(lo_x, lo_l_ext),
                Point::new(lo_x, lo_h_ext),
                Point::new(hi_x, hi_h_ext),
                Point::new(hi_x, hi_l_ext),
            ]))
        }
        _ => None,
    }
}

fn pair_check(e1: &AAEdge, e2: &AAEdge, min: i64, is_width: bool) -> Option<Polygon> {
    match (e1, e2) {
        (
            AAEdge::Horizontal {
                y: y1,
                x_lo: l1,
                x_hi: h1,
                faces_neg_y: f1,
            },
            AAEdge::Horizontal {
                y: y2,
                x_lo: l2,
                x_hi: h2,
                faces_neg_y: f2,
            },
        ) => {
            if f1 == f2 || y1 == y2 {
                return None;
            }
            // Identify lower vs upper edge (smaller y is "lower").
            let (lo_y, hi_y, lo_l, lo_h, hi_l, hi_h, lo_faces_neg) = if y1 < y2 {
                (*y1, *y2, *l1, *h1, *l2, *h2, *f1)
            } else {
                (*y2, *y1, *l2, *h2, *l1, *h1, *f2)
            };
            if lo_faces_neg != is_width {
                return None;
            }
            let dist = hi_y - lo_y;
            if dist >= min || dist == 0 {
                return None;
            }
            // Projection overlap (in x).
            let proj_lo = lo_l.max(hi_l);
            let proj_hi = lo_h.min(hi_h);
            if proj_lo >= proj_hi {
                return None;
            }
            // Parallelogram: lower edge full span at y=lo_y, upper edge full
            // span at y=hi_y, connected by 45° diagonals at the ends.
            Some(Polygon::from_hull(vec![
                Point::new(lo_l, lo_y),
                Point::new(lo_h, lo_y),
                Point::new(hi_h, hi_y),
                Point::new(hi_l, hi_y),
            ]))
        }
        (
            AAEdge::Vertical {
                x: x1,
                y_lo: l1,
                y_hi: h1,
                faces_neg_x: f1,
            },
            AAEdge::Vertical {
                x: x2,
                y_lo: l2,
                y_hi: h2,
                faces_neg_x: f2,
            },
        ) => {
            if f1 == f2 || x1 == x2 {
                return None;
            }
            let (lo_x, hi_x, lo_l, lo_h, hi_l, hi_h, lo_faces_neg) = if x1 < x2 {
                (*x1, *x2, *l1, *h1, *l2, *h2, *f1)
            } else {
                (*x2, *x1, *l2, *h2, *l1, *h1, *f2)
            };
            if lo_faces_neg != is_width {
                return None;
            }
            let dist = hi_x - lo_x;
            if dist >= min || dist == 0 {
                return None;
            }
            let proj_lo = lo_l.max(hi_l);
            let proj_hi = lo_h.min(hi_h);
            if proj_lo >= proj_hi {
                return None;
            }
            Some(Polygon::from_hull(vec![
                Point::new(lo_x, lo_l),
                Point::new(lo_x, lo_h),
                Point::new(hi_x, hi_h),
                Point::new(hi_x, hi_l),
            ]))
        }
        _ => None,
    }
}

/// Enumerate width violations for a single polygon's edge set, using
/// orientation-bucketed bisection rather than the dense `O(E²)` loop.
///
/// Width pairs only arise between two edges of the same orientation
/// whose face directions are opposite — for horizontals, that means
/// the lower-`y` edge faces `-y` and the higher-`y` edge faces `+y`;
/// vertical case is symmetric in `x`. We separate the polygon's edges
/// into the four buckets by orientation+face, sort the "upper" bucket
/// by perpendicular coord, and for each "lower" edge run a bisection
/// to find candidates whose perpendicular distance is in `(0, min)`.
/// The full corner-aware predicate then runs only on those candidates,
/// keeping behavior bit-identical with the dense path.
///
/// Cost: `O(E · log E + V)` where `V` is the number of viable pairs.
/// For polygons with many edges (e.g. fractured analog shapes,
/// staircase metal jogs, large standard cells), this is materially
/// faster than the `O(E²)` baseline; for tiny polygons (<≈16 edges)
/// the dense path is still preferable, but the asymptotic cost
/// dominates only at the long tail and the constants here are small.
pub(crate) fn width_pairs_indexed(
    edges: &[AAEdge],
    concave: &std::collections::HashSet<klayout_core::Point>,
    min: i64,
) -> Vec<klayout_core::Polygon> {
    let mut out: Vec<klayout_core::Polygon> = Vec::new();
    if edges.is_empty() || min <= 0 {
        return out;
    }

    // Bucket indices by orientation and face direction. The "lower"
    // bucket of each pair holds candidates whose perpendicular coord
    // is the smaller of the two; the "upper" bucket holds the larger.
    let mut h_lower: Vec<usize> = Vec::new(); // horizontals facing -y
    let mut h_upper: Vec<usize> = Vec::new(); // horizontals facing +y
    let mut v_lower: Vec<usize> = Vec::new(); // verticals facing -x
    let mut v_upper: Vec<usize> = Vec::new(); // verticals facing +x
    for (i, e) in edges.iter().enumerate() {
        match e {
            AAEdge::Horizontal { faces_neg_y, .. } => {
                if *faces_neg_y {
                    h_lower.push(i);
                } else {
                    h_upper.push(i);
                }
            }
            AAEdge::Vertical { faces_neg_x, .. } => {
                if *faces_neg_x {
                    v_lower.push(i);
                } else {
                    v_upper.push(i);
                }
            }
        }
    }

    // Sort the "upper" buckets by perpendicular coord and prebuild a
    // parallel coord vector for `partition_point` lookups.
    h_upper.sort_by_key(|&i| match edges[i] {
        AAEdge::Horizontal { y, .. } => y,
        _ => i64::MIN,
    });
    let h_upper_y: Vec<i64> = h_upper
        .iter()
        .map(|&i| match edges[i] {
            AAEdge::Horizontal { y, .. } => y,
            _ => i64::MIN,
        })
        .collect();

    v_upper.sort_by_key(|&i| match edges[i] {
        AAEdge::Vertical { x, .. } => x,
        _ => i64::MIN,
    });
    let v_upper_x: Vec<i64> = v_upper
        .iter()
        .map(|&i| match edges[i] {
            AAEdge::Vertical { x, .. } => x,
            _ => i64::MIN,
        })
        .collect();

    // Horizontal pairs.
    for &li in &h_lower {
        let ly = match edges[li] {
            AAEdge::Horizontal { y, .. } => y,
            _ => continue,
        };
        // Want partner y in the open interval (ly, ly + min).
        let start = h_upper_y.partition_point(|&y| y <= ly);
        let end = h_upper_y.partition_point(|&y| y < ly + min);
        for &ui in &h_upper[start..end] {
            if let Some(p) = width_pair_with_corners(&edges[li], &edges[ui], min, concave) {
                out.push(p);
            }
        }
    }

    // Vertical pairs.
    for &li in &v_lower {
        let lx = match edges[li] {
            AAEdge::Vertical { x, .. } => x,
            _ => continue,
        };
        let start = v_upper_x.partition_point(|&x| x <= lx);
        let end = v_upper_x.partition_point(|&x| x < lx + min);
        for &ui in &v_upper[start..end] {
            if let Some(p) = width_pair_with_corners(&edges[li], &edges[ui], min, concave) {
                out.push(p);
            }
        }
    }

    out
}
