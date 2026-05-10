//! LVS comparison tests.

use klayout_connect::{lvs_compare, Device, DeviceKind, Net, NetId, Netlist};
use klayout_core::{Bbox, Point};
use smol_str::SmolStr;

fn nl(names: &[&str]) -> Netlist {
    let mut nl = Netlist::new();
    for name in names {
        let mut net = Net::new(NetId(0), *name);
        net.bbox = Bbox::new(Point::new(0, 0), Point::new(10, 10));
        nl.insert(net);
    }
    nl
}

fn nmos(name: &str, gate: &str, source: &str, drain: &str, l: f64, w: f64) -> Device {
    let mut d = Device::new(DeviceKind::Nmos, name, Bbox::EMPTY);
    d.terminals.insert(SmolStr::from("gate"), SmolStr::from(gate));
    d.terminals.insert(SmolStr::from("source"), SmolStr::from(source));
    d.terminals.insert(SmolStr::from("drain"), SmolStr::from(drain));
    d.params.insert(SmolStr::from("l"), l);
    d.params.insert(SmolStr::from("w"), w);
    d
}

#[test]
fn identical_designs_match_cleanly() {
    let layout_nl = nl(&["VDD", "GND", "OUT", "IN"]);
    let layout_devs = vec![
        nmos("M1", "IN", "GND", "OUT", 10.0, 10.0),
        nmos("M2", "OUT", "GND", "VDD", 10.0, 10.0),
    ];
    let report = lvs_compare(&layout_nl, &layout_devs, &layout_nl, &layout_devs);
    assert!(report.is_clean(), "self-comparison should be clean: {:?}", report);
    assert_eq!(report.matched_nets.len(), 4);
    assert_eq!(report.matched_devices.len(), 2);
}

#[test]
fn missing_net_is_reported() {
    let layout = nl(&["A", "B", "C"]);
    let schem = nl(&["A", "B"]);
    let report = lvs_compare(&layout, &[], &schem, &[]);
    assert!(!report.is_clean());
    assert_eq!(report.matched_nets.len(), 2);
    assert_eq!(
        report.unmatched_layout_nets,
        vec![SmolStr::from("C")]
    );
}

#[test]
fn device_kind_mismatch_is_flagged() {
    let layout_nl = nl(&["VDD", "GND", "OUT", "IN"]);
    let layout_devs = vec![nmos("M1", "IN", "GND", "OUT", 10.0, 10.0)];

    let mut schem_dev = nmos("M1", "IN", "GND", "OUT", 10.0, 10.0);
    schem_dev.kind = DeviceKind::Pmos;
    let schem_devs = vec![schem_dev];

    let report = lvs_compare(&layout_nl, &layout_devs, &layout_nl, &schem_devs);
    assert_eq!(report.device_mismatches.len(), 1);
    assert!(report.device_mismatches[0].reason.contains("kind"));
}

#[test]
fn structural_match_handles_renamed_devices() {
    // Same circuit, different device names. Structural matching should
    // pair them up by their connection signatures.
    let nets = nl(&["VDD", "GND", "OUT", "IN"]);
    let layout_devs = vec![nmos("M1", "IN", "GND", "OUT", 10.0, 10.0)];
    let schem_devs = vec![nmos("X1", "IN", "GND", "OUT", 10.0, 10.0)];

    let report = lvs_compare(&nets, &layout_devs, &nets, &schem_devs);
    assert_eq!(
        report.matched_devices.len(),
        1,
        "structural match should pair M1 with X1: {:?}",
        report
    );
    assert!(report.unmatched_layout_devices.is_empty());
    assert!(report.unmatched_schem_devices.is_empty());
}

#[test]
fn layout_with_extra_device_is_unmatched() {
    let nets = nl(&["A", "B", "C"]);
    let layout_devs = vec![
        nmos("M1", "A", "B", "C", 10.0, 10.0),
        nmos("M2", "A", "B", "C", 20.0, 20.0),
    ];
    let schem_devs = vec![nmos("X1", "A", "B", "C", 10.0, 10.0)];
    let report = lvs_compare(&nets, &layout_devs, &nets, &schem_devs);
    // Both M1 and M2 share the same kind+terminal signature; structural
    // matching pairs ONE of them with X1 and leaves the other unmatched.
    assert_eq!(report.matched_devices.len(), 1);
    assert_eq!(report.unmatched_layout_devices.len(), 1);
}
