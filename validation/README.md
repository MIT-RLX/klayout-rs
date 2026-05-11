# Differential validation against KLayout

This directory holds the oracle harness that verifies klayout-rs produces
the same results as KLayout's reference C++ engine (via `klayout.db`).

## TL;DR — current parity

```text
cargo test -p klayout-validate --test parity_report -- --nocapture
```

prints a Markdown table summarising pass/fail counts per suite. As of
the last corpus regen (KLayout 0.30.8 + OpenROAD 26Q2): **4893 / 4893
cases pass (100.00%)** across two oracles — `klayout.db` for layout
formats and `openroad/orfs` (OpenSTA / OpenDB) via Docker for Liberty,
LEF, DEF, and SPEF. Strict
vertex-level equality (no point-set or symmetric-difference fudge —
every polygon must match the reference vertex-for-vertex after
canonicalising to lowest-y/lowest-x first). The `parity_report` test
is the regression gate — if any case diverges, the test fails.

Suites with `n/a` in the report need a corpus regen (run
`oracle.py <suite>` to populate them — requires `klayout.db` install).
Suites without an oracle (LEF/DEF/Liberty/SPEF/CIF/DXF/MAG) are listed
at the bottom of the report with an explanation of why.

## How it works

1. **`oracle.py`** runs against a real `klayout.db` install, exercising
   `db.Trans`, `db.Box`, `db.Layout` etc. with a fixed-seed PRNG. It
   serializes inputs + KLayout's results into JSON and writes them to
   `corpus/`.
2. **`klayout-validate`** is a pure-Rust crate. Its tests load the JSON
   corpus and assert klayout-rs produces the same outputs — exact integer
   equality, no tolerance.

CI does not need klayout.db installed: the corpus is checked in and the
Rust tests are self-contained. Regenerate the corpus only when:
- You add new oracle cases.
- You upgrade the reference KLayout version (and want to verify the new
  version against your impl).
- A test fails and you suspect the corpus is wrong (rare).

## Regenerating the corpus

```sh
# One-time: install klayout in a venv (do NOT use system pip)
python3 -m venv .venv
.venv/bin/pip install klayout

# Regenerate everything (klayout.db oracle)
.venv/bin/python validation/oracle.py

# Or just one suite
.venv/bin/python validation/oracle.py trans
.venv/bin/python validation/oracle.py bbox
.venv/bin/python validation/oracle.py gds
```

To regenerate against a **pinned KLayout built inside Docker** (same git ref as
`validation/docker/klayout-git-ref`), build the image and run the oracle with a
writable corpus dir — see [`validation/docker/README.md`](docker/README.md).

For formats KLayout can't read (Liberty, soon SPEF / LEF / DEF), the
oracle is OpenSTA / OpenDB run inside Docker:

```sh
# One-time: pull the image (~5GB, AMD64-only — runs under emulation
# on Apple Silicon).
docker pull --platform linux/amd64 openroad/orfs:latest

# Regenerate the Liberty corpus.
python3 validation/oracle_external.py liberty
```

CI does not need Docker — the corpus JSON is checked in.

## `density_window` combinatorial suite

`corpus/drc_density_grid.json` is a **KLayout-driven Cartesian product**
of tiling knobs (`padding_zero` / `padding_ignore`, `with`/`without` density,
`tile_origin`, `tile_count`, singleton frame, window vs step). It is **not**
the full continuous (coordinates × floats) space — expand `gen_drc_density_grid()`
in `oracle.py` when you need more coverage. Regenerate:

```sh
.venv/bin/python validation/oracle.py drc_density_grid
```

## Docker-only PDK + OpenROAD smoke (optional)

Heavyweight integration checks (network + OpenROAD-flow) live in
[`validation/docker/README.md`](docker/README.md):

```sh
./validation/docker/run_benchmark.sh pdk-smoke
./validation/docker/run_benchmark.sh openroad-gcd-synth
```

Rebuild the pinned-AMD64 oracle image with:

```sh
docker build -t klayout-rs-oracle:latest validation/docker/
```

## What's covered

| Suite                   | Cases    | What's verified                                                |
|-------------------------|----------|----------------------------------------------------------------|
| `trans.json`            | ~1500    | `Trans::apply`, `compose`, `inverse` vs `db.Trans`             |
| `bbox.json`             | ~2000    | `Bbox::union`, `intersection`, `contains`, `apply_bbox`        |
| `gds/*.gds` — reader    | 14 files | KLayout writes, our reader matches canonical dump              |
| writer — round-trip     | 11 cases | We write, KLayout reads, dumps match our in-memory canonical   |
| `region.json`           | 24 cases | Boolean ops + `size` vs `db.Region` (point-set equivalence)    |
| `drc.json` | 40 cases | DRC primitives + `density_window` fixtures vs `db.Region` checks / `TilingProcessor` |
| `drc_density_grid.json` | 1536 cases | `density_window` combinatorial grid (same oracle as `drc`) |
| `oasis/*.oas` — reader  | 3 files  | KLayout writes OASIS, our reader matches canonical dump        |
| `polygon_ops.json`      | 60 cases | `area`, `perimeter`, `bbox` vs `db.Polygon`                    |

Reader fixtures cover: single box, mixed shapes (polygon/path/text), hierarchical
SREFs with rotation+mirror, regular AREF, all 8 SREF orientations, every `PathCap`
variant, irregular polygons, multi-layer cells, 3-level hierarchy, empty cells,
rotated/mirrored AREF, multiple text labels, mixed shapes-and-instances.

Writer fixtures cover the same surface in the opposite direction, exercised
by writing GDS via `klayout-io` and verifying KLayout (subprocess) reads
back something that dumps identically to our in-memory canonical view.

## Conventions discovered along the way

These are real KLayout behaviors we matched after the corpus revealed
divergences. Documented here so we don't re-discover them later.

- **`Trans.rot`** is a single 0..7 code combining rotation and mirror
  (0..3 = r0/r90/r180/r270, 4..7 = m0/m45/m90/m135). The constructor
  takes `(rot, mirror, dx, dy)` separately but the field combines them.
- **`db.Box(0, 0, 0, 0)` is *not* empty.** Empty is reserved for the
  default-constructed `db.Box()` (sentinel left=right=bottom=top=0
  *combined with* internal flags — it's not directly representable as a
  4-tuple of coordinates). KLayout's `box.width()` on the empty sentinel
  returns u32-wrap garbage; ignore it.
- **KLayout writes `db.Box` shapes as `BOUNDARY` records** in GDS
  (5 closed points), not as `BOX` records. On read it reclassifies
  axis-aligned 4-vertex polygons back to `Box`. Our reader does the same.
- **KLayout writes `LIBNAME = "LIB"`** by default, even when the in-memory
  `layout.libname` is empty. (`libname` may not even be exposed as a
  Python attribute — `hasattr(layout, "libname")` was False on 0.30.8.)
  We don't compare LIBNAME in the GDS test.
- **`CellInstArray.a` / `.b`** are *not* the GDS COLROW columns / rows
  in the order you'd expect. After read, KLayout assigns:
    `a <- (XY[2] - origin) / na` — the GDS "Y-direction" / num_rows
    `b <- (XY[1] - origin) / nb` — the GDS "X-direction" / num_cols
  So `(a, na)` is the slow axis, `(b, nb)` is the fast one. Our
  validation maps `Repetition::Regular { col, row, n_cols, n_rows }`
  through `(a=row, b=col, na=n_rows, nb=n_cols)` to match.
- **GDS UNITS encoding precision.** `1e-6 / 1000.0 != 1e-9` in IEEE
  f64 (one ULP off). The naive form makes our writer encode bytes
  KLayout reads as `0.0009999999999999998` instead of `0.001`. We use
  `user_unit * 1e-6` instead, which is bit-exact for `dbu = 1000`.
- **Polygon canonicalization.** KLayout normalizes hulls to clockwise
  winding starting from the lowest-y/lowest-x vertex on database insert.
  Our `Polygon::from_hull` does the same so cross-tool comparisons see
  byte-equal vertex sequences. `from_hull_raw` skips this for the rare
  case where vertex order matters.
- **Path extension reporting.** KLayout reports effective endpoint
  extensions as `width/2` for `PATHTYPE 1` (round) and `PATHTYPE 2`
  (extended-square), and the user-set values for `PATHTYPE 4` (custom).
  Our canonical dump derives extensions the same way for parity.
- **Region topology canonicalisation.** `i_overlay` produces disjoint
  simple polygons by default. KLayout's boolean ops produce
  polygon-with-holes (XOR of overlapping rects = one polygon-with-hole
  whose hull traces the union outline and whose hole is the overlap).
  The validation harness's `klayoutize` pass detects pairs of disjoint
  polygons that share two vertices and merges them by stitching outer
  arcs into a hull and inner arcs into a hole — producing KLayout's
  representation byte-for-byte. Compared with strict vertex equality.
- **Hole winding convention.** Our `Polygon` stores hulls as CW and holes
  as CCW so winding-number rules (`FillRule::NonZero`) correctly subtract
  holes. `Polygon::add_hole` normalizes input hole windings; raw
  `polygon.holes.push(...)` skips normalization and is unsafe.
- **`db.Region.sized` mode.** Mode 0 (Acute) and mode 1 (Square)
  *chamfer* corners. Mode 2 (Octagon) and mode 3 (Round) preserve sharp
  corners on axis-aligned input. Our `SizeJoin::Miter` matches mode 2/3.
- **DRC width uses exact edge-pair analysis** (matches KLayout).
  Even-DBU thresholds work correctly — `width(p, 10)` with a polygon
  of width exactly 10 returns empty, matching KLayout's strict
  less-than. Concave-corner triangles are emitted via the
  `concave_corners` cross-product test plus chamfer-length extension,
  matching KLayout's edge-pair output vertex-for-vertex.
- **DRC chamfer formula.** For `enclosing`, `overlap`, and concave-corner
  width extensions, KLayout uses `round(sqrt(min² - dist²))` as the
  parallel-axis chamfer length: the chamfer endpoint sits at perpendicular
  distance exactly `min` from the inner edge endpoint. Now matches.
- **OASIS plist-type 4 g-delta encoding.** Form-1 (raw bit 0 = 0):
  octangular delta, `dir = (raw >> 1) & 7`, `mag = raw >> 4`. Form-2
  (raw bit 0 = 1): non-octangular delta, `dx_sign = (raw >> 1) & 1`,
  `dx_mag = raw >> 2`, then a signed-int for dy. Form-2 only valid in
  type 4 / type 5 lists. KLayout switches to type-4 the moment any
  edge is non-octangular; reader must handle both forms.
- **OASIS manhattan polygon implicit close.** Type 0/1 lists encode
  `count = N - 2` deltas; the last 2 edges are implicit (perpendicular,
  closing back to start). Reader materialises one extra delta to land
  on vertex N-1; the close back to vertex 0 is implicit in the hull.
- **OASIS PROPERTY info-byte UUUUVCNS.** C[2] = 1 means propname is
  *explicit* (follows), not modal — a common mis-read of the spec.
  N[1] selects refnum vs name-string when C=1. V[3] = 1 means modal
  value-list (no values follow).
- **OASIS PLACEMENT info-byte CNXYRAAF.** C[7] = 1 means name is
  explicit; N[6] (only meaningful when C=1) selects refnum vs
  name-string. The two bits aren't independent "string" / "refnum"
  alternatives.
- **DRC concave-corner extensions.** At inside corners (interior angle
  > 180°), KLayout's edge-pair check extends width violations along the
  longer edge by `chamfer_len` past the projection — capturing the
  triangle-shaped near-corner region that erosion-based DRC misses.
  Detection: vertices where the cross product of incoming × outgoing
  edge vectors is positive (left turn on a CW polygon).

## Layout

```
validation/
├── README.md                   # this file
├── oracle.py                   # generates the corpus from klayout.db
├── corpus/                     # checked-in fixtures (regenerable)
│   ├── trans.json
│   ├── bbox.json
│   └── gds/
│       ├── single_box.{gds,json}
│       ├── mixed.{gds,json}
│       ├── hierarchy.{gds,json}
│       └── aref.{gds,json}
└── klayout-validate/           # pure-Rust validator crate
    ├── Cargo.toml
    └── tests/
        ├── trans.rs
        ├── bbox.rs
        └── gds.rs
```
