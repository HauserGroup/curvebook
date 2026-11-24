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
        let cd_start = cd_offset as usize;
        let cd_end = cd_start.checked_add(cd_size as usize).ok_or_else(|| String::from("ZIP central directory offset overflow"))?;
        if cd_end > eocd {
            return Err(String::from("ZIP central directory extends past end-of-central-directory"));
        }

        let mut cursor = cd_start;
        let mut total_uncompressed: usize = 0;
        let mut seen = BTreeSet::new();
        let mut entries = Vec::with_capacity(total_entries.min(256));

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
            let lfh_offset = read_u32_le(bytes, cursor + 42)? as usize;

            if (flags & 0x0001) != 0 {
                return Err(String::from("encrypted ZIP entries are unsupported"));
            }
            if (version_made_by >> 8) == 3 && ((ext_attr >> 16) & 0o17_0000) == 0o12_0000 {
                return Err(String::from("ZIP archive contains symlink entry"));
            }

            let name_start = cursor + CD_FIXED_LEN;
            let name_end = name_start.checked_add(name_len).ok_or_else(|| String::from("ZIP entry name overflow"))?;
            let next_cursor =
                name_end.checked_add(extra_len).and_then(|v| v.checked_add(comment_len)).ok_or_else(|| String::from("ZIP record length overflow"))?;
            if next_cursor > cd_end {
                return Err(String::from("ZIP central directory entry exceeds bounds"));
            }
            let raw_name = bytes.get(name_start..name_end).ok_or_else(|| String::from("ZIP entry name out of bounds"))?;
            let raw_name = std::str::from_utf8(raw_name).map_err(|_| String::from("ZIP entry path is not valid UTF-8"))?;
            cursor = next_cursor;

            let is_dir = raw_name.ends_with('/');
            let trimmed = if is_dir { raw_name.trim_end_matches('/') } else { raw_name };
            let normalized = sanitize_zip_path(trimmed)?;
            if is_dir {
                if uncomp_size != 0 {
                    return Err(format!("ZIP directory entry `{raw_name}` has non-zero payload"));
                }
                continue;
            }
            if !seen.insert(normalized.clone()) {
                return Err(format!("duplicate ZIP entry `{normalized}`"));
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
        Ok(Self { entries })
    }

    /// Iterates over decoded `(relative_path, payload_bytes)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.entries.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
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
}
