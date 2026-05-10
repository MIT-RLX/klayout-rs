//! CDL writer tests — verify the netlist round-trips into a valid
//! .SUBCKT block with pin and net annotations.

use klayout_connect::extract_flat;
use klayout_core::{Bbox, CellBuilder, LayerInfo, Library, Point, Rect, Text};

fn lib() -> Library {
    Library::new("t", 1000)
}

#[test]
fn cdl_emits_subckt_header() {
    let lib = lib();
    let m1 = lib.layer(LayerInfo::gds(10, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));
    let mut cb = CellBuilder::new("top");
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(50, 0), Point::new(60, 10))));
    cb.add_shape(lbl, Text::new("VDD", Point::new(5, 5)));
    cb.add_shape(lbl, Text::new("GND", Point::new(55, 5)));
    let id = lib.insert(cb);
    let nl = extract_flat(&lib, id, m1, lbl);

    let cdl = nl.to_cdl("TOP", &["VDD", "GND"]);
    assert!(cdl.contains(".SUBCKT TOP VDD GND"));
    assert!(cdl.contains(".ENDS TOP"));
    assert!(cdl.contains("VDD: bbox="));
    assert!(cdl.contains("GND: bbox="));
}

#[test]
fn cdl_distinguishes_pins_from_internal_nets() {
    let lib = lib();
    let m1 = lib.layer(LayerInfo::gds(10, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));
    let mut cb = CellBuilder::new("top");
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(50, 0), Point::new(60, 10))));
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(100, 0), Point::new(110, 10))));
    cb.add_shape(lbl, Text::new("VDD", Point::new(5, 5)));
    cb.add_shape(lbl, Text::new("internal", Point::new(55, 5)));
    cb.add_shape(lbl, Text::new("GND", Point::new(105, 5)));
    let id = lib.insert(cb);
    let nl = extract_flat(&lib, id, m1, lbl);

    let cdl = nl.to_cdl("TOP", &["VDD", "GND"]);
    // Pin nets get "*  pin"; internal nets get "*  net".
    assert!(cdl.contains("*  pin VDD"));
    assert!(cdl.contains("*  pin GND"));
    assert!(cdl.contains("*  net internal"));
}

#[test]
fn cdl_documents_device_placeholder() {
    let lib = lib();
    let m1 = lib.layer(LayerInfo::gds(10, 0));
    let lbl = lib.layer(LayerInfo::gds(99, 0));
    let mut cb = CellBuilder::new("c");
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
    let id = lib.insert(cb);
    let nl = extract_flat(&lib, id, m1, lbl);

    let cdl = nl.to_cdl("C", &[]);
    assert!(cdl.contains("devices:"));
}
