//! Pure-Rust FIPS PUB 180-4 SHA-256 digest and verified corpus file loader (`docs/development.md`
//! §Verified NIST StRD corpus).

use std::fmt;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

const K256: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

const H0: [u32; 8] = [0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19];

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

fn compress_block(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0_u32; 64];
    for (i, slot) in w.iter_mut().take(16).enumerate() {
        let base = i * 4;
        if let Some(chunk) = block.get(base..base + 4) {
            let mut arr = [0_u8; 4];
            arr.copy_from_slice(chunk);
            *slot = u32::from_be_bytes(arr);
        }
    }
    for i in 16..64 {
        let w15 = w[i - 15];
        let w2 = w[i - 2];
        let s0 = w15.rotate_right(7) ^ w15.rotate_right(18) ^ (w15 >> 3);
        let s1 = w2.rotate_right(17) ^ w2.rotate_right(19) ^ (w2 >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K256[i]).wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

/// Compute the 32-byte FIPS 180-4 SHA-256 digest of `data`.
#[must_use]
pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut state = H0;
    let (blocks, rem) = data.as_chunks::<64>();
    for block in blocks {
        compress_block(&mut state, block);
    }
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut tail = [0_u8; 128];
    if let Some(prefix) = tail.get_mut(..rem.len()) {
        prefix.copy_from_slice(rem);
    }
    if let Some(pad_byte) = tail.get_mut(rem.len()) {
        *pad_byte = 0x80;
    }

    let total_tail_len = if rem.len() < 56 { 64 } else { 128 };
    if let Some(len_slot) = tail.get_mut(total_tail_len - 8..total_tail_len) {
        len_slot.copy_from_slice(&bit_len.to_be_bytes());
    }

    let mut first_block = [0_u8; 64];
    first_block.copy_from_slice(&tail[..64]);
    compress_block(&mut state, &first_block);
    if total_tail_len == 128 {
        let mut second_block = [0_u8; 64];
        second_block.copy_from_slice(&tail[64..128]);
        compress_block(&mut state, &second_block);
    }

    let mut out = [0_u8; 32];
    for (idx, word) in state.iter().enumerate() {
        let bytes = word.to_be_bytes();
        if let Some(slot) = out.get_mut(idx * 4..idx * 4 + 4) {
            slot.copy_from_slice(&bytes);
        }
    }
    out
}

/// Compute the lowercase 64-character hexadecimal SHA-256 digest of `data`.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = sha256_bytes(data);
    let mut s = String::with_capacity(64);
    for byte in digest {
        s.push(char::from(HEX[usize::from(byte >> 4)]));
        s.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    s
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
