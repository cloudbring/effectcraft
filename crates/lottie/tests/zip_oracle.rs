//! #188: OLD (inline bitwise CRC-32) vs NEW (`craft_hash::crc32`) `.lottie` ZIP writer.
//!
//! OLD is `zip.rs` (`crc32` and `store`) from effectcraft @ 7870b5916ae0749d497d880989683bf2946d6c8b,
//! vendored in `tests/old/`. Both `store`s are public and byte-for-byte comparable, so the whole
//! archive (local headers, CRCs, central directory) is compared, over random entries and corpus files.

mod old;

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
    #![proptest_config(ProptestConfig { cases: 300, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn store_is_byte_identical(entries in prop::collection::vec(("[a-zA-Z0-9_./é日-]{0,30}", prop::collection::vec(any::<u8>(), 0..20_000)), 0..6)) {
        prop_assert_eq!(effectcraft_lottie::zip::store(&entries), old::zip::store(&entries));
    }
}

#[test]
fn corpus_files_store_identically() {
    let files = corpus_sample(40);
    let mut total = 0u64;
    for pair in files.chunks(3) {
        let entries: Vec<(String, Vec<u8>)> = pair.iter().map(|(p, b)| (p.file_name().unwrap().to_string_lossy().into_owned(), b.clone())).collect();
        assert_eq!(effectcraft_lottie::zip::store(&entries), old::zip::store(&entries));
        total += entries.iter().map(|(_, b)| b.len() as u64).sum::<u64>();
    }
    eprintln!("corpus: {} files, {} bytes archived identically", files.len(), total);
}

/// The empty archive and an empty entry.
#[test]
fn degenerate_archives() {
    for entries in [vec![], vec![("a".to_string(), vec![])], vec![(String::new(), vec![0])]] {
        assert_eq!(effectcraft_lottie::zip::store(&entries), old::zip::store(&entries));
    }
}
