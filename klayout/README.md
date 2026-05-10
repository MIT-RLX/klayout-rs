# klayout

Prelude crate for [`klayout-rs`](https://github.com/MIT-RLX/klayout-rs).

Re-exports every workspace crate so downstream code can depend on
`klayout` alone instead of enumerating every `klayout-*` crate by
hand.

## Usage

```toml
[dependencies]
klayout = "0.0.1"
```

```rust,ignore
use klayout::prelude::*;

let lib: Library = klayout::io::read_gds_path("design.gds")?;
let region: Region = ...;
```

## Re-exports

| Path | Source crate |
|------|--------------|
| `klayout::core` | `klayout-core` |
| `klayout::io` | `klayout-io` |
| `klayout::geom` | `klayout-geom` |
| `klayout::spatial` | `klayout-spatial` |
| `klayout::pdk` | `klayout-pdk` |
| `klayout::drc` | `klayout-drc` |
| `klayout::deck` | `klayout-deck` |
| `klayout::connect` | `klayout-connect` |
| `klayout::place` | `klayout-place` |
| `klayout::cts` | `klayout-cts` |
| `klayout::route` | `klayout-route` |
| `klayout::lef` | `klayout-lef` |
| `klayout::liberty` | `klayout-liberty` |
| `klayout::sta` | `klayout-sta` |

## `klayout::prelude`

Glob import for the most-frequently used names: `Library`, `Cell`,
`CellId`, `LayerIndex`, `LayerInfo`, `Bbox`, `Point`, `Polygon`,
`Path`, `Port`, `Shape`, `Trans`, `Region`, `SpatialIndex`,
`read_gds_path`, `write_gds_path`.

## License

Licensed under GPL-3.0-only.
