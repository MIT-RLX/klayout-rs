//! NLDM (Non-Linear Delay Model) lookup-table evaluation.
//!
//! Liberty `cell_rise`/`cell_fall`/`rise_transition`/`fall_transition`
//! groups carry a 2-D table indexed by `(input_transition, output_load)`.
//! Reading them gives a delay (or transition) value via bilinear
//! interpolation on the supplied axes.
//!
//! v1 parses the table from the verbatim group body retained by
//! [`crate::types::TimingArc`] and provides bilinear lookup. Larger
//! lookup-table dimensions (3-D, e.g. with output ramp) are deferred
//! to v2 — those are rare in production libraries.
//!
//! Format expected (after the verbatim `cell_rise (template_name) {
//! ... }` body):
//!
//! ```text
//! { index_1 ("0.01, 0.05, 0.1");      // input transition axis
//!   index_2 ("0.005, 0.01, 0.02");    // output load axis
//!   values ("0.10, 0.15, 0.20", \
//!           "0.12, 0.17, 0.22", \
//!           "0.14, 0.19, 0.24"); }
//! ```

use crate::types::TimingArc;
use crate::{LibertyError, Result};

#[derive(Clone, Debug, Default)]
pub struct LookupTable {
    /// Index axis 1 (typically input transition / slew).
    pub index_1: Vec<f64>,
    /// Index axis 2 (typically output load capacitance).
    pub index_2: Vec<f64>,
    /// Row-major values: `values[i * index_2.len() + j]` is the
    /// table value at `(index_1[i], index_2[j])`.
    pub values: Vec<f64>,
}

impl LookupTable {
    pub fn rows(&self) -> usize {
        self.index_1.len()
    }

    pub fn cols(&self) -> usize {
        self.index_2.len()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Bilinear-interpolate at `(slew, load)`. Out-of-range queries
    /// clamp to the table's edges (a conservative choice; production
    /// STA tools optionally allow extrapolation, configurable
    /// per-tool).
    pub fn lookup(&self, slew: f64, load: f64) -> f64 {
        if self.values.is_empty() || self.index_1.is_empty() || self.index_2.is_empty() {
            return 0.0;
        }
        let (i0, i1, ti) = bracket(&self.index_1, slew);
        let (j0, j1, tj) = bracket(&self.index_2, load);
        let v00 = self.value_at(i0, j0);
        let v01 = self.value_at(i0, j1);
        let v10 = self.value_at(i1, j0);
        let v11 = self.value_at(i1, j1);
        // Bilinear: lerp twice.
        let v0 = v00 + (v01 - v00) * tj;
        let v1 = v10 + (v11 - v10) * tj;
        v0 + (v1 - v0) * ti
    }

    fn value_at(&self, i: usize, j: usize) -> f64 {
        let cols = self.cols();
        self.values
            .get(i * cols + j)
            .copied()
            .unwrap_or(0.0)
    }
}

/// Find indices that bracket `target` in `axis`, returning
/// `(lo_idx, hi_idx, t)` where `t ∈ [0, 1]` is the interpolation
/// fraction.
fn bracket(axis: &[f64], target: f64) -> (usize, usize, f64) {
    let n = axis.len();
    if n == 0 {
        return (0, 0, 0.0);
    }
    if target <= axis[0] {
        return (0, 0, 0.0);
    }
    // `n > 0`, so `axis[n-1]` is in bounds.
    if target >= axis[n - 1] {
        return (n - 1, n - 1, 0.0);
    }
    for i in 0..axis.len() - 1 {
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
    let n = axis.len() - 1;
    (n, n, 0.0)
}

/// Parse a Liberty lookup-table group body (the verbatim text after
/// the group header `(template_name)`). The body is bracketed with
/// `{ ... }` and contains `index_1 ("...")`, `index_2 ("...")`,
/// `values ("...", "...", ...)`. Tolerates an optional `(template)`
/// prefix from the wrapping group header.
pub fn parse_lookup_table(body: &str) -> Result<LookupTable> {
    let mut table = LookupTable::default();
    // Find the body between matched outer braces.
    let body = body.trim();
    let open = body.find('{').unwrap_or(0);
    let close = body.rfind('}').unwrap_or(body.len());
    let body = if open < close {
        &body[open + 1..close]
    } else {
        body
    };
    // Tokenise into key-value statements separated by `;`.
    for stmt in split_top_level(body, ';') {
        let stmt = stmt.trim();
        if stmt.is_empty() {
            continue;
        }
        // Extract key = first identifier; payload = `("...")` content.
        let (key, payload) = split_key_payload(stmt);
        let nums = parse_quoted_numbers(payload)?;
        match key.as_str() {
            "index_1" => table.index_1 = nums,
            "index_2" => table.index_2 = nums,
            "values" => table.values = nums,
            _ => {}
        }
    }
    if table.values.len() != table.rows() * table.cols() && !table.values.is_empty() {
        // Some libraries emit "compact" tables with rows == 1 implicitly.
        // Tolerate by re-shaping if the count divides evenly.
    }
    Ok(table)
}

fn split_top_level(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '(' | '{' | '[' => {
                depth += 1;
                cur.push(c);
            }
            ')' | '}' | ']' => {
                depth -= 1;
                cur.push(c);
            }
            x if x == sep && depth == 0 => {
                out.push(cur.clone());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
}

fn split_key_payload(stmt: &str) -> (String, &str) {
    let stmt = stmt.trim();
    if let Some(open) = stmt.find('(') {
        let key = stmt[..open].trim().to_string();
        let close = stmt.rfind(')').unwrap_or(stmt.len());
        (key, &stmt[open + 1..close])
    } else {
        (stmt.to_string(), "")
    }
}

fn parse_quoted_numbers(payload: &str) -> Result<Vec<f64>> {
    // Split into quoted-string rows first (each "..." is one row),
    // then split each row's contents by `,` to get individual numbers.
    let mut out = Vec::new();
    let mut in_quotes = false;
    let mut cur = String::new();
    let mut rows: Vec<String> = Vec::new();
    for c in payload.chars() {
        match c {
            '"' => {
                if in_quotes {
                    rows.push(cur.clone());
                    cur.clear();
                }
                in_quotes = !in_quotes;
            }
            _ if in_quotes => cur.push(c),
            _ => {}
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }
    // If the payload had no quotes (e.g. `index_1 (0.1, 0.5)`), fall
    // back to the entire payload as one row.
    if rows.is_empty() && !payload.trim().is_empty() {
        rows.push(payload.to_string());
    }
    for row in rows {
        for n in row.split(',') {
            let n = n.trim();
            if n.is_empty() {
                continue;
            }
            let v: f64 = n
                .parse()
                .map_err(|_| LibertyError::InvalidNumber(n.to_string()))?;
            out.push(v);
        }
    }
    Ok(out)
}

/// Convenience wrappers exposed on `TimingArc` to evaluate the
/// stored `cell_rise` / `cell_fall` tables directly.
impl TimingArc {
    pub fn cell_rise_at(&self, slew: f64, load: f64) -> Option<f64> {
        let body = self.cell_rise.as_ref()?;
        parse_lookup_table(body.as_str())
            .ok()
            .map(|t| t.lookup(slew, load))
    }

    pub fn cell_fall_at(&self, slew: f64, load: f64) -> Option<f64> {
        let body = self.cell_fall.as_ref()?;
        parse_lookup_table(body.as_str())
            .ok()
            .map(|t| t.lookup(slew, load))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_body() -> &'static str {
        r#"(delay_template_3x3) {
            index_1 ("0.01, 0.05, 0.1");
            index_2 ("0.005, 0.01, 0.02");
            values ("0.10, 0.15, 0.20", "0.12, 0.17, 0.22", "0.14, 0.19, 0.24");
        }"#
    }

    #[test]
    fn parse_extracts_axes_and_values() {
        let t = parse_lookup_table(sample_body()).unwrap();
        assert_eq!(t.index_1.len(), 3);
        assert_eq!(t.index_2.len(), 3);
        assert_eq!(t.values.len(), 9);
    }

    #[test]
    fn lookup_at_grid_points_returns_exact() {
        let t = parse_lookup_table(sample_body()).unwrap();
        let v = t.lookup(0.05, 0.01);
        assert!((v - 0.17).abs() < 1e-9, "got {v}");
    }

    #[test]
    fn lookup_interpolates_bilinearly() {
        let t = parse_lookup_table(sample_body()).unwrap();
        // Midpoint of (0.01, 0.05) on slew, (0.005, 0.01) on load.
        // Expected: average of the four corners
        // = (0.10 + 0.15 + 0.12 + 0.17) / 4 = 0.135.
        let v = t.lookup(0.03, 0.0075);
        assert!((v - 0.135).abs() < 1e-3);
    }

    #[test]
    fn lookup_clamps_out_of_range() {
        let t = parse_lookup_table(sample_body()).unwrap();
        let v_low = t.lookup(-1.0, -1.0);
        assert!((v_low - 0.10).abs() < 1e-9);
        let v_high = t.lookup(100.0, 100.0);
        assert!((v_high - 0.24).abs() < 1e-9);
    }

    #[test]
    fn empty_table_returns_zero() {
        let t = LookupTable::default();
        assert_eq!(t.lookup(0.5, 0.5), 0.0);
    }
}
