//! End-to-end flow demo.
//!
//! Walks a tiny synthetic design through every major stage of the
//! workspace:
//!
//! ```text
//!   1. Build a synthetic netlist (one clock + 8 flops + combinational fanout).
//!   2. Quadratic global placement (B2B + CG).
//!   3. Legalize cells onto a row site grid.
//!   4. Build a clock-tree with DME (zero-skew under unit-wire delay).
//!   5. RSMT topology for the data net.
//!   6. Pathfinder global routing on a coarse GCell grid.
//!   7. DRC width check on the resulting wire geometry.
//!   8. Build a timing graph, propagate arrival, report worst slack.
//!   9. Write the placed + routed cell as GDSII.
//! ```
//!
//! Run with:
//!
//! ```bash
//! cargo run --example end_to_end --release
//! ```
//!
//! Output lands under `/tmp/klayout-rs-demo/` — the GDS, an HPWL
//! report, the clock-tree skew, and the worst-slack timing report.
//!
//! The design is intentionally small (16 cells, ≤32 net edges) so
//! the demo runs in well under a second on a laptop. The point is
//! not to validate algorithm quality — that's what the per-crate
//! benchmarks and the validation suite are for — but to prove every
//! stage's API actually composes into a working flow.

use klayout::core::{
    Bbox, CellBuilder, Instance, LayerInfo, Library, Path as CorePath, Point, Rect, Trans, Vec2,
};
use klayout::cts::{
    synthesise_clock_tree_dme, BranchChild, ClockSink, ClockTree, DmeConfig,
};
use klayout::drc::width;
use klayout::geom::Region;
use klayout::io::write_gds_path;
use klayout::place::{
    legalize, quadratic_place, types::*, QuadraticConfig,
};
use klayout::route::{
    rsmt, route_pathfinder, CapacityGrid, GCellGrid, PathfinderConfig, PathfinderRequest,
};
use klayout::sta::{
    arrival::{compute_arrivals, compute_required, compute_slacks, topo_sort},
    graph::{EdgeKind, NodeId, NodeKind, TimingGraph},
};
use smol_str::SmolStr;
use std::path::PathBuf;

// Tiny PDK: three layers — diffusion, metal-1, metal-2.
const M1_GDS_LAYER: u16 = 1;
const M2_GDS_LAYER: u16 = 2;

const CELL_WIDTH_DBU: i64 = 100;
const CELL_HEIGHT_DBU: i64 = 200;
const ROW_SITE_DBU: i64 = 10;
const FLOORPLAN_W_DBU: i64 = 4_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from("/tmp/klayout-rs-demo");
    std::fs::create_dir_all(&out_dir)?;

    println!("=== klayout-rs end-to-end demo ===");

    // ------------------------------------------------------------------
    // 1. Build a synthetic netlist + floorplan.
    //
    // 8 flip-flops + 1 input buffer + 1 inverter chain + 1 clock buffer
    // = 16 logical instances. The "design" is a shift-register-like
    // pattern: clk → ff0 → ff1 → ... → ff7, with combinational logic
    // between every other stage.
    // ------------------------------------------------------------------
    let lib = Library::new("demo_top", 1000); // dbu = 1nm
    let m1 = lib.layer(LayerInfo::named("M1", M1_GDS_LAYER, 0));
    let m2 = lib.layer(LayerInfo::named("M2", M2_GDS_LAYER, 0));

    let mut p = Placement::new();

    // Two rows of legal placement.
    for r in 0..2 {
        p.add_row(Row {
            origin: Point::new(0, r * CELL_HEIGHT_DBU),
            site_width: ROW_SITE_DBU,
            num_sites: (FLOORPLAN_W_DBU / ROW_SITE_DBU) as u32,
            height: CELL_HEIGHT_DBU,
        });
    }

    // Cells. Index 0 is the clock-input pad (fixed at left edge).
    let clk_pad_idx = p.add_cell(Cell {
        name: "clk_pad".into(),
        width: CELL_WIDTH_DBU,
        height: CELL_HEIGHT_DBU,
        position: Point::new(0, 0),
        fixed: true,
    });
    let din_pad_idx = p.add_cell(Cell {
        name: "din_pad".into(),
        width: CELL_WIDTH_DBU,
        height: CELL_HEIGHT_DBU,
        position: Point::new(0, CELL_HEIGHT_DBU),
        fixed: true,
    });

    // 8 flip-flops + interleaved inverters.
    let mut ff_idx = Vec::with_capacity(8);
    let mut inv_idx = Vec::with_capacity(7);
    for i in 0..8 {
        ff_idx.push(p.add_cell(Cell {
            name: SmolStr::from(format!("ff{i}")),
            width: CELL_WIDTH_DBU,
            height: CELL_HEIGHT_DBU,
            // Initial position: spread across the floorplan so the
            // optimizer has somewhere meaningful to start.
            position: Point::new((i + 2) * 350, (i % 2) * CELL_HEIGHT_DBU),
            fixed: false,
        }));
        if i < 7 {
            inv_idx.push(p.add_cell(Cell {
                name: SmolStr::from(format!("inv{i}")),
                width: CELL_WIDTH_DBU,
                height: CELL_HEIGHT_DBU,
                position: Point::new((i + 2) * 350 + 150, (i % 2) * CELL_HEIGHT_DBU),
                fixed: false,
            }));
        }
    }
    let dout_pad_idx = p.add_cell(Cell {
        name: "dout_pad".into(),
        width: CELL_WIDTH_DBU,
        height: CELL_HEIGHT_DBU,
        position: Point::new(FLOORPLAN_W_DBU - CELL_WIDTH_DBU, 0),
        fixed: true,
    });

    // Clock net: pad → every flip-flop's clock pin. High weight so the
    // placer pulls flops near the clock-pad column.
    let mut clk_net_pins = vec![clk_pad_idx];
    clk_net_pins.extend(ff_idx.iter().copied());
    p.add_net(Net {
        name: "clk".into(),
        cell_indices: clk_net_pins.clone(),
        weight: 4.0,
    });

    // Data path: din → ff0 → inv0 → ff1 → inv1 → ... → ff7 → dout.
    let mut prev = din_pad_idx;
    for i in 0..8 {
        p.add_net(Net {
            name: SmolStr::from(format!("d{i}")),
            cell_indices: vec![prev, ff_idx[i]],
            weight: 1.0,
        });
        if i < 7 {
            p.add_net(Net {
                name: SmolStr::from(format!("q{i}")),
                cell_indices: vec![ff_idx[i], inv_idx[i]],
                weight: 1.0,
            });
            prev = inv_idx[i];
        } else {
            p.add_net(Net {
                name: "dout_n".into(),
                cell_indices: vec![ff_idx[i], dout_pad_idx],
                weight: 1.0,
            });
        }
    }

    let hpwl_initial = p.hpwl();
    println!("[1] netlist built: {} cells, {} nets, HPWL = {hpwl_initial}", p.cells.len(), p.nets.len());

    // ------------------------------------------------------------------
    // 2. Quadratic global placement.
    // ------------------------------------------------------------------
    quadratic_place(&mut p, &QuadraticConfig::default());
    let hpwl_after_qp = p.hpwl();
    println!(
        "[2] quadratic placement: HPWL {hpwl_initial} → {hpwl_after_qp} ({:+.1}%)",
        100.0 * (hpwl_after_qp - hpwl_initial) as f64 / hpwl_initial.max(1) as f64
    );

    // ------------------------------------------------------------------
    // 3. Legalize.
    // ------------------------------------------------------------------
    legalize(&mut p);
    let hpwl_after_legal = p.hpwl();
    println!(
        "[3] legalize: HPWL → {hpwl_after_legal} (delta {:+})",
        hpwl_after_legal - hpwl_after_qp
    );

    // ------------------------------------------------------------------
    // 4. CTS via DME.
    // ------------------------------------------------------------------
    let clk_source = Point::new(
        p.cells[clk_pad_idx].position.x + CELL_WIDTH_DBU,
        p.cells[clk_pad_idx].position.y + CELL_HEIGHT_DBU / 2,
    );
    let clk_sinks: Vec<ClockSink> = ff_idx
        .iter()
        .map(|&i| ClockSink {
            name: p.cells[i].name.clone(),
            at: Point::new(
                p.cells[i].position.x + CELL_WIDTH_DBU / 2,
                p.cells[i].position.y + CELL_HEIGHT_DBU / 2,
            ),
        })
        .collect();
    let cts: ClockTree = synthesise_clock_tree_dme(clk_source, &clk_sinks, &DmeConfig::default());
    println!(
        "[4] CTS DME: {} buffers, skew {} DBU, total wire {} DBU",
        cts.buffer_count,
        cts.skew(),
        cts.total_wire_length()
    );

    // ------------------------------------------------------------------
    // 5. RSMT for the longest data net.
    //    (Pathfinder routes 2-pin nets natively; we use RSMT to
    //    decompose any multi-pin net into 2-pin requests.)
    // ------------------------------------------------------------------
    let data_pins: Vec<Point> = ff_idx
        .iter()
        .map(|&i| {
            Point::new(
                p.cells[i].position.x + CELL_WIDTH_DBU / 2,
                p.cells[i].position.y + CELL_HEIGHT_DBU / 2,
            )
        })
        .collect();
    let rsmt_tree = rsmt(&data_pins);
    println!(
        "[5] RSMT for the 8 FF positions: {} Steiner points, total length {} DBU",
        rsmt_tree.steiner_count(),
        rsmt_tree.total_length()
    );

    // ------------------------------------------------------------------
    // 6. Pathfinder global routing for the data path.
    // ------------------------------------------------------------------
    let bbox = Bbox::new(
        Point::new(0, 0),
        Point::new(FLOORPLAN_W_DBU, 2 * CELL_HEIGHT_DBU),
    );
    let gcell = 200i64;
    let grid = GCellGrid::new(bbox, gcell, gcell);
    let mut cap = CapacityGrid::uniform(&grid, 8);

    // 2-pin requests across consecutive data-path stages.
    let mut reqs: Vec<PathfinderRequest> = Vec::new();
    let stages = std::iter::once(din_pad_idx)
        .chain(ff_idx.iter().copied())
        .chain(std::iter::once(dout_pad_idx))
        .collect::<Vec<_>>();
    for w in stages.windows(2) {
        let a = &p.cells[w[0]];
        let b = &p.cells[w[1]];
        reqs.push(PathfinderRequest {
            net_name: SmolStr::from(format!("d_{}_{}", a.name, b.name)),
            src: Point::new(a.position.x + CELL_WIDTH_DBU / 2, a.position.y + CELL_HEIGHT_DBU / 2),
            sink: Point::new(b.position.x + CELL_WIDTH_DBU / 2, b.position.y + CELL_HEIGHT_DBU / 2),
        });
    }
    let pf = route_pathfinder(&grid, &mut cap, &reqs, &PathfinderConfig::default());
    println!(
        "[6] Pathfinder: {} routes in {} iterations, residual overflow {}",
        pf.routes.len(),
        pf.iterations,
        pf.residual_overflow
    );

    // ------------------------------------------------------------------
    // 7. Materialize routed wires as Path shapes on M1, then run
    //    DRC width to confirm the routed centerlines are non-degenerate.
    // ------------------------------------------------------------------
    let wire_w = 50i64; // 0.05 µm
    let mut top = CellBuilder::new("demo_top");

    // Cell instances: use a small rect-stub layout for each cell on M1.
    for c in &p.cells {
        let bb = Bbox::new(
            c.position,
            Point::new(c.position.x + c.width, c.position.y + c.height),
        );
        top.add_shape(m1, Rect::new(bb));
    }

    // Routed centerlines on M2.
    for route in &pf.routes {
        if route.gcells.len() < 2 {
            continue;
        }
        let centers: Vec<Point> = route.gcells.iter().map(|g| grid.center(*g)).collect();
        let path = CorePath::new(centers.iter().copied(), wire_w);
        top.add_shape(m2, path);
    }

    // Clock-tree wire visualisation: parent-to-child Manhattan paths
    // on M2. Detour wires are reported in CTS but not materialised
    // (the detailed router would emit the snake-routing geometry; we
    // skip it for the demo).
    for branch in &cts.branches {
        for child in &branch.children {
            let dst = match child {
                BranchChild::Branch(id) => cts.branches[id.0 as usize].at,
                BranchChild::Sink(_, p) => *p,
            };
            let path = CorePath::new([branch.at, dst].iter().copied(), wire_w);
            top.add_shape(m2, path);
        }
    }

    let top_id = lib.insert(top);
    println!("[7] geometry written into top cell");

    // DRC width on M2: extract a Region of the routed shapes and run
    // the indexed-edge width kernel against the per-layer min width.
    let routing_region = Region::from_cell_layer(&lib, top_id, m2);
    let min_w = wire_w; // require every wire to be ≥ wire_w wide; should pass
    let viols = width(&routing_region, min_w);
    println!(
        "[7b] DRC width(min={min_w}) on M2: {} violation polygons (expect 0 since centerlines have width = {min_w})",
        viols.len()
    );

    // ------------------------------------------------------------------
    // 8. Build a small timing graph for the data path and report slack.
    // ------------------------------------------------------------------
    let mut tg = TimingGraph::new();
    let din_node = tg.add_node("din_pad/Y", NodeKind::PrimaryInput);
    let mut prev_q = din_node;
    let mut ff_q_nodes: Vec<NodeId> = Vec::new();
    for i in 0..8 {
        let ff = &p.cells[ff_idx[i]];
        let d = tg.add_node(format!("{}/D", ff.name), NodeKind::CellInput);
        let q = tg.add_node(format!("{}/Q", ff.name), NodeKind::CellOutput);
        // Net delay (1 unit / DBU stand-in).
        let from_pos = match prev_q.0 {
            0 => Point::new(0, 0),
            _ => {
                let drv = ff_idx[(i.max(1)) - 1];
                Point::new(p.cells[drv].position.x, p.cells[drv].position.y)
            }
        };
        let net_delay = manhattan(from_pos, ff.position) as f64 * 0.001; // ps per DBU
        tg.add_edge(prev_q, d, EdgeKind::Net, net_delay);
        // Cell arc D→Q (placeholder fixed delay).
        tg.add_edge(d, q, EdgeKind::CellArc, 0.05);
        ff_q_nodes.push(q);
        prev_q = q;
    }
    let dout_node = tg.add_node("dout_pad/A", NodeKind::PrimaryOutput);
    tg.add_edge(prev_q, dout_node, EdgeKind::Net, 0.01);
    tg.finalize();

    // 1 ns clock period as the required-time budget at the output.
    let arrivals = compute_arrivals(&tg, &[(din_node, 0.0)])?;
    let requireds = compute_required(&tg, &[(dout_node, 1.0)], 1.0)?;
    let slacks = compute_slacks(&arrivals, &requireds);
    let order = topo_sort(&tg)?;
    let worst = order
        .iter()
        .map(|n| (slacks[n.0 as usize], *n))
        .filter(|(s, _)| s.is_finite())
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if let Some((slack, n)) = worst {
        println!(
            "[8] STA: worst-slack node = {} ({slack:.4} ns)",
            tg.nodes[n.0 as usize].name
        );
    }

    // ------------------------------------------------------------------
    // 9. Write GDS.
    // ------------------------------------------------------------------
    let gds_path = out_dir.join("demo_top.gds");
    write_gds_path(&lib, &gds_path)?;
    println!("[9] GDS written to {}", gds_path.display());

    println!("=== done ===");
    let _ = (Trans::IDENTITY, Vec2::new(0, 0), Instance::new); // silence unused-import lint if reorganized
    Ok(())
}

fn manhattan(a: Point, b: Point) -> i64 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}
