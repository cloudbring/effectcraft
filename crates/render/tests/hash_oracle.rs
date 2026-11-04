//! #188: OLD (inline FNV folds) vs NEW (`craft_hash::{Fnv64, ContentHash, fnv1a64}`) in effectcraft.
//!
//! OLD is vendored in `tests/old/` (effectcraft @ 7870b5916ae0749d497d880989683bf2946d6c8b).
//! `effectcraft_render::disk_cache::Hash128` is public, so the real type is compared. The cache
//! `KeyHasher`, `checksum` and the effects/engine `fnv` helpers are private; each `new_*` below
//! restates the call exactly as the tree has it (with its location) and is compared to OLD.

mod old;

use effectcraft_render::disk_cache::Hash128;
use proptest::prelude::*;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

// ---- NEW, as the tree has it ------------------------------------------------------------------

/// render/src/cache.rs
struct NewKeyHasher(craft_hash::Fnv64);

impl Hasher for NewKeyHasher {
    fn finish(&self) -> u64 {
        self.0.finish()
    }
    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
}

/// effects/src/{roto,puppet,object_matte,warp_stab}.rs, engine/src/analysis/*
fn new_fnv_seeded(state: &mut u64, bytes: &[u8]) {
    let mut hash = craft_hash::Fnv64::from_state(*state);
    hash.update(bytes);
    *state = hash.finish();
}

fn corpus_sample(max: usize) -> Vec<(PathBuf, Vec<u8>)> {
    let root = PathBuf::from(std::env::var("ARTCRAFT_CORPUS").unwrap_or_else(|_| "/Users/emm/ArtCraft/corpus".into()));
    let mut files = vec![];
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if e.metadata().is_ok_and(|m| m.len() > 0 && m.len() <= 6 << 20) {
                files.push(p);
            }
        }
    }
    files.sort();
    if files.is_empty() {
        eprintln!("SKIP corpus: no files under {}", root.display());
        return vec![];
    }
    let step = (files.len() / max).max(1);
    files.into_iter().step_by(step).take(max).filter_map(|p| std::fs::read(&p).ok().map(|b| (p, b))).collect()
}

fn check_all(bytes: &[u8], seed: u64) {
    // Public type.
    let (mut o, mut n) = (old::Hash128::default(), Hash128::default());
    o.write(bytes);
    n.write(bytes);
    assert_eq!(n.finish(), o.finish(), "Hash128::write");
    o.write_u64(seed);
    n.write_u64(seed);
    assert_eq!(n.finish(), o.finish(), "Hash128::write_u64");
    // Restated private sites.
    assert_eq!(craft_hash::fnv1a64(bytes), old::checksum(bytes), "disk_cache checksum");
    assert_eq!(craft_hash::fnv1a64(bytes), old::fnv(bytes), "analysis fnv");
    let (mut a, mut b) = (seed, seed);
    old::fnv_seeded(&mut a, bytes);
    new_fnv_seeded(&mut b, bytes);
    assert_eq!(a, b, "seeded fnv");
    let (mut ko, mut kn) = (old::KeyHasher(seed), NewKeyHasher(craft_hash::Fnv64::from_state(seed)));
    ko.write(bytes);
    kn.write(bytes);
    assert_eq!(kn.finish(), ko.finish(), "KeyHasher");
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 500, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn random_bytes_digest_identically(bytes in prop::collection::vec(any::<u8>(), 0..70_000), seed in prop_oneof![Just(0u64), Just(0xcbf2_9ce4_8422_2325), Just(u64::MAX), any::<u64>()]) {
        check_all(&bytes, seed);
    }

    /// `KeyHasher` is driven by `Hash` impls: integers, strings, tuples and slices call `write`
    /// many times with different widths. Both hashers must end in the same state.
    #[test]
    fn key_hasher_over_hash_impls(seed in any::<u64>(), n in any::<u64>(), s in "[ -~é日]{0,40}", v in prop::collection::vec(any::<(u8, i32, bool)>(), 0..30), f in any::<f64>()) {
        let (mut ko, mut kn) = (old::KeyHasher(seed), NewKeyHasher(craft_hash::Fnv64::from_state(seed)));
        n.hash(&mut ko);
        n.hash(&mut kn);
        s.hash(&mut ko);
        s.hash(&mut kn);
        v.hash(&mut ko);
        v.hash(&mut kn);
        f.to_bits().hash(&mut ko);
        f.to_bits().hash(&mut kn);
        ko.write_u8(0xff);
        kn.write_u8(0xff);
        prop_assert_eq!(kn.finish(), ko.finish());
    }

    /// Hash128 fed in pieces equals one write.
    #[test]
    fn hash128_chunking(bytes in prop::collection::vec(any::<u8>(), 0..3000), cut in 1usize..200) {
        let (mut whole, mut parts, mut o) = (Hash128::default(), Hash128::default(), old::Hash128::default());
        whole.write(&bytes);
        for p in bytes.chunks(cut) {
            parts.write(p);
        }
        o.write(&bytes);
        prop_assert_eq!(whole.finish(), parts.finish());
        prop_assert_eq!(whole.finish(), o.finish());
    }
}

#[test]
fn corpus_files_digest_identically() {
    let files = corpus_sample(80);
    let mut total = 0u64;
    for (i, (_path, bytes)) in files.iter().enumerate() {
        check_all(bytes, 0x9e37_79b9_7f4a_7c15 ^ i as u64);
        total += bytes.len() as u64;
    }
    eprintln!("corpus: {} files, {} bytes digested identically", files.len(), total);
}
