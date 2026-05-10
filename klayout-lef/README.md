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

## Coverage (v1)

- LEF: `MACRO`, `PIN`, `PORT`, `OBS`, `LAYER`, `VIA`, `SITE`.
- DEF: `DESIGN`, `UNITS`, `DIEAREA`, `ROW`, `COMPONENTS`, `PINS`,
  `NETS`.

Antenna properties, full routing records, and blockage flavours are a
follow-up; the parser scaffolding is structured to add records
incrementally.

## Example

```rust,ignore
use klayout_lef::{read_lef_full, write_def_full};

let library = read_lef_full("sky130_fd_sc_hd.lef")?;
write_def_full("design.def", &design)?;
```

## License

Licensed under GPL-3.0-only.
