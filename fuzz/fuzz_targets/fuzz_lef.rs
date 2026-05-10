#![no_main]
//! LEF reader fuzz target. The LEF tokenizer + recursive-descent
//! parser handles macros, layers, vias, sites, and bracketed
//! NONDEFAULTRULE / VIA / etc. sections; each has its own state
//! machine. Fuzzing exercises the cross-state transitions.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = klayout_lef::read_lef(data);
});
