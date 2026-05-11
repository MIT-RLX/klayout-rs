#!/usr/bin/env python3
"""Regenerate KLayout LEF/DEF layout dumps for differential parity with klayout-rs.

Reads the same ``tech.lef`` + ``cells.lef`` + ``<name>.def`` triples as
``oracle_external.py`` / ``validation/klayout-validate/tests/def.rs``, imports
them with ``klayout.db`` LEF/DEF reader, then writes a **compact** canonical JSON
that compares semantic geometry (layer base names, no pin text labels).

Usage:
  python validation/oracle_klayout_lefdef.py           # all fixtures
  python validation/oracle_klayout_lefdef.py inv_chain

Requires ``klayout.db`` (same as ``oracle.py``). Override corpus root with
``ORACLE_CORPUS_DIR``.

Regenerated files: ``validation/corpus/klayout_lefdef/<name>.json``
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path

import klayout.db as db

_CORPUS_DEFAULT = Path(__file__).resolve().parent / "corpus"
CORPUS = Path(os.environ.get("ORACLE_CORPUS_DIR", str(_CORPUS_DEFAULT))).resolve()
OUT_DIR = CORPUS / "klayout_lefdef"
DEF_DIR = CORPUS / "def"

# Keep in sync with validation/klayout-validate/tests/def.rs
FIXTURES = [
    "inv_chain",
    "mixed_cells",
    "orientations",
    "nand_chain",
    "seq_design",
    "with_routes",
]


def layer_base(name: str) -> str:
    if not name:
        return ""
    base = name.split(".", 1)[0]
    if base == "DIEAREA":
        return "OUTLINE"
    return base


def instance_dump_parity(inst, layout) -> dict:
    sname = layout.cell(inst.cell_index).name
    t = inst.trans
    rec: dict = {
        "sname": sname,
        "trans": [int(t.rot), int(t.disp.x), int(t.disp.y)],
    }
    if inst.is_regular_array():
        rec["array"] = {
            "a": [inst.a.x, inst.a.y],
            "b": [inst.b.x, inst.b.y],
            "na": inst.na,
            "nb": inst.nb,
        }
    return rec


def _polygon_to_axis_aligned_box(hull: list) -> tuple[int, int, int, int] | None:
    """If hull is exactly four orthogonal corners, return (left,bottom,right,top)."""
    if len(hull) != 4:
        return None
    xs = [p[0] for p in hull]
    ys = [p[1] for p in hull]
    if len(set(xs)) != 2 or len(set(ys)) != 2:
        return None
    for i in range(4):
        x0, y0 = hull[i]
        x1, y1 = hull[(i + 1) % 4]
        if x0 != x1 and y0 != y1:
            return None
    return (min(xs), min(ys), max(xs), max(ys))


def shape_dump_parity(s, info) -> dict | None:
    """Drop text labels; emit layer_base instead of numeric GDS (varies by reader)."""
    if s.is_text():
        return None
    lb = layer_base(getattr(info, "name", None) or "")
    if s.is_polygon() or s.is_simple_polygon():
        poly = s.polygon
        hull = [[p.x, p.y] for p in poly.each_point_hull()]
        holes = []
        for hi in range(poly.holes()):
            holes.append([[p.x, p.y] for p in poly.each_point_hole(hi)])
        if not holes:
            rect = _polygon_to_axis_aligned_box(hull)
            if rect is not None:
                left, bottom, right, top = rect
                return {
                    "type": "box",
                    "layer_base": lb,
                    "left": left,
                    "bottom": bottom,
                    "right": right,
                    "top": top,
                }
        return {
            "type": "polygon",
            "layer_base": lb,
            "hull": hull,
            "holes": holes,
        }
    if s.is_box():
        b = s.box
        return {
            "type": "box",
            "layer_base": lb,
            "left": b.left,
            "bottom": b.bottom,
            "right": b.right,
            "top": b.top,
        }
    if s.is_path():
        p = s.path
        pts = [[pt.x, pt.y] for pt in p.each_point()]
        return {
            "type": "path",
            "layer_base": lb,
            "width": p.width,
            "points": pts,
            "begin_ext": p.bgn_ext,
            "end_ext": p.end_ext,
            "round": p.is_round(),
        }
    return {"type": "unknown", "layer_base": lb}


def compact_layout_dump(layout: db.Layout) -> dict:
    cells = []
    bottom_up = list(layout.each_cell_bottom_up())
    by_name = {layout.cell(ci).name: layout.cell(ci) for ci in bottom_up}
    for name in sorted(by_name.keys()):
        c = by_name[name]
        shapes = []
        for li in layout.layer_indices():
            info = layout.get_info(li)
            for s in c.shapes(li).each():
                d = shape_dump_parity(s, info)
                if d is not None:
                    shapes.append(d)
        shapes.sort(key=lambda s: json.dumps(s, sort_keys=True))

        instances = []
        for inst in c.each_inst():
            instances.append(instance_dump_parity(inst, layout))
        instances.sort(key=lambda i: json.dumps(i, sort_keys=True))

        cells.append({"name": c.name, "shapes": shapes, "instances": instances})

    return {
        "dbu_um": layout.dbu,
        "klayout_db_version": getattr(db, "__version__", "unknown"),
        "cells": cells,
    }


def read_lefdef(def_path: Path) -> db.Layout:
    work_dir = def_path.parent.resolve()
    tech = work_dir / "tech.lef"
    cells = work_dir / "cells.lef"
    if not tech.is_file() or not cells.is_file():
        raise FileNotFoundError(f"need tech.lef and cells.lef beside {def_path}")

    layout = db.Layout()
    opt = db.LoadLayoutOptions()
    conf = db.LEFDEFReaderConfiguration()
    conf.lef_files = [str(tech), str(cells)]
    conf.produce_lef_labels = False
    conf.produce_labels = False
    opt.lefdef_config = conf
    layout.read(str(def_path.resolve()), opt)
    return layout


def gen_fixture(name: str) -> None:
    def_path = DEF_DIR / f"{name}.def"
    if not def_path.is_file():
        raise FileNotFoundError(def_path)
    layout = read_lefdef(def_path)
    doc = compact_layout_dump(layout)
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    out_path = OUT_DIR / f"{name}.json"
    out_path.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n")
    print(f"wrote {out_path.relative_to(CORPUS.parent)}")


def main() -> None:
    names = sys.argv[1:] or FIXTURES
    for n in names:
        gen_fixture(n)


if __name__ == "__main__":
    main()
