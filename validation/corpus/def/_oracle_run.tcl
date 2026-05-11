read_lef /work/tech.lef
read_lef /work/cells.lef
read_def /work/with_routes.def
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
  set sp [$net isSpecial]
  if { $sp } {
    puts "SNET [$net getName] conns=[join $sorted { }]"
  } else {
    puts "NET [$net getName] conns=[join $sorted { }]"
  }
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
          puts "  WIRE sp=[expr {$sp ? 1 : 0}] net=[$net getName] layer=$cur_layer x1=$last_x y1=$last_y x2=$x y2=$y"
        }
        set last_x $x
        set last_y $y
      }
    }
  }
}
exit
