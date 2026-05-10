# klayout-liberty

Reader for the Liberty (`.lib`) timing-library format used by digital
sign-off STA. Part of the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

Liberty files are nested `group_name (args) { ... }` blocks
containing simple key-value attributes (`key : value ;`) and complex
attributes (`key (v1, v2, ...) ;`). Cells, pins, timing arcs, and
lookup tables all use this same recursive structure.

## What v1 covers

The data STA tools actually consume in 90% of flows:

- Library-level units (`time_unit`, `capacitive_load_unit`,
  `voltage_unit`, …).
- Per-cell `area`, `cell_leakage_power`.
- Per-pin `direction`, `capacitance`, `function`, `clock`.
- `timing` arcs with `related_pin`, `timing_sense`, `timing_type`.
- Raw text of `cell_rise` / `cell_fall` / `rise_transition` /
  `fall_transition` lookup tables.

The lookup-table values are kept verbatim; consumers parse the index
grids themselves. `nldm::parse_lookup_table` decodes them into
`LookupTable` for NLDM 2-D bilinear interpolation.

## API

| Function | Returns |
|----------|---------|
| `parse_liberty(&str)` | `Library` |
| `parse_liberty_path(path)` | `Library` |
| `parse_lookup_table(&str)` | `LookupTable` |

## Example

```rust,ignore
use klayout_liberty::parse_liberty_path;

let lib = parse_liberty_path("sky130_fd_sc_hd__tt_025C_1v80.lib")?;
for cell in lib.cells {
    println!("{} area={}", cell.name, cell.area);
}
```

## License

Licensed under GPL-3.0-only.
