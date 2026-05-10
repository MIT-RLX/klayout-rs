//! Connectivity extraction smoke tests.

use klayout_connect::extract_flat;
use klayout_core::{Bbox, CellBuilder, LayerInfo, Library, Point, Rect, Text};

#[test]
fn two_disjoint_rects_are_two_nets() {
    let lib = Library::new("t", 1000);
    let m1 = lib.layer(LayerInfo::named("METAL1", 10, 0));
    let lbl = lib.layer(LayerInfo::named("LABEL", 99, 0));
    let mut cb = CellBuilder::new("top");
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(50, 50), Point::new(60, 60))));
    let id = lib.insert(cb);

    let nl = extract_flat(&lib, id, m1, lbl);
    assert_eq!(nl.len(), 2);
}

#[test]
fn touching_rects_are_one_net() {
    let lib = Library::new("t", 1000);
    let m1 = lib.layer(LayerInfo::gds(10, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));
    let mut cb = CellBuilder::new("top");
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    // Touching at x=10
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(10, 0), Point::new(20, 10))));
    let id = lib.insert(cb);

    let nl = extract_flat(&lib, id, m1, lbl);
    assert_eq!(nl.len(), 1);
}

#[test]
fn labeled_net_takes_label_name() {
    let lib = Library::new("t", 1000);
    let m1 = lib.layer(LayerInfo::gds(10, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));
    let mut cb = CellBuilder::new("top");
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(50, 0), Point::new(60, 10))));
    cb.add_shape(lbl, Text::new("VDD", Point::new(5, 5)));
    cb.add_shape(lbl, Text::new("GND", Point::new(55, 5)));
    let id = lib.insert(cb);

    let nl = extract_flat(&lib, id, m1, lbl);
    assert_eq!(nl.len(), 2);
    assert!(nl.by_name("VDD").is_some());
    assert!(nl.by_name("GND").is_some());
}

#[test]
fn unlabeled_nets_get_auto_names() {
    let lib = Library::new("t", 1000);
    let m1 = lib.layer(LayerInfo::gds(10, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));
    let mut cb = CellBuilder::new("top");
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(50, 0), Point::new(60, 10))));
    let id = lib.insert(cb);

    let nl = extract_flat(&lib, id, m1, lbl);
    assert_eq!(nl.len(), 2);
    assert!(nl.by_name("net_0").is_some());
    assert!(nl.by_name("net_1").is_some());
}
