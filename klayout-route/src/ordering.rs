//! Net ordering heuristics for routing.
//!
//! Routing order matters: with sequential routing (each net obstructs
//! the next), the first nets get easier paths and later nets get
//! harder ones. Picking a good order improves routability and total
//! wirelength. Common strategies:
//!
//! * **HPWL ascending** (shortest first) — biases toward routing
//!   easy short nets first, leaving the channels for the long nets.
//!   Often the default in commercial routers.
//! * **HPWL descending** (longest first) — opposite trade-off; long
//!   nets get the freshest channels, short nets fit around them.
//!   Better when long nets dominate the wirelength budget.
//! * **Fanout descending** — high-fanout nets first, on the theory
//!   that they're harder to route (more pins → bigger bbox).
//! * **Criticality-based** — caller supplies a per-net weight, sorted
//!   descending. Used for clock / reset / scan nets that need
//!   priority.

use crate::detailed::RouteRequest;
use klayout_core::Bbox;
#[cfg(test)]
use klayout_core::Point;

pub fn hpwl(req: &RouteRequest) -> i64 {
    if req.pins.is_empty() {
        return 0;
    }
    let mut bb = Bbox::EMPTY;
    for (p, _) in &req.pins {
        bb = bb.union(&Bbox::new(*p, *p));
    }
    bb.width() + bb.height()
}

pub fn fanout(req: &RouteRequest) -> usize {
    req.pins.len()
}

/// Return a permutation `order` such that `requests[order[0]]` has
/// the smallest HPWL.
pub fn hpwl_ascending(requests: &[RouteRequest]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..requests.len()).collect();
    idx.sort_by_key(|&i| hpwl(&requests[i]));
    idx
}

pub fn hpwl_descending(requests: &[RouteRequest]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..requests.len()).collect();
    idx.sort_by_key(|&i| std::cmp::Reverse(hpwl(&requests[i])));
    idx
}

pub fn fanout_descending(requests: &[RouteRequest]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..requests.len()).collect();
    idx.sort_by_key(|&i| std::cmp::Reverse(fanout(&requests[i])));
    idx
}

/// Caller-supplied criticality weight per request. Higher weight =
/// route earlier. Stable order on ties (preserves input order).
pub fn criticality_descending(requests: &[RouteRequest], weights: &[f64]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..requests.len()).collect();
    idx.sort_by(|&a, &b| {
        let wa = weights.get(a).copied().unwrap_or(0.0);
        let wb = weights.get(b).copied().unwrap_or(0.0);
        wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
    });
    idx
}

/// Apply an order permutation to a request batch. Returns a new Vec
/// in the requested order.
pub fn reorder(requests: Vec<RouteRequest>, order: &[usize]) -> Vec<RouteRequest> {
    let mut owned: Vec<Option<RouteRequest>> = requests.into_iter().map(Some).collect();
    let mut out: Vec<RouteRequest> = Vec::with_capacity(owned.len());
    for &i in order {
        if let Some(r) = owned[i].take() {
            out.push(r);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detailed::RouteRequest;

    fn req(name: &str, pins: Vec<Point>) -> RouteRequest {
        RouteRequest {
            net_name: name.into(),
            pins: pins.into_iter().map(|p| (p, 0)).collect(),
            ndr: None,
        }
    }

    #[test]
    fn hpwl_computes_perimeter_of_bounding_box() {
        let r = req("n", vec![Point::new(0, 0), Point::new(10, 5)]);
        assert_eq!(hpwl(&r), 15);
    }

    #[test]
    fn hpwl_ascending_sorts_short_first() {
        let reqs = vec![
            req("long", vec![Point::new(0, 0), Point::new(100, 100)]),
            req("short", vec![Point::new(0, 0), Point::new(5, 5)]),
            req("medium", vec![Point::new(0, 0), Point::new(50, 50)]),
        ];
        let order = hpwl_ascending(&reqs);
        assert_eq!(reqs[order[0]].net_name.as_str(), "short");
        assert_eq!(reqs[order[2]].net_name.as_str(), "long");
    }

    #[test]
    fn fanout_descending_high_pin_count_first() {
        let reqs = vec![
            req("a", vec![Point::new(0, 0), Point::new(10, 0)]),
            req("b", vec![
                Point::new(0, 0),
                Point::new(10, 0),
                Point::new(20, 0),
                Point::new(30, 0),
            ]),
            req("c", vec![Point::new(0, 0), Point::new(10, 0), Point::new(20, 0)]),
        ];
        let order = fanout_descending(&reqs);
        assert_eq!(reqs[order[0]].net_name.as_str(), "b"); // 4 pins
    }

    #[test]
    fn criticality_descending_respects_weights() {
        let reqs = vec![
            req("a", vec![Point::new(0, 0), Point::new(10, 0)]),
            req("clk", vec![Point::new(0, 0), Point::new(10, 0)]),
            req("b", vec![Point::new(0, 0), Point::new(10, 0)]),
        ];
        let weights = vec![1.0, 100.0, 1.0];
        let order = criticality_descending(&reqs, &weights);
        assert_eq!(reqs[order[0]].net_name.as_str(), "clk");
    }

    #[test]
    fn reorder_returns_in_specified_order() {
        let reqs = vec![req("a", vec![]), req("b", vec![]), req("c", vec![])];
        let out = reorder(reqs, &[2, 0, 1]);
        assert_eq!(out[0].net_name.as_str(), "c");
        assert_eq!(out[1].net_name.as_str(), "a");
        assert_eq!(out[2].net_name.as_str(), "b");
    }
}
