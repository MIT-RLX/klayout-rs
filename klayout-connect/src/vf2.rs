//! VF2 graph-isomorphism matcher for LVS.
//!
//! Models the layout/schematic netlists as bipartite (device, net)
//! graphs with terminal-labeled edges. Devices match only when their
//! kind matches; nets are unlabeled. The matcher returns either a full
//! one-to-one device pairing (with the corresponding net pairing
//! implied by terminal labels) or `None` when the two graphs are not
//! isomorphic on the device + connectivity surface.
//!
//! This is the simplified VF2 — no T1/T2 terminal sets, no lookahead.
//! It's sufficient for designs where isomorphism is determined by
//! connectivity alone (typical for unnamed LVS where every device's
//! kind + neighbors uniquely classifies it). For pathological cases
//! the simplified search may explore more states than canonical VF2.

use crate::device::Device;
#[cfg(test)]
use crate::device::DeviceKind;
use smol_str::SmolStr;
use std::collections::HashMap;

/// Successful match — `dev_pairs[i] = (layout_dev_name, schem_dev_name)`,
/// `net_pairs` similarly.
#[derive(Default, Clone, Debug)]
pub struct Vf2Match {
    pub dev_pairs: Vec<(SmolStr, SmolStr)>,
    pub net_pairs: Vec<(SmolStr, SmolStr)>,
}

/// Run VF2 between two device lists. Returns `Some(Vf2Match)` if the
/// layout-side bipartite graph is isomorphic to the schematic-side one.
pub fn vf2_match(layout_devs: &[Device], schem_devs: &[Device]) -> Option<Vf2Match> {
    if layout_devs.len() != schem_devs.len() {
        return None;
    }
    let mut state = State::default();
    if recurse(layout_devs, schem_devs, &mut state) {
        let mut m = Vf2Match::default();
        for (l, s) in &state.dev_l_to_s {
            m.dev_pairs.push((l.clone(), s.clone()));
        }
        for (l, s) in &state.net_l_to_s {
            m.net_pairs.push((l.clone(), s.clone()));
        }
        m.dev_pairs.sort();
        m.net_pairs.sort();
        Some(m)
    } else {
        None
    }
}

#[derive(Default, Clone)]
struct State {
    dev_l_to_s: HashMap<SmolStr, SmolStr>,
    dev_s_to_l: HashMap<SmolStr, SmolStr>,
    net_l_to_s: HashMap<SmolStr, SmolStr>,
    net_s_to_l: HashMap<SmolStr, SmolStr>,
}

fn recurse(layout: &[Device], schem: &[Device], state: &mut State) -> bool {
    // Pick the next unmatched layout device. Tie-break by max number of
    // already-matched terminals — the most-constrained device first
    // prunes the search aggressively.
    let next = layout
        .iter()
        .filter(|d| !state.dev_l_to_s.contains_key(&d.name))
        .max_by_key(|d| matched_terminal_count(d, &state.net_l_to_s));
    let Some(d_l) = next else {
        return true; // All matched.
    };

    let candidates: Vec<&Device> = schem
        .iter()
        .filter(|d| {
            d.kind == d_l.kind && !state.dev_s_to_l.contains_key(&d.name)
        })
        .collect();

    for d_s in candidates {
        if let Some(undo) = propose_pair(d_l, d_s, state) {
            // Commit — this also extends net mappings for any
            // newly-paired terminals.
            if recurse(layout, schem, state) {
                return true;
            }
            undo.apply(state);
        }
    }
    false
}

/// Try to pair `d_l` with `d_s`. Returns an `Undo` that reverts the
/// state on backtrack, or `None` if the pairing is inconsistent.
fn propose_pair(d_l: &Device, d_s: &Device, state: &mut State) -> Option<Undo> {
    if d_l.kind != d_s.kind {
        return None;
    }
    // Both devices must have the same set of terminal names.
    if d_l.terminals.len() != d_s.terminals.len() {
        return None;
    }
    let mut undo = Undo::default();
    let mut tentative_net_pairs: Vec<(SmolStr, SmolStr)> = Vec::new();

    for (term, n_l) in &d_l.terminals {
        let n_s = d_s.terminals.get(term)?;
        // Existing mapping consistency.
        if let Some(mapped) = state.net_l_to_s.get(n_l) {
            if mapped != n_s {
                return None;
            }
        } else if let Some(mapped_back) = state.net_s_to_l.get(n_s) {
            if mapped_back != n_l {
                return None;
            }
        } else {
            // Both unmatched — pair them tentatively. Watch for the
            // same n_l appearing twice in this device's terminals
            // mapping to two different n_s values.
            if let Some((_, prev_s)) = tentative_net_pairs.iter().find(|(l, _)| l == n_l) {
                if prev_s != n_s {
                    return None;
                }
            } else if let Some((prev_l, _)) =
                tentative_net_pairs.iter().find(|(_, s)| s == n_s)
            {
                if prev_l != n_l {
                    return None;
                }
            } else {
                tentative_net_pairs.push((n_l.clone(), n_s.clone()));
            }
        }
    }

    // Commit device pair.
    state.dev_l_to_s.insert(d_l.name.clone(), d_s.name.clone());
    state.dev_s_to_l.insert(d_s.name.clone(), d_l.name.clone());
    undo.devs.push((d_l.name.clone(), d_s.name.clone()));
    // Commit net pairs.
    for (n_l, n_s) in tentative_net_pairs {
        state.net_l_to_s.insert(n_l.clone(), n_s.clone());
        state.net_s_to_l.insert(n_s.clone(), n_l.clone());
        undo.nets.push((n_l, n_s));
    }
    Some(undo)
}

#[derive(Default)]
struct Undo {
    devs: Vec<(SmolStr, SmolStr)>,
    nets: Vec<(SmolStr, SmolStr)>,
}

impl Undo {
    fn apply(&self, state: &mut State) {
        for (l, s) in &self.devs {
            state.dev_l_to_s.remove(l);
            state.dev_s_to_l.remove(s);
        }
        for (l, s) in &self.nets {
            state.net_l_to_s.remove(l);
            state.net_s_to_l.remove(s);
        }
    }
}

fn matched_terminal_count(d: &Device, net_l_to_s: &HashMap<SmolStr, SmolStr>) -> usize {
    d.terminals
        .values()
        .filter(|n| net_l_to_s.contains_key(*n))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mos(name: &str, kind: DeviceKind, g: &str, s: &str, d: &str) -> Device {
        let mut dev = Device::new(kind, name, klayout_core::Bbox::EMPTY);
        dev.terminals.insert("gate".into(), g.into());
        dev.terminals.insert("source".into(), s.into());
        dev.terminals.insert("drain".into(), d.into());
        dev
    }

    #[test]
    fn isomorphic_inverter() {
        // Inverter: NMOS(in→out, vss, vss), PMOS(in→out, vdd, vdd).
        let layout = vec![
            mos("nm0", DeviceKind::Nmos, "i", "vss", "o"),
            mos("pm0", DeviceKind::Pmos, "i", "vdd", "o"),
        ];
        let schem = vec![
            mos("MN", DeviceKind::Nmos, "A", "GND", "Y"),
            mos("MP", DeviceKind::Pmos, "A", "VDD", "Y"),
        ];
        let m = vf2_match(&layout, &schem).expect("isomorphic");
        assert_eq!(m.dev_pairs.len(), 2);
        // Net pairing is implied by terminals.
        let pairs: HashMap<SmolStr, SmolStr> = m.net_pairs.into_iter().collect();
        assert_eq!(pairs.get("i").map(|s| s.as_str()), Some("A"));
        assert_eq!(pairs.get("o").map(|s| s.as_str()), Some("Y"));
    }

    #[test]
    fn nonisomorphic_kind_mismatch() {
        let layout = vec![mos("a", DeviceKind::Nmos, "g", "s", "d")];
        let schem = vec![mos("a", DeviceKind::Pmos, "g", "s", "d")];
        assert!(vf2_match(&layout, &schem).is_none());
    }

    #[test]
    fn nonisomorphic_count_mismatch() {
        let layout = vec![mos("a", DeviceKind::Nmos, "g", "s", "d")];
        let schem: Vec<Device> = vec![];
        assert!(vf2_match(&layout, &schem).is_none());
    }

    #[test]
    fn isomorphic_two_inverter_chain() {
        // Two cascaded inverters — verifies that VF2 follows
        // connectivity, not names.
        let layout = vec![
            mos("a0", DeviceKind::Nmos, "in", "vss", "mid"),
            mos("a1", DeviceKind::Pmos, "in", "vdd", "mid"),
            mos("a2", DeviceKind::Nmos, "mid", "vss", "out"),
            mos("a3", DeviceKind::Pmos, "mid", "vdd", "out"),
        ];
        let schem = vec![
            mos("X", DeviceKind::Nmos, "I", "GND", "M"),
            mos("Y", DeviceKind::Pmos, "I", "VDD", "M"),
            mos("Z", DeviceKind::Nmos, "M", "GND", "O"),
            mos("W", DeviceKind::Pmos, "M", "VDD", "O"),
        ];
        let m = vf2_match(&layout, &schem).expect("isomorphic");
        assert_eq!(m.dev_pairs.len(), 4);
        let pairs: HashMap<SmolStr, SmolStr> = m.net_pairs.into_iter().collect();
        assert_eq!(pairs.get("in").map(|s| s.as_str()), Some("I"));
        assert_eq!(pairs.get("mid").map(|s| s.as_str()), Some("M"));
        assert_eq!(pairs.get("out").map(|s| s.as_str()), Some("O"));
    }
}
