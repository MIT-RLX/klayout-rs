//! Antenna-check smoke tests.

use klayout_connect::{antenna_check, AntennaViolation, Device, DeviceKind};
use klayout_connect::{Net, NetId, Netlist};
use klayout_core::{Bbox, Point};
use smol_str::SmolStr;
use std::collections::HashMap;

fn mk_nl(nets: &[(&str, Bbox)]) -> Netlist {
    let mut nl = Netlist::new();
    for (name, bbox) in nets {
        let mut net = Net::new(NetId(0), *name);
        net.bbox = *bbox;
        nl.insert(net);
    }
    nl
}

fn mk_nmos(name: &str, gate_net: &str, l: f64, w: f64) -> Device {
    let mut d = Device::new(DeviceKind::Nmos, name, Bbox::EMPTY);
    d.terminals.insert(SmolStr::from("gate"), SmolStr::from(gate_net));
    d.params.insert(SmolStr::from("l"), l);
    d.params.insert(SmolStr::from("w"), w);
    d
}

#[test]
fn passes_when_metal_to_gate_ratio_below_threshold() {
    let nl = mk_nl(&[("A", Bbox::new(Point::new(0, 0), Point::new(100, 10)))]);
    let devices = vec![mk_nmos("M1", "A", 10.0, 10.0)]; // gate area 100
    let mut metal: HashMap<SmolStr, f64> = HashMap::new();
    metal.insert(SmolStr::from("A"), 1000.0); // 1000 / 100 = ratio 10
    let viols = antenna_check(&nl, &devices, &metal, 100.0);
    assert!(viols.is_empty());
}

#[test]
fn fires_when_ratio_exceeds_threshold() {
    let nl = mk_nl(&[("A", Bbox::new(Point::new(0, 0), Point::new(100, 10)))]);
    let devices = vec![mk_nmos("M1", "A", 10.0, 10.0)]; // gate area 100
    let mut metal: HashMap<SmolStr, f64> = HashMap::new();
    metal.insert(SmolStr::from("A"), 50_000.0); // ratio 500
    let viols: Vec<AntennaViolation> = antenna_check(&nl, &devices, &metal, 200.0);
    assert_eq!(viols.len(), 1);
    assert_eq!(viols[0].net.as_str(), "A");
    assert!((viols[0].ratio - 500.0).abs() < 1e-9);
}

#[test]
fn ignores_nets_with_no_connected_gates() {
    let nl = mk_nl(&[("ungated", Bbox::new(Point::new(0, 0), Point::new(100, 10)))]);
    let devices: Vec<Device> = Vec::new();
    let mut metal: HashMap<SmolStr, f64> = HashMap::new();
    metal.insert(SmolStr::from("ungated"), 1_000_000.0);
    let viols = antenna_check(&nl, &devices, &metal, 10.0);
    assert!(viols.is_empty(), "ungated nets aren't antenna-relevant");
}

#[test]
fn accumulates_gate_area_across_devices_on_same_net() {
    // Two devices share gate net "A": cumulative gate area = 100 + 100 = 200.
    // Metal/gate = 30000/200 = 150. Below threshold 200 → pass.
    let nl = mk_nl(&[("A", Bbox::new(Point::new(0, 0), Point::new(100, 10)))]);
    let devices = vec![
        mk_nmos("M1", "A", 10.0, 10.0),
        mk_nmos("M2", "A", 10.0, 10.0),
    ];
    let mut metal: HashMap<SmolStr, f64> = HashMap::new();
    metal.insert(SmolStr::from("A"), 30_000.0);
    let viols = antenna_check(&nl, &devices, &metal, 200.0);
    assert!(viols.is_empty());
    // Now bump the threshold below 150 → fires.
    let viols = antenna_check(&nl, &devices, &metal, 100.0);
    assert_eq!(viols.len(), 1);
}
