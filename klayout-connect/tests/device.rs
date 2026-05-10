//! Device extraction tests — synthetic NMOS layouts.

use klayout_connect::{extract_mos, DeviceKind, MosLayers};
use klayout_core::{Bbox, CellBuilder, LayerInfo, Library, Point, Rect, Text};

fn lib() -> Library {
    Library::new("t", 1000)
}

#[test]
fn single_nmos_extracted() {
    let lib = lib();
    let poly = lib.layer(LayerInfo::gds(20, 0));
    let diff = lib.layer(LayerInfo::gds(30, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));

    let mut cb = CellBuilder::new("nmos");
    cb.add_shape(diff, Rect::new(Bbox::new(Point::new(0, 0), Point::new(100, 30))));
    cb.add_shape(poly, Rect::new(Bbox::new(Point::new(40, -10), Point::new(50, 40))));
    cb.add_shape(lbl, Text::new("G", Point::new(45, -5)));
    cb.add_shape(lbl, Text::new("S", Point::new(20, 15)));
    cb.add_shape(lbl, Text::new("D", Point::new(80, 15)));
    let cell_id = lib.insert(cb);

    let layers = MosLayers {
        poly,
        diff,
        nwell: None,
        label: lbl,
    };
    let devices = extract_mos(&lib, cell_id, layers);
    assert_eq!(devices.len(), 1);
    let d = &devices[0];
    assert_eq!(d.kind, DeviceKind::Nmos);
    assert_eq!(d.params.get("l").copied(), Some(10.0));
    assert_eq!(d.params.get("w").copied(), Some(30.0));
    assert_eq!(d.terminals.get("gate").map(|s| s.as_str()), Some("G"));
    // Source/drain assignment is order-dependent (whichever piece is found
    // first becomes "source"); the IMPORTANT thing is both nets are present.
    let s = d.terminals.get("source").map(|s| s.as_str());
    let dn = d.terminals.get("drain").map(|s| s.as_str());
    assert!(matches!(
        (s, dn),
        (Some("S"), Some("D")) | (Some("D"), Some("S"))
    ));
}

#[test]
fn pmos_distinguished_via_nwell() {
    let lib = lib();
    let poly = lib.layer(LayerInfo::gds(20, 0));
    let diff = lib.layer(LayerInfo::gds(30, 0));
    let nwell = lib.layer(LayerInfo::gds(40, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));

    let mut cb = CellBuilder::new("pmos");
    cb.add_shape(diff, Rect::new(Bbox::new(Point::new(0, 0), Point::new(100, 30))));
    cb.add_shape(poly, Rect::new(Bbox::new(Point::new(40, -10), Point::new(50, 40))));
    cb.add_shape(nwell, Rect::new(Bbox::new(Point::new(-50, -50), Point::new(150, 80))));
    let cell_id = lib.insert(cb);

    let layers = MosLayers {
        poly,
        diff,
        nwell: Some(nwell),
        label: lbl,
    };
    let devices = extract_mos(&lib, cell_id, layers);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].kind, DeviceKind::Pmos);
}

#[test]
fn no_devices_when_poly_does_not_cross_diff() {
    let lib = lib();
    let poly = lib.layer(LayerInfo::gds(20, 0));
    let diff = lib.layer(LayerInfo::gds(30, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));

    let mut cb = CellBuilder::new("no_mos");
    cb.add_shape(diff, Rect::new(Bbox::new(Point::new(0, 0), Point::new(100, 30))));
    cb.add_shape(poly, Rect::new(Bbox::new(Point::new(200, 0), Point::new(210, 30))));
    let cell_id = lib.insert(cb);

    let layers = MosLayers { poly, diff, nwell: None, label: lbl };
    let devices = extract_mos(&lib, cell_id, layers);
    assert_eq!(devices.len(), 0);
}

#[test]
fn two_separate_devices_extracted() {
    let lib = lib();
    let poly = lib.layer(LayerInfo::gds(20, 0));
    let diff = lib.layer(LayerInfo::gds(30, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));

    let mut cb = CellBuilder::new("two_mos");
    cb.add_shape(diff, Rect::new(Bbox::new(Point::new(0, 0), Point::new(100, 30))));
    cb.add_shape(diff, Rect::new(Bbox::new(Point::new(500, 0), Point::new(600, 30))));
    cb.add_shape(poly, Rect::new(Bbox::new(Point::new(40, -10), Point::new(50, 40))));
    cb.add_shape(poly, Rect::new(Bbox::new(Point::new(540, -10), Point::new(550, 40))));
    let cell_id = lib.insert(cb);

    let layers = MosLayers { poly, diff, nwell: None, label: lbl };
    let devices = extract_mos(&lib, cell_id, layers);
    assert_eq!(devices.len(), 2);
}
