#!/usr/bin/env python3
"""External oracle harness using OpenROAD (OpenSTA + OpenDB) via Docker.

Dumps reference parses of formats that `klayout.db` can't read plus OpenROAD-only flows:
  - Liberty (.lib) -> via OpenSTA's `read_liberty`
  - SPEF       -> via OpenSTA's `read_spef`        (deferred)
  - LEF        -> via OpenDB's `read_lef`          (deferred)
  - DEF        -> via OpenDB's `read_def`          (deferred)
  - CTS downstream: mini DEF + `cts_demo.lib` + `clock_tree_synthesis`; CK bbox + `report_cts`
Usage:
    python validation/oracle_external.py liberty         # regen liberty corpus
    python validation/oracle_external.py cts_downstream  # regen DEF+CTS JSON (Docker)
    python validation/oracle_external.py all             # regen all sub-suites (inc. CTS)

Requires Docker. The first invocation pulls the OpenROAD image (~5GB);
subsequent runs are fast (the corpus is checked in, so CI does not need
Docker).
"""
from __future__ import annotations
import json
import os
import re
import shutil
import subprocess
import sys
import textwrap
from pathlib import Path

ROOT = Path(__file__).resolve().parent
CORPUS = ROOT / "corpus"
# Custom image extending openroad/orfs with our SPEF dumper.
# Build with `docker build -t klayout-rs-oracle:latest validation/docker/`.
IMAGE = "klayout-rs-oracle:latest"
OPENROAD_BIN = "/OpenROAD-flow-scripts/tools/install/OpenROAD/bin/openroad"
SPEF_DUMP_BIN = "/opt/oracle/spef_dump.py"


def docker_run(host_dir: Path, tcl_script: str) -> str:
    """Run OpenROAD with `tcl_script` inside a container that mounts
    `host_dir` at /work. Returns combined stdout."""
    script_path = host_dir / "_oracle_run.tcl"
    script_path.write_text(tcl_script)
    cmd = [
        "docker", "run", "--rm", "--platform", "linux/amd64",
        "-v", f"{host_dir}:/work",
        IMAGE, OPENROAD_BIN, "-no_init", "/work/_oracle_run.tcl",
    ]
    r = subprocess.run(cmd, capture_output=True, text=True)
    script_path.unlink(missing_ok=True)
    if r.returncode != 0:
        raise RuntimeError(f"openroad failed:\n{r.stdout}\n{r.stderr}")
    return r.stdout


# ---------------- Liberty ----------------

LIB_FIXTURES: list[tuple[str, str]] = [
    ("inv_buf", textwrap.dedent("""
        library(test) {
          time_unit : "1ns";
          voltage_unit : "1V";
          cell(INV) {
            area : 1.0;
            pin(A) { direction : input; capacitance : 0.50; }
            pin(Y) { direction : output; }
          }
          cell(BUF) {
            area : 2.0;
            pin(A) { direction : input; capacitance : 0.60; }
            pin(Y) { direction : output; }
          }
        }
    """).lstrip()),
    ("two_input", textwrap.dedent("""
        library(t2) {
          cell(NAND2) {
            area : 1.5;
            pin(A) { direction : input; capacitance : 0.40; }
            pin(B) { direction : input; capacitance : 0.45; }
            pin(Y) { direction : output; }
          }
          cell(NOR2) {
            area : 1.6;
            pin(A) { direction : input; capacitance : 0.35; }
            pin(B) { direction : input; capacitance : 0.42; }
            pin(Y) { direction : output; }
          }
        }
    """).lstrip()),
    ("ff_simple", textwrap.dedent("""
        library(tff) {
          cell(DFF) {
            area : 4.0;
            pin(D)  { direction : input; capacitance : 0.30; }
            pin(CK) { direction : input; capacitance : 0.50; clock : true; }
            pin(Q)  { direction : output; }
          }
        }
    """).lstrip()),
    ("complex_gates", textwrap.dedent("""
        library(cg) {
          cell(AOI21) {
            area : 2.5;
            pin(A1) { direction : input; capacitance : 0.40; }
            pin(A2) { direction : input; capacitance : 0.41; }
            pin(B)  { direction : input; capacitance : 0.50; }
            pin(Y)  { direction : output; }
          }
          cell(OAI22) {
            area : 3.0;
            pin(A1) { direction : input; capacitance : 0.38; }
            pin(A2) { direction : input; capacitance : 0.39; }
            pin(B1) { direction : input; capacitance : 0.42; }
            pin(B2) { direction : input; capacitance : 0.43; }
            pin(Y)  { direction : output; }
          }
        }
    """).lstrip()),
    ("multi_output", textwrap.dedent("""
        library(mo) {
          cell(HA) {
            area : 1.8;
            pin(A) { direction : input; capacitance : 0.35; }
            pin(B) { direction : input; capacitance : 0.36; }
            pin(S) { direction : output; }
            pin(C) { direction : output; }
          }
          cell(MUX2) {
            area : 2.2;
            pin(I0) { direction : input; capacitance : 0.40; }
            pin(I1) { direction : input; capacitance : 0.41; }
            pin(S)  { direction : input; capacitance : 0.30; }
            pin(Z)  { direction : output; }
          }
        }
    """).lstrip()),
    ("inout_pins", textwrap.dedent("""
        library(io) {
          cell(BIDIR) {
            area : 5.0;
            pin(IO) { direction : inout; capacitance : 1.20; }
            pin(EN) { direction : input; capacitance : 0.40; }
          }
        }
    """).lstrip()),
]


def gen_liberty(out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    work = out_dir
    for name, body in LIB_FIXTURES:
        lib_path = out_dir / f"{name}.lib"
        lib_path.write_text(body)

    # Build one big TCL script that loads each fixture in its own
    # `read_liberty` call and emits a `=== name` delimiter so we can
    # split the output back per-fixture. Running them separately would
    # need one container start per file (slow under emulation).
    tcl_lines = []
    for name, _ in LIB_FIXTURES:
        tcl_lines.append(f'puts "=== {name}"')
        tcl_lines.append(f'set lib [read_liberty /work/{name}.lib]')
        tcl_lines.append('foreach cell [get_lib_cells *] {')
        tcl_lines.append('  set cn [get_property $cell name]')
        tcl_lines.append('  set ar [get_property $cell area]')
        tcl_lines.append('  puts "CELL $cn area=$ar"')
        tcl_lines.append('  foreach pin [get_lib_pins -of_objects $cell] {')
        tcl_lines.append('    set pn [get_property $pin name]')
        tcl_lines.append('    set dir [get_property $pin direction]')
        tcl_lines.append('    set cap [get_property $pin capacitance]')
        tcl_lines.append('    puts "  PIN $pn dir=$dir cap=$cap"')
        tcl_lines.append('  }')
        tcl_lines.append('}')
        # Each library is sticky in the global namespace; clear before
        # loading the next so we don't pick up stale cells.
        tcl_lines.append('foreach c [get_lib_cells *] { }')
    tcl_lines.append('exit')
    tcl = "\n".join(tcl_lines) + "\n"

    out = docker_run(work, tcl)

    # Split per-fixture and parse the lines into structured JSON.
    sections: dict[str, list[str]] = {}
    cur: str | None = None
    for line in out.splitlines():
        if line.startswith("=== "):
            cur = line[4:].strip()
            sections[cur] = []
        elif cur is not None:
            sections[cur].append(line)

    for name, lines in sections.items():
        cells_seen: list[dict] = []
        cur_cell: dict | None = None
        for ln in lines:
            s = ln.strip()
            if s.startswith("CELL "):
                # CELL <name> area=<area>
                if cur_cell is not None:
                    cells_seen.append(cur_cell)
                parts = s.split()
                cell_name = parts[1]
                area = None
                for tok in parts[2:]:
                    if tok.startswith("area="):
                        v = tok.split("=", 1)[1]
                        area = float(v) if v not in ("", "INF") else None
                cur_cell = {"name": cell_name, "area": area, "pins": []}
            elif s.startswith("PIN "):
                parts = s.split()
                pin_name = parts[1]
                direction = None
                cap = None
                for tok in parts[2:]:
                    if tok.startswith("dir="):
                        direction = tok.split("=", 1)[1]
                    elif tok.startswith("cap="):
                        cap = float(tok.split("=", 1)[1])
                if cur_cell is not None:
                    cur_cell["pins"].append({
                        "name": pin_name, "direction": direction, "cap": cap
                    })
        if cur_cell is not None:
            cells_seen.append(cur_cell)
        # Filter out cells that aren't from this library (carryover from
        # the global lookup). We re-read the source to know which cells
        # the .lib defined.
        src = (out_dir / f"{name}.lib").read_text()
        defined = set()
        i = 0
        while True:
            j = src.find("cell(", i)
            if j == -1:
                break
            k = src.find(")", j)
            if k == -1:
                break
            defined.add(src[j + 5:k].strip())
            i = k + 1
        cells_seen = [c for c in cells_seen if c["name"] in defined]
        # Canonical order: cells by name, pins by name. Lets the Rust
        # side compare without depending on source-order or
        # OpenSTA-internal-storage-order.
        for c in cells_seen:
            c["pins"].sort(key=lambda p: p["name"])
        cells_seen.sort(key=lambda c: c["name"])

        json_path = out_dir / f"{name}.json"
        json_path.write_text(json.dumps({"cells": cells_seen}, indent=2, sort_keys=True))
        print(f"wrote {json_path.relative_to(ROOT.parent)}")


# ---------------- SPEF ----------------

SPEF_FIXTURES: list[tuple[str, str]] = [
    ("two_net", textwrap.dedent("""
        *SPEF "IEEE 1481-1998"
        *DESIGN "top"
        *DATE "now"
        *VENDOR "test"
        *PROGRAM "klayout-rs"
        *VERSION "0.1"
        *DESIGN_FLOW ""
        *DIVIDER /
        *DELIMITER :
        *BUS_DELIMITER [ ]
        *T_UNIT 1 NS
        *C_UNIT 1 FF
        *R_UNIT 1 OHM
        *L_UNIT 1 HENRY

        *NAME_MAP
        *1 net1
        *2 net2

        *D_NET *1 1.500
        *CONN
        *I U1:Y I
        *I U2:A I
        *END

        *D_NET *2 2.250
        *CONN
        *I U2:Y I
        *I U3:A I
        *END
    """).lstrip()),
    ("many_nets", textwrap.dedent("""
        *SPEF "IEEE 1481-1998"
        *DESIGN "many"
        *DATE "now"
        *VENDOR "test"
        *PROGRAM "klayout-rs"
        *VERSION "0.1"
        *DESIGN_FLOW ""
        *DIVIDER /
        *DELIMITER :
        *BUS_DELIMITER [ ]
        *T_UNIT 1 NS
        *C_UNIT 1 FF
        *R_UNIT 1 OHM
        *L_UNIT 1 HENRY

        *NAME_MAP
        *1 n1
        *2 n2
        *3 n3
        *4 n4
        *5 n5

        *D_NET *1 0.500
        *CONN
        *I U1:Y I
        *I U2:A I
        *END

        *D_NET *2 0.750
        *CONN
        *I U2:Y I
        *I U3:A I
        *END

        *D_NET *3 1.250
        *CONN
        *I U3:Y I
        *I U4:A I
        *I U5:A I
        *END

        *D_NET *4 0.420
        *CONN
        *I U4:Y I
        *I U6:A I
        *END

        *D_NET *5 1.100
        *CONN
        *I U5:Y I
        *I U6:B I
        *END
    """).lstrip()),
    ("multi_coupling", textwrap.dedent("""
        *SPEF "IEEE 1481-1998"
        *DESIGN "mc"
        *DATE "now"
        *VENDOR "test"
        *PROGRAM "klayout-rs"
        *VERSION "0.1"
        *DESIGN_FLOW ""
        *DIVIDER /
        *DELIMITER :
        *BUS_DELIMITER [ ]
        *T_UNIT 1 NS
        *C_UNIT 1 FF
        *R_UNIT 1 OHM
        *L_UNIT 1 HENRY

        *NAME_MAP
        *1 victim
        *2 agg1
        *3 agg2
        *4 agg3

        *D_NET *1 2.000
        *CONN
        *I A:Y I
        *I B:A I
        *CAP
        1 *1:GROUND *2:GROUND 0.250
        2 *1:GROUND *3:GROUND 0.180
        3 *1:GROUND *4:GROUND 0.075
        *END

        *D_NET *2 0.500
        *CONN
        *I X1:Y I
        *I X2:A I
        *END

        *D_NET *3 0.500
        *CONN
        *I Y1:Y I
        *I Y2:A I
        *END

        *D_NET *4 0.500
        *CONN
        *I Z1:Y I
        *I Z2:A I
        *END
    """).lstrip()),
    ("with_coupling", textwrap.dedent("""
        *SPEF "IEEE 1481-1998"
        *DESIGN "top2"
        *DATE "now"
        *VENDOR "test"
        *PROGRAM "klayout-rs"
        *VERSION "0.1"
        *DESIGN_FLOW ""
        *DIVIDER /
        *DELIMITER :
        *BUS_DELIMITER [ ]
        *T_UNIT 1 NS
        *C_UNIT 1 FF
        *R_UNIT 1 OHM
        *L_UNIT 1 HENRY

        *NAME_MAP
        *1 a
        *2 b
        *3 c

        *D_NET *1 1.000
        *CONN
        *I X1:Y I
        *I X2:A I
        *CAP
        1 *1:GROUND *2:GROUND 0.150
        2 *1:GROUND *3:GROUND 0.075
        *END

        *D_NET *2 1.200
        *CONN
        *I X3:Y I
        *I X4:A I
        *END

        *D_NET *3 0.800
        *CONN
        *I X5:Y I
        *I X6:A I
        *END
    """).lstrip()),
]


def gen_spef(out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    for name, body in SPEF_FIXTURES:
        spef_path = out_dir / f"{name}.spef"
        spef_path.write_text(body)
        json_path = out_dir / f"{name}.json"
        cmd = [
            "docker", "run", "--rm", "--platform", "linux/amd64",
            "-v", f"{out_dir}:/work",
            IMAGE, "python3", SPEF_DUMP_BIN,
            f"/work/{name}.spef", f"/work/{name}.json",
        ]
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            raise RuntimeError(f"spef_dump failed for {name}:\n{r.stdout}\n{r.stderr}")
        print(f"wrote {json_path.relative_to(ROOT.parent)}")


# ---------------- LEF / DEF ----------------

LEF_TECH = textwrap.dedent("""
    VERSION 5.7 ;
    BUSBITCHARS "[]" ;
    DIVIDERCHAR "/" ;
    UNITS
      DATABASE MICRONS 1000 ;
    END UNITS

    LAYER metal1
      TYPE ROUTING ;
      DIRECTION HORIZONTAL ;
      WIDTH 0.14 ;
      PITCH 0.28 ;
    END metal1

    LAYER metal2
      TYPE ROUTING ;
      DIRECTION VERTICAL ;
      WIDTH 0.14 ;
      PITCH 0.28 ;
    END metal2

    SITE core
      CLASS CORE ;
      SIZE 0.46 BY 1.4 ;
    END core

    END LIBRARY
""").lstrip()

LEF_CELLS = textwrap.dedent("""
    VERSION 5.7 ;
    BUSBITCHARS "[]" ;
    DIVIDERCHAR "/" ;

    MACRO INV
      CLASS CORE ;
      ORIGIN 0 0 ;
      SIZE 0.92 BY 1.4 ;
      SYMMETRY X Y ;
      SITE core ;
      PIN A
        DIRECTION INPUT ; USE SIGNAL ;
        PORT LAYER metal1 ; RECT 0.1 0.5 0.3 0.9 ; END
      END A
      PIN Y
        DIRECTION OUTPUT ; USE SIGNAL ;
        PORT LAYER metal1 ; RECT 0.6 0.5 0.8 0.9 ; END
      END Y
    END INV

    MACRO BUF
      CLASS CORE ;
      ORIGIN 0 0 ;
      SIZE 1.38 BY 1.4 ;
      SYMMETRY X Y ;
      SITE core ;
      PIN A
        DIRECTION INPUT ; USE SIGNAL ;
        PORT LAYER metal1 ; RECT 0.1 0.5 0.3 0.9 ; END
      END A
      PIN Y
        DIRECTION OUTPUT ; USE SIGNAL ;
        PORT LAYER metal1 ; RECT 1.05 0.5 1.25 0.9 ; END
      END Y
    END BUF

    MACRO NAND2
      CLASS CORE ;
      ORIGIN 0 0 ;
      SIZE 1.84 BY 1.4 ;
      SYMMETRY X Y ;
      SITE core ;
      PIN A
        DIRECTION INPUT ; USE SIGNAL ;
        PORT LAYER metal1 ; RECT 0.1 0.5 0.3 0.9 ; END
      END A
      PIN B
        DIRECTION INPUT ; USE SIGNAL ;
        PORT LAYER metal1 ; RECT 0.55 0.5 0.75 0.9 ; END
      END B
      PIN Y
        DIRECTION OUTPUT ; USE SIGNAL ;
        PORT LAYER metal1 ; RECT 1.55 0.5 1.75 0.9 ; END
      END Y
    END NAND2

    MACRO NOR2
      CLASS CORE ;
      ORIGIN 0 0 ;
      SIZE 1.84 BY 1.4 ;
      SYMMETRY X Y ;
      SITE core ;
      PIN A
        DIRECTION INPUT ; USE SIGNAL ;
        PORT LAYER metal2 ; RECT 0.1 0.45 0.3 0.95 ; END
      END A
      PIN B
        DIRECTION INPUT ; USE SIGNAL ;
        PORT LAYER metal2 ; RECT 0.55 0.45 0.75 0.95 ; END
      END B
      PIN Y
        DIRECTION OUTPUT ; USE SIGNAL ;
        PORT LAYER metal2 ; RECT 1.55 0.45 1.75 0.95 ; END
      END Y
    END NOR2

    MACRO DFF
      CLASS CORE ;
      ORIGIN 0 0 ;
      SIZE 3.68 BY 1.4 ;
      SYMMETRY X Y ;
      SITE core ;
      PIN D
        DIRECTION INPUT ; USE SIGNAL ;
        PORT LAYER metal1 ; RECT 0.1 0.5 0.3 0.9 ; END
      END D
      PIN CK
        DIRECTION INPUT ; USE CLOCK ;
        PORT LAYER metal1 ; RECT 0.55 0.5 0.75 0.9 ; END
      END CK
      PIN Q
        DIRECTION OUTPUT ; USE SIGNAL ;
        PORT LAYER metal1 ; RECT 3.4 0.5 3.6 0.9 ; END
      END Q
    END DFF

    END LIBRARY
""").lstrip()

DEF_FIXTURES: list[tuple[str, str]] = [
    ("inv_chain", textwrap.dedent("""
        VERSION 5.7 ;
        DIVIDERCHAR "/" ;
        BUSBITCHARS "[]" ;
        DESIGN top ;
        UNITS DISTANCE MICRONS 1000 ;
        DIEAREA ( 0 0 ) ( 10000 10000 ) ;

        COMPONENTS 3 ;
        - U1 INV + PLACED ( 1000 1000 ) N ;
        - U2 INV + PLACED ( 3000 1000 ) N ;
        - U3 INV + PLACED ( 5000 1000 ) N ;
        END COMPONENTS

        NETS 2 ;
        - net1 ( U1 Y ) ( U2 A ) ;
        - net2 ( U2 Y ) ( U3 A ) ;
        END NETS

        END DESIGN
    """).lstrip()),
    ("orientations", textwrap.dedent("""
        VERSION 5.7 ;
        DIVIDERCHAR "/" ;
        BUSBITCHARS "[]" ;
        DESIGN top ;
        UNITS DISTANCE MICRONS 1000 ;
        DIEAREA ( 0 0 ) ( 20000 20000 ) ;

        COMPONENTS 8 ;
        - U_N  INV + PLACED ( 1000  1000 ) N ;
        - U_S  INV + PLACED ( 1000  3000 ) S ;
        - U_E  INV + PLACED ( 1000  5000 ) E ;
        - U_W  INV + PLACED ( 1000  7000 ) W ;
        - U_FN INV + PLACED ( 1000  9000 ) FN ;
        - U_FS INV + PLACED ( 1000 11000 ) FS ;
        - U_FE INV + PLACED ( 1000 13000 ) FE ;
        - U_FW INV + PLACED ( 1000 15000 ) FW ;
        END COMPONENTS

        NETS 0 ;
        END NETS

        END DESIGN
    """).lstrip()),
    ("mixed_cells", textwrap.dedent("""
        VERSION 5.7 ;
        DIVIDERCHAR "/" ;
        BUSBITCHARS "[]" ;
        DESIGN top ;
        UNITS DISTANCE MICRONS 1000 ;
        DIEAREA ( 0 0 ) ( 20000 5000 ) ;

        COMPONENTS 4 ;
        - U1 INV + PLACED ( 1000 1000 ) N ;
        - U2 BUF + PLACED ( 3000 1000 ) N ;
        - U3 INV + PLACED ( 6000 1000 ) N ;
        - U4 BUF + PLACED ( 8000 1000 ) N ;
        END COMPONENTS

        NETS 3 ;
        - n1 ( U1 Y ) ( U2 A ) ;
        - n2 ( U2 Y ) ( U3 A ) ;
        - n3 ( U3 Y ) ( U4 A ) ;
        END NETS

        END DESIGN
    """).lstrip()),
    ("nand_chain", textwrap.dedent("""
        VERSION 5.7 ;
        DIVIDERCHAR "/" ;
        BUSBITCHARS "[]" ;
        DESIGN top ;
        UNITS DISTANCE MICRONS 1000 ;
        DIEAREA ( 0 0 ) ( 30000 5000 ) ;

        COMPONENTS 6 ;
        - G1 NAND2 + PLACED (  1000 1000 ) N ;
        - G2 NAND2 + PLACED (  4000 1000 ) N ;
        - G3 NOR2  + PLACED (  7000 1000 ) N ;
        - G4 NAND2 + PLACED ( 10000 1000 ) N ;
        - G5 INV   + PLACED ( 13000 1000 ) N ;
        - G6 BUF   + PLACED ( 14500 1000 ) N ;
        END COMPONENTS

        NETS 5 ;
        - w1 ( G1 Y ) ( G3 A ) ;
        - w2 ( G2 Y ) ( G3 B ) ;
        - w3 ( G3 Y ) ( G4 A ) ;
        - w4 ( G4 Y ) ( G5 A ) ;
        - w5 ( G5 Y ) ( G6 A ) ;
        END NETS

        END DESIGN
    """).lstrip()),
    ("with_routes", textwrap.dedent("""
        VERSION 5.7 ;
        DIVIDERCHAR "/" ;
        BUSBITCHARS "[]" ;
        DESIGN top ;
        UNITS DISTANCE MICRONS 1000 ;
        DIEAREA ( 0 0 ) ( 30000 10000 ) ;

        COMPONENTS 3 ;
        - U1 INV + PLACED (  1000 1000 ) N ;
        - U2 BUF + PLACED ( 10000 1000 ) N ;
        - U3 INV + PLACED ( 20000 1000 ) N ;
        END COMPONENTS

        PINS 2 ;
        - IN  + NET in_n  + DIRECTION INPUT  + USE SIGNAL
            + LAYER metal1 ( -50 -50 ) ( 50 50 ) + PLACED ( 0 5000 ) N ;
        - OUT + NET out_n + DIRECTION OUTPUT + USE SIGNAL
            + LAYER metal1 ( -50 -50 ) ( 50 50 ) + PLACED ( 30000 5000 ) N ;
        END PINS

        NETS 2 ;
        - n1 ( U1 Y ) ( U2 A )
            + ROUTED metal1 ( 1500 1200 ) ( 9500 1200 ) ;
        - n2 ( U2 Y ) ( U3 A )
            + ROUTED metal2 ( 11000 1200 ) ( 19500 1200 ) ;
        END NETS

        END DESIGN
    """).lstrip()),
    ("seq_design", textwrap.dedent("""
        VERSION 5.7 ;
        DIVIDERCHAR "/" ;
        BUSBITCHARS "[]" ;
        DESIGN seq ;
        UNITS DISTANCE MICRONS 1000 ;
        DIEAREA ( 0 0 ) ( 50000 10000 ) ;

        COMPONENTS 4 ;
        - FF1 DFF + PLACED (  1000 1000 ) N ;
        - FF2 DFF + PLACED ( 10000 1000 ) N ;
        - C1  INV + PLACED (  6000 1000 ) N ;
        - CK_BUF BUF + PLACED ( 1000 5000 ) N ;
        END COMPONENTS

        NETS 4 ;
        - clk     ( CK_BUF Y ) ( FF1 CK ) ( FF2 CK ) ;
        - data    ( FF1 Q ) ( C1 A ) ;
        - data_bar ( C1 Y ) ( FF2 D ) ;
        - clk_in  ( CK_BUF A ) ;
        END NETS

        END DESIGN
    """).lstrip()),
]


def _lef_dump_tcl() -> str:
    return textwrap.dedent("""
        read_lef /work/tech.lef
        read_lef /work/cells.lef
        set db [ord::get_db]
        set tech [$db getTech]
        foreach layer [$tech getLayers] {
          puts "LAYER [$layer getName] type=[$layer getType]"
        }
        foreach lib [$db getLibs] {
          foreach m [$lib getMasters] {
            puts "MACRO [$m getName] w=[$m getWidth] h=[$m getHeight]"
            foreach mt [$m getMTerms] {
              puts "  PIN [$mt getName] dir=[$mt getIoType] use=[$mt getSigType]"
              foreach mpin [$mt getMPins] {
                foreach geom [$mpin getGeometry] {
                  set lay [[$geom getTechLayer] getName]
                  puts "    RECT layer=$lay xmin=[$geom xMin] ymin=[$geom yMin] xmax=[$geom xMax] ymax=[$geom yMax]"
                }
              }
            }
          }
        }
        exit
    """).lstrip()


def _def_dump_tcl(def_name: str) -> str:
    template = textwrap.dedent("""
        read_lef /work/tech.lef
        read_lef /work/cells.lef
        read_def /work/__DEF_NAME__.def
        set blk [[[ord::get_db] getChip] getBlock]
        set die [$blk getDieArea]
        puts "DESIGN [$blk getName] die=[$die xMin] [$die yMin] [$die xMax] [$die yMax]"
        foreach inst [$blk getInsts] {
          set ox [$inst getOrigin]
          puts "INST [$inst getName] master=[[$inst getMaster] getName] x=[lindex $ox 0] y=[lindex $ox 1]"
        }
        foreach bterm [$blk getBTerms] {
          set sig [$bterm getSigType]
          set io  [$bterm getIoType]
          puts "BPIN [$bterm getName] dir=$io use=$sig"
        }
        foreach net [$blk getNets] {
          set conns [list]
          foreach iterm [$net getITerms] {
            lappend conns "[[$iterm getInst] getName]:[[$iterm getMTerm] getName]"
          }
          set sorted [lsort $conns]
          puts "NET [$net getName] conns=[join $sorted { }]"
          # Routed wires: for each segment, dump layer + (x1,y1)-(x2,y2).
          set wire [$net getWire]
          if { $wire ne "NULL" } {
            set decoder [odb::dbWireDecoder]
            $decoder begin $wire
            set last_x 0
            set last_y 0
            set cur_layer ""
            while { 1 } {
              set op [$decoder next]
              if { $op == "END_DECODE" } { break }
              if { $op == "PATH" || $op == "SHORT" || $op == "VWIRE" } {
                set cur_layer [[$decoder getLayer] getName]
                set last_x 0
                set last_y 0
              } elseif { $op == "POINT" } {
                set p [$decoder getPoint]
                set x [lindex $p 0]
                set y [lindex $p 1]
                if { $cur_layer ne "" && ($x != $last_x || $y != $last_y) } {
                  puts "  WIRE net=[$net getName] layer=$cur_layer x1=$last_x y1=$last_y x2=$x y2=$y"
                }
                set last_x $x
                set last_y $y
              }
            }
          }
        }
        exit
    """).lstrip()
    return template.replace("__DEF_NAME__", def_name)


def _docker_openroad(host_dir: Path, tcl: str) -> str:
    script = host_dir / "_oracle_run.tcl"
    script.write_text(tcl)
    cmd = [
        "docker", "run", "--rm", "--platform", "linux/amd64",
        "-v", f"{host_dir}:/work",
        IMAGE, "openroad", "-no_init", "/work/_oracle_run.tcl",
    ]
    r = subprocess.run(cmd, capture_output=True, text=True)
    script.unlink(missing_ok=True)
    if r.returncode != 0:
        raise RuntimeError(f"openroad failed:\n{r.stdout}\n{r.stderr}")
    return r.stdout


def gen_lef(out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "tech.lef").write_text(LEF_TECH)
    (out_dir / "cells.lef").write_text(LEF_CELLS)

    out = _docker_openroad(out_dir, _lef_dump_tcl())
    layers: list[dict] = []
    macros: list[dict] = []
    cur_macro: dict | None = None
    cur_pin: dict | None = None
    for line in out.splitlines():
        s = line.strip()
        if s.startswith("LAYER "):
            parts = s.split()
            layer = {"name": parts[1]}
            for tok in parts[2:]:
                if tok.startswith("type="):
                    layer["type"] = tok.split("=", 1)[1]
            layers.append(layer)
        elif s.startswith("MACRO "):
            if cur_macro is not None:
                if cur_pin is not None:
                    cur_macro["pins"].append(cur_pin)
                    cur_pin = None
                macros.append(cur_macro)
            parts = s.split()
            cur_macro = {"name": parts[1], "pins": []}
            for tok in parts[2:]:
                if tok.startswith("w="):
                    cur_macro["width_dbu"] = int(tok.split("=", 1)[1])
                elif tok.startswith("h="):
                    cur_macro["height_dbu"] = int(tok.split("=", 1)[1])
        elif s.startswith("PIN "):
            if cur_pin is not None and cur_macro is not None:
                cur_macro["pins"].append(cur_pin)
            parts = s.split()
            cur_pin = {"name": parts[1], "ports": []}
            for tok in parts[2:]:
                if tok.startswith("dir="):
                    cur_pin["direction"] = tok.split("=", 1)[1]
                elif tok.startswith("use="):
                    cur_pin["use"] = tok.split("=", 1)[1]
        elif s.startswith("RECT "):
            parts = s.split()
            rect = {}
            for tok in parts[1:]:
                k, v = tok.split("=", 1)
                rect[k] = int(v) if k != "layer" else v
            if cur_pin is not None:
                cur_pin["ports"].append(rect)
    if cur_pin is not None and cur_macro is not None:
        cur_macro["pins"].append(cur_pin)
    if cur_macro is not None:
        macros.append(cur_macro)
    for m in macros:
        m["pins"].sort(key=lambda p: p["name"])
        for p in m["pins"]:
            p["ports"].sort(key=lambda r: (r["layer"], r["xmin"], r["ymin"]))
    macros.sort(key=lambda m: m["name"])
    layers.sort(key=lambda l: l["name"])
    (out_dir / "lef.json").write_text(
        json.dumps({"layers": layers, "macros": macros}, indent=2, sort_keys=True)
    )
    print(f"wrote validation/corpus/lef/lef.json")


def gen_def(out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    # DEF needs the LEFs alongside.
    (out_dir / "tech.lef").write_text(LEF_TECH)
    (out_dir / "cells.lef").write_text(LEF_CELLS)
    for name, body in DEF_FIXTURES:
        (out_dir / f"{name}.def").write_text(body)
        out = _docker_openroad(out_dir, _def_dump_tcl(name))
        design = ""
        die: list[int] = []
        instances: list[dict] = []
        nets: list[dict] = []
        bpins: list[dict] = []
        wires_by_net: dict[str, list[dict]] = {}
        for line in out.splitlines():
            s = line.strip()
            if s.startswith("DESIGN "):
                parts = s.split()
                design = parts[1]
                if "die=" in s:
                    di = s.index("die=") + 4
                    die = [int(x) for x in s[di:].split()]
            elif s.startswith("INST "):
                parts = s.split()
                inst = {"name": parts[1]}
                for tok in parts[2:]:
                    if tok.startswith("master="):
                        inst["master"] = tok.split("=", 1)[1]
                    elif tok.startswith("x="):
                        inst["x_dbu"] = int(tok.split("=", 1)[1])
                    elif tok.startswith("y="):
                        inst["y_dbu"] = int(tok.split("=", 1)[1])
                instances.append(inst)
            elif s.startswith("BPIN "):
                parts = s.split()
                bp = {"name": parts[1]}
                for tok in parts[2:]:
                    if tok.startswith("dir="):
                        bp["direction"] = tok.split("=", 1)[1]
                    elif tok.startswith("use="):
                        bp["use"] = tok.split("=", 1)[1]
                bpins.append(bp)
            elif s.startswith("NET "):
                parts = s.split(None, 2)
                nm = parts[1]
                conns: list[str] = []
                if len(parts) >= 3 and parts[2].startswith("conns="):
                    conns = parts[2][len("conns="):].split()
                nets.append({"name": nm, "connections": sorted(conns)})
            elif s.startswith("WIRE "):
                parts = s.split()
                w = {}
                nm = ""
                for tok in parts[1:]:
                    k, v = tok.split("=", 1)
                    if k == "net":
                        nm = v
                    elif k == "layer":
                        w[k] = v
                    else:
                        w[k] = int(v)
                wires_by_net.setdefault(nm, []).append(w)
        instances.sort(key=lambda i: i["name"])
        bpins.sort(key=lambda b: b["name"])
        for n in nets:
            ws = wires_by_net.get(n["name"], [])
            ws.sort(key=lambda w: (w["layer"], w["x1"], w["y1"], w["x2"], w["y2"]))
            n["wires"] = ws
        nets.sort(key=lambda n: n["name"])
        (out_dir / f"{name}.json").write_text(
            json.dumps(
                {
                    "design": design,
                    "die": die,
                    "instances": instances,
                    "bpins": bpins,
                    "nets": nets,
                },
                indent=2,
                sort_keys=True,
            )
        )
        print(f"wrote validation/corpus/def/{name}.json")


# ---------------- CTS downstream (DEF + Liberty + TritonCTS) ----------------

REPO_ROOT = ROOT.parent


def _between(text: str, start: str, end: str) -> str:
    i = text.find(start)
    if i < 0:
        raise RuntimeError(f"missing marker {start!r} in openroad output")
    i += len(start)
    j = text.find(end, i)
    if j < 0:
        raise RuntimeError(f"missing marker {end!r} in openroad output")
    return text[i:j].strip()


def _parse_sink_ck_centers(block: str) -> list[tuple[str, int, int]]:
    out: list[tuple[str, int, int]] = []
    for line in block.splitlines():
        s = line.strip()
        if not s.startswith("SINK_CK "):
            continue
        parts = s.split()
        if len(parts) < 6:
            continue
        inst = parts[1]
        xmin, ymin, xmax, ymax = (int(parts[2]), int(parts[3]), int(parts[4]), int(parts[5]))
        out.append((inst, (xmin + xmax) // 2, (ymin + ymax) // 2))
    out.sort(key=lambda t: t[0])
    return out


def _parse_openroad_report(txt: str) -> dict:
    """Structured fields from ``report_cts`` output (+ buffer master counts)."""
    m: dict = {}
    for line in txt.splitlines():
        line_l = line.strip()
        for pat, key in (
            (r"Clock Roots:\s*(\d+)", "clock_roots"),
            (r"Buffers Inserted:\s*(\d+)", "buffers_inserted"),
            (r"Clock Subnets:\s*(\d+)", "clock_subnets"),
            (r"Total number of Sinks:\s*(\d+)", "sinks"),
        ):
            mo = re.search(pat, line_l, re.I)
            if mo:
                m[key] = int(mo.group(1))

    in_bufs = False
    bu: dict[str, int] = {}
    for line in txt.splitlines():
        sl = line.strip()
        if sl.startswith("Buffers used"):
            in_bufs = True
            continue
        if in_bufs:
            if not sl:
                in_bufs = False
                continue
            mo = re.match(r"^(\S+)\s*:\s*(\d+)\s*$", sl)
            if mo:
                bu[mo.group(1)] = int(mo.group(2))

    if bu:
        m["buffer_usage"] = dict(sorted(bu.items()))
    return m


def _rust_dme_expect(
    source: tuple[int, int], sinks: list[tuple[str, int, int]]
) -> dict:
    payload = {
        "source": [source[0], source[1]],
        "sinks_ck_dbu": [[n, x, y] for (n, x, y) in sinks],
        "dme_config": {"allow_detour": True},
    }
    cmd = [
        "cargo",
        "run",
        "-q",
        "-p",
        "klayout-cts",
        "--example",
        "emit_dme_metrics",
    ]
    r = subprocess.run(
        cmd,
        cwd=REPO_ROOT,
        input=json.dumps(payload).encode(),
        capture_output=True,
    )
    if r.returncode != 0:
        raise RuntimeError(
            f"emit_dme_metrics failed: {r.stderr.decode()}{r.stdout.decode()}"
        )
    line = r.stdout.decode().strip().splitlines()[-1]
    return json.loads(line)


_ORACLE_CTS_TCL = textwrap.dedent(
    r"""
    read_lef /work/tech.lef
    read_lef /work/cells.lef
    read_liberty /work/cts_demo.lib
    read_def /work/current.def
    create_clock -period 10 [get_ports clk]

    set_wire_rc -signal -layer metal1 -h_resistance 2.0e-04 -h_capacitance 0.10e-15
    set_wire_rc -signal -layer metal2 -v_resistance 2.0e-04 -v_capacitance 0.10e-15
    set_wire_rc -clock -layer metal2 -v_resistance 2.0e-04 -v_capacitance 0.12e-15

    puts OPENRO_DUMP_SINK_START
    set blk [[[ord::get_db] getChip] getBlock]
    foreach inst [$blk getInsts] {
      set nm [$inst getName]
      foreach iterm [$inst getITerms] {
        set mt [$iterm getMTerm]
        if {[$mt getName] eq "CK"} {
          set box [$iterm getBBox]
          puts "SINK_CK $nm [$box xMin] [$box yMin] [$box xMax] [$box yMax]"
        }
      }
    }
    puts OPENRO_DUMP_SINK_END

    set_cts_config -root_buf BUF -buf_list BUF -wire_unit 25
    clock_tree_synthesis -buf_list BUF
    report_cts -out_file /work/__rpt__

    puts OPENRO_REPORT_BODY_START
    puts [exec cat /work/__rpt__]
    puts OPENRO_REPORT_BODY_END

    exit
    """
).strip()


def gen_cts_downstream() -> None:
    work = CORPUS / "cts_downstream"
    out_json = work / "cts_downstream.json"
    cases_js: list[dict] = []
    for case_name, def_fname in (
        ("two_ff", "two_ff.def"),
        ("four_ff", "four_ff.def"),
    ):
        shutil.copy(work / def_fname, work / "current.def")
        # Unique report file name (docker volume persists between runs).
        rpt_name = "report_" + case_name + ".txt"
        tcl = _ORACLE_CTS_TCL.replace("__rpt__", rpt_name)
        stdout = _docker_openroad(work, tcl)

        sinks = _parse_sink_ck_centers(
            _between(stdout, "OPENRO_DUMP_SINK_START", "OPENRO_DUMP_SINK_END")
        )
        if len(sinks) < 2:
            raise RuntimeError(f"{case_name}: expected >=2 sinks, got {sinks}")

        cx = round(sum(xy[1] for xy in sinks) / len(sinks))
        cy = round(sum(xy[2] for xy in sinks) / len(sinks))
        dme_expect = _rust_dme_expect((cx, cy), sinks)

        rpt_txt = _between(
            stdout, "OPENRO_REPORT_BODY_START", "OPENRO_REPORT_BODY_END"
        )
        openroad = _parse_openroad_report(rpt_txt)
        req = {
            "clock_roots",
            "buffers_inserted",
            "clock_subnets",
            "sinks",
            "buffer_usage",
        }
        if req - set(openroad.keys()):
            raise RuntimeError(f"{case_name}: bad report_cts parse: {openroad!r}")

        cases_js.append(
            {
                "name": case_name,
                "def_file": def_fname,
                "clock_net": "clk",
                "ck_pin": "CK",
                "dme_source_dbu": [cx, cy],
                "sinks_ck_dbu": [[n, x, y] for (n, x, y) in sinks],
                "openroad_after_cts": openroad,
                "dme_expect": dme_expect,
            }
        )

        (work / rpt_name).unlink(missing_ok=True)

    (work / "current.def").unlink(missing_ok=True)

    root = {
        "suite": "cts_downstream",
        "oracle": (
            "OpenROAD in klayout-rs-oracle:latest: read_def + read_liberty + "
            "clock_tree_synthesis; report_cts metrics + OpenDB CK bbox centers "
            "for parity with placement DEF + klayout_lef + klayout_cts DME."
        ),
        "dme_config": {"allow_detour": True},
        "cases": cases_js,
    }
    out_json.write_text(json.dumps(root, indent=2, sort_keys=True) + "\n")
    print(f"wrote {out_json.relative_to(REPO_ROOT)}")


# ---------------- main ----------------

def main():
    targets = sys.argv[1:] or ["all"]
    if "all" in targets:
        targets = ["liberty", "spef", "lef", "def", "cts_downstream"]
    for t in targets:
        if t == "liberty":
            gen_liberty(CORPUS / "liberty")
        elif t == "spef":
            gen_spef(CORPUS / "spef")
        elif t == "lef":
            gen_lef(CORPUS / "lef")
        elif t == "def":
            gen_def(CORPUS / "def")
        elif t == "cts_downstream":
            gen_cts_downstream()
        else:
            print(f"unknown target: {t}", file=sys.stderr)
            sys.exit(2)


if __name__ == "__main__":
    main()
