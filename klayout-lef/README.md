# klayout-lef

LEF / DEF reader and writer for the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

These two text formats are how digital flows hand layouts off:

- **LEF** — cell library abstracts: boundary + pin geometry, no
  internal routing.
- **DEF** — placed-and-routed netlist that references LEF cells.

## API

| Direction | Function |
|-----------|----------|
| LEF read | `read_lef`, `read_lef_full` |
| LEF write | `write_lef`, `write_lef_full` |
| DEF read | `read_def`, `read_def_full` |
| DEF write | `write_def`, `write_def_full` |

Errors return `klayout_lef::Result<T>`; the error type `LefError`
includes the failing token's line number.

## Coverage

- **OpenDB JSON parity** — `validation/klayout-validate/tests/{lef,def}.rs` (Docker-regenerated corpus).
- **DEF import combinations** — `tests/def_import_coverage.rs`: SPECIALNETS (routed stripe, `USE`), design-level `VIAS`, `ROW` / `TRACKS` / `GCELLGRID`, `BLOCKAGES` / `REGIONS` / `GROUPS`, net routing with `*` coordinates, wire-then-via segment order, and `( PIN … )` taps. Round-trip coverage remains in `tests/parity.rs` and `tests/roundtrip.rs`.

### Records handled in this crate

- LEF: `MACRO`, `PIN`, `PORT`, `OBS`, `LAYER`, `VIA`, `SITE`.
- DEF: `DESIGN`, `UNITS`, `DIEAREA`, `ROW`, `TRACKS`, `GCELLGRID`, `VIAS`, `COMPONENTS`, `PINS`, `NETS`, `SPECIALNETS`, `BLOCKAGES`, `REGIONS`, `GROUPS`.

Deeper LEF rule decks (spacing / antenna tables), STYLE/SHIELD on nets, and full property-definition surfaces are still incremental extensions.

## Example

```rust,ignore
use klayout_lef::{read_lef_full, write_def_full};

let library = read_lef_full("sky130_fd_sc_hd.lef")?;
write_def_full("design.def", &design)?;
```

## License

Licensed under GPL-3.0-only.
