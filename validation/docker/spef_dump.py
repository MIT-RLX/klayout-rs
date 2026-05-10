#!/usr/bin/env python3
"""SPEF -> JSON dumper used as the parity oracle.

Independent IEEE 1481 SPEF parser. Runs inside the
`klayout-rs-oracle` Docker image so tests are reproducible and don't
depend on the host's python install. Output is the canonical
`{"design": ..., "nets": [{"name", "total_cap", "connections", "coupling"}]}`
JSON shape consumed by `tests/spef.rs`.
"""
import json
import re
import sys


def parse_spef(text: str) -> dict:
    # Strip comments (// ... and /* ... */) to make line-based parsing
    # robust.
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    text = re.sub(r"//[^\n]*", "", text)

    design = ""
    name_map: dict[str, str] = {}
    nets: list[dict] = []

    # Pull *DESIGN.
    m = re.search(r'\*DESIGN\s+"([^"]*)"', text)
    if m:
        design = m.group(1)

    # Pull *NAME_MAP block: lines like `*1 net1`.
    nm_match = re.search(r"\*NAME_MAP([\s\S]*?)(?:\*PORTS|\*POWER_NETS|\*GROUND_NETS|\*D_NET)", text)
    if nm_match:
        for line in nm_match.group(1).splitlines():
            line = line.strip()
            if line.startswith("*") and not line.startswith("*NAME_MAP"):
                parts = line.split(None, 1)
                if len(parts) == 2 and parts[0].startswith("*"):
                    name_map[parts[0][1:]] = parts[1].strip()

    def resolve(tok: str) -> str:
        if tok.startswith("*"):
            tok = tok[1:]
            if ":" in tok:
                head, tail = tok.split(":", 1)
                return f"{name_map.get(head, head)}:{tail}"
            return name_map.get(tok, tok)
        return tok

    # Find each *D_NET ... *END block.
    for m in re.finditer(r"\*D_NET\s+(\S+)\s+(\S+)([\s\S]*?)\*END", text):
        net_ref = m.group(1)
        total_cap = float(m.group(2))
        body = m.group(3)
        net_name = resolve(net_ref)

        connections: list[str] = []
        coupling: list[tuple[str, float]] = []
        resistance = 0.0

        in_conn = False
        in_cap = False
        in_res = False
        for line in body.splitlines():
            s = line.strip()
            if not s:
                continue
            if s.startswith("*CONN"):
                in_conn, in_cap, in_res = True, False, False
                continue
            if s.startswith("*CAP"):
                in_conn, in_cap, in_res = False, True, False
                continue
            if s.startswith("*RES"):
                in_conn, in_cap, in_res = False, False, True
                continue

            if in_conn and s.startswith("*I"):
                # `*I U1:Y I` — pin connection.
                parts = s.split()
                if len(parts) >= 2:
                    connections.append(resolve(parts[1]))
            elif in_cap:
                # Two forms:
                #   `1 *3:GROUND 1.5`           — lumped to ground
                #   `1 *3:GROUND *4:GROUND 0.5` — coupling cap
                parts = s.split()
                if len(parts) == 3:
                    pass  # ignore lumped-to-ground (folded into total_cap)
                elif len(parts) == 4:
                    other_pin = resolve(parts[2])
                    other_net = other_pin.split(":", 1)[0]
                    coupling.append((other_net, float(parts[3])))
            elif in_res and s and not s.startswith("*"):
                parts = s.split()
                if len(parts) == 4:
                    try:
                        resistance += float(parts[3])
                    except ValueError:
                        pass

        # Canonicalise: sort connections, coupling.
        connections.sort()
        coupling.sort()
        nets.append({
            "name": net_name,
            "total_cap": total_cap,
            "connections": connections,
            "coupling": [list(c) for c in coupling],
            "resistance": resistance,
        })

    nets.sort(key=lambda n: n["name"])
    return {"design": design, "nets": nets}


def main():
    if len(sys.argv) != 3:
        print("usage: spef_dump.py <input.spef> <output.json>", file=sys.stderr)
        sys.exit(2)
    with open(sys.argv[1]) as f:
        text = f.read()
    out = parse_spef(text)
    with open(sys.argv[2], "w") as f:
        json.dump(out, f, indent=2, sort_keys=True)


if __name__ == "__main__":
    main()
