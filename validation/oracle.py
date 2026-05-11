#!/usr/bin/env python3
"""Generate validation corpus by exercising klayout.db.

Usage:
    python oracle.py            # regenerate everything in ../corpus/
    python oracle.py trans      # just trans.json
    python oracle.py bbox       # just bbox.json
    python oracle.py gds        # just gds/*

Determinism: random cases use a fixed seed so the corpus diff is meaningful
when KLayout's behavior changes (which would be the only reason to regenerate).

Corpus output directory defaults to ``validation/corpus/``. Override with
``ORACLE_CORPUS_DIR=/abs/path`` (used by the pinned-KLayout Docker image to
write into a temp mount).
"""
from __future__ import annotations

import json
import os
import random
import sys
from pathlib import Path

import klayout.db as db

# Override for Docker / temp-dir oracle runs: `ORACLE_CORPUS_DIR=/out oracle.py drc`
_CORPUS_DEFAULT = Path(__file__).resolve().parent / "corpus"
CORPUS = Path(os.environ.get("ORACLE_CORPUS_DIR", str(_CORPUS_DEFAULT))).resolve()
SEED = 0xC0FFEE
N_RANDOM = 500


def trans_to_arr(t: db.Trans) -> list[int]:
    """Encode as [rot_combined_0..7, dx, dy].

    KLayout combines rotation + mirror into a single 0..7 code:
      0..3 = r0/r90/r180/r270, 4..7 = m0/m45/m90/m135.
    """
    return [int(t.rot), int(t.disp.x), int(t.disp.y)]


def random_trans() -> db.Trans:
    rot_combined = random.randint(0, 7)
    rot = rot_combined % 4
    mirror = rot_combined >= 4
    dx = random.randint(-1_000_000, 1_000_000)
    dy = random.randint(-1_000_000, 1_000_000)
    return db.Trans(rot, mirror, dx, dy)


def random_point() -> db.Point:
    return db.Point(
        random.randint(-1_000_000, 1_000_000),
        random.randint(-1_000_000, 1_000_000),
    )


def random_box() -> db.Box:
    a = random_point()
    b = random_point()
    return db.Box(a, b)


# ---------- trans corpus ----------

def gen_trans() -> dict:
    random.seed(SEED)
    apply_cases = []
    # All 8 orientations applied to a few well-known points
    for rot_c in range(8):
        rot = rot_c % 4
        mirror = rot_c >= 4
        t = db.Trans(rot, mirror, 0, 0)
        for p in [(1, 0), (0, 1), (3, 5), (-7, 2), (10, -10), (0, 0)]:
            r = t.trans(db.Point(*p))
            apply_cases.append({
                "trans": [rot_c, 0, 0],
                "point": list(p),
                "result": [r.x, r.y],
            })
    # Random transforms with translation
    for _ in range(N_RANDOM):
        t = random_trans()
        p = random_point()
        r = t.trans(p)
        apply_cases.append({
            "trans": trans_to_arr(t),
            "point": [p.x, p.y],
            "result": [r.x, r.y],
        })

    compose_cases = []
    for _ in range(N_RANDOM):
        a, b = random_trans(), random_trans()
        r = a * b
        compose_cases.append({
            "a": trans_to_arr(a),
            "b": trans_to_arr(b),
            "result": trans_to_arr(r),
        })

    inverse_cases = []
    for _ in range(N_RANDOM):
        t = random_trans()
        r = t.inverted()
        inverse_cases.append({
            "trans": trans_to_arr(t),
            "result": trans_to_arr(r),
        })

    return {
        "klayout_version": db.__version__ if hasattr(db, "__version__") else "unknown",
        "apply_cases": apply_cases,
        "compose_cases": compose_cases,
        "inverse_cases": inverse_cases,
    }


# ---------- bbox corpus ----------

def box_to_arr(b: db.Box) -> list[int] | None:
    if b.empty():
        return None
    return [b.left, b.bottom, b.right, b.top]


def gen_bbox() -> dict:
    random.seed(SEED + 1)
    union_cases = []
    intersect_cases = []
    contains_cases = []
    transform_cases = []

    for _ in range(N_RANDOM):
        a = random_box()
        b = random_box()
        union_cases.append({
            "a": box_to_arr(a),
            "b": box_to_arr(b),
            "result": box_to_arr(a + b),
        })
        intersect_cases.append({
            "a": box_to_arr(a),
            "b": box_to_arr(b),
            "result": box_to_arr(a & b),
        })

    for _ in range(N_RANDOM):
        b = random_box()
        p = random_point()
        contains_cases.append({
            "box": box_to_arr(b),
            "point": [p.x, p.y],
            "result": bool(b.contains(p)),
        })

    for _ in range(N_RANDOM):
        t = random_trans()
        b = random_box()
        r = t.trans(b)
        transform_cases.append({
            "trans": trans_to_arr(t),
            "box": box_to_arr(b),
            "result": box_to_arr(r),
        })

    # Edge cases
    edge = [
        db.Box(),                  # default-constructed (empty)
        db.Box(0, 0, 0, 0),        # point box
        db.Box(0, 0, 10, 0),       # zero-height
        db.Box(5, 5, 5, 10),       # zero-width
        db.Box(0, 0, 1_000_000, 1_000_000),
    ]
    edge_cases = [{"box": box_to_arr(b), "empty": b.empty(),
                   "width": b.width(), "height": b.height()} for b in edge]

    return {
        "klayout_version": db.__version__ if hasattr(db, "__version__") else "unknown",
        "union_cases": union_cases,
        "intersect_cases": intersect_cases,
        "contains_cases": contains_cases,
        "transform_cases": transform_cases,
        "edge_cases": edge_cases,
    }


# ---------- gds corpus ----------

def canonical_layout_dump(layout: db.Layout) -> dict:
    """Produce a canonical JSON dump of a klayout.db Layout.

    Used as ground-truth for our reader. Coordinates are integer DBU.
    Cells listed in topo order (children first), shapes sorted by
    (layer, datatype, type, then encoded geometry) for determinism.
    """
    cells = []
    name_for_index = {ci: layout.cell(ci).name for ci in layout.each_cell_top_down()}

    # Walk in bottom-up order so the output mirrors how a topo-sort would dump it.
    bottom_up = list(layout.each_cell_bottom_up())
    for ci in bottom_up:
        c = layout.cell(ci)
        shapes = []
        for li in layout.layer_indices():
            info = layout.get_info(li)
            for s in c.shapes(li).each():
                shapes.append(shape_dump(s, info))
        shapes.sort(key=lambda s: (s["layer"], s["datatype"], s["type"], json.dumps(s, sort_keys=True)))

        instances = []
        for inst in c.each_inst():
            instances.append(instance_dump(inst, layout))
        instances.sort(key=lambda i: json.dumps(i, sort_keys=True))

        cells.append({
            "name": c.name,
            "shapes": shapes,
            "instances": instances,
        })

    return {
        "dbu_um": layout.dbu,
        "cells": cells,
    }


def shape_dump(s, info) -> dict:
    if s.is_polygon() or s.is_simple_polygon():
        poly = s.polygon
        hull = [[p.x, p.y] for p in poly.each_point_hull()]
        holes = []
        for hi in range(poly.holes()):
            holes.append([[p.x, p.y] for p in poly.each_point_hole(hi)])
        return {"type": "polygon", "layer": info.layer, "datatype": info.datatype,
                "hull": hull, "holes": holes}
    if s.is_box():
        b = s.box
        return {"type": "box", "layer": info.layer, "datatype": info.datatype,
                "left": b.left, "bottom": b.bottom, "right": b.right, "top": b.top}
    if s.is_path():
        p = s.path
        pts = [[pt.x, pt.y] for pt in p.each_point()]
        return {"type": "path", "layer": info.layer, "datatype": info.datatype,
                "width": p.width, "points": pts,
                "begin_ext": p.bgn_ext, "end_ext": p.end_ext, "round": p.is_round()}
    if s.is_text():
        t = s.text
        return {"type": "text", "layer": info.layer, "datatype": info.datatype,
                "string": t.string, "x": t.position().x, "y": t.position().y}
    return {"type": "unknown", "layer": info.layer, "datatype": info.datatype}


def instance_dump(inst, layout) -> dict:
    sname = layout.cell(inst.cell_index).name
    t = inst.trans
    rec = {"sname": sname, "trans": [int(t.rot), int(t.disp.x), int(t.disp.y)]}
    if inst.is_regular_array():
        # KLayout's CellInstArray fields: a/b/na/nb. KLayout assigns
        #   a <- the GDS XY[2] (Y-direction) vector / na
        #   b <- the GDS XY[1] (X-direction) vector / nb
        # which swaps the apparent "rows/cols" labels — use a/b/na/nb directly
        # to avoid that confusion.
        rec["array"] = {
            "a": [inst.a.x, inst.a.y],
            "b": [inst.b.x, inst.b.y],
            "na": inst.na,
            "nb": inst.nb,
        }
    if inst.has_prop_id():
        props = list(layout.properties(inst.prop_id))
        props.sort(key=lambda kv: int(kv[0]) if str(kv[0]).isdigit() else 0)
        rec["properties"] = [[int(k), str(v)] for k, v in props]
    return rec


def gen_gds():
    out_dir = CORPUS / "gds"
    out_dir.mkdir(parents=True, exist_ok=True)

    cases = []

    # Case 1: single box
    layout = db.Layout()
    layout.dbu = 0.001
    top = layout.create_cell("box_only")
    layer = layout.layer(1, 0)
    top.shapes(layer).insert(db.Box(0, 0, 100, 50))
    cases.append(("single_box", layout))

    # Case 2: polygon, path, text on different layers
    layout = db.Layout()
    layout.dbu = 0.001
    top = layout.create_cell("mixed")
    l_wg = layout.layer(1, 0)
    l_metal = layout.layer(2, 0)
    l_label = layout.layer(99, 0)
    top.shapes(l_wg).insert(db.Polygon([
        db.Point(0, 0), db.Point(100, 0),
        db.Point(100, 50), db.Point(0, 50),
    ]))
    path = db.Path([db.Point(0, 0), db.Point(50, 0), db.Point(50, 50)], 10)
    top.shapes(l_metal).insert(path)
    top.shapes(l_label).insert(db.Text("HELLO", db.Trans(0, False, 25, 25)))
    cases.append(("mixed", layout))

    # Case 3: hierarchy with rotated SREF
    layout = db.Layout()
    layout.dbu = 0.001
    child = layout.create_cell("unit")
    l = layout.layer(1, 0)
    child.shapes(l).insert(db.Box(0, 0, 10, 10))
    parent = layout.create_cell("parent")
    parent.insert(db.CellInstArray(child.cell_index(), db.Trans(0, False, 0, 0)))
    parent.insert(db.CellInstArray(child.cell_index(), db.Trans(1, False, 50, 0)))   # R90
    parent.insert(db.CellInstArray(child.cell_index(), db.Trans(4, False, 100, 0)))  # m0
    cases.append(("hierarchy", layout))

    # Case 4: regular AREF
    layout = db.Layout()
    layout.dbu = 0.001
    unit = layout.create_cell("unit")
    l = layout.layer(1, 0)
    unit.shapes(l).insert(db.Box(0, 0, 10, 10))
    arr_cell = layout.create_cell("arr")
    arr_cell.insert(db.CellInstArray(
        unit.cell_index(),
        db.Trans(0, False, 0, 0),
        db.Vector(20, 0),   # column step
        db.Vector(0, 30),   # row step
        4, 3,
    ))
    cases.append(("aref", layout))

    # Case 5: every one of the 8 SREF orientations.
    layout = db.Layout()
    layout.dbu = 0.001
    unit = layout.create_cell("u")
    l = layout.layer(1, 0)
    unit.shapes(l).insert(db.Box(0, 0, 30, 10))
    parent = layout.create_cell("eight")
    for rot_combined in range(8):
        rot = rot_combined % 4
        mirror = rot_combined >= 4
        parent.insert(db.CellInstArray(
            unit.cell_index(),
            db.Trans(rot, mirror, rot_combined * 100, 0),
        ))
    cases.append(("eight_orientations", layout))

    # Case 6: paths with each cap type and various widths.
    layout = db.Layout()
    layout.dbu = 0.001
    top = layout.create_cell("paths")
    l_a = layout.layer(1, 0)
    l_b = layout.layer(2, 0)
    # PATHTYPE 0 = flat
    p0 = db.Path([db.Point(0, 0), db.Point(100, 0)], 10)
    p0.path_type = 0
    top.shapes(l_a).insert(p0)
    # PATHTYPE 1 = round
    p1 = db.Path([db.Point(0, 30), db.Point(100, 30)], 10)
    p1.path_type = 1
    top.shapes(l_a).insert(p1)
    # PATHTYPE 2 = extended-square (half-width past endpoints)
    p2 = db.Path([db.Point(0, 60), db.Point(100, 60)], 10)
    p2.path_type = 2
    top.shapes(l_a).insert(p2)
    # PATHTYPE 4 = custom extensions (BGNEXTN/ENDEXTN)
    p4 = db.Path([db.Point(0, 90), db.Point(100, 90)], 10)
    p4.path_type = 4
    p4.bgn_ext = 5
    p4.end_ext = 7
    top.shapes(l_a).insert(p4)
    # Wider path on a different layer
    pw = db.Path(
        [db.Point(0, 200), db.Point(50, 200), db.Point(50, 300)], 30
    )
    top.shapes(l_b).insert(pw)
    cases.append(("path_caps", layout))

    # Case 7: irregular polygon with many vertices.
    layout = db.Layout()
    layout.dbu = 0.001
    top = layout.create_cell("hex")
    l = layout.layer(1, 0)
    top.shapes(l).insert(db.Polygon([
        db.Point(0, 50), db.Point(43, 25), db.Point(43, -25),
        db.Point(0, -50), db.Point(-43, -25), db.Point(-43, 25),
    ]))
    # Plus a non-rectangular 5-vertex shape so it doesn't get classified as a box.
    top.shapes(l).insert(db.Polygon([
        db.Point(100, 0), db.Point(150, 0),
        db.Point(150, 50), db.Point(125, 75),
        db.Point(100, 50),
    ]))
    cases.append(("polygons", layout))

    # Case 8: shapes spread across many layers.
    layout = db.Layout()
    layout.dbu = 0.001
    top = layout.create_cell("multilayer")
    for i, dt in enumerate([(1, 0), (2, 0), (2, 1), (3, 0), (10, 5)]):
        l = layout.layer(*dt)
        top.shapes(l).insert(db.Box(i * 20, 0, i * 20 + 10, 10))
    cases.append(("multilayer", layout))

    # Case 9: 3-level hierarchy.
    layout = db.Layout()
    layout.dbu = 0.001
    leaf = layout.create_cell("leaf")
    l = layout.layer(1, 0)
    leaf.shapes(l).insert(db.Box(0, 0, 5, 5))
    mid = layout.create_cell("mid")
    mid.insert(db.CellInstArray(leaf.cell_index(), db.Trans(0, False, 0, 0)))
    mid.insert(db.CellInstArray(leaf.cell_index(), db.Trans(0, False, 10, 0)))
    top = layout.create_cell("top")
    top.insert(db.CellInstArray(mid.cell_index(), db.Trans(0, False, 0, 0)))
    top.insert(db.CellInstArray(mid.cell_index(), db.Trans(2, False, 100, 100)))  # R180
    top.insert(db.CellInstArray(mid.cell_index(), db.Trans(5, False, 200, 0)))    # m45
    cases.append(("multilevel", layout))

    # Case 10: empty cell + cell that only contains instances.
    layout = db.Layout()
    layout.dbu = 0.001
    empty = layout.create_cell("empty")
    refonly = layout.create_cell("refonly")
    refonly.insert(db.CellInstArray(empty.cell_index(), db.Trans(0, False, 0, 0)))
    cases.append(("empty_and_refonly", layout))

    # Case 11: AREF with rotation (R90).
    layout = db.Layout()
    layout.dbu = 0.001
    unit = layout.create_cell("u")
    l = layout.layer(1, 0)
    unit.shapes(l).insert(db.Box(0, 0, 10, 10))
    arr_cell = layout.create_cell("rot_arr")
    arr_cell.insert(db.CellInstArray(
        unit.cell_index(),
        db.Trans(1, False, 100, 200),  # R90
        db.Vector(15, 0),
        db.Vector(0, 25),
        3, 2,
    ))
    cases.append(("aref_rotated", layout))

    # Case 12: AREF with mirror+rotation.
    layout = db.Layout()
    layout.dbu = 0.001
    unit = layout.create_cell("u")
    l = layout.layer(1, 0)
    unit.shapes(l).insert(db.Box(0, 0, 10, 10))
    arr_cell = layout.create_cell("mir_arr")
    arr_cell.insert(db.CellInstArray(
        unit.cell_index(),
        db.Trans(6, False, 50, 50),  # m90
        db.Vector(20, 5),
        db.Vector(-3, 30),
        2, 2,
    ))
    cases.append(("aref_mirrored", layout))

    # Case 13: text on multiple layers, multiple labels.
    layout = db.Layout()
    layout.dbu = 0.001
    top = layout.create_cell("labels")
    l_a = layout.layer(99, 0)
    l_b = layout.layer(99, 1)
    top.shapes(l_a).insert(db.Text("A", db.Trans(0, False, 0, 0)))
    top.shapes(l_a).insert(db.Text("B", db.Trans(0, False, 100, 0)))
    top.shapes(l_b).insert(db.Text("hello world", db.Trans(0, False, 50, 50)))
    cases.append(("texts", layout))

    # Case 14: cell with shapes AND instances (mixed).
    layout = db.Layout()
    layout.dbu = 0.001
    leaf = layout.create_cell("dot")
    l = layout.layer(1, 0)
    leaf.shapes(l).insert(db.Box(-1, -1, 1, 1))
    grid = layout.create_cell("grid")
    grid.shapes(l).insert(db.Box(0, 0, 100, 100))   # frame
    for i in range(4):
        for j in range(4):
            grid.insert(db.CellInstArray(
                leaf.cell_index(),
                db.Trans(0, False, 25 * i + 12, 25 * j + 12),
            ))
    cases.append(("mixed_shapes_and_insts", layout))

    for name, layout in cases:
        gds_path = out_dir / f"{name}.gds"
        json_path = out_dir / f"{name}.json"
        layout.write(str(gds_path))
        # Round-trip through KLayout's reader so the canonical dump describes
        # what's actually in the file (e.g. KLayout's default LIBNAME = "LIB"
        # even when the in-memory libname is empty).
        rt = db.Layout()
        rt.read(str(gds_path))
        with open(json_path, "w") as f:
            json.dump(canonical_layout_dump(rt), f, indent=2, sort_keys=True)
        print(f"  wrote {gds_path.name} + {json_path.name}")


# ---------- entry point ----------

def verify_gds(path: str):
    """Read a GDS file via klayout.db and emit its canonical JSON dump to stdout.

    Used as a subprocess by the Rust writer-parity tests:
    they write a GDS via klayout-io, then invoke `oracle.py verify <path>`
    to get KLayout's view of the file, and compare against their own dump.
    """
    layout = db.Layout()
    layout.read(path)
    sys.stdout.write(json.dumps(canonical_layout_dump(layout), sort_keys=True))


# ---------- region corpus ----------

def canonicalize_polygon(pts: list[list[int]], hole: bool = False) -> list[list[int]]:
    """Canonicalize: lowest-y/lowest-x first. Hulls are CW, holes are CCW
    (matches `klayout_core::Polygon` convention)."""
    if len(pts) < 3:
        return pts
    start = 0
    for i in range(1, len(pts)):
        if (pts[i][1] < pts[start][1]
                or (pts[i][1] == pts[start][1] and pts[i][0] < pts[start][0])):
            start = i
    rotated = pts[start:] + pts[:start]
    s = 0
    n = len(rotated)
    for i in range(n):
        a, b = rotated[i], rotated[(i + 1) % n]
        s += a[0] * b[1] - b[0] * a[1]
    # Hulls: ensure s < 0 (CW). Holes: ensure s > 0 (CCW).
    want_ccw = hole
    is_ccw = s > 0
    if want_ccw != is_ccw:
        rotated = [rotated[0]] + list(reversed(rotated[1:]))
    return rotated


def dump_region(r: db.Region) -> list[dict]:
    """Merged + canonicalized list of polygons with holes preserved.
    Each polygon is `{"hull": [...], "holes": [[...], ...]}`. KLayout's
    boolean engine produces polygon-with-hole results (e.g. an XOR returns
    a doughnut as one polygon-with-hole rather than splitting topologically),
    so we need to round-trip that information for accurate comparison."""
    r_merged = r.merged()
    polys: list[dict] = []
    for p in r_merged.each():
        hull = canonicalize_polygon([[pt.x, pt.y] for pt in p.each_point_hull()])
        holes = []
        for hi in range(p.holes()):
            holes.append(canonicalize_polygon(
                [[pt.x, pt.y] for pt in p.each_point_hole(hi)],
                hole=True,
            ))
        # Sort holes deterministically.
        holes.sort(key=lambda h: (h[0][0], h[0][1], len(h)))
        polys.append({"hull": hull, "holes": holes})

    def keyfn(p):
        h = p["hull"]
        xs = [pt[0] for pt in h]
        ys = [pt[1] for pt in h]
        return (min(xs), min(ys), max(xs), max(ys), len(h))
    polys.sort(key=keyfn)
    return polys


def gen_region() -> dict:
    cases = []

    def make_region(rects):
        r = db.Region()
        for (x0, y0, x1, y1) in rects:
            r.insert(db.Box(x0, y0, x1, y1))
        return r

    fixtures = [
        ("disjoint", [(0, 0, 10, 10)], [(20, 0, 30, 10)]),
        ("overlap", [(0, 0, 10, 10)], [(5, 5, 15, 15)]),
        ("touching", [(0, 0, 10, 10)], [(10, 0, 20, 10)]),
        ("nested", [(0, 0, 100, 100)], [(20, 20, 50, 50)]),
        ("multi_a", [(0, 0, 10, 10), (20, 0, 30, 10)], [(5, 0, 25, 10)]),
        ("L_shapes", [(0, 0, 30, 10), (0, 0, 10, 30)], [(20, 20, 40, 40)]),
    ]

    for name, ra, rb in fixtures:
        a = make_region(ra)
        b = make_region(rb)
        cases.append({
            "name": name,
            "a_rects": [list(t) for t in ra],
            "b_rects": [list(t) for t in rb],
            "union": dump_region(a + b),
            "intersect": dump_region(a & b),
            "difference": dump_region(a - b),
            "xor": dump_region(a ^ b),
        })

    size_cases = []
    for name, rects, delta in [
        ("rect_grow", [(0, 0, 100, 100)], 20),
        ("rect_shrink", [(0, 0, 100, 100)], -10),
        ("rect_zero", [(0, 0, 100, 100)], 0),
        ("two_disjoint_grow", [(0, 0, 10, 10), (50, 0, 60, 10)], 5),
        ("two_close_grow_merges", [(0, 0, 10, 10), (15, 0, 25, 10)], 5),
        ("L_grow", [(0, 0, 30, 10), (0, 0, 10, 30)], 3),
    ]:
        r = db.Region()
        for (x0, y0, x1, y1) in rects:
            r.insert(db.Box(x0, y0, x1, y1))
        size_cases.append({
            "name": name,
            "rects": [list(t) for t in rects],
            "delta": delta,
            "result": dump_region(r.sized(delta, delta, 2)),  # mode 2 = sharp corners
        })

    return {
        "klayout_version": db.__version__ if hasattr(db, "__version__") else "unknown",
        "boolean_cases": cases,
        "size_cases": size_cases,
    }


def density_window_violations(
    region: db.Region,
    win_w: int,
    win_h: int,
    step_x: int,
    step_y: int,
    dmin: float,
    dmax: float,
    *,
    padding: str = "zero",
    boundary_rects: list | None = None,
    tile_origin: tuple[int, int] | None = None,
    tile_count: tuple[int, int] | None = None,
    explicit_frame: bool = True,
    inverse: bool = False,
) -> db.Region:
    """Reference for ``density_window``: KLayout DRC ``without_density`` / ``with_density``.

    Uses ``TilingProcessor`` like KLayout DRC. ``inverse=False`` (default) matches
    ``without_density`` (emit tiles **outside** the density band). ``inverse=True``
    matches ``with_density`` (emit tiles **inside** the band).

    ``explicit_frame``: when True, sets ``tp.frame`` to the union of layer + boundary
    bbox so 1×1 grids still get ``_tile`` (common for deck-level checks). When False,
    matches bare Ruby without ``frame`` — singleton tile plans produce no output.
    """
    region.merge()
    if region.is_empty():
        return db.Region()

    tp = db.TilingProcessor()
    tp.dbu = 1.0
    tp.scale_to_dbu = False

    bb_layer = region.bbox()
    boundary = db.Region()
    if boundary_rects:
        for x0, y0, x1, y1 in boundary_rects:
            boundary.insert(db.Box(x0, y0, x1, y1))
        boundary.merge()
    else:
        boundary.insert(db.Box(bb_layer.left, bb_layer.bottom, bb_layer.right, bb_layer.top))

    bb_b = boundary.bbox()
    union = db.DBox(
        min(bb_layer.left, bb_b.left),
        min(bb_layer.bottom, bb_b.bottom),
        max(bb_layer.right, bb_b.right),
        max(bb_layer.top, bb_b.top),
    )

    if explicit_frame:
        tp.frame = db.DBox(float(union.left), float(union.bottom), float(union.right), float(union.top))

    res = db.Region()
    tp.input("input", region)
    tp.input("boundary", boundary)
    tp.tile_size(float(step_x), float(step_y))
    xb = 0.5 * (float(win_w) - float(step_x))
    yb = 0.5 * (float(win_h) - float(step_y))
    if xb < 0:
        xb = 0.0
    if yb < 0:
        yb = 0.0
    tp.tile_border(xb, yb)
    tp.var("vmin", float(dmin))
    tp.var("vmax", float(dmax))
    tp.var("xoverlap", xb / tp.dbu)
    tp.var("yoverlap", yb / tp.dbu)
    if tile_origin is not None:
        tp.tile_origin(float(tile_origin[0]), float(tile_origin[1]))
    if tile_count is not None:
        tp.tiles(int(tile_count[0]), int(tile_count[1]))
    tp.output("res", res)

    if inverse:
        in_band_test = "((d > vmin - 1e-10 && d < vmax + 1e-10) == true)"
    else:
        in_band_test = "((d > vmin - 1e-10 && d < vmax + 1e-10) != true)"

    if padding == "zero":
        tp.queue(
            f"""
_tile && (
  var bx = _tile.bbox.enlarged(xoverlap, yoverlap);
  var d = to_f(input.area(bx)) / to_f(bx.area);
  {in_band_test} && _output(res, bx, false)
)
"""
        )
    elif padding == "ignore":
        tp.queue(
            f"""
_tile && (
  var bx = _tile.bbox.enlarged(xoverlap, yoverlap);
  var ba = boundary.area(bx);
  ba > 0 && (
    var d = to_f(input.area(bx)) / to_f(ba);
    {in_band_test} && _output(res, bx, false)
  )
)
"""
        )
    else:
        raise ValueError(f"unknown padding {padding!r}")

    tp.execute("density_window_ref")
    return res


# ---------- DRC corpus ----------


def gen_drc() -> dict:
    """DRC validation corpus. `width`, `space`, and `separation` use exact
    edge-pair analysis on our side and match KLayout's output for any
    axis-aligned input. `density_window` matches KLayout DRC
    ``without_density`` / ``with_density`` (``TilingProcessor``): ``padding_zero``,
    ``padding_ignore``, ``tile_boundary``, ``tile_origin``, ``tile_count``, and
    singleton-tile semantics via ``evaluate_singleton_tiles`` / explicit frame, and
    ``with_density`` (corpus ``density_output: "inside"``) combined with those knobs.
    `enclosing` and `overlap` use shrink-based approximations on our side; the corpus
    still records KLayout ``enclosing_check`` / ``overlap_check`` polygons
    and asserts we match on these fixtures."""
    cases = []

    def make_region(rects):
        r = db.Region()
        for x0, y0, x1, y1 in rects:
            r.insert(db.Box(x0, y0, x1, y1))
        return r

    # ----- width -----
    width_cases = [
        ("wide_no_violation", [(0, 0, 100, 50)], 11),
        ("thin_full_violation", [(0, 0, 100, 5)], 11),
        ("just_above", [(0, 0, 100, 11)], 11),
        ("just_below", [(0, 0, 100, 9)], 11),
        ("multi_polygon_mix", [(0, 0, 100, 5), (200, 0, 300, 50)], 11),
        ("L_shape_thin_arm", [(0, 0, 100, 50), (0, 0, 5, 100)], 11),
        # Even-min thresholds — KLayout's strict-less-than passes a
        # polygon of width exactly equal to min.
        ("even_at_threshold", [(0, 0, 100, 10)], 10),
        ("even_below", [(0, 0, 100, 8)], 10),
        ("even_above", [(0, 0, 100, 12)], 10),
    ]
    for name, rects, m in width_cases:
        r = make_region(rects)
        cases.append({
            "rule": "width",
            "name": name,
            "rects_a": [list(t) for t in rects],
            "min": m,
            "result": dump_region(r.width_check(m).polygons()),
        })

    # ----- space -----
    space_cases = [
        ("far", [(0, 0, 10, 10), (50, 0, 60, 10)], 11),
        ("close_h", [(0, 0, 10, 10), (15, 0, 25, 10)], 11),
        ("close_v", [(0, 0, 10, 10), (0, 15, 10, 25)], 11),
        ("just_above", [(0, 0, 10, 10), (21, 0, 31, 10)], 11),  # 11 apart
        ("multi_three", [(0, 0, 10, 10), (15, 0, 25, 10), (50, 0, 60, 10)], 11),
        ("partial_overlap_y",
         [(0, 0, 10, 10), (15, 5, 25, 15)], 11),
    ]
    for name, rects, m in space_cases:
        r = make_region(rects)
        cases.append({
            "rule": "space",
            "name": name,
            "rects_a": [list(t) for t in rects],
            "min": m,
            "result": dump_region(r.space_check(m).polygons()),
        })

    # ----- separation (cross-layer) -----
    sep_cases = [
        ("disjoint_far", [(0, 0, 10, 10)], [(50, 0, 60, 10)], 11),
        ("disjoint_close", [(0, 0, 10, 10)], [(15, 0, 25, 10)], 11),
        ("disjoint_close_v", [(0, 0, 10, 10)], [(0, 15, 10, 25)], 11),
    ]
    for name, ra, rb, m in sep_cases:
        a = make_region(ra)
        b = make_region(rb)
        cases.append({
            "rule": "separation",
            "name": name,
            "rects_a": [list(t) for t in ra],
            "rects_b": [list(t) for t in rb],
            "min": m,
            "result": dump_region(a.separation_check(b, m).polygons()),
        })

    # ----- enclosing -----
    enc_cases = [
        ("safe", [(0, 0, 100, 100)], [(20, 20, 50, 50)], 11),
        ("violates_left", [(0, 0, 100, 100)], [(5, 20, 50, 50)], 11),
        ("violates_corner", [(0, 0, 100, 100)], [(5, 5, 50, 50)], 11),
        ("large_min", [(0, 0, 100, 100)], [(5, 20, 50, 50)], 21),
    ]
    for name, ro, ri, m in enc_cases:
        outer = make_region(ro)
        inner = make_region(ri)
        cases.append({
            "rule": "enclosing",
            "name": name,
            "rects_a": [list(t) for t in ro],
            "rects_b": [list(t) for t in ri],
            "min": m,
            "result": dump_region(outer.enclosing_check(inner, m).polygons()),
        })

    # ----- overlap -----
    over_cases = [
        ("wide_overlap", [(0, 0, 100, 100)], [(50, 50, 150, 150)], 11),
        ("thin_corner", [(0, 0, 100, 100)], [(95, 95, 200, 200)], 11),
    ]
    for name, ra, rb, m in over_cases:
        a = make_region(ra)
        b = make_region(rb)
        cases.append({
            "rule": "overlap",
            "name": name,
            "rects_a": [list(t) for t in ra],
            "rects_b": [list(t) for t in rb],
            "min": m,
            "result": dump_region(a.overlap_check(b, m).polygons()),
        })

    # ----- density_window (KLayout without_density / TilingProcessor) -----
    def density_case(
        name: str,
        rects,
        win: tuple,
        step: tuple,
        dmin: float,
        dmax: float,
        *,
        padding: str = "zero",
        boundary_rects: list | None = None,
        tile_origin: tuple | None = None,
        tile_count: tuple | None = None,
        evaluate_singleton_tiles: bool = True,
        density_output: str = "outside",
    ):
        inverse = density_output == "inside"
        r = make_region(rects)
        viol = density_window_violations(
            r,
            win[0],
            win[1],
            step[0],
            step[1],
            dmin,
            dmax,
            padding=padding,
            boundary_rects=boundary_rects,
            tile_origin=tile_origin,
            tile_count=tile_count,
            explicit_frame=evaluate_singleton_tiles,
            inverse=inverse,
        )
        entry = {
            "rule": "density_window",
            "name": name,
            "rects_a": [list(t) for t in rects],
            "rects_b": [],
            "min": 0,
            "window": [win[0], win[1]],
            "step": [step[0], step[1]],
            "density_min": dmin,
            "density_max": dmax,
            "density_padding": padding,
            "evaluate_singleton_tiles": evaluate_singleton_tiles,
            "result": dump_region(viol),
        }
        if density_output != "outside":
            entry["density_output"] = density_output
        if boundary_rects is not None:
            entry["boundary_rects"] = [list(t) for t in boundary_rects]
        if tile_origin is not None:
            entry["tile_origin"] = [tile_origin[0], tile_origin[1]]
        if tile_count is not None:
            entry["tile_count"] = [tile_count[0], tile_count[1]]
        cases.append(entry)

    density_case("passes_in_range", [(0, 0, 50, 50)], (100, 100), (100, 100), 0.20, 0.30)
    density_case(
        "with_density_in_band_emits_window",
        [(0, 0, 50, 50)],
        (100, 100),
        (100, 100),
        0.20,
        0.30,
        density_output="inside",
    )
    # `with_density` + same knobs as other fixtures (KLayout `inverse` / TilingProcessor).
    density_case(
        "with_density_tile_origin_corner",
        [(0, 0, 30, 30)],
        (100, 100),
        (100, 100),
        0.05,
        0.15,
        tile_origin=(0, 0),
        density_output="inside",
    )
    density_case(
        "with_density_padding_ignore_strip",
        [(0, 0, 50, 10)],
        (100, 100),
        (100, 100),
        0.45,
        0.55,
        padding="ignore",
        boundary_rects=[(0, 0, 200, 10)],
        density_output="inside",
    )
    density_case(
        "with_density_tile_count_fixed",
        [(0, 0, 250, 10)],
        (100, 100),
        (100, 100),
        0.3,
        0.7,
        tile_count=(3, 1),
        density_output="inside",
    )
    density_case(
        "with_density_strict_singleton_no_output",
        [(0, 0, 10, 10)],
        (100, 100),
        (100, 100),
        0.3,
        0.7,
        evaluate_singleton_tiles=False,
        density_output="inside",
    )
    density_case(
        "with_density_window_larger_than_step",
        [(0, 0, 40, 40)],
        (120, 120),
        (100, 100),
        0.10,
        0.20,
        density_output="inside",
    )
    density_case(
        "with_density_sliding_multi_tile",
        [(0, 0, 90, 90), (500, 0, 510, 10)],
        (100, 100),
        (100, 100),
        0.30,
        0.70,
        density_output="inside",
    )
    density_case("flags_below_minimum", [(0, 0, 10, 10)], (100, 100), (100, 100), 0.30, 0.70)
    density_case("flags_above_maximum", [(0, 0, 90, 90)], (100, 100), (100, 100), 0.30, 0.70)
    density_case(
        "sliding_multi_tile",
        [(0, 0, 90, 90), (500, 0, 510, 10)],
        (100, 100),
        (100, 100),
        0.30,
        0.70,
    )
    density_case(
        "window_larger_than_step",
        [(0, 0, 40, 40)],
        (120, 120),
        (100, 100),
        0.10,
        0.20,
    )
    density_case(
        "padding_ignore_strip",
        [(0, 0, 50, 10)],
        (100, 100),
        (100, 100),
        0.45,
        0.55,
        padding="ignore",
        boundary_rects=[(0, 0, 200, 10)],
    )
    density_case(
        "tile_origin_corner",
        [(0, 0, 30, 30)],
        (100, 100),
        (100, 100),
        0.05,
        0.15,
        tile_origin=(0, 0),
    )
    density_case(
        "tile_count_fixed",
        [(0, 0, 250, 10)],
        (100, 100),
        (100, 100),
        0.3,
        0.7,
        tile_count=(3, 1),
    )
    density_case(
        "strict_singleton_no_output",
        [(0, 0, 10, 10)],
        (100, 100),
        (100, 100),
        0.3,
        0.7,
        evaluate_singleton_tiles=False,
    )

    return {
        "klayout_version": db.__version__ if hasattr(db, "__version__") else "unknown",
        "cases": cases,
    }


def gen_drc_density_grid() -> dict:
    """Bounded Cartesian product: every case is KLayout ``TilingProcessor`` output.

    Covers discrete combinations of padding, ``with_density`` / ``without_density``
    (``density_output``), singleton ``tp.frame`` behavior, ``tile_origin``,
    ``tile_count``, window/step pairs, and a small geometry set — not the
    continuous (coord, float) space, but the full *knob* surface of our port.
    Regenerate with: ``python validation/oracle.py drc_density_grid``.
    """

    def make_region(rects):
        r = db.Region()
        for x0, y0, x1, y1 in rects:
            r.insert(db.Box(x0, y0, x1, y1))
        return r

    geometries = [
        ("sq50", [(0, 0, 50, 50)], None),
        ("sq10", [(0, 0, 10, 10)], None),
        ("dual", [(0, 0, 90, 90), (500, 0, 510, 10)], None),
        ("strip", [(0, 0, 50, 10)], [(0, 0, 200, 10)]),
    ]
    win_steps = [
        ((100, 100), (100, 100)),
        ((100, 100), (50, 50)),
        ((120, 120), (100, 100)),
        ((100, 100), (100, 50)),
        ((80, 80), (80, 80)),
        ((100, 120), (100, 100)),
    ]
    density_pairs = ((0.2, 0.3), (0.3, 0.7))
    cases = []
    idx = 0

    for _gid, rects, opt_boundary in geometries:
        for win, step in win_steps:
            if win[0] < step[0] or win[1] < step[1]:
                continue
            for padding in ("zero", "ignore"):
                for density_output in ("outside", "inside"):
                    for evaluate_singleton_tiles in (True, False):
                        for tile_origin in (None, (0, 0)):
                            for tile_count in (None, (3, 1)):
                                for dmin, dmax in density_pairs:
                                    boundary_rects = None
                                    if padding == "ignore" and opt_boundary is not None:
                                        boundary_rects = opt_boundary
                                    inverse = density_output == "inside"
                                    r = make_region(rects)
                                    viol = density_window_violations(
                                        r,
                                        win[0],
                                        win[1],
                                        step[0],
                                        step[1],
                                        dmin,
                                        dmax,
                                        padding=padding,
                                        boundary_rects=boundary_rects,
                                        tile_origin=tile_origin,
                                        tile_count=tile_count,
                                        explicit_frame=evaluate_singleton_tiles,
                                        inverse=inverse,
                                    )
                                    name = f"dg_{idx:05d}"
                                    entry = {
                                        "rule": "density_window",
                                        "name": name,
                                        "rects_a": [list(t) for t in rects],
                                        "rects_b": [],
                                        "min": 0,
                                        "window": [win[0], win[1]],
                                        "step": [step[0], step[1]],
                                        "density_min": dmin,
                                        "density_max": dmax,
                                        "density_padding": padding,
                                        "evaluate_singleton_tiles": evaluate_singleton_tiles,
                                        "result": dump_region(viol),
                                    }
                                    if density_output != "outside":
                                        entry["density_output"] = density_output
                                    if boundary_rects is not None:
                                        entry["boundary_rects"] = [
                                            list(t) for t in boundary_rects
                                        ]
                                    if tile_origin is not None:
                                        entry["tile_origin"] = [
                                            tile_origin[0],
                                            tile_origin[1],
                                        ]
                                    if tile_count is not None:
                                        entry["tile_count"] = [
                                            tile_count[0],
                                            tile_count[1],
                                        ]
                                    cases.append(entry)
                                    idx += 1

    return {
        "klayout_version": db.__version__ if hasattr(db, "__version__") else "unknown",
        "cases": cases,
    }


def gen_polygon_ops() -> dict:
    """Per-polygon scalar properties from klayout.db: area, perimeter,
    bbox. Used by klayout-validate/tests/polygon_ops.rs to verify our
    Polygon math agrees with KLayout exactly."""
    random.seed(SEED)
    cases = []

    fixtures = [
        ("rect_10x5", [(0, 0), (10, 0), (10, 5), (0, 5)]),
        ("triangle", [(0, 0), (10, 0), (5, 10)]),
        ("L_shape", [(0, 0), (10, 0), (10, 4), (4, 4), (4, 10), (0, 10)]),
        ("T_shape", [(0, 0), (30, 0), (30, 10), (20, 10), (20, 20), (10, 20), (10, 10), (0, 10)]),
        ("pentagon", [(0, 0), (10, 0), (15, 5), (5, 12), (-5, 5)]),
        ("hexagon", [(0, 0), (8, 0), (12, 7), (8, 14), (0, 14), (-4, 7)]),
        ("convex_5", [(0, 0), (20, 0), (25, 10), (10, 20), (-5, 10)]),
        ("zigzag", [(0, 0), (10, 0), (10, 5), (5, 5), (5, 10), (15, 10), (15, 0), (25, 0), (25, 15), (0, 15)]),
        ("very_thin", [(0, 0), (1000, 0), (1000, 1), (0, 1)]),
        ("very_long", [(0, 0), (100000, 0), (100000, 10), (0, 10)]),
    ]

    for name, pts in fixtures:
        p = db.Polygon([db.Point(x, y) for (x, y) in pts])
        bbox = p.bbox()
        cases.append({
            "name": name,
            "hull": [list(t) for t in pts],
            "area": int(p.area()),
            "perimeter": int(p.perimeter()),
            "bbox": [bbox.left, bbox.bottom, bbox.right, bbox.top],
        })

    # Random simple polygons (rectangles + triangles).
    for i in range(50):
        x0 = random.randint(-1000, 1000)
        y0 = random.randint(-1000, 1000)
        w = random.randint(1, 500)
        h = random.randint(1, 500)
        pts = [(x0, y0), (x0 + w, y0), (x0 + w, y0 + h), (x0, y0 + h)]
        p = db.Polygon([db.Point(x, y) for (x, y) in pts])
        bbox = p.bbox()
        cases.append({
            "name": f"rand_rect_{i}",
            "hull": [list(t) for t in pts],
            "area": int(p.area()),
            "perimeter": int(p.perimeter()),
            "bbox": [bbox.left, bbox.bottom, bbox.right, bbox.top],
        })

    return {"cases": cases}


def gen_oasis():
    """Mirror of gen_gds but for OASIS. KLayout writes a .oas; our
    reader loads it and dumps the canonical JSON; tests assert the
    dumps match."""
    out_dir = CORPUS / "oasis"
    out_dir.mkdir(parents=True, exist_ok=True)
    fixtures = []

    # Single box.
    layout = db.Layout()
    layout.dbu = 0.001
    cell = layout.create_cell("BOX")
    layer = layout.layer(1, 0)
    cell.shapes(layer).insert(db.Box(0, 0, 100, 50))
    p = out_dir / "single_box.oas"
    layout.write(str(p))
    fixtures.append(("single_box", p, layout))

    # Multi-cell hierarchy.
    layout = db.Layout()
    layout.dbu = 0.001
    layer = layout.layer(1, 0)
    leaf = layout.create_cell("leaf")
    leaf.shapes(layer).insert(db.Box(0, 0, 10, 10))
    top = layout.create_cell("top")
    top.shapes(layer).insert(db.Box(20, 0, 30, 10))
    top.insert(db.CellInstArray(leaf.cell_index(), db.Trans(0, False, 50, 0)))
    p = out_dir / "hierarchy.oas"
    layout.write(str(p))
    fixtures.append(("hierarchy", p, layout))

    # Multiple shapes covering plist-type 0/1 (manhattan), 4 (g-delta
    # form-1 octangular and form-2 non-octangular), and rectangle.
    layout = db.Layout()
    layout.dbu = 0.001
    cell = layout.create_cell("POLYS")
    layer = layout.layer(2, 0)
    cell.shapes(layer).insert(
        db.Polygon([
            db.Point(0, 0), db.Point(20, 0),
            db.Point(20, 10), db.Point(10, 10),
            db.Point(10, 20), db.Point(0, 20),
        ])
    )
    cell.shapes(layer).insert(db.Box(50, 0, 70, 20))
    cell.shapes(layer).insert(
        db.Polygon([db.Point(0, 30), db.Point(20, 30), db.Point(15, 40), db.Point(5, 40)])
    )
    cell.shapes(layer).insert(
        db.Polygon([db.Point(30, 30), db.Point(40, 40), db.Point(30, 40)])
    )
    # Negative dx form-2 g-delta (the (-5, 10) edge in this hull).
    cell.shapes(layer).insert(
        db.Polygon([db.Point(50, 30), db.Point(40, 50), db.Point(45, 50)])
    )
    p = out_dir / "polys.oas"
    layout.write(str(p))
    fixtures.append(("polys", p, layout))

    # Multi-layer cell with several shapes per layer.
    layout = db.Layout()
    layout.dbu = 0.001
    cell = layout.create_cell("MULTI")
    l1 = layout.layer(1, 0)
    l2 = layout.layer(2, 0)
    l3 = layout.layer(5, 7)
    for i in range(5):
        cell.shapes(l1).insert(db.Box(i * 30, 0, i * 30 + 20, 10))
    for i in range(3):
        cell.shapes(l2).insert(db.Box(i * 50, 100, i * 50 + 40, 140))
    cell.shapes(l3).insert(
        db.Polygon([db.Point(0, 200), db.Point(50, 200), db.Point(50, 250),
                    db.Point(25, 280), db.Point(0, 250)])
    )
    p = out_dir / "multi_layer.oas"
    layout.write(str(p))
    fixtures.append(("multi_layer", p, layout))

    # Multiple instances of multiple cells (deeper hierarchy).
    layout = db.Layout()
    layout.dbu = 0.001
    layer = layout.layer(1, 0)
    leaf_a = layout.create_cell("LEAFA")
    leaf_a.shapes(layer).insert(db.Box(0, 0, 10, 10))
    leaf_b = layout.create_cell("LEAFB")
    leaf_b.shapes(layer).insert(db.Box(0, 0, 20, 5))
    mid = layout.create_cell("MID")
    mid.insert(db.CellInstArray(leaf_a.cell_index(), db.Trans(0, False, 0, 0)))
    mid.insert(db.CellInstArray(leaf_a.cell_index(), db.Trans(0, False, 30, 0)))
    mid.insert(db.CellInstArray(leaf_b.cell_index(), db.Trans(0, False, 0, 30)))
    top = layout.create_cell("TOP_HIER")
    top.insert(db.CellInstArray(mid.cell_index(), db.Trans(0, False, 0, 0)))
    top.insert(db.CellInstArray(mid.cell_index(), db.Trans(0, False, 100, 0)))
    p = out_dir / "deep_hier.oas"
    layout.write(str(p))
    fixtures.append(("deep_hier", p, layout))

    # Write the canonical JSON dump for each fixture.
    for name, p, layout in fixtures:
        dump = canonical_layout_dump(layout)
        json_p = p.with_suffix(".json")
        json_p.write_text(json.dumps(dump, indent=2))
        print(f"wrote {p.name} + {json_p.name}")


CIF_FIXTURES: dict[str, str] = {
    "single_box": (
        "(simple);\n"
        "DS 1 1 1;\n"
        "L L1;\n"
        "B 100 50 50 25;\n"
        "DF;\n"
        "C 1;\n"
        "E\n"
    ),
    "two_layers": (
        "(two layers);\n"
        "DS 1 1 1;\n"
        "L L1;\n"
        "B 100 50 50 25;\n"
        "L L2;\n"
        "B 200 100 200 50;\n"
        "DF;\n"
        "C 1;\n"
        "E\n"
    ),
    "polygon_shape": (
        "(triangle polygon);\n"
        "DS 1 1 1;\n"
        "L L1;\n"
        "P 0 0 100 0 50 100;\n"
        "DF;\n"
        "C 1;\n"
        "E\n"
    ),
    "multi_layer_polys": (
        "(multi-layer polygon mix);\n"
        "DS 1 1 1;\n"
        "L L1;\n"
        "B 50 50 25 25;\n"
        "P 100 0 200 0 150 100;\n"
        "L L3;\n"
        "B 30 30 250 25;\n"
        "L L7;\n"
        "P 300 0 400 0 400 100 300 100;\n"
        "DF;\n"
        "C 1;\n"
        "E\n"
    ),
    "hierarchy": (
        "(hier with translated child);\n"
        "DS 2 1 1;\n"
        "L L1;\n"
        "B 50 50 25 25;\n"
        "DF;\n"
        "DS 1 1 1;\n"
        "L L2;\n"
        "B 100 100 50 50;\n"
        "C 2 T 200 200;\n"
        "C 2 T 400 0;\n"
        "DF;\n"
        "C 1;\n"
        "E\n"
    ),
    "rot_mirror": (
        "(rotated and mirrored child instances);\n"
        "DS 2 1 1;\n"
        "L L1;\n"
        "B 100 50 50 25;\n"
        "DF;\n"
        "DS 1 1 1;\n"
        "L L2;\n"
        "B 50 50 25 25;\n"
        "C 2 T 0 0;\n"
        "C 2 R 0 1 T 200 0;\n"
        "C 2 R -1 0 T 400 0;\n"
        "C 2 MX T 600 0;\n"
        "DF;\n"
        "C 1;\n"
        "E\n"
    ),
}


def _dxf(layer_table: str, entities: str) -> str:
    return (
        "0\nSECTION\n2\nHEADER\n0\nENDSEC\n"
        "0\nSECTION\n2\nTABLES\n"
        f"0\nTABLE\n2\nLAYER\n70\n9\n{layer_table}0\nENDTAB\n"
        "0\nENDSEC\n"
        f"0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    )


DXF_LAYERS_1 = "0\nLAYER\n2\n1\n70\n0\n62\n7\n6\nCONTINUOUS\n"
DXF_LAYERS_3 = (
    "0\nLAYER\n2\n1\n70\n0\n62\n7\n6\nCONTINUOUS\n"
    "0\nLAYER\n2\n2\n70\n0\n62\n7\n6\nCONTINUOUS\n"
    "0\nLAYER\n2\n3\n70\n0\n62\n7\n6\nCONTINUOUS\n"
)


DXF_FIXTURES: dict[str, str] = {
    # Proper DXF needs HEADER + ENTITIES sections for KLayout's
    # auto-detect to fire. Minimal closed polyline on layer 1:
    "rectangle": _dxf(
        DXF_LAYERS_1,
        "0\nLWPOLYLINE\n8\n1\n90\n4\n70\n1\n"
        "10\n0\n20\n0\n10\n10\n20\n0\n10\n10\n20\n10\n10\n0\n20\n10\n",
    ),
    "two_polylines": _dxf(
        DXF_LAYERS_3,
        # Triangle on layer 1
        "0\nLWPOLYLINE\n8\n1\n90\n3\n70\n1\n10\n0\n20\n0\n10\n10\n20\n0\n10\n5\n20\n10\n"
        # Square on layer 2
        "0\nLWPOLYLINE\n8\n2\n90\n4\n70\n1\n10\n20\n20\n0\n10\n30\n20\n0\n10\n30\n20\n10\n10\n20\n20\n10\n",
    ),
    "lines_polys": _dxf(
        DXF_LAYERS_3,
        # Two LINEs on layer 1, one polygon on layer 3
        "0\nLINE\n8\n1\n10\n0\n20\n0\n11\n10\n21\n0\n"
        "0\nLINE\n8\n1\n10\n0\n20\n10\n11\n10\n21\n10\n"
        "0\nLWPOLYLINE\n8\n3\n90\n4\n70\n1\n"
        "10\n100\n20\n0\n10\n200\n20\n0\n10\n200\n20\n50\n10\n100\n20\n50\n",
    ),
}


MAG_FIXTURES: dict[str, str] = {
    "single_box": (
        "magic\n"
        "tech scmos\n"
        "timestamp 0\n"
        "<< metal1 >>\n"
        "rect 0 0 100 50\n"
        "<< end >>\n"
    ),
    "multi_layer": (
        "magic\n"
        "tech scmos\n"
        "timestamp 0\n"
        "<< metal1 >>\n"
        "rect 0 0 50 50\n"
        "rect 100 0 200 50\n"
        "<< metal2 >>\n"
        "rect 25 100 75 200\n"
        "<< poly >>\n"
        "rect 0 300 30 330\n"
        "<< end >>\n"
    ),
    "many_rects": (
        "magic\n"
        "tech scmos\n"
        "timestamp 0\n"
        "<< metal1 >>\n"
        "rect 0 0 10 10\n"
        "rect 20 0 30 10\n"
        "rect 40 0 50 10\n"
        "rect 60 0 70 10\n"
        "rect 80 0 90 10\n"
        "rect 0 20 10 30\n"
        "rect 20 20 30 30\n"
        "<< end >>\n"
    ),
}


FORMAT_FIXTURES = {"cif": CIF_FIXTURES, "dxf": DXF_FIXTURES, "mag": MAG_FIXTURES}


def gen_format_roundtrip(fmt: str) -> None:
    """Write fixture .<fmt> files + KLayout's canonical JSON dump for each.
    Uses klayout.db.read directly — no Docker needed (klayout.db has
    built-in CIF/DXF/MAG readers, just no oracle for the higher-level
    formats LEF/Liberty/SPEF). The Rust side reads the same .<fmt> with
    our parser and asserts dumps match."""
    out_dir = CORPUS / fmt
    out_dir.mkdir(parents=True, exist_ok=True)
    fixtures = FORMAT_FIXTURES[fmt]
    for name, body in fixtures.items():
        src = out_dir / f"{name}.{fmt}"
        src.write_text(body)
        layout = db.Layout()
        try:
            layout.read(str(src))
        except Exception as e:
            print(f"  {name}: KLayout read failed ({e}); skipping", file=sys.stderr)
            continue
        dump = canonical_layout_dump(layout)
        json_p = out_dir / f"{name}.json"
        json_p.write_text(json.dumps(dump, indent=2, sort_keys=True))
        print(f"wrote {src.name} + {json_p.name}")


def main():
    CORPUS.mkdir(parents=True, exist_ok=True)
    if sys.argv[1:2] == ["verify"]:
        if len(sys.argv) != 3:
            print("usage: oracle.py verify <gds_path>", file=sys.stderr)
            sys.exit(2)
        verify_gds(sys.argv[2])
        return

    targets = sys.argv[1:] or [
        "trans", "bbox", "gds", "region", "drc", "polygon_ops", "oasis",
        "cif", "dxf", "mag",
    ]
    for t in targets:
        if t == "trans":
            (CORPUS / "trans.json").write_text(json.dumps(gen_trans(), indent=2))
            print("wrote trans.json")
        elif t == "bbox":
            (CORPUS / "bbox.json").write_text(json.dumps(gen_bbox(), indent=2))
            print("wrote bbox.json")
        elif t == "gds":
            print("generating gds fixtures...")
            gen_gds()
        elif t == "region":
            (CORPUS / "region.json").write_text(json.dumps(gen_region(), indent=2))
            print("wrote region.json")
        elif t == "drc":
            (CORPUS / "drc.json").write_text(json.dumps(gen_drc(), indent=2))
            print("wrote drc.json")
        elif t == "drc_density_grid":
            grid = gen_drc_density_grid()
            (CORPUS / "drc_density_grid.json").write_text(json.dumps(grid, indent=2))
            print(f"wrote drc_density_grid.json ({len(grid['cases'])} cases)")
        elif t == "polygon_ops":
            (CORPUS / "polygon_ops.json").write_text(
                json.dumps(gen_polygon_ops(), indent=2)
            )
            print("wrote polygon_ops.json")
        elif t == "oasis":
            print("generating oasis fixtures...")
            gen_oasis()
        elif t == "cif":
            print("generating cif fixtures...")
            gen_format_roundtrip("cif")
        elif t == "dxf":
            print("generating dxf fixtures...")
            gen_format_roundtrip("dxf")
        elif t == "mag":
            print("generating mag fixtures...")
            gen_format_roundtrip("mag")
        else:
            print(f"unknown target: {t}", file=sys.stderr)
            sys.exit(1)


if __name__ == "__main__":
    main()
