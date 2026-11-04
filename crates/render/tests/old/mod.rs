//! OLD hash code, vendored verbatim for the #188 old-vs-new equivalence test.
//!
//! Source: https://github.com/storytold/effectcraft @ 7870b5916ae0749d497d880989683bf2946d6c8b
//! (merge-base of cloudbring/effectcraft `algo/37-craft-hash` with `origin/main`). Each item names its file;
//! bodies are unchanged (only made `pub`). They are private to their crates, hence vendored.

use std::hash::Hasher;

/// crates/render/src/disk_cache.rs: FNV-1a 128-bit (content keys).
#[derive(Clone, Copy)]
pub struct Hash128(u128);

impl Default for Hash128 {
    fn default() -> Self {
        Hash128(0x6c62_272e_07bb_0142_62b8_2175_6295_c58d)
    }
}

impl Hash128 {
    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u128;
            self.0 = self.0.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
        }
    }
    pub fn write_u64(&mut self, v: u64) {
        self.write(&v.to_le_bytes());
    }
    pub fn finish(&self) -> u128 {
        self.0
    }
}

/// crates/render/src/disk_cache.rs
pub fn checksum(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// crates/render/src/cache.rs: FNV-1a style 64-bit hasher fed by `Debug` output and raw values.
pub struct KeyHasher(pub u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// crates/effects/src/{roto,puppet,object_matte,warp_stab}.rs and crates/engine/src/analysis/*: seeded fold.
pub fn fnv_seeded(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= *b as u64;
        *h = h.wrapping_mul(0x0100_0000_01b3);
    }
}

/// crates/effects/src/analysis_cache.rs, engine/src/analysis/{roto,warp}.rs: plain fold.
pub fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}
