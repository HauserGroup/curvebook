//! Bounded, deterministic pure-Rust ZIP reader and writer built over `flate2` and `crc32fast`.
//!
//! Supports:
//! - Standard ZIP and ZIP64 reading per PKWARE `APPNOTE.TXT` §4.3.14–4.3.16 (ZIP64 EOCD record
//!   and locator) and §4.5.3 (`0x0001` ZIP64 extended information extra field), validating every
//!   ZIP64 field against [`ZipLimits`] before allocating.
//! - Strict default reading (`ZipReadOptions::strict()`) for native `.curvebook`/`.cbtemplate`
//!   archives alongside [`ZipReadOptions::external_interop()`] for external archives (`\` -> `/`
//!   normalization, `./` dropping, post-normalization & optional case-folded collision rejection,
//!   and CP437 filename decoding per `APPNOTE.TXT` Appendix D / Unicode `CP437.TXT`).
//! - Lazy [`ZipIndex`] that parses only the central directory up front, decompresses entries on
//!   demand with CRC-32 verification and a cumulative budget of decoded bytes actually read, and
//!   exposes unread entry names and coarse name classes without decompressing them.

mod cp437;

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;

pub use cp437::decode_cp437;

use cp437::decode_zip_entry_name;

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
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const ZIP64_EOCD_LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_EXTRA_HEADER_ID: u16 = 0x0001;
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

/// Options governing ZIP entry path normalization, filename decoding, and duplicate checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZipReadOptions {
    /// Normalize `\` separators to `/` and drop `./` segments while retaining strict rejection of
    /// `..`, absolute paths, drive prefixes, and NUL bytes.
    pub normalize_separators_and_dot_segments: bool,
    /// When general-purpose bit 11 (UTF-8 flag) is unset and an entry name is not valid UTF-8,
    /// decode it as IBM Code Page 437 per PKWARE `APPNOTE.TXT` Appendix D (`CP437.TXT`).
    pub decode_cp437_filenames: bool,
    /// Reject normalized entry paths that collide under Unicode case-folding as
    /// [`FormatError::DuplicateEntry`].
    pub reject_case_folded_duplicates: bool,
}

impl Default for ZipReadOptions {
    fn default() -> Self {
        Self::strict()
    }
}

impl ZipReadOptions {
    /// Strict native archive policy used by `.curvebook` and `.cbtemplate` archives:
    /// UTF-8 entry names only, no backslashes, no `./` segments, case-sensitive uniqueness.
    #[must_use]
    pub const fn strict() -> Self {
        Self { normalize_separators_and_dot_segments: false, decode_cp437_filenames: false, reject_case_folded_duplicates: false }
    }

    /// Interoperability policy for external third-party ZIP containers:
    /// normalizes `\` to `/`, drops `./` segments, rejects post-normalization collisions as
    /// [`FormatError::DuplicateEntry`], and decodes non-UTF-8 entry names as CP437 when bit 11
    /// is unset. Strict rejection of `..`, absolute paths, drive prefixes, and NUL is preserved.
    #[must_use]
    pub const fn external_interop() -> Self {
        Self { normalize_separators_and_dot_segments: true, decode_cp437_filenames: true, reject_case_folded_duplicates: false }
    }

    /// Enable or disable case-folded collision detection (for archives using case-insensitive
    /// entry lookups).
    #[must_use]
    pub const fn with_reject_case_folded_duplicates(mut self, reject: bool) -> Self {
        self.reject_case_folded_duplicates = reject;
        self
    }
}

/// Coarse name-based classification of a ZIP entry for reporting unread/opaque entries without
/// decompressing their payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ZipEntryClass {
    Preview,
    Cache,
    Database,
    Other,
}

/// Classify a normalized ZIP entry path by name class (`Preview`, `Cache`, `Database`, `Other`).
#[must_use]
pub fn classify_zip_entry_name(path: &str) -> ZipEntryClass {
    let lower = path.to_ascii_lowercase();
    let trimmed = lower.trim_end_matches('/');
    let mut segments = trimmed.split('/');
    let mut last_seg = "";
    let mut has_db_dir = false;
    let mut has_cache_dir = false;
    let mut has_preview_dir = false;

    for seg in &mut segments {
        last_seg = seg;
        if matches!(seg, "db" | "database" | "databases") {
            has_db_dir = true;
        } else if matches!(seg, "cache" | "caches" | "__pycache__") {
            has_cache_dir = true;
        } else if matches!(seg, "preview" | "previews" | "thumbnail" | "thumbnails" | "thumb" | "thumbs" | "quicklook") {
            has_preview_dir = true;
        }
    }

    let (stem, ext) = last_seg.rsplit_once('.').map_or((last_seg, ""), |(s, e)| (s, e));
    if has_db_dir || matches!(ext, "db" | "sqlite" | "sqlite3" | "mdb" | "accdb" | "ldb" | "wal" | "shm") {
        ZipEntryClass::Database
    } else if has_cache_dir || matches!(ext, "cache" | "tmp" | "bak") || stem.contains("cache") {
        ZipEntryClass::Cache
    } else if has_preview_dir || stem.contains("preview") || stem.contains("thumbnail") || stem.starts_with("thumb") {
        ZipEntryClass::Preview
    } else {
        ZipEntryClass::Other
    }
}

/// Metadata for a single ZIP central-directory entry inspected without decompressing its payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntryInfo {
    pub path: String,
    pub is_dir: bool,
    pub method: u16,
    pub crc32: u32,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
}

/// Sanitize an archive-internal relative path under strict native rules, rejecting traversal
/// (`..`), absolute paths, backslashes, `./` segments, drive prefixes, and NUL bytes.
pub fn sanitize_zip_path(raw: &str) -> Result<String, FormatError> {
    sanitize_zip_path_with_options(raw, ZipReadOptions::strict())
}

/// Sanitize (and optionally normalize under [`ZipReadOptions::external_interop`]) an archive
/// entry path.
pub fn sanitize_zip_path_with_options(raw: &str, options: ZipReadOptions) -> Result<String, FormatError> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }
    if !options.normalize_separators_and_dot_segments {
        return sanitize_zip_path_strict(raw);
    }
    if raw.starts_with('/') || raw.starts_with('\\') {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }

    let normalized: String = raw.chars().map(|ch| if ch == '\\' { '/' } else { ch }).collect();
    let is_dir = normalized.ends_with('/');
    let trimmed = normalized.strip_suffix('/').unwrap_or(&normalized);
    if trimmed.is_empty() {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }

    let parts: Vec<&str> = trimmed.split('/').collect();
    let last_idx = parts.len().saturating_sub(1);
    let mut clean_parts: Vec<&str> = Vec::with_capacity(parts.len());

    for (idx, &seg) in parts.iter().enumerate() {
        if seg.is_empty() || seg == ".." {
            return Err(FormatError::UnsafePath { path: raw.to_owned() });
        }
        if seg == "." {
            // Only drop `./` segments (interior or followed by trailing `/`); reject bare trailing `a/.`.
            if idx == last_idx && !is_dir {
                return Err(FormatError::UnsafePath { path: raw.to_owned() });
            }
            continue;
        }
        if clean_parts.is_empty() && seg.contains(':') {
            return Err(FormatError::UnsafePath { path: raw.to_owned() });
        }
        clean_parts.push(seg);
    }

    if clean_parts.is_empty() {
        return Err(FormatError::UnsafePath { path: raw.to_owned() });
    }
    let joined = clean_parts.join("/");
    Ok(if is_dir { format!("{joined}/") } else { joined })
}

fn sanitize_zip_path_strict(raw: &str) -> Result<String, FormatError> {
    if raw.contains('\\') || raw.starts_with('/') {
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

    /// Add an explicit empty directory entry (ending with `/`) to the archive.
    ///
    /// Adding an already-registered directory entry is a deterministic no-op.
    pub fn add_dir(&mut self, path: &str) -> Result<(), FormatError> {
        let with_slash = if path.ends_with('/') { path.to_owned() } else { format!("{path}/") };
        let clean = sanitize_zip_path(&with_slash)?;
        if self.entries.contains_key(&clean) {
            return Ok(());
        }
        if self.entries.len() >= self.limits.max_entries {
            return Err(FormatError::ZipBomb { reason: format!("entry count exceeds limit {}", self.limits.max_entries) });
        }
        self.entries.insert(clean, Vec::new());
        Ok(())
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

#[derive(Debug, Clone)]
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

impl ParsedCdEntry {
    fn to_info(&self) -> ZipEntryInfo {
        ZipEntryInfo {
            path: self.path.clone(),
            is_dir: self.is_dir,
            method: self.method,
            crc32: self.crc,
            compressed_size: self.compressed_size,
            uncompressed_size: self.uncompressed_size,
        }
    }
}

impl ZipArchive {
    /// Parse and decompress a ZIP archive from raw bytes using [`ZipLimits::default`] and strict
    /// native path rules.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::from_bytes_with_options(bytes, ZipLimits::default(), ZipReadOptions::strict())
    }

    /// Parse and decompress a ZIP archive enforcing explicit [`ZipLimits`] and strict native path
    /// rules.
    pub fn from_bytes_with_limits(bytes: &[u8], limits: ZipLimits) -> Result<Self, FormatError> {
        Self::from_bytes_with_options(bytes, limits, ZipReadOptions::strict())
    }

    /// Parse and decompress a ZIP archive enforcing explicit [`ZipLimits`] and [`ZipReadOptions`].
    pub fn from_bytes_with_options(bytes: &[u8], limits: ZipLimits, options: ZipReadOptions) -> Result<Self, FormatError> {
        let eocd = locate_eocd(bytes, Some(limits.max_entries))?;
        let cd_entries = parse_central_directory(bytes, &eocd, limits, options, true)?;

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
        let eocd = locate_eocd(bytes, None)?;
        let mut cursor = eocd.cd_offset;
        let cd_end = eocd.cd_offset.saturating_add(eocd.cd_size);
        let mut out = BTreeMap::new();

        for _ in 0..eocd.total_entries {
            let (raw_cd, record_end) = read_cd_header_record(bytes, cursor, cd_end, eocd.is_zip64)?;
            if (raw_cd.flags & FLAG_ENCRYPTED) != 0 {
                return Err(FormatError::UnsupportedFormat { reason: String::from("encrypted ZIP entries are not supported") });
            }
            if let Ok(raw_name) = std::str::from_utf8(raw_cd.name_bytes)
                && let Ok(clean) = sanitize_zip_path(raw_name)
                && names.contains(&clean.as_str())
                && raw_cd.uncompressed_size <= max_entry_bytes
            {
                let cd = ParsedCdEntry {
                    path: clean.clone(),
                    is_dir: false,
                    flags: raw_cd.flags,
                    method: raw_cd.method,
                    crc: raw_cd.crc,
                    compressed_size: raw_cd.compressed_size,
                    uncompressed_size: raw_cd.uncompressed_size,
                    local_offset: raw_cd.local_offset,
                };
                if let Ok(payload) = read_and_verify_entry(bytes, &cd, max_entry_bytes, max_entry_bytes) {
                    out.insert(clean, payload);
                }
            }
            cursor = record_end;
        }
        Ok(out)
    }
}

struct IndexedEntry {
    cd: ParsedCdEntry,
    was_read: bool,
}

/// Lazy ZIP reader that indexes only the central directory up front and decompresses entries on
/// demand with CRC-32 verification and a cumulative budget of decoded bytes actually read.
///
/// Unread entries do not consume the cumulative read budget and can be inspected by name and
/// [`ZipEntryClass`] without decompressing their payloads.
pub struct ZipIndex<'a> {
    bytes: &'a [u8],
    limits: ZipLimits,
    entries: BTreeMap<String, IndexedEntry>,
    total_cd_entries: usize,
    bytes_read: u64,
}

impl<'a> ZipIndex<'a> {
    /// Index a ZIP archive central directory using [`ZipLimits::default`] and strict native rules.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, FormatError> {
        Self::from_bytes_with_options(bytes, ZipLimits::default(), ZipReadOptions::strict())
    }

    /// Index a ZIP archive central directory enforcing explicit [`ZipLimits`] and strict native rules.
    pub fn from_bytes_with_limits(bytes: &'a [u8], limits: ZipLimits) -> Result<Self, FormatError> {
        Self::from_bytes_with_options(bytes, limits, ZipReadOptions::strict())
    }

    /// Index a ZIP archive central directory enforcing explicit [`ZipLimits`] and [`ZipReadOptions`].
    ///
    /// Enforces `limits.max_entries` on the central-directory entry count before allocating,
    /// validates ZIP64 metadata, path safety, and uniqueness, and defers decompression and
    /// uncompressed byte budgets to [`Self::read_entry`] / [`Self::read_entry_utf8`].
    pub fn from_bytes_with_options(bytes: &'a [u8], limits: ZipLimits, options: ZipReadOptions) -> Result<Self, FormatError> {
        let eocd = locate_eocd(bytes, Some(limits.max_entries))?;
        let cd_entries = parse_central_directory(bytes, &eocd, limits, options, false)?;
        let total_cd_entries = cd_entries.len();

        let mut entries = BTreeMap::new();
        for cd in cd_entries {
            if cd.is_dir {
                continue;
            }
            entries.insert(cd.path.clone(), IndexedEntry { cd, was_read: false });
        }
        Ok(Self { bytes, limits, entries, total_cd_entries, bytes_read: 0 })
    }

    /// Number of non-directory file entries indexed in this archive.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether this archive contains zero non-directory file entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total number of central-directory records parsed (including directory entries).
    #[must_use]
    pub const fn total_central_directory_entries(&self) -> usize {
        self.total_cd_entries
    }

    /// Whether a non-directory entry with normalized path `path` exists in the index.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }

    /// Inspect central-directory metadata for `path` without decompressing it.
    #[must_use]
    pub fn entry_info(&self, path: &str) -> Option<ZipEntryInfo> {
        self.entries.get(path).map(|entry| entry.cd.to_info())
    }

    /// Iterate over all indexed file entry paths in deterministic sorted order.
    pub fn entry_names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Iterate over metadata for all indexed file entries in deterministic sorted order.
    pub fn entries(&self) -> impl Iterator<Item = ZipEntryInfo> + '_ {
        self.entries.values().map(|entry| entry.cd.to_info())
    }

    /// Cumulative decompressed bytes actually read through this index so far.
    #[must_use]
    pub const fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    /// Remaining cumulative decompressed byte budget available for future reads.
    #[must_use]
    pub const fn remaining_budget(&self) -> u64 {
        self.limits.max_total_uncompressed_bytes.saturating_sub(self.bytes_read)
    }

    /// Whether `path` has been decompressed via [`Self::read_entry`] or [`Self::read_entry_utf8`].
    #[must_use]
    pub fn is_read(&self, path: &str) -> bool {
        self.entries.get(path).is_some_and(|entry| entry.was_read)
    }

    /// Decompress and CRC-verify `path` on demand, charging decoded bytes against the cumulative
    /// budget (`limits.max_total_uncompressed_bytes`) and checking `limits.max_entry_uncompressed_bytes`
    /// before allocating.
    pub fn read_entry(&mut self, path: &str) -> Result<Option<Vec<u8>>, FormatError> {
        let Some(entry) = self.entries.get_mut(path) else {
            return Ok(None);
        };
        if (entry.cd.flags & FLAG_ENCRYPTED) != 0 {
            return Err(FormatError::UnsupportedFormat { reason: String::from("encrypted ZIP entries are not supported") });
        }
        if entry.cd.uncompressed_size > self.limits.max_entry_uncompressed_bytes {
            return Err(FormatError::ZipBomb {
                reason: format!("entry `{}` declared size {} exceeds limit", entry.cd.path, entry.cd.uncompressed_size),
            });
        }
        let remaining = self.limits.max_total_uncompressed_bytes.saturating_sub(self.bytes_read);
        if entry.cd.uncompressed_size > remaining {
            return Err(FormatError::ZipBomb {
                reason: format!(
                    "entry `{}` declared size {} exceeds remaining cumulative budget {remaining}",
                    entry.cd.path, entry.cd.uncompressed_size
                ),
            });
        }

        let payload = read_and_verify_entry(self.bytes, &entry.cd, remaining, self.limits.max_entry_uncompressed_bytes)?;
        self.bytes_read = self.bytes_read.saturating_add(payload.len() as u64);
        entry.was_read = true;
        Ok(Some(payload))
    }

    /// Decompress, CRC-verify, and UTF-8 decode `path` on demand.
    pub fn read_entry_utf8(&mut self, path: &str) -> Result<Option<String>, FormatError> {
        let Some(bytes) = self.read_entry(path)? else {
            return Ok(None);
        };
        let text = String::from_utf8(bytes).map_err(|err| FormatError::InvalidZip { reason: format!("entry `{path}` is not valid UTF-8: {err}") })?;
        Ok(Some(text))
    }

    /// Iterate over normalized paths of entries that have not been read, in deterministic sorted
    /// order, without decompressing them.
    pub fn unread_entry_names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().filter(|(_, entry)| !entry.was_read).map(|(path, _)| path.as_str())
    }

    /// Iterate over metadata for unread entries without decompressing them.
    pub fn unread_entries(&self) -> impl Iterator<Item = ZipEntryInfo> + '_ {
        self.entries.values().filter(|entry| !entry.was_read).map(|entry| entry.cd.to_info())
    }

    /// Group unread entries by [`ZipEntryClass`] (`Preview`, `Cache`, `Database`, `Other`) without
    /// decompressing them.
    #[must_use]
    pub fn unread_entry_counts_by_class(&self) -> BTreeMap<ZipEntryClass, usize> {
        let mut counts = BTreeMap::new();
        for path in self.unread_entry_names() {
            let class = classify_zip_entry_name(path);
            *counts.entry(class).or_insert(0) += 1;
        }
        counts
    }
}

struct EocdInfo {
    total_entries: usize,
    cd_size: usize,
    cd_offset: usize,
    is_zip64: bool,
}

struct StandardEocdFields {
    disk: u16,
    cd_disk: u16,
    entries_disk: u16,
    total_entries: u16,
    cd_size: u32,
    cd_offset: u32,
}

fn read_standard_eocd_fields(bytes: &[u8], pos: usize) -> Result<StandardEocdFields, FormatError> {
    Ok(StandardEocdFields {
        disk: read_u16_le(bytes, pos + 4)?,
        cd_disk: read_u16_le(bytes, pos + 6)?,
        entries_disk: read_u16_le(bytes, pos + 8)?,
        total_entries: read_u16_le(bytes, pos + 10)?,
        cd_size: read_u32_le(bytes, pos + 12)?,
        cd_offset: read_u32_le(bytes, pos + 16)?,
    })
}

fn has_zip64_locator_before(bytes: &[u8], eocd_pos: usize) -> Result<bool, FormatError> {
    if eocd_pos < 20 {
        return Ok(false);
    }
    Ok(read_u32_le(bytes, eocd_pos - 20)? == ZIP64_EOCD_LOCATOR_SIG)
}

fn parse_zip64_eocd(
    bytes: &[u8],
    eocd_pos: usize,
    std_eocd: &StandardEocdFields,
    max_entries: Option<usize>,
) -> Result<Result<EocdInfo, FormatError>, FormatError> {
    let loc_pos = eocd_pos - 20;
    let z64_disk = read_u32_le(bytes, loc_pos + 4)?;
    let z64_offset_u64 = read_u64_le(bytes, loc_pos + 8)?;
    let total_disks = read_u32_le(bytes, loc_pos + 16)?;
    if z64_disk != 0 || total_disks > 1 {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("multi-disk ZIP64 archives are not supported") }));
    }
    let Ok(z64_pos) = usize::try_from(z64_offset_u64) else {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("ZIP64 EOCD offset overflow") }));
    };
    if z64_pos.checked_add(56).is_none_or(|end| end > loc_pos) {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("ZIP64 EOCD offset out of bounds") }));
    }
    if read_u32_le(bytes, z64_pos)? != ZIP64_EOCD_SIG {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("invalid ZIP64 EOCD signature") }));
    }
    let record_size = read_u64_le(bytes, z64_pos + 4)?;
    if record_size < 44 {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("truncated ZIP64 EOCD record size") }));
    }
    let Ok(record_total) = u64::checked_add(record_size, 12).and_then(|v| usize::try_from(v).ok()).ok_or(()) else {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("ZIP64 EOCD record size overflow") }));
    };
    if z64_pos.checked_add(record_total).is_none_or(|end| end > loc_pos) {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("ZIP64 EOCD record extends past locator") }));
    }

    let disk = read_u32_le(bytes, z64_pos + 16)?;
    let cd_disk = read_u32_le(bytes, z64_pos + 20)?;
    if disk != 0 || cd_disk != 0 {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("multi-disk ZIP64 archives are not supported") }));
    }
    if (std_eocd.disk != u16::MAX && std_eocd.disk != 0) || (std_eocd.cd_disk != u16::MAX && std_eocd.cd_disk != 0) {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("multi-disk ZIP archives are not supported") }));
    }

    let entries_disk = read_u64_le(bytes, z64_pos + 24)?;
    let total_entries_u64 = read_u64_le(bytes, z64_pos + 32)?;
    if entries_disk != total_entries_u64 {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("inconsistent ZIP64 central directory entry count") }));
    }
    let cd_size_u64 = read_u64_le(bytes, z64_pos + 40)?;
    let cd_offset_u64 = read_u64_le(bytes, z64_pos + 48)?;

    if std_eocd.total_entries != u16::MAX && u64::from(std_eocd.total_entries) != total_entries_u64 {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("ZIP64 entry count conflicts with standard EOCD") }));
    }
    if std_eocd.cd_size != u32::MAX && u64::from(std_eocd.cd_size) != cd_size_u64 {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("ZIP64 central directory size conflicts with standard EOCD") }));
    }
    if std_eocd.cd_offset != u32::MAX && u64::from(std_eocd.cd_offset) != cd_offset_u64 {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("ZIP64 central directory offset conflicts with standard EOCD") }));
    }

    finalize_eocd_bounds(
        bytes,
        RawEocdBounds { total_entries: total_entries_u64, cd_size: cd_size_u64, cd_offset: cd_offset_u64, upper_bound: z64_pos, is_zip64: true },
        max_entries,
    )
}

struct RawEocdBounds {
    total_entries: u64,
    cd_size: u64,
    cd_offset: u64,
    upper_bound: usize,
    is_zip64: bool,
}

fn finalize_eocd_bounds(bytes: &[u8], raw: RawEocdBounds, max_entries: Option<usize>) -> Result<Result<EocdInfo, FormatError>, FormatError> {
    if let Some(limit) = max_entries
        && raw.total_entries > limit as u64
    {
        return Ok(Err(FormatError::ZipBomb { reason: format!("ZIP entry count {} exceeds limit {limit}", raw.total_entries) }));
    }
    let Ok(cd_size) = usize::try_from(raw.cd_size) else {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("central directory size overflow") }));
    };
    let Ok(cd_offset) = usize::try_from(raw.cd_offset) else {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("central directory offset overflow") }));
    };
    let Some(cd_end) = cd_offset.checked_add(cd_size) else {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("central directory range overflow") }));
    };
    if cd_end > raw.upper_bound {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("central directory extends past EOCD") }));
    }
    if raw.total_entries > (cd_size as u64) / 46 {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("central directory too small for declared entry count") }));
    }
    let Ok(total_entries) = usize::try_from(raw.total_entries) else {
        return Ok(Err(FormatError::ZipBomb { reason: String::from("ZIP entry count exceeds usize") }));
    };
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
    Ok(Ok(EocdInfo { total_entries, cd_size, cd_offset, is_zip64: raw.is_zip64 }))
}

fn validate_eocd_candidate(bytes: &[u8], pos: usize, max_entries: Option<usize>) -> Result<Result<EocdInfo, FormatError>, FormatError> {
    let std_eocd = read_standard_eocd_fields(bytes, pos)?;
    if has_zip64_locator_before(bytes, pos)? {
        return parse_zip64_eocd(bytes, pos, &std_eocd, max_entries);
    }

    if std_eocd.disk != 0 || std_eocd.cd_disk != 0 {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("multi-disk ZIP archives are not supported") }));
    }
    if std_eocd.entries_disk != std_eocd.total_entries {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("inconsistent central directory entry count") }));
    }
    if std_eocd.cd_size == u32::MAX || std_eocd.cd_offset == u32::MAX {
        return Ok(Err(FormatError::InvalidZip { reason: String::from("ZIP64 sentinel in EOCD without ZIP64 locator") }));
    }
    finalize_eocd_bounds(
        bytes,
        RawEocdBounds {
            total_entries: u64::from(std_eocd.total_entries),
            cd_size: u64::from(std_eocd.cd_size),
            cd_offset: u64::from(std_eocd.cd_offset),
            upper_bound: pos,
            is_zip64: false,
        },
        max_entries,
    )
}

fn locate_eocd(bytes: &[u8], max_entries: Option<usize>) -> Result<EocdInfo, FormatError> {
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
                match validate_eocd_candidate(bytes, pos, max_entries)? {
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

struct RawCdHeader<'a> {
    name_bytes: &'a [u8],
    flags: u16,
    method: u16,
    crc: u32,
    compressed_size: u64,
    uncompressed_size: u64,
    local_offset: usize,
}

fn read_cd_header_record(bytes: &[u8], cursor: usize, cd_end: usize, archive_is_zip64: bool) -> Result<(RawCdHeader<'_>, usize), FormatError> {
    if cursor.checked_add(46).is_none_or(|end| end > cd_end) {
        return Err(FormatError::InvalidZip { reason: String::from("truncated central directory header") });
    }
    if read_u32_le(bytes, cursor)? != CENTRAL_DIR_HEADER_SIG {
        return Err(FormatError::InvalidZip { reason: String::from("invalid central directory signature") });
    }
    let flags = read_u16_le(bytes, cursor + 8)?;
    let method = read_u16_le(bytes, cursor + 10)?;
    let crc = read_u32_le(bytes, cursor + 16)?;
    let compressed_32 = read_u32_le(bytes, cursor + 20)?;
    let uncompressed_32 = read_u32_le(bytes, cursor + 24)?;
    let name_len = usize::from(read_u16_le(bytes, cursor + 28)?);
    let extra_len = usize::from(read_u16_le(bytes, cursor + 30)?);
    let comment_len = usize::from(read_u16_le(bytes, cursor + 32)?);
    let disk_start_16 = read_u16_le(bytes, cursor + 34)?;
    let local_offset_32 = read_u32_le(bytes, cursor + 42)?;

    let name_start = cursor + 46;
    let name_end =
        name_start.checked_add(name_len).ok_or_else(|| FormatError::InvalidZip { reason: String::from("central directory filename overflow") })?;
    let extra_end =
        name_end.checked_add(extra_len).ok_or_else(|| FormatError::InvalidZip { reason: String::from("central directory extra field overflow") })?;
    let record_end =
        extra_end.checked_add(comment_len).ok_or_else(|| FormatError::InvalidZip { reason: String::from("central directory record overflow") })?;
    if record_end > cd_end {
        return Err(FormatError::InvalidZip { reason: String::from("central directory record extends past directory end") });
    }

    let name_bytes = bytes
        .get(name_start..name_end)
        .ok_or_else(|| FormatError::InvalidZip { reason: String::from("central directory entry filename out of bounds") })?;
    let extra_bytes = bytes
        .get(name_end..extra_end)
        .ok_or_else(|| FormatError::InvalidZip { reason: String::from("central directory extra field out of bounds") })?;

    let (uncompressed_size, compressed_size, local_offset_u64) =
        resolve_cd_zip64_fields(extra_bytes, uncompressed_32, compressed_32, local_offset_32, disk_start_16, archive_is_zip64)?;
    let local_offset =
        usize::try_from(local_offset_u64).map_err(|_| FormatError::InvalidZip { reason: String::from("local header offset overflow") })?;

    Ok((RawCdHeader { name_bytes, flags, method, crc, compressed_size, uncompressed_size, local_offset }, record_end))
}

/// Resolve per-entry ZIP64 fields from the `0x0001` extra field per PKWARE `APPNOTE.TXT` §4.5.3.
///
/// Field order inside `0x0001` (present only for fields set to `0xFFFF_FFFF` / `0xFFFF` in the
/// fixed header):
/// 1. Original (uncompressed) size (`u64`)
/// 2. Compressed size (`u64`)
/// 3. Relative local header offset (`u64`)
/// 4. Disk start number (`u32`)
fn resolve_cd_zip64_fields(
    extra: &[u8],
    uncompressed_32: u32,
    compressed_32: u32,
    local_offset_32: u32,
    disk_start_16: u16,
    archive_is_zip64: bool,
) -> Result<(u64, u64, u64), FormatError> {
    let need_uncomp = uncompressed_32 == u32::MAX;
    let need_comp = compressed_32 == u32::MAX;
    let need_offset = local_offset_32 == u32::MAX;
    let need_disk = disk_start_16 == u16::MAX;

    if !need_disk && disk_start_16 != 0 {
        return Err(FormatError::InvalidZip { reason: String::from("multi-disk ZIP entry is not supported") });
    }
    if !(need_uncomp || need_comp || need_offset || need_disk) {
        return Ok((u64::from(uncompressed_32), u64::from(compressed_32), u64::from(local_offset_32)));
    }

    let Some(z64_data) = find_extra_field(extra, ZIP64_EXTRA_HEADER_ID) else {
        if !archive_is_zip64 && !need_disk {
            // Plain non-ZIP64 archive with literal 0xFFFF_FFFF sizes (e.g. declared 4 GiB bomb):
            // return literal 32-bit values so downstream ZipLimits check surfaces ZipBomb cleanly.
            return Ok((u64::from(uncompressed_32), u64::from(compressed_32), u64::from(local_offset_32)));
        }
        return Err(FormatError::InvalidZip { reason: String::from("missing ZIP64 0x0001 extra field for sentinel entry header") });
    };
    let mut off = 0_usize;

    let uncompressed = if need_uncomp {
        let val = read_u64_le(z64_data, off)?;
        off += 8;
        val
    } else {
        u64::from(uncompressed_32)
    };
    let compressed = if need_comp {
        let val = read_u64_le(z64_data, off)?;
        off += 8;
        val
    } else {
        u64::from(compressed_32)
    };
    let local_offset = if need_offset {
        let val = read_u64_le(z64_data, off)?;
        off += 8;
        val
    } else {
        u64::from(local_offset_32)
    };
    if need_disk {
        let disk_start = read_u32_le(z64_data, off)?;
        if disk_start != 0 {
            return Err(FormatError::InvalidZip { reason: String::from("multi-disk ZIP64 entry is not supported") });
        }
    }
    Ok((uncompressed, compressed, local_offset))
}

fn find_extra_field(extra: &[u8], target_id: u16) -> Option<&[u8]> {
    let mut pos = 0_usize;
    while pos.checked_add(4).is_some_and(|hdr_end| hdr_end <= extra.len()) {
        let header_id = read_u16_le(extra, pos).ok()?;
        let data_len = usize::from(read_u16_le(extra, pos + 2).ok()?);
        let data_start = pos + 4;
        let data_end = data_start.checked_add(data_len)?;
        if data_end > extra.len() {
            return None;
        }
        if header_id == target_id {
            return extra.get(data_start..data_end);
        }
        pos = data_end;
    }
    None
}

fn parse_central_directory(
    bytes: &[u8],
    eocd: &EocdInfo,
    limits: ZipLimits,
    options: ZipReadOptions,
    enforce_eager_size_budgets: bool,
) -> Result<Vec<ParsedCdEntry>, FormatError> {
    let mut cursor = eocd.cd_offset;
    let cd_end = eocd.cd_offset.saturating_add(eocd.cd_size);
    let mut seen = HashSet::with_capacity(eocd.total_entries);
    let mut seen_folded: Option<HashSet<String>> = options.reject_case_folded_duplicates.then(|| HashSet::with_capacity(eocd.total_entries));
    let mut out = Vec::with_capacity(eocd.total_entries);
    let mut declared_total: u64 = 0;

    for _ in 0..eocd.total_entries {
        let (raw_cd, record_end) = read_cd_header_record(bytes, cursor, cd_end, eocd.is_zip64)?;
        if (raw_cd.flags & FLAG_ENCRYPTED) != 0 {
            return Err(FormatError::UnsupportedFormat { reason: String::from("encrypted ZIP entries are not supported") });
        }

        let raw_name = decode_zip_entry_name(raw_cd.name_bytes, raw_cd.flags, options.decode_cp437_filenames)?;
        let clean = sanitize_zip_path_with_options(&raw_name, options)?;
        if !seen.insert(clean.clone()) {
            return Err(FormatError::DuplicateEntry { path: clean });
        }
        if let Some(folded_set) = seen_folded.as_mut()
            && !folded_set.insert(clean.to_lowercase())
        {
            return Err(FormatError::DuplicateEntry { path: clean });
        }

        if enforce_eager_size_budgets {
            if raw_cd.uncompressed_size > limits.max_entry_uncompressed_bytes {
                return Err(FormatError::ZipBomb { reason: format!("entry `{clean}` declared size {} exceeds limit", raw_cd.uncompressed_size) });
            }
            declared_total = declared_total
                .checked_add(raw_cd.uncompressed_size)
                .ok_or_else(|| FormatError::ZipBomb { reason: String::from("declared total uncompressed size overflow") })?;
            if declared_total > limits.max_total_uncompressed_bytes {
                return Err(FormatError::ZipBomb { reason: String::from("declared total uncompressed size exceeds archive limit") });
            }
        }

        let is_dir = clean.ends_with('/');
        out.push(ParsedCdEntry {
            path: clean,
            is_dir,
            flags: raw_cd.flags,
            method: raw_cd.method,
            crc: raw_cd.crc,
            compressed_size: raw_cd.compressed_size,
            uncompressed_size: raw_cd.uncompressed_size,
            local_offset: raw_cd.local_offset,
        });
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
            let initial_cap = usize::try_from(cd.uncompressed_size.min(entry_cap).min(1_048_576)).unwrap_or(0);
            let mut buf = Vec::with_capacity(initial_cap);
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

fn read_u64_le(bytes: &[u8], offset: usize) -> Result<u64, FormatError> {
    let slice = offset
        .checked_add(8)
        .and_then(|end| bytes.get(offset..end))
        .ok_or_else(|| FormatError::InvalidZip { reason: String::from("unexpected EOF reading u64") })?;
    let &[b0, b1, b2, b3, b4, b5, b6, b7] = slice else {
        return Err(FormatError::InvalidZip { reason: String::from("unexpected EOF reading u64") });
    };
    Ok(u64::from_le_bytes([b0, b1, b2, b3, b4, b5, b6, b7]))
}
