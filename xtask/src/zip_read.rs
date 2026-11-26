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
    pub max_entries: usize,
    pub max_total_uncompressed_bytes: usize,
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

/// Returns `true` when `bytes` contains a valid trailing ZIP End-Of-Central-Directory record.
pub(crate) fn has_zip_eocd(bytes: &[u8]) -> bool {
    find_eocd(bytes).is_ok()
}

impl ZipArchive {
    /// Parses and decompresses every file entry from `bytes`, enforcing `limits`, CRC-32, and safe paths.
    pub fn from_bytes_with_limits(bytes: &[u8], limits: ZipLimits) -> Result<Self, String> {
        let eocd = find_eocd(bytes)?;
        let (disk, cd_disk, disk_entries, total_entries) =
            (read_u16_le(bytes, eocd + 4)?, read_u16_le(bytes, eocd + 6)?, read_u16_le(bytes, eocd + 8)?, read_u16_le(bytes, eocd + 10)?);
        let (cd_size, cd_offset, comment_len) =
            (read_u32_le(bytes, eocd + 12)?, read_u32_le(bytes, eocd + 16)?, usize::from(read_u16_le(bytes, eocd + 20)?));
        let archive_comment = bytes.get(eocd + EOCD_MIN_LEN..eocd + EOCD_MIN_LEN + comment_len).unwrap_or_default().to_vec();
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
        let (cd_start_raw, cd_size_usize) = (cd_offset as usize, cd_size as usize);
        let cd_end_raw = cd_start_raw.checked_add(cd_size_usize).ok_or_else(|| String::from("ZIP central directory offset overflow"))?;
        if cd_end_raw > eocd {
            return Err(String::from("ZIP central directory extends past end-of-central-directory"));
        }
        let (cd_start, cd_end, preamble_delta) = if total_entries > 0 && cd_end_raw < eocd {
            let delta = eocd - cd_end_raw;
            let adj_start = cd_start_raw.checked_add(delta).ok_or_else(|| String::from("ZIP central directory offset overflow"))?;
            let adj_end = adj_start.checked_add(cd_size_usize).ok_or_else(|| String::from("ZIP central directory offset overflow"))?;
            let raw_ok = cd_start_raw.checked_add(4).and_then(|e| bytes.get(cd_start_raw..e)) == Some(CD_SIG.as_slice());
            let adj_ok = adj_start.checked_add(4).and_then(|e| bytes.get(adj_start..e)) == Some(CD_SIG.as_slice());
            if raw_ok && adj_ok {
                return Err(String::from("ambiguous dual ZIP central directory (polyglot decoy)"));
            }
            if !raw_ok && adj_ok { (adj_start, adj_end, delta) } else { (cd_start_raw, cd_end_raw, 0) }
        } else {
            (cd_start_raw, cd_end_raw, 0)
        };

        let (mut cursor, mut total_uncompressed, mut seen) = (cd_start, 0usize, BTreeSet::new());
        let (mut entries, mut entry_comments) = (Vec::with_capacity(total_entries.min(256)), Vec::new());

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
            let raw_name = std::str::from_utf8(bytes.get(name_start..name_end).ok_or_else(|| String::from("ZIP entry name out of bounds"))?)
                .map_err(|_| String::from("ZIP entry path is not valid UTF-8"))?;
            let comment_bytes = bytes.get(comment_start..next_cursor).unwrap_or_default();
            cursor = next_cursor;

            let is_dir = raw_name.ends_with('/');
            let normalized = sanitize_zip_path(if is_dir { raw_name.trim_end_matches('/') } else { raw_name })?;
            if !comment_bytes.is_empty() {
                entry_comments.push((normalized.clone(), comment_bytes.to_vec()));
            }
            if is_dir {
                if uncomp_size != 0 {
                    return Err(format!("ZIP directory entry `{raw_name}` has non-zero payload"));
                }
                continue;
            }
            if !seen.insert(normalized.to_ascii_lowercase()) {
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

    pub fn iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.entries.iter().map(|(n, b)| (n.as_str(), b.as_slice()))
    }

    pub fn archive_comment(&self) -> &[u8] {
        &self.archive_comment
    }

    pub fn entry_comments(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.entry_comments.iter().map(|(n, b)| (n.as_str(), b.as_slice()))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn find_eocd(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() < EOCD_MIN_LEN {
        return Err(String::from("input too small to be a ZIP archive"));
    }
    let min_pos = bytes.len().saturating_sub(EOCD_MIN_LEN + EOCD_MAX_COMMENT);
    for pos in (min_pos..=bytes.len() - EOCD_MIN_LEN).rev() {
        if bytes.get(pos..pos + 4) == Some(EOCD_SIG.as_slice()) && pos + EOCD_MIN_LEN + usize::from(read_u16_le(bytes, pos + 20)?) == bytes.len() {
            return Ok(pos);
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
    if lfh_offset.checked_add(LFH_FIXED_LEN).is_none_or(|end| end > bytes.len()) || bytes.get(lfh_offset..lfh_offset + 4) != Some(LFH_SIG.as_slice())
    {
        return Err(format!("ZIP entry `{name}` has invalid local file header"));
    }
    let data_start = lfh_offset
        .checked_add(LFH_FIXED_LEN)
        .and_then(|v| v.checked_add(usize::from(read_u16_le(bytes, lfh_offset + 26).ok()?)))
        .and_then(|v| v.checked_add(usize::from(read_u16_le(bytes, lfh_offset + 28).ok()?)))
        .ok_or_else(|| format!("ZIP entry `{name}` data offset overflow"))?;
    let data_end = data_start.checked_add(comp_size).ok_or_else(|| format!("ZIP entry `{name}` payload length overflow"))?;
    let compressed = bytes.get(data_start..data_end).ok_or_else(|| format!("ZIP entry `{name}` payload truncated"))?;
    let out = match method {
        0 => {
            if comp_size != uncomp_size {
                return Err(format!("ZIP stored entry `{name}` length mismatch"));
            }
            compressed.to_vec()
        }
        8 => {
            let cap = uncomp_size.min(limits.max_entry_uncompressed_bytes);
            let mut buf = Vec::with_capacity(cap.min(1024 * 1024));
            DeflateDecoder::new(compressed)
                .take(cap.saturating_add(1) as u64)
                .read_to_end(&mut buf)
                .map_err(|e| format!("ZIP `{name}` deflate: {e}"))?;
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
    if raw.is_empty()
        || raw.len() > MAX_ZIP_PATH_LEN
        || raw.contains(['\0', '\\'])
        || raw.starts_with('/')
        || raw.as_bytes().get(1) == Some(&b':')
        || raw.split('/').any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err(format!("unsafe ZIP entry path `{raw}`"));
    }
    Ok(raw.to_owned())
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let s = offset.checked_add(2).and_then(|e| bytes.get(offset..e)).ok_or_else(|| String::from("unexpected EOF reading ZIP u16"))?;
    Ok(u16::from_le_bytes(s.try_into().map_err(|_| String::from("invalid ZIP u16"))?))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let s = offset.checked_add(4).and_then(|e| bytes.get(offset..e)).ok_or_else(|| String::from("unexpected EOF reading ZIP u32"))?;
    Ok(u32::from_le_bytes(s.try_into().map_err(|_| String::from("invalid ZIP u32"))?))
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
        let (mut out, mut cd) = (Vec::new(), Vec::new());
        for (name, data) in &self.entries {
            let offset = u32::try_from(out.len()).map_err(|_| String::from("test zip overflow"))?;
            let size = u32::try_from(data.len()).map_err(|_| String::from("test zip entry overflow"))?;
            let name_len = u16::try_from(name.len()).map_err(|_| String::from("test zip name overflow"))?;
            let crc = crc32fast::hash(data);
            for chunk in [
                &LFH_SIG[..],
                &20u16.to_le_bytes(),
                &[0; 8],
                &crc.to_le_bytes(),
                &size.to_le_bytes(),
                &size.to_le_bytes(),
                &name_len.to_le_bytes(),
                &[0; 2],
                name.as_bytes(),
                data,
            ] {
                out.extend_from_slice(chunk);
            }
            for chunk in [
                &CD_SIG[..],
                &20u16.to_le_bytes(),
                &20u16.to_le_bytes(),
                &[0; 8],
                &crc.to_le_bytes(),
                &size.to_le_bytes(),
                &size.to_le_bytes(),
                &name_len.to_le_bytes(),
                &[0; 12],
                &offset.to_le_bytes(),
                name.as_bytes(),
            ] {
                cd.extend_from_slice(chunk);
            }
        }
        let cd_offset = u32::try_from(out.len()).map_err(|_| String::from("test zip cd offset overflow"))?;
        let cd_size = u32::try_from(cd.len()).map_err(|_| String::from("test zip cd size overflow"))?;
        let count = u16::try_from(self.entries.len()).map_err(|_| String::from("test zip entry count overflow"))?;
        for chunk in
            [&cd[..], &EOCD_SIG, &[0; 4], &count.to_le_bytes(), &count.to_le_bytes(), &cd_size.to_le_bytes(), &cd_offset.to_le_bytes(), &[0; 2]]
        {
            out.extend_from_slice(chunk);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_limits_traversal_crc_case_collision_and_symlinks() {
        let mut zw = TestZipWriter::new();
        zw.add_file("dir/hello.txt", b"hello world".to_vec()).unwrap();
        let bytes = zw.finish().unwrap();
        let arc = ZipArchive::from_bytes_with_limits(&bytes, ZipLimits::default()).unwrap();
        assert_eq!(arc.iter().collect::<Vec<_>>(), vec![("dir/hello.txt", b"hello world".as_slice())]);
        assert!(
            ZipArchive::from_bytes_with_limits(
                &bytes,
                ZipLimits { max_entries: 1, max_total_uncompressed_bytes: 5, max_entry_uncompressed_bytes: 5 }
            )
            .is_err()
        );

        let mut bad = TestZipWriter::new();
        bad.add_file("../escape.txt", b"bad".to_vec()).unwrap();
        assert!(ZipArchive::from_bytes_with_limits(&bad.finish().unwrap(), ZipLimits::default()).is_err());

        let mut corrupt = bytes.clone();
        let idx = corrupt.windows(11).position(|w| w == b"hello world").unwrap();
        corrupt[idx] ^= 0xff;
        assert!(ZipArchive::from_bytes_with_limits(&corrupt, ZipLimits::default()).is_err());

        let mut dup = TestZipWriter::new();
        dup.add_file("doc/README.txt", b"a".to_vec()).unwrap();
        dup.add_file("doc/readme.txt", b"b".to_vec()).unwrap();
        assert!(ZipArchive::from_bytes_with_limits(&dup.finish().unwrap(), ZipLimits::default()).unwrap_err().contains("case-colliding"));

        let mut sym = TestZipWriter::new();
        sym.add_file("link.txt", b"/etc/passwd".to_vec()).unwrap();
        let mut sbytes = sym.finish().unwrap();
        let cd = sbytes.windows(4).position(|w| w == CD_SIG).unwrap();
        sbytes[cd + 4..cd + 6].copy_from_slice(&((19u16 << 8) | 20u16).to_le_bytes());
        sbytes[cd + 38..cd + 42].copy_from_slice(&(0o12_0777u32 << 16).to_le_bytes());
        assert!(ZipArchive::from_bytes_with_limits(&sbytes, ZipLimits::default()).unwrap_err().contains("symlink"));
    }
}
