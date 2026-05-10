use klayout_core::{
    clip_cell, flatten, remap_layers, replace_instances, Angle90, Bbox, BuildCtx, CellBuilder,
    Component, Instance, LayerInfo, LayerMap, Library, Point, Port, Rect, Rot4, Trans, Vec2,
};
use std::hash::Hash;

#[derive(Hash)]
struct Rectangle {
    width_dbu: i64,
    height_dbu: i64,
}

impl Component for Rectangle {
    fn build(&self, ctx: &BuildCtx<'_>) -> CellBuilder {
        let wg = ctx.lib.layer(LayerInfo::named("WG", 1, 0));
        let mut b = CellBuilder::new("");
        b.add_shape(
            wg,
            Rect::new(Bbox::new(
                Point::new(0, 0),
                Point::new(self.width_dbu, self.height_dbu),
            )),
        );
        b.add_port(Port::new(
            "o1",
            wg,
            Point::new(0, self.height_dbu / 2),
            Angle90::W,
            self.height_dbu,
        ));
        b.add_port(Port::new(
            "o2",
            wg,
            Point::new(self.width_dbu, self.height_dbu / 2),
            Angle90::E,
            self.height_dbu,
        ));
        b
    }
}

#[derive(Hash)]
struct PairOfRects {
    width_dbu: i64,
    height_dbu: i64,
    spacing_dbu: i64,
}

impl Component for PairOfRects {
    fn build(&self, ctx: &BuildCtx<'_>) -> CellBuilder {
        let r = ctx.lib.build(&Rectangle {
            width_dbu: self.width_dbu,
            height_dbu: self.height_dbu,
        });
        let mut b = CellBuilder::new("pair");
        b.add_instance(Instance::new(r, Trans::IDENTITY));
        b.add_instance(Instance::new(
            r,
            Trans::translate(Vec2::new(self.width_dbu + self.spacing_dbu, 0)),
        ));
        b
    }
}

#[test]
fn build_and_dedup() {
    let lib = Library::new("test", 1000);

    let a = lib.build(&Rectangle {
        width_dbu: 100,
        height_dbu: 50,
    });
    let b = lib.build(&Rectangle {
        width_dbu: 100,
        height_dbu: 50,
    });
    assert_eq!(a, b, "param dedup should reuse the same CellId");
    assert_eq!(lib.cell_count(), 1);

    let c = lib.build(&Rectangle {
        width_dbu: 200,
        height_dbu: 50,
    });
    assert_ne!(a, c);
    assert_eq!(lib.cell_count(), 2);
}

#[test]
fn content_dedup_across_paths() {
    let lib = Library::new("test", 1000);

    // Two builders with identical content but different param origins
    // (one direct via builder, one via Component) should dedup.
    let direct = {
        let wg = lib.layer(LayerInfo::named("WG", 1, 0));
        let mut b = CellBuilder::new("manual");
        b.add_shape(
            wg,
            Rect::new(Bbox::new(Point::new(0, 0), Point::new(100, 50))),
        );
        b.add_port(Port::new(
            "o1",
            wg,
            Point::new(0, 25),
            Angle90::W,
            50,
        ));
        b.add_port(Port::new(
            "o2",
            wg,
            Point::new(100, 25),
            Angle90::E,
            50,
        ));
        lib.insert(b)
    };

    let via_component = lib.build(&Rectangle {
        width_dbu: 100,
        height_dbu: 50,
    });

    assert_eq!(
        direct, via_component,
        "different code paths producing identical content should dedup"
    );
}

#[test]
fn hierarchy_full_bbox() {
    let lib = Library::new("test", 1000);
    let pair = lib.build(&PairOfRects {
        width_dbu: 100,
        height_dbu: 50,
        spacing_dbu: 30,
    });

    let cell = lib.get(pair);
    let bbox = cell.full_bbox(&lib);
    // First rect: (0,0)-(100,50). Second rect placed at x=130: (130,0)-(230,50).
    assert_eq!(bbox.min, Point::new(0, 0));
    assert_eq!(bbox.max, Point::new(230, 50));
}

#[test]
fn content_hash_is_deterministic() {
    let make = || {
        let lib = Library::new("test", 1000);
        let id = lib.build(&Rectangle {
            width_dbu: 100,
            height_dbu: 50,
        });
        lib.get(id).content_hash()
    };
    assert_eq!(make(), make());
}

#[test]
fn content_hash_independent_of_insertion_order() {
    let lib1 = Library::new("a", 1000);
    let lib2 = Library::new("b", 1000);
    let wg1 = lib1.layer(LayerInfo::named("WG", 1, 0));
    let wg2 = lib2.layer(LayerInfo::named("WG", 1, 0));

    let mut b1 = CellBuilder::new("c");
    b1.add_shape(wg1, Bbox::new(Point::new(0, 0), Point::new(10, 10)));
    b1.add_shape(wg1, Bbox::new(Point::new(20, 20), Point::new(30, 30)));

    let mut b2 = CellBuilder::new("c");
    b2.add_shape(wg2, Bbox::new(Point::new(20, 20), Point::new(30, 30)));
    b2.add_shape(wg2, Bbox::new(Point::new(0, 0), Point::new(10, 10)));

    let id1 = lib1.insert(b1);
    let id2 = lib2.insert(b2);
    assert_eq!(
        lib1.get(id1).content_hash(),
        lib2.get(id2).content_hash(),
        "shape order should not affect content hash"
    );
}

#[test]
fn port_transform_under_rotation() {
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));
    let p = Port::new("o1", wg, Point::new(10, 0), Angle90::E, 50);
    let t = Trans::rotate(Rot4::R90);
    let p2 = p.transform(t);
    assert_eq!(p2.center, Point::new(0, 10));
    assert_eq!(p2.angle, Angle90::N);
}

#[test]
fn clip_cell_restricts_to_bbox() {
    let lib = Library::new("t", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));
    let mut cb = CellBuilder::new("c");
    cb.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    cb.add_shape(l, Rect::new(Bbox::new(Point::new(50, 50), Point::new(60, 60))));
    cb.add_shape(l, Rect::new(Bbox::new(Point::new(5, 5), Point::new(55, 55))));
    let id = lib.insert(cb);

    // Clip to bbox covering only the first rect plus part of the diagonal.
    let clipped = clip_cell(&lib, id, Bbox::new(Point::new(0, 0), Point::new(20, 20)), "clipped");
    let new_id = lib.insert(clipped);
    let cell = lib.get(new_id);
    let shapes: Vec<_> = cell.layers().flat_map(|l| cell.shapes_on(l)).collect();
    // First rect (0..10) fully inside, kept whole.
    // Third rect (5..55) intersects the clip, kept whole (approximate clip).
    // Second rect (50..60) is outside the clip → dropped.
    assert_eq!(shapes.len(), 2);
}

#[test]
fn clip_cell_drops_instances_outside() {
    let lib = Library::new("t", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));
    let mut leaf = CellBuilder::new("u");
    leaf.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(5, 5))));
    let leaf_id = lib.insert(leaf);

    let mut top = CellBuilder::new("top");
    top.add_instance(Instance::new(leaf_id, Trans::IDENTITY));
    top.add_instance(Instance::new(leaf_id, Trans::translate(Vec2::new(50, 0))));
    let top_id = lib.insert(top);

    let clipped = clip_cell(&lib, top_id, Bbox::new(Point::new(0, 0), Point::new(20, 20)), "c");
    let cid = lib.insert(clipped);
    let cell = lib.get(cid);
    assert_eq!(cell.instances().len(), 1, "instance at (50,0) is outside the clip");
}

#[test]
fn flatten_inlines_instance_shapes() {
    let lib = Library::new("t", 1000);
    let l = lib.layer(LayerInfo::named("WG", 1, 0));
    let mut leaf = CellBuilder::new("leaf");
    leaf.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let leaf_id = lib.insert(leaf);

    let mut top = CellBuilder::new("top");
    top.add_instance(Instance::new(leaf_id, Trans::IDENTITY));
    top.add_instance(Instance::new(leaf_id, Trans::translate(Vec2::new(50, 0))));
    let top_id = lib.insert(top);

    let flat = flatten(&lib, top_id, usize::MAX, "top_flat");
    let flat_id = lib.insert(flat);
    let cell = lib.get(flat_id);
    // Each instance contributes one shape → 2 shapes after flattening.
    let shape_count: usize = cell.layers().map(|l| cell.shapes_on(l).count()).sum();
    assert_eq!(shape_count, 2);
    // No instances remain at full flatten.
    assert_eq!(cell.instances().len(), 0);
}

#[test]
fn remap_layers_translates_shapes() {
    let lib = Library::new("t", 1000);
    let from = lib.layer(LayerInfo::named("WG", 1, 0));
    let to = lib.layer(LayerInfo::named("METAL1", 10, 0));
    let mut cb = CellBuilder::new("c");
    cb.add_shape(from, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let id = lib.insert(cb);

    let map = LayerMap::new().map(from, to);
    let remapped = remap_layers(&lib, id, &map);
    let new_id = lib.insert(remapped);
    let cell = lib.get(new_id);
    assert_eq!(cell.shapes_on(from).count(), 0);
    assert_eq!(cell.shapes_on(to).count(), 1);
}

#[test]
fn replace_instances_swaps_target() {
    let lib = Library::new("t", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));

    let mut leaf_a = CellBuilder::new("a");
    leaf_a.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let a_id = lib.insert(leaf_a);

    let mut leaf_b = CellBuilder::new("b");
    leaf_b.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(20, 20))));
    let b_id = lib.insert(leaf_b);

    let mut top = CellBuilder::new("top");
    top.add_instance(Instance::new(a_id, Trans::IDENTITY));
    top.add_instance(Instance::new(a_id, Trans::translate(Vec2::new(50, 0))));
    let top_id = lib.insert(top);

    let new_top = replace_instances(&lib, top_id, |id| id == a_id, b_id);
    let new_top_id = lib.insert(new_top);
    let cell = lib.get(new_top_id);
    assert_eq!(cell.instances().len(), 2);
    for inst in cell.instances() {
        assert_eq!(inst.cell, b_id);
    }
}

#[test]
fn top_cells_excludes_children() {
    let lib = Library::new("test", 1000);
    let _pair = lib.build(&PairOfRects {
        width_dbu: 100,
        height_dbu: 50,
        spacing_dbu: 30,
    });
    // Library contains 2 cells: Rectangle (child) and PairOfRects (parent).
    // Only PairOfRects should be a top cell.
    let tops = lib.top_cells();
    assert_eq!(tops.len(), 1);
    assert_eq!(lib.get(tops[0]).name().as_str(), "pair");
}
