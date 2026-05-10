//! OASIS round-trip tests: build a Library, write OASIS, read back, compare.

use klayout_core::{
    Bbox, CellBuilder, Instance, LayerInfo, Library, Path, Point, Polygon, Rect, Rot4, Trans,
    Vec2,
};
use klayout_io::oasis::write_oasis_bytes_with;
use klayout_io::{read_oasis_bytes, write_oasis_bytes};

#[test]
fn empty_library_roundtrip() {
    let lib = Library::new("empty", 1000);
    let bytes = write_oasis_bytes(&lib).unwrap();
    let lib2 = read_oasis_bytes(&bytes).unwrap();
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

    let bytes = write_oasis_bytes(&lib).unwrap();
    let lib2 = read_oasis_bytes(&bytes).unwrap();
    assert_eq!(lib2.cell_count(), 1);
    let id2 = lib2.by_name("box").expect("cell name preserved");
    assert_eq!(lib2.get(id2).content_hash(), original_hash);
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
    let h = lib.get(id).content_hash();

    let bytes = write_oasis_bytes(&lib).unwrap();
    let lib2 = read_oasis_bytes(&bytes).unwrap();
    let id2 = lib2.by_name("poly").unwrap();
    assert_eq!(lib2.get(id2).content_hash(), h);
}

#[test]
fn path_roundtrip() {
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));
    let mut cb = CellBuilder::new("path");
    let p = Path::new(
        [Point::new(0, 0), Point::new(100, 0), Point::new(100, 50)],
        20,
    );
    cb.add_shape(wg, p);
    lib.insert(cb);

    let bytes = write_oasis_bytes(&lib).unwrap();
    let lib2 = read_oasis_bytes(&bytes).unwrap();
    let id2 = lib2.by_name("path").unwrap();
    let cell2 = lib2.get(id2);
    let layer2 = lib2.layer(LayerInfo::gds(1, 0));
    let shapes: Vec<_> = cell2.shapes_on(layer2).collect();
    assert_eq!(shapes.len(), 1);
    if let klayout_core::Shape::Path(p) = &shapes[0] {
        assert_eq!(p.points.len(), 3);
        assert_eq!(p.width, 20);
    } else {
        panic!("expected Path");
    }
}

#[test]
fn text_roundtrip() {
    let lib = Library::new("test", 1000);
    let label = lib.layer(LayerInfo::named("LABEL", 5, 0));
    let mut cb = CellBuilder::new("text");
    let t = klayout_core::Text::new("hello", Point::new(10, 20));
    cb.add_shape(label, t);
    lib.insert(cb);

    let bytes = write_oasis_bytes(&lib).unwrap();
    let lib2 = read_oasis_bytes(&bytes).unwrap();
    let id2 = lib2.by_name("text").unwrap();
    let cell2 = lib2.get(id2);
    let li = lib2.layer(LayerInfo::gds(5, 0));
    let shapes: Vec<_> = cell2.shapes_on(li).collect();
    assert_eq!(shapes.len(), 1);
    if let klayout_core::Shape::Text(t) = &shapes[0] {
        assert_eq!(t.string.as_str(), "hello");
        assert_eq!(t.anchor, Point::new(10, 20));
    } else {
        panic!("expected Text shape");
    }
}

#[test]
fn property_records_are_tolerated() {
    // Build a minimal valid OASIS file, then splice a PROPERTY record
    // (28) with one a-string value into it just before END. The reader
    // must accept the file without erroring on PROPERTY.
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));
    let mut cb = CellBuilder::new("c");
    cb.add_shape(wg, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    lib.insert(cb);
    let mut bytes = klayout_io::oasis::write_oasis_bytes_with(&lib, usize::MAX).unwrap();

    // Find END (record code 2) and inject PROPERTY (28) before it.
    let end_pos = bytes
        .iter()
        .rposition(|&b| b == 2)
        .expect("END record present");
    // PROPERTY info byte: bits = 0b00010110 → uuuu=0001, V=0 (values present),
    // C=1 (name refnum), N=1 (no name—use modal), S=0. Then no name field
    // (N=1), 1 value (uuuu=1): type=10 (a-string), len=3, "hi!".
    // To keep payload simple we use V=0 with uuuu=1 plus N=1 (no name).
    // Layout: [28] [info=0b00010010] [value-type=10] [string-len=3] [h][i][!]
    // info bits: S=0, N=1 (bit1=1), C=0 (bit2=0), V=0 (bit3=0), UUUU=0001 (bits 4..7).
    let info: u8 = 0b0001_0010;
    let mut prop = vec![28u8, info, 10, 3, b'h', b'i', b'!'];
    // Splice prop just before END.
    let tail = bytes.split_off(end_pos);
    bytes.append(&mut prop);
    bytes.extend_from_slice(&tail);

    // Should not error.
    let lib2 = read_oasis_bytes(&bytes).unwrap();
    assert_eq!(lib2.cell_count(), 1);
}

#[test]
fn cblock_compressed_roundtrip() {
    // Force CBLOCK on every cell by setting threshold to 1.
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));
    let mut cb = CellBuilder::new("dense");
    // 200 boxes — produces a payload comfortably above any threshold,
    // and is highly redundant so deflate actually saves bytes.
    for i in 0..200i64 {
        cb.add_shape(
            wg,
            Rect::new(Bbox::new(
                Point::new(i * 10, 0),
                Point::new(i * 10 + 5, 5),
            )),
        );
    }
    let id = lib.insert(cb);
    let original_hash = lib.get(id).content_hash();

    let bytes_compressed = write_oasis_bytes_with(&lib, 1).unwrap();
    let bytes_plain = write_oasis_bytes_with(&lib, usize::MAX).unwrap();
    assert!(
        bytes_compressed.len() < bytes_plain.len(),
        "CBLOCK should reduce size on redundant input ({} >= {})",
        bytes_compressed.len(),
        bytes_plain.len()
    );

    let lib2 = read_oasis_bytes(&bytes_compressed).unwrap();
    let id2 = lib2.by_name("dense").unwrap();
    assert_eq!(lib2.get(id2).content_hash(), original_hash);
}

#[test]
fn cblock_with_multiple_cells() {
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));
    for c in 0..5i64 {
        let mut cb = CellBuilder::new(format!("c{c}"));
        for i in 0..50i64 {
            cb.add_shape(
                wg,
                Rect::new(Bbox::new(
                    Point::new(i * 10, c * 100),
                    Point::new(i * 10 + 5, c * 100 + 5),
                )),
            );
        }
        lib.insert(cb);
    }
    let bytes = write_oasis_bytes_with(&lib, 1).unwrap();
    let lib2 = read_oasis_bytes(&bytes).unwrap();
    assert_eq!(lib2.cell_count(), 5);
}

#[test]
fn hierarchy_roundtrip() {
    let lib = Library::new("test", 1000);
    let wg = lib.layer(LayerInfo::named("WG", 1, 0));

    let mut cb = CellBuilder::new("child");
    cb.add_shape(wg, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let child_id = lib.insert(cb);

    let mut pb = CellBuilder::new("parent");
    pb.add_instance(Instance::new(child_id, Trans::IDENTITY));
    pb.add_instance(Instance::new(
        child_id,
        Trans::new(Rot4::R90, false, Vec2::new(50, 0)),
    ));
    lib.insert(pb);

    let bytes = write_oasis_bytes(&lib).unwrap();
    let lib2 = read_oasis_bytes(&bytes).unwrap();
    let parent_id = lib2.by_name("parent").unwrap();
    let parent2 = lib2.get(parent_id);
    assert_eq!(parent2.instances().len(), 2);
}
