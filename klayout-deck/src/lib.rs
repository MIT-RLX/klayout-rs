//! `klayout-deck` — declarative DRC rule decks.
//!
//! Replaces hand-rolled chains of `klayout_drc::*` calls with a
//! readable rule-deck literal:
//!
//! ```ignore
//! use klayout_deck::deck;
//! use klayout_geom::Region;
//!
//! struct Layers {
//!     m1: Region,
//!     m2: Region,
//!     via12: Region,
//! }
//!
//! deck! {
//!     Sky130 (layers: Layers) {
//!         rule "M1.W.1": width(layers.m1, 100);
//!         rule "M1.S.1": space(layers.m1, 200);
//!         rule "M1.A.1": area_min(layers.m1, 6000);
//!         rule "V12.E": enclosing(layers.m1, layers.via12, 50);
//!         rule "M1.M2.OL": overlap(layers.m1, layers.m2, 100);
//!         rule "M1.DEN.1": density_window(&layers.m1, (1000, 1000), (500, 500), 0.30, 0.70);
//!     }
//! }
//!
//! let report = Sky130::run(&layers);
//! assert_eq!(report.failed_rules().count(), 0);
//! ```
//!
//! Each rule expression evaluates to a `Region` (the violation area).
//! A rule fails if its violation region is non-empty. The macro
//! generates a `RuleDeck` struct with a `run` method returning a
//! `DrcReport`.

use klayout_geom::Region;
use smol_str::SmolStr;

mod rdb;

/// One rule's outcome.
#[derive(Clone, Debug)]
pub struct RuleResult {
    pub name: SmolStr,
    pub violations: Region,
}

impl RuleResult {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }

    pub fn failed(&self) -> bool {
        !self.violations.is_empty()
    }
}

/// All rules' outcomes from one `RuleDeck::run` invocation.
#[derive(Clone, Debug, Default)]
pub struct DrcReport {
    pub results: Vec<RuleResult>,
}

impl DrcReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, name: impl Into<SmolStr>, violations: Region) {
        self.results.push(RuleResult {
            name: name.into(),
            violations,
        });
    }

    pub fn failed_rules(&self) -> impl Iterator<Item = &RuleResult> {
        self.results.iter().filter(|r| r.failed())
    }

    pub fn total_violations(&self) -> usize {
        self.results.iter().map(|r| r.violations.len()).sum()
    }

    pub fn is_clean(&self) -> bool {
        self.results.iter().all(|r| r.passed())
    }
}

// Re-exports the deck macro reaches into.
#[doc(hidden)]
pub mod __exports {
    pub use klayout_drc::{
        area_min, density, density_window, density_window_with_config, enclosing, overlap, separation,
        space, width, DensityPadding, DensityWindowConfig, DensityWindowOutput,
    };
}

/// Declare a rule deck.
///
/// Syntax:
/// ```text
/// deck! {
///     <DeckName> (layers: <LayersTy>) {
///         rule "<rule-name>": <expr>;
///         ...
///     }
/// }
/// ```
///
/// Each `<expr>` is a Rust expression returning a `Region`. The deck's
/// `run(&layers)` method evaluates each rule against `layers` and
/// returns a `DrcReport`. Rule names are kept verbatim — use them as
/// foundry rule IDs (e.g. "M1.W.1").
#[macro_export]
macro_rules! deck {
    (
        $name:ident ( $layers_ident:ident : $layers_ty:ty ) {
            $( rule $rule_name:literal : $rule_body:expr ; )*
        }
    ) => {
        pub struct $name;

        impl $name {
            pub fn run($layers_ident: &$layers_ty) -> $crate::DrcReport {
                #[allow(unused_imports)]
                use $crate::__exports::*;
                let mut report = $crate::DrcReport::new();
                $(
                    let viols: $crate::__Region = { $rule_body };
                    report.push($rule_name, viols);
                )*
                report
            }
        }
    };
}

// Internal alias so the macro can refer to Region without forcing the
// caller to import it. Behind a hidden module so it doesn't pollute docs.
#[doc(hidden)]
pub type __Region = Region;
