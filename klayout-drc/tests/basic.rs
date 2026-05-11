//! Basic DRC sanity — internal consistency before validating against KLayout.

use klayout_core::{Bbox, Point, Polygon};
use klayout_drc::{
    area_min, density, density_window, density_window_with_config, enclosing, overlap, separation,
    space, space_any, width, width_any, DensityPadding, DensityWindowConfig, DensityWindowOutput,
};
use klayout_geom::Region;

fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Polygon {
    Polygon::rect(Bbox::new(Point::new(x0, y0), Point::new(x1, y1)))
}

fn region(rs: impl IntoIterator<Item = Polygon>) -> Region {
    Region::from_polygons(rs)
}

#[test]
fn width_passes_wide_enough() {
    let r = region([rect(0, 0, 100, 50)]);  // 50 tall, 100 wide
    assert!(width(&r, 40).is_empty(), "width 50 should pass min=40");
    // Odd min: exact match with KLayout's strict-less-than semantics.
    assert!(width(&r, 49).is_empty(), "width 50 should pass min=49");
}

#[test]
fn width_flags_too_thin() {
    let r = region([rect(0, 0, 100, 5)]);  // 5 tall — way too thin for min=10
    let v = width(&r, 10);
    assert!(!v.is_empty());
    // Whole rect is the violation
    assert_eq!(v.bbox(), Bbox::new(Point::new(0, 0), Point::new(100, 5)));
}

#[test]
fn space_passes_when_far() {
    let r = region([rect(0, 0, 10, 10), rect(50, 0, 60, 10)]);
    assert!(space(&r, 30).is_empty(), "40 apart should pass min=30");
}

#[test]
fn space_flags_when_close() {
    let r = region([rect(0, 0, 10, 10), rect(15, 0, 25, 10)]);  // 5 apart
    let v = space(&r, 10);
    assert!(!v.is_empty());
}

#[test]
fn separation_two_layers() {
    let a = region([rect(0, 0, 10, 10)]);
    let b = region([rect(15, 0, 25, 10)]);  // 5 apart from a
    assert!(separation(&a, &b, 4).is_empty(), "5 apart, min=4 should pass");
    assert!(!separation(&a, &b, 10).is_empty(), "5 apart, min=10 should flag");
}

#[test]
fn enclosing_passes_when_safe() {
    let outer = region([rect(0, 0, 100, 100)]);
    let inner = region([rect(20, 20, 50, 50)]);  // 20 from edge
    assert!(enclosing(&outer, &inner, 10).is_empty());
    assert!(enclosing(&outer, &inner, 20).is_empty(), "exactly 20 from edge passes min=20");
}

#[test]
fn enclosing_flags_when_too_close() {
    let outer = region([rect(0, 0, 100, 100)]);
    let inner = region([rect(5, 5, 50, 50)]);  // 5 from edge
    assert!(!enclosing(&outer, &inner, 10).is_empty());
}

#[test]
fn overlap_passes_when_wide_enough() {
    let a = region([rect(0, 0, 100, 100)]);
    let b = region([rect(50, 50, 150, 150)]);  // overlaps in (50,50)-(100,100), width 50
    assert!(overlap(&a, &b, 40).is_empty());
}

#[test]
fn overlap_flags_when_thin() {
    let a = region([rect(0, 0, 100, 100)]);
    let b = region([rect(95, 95, 200, 200)]);  // overlaps 5x5
    assert!(!overlap(&a, &b, 10).is_empty());
}

#[test]
fn density_passes_in_target_range() {
    // 50x50 polygon in a 100x100 layout = 25% density. Single 100x100 window.
    let r = region([rect(0, 0, 50, 50)]);
    let viols = density(&r, (100, 100), (100, 100), 0.20, 0.30);
    assert!(viols.is_empty(), "25% density passes range [0.20, 0.30]");
}

#[test]
fn density_flags_below_minimum() {
    let r = region([rect(0, 0, 10, 10)]); // 100/10000 = 1%
    let viols = density(&r, (100, 100), (100, 100), 0.30, 0.70);
    assert_eq!(viols.len(), 1, "1% density falls below min=30%");
}

#[test]
fn density_flags_above_maximum() {
    let r = region([rect(0, 0, 90, 90)]); // 8100/10000 = 81%
    let viols = density(&r, (100, 100), (100, 100), 0.30, 0.70);
    assert_eq!(viols.len(), 1, "81% density above max=70%");
}

#[test]
fn density_sliding_window_iterates() {
    // Two blobs in one bbox — centered tile grid yields several out-of-range windows;
    // results are merged so touching violation strips become one polygon.
    let r = region([
        rect(0, 0, 90, 90),       // dense tile
        rect(500, 0, 510, 10),    // sparse strip
    ]);
    let viols = density(&r, (100, 100), (100, 100), 0.30, 0.70);
    assert!(
        !viols.is_empty(),
        "expect merged violation geometry from multi-tile density scan",
    );
}

#[test]
fn density_strict_singleton_matches_klayout() {
    let r = region([rect(0, 0, 10, 10)]);
    let cfg = DensityWindowConfig::klayout_strict_singleton();
    let v = density_window_with_config(&r, (100, 100), (100, 100), 0.30, 0.70, &cfg);
    assert!(v.is_empty(), "bare KLayout 1×1 tile plan has no _tile → no violations");
}

#[test]
fn density_padding_ignore_changes_denominator() {
    // Narrow metal in a wide boundary strip — Ignore uses overlap area as denom.
    let r = region([rect(0, 0, 50, 10)]);
    let boundary = region([rect(0, 0, 200, 10)]);
    let mut cfg = DensityWindowConfig::default();
    cfg.boundary = Some(boundary);
    cfg.padding = DensityPadding::Ignore;
    let v = density_window_with_config(&r, (100, 100), (100, 100), 0.45, 0.55, &cfg);
    assert!(
        !v.is_empty(),
        "50% line density in strip should fail a tight [0.45,0.55] band under padding_ignore",
    );
}

#[test]
fn density_with_density_emits_in_band_windows() {
    let r = region([rect(0, 0, 50, 50)]);
    let mut cfg = DensityWindowConfig::default();
    cfg.output = DensityWindowOutput::InsideBand;
    let got = density_window_with_config(&r, (100, 100), (100, 100), 0.20, 0.30, &cfg);
    let outside = density_window_with_config(&r, (100, 100), (100, 100), 0.20, 0.30, &DensityWindowConfig::default());
    assert!(outside.is_empty(), "25% in band → no without_density output");
    assert_eq!(got.len(), 1);
    assert_eq!(
        got.bbox(),
        Bbox::new(Point::new(-25, -25), Point::new(75, 75)),
    );
}

#[test]
fn density_window_matches_density() {
    let r = region([rect(0, 0, 10, 10)]);
    let a = density(&r, (100, 100), (100, 100), 0.30, 0.70);
    let b = density_window(&r, (100, 100), (100, 100), 0.30, 0.70);
    assert_eq!(a.len(), b.len());
    assert_eq!(a.bbox(), b.bbox());
}

#[test]
fn area_filters_small_polygons() {
    let r = region([rect(0, 0, 10, 10), rect(50, 0, 51, 1)]);  // 100 vs 1
    let v = area_min(&r, 10);
    assert_eq!(v.len(), 1, "1-DBU² polygon should be flagged for min=10");
    assert_eq!(v.bbox(), Bbox::new(Point::new(50, 0), Point::new(51, 1)));
}

#[test]
fn width_any_matches_width_on_aa_input() {
    // 5-tall thin wire — should be flagged by both kernels.
    let r = region([rect(0, 0, 100, 5)]);
    let v_aa = width(&r, 10);
    let v_any = width_any(&r, 10);
    assert_eq!(v_aa.is_empty(), v_any.is_empty());
    assert_eq!(v_aa.len(), v_any.len());
}

#[test]
fn space_any_matches_space_on_aa_input() {
    let r = region([rect(0, 0, 10, 10), rect(15, 0, 25, 10)]);
    let v_aa = space(&r, 10);
    let v_any = space_any(&r, 10);
    assert_eq!(v_aa.is_empty(), v_any.is_empty());
    assert_eq!(v_aa.len(), v_any.len());
}

#[test]
fn width_any_flags_diagonal_thin_strip() {
    // 100×4 rectangle rotated by 45° around origin — all four edges are
    // diagonal. Width perpendicular ≈ 4 → should fail width_any(10).
    // Hull rounded to integer DBU.
    let pts = vec![
        Point::new(0, 0),
        Point::new(71, 71),
        Point::new(68, 74),
        Point::new(-3, 3),
    ];
    let r = region([Polygon::from_hull(pts)]);
    let v = width_any(&r, 10);
    assert!(!v.is_empty(), "rotated thin rect should flag width_any");
    // The axis-aligned kernel should miss it — no purely horizontal or
    // vertical edges in this polygon.
    assert!(width(&r, 10).is_empty(), "AA width should miss diagonal");
}
