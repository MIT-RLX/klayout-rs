//! Round-trip tests: build a Library, write GDS, read it back, compare.

use klayout_core::{
    Bbox, CellBuilder, Instance, LayerInfo, Library, Path, PathCap, Point, Polygon,
    PropertyValue, Rect, Repetition, Rot4, Trans, Vec2,
};
use klayout_io::{read_gds_bytes, write_gds_bytes};

#[test]
fn empty_library_roundtrip() {
    let lib = Library::new("empty", 1000);
    let bytes = write_gds_bytes(&lib).unwrap();
    let lib2 = read_gds_bytes(&bytes).unwrap();
    assert_eq!(lib2.cell_count(), 0);
    assert_eq!(lib2.dbu(), 1000);
}

#[test]
fn single_box_roundtrip() {
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));
    let mut cb = CellBuilder::new("box");
    cb.add_shape(wg, Rect::new(Bbox::new(Point::new(0, 0), Point::new(100, 50))));
    let id = lib.insert(cb);
    let original_hash = lib.get(id).content_hash();

    let bytes = write_gds_bytes(&lib).unwrap();
    let lib2 = read_gds_bytes(&bytes).unwrap();
    assert_eq!(lib2.cell_count(), 1);

    let id2 = lib2.by_name("box").expect("cell name preserved");
    let cell2 = lib2.get(id2);
    // The Box round-trips as a Box record, so content should match.
    assert_eq!(
        cell2.content_hash(),
        original_hash,
        "content hash should be preserved across GDS round-trip"
    );
}

#[test]
fn polygon_roundtrip() {
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));
    let mut cb = CellBuilder::new("poly");
    let pts = vec![
        Point::new(0, 0),
        Point::new(100, 0),
        Point::new(100, 50),
        Point::new(50, 80),
        Point::new(0, 50),
    ];
    cb.add_shape(wg, Polygon::from_hull(pts.clone()));
    let id = lib.insert(cb);
    let original_hash = lib.get(id).content_hash();

    let bytes = write_gds_bytes(&lib).unwrap();
    let lib2 = read_gds_bytes(&bytes).unwrap();
    let id2 = lib2.by_name("poly").unwrap();
    assert_eq!(lib2.get(id2).content_hash(), original_hash);
}

#[test]
fn path_roundtrip() {
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));
    let mut cb = CellBuilder::new("path");
    let mut p = Path::new(
        [Point::new(0, 0), Point::new(100, 0), Point::new(100, 50)],
        20,
    );
    p.cap = PathCap::Round;
    cb.add_shape(wg, p);
    let id = lib.insert(cb);
    let h = lib.get(id).content_hash();

    let bytes = write_gds_bytes(&lib).unwrap();
    let lib2 = read_gds_bytes(&bytes).unwrap();
    let id2 = lib2.by_name("path").unwrap();
    assert_eq!(lib2.get(id2).content_hash(), h);
}

#[test]
fn hierarchy_roundtrip() {
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));

    // Child cell.
    let mut cb = CellBuilder::new("child");
    cb.add_shape(wg, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let child_id = lib.insert(cb);

    // Parent with two SREFs.
    let mut pb = CellBuilder::new("parent");
    pb.add_instance(Instance::new(child_id, Trans::IDENTITY));
    pb.add_instance(Instance::new(
        child_id,
        Trans::new(Rot4::R90, false, Vec2::new(50, 0)),
    ));
    let parent_id = lib.insert(pb);
    let parent_hash = lib.get(parent_id).content_hash();
    let child_hash = lib.get(child_id).content_hash();

    let bytes = write_gds_bytes(&lib).unwrap();
    let lib2 = read_gds_bytes(&bytes).unwrap();
    assert_eq!(lib2.cell_count(), 2);

    let parent_id2 = lib2.by_name("parent").unwrap();
    let child_id2 = lib2.by_name("child").unwrap();
    assert_eq!(lib2.get(parent_id2).content_hash(), parent_hash);
    assert_eq!(lib2.get(child_id2).content_hash(), child_hash);
}

#[test]
fn aref_roundtrip() {
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));

    let mut cb = CellBuilder::new("unit");
    cb.add_shape(wg, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let unit_id = lib.insert(cb);

    let mut pb = CellBuilder::new("array");
    pb.add_instance(Instance::new(unit_id, Trans::IDENTITY).with_repetition(
        Repetition::Regular {
            col: Vec2::new(20, 0),
            row: Vec2::new(0, 30),
            n_cols: 4,
            n_rows: 3,
        },
    ));
    let _arr_id = lib.insert(pb);
    let h = lib.get(_arr_id).content_hash();

    let bytes = write_gds_bytes(&lib).unwrap();
    let lib2 = read_gds_bytes(&bytes).unwrap();
    let arr_id2 = lib2.by_name("array").unwrap();
    assert_eq!(lib2.get(arr_id2).content_hash(), h);
}

#[test]
fn deterministic_output() {
    let make = || {
        let lib = Library::new("d", 1000);
        let wg = lib.layer(LayerInfo::named("WG", 1, 0));
        let mut cb = CellBuilder::new("c");
        cb.add_shape(wg, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
        let _ = lib.insert(cb);
        write_gds_bytes(&lib).unwrap()
    };
    assert_eq!(make(), make(), "writer must be deterministic");
}

#[test]
fn polygon_with_one_hole_roundtrip() {
    let lib = Library::new("test", 1000);
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
    let id = lib.insert(cb);
    let original_hash = lib.get(id).content_hash();

    let bytes = write_gds_bytes(&lib).unwrap();
    let lib2 = read_gds_bytes(&bytes).unwrap();
    let id2 = lib2.by_name("p").unwrap();
    assert_eq!(
        lib2.get(id2).content_hash(),
        original_hash,
        "polygon-with-hole content_hash should survive write→read via keyhole"
    );

    let cell2 = lib2.get(id2);
    let polys: Vec<_> = cell2
        .layers()
        .flat_map(|li| cell2.shapes_on(li).filter_map(|s| match s {
            klayout_core::Shape::Polygon(p) => Some(p),
            _ => None,
        }))
        .collect();
    assert_eq!(polys.len(), 1);
    assert_eq!(polys[0].holes.len(), 1, "one hole reconstructed from keyhole");
}

#[test]
fn polygon_with_two_holes_roundtrip() {
    let lib = Library::new("test", 1000);
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
    let id = lib.insert(cb);
    let original_hash = lib.get(id).content_hash();

    let bytes = write_gds_bytes(&lib).unwrap();
    let lib2 = read_gds_bytes(&bytes).unwrap();
    let id2 = lib2.by_name("p").unwrap();
    assert_eq!(lib2.get(id2).content_hash(), original_hash);

    let cell2 = lib2.get(id2);
    let polys: Vec<_> = cell2
        .layers()
        .flat_map(|li| cell2.shapes_on(li).filter_map(|s| match s {
            klayout_core::Shape::Polygon(p) => Some(p),
            _ => None,
        }))
        .collect();
    assert_eq!(polys.len(), 1);
    assert_eq!(polys[0].holes.len(), 2);
}

#[test]
fn instance_properties_roundtrip() {
    let lib = Library::new("test", 1000);
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

    let bytes = write_gds_bytes(&lib).unwrap();
    let lib2 = read_gds_bytes(&bytes).unwrap();
    let top2 = lib2.get(lib2.by_name("top").unwrap());
    assert_eq!(top2.instances().len(), 1);
    let inst2 = &top2.instances()[0];
    assert_eq!(
        inst2.properties.get("1").map(|v| match v {
            PropertyValue::String(s) => s.as_str(),
            _ => "",
        }),
        Some("hello")
    );
    assert_eq!(
        inst2.properties.get("42").map(|v| match v {
            PropertyValue::String(s) => s.as_str(),
            _ => "",
        }),
        Some("world")
    );
}

#[test]
fn cell_properties_roundtrip() {
    let lib = Library::new("test", 1000);
    let l = lib.layer(LayerInfo::gds(1, 0));
    let mut cb = CellBuilder::new("c");
    cb.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    cb.set_property("1", PropertyValue::String("layout_id".into()));
    cb.set_property("42", PropertyValue::String("rev_3".into()));
    lib.insert(cb);

    let bytes = write_gds_bytes(&lib).unwrap();
    let lib2 = read_gds_bytes(&bytes).unwrap();
    let cell2 = lib2.get(lib2.by_name("c").unwrap());
    let props = cell2.properties();
    assert_eq!(
        props.get("1").map(|v| match v {
            PropertyValue::String(s) => s.as_str(),
            _ => "",
        }),
        Some("layout_id")
    );
    assert_eq!(
        props.get("42").map(|v| match v {
            PropertyValue::String(s) => s.as_str(),
            _ => "",
        }),
        Some("rev_3")
    );
}

#[test]
fn unknown_records_are_skipped() {
    // Build a minimal valid GDS with an unknown record injected before ENDLIB.
    let mut bytes = write_gds_bytes(&Library::new("u", 1000)).unwrap();
    let endlib_pos = bytes.len() - 4;
    let mut spliced = bytes[..endlib_pos].to_vec();
    // Inject a 6-byte unknown record (kind 0x9999).
    spliced.extend_from_slice(&[0x00, 0x06, 0x99, 0x99, 0x00, 0x00]);
    spliced.extend_from_slice(&bytes[endlib_pos..]);
    bytes = spliced;
    let lib = read_gds_bytes(&bytes).unwrap();
    assert_eq!(lib.cell_count(), 0);
}
