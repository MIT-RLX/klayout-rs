#![no_main]
//! GDSII reader fuzz target.
//!
//! Property: the reader must terminate without panicking on **any**
//! input byte sequence. Malformed GDSII (truncated records, bogus
//! record IDs, oversized lengths, recursive structure references)
//! should return a typed `IoError`, not abort. Crashes here represent
//! security risks for any flow that ingests untrusted GDSII (e.g.
//! foundry handoff, customer netlists).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = klayout_io::read_gds_bytes(data);
});
