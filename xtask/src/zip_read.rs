//! Minimal bounded pure-Rust ZIP reader for `xtask` asset, cleanroom, brand, and release checks.

use std::collections::BTreeSet;
use std::io::Read;

use flate2::read::DeflateDecoder;

const EOCD_SIG: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
const CD_SIG: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
const LFH_SIG: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
const EOCD_MIN_LEN: usize = 22;
const EOCD_MAX_COMMENT: usize = 65_535;
const CD_FIXED_LEN: usize = 46;
const LFH_FIXED_LEN: usize = 30;
const MAX_ZIP_PATH_LEN: usize = 512;

/// Resource limits enforced when unpacking a ZIP archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipLimits {
    /// Maximum number of non-directory entries allowed in an archive.
    pub max_entries: usize,
    /// Maximum aggregate uncompressed byte count across all entries.
    pub max_total_uncompressed_bytes: usize,
    /// Maximum uncompressed byte count for any single entry.
    pub max_entry_uncompressed_bytes: usize,
}

impl Default for ZipLimits {
    fn default() -> Self {
        Self { max_entries: 4_096, max_total_uncompressed_bytes: 128 * 1024 * 1024, max_entry_uncompressed_bytes: 64 * 1024 * 1024 }
    }
}

/// Decoded in-memory ZIP archive validated against [`ZipLimits`].
#[derive(Debug, Clone)]
pub struct ZipArchive {
    entries: Vec<(String, Vec<u8>)>,
    archive_comment: Vec<u8>,
    entry_comments: Vec<(String, Vec<u8>)>,
}

/// Returns `true` when `bytes` contains a valid trailing ZIP End-Of-Central-Directory record
/// (including PKWARE-valid preamble-prefixed ZIP archives).
pub(crate) fn has_zip_eocd(bytes: &[u8]) -> bool {
    find_eocd(bytes).is_ok()
}

impl ZipArchive {
    /// Parses and decompresses every file entry from `bytes`, enforcing `limits`, CRC-32, and safe paths.
    pub fn from_bytes_with_limits(bytes: &[u8], limits: ZipLimits) -> Result<Self, String> {
        let eocd = find_eocd(bytes)?;
        let disk = read_u16_le(bytes, eocd + 4)?;
        let cd_disk = read_u16_le(bytes, eocd + 6)?;
        let disk_entries = read_u16_le(bytes, eocd + 8)?;
        let total_entries = read_u16_le(bytes, eocd + 10)?;
        let cd_size = read_u32_le(bytes, eocd + 12)?;
        let cd_offset = read_u32_le(bytes, eocd + 16)?;
        let eocd_comment_len = usize::from(read_u16_le(bytes, eocd + 20)?);
        let archive_comment = bytes.get(eocd + EOCD_MIN_LEN..eocd + EOCD_MIN_LEN + eocd_comment_len).unwrap_or_default().to_vec();
        if disk != 0 || cd_disk != 0 || disk_entries != total_entries {
            return Err(String::from("multi-disk ZIP archives are unsupported"));
        }
        if total_entries == u16::MAX || cd_size == u32::MAX || cd_offset == u32::MAX {
            return Err(String::from("ZIP64 archives are unsupported"));
        }
        let total_entries = usize::from(total_entries);
        if total_entries > limits.max_entries {
            return Err(format!("ZIP entry count {total_entries} exceeds limit {}", limits.max_entries));
        }
        let cd_start_raw = cd_offset as usize;
        let cd_size_usize = cd_size as usize;
        let cd_end_raw = cd_start_raw.checked_add(cd_size_usize).ok_or_else(|| String::from("ZIP central directory offset overflow"))?;
        if cd_end_raw > eocd {
            return Err(String::from("ZIP central directory extends past end-of-central-directory"));
        }
        // Standard Info-ZIP / Python `zipfile` preamble-offset adjustment for self-extracting or
        // `cat image.png archive.zip` polyglots whose EOCD/CD offsets were not rewritten via `zip -A`.
        // Fail closed when BOTH the unadjusted and adjusted offsets start with a central-directory
        // signature (dual-CD decoy polyglot).
        let (cd_start, cd_end, preamble_delta) = if total_entries > 0 && cd_end_raw < eocd {
            let delta = eocd - cd_end_raw;
            let adj_start = cd_start_raw.checked_add(delta).ok_or_else(|| String::from("ZIP central directory offset overflow"))?;
            let adj_end = adj_start.checked_add(cd_size_usize).ok_or_else(|| String::from("ZIP central directory offset overflow"))?;
            let raw_has_cd = cd_start_raw.checked_add(4).and_then(|e| bytes.get(cd_start_raw..e)) == Some(CD_SIG.as_slice());
            let adj_has_cd = adj_start.checked_add(4).and_then(|e| bytes.get(adj_start..e)) == Some(CD_SIG.as_slice());
            if raw_has_cd && adj_has_cd {
                return Err(String::from("ambiguous dual ZIP central directory (polyglot decoy)"));
            }
            if !raw_has_cd && adj_has_cd { (adj_start, adj_end, delta) } else { (cd_start_raw, cd_end_raw, 0usize) }
        } else {
            (cd_start_raw, cd_end_raw, 0usize)
        };

        let mut cursor = cd_start;
        let mut total_uncompressed: usize = 0;
        let mut seen = BTreeSet::new();
        let mut entries = Vec::with_capacity(total_entries.min(256));
        let mut entry_comments = Vec::new();

        for _ in 0..total_entries {
            if cursor.checked_add(CD_FIXED_LEN).is_none_or(|end| end > cd_end) {
                return Err(String::from("truncated ZIP central directory header"));
            }
            if bytes.get(cursor..cursor + 4) != Some(CD_SIG.as_slice()) {
                return Err(String::from("invalid ZIP central directory signature"));
            }
            let version_made_by = read_u16_le(bytes, cursor + 4)?;
            let flags = read_u16_le(bytes, cursor + 8)?;
            let method = read_u16_le(bytes, cursor + 10)?;
            let expected_crc = read_u32_le(bytes, cursor + 16)?;
            let comp_size = read_u32_le(bytes, cursor + 20)? as usize;
            let uncomp_size = read_u32_le(bytes, cursor + 24)? as usize;
            let name_len = usize::from(read_u16_le(bytes, cursor + 28)?);
            let extra_len = usize::from(read_u16_le(bytes, cursor + 30)?);
            let comment_len = usize::from(read_u16_le(bytes, cursor + 32)?);
            let ext_attr = read_u32_le(bytes, cursor + 38)?;
            let lfh_offset = (read_u32_le(bytes, cursor + 42)? as usize)
                .checked_add(preamble_delta)
                .ok_or_else(|| String::from("ZIP local file header offset overflow"))?;

            if (flags & 0x0001) != 0 {
                return Err(String::from("encrypted ZIP entries are unsupported"));
            }
            // Reject symlinks created under Unix (host OS 3) or macOS/Darwin (host OS 19).
            if matches!(version_made_by >> 8, 3 | 19) && ((ext_attr >> 16) & 0o17_0000) == 0o12_0000 {
                return Err(String::from("ZIP archive contains symlink entry"));
            }

            let name_start = cursor + CD_FIXED_LEN;
            let name_end = name_start.checked_add(name_len).ok_or_else(|| String::from("ZIP entry name overflow"))?;
            let comment_start = name_end.checked_add(extra_len).ok_or_else(|| String::from("ZIP extra length overflow"))?;
            let next_cursor = comment_start.checked_add(comment_len).ok_or_else(|| String::from("ZIP record length overflow"))?;
            if next_cursor > cd_end {
                return Err(String::from("ZIP central directory entry exceeds bounds"));
            }
            let raw_name = bytes.get(name_start..name_end).ok_or_else(|| String::from("ZIP entry name out of bounds"))?;
            let raw_name = std::str::from_utf8(raw_name).map_err(|_| String::from("ZIP entry path is not valid UTF-8"))?;
            let comment_bytes = bytes.get(comment_start..next_cursor).unwrap_or_default();
            cursor = next_cursor;

            let is_dir = raw_name.ends_with('/');
            let trimmed = if is_dir { raw_name.trim_end_matches('/') } else { raw_name };
            let normalized = sanitize_zip_path(trimmed)?;
            if !comment_bytes.is_empty() {
                entry_comments.push((normalized.clone(), comment_bytes.to_vec()));
            }
            if is_dir {
                if uncomp_size != 0 {
                    return Err(format!("ZIP directory entry `{raw_name}` has non-zero payload"));
                }
                continue;
            }
            let folded = normalized.to_ascii_lowercase();
            if !seen.insert(folded) {
                return Err(format!("duplicate or case-colliding ZIP entry `{normalized}`"));
            }
            if uncomp_size > limits.max_entry_uncompressed_bytes {
                return Err(format!("ZIP entry `{normalized}` exceeds single-entry uncompressed limit"));
            }
            total_uncompressed =
                total_uncompressed.checked_add(uncomp_size).ok_or_else(|| String::from("ZIP aggregate uncompressed size overflow"))?;
            if total_uncompressed > limits.max_total_uncompressed_bytes {
                return Err(format!("ZIP aggregate uncompressed size exceeds limit {}", limits.max_total_uncompressed_bytes));
            }

            let payload =
                read_entry_payload(bytes, &CdEntry { name: &normalized, lfh_offset, method, comp_size, uncomp_size, expected_crc }, limits)?;
            entries.push((normalized, payload));
        }
        if cursor != cd_end {
            return Err(String::from("ZIP central directory size mismatch"));
        }
        Ok(Self { entries, archive_comment, entry_comments })
    }

    /// Iterates over decoded `(relative_path, payload_bytes)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.entries.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
    }

    /// Raw EOCD archive comment bytes (may be empty).
    pub fn archive_comment(&self) -> &[u8] {
        &self.archive_comment
    }

    /// Iterates over non-empty per-entry central-directory comments `(relative_path, comment_bytes)`.
    pub fn entry_comments(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.entry_comments.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
    }

    /// Returns `true` when the archive contains zero file entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn find_eocd(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() < EOCD_MIN_LEN {
        return Err(String::from("input too small to be a ZIP archive"));
    }
    let min_pos = bytes.len().saturating_sub(EOCD_MIN_LEN + EOCD_MAX_COMMENT);
    let max_pos = bytes.len() - EOCD_MIN_LEN;
    for pos in (min_pos..=max_pos).rev() {
        if bytes.get(pos..pos + 4) == Some(EOCD_SIG.as_slice()) {
            let comment_len = usize::from(read_u16_le(bytes, pos + 20)?);
            if pos + EOCD_MIN_LEN + comment_len == bytes.len() {
                return Ok(pos);
            }
        }
    }
    Err(String::from("missing ZIP end-of-central-directory record"))
}

struct CdEntry<'a> {
    name: &'a str,
    lfh_offset: usize,
    method: u16,
    comp_size: usize,
    uncomp_size: usize,
    expected_crc: u32,
}

fn read_entry_payload(bytes: &[u8], entry: &CdEntry<'_>, limits: ZipLimits) -> Result<Vec<u8>, String> {
    let CdEntry { name, lfh_offset, method, comp_size, uncomp_size, expected_crc } = *entry;
    if lfh_offset.checked_add(LFH_FIXED_LEN).is_none_or(|end| end > bytes.len()) {
        return Err(format!("ZIP entry `{name}` local header out of bounds"));
    }
    if bytes.get(lfh_offset..lfh_offset + 4) != Some(LFH_SIG.as_slice()) {
        return Err(format!("ZIP entry `{name}` has invalid local file header signature"));
    }
    let lfh_name_len = usize::from(read_u16_le(bytes, lfh_offset + 26)?);
    let lfh_extra_len = usize::from(read_u16_le(bytes, lfh_offset + 28)?);
    let data_start = lfh_offset
        .checked_add(LFH_FIXED_LEN)
        .and_then(|v| v.checked_add(lfh_name_len))
        .and_then(|v| v.checked_add(lfh_extra_len))
        .ok_or_else(|| format!("ZIP entry `{name}` data offset overflow"))?;
    let data_end = data_start.checked_add(comp_size).ok_or_else(|| format!("ZIP entry `{name}` payload length overflow"))?;
    let compressed = bytes.get(data_start..data_end).ok_or_else(|| format!("ZIP entry `{name}` payload truncated"))?;

    let out = match method {
        0 => {
            if comp_size != uncomp_size {
                return Err(format!("ZIP stored entry `{name}` compressed/uncompressed length mismatch"));
            }
            compressed.to_vec()
        }
        8 => {
            let cap = uncomp_size.min(limits.max_entry_uncompressed_bytes);
            let mut decoder = DeflateDecoder::new(compressed).take(cap.saturating_add(1) as u64);
            let mut buf = Vec::with_capacity(cap.min(1024 * 1024));
            decoder.read_to_end(&mut buf).map_err(|e| format!("ZIP entry `{name}` deflate error: {e}"))?;
            if buf.len() != uncomp_size {
                return Err(format!("ZIP entry `{name}` uncompressed size mismatch"));
            }
            buf
        }
        other => return Err(format!("ZIP entry `{name}` uses unsupported compression method {other}")),
    };
    if crc32fast::hash(&out) != expected_crc {
        return Err(format!("ZIP entry `{name}` CRC-32 mismatch"));
    }
    Ok(out)
}

fn sanitize_zip_path(raw: &str) -> Result<String, String> {
    if raw.is_empty() || raw.len() > MAX_ZIP_PATH_LEN || raw.contains('\0') || raw.contains('\\') || raw.starts_with('/') {
        return Err(format!("unsafe ZIP entry path `{raw}`"));
    }
    if raw.as_bytes().get(1) == Some(&b':') {
        return Err(format!("unsafe drive-prefixed ZIP entry path `{raw}`"));
    }
    for segment in raw.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(format!("unsafe traversal segment in ZIP entry path `{raw}`"));
        }
    }
    Ok(raw.to_owned())
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let end = offset.checked_add(2).ok_or_else(|| String::from("ZIP integer offset overflow"))?;
    let slice = bytes.get(offset..end).ok_or_else(|| String::from("unexpected EOF reading ZIP u16"))?;
    let arr: [u8; 2] = slice.try_into().map_err(|_| String::from("invalid ZIP u16 slice"))?;
    Ok(u16::from_le_bytes(arr))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let end = offset.checked_add(4).ok_or_else(|| String::from("ZIP integer offset overflow"))?;
    let slice = bytes.get(offset..end).ok_or_else(|| String::from("unexpected EOF reading ZIP u32"))?;
    let arr: [u8; 4] = slice.try_into().map_err(|_| String::from("invalid ZIP u32 slice"))?;
    Ok(u32::from_le_bytes(arr))
}

#[cfg(test)]
pub(crate) struct TestZipWriter {
    entries: Vec<(String, Vec<u8>)>,
}

#[cfg(test)]
impl TestZipWriter {
    pub(crate) fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub(crate) fn add_file(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.entries.push((name.to_owned(), bytes));
        Ok(())
    }

    pub(crate) fn finish(self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, data) in &self.entries {
            let offset = u32::try_from(out.len()).map_err(|_| String::from("test zip overflow"))?;
            let size = u32::try_from(data.len()).map_err(|_| String::from("test zip entry overflow"))?;
            let name_len = u16::try_from(name.len()).map_err(|_| String::from("test zip name overflow"))?;
            let crc = crc32fast::hash(data);
            out.extend_from_slice(&LFH_SIG);
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&name_len.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);

            cd.extend_from_slice(&CD_SIG);
            cd.extend_from_slice(&20u16.to_le_bytes());
            cd.extend_from_slice(&20u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&crc.to_le_bytes());
            cd.extend_from_slice(&size.to_le_bytes());
            cd.extend_from_slice(&size.to_le_bytes());
            cd.extend_from_slice(&name_len.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u32.to_le_bytes());
            cd.extend_from_slice(&offset.to_le_bytes());
            cd.extend_from_slice(name.as_bytes());
        }
        let cd_offset = u32::try_from(out.len()).map_err(|_| String::from("test zip cd offset overflow"))?;
        let cd_size = u32::try_from(cd.len()).map_err(|_| String::from("test zip cd size overflow"))?;
        let count = u16::try_from(self.entries.len()).map_err(|_| String::from("test zip entry count overflow"))?;
        out.extend_from_slice(&cd);
        out.extend_from_slice(&EOCD_SIG);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_limits() {
        let mut zw = TestZipWriter::new();
        zw.add_file("dir/hello.txt", b"hello world".to_vec()).expect("add file");
        let bytes = zw.finish().expect("build test zip");
        let arc = ZipArchive::from_bytes_with_limits(&bytes, ZipLimits::default()).expect("parse zip");
        let items: Vec<_> = arc.iter().collect();
        assert_eq!(items, vec![("dir/hello.txt", b"hello world".as_slice())]);
        let tiny = ZipLimits { max_entries: 1, max_total_uncompressed_bytes: 5, max_entry_uncompressed_bytes: 5 };
        assert!(ZipArchive::from_bytes_with_limits(&bytes, tiny).is_err());
    }

    #[test]
    fn rejects_traversal_and_bad_crc() {
        let mut zw = TestZipWriter::new();
        zw.add_file("../escape.txt", b"bad".to_vec()).expect("add file");
        let bad_path = zw.finish().expect("build");
        assert!(ZipArchive::from_bytes_with_limits(&bad_path, ZipLimits::default()).is_err());

        let mut zw2 = TestZipWriter::new();
        zw2.add_file("ok.txt", b"abc".to_vec()).expect("add file");
        let mut corrupted = zw2.finish().expect("build");
        let idx = corrupted.windows(3).position(|w| w == b"abc").expect("payload");
        corrupted[idx] ^= 0xff;
        assert!(ZipArchive::from_bytes_with_limits(&corrupted, ZipLimits::default()).is_err());
    }

    #[test]
    fn aud_12_17_rejects_case_colliding_entries() {
        let mut zw = TestZipWriter::new();
        zw.add_file("doc/README.txt", b"dirty".to_vec()).unwrap();
        zw.add_file("doc/readme.txt", b"clean".to_vec()).unwrap();
        let bytes = zw.finish().unwrap();
        let err = ZipArchive::from_bytes_with_limits(&bytes, ZipLimits::default()).unwrap_err();
        assert!(err.contains("duplicate or case-colliding"), "{err}");
    }

    #[test]
    fn aud_12_45_rejects_macos_host_os_19_symlink_and_exposes_comments() {
        // Build a ZIP with host OS 19 (macOS/Darwin) + Unix symlink mode 0o120000 in ext_attr.
        let mut zw = TestZipWriter::new();
        zw.add_file("link.txt", b"/etc/passwd".to_vec()).unwrap();
        let mut bytes = zw.finish().unwrap();
        let cd_pos = bytes.windows(4).position(|w| w == CD_SIG).unwrap();
        bytes[cd_pos + 4..cd_pos + 6].copy_from_slice(&((19u16 << 8) | 20u16).to_le_bytes());
        bytes[cd_pos + 38..cd_pos + 42].copy_from_slice(&((0o12_0777u32) << 16).to_le_bytes());
        let err = ZipArchive::from_bytes_with_limits(&bytes, ZipLimits::default()).unwrap_err();
        assert!(err.contains("symlink"), "{err}");

        // Build a ZIP with both an entry comment and an EOCD archive comment.
        let mut zw2 = TestZipWriter::new();
        zw2.add_file("note.txt", b"ok".to_vec()).unwrap();
        let base = zw2.finish().unwrap();
        let eocd_pos = base.windows(4).rposition(|w| w == EOCD_SIG).unwrap();
        let entry_comment = b"entry-note";
        let cd_pos2 = base.windows(4).position(|w| w == CD_SIG).unwrap();
        let mut with_comments = base[..eocd_pos].to_vec();
        with_comments.extend_from_slice(entry_comment);
        // Patch entry comment length and CD size in EOCD.
        with_comments[cd_pos2 + 32..cd_pos2 + 34].copy_from_slice(&(entry_comment.len() as u16).to_le_bytes());
        let new_cd_size = (with_comments.len() - cd_pos2) as u32;
        let mut eocd = base[eocd_pos..].to_vec();
        eocd[12..16].copy_from_slice(&new_cd_size.to_le_bytes());
        let arc_comment = b"archive-note";
        eocd[20..22].copy_from_slice(&(arc_comment.len() as u16).to_le_bytes());
        with_comments.extend_from_slice(&eocd);
        with_comments.extend_from_slice(arc_comment);

        let arc = ZipArchive::from_bytes_with_limits(&with_comments, ZipLimits::default()).unwrap();
        assert_eq!(arc.archive_comment(), arc_comment);
        let comments: Vec<_> = arc.entry_comments().collect();
        assert_eq!(comments, vec![("note.txt", entry_comment.as_slice())]);
    }

    #[test]
    fn round3_216_rejects_dual_central_directory_polyglot_decoy() {
        // Construct a dual-CD decoy: clean ZIP followed by appended raw ZIP whose EOCD points to
        // `cd_start_raw` (which starts with CD_SIG inside the first ZIP) while `cd_start_raw + delta`
        // also starts with CD_SIG in the appended ZIP.
        let mut zw1 = TestZipWriter::new();
        zw1.add_file("decoy.txt", b"clean".to_vec()).unwrap();
        let z1 = zw1.finish().unwrap();
        let mut zw2 = TestZipWriter::new();
        zw2.add_file("hidden.txt", b"payload".to_vec()).unwrap();
        let z2 = zw2.finish().unwrap();
        let cd2_off = z2.windows(4).position(|w| w == CD_SIG).unwrap();
        let mut preamble = vec![0x89, b'P', b'N', b'G'];
        // Pad preamble so `CD_SIG` sits at byte offset `cd2_off` (matching z2's raw EOCD cd_offset).
        if preamble.len() < cd2_off {
            preamble.resize(cd2_off, 0);
        }
        preamble.extend_from_slice(&CD_SIG);
        preamble.extend_from_slice(&z1);
        let mut dual = preamble;
        dual.extend_from_slice(&z2);
        let err = ZipArchive::from_bytes_with_limits(&dual, ZipLimits::default()).unwrap_err();
        assert!(err.contains("ambiguous dual ZIP central directory"), "{err}");
    }
}
