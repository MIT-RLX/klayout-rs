//! Electromigration (EM) current-density check.
//!
//! Each routed wire carries a current density `J = I / (W · T)`,
//! where `W` is the metal width and `T` is the metal thickness from
//! the PDK process file. Per the IEEE EM lifetime model (Black 1969),
//! median time to failure scales as
//!
//! ```text
//!     MTTF = A · J^{-n} · exp(Eₐ / kT)
//! ```
//!
//! The PDK encodes a per-layer **maximum allowed current density
//! `J_max`** at the target use temperature; any segment with
//! `J > J_max` is an EM violation.
//!
//! This module evaluates `J` per segment of a [`crate::RoutedNet`]
//! (or any sequence of horizontal/vertical wire pieces) given its
//! per-net current and the layer-stack metadata, and emits one
//! [`EmViolation`] per segment that exceeds the limit.
//!
//! ## What this is NOT
//!
//! * **A self-heat / Joule-heat thermal solver.** Real sign-off
//!   tools couple EM to a thermal-aware grid that bumps `J_max`
//!   downward by the local temperature; this v1 takes the static
//!   `J_max` from the layer table.
//! * **A bidirectional / RMS / peak EM check.** EM lifetime
//!   distinguishes DC (unidirectional), AC (bidirectional), and
//!   peak current — Liberty / SPEF would supply the per-net current
//!   waveform. v1 takes a single scalar current per net (DC RMS
//!   equivalent) so the check stays self-contained.
//! * **Via EM.** This crate's via-current model is per-wire-segment;
//!   adding a separate via-array EM check requires the via current
//!   table from the PDK and is the next upgrade.

use crate::detailed::RoutedNet;
use crate::multilayer::RouteSegment;
use klayout_core::Point;
use smol_str::SmolStr;

/// Per-layer EM-relevant constants. Field units are application-
/// dependent — typical PDK tables give `j_max_a_per_um2` (A/μm²) at
/// a stated reference temperature; thickness in nm; default routing
/// width (DBU) used when the route segment doesn't override it.
#[derive(Clone, Debug)]
pub struct EmLayer {
    pub layer_idx: usize,
    /// Metal thickness (nm). Combined with width to get cross-section.
    pub thickness_nm: f64,
    /// Maximum allowed current density (A/μm²). Above this, the
    /// segment is flagged as an EM violation.
    pub j_max_a_per_um2: f64,
    /// Default routing width on this layer (DBU). The detailed
    /// router emits a polyline per layer without per-segment width
    /// metadata, so the EM check derives width from this layer
    /// table.
    pub default_width_dbu: i64,
}

/// One per-net-per-segment current input. `current_amperes` is the
/// scalar (DC-equivalent) current; the layer index keys into the
/// supplied [`EmLayer`] table.
#[derive(Clone, Debug)]
pub struct NetCurrent {
    pub net_name: SmolStr,
    pub current_amperes: f64,
}

/// One EM violation. `j_a_per_um2` is the computed current density;
/// `j_max_a_per_um2` is the layer's limit; the ratio
/// `j_a_per_um2 / j_max_a_per_um2 > 1` is the over-spec factor.
#[derive(Clone, Debug)]
pub struct EmViolation {
    pub net_name: SmolStr,
    pub layer_idx: usize,
    pub from: Point,
    pub to: Point,
    pub width_nm: i64,
    pub j_a_per_um2: f64,
    pub j_max_a_per_um2: f64,
}

/// Run the EM check over a list of routed nets and their per-net
/// currents. Returns one [`EmViolation`] per overcurrent segment.
///
/// `dbu_per_um` converts wire-width DBU to micrometers — pass
/// `Library::dbu()` from your design.
pub fn em_check(
    routed: &[RoutedNet],
    currents: &[NetCurrent],
    layers: &[EmLayer],
    dbu_per_um: f64,
) -> Vec<EmViolation> {
    use std::collections::HashMap;
    let mut by_layer: HashMap<usize, &EmLayer> = HashMap::new();
    for l in layers {
        by_layer.insert(l.layer_idx, l);
    }
    let mut by_net: HashMap<&SmolStr, f64> = HashMap::new();
    for c in currents {
        by_net.insert(&c.net_name, c.current_amperes);
    }

    let mut out = Vec::new();
    for net in routed {
        let Some(&i_a) = by_net.get(&net.net_name) else {
            continue;
        };
        for seg in &net.segments {
            let RouteSegment::Wire { layer_idx, points } = seg else {
                continue; // Vias handled separately, not yet implemented.
            };
            let Some(layer) = by_layer.get(layer_idx) else {
                continue;
            };
            let width_dbu = layer.default_width_dbu;
            let width_um = width_dbu as f64 / dbu_per_um;
            if width_um <= 0.0 {
                continue;
            }
            let thickness_um = layer.thickness_nm / 1000.0;
            let area_um2 = width_um * thickness_um;
            if area_um2 <= 0.0 {
                continue;
            }
            let j = i_a / area_um2;
            if j > layer.j_max_a_per_um2 {
                // Emit one violation per polyline segment.
                for w in points.windows(2) {
                    out.push(EmViolation {
                        net_name: net.net_name.clone(),
                        layer_idx: *layer_idx,
                        from: w[0],
                        to: w[1],
                        width_nm: width_dbu,
                        j_a_per_um2: j,
                        j_max_a_per_um2: layer.j_max_a_per_um2,
                    });
                }
            }
        }
    }
    out
}

/// Median time to failure (years) per Black's equation. Useful for
/// reporting how badly an over-spec segment will fail. `j_ref` and
/// `mttf_ref` come from the PDK qualification; `n` is typically 1
/// (Joule-heated) or 2 (purely diffusion-driven). Activation energy
/// is folded into the reference so the caller doesn't have to wire a
/// temperature term.
pub fn black_mttf_years(
    j_a_per_um2: f64,
    j_ref_a_per_um2: f64,
    mttf_ref_years: f64,
    n: f64,
) -> f64 {
    if j_a_per_um2 <= 0.0 || j_ref_a_per_um2 <= 0.0 {
        return f64::INFINITY;
    }
    mttf_ref_years * (j_ref_a_per_um2 / j_a_per_um2).powf(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detailed::RoutedNet;
    use crate::multilayer::RouteSegment;

    fn wire_seg(layer_idx: usize, x0: i64, y0: i64, x1: i64, y1: i64) -> RouteSegment {
        RouteSegment::Wire {
            layer_idx,
            points: vec![Point::new(x0, y0), Point::new(x1, y1)],
        }
    }

    #[test]
    fn within_limit_yields_no_violation() {
        let routed = vec![RoutedNet {
            net_name: "n".into(),
            segments: vec![wire_seg(0, 0, 0, 1000, 0)],
        }];
        let currents = vec![NetCurrent {
            net_name: "n".into(),
            current_amperes: 1.0, // 1 A (unrealistic but illustrative)
        }];
        let layers = vec![EmLayer {
            layer_idx: 0,
            thickness_nm: 200.0,
            j_max_a_per_um2: 30.0, // typical M1 budget
            default_width_dbu: 200, // 0.2 μm with dbu_per_um=1000
        }];
        let v = em_check(&routed, &currents, &layers, 1000.0);
        // j = 1A / (0.2 μm × 0.2 μm) = 25 A/μm² < 30
        assert!(v.is_empty(), "1A over 0.2×0.2 μm² = 25 A/μm² < 30");
    }

    #[test]
    fn over_limit_flags_violation() {
        let routed = vec![RoutedNet {
            net_name: "n".into(),
            segments: vec![wire_seg(0, 0, 0, 1000, 0)],
        }];
        let currents = vec![NetCurrent {
            net_name: "n".into(),
            current_amperes: 1.0, // 1 A
        }];
        let layers = vec![EmLayer {
            layer_idx: 0,
            thickness_nm: 200.0,
            j_max_a_per_um2: 30.0,
            default_width_dbu: 100, // 0.1 μm
        }];
        // j = 1A / (0.1 μm × 0.2 μm) = 50 A/μm² > 30 A/μm²
        let v = em_check(&routed, &currents, &layers, 1000.0);
        assert_eq!(v.len(), 1);
        assert!(v[0].j_a_per_um2 > v[0].j_max_a_per_um2);
    }

    #[test]
    fn missing_current_or_layer_skips_segment() {
        let routed = vec![RoutedNet {
            net_name: "n".into(),
            segments: vec![wire_seg(0, 0, 0, 1000, 0)],
        }];
        // No currents → skip.
        assert_eq!(em_check(&routed, &[], &[], 1000.0).len(), 0);
        // Current but no layer table → skip.
        let cur = vec![NetCurrent {
            net_name: "n".into(),
            current_amperes: 1.0,
        }];
        assert_eq!(em_check(&routed, &cur, &[], 1000.0).len(), 0);
    }

    #[test]
    fn black_mttf_decreases_with_overcurrent() {
        // Reference: J=10 A/μm² gives 10-year MTTF, n=1.
        let mttf_at_ref = black_mttf_years(10.0, 10.0, 10.0, 1.0);
        let mttf_at_2x = black_mttf_years(20.0, 10.0, 10.0, 1.0);
        // 2× overcurrent → ½ MTTF for n=1.
        assert!((mttf_at_ref - 10.0).abs() < 1e-9);
        assert!((mttf_at_2x - 5.0).abs() < 1e-9);
    }

    #[test]
    fn black_mttf_quadratic_under_n_2() {
        // n=2 → MTTF ∝ J^-2, so 2× overcurrent → ¼ MTTF.
        let mttf = black_mttf_years(20.0, 10.0, 10.0, 2.0);
        assert!((mttf - 2.5).abs() < 1e-9);
    }
}
