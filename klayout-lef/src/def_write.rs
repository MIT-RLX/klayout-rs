//! DEF writer — emit a top cell as a DEF placed-design file.
//!
//! [`write_def`] takes a `Library` + top cell and emits a minimal DEF
//! (DESIGN/UNITS/DIEAREA/COMPONENTS). [`write_def_full`] consumes a
//! [`DefDesign`] and emits the full DEF surface — ROW, TRACKS, GCELLGRID,
//! VIAS, PINS, NETS, SPECIALNETS, BLOCKAGES, REGIONS, GROUPS — round-trip
//! symmetric with `read_def_full`.

use crate::types::{
    Blockage, BlockageKind, DefDesign, DefVia, DesignPin, GcellGrid, Group, NetConnect,
    PinDirection, PinUse, Region, Row, RouteNet, RouteSegment, Track, TrackDirection,
};
use klayout_core::{CellId, Library, Rot4};
use std::fmt::Write;

pub fn write_def(lib: &Library, top: CellId) -> String {
    let cell = lib.get(top);
    let mut out = String::new();
    let _ = writeln!(out, "VERSION 5.7 ;");
    let _ = writeln!(out, "DIVIDERCHAR \"/\" ;");
    let _ = writeln!(out, "BUSBITCHARS \"[]\" ;");
    let _ = writeln!(out, "DESIGN {} ;", cell.name());
    let _ = writeln!(out, "UNITS DISTANCE MICRONS {} ;", lib.dbu());

    let bbox = cell.full_bbox(lib);
    if !bbox.is_empty() {
        let _ = writeln!(
            out,
            "DIEAREA ( {} {} ) ( {} {} ) ;",
            bbox.min.x, bbox.min.y, bbox.max.x, bbox.max.y
        );
    }
    let _ = writeln!(out);

    let count = cell.instances().len();
    let _ = writeln!(out, "COMPONENTS {} ;", count);
    for (i, inst) in cell.instances().iter().enumerate() {
        let child = lib.get(inst.cell);
        let orient = trans_to_orient(inst.trans);
        let _ = writeln!(
            out,
            "  - i{} {} + PLACED ( {} {} ) {} ;",
            i,
            child.name(),
            inst.trans.disp.x,
            inst.trans.disp.y,
            orient,
        );
    }
    let _ = writeln!(out, "END COMPONENTS");
    let _ = writeln!(out);

    let _ = writeln!(out, "END DESIGN");
    out
}

/// Emit a complete DEF file from a `DefDesign`.
pub fn write_def_full(lib: &Library, design: &DefDesign) -> String {
    let mut out = String::new();
    if let Some(v) = design.version {
        let _ = writeln!(out, "VERSION {v} ;");
    } else {
        let _ = writeln!(out, "VERSION 5.7 ;");
    }
    if let Some(d) = &design.divider_char {
        let _ = writeln!(out, "DIVIDERCHAR \"{d}\" ;");
    }
    if let Some(b) = &design.bus_bit_chars {
        let _ = writeln!(out, "BUSBITCHARS \"{b}\" ;");
    }
    if !design.design_name.is_empty() {
        let _ = writeln!(out, "DESIGN {} ;", design.design_name);
    } else if let Some(top) = design.top {
        let _ = writeln!(out, "DESIGN {} ;", lib.get(top).name());
    }
    let dbu = if design.units_dbu_per_micron != 0 {
        design.units_dbu_per_micron
    } else {
        lib.dbu()
    };
    let _ = writeln!(out, "UNITS DISTANCE MICRONS {dbu} ;");

    if let Some(b) = design.diearea {
        let _ = writeln!(
            out,
            "DIEAREA ( {} {} ) ( {} {} ) ;",
            b.min.x, b.min.y, b.max.x, b.max.y
        );
    } else if let Some(top) = design.top {
        let bbox = lib.get(top).full_bbox(lib);
        if !bbox.is_empty() {
            let _ = writeln!(
                out,
                "DIEAREA ( {} {} ) ( {} {} ) ;",
                bbox.min.x, bbox.min.y, bbox.max.x, bbox.max.y
            );
        }
    }
    let _ = writeln!(out);

    for r in &design.rows {
        write_row(&mut out, r);
    }
    if !design.rows.is_empty() {
        let _ = writeln!(out);
    }

    for t in &design.tracks {
        write_track(&mut out, t);
    }
    if !design.tracks.is_empty() {
        let _ = writeln!(out);
    }

    for g in &design.gcell_grids {
        write_gcell(&mut out, g);
    }
    if !design.gcell_grids.is_empty() {
        let _ = writeln!(out);
    }

    if !design.vias.is_empty() {
        let _ = writeln!(out, "VIAS {} ;", design.vias.len());
        for v in &design.vias {
            write_def_via(&mut out, v);
        }
        let _ = writeln!(out, "END VIAS");
        let _ = writeln!(out);
    }

    if let Some(top) = design.top {
        let cell = lib.get(top);
        let count = cell.instances().len();
        let _ = writeln!(out, "COMPONENTS {} ;", count);
        for (i, inst) in cell.instances().iter().enumerate() {
            let child = lib.get(inst.cell);
            let orient = trans_to_orient(inst.trans);
            let _ = writeln!(
                out,
                "  - i{} {} + PLACED ( {} {} ) {} ;",
                i,
                child.name(),
                inst.trans.disp.x,
                inst.trans.disp.y,
                orient,
            );
        }
        let _ = writeln!(out, "END COMPONENTS");
        let _ = writeln!(out);
    }

    if !design.pins.is_empty() {
        let _ = writeln!(out, "PINS {} ;", design.pins.len());
        for p in &design.pins {
            write_pin(&mut out, p);
        }
        let _ = writeln!(out, "END PINS");
        let _ = writeln!(out);
    }

    if !design.blockages.is_empty() {
        let _ = writeln!(out, "BLOCKAGES {} ;", design.blockages.len());
        for b in &design.blockages {
            write_blockage(&mut out, b);
        }
        let _ = writeln!(out, "END BLOCKAGES");
        let _ = writeln!(out);
    }

    if !design.regions.is_empty() {
        let _ = writeln!(out, "REGIONS {} ;", design.regions.len());
        for r in &design.regions {
            write_region(&mut out, r);
        }
        let _ = writeln!(out, "END REGIONS");
        let _ = writeln!(out);
    }

    if !design.groups.is_empty() {
        let _ = writeln!(out, "GROUPS {} ;", design.groups.len());
        for g in &design.groups {
            write_group(&mut out, g);
        }
        let _ = writeln!(out, "END GROUPS");
        let _ = writeln!(out);
    }

    if !design.nets.is_empty() {
        let _ = writeln!(out, "NETS {} ;", design.nets.len());
        for n in &design.nets {
            write_net(&mut out, n, false);
        }
        let _ = writeln!(out, "END NETS");
        let _ = writeln!(out);
    }

    if !design.special_nets.is_empty() {
        let _ = writeln!(out, "SPECIALNETS {} ;", design.special_nets.len());
        for n in &design.special_nets {
            write_net(&mut out, n, true);
        }
        let _ = writeln!(out, "END SPECIALNETS");
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "END DESIGN");
    out
}

fn write_row(out: &mut String, r: &Row) {
    let _ = writeln!(
        out,
        "ROW {} {} {} {} {} DO {} BY {} STEP {} {} ;",
        r.name,
        r.site,
        r.origin.0,
        r.origin.1,
        r.orient,
        r.num_x,
        r.num_y,
        r.step_x,
        r.step_y,
    );
}

fn write_track(out: &mut String, t: &Track) {
    let dir = match t.direction {
        TrackDirection::X => "X",
        TrackDirection::Y => "Y",
    };
    let mut line = format!(
        "TRACKS {dir} {} DO {} STEP {}",
        t.start, t.num_tracks, t.step
    );
    if !t.layers.is_empty() {
        line.push_str(" LAYER");
        for l in &t.layers {
            line.push(' ');
            line.push_str(l.as_str());
        }
    }
    line.push_str(" ;");
    let _ = writeln!(out, "{line}");
}

fn write_gcell(out: &mut String, g: &GcellGrid) {
    let dir = match g.direction {
        TrackDirection::X => "X",
        TrackDirection::Y => "Y",
    };
    let _ = writeln!(
        out,
        "GCELLGRID {dir} {} DO {} STEP {} ;",
        g.start, g.num, g.step
    );
}

fn write_def_via(out: &mut String, v: &DefVia) {
    let _ = writeln!(out, "  - {}", v.name);
    if let Some(rule) = &v.via_rule {
        let _ = writeln!(out, "    + VIARULE {rule}");
    }
    for s in &v.shapes {
        let b = s.bbox;
        let _ = writeln!(
            out,
            "    + RECT {} ( {} {} ) ( {} {} )",
            s.layer, b.min.x, b.min.y, b.max.x, b.max.y
        );
    }
    let _ = writeln!(out, "  ;");
}

fn write_pin(out: &mut String, p: &DesignPin) {
    let mut head = format!("  - {} + NET {}", p.name, p.net);
    if let Some(d) = p.direction {
        let s = match d {
            PinDirection::Input => "INPUT",
            PinDirection::Output => "OUTPUT",
            PinDirection::Inout => "INOUT",
            PinDirection::Feedthru => "FEEDTHRU",
        };
        head.push_str(&format!(" + DIRECTION {s}"));
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
        head.push_str(&format!(" + USE {s}"));
    }
    if let (Some(layer), Some(b)) = (&p.layer, p.layer_bbox) {
        head.push_str(&format!(
            " + LAYER {layer} ( {} {} ) ( {} {} )",
            b.min.x, b.min.y, b.max.x, b.max.y
        ));
    }
    if let (Some(pl), Some(or)) = (p.placed, &p.orient) {
        let kw = if p.fixed { "FIXED" } else { "PLACED" };
        head.push_str(&format!(" + {kw} ( {} {} ) {or}", pl.0, pl.1));
    }
    head.push_str(" ;");
    let _ = writeln!(out, "{head}");
}

fn write_blockage(out: &mut String, b: &Blockage) {
    match b.kind {
        BlockageKind::Placement => {
            let _ = writeln!(out, "  - PLACEMENT");
        }
        BlockageKind::Routing => {
            let layer = b.layer.as_deref().unwrap_or("M1");
            let _ = writeln!(out, "  - LAYER {layer}");
        }
    }
    if let Some(c) = &b.component {
        let _ = writeln!(out, "    + COMPONENT {c}");
    }
    for bb in &b.bboxes {
        let _ = writeln!(
            out,
            "    + RECT ( {} {} ) ( {} {} )",
            bb.min.x, bb.min.y, bb.max.x, bb.max.y
        );
    }
    let _ = writeln!(out, "  ;");
}

fn write_region(out: &mut String, r: &Region) {
    let mut line = format!("  - {}", r.name);
    for b in &r.bboxes {
        line.push_str(&format!(
            " ( {} {} ) ( {} {} )",
            b.min.x, b.min.y, b.max.x, b.max.y
        ));
    }
    if let Some(k) = &r.kind {
        line.push_str(&format!(" + TYPE {k}"));
    }
    line.push_str(" ;");
    let _ = writeln!(out, "{line}");
}

fn write_group(out: &mut String, g: &Group) {
    let mut head = format!("  - {}", g.name);
    for m in &g.members {
        head.push(' ');
        head.push_str(m.as_str());
    }
    if let Some(r) = &g.region {
        head.push_str(&format!(" + REGION {r}"));
    }
    head.push_str(" ;");
    let _ = writeln!(out, "{head}");
}

fn write_net(out: &mut String, n: &RouteNet, is_special: bool) {
    let mut line = format!("  - {}", n.name);
    for c in &n.connects {
        write_connect(&mut line, c);
    }
    if let Some(u) = n.use_ {
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
        line.push_str(&format!(" + USE {s}"));
    }
    let _ = writeln!(out, "{line}");
    for seg in &n.segments {
        write_segment(out, seg, is_special);
    }
    let _ = writeln!(out, "  ;");
}

fn write_connect(out: &mut String, c: &NetConnect) {
    if let Some(inst) = &c.instance {
        out.push_str(&format!(" ( {inst} {} )", c.pin));
    } else {
        out.push_str(&format!(" ( PIN {} )", c.pin));
    }
}

fn write_segment(out: &mut String, seg: &RouteSegment, is_special: bool) {
    match seg {
        RouteSegment::Wire { layer, points, width } => {
            let mut line = if is_special {
                let w = width.unwrap_or(0);
                format!("    + ROUTED {layer} {w}")
            } else {
                format!("    + ROUTED {layer}")
            };
            for p in points {
                line.push_str(&format!(" ( {} {} )", p.x, p.y));
            }
            let _ = writeln!(out, "{line}");
        }
        RouteSegment::Via { via_name, at } => {
            let _ = writeln!(
                out,
                "    + ROUTED ( {} {} ) {}",
                at.x, at.y, via_name
            );
        }
        RouteSegment::Rect { layer, bbox } => {
            let _ = writeln!(
                out,
                "    + RECT {layer} ( {} {} ) ( {} {} )",
                bbox.min.x, bbox.min.y, bbox.max.x, bbox.max.y
            );
        }
    }
}

fn trans_to_orient(t: klayout_core::Trans) -> &'static str {
    match (t.rot, t.mirror) {
        (Rot4::R0, false) => "N",
        (Rot4::R90, false) => "W",
        (Rot4::R180, false) => "S",
        (Rot4::R270, false) => "E",
        (Rot4::R0, true) => "FN",
        (Rot4::R90, true) => "FW",
        (Rot4::R180, true) => "FS",
        (Rot4::R270, true) => "FE",
    }
}
