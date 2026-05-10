//! Hierarchical connectivity extraction tests.

use klayout_connect::{extract_hierarchical, Conductor, ExtractConfig, Via};
use klayout_core::{Bbox, CellBuilder, Instance, LayerInfo, Library, Point, Rect, Trans, Vec2};

fn lib() -> Library {
    Library::new("t", 1000)
}

#[test]
fn instance_descent_finds_nets_at_top() {
    let lib = lib();
    let m1 = lib.layer(LayerInfo::gds(10, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));

    // Leaf cell with one rect.
    let mut leaf = CellBuilder::new("leaf");
    leaf.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let leaf_id = lib.insert(leaf);

    // Top cell with two instances of leaf far apart.
    let mut top = CellBuilder::new("top");
    top.add_instance(Instance::new(leaf_id, Trans::IDENTITY));
    top.add_instance(Instance::new(leaf_id, Trans::translate(Vec2::new(50, 0))));
    let top_id = lib.insert(top);

    let cfg = ExtractConfig {
        conductors: vec![Conductor {
            layer: m1,
            label_layer: lbl,
        }],
        vias: vec![],
    };
    let nl = extract_hierarchical(&lib, top_id, &cfg);
    assert_eq!(nl.len(), 2, "two disjoint instances → two nets");
}

#[test]
fn touching_instances_merge_into_one_net() {
    let lib = lib();
    let m1 = lib.layer(LayerInfo::gds(10, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));

    let mut leaf = CellBuilder::new("leaf");
    leaf.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let leaf_id = lib.insert(leaf);

    let mut top = CellBuilder::new("top");
    top.add_instance(Instance::new(leaf_id, Trans::IDENTITY));
    // Touching at x=10
    top.add_instance(Instance::new(leaf_id, Trans::translate(Vec2::new(10, 0))));
    let top_id = lib.insert(top);

    let cfg = ExtractConfig {
        conductors: vec![Conductor {
            layer: m1,
            label_layer: lbl,
        }],
        vias: vec![],
    };
    let nl = extract_hierarchical(&lib, top_id, &cfg);
    assert_eq!(nl.len(), 1);
}

#[test]
fn via_stitches_two_layers() {
    let lib = lib();
    let m1 = lib.layer(LayerInfo::gds(10, 0));
    let m2 = lib.layer(LayerInfo::gds(20, 0));
    let via12 = lib.layer(LayerInfo::gds(11, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));

    let mut top = CellBuilder::new("top");
    // M1 piece A
    top.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    // M2 piece B
    top.add_shape(m2, Rect::new(Bbox::new(Point::new(5, 5), Point::new(20, 20))));
    // VIA12 connecting them in the overlap
    top.add_shape(via12, Rect::new(Bbox::new(Point::new(6, 6), Point::new(8, 8))));
    let top_id = lib.insert(top);

    let cfg = ExtractConfig {
        conductors: vec![
            Conductor { layer: m1, label_layer: lbl },
            Conductor { layer: m2, label_layer: lbl },
        ],
        vias: vec![Via { layer: via12, a: m1, b: m2 }],
    };
    let nl = extract_hierarchical(&lib, top_id, &cfg);
    assert_eq!(nl.len(), 1, "via should join the m1 and m2 pieces into one net");
}

#[test]
fn via_does_not_stitch_when_outside_overlap() {
    let lib = lib();
    let m1 = lib.layer(LayerInfo::gds(10, 0));
    let m2 = lib.layer(LayerInfo::gds(20, 0));
    let via12 = lib.layer(LayerInfo::gds(11, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));

    let mut top = CellBuilder::new("top");
    top.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    top.add_shape(m2, Rect::new(Bbox::new(Point::new(50, 50), Point::new(60, 60))));
    // Via at (100,100) — touches neither piece
    top.add_shape(via12, Rect::new(Bbox::new(Point::new(100, 100), Point::new(102, 102))));
    let top_id = lib.insert(top);

    let cfg = ExtractConfig {
        conductors: vec![
            Conductor { layer: m1, label_layer: lbl },
            Conductor { layer: m2, label_layer: lbl },
        ],
        vias: vec![Via { layer: via12, a: m1, b: m2 }],
    };
    let nl = extract_hierarchical(&lib, top_id, &cfg);
    assert_eq!(nl.len(), 2);
}

#[test]
fn nested_hierarchy_three_levels() {
    let lib = lib();
    let m1 = lib.layer(LayerInfo::gds(10, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));

    let mut leaf = CellBuilder::new("leaf");
    leaf.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(5, 5))));
    let leaf_id = lib.insert(leaf);

    let mut mid = CellBuilder::new("mid");
    mid.add_instance(Instance::new(leaf_id, Trans::IDENTITY));
    mid.add_instance(Instance::new(leaf_id, Trans::translate(Vec2::new(10, 0))));
    let mid_id = lib.insert(mid);

    let mut top = CellBuilder::new("top");
    top.add_instance(Instance::new(mid_id, Trans::IDENTITY));
    top.add_instance(Instance::new(mid_id, Trans::translate(Vec2::new(0, 50))));
    let top_id = lib.insert(top);

    let cfg = ExtractConfig {
        conductors: vec![Conductor { layer: m1, label_layer: lbl }],
        vias: vec![],
    };
    let nl = extract_hierarchical(&lib, top_id, &cfg);
    // Each mid has 2 disjoint leaves; top has 2 mids → 4 disjoint nets total.
    assert_eq!(nl.len(), 4);
}
