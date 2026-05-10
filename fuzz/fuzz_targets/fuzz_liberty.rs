#![no_main]
//! Liberty parser fuzz target. The Liberty tokenizer is the most
//! complex parser in the workspace (nested groups, attribute lists,
//! quoted-comma-separated lookup-table payloads); this target hits
//! its corner cases hard.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = klayout_liberty::parse_liberty(data);
});
