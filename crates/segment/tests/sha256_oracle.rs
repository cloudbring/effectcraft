//! #188: OLD (hand-written SHA-256) vs NEW (`craft_hash::sha256`) in `effectcraft_segment::sha256`.
//!
//! OLD is the whole `pub mod sha256` of effectcraft @ 7870b5916ae0749d497d880989683bf2946d6c8b,
//! vendored in `tests/old/` (a from-scratch FIPS 180-4 implementation, independent of the `sha2`
//! crate that `craft-hash` wraps), so this is also an independent-implementation check.

mod old;

use effectcraft_segment::sha256;
use proptest::prelude::*;
use std::path::PathBuf;

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

proptest! {
    #![proptest_config(ProptestConfig { cases: 500, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn random_bytes(bytes in prop::collection::vec(any::<u8>(), 0..70_000)) {
        prop_assert_eq!(sha256::digest(&bytes), old::sha256::digest(&bytes));
        prop_assert_eq!(sha256::hex(&bytes), old::sha256::hex(&bytes));
    }

    /// Padding edges: lengths around the 55/56/63/64-byte block and length-field boundaries.
    #[test]
    fn padding_boundaries(len in prop_oneof![0usize..140, 183usize..200], fill in any::<u8>()) {
        let bytes: Vec<u8> = (0..len).map(|i| fill.wrapping_add(i as u8).wrapping_mul(31)).collect();
        prop_assert_eq!(sha256::digest(&bytes), old::sha256::digest(&bytes));
    }
}

#[test]
fn corpus_files() {
    let files = corpus_sample(80);
    let mut total = 0u64;
    for (path, bytes) in &files {
        assert_eq!(sha256::hex(bytes), old::sha256::hex(bytes), "{}", path.display());
        total += bytes.len() as u64;
    }
    eprintln!("corpus: {} files, {} bytes digested identically", files.len(), total);
}
