read_lef /work/tech.lef
read_lef /work/cells.lef
read_def /work/special_stripes.def
set blk [[[ord::get_db] getChip] getBlock]
puts [info commands *SWire*]
puts [info commands *swire*]
