//! Antenna check — flag nets whose metal area exceeds a multiple of the
//! gate area connected to that net.
//!
//! Antenna damage occurs during fabrication when a long metal trace
//! collects charge that discharges through a connected gate. The check
//! is the cumulative ratio of metal area / gate area along each net.
//! Foundry rules typically allow ratios of 200–10000 depending on the
//! metal layer and process; we leave the threshold to the caller.
//!
//! Inputs:
//! * a `Netlist` (per-metal-layer or unified — gate areas accumulate).
//! * extracted `Device`s — to find each net's gate area.
//! * the per-layer metal area for each net.
//!
//! Output: one `AntennaViolation` per offending net.

use crate::device::{Device, DeviceKind};
use crate::netlist::Netlist;
use smol_str::SmolStr;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct AntennaViolation {
    pub net: SmolStr,
    pub metal_area: f64,
    pub gate_area: f64,
    pub ratio: f64,
    pub max_allowed: f64,
}

/// Compute antenna ratios for each net in `nl` and flag any whose
/// metal-area / gate-area exceeds `max_ratio`.
///
/// `metal_area_per_net` should provide the cumulative area on metal
/// layers up to (and including) the layer being checked. Real antenna
/// rules check each metal level cumulatively — call this once per
/// metal layer with the cumulative area, or call it once with the full
/// stack-summed area for a coarse global check.
pub fn antenna_check(
    nl: &Netlist,
    devices: &[Device],
    metal_area_per_net: &HashMap<SmolStr, f64>,
    max_ratio: f64,
) -> Vec<AntennaViolation> {
    // Sum gate area per net, looking up each device's gate terminal.
    let mut gate_area_per_net: HashMap<SmolStr, f64> = HashMap::new();
    for d in devices {
        if !matches!(d.kind, DeviceKind::Nmos | DeviceKind::Pmos) {
            continue;
        }
        let Some(gate_net) = d.terminals.get("gate") else {
            continue;
        };
        let l = d.params.get("l").copied().unwrap_or(0.0);
        let w = d.params.get("w").copied().unwrap_or(0.0);
        let area = l * w;
        if area > 0.0 {
            *gate_area_per_net.entry(gate_net.clone()).or_insert(0.0) += area;
        }
    }

    let mut violations: Vec<AntennaViolation> = Vec::new();
    for net in nl.nets() {
        let metal_area = metal_area_per_net.get(&net.name).copied().unwrap_or(0.0);
        let gate_area = gate_area_per_net.get(&net.name).copied().unwrap_or(0.0);
        // Nets without any connected gate aren't antenna-relevant.
        if gate_area <= 0.0 || metal_area <= 0.0 {
            continue;
        }
        let ratio = metal_area / gate_area;
        if ratio > max_ratio {
            violations.push(AntennaViolation {
                net: net.name.clone(),
                metal_area,
                gate_area,
                ratio,
                max_allowed: max_ratio,
            });
        }
    }
    violations
}

/// Helper: compute per-net metal area on a given layer by summing the
/// `bbox` area of each net's shapes. This is approximate (uses bboxes,
/// not exact polygon area); for sign-off, use exact polygon areas via
/// `klayout-geom` integration.
pub fn metal_area_per_net(nl: &Netlist) -> HashMap<SmolStr, f64> {
    let mut out: HashMap<SmolStr, f64> = HashMap::new();
    for net in nl.nets() {
        let area = (net.bbox.width() as f64) * (net.bbox.height() as f64);
        out.insert(net.name.clone(), area);
    }
    out
}
