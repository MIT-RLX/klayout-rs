//! SPEF (Standard Parasitic Exchange Format) writer.
//!
//! IEEE 1481 text format for parasitic data. Every STA / IR-drop tool
//! consumes it: OpenSTA, OpenROAD, PrimeTime, Innovus.
//!
//! v1 emits the standard sections in detailed format (`*D_NET`):
//! * Header — `*SPEF`, `*DESIGN`, `*DATE`, `*VENDOR`, `*PROGRAM`,
//!   `*VERSION`, `*DESIGN_FLOW`, `*DIVIDER`, `*DELIMITER`, `*BUS_DELIMITER`.
//! * Units — `*T_UNIT`, `*C_UNIT`, `*R_UNIT`, `*L_UNIT`.
//! * Name map — `*NAME_MAP` mapping numeric IDs to net/instance names
//!   (compresses long names; required by some tools).
//! * Per-net `*D_NET` — net total cap, `*CONN` (connections),
//!   `*CAP` (lumped + coupling), `*RES` (per-segment resistance).
//!
//! v1 is **lumped**: each net gets one R (sum) and one ground C with
//! coupling caps to neighbors. Distributed RC (multi-segment per net)
//! is a v2 — needs polygon skeletonization. The lumped form is the
//! standard first-pass for back-annotation flows.
//!
//! A tiny reader is included for round-trip verification but only
//! parses the lumped subset we emit.

use crate::pex::NetParasitics;
use smol_str::SmolStr;
use std::collections::HashMap;
use std::fmt::Write as _;

#[derive(Clone, Debug)]
pub struct SpefHeader {
    pub design: String,
    pub date: String,
    pub vendor: String,
    pub program: String,
    pub version: String,
    pub design_flow: String,
    pub divider: char,
    pub delimiter: char,
    pub bus_delimiter: (char, char),
    /// Time unit (ns / ps).
    pub t_unit: SpefUnit,
    /// Capacitance unit (PF / FF).
    pub c_unit: SpefUnit,
    /// Resistance unit (OHM / KOHM).
    pub r_unit: SpefUnit,
    /// Inductance unit (HENRY / MH / UH).
    pub l_unit: SpefUnit,
}

impl Default for SpefHeader {
    fn default() -> Self {
        Self {
            design: "design".into(),
            date: "1970-01-01".into(),
            vendor: "klayout-rs".into(),
            program: "klayout-rs PEX".into(),
            version: "0.0".into(),
            design_flow: "EXTERNAL".into(),
            divider: '/',
            delimiter: ':',
            bus_delimiter: ('[', ']'),
            t_unit: SpefUnit::new(1.0, "NS"),
            c_unit: SpefUnit::new(1.0, "FF"),
            r_unit: SpefUnit::new(1.0, "OHM"),
            l_unit: SpefUnit::new(1.0, "HENRY"),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct SpefUnit {
    pub multiplier: f64,
    pub label: &'static str,
}

impl SpefUnit {
    pub const fn new(mult: f64, label: &'static str) -> Self {
        Self {
            multiplier: mult,
            label,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SpefNet {
    pub name: SmolStr,
    /// Lumped capacitance in `c_unit` units.
    pub total_cap: f64,
    /// Pin connections — strings of the form `inst/pin` (interpreted by
    /// the consumer; SPEF doesn't validate them).
    pub connections: Vec<SmolStr>,
    /// Coupling caps to other nets. Each entry: `(other_net_name, cap)`.
    pub coupling: Vec<(SmolStr, f64)>,
    /// Lumped resistance.
    pub resistance: f64,
}

/// Emit a SPEF file as a String. Net order is preserved; deterministic.
pub fn write_spef(header: &SpefHeader, nets: &[SpefNet]) -> String {
    let mut out = String::new();
    write_header(&mut out, header);

    // Name map: assign numeric ids 1..=N to net names so subsequent
    // *D_NET sections can refer to them as `*1`, `*2`, etc.
    let mut id_for: HashMap<SmolStr, u32> = HashMap::new();
    let _ = writeln!(out, "*NAME_MAP");
    for (i, n) in nets.iter().enumerate() {
        let id = (i as u32) + 1;
        let _ = writeln!(out, "*{} {}", id, n.name);
        id_for.insert(n.name.clone(), id);
    }
    let _ = writeln!(out);

    // Per-net detailed records.
    for n in nets {
        write_d_net(&mut out, n, &id_for);
    }
    out
}

fn write_header(out: &mut String, h: &SpefHeader) {
    let _ = writeln!(out, "*SPEF \"IEEE 1481-1998\"");
    let _ = writeln!(out, "*DESIGN \"{}\"", h.design);
    let _ = writeln!(out, "*DATE \"{}\"", h.date);
    let _ = writeln!(out, "*VENDOR \"{}\"", h.vendor);
    let _ = writeln!(out, "*PROGRAM \"{}\"", h.program);
    let _ = writeln!(out, "*VERSION \"{}\"", h.version);
    let _ = writeln!(out, "*DESIGN_FLOW \"{}\"", h.design_flow);
    let _ = writeln!(out, "*DIVIDER {}", h.divider);
    let _ = writeln!(out, "*DELIMITER {}", h.delimiter);
    let _ = writeln!(
        out,
        "*BUS_DELIMITER {} {}",
        h.bus_delimiter.0, h.bus_delimiter.1
    );
    let _ = writeln!(out, "*T_UNIT {} {}", h.t_unit.multiplier, h.t_unit.label);
    let _ = writeln!(out, "*C_UNIT {} {}", h.c_unit.multiplier, h.c_unit.label);
    let _ = writeln!(out, "*R_UNIT {} {}", h.r_unit.multiplier, h.r_unit.label);
    let _ = writeln!(out, "*L_UNIT {} {}", h.l_unit.multiplier, h.l_unit.label);
    let _ = writeln!(out);
}

fn write_d_net(out: &mut String, n: &SpefNet, ids: &HashMap<SmolStr, u32>) {
    let id = ids.get(&n.name).copied().unwrap_or(0);
    let _ = writeln!(out, "*D_NET *{} {}", id, fmt_value(n.total_cap));
    if !n.connections.is_empty() {
        let _ = writeln!(out, "*CONN");
        for c in &n.connections {
            let _ = writeln!(out, "*I {} I", c);
        }
    }
    if !n.coupling.is_empty() {
        let _ = writeln!(out, "*CAP");
        for (i, (other, cap)) in n.coupling.iter().enumerate() {
            let other_id = ids.get(other).copied().unwrap_or(0);
            let _ = writeln!(
                out,
                "{} *{}:GROUND *{}:GROUND {}",
                i + 1,
                id,
                other_id,
                fmt_value(*cap)
            );
        }
    }
    if n.resistance > 0.0 {
        let _ = writeln!(out, "*RES");
        let _ = writeln!(
            out,
            "1 *{}:1 *{}:2 {}",
            id,
            id,
            fmt_value(n.resistance)
        );
    }
    let _ = writeln!(out, "*END");
    let _ = writeln!(out);
}

fn fmt_value(v: f64) -> String {
    // SPEF allows decimal or scientific. Use compact format with up to
    // 6 significant digits — enough for back-annotation precision.
    if v == 0.0 {
        "0".into()
    } else if v.abs() >= 1e-3 && v.abs() < 1e6 {
        format!("{v:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    } else {
        format!("{v:.6e}")
    }
}

/// Convenience: build SPEF nets directly from `NetParasitics`. Each
/// net's coupling list must be name-resolved (caller decides the
/// naming scheme; PEX only knows net indices).
pub fn nets_from_pex(
    parasitics: &[NetParasitics],
    name_for: impl Fn(usize) -> SmolStr,
) -> Vec<SpefNet> {
    let mut out = Vec::with_capacity(parasitics.len());
    for p in parasitics {
        let total_cap = p.ground_cap + p.coupling.iter().map(|(_, c)| c).sum::<f64>();
        let coupling: Vec<(SmolStr, f64)> = p
            .coupling
            .iter()
            .map(|(idx, c)| (name_for(*idx), *c))
            .collect();
        out.push(SpefNet {
            name: name_for(p.net_index),
            total_cap,
            connections: Vec::new(),
            coupling,
            resistance: p.resistance,
        });
    }
    out
}

// ----- Tiny reader for round-trip tests -----

/// Minimal SPEF reader — supports the lumped subset we emit. Returns
/// (header_design_name, nets). Sufficient for round-trip verification.
pub fn read_spef(text: &str) -> Result<(String, Vec<SpefNet>), String> {
    let mut design = String::new();
    let mut name_map: HashMap<u32, SmolStr> = HashMap::new();
    let mut nets: Vec<SpefNet> = Vec::new();
    let mut current: Option<SpefNet> = None;
    let mut section: Section = Section::Header;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("*DESIGN ") {
            design = rest.trim().trim_matches('"').to_string();
            continue;
        }
        if line == "*NAME_MAP" {
            section = Section::NameMap;
            continue;
        }
        if let Some(rest) = line.strip_prefix("*D_NET") {
            if let Some(c) = current.take() {
                nets.push(c);
            }
            section = Section::DNet;
            // *D_NET *<id> <total_cap>
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() < 2 {
                return Err(format!("malformed D_NET: {line}"));
            }
            let id = parts[0]
                .strip_prefix('*')
                .ok_or("missing * on net id")?
                .parse::<u32>()
                .map_err(|e| e.to_string())?;
            let cap: f64 = parts[1].parse().map_err(|e: std::num::ParseFloatError| e.to_string())?;
            let name = name_map
                .get(&id)
                .cloned()
                .unwrap_or_else(|| SmolStr::from(format!("net_{id}")));
            current = Some(SpefNet {
                name,
                total_cap: cap,
                connections: Vec::new(),
                coupling: Vec::new(),
                resistance: 0.0,
            });
            continue;
        }
        if line == "*CONN" || line == "*CAP" || line == "*RES" {
            // section markers within *D_NET
            continue;
        }
        if line == "*END" {
            if let Some(c) = current.take() {
                nets.push(c);
            }
            continue;
        }
        match section {
            Section::NameMap => {
                if let Some(rest) = line.strip_prefix('*') {
                    let parts: Vec<&str> = rest.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(id) = parts[0].parse::<u32>() {
                            name_map.insert(id, SmolStr::from(parts[1]));
                        }
                    }
                }
            }
            Section::DNet => {
                if let Some(c) = current.as_mut() {
                    if let Some(rest) = line.strip_prefix("*I ") {
                        let parts: Vec<&str> = rest.split_whitespace().collect();
                        if !parts.is_empty() {
                            c.connections.push(SmolStr::from(parts[0]));
                        }
                    } else if line.starts_with(|ch: char| ch.is_ascii_digit()) {
                        // `<i> *<n1>:GROUND *<n2>:GROUND <cap>` (cap entry)
                        // or `<i> *<n>:1 *<n>:2 <res>` (res entry — we don't separate)
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() == 4 && parts[1].contains(":GROUND") {
                            let other = parse_net_ref(parts[2], &name_map);
                            let cap: f64 = parts[3].parse().unwrap_or(0.0);
                            c.coupling.push((other, cap));
                        } else if parts.len() == 4 {
                            let res: f64 = parts[3].parse().unwrap_or(0.0);
                            c.resistance += res;
                        }
                    }
                }
            }
            Section::Header => {}
        }
    }
    if let Some(c) = current.take() {
        nets.push(c);
    }
    Ok((design, nets))
}

#[derive(Copy, Clone, Debug)]
enum Section {
    Header,
    NameMap,
    DNet,
}

fn parse_net_ref(s: &str, name_map: &HashMap<u32, SmolStr>) -> SmolStr {
    // Accept `*<id>:GROUND` or `*<id>` or a literal name.
    let stripped = s.trim_start_matches('*');
    let id_part = stripped.split(':').next().unwrap_or(stripped);
    if let Ok(id) = id_part.parse::<u32>() {
        if let Some(name) = name_map.get(&id) {
            return name.clone();
        }
    }
    SmolStr::from(id_part)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> SpefHeader {
        SpefHeader {
            design: "test".into(),
            ..SpefHeader::default()
        }
    }

    #[test]
    fn empty_design_writes_header_only() {
        let s = write_spef(&header(), &[]);
        assert!(s.contains("*DESIGN \"test\""));
        assert!(s.contains("*NAME_MAP"));
    }

    #[test]
    fn lumped_net_round_trip() {
        let nets = vec![
            SpefNet {
                name: "VDD".into(),
                total_cap: 1.5,
                connections: vec!["u1/A".into(), "u2/B".into()],
                coupling: vec![],
                resistance: 0.5,
            },
            SpefNet {
                name: "GND".into(),
                total_cap: 0.0,
                connections: vec![],
                coupling: vec![],
                resistance: 0.0,
            },
        ];
        let s = write_spef(&header(), &nets);
        let (design, parsed) = read_spef(&s).unwrap();
        assert_eq!(design, "test");
        assert_eq!(parsed.len(), 2);
        let vdd = parsed.iter().find(|n| n.name == "VDD").unwrap();
        assert!((vdd.total_cap - 1.5).abs() < 1e-6);
        assert!((vdd.resistance - 0.5).abs() < 1e-6);
        assert_eq!(vdd.connections.len(), 2);
    }

    #[test]
    fn coupling_caps_are_paired_correctly() {
        let nets = vec![
            SpefNet {
                name: "A".into(),
                total_cap: 0.3,
                connections: vec![],
                coupling: vec![("B".into(), 0.1)],
                resistance: 0.0,
            },
            SpefNet {
                name: "B".into(),
                total_cap: 0.3,
                connections: vec![],
                coupling: vec![("A".into(), 0.1)],
                resistance: 0.0,
            },
        ];
        let s = write_spef(&header(), &nets);
        let (_, parsed) = read_spef(&s).unwrap();
        let a = parsed.iter().find(|n| n.name == "A").unwrap();
        assert_eq!(a.coupling.len(), 1);
        assert_eq!(a.coupling[0].0.as_str(), "B");
        assert!((a.coupling[0].1 - 0.1).abs() < 1e-6);
    }

    #[test]
    fn nets_from_pex_assembles_total_cap() {
        use crate::pex::NetParasitics;
        use klayout_core::Bbox;
        let p = vec![
            NetParasitics {
                net_index: 0,
                bbox: Bbox::EMPTY,
                name: None,
                resistance: 1.0,
                ground_cap: 0.5,
                coupling: vec![(1, 0.2)],
                area_dbu2: 0,
                perimeter_dbu: 0,
            },
            NetParasitics {
                net_index: 1,
                bbox: Bbox::EMPTY,
                name: None,
                resistance: 0.5,
                ground_cap: 0.4,
                coupling: vec![(0, 0.2)],
                area_dbu2: 0,
                perimeter_dbu: 0,
            },
        ];
        let nets = nets_from_pex(&p, |i| SmolStr::from(format!("n{i}")));
        assert_eq!(nets.len(), 2);
        // total_cap = ground_cap + sum(coupling) = 0.5 + 0.2 = 0.7
        assert!((nets[0].total_cap - 0.7).abs() < 1e-9);
        assert_eq!(nets[0].coupling[0].0.as_str(), "n1");
    }
}
