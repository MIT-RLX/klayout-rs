# OpenROAD smoke: ingest SkyWater sky130_hd tech + single standard cell.
# Expects flat files mounted at /work:
#   - sky130_fd_sc_hd.tlef   (technology LEF from skywater-pdk-libs-sky130_fd_sc_hd)
#   - sky130_cell.lef       (macro LEF — e.g. sky130_fd_sc_hd__inv_1.lef)
#
# Printed line `PDK_SMOKE masters_loaded=<n>` is parsed by run_benchmark.sh.

read_lef /work/sky130_fd_sc_hd.tlef
read_lef /work/sky130_cell.lef

set db [ord::get_db]
set masters 0
foreach lib [$db getLibs] {
  incr masters [llength [$lib getMasters]]
}
puts "PDK_SMOKE masters_loaded=$masters"
exit
