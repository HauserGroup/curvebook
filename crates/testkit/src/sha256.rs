//! Pure-Rust FIPS PUB 180-4 SHA-256 digest and verified corpus file loader (`docs/development.md`
//! §Verified NIST StRD corpus).

use std::fmt;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

/// Pinned corpus file manifest entry with exact byte count and lowercase hex SHA-256 digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorpusManifestEntry<'a> {
    pub relative_path: &'a str,
    pub expected_bytes: usize,
    pub sha256_hex: &'a str,
}

/// Errors encountered when loading or verifying an on-disk reference corpus file.
#[derive(Debug)]
pub enum CorpusError {
    Io(std::io::Error),
    SizeMismatch { expected: usize, actual: usize },
    DigestMismatch { expected: String, actual: String },
    InvalidUtf8,
}

impl fmt::Display for CorpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "corpus I/O error: {err}"),
            Self::SizeMismatch { expected, actual } => {
                write!(f, "corpus byte size mismatch: expected {expected}, got {actual}")
            }
            Self::DigestMismatch { expected, actual } => {
                write!(f, "corpus SHA-256 mismatch: expected {expected}, got {actual}")
            }
            Self::InvalidUtf8 => write!(f, "corpus file is not valid UTF-8"),
        }
    }
}

impl std::error::Error for CorpusError {}

/// Compute the 32-byte FIPS 180-4 SHA-256 digest of `data`.
#[must_use]
pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    curvebook_platform::sha256::sha256_bytes(data)
}

/// Compute the lowercase 64-character hexadecimal SHA-256 digest of `data`.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    curvebook_platform::sha256::sha256_hex(data)
}

/// Verify `bytes` against an expected byte length and lowercase/uppercase SHA-256 hex string.
///
/// # Errors
/// Returns [`CorpusError::SizeMismatch`] or [`CorpusError::DigestMismatch`] on mismatch.
pub fn verify_blob(bytes: &[u8], expected_size: usize, expected_sha256_hex: &str) -> Result<(), CorpusError> {
    if bytes.len() != expected_size {
        return Err(CorpusError::SizeMismatch { expected: expected_size, actual: bytes.len() });
    }
    let actual_hex = sha256_hex(bytes);
    if !actual_hex.eq_ignore_ascii_case(expected_sha256_hex.trim()) {
        return Err(CorpusError::DigestMismatch { expected: expected_sha256_hex.to_ascii_lowercase(), actual: actual_hex });
    }
    Ok(())
}

fn validate_corpus_relative_path(corpus_root: &Path, relative_path: &str) -> Result<PathBuf, CorpusError> {
    if relative_path.is_empty()
        || relative_path.contains(['\\', '\0', ':'])
        || relative_path.starts_with('/')
        || relative_path.ends_with('/')
        || relative_path.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(CorpusError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid corpus relative path `{relative_path}`"),
        )));
    }
    let mut full_path = corpus_root.to_path_buf();
    let mut component_count = 0_usize;
    for component in Path::new(relative_path).components() {
        match component {
            Component::Normal(part) => {
                component_count += 1;
                full_path.push(part);
                match std::fs::symlink_metadata(&full_path) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        return Err(CorpusError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!("corpus path `{relative_path}` must not traverse or reference a symlink"),
                        )));
                    }
                    Ok(_) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(CorpusError::Io(err)),
                }
            }
            Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(CorpusError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("corpus relative path `{relative_path}` must contain only normal path components"),
                )));
            }
        }
    }
    if component_count == 0 {
        return Err(CorpusError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "corpus relative path must not be empty")));
    }
    Ok(full_path)
}

/// Read a corpus file under `corpus_root`, verifying path safety, regular non-symlink status, exact byte size, and SHA-256 before returning UTF-8 text.
///
/// # Errors
/// Returns [`CorpusError`] if the path is invalid or a symlink, the file cannot be read, fails size/digest verification, or is non-UTF-8.
pub fn load_verified_corpus_file(corpus_root: &Path, entry: CorpusManifestEntry<'_>) -> Result<String, CorpusError> {
    let full_path = validate_corpus_relative_path(corpus_root, entry.relative_path)?;
    let pre_meta = std::fs::symlink_metadata(&full_path).map_err(CorpusError::Io)?;
    if pre_meta.file_type().is_symlink() || !pre_meta.file_type().is_file() {
        return Err(CorpusError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("corpus entry `{}` must be a regular non-symlink file", entry.relative_path),
        )));
    }
    let expected_u64 = u64::try_from(entry.expected_bytes).unwrap_or(u64::MAX);
    if pre_meta.len() != expected_u64 {
        return Err(CorpusError::SizeMismatch { expected: entry.expected_bytes, actual: usize::try_from(pre_meta.len()).unwrap_or(usize::MAX) });
    }

    let file = std::fs::File::open(&full_path).map_err(CorpusError::Io)?;
    let fd_meta = file.metadata().map_err(CorpusError::Io)?;
    if !fd_meta.file_type().is_file() {
        return Err(CorpusError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("corpus entry `{}` is not a regular file", entry.relative_path),
        )));
    }
    if fd_meta.len() != expected_u64 {
        return Err(CorpusError::SizeMismatch { expected: entry.expected_bytes, actual: usize::try_from(fd_meta.len()).unwrap_or(usize::MAX) });
    }

    let mut bytes = Vec::with_capacity(entry.expected_bytes);
    let read_cap = expected_u64.saturating_add(1);
    file.take(read_cap).read_to_end(&mut bytes).map_err(CorpusError::Io)?;
    verify_blob(&bytes, entry.expected_bytes, entry.sha256_hex)?;
    String::from_utf8(bytes).map_err(|_| CorpusError::InvalidUtf8)
}

#[cfg(test)]
mod tests {
    use super::{CorpusError, CorpusManifestEntry, load_verified_corpus_file, sha256_hex, verify_blob};

    #[test]
    fn fips_180_4_test_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let entry = CorpusManifestEntry {
            relative_path: "sample.dat",
            expected_bytes: 3,
            sha256_hex: "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
        };
        assert!(verify_blob(b"abc", entry.expected_bytes, entry.sha256_hex).is_ok());
        assert!(verify_blob(b"abcd", entry.expected_bytes, entry.sha256_hex).is_err());
    }

    #[test]
    fn load_verified_corpus_file_rejects_traversal_symlinks_and_size_mismatch_before_read() {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let root = std::env::temp_dir().join(format!("curvebook-testkit-sha256-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(root.join("univ")).unwrap();
        let sample = root.join("univ/sample.dat");
        std::fs::write(&sample, b"abc").unwrap();

        let digest_abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(
            load_verified_corpus_file(&root, CorpusManifestEntry { relative_path: "univ/sample.dat", expected_bytes: 3, sha256_hex: digest_abc })
                .unwrap(),
            "abc"
        );

        for bad_rel in ["", "../escape.dat", "/abs.dat", "univ\\sample.dat", "C:sample.dat", "univ/./sample.dat"] {
            let err = load_verified_corpus_file(&root, CorpusManifestEntry { relative_path: bad_rel, expected_bytes: 3, sha256_hex: digest_abc })
                .unwrap_err();
            assert!(matches!(err, CorpusError::Io(_)), "expected Io error for `{bad_rel}`, got {err}");
        }

        // Size mismatch must fail cleanly without panicking or allocating huge expected size.
        let size_err = load_verified_corpus_file(
            &root,
            CorpusManifestEntry { relative_path: "univ/sample.dat", expected_bytes: usize::MAX, sha256_hex: digest_abc },
        )
        .unwrap_err();
        assert!(matches!(size_err, CorpusError::SizeMismatch { expected: usize::MAX, actual: 3 }), "unexpected error: {size_err}");

        #[cfg(unix)]
        {
            let link = root.join("univ/link.dat");
            std::os::unix::fs::symlink(&sample, &link).unwrap();
            let link_err =
                load_verified_corpus_file(&root, CorpusManifestEntry { relative_path: "univ/link.dat", expected_bytes: 3, sha256_hex: digest_abc })
                    .unwrap_err();
            assert!(matches!(link_err, CorpusError::Io(_)), "expected symlink rejection, got {link_err}");
        }

        let _ = std::fs::remove_dir_all(&root);
    }
}
