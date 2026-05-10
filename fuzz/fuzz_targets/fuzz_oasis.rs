#![no_main]
//! OASIS reader fuzz target. Same contract as `fuzz_gds`: any byte
//! sequence in, typed error or `Ok` out, never a panic. OASIS has a
//! larger surface (CBLOCK compression, modal variables, variable-
//! length integers) so this target exercises more parsing depth.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = klayout_io::read_oasis_bytes(data);
});
