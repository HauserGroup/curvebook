//! Bounded, deterministic pure-Rust ZIP reader and writer built over `flate2` and `crc32fast`.

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

use crate::error::FormatError;

/// Default maximum entry count allowed in a single ZIP archive.
pub const MAX_ZIP_ENTRIES: usize = 4_096;

/// Default maximum cumulative uncompressed bytes allowed across all ZIP entries (128 MiB).
pub const MAX_ZIP_UNCOMPRESSED_BYTES: u64 = 128 * 1024 * 1024;

/// Default maximum uncompressed bytes allowed for any single ZIP entry (64 MiB).
pub const MAX_ZIP_ENTRY_BYTES: u64 = 64 * 1024 * 1024;

const LOCAL_FILE_HEADER_SIG: u32 = 0x0403_4b50;
const CENTRAL_DIR_HEADER_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATE: u16 = 8;
const FLAG_ENCRYPTED: u16 = 0x0001;
const FLAG_UTF8: u16 = 0x0800;
const DETERMINISTIC_DOS_DATE: u16 = 0x0021; // 1980-01-01
const DETERMINISTIC_DOS_TIME: u16 = 0x0000;

/// Security and memory-safety bounds enforced when reading or writing ZIP archives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipLimits {
    pub max_entries: usize,
    pub max_total_uncompressed_bytes: u64,
    pub max_entry_uncompressed_bytes: u64,
}

impl Default for ZipLimits {
    fn default() -> Self {
        Self {
            max_entries: MAX_ZIP_ENTRIES,
            max_total_uncompressed_bytes: MAX_ZIP_UNCOMPRESSED_BYTES,
            max_entry_uncompressed_bytes: MAX_ZIP_ENTRY_BYTES,
        }
    }
}

/// Sanitize an archive-internal relative path, rejecting traversal (`..`), absolute paths,
/// backslashes, drive prefixes, and NUL bytes.
pub fn sanitize_zip_path(raw: &str) -> Result<String, FormatError> {
    if raw.is_empty() || raw.contains('\0') || raw.contains('\\') || raw.starts_with('/') {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }
    let is_dir = raw.ends_with('/');
    let trimmed = raw.strip_suffix('/').unwrap_or(raw);
    if trimmed.is_empty() {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }
    for (idx, seg) in trimmed.split('/').enumerate() {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(FormatError::UnsafePath { path: raw.to_owned() });
        }
        if idx == 0 && seg.contains(':') {
            return Err(FormatError::UnsafePath { path: raw.to_owned() });
        }
    }
    Ok(if is_dir { format!("{trimmed}/") } else { trimmed.to_owned() })
}

/// Deterministic ZIP archive builder writing entries sorted lexicographically by path.
#[derive(Debug, Default)]
pub struct ZipWriter {
    entries: BTreeMap<String, Vec<u8>>,
    total_uncompressed: u64,
    limits: ZipLimits,
}

impl ZipWriter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_limits(limits: ZipLimits) -> Self {
        Self { entries: BTreeMap::new(), total_uncompressed: 0, limits }
    }

    /// Add a file entry to the archive, rejecting unsafe paths, duplicates, and limit violations.
    pub fn add_file(&mut self, path: &str, data: Vec<u8>) -> Result<(), FormatError> {
        let clean = sanitize_zip_path(path)?;
        if clean.ends_with('/') {
            return Err(FormatError::UnsafePath { path: path.to_owned() });
        }
        if self.entries.contains_key(&clean) {
            return Err(FormatError::DuplicateEntry { path: clean });
        }
        if self.entries.len() >= self.limits.max_entries {
            return Err(FormatError::ZipBomb { reason: format!("entry count exceeds limit {}", self.limits.max_entries) });
        }
        let len = data.len() as u64;
        if len > self.limits.max_entry_uncompressed_bytes {
            return Err(FormatError::ZipBomb { reason: format!("entry `{clean}` size {len} exceeds limit") });
        }
        let next_total = self
            .total_uncompressed
            .checked_add(len)
            .ok_or_else(|| FormatError::ZipBomb { reason: String::from("total uncompressed size overflow") })?;
        if next_total > self.limits.max_total_uncompressed_bytes {
            return Err(FormatError::ZipBomb { reason: String::from("cumulative uncompressed bytes exceed limit") });
        }
        self.total_uncompressed = next_total;
        self.entries.insert(clean, data);
        Ok(())
    }

    /// Serialize all queued entries into a deterministic ZIP byte buffer.
    pub fn finish(self) -> Result<Vec<u8>, FormatError> {
        if self.entries.len() > usize::from(u16::MAX) {
            return Err(FormatError::ZipBomb { reason: String::from("entry count exceeds u16 ZIP limit") });
        }
        let mut out = Vec::new();
        let mut central_records = Vec::with_capacity(self.entries.len());

        for (name, raw) in &self.entries {
            let local_offset =
                u32::try_from(out.len()).map_err(|_| FormatError::ZipBomb { reason: String::from("archive exceeds 4 GiB offset limit") })?;
            let crc = crc32fast::hash(raw);
            let uncompressed_len =
                u32::try_from(raw.len()).map_err(|_| FormatError::ZipBomb { reason: String::from("entry exceeds 4 GiB size limit") })?;
            let (method, payload) = compress_entry(raw)?;
            let compressed_len =
                u32::try_from(payload.len()).map_err(|_| FormatError::ZipBomb { reason: String::from("compressed entry exceeds 4 GiB limit") })?;
            let name_bytes = name.as_bytes();
            let name_len = u16::try_from(name_bytes.len()).map_err(|_| FormatError::UnsafePath { path: name.clone() })?;

            write_local_header(&mut out, method, crc, compressed_len, uncompressed_len, name_len, name_bytes);
            out.extend_from_slice(&payload);
            central_records.push(CentralRecord { name_bytes: name_bytes.to_vec(), method, crc, compressed_len, uncompressed_len, local_offset });
        }

        let cd_offset =
            u32::try_from(out.len()).map_err(|_| FormatError::ZipBomb { reason: String::from("central directory offset exceeds 4 GiB") })?;
        let mut cd_size: u32 = 0;
        for rec in &central_records {
            let before = out.len();
            write_central_header(&mut out, rec);
            let written = u32::try_from(out.len() - before).map_err(|_| FormatError::ZipBomb { reason: String::from("central record too large") })?;
            cd_size = cd_size.checked_add(written).ok_or_else(|| FormatError::ZipBomb { reason: String::from("central directory exceeds 4 GiB") })?;
        }
        let count =
            u16::try_from(central_records.len()).map_err(|_| FormatError::ZipBomb { reason: String::from("entry count exceeds u16 ZIP limit") })?;
        write_eocd(&mut out, count, cd_size, cd_offset);
        Ok(out)
    }
}

struct CentralRecord {
    name_bytes: Vec<u8>,
    method: u16,
    crc: u32,
    compressed_len: u32,
    uncompressed_len: u32,
    local_offset: u32,
}

fn compress_entry(raw: &[u8]) -> Result<(u16, Vec<u8>), FormatError> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(raw)?;
    let deflated = encoder.finish()?;
    if deflated.len() < raw.len() { Ok((METHOD_DEFLATE, deflated)) } else { Ok((METHOD_STORED, raw.to_vec())) }
}

fn write_local_header(out: &mut Vec<u8>, method: u16, crc: u32, compressed_len: u32, uncompressed_len: u32, name_len: u16, name_bytes: &[u8]) {
    out.extend_from_slice(&LOCAL_FILE_HEADER_SIG.to_le_bytes());
    out.extend_from_slice(&20_u16.to_le_bytes());
    out.extend_from_slice(&FLAG_UTF8.to_le_bytes());
    out.extend_from_slice(&method.to_le_bytes());
    out.extend_from_slice(&DETERMINISTIC_DOS_TIME.to_le_bytes());
    out.extend_from_slice(&DETERMINISTIC_DOS_DATE.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&compressed_len.to_le_bytes());
    out.extend_from_slice(&uncompressed_len.to_le_bytes());
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(name_bytes);
}

fn write_central_header(out: &mut Vec<u8>, rec: &CentralRecord) {
    let name_len = rec.name_bytes.len() as u16;
    out.extend_from_slice(&CENTRAL_DIR_HEADER_SIG.to_le_bytes());
    out.extend_from_slice(&0x0314_u16.to_le_bytes());
    out.extend_from_slice(&20_u16.to_le_bytes());
    out.extend_from_slice(&FLAG_UTF8.to_le_bytes());
    out.extend_from_slice(&rec.method.to_le_bytes());
    out.extend_from_slice(&DETERMINISTIC_DOS_TIME.to_le_bytes());
    out.extend_from_slice(&DETERMINISTIC_DOS_DATE.to_le_bytes());
    out.extend_from_slice(&rec.crc.to_le_bytes());
    out.extend_from_slice(&rec.compressed_len.to_le_bytes());
    out.extend_from_slice(&rec.uncompressed_len.to_le_bytes());
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&rec.local_offset.to_le_bytes());
    out.extend_from_slice(&rec.name_bytes);
}

fn write_eocd(out: &mut Vec<u8>, count: u16, cd_size: u32, cd_offset: u32) {
    out.extend_from_slice(&EOCD_SIG.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
}

/// Decoded in-memory ZIP archive with pre-verified CRC-32 and security bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipArchive {
    entries: BTreeMap<String, Vec<u8>>,
}

struct ParsedCdEntry {
    path: String,
    is_dir: bool,
    flags: u16,
    method: u16,
    crc: u32,
    compressed_size: u64,
    uncompressed_size: u64,
    local_offset: usize,
}

impl ZipArchive {
    /// Parse and decompress a ZIP archive from raw bytes using [`ZipLimits::default`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::from_bytes_with_limits(bytes, ZipLimits::default())
    }

    /// Parse and decompress a ZIP archive enforcing explicit [`ZipLimits`].
    pub fn from_bytes_with_limits(bytes: &[u8], limits: ZipLimits) -> Result<Self, FormatError> {
        let eocd = locate_eocd(bytes)?;
        if eocd.total_entries > limits.max_entries {
            return Err(FormatError::ZipBomb { reason: format!("ZIP entry count {} exceeds limit {}", eocd.total_entries, limits.max_entries) });
        }
        let cd_entries = parse_central_directory(bytes, &eocd, limits)?;

        let mut remaining_budget = limits.max_total_uncompressed_bytes;
        let mut entries = BTreeMap::new();
        for cd in cd_entries {
            if cd.is_dir {
                continue;
            }
            let payload = read_and_verify_entry(bytes, &cd, remaining_budget, limits.max_entry_uncompressed_bytes)?;
            remaining_budget = remaining_budget.saturating_sub(payload.len() as u64);
            entries.insert(cd.path, payload);
        }
        Ok(Self { entries })
    }

    /// Borrow decompressed bytes for `path`.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&[u8]> {
        self.entries.get(path).map(Vec::as_slice)
    }

    /// Borrow decompressed UTF-8 text for `path`, if valid UTF-8.
    pub fn get_utf8(&self, path: &str) -> Result<Option<&str>, FormatError> {
        let Some(bytes) = self.get(path) else {
            return Ok(None);
        };
        let text =
            std::str::from_utf8(bytes).map_err(|err| FormatError::InvalidZip { reason: format!("entry `{path}` is not valid UTF-8: {err}") })?;
        Ok(Some(text))
    }

    /// Iterate over `(path, bytes)` entries in deterministic sorted order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Best-effort bounded read of only `names` from `bytes` without enforcing archive-wide entry
    /// counts or cumulative uncompressed budgets on unrelated entries.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn peek_named_entries_bounded(bytes: &[u8], names: &[&str], max_entry_bytes: u64) -> Result<BTreeMap<String, Vec<u8>>, FormatError> {
        let eocd = locate_eocd(bytes)?;
        let mut cursor = eocd.cd_offset;
        let cd_end = eocd.cd_offset + eocd.cd_size;
        let mut out = BTreeMap::new();

        for _ in 0..eocd.total_entries {
            if cursor + 46 > cd_end || read_u32_le(bytes, cursor)? != CENTRAL_DIR_HEADER_SIG {
                return Err(FormatError::InvalidZip { reason: String::from("invalid central directory while peeking entries") });
            }
            let flags = read_u16_le(bytes, cursor + 8)?;
            if (flags & FLAG_ENCRYPTED) != 0 {
                return Err(FormatError::UnsupportedFormat { reason: String::from("encrypted ZIP entries are not supported") });
            }
            let method = read_u16_le(bytes, cursor + 10)?;
            let crc = read_u32_le(bytes, cursor + 16)?;
            let compressed_size = u64::from(read_u32_le(bytes, cursor + 20)?);
            let uncompressed_size = u64::from(read_u32_le(bytes, cursor + 24)?);
            let name_len = usize::from(read_u16_le(bytes, cursor + 28)?);
            let extra_len = usize::from(read_u16_le(bytes, cursor + 30)?);
            let comment_len = usize::from(read_u16_le(bytes, cursor + 32)?);
            let local_offset = usize::try_from(read_u32_le(bytes, cursor + 42)?)
                .map_err(|_| FormatError::InvalidZip { reason: String::from("local header offset overflow") })?;
            let name_start = cursor + 46;
            let record_end = name_start
                .checked_add(name_len)
                .and_then(|v| v.checked_add(extra_len))
                .and_then(|v| v.checked_add(comment_len))
                .ok_or_else(|| FormatError::InvalidZip { reason: String::from("central directory record overflow") })?;
            if record_end > cd_end {
                return Err(FormatError::InvalidZip { reason: String::from("central directory record extends past directory end") });
            }
            if let Some(name_bytes) = name_start.checked_add(name_len).and_then(|end| bytes.get(name_start..end))
                && let Ok(raw_name) = std::str::from_utf8(name_bytes)
                && let Ok(clean) = sanitize_zip_path(raw_name)
                && names.contains(&clean.as_str())
                && uncompressed_size <= max_entry_bytes
            {
                let cd = ParsedCdEntry { path: clean.clone(), is_dir: false, flags, method, crc, compressed_size, uncompressed_size, local_offset };
                if let Ok(payload) = read_and_verify_entry(bytes, &cd, max_entry_bytes, max_entry_bytes) {
                    out.insert(clean, payload);
                }
            }
            cursor = record_end;
        }
        Ok(out)
    }
}

struct EocdInfo {
    total_entries: usize,
    cd_size: usize,
    cd_offset: usize,
}

fn validate_eocd_candidate(bytes: &[u8], pos: usize) -> Result<Result<EocdInfo, FormatError>, FormatError> {
    let disk = read_u16_le(bytes, pos + 4)?;
    let cd_disk = read_u16_le(bytes, pos + 6)?;
    if disk != 0 || cd_disk != 0 {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("multi-disk ZIP archives are not supported") }));
    }
    let entries_disk = usize::from(read_u16_le(bytes, pos + 8)?);
    let total_entries = usize::from(read_u16_le(bytes, pos + 10)?);
    if entries_disk != total_entries {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("inconsistent central directory entry count") }));
    }
    let Ok(cd_size) = usize::try_from(read_u32_le(bytes, pos + 12)?) else {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("central directory size overflow") }));
    };
    let Ok(cd_offset) = usize::try_from(read_u32_le(bytes, pos + 16)?) else {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("central directory offset overflow") }));
    };
    let Some(cd_end) = cd_offset.checked_add(cd_size) else {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("central directory range overflow") }));
    };
    if cd_end > pos {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("central directory extends past EOCD") }));
    }
    if total_entries > 0 {
        if cd_size < 46 {
            return Ok(Err(FormatError::InvalidZip { reason: String::from("truncated central directory header") }));
        }
        if read_u32_le(bytes, cd_offset)? != CENTRAL_DIR_HEADER_SIG {
            return Ok(Err(FormatError::InvalidZip { reason: String::from("invalid central directory signature") }));
        }
    } else if cd_size != 0 {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("central directory trailing bytes mismatch") }));
    }
    Ok(Ok(EocdInfo { total_entries, cd_size, cd_offset }))
}

fn locate_eocd(bytes: &[u8]) -> Result<EocdInfo, FormatError> {
    if bytes.len() < 22 {
        return Err(FormatError::InvalidZip { reason: String::from("buffer smaller than 22-byte EOCD") });
    }
    let min_offset = bytes.len().saturating_sub(22 + usize::from(u16::MAX));
    let mut pos = bytes.len() - 22;
    let mut candidate_err: Option<FormatError> = None;
    loop {
        if read_u32_le(bytes, pos)? == EOCD_SIG {
            let comment_len = usize::from(read_u16_le(bytes, pos + 20)?);
            if pos + 22 + comment_len == bytes.len() {
                match validate_eocd_candidate(bytes, pos)? {
                    Ok(eocd) => return Ok(eocd),
                    Err(err) => {
                        if candidate_err.is_none() {
                            candidate_err = Some(err);
                        }
                    }
                }
            }
        }
        if pos == min_offset {
            break;
        }
        pos -= 1;
    }
    Err(candidate_err.unwrap_or_else(|| FormatError::InvalidZip { reason: String::from("missing End of Central Directory record") }))
}

fn parse_central_directory(bytes: &[u8], eocd: &EocdInfo, limits: ZipLimits) -> Result<Vec<ParsedCdEntry>, FormatError> {
    let mut cursor = eocd.cd_offset;
    let cd_end = eocd.cd_offset + eocd.cd_size;
    let mut seen = HashSet::with_capacity(eocd.total_entries);
    let mut out = Vec::with_capacity(eocd.total_entries);
    let mut declared_total: u64 = 0;

    for _ in 0..eocd.total_entries {
        if cursor + 46 > cd_end {
            return Err(FormatError::InvalidZip { reason: String::from("truncated central directory header") });
        }
        if read_u32_le(bytes, cursor)? != CENTRAL_DIR_HEADER_SIG {
            return Err(FormatError::InvalidZip { reason: String::from("invalid central directory signature") });
        }
        let flags = read_u16_le(bytes, cursor + 8)?;
        if (flags & FLAG_ENCRYPTED) != 0 {
            return Err(FormatError::UnsupportedFormat { reason: String::from("encrypted ZIP entries are not supported") });
        }
        let method = read_u16_le(bytes, cursor + 10)?;
        let crc = read_u32_le(bytes, cursor + 16)?;
        let compressed_size = u64::from(read_u32_le(bytes, cursor + 20)?);
        let uncompressed_size = u64::from(read_u32_le(bytes, cursor + 24)?);
        let name_len = usize::from(read_u16_le(bytes, cursor + 28)?);
        let extra_len = usize::from(read_u16_le(bytes, cursor + 30)?);
        let comment_len = usize::from(read_u16_le(bytes, cursor + 32)?);
        let local_offset = usize::try_from(read_u32_le(bytes, cursor + 42)?)
            .map_err(|_| FormatError::InvalidZip { reason: String::from("local header offset overflow") })?;

        let name_start = cursor + 46;
        let record_end = name_start
            .checked_add(name_len)
            .and_then(|v| v.checked_add(extra_len))
            .and_then(|v| v.checked_add(comment_len))
            .ok_or_else(|| FormatError::InvalidZip { reason: String::from("central directory record overflow") })?;
        if record_end > cd_end {
            return Err(FormatError::InvalidZip { reason: String::from("central directory record extends past directory end") });
        }

        let name_bytes = name_start
            .checked_add(name_len)
            .and_then(|end| bytes.get(name_start..end))
            .ok_or_else(|| FormatError::InvalidZip { reason: String::from("central directory entry filename out of bounds") })?;
        let raw_name =
            std::str::from_utf8(name_bytes).map_err(|_| FormatError::InvalidZip { reason: String::from("non-UTF-8 ZIP entry filename") })?;
        let clean = sanitize_zip_path(raw_name)?;
        if !seen.insert(clean.clone()) {
            return Err(FormatError::DuplicateEntry { path: clean });
        }
        if uncompressed_size > limits.max_entry_uncompressed_bytes {
            return Err(FormatError::ZipBomb { reason: format!("entry `{clean}` declared size {uncompressed_size} exceeds limit") });
        }
        declared_total = declared_total
            .checked_add(uncompressed_size)
            .ok_or_else(|| FormatError::ZipBomb { reason: String::from("declared total uncompressed size overflow") })?;
        if declared_total > limits.max_total_uncompressed_bytes {
            return Err(FormatError::ZipBomb { reason: String::from("declared total uncompressed size exceeds archive limit") });
        }

        let is_dir = clean.ends_with('/');
        out.push(ParsedCdEntry { path: clean, is_dir, flags, method, crc, compressed_size, uncompressed_size, local_offset });
        cursor = record_end;
    }

    if cursor != cd_end {
        return Err(FormatError::InvalidZip { reason: String::from("central directory trailing bytes mismatch") });
    }
    Ok(out)
}

fn read_and_verify_entry(bytes: &[u8], cd: &ParsedCdEntry, remaining_budget: u64, max_entry_bytes: u64) -> Result<Vec<u8>, FormatError> {
    if cd.local_offset.checked_add(30).is_none_or(|end| end > bytes.len()) {
        return Err(FormatError::InvalidZip { reason: format!("truncated local header for `{}`", cd.path) });
    }
    if read_u32_le(bytes, cd.local_offset)? != LOCAL_FILE_HEADER_SIG {
        return Err(FormatError::InvalidZip { reason: format!("invalid local header signature for `{}`", cd.path) });
    }
    let name_len = usize::from(read_u16_le(bytes, cd.local_offset + 26)?);
    let extra_len = usize::from(read_u16_le(bytes, cd.local_offset + 28)?);
    let data_start = cd
        .local_offset
        .checked_add(30)
        .and_then(|v| v.checked_add(name_len))
        .and_then(|v| v.checked_add(extra_len))
        .ok_or_else(|| FormatError::InvalidZip { reason: String::from("local header data offset overflow") })?;
    let comp_len = usize::try_from(cd.compressed_size).map_err(|_| FormatError::ZipBomb { reason: String::from("compressed size exceeds usize") })?;
    let data_end =
        data_start.checked_add(comp_len).ok_or_else(|| FormatError::InvalidZip { reason: String::from("compressed payload range overflow") })?;
    let compressed =
        bytes.get(data_start..data_end).ok_or_else(|| FormatError::InvalidZip { reason: format!("entry `{}` payload extends past EOF", cd.path) })?;
    let entry_cap = max_entry_bytes.min(remaining_budget);

    let decompressed = match cd.method {
        METHOD_STORED => {
            if cd.compressed_size != cd.uncompressed_size && (cd.flags & 0x0008) == 0 {
                return Err(FormatError::InvalidZip { reason: format!("stored entry `{}` size mismatch", cd.path) });
            }
            if (compressed.len() as u64) > entry_cap {
                return Err(FormatError::ZipBomb { reason: format!("stored entry `{}` exceeds byte limit", cd.path) });
            }
            compressed.to_vec()
        }
        METHOD_DEFLATE => {
            let cap_plus_one = entry_cap.saturating_add(1);
            let mut decoder = DeflateDecoder::new(compressed).take(cap_plus_one);
            let mut buf = Vec::with_capacity(usize::try_from(cd.uncompressed_size.min(1_048_576)).unwrap_or(0));
            decoder.read_to_end(&mut buf).map_err(|err| FormatError::InvalidZip { reason: format!("deflate error in `{}`: {err}", cd.path) })?;
            if (buf.len() as u64) > entry_cap {
                return Err(FormatError::ZipBomb { reason: format!("entry `{}` decompressed payload exceeds security limit", cd.path) });
            }
            buf
        }
        other => return Err(FormatError::UnsupportedCompression { entry: cd.path.clone(), method: other }),
    };

    if (decompressed.len() as u64) != cd.uncompressed_size {
        return Err(FormatError::InvalidZip {
            reason: format!("entry `{}` uncompressed size mismatch (expected {}, got {})", cd.path, cd.uncompressed_size, decompressed.len()),
        });
    }
    let actual_crc = crc32fast::hash(&decompressed);
    if actual_crc != cd.crc {
        return Err(FormatError::CorruptCrc { entry: cd.path.clone(), expected: cd.crc, actual: actual_crc });
    }
    Ok(decompressed)
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, FormatError> {
    let slice = offset
        .checked_add(2)
        .and_then(|end| bytes.get(offset..end))
        .ok_or_else(|| FormatError::InvalidZip { reason: String::from("unexpected EOF reading u16") })?;
    let &[b0, b1] = slice else {
        return Err(FormatError::InvalidZip { reason: String::from("unexpected EOF reading u16") });
    };
    Ok(u16::from_le_bytes([b0, b1]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, FormatError> {
    let slice = offset
        .checked_add(4)
        .and_then(|end| bytes.get(offset..end))
        .ok_or_else(|| FormatError::InvalidZip { reason: String::from("unexpected EOF reading u32") })?;
    let &[b0, b1, b2, b3] = slice else {
        return Err(FormatError::InvalidZip { reason: String::from("unexpected EOF reading u32") });
    };
    Ok(u32::from_le_bytes([b0, b1, b2, b3]))
}
