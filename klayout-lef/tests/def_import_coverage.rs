//! DEF **import** surface tests: end-to-end `read_def_full` into [`DefDesign`] + top cell,
//! covering section combinations and edge cases that are not all represented in the
//! OpenDB JSON corpus (special nets have no `dbWire` in OpenDB, so geometry is asserted here only).

use klayout_lef::types::{
    BlockageKind, PinUse, RouteSegment, TrackDirection,
};
use klayout_lef::{read_def_full, read_lef_full};

/// Technology + cut + stacked via + one inverter — enough for placement and via-in-route tests.
const LEF_TECH_VIA_INV: &[u8] = br#"
VERSION 5.7 ;
BUSBITCHARS "[]" ;
DIVIDERCHAR "/" ;
UNITS
  DATABASE MICRONS 1000 ;
END UNITS

LAYER metal1
  TYPE ROUTING ;
  DIRECTION HORIZONTAL ;
  WIDTH 0.14 ;
  PITCH 0.28 ;
END metal1

LAYER via12
  TYPE CUT ;
  SPACING 0.15 ;
END via12

LAYER metal2
  TYPE ROUTING ;
  DIRECTION VERTICAL ;
  WIDTH 0.14 ;
  PITCH 0.28 ;
END metal2

VIA M1_M2 DEFAULT
  LAYER metal1 ;
    RECT -0.07 -0.07 0.07 0.07 ;
  LAYER via12 ;
    RECT -0.05 -0.05 0.05 0.05 ;
  LAYER metal2 ;
    RECT -0.07 -0.07 0.07 0.07 ;
END M1_M2

SITE core
  CLASS CORE ;
  SIZE 0.46 BY 1.4 ;
END core

MACRO INV
  CLASS CORE ;
  ORIGIN 0 0 ;
  SIZE 0.92 BY 1.4 ;
  SYMMETRY X Y ;
  SITE core ;
  PIN A
    DIRECTION INPUT ; USE SIGNAL ;
    PORT LAYER metal1 ; RECT 0.1 0.5 0.3 0.9 ; END
  END A
  PIN Y
    DIRECTION OUTPUT ; USE SIGNAL ;
    PORT LAYER metal1 ; RECT 0.6 0.5 0.8 0.9 ; END
  END Y
END INV

END LIBRARY
"#;

fn lib_with_inv() -> klayout_core::Library {
    read_lef_full(LEF_TECH_VIA_INV).expect("lef").library
}

#[test]
fn special_nets_parsed_with_routed_stripe_and_use() {
    let lib = lib_with_inv();
    let def = br#"
VERSION 5.7 ;
DESIGN pwr ;
UNITS DISTANCE MICRONS 1000 ;
DIEAREA ( 0 0 ) ( 5000 5000 ) ;
COMPONENTS 0 ;
END COMPONENTS
SPECIALNETS 1 ;
- VDD
  + ROUTED metal2 480 ( 0 2000 ) ( 4500 2000 )
  + USE POWER ;
END SPECIALNETS
NETS 0 ;
END NETS
END DESIGN
"#;
        let d = read_def_full(def, &lib, None).expect("def");
    assert_eq!(d.design_name, "pwr");
    let sn = &d.special_nets[0];
    assert_eq!(sn.name.as_str(), "VDD");
    assert_eq!(sn.use_, Some(PinUse::Power));
    assert!(sn.segments.iter().any(|s| matches!(
        s,
        RouteSegment::Wire { layer, width: Some(480), .. }
            if layer == "metal2"
    )));
}

#[test]
fn design_level_vias_block_parsed() {
    let lib = lib_with_inv();
    let def = br#"
VERSION 5.7 ;
DESIGN v ;
UNITS DISTANCE MICRONS 1000 ;
DIEAREA ( 0 0 ) ( 10000 10000 ) ;
VIAS 1 ;
- DV1 + VIARULE testvr + RECT metal1 ( 0 0 ) ( 100 100 ) ;
END VIAS
COMPONENTS 0 ;
END COMPONENTS
NETS 0 ;
END NETS
END DESIGN
"#;
        let d = read_def_full(def, &lib, None).expect("def");
    assert_eq!(d.vias.len(), 1);
    assert_eq!(d.vias[0].name.as_str(), "DV1");
    assert_eq!(d.vias[0].via_rule.as_ref().map(|s| s.as_str()), Some("testvr"));
}

#[test]
fn row_tracks_and_gcell_recorded() {
    let lib = lib_with_inv();
    let def = br#"
VERSION 5.7 ;
DESIGN grid ;
UNITS DISTANCE MICRONS 1000 ;
DIEAREA ( 0 0 ) ( 20000 20000 ) ;
ROW r0 core 1000 1000 N DO 3 BY 1 STEP 460 0 ;
TRACKS X 0 DO 50 STEP 200 LAYER metal1 ;
TRACKS Y 0 DO 50 STEP 200 LAYER metal2 ;
GCELLGRID X 0 DO 20 STEP 500 ;
GCELLGRID Y 0 DO 20 STEP 500 ;
COMPONENTS 0 ;
END COMPONENTS
NETS 0 ;
END NETS
END DESIGN
"#;
        let d = read_def_full(def, &lib, None).expect("def");
    assert_eq!(d.rows.len(), 1);
    assert_eq!(d.rows[0].num_x, 3);
    assert_eq!(d.tracks.len(), 2);
    assert_eq!(d.gcell_grids.len(), 2);
    assert_eq!(d.tracks[0].direction, TrackDirection::X);
    assert_eq!(d.gcell_grids[0].direction, TrackDirection::X);
}

#[test]
fn blockages_regions_groups_round_trip_metadata() {
    let lib = lib_with_inv();
    let def = br#"
VERSION 5.7 ;
DESIGN mix ;
UNITS DISTANCE MICRONS 1000 ;
DIEAREA ( 0 0 ) ( 10000 10000 ) ;
COMPONENTS 1 ;
- U0 INV + PLACED ( 2000 2000 ) N ;
END COMPONENTS
BLOCKAGES 2 ;
- LAYER metal1 RECT ( 0 0 ) ( 500 500 ) ;
- PLACEMENT RECT ( 1000 1000 ) ( 1500 1500 ) ;
END BLOCKAGES
REGIONS 1 ;
- rfence ( 0 0 ) ( 9000 9000 ) + TYPE FENCE ;
END REGIONS
GROUPS 1 ;
- g0 U0 + REGION rfence ;
END GROUPS
NETS 0 ;
END NETS
END DESIGN
"#;
        let d = read_def_full(def, &lib, None).expect("def");
    assert_eq!(d.blockages.len(), 2);
    assert!(d
        .blockages
        .iter()
        .any(|b| b.kind == BlockageKind::Placement));
    assert!(d
        .blockages
        .iter()
        .any(|b| b.kind == BlockageKind::Routing && b.layer.is_some()));
    assert_eq!(d.regions[0].kind.as_ref().map(|s| s.as_str()), Some("FENCE"));
    assert_eq!(d.groups[0].members.len(), 1);
}

#[test]
fn net_routing_star_wildcards() {
    let lib = lib_with_inv();
    let def = br#"
VERSION 5.7 ;
DESIGN star ;
UNITS DISTANCE MICRONS 1000 ;
DIEAREA ( 0 0 ) ( 10000 10000 ) ;
COMPONENTS 2 ;
- A INV + PLACED ( 1000 1000 ) N ;
- B INV + PLACED ( 8000 1000 ) N ;
END COMPONENTS
NETS 1 ;
- n0 ( A Y ) ( B A )
  + ROUTED metal1 ( 2000 1200 ) ( * 3500 ) ( 7000 * ) ;
END NETS
END DESIGN
"#;
        let d = read_def_full(def, &lib, None).expect("def");
    let net = d.nets.iter().find(|n| n.name == "n0").expect("net");
    let seg = net
        .segments
        .iter()
        .find_map(|s| match s {
            RouteSegment::Wire { points, .. } => Some(points.clone()),
            _ => None,
        })
        .expect("wire");
    assert_eq!(
        seg,
        vec![
            klayout_core::Point::new(2000, 1200),
            klayout_core::Point::new(2000, 3500),
            klayout_core::Point::new(7000, 3500),
        ]
    );
}

#[test]
fn net_routed_wire_then_via_segment_order() {
    let lib = lib_with_inv();
    let def = br#"
VERSION 5.7 ;
DESIGN viaord ;
UNITS DISTANCE MICRONS 1000 ;
DIEAREA ( 0 0 ) ( 15000 10000 ) ;
COMPONENTS 2 ;
- U1 INV + PLACED ( 1000 1000 ) N ;
- U2 INV + PLACED ( 12000 1000 ) N ;
END COMPONENTS
NETS 1 ;
- mid ( U1 Y ) ( U2 A )
  + ROUTED metal1 ( 2000 1200 ) ( 5000 1200 ) M1_M2
  + NEW metal2 ( 5000 1200 ) ( 11000 1200 ) ;
END NETS
END DESIGN
"#;
        let d = read_def_full(def, &lib, None).expect("def");
    let net = d.nets.iter().find(|n| n.name == "mid").expect("net");
    assert_eq!(net.segments.len(), 3);
    assert!(matches!(&net.segments[0], RouteSegment::Wire { .. }));
    assert!(matches!(
        &net.segments[1],
        RouteSegment::Via {
            via_name,
            at,
        } if via_name == "M1_M2" && at.x == 5000 && at.y == 1200
    ));
    assert!(matches!(&net.segments[2], RouteSegment::Wire { .. }));
}

#[test]
fn pins_section_net_pins_and_instances() {
    let lib = lib_with_inv();
    let def = br#"
VERSION 5.7 ;
DESIGN top ;
UNITS DISTANCE MICRONS 1000 ;
DIEAREA ( 0 0 ) ( 10000 10000 ) ;
COMPONENTS 1 ;
- I0 INV + PLACED ( 5000 5000 ) N ;
END COMPONENTS
PINS 1 ;
- PAD + NET pad_n + DIRECTION INPUT + USE SIGNAL
  + LAYER metal1 ( -50 -50 ) ( 50 50 ) + PLACED ( 0 5000 ) N ;
END PINS
NETS 2 ;
- pad_n ( PIN PAD ) ( I0 A ) ;
- inner ( I0 Y ) ;
END NETS
END DESIGN
"#;
        let d = read_def_full(def, &lib, None).expect("def");
    let n_pad = d.nets.iter().find(|n| n.name == "pad_n").expect("pad_n");
    assert!(n_pad.connects.iter().any(|c| c.instance.is_none() && c.pin == "PAD"));
    assert!(n_pad
        .connects
        .iter()
        .any(|c| c.instance.as_ref().map(|s| s.as_str()) == Some("I0") && c.pin == "A"));
}
