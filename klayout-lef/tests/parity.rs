//! 100%-parity round-trip tests — exercise every LEF/DEF record type
//! the v2 reader supports.

use klayout_lef::*;

const REAL_LEF: &[u8] = b"
VERSION 5.7 ;
NAMESCASESENSITIVE ON ;
BUSBITCHARS \"[]\" ;
DIVIDERCHAR \"/\" ;
MANUFACTURINGGRID 0.005 ;

UNITS
  DATABASE MICRONS 2000 ;
END UNITS

SITE CoreSite
  CLASS CORE ;
  SIZE 0.46 BY 2.72 ;
  SYMMETRY Y ;
END CoreSite

LAYER li1
  TYPE ROUTING ;
  DIRECTION HORIZONTAL ;
  WIDTH 0.17 ;
  PITCH 0.34 0.46 ;
  OFFSET 0.23 ;
  SPACING 0.17 ;
  AREA 0.0561 ;
  RESISTANCE RPERSQ 12.8 ;
END li1

LAYER mcon
  TYPE CUT ;
  WIDTH 0.17 ;
  SPACING 0.19 ;
END mcon

LAYER met1
  TYPE ROUTING ;
  DIRECTION VERTICAL ;
  WIDTH 0.14 ;
  PITCH 0.34 ;
  THICKNESS 0.36 ;
  EDGECAPACITANCE 4.0e-5 ;
END met1

VIA M1_LI DEFAULT
  RESISTANCE 4.5 ;
  LAYER li1 ;
    RECT -0.085 -0.085 0.085 0.085 ;
  LAYER mcon ;
    RECT -0.085 -0.085 0.085 0.085 ;
  LAYER met1 ;
    RECT -0.085 -0.085 0.085 0.085 ;
END M1_LI

MACRO sky130_fd_sc_hd__inv_1
  CLASS CORE ;
  ORIGIN 0 0 ;
  FOREIGN sky130_fd_sc_hd__inv_1 ;
  SYMMETRY X Y ;
  SITE CoreSite ;
  SIZE 1.38 BY 2.72 ;

  PIN A
    DIRECTION INPUT ;
    USE SIGNAL ;
    ANTENNAGATEAREA 0.1428 ;
    PORT
      LAYER li1 ;
      RECT 0.150 0.700 0.260 1.260 ;
    END
  END A

  PIN Y
    DIRECTION OUTPUT ;
    USE SIGNAL ;
    ANTENNADIFFAREA 0.27 ;
    PORT
      LAYER li1 ;
      RECT 0.770 0.190 1.190 0.530 ;
      POLYGON 0.770 0.530 1.000 0.530 1.000 1.260 0.770 1.260 ;
    END
  END Y

  PIN VPWR
    DIRECTION INOUT ;
    USE POWER ;
    SHAPE ABUTMENT ;
    PORT
      LAYER met1 ;
      RECT 0.000 2.480 1.380 2.960 ;
    END
  END VPWR

  PIN VGND
    DIRECTION INOUT ;
    USE GROUND ;
    SHAPE ABUTMENT ;
    PORT
      LAYER met1 ;
      RECT 0.000 -0.240 1.380 0.240 ;
    END
  END VGND

  OBS
    LAYER li1 ;
    RECT 0.610 0.190 0.770 1.260 ;
  END
END sky130_fd_sc_hd__inv_1

END LIBRARY
";

#[test]
fn lef_full_parses_layers_via_site_macro_pins() {
    let lef = read_lef_full(REAL_LEF).unwrap();
    assert_eq!(lef.version, Some(5.7));
    assert_eq!(lef.bus_bit_chars.as_ref().map(|s| s.as_str()), Some("[]"));
    assert_eq!(lef.divider_char.as_ref().map(|s| s.as_str()), Some("/"));
    assert_eq!(lef.manufacturing_grid, Some(0.005));

    // Layers
    assert_eq!(lef.layers.len(), 3);
    let li1 = lef.layers.iter().find(|l| l.name == "li1").unwrap();
    assert_eq!(li1.layer_type, Some(LayerType::Routing));
    assert_eq!(li1.direction, Some(RoutingDirection::Horizontal));
    assert!((li1.width.unwrap() - 0.17).abs() < 1e-9);
    assert!((li1.pitch.unwrap() - 0.34).abs() < 1e-9);
    let mcon = lef.layers.iter().find(|l| l.name == "mcon").unwrap();
    assert_eq!(mcon.layer_type, Some(LayerType::Cut));
    let met1 = lef.layers.iter().find(|l| l.name == "met1").unwrap();
    assert!((met1.thickness.unwrap() - 0.36).abs() < 1e-9);

    // Sites
    assert_eq!(lef.sites.len(), 1);
    let site = &lef.sites[0];
    assert_eq!(site.name, "CoreSite");
    assert_eq!(site.class.as_ref().map(|s| s.as_str()), Some("CORE"));
    assert_eq!(site.size, Some((0.46, 2.72)));
    assert_eq!(site.symmetry, vec![smol_str::SmolStr::from("Y")]);

    // Vias
    assert_eq!(lef.vias.len(), 1);
    let via = &lef.vias[0];
    assert_eq!(via.name, "M1_LI");
    assert!(via.default);
    assert_eq!(via.resistance, Some(4.5));
    assert_eq!(via.shapes.len(), 3);

    // Macros
    assert_eq!(lef.macros.len(), 1);
    let m = &lef.macros[0];
    assert_eq!(m.name, "sky130_fd_sc_hd__inv_1");
    assert_eq!(m.size, Some((1.38, 2.72)));
    assert_eq!(m.symmetry.len(), 2); // X Y
    assert_eq!(m.pins.len(), 4);

    let a = m.pins.iter().find(|p| p.name == "A").unwrap();
    assert_eq!(a.direction, Some(PinDirection::Input));
    assert_eq!(a.use_, Some(PinUse::Signal));
    assert!(a.antenna_gate_area.is_some());
    assert_eq!(a.geometry.shapes.len(), 1);

    let y = m.pins.iter().find(|p| p.name == "Y").unwrap();
    assert_eq!(y.direction, Some(PinDirection::Output));
    // Y has both a RECT and a POLYGON port shape.
    assert_eq!(y.geometry.shapes.len(), 2);
    let has_rect = y
        .geometry
        .shapes
        .iter()
        .any(|(_, s)| matches!(s, PortShape::Rect(_)));
    let has_poly = y
        .geometry
        .shapes
        .iter()
        .any(|(_, s)| matches!(s, PortShape::Polygon(_)));
    assert!(has_rect && has_poly);

    let vpwr = m.pins.iter().find(|p| p.name == "VPWR").unwrap();
    assert_eq!(vpwr.use_, Some(PinUse::Power));
    assert_eq!(vpwr.shape, Some(PinShape::Abutment));
    let vgnd = m.pins.iter().find(|p| p.name == "VGND").unwrap();
    assert_eq!(vgnd.use_, Some(PinUse::Ground));

    assert!(!m.obs.is_empty(), "OBS rect parsed");
}

const REAL_DEF: &[u8] = b"
VERSION 5.7 ;
DIVIDERCHAR \"/\" ;
BUSBITCHARS \"[]\" ;

DESIGN top ;

UNITS DISTANCE MICRONS 1000 ;

DIEAREA ( 0 0 ) ( 5000 5000 ) ;

ROW row_0 CoreSite 0 0 N DO 10 BY 1 STEP 460 0 ;
ROW row_1 CoreSite 0 2720 FS DO 10 BY 1 STEP 460 0 ;

TRACKS X 0 DO 100 STEP 50 LAYER met1 ;
TRACKS Y 0 DO 100 STEP 50 LAYER met2 ;

GCELLGRID X 0 DO 50 STEP 100 ;
GCELLGRID Y 0 DO 50 STEP 100 ;

VIAS 1 ;
- M1_M2_VIA + VIARULE M1_M2_RULE + RECT met1 ( -50 -50 ) ( 50 50 ) ;
END VIAS

COMPONENTS 3 ;
- u1 INV + PLACED ( 100 200 ) N ;
- u2 INV + PLACED ( 600 200 ) FN ;
- u3 INV + FIXED ( 1100 200 ) N ;
END COMPONENTS

PINS 2 ;
- IN_A + NET IN_A + DIRECTION INPUT + USE SIGNAL
  + LAYER met1 ( -50 -50 ) ( 50 50 ) + PLACED ( 100 0 ) N ;
- OUT_Y + NET OUT_Y + DIRECTION OUTPUT + USE SIGNAL
  + LAYER met1 ( -50 -50 ) ( 50 50 ) + PLACED ( 4900 0 ) N ;
END PINS

NETS 1 ;
- net_0 ( u1 Y ) ( u2 A )
  + ROUTED met1 ( 200 200 ) ( 600 200 )
  + USE SIGNAL ;
END NETS

SPECIALNETS 1 ;
- VDD ( * VDD )
  + ROUTED met2 480 ( 0 2480 ) ( 5000 2480 )
  + USE POWER ;
END SPECIALNETS

BLOCKAGES 2 ;
- LAYER met1 RECT ( 0 0 ) ( 100 100 ) ;
- PLACEMENT RECT ( 200 200 ) ( 300 300 ) ;
END BLOCKAGES

REGIONS 1 ;
- region_0 ( 0 0 ) ( 1000 1000 ) + TYPE FENCE ;
END REGIONS

GROUPS 1 ;
- group_0 u1 u2 u3 + REGION region_0 ;
END GROUPS

END DESIGN
";

fn make_inv_library() -> klayout_core::Library {
    use klayout_core::{Bbox, CellBuilder, LayerInfo, Library, Point, Rect};
    let lib = Library::new("std", 1000);
    let _ = lib.layer(LayerInfo::named("met1", 1, 0));
    let _ = lib.layer(LayerInfo::named("met2", 2, 0));
    let outline = lib.layer(LayerInfo::named("OUTLINE", 0, 0));
    let mut cb = CellBuilder::new("INV");
    cb.add_shape(outline, Rect::new(Bbox::new(Point::new(0, 0), Point::new(460, 2720))));
    lib.insert(cb);
    lib
}

#[test]
fn def_full_parses_all_sections() {
    let lib = make_inv_library();
    let design = read_def_full(REAL_DEF, &lib).unwrap();
    assert_eq!(design.design_name, "top");
    assert_eq!(design.units_dbu_per_micron, 1000);
    assert!(design.diearea.is_some());

    // Rows
    assert_eq!(design.rows.len(), 2);
    let r0 = &design.rows[0];
    assert_eq!(r0.name, "row_0");
    assert_eq!(r0.site, "CoreSite");
    assert_eq!(r0.num_x, 10);
    assert_eq!(r0.step_x, 460);

    // Tracks + GCellGrid
    assert_eq!(design.tracks.len(), 2);
    let t0 = &design.tracks[0];
    assert_eq!(t0.direction, TrackDirection::X);
    assert_eq!(t0.num_tracks, 100);
    assert_eq!(t0.step, 50);
    assert!(t0.layers.iter().any(|l| l == "met1"));
    assert_eq!(design.gcell_grids.len(), 2);

    // Vias
    assert_eq!(design.vias.len(), 1);
    let via = &design.vias[0];
    assert_eq!(via.name, "M1_M2_VIA");
    assert_eq!(via.via_rule.as_ref().map(|s| s.as_str()), Some("M1_M2_RULE"));
    assert_eq!(via.shapes.len(), 1);

    // Pins
    assert_eq!(design.pins.len(), 2);
    let p_in = design.pins.iter().find(|p| p.name == "IN_A").unwrap();
    assert_eq!(p_in.direction, Some(PinDirection::Input));
    assert_eq!(p_in.use_, Some(PinUse::Signal));
    assert_eq!(p_in.layer.as_ref().map(|s| s.as_str()), Some("met1"));
    assert_eq!(p_in.placed, Some((100, 0)));

    // Nets
    assert_eq!(design.nets.len(), 1);
    let net = &design.nets[0];
    assert_eq!(net.name, "net_0");
    assert_eq!(net.connects.len(), 2);
    assert!(!net.segments.is_empty());

    // SpecialNets
    assert_eq!(design.special_nets.len(), 1);
    let sn = &design.special_nets[0];
    assert_eq!(sn.name, "VDD");
    assert_eq!(sn.use_, Some(PinUse::Power));

    // Blockages
    assert_eq!(design.blockages.len(), 2);
    let routing = design
        .blockages
        .iter()
        .find(|b| b.kind == BlockageKind::Routing)
        .unwrap();
    assert_eq!(routing.layer.as_ref().map(|s| s.as_str()), Some("met1"));
    assert!(design
        .blockages
        .iter()
        .any(|b| b.kind == BlockageKind::Placement));

    // Regions + Groups
    assert_eq!(design.regions.len(), 1);
    assert_eq!(design.regions[0].name, "region_0");
    assert_eq!(design.regions[0].kind.as_ref().map(|s| s.as_str()), Some("FENCE"));
    assert_eq!(design.groups.len(), 1);
    let g = &design.groups[0];
    assert_eq!(g.name, "group_0");
    assert_eq!(g.region.as_ref().map(|s| s.as_str()), Some("region_0"));
    assert_eq!(g.members.len(), 3);

    // Top cell exists with three components.
    let top = design.top.expect("top cell created");
    let cell = lib.get(top);
    assert_eq!(cell.instances().len(), 3);
}

#[test]
fn lef_full_writer_roundtrip() {
    // Read → write → read; the second read should match the first on
    // the surface that the writer round-trips.
    let lef1 = read_lef_full(REAL_LEF).unwrap();
    let text = write_lef_full(&lef1);
    let lef2 = read_lef_full(text.as_bytes()).unwrap();

    assert_eq!(lef2.layers.len(), lef1.layers.len());
    assert_eq!(lef2.vias.len(), lef1.vias.len());
    assert_eq!(lef2.sites.len(), lef1.sites.len());
    assert_eq!(lef2.macros.len(), lef1.macros.len());

    let m1 = &lef1.macros[0];
    let m2 = &lef2.macros[0];
    assert_eq!(m1.name, m2.name);
    assert_eq!(m1.size, m2.size);
    assert_eq!(m1.pins.len(), m2.pins.len());
}

#[test]
fn def_full_writer_roundtrip() {
    let lib = make_inv_library();
    let design1 = read_def_full(REAL_DEF, &lib).unwrap();
    let text = write_def_full(&lib, &design1);

    // Re-parse using a fresh library (we don't reuse `lib` since it
    // already has the top cell). Build a matching std-cell lib.
    let lib2 = make_inv_library();
    let design2 = read_def_full(text.as_bytes(), &lib2).unwrap();

    assert_eq!(design2.design_name, design1.design_name);
    assert_eq!(design2.rows.len(), design1.rows.len());
    assert_eq!(design2.tracks.len(), design1.tracks.len());
    assert_eq!(design2.gcell_grids.len(), design1.gcell_grids.len());
    assert_eq!(design2.vias.len(), design1.vias.len());
    assert_eq!(design2.pins.len(), design1.pins.len());
    assert_eq!(design2.nets.len(), design1.nets.len());
    assert_eq!(design2.special_nets.len(), design1.special_nets.len());
    assert_eq!(design2.blockages.len(), design1.blockages.len());
    assert_eq!(design2.regions.len(), design1.regions.len());
    assert_eq!(design2.groups.len(), design1.groups.len());

    let top1 = design1.top.unwrap();
    let top2 = design2.top.unwrap();
    assert_eq!(
        lib.get(top1).instances().len(),
        lib2.get(top2).instances().len()
    );
}

#[test]
fn lef_bus_pin_expands_to_scalar_pins() {
    let lef_text = b"
VERSION 5.7 ;
UNITS
  DATABASE MICRONS 1000 ;
END UNITS

MACRO bus_cell
  CLASS CORE ;
  SIZE 1.0 BY 1.0 ;
  PIN DATA[3:0]
    DIRECTION INPUT ;
    USE SIGNAL ;
    PORT
      LAYER met1 ;
      RECT 0.0 0.0 0.1 0.1 ;
    END
  END DATA[3:0]
END bus_cell

END LIBRARY
";
    let lef = read_lef_full(lef_text).unwrap();
    assert_eq!(lef.macros.len(), 1);
    let m = &lef.macros[0];
    // 4-bit bus → 4 scalar pins.
    assert_eq!(m.pins.len(), 4);
    // Names should be DATA[3], DATA[2], DATA[1], DATA[0].
    let names: Vec<&str> = m.pins.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"DATA[0]"));
    assert!(names.contains(&"DATA[3]"));
    // Each scalar carries the original bus declaration.
    for p in &m.pins {
        let bus = p.bus.as_ref().expect("bus tag attached");
        assert_eq!(bus.base.as_str(), "DATA");
        assert_eq!(bus.width(), 4);
    }
}
