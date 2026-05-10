# Citations

Every non-trivial algorithm in this workspace traces back to a paper.
This file maps each module to its source publication. When a module
implements a simplification or variant of the cited algorithm, the
caveat is noted alongside.

If you cite this codebase in academic work, please cite the underlying
algorithm authors as well — they did the substantive theoretical work;
we provide a Rust implementation.

---

## Geometry — `klayout-geom`

| Operation | Algorithm | Reference |
|-----------|-----------|-----------|
| Boolean ops (union, intersection, difference, XOR) | Bentley–Ottmann sweep over polygon edges | Bentley & Ottmann 1979, *Algorithms for reporting and counting geometric intersections*, IEEE TC-28(9). Implementation via the [`i_overlay`](https://crates.io/crates/i_overlay) crate. |
| Polygon simplification (open + closed polylines) | Ramer–Douglas–Peucker iterative point-elimination | Ramer 1972, *An iterative procedure for the polygonal approximation of plane curves*, CGIP 1(3); Douglas & Peucker 1973, *Algorithms for the reduction of the number of points required to represent a digitized line or its caricature*, Cartographica 10(2). |
| Sizing (Minkowski-dilate by a disk approximated as a square) | Minkowski sum of axis-aligned regions | Standard computational-geometry result; see e.g. Berg et al., *Computational Geometry: Algorithms and Applications*, 3rd ed., §13.3. |

## Spatial indexing — `klayout-spatial`

| Index | Algorithm | Reference |
|-------|-----------|-----------|
| Grid bucketing | Fixed-cell uniform grid | Folklore; coverage in Sedgewick & Wayne, *Algorithms*, 4th ed., §4.4. |
| R-tree (`RTreeIndex`) | R*-tree with bulk loading | Beckmann, Kriegel, Schneider, Seeger 1990, *The R*-tree: an efficient and robust access method for points and rectangles*, SIGMOD '90. Implementation via the [`rstar`](https://crates.io/crates/rstar) crate. |

## DRC — `klayout-drc`

| Rule | Algorithm | Reference |
|------|-----------|-----------|
| `width`, `space`, `separation` (axis-aligned) | Edge-pair distance check matching KLayout's `width_check` / `space_check` semantics | Klein 2017–, [KLayout Reference: Region](https://www.klayout.de/doc-qt5/code/class_Region.html) — the C++ engine in `db::Region` is the reference oracle. |
| `enclosing`, `overlap` | Edge-pair check with chamfer extensions for endpoint corners | Same reference; chamfer formula reverse-engineered to match KLayout's `enclosing_check` / `overlap_check` byte-exactly on the 18-case validation corpus. |
| `width` per-polygon edge enumeration | Orientation-bucketed bisection (`O(E·log E)` instead of `O(E²)`) | Original to this codebase; reduces a constant factor on highly-fractured polygons. The cross-polygon spatial index uses [`klayout-spatial`](klayout-spatial) atop `rstar`. |
| `density` window scan | Sliding-window area integral | Folklore; see KLayout's `compute_density` for the reference behavior. |
| Concave-corner detection | Cross-product sign test on consecutive hull edges | Standard computational-geometry result. |
| Density fill, OPC, line-end protection (`density_fill`, `opc`, `lfd`) | Pattern-matching v1 implementations | Internal heuristics; not yet validated against an industry-standard rule deck. |

## I/O — `klayout-io`

| Format | Reference |
|--------|-----------|
| GDSII | Ohnishi, *GDS II Stream Format Manual*, Calma Co. 1987; Cadence Stream Format documentation. |
| OASIS | SEMI Standard P39, *Open Artwork System Interchange Standard*. Implementation validated for reader and writer against KLayout 0.30.8 on a 26-case corpus. |
| CIF, MAG, DXF | Mead/Conway 1980 (CIF); Berkeley Magic manual (MAG); Autodesk DXF Reference (DXF). v1 readers; round-trip not validated. |

## Routing — `klayout-route`

| Algorithm | Reference |
|-----------|-----------|
| **RSMT** (Rectilinear Steiner Minimum Tree) via Iterated 1-Steiner | Kahng & Robins 1992, *A new class of iterative Steiner tree heuristics with good performance*, IEEE TCAD 11(7). The Hanan-grid theorem (every Steiner-optimal point lies on `(x_i, y_j)` for some pin `x_i`, `y_j`) is from Hanan 1966, *On Steiner's problem with rectilinear distance*, SIAM J. Appl. Math. 14(2). |
| **Pathfinder** global router (negotiated congestion, history cost, rip-up-and-reroute) | McMurchie & Ebeling 1995, *PathFinder: a negotiation-based performance-driven router for FPGAs*, ACM/SIGDA International Symposium on FPGAs (FPGA '95). |
| Single-net A* on a 3-D GCell graph | Hart, Nilsson, Raphael 1968, *A formal basis for the heuristic determination of minimum cost paths*, IEEE TSSC 4(2). Manhattan heuristic with bend penalty. |
| Power-grid routing (rings + stripes) | Singh & Sapatnekar 2008, *Robust early P/G routing for high-performance microprocessors*, ASP-DAC '08. v1 implements the ring + uniform-stripe variant only. |
| Antenna check + jumper insertion | Su, Ho, Chen 2013, *Antenna-aware routing*, ACM TODAES 18(3). v1 evaluates ratio violations and inserts up-jumper diodes; full antenna effect modeling (per-area + per-perimeter, with diode/area-cap canceling) is partial. |
| Crosstalk shielding | Sundaresan & Mahapatra 2007, *Crosstalk-aware routing in nanometer technologies*, IET TCAS. v1 inserts shield wires for high-aggressor edges only. |
| **Electromigration** MTTF model | Black 1969, *Electromigration—a brief survey and some recent results*, IEEE TED 16(4). Per-segment current density `J = I/(W·T)` against per-layer `J_max`. v1 takes a single DC current per net; AC/RMS/peak distinctions deferred. |

## Placement — `klayout-place`

| Algorithm | Reference |
|-----------|-----------|
| **Bound-to-bound (B2B) quadratic placement** with conjugate-gradient solve | Viswanathan & Chu 2005, *FastPlace: efficient analytical placement using cell shifting, iterative local refinement and a hybrid net model*, ISPD '05. The B2B net model linearizes HPWL into weighted-quadratic springs whose minimum coincides with HPWL's L1 minimum; CG on the resulting sparse Laplacian is from Hestenes & Stiefel 1952, *Methods of conjugate gradients for solving linear systems*, J. Res. NBS 49(6). |
| Force-directed global placement | Quinn & Breuer 1979, *A force directed component placement procedure for printed circuit boards*, IEEE TCAS 26(6). The classical pre-quadratic technique. |
| Tetris legalization | Hill 2002, *Method and system for high speed detailed placement of cells within an integrated circuit design*, US Patent 6,370,673. |
| Detailed pairwise-swap improvement | Caldwell, Kahng, Markov 2000, *Optimal partitioners and end-case placers for standard-cell layout*, IEEE TCAD 19(11). |
| Pin optimization (face-side selection + collision avoidance) | Internal heuristic; analogous to Cadence Innovus's `placePIN` algorithm at a high level. |

## CTS — `klayout-cts`

| Algorithm | Reference |
|-----------|-----------|
| **Deferred-Merge Embedding (DME)** with merging-segment construction in rotated `(u, v)` coordinates | Chao, Hsu, Ho 1992, *Zero skew clock net routing*, DAC '92; Edahiro 1993, *A clustering-based optimization algorithm in zero-skew routings*, DAC '93. The rotated-coordinate trick (turning L1 perpendicular bisector into L∞ axis-aligned) is from Cong, Kahng, Koh, Tsao 1996, *Bounded-skew clock and Steiner routing*, ACM TODAES 1(3). |
| H-tree synthesis | Bakoglu 1990, *Circuits, Interconnections, and Packaging for VLSI*, Addison-Wesley, §8.3. The recursive-bisection method-of-means we ship is the textbook H-tree. |
| Buffer insertion (van Ginneken Pareto pruning) | van Ginneken 1990, *Buffer placement in distributed RC-tree networks for minimal Elmore delay*, ISCAS '90. |

## Connectivity & extraction — `klayout-connect`

| Operation | Algorithm | Reference |
|-----------|-----------|-----------|
| Net extraction (poly merge per layer + via stitching across layers) | Standard inverse-flat-flatten flow | KLayout's `extract_nets`; classic CAD-tool pipeline. |
| MOSFET device recognition | `gates = poly ∩ diff`, `S/D = diff − gates` pattern | Mead & Conway 1980, *Introduction to VLSI Systems*; standard practice in Magic/Cadence/KLayout. |
| LVS structural matching | Iterated name + signature comparison with VF2 fallback | Cordella, Foggia, Sansone, Vento 2001, *An improved algorithm for matching large graphs*, IAPR-TC15 Workshop on Graph-based Representations. The simplified VF2 we ship omits the canonical T1/T2 candidate-set lookahead pruning; full VF2 with lookahead is the next algorithmic upgrade. |
| LVS parameter tolerance | `|a−b| ≤ abs_tol + rel_tol·max(|a|,|b|)` rule | Standard practice in industry LVS tools (Calibre, Assura, Pegasus). |
| Flat parasitic extraction (R + ground C + coupling C) | Pattern-matched 2-D | Boyer, Wang, Goyal, Lasdon 1983, *A boundary-element method for capacitance computation*, IEEE TCAD 2(2). v1 uses bbox-resistance + edge-pair-coupling approximation; not a field solver. |
| 2.5-D multilayer PEX (inter-metal area cap + via R) | Pattern-matched layer-stack model | Berkelaar 1992, *Statistical delay calculation, a linear time method*, ACM TACAS '92, supplementary. v1 uses parallel-plate area + lumped via resistance. Real 2.5-D extractors (StarRC, Calibre xRC) solve Maxwell on a 3-D mesh. |

## Static timing analysis — `klayout-sta`

| Component | Algorithm | Reference |
|-----------|-----------|-----------|
| Forward / backward propagation, slack | Topological-order arrival accumulation | Hitchcock, Smith, Cheng 1982, *Timing analysis of computer hardware*, IBM J. Res. Dev. 26(1). |
| **NLDM** 2-D bilinear table lookup | Synopsys Liberty Reference Manual; standard in every commercial STA tool. |
| **CPPR** (Common-Path Pessimism Removal) via clock-tree LCA | Sapatnekar 2004, *Timing*, Springer, §7.3; CPPR formalization in Hu, Sinha, Keller 2014, *Common-path pessimism removal in static timing analysis*, IEEE TCAD 33(5). |
| AOCV / POCV (advanced / parametric on-chip variation) derate | Liberty `ocv_derate` / `ocv_sigma_*` model semantics; Synopsys white paper, *AOCV: a new methodology for on-chip variation timing analysis*, 2010. |
| Topological sort | Kahn 1962, *Topological sorting of large networks*, CACM 5(11). |
| Hold-time check | Standard delay-test theory; see Bhasker & Chadha 2009, *Static Timing Analysis for Nanometer Designs*, Springer, ch. 4. |

## Liberty parsing — `klayout-liberty`

| Component | Reference |
|-----------|-----------|
| Liberty grammar | Synopsys, *Library Compiler User Guide and Reference Manual*, current edition. v1 covers cell-level + arc-level NLDM tables; CCS is parsed-but-ignored. |
| 2-D bilinear table lookup | Folklore; standard interpolation. |

## LEF / DEF parsing — `klayout-lef`

| Component | Reference |
|-----------|-----------|
| LEF / DEF grammar | Cadence, *LEF/DEF Language Reference*, version 5.8. v1 covers macros, layers, vias, sites, and the DEF placement / routing payloads exercised by a typical digital flow. |

## Validation methodology — `validation/`

| Component | Reference |
|-----------|-----------|
| Differential validation against a reference oracle | Yang, Chen, Avgerinos, Brumley 2011, *Symbolic execution of multithreaded programs from arbitrary program contexts*, OOPSLA '11 — methodology framing. The specific "single-implementation oracle" pattern we use is folklore; e.g., `csmith` for C compilers, `differential-testing` for OS kernels. The oracle is KLayout's C++ engine via `klayout.db` Python bindings; the corpus is checked in so CI doesn't depend on the oracle being installed. |

---

## Citing this work

If you reference any of these algorithms via this codebase, please cite
both the underlying paper and (optionally) this repository for the
implementation. Example BibTeX:

```bibtex
@inproceedings{ChaoHsuHo1992,
  author    = {Ting-Hai Chao and Yu-Chin Hsu and Jan-Ming Ho},
  title     = {Zero skew clock net routing},
  booktitle = {Proceedings of the 29th ACM/IEEE Design Automation Conference},
  year      = {1992},
  pages     = {518--523},
}

@misc{KlayoutRs,
  title = {klayout-rs: a Rust workspace of EDA primitives},
  note  = {https://github.com/MIT-RLX/klayout-rs},
  year  = {2026},
}
```
