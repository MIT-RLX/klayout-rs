//! LEF writer — emit a `Library` (basic) or a `LefLibrary` (rich).
//!
//! [`write_lef`] emits one MACRO per cell, deriving SIZE/PORT data from
//! `klayout_core` shapes. [`write_lef_full`] consumes a [`LefLibrary`]
//! and emits the full LEF surface — VERSION + UNITS + LAYER + VIA + SITE
//! records + every MACRO with its rich PIN/OBS metadata. Round-trip
//! symmetric to `read_lef_full`.

use crate::types::{
    LayerSpec, LayerType, LefLibrary, MacroSpec, PinDirection, PinShape, PinSpec, PinUse,
    PortShape, RoutingDirection, SiteSpec, ViaSpec,
};
use klayout_core::{CellId, Library, Shape};
use std::fmt::Write;

/// Emit a basic LEF skeleton from a `klayout_core::Library`. Ports come
/// from cell port metadata; everything else is derived.
pub fn write_lef(lib: &Library, cells: &[CellId]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "VERSION 5.7 ;");
    let _ = writeln!(out, "BUSBITCHARS \"[]\" ;");
    let _ = writeln!(out, "DIVIDERCHAR \"/\" ;");
    let _ = writeln!(out);
    let _ = writeln!(out, "UNITS");
    let _ = writeln!(out, "  DATABASE MICRONS {} ;", lib.dbu());
    let _ = writeln!(out, "END UNITS");
    let _ = writeln!(out);

    for &cell_id in cells {
        write_macro(&mut out, lib, cell_id);
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "END LIBRARY");
    out
}

/// Emit a full LEF library file, including layer rules, vias, sites,
/// and macros with rich PIN/OBS metadata. Round-trip symmetric with
/// `read_lef_full`.
pub fn write_lef_full(lef: &LefLibrary) -> String {
    let mut out = String::new();
    if let Some(v) = lef.version {
        let _ = writeln!(out, "VERSION {v} ;");
    } else {
        let _ = writeln!(out, "VERSION 5.7 ;");
    }
    if let Some(b) = &lef.bus_bit_chars {
        let _ = writeln!(out, "BUSBITCHARS \"{b}\" ;");
    }
    if let Some(d) = &lef.divider_char {
        let _ = writeln!(out, "DIVIDERCHAR \"{d}\" ;");
    }
    if let Some(g) = lef.manufacturing_grid {
        let _ = writeln!(out, "MANUFACTURINGGRID {g} ;");
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "UNITS");
    let _ = writeln!(out, "  DATABASE MICRONS {} ;", lef.library.dbu());
    let _ = writeln!(out, "END UNITS");
    let _ = writeln!(out);

    for layer in &lef.layers {
        write_layer(&mut out, layer);
        let _ = writeln!(out);
    }
    for via in &lef.vias {
        write_via(&mut out, via);
        let _ = writeln!(out);
    }
    for site in &lef.sites {
        write_site(&mut out, site);
        let _ = writeln!(out);
    }
    for m in &lef.macros {
        write_macro_spec(&mut out, m);
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "END LIBRARY");
    out
}

fn write_layer(out: &mut String, l: &LayerSpec) {
    let _ = writeln!(out, "LAYER {}", l.name);
    if let Some(t) = l.layer_type {
        let s = match t {
            LayerType::Routing => "ROUTING",
            LayerType::Cut => "CUT",
            LayerType::Masterslice => "MASTERSLICE",
            LayerType::Overlap => "OVERLAP",
            LayerType::Implant => "IMPLANT",
            LayerType::Other => "OTHER",
        };
        let _ = writeln!(out, "  TYPE {s} ;");
    }
    if let Some(d) = l.direction {
        let s = match d {
            RoutingDirection::Horizontal => "HORIZONTAL",
            RoutingDirection::Vertical => "VERTICAL",
            RoutingDirection::Diag45 => "DIAG45",
            RoutingDirection::Diag135 => "DIAG135",
        };
        let _ = writeln!(out, "  DIRECTION {s} ;");
    }
    if let Some(w) = l.width {
        let _ = writeln!(out, "  WIDTH {w} ;");
    }
    if let Some(p) = l.pitch {
        let _ = writeln!(out, "  PITCH {p} ;");
    }
    if let Some(o) = l.offset {
        let _ = writeln!(out, "  OFFSET {o} ;");
    }
    for s in &l.spacing {
        let mut line = format!("  SPACING {}", s.min_spacing);
        if s.same_net {
            line.push_str(" SAMENET");
        }
        if let (Some(lo), Some(hi)) = (s.range_min, s.range_max) {
            line.push_str(&format!(" RANGE {lo} {hi}"));
        }
        line.push_str(" ;");
        let _ = writeln!(out, "{line}");
    }
    if let Some(a) = l.min_area {
        let _ = writeln!(out, "  AREA {a} ;");
    }
    if let Some(s) = l.min_step {
        let _ = writeln!(out, "  MINSTEP {s} ;");
    }
    if let Some(t) = l.thickness {
        let _ = writeln!(out, "  THICKNESS {t} ;");
    }
    if let Some(r) = l.resistance_per_sq {
        let _ = writeln!(out, "  RESISTANCE RPERSQ {r} ;");
    }
    if let Some(c) = l.capacitance_per_sq {
        let _ = writeln!(out, "  CAPACITANCE CPERSQDIST {c} ;");
    }
    if let Some(c) = l.edge_capacitance {
        let _ = writeln!(out, "  EDGECAPACITANCE {c} ;");
    }
    if let Some(f) = l.antenna_diff_area_factor {
        let _ = writeln!(out, "  ANTENNADIFFAREARATIO {f} ;");
    }
    if let Some(f) = l.antenna_metal_area_factor {
        let _ = writeln!(out, "  ANTENNAAREARATIO {f} ;");
    }
    let _ = writeln!(out, "END {}", l.name);
}

fn write_via(out: &mut String, v: &ViaSpec) {
    let mut head = format!("VIA {} ", v.name);
    if v.default {
        head.push_str("DEFAULT ");
    }
    let _ = writeln!(out, "{}", head.trim_end());
    if let Some(r) = v.resistance {
        let _ = writeln!(out, "  RESISTANCE {r} ;");
    }
    if let Some(rule) = &v.via_rule {
        let _ = writeln!(out, "  VIARULE {rule} ;");
        if let Some((cw, ch)) = v.cut_size {
            let _ = writeln!(out, "  CUTSIZE {cw} {ch} ;");
        }
        if let Some((bot, cut, top)) = &v.layers {
            let _ = writeln!(out, "  LAYERS {bot} {cut} {top} ;");
        }
    }
    for s in &v.shapes {
        let b = s.bbox;
        let _ = writeln!(out, "  LAYER {} ;", s.layer);
        let _ = writeln!(
            out,
            "    RECT {} {} {} {} ;",
            b.min.x, b.min.y, b.max.x, b.max.y
        );
    }
    let _ = writeln!(out, "END {}", v.name);
}

fn write_site(out: &mut String, s: &SiteSpec) {
    let _ = writeln!(out, "SITE {}", s.name);
    if let Some(c) = &s.class {
        let _ = writeln!(out, "  CLASS {c} ;");
    }
    if !s.symmetry.is_empty() {
        let mut line = String::from("  SYMMETRY");
        for sym in &s.symmetry {
            line.push(' ');
            line.push_str(sym.as_str());
        }
        line.push_str(" ;");
        let _ = writeln!(out, "{line}");
    }
    if let Some((w, h)) = s.size {
        let _ = writeln!(out, "  SIZE {w} BY {h} ;");
    }
    let _ = writeln!(out, "END {}", s.name);
}

fn write_macro_spec(out: &mut String, m: &MacroSpec) {
    let _ = writeln!(out, "MACRO {}", m.name);
    if let Some(c) = &m.class {
        let _ = writeln!(out, "  CLASS {c} ;");
    } else {
        let _ = writeln!(out, "  CLASS CORE ;");
    }
    if let Some((ox, oy)) = m.origin {
        let _ = writeln!(out, "  ORIGIN {ox} {oy} ;");
    }
    if let Some((fname, fx, fy)) = &m.foreign {
        let _ = writeln!(out, "  FOREIGN {fname} {fx} {fy} ;");
    }
    if let Some((w, h)) = m.size {
        let _ = writeln!(out, "  SIZE {w} BY {h} ;");
    }
    if !m.symmetry.is_empty() {
        let mut line = String::from("  SYMMETRY");
        for s in &m.symmetry {
            line.push(' ');
            line.push_str(s.as_str());
        }
        line.push_str(" ;");
        let _ = writeln!(out, "{line}");
    }
    if let Some(s) = &m.site {
        let _ = writeln!(out, "  SITE {s} ;");
    }
    for p in &m.pins {
        write_pin_spec(out, p);
    }
    if !m.obs.is_empty() {
        let _ = writeln!(out, "  OBS");
        let mut current_layer: Option<&str> = None;
        for (layer, port) in &m.obs {
            if current_layer != Some(layer.as_str()) {
                let _ = writeln!(out, "    LAYER {layer} ;");
                current_layer = Some(layer.as_str());
            }
            write_port_shape(out, port, "    ");
        }
        let _ = writeln!(out, "  END");
    }
    let _ = writeln!(out, "END {}", m.name);
}

fn write_pin_spec(out: &mut String, p: &PinSpec) {
    let _ = writeln!(out, "  PIN {}", p.name);
    if let Some(d) = p.direction {
        let s = match d {
            PinDirection::Input => "INPUT",
            PinDirection::Output => "OUTPUT",
            PinDirection::Inout => "INOUT",
            PinDirection::Feedthru => "FEEDTHRU",
        };
        let _ = writeln!(out, "    DIRECTION {s} ;");
    }
    if let Some(u) = p.use_ {
        let s = match u {
            PinUse::Signal => "SIGNAL",
            PinUse::Power => "POWER",
            PinUse::Ground => "GROUND",
            PinUse::Clock => "CLOCK",
            PinUse::Analog => "ANALOG",
            PinUse::Reset => "RESET",
            PinUse::Tieoff => "TIEOFF",
            PinUse::Scan => "SCAN",
        };
        let _ = writeln!(out, "    USE {s} ;");
    }
    if let Some(sh) = p.shape {
        let s = match sh {
            PinShape::Abutment => "ABUTMENT",
            PinShape::Ring => "RING",
            PinShape::Feedthru => "FEEDTHRU",
        };
        let _ = writeln!(out, "    SHAPE {s} ;");
    }
    if let Some(a) = p.antenna_gate_area {
        let _ = writeln!(out, "    ANTENNAGATEAREA {a} ;");
    }
    if let Some(a) = p.antenna_diff_area {
        let _ = writeln!(out, "    ANTENNADIFFAREA {a} ;");
    }
    if !p.geometry.shapes.is_empty() {
        let _ = writeln!(out, "    PORT");
        let mut current_layer: Option<&str> = None;
        for (layer, port) in &p.geometry.shapes {
            if current_layer != Some(layer.as_str()) {
                let _ = writeln!(out, "      LAYER {layer} ;");
                current_layer = Some(layer.as_str());
            }
            write_port_shape(out, port, "      ");
        }
        let _ = writeln!(out, "    END");
    }
    let _ = writeln!(out, "  END {}", p.name);
}

fn write_port_shape(out: &mut String, shape: &PortShape, indent: &str) {
    match shape {
        PortShape::Rect(b) => {
            let _ = writeln!(
                out,
                "{indent}RECT {} {} {} {} ;",
                b.min.x, b.min.y, b.max.x, b.max.y
            );
        }
        PortShape::Polygon(pts) => {
            let mut line = format!("{indent}POLYGON");
            for p in pts {
                line.push_str(&format!(" {} {}", p.x, p.y));
            }
            line.push_str(" ;");
            let _ = writeln!(out, "{line}");
        }
    }
}

// Basic-mode emitter — same as before, used by `write_lef`.
fn write_macro(out: &mut String, lib: &Library, cell_id: CellId) {
    let cell = lib.get(cell_id);
    let _ = writeln!(out, "MACRO {}", cell.name());
    let _ = writeln!(out, "  CLASS CORE ;");

    let dbu = lib.dbu() as f64;

    let outline_layer = lib.layer_by_name("OUTLINE");
    let size_bbox = outline_layer
        .and_then(|l| {
            let mut b = klayout_core::Bbox::EMPTY;
            for s in cell.shapes_on(l) {
                b = b.union(&s.bbox());
            }
            if b.is_empty() {
                None
            } else {
                Some(b)
            }
        })
        .unwrap_or_else(|| cell.local_bbox());
    if !size_bbox.is_empty() {
        let _ = writeln!(
            out,
            "  SIZE {} BY {} ;",
            (size_bbox.width() as f64) / dbu,
            (size_bbox.height() as f64) / dbu,
        );
    }

    let mut pin_n = 0usize;
    for layer in cell.layers() {
        if Some(layer) == outline_layer {
            continue;
        }
        let info = lib.layer_info(layer);
        for shape in cell.shapes_on(layer) {
            let bbox = match shape {
                Shape::Box(r) => r.bbox,
                _ => shape.bbox(),
            };
            if bbox.is_empty() {
                continue;
            }
            let port_name = cell
                .ports()
                .iter()
                .find(|p| p.layer == layer && bbox.contains(p.center))
                .map(|p| p.name.to_string())
                .unwrap_or_else(|| {
                    let n = pin_n;
                    pin_n += 1;
                    format!("pin_{n}")
                });
            let _ = writeln!(out, "  PIN {}", port_name);
            let _ = writeln!(out, "    DIRECTION INOUT ;");
            let _ = writeln!(out, "    PORT");
            let _ = writeln!(out, "      LAYER {} ;", info.name.as_str());
            let _ = writeln!(
                out,
                "      RECT {} {} {} {} ;",
                (bbox.min.x as f64) / dbu,
                (bbox.min.y as f64) / dbu,
                (bbox.max.x as f64) / dbu,
                (bbox.max.y as f64) / dbu,
            );
            let _ = writeln!(out, "    END");
            let _ = writeln!(out, "  END {}", port_name);
        }
    }

    let _ = writeln!(out, "END {}", cell.name());
}
