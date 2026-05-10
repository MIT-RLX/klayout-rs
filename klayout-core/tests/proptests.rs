//! Property-based invariants for the core math.
//!
//! These complement the corpus-based KLayout differential tests:
//! corpus tests verify *we match KLayout* on specific inputs; these
//! verify *the math behaves correctly* across the entire input space
//! the type system permits. A failure here is a real bug — the
//! invariants tested are universal mathematical properties.

use klayout_core::{
    Bbox, CellBuilder, LayerInfo, Library, Point, Polygon, Rect, Rot4, Trans, Vec2,
};
use proptest::prelude::*;

// ---- generators ----

fn arb_point() -> impl Strategy<Value = Point> {
    (-1_000_000i64..=1_000_000i64, -1_000_000i64..=1_000_000i64).prop_map(|(x, y)| Point::new(x, y))
}

fn arb_rot4() -> impl Strategy<Value = Rot4> {
    prop_oneof![
        Just(Rot4::R0),
        Just(Rot4::R90),
        Just(Rot4::R180),
        Just(Rot4::R270),
    ]
}

fn arb_trans() -> impl Strategy<Value = Trans> {
    (arb_rot4(), any::<bool>(), arb_point()).prop_map(|(rot, mirror, disp)| {
        Trans::new(rot, mirror, Vec2::new(disp.x, disp.y))
    })
}

fn arb_bbox() -> impl Strategy<Value = Bbox> {
    (arb_point(), arb_point()).prop_map(|(a, b)| {
        Bbox::new(
            Point::new(a.x.min(b.x), a.y.min(b.y)),
            Point::new(a.x.max(b.x), a.y.max(b.y)),
        )
    })
}

// ---- Trans invariants ----

proptest! {
    /// `t.compose(t.inverse())` and `t.inverse().compose(t)` both equal IDENTITY.
    #[test]
    fn trans_compose_inverse_is_identity(t in arb_trans()) {
        prop_assert_eq!(t.compose(t.inverse()), Trans::IDENTITY);
        prop_assert_eq!(t.inverse().compose(t), Trans::IDENTITY);
    }

    /// `(a.compose(b)).apply(p) == a.apply(b.apply(p))` for all (a, b, p).
    #[test]
    fn trans_compose_matches_apply(
        a in arb_trans(), b in arb_trans(), p in arb_point()
    ) {
        prop_assert_eq!(a.compose(b).apply(p), a.apply(b.apply(p)));
    }

    /// Composition is associative: `(a∘b)∘c == a∘(b∘c)`.
    #[test]
    fn trans_compose_associative(
        a in arb_trans(), b in arb_trans(), c in arb_trans(),
    ) {
        prop_assert_eq!(a.compose(b).compose(c), a.compose(b.compose(c)));
    }

    /// Applying inverse undoes apply.
    #[test]
    fn trans_apply_inverse_roundtrip(t in arb_trans(), p in arb_point()) {
        prop_assert_eq!(t.inverse().apply(t.apply(p)), p);
    }
}

// ---- Bbox invariants ----

proptest! {
    /// Union is commutative.
    #[test]
    fn bbox_union_commutative(a in arb_bbox(), b in arb_bbox()) {
        prop_assert_eq!(a.union(&b), b.union(&a));
    }

    /// Intersection is commutative.
    #[test]
    fn bbox_intersection_commutative(a in arb_bbox(), b in arb_bbox()) {
        prop_assert_eq!(a.intersection(&b), b.intersection(&a));
    }

    /// Self-union and self-intersect are no-ops.
    #[test]
    fn bbox_self_union_idempotent(a in arb_bbox()) {
        prop_assert_eq!(a.union(&a), a);
        prop_assert_eq!(a.intersection(&a), a);
    }

    /// Union contains both inputs.
    #[test]
    fn bbox_union_contains_both(a in arb_bbox(), b in arb_bbox()) {
        let u = a.union(&b);
        if !a.is_empty() {
            prop_assert!(u.contains(a.min));
            prop_assert!(u.contains(a.max));
        }
        if !b.is_empty() {
            prop_assert!(u.contains(b.min));
            prop_assert!(u.contains(b.max));
        }
    }

    /// Intersection is contained in both inputs.
    #[test]
    fn bbox_intersection_contained_in_both(a in arb_bbox(), b in arb_bbox()) {
        let i = a.intersection(&b);
        if !i.is_empty() {
            prop_assert!(a.contains(i.min));
            prop_assert!(a.contains(i.max));
            prop_assert!(b.contains(i.min));
            prop_assert!(b.contains(i.max));
        }
    }

    /// `intersects` agrees with `intersection().is_empty()`.
    #[test]
    fn bbox_intersects_matches_intersection(a in arb_bbox(), b in arb_bbox()) {
        let intersects = a.intersects(&b);
        let int_empty = a.intersection(&b).is_empty();
        prop_assert_eq!(intersects, !int_empty);
    }

    /// Trans applied to a bbox preserves its area (when bbox is non-empty
    /// and trans is orthogonal).
    #[test]
    fn trans_apply_bbox_preserves_area(t in arb_trans(), b in arb_bbox()) {
        if b.is_empty() { return Ok(()); }
        let area_before = (b.width() as i128) * (b.height() as i128);
        let after = t.apply_bbox(b);
        let area_after = (after.width() as i128) * (after.height() as i128);
        prop_assert_eq!(area_before, area_after);
    }
}

// ---- Polygon invariants ----

#[allow(dead_code)]
fn arb_rect_polygon() -> impl Strategy<Value = Polygon> {
    arb_bbox().prop_map(Polygon::rect)
}

proptest! {
    /// Polygon::from_hull canonicalizes — same vertex set in any rotation
    /// or winding produces the same Polygon.
    #[test]
    fn polygon_from_hull_canonicalizes(b in arb_bbox()) {
        if b.is_empty() { return Ok(()); }
        let corners = b.corners();
        // Try all 4 rotations of the corner list.
        let p0 = Polygon::from_hull(corners);
        for k in 1..4 {
            let mut rotated: Vec<Point> = Vec::with_capacity(4);
            for i in 0..4 {
                rotated.push(corners[(i + k) % 4]);
            }
            let p = Polygon::from_hull(rotated);
            prop_assert_eq!(&p, &p0);
        }
        // And reversed (CCW vs CW).
        let mut rev = corners.to_vec();
        rev.reverse();
        let p_rev = Polygon::from_hull(rev);
        prop_assert_eq!(&p_rev, &p0);
    }
}

// ---- ContentHash invariants ----

proptest! {
    /// Same content → same hash, regardless of which Library it was inserted into.
    #[test]
    fn content_hash_invariant_across_libraries(b in arb_bbox()) {
        if b.is_empty() { return Ok(()); }
        let mk = || {
            let lib = Library::new("t", 1000);
            let l = lib.layer(LayerInfo::gds(1, 0));
            let mut cb = CellBuilder::new("c");
            cb.add_shape(l, Rect::new(b));
            let id = lib.insert(cb);
            (lib, id)
        };
        let (l1, i1) = mk();
        let (l2, i2) = mk();
        prop_assert_eq!(l1.get(i1).content_hash(), l2.get(i2).content_hash());
    }

    /// Shape insertion order does not affect content hash.
    #[test]
    fn content_hash_insertion_order_invariant(
        b1 in arb_bbox(), b2 in arb_bbox(),
    ) {
        if b1.is_empty() || b2.is_empty() { return Ok(()); }
        let lib = Library::new("t", 1000);
        let l = lib.layer(LayerInfo::gds(1, 0));

        let mut cb1 = CellBuilder::new("a");
        cb1.add_shape(l, Rect::new(b1));
        cb1.add_shape(l, Rect::new(b2));
        let h1 = lib.insert(cb1);

        let mut cb2 = CellBuilder::new("b");
        cb2.add_shape(l, Rect::new(b2));
        cb2.add_shape(l, Rect::new(b1));
        let h2 = lib.insert(cb2);

        prop_assert_eq!(lib.get(h1).content_hash(), lib.get(h2).content_hash());
    }
}
