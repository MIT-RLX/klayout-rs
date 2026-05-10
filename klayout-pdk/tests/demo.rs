//! End-to-end test of the `pdk!` macro: declare a PDK, build a cell using
//! typed layer indices, write+read GDS, verify content survives the round-trip.

use klayout_core::{Angle90, Bbox, CellBuilder, Instance, Point, Port, Rect, Trans, Vec2};
use klayout_io::{read_gds_bytes, write_gds_bytes};

klayout_pdk::pdk! {
    pub Demo {
        dbu: 1000,
        layers: {
            WG = (1, 0),
            CLAD = (2, 0),
            METAL1 = (10, 0),
            VIA12 = (11, 0),
            METAL2 = (12, 0),
            LABEL = (99, 0),
        }
        ports: { Optical, Electrical, Pad }
    }
}

#[test]
fn pdk_layers_register_and_dedup() {
    let lib = Demo::new_library("test");
    let pdk = Demo::register(&lib);

    // Layer indices must be valid and resolve to the right (layer, datatype).
    assert_eq!(lib.layer_info(pdk.WG).layer, 1);
    assert_eq!(lib.layer_info(pdk.CLAD).layer, 2);
    assert_eq!(lib.layer_info(pdk.METAL1).layer, 10);
    assert_eq!(lib.layer_info(pdk.LABEL).layer, 99);

    // Re-registering on the same library must yield the same indices.
    let pdk2 = Demo::register(&lib);
    assert_eq!(pdk.WG, pdk2.WG);
    assert_eq!(pdk.METAL1, pdk2.METAL1);
}

#[test]
fn port_kind_ids_are_stable_and_distinct() {
    assert_ne!(Demo::Optical, Demo::Electrical);
    assert_ne!(Demo::Optical, Demo::Pad);
    assert_ne!(Demo::Electrical, Demo::Pad);
    // ANY is reserved (id=0). None of our variants should collide with it.
    assert_ne!(Demo::Optical, klayout_core::PortKindId::ANY);
}

#[test]
fn dbu_constant_matches_library() {
    assert_eq!(Demo::DBU, 1000);
    let lib = Demo::new_library("test");
    assert_eq!(lib.dbu(), Demo::DBU);
}

#[test]
fn build_cell_with_typed_layers_and_ports() {
    let lib = Demo::new_library("demo");
    let pdk = Demo::register(&lib);

    let mut cb = CellBuilder::new("waveguide_taper");
    cb.add_shape(
        pdk.WG,
        Rect::new(Bbox::new(Point::new(0, 0), Point::new(1000, 200))),
    );
    cb.add_shape(
        pdk.CLAD,
        Rect::new(Bbox::new(Point::new(-500, -300), Point::new(1500, 500))),
    );
    cb.add_port(
        Port::new("o1", pdk.WG, Point::new(0, 100), Angle90::W, 200)
            .with_kind(Demo::Optical),
    );
    cb.add_port(
        Port::new("o2", pdk.WG, Point::new(1000, 100), Angle90::E, 200)
            .with_kind(Demo::Optical),
    );
    let id = lib.insert(cb);

    let cell = lib.get(id);
    assert_eq!(cell.ports().len(), 2);
    assert_eq!(cell.port("o1").unwrap().kind, Demo::Optical);
    assert_eq!(cell.port("o1").unwrap().layer, pdk.WG);
}

#[test]
fn pdk_cells_round_trip_through_gds() {
    let lib = Demo::new_library("demo");
    let pdk = Demo::register(&lib);

    let mut leaf = CellBuilder::new("u");
    leaf.add_shape(pdk.WG, Rect::new(Bbox::new(Point::new(0, 0), Point::new(50, 50))));
    leaf.add_shape(pdk.METAL1, Rect::new(Bbox::new(Point::new(10, 10), Point::new(40, 40))));
    let leaf_id = lib.insert(leaf);

    let mut top = CellBuilder::new("top");
    top.add_instance(Instance::new(leaf_id, Trans::IDENTITY));
    top.add_instance(Instance::new(
        leaf_id,
        Trans::translate(Vec2::new(100, 0)),
    ));
    let top_id = lib.insert(top);
    let original_hash = lib.get(top_id).content_hash();

    let bytes = write_gds_bytes(&lib).unwrap();
    let lib2 = read_gds_bytes(&bytes).unwrap();

    // The PDK can register against the freshly-read library and get the
    // same layer indices (since (1,0) and (10,0) are already in the file).
    let pdk2 = Demo::register(&lib2);
    assert_eq!(lib2.layer_info(pdk2.WG).layer, 1);
    assert_eq!(lib2.layer_info(pdk2.METAL1).layer, 10);

    // Content survives the round-trip.
    let top_id2 = lib2.by_name("top").unwrap();
    assert_eq!(lib2.get(top_id2).content_hash(), original_hash);
}
