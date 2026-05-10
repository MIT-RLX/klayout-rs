//! Content-addressable hashing for cells and component params.
//!
//! `ContentHash` is a 32-byte blake3 of a cell's canonical encoding. Two cells
//! with byte-identical canonical encodings hash equal across runs and machines.
//! This is what enables structural dedup, cross-library comparison, and
//! incremental verification caching.

use std::fmt;

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    pub const ZERO: Self = Self([0; 32]);

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn short_hex(&self) -> String {
        let mut s = String::with_capacity(16);
        for b in &self.0[..8] {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }

    pub fn full_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in &self.0 {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({})", self.short_hex())
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short_hex())
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParamHash(pub [u8; 32]);

impl ParamHash {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn short_hex(&self) -> String {
        let mut s = String::with_capacity(16);
        for b in &self.0[..8] {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }
}

impl fmt::Debug for ParamHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ParamHash({})", self.short_hex())
    }
}

/// `std::hash::Hasher` adapter that streams into blake3 and produces 32 bytes.
///
/// Used by the default `Component::param_hash` impl so any `T: Hash` whose
/// `Hash` impl is deterministic gets a stable cross-run cache key. Floats
/// must be quantized before reaching this — see [crate::component].
pub struct ParamHasher(blake3::Hasher);

impl Default for ParamHasher {
    fn default() -> Self {
        Self(blake3::Hasher::new())
    }
}

impl ParamHasher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn finalize(self) -> ParamHash {
        ParamHash(*self.0.finalize().as_bytes())
    }
}

impl std::hash::Hasher for ParamHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    fn finish(&self) -> u64 {
        let h = self.0.finalize();
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&h.as_bytes()[..8]);
        u64::from_le_bytes(buf)
    }
}
