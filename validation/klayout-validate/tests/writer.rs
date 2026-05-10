//! GDS writer parity: build a Library, write GDS via our writer, then have
//! KLayout (subprocess) read+dump it. Compare to our own canonical_dump.
//!
//! Skipped if no klayout-equipped python is available.

use klayout_core::{
    Bbox, CellBuilder, Instance, LayerInfo, Library, Path, PathCap, Point, Polygon,
    PropertyValue, Rect, Repetition, Rot4, Text, Trans, Vec2,
};
use klayout_io::write_gds_path;
use klayout_validate::{canonical_dump, klayout_canonical_dump, klayout_python};

fn check_lib(case: &str, lib: &Library) {
    let Some(_) = klayout_python() else {
        eprintln!("skipping '{case}' — no KLayout-equipped python (set KLAYOUT_PYTHON or install /tmp/klayout-venv)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let gds_path = tmp.path().join(format!("{case}.gds"));
    write_gds_path(lib, &gds_path).unwrap_or_else(|e| panic!("write failed: {e}"));

    let ours = canonical_dump(lib);
    let theirs = klayout_canonical_dump(&gds_path).expect("klayout python missing");

    if ours != theirs {
        let o = serde_json::to_string_pretty(&ours).unwrap();
        let t = serde_json::to_string_pretty(&theirs).unwrap();
        panic!(
            "writer parity mismatch for '{case}'\n\n--- ours ---\n{o}\n\n--- klayout ---\n{t}\n"
        );
    }
}

#[test]
fn single_box() {
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));
    let mut cb = CellBuilder::new("box");
    cb.add_shape(wg, Rect::new(Bbox::new(Point::new(0, 0), Point::new(100, 50))));
    lib.insert(cb);
    check_lib("single_box", &lib);
}

#[test]
fn polygon_irregular() {
    let lib = Library::new("test", 1000);
    let l = lib.layer(LayerInfo::named("WG", 1, 0));
    let mut cb = CellBuilder::new("hex");
    cb.add_shape(
        l,
        Polygon::from_hull([
            Point::new(0, 50),
            Point::new(43, 25),
            Point::new(43, -25),
            Point::new(0, -50),
            Point::new(-43, -25),
            Point::new(-43, 25),
        ]),
    );
    lib.insert(cb);
    check_lib("polygon_irregular", &lib);
}

#[test]
fn paths_each_cap() {
    let lib = Library::new("test", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));
    let mut cb = CellBuilder::new("paths");

    let mut p_flat = Path::new([Point::new(0, 0), Point::new(100, 0)], 10);
    p_flat.cap = PathCap::Flat;
    cb.add_shape(l, p_flat);

    let mut p_round = Path::new([Point::new(0, 30), Point::new(100, 30)], 10);
    p_round.cap = PathCap::Round;
    cb.add_shape(l, p_round);

    let mut p_ext = Path::new([Point::new(0, 60), Point::new(100, 60)], 10);
    p_ext.cap = PathCap::Extended;
    cb.add_shape(l, p_ext);

    let mut p_custom = Path::new([Point::new(0, 90), Point::new(100, 90)], 10);
    p_custom.cap = PathCap::Extended;
    p_custom.begin_ext = 5;
    p_custom.end_ext = 7;
    cb.add_shape(l, p_custom);

    lib.insert(cb);
    check_lib("paths_each_cap", &lib);
}

#[test]
fn text_basic() {
    let lib = Library::new("test", 1000);
    let l = lib.layer(LayerInfo::gds(99, 0));
    let mut cb = CellBuilder::new("labels");
    cb.add_shape(l, Text::new("HELLO", Point::new(10, 20)));
    cb.add_shape(l, Text::new("world", Point::new(30, 40)));
    lib.insert(cb);
    check_lib("text_basic", &lib);
}

#[test]
fn eight_sref_orientations() {
    let lib = Library::new("test", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));
    let mut leaf = CellBuilder::new("u");
    leaf.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(30, 10))));
    let leaf_id = lib.insert(leaf);

    let mut parent = CellBuilder::new("eight");
    let rots = [Rot4::R0, Rot4::R90, Rot4::R180, Rot4::R270];
    let mut i = 0;
    for &mirror in &[false, true] {
        for &rot in &rots {
            parent.add_instance(Instance::new(
                leaf_id,
                Trans::new(rot, mirror, Vec2::new(i * 100, 0)),
            ));
            i += 1;
        }
    }
    lib.insert(parent);
    check_lib("eight_sref_orientations", &lib);
}

#[test]
fn aref_regular() {
    let lib = Library::new("test", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));
    let mut leaf = CellBuilder::new("u");
    leaf.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let leaf_id = lib.insert(leaf);

    let mut arr = CellBuilder::new("arr");
    arr.add_instance(
        Instance::new(leaf_id, Trans::IDENTITY).with_repetition(Repetition::Regular {
            col: Vec2::new(20, 0),
            row: Vec2::new(0, 30),
            n_cols: 4,
            n_rows: 3,
        }),
    );
    lib.insert(arr);
    check_lib("aref_regular", &lib);
}

#[test]
fn aref_rotated_translated() {
    let lib = Library::new("test", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));
    let mut leaf = CellBuilder::new("u");
    leaf.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let leaf_id = lib.insert(leaf);

    let mut arr = CellBuilder::new("arr");
    arr.add_instance(
        Instance::new(leaf_id, Trans::new(Rot4::R90, false, Vec2::new(100, 200)))
            .with_repetition(Repetition::Regular {
                col: Vec2::new(15, 0),
                row: Vec2::new(0, 25),
                n_cols: 3,
                n_rows: 2,
            }),
    );
    lib.insert(arr);
    check_lib("aref_rotated_translated", &lib);
}

#[test]
fn multilevel_hierarchy() {
    let lib = Library::new("test", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));

    let mut leaf = CellBuilder::new("leaf");
    leaf.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(5, 5))));
    let leaf_id = lib.insert(leaf);

    let mut mid = CellBuilder::new("mid");
    mid.add_instance(Instance::new(leaf_id, Trans::IDENTITY));
    mid.add_instance(Instance::new(leaf_id, Trans::translate(Vec2::new(10, 0))));
    let mid_id = lib.insert(mid);

    let mut top = CellBuilder::new("top");
    top.add_instance(Instance::new(mid_id, Trans::IDENTITY));
    top.add_instance(Instance::new(
        mid_id,
        Trans::new(Rot4::R180, false, Vec2::new(100, 100)),
    ));
    top.add_instance(Instance::new(
        mid_id,
        Trans::new(Rot4::R90, true, Vec2::new(200, 0)),
    ));
    lib.insert(top);

    check_lib("multilevel_hierarchy", &lib);
}

#[test]
fn multilayer_shapes() {
    let lib = Library::new("test", 1000);
    let layers = [(1, 0), (2, 0), (2, 1), (3, 0), (10, 5)];
    let mut cb = CellBuilder::new("ml");
    for (i, (la, dt)) in layers.iter().enumerate() {
        let l = lib.layer(LayerInfo::gds(*la, *dt));
        cb.add_shape(
            l,
            Rect::new(Bbox::new(
                Point::new((i as i64) * 20, 0),
                Point::new((i as i64) * 20 + 10, 10),
            )),
        );
    }
    lib.insert(cb);
    check_lib("multilayer_shapes", &lib);
}

#[test]
fn empty_cell_and_ref_only() {
    let lib = Library::new("test", 1000);
    let _ = lib.layer(LayerInfo::gds(1, 0)); // unused, but registered
    let empty = lib.insert(CellBuilder::new("empty"));
    let mut refonly = CellBuilder::new("refonly");
    refonly.add_instance(Instance::new(empty, Trans::IDENTITY));
    lib.insert(refonly);
    check_lib("empty_cell_and_ref_only", &lib);
}

#[test]
fn polygon_with_one_hole() {
    let lib = Library::new("demo", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));
    let mut cb = CellBuilder::new("p");
    let mut p = Polygon::from_hull([
        Point::new(0, 0),
        Point::new(100, 0),
        Point::new(100, 100),
        Point::new(0, 100),
    ]);
    p.add_hole([
        Point::new(20, 20),
        Point::new(50, 20),
        Point::new(50, 50),
        Point::new(20, 50),
    ]);
    cb.add_shape(l, p);
    lib.insert(cb);
    check_lib("polygon_with_one_hole", &lib);
}

#[test]
fn polygon_with_two_holes() {
    let lib = Library::new("demo", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));
    let mut cb = CellBuilder::new("p");
    let mut p = Polygon::from_hull([
        Point::new(0, 0),
        Point::new(200, 0),
        Point::new(200, 100),
        Point::new(0, 100),
    ]);
    p.add_hole([
        Point::new(20, 20),
        Point::new(50, 20),
        Point::new(50, 50),
        Point::new(20, 50),
    ]);
    p.add_hole([
        Point::new(120, 30),
        Point::new(160, 30),
        Point::new(160, 70),
        Point::new(120, 70),
    ]);
    cb.add_shape(l, p);
    lib.insert(cb);
    check_lib("polygon_with_two_holes", &lib);
}

#[test]
fn instance_properties() {
    let lib = Library::new("demo", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));
    let mut leaf = CellBuilder::new("u");
    leaf.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let leaf_id = lib.insert(leaf);

    let mut top = CellBuilder::new("top");
    let mut inst = Instance::new(leaf_id, Trans::translate(Vec2::new(20, 20)));
    inst.properties.set("1", PropertyValue::String("hello".into()));
    inst.properties.set("42", PropertyValue::String("world".into()));
    top.add_instance(inst);
    lib.insert(top);

    check_lib("instance_properties", &lib);
}

#[test]
fn mixed_shapes_and_insts() {
    let lib = Library::new("test", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));

    let mut leaf = CellBuilder::new("dot");
    leaf.add_shape(l, Rect::new(Bbox::new(Point::new(-1, -1), Point::new(1, 1))));
    let dot = lib.insert(leaf);

    let mut grid = CellBuilder::new("grid");
    grid.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(100, 100))));
    for i in 0..4 {
        for j in 0..4 {
            grid.add_instance(Instance::new(
                dot,
                Trans::translate(Vec2::new(25 * i + 12, 25 * j + 12)),
            ));
        }
    }
    lib.insert(grid);
    check_lib("mixed_shapes_and_insts", &lib);
}
