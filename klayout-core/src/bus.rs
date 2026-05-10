//! Bus-pin name parsing and expansion.
//!
//! LEF / DEF / Liberty all use `A[3:0]` (or `A<3:0>`) to declare a
//! 4-bit bus pin. The data model here is scalar — each bit becomes
//! its own `Pin` / `Net` / `Port`. This module provides:
//!
//! * [`parse_bus`] — `"A[3:0]"` → `BusName { base, lo, hi, ascending }`.
//! * [`BusName::expand`] — yield the scalar names `A[3]`, `A[2]`, …
//! * [`format_bus_pin`] — given base + index, produce `A[3]`.
//! * [`Bus`] — captured original bus declaration so emitters can
//!   round-trip the bracketed form back to LEF / DEF / Liberty.
//!
//! Bracket convention defaults to `[` / `]`; alternates are accepted
//! via [`BusBitChars`] (`<>`, `()`, `{}`).

use smol_str::SmolStr;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct BusBitChars {
    pub open: char,
    pub close: char,
}

impl Default for BusBitChars {
    fn default() -> Self {
        Self {
            open: '[',
            close: ']',
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BusName {
    pub base: SmolStr,
    pub lo: i32,
    pub hi: i32,
    /// `true` if the original notation went lo:hi (e.g. `A[0:3]`),
    /// `false` for descending (e.g. `A[3:0]`). Round-trip emission
    /// preserves the original direction.
    pub ascending: bool,
}

impl BusName {
    /// Iterate scalar pin names in the original direction.
    pub fn expand(&self, chars: BusBitChars) -> impl Iterator<Item = String> + '_ {
        let (start, step): (i32, i32) = if self.ascending {
            (self.lo, 1)
        } else {
            (self.hi, -1)
        };
        let count = (self.hi - self.lo).unsigned_abs() as usize + 1;
        (0..count).map(move |k| {
            let i = start + (k as i32) * step;
            format!("{}{}{i}{}", self.base, chars.open, chars.close)
        })
    }

    pub fn width(&self) -> u32 {
        (self.hi - self.lo).unsigned_abs() + 1
    }
}

/// Parse a bus name. Returns `None` if `s` doesn't contain bracket
/// notation; the caller should treat that as a scalar pin.
pub fn parse_bus(s: &str, chars: BusBitChars) -> Option<BusName> {
    let open = s.find(chars.open)?;
    let close = s.find(chars.close)?;
    if open >= close {
        return None;
    }
    let base = SmolStr::from(s[..open].trim());
    let inside = &s[open + 1..close];
    let parts: Vec<&str> = inside.split(':').collect();
    let (a, b) = match parts.as_slice() {
        [single] => {
            // Scalar bit reference like `A[3]` — lo=hi=3, ascending true.
            let n: i32 = single.trim().parse().ok()?;
            (n, n)
        }
        [a, b] => {
            let a: i32 = a.trim().parse().ok()?;
            let b: i32 = b.trim().parse().ok()?;
            (a, b)
        }
        _ => return None,
    };
    let lo = a.min(b);
    let hi = a.max(b);
    let ascending = a <= b;
    Some(BusName {
        base,
        lo,
        hi,
        ascending,
    })
}

/// Format a single bit of a bus: `(base, index)` → `base[index]`.
pub fn format_bus_pin(base: &str, index: i32, chars: BusBitChars) -> String {
    format!("{base}{}{index}{}", chars.open, chars.close)
}

/// One bus declaration captured during LEF/DEF/Liberty parsing.
/// Emitters re-render the bracket form to round-trip cleanly.
#[derive(Clone, Debug)]
pub struct Bus {
    pub name: BusName,
    pub member_pins: Vec<SmolStr>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_descending_range() {
        let b = parse_bus("DATA[3:0]", BusBitChars::default()).unwrap();
        assert_eq!(b.base.as_str(), "DATA");
        assert_eq!(b.lo, 0);
        assert_eq!(b.hi, 3);
        assert!(!b.ascending);
        assert_eq!(b.width(), 4);
    }

    #[test]
    fn parse_ascending_range() {
        let b = parse_bus("Q[0:7]", BusBitChars::default()).unwrap();
        assert!(b.ascending);
        assert_eq!(b.width(), 8);
    }

    #[test]
    fn parse_single_bit() {
        let b = parse_bus("A[5]", BusBitChars::default()).unwrap();
        assert_eq!(b.lo, 5);
        assert_eq!(b.hi, 5);
        assert_eq!(b.width(), 1);
    }

    #[test]
    fn parse_returns_none_for_scalar() {
        assert!(parse_bus("scalar_pin", BusBitChars::default()).is_none());
    }

    #[test]
    fn alternate_bit_chars() {
        let chars = BusBitChars {
            open: '<',
            close: '>',
        };
        let b = parse_bus("D<7:0>", chars).unwrap();
        assert_eq!(b.width(), 8);
        assert_eq!(b.base.as_str(), "D");
    }

    #[test]
    fn expand_yields_scalar_names() {
        let b = parse_bus("A[3:0]", BusBitChars::default()).unwrap();
        let names: Vec<String> = b.expand(BusBitChars::default()).collect();
        assert_eq!(names, vec!["A[3]", "A[2]", "A[1]", "A[0]"]);
    }

    #[test]
    fn ascending_expand_preserves_direction() {
        let b = parse_bus("A[0:3]", BusBitChars::default()).unwrap();
        let names: Vec<String> = b.expand(BusBitChars::default()).collect();
        assert_eq!(names, vec!["A[0]", "A[1]", "A[2]", "A[3]"]);
    }

    #[test]
    fn format_bus_pin_uses_chosen_chars() {
        let s = format_bus_pin("VAL", 42, BusBitChars { open: '<', close: '>' });
        assert_eq!(s, "VAL<42>");
    }
}
