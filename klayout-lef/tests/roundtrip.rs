//! LEF / DEF round-trip tests.
//!
//! These verify that:
//! 1. A LEF file we write can be read back into a Library that has the
//!    same macros with the same pin geometry.
//! 2. A DEF file we write referencing a LEF cell library can be read back
//!    into a Library with the same instances and placements.
//! 3. A hand-crafted "real-shaped" LEF/DEF parses without error.

use klayout_core::{Bbox, CellBuilder, Instance, LayerInfo, Library, Point, Rect, Trans, Vec2};
use klayout_lef::{read_def, read_lef, write_def, write_lef};

fn make_lib_with_inv() -> Library {
    let lib = Library::new("std", 1000);
    let m1 = lib.layer(LayerInfo::named("M1", 1, 0));
    let outline = lib.layer(LayerInfo::named("OUTLINE", 0, 0));

    let mut cb = CellBuilder::new("INV");
    // 460 × 2720 nm cell at 1000 DBU/μm = 0.46 × 2.72 μm.
    cb.add_shape(outline, Rect::new(Bbox::new(Point::new(0, 0), Point::new(460, 2720))));
    // Input pin at lower-left.
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(100, 500), Point::new(300, 1000))));
    // Output pin at upper-left.
    cb.add_shape(m1, Rect::new(Bbox::new(Point::new(100, 1500), Point::new(300, 2000))));
    lib.insert(cb);
    lib
}

#[test]
fn lef_roundtrip_preserves_macro_size() {
    let lib = make_lib_with_inv();
    let inv_id = lib.by_name("INV").unwrap();
    let lef_text = write_lef(&lib, &[inv_id]);
    assert!(lef_text.contains("MACRO INV"));
    assert!(lef_text.contains("END INV"));
    assert!(lef_text.contains("SIZE 0.46 BY 2.72 ;"));

    let lib2 = read_lef(lef_text.as_bytes()).unwrap();
    let inv2_id = lib2.by_name("INV").expect("INV macro round-tripped");
    let inv2 = lib2.get(inv2_id);
    // OUTLINE rect should be present at the same dimensions.
    let outline = lib2.layer_by_name("OUTLINE").unwrap();
    let outline_shapes: Vec<_> = inv2.shapes_on(outline).collect();
    assert_eq!(outline_shapes.len(), 1);
    let bbox = outline_shapes[0].bbox();
    assert_eq!(bbox.width(), 460);
    assert_eq!(bbox.height(), 2720);
}

#[test]
fn lef_roundtrip_preserves_pin_count_and_layer() {
    let lib = make_lib_with_inv();
    let inv_id = lib.by_name("INV").unwrap();
    let lef_text = write_lef(&lib, &[inv_id]);
    assert!(lef_text.contains("LAYER M1 ;"));
    assert!(lef_text.contains("PIN pin_0"));
    assert!(lef_text.contains("PIN pin_1"));

    let lib2 = read_lef(lef_text.as_bytes()).unwrap();
    let inv2_id = lib2.by_name("INV").unwrap();
    let inv2 = lib2.get(inv2_id);
    // Two pin rectangles on the M1 layer.
    let m1 = lib2.layer_by_name("M1").unwrap();
    assert_eq!(inv2.shapes_on(m1).count(), 2);
}

#[test]
fn def_roundtrip_preserves_components() {
    let lib = make_lib_with_inv();
    let inv_id = lib.by_name("INV").unwrap();

    // Build a top cell with three instances of INV.
    let mut top = CellBuilder::new("top");
    top.add_instance(Instance::new(inv_id, Trans::IDENTITY));
    top.add_instance(Instance::new(inv_id, Trans::translate(Vec2::new(500, 0))));
    top.add_instance(Instance::new(inv_id, Trans::translate(Vec2::new(1000, 0))));
    let top_id = lib.insert(top);

    let def_text = write_def(&lib, top_id);
    assert!(def_text.contains("DESIGN top ;"));
    assert!(def_text.contains("COMPONENTS 3 ;"));
    assert!(def_text.contains("END COMPONENTS"));
    assert!(def_text.contains("INV"));
    assert!(def_text.contains("PLACED"));

    // Read back into a fresh library that has the same INV cell.
    let lib2 = make_lib_with_inv();
    let top2_id = read_def(def_text.as_bytes(), &lib2).unwrap();
    let top2 = lib2.get(top2_id);
    assert_eq!(top2.instances().len(), 3);
    // Placements preserved.
    let positions: Vec<(i64, i64)> =
        top2.instances().iter().map(|i| (i.trans.disp.x, i.trans.disp.y)).collect();
    assert!(positions.contains(&(0, 0)));
    assert!(positions.contains(&(500, 0)));
    assert!(positions.contains(&(1000, 0)));
}

#[test]
fn def_orientation_preserved() {
    let lib = make_lib_with_inv();
    let inv_id = lib.by_name("INV").unwrap();
    let mut top = CellBuilder::new("rot");
    top.add_instance(Instance::new(
        inv_id,
        Trans::new(klayout_core::Rot4::R90, false, Vec2::new(100, 100)),
    ));
    top.add_instance(Instance::new(
        inv_id,
        Trans::new(klayout_core::Rot4::R0, true, Vec2::new(200, 200)),
    ));
    let top_id = lib.insert(top);

    let def_text = write_def(&lib, top_id);
    assert!(def_text.contains(" W ;"), "R90 should serialize as W");
    assert!(def_text.contains(" FN ;"), "R0+mirror should serialize as FN");

    let lib2 = make_lib_with_inv();
    let top2_id = read_def(def_text.as_bytes(), &lib2).unwrap();
    let top2 = lib2.get(top2_id);
    assert_eq!(top2.instances().len(), 2);
    let rots: Vec<_> = top2.instances().iter().map(|i| (i.trans.rot, i.trans.mirror)).collect();
    assert!(rots.contains(&(klayout_core::Rot4::R90, false)));
    assert!(rots.contains(&(klayout_core::Rot4::R0, true)));
}

#[test]
fn lef_with_real_shaped_input_parses() {
    // A snippet hand-written in LEF idiom. Verifies the parser handles
    // lots of skip-able statements without choking.
    let lef = b"
VERSION 5.7 ;
NAMESCASESENSITIVE ON ;
BUSBITCHARS \"[]\" ;
DIVIDERCHAR \"/\" ;

UNITS
  DATABASE MICRONS 2000 ;
END UNITS

SITE CoreSite
  CLASS CORE ;
  SIZE 0.46 BY 2.72 ;
END CoreSite

LAYER M1
  TYPE ROUTING ;
  WIDTH 0.07 ;
END M1

MACRO BUF
  CLASS CORE ;
  ORIGIN 0 0 ;
  SIZE 0.69 BY 2.72 ;
  SYMMETRY X Y ;
  PIN A
    DIRECTION INPUT ;
    USE SIGNAL ;
    PORT
      LAYER M1 ;
      RECT 0.1 0.5 0.3 1.0 ;
    END
  END A
  PIN Y
    DIRECTION OUTPUT ;
    PORT
      LAYER M1 ;
      RECT 0.1 1.5 0.3 2.0 ;
    END
  END Y
END BUF

END LIBRARY
";
    let lib = read_lef(lef).unwrap();
    let buf = lib.by_name("BUF").expect("BUF macro");
    assert_eq!(lib.get(buf).name().as_str(), "BUF");
}

#[test]
fn def_with_real_shaped_input_parses() {
    let stdcell = make_lib_with_inv();
    let def = b"
VERSION 5.7 ;
DIVIDERCHAR \"/\" ;
BUSBITCHARS \"[]\" ;

DESIGN top ;

UNITS DISTANCE MICRONS 1000 ;

DIEAREA ( 0 0 ) ( 5000 5000 ) ;

ROW row_0 CoreSite 0 0 N DO 10 BY 1 STEP 460 0 ;

TRACKS X 0 DO 100 STEP 50 LAYER M1 ;

COMPONENTS 2 ;
- u1 INV + PLACED ( 100 200 ) N ;
- u2 INV + FIXED ( 600 200 ) FN ;
END COMPONENTS

PINS 1 ;
- VDD + NET VDD + DIRECTION INOUT + USE POWER + PLACED ( 0 2500 ) N ;
END PINS

END DESIGN
";
    let top_id = read_def(def, &stdcell).unwrap();
    let cell = stdcell.get(top_id);
    assert_eq!(cell.instances().len(), 2);
}
