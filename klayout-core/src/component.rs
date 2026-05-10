//! Components: parameterized cell factories with deterministic param hashing.
//!
//! Two-level dedup, both keyed by hash:
//!   1. `ParamHash` over `Component` args  — skip rebuilding identical params.
//!   2. `ContentHash` over the frozen Cell — dedup cells produced by different
//!      paths that happen to be byte-identical.
//!
//! Floats in params must be quantized (e.g. `Microns(i64)` newtype) before
//! reaching `Hash`. The core does not provide a float-aware hasher on purpose:
//! quantization should be visible at the call site, not magic.

use crate::cell::CellBuilder;
use crate::hash::{ParamHash, ParamHasher};
use crate::library::Library;
use std::hash::Hash;

pub struct BuildCtx<'a> {
    pub lib: &'a Library,
}

impl<'a> BuildCtx<'a> {
    pub fn new(lib: &'a Library) -> Self {
        Self { lib }
    }
}

pub trait Component: Hash + 'static {
    fn build(&self, ctx: &BuildCtx<'_>) -> CellBuilder;

    fn param_hash(&self) -> ParamHash {
        let mut h = ParamHasher::new();
        // Type name as discriminator so two different Component types with
        // accidentally-equal Hash payloads don't collide. `type_name` isn't
        // guaranteed stable across compiler versions, but is stable within a
        // given build — the dedup cache is per-process anyway.
        std::hash::Hasher::write(&mut h, std::any::type_name::<Self>().as_bytes());
        std::hash::Hasher::write_u8(&mut h, 0);
        Hash::hash(self, &mut h);
        h.finalize()
    }

    fn cell_name(&self) -> String {
        format!(
            "{}_{}",
            std::any::type_name::<Self>()
                .rsplit("::")
                .next()
                .unwrap_or("cell"),
            self.param_hash().short_hex()
        )
    }
}
