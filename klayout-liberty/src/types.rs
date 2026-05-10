//! Liberty data model.

use smol_str::SmolStr;
use std::collections::HashMap;

#[derive(Default, Clone, Debug)]
pub struct Library {
    pub name: SmolStr,
    /// Library-level simple attributes by name. Examples:
    /// `time_unit = "1ns"`, `voltage_unit = "1V"`, `delay_model =
    /// "table_lookup"`.
    pub attributes: HashMap<SmolStr, SmolStr>,
    pub cells: Vec<Cell>,
}

#[derive(Default, Clone, Debug)]
pub struct Cell {
    pub name: SmolStr,
    pub area: Option<f64>,
    pub leakage_power: Option<f64>,
    pub function: Option<SmolStr>,
    pub pins: Vec<Pin>,
    pub attributes: HashMap<SmolStr, SmolStr>,
    /// Per-state-condition leakage values. Each entry corresponds to
    /// one `leakage_power () { when : "..."; value : ...; }` group.
    pub leakage_groups: Vec<LeakageGroup>,
    /// Statetable for sequential cells (FF / latch). Captures the
    /// truth-table-style description of next-state values.
    pub statetable: Option<StateTable>,
}

#[derive(Default, Clone, Debug)]
pub struct LeakageGroup {
    /// Boolean state condition under which this leakage applies
    /// (`when : "!A & B"`). Empty means "default".
    pub when: SmolStr,
    pub value: f64,
    /// Optional related-pin reference.
    pub related_pg_pin: Option<SmolStr>,
}

#[derive(Default, Clone, Debug)]
pub struct StateTable {
    /// Whitespace-separated list of input pin names.
    pub input_pins: SmolStr,
    /// Whitespace-separated list of output / internal pin names.
    pub internal_pins: SmolStr,
    /// One row per state-table line. Each row is the verbatim text
    /// after `table : "...";`.
    pub rows: Vec<SmolStr>,
}

#[derive(Default, Clone, Debug)]
pub struct Pin {
    pub name: SmolStr,
    pub direction: Option<PinDirection>,
    pub capacitance: Option<f64>,
    pub max_capacitance: Option<f64>,
    pub max_transition: Option<f64>,
    pub function: Option<SmolStr>,
    pub clock: bool,
    pub timing: Vec<TimingArc>,
    /// Internal-power (switching/static) groups attached to this pin.
    pub internal_power: Vec<InternalPower>,
    /// If this pin is a bus member (e.g. "DATA[3]"), the original
    /// bus declaration. `None` for scalar pins.
    pub bus: Option<klayout_core::BusName>,
}

#[derive(Default, Clone, Debug)]
pub struct InternalPower {
    /// `when : "..."` boolean condition.
    pub when: SmolStr,
    /// Related (driving) pin name.
    pub related_pin: Option<SmolStr>,
    /// Energy lookup tables — kept as raw text in v1.
    pub rise_power: Option<SmolStr>,
    pub fall_power: Option<SmolStr>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PinDirection {
    Input,
    Output,
    Inout,
    Internal,
}

#[derive(Default, Clone, Debug)]
pub struct TimingArc {
    pub related_pin: Option<SmolStr>,
    pub timing_sense: Option<TimingSense>,
    pub timing_type: Option<SmolStr>,
    /// Lookup tables — kept as raw text for v1.
    pub cell_rise: Option<SmolStr>,
    pub cell_fall: Option<SmolStr>,
    pub rise_transition: Option<SmolStr>,
    pub fall_transition: Option<SmolStr>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TimingSense {
    Positive,
    Negative,
    NonUnate,
}
