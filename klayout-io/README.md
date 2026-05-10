# klayout-io

Layout-format readers and writers for the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace. Reading
produces a `klayout_core::Library`; writing consumes one.

## Formats

| Format | Read | Write |
|--------|:----:|:-----:|
| GDSII (`.gds`, `.gds.gz`) | yes | yes |
| OASIS (`.oas`) | yes | yes |
| CIF | yes | yes |
| MAGIC `.mag` | yes | yes |
| AutoCAD DXF | yes | yes |

GDSII and OASIS are differentially validated against KLayout's
reference C++ engine across the corpus shipped in
[`validation/`](../validation).

## Quick start

```rust,ignore
use klayout_io::{read_gds_path, write_gds_path};

let lib = read_gds_path("design.gds")?;
println!("loaded {} cells", lib.cells().count());
write_gds_path("out.gds", &lib)?;
```

For OASIS, CIF, MAG, and DXF use the analogous
`read_oasis_path` / `read_cif_path` / `read_mag_path` / `read_dxf_path`
(and matching `write_*`) entry points.

## Errors

All readers and writers return `klayout_io::Result<T>` whose error
type is `IoError`. Errors carry the failing record's byte offset where
the underlying format permits.

## License

Licensed under GPL-3.0-only.
