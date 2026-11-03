//! ZIP helpers for `.lottie` (dotLottie) archives.

use craft_archive::{ZipLimits, ZipReader, ZipWriter};

const MAX_ENTRIES: usize = 1_024;
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

fn limits() -> ZipLimits {
    ZipLimits { max_entries: MAX_ENTRIES, max_entry_bytes: MAX_ENTRY_BYTES, max_total_bytes: MAX_TOTAL_BYTES }
}

/// Build a deterministic stored ZIP archive with the given (name, bytes) entries.
pub fn store(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = ZipWriter::in_memory();
    for (name, bytes) in entries {
        if writer.add_stored(name, bytes).is_err() {
            return Vec::new();
        }
    }
    match writer.finish() {
        Ok(output) => output.into_inner(),
        Err(_) => Vec::new(),
    }
}

/// Read bounded, relative-path entries from a dotLottie ZIP archive.
///
/// dotLottie permits DEFLATE compression, so [`ZipReader`] accepts both stored and deflated
/// entries after validating names, declared sizes, decoded sizes, CRCs, and total output.
pub fn read_stored(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let Ok(mut reader) = ZipReader::open(data, limits()) else {
        return Vec::new();
    };
    let names = reader.names().to_vec();
    let mut entries = Vec::new();
    for name in names {
        match reader.read(&name) {
            Ok(Some(bytes)) => entries.push((name, bytes)),
            Ok(None) | Err(_) => return Vec::new(),
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Write};

    use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

    fn zip(entries: &[(&str, &[u8])], method: CompressionMethod) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(method);
        for (name, bytes) in entries {
            writer.start_file(name, options)?;
            writer.write_all(bytes)?;
        }
        Ok(writer.finish()?.into_inner())
    }

    #[test]
    fn store_reopens_with_zip_and_roundtrips() -> Result<(), Box<dyn std::error::Error>> {
        let entries = [("a.json".to_string(), b"{}".to_vec()), ("b/c.txt".to_string(), b"hello".to_vec())];
        let bytes = super::store(&entries);
        let mut archive = ZipArchive::new(Cursor::new(&bytes))?;
        assert_eq!(archive.len(), entries.len());
        for (index, (name, expected)) in entries.iter().enumerate() {
            let mut entry = archive.by_index(index)?;
            assert_eq!(entry.name()?.as_ref(), name);
            let mut actual = Vec::new();
            entry.read_to_end(&mut actual)?;
            assert_eq!(actual, *expected);
        }
        assert_eq!(super::read_stored(&bytes), entries);
        Ok(())
    }

    #[test]
    fn deflated_dotlottie_entries_are_accepted() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = zip(&[("animations/main.json", b"{}")], CompressionMethod::Deflated)?;
        assert_eq!(super::read_stored(&bytes), [("animations/main.json".to_string(), b"{}".to_vec())]);
        Ok(())
    }

    #[test]
    fn hostile_archives_are_rejected_without_panicking() -> Result<(), Box<dyn std::error::Error>> {
        let unsafe_name = zip(&[("../up.txt", b"x")], CompressionMethod::Stored)?;
        assert!(super::read_stored(&unsafe_name).is_empty());

        let mut oversized = super::store(&[("ok.txt".to_string(), b"x".to_vec())]);
        for signature in [b"PK\x03\x04".as_slice(), b"PK\x01\x02".as_slice()] {
            let offset = oversized.windows(signature.len()).position(|window| window == signature).ok_or("missing ZIP header")?;
            oversized[offset + 24..offset + 28].copy_from_slice(&u32::try_from(super::MAX_ENTRY_BYTES + 1)?.to_le_bytes());
        }
        assert!(super::read_stored(&oversized).is_empty());
        Ok(())
    }
}
