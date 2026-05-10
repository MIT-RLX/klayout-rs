#![no_main]
//! DEF reader fuzz target. Needs a non-empty Library since
//! `read_def` resolves macro names against an existing library; a
//! one-cell scratch library is enough to exercise the parser.

use klayout_core::{CellBuilder, Library};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let lib = Library::new("fuzz", 1000);
    // Provide a single macro the DEF can attempt to resolve. The
    // parser should still tolerate unresolved macro references and
    // arbitrary section ordering.
    let _ = lib.insert(CellBuilder::new("any"));
    let _ = klayout_lef::read_def(data, &lib);
});
