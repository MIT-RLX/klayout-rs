//! Parse DEF+LEF clock sinks (`CK`-style pin) — used downstream of CTS plumbing.

use klayout_core::{Library, Point, PropertyValue, Rot4};
use klayout_lef::{DefDesign, LefLibrary, PortShape};

fn macro_pin_centers(macro_name: &str, pin_name: &str, lef: &LefLibrary) -> Option<Point> {
    let m: &klayout_lef::MacroSpec = lef
        .macros
        .iter()
        .find(|x| x.name.as_str() == macro_name)?;
    let p = m
        .pins
        .iter()
        .find(|p| p.name.as_str() == pin_name)?;
    for (_, sh) in &p.geometry.shapes {
        if let PortShape::Rect(bb) = sh {
            return Some(Point::new(
                (bb.min.x + bb.max.x) / 2,
                (bb.min.y + bb.max.y) / 2,
            ));
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstPlace {
    pub def_name: String,
    pub master: String,
    pub at: Point,
}

fn placement_instances(place_lib: &Library, design: &DefDesign) -> Result<Vec<InstPlace>, String> {
    let Some(top_id) = design.top else {
        return Err("DEF has no top cell".into());
    };
    let top = place_lib.get(top_id);
    let mut insts = Vec::new();
    for i in top.instances() {
        if i.trans.rot != Rot4::R0 || i.trans.mirror {
            return Err("cts_downstream: non-N placements are not handled yet".into());
        }
        let def_name = i
            .properties
            .get("def_name")
            .and_then(|v| match v {
                PropertyValue::String(s) => Some(s.as_str()),
                _ => None,
            })
            .ok_or_else(|| "instance missing def_name".to_string())?;
        let master = place_lib.get(i.cell).name().as_str().to_string();
        insts.push(InstPlace {
            def_name: def_name.to_string(),
            master,
            at: Point::new(i.trans.disp.x, i.trans.disp.y),
        });
    }
    insts.sort_by(|a, b| a.def_name.cmp(&b.def_name));
    Ok(insts)
}

/// CK pin centroid in DBU for every instance pin on `clock_net` named `ck_pin`.
///
/// Fixtures use `N`-orientation flops only (`Rot4::R0`, no mirror).
pub fn ck_sink_centers_placement_north(
    lef: &LefLibrary,
    place_library: &Library,
    design: &DefDesign,
    clock_net: &str,
    ck_pin: &str,
) -> Result<Vec<(String, Point)>, String> {
    let placements = placement_instances(place_library, design)?;
    let placements_by_name: std::collections::BTreeMap<String, &InstPlace> =
        placements.iter().map(|p| (p.def_name.clone(), p)).collect();

    let net = design
        .nets
        .iter()
        .find(|n| n.name.as_str() == clock_net)
        .ok_or_else(|| format!("net {clock_net:?} not found"))?;

    let mut out: Vec<(String, Point)> = Vec::new();
    for c in &net.connects {
        if c.pin.as_str() != ck_pin {
            continue;
        }
        let Some(nm) = c.instance.clone() else {
            continue;
        };
        let inst = placements_by_name
            .get(nm.as_str())
            .ok_or_else(|| format!("unknown instance {:?}", nm.as_str()))?;
        let pin_rel = macro_pin_centers(inst.master.as_str(), ck_pin, lef).ok_or_else(|| {
            format!(
                "no RECT geometry for {} pin {} in placement LEF",
                inst.master, ck_pin
            )
        })?;
        let abs = Point::new(inst.at.x + pin_rel.x, inst.at.y + pin_rel.y);
        out.push((inst.def_name.clone(), abs));
    }
    out.sort_by(|a, b| (a.0.clone(), a.1.x, a.1.y).cmp(&(b.0.clone(), b.1.x, b.1.y)));
    Ok(out)
}
