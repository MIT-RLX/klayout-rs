//! LVS comparison — match a layout-extracted design against a reference
//! schematic netlist.
//!
//! v1 strategy:
//! 1. **Name correspondence first.** Nets and devices with the same names
//!    in both designs are matched directly. This handles the common case
//!    where the layout is properly labeled.
//! 2. **Structural fallback for unnamed.** Remaining nets/devices match
//!    by their connection signature: a device matches if its terminal
//!    nets all have correspondents on the other side. Iterates until
//!    fixed-point.
//! 3. **Report mismatches.** Anything left unmatched at the end is a
//!    structural difference.
//!
//! Not implemented in v1: graph-isomorphism-based matching for large
//! unnamed designs (would need VF2 or similar), parameter-tolerance
//! matching (W/L within ε), property-based device differentiation.
//! These are necessary for sign-off LVS but a basic name+structural
//! comparison covers small/medium designs and is enough to validate
//! that the layout-extraction → comparison pipeline works.

use crate::device::{Device, DeviceKind};
use crate::netlist::Netlist;
use crate::vf2::vf2_match;
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};

/// One LVS run's outcome.
#[derive(Default, Clone, Debug)]
pub struct LvsReport {
    pub matched_nets: Vec<(SmolStr, SmolStr)>,
    pub matched_devices: Vec<(SmolStr, SmolStr)>,
    pub unmatched_layout_nets: Vec<SmolStr>,
    pub unmatched_schem_nets: Vec<SmolStr>,
    pub unmatched_layout_devices: Vec<SmolStr>,
    pub unmatched_schem_devices: Vec<SmolStr>,
    pub device_mismatches: Vec<DeviceMismatch>,
}

#[derive(Clone, Debug)]
pub struct DeviceMismatch {
    pub layout: SmolStr,
    pub schematic: SmolStr,
    pub reason: SmolStr,
}

impl LvsReport {
    pub fn is_clean(&self) -> bool {
        self.unmatched_layout_nets.is_empty()
            && self.unmatched_schem_nets.is_empty()
            && self.unmatched_layout_devices.is_empty()
            && self.unmatched_schem_devices.is_empty()
            && self.device_mismatches.is_empty()
    }
}

/// Per-parameter tolerance for `lvs_compare_with_tol`. Two devices'
/// parameters are considered equivalent iff every shared key satisfies
/// `|a − b| ≤ abs_tol + rel_tol · max(|a|, |b|)`. Missing keys on
/// either side are ignored (we only check what both have).
#[derive(Clone, Debug)]
pub struct ParamTolerance {
    pub abs_tol: f64,
    pub rel_tol: f64,
}

impl Default for ParamTolerance {
    fn default() -> Self {
        // Default: 1% relative + 1pm absolute (the latter is small
        // enough to not matter for any electrical parameter, but
        // catches genuine equality even after f64 rounding).
        Self {
            abs_tol: 1e-12,
            rel_tol: 0.01,
        }
    }
}

/// LVS comparison ignoring device parameters (W/L tolerance, etc).
/// Wraps `lvs_compare_with_tol` with `ParamTolerance::default()`.
pub fn lvs_compare(
    layout_nl: &Netlist,
    layout_devs: &[Device],
    schem_nl: &Netlist,
    schem_devs: &[Device],
) -> LvsReport {
    lvs_compare_with_tol(
        layout_nl,
        layout_devs,
        schem_nl,
        schem_devs,
        &ParamTolerance::default(),
    )
}

/// LVS comparison with explicit per-parameter tolerance. Mismatched
/// parameters (e.g. W/L not within `tol`) are reported as device
/// mismatches even when the structural match succeeds.
pub fn lvs_compare_with_tol(
    layout_nl: &Netlist,
    layout_devs: &[Device],
    schem_nl: &Netlist,
    schem_devs: &[Device],
    tol: &ParamTolerance,
) -> LvsReport {
    let mut report = LvsReport::default();

    // ---- Phase 1: net name correspondence ----
    let layout_nets: HashSet<SmolStr> = layout_nl.nets().iter().map(|n| n.name.clone()).collect();
    let schem_nets: HashSet<SmolStr> = schem_nl.nets().iter().map(|n| n.name.clone()).collect();
    let net_intersection: HashSet<SmolStr> =
        layout_nets.intersection(&schem_nets).cloned().collect();
    for n in &net_intersection {
        report.matched_nets.push((n.clone(), n.clone()));
    }
    for n in &layout_nets {
        if !net_intersection.contains(n) {
            report.unmatched_layout_nets.push(n.clone());
        }
    }
    for n in &schem_nets {
        if !net_intersection.contains(n) {
            report.unmatched_schem_nets.push(n.clone());
        }
    }
    report.unmatched_layout_nets.sort();
    report.unmatched_schem_nets.sort();
    report.matched_nets.sort();

    // ---- Phase 2: device matching by name + structural sanity ----
    let layout_by_name: HashMap<SmolStr, &Device> =
        layout_devs.iter().map(|d| (d.name.clone(), d)).collect();
    let schem_by_name: HashMap<SmolStr, &Device> =
        schem_devs.iter().map(|d| (d.name.clone(), d)).collect();

    let mut layout_matched: HashSet<SmolStr> = HashSet::new();
    let mut schem_matched: HashSet<SmolStr> = HashSet::new();

    // Match by device name; verify kind + terminal-net correspondence.
    for (name, l_dev) in &layout_by_name {
        if let Some(s_dev) = schem_by_name.get(name) {
            let mut reason = device_compatible(l_dev, s_dev, &net_intersection);
            if reason.is_none() {
                reason = parameter_compatible(l_dev, s_dev, tol);
            }
            if let Some(reason) = reason {
                report.device_mismatches.push(DeviceMismatch {
                    layout: l_dev.name.clone(),
                    schematic: s_dev.name.clone(),
                    reason,
                });
            } else {
                report.matched_devices.push((l_dev.name.clone(), s_dev.name.clone()));
            }
            layout_matched.insert(name.clone());
            schem_matched.insert(name.clone());
        }
    }

    // ---- Phase 3: structural matching for unmatched devices ----
    // For unmatched devices, match by structural signature: device kind
    // + sorted terminal-net names (using only matched nets). Iterate to
    // fixed-point so newly-matched devices help match more nets in turn.
    let mut changed = true;
    while changed {
        changed = false;
        let mut by_sig: HashMap<DeviceSignature, Vec<&Device>> = HashMap::new();
        for d in layout_devs {
            if layout_matched.contains(&d.name) {
                continue;
            }
            by_sig
                .entry(device_signature(d, &net_intersection))
                .or_default()
                .push(d);
        }
        for d in schem_devs {
            if schem_matched.contains(&d.name) {
                continue;
            }
            let sig = device_signature(d, &net_intersection);
            if let Some(layout_candidates) = by_sig.get_mut(&sig) {
                if let Some(l_dev) = layout_candidates.pop() {
                    report
                        .matched_devices
                        .push((l_dev.name.clone(), d.name.clone()));
                    layout_matched.insert(l_dev.name.clone());
                    schem_matched.insert(d.name.clone());
                    changed = true;
                }
            }
        }
    }

    for d in layout_devs {
        if !layout_matched.contains(&d.name) {
            report.unmatched_layout_devices.push(d.name.clone());
        }
    }
    for d in schem_devs {
        if !schem_matched.contains(&d.name) {
            report.unmatched_schem_devices.push(d.name.clone());
        }
    }
    report.unmatched_layout_devices.sort();
    report.unmatched_schem_devices.sort();
    report.matched_devices.sort();

    report
}

/// Compare every parameter present on **both** devices. A missing
/// parameter on either side is silently ignored — extraction may
/// supply parameters the schematic doesn't (e.g. effective drain area)
/// or vice versa. Use the tolerance struct's combined `abs_tol +
/// rel_tol · max(|a|, |b|)` rule.
fn parameter_compatible(a: &Device, b: &Device, tol: &ParamTolerance) -> Option<SmolStr> {
    for (k, &av) in &a.params {
        if let Some(&bv) = b.params.get(k) {
            let allowed = tol.abs_tol + tol.rel_tol * av.abs().max(bv.abs());
            if (av - bv).abs() > allowed {
                return Some(SmolStr::from(format!(
                    "param {k}: {av} vs {bv} (Δ {:.4e} > tol {:.4e})",
                    (av - bv).abs(),
                    allowed
                )));
            }
        }
    }
    None
}

fn device_compatible(a: &Device, b: &Device, net_isect: &HashSet<SmolStr>) -> Option<SmolStr> {
    if a.kind != b.kind {
        return Some(SmolStr::from(format!("kind: {:?} vs {:?}", a.kind, b.kind)));
    }
    // Terminal-by-terminal: each connected net should have a matched
    // counterpart (same name in both, present in the net intersection).
    let mut terms: HashSet<&SmolStr> = a.terminals.keys().collect();
    for k in b.terminals.keys() {
        terms.insert(k);
    }
    for term in terms {
        let an = a.terminals.get(term);
        let bn = b.terminals.get(term);
        match (an, bn) {
            (Some(x), Some(y)) if x == y && net_isect.contains(x) => {}
            (None, None) => {}
            _ => {
                return Some(SmolStr::from(format!(
                    "terminal {term}: {an:?} vs {bn:?}"
                )))
            }
        }
    }
    None
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct DeviceSignature {
    kind: DeviceKind,
    /// Sorted list of (terminal-name, connected-net-name) using only
    /// nets in the matched-net intersection. Devices with all-unmatched
    /// nets get an empty signature and won't structurally match.
    terms: Vec<(SmolStr, SmolStr)>,
}

/// LVS using full VF2 graph isomorphism. Use this when the layout and
/// schematic do not share net or device names (i.e. the cheap
/// name-correspondence path in [`lvs_compare`] won't catch the match).
///
/// Returns a clean report on isomorphism. On non-isomorphism, falls
/// back to `lvs_compare` so callers see *which* devices couldn't be
/// paired, not just a global "no match".
pub fn lvs_compare_vf2(
    layout_nl: &Netlist,
    layout_devs: &[Device],
    schem_nl: &Netlist,
    schem_devs: &[Device],
) -> LvsReport {
    if let Some(m) = vf2_match(layout_devs, schem_devs) {
        let mut report = LvsReport {
            matched_devices: m.dev_pairs,
            matched_nets: m.net_pairs,
            ..Default::default()
        };
        report.matched_devices.sort();
        report.matched_nets.sort();
        return report;
    }
    lvs_compare(layout_nl, layout_devs, schem_nl, schem_devs)
}

fn device_signature(d: &Device, net_isect: &HashSet<SmolStr>) -> DeviceSignature {
    let mut terms: Vec<(SmolStr, SmolStr)> = d
        .terminals
        .iter()
        .filter_map(|(t, n)| {
            if net_isect.contains(n) {
                Some((t.clone(), n.clone()))
            } else {
                None
            }
        })
        .collect();
    terms.sort();
    DeviceSignature {
        kind: d.kind,
        terms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceKind;
    use crate::netlist::Netlist;
    use klayout_core::{Bbox, Point};

    fn nl_with_nets(names: &[&str]) -> Netlist {
        let mut nl = Netlist::new();
        for (i, n) in names.iter().enumerate() {
            nl.insert(crate::netlist::Net::new(
                crate::netlist::NetId(i as u32),
                SmolStr::from(*n),
            ));
        }
        nl
    }

    fn mos(name: &str, kind: DeviceKind, gate: &str, src: &str, drn: &str) -> Device {
        let mut d = Device::new(kind, name, Bbox::new(Point::new(0, 0), Point::new(1, 1)));
        d.terminals.insert("gate".into(), gate.into());
        d.terminals.insert("source".into(), src.into());
        d.terminals.insert("drain".into(), drn.into());
        d
    }

    #[test]
    fn parameter_tolerance_within_eps_passes() {
        let l_nl = nl_with_nets(&["g", "s", "d"]);
        let s_nl = nl_with_nets(&["g", "s", "d"]);
        let mut l = mos("M1", DeviceKind::Nmos, "g", "s", "d");
        let mut s = mos("M1", DeviceKind::Nmos, "g", "s", "d");
        l.params.insert("W".into(), 0.500);
        s.params.insert("W".into(), 0.503); // 0.6% mismatch
        let r = lvs_compare(&l_nl, &[l], &s_nl, &[s]);
        assert!(r.is_clean(), "tolerance default should accept 0.6% W");
    }

    #[test]
    fn parameter_tolerance_outside_eps_fails() {
        let l_nl = nl_with_nets(&["g", "s", "d"]);
        let s_nl = nl_with_nets(&["g", "s", "d"]);
        let mut l = mos("M1", DeviceKind::Nmos, "g", "s", "d");
        let mut s = mos("M1", DeviceKind::Nmos, "g", "s", "d");
        l.params.insert("W".into(), 0.500);
        s.params.insert("W".into(), 0.600); // 20% mismatch
        let r = lvs_compare(&l_nl, &[l], &s_nl, &[s]);
        assert!(!r.is_clean(), "20% W mismatch must be flagged");
        assert_eq!(r.device_mismatches.len(), 1);
    }

    #[test]
    fn explicit_tolerance_overrides_default() {
        let l_nl = nl_with_nets(&["g", "s", "d"]);
        let s_nl = nl_with_nets(&["g", "s", "d"]);
        let mut l = mos("M1", DeviceKind::Nmos, "g", "s", "d");
        let mut s = mos("M1", DeviceKind::Nmos, "g", "s", "d");
        l.params.insert("W".into(), 0.500);
        s.params.insert("W".into(), 0.600); // 20% mismatch
        let loose = ParamTolerance {
            abs_tol: 0.0,
            rel_tol: 0.5, // 50% — should accept
        };
        let r = lvs_compare_with_tol(&l_nl, &[l], &s_nl, &[s], &loose);
        assert!(r.is_clean());
    }

    #[test]
    fn missing_param_on_one_side_is_ignored() {
        let l_nl = nl_with_nets(&["g", "s", "d"]);
        let s_nl = nl_with_nets(&["g", "s", "d"]);
        let mut l = mos("M1", DeviceKind::Nmos, "g", "s", "d");
        let s = mos("M1", DeviceKind::Nmos, "g", "s", "d");
        l.params.insert("effective_drain_area".into(), 1e-15);
        // schem has no such param.
        let r = lvs_compare(&l_nl, &[l], &s_nl, &[s]);
        assert!(r.is_clean());
    }
}
