# klayout-sta

Static timing analysis primitives for the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

Closes the digital sign-off loop: with Liberty (cell timing) +
LEF/DEF (placement) + SPEF (parasitics) + hierarchical netlist
(connectivity) all produced by sister crates, this crate evaluates
per-pin arrival / required times and reports slack on every endpoint.

## Algorithm

1. **Timing graph** — every pin in the design is a node; cell timing
   arcs become intra-cell edges; net interconnect becomes inter-cell
   edges with optional RC delay from SPEF.
2. **Forward propagation** — topo-sort from primary inputs / clock
   pins, accumulate max-delay along each path (arrival times).
3. **Backward propagation** — start from primary outputs / endpoint
   setup constraints, walk backward, take min of `(req − arc_delay)`.
4. **Slack** — `required − arrival` per pin. Negative slack on a
   timing endpoint = violation.

NLDM 2-D bilinear interpolation drives delay; AOCV / POCV derate is
applied per arc; CPPR is computed via clock-tree LCA.

## Capabilities

- Combinational and sequential paths, setup + hold checks.
- Multi-corner sweep (BC / TC / WC).
- CDC analysis on async crossings.
- Slew propagation (optional).

## Limits (v1)

- No CCS, no SDF write.
- No false / multicycle path support.
- Multi-corner orchestration is left to the caller.

## License

Licensed under GPL-3.0-only.
