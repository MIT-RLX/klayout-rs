//! CCS (Composite Current Source) timing model — v1.
//!
//! Liberty's CCS replaces the NLDM scalar `(input_slew, output_load)
//! → delay` table with a current-waveform output: at each `(slew,
//! load)` grid point, the cell ships a sampled `i(t)` curve. Using
//! that current waveform plus the receiver's RC tree yields the
//! actual voltage waveform at the sink — and from there delay,
//! transition, and noise immunity.
//!
//! ## What this v1 covers
//!
//! * **`CcsTable`** — the data structure. A 2-D index `(slew, cap)`
//!   over a fixed time vector, with one current value per
//!   `(i, j, t)` cell. Reads from the same Liberty `body` strings
//!   that NLDM tables already use.
//! * **`integrate_to_voltage`** — given a `CcsTable` cell and a
//!   load capacitance, integrate `i(t) / C` to recover the
//!   receiver's voltage waveform. Then the 50%-Vdd crossing time
//!   defines the propagation delay.
//! * **`ccs_delay`** — top-level `(slew, load) → delay` extraction
//!   that mirrors `NLDM::lookup`'s API surface.
//!
//! ## What this is NOT
//!
//! * **A receiver-pin model.** Real CCS includes a separate
//!   per-pin "receiver" current waveform that captures Miller-
//!   capacitance and slew degradation through the pin's input
//!   stage. We model the receiver as a lumped capacitance only.
//! * **Multi-stage waveform composition.** A net's output waveform
//!   should drive every fanout pin's CCS receiver model; v1
//!   computes per-arc delays in isolation.
//! * **A Liberty CCS reader.** The Liberty `body` parser shipped
//!   in [`klayout_liberty::nldm`] only handles `index_1`,
//!   `index_2`, `values` for NLDM tables. CCS tables additionally
//!   carry an `index_3` time vector and a 3-D `values` block. The
//!   reader is the next step; for v1 callers construct
//!   `CcsTable` programmatically.

#[derive(Clone, Debug, Default)]
pub struct CcsTable {
    /// Input transition (slew) axis — same as NLDM `index_1`.
    pub index_1: Vec<f64>,
    /// Output load capacitance axis — same as NLDM `index_2`.
    pub index_2: Vec<f64>,
    /// Time vector for the current waveform sampling.
    pub index_3: Vec<f64>,
    /// 3-D current values: `values[(i * n2 + j) * n3 + k]` = i(t_k)
    /// at slew_i, load_j. Units: amperes.
    pub values: Vec<f64>,
}

impl CcsTable {
    pub fn n_slews(&self) -> usize {
        self.index_1.len()
    }
    pub fn n_loads(&self) -> usize {
        self.index_2.len()
    }
    pub fn n_times(&self) -> usize {
        self.index_3.len()
    }

    /// Bilinear-interpolated current waveform at `(slew, load)`.
    /// Returns a per-time-sample current vector.
    pub fn current_waveform(&self, slew: f64, load: f64) -> Vec<f64> {
        let n3 = self.n_times();
        if self.values.is_empty() || n3 == 0 {
            return Vec::new();
        }
        let (i0, i1, ti) = bracket(&self.index_1, slew);
        let (j0, j1, tj) = bracket(&self.index_2, load);
        let mut out = Vec::with_capacity(n3);
        for k in 0..n3 {
            let v00 = self.value_at(i0, j0, k);
            let v01 = self.value_at(i0, j1, k);
            let v10 = self.value_at(i1, j0, k);
            let v11 = self.value_at(i1, j1, k);
            let v0 = v00 + (v01 - v00) * tj;
            let v1 = v10 + (v11 - v10) * tj;
            out.push(v0 + (v1 - v0) * ti);
        }
        out
    }

    fn value_at(&self, i: usize, j: usize, k: usize) -> f64 {
        let n2 = self.n_loads();
        let n3 = self.n_times();
        let idx = (i * n2 + j) * n3 + k;
        self.values.get(idx).copied().unwrap_or(0.0)
    }
}

fn bracket(axis: &[f64], target: f64) -> (usize, usize, f64) {
    let n = axis.len();
    if n == 0 {
        return (0, 0, 0.0);
    }
    if target <= axis[0] {
        return (0, 0, 0.0);
    }
    if target >= axis[n - 1] {
        return (n - 1, n - 1, 0.0);
    }
    for i in 0..n - 1 {
        if axis[i] <= target && target <= axis[i + 1] {
            let span = axis[i + 1] - axis[i];
            let t = if span > 0.0 {
                (target - axis[i]) / span
            } else {
                0.0
            };
            return (i, i + 1, t);
        }
    }
    (n - 1, n - 1, 0.0)
}

/// Integrate `i(t) / C` to recover the voltage waveform at the
/// receiver pin.
///
/// `current` is the per-time-sample current (output of
/// [`CcsTable::current_waveform`]). `times` is the corresponding
/// time vector (`CcsTable::index_3`). `load_cap` is the lumped
/// receiver capacitance (in farads). Returns `(times, voltages)`
/// with `voltages[0] = 0`.
pub fn integrate_to_voltage(
    current: &[f64],
    times: &[f64],
    load_cap: f64,
) -> (Vec<f64>, Vec<f64>) {
    let n = current.len().min(times.len());
    if n == 0 || load_cap <= 0.0 {
        return (Vec::new(), Vec::new());
    }
    let mut v = vec![0.0; n];
    let mut t_out = vec![0.0; n];
    let mut accum = 0.0;
    t_out[0] = times[0];
    for k in 1..n {
        let dt = times[k] - times[k - 1];
        let i_avg = 0.5 * (current[k] + current[k - 1]);
        accum += i_avg * dt / load_cap;
        v[k] = accum;
        t_out[k] = times[k];
    }
    (t_out, v)
}

/// `(slew, load) → propagation delay` extracted from a CCS table by
/// integrating to the receiver-voltage waveform and finding the 50%-
/// Vdd crossing.
///
/// `vdd` is the supply voltage. Returns `0.0` when the waveform never
/// crosses 50% Vdd within the sampled time window (degenerate input).
pub fn ccs_delay(table: &CcsTable, slew: f64, load: f64, vdd: f64) -> f64 {
    let current = table.current_waveform(slew, load);
    let (times, voltages) = integrate_to_voltage(&current, &table.index_3, load);
    let half = 0.5 * vdd;
    for k in 1..voltages.len() {
        if (voltages[k - 1] < half && voltages[k] >= half)
            || (voltages[k - 1] > half && voltages[k] <= half)
        {
            // Linear interp of the crossing time.
            let dv = voltages[k] - voltages[k - 1];
            if dv.abs() < 1e-30 {
                return times[k];
            }
            let frac = (half - voltages[k - 1]) / dv;
            return times[k - 1] + frac * (times[k] - times[k - 1]);
        }
    }
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp_table() -> CcsTable {
        // 1×1×N table with a linear-ramp current waveform from 0 to
        // 1 µA over 100 ps. Integrated into a 1 fF cap:
        //   V_end = ½ · i_max · t / C
        //         = ½ · 1e-6 · 100e-12 / 1e-15
        //         = 5e-14 / 1e-15
        //         = 50 mV
        // 50% of Vdd=100 mV is 50 mV → crossing at the end of the
        // sample window (t ≈ 100 ps).
        let times: Vec<f64> = (0..101).map(|k| k as f64 * 1e-12).collect();
        let current: Vec<f64> = times.iter().map(|t| t / 1e-4).collect(); // rises 0→1µA over 100ps
        CcsTable {
            index_1: vec![0.05e-9],
            index_2: vec![1e-15],
            index_3: times,
            values: current,
        }
    }

    #[test]
    fn waveform_extraction_returns_correct_length() {
        let t = ramp_table();
        let w = t.current_waveform(0.05e-9, 1e-15);
        assert_eq!(w.len(), 101);
        // First sample 0 A, last 1 µA.
        assert!((w[0] - 0.0).abs() < 1e-12);
        assert!((w[100] - 1e-6).abs() < 1e-12);
    }

    #[test]
    fn integrate_voltage_monotonic() {
        let t = ramp_table();
        let current = t.current_waveform(0.05e-9, 1e-15);
        let (_, v) = integrate_to_voltage(&current, &t.index_3, 1e-15);
        for w in v.windows(2) {
            assert!(w[1] >= w[0] - 1e-12);
        }
        // End voltage = ½ × i_max × t_total / C = ½ × 1 × 100ps / 1fF
        // = 0.05 V.
        assert!((v[100] - 0.05).abs() < 1e-3);
    }

    #[test]
    fn ccs_delay_extracts_50pct_crossing() {
        let t = ramp_table();
        // Vdd = 100 mV. 50% crossing happens when the integral
        // reaches 50 mV — which is at t = 100 ps (the very end).
        let d = ccs_delay(&t, 0.05e-9, 1e-15, 0.1);
        assert!(d > 0.0);
        assert!(d <= 1.01e-10);
    }

    #[test]
    fn empty_table_returns_zero_delay() {
        let t = CcsTable::default();
        assert_eq!(ccs_delay(&t, 0.0, 0.0, 1.0), 0.0);
    }
}
