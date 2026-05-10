//! DRC waivers — exempt regions from violation reporting.
//!
//! Production decks always need a waiver mechanism: known-good
//! analog islands, IP blocks already signed off elsewhere, and ECO
//! patches that the design team has accepted out-of-band. Without
//! waivers every signoff run is buried in stale violations.
//!
//! Model:
//! * [`Waiver`] — a name (rule id), a region (the exempt area), and
//!   an optional reason string for audit logs.
//! * [`apply_waivers`] — given the rule's violation `Region`, subtract
//!   any waiver region whose `rule_id` matches. Violations that fall
//!   *partially* outside any waiver are kept; the remaining slice is
//!   reported. Violations entirely covered by waivers vanish.
//! * [`apply_waivers_logged`] — same as `apply_waivers` but returns
//!   the per-waiver "consumed" sub-region for audit reporting.

use klayout_geom::{difference, intersection, Region};
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct Waiver {
    pub rule_id: SmolStr,
    pub region: Region,
    pub reason: SmolStr,
}

impl Waiver {
    pub fn new(rule_id: impl Into<SmolStr>, region: Region) -> Self {
        Self {
            rule_id: rule_id.into(),
            region,
            reason: SmolStr::default(),
        }
    }

    pub fn with_reason(mut self, reason: impl Into<SmolStr>) -> Self {
        self.reason = reason.into();
        self
    }
}

/// Subtract any matching waiver region from the violations of
/// `rule_id`. Non-matching waivers are ignored.
pub fn apply_waivers(rule_id: &str, violations: Region, waivers: &[Waiver]) -> Region {
    let mut result = violations;
    for w in waivers {
        if w.rule_id != rule_id {
            continue;
        }
        result = difference(&result, &w.region);
    }
    result
}

/// Apply waivers and return both the surviving violations and the per-
/// waiver consumed area (for audit).
pub fn apply_waivers_logged(
    rule_id: &str,
    violations: Region,
    waivers: &[Waiver],
) -> (Region, Vec<WaiverHit>) {
    let mut surviving = violations.clone();
    let mut hits: Vec<WaiverHit> = Vec::new();
    for w in waivers {
        if w.rule_id != rule_id {
            continue;
        }
        let consumed = intersection(&surviving, &w.region);
        if !consumed.is_empty() {
            hits.push(WaiverHit {
                rule_id: w.rule_id.clone(),
                reason: w.reason.clone(),
                consumed,
            });
            surviving = difference(&surviving, &w.region);
        }
    }
    (surviving, hits)
}

#[derive(Clone, Debug)]
pub struct WaiverHit {
    pub rule_id: SmolStr,
    pub reason: SmolStr,
    pub consumed: Region,
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::{Bbox, Point, Polygon};

    fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Polygon {
        Polygon::rect(Bbox::new(Point::new(x0, y0), Point::new(x1, y1)))
    }

    #[test]
    fn waiver_fully_covers_violation() {
        let viol = Region::from_polygons([rect(0, 0, 10, 10)]);
        let w = Waiver::new(
            "M1.W.1",
            Region::from_polygons([rect(-5, -5, 15, 15)]),
        );
        let result = apply_waivers("M1.W.1", viol, &[w]);
        assert!(result.is_empty());
    }

    #[test]
    fn waiver_partially_covers_violation() {
        let viol = Region::from_polygons([rect(0, 0, 100, 10)]);
        let w = Waiver::new(
            "M1.W.1",
            Region::from_polygons([rect(0, 0, 50, 10)]),
        );
        let result = apply_waivers("M1.W.1", viol, &[w]);
        // Half the violation remains.
        assert!(!result.is_empty());
        assert_eq!(result.bbox().min.x, 50);
    }

    #[test]
    fn non_matching_rule_id_ignored() {
        let viol = Region::from_polygons([rect(0, 0, 10, 10)]);
        let w = Waiver::new(
            "OTHER.RULE",
            Region::from_polygons([rect(-5, -5, 15, 15)]),
        );
        let result = apply_waivers("M1.W.1", viol.clone(), &[w]);
        assert_eq!(result.len(), viol.len());
    }

    #[test]
    fn multiple_waivers_compose() {
        let viol = Region::from_polygons([
            rect(0, 0, 10, 10),
            rect(100, 0, 110, 10),
        ]);
        let w1 = Waiver::new(
            "M1.W.1",
            Region::from_polygons([rect(0, 0, 10, 10)]),
        );
        let w2 = Waiver::new(
            "M1.W.1",
            Region::from_polygons([rect(100, 0, 110, 10)]),
        );
        let result = apply_waivers("M1.W.1", viol, &[w1, w2]);
        assert!(result.is_empty());
    }

    #[test]
    fn logged_variant_returns_hits() {
        let viol = Region::from_polygons([rect(0, 0, 10, 10)]);
        let w = Waiver::new(
            "M1.W.1",
            Region::from_polygons([rect(0, 0, 5, 10)]),
        )
        .with_reason("analog island AB-7");
        let (remaining, hits) = apply_waivers_logged("M1.W.1", viol, &[w]);
        assert!(!remaining.is_empty());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].reason.as_str(), "analog island AB-7");
        assert!(!hits[0].consumed.is_empty());
    }
}
