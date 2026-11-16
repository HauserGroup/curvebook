//! `cargo xtask brands`: keep competitor vendor/product trademarks and retired suite branding out
//! of user-facing strings, package descriptions, workflows, and packaging metadata (`AGENTS.md`).

use std::collections::BTreeSet;
use std::path::Path;

use lumencraft_chart::compile_graph;
use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, ConstantValue, GraphSheet, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, ProjectDocument, ResultSheet, SheetId,
    TableKind,
};
use lumencraft_engine::Session;
use lumencraft_format::{ZipArchive, ZipLimits, save_project_bytes, save_template_bytes};
use lumencraft_pdf::{PdfExportOptions, render_pdf};
use lumencraft_svg::{SvgExportOptions, render_svg};
use lumencraft_ui_egui::dialogs_canvas::{ANALYSIS_CATALOG, ActiveInspector, AnalysisCategory, CommandPaletteState};
use lumencraft_ui_egui::i18n;
use lumencraft_ui_egui::menus::{TOOLBAR_BUTTONS, menu_bar_tooltips, toolbar_tooltips};
use lumencraft_ui_egui::workspace_nav::{SAMPLE_FIXTURES, TableCreationDraft, WelcomeMode, instantiate_sample_fixture};
use lumencraft_ui_egui::{ActiveDialog, ClipboardBundle, UiApp, UiTreeSnapshot, inspect_ui_tree};

/// Competitor vendor and product names checked on ASCII word boundaries (case-insensitive).
const BRANDS: &[&str] =
    &["graphpad", "dotmatics", "prism", "instat", "statmate", "jmp", "spss", "minitab", "originlab", "originpro", "sigmaplot", "systat"];

/// Optical modifiers that immediately precede `"prism"` inside `<title>`/`<desc>` icon metadata.
const OPTICAL_PRISM_MODIFIERS: &[&str] =
    &["optical", "refractive", "triangular", "glass", "crystal", "dispersive", "dispersion", "dichroic", "achromatic"];

/// External suite branding and infrastructure forbidden across all non-xtask files in this standalone repository.
const SUITE_BRANDS: &[&str] = &[
    "artcraft",
    "getartcraft",
    "storytold",
    "storyteller",
    "craft-fonts",
    "craft_fonts",
    "craft_fonts_dir",
    "craft-common-actions",
    "craft-contrib",
    "craft-house-rules",
    "craft-ui",
    "craft_ui",
    "craft-keymap",
    "craft-update",
    "craft-window",
];

const ALLOW_MARKER: &str = "brand-ok";

/// Binary asset formats that do not carry UTF-8/UTF-16 text.
const BINARY_ASSET_EXT: &[&str] = &[
    "png",
    "jpg",
    "jpeg",
    "gif",
    "webp",
    "bmp",
    "tif",
    "tiff",
    "ico",
    "icns",
    "avif",
    "svgz",
    "pdf",
    "eps",
    "icc",
    "icm",
    "cur",
    "ani",
    "ase",
    "aco",
    "ttf",
    "otf",
    "ttc",
    "woff",
    "woff2",
    "eot",
    "pfb",
    "zip",
    "lumencraft",
    "lctemplate",
    "prism",
    "pzf",
    "xlsx",
    "mp4",
    "wav",
    "mp3",
];

/// The only file in which a suite name may appear, and only on a line that carries [`ALLOW_MARKER`].
/// The README's non-affiliation disclaimer has to name the suite it disclaims. Every other file,
/// and every unmarked README line, stays strict so suite branding cannot drift back into docs,
/// workflows, or packaging.
const SUITE_DISCLAIMER_PATH: &str = "README.md";

/// Suite names the disclaimer may use. Infrastructure identifiers (`craft-*` crates, the suite's
/// domain) are deliberately absent: naming the suite is a disclaimer, referencing its
/// infrastructure would be a dependency, which the standalone-repository rule forbids.
const SUITE_DISCLAIMER_NAMES: &[&str] = &["artcraft"];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BrandHit {
    pub path: String,
    pub line: usize,
    pub brand: &'static str,
}

enum DecodedFile {
    Text(String),
    BinaryAsset,
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let mut hits = scan_repo_checked(root, &files)?;
    hits.extend(check_ui_surfaces()?);
    if hits.is_empty() {
        let i18n_total = i18n::LOCALES.len() * i18n::catalog_len();
        println!(
            "brands: 0 unapproved brand or suite mentions across {} tracked files ({i18n_total} i18n entries, 26 dialogs, 7 inspectors, and export metadata clean)",
            files.len()
        );
        return Ok(());
    }
    let lines: Vec<String> = hits.iter().map(|h| format!("{}:{}: `{}`", h.path, h.line, h.brand)).collect();
    Err(format!(
        "{} unapproved brand/suite mention(s) in user-facing, UI, export, workflow, or packaging surfaces (use neutral scientific wording, or mark intentional format compatibility notes with `{ALLOW_MARKER}`):\n  {}",
        hits.len(),
        lines.join("\n  ")
    ))
}

pub fn scan_repo_checked(root: &Path, files: &[String]) -> Result<Vec<BrandHit>, String> {
    let mut hits = Vec::new();
    let mut scan_errors = Vec::new();
    for path in files {
        if skip_path(path) {
            continue;
        }
        let bytes = match std::fs::read(root.join(path)) {
            Ok(b) => b,
            Err(e) => {
                scan_errors.push(format!("cannot scan {path}: {e}"));
                continue;
            }
        };
        match decode_text_file(path, &bytes) {
            Ok(DecodedFile::Text(text)) => scan_file(path, &text, &mut hits),
            Ok(DecodedFile::BinaryAsset) => {}
            Err(msg) => scan_errors.push(msg),
        }
    }
    if !scan_errors.is_empty() {
        if !hits.is_empty() {
            for h in &hits {
                scan_errors.push(format!("{}:{}: `{}`", h.path, h.line, h.brand));
            }
        }
        return Err(scan_errors.join("\n  "));
    }
    Ok(hits)
}

fn is_binary_asset_ext(path: &str) -> bool {
    Path::new(path).extension().and_then(|e| e.to_str()).is_some_and(|ext| BINARY_ASSET_EXT.iter().any(|b| ext.eq_ignore_ascii_case(b)))
}

fn decode_text_file(path: &str, bytes: &[u8]) -> Result<DecodedFile, String> {
    if let Some(bomless) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return match std::str::from_utf8(bomless) {
            Ok(s) => Ok(DecodedFile::Text(s.to_owned())),
            Err(e) => Err(format!("cannot scan {path}: invalid UTF-8 after BOM: {e}")),
        };
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Ok(DecodedFile::Text(decode_utf16(bytes.get(2..).unwrap_or_default(), true)));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return Ok(DecodedFile::Text(decode_utf16(bytes.get(2..).unwrap_or_default(), false)));
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok(DecodedFile::Text(s.to_owned()));
    }
    if let Some(little_endian) = detect_bomless_utf16(bytes) {
        return Ok(DecodedFile::Text(decode_utf16(bytes, little_endian)));
    }
    if is_binary_asset_ext(path) {
        return Ok(DecodedFile::BinaryAsset);
    }
    Err(format!("cannot scan {path}: file is not valid UTF-8 or UTF-16"))
}

fn detect_bomless_utf16(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 8 || !bytes.contains(&0) {
        return None;
    }
    let mut le_ascii = 0usize;
    let mut be_ascii = 0usize;
    let (pairs, _) = bytes.as_chunks::<2>();
    let total_pairs = pairs.len();
    for &[a, b] in pairs {
        if a != 0 && (a.is_ascii_graphic() || a.is_ascii_whitespace()) && b == 0 {
            le_ascii += 1;
        } else if a == 0 && b != 0 && (b.is_ascii_graphic() || b.is_ascii_whitespace()) {
            be_ascii += 1;
        }
    }
    if le_ascii >= 4 && le_ascii * 4 >= total_pairs && le_ascii > be_ascii.saturating_mul(2) {
        Some(true)
    } else if be_ascii >= 4 && be_ascii * 4 >= total_pairs && be_ascii > le_ascii.saturating_mul(2) {
        Some(false)
    } else {
        None
    }
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let (pairs, remainder) = bytes.as_chunks::<2>();
    let units = pairs.iter().map(|&arr| if little_endian { u16::from_le_bytes(arr) } else { u16::from_be_bytes(arr) });
    let mut out = String::with_capacity(pairs.len());
    for ch in char::decode_utf16(units) {
        out.push(ch.unwrap_or('\u{FFFD}'));
    }
    if !remainder.is_empty() {
        out.push('\u{FFFD}');
    }
    out
}

fn skip_path(path: &str) -> bool {
    path.starts_with("xtask/")
        || path.starts_with("crates/testkit/")
        || path.starts_with("tests/")
        || path.contains("/tests/")
        || path.ends_with("_test.rs")
}

fn scan_file(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let start_idx = out.len();
    scan_suite_lines(path, text, out);
    let ext = Path::new(path).extension();
    let is_rs = ext.is_some_and(|e| e.eq_ignore_ascii_case("rs"));
    let is_svg = ext.is_some_and(|e| e.eq_ignore_ascii_case("svg"));
    let is_html = ext.is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"));
    let is_cargo_toml = path == "Cargo.toml" || path.ends_with("/Cargo.toml");
    let is_non_md_doc = path.starts_with("docs/") && !ext.is_some_and(|e| e.eq_ignore_ascii_case("md"));
    if is_rs {
        scan_rust(path, text, out);
    } else if is_cargo_toml {
        scan_cargo_toml(path, text, out);
    } else if is_svg {
        scan_svg(path, text, out);
    } else if path == "crates/tools/src/cursors.toml"
        || path.starts_with("packaging/")
        || path.starts_with(".github/")
        || path.starts_with("apps/")
        || is_non_md_doc
    {
        scan_plain_lines(path, text, out);
        if is_html {
            let lines: Vec<&str> = text.lines().collect();
            for hit in check_html_metadata(path, text) {
                let line_allowed =
                    lines.get(hit.line.saturating_sub(1)).is_some_and(|l| l.contains(ALLOW_MARKER)) || prev_line_allows(&lines, hit.line);
                if !line_allowed {
                    out.push(hit);
                }
            }
        }
    }
    dedup_tail_hits(out, start_idx);
}

fn dedup_tail_hits(hits: &mut Vec<BrandHit>, from: usize) {
    let mut seen: BTreeSet<(usize, &'static str)> = BTreeSet::new();
    let mut idx = from;
    while idx < hits.len() {
        if seen.insert((hits[idx].line, hits[idx].brand)) {
            idx += 1;
        } else {
            hits.remove(idx);
        }
    }
}

fn scan_suite_lines(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    for (idx, line) in text.lines().enumerate() {
        if path == SUITE_DISCLAIMER_PATH && line.contains(ALLOW_MARKER) {
            let strict: Vec<&'static str> = SUITE_BRANDS.iter().copied().filter(|brand| !SUITE_DISCLAIMER_NAMES.contains(brand)).collect();
            collect_line_words(path, idx + 1, line, &strict, PrismPolicy::Strict, out);
        } else {
            collect_line_words(path, idx + 1, line, SUITE_BRANDS, PrismPolicy::Strict, out);
        }
    }
}

fn is_comment_allow_line(line: &str) -> bool {
    if !line.contains(ALLOW_MARKER) {
        return false;
    }
    let trimmed = line.trim();
    trimmed.starts_with('#')
        || trimmed.starts_with("//")
        || (trimmed.starts_with("<!--") && trimmed.ends_with("-->"))
        || (trimmed.starts_with("/*") && trimmed.ends_with("*/"))
}

fn prev_line_allows(lines: &[&str], one_based_line: usize) -> bool {
    one_based_line.checked_sub(2).and_then(|idx| lines.get(idx)).is_some_and(|prev| is_comment_allow_line(prev))
}

fn scan_cargo_toml(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let mut scanner = TomlScanner::new(path, text, out);
    scanner.run();
}

fn scan_plain_lines(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let lines: Vec<&str> = text.lines().collect();
    for (idx, &line) in lines.iter().enumerate() {
        if line.contains(ALLOW_MARKER) || prev_line_allows(&lines, idx + 1) {
            continue;
        }
        collect_line_words(path, idx + 1, line, BRANDS, PrismPolicy::Standard, out);
    }
}

struct SvgChunk<'a> {
    text: &'a str,
    is_meta_text: bool,
    prev_word: Option<String>,
}

#[derive(Default)]
struct SvgState {
    in_comment: bool,
    in_tag: bool,
    tag_buf: String,
    in_meta: bool,
    prev_meta_word: Option<String>,
}

impl SvgState {
    fn split_line_chunks<'a>(&mut self, line: &'a str) -> Vec<SvgChunk<'a>> {
        let mut chunks = Vec::new();
        let bytes = line.as_bytes();
        let mut i = 0usize;
        let mut span_start = 0usize;
        while i < bytes.len() {
            if self.in_comment {
                if bytes.get(i..i + 3) == Some(b"-->") {
                    i += 3;
                    self.in_comment = false;
                    span_start = i;
                } else {
                    i += 1;
                }
                continue;
            }
            if self.in_tag {
                if bytes[i] == b'>' {
                    self.tag_buf.push_str(&line[span_start..=i]);
                    self.apply_completed_tag();
                    chunks.push(SvgChunk { text: &line[span_start..=i], is_meta_text: false, prev_word: None });
                    i += 1;
                    span_start = i;
                } else {
                    i += 1;
                }
                continue;
            }
            if bytes.get(i..i + 4) == Some(b"<!--") {
                self.push_text_chunk(line, span_start, i, &mut chunks);
                self.in_comment = true;
                i += 4;
                span_start = i;
                continue;
            }
            if bytes[i] == b'<' {
                self.push_text_chunk(line, span_start, i, &mut chunks);
                self.in_tag = true;
                self.tag_buf.clear();
                span_start = i;
            }
            i += 1;
        }
        if span_start < line.len() {
            if self.in_tag {
                self.tag_buf.push_str(&line[span_start..]);
            }
            if !self.in_comment && !self.in_tag {
                self.push_text_chunk(line, span_start, line.len(), &mut chunks);
            } else {
                chunks.push(SvgChunk { text: &line[span_start..], is_meta_text: false, prev_word: None });
            }
        }
        chunks
    }

    fn push_text_chunk<'a>(&mut self, line: &'a str, start: usize, end: usize, chunks: &mut Vec<SvgChunk<'a>>) {
        if start >= end {
            return;
        }
        let text = &line[start..end];
        let prev_word = if self.in_meta { self.prev_meta_word.clone() } else { None };
        chunks.push(SvgChunk { text, is_meta_text: self.in_meta, prev_word });
        if self.in_meta && !text.trim().is_empty() {
            let tail = trailing_ascii_word(text.trim_end()).to_ascii_lowercase();
            self.prev_meta_word = if tail.is_empty() { None } else { Some(tail) };
        }
    }

    fn apply_completed_tag(&mut self) {
        self.in_tag = false;
        let inner = self.tag_buf.trim_start_matches('<').trim_end_matches('>').trim();
        let lower = inner.to_ascii_lowercase();
        let is_close = lower.starts_with('/');
        let name_src = lower.trim_start_matches('/');
        let tag_name = name_src.split(|c: char| c.is_ascii_whitespace() || c == '/').next().unwrap_or("");
        if matches!(tag_name, "title" | "desc") {
            let self_closing = inner.ends_with('/');
            if is_close {
                self.in_meta = false;
                self.prev_meta_word = None;
            } else if !self_closing {
                self.in_meta = true;
                self.prev_meta_word = None;
            }
        }
        self.tag_buf.clear();
    }
}

fn scan_svg(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut state = SvgState::default();
    for (idx, &line) in lines.iter().enumerate() {
        let chunks = state.split_line_chunks(line);
        if line.contains(ALLOW_MARKER) || prev_line_allows(&lines, idx + 1) {
            continue;
        }
        let mut line_brands: BTreeSet<&'static str> = BTreeSet::new();
        for chunk in chunks {
            let policy =
                if chunk.is_meta_text { PrismPolicy::SvgTitleOrDesc { prev_word: chunk.prev_word.as_deref() } } else { PrismPolicy::Standard };
            let lower = chunk.text.to_ascii_lowercase();
            for &brand in BRANDS {
                if !line_brands.contains(brand) && has_word_match(chunk.text, &lower, brand, policy) {
                    line_brands.insert(brand);
                    out.push(BrandHit { path: path.to_owned(), line: idx + 1, brand });
                }
            }
        }
    }
}

struct TomlScanner<'a> {
    path: &'a str,
    src: &'a str,
    bytes: &'a [u8],
    lines: Vec<&'a str>,
    pos: usize,
    line: usize,
    in_metadata_section: bool,
    in_workspace_section: bool,
    seen: BTreeSet<(usize, &'static str)>,
    out: &'a mut Vec<BrandHit>,
}

impl<'a> TomlScanner<'a> {
    fn new(path: &'a str, src: &'a str, out: &'a mut Vec<BrandHit>) -> Self {
        Self {
            path,
            src,
            bytes: src.as_bytes(),
            lines: src.lines().collect(),
            pos: 0,
            line: 1,
            in_metadata_section: false,
            in_workspace_section: false,
            seen: BTreeSet::new(),
            out,
        }
    }

    fn run(&mut self) {
        while self.pos < self.bytes.len() {
            self.skip_ws_and_comments();
            if self.pos >= self.bytes.len() {
                break;
            }
            if self.bytes[self.pos] == b'[' {
                self.read_table_header();
            } else {
                self.read_statement();
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn advance_one(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    fn push_next_char(&mut self, text: &mut String, byte_lines: &mut Vec<usize>) {
        if let Some(rest) = self.src.get(self.pos..)
            && let Some(ch) = rest.chars().next()
        {
            let line = self.line;
            push_char_with_line(text, byte_lines, ch, line);
            self.pos += ch.len_utf8();
            if ch == '\n' {
                self.line += 1;
            }
        } else {
            self.advance_one();
        }
    }

    fn skip_comment(&mut self) {
        while let Some(b) = self.peek() {
            if b == b'\n' {
                break;
            }
            self.pos += 1;
        }
    }

    fn skip_ws_and_comments(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\r' | b'\n' => {
                    self.advance_one();
                }
                b'#' => self.skip_comment(),
                _ => break,
            }
        }
    }

    fn skip_inline_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
            self.pos += 1;
        }
    }

    fn read_table_header(&mut self) {
        let start_line = self.line;
        let prev_allows = prev_line_allows(&self.lines, start_line);
        let double = self.bytes.get(self.pos..self.pos + 2) == Some(b"[[");
        self.pos += if double { 2 } else { 1 };
        let (segments, key_lits) = self.read_dotted_key(true);
        if self.peek() == Some(b']') {
            self.pos += 1;
            if double && self.peek() == Some(b']') {
                self.pos += 1;
            }
        }
        while let Some(b) = self.advance_one() {
            if b == b'\n' {
                break;
            }
        }
        self.in_workspace_section = segments.len() == 1 && segments[0] == "workspace";
        self.in_metadata_section = is_metadata_table_segments(&segments);
        if self.in_metadata_section {
            for lit in &key_lits {
                self.check_literal(lit, prev_allows);
            }
        }
    }

    fn read_statement(&mut self) {
        let stmt_prev_allow = prev_line_allows(&self.lines, self.line);
        let (segments, key_lits) = self.read_dotted_key(false);
        if self.peek() != Some(b'=') {
            if let Some(b) = self.peek() {
                if b == b'\n' {
                    self.advance_one();
                } else {
                    while let Some(next) = self.advance_one() {
                        if next == b'\n' {
                            break;
                        }
                    }
                }
            }
            return;
        }
        self.pos += 1; // '='
        let is_meta = self.in_metadata_section || is_metadata_key_segments(&segments, self.in_workspace_section);
        if is_meta {
            for lit in &key_lits {
                self.check_literal(lit, stmt_prev_allow);
            }
        }
        self.scan_value(is_meta, stmt_prev_allow);
    }

    fn read_dotted_key(&mut self, in_header: bool) -> (Vec<String>, Vec<ExtractedLiteral>) {
        let mut segments = Vec::new();
        let mut lits = Vec::new();
        while let Some(b) = self.peek() {
            match b {
                b'\n' | b'#' => break,
                b']' if in_header => break,
                b'=' if !in_header => break,
                b'.' | b' ' | b'\t' | b'\r' => self.pos += 1,
                b'"' => {
                    let lit = self.read_basic_single_string();
                    segments.push(lit.text.clone());
                    lits.push(lit);
                }
                b'\'' => {
                    let lit = self.read_literal_single_string();
                    segments.push(lit.text.clone());
                    lits.push(lit);
                }
                _ => {
                    let lit = self.read_bare_token();
                    if !lit.text.is_empty() {
                        segments.push(lit.text.clone());
                        lits.push(lit);
                    }
                }
            }
        }
        (segments, lits)
    }

    fn scan_value(&mut self, is_meta: bool, stmt_prev_allow: bool) {
        let mut depth = 0usize;
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b'\n' => {
                    self.advance_one();
                    if depth == 0 {
                        break;
                    }
                }
                b'#' => self.skip_comment(),
                b'[' | b'{' => {
                    depth += 1;
                    self.pos += 1;
                }
                b']' | b'}' => {
                    depth = depth.saturating_sub(1);
                    self.pos += 1;
                }
                b'"' => {
                    let lit = if self.bytes.get(self.pos..self.pos + 3) == Some(b"\"\"\"") {
                        self.read_basic_multiline_string()
                    } else {
                        self.read_basic_single_string()
                    };
                    if is_meta {
                        self.check_literal(&lit, stmt_prev_allow);
                    }
                }
                b'\'' => {
                    let lit = if self.bytes.get(self.pos..self.pos + 3) == Some(b"'''") {
                        self.read_literal_multiline_string()
                    } else {
                        self.read_literal_single_string()
                    };
                    if is_meta {
                        self.check_literal(&lit, stmt_prev_allow);
                    }
                }
                _ => {
                    let lit = self.read_bare_token();
                    if is_meta && !lit.text.is_empty() {
                        self.check_literal(&lit, stmt_prev_allow);
                    }
                }
            }
        }
    }

    fn read_bare_token(&mut self) -> ExtractedLiteral {
        let start_line = self.line;
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' {
                self.push_next_char(&mut text, &mut byte_lines);
            } else {
                break;
            }
        }
        if text.is_empty() && self.pos < self.bytes.len() {
            self.advance_one();
        }
        ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start: 0, quote_end: 0 }
    }

    fn read_basic_single_string(&mut self) -> ExtractedLiteral {
        let start_line = self.line;
        self.pos += 1; // '"'
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while let Some(b) = self.peek() {
            if b == b'"' {
                self.pos += 1;
                break;
            }
            if b == b'\n' {
                break;
            }
            if b == b'\\' {
                self.read_toml_escape(&mut text, &mut byte_lines, false);
                continue;
            }
            self.push_next_char(&mut text, &mut byte_lines);
        }
        ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start: 0, quote_end: 0 }
    }

    fn read_basic_multiline_string(&mut self) -> ExtractedLiteral {
        let start_line = self.line;
        self.pos += 3; // "\"\"\""
        self.trim_opening_newline();
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while let Some(b) = self.peek() {
            if self.bytes.get(self.pos..self.pos + 3) == Some(b"\"\"\"") {
                self.consume_closing_triple(b'"', &mut text, &mut byte_lines);
                break;
            }
            if b == b'\\' {
                self.read_toml_escape(&mut text, &mut byte_lines, true);
                continue;
            }
            self.push_next_char(&mut text, &mut byte_lines);
        }
        ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start: 0, quote_end: 0 }
    }

    fn read_literal_single_string(&mut self) -> ExtractedLiteral {
        let start_line = self.line;
        self.pos += 1; // '\''
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while let Some(b) = self.peek() {
            if b == b'\'' {
                self.pos += 1;
                break;
            }
            if b == b'\n' {
                break;
            }
            self.push_next_char(&mut text, &mut byte_lines);
        }
        ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start: 0, quote_end: 0 }
    }

    fn read_literal_multiline_string(&mut self) -> ExtractedLiteral {
        let start_line = self.line;
        self.pos += 3; // "'''"
        self.trim_opening_newline();
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while self.pos < self.bytes.len() {
            if self.bytes.get(self.pos..self.pos + 3) == Some(b"'''") {
                self.consume_closing_triple(b'\'', &mut text, &mut byte_lines);
                break;
            }
            self.push_next_char(&mut text, &mut byte_lines);
        }
        ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start: 0, quote_end: 0 }
    }

    fn trim_opening_newline(&mut self) {
        if self.bytes.get(self.pos..self.pos + 2) == Some(b"\r\n") {
            self.pos += 2;
            self.line += 1;
        } else if self.peek() == Some(b'\n') {
            self.pos += 1;
            self.line += 1;
        }
    }

    fn consume_closing_triple(&mut self, quote: u8, text: &mut String, byte_lines: &mut Vec<usize>) {
        let mut run = 0usize;
        while self.bytes.get(self.pos + run) == Some(&quote) && run < 5 {
            run += 1;
        }
        let extra = run.saturating_sub(3);
        let line = self.line;
        for _ in 0..extra {
            push_char_with_line(text, byte_lines, char::from(quote), line);
        }
        self.pos += run;
    }

    fn read_toml_escape(&mut self, text: &mut String, byte_lines: &mut Vec<usize>, multiline: bool) {
        let esc_line = self.line;
        self.pos += 1; // '\\'
        let Some(next) = self.peek() else {
            return;
        };
        if multiline && is_line_ending_backslash(self.bytes, self.pos) {
            self.skip_inline_ws();
            if self.peek() == Some(b'\n') {
                self.advance_one();
            }
            self.skip_multiline_ws();
            return;
        }
        self.advance_one();
        match next {
            b'b' => push_char_with_line(text, byte_lines, '\u{0008}', esc_line),
            b't' => push_char_with_line(text, byte_lines, '\t', esc_line),
            b'n' => push_char_with_line(text, byte_lines, '\n', esc_line),
            b'f' => push_char_with_line(text, byte_lines, '\u{000C}', esc_line),
            b'r' => push_char_with_line(text, byte_lines, '\r', esc_line),
            b'"' => push_char_with_line(text, byte_lines, '"', esc_line),
            b'\\' => push_char_with_line(text, byte_lines, '\\', esc_line),
            b'u' => self.push_unicode_escape(text, byte_lines, 4, esc_line),
            b'U' => self.push_unicode_escape(text, byte_lines, 8, esc_line),
            other => push_char_with_line(text, byte_lines, char::from(other), esc_line),
        }
    }

    fn skip_multiline_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.advance_one();
        }
    }

    fn push_unicode_escape(&mut self, text: &mut String, byte_lines: &mut Vec<usize>, digits: usize, line: usize) {
        if let Some(hex) = self.src.get(self.pos..self.pos + digits)
            && let Ok(code) = u32::from_str_radix(hex, 16)
            && let Some(ch) = char::from_u32(code)
        {
            self.pos += digits;
            push_char_with_line(text, byte_lines, ch, line);
        }
    }

    fn check_literal(&mut self, lit: &ExtractedLiteral, stmt_prev_allow: bool) {
        let allowed =
            stmt_prev_allow || line_range_allows(&self.lines, lit.start_line, lit.end_line) || prev_line_allows(&self.lines, lit.start_line);
        if !allowed {
            collect_literal_words(self.path, lit, BRANDS, PrismPolicy::Standard, &mut self.seen, self.out);
        }
    }
}

fn line_range_allows(lines: &[&str], start_line: usize, end_line: usize) -> bool {
    (start_line..=end_line).any(|ln| lines.get(ln.saturating_sub(1)).is_some_and(|src| src.contains(ALLOW_MARKER)))
}

fn is_line_ending_backslash(bytes: &[u8], mut pos: usize) -> bool {
    while matches!(bytes.get(pos), Some(b' ' | b'\t' | b'\r')) {
        pos += 1;
    }
    bytes.get(pos) == Some(&b'\n')
}

fn is_metadata_table_segments(segments: &[String]) -> bool {
    match segments {
        [first] => first == "package",
        [first, second, ..] => {
            (first == "package" || first == "workspace")
                && (second == "package" && segments.len() == 2 && first == "workspace" || second == "metadata")
        }
        _ => false,
    }
}

fn is_metadata_key_segments(segments: &[String], in_workspace_section: bool) -> bool {
    let first = segments.first().map(String::as_str);
    let second = segments.get(1).map(String::as_str);
    if first == Some("package") {
        return true;
    }
    if first == Some("workspace") && matches!(second, Some("package" | "metadata")) {
        return true;
    }
    in_workspace_section && matches!(first, Some("package" | "metadata"))
}

struct ExtractedLiteral {
    text: String,
    byte_lines: Vec<usize>,
    start_line: usize,
    end_line: usize,
    quote_start: usize,
    quote_end: usize,
}

struct RustLexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
}

impl<'a> RustLexer<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, bytes: src.as_bytes(), pos: 0, line: 1 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn prev_is_ident(&self) -> bool {
        self.pos > 0 && (self.bytes[self.pos - 1].is_ascii_alphanumeric() || self.bytes[self.pos - 1] == b'_')
    }

    fn advance_one(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    fn try_skip_comment(&mut self) -> bool {
        if self.bytes.get(self.pos..self.pos + 2) == Some(b"//") {
            self.pos += 2;
            while let Some(b) = self.advance_one() {
                if b == b'\n' {
                    break;
                }
            }
            return true;
        }
        if self.bytes.get(self.pos..self.pos + 2) == Some(b"/*") {
            self.pos += 2;
            let mut depth = 1usize;
            while self.pos < self.bytes.len() && depth > 0 {
                if self.bytes.get(self.pos..self.pos + 2) == Some(b"/*") {
                    depth += 1;
                    self.pos += 2;
                } else if self.bytes.get(self.pos..self.pos + 2) == Some(b"*/") {
                    depth -= 1;
                    self.pos += 2;
                } else {
                    self.advance_one();
                }
            }
            return true;
        }
        false
    }

    fn try_skip_char_literal(&mut self) -> bool {
        let q_pos = if self.peek() == Some(b'\'') {
            self.pos
        } else if self.peek() == Some(b'b') && self.bytes.get(self.pos + 1) == Some(&b'\'') && !self.prev_is_ident() {
            self.pos + 1
        } else {
            return false;
        };
        if self.bytes.get(q_pos + 1) == Some(&b'\\') {
            let mut p = q_pos + 2;
            if p < self.bytes.len() && self.bytes[p] != b'\n' {
                p += 1;
                let limit = (p + 14).min(self.bytes.len());
                while p < limit && self.bytes[p] != b'\n' {
                    if self.bytes[p] == b'\'' {
                        self.pos = p + 1;
                        return true;
                    }
                    p += 1;
                }
            }
            return false;
        }
        if let Some(rest) = self.src.get(q_pos + 1..)
            && let Some(ch) = rest.chars().next().filter(|&c| c != '\'' && c != '\n' && c != '\r')
            && self.bytes.get(q_pos + 1 + ch.len_utf8()) == Some(&b'\'')
        {
            self.pos = q_pos + 1 + ch.len_utf8() + 1;
            return true;
        }
        false
    }

    fn try_read_raw_string(&mut self) -> Option<ExtractedLiteral> {
        if self.prev_is_ident() {
            return None;
        }
        let r_pos = if self.peek() == Some(b'r') {
            self.pos
        } else if matches!(self.peek(), Some(b'b' | b'c')) && self.bytes.get(self.pos + 1) == Some(&b'r') {
            self.pos + 1
        } else {
            return None;
        };
        let mut hashes = 0usize;
        while self.bytes.get(r_pos + 1 + hashes) == Some(&b'#') {
            hashes += 1;
        }
        let open_quote = r_pos + 1 + hashes;
        if self.bytes.get(open_quote) != Some(&b'"') {
            return None;
        }
        let quote_start = self.pos;
        let start_line = self.line;
        self.pos = open_quote + 1;
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while self.pos < self.bytes.len() {
            if self.bytes[self.pos] == b'"' && self.bytes.get(self.pos + 1..self.pos + 1 + hashes).is_some_and(|s| s.iter().all(|&b| b == b'#')) {
                self.pos += 1 + hashes;
                break;
            }
            if let Some(rest) = self.src.get(self.pos..)
                && let Some(ch) = rest.chars().next()
            {
                let line = self.line;
                push_char_with_line(&mut text, &mut byte_lines, ch, line);
                self.pos += ch.len_utf8();
                if ch == '\n' {
                    self.line += 1;
                }
            } else {
                self.advance_one();
            }
        }
        Some(ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start, quote_end: self.pos })
    }

    fn try_read_quoted_string(&mut self) -> Option<ExtractedLiteral> {
        let quote_start = self.pos;
        let start_line = self.line;
        if self.peek() == Some(b'"') {
            self.pos += 1;
        } else if matches!(self.peek(), Some(b'b' | b'c')) && self.bytes.get(self.pos + 1) == Some(&b'"') && !self.prev_is_ident() {
            self.pos += 2;
        } else {
            return None;
        }
        let mut text = String::new();
        let mut byte_lines = Vec::new();
        while let Some(b) = self.peek() {
            if b == b'"' {
                self.pos += 1;
                break;
            }
            if b == b'\\' {
                self.read_string_escape(&mut text, &mut byte_lines);
                continue;
            }
            if let Some(rest) = self.src.get(self.pos..)
                && let Some(ch) = rest.chars().next()
            {
                let line = self.line;
                push_char_with_line(&mut text, &mut byte_lines, ch, line);
                self.pos += ch.len_utf8();
                if ch == '\n' {
                    self.line += 1;
                }
            } else {
                self.advance_one();
            }
        }
        Some(ExtractedLiteral { text, byte_lines, start_line, end_line: self.line, quote_start, quote_end: self.pos })
    }

    fn read_string_escape(&mut self, text: &mut String, byte_lines: &mut Vec<usize>) {
        let esc_line = self.line;
        self.pos += 1; // '\'
        let Some(next) = self.peek() else {
            return;
        };
        if next == b'\n' || (next == b'\r' && self.bytes.get(self.pos + 1) == Some(&b'\n')) {
            if next == b'\r' {
                self.pos += 1;
            }
            self.advance_one(); // '\n'
            while let Some(ws) = self.peek() {
                if ws == b' ' || ws == b'\t' {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            return;
        }
        self.advance_one();
        let ch = match next {
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'0' => '\0',
            b'\\' => '\\',
            b'"' => '"',
            b'\'' => '\'',
            other => char::from(other),
        };
        push_char_with_line(text, byte_lines, ch, esc_line);
    }

    fn try_skip_cfg_test_item(&mut self) -> bool {
        if self.bytes.get(self.pos..self.pos + 2) != Some(b"#[") {
            return false;
        }
        let saved_pos = self.pos;
        let saved_line = self.line;
        let Some(inner) = self.read_bracket_attr() else {
            self.pos = saved_pos;
            self.line = saved_line;
            return false;
        };
        if !is_cfg_test_attr(&inner) {
            self.pos = saved_pos;
            self.line = saved_line;
            return false;
        }
        self.skip_following_attrs_and_ws();
        self.skip_annotated_item();
        true
    }

    fn read_bracket_attr(&mut self) -> Option<String> {
        self.pos += 2; // "#["
        let start = self.pos;
        let mut depth = 1usize;
        while self.pos < self.bytes.len() && depth > 0 {
            if self.try_skip_comment() || self.try_skip_char_literal() {
                continue;
            }
            if self.try_read_raw_string().is_some() || self.try_read_quoted_string().is_some() {
                continue;
            }
            match self.bytes[self.pos] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            self.advance_one();
        }
        if depth == 0 { self.src.get(start..self.pos - 1).map(str::to_owned) } else { None }
    }

    fn skip_following_attrs_and_ws(&mut self) {
        loop {
            while let Some(b) = self.peek() {
                if b.is_ascii_whitespace() {
                    self.advance_one();
                } else {
                    break;
                }
            }
            if self.try_skip_comment() {
                continue;
            }
            if self.bytes.get(self.pos..self.pos + 2) == Some(b"#[") {
                if self.read_bracket_attr().is_none() {
                    break;
                }
                continue;
            }
            break;
        }
    }

    fn skip_annotated_item(&mut self) {
        let mut brace_depth = 0usize;
        let mut saw_brace = false;
        while self.pos < self.bytes.len() {
            if self.try_skip_comment() || self.try_skip_char_literal() {
                continue;
            }
            if self.try_read_raw_string().is_some() || self.try_read_quoted_string().is_some() {
                continue;
            }
            match self.bytes[self.pos] {
                b'{' => {
                    brace_depth += 1;
                    saw_brace = true;
                    self.advance_one();
                }
                b'}' if saw_brace => {
                    brace_depth = brace_depth.saturating_sub(1);
                    self.advance_one();
                    if brace_depth == 0 {
                        return;
                    }
                }
                b';' if !saw_brace => {
                    self.advance_one();
                    return;
                }
                _ => {
                    self.advance_one();
                }
            }
        }
    }
}

fn push_char_with_line(text: &mut String, byte_lines: &mut Vec<usize>, ch: char, line: usize) {
    let before = text.len();
    text.push(ch);
    byte_lines.resize(text.len(), line);
    let _ = before;
}

fn is_cfg_test_attr(inner: &str) -> bool {
    let compact: String = inner.chars().filter(|c| !c.is_whitespace()).collect();
    if compact == "cfg(test)" {
        return true;
    }
    let Some(rest) = compact.strip_prefix("cfg(all(").and_then(|s| s.strip_suffix("))")) else {
        return false;
    };
    let mut depth = 0usize;
    let mut start = 0usize;
    for (idx, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if rest[start..idx] == *"test" {
                    return true;
                }
                start = idx + 1;
            }
            _ => {}
        }
    }
    rest[start..] == *"test"
}

fn is_ui_or_app_rust_path(path: &str) -> bool {
    path.starts_with("crates/ui-egui/src/") || path.starts_with("apps/")
}

fn scan_rust(path: &str, text: &str, out: &mut Vec<BrandHit>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut lexer = RustLexer::new(text);
    let mut seen: BTreeSet<(usize, &'static str)> = BTreeSet::new();
    let strict_ui_or_app = is_ui_or_app_rust_path(path);

    while lexer.pos < lexer.bytes.len() {
        if lexer.try_skip_comment() || lexer.try_skip_char_literal() || lexer.try_skip_cfg_test_item() {
            continue;
        }
        if let Some(lit) = lexer.try_read_raw_string().or_else(|| lexer.try_read_quoted_string()) {
            let line_allows =
                (lit.start_line..=lit.end_line).any(|ln| lines.get(ln.saturating_sub(1)).is_some_and(|src_line| src_line.contains(ALLOW_MARKER)));
            if !line_allows {
                let policy = if strict_ui_or_app {
                    PrismPolicy::UiOrAppRustLiteral {
                        allow_bare_prism: is_import_export_prism_literal(text, &lit),
                        allow_dot_prism: is_dot_prism_extension_description(&lit.text),
                    }
                } else {
                    PrismPolicy::RustLiteral
                };
                collect_literal_words(path, &lit, BRANDS, policy, &mut seen, out);
            }
            continue;
        }
        lexer.advance_one();
    }
}

#[derive(Clone, Copy)]
enum PrismPolicy<'a> {
    Strict,
    Standard,
    RustLiteral,
    UiOrAppRustLiteral { allow_bare_prism: bool, allow_dot_prism: bool },
    UiTextRun { allow_dot_prism: bool },
    SvgTitleOrDesc { prev_word: Option<&'a str> },
}

fn collect_literal_words(
    path: &str,
    lit: &ExtractedLiteral,
    words: &[&'static str],
    policy: PrismPolicy<'_>,
    seen: &mut BTreeSet<(usize, &'static str)>,
    out: &mut Vec<BrandHit>,
) {
    let lower = lit.text.to_ascii_lowercase();
    for &brand in words {
        collect_word_matches(&lit.text, &lower, brand, policy, |pos| {
            let line = lit.byte_lines.get(pos).copied().unwrap_or(lit.start_line);
            if seen.insert((line, brand)) {
                out.push(BrandHit { path: path.to_owned(), line, brand });
            }
        });
    }
}

fn collect_line_words(path: &str, line_no: usize, haystack: &str, words: &[&'static str], policy: PrismPolicy<'_>, out: &mut Vec<BrandHit>) {
    let lower = haystack.to_ascii_lowercase();
    for &brand in words {
        if has_word_match(haystack, &lower, brand, policy) {
            out.push(BrandHit { path: path.to_owned(), line: line_no, brand });
        }
    }
}

fn has_word_match(raw: &str, lower: &str, word: &'static str, policy: PrismPolicy<'_>) -> bool {
    let mut found = false;
    collect_word_matches(raw, lower, word, policy, |_| {
        found = true;
    });
    found
}

fn collect_word_matches(raw: &str, lower: &str, word: &'static str, policy: PrismPolicy<'_>, mut on_match: impl FnMut(usize)) {
    let bytes = lower.as_bytes();
    let wlen = word.len();
    let mut start = 0;
    while let Some(rel) = lower[start..].find(word) {
        let pos = start + rel;
        let end = pos + wlen;
        let left_ok = pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric();
        let right_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if left_ok && right_ok && !(word == "prism" && is_prism_match_exempt(raw, lower, pos, policy)) {
            on_match(pos);
        }
        start = pos + 1;
    }
}

fn is_prism_match_exempt(raw: &str, lower: &str, pos: usize, policy: PrismPolicy<'_>) -> bool {
    let end = pos + "prism".len();
    let is_lower_dot_ext = raw.as_bytes().get(pos..end) == Some(b"prism") && is_prism_extension_token(lower.as_bytes(), pos, end);
    match policy {
        PrismPolicy::Strict => false,
        PrismPolicy::Standard => is_lower_dot_ext,
        PrismPolicy::RustLiteral => raw == "prism" || is_lower_dot_ext,
        PrismPolicy::UiOrAppRustLiteral { allow_bare_prism, allow_dot_prism } => {
            (allow_bare_prism && raw == "prism") || (allow_dot_prism && is_lower_dot_ext)
        }
        PrismPolicy::UiTextRun { allow_dot_prism } => allow_dot_prism && is_lower_dot_ext,
        PrismPolicy::SvgTitleOrDesc { prev_word } => is_lower_dot_ext || is_preceded_by_optical_modifier(&lower[..pos], prev_word),
    }
}

fn is_dot_prism_extension_description(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed == ".prism" || trimmed == "*.prism" {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    if !lower.contains(".prism") || lower.contains("http://") || lower.contains("https://") || lower.contains("www.") {
        return false;
    }
    const FORMAT_TOKENS: &[&str] = &[
        ".pzfx",
        ".lumencraft",
        ".lctemplate",
        ".csv",
        ".tsv",
        "import",
        "export",
        "extension",
        "filter",
        "accept",
        "format",
        "archive",
        "file",
        "drop",
    ];
    FORMAT_TOKENS.iter().any(|tok| lower.contains(tok))
}

fn tail_char_slice(s: &str, max_bytes: usize) -> &str {
    let mut start = s.len().saturating_sub(max_bytes);
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

fn head_char_slice(s: &str, max_bytes: usize) -> &str {
    let mut end = s.len().min(max_bytes);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn statement_prefix_tail(before: &str, max_bytes: usize) -> &str {
    let window = tail_char_slice(before, max_bytes);
    if let Some(pos) = window.rfind([';', '{', '}']) { &window[pos + 1..] } else { window }
}

fn is_import_export_prism_literal(src: &str, lit: &ExtractedLiteral) -> bool {
    if lit.text != "prism" {
        return false;
    }
    let before = src.get(..lit.quote_start).unwrap_or("");
    let after = src.get(lit.quote_end..).unwrap_or("");
    is_prism_match_pattern(before, after) || is_prism_extension_array_item(before, after) || is_prism_format_call_arg(before)
}

fn is_prism_match_pattern(before: &str, after: &str) -> bool {
    let before_trim = before.trim_end();
    let after_trim = after.trim_start();

    if before_trim.ends_with("Some(") && after_trim.starts_with(')') {
        let post_paren = after_trim[1..].trim_start();
        if post_paren.starts_with("=>") || post_paren.starts_with('|') || post_paren.starts_with("if ") {
            return true;
        }
    }

    if before_trim.ends_with('|') || after_trim.starts_with('|') {
        let arm_after = head_char_slice(after_trim, 160);
        let stmt_before = statement_prefix_tail(before, 200);
        if arm_after.contains("=>")
            || stmt_before.contains("matches!")
            || arm_after.contains("\"pzfx\"")
            || stmt_before.contains("\"pzfx\"")
            || arm_after.contains("\"lumencraft\"")
            || stmt_before.contains("\"lumencraft\"")
        {
            return true;
        }
    }

    if before_trim.ends_with("==") || before_trim.ends_with("!=") {
        let ident_part = before_trim.trim_end_matches(['=', '!']).trim_end();
        let ident = trailing_ascii_word(ident_part).to_ascii_lowercase();
        return ident.contains("ext")
            || ident.contains("format")
            || ident.contains("fmt")
            || ident.contains("filter")
            || ident.contains("kind")
            || ident.contains("importer");
    }

    false
}

fn find_enclosing_bracket_open(before: &str) -> Option<usize> {
    let window = tail_char_slice(before, 512);
    let offset = before.len() - window.len();
    let bytes = window.as_bytes();
    let mut depth = 0usize;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b']' => depth += 1,
            b'[' => {
                if depth == 0 {
                    return Some(offset + i);
                }
                depth -= 1;
            }
            b';' | b'{' | b'}' => return None,
            _ => {}
        }
    }
    None
}

fn find_enclosing_bracket_close(after: &str) -> Option<usize> {
    let window = head_char_slice(after, 512);
    let bytes = window.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'[' => depth += 1,
            b']' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            b';' | b'{' | b'}' => return None,
            _ => {}
        }
    }
    None
}

fn is_prism_extension_array_item(before: &str, after: &str) -> bool {
    let Some(open_idx) = find_enclosing_bracket_open(before) else {
        return false;
    };
    let Some(close_idx) = find_enclosing_bracket_close(after) else {
        return false;
    };
    let inner_before = &before[open_idx..];
    let inner_after = &after[..close_idx];
    const PEER_EXT_LITERALS: &[&str] = &["\"pzfx\"", "\"lctemplate\"", "\"lumencraft\"", "\"lumencraft.d\"", "\"csv\"", "\"tsv\""];
    if PEER_EXT_LITERALS.iter().any(|peer| inner_before.contains(peer) || inner_after.contains(peer)) {
        return true;
    }
    let prefix = statement_prefix_tail(&before[..open_idx], 160).to_ascii_lowercase();
    const ARRAY_CONTEXT_MARKERS: &[&str] = &["add_filter", "extension", "file_filter", "import", "export", "accept", "format_"];
    ARRAY_CONTEXT_MARKERS.iter().any(|m| prefix.contains(m))
}

fn is_prism_format_call_arg(before: &str) -> bool {
    let stmt = statement_prefix_tail(before, 220).to_ascii_lowercase();
    let trimmed = before.trim_end();
    if let Some(paren_pos) = trimmed.rfind('(') {
        let callee = trailing_ascii_word(trimmed[..paren_pos].trim_end()).to_ascii_lowercase();
        if matches!(callee.as_str(), "label" | "title" | "text" | "button" | "tooltip" | "heading" | "window_title" | "set_title") {
            return false;
        }
    }
    if let Some(colon_pos) = trimmed.rfind(':') {
        let field = trailing_ascii_word(trimmed[..colon_pos].trim_end()).to_ascii_lowercase();
        if matches!(field.as_str(), "label" | "title" | "text" | "tooltip" | "summary" | "beginner" | "message" | "placeholder") {
            return false;
        }
    }
    const CALL_MARKERS: &[&str] = &[
        "apply_import_report",
        "load_import_report",
        "importer_by_format",
        "exporter_by_format",
        "add_filter",
        "open_prism",
        "import_prism",
        "formatdescriptor",
        "file_extensions",
        "import_extensions",
        "export_extensions",
        "format_tag",
        "format_id",
    ];
    CALL_MARKERS.iter().any(|m| stmt.contains(m))
}

fn is_prism_extension_token(bytes: &[u8], pos: usize, end: usize) -> bool {
    if pos == 0 || bytes[pos - 1] != b'.' {
        return false;
    }
    let followed_by_domain_segment = bytes.get(end) == Some(&b'.') && bytes.get(end + 1).is_some_and(u8::is_ascii_alphanumeric);
    !followed_by_domain_segment
}

fn is_preceded_by_optical_modifier(before_lower: &str, prev_word: Option<&str>) -> bool {
    let trimmed = before_lower.trim_end();
    let word = if trimmed.is_empty() { prev_word.unwrap_or("") } else { trailing_ascii_word(trimmed) };
    OPTICAL_PRISM_MODIFIERS.contains(&word)
}

fn trailing_ascii_word(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = bytes.len();
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'-') {
        start -= 1;
    }
    &s[start..]
}

fn check_single_string(path: &str, line: usize, text: &str, prism_policy: PrismPolicy<'_>, out: &mut Vec<BrandHit>) {
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    let lower = text.to_ascii_lowercase();
    for &brand in SUITE_BRANDS {
        if has_word_match(text, &lower, brand, PrismPolicy::Strict) && seen.insert(brand) {
            out.push(BrandHit { path: path.to_owned(), line, brand });
        }
    }
    for &brand in BRANDS {
        if has_word_match(text, &lower, brand, prism_policy) && seen.insert(brand) {
            out.push(BrandHit { path: path.to_owned(), line, brand });
        }
    }
}

/// Scan arbitrary UI, `LayoutTree`, `AccessibilityMeta`, or tooltip `&str` slices for external
/// brands and suite names. Allows `.prism` / `*.prism` only inside explicit import/export
/// file-type descriptions.
#[must_use]
pub fn check_text_runs<I, S>(source: &str, runs: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = Vec::new();
    for (idx, run) in runs.into_iter().enumerate() {
        let text = run.as_ref();
        let policy = PrismPolicy::UiTextRun { allow_dot_prism: is_dot_prism_extension_description(text) };
        check_single_string(source, idx + 1, text, policy, &mut out);
    }
    out
}

/// Strictly scan text runs, rejecting every external brand, suite name, and `"prism"` mention
/// (including `.prism`).
#[must_use]
pub fn check_text_runs_strict<I, S>(source: &str, runs: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = Vec::new();
    for (idx, run) in runs.into_iter().enumerate() {
        check_single_string(source, idx + 1, run.as_ref(), PrismPolicy::Strict, &mut out);
    }
    out
}

/// Scan `(label, text)` pairs (for example `(node.id, node.a11y.label)` or `(key, text)`),
/// tagging each hit with `label`.
#[must_use]
pub fn check_labeled_text_runs<'a, I>(runs: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut out = Vec::new();
    for (idx, (label, text)) in runs.into_iter().enumerate() {
        let policy = PrismPolicy::UiTextRun { allow_dot_prism: is_dot_prism_extension_description(text) };
        check_single_string(label, idx + 1, text, policy, &mut out);
    }
    out
}

fn is_i18n_format_extension_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("file_filter")
        || lower.contains("format_filter")
        || lower.contains("file_extensions")
        || lower.contains("import_extensions")
        || lower.contains("export_extensions")
}

/// Scan `(key, locale, text)` i18n catalog entries across locales.
///
/// Rejects `"prism"` (including `.prism`) everywhere outside explicit file-extension/filter keys.
#[must_use]
pub fn check_i18n_catalog_entries<'a, I>(entries: I) -> Vec<BrandHit>
where
    I: IntoIterator<Item = (&'a str, &'a str, &'a str)>,
{
    let mut out = Vec::new();
    for (key, locale, text) in entries {
        let policy = if is_i18n_format_extension_key(key) {
            PrismPolicy::UiTextRun { allow_dot_prism: is_dot_prism_extension_description(text) }
        } else {
            PrismPolicy::Strict
        };
        let path = format!("i18n:{locale}:{key}");
        check_single_string(&path, 1, text, policy, &mut out);
    }
    out
}

/// Scan all registered 16-locale `i18n::CATALOG` translations and locale display names.
#[must_use]
pub fn check_i18n_catalog() -> Vec<BrandHit> {
    let mut entries: Vec<(&str, &str, &str)> = Vec::with_capacity(i18n::CATALOG.len() * i18n::LOCALES.len() + i18n::LOCALES.len());
    for entry in i18n::CATALOG {
        for (idx, &locale) in i18n::LOCALES.iter().enumerate() {
            if let Some(&text) = entry.translations.get(idx) {
                entries.push((entry.key, locale, text));
            }
        }
    }
    for &locale in i18n::LOCALES {
        entries.push(("locale_display_name", locale, i18n::locale_display_name(locale)));
    }
    check_i18n_catalog_entries(entries)
}

fn decode_xml_or_html_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after_amp = &rest[amp + 1..];
        if let Some(semi) = after_amp.find(';')
            && semi <= 12
            && let Some(decoded) = decode_named_or_numeric_entity(&after_amp[..semi])
        {
            out.push(decoded);
            rest = &after_amp[semi + 1..];
            continue;
        }
        out.push('&');
        rest = after_amp;
    }
    out.push_str(rest);
    out
}

fn decode_named_or_numeric_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        "hellip" => Some('…'),
        "mdash" => Some('—'),
        "ndash" => Some('–'),
        _ => {
            let num = entity.strip_prefix('#')?;
            let code = if let Some(hex) = num.strip_prefix(['x', 'X']) { u32::from_str_radix(hex, 16).ok()? } else { num.parse::<u32>().ok()? };
            char::from_u32(code)
        }
    }
}

fn extract_tag_name(raw_tag_inner: &str) -> (bool, bool, &str) {
    let trimmed = raw_tag_inner.trim();
    let is_close = trimmed.starts_with('/');
    let self_closing = !is_close && trimmed.ends_with('/');
    let body = trimmed.trim_start_matches('/').trim_end_matches('/').trim();
    let raw_name = body.split(|c: char| c.is_ascii_whitespace() || c == '/').next().unwrap_or("");
    let local_name = raw_name.rsplit_once(':').map_or(raw_name, |(_, local)| local);
    (is_close, self_closing, local_name)
}

fn count_newlines(s: &str) -> usize {
    s.bytes().filter(|&b| b == b'\n').count()
}

fn scan_xml_text_fragment(source: &str, start_line: usize, raw: &str, policy: PrismPolicy<'_>, out: &mut Vec<BrandHit>) {
    let decoded = decode_xml_or_html_entities(raw);
    for (offset, line_text) in decoded.lines().enumerate() {
        check_single_string(source, start_line + offset, line_text, policy, out);
    }
}

/// Inspect rendered SVG export metadata (`<title>`, `<desc>`, `<metadata>`) and text elements
/// strictly against external brands and suite names.
#[must_use]
pub fn check_svg_metadata(source: &str, svg: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut pos = 0usize;
    let mut meta_depth = 0usize;
    let mut skip_depth = 0usize;
    let mut text_depth = 0usize;
    let mut active_start_line = 1usize;
    let mut buf = String::new();

    while pos < svg.len() {
        let rest = &svg[pos..];
        if rest.starts_with("<!--") {
            let end = rest.find("-->").map_or(svg.len(), |rel| pos + rel + 3);
            line += count_newlines(&svg[pos..end]);
            pos = end;
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            let end = rest.find("]]>").map_or(svg.len(), |rel| pos + rel + 3);
            if skip_depth == 0 && (meta_depth > 0 || text_depth > 0) {
                let inner_end = if end == svg.len() { end } else { end - 3 };
                buf.push_str(&svg[pos + 9..inner_end]);
            }
            line += count_newlines(&svg[pos..end]);
            pos = end;
            continue;
        }
        if rest.starts_with('<')
            && let Some(gt_rel) = rest.find('>')
        {
            let tag_end = pos + gt_rel + 1;
            let raw_inner = &svg[pos + 1..tag_end - 1];
            let (is_close, self_closing, name) = extract_tag_name(raw_inner);
            let lower = name.to_ascii_lowercase();
            let is_meta_tag = matches!(lower.as_str(), "title" | "desc" | "metadata");
            let is_text_tag = matches!(lower.as_str(), "text" | "tspan");
            let is_skip_tag = matches!(lower.as_str(), "style" | "image");

            if is_close {
                if is_skip_tag {
                    skip_depth = skip_depth.saturating_sub(1);
                } else if is_meta_tag {
                    meta_depth = meta_depth.saturating_sub(1);
                    if meta_depth == 0 && text_depth == 0 && !buf.trim().is_empty() {
                        scan_xml_text_fragment(source, active_start_line, &buf, PrismPolicy::Strict, &mut out);
                        buf.clear();
                    }
                } else if is_text_tag {
                    text_depth = text_depth.saturating_sub(1);
                    if meta_depth == 0 && text_depth == 0 && !buf.trim().is_empty() {
                        scan_xml_text_fragment(source, active_start_line, &buf, PrismPolicy::Strict, &mut out);
                        buf.clear();
                    }
                }
            } else if !self_closing {
                if is_skip_tag {
                    skip_depth += 1;
                } else if is_meta_tag {
                    if meta_depth == 0 && text_depth == 0 {
                        active_start_line = line;
                        buf.clear();
                    }
                    meta_depth += 1;
                } else if is_text_tag {
                    if meta_depth == 0 && text_depth == 0 {
                        active_start_line = line;
                        buf.clear();
                    }
                    text_depth += 1;
                }
            }
            line += count_newlines(&svg[pos..tag_end]);
            pos = tag_end;
            continue;
        }
        let next_lt = rest.find('<').map_or(svg.len(), |rel| pos + rel.max(1));
        if skip_depth == 0 && (meta_depth > 0 || text_depth > 0) {
            if buf.is_empty() {
                active_start_line = line;
            }
            buf.push_str(&svg[pos..next_lt]);
        }
        line += count_newlines(&svg[pos..next_lt]);
        pos = next_lt;
    }

    if !buf.trim().is_empty() {
        scan_xml_text_fragment(source, active_start_line, &buf, PrismPolicy::Strict, &mut out);
    }
    out
}

fn is_pdf_stream_start(bytes: &[u8], pos: usize) -> Option<usize> {
    if !bytes.get(pos..pos + 6).is_some_and(|s| s == b"stream") {
        return None;
    }
    if pos > 0 && !bytes[pos - 1].is_ascii_whitespace() && bytes[pos - 1] != b'>' {
        return None;
    }
    if bytes.get(pos + 6..pos + 8) == Some(b"\r\n") {
        Some(pos + 8)
    } else if matches!(bytes.get(pos + 6), Some(b'\n' | b'\r')) {
        Some(pos + 7)
    } else {
        None
    }
}

fn find_pdf_endstream(bytes: &[u8], mut pos: usize, line: &mut usize) -> usize {
    while pos < bytes.len() {
        if bytes.get(pos..pos + 9) == Some(b"endstream") {
            return pos + 9;
        }
        if bytes[pos] == b'\n' {
            *line += 1;
        }
        pos += 1;
    }
    bytes.len()
}

fn decode_pdf_bytes_to_string(raw: &[u8]) -> String {
    if let Some(rest) = raw.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16(rest, false);
    }
    String::from_utf8_lossy(raw).into_owned()
}

fn decode_pdf_literal_string(bytes: &[u8], pos: &mut usize, line: &mut usize) -> String {
    *pos += 1; // '('
    let mut depth = 1usize;
    let mut raw = Vec::new();
    while *pos < bytes.len() && depth > 0 {
        let b = bytes[*pos];
        *pos += 1;
        match b {
            b'(' => {
                depth += 1;
                raw.push(b'(');
            }
            b')' => {
                depth -= 1;
                if depth > 0 {
                    raw.push(b')');
                }
            }
            b'\\' => decode_pdf_escape(bytes, pos, line, &mut raw),
            b'\n' => {
                *line += 1;
                raw.push(b'\n');
            }
            other => raw.push(other),
        }
    }
    decode_pdf_bytes_to_string(&raw)
}

fn decode_pdf_escape(bytes: &[u8], pos: &mut usize, line: &mut usize, raw: &mut Vec<u8>) {
    let Some(&esc) = bytes.get(*pos) else {
        return;
    };
    *pos += 1;
    match esc {
        b'n' => raw.push(b'\n'),
        b'r' => raw.push(b'\r'),
        b't' => raw.push(b'\t'),
        b'b' => raw.push(0x08),
        b'f' => raw.push(0x0C),
        b'(' | b')' | b'\\' => raw.push(esc),
        b'\r' => {
            if bytes.get(*pos) == Some(&b'\n') {
                *pos += 1;
            }
            *line += 1;
        }
        b'\n' => {
            *line += 1;
        }
        b'0'..=b'7' => {
            let mut val = u16::from(esc - b'0');
            for _ in 0..2 {
                if let Some(&d) = bytes.get(*pos).filter(|&&c| (b'0'..=b'7').contains(&c)) {
                    val = val * 8 + u16::from(d - b'0');
                    *pos += 1;
                } else {
                    break;
                }
            }
            raw.push((val & 0xFF) as u8);
        }
        other => raw.push(other),
    }
}

fn decode_pdf_hex_string(bytes: &[u8], pos: &mut usize, line: &mut usize) -> String {
    *pos += 1; // '<'
    let mut nibbles = Vec::new();
    while *pos < bytes.len() {
        let b = bytes[*pos];
        *pos += 1;
        if b == b'>' {
            break;
        }
        if b == b'\n' {
            *line += 1;
        }
        if let Some(n) = (b as char).to_digit(16)
            && let Ok(d) = u8::try_from(n)
        {
            nibbles.push(d);
        }
    }
    if nibbles.len() % 2 == 1 {
        nibbles.push(0);
    }
    let mut raw = Vec::with_capacity(nibbles.len() / 2);
    for pair in nibbles.as_chunks::<2>().0 {
        raw.push((pair[0] << 4) | pair[1]);
    }
    decode_pdf_bytes_to_string(&raw)
}

/// Inspect PDF document metadata (`/Creator`, `/Producer`, `/Title`, `/Author`, `/Subject`,
/// `/Keywords`), decoding both literal PDF strings and UTF-16BE hex strings.
#[must_use]
pub fn check_pdf_metadata(source: &str, pdf_bytes: &[u8]) -> Vec<BrandHit> {
    const PDF_INFO_KEYS: &[&str] = &["Creator", "Producer", "Title", "Author", "Subject", "Keywords"];
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut pos = 0usize;

    while pos < pdf_bytes.len() {
        if let Some(stream_start) = is_pdf_stream_start(pdf_bytes, pos) {
            line += pdf_bytes[pos..stream_start].iter().filter(|&&b| b == b'\n').count();
            pos = find_pdf_endstream(pdf_bytes, stream_start, &mut line);
            continue;
        }
        let b = pdf_bytes[pos];
        if b == b'\n' {
            line += 1;
            pos += 1;
            continue;
        }
        if b == b'/' {
            pos += 1;
            let key_start = pos;
            while pos < pdf_bytes.len() && pdf_bytes[pos].is_ascii_alphanumeric() {
                pos += 1;
            }
            let key = String::from_utf8_lossy(&pdf_bytes[key_start..pos]);
            if PDF_INFO_KEYS.contains(&key.as_ref()) {
                while pos < pdf_bytes.len() && matches!(pdf_bytes[pos], b' ' | b'\t' | b'\r' | b'\n' | 0 | 0x0C) {
                    if pdf_bytes[pos] == b'\n' {
                        line += 1;
                    }
                    pos += 1;
                }
                let val_line = line;
                if pdf_bytes.get(pos) == Some(&b'(') {
                    let decoded = decode_pdf_literal_string(pdf_bytes, &mut pos, &mut line);
                    check_single_string(source, val_line, &decoded, PrismPolicy::Strict, &mut out);
                } else if pdf_bytes.get(pos) == Some(&b'<') && pdf_bytes.get(pos + 1) != Some(&b'<') {
                    let decoded = decode_pdf_hex_string(pdf_bytes, &mut pos, &mut line);
                    check_single_string(source, val_line, &decoded, PrismPolicy::Strict, &mut out);
                }
            }
            continue;
        }
        pos += 1;
    }
    out
}

fn extract_html_attr(raw_tag_inner: &str, attr_name: &str) -> Option<String> {
    let lower = raw_tag_inner.to_ascii_lowercase();
    let mut search = 0usize;
    while let Some(rel) = lower[search..].find(attr_name) {
        let pos = search + rel;
        let end = pos + attr_name.len();
        let left_ok = pos == 0 || lower.as_bytes()[pos - 1].is_ascii_whitespace();
        if !left_ok {
            search = end;
            continue;
        }
        let after = raw_tag_inner.get(end..)?.trim_start();
        let Some(after_eq) = after.strip_prefix('=') else {
            search = end;
            continue;
        };
        let val_src = after_eq.trim_start();
        if let Some(q) = val_src.chars().next().filter(|&c| c == '"' || c == '\'') {
            let inner = &val_src[1..];
            if let Some(close) = inner.find(q) {
                return Some(decode_xml_or_html_entities(&inner[..close]));
            }
        }
        search = end;
    }
    None
}

/// Inspect HTML metadata (`<title>`, `<meta content=...>`, `aria-label`/`alt`/`title` attributes,
/// embedded `<svg>` metadata, and visible DOM text) for external brand or suite mentions.
#[must_use]
pub fn check_html_metadata(source: &str, html: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut pos = 0usize;
    let mut strict_depth = 0usize;
    let mut in_script_or_style = false;
    let mut text_start_line = 1usize;
    let mut buf = String::new();

    while pos < html.len() {
        let rest = &html[pos..];
        if rest.starts_with("<!--") {
            let end = rest.find("-->").map_or(html.len(), |rel| pos + rel + 3);
            line += count_newlines(&html[pos..end]);
            pos = end;
            continue;
        }
        if rest.starts_with('<')
            && let Some(gt_rel) = rest.find('>')
        {
            if !buf.trim().is_empty() && !in_script_or_style {
                flush_html_text(source, text_start_line, &buf, strict_depth > 0, &mut out);
            }
            buf.clear();
            let tag_end = pos + gt_rel + 1;
            let raw_inner = &html[pos + 1..tag_end - 1];
            let (is_close, self_closing, name) = extract_tag_name(raw_inner);
            let lower = name.to_ascii_lowercase();
            let tag_line = line;

            if matches!(lower.as_str(), "script" | "style") {
                if is_close {
                    in_script_or_style = false;
                } else if !self_closing {
                    in_script_or_style = true;
                }
            } else if !in_script_or_style {
                if !is_close {
                    if lower == "meta"
                        && let Some(content) = extract_html_attr(raw_inner, "content")
                    {
                        check_single_string(source, tag_line, &content, PrismPolicy::Strict, &mut out);
                    }
                    for attr in ["aria-label", "alt", "title"] {
                        if let Some(val) = extract_html_attr(raw_inner, attr) {
                            check_single_string(source, tag_line, &val, PrismPolicy::Strict, &mut out);
                        }
                    }
                }
                if matches!(lower.as_str(), "title" | "desc" | "metadata") {
                    if is_close {
                        strict_depth = strict_depth.saturating_sub(1);
                    } else if !self_closing {
                        strict_depth += 1;
                    }
                }
            }
            line += count_newlines(&html[pos..tag_end]);
            pos = tag_end;
            continue;
        }
        let next_lt = rest.find('<').map_or(html.len(), |rel| pos + rel.max(1));
        if !in_script_or_style {
            if buf.is_empty() {
                text_start_line = line;
            }
            buf.push_str(&html[pos..next_lt]);
        }
        line += count_newlines(&html[pos..next_lt]);
        pos = next_lt;
    }

    if !buf.trim().is_empty() && !in_script_or_style {
        flush_html_text(source, text_start_line, &buf, strict_depth > 0, &mut out);
    }
    out
}

fn flush_html_text(source: &str, start_line: usize, raw: &str, strict: bool, out: &mut Vec<BrandHit>) {
    let decoded = decode_xml_or_html_entities(raw);
    for (offset, line_text) in decoded.lines().enumerate() {
        let policy =
            if strict { PrismPolicy::Strict } else { PrismPolicy::UiTextRun { allow_dot_prism: is_dot_prism_extension_description(line_text) } };
        check_single_string(source, start_line + offset, line_text, policy, out);
    }
}

/// Inspect `.lumencraft`/`.lctemplate` `manifest.json` keys and string values strictly.
#[must_use]
pub fn check_manifest_metadata(source: &str, manifest_json: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    match serde_json::from_str::<serde_json::Value>(manifest_json) {
        Ok(val) => visit_json_strings(source, &val, 1, &mut out),
        Err(_) => {
            for (idx, line) in manifest_json.lines().enumerate() {
                check_single_string(source, idx + 1, line, PrismPolicy::Strict, &mut out);
            }
        }
    }
    out
}

fn visit_json_strings(source: &str, val: &serde_json::Value, idx_hint: usize, out: &mut Vec<BrandHit>) {
    match val {
        serde_json::Value::String(s) => {
            check_single_string(source, idx_hint, s, PrismPolicy::Strict, out);
        }
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                visit_json_strings(source, item, i + 1, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (i, (k, v)) in map.iter().enumerate() {
                check_single_string(source, i + 1, k, PrismPolicy::Strict, out);
                visit_json_strings(source, v, i + 1, out);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

/// Inspect a desktop or browser window title string strictly.
#[must_use]
pub fn check_window_title(source: &str, title: &str) -> Vec<BrandHit> {
    let mut out = Vec::new();
    check_single_string(source, 1, title, PrismPolicy::Strict, &mut out);
    out
}

fn collect_snapshot_strings(snap: &UiTreeSnapshot, context: &str, hits: &mut Vec<BrandHit>) {
    let mut runs: Vec<(String, String)> = Vec::new();
    if let Some(banner) = &snap.status_banner {
        runs.push((format!("{context}:status_banner"), banner.clone()));
    }
    if let Some(target) = &snap.context_menu_target {
        runs.push((format!("{context}:context_menu_target"), target.clone()));
    }
    if let Some(diag) = &snap.result_diagnostic_banner {
        runs.push((format!("{context}:diag.summary"), diag.summary.clone()));
        runs.push((format!("{context}:diag.limitation"), diag.limitation_note.clone()));
        if let Some(metric) = &diag.diagnostic_metric {
            runs.push((format!("{context}:diag.metric"), metric.clone()));
        }
    }
    if let Some(menus) = &snap.menus {
        for m in menus {
            runs.push((format!("{context}:menu:{}", m.menu_key), m.title.clone()));
            for n in &m.items {
                push_widget_node_runs(context, n, &mut runs);
            }
        }
    }
    for (group, nodes) in [
        ("toolbar", snap.toolbar.as_deref()),
        ("tabs", snap.tabs.as_deref()),
        ("canvas", snap.canvas_elements.as_deref()),
        ("grid", snap.grid_cells.as_deref()),
    ] {
        if let Some(slice) = nodes {
            for n in slice {
                push_widget_node_runs(&format!("{context}:{group}"), n, &mut runs);
            }
        }
    }
    if let Some(nav) = &snap.navigator_sheets {
        for entry in nav {
            let nav_ctx = format!("{context}:nav:{}", entry.sheet_id);
            hits.extend(check_text_runs_strict(&nav_ctx, [&entry.title, &entry.label]));
        }
    }
    if let Some(dlg) = &snap.dialog {
        runs.push((format!("{context}:dialog:{}:title", dlg.dialog_kind), dlg.title.clone()));
        if let Some(stage) = &dlg.stage {
            runs.push((format!("{context}:dialog:{}:stage", dlg.dialog_kind), stage.clone()));
        }
        for n in dlg.tabs.iter().chain(&dlg.controls).chain(&dlg.buttons) {
            push_widget_node_runs(&format!("{context}:dialog:{}", dlg.dialog_kind), n, &mut runs);
        }
    }
    for (label, text) in &runs {
        let policy = PrismPolicy::UiTextRun { allow_dot_prism: is_dot_prism_extension_description(text) };
        check_single_string(label, 1, text, policy, hits);
    }
}

fn push_widget_node_runs(prefix: &str, node: &lumencraft_ui_egui::inspect::UiWidgetNode, out: &mut Vec<(String, String)>) {
    out.push((format!("{prefix}:{}", node.widget_id), node.label.clone()));
    if let Some(reason) = &node.disabled_reason {
        out.push((format!("{prefix}:{}:disabled_reason", node.widget_id), reason.clone()));
    }
    if let Some(serde_json::Value::String(v)) = &node.value {
        out.push((format!("{prefix}:{}:value", node.widget_id), v.clone()));
    }
}

fn build_brands_sweep_document() -> Result<ProjectDocument, String> {
    let mut doc = ProjectDocument::new("LumenCraft Brand Sweep").map_err(|e| format!("project init: {e}"))?;
    for (idx, fixture) in SAMPLE_FIXTURES.iter().enumerate() {
        let id = format!("data-{}", idx + 1);
        let ds = instantiate_sample_fixture(fixture.id, id).map_err(|e| format!("fixture {}: {e}", fixture.id))?;
        doc.add_data_sheet(ds).map_err(|e| format!("add fixture {}: {e}", fixture.id))?;
    }
    let mut info = InfoSheet::new_preseeded("info-1", "Protocol Constants").map_err(|e| format!("info sheet: {e}"))?;
    info.constants
        .push(lumencraft_doc::InfoConstant::new("ControlEC50", ConstantValue::Number(12.5), Some(String::from("nM"))).map_err(|e| e.to_string())?);
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    let spec =
        AnalysisSpec::new("res-1", "data-2", AnalysisKind::OneWayAnova(lumencraft_doc::OneWayAnovaSpec::default())).map_err(|e| e.to_string())?;
    let res = ResultSheet::new(spec).map_err(|e| e.to_string())?;
    doc.add_result_sheet(res).map_err(|e| e.to_string())?;

    if let Some(ds_xy) = doc.data_sheet(&SheetId::new("data-1")).cloned() {
        let mut g1 = GraphSheet::new_uninitialized_for_data("graph-1", &ds_xy).map_err(|e| e.to_string())?;
        g1.initialized = true;
        g1.title = String::from("Dose-Response Inhibition");
        doc.add_graph_sheet(g1).map_err(|e| e.to_string())?;

        let mut g2 = GraphSheet::new_uninitialized_for_data("graph-2", &ds_xy).map_err(|e| e.to_string())?;
        g2.initialized = true;
        g2.title = String::from("Secondary Curve Panel");
        doc.add_graph_sheet(g2).map_err(|e| e.to_string())?;
    }

    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-a"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: lumencraft_doc::RectPt::new(18.0, 18.0, 220.0, 160.0),
        scale_percent: 100.0,
        badge_override: Some(String::from("A")),
    });
    let layout = LayoutSheet::new("layout-1", "Publication Panel Layout", layout_spec).map_err(|e| e.to_string())?;
    doc.add_layout_sheet(layout).map_err(|e| e.to_string())?;
    Ok(doc)
}

fn check_tooltips_and_catalogs(app: &mut UiApp, hits: &mut Vec<BrandHit>) {
    for &btn in TOOLBAR_BUTTONS {
        hits.extend(check_labeled_text_runs([("TOOLBAR_BUTTONS.id", btn.id), ("TOOLBAR_BUTTONS.label_key", btn.label_key)]));
    }
    for &locale in i18n::LOCALES {
        app.locale = locale.to_owned();
        for section in menu_bar_tooltips(app) {
            let ctx = format!("menu:{locale}:{}", section.menu_key);
            hits.extend(check_text_runs(&ctx, [section.title]));
            for tip in section.items {
                let p_label = format!("{ctx}:{}:label", tip.command_id);
                let p_loc = format!("{ctx}:{}:localized", tip.command_id);
                let p_sum = format!("{ctx}:{}:summary", tip.command_id);
                let p_beg = format!("{ctx}:{}:beginner", tip.command_id);
                let p_path = format!("{ctx}:{}:menu_path", tip.command_id);
                hits.extend(check_labeled_text_runs([
                    (p_label.as_str(), tip.label),
                    (p_loc.as_str(), tip.localized_label),
                    (p_sum.as_str(), tip.summary),
                    (p_beg.as_str(), tip.beginner),
                    (p_path.as_str(), tip.menu_path),
                ]));
            }
        }
        for tb in toolbar_tooltips(app) {
            let ctx = format!("toolbar:{locale}:{}", tb.button.id);
            hits.extend(check_text_runs(&ctx, [tb.caption, tb.tooltip.localized_label, tb.tooltip.summary, tb.tooltip.beginner]));
        }
    }
    app.locale = String::from("en");

    for cat in AnalysisCategory::ALL {
        hits.extend(check_text_runs("AnalysisCategory", [cat.label()]));
    }
    for item in ANALYSIS_CATALOG {
        hits.extend(check_labeled_text_runs([
            ("ANALYSIS_CATALOG.label", item.label),
            ("ANALYSIS_CATALOG.summary", item.summary),
            ("ANALYSIS_CATALOG.keywords", item.keywords),
        ]));
    }
    for kind in TableKind::ALL {
        let draft = TableCreationDraft::for_kind(kind);
        hits.extend(check_text_runs("TableKind", [kind.label(), draft.title.as_str()]));
    }
    for fixture in SAMPLE_FIXTURES {
        hits.extend(check_labeled_text_runs([("SAMPLE_FIXTURES.title", fixture.title), ("SAMPLE_FIXTURES.summary", fixture.summary)]));
    }
    let palette = CommandPaletteState::default();
    for entry in palette.matches_with_shortcuts(Some(app.session()), &app.shortcuts) {
        hits.extend(check_labeled_text_runs([
            ("CommandPalette.label", entry.label),
            ("CommandPalette.menu_path", entry.menu_path),
            ("CommandPalette.summary", entry.summary),
            ("CommandPalette.beginner", entry.beginner),
        ]));
    }
}

fn check_all_dialogs_and_inspectors(app: &mut UiApp, hits: &mut Vec<BrandHit>) {
    let data_id = SheetId::new("data-1");
    let res_id = SheetId::new("res-1");
    let graph_id = SheetId::new("graph-1");
    let layout_id = SheetId::new("layout-1");

    for mode in [WelcomeMode::EmptyTypedTable, WelcomeMode::TutorialSample, WelcomeMode::FromTemplate, WelcomeMode::ClonePipeline] {
        app.nav.welcome.open = true;
        app.nav.welcome.mode = mode;
        let snap = inspect_ui_tree(app, None, true);
        collect_snapshot_strings(&snap, &format!("welcome:{mode:?}"), hits);
    }
    app.nav.welcome.open = false;

    for inspector in [
        ActiveInspector::None,
        ActiveInspector::FormatGraph,
        ActiveInspector::FormatAxes,
        ActiveInspector::PairwiseComparisons,
        ActiveInspector::BubblePlot,
        ActiveInspector::PageLayout,
        ActiveInspector::ResultDetails,
    ] {
        app.canvas_dialogs.active_inspector = inspector;
        hits.extend(check_text_runs("ActiveInspector", [inspector.label()]));
        let snap = inspect_ui_tree(app, None, true);
        collect_snapshot_strings(&snap, &format!("inspector:{inspector:?}"), hits);
    }

    let dialogs = [
        ActiveDialog::FormatAxes { sheet_id: graph_id.clone() },
        ActiveDialog::AnalyzeSheet { source_sheet_id: data_id.clone() },
        ActiveDialog::DuplicateAnalysisPrompt {
            source_sheet_id: data_id.clone(),
            existing_result_ids: vec![res_id.clone()],
            pending_entry_id: String::from("stats.one_way_anova"),
        },
        ActiveDialog::EditAnalysisParameters { result_sheet_id: res_id.clone() },
        ActiveDialog::ChangeDataAnalyzed { result_sheet_id: res_id.clone() },
        ActiveDialog::ResultViewer { result_sheet_id: res_id.clone() },
        ActiveDialog::ConstantHookPicker { target_sheet_id: res_id.clone() },
        ActiveDialog::ChangeGraphType { graph_sheet_id: graph_id.clone() },
        ActiveDialog::FormatGraph { graph_sheet_id: graph_id.clone() },
        ActiveDialog::PairwiseComparisons { graph_sheet_id: graph_id.clone() },
        ActiveDialog::BubblePlotInspector { graph_sheet_id: graph_id.clone() },
        ActiveDialog::ApplyStyleFrom { target_graph_id: graph_id.clone() },
        ActiveDialog::PageLayoutComposer { layout_sheet_id: layout_id.clone() },
        ActiveDialog::ExportFigure { sheet_id: graph_id.clone() },
        ActiveDialog::ConvertArchetype { sheet_id: data_id.clone() },
        ActiveDialog::ImportReshape,
        ActiveDialog::ExcludeWithReason { sheet_id: data_id.clone() },
        ActiveDialog::FormatSelectedPoints { graph_sheet_id: graph_id.clone() },
        ActiveDialog::PowerAndSampleSize,
        ActiveDialog::Print { sheet_id: Some(layout_id.clone()) },
        ActiveDialog::RandomizeGroups,
        ActiveDialog::RandomSampleRows { source_sheet_id: Some(data_id.clone()) },
        ActiveDialog::SimulateDistributions,
        ActiveDialog::InsertModelEquation { graph_sheet_id: graph_id.clone() },
        ActiveDialog::CommandPalette,
        ActiveDialog::About,
    ];
    for dlg in dialogs {
        app.canvas_dialogs.active_dialog = dlg.clone();
        let snap = inspect_ui_tree(app, None, true);
        collect_snapshot_strings(&snap, "dialog", hits);
    }
    app.canvas_dialogs.active_dialog = ActiveDialog::None;
}

fn check_export_metadata_sweep(doc: &ProjectDocument, hits: &mut Vec<BrandHit>) -> Result<(), String> {
    let graph = doc.graph_sheet(&SheetId::new("graph-1")).ok_or_else(|| String::from("sweep graph-1 missing"))?;
    let mut scene = compile_graph(graph, doc).map_err(|e| format!("compile sweep graph: {e}"))?;
    scene.title = String::from("Dose-Response Inhibition");

    let svg = render_svg(&scene, &SvgExportOptions::default()).map_err(|e| format!("sweep render_svg: {e}"))?;
    hits.extend(check_svg_metadata("export:render_svg", &svg));

    let pdf_ascii = render_pdf(&scene, &PdfExportOptions::default()).map_err(|e| format!("sweep render_pdf: {e}"))?;
    hits.extend(check_pdf_metadata("export:render_pdf", &pdf_ascii));

    let mut unicode_scene = scene.clone();
    unicode_scene.title = String::from("Étude Dose–Réponse — αβγ");
    let pdf_unicode = render_pdf(&unicode_scene, &PdfExportOptions::default()).map_err(|e| format!("sweep unicode render_pdf: {e}"))?;
    hits.extend(check_pdf_metadata("export:render_pdf:unicode", &pdf_unicode));

    let bundle = ClipboardBundle::from_scene(&scene, 144)?;
    hits.extend(check_svg_metadata("clipboard:svg", &bundle.svg));
    hits.extend(check_pdf_metadata("clipboard:pdf", &bundle.pdf_bytes));
    hits.extend(check_html_metadata("clipboard:html", &bundle.html));

    for (label, bytes_res) in
        [("archive:project:manifest.json", save_project_bytes(doc)), ("archive:template:manifest.json", save_template_bytes(doc))]
    {
        let bytes = bytes_res.map_err(|e| format!("{label}: {e}"))?;
        let zip = ZipArchive::from_bytes_with_limits(&bytes, ZipLimits::default()).map_err(|e| format!("{label} zip: {e}"))?;
        let manifest = zip.get_utf8("manifest.json").map_err(|e| format!("{label}: {e}"))?.ok_or_else(|| format!("{label} missing manifest.json"))?;
        hits.extend(check_manifest_metadata(label, manifest));
    }

    hits.extend(check_window_title("desktop:window_title:untitled", "Untitled — LumenCraft"));
    hits.extend(check_window_title("desktop:window_title:dirty", "study.lumencraft * — LumenCraft"));
    Ok(())
}

/// Sweep in-process UI strings across 16-locale `i18n`, tooltips, all 8 table archetypes,
/// all 26 `ActiveDialog` states, all 7 `ActiveInspector` tabs, `CommandPalette`, and
/// SVG/PDF/HTML/manifest/window export metadata.
pub fn check_ui_surfaces() -> Result<Vec<BrandHit>, String> {
    let mut hits = check_i18n_catalog();

    let mut empty_app = UiApp::new();
    let empty_snap = inspect_ui_tree(&empty_app, None, true);
    collect_snapshot_strings(&empty_snap, "empty_ui", &mut hits);
    check_tooltips_and_catalogs(&mut empty_app, &mut hits);

    let doc = build_brands_sweep_document()?;
    check_export_metadata_sweep(&doc, &mut hits)?;

    let mut app = UiApp::from_session(Session::from_document(doc));
    check_all_dialogs_and_inspectors(&mut app, &mut hits);

    Ok(hits)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        BrandHit, check_html_metadata, check_i18n_catalog, check_i18n_catalog_entries, check_labeled_text_runs, check_manifest_metadata,
        check_pdf_metadata, check_svg_metadata, check_text_runs, check_text_runs_strict, check_ui_surfaces, check_window_title, run, scan_file,
        scan_repo_checked,
    };

    fn temp_dir(label: &str) -> PathBuf {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("lc-brands-{}-{label}-{stamp}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn to_utf16_bytes(s: &str, little_endian: bool, include_bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if include_bom {
            out.extend_from_slice(if little_endian { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        for unit in s.encode_utf16() {
            let bytes = if little_endian { unit.to_le_bytes() } else { unit.to_be_bytes() };
            out.extend_from_slice(&bytes);
        }
        out
    }

    #[test]
    fn flags_rust_string_literals_but_ignores_comments_and_cfg_test() {
        let mut hits = Vec::new();
        let src = "\
// GraphPad comment is fine
pub const LABEL: &str = \"Import GraphPad table\";
pub const OK: &str = \"Import OriginPro table\"; // brand-ok
pub fn instantiate() -> &'static str { \"instantiate\" }
#[cfg(test)]
mod tests {
    const T: &str = \"SPSS fixture\";
}
";
        scan_file("crates/ui-egui/src/lib.rs", src, &mut hits);
        assert_eq!(hits, vec![BrandHit { path: String::from("crates/ui-egui/src/lib.rs"), line: 2, brand: "graphpad" }]);
    }

    #[test]
    fn cfg_test_helper_does_not_hide_later_production_code() {
        let mut hits = Vec::new();
        let src = "\
#[cfg(test)]
fn test_only_helper() {
    let _ = \"SPSS fixture }; // brace inside string\";
}

pub fn shipped_error() -> &'static str {
    let q = '\"';
    let _ = q;
    \"GraphPad-compatible: unknown option\"
}

#[cfg(all(test, unix))]
fn unix_test_helper() {
    let _ = \"Minitab\";
}

#[cfg(not(test))]
pub const PROD: &str = \"Dotmatics\";
";
        scan_file("apps/lumencraft/src/main.rs", src, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("apps/lumencraft/src/main.rs"), line: 9, brand: "graphpad" },
                BrandHit { path: String::from("apps/lumencraft/src/main.rs"), line: 18, brand: "dotmatics" },
            ]
        );
    }

    #[test]
    fn flags_multiline_string_literals_continuations_and_raw_strings() {
        let mut hits = Vec::new();
        let src = "\
pub const CONT: &str = \"Import from \\
    GraphPad table\";
pub const SPLIT: &str = \"Import Graph\\
Pad table\";
pub const ESC_NL: &str = \"line1\\nOriginPro\";
pub const RAW: &str = r#\"hello
Minitab\"#;
pub const ALLOWED: &str = \"Import from \\
    SigmaPlot table\"; // brand-ok
";
        scan_file("crates/format/src/lib.rs", src, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 2, brand: "graphpad" },
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 3, brand: "graphpad" },
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 5, brand: "originpro" },
                BrandHit { path: String::from("crates/format/src/lib.rs"), line: 7, brand: "minitab" },
            ]
        );
    }

    #[test]
    fn decodes_utf16_files_skips_binary_assets_and_fails_on_non_utf8_text() {
        let dir = temp_dir("encoding");
        std::fs::create_dir_all(dir.join("packaging/windows")).unwrap();
        std::fs::create_dir_all(dir.join("packaging/linux")).unwrap();
        std::fs::create_dir_all(dir.join("assets")).unwrap();

        std::fs::write(dir.join("packaging/windows/app.rc"), to_utf16_bytes("VALUE \"Product\", \"GraphPad\"\n", true, true)).unwrap();
        std::fs::write(dir.join("packaging/windows/clean.rc"), to_utf16_bytes("VALUE \"Product\", \"LumenCraft\"\n", false, true)).unwrap();
        std::fs::write(dir.join("assets/icon.png"), [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0xFF]).unwrap();

        let files = vec![String::from("packaging/windows/app.rc"), String::from("packaging/windows/clean.rc"), String::from("assets/icon.png")];
        let hits = scan_repo_checked(&dir, &files).unwrap();
        assert_eq!(hits, vec![BrandHit { path: String::from("packaging/windows/app.rc"), line: 1, brand: "graphpad" }]);

        // Non-UTF-8 Latin-1 byte in a packaging file must fail rather than be silently skipped.
        std::fs::write(dir.join("packaging/linux/latin1.desktop"), b"[Desktop Entry]\nComment=\xa9 LumenCraft\n").unwrap();
        let err = scan_repo_checked(&dir, &[String::from("packaging/linux/latin1.desktop")]).unwrap_err();
        assert!(err.contains("cannot scan packaging/linux/latin1.desktop"), "{err}");

        // Verify git-backed run also rejects non-UTF-8 text cleanly if tested directly via scan_repo_checked.
        let _ = run;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn desktop_entry_honors_preceding_comment_allow_marker() {
        let mut hits = Vec::new();
        let desktop = "\
[Desktop Entry]
Name=LumenCraft
# Compatibility search term: brand-ok
Keywords=biostatistics;regression;GraphPad;
";
        scan_file("packaging/linux/org.lumencraft.LumenCraft.desktop", desktop, &mut hits);
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn flags_artcraft_suite_references_in_docs_and_workflows() {
        let mut hits = Vec::new();
        scan_file("docs/releasing.md", "Installs under Programs\\ArtCraft\\LumenCraft or https://getartcraft.com\n", &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("docs/releasing.md"), line: 1, brand: "artcraft" },
                BrandHit { path: String::from("docs/releasing.md"), line: 1, brand: "getartcraft" },
            ]
        );
    }

    #[test]
    fn readme_disclaimer_line_with_marker_may_name_the_suite() {
        let mut hits = Vec::new();
        scan_file("README.md", "Not affiliated with ArtCraft. <!-- brand-ok -->\n", &mut hits);
        assert!(hits.is_empty(), "{hits:?}");
    }

    #[test]
    fn suite_disclaimer_exemption_is_limited_to_marked_readme_lines_and_the_suite_name() {
        let mut hits = Vec::new();
        // No marker: still flagged in the README.
        scan_file("README.md", "Part of ArtCraft.\n", &mut hits);
        // Marker in any other file: still flagged.
        scan_file("docs/releasing.md", "Not affiliated with ArtCraft. <!-- brand-ok -->\n", &mut hits);
        // Marked README line: suite infrastructure is still flagged.
        scan_file("README.md", "See https://getartcraft.com and craft-ui. <!-- brand-ok -->\n", &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("README.md"), line: 1, brand: "artcraft" },
                BrandHit { path: String::from("docs/releasing.md"), line: 1, brand: "artcraft" },
                BrandHit { path: String::from("README.md"), line: 1, brand: "getartcraft" },
                BrandHit { path: String::from("README.md"), line: 1, brand: "craft-ui" },
            ]
        );
    }

    #[test]
    fn flags_unmarked_prism_mentions_while_allowing_extensions_and_format_tokens() {
        let mut hits = Vec::new();
        let rust_src = r#"
pub fn ext_match(ext: &str) -> &'static str {
    match ext {
        "prism" | "pzfx" => "project.prism",
        _ => "other",
    }
}
pub const MSG_OK: &str = "Import *.prism or table.importPrism files";
pub const MSG_BRAND: &str = "Import .prism or Prism project";
pub const LABEL_BAD: &str = "Prism Archive";
pub const DOMAIN_BAD: &str = "https://www.prism.com/download";
pub const LABEL_MARKED: &str = "Prism Archive"; // brand-ok
#[cfg(test)]
mod tests {
    const TEST_FIXTURE: &str = "GraphPad Prism";
}
"#;
        scan_file("crates/format/src/registry.rs", rust_src, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/format/src/registry.rs"), line: 9, brand: "prism" },
                BrandHit { path: String::from("crates/format/src/registry.rs"), line: 10, brand: "prism" },
                BrandHit { path: String::from("crates/format/src/registry.rs"), line: 11, brand: "prism" },
            ]
        );
    }

    #[test]
    fn svg_allows_optical_prism_metadata_and_flags_non_optical_uses() {
        let mut hits = Vec::new();
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
  <title>LumenCraft icon — Mole holding a refractive prism</title>
  <desc>Clean geometric vector icon holding a glowing triangular optical
  prism dispersing light into a spectrum.</desc>
  <title>Prism Scientific Studio</title>
  <text x="0" y="10">optical prism</text>
  <!-- Compatibility icon label: brand-ok -->
  <title>Prism Archive</title>
</svg>
"#;
        scan_file("assets/app-icon/lumencraft.svg", svg, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("assets/app-icon/lumencraft.svg"), line: 5, brand: "prism" },
                BrandHit { path: String::from("assets/app-icon/lumencraft.svg"), line: 6, brand: "prism" },
            ]
        );
    }

    #[test]
    fn scans_github_issue_templates_and_apps_non_rust_files() {
        let mut hits = Vec::new();
        let template = "\
name: Numerical discrepancy
body:
  - type: markdown
    attributes:
      value: |
        Do not attach screenshots from Prism or OriginPro.
        Export `.prism` cleanly instead.
        # brand-ok compatibility mention: Prism
";
        scan_file(".github/ISSUE_TEMPLATE/numerical_result.yml", template, &mut hits);

        let html = "\
<!doctype html>
<html>
  <head><title>LumenCraft</title></head>
  <body>
    <input accept=\".lcf,.prism,.pzfx\">
    <p>Drop a Prism file here.</p>
    <p>Drop a Prism file here.</p> <!-- brand-ok -->
  </body>
</html>
";
        scan_file("apps/lumencraft-web/index.html", html, &mut hits);

        let web_rs = "\
#[cfg(test)]
mod tests {
    const FIXTURE: &str = \"Prism\";
}
pub fn prod_format() -> &'static str { \"prism\" }
";
        scan_file("apps/lumencraft-web/src/lib.rs", web_rs, &mut hits);

        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from(".github/ISSUE_TEMPLATE/numerical_result.yml"), line: 6, brand: "prism" },
                BrandHit { path: String::from(".github/ISSUE_TEMPLATE/numerical_result.yml"), line: 6, brand: "originpro" },
                BrandHit { path: String::from("apps/lumencraft-web/index.html"), line: 6, brand: "prism" },
                BrandHit { path: String::from("apps/lumencraft-web/src/lib.rs"), line: 5, brand: "prism" },
            ]
        );
    }

    #[test]
    fn scans_cargo_toml_multiline_descriptions_keywords_categories_and_metadata() {
        let mut hits = Vec::new();
        let toml = r#"
package.metadata.tool = "SPSS"

[package]
name = "demo"
description = """
Curve fitting for .prism files,
unlike GraphPad
or Prism."""
keywords = [
  "biostatistics",
  "Prism", # brand-ok
  "OriginPro",
]
categories = ["science", "prism"]
# Allowed note: brand-ok
homepage = """
https://example.com/prism
"""
documentation = """
Prism docs
""" # brand-ok

[package.metadata]
short_desc = "JMP importer"

[package.metadata.prism]
enabled = true

[workspace.package]
description = '''
Continuation
Minitab note
'''

[workspace.metadata]
suite_note = "Systat"

[workspace]
package = { description = "Prism workspace" }

[dependencies]
# Ignored dependency comment mentioning GraphPad and Prism
serde = "1"
"#;
        scan_file("crates/demo/Cargo.toml", toml, &mut hits);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 2, brand: "spss" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 8, brand: "graphpad" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 9, brand: "prism" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 13, brand: "originpro" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 15, brand: "prism" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 25, brand: "jmp" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 27, brand: "prism" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 33, brand: "minitab" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 37, brand: "systat" },
                BrandHit { path: String::from("crates/demo/Cargo.toml"), line: 40, brand: "prism" },
            ]
        );
    }

    #[test]
    fn tightened_ui_and_app_literal_prism_policy() {
        let mut hits = Vec::new();
        let ui_rs = r#"
pub fn dialog_strings() {
    let ok_brand = "LumenCraft";
    let ok_slug = "lumencraft";
    let ok_art = "approved-seven-pane";
    let ok_win95 = "lumencraft-win95-2508";
    let ok_filter = "Import (*.lumencraft, *.pzfx, *.prism)";
    let _ = (ok_brand, ok_slug, ok_art, ok_win95, ok_filter);

    let bad_bare = "prism";
    let bad_sentence = "Switch to .prism for layout";
    let bad_vendor = "GraphPad";
    let bad_suite = "ArtCraft";
    let bad_cap = "Prism";
    let _ = (bad_bare, bad_sentence, bad_vendor, bad_suite, bad_cap);
}
"#;
        scan_file("crates/ui-egui/src/dialogs_canvas.rs", ui_rs, &mut hits);

        let app_rs = r#"
pub fn detect_ext(ext: Option<&str>) -> &'static str {
    match ext {
        Some("prism") => "external",
        _ => "native",
    }
}
pub fn add_filters(dialog: DummyDialog) {
    dialog.add_filter("Supported files", &["lumencraft", "pzfx", "prism"]);
}
pub fn status_line() -> &'static str {
    "prism"
}
"#;
        scan_file("apps/lumencraft-web/src/lib.rs", app_rs, &mut hits);

        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("crates/ui-egui/src/dialogs_canvas.rs"), line: 13, brand: "artcraft" },
                BrandHit { path: String::from("crates/ui-egui/src/dialogs_canvas.rs"), line: 10, brand: "prism" },
                BrandHit { path: String::from("crates/ui-egui/src/dialogs_canvas.rs"), line: 11, brand: "prism" },
                BrandHit { path: String::from("crates/ui-egui/src/dialogs_canvas.rs"), line: 12, brand: "graphpad" },
                BrandHit { path: String::from("crates/ui-egui/src/dialogs_canvas.rs"), line: 14, brand: "prism" },
                BrandHit { path: String::from("apps/lumencraft-web/src/lib.rs"), line: 12, brand: "prism" },
            ]
        );
    }

    #[test]
    fn checks_16_locale_i18n_catalog_and_entries() {
        assert_eq!(check_i18n_catalog(), Vec::new());

        let entries = [
            ("menu.file.open", "en", "LumenCraft Project"),
            ("dialog.open.file_filter", "de", "Data files (*.lumencraft, *.pzfx, *.prism)"),
            ("menu.help.about", "fr", "Built for GraphPad users"),
            ("cmd.table.importPrism", "ja", "Prism からインポート"),
            ("toolbar.export", "ar", "Export prism archive"),
            ("dialog.open.file_filter", "es", "Prism files (*.prism)"),
            ("inspector.title", "ko", "ArtCraft Inspector"),
        ];
        let hits = check_i18n_catalog_entries(entries);
        assert_eq!(
            hits,
            vec![
                BrandHit { path: String::from("i18n:fr:menu.help.about"), line: 1, brand: "graphpad" },
                BrandHit { path: String::from("i18n:ja:cmd.table.importPrism"), line: 1, brand: "prism" },
                BrandHit { path: String::from("i18n:ar:toolbar.export"), line: 1, brand: "prism" },
                BrandHit { path: String::from("i18n:es:dialog.open.file_filter"), line: 1, brand: "prism" },
                BrandHit { path: String::from("i18n:ko:inspector.title"), line: 1, brand: "artcraft" },
            ]
        );
    }

    #[test]
    fn checks_text_runs_and_labeled_text_runs() {
        let runs = [
            "LumenCraft 1.0",
            "approved-seven-pane",
            "Import .pzfx / .prism archive",
            "Welcome to prism",
            "Powered by OriginPro and GraphPad",
            "Open Prism workspace",
        ];
        assert_eq!(
            check_text_runs("layout_tree", runs),
            vec![
                BrandHit { path: String::from("layout_tree"), line: 4, brand: "prism" },
                BrandHit { path: String::from("layout_tree"), line: 5, brand: "graphpad" },
                BrandHit { path: String::from("layout_tree"), line: 5, brand: "originpro" },
                BrandHit { path: String::from("layout_tree"), line: 6, brand: "prism" },
            ]
        );

        assert_eq!(
            check_text_runs_strict("window_labels", ["LumenCraft", "Import .prism archive"]),
            vec![BrandHit { path: String::from("window_labels"), line: 2, brand: "prism" }]
        );

        let labeled =
            [("node#1", "LumenCraft Canvas"), ("node#2", "File formats: *.pzfx, *.prism"), ("node#3", "Open in OriginPro"), ("node#4", "prism mode")];
        assert_eq!(
            check_labeled_text_runs(labeled),
            vec![
                BrandHit { path: String::from("node#3"), line: 3, brand: "originpro" },
                BrandHit { path: String::from("node#4"), line: 4, brand: "prism" },
            ]
        );
    }

    #[test]
    fn checks_export_and_window_metadata() {
        let clean_svg = r#"<svg><title>LumenCraft Chart</title><desc>Dose curve</desc><metadata>LumenCraft</metadata><text>Group A</text></svg>"#;
        assert_eq!(check_svg_metadata("clean.svg", clean_svg), Vec::new());

        let dirty_svg =
            "<svg>\n<title>Prism &amp; OriginPro</title>\n<desc>prism plot</desc>\n<metadata><author>GraphPad</author></metadata>\n</svg>";
        assert_eq!(
            check_svg_metadata("dirty.svg", dirty_svg),
            vec![
                BrandHit { path: String::from("dirty.svg"), line: 2, brand: "prism" },
                BrandHit { path: String::from("dirty.svg"), line: 2, brand: "originpro" },
                BrandHit { path: String::from("dirty.svg"), line: 3, brand: "prism" },
                BrandHit { path: String::from("dirty.svg"), line: 4, brand: "graphpad" },
            ]
        );

        let clean_pdf = b"%PDF-1.4\n1 0 obj\n<< /Title (LumenCraft) /Creator (LumenCraft) /Producer (LumenCraft) >>\nendobj\n";
        assert_eq!(check_pdf_metadata("clean.pdf", clean_pdf), Vec::new());

        // UTF-16BE BOM + "GraphPad" = FEFF 0047 0072 0061 0070 0068 0050 0061 0064
        let dirty_pdf = b"%PDF-1.4\n1 0 obj\n<< /Creator (Prism) /Producer (ArtCraft) /Title <FEFF00470072006100700068005000610064> >>\nendobj\n";
        assert_eq!(
            check_pdf_metadata("dirty.pdf", dirty_pdf),
            vec![
                BrandHit { path: String::from("dirty.pdf"), line: 3, brand: "prism" },
                BrandHit { path: String::from("dirty.pdf"), line: 3, brand: "artcraft" },
                BrandHit { path: String::from("dirty.pdf"), line: 3, brand: "graphpad" },
            ]
        );

        let clean_html = "<!doctype html>\n<html>\n<head><title>LumenCraft</title><meta name=\"generator\" content=\"LumenCraft\"></head>\n<body><input accept=\".lcf,.prism,.pzfx\"></body>\n</html>";
        assert_eq!(check_html_metadata("clean.html", clean_html), Vec::new());

        let dirty_html = "<html>\n<head><title>Prism</title><meta name=\"author\" content=\"OriginLab\"></head>\n<body aria-label=\"GraphPad workspace\"></body>\n</html>";
        assert_eq!(
            check_html_metadata("dirty.html", dirty_html),
            vec![
                BrandHit { path: String::from("dirty.html"), line: 2, brand: "prism" },
                BrandHit { path: String::from("dirty.html"), line: 2, brand: "originlab" },
                BrandHit { path: String::from("dirty.html"), line: 3, brand: "graphpad" },
            ]
        );

        let clean_manifest = r#"{"app":"LumenCraft","schema_version":1,"title":"Dose-Response Study"}"#;
        assert_eq!(check_manifest_metadata("manifest.json", clean_manifest), Vec::new());
        let dirty_manifest = r#"{"app":"LumenCraft","format":".prism","generator":"Prism","vendor":"GraphPad"}"#;
        assert_eq!(
            check_manifest_metadata("manifest.json", dirty_manifest),
            vec![
                BrandHit { path: String::from("manifest.json"), line: 2, brand: "prism" },
                BrandHit { path: String::from("manifest.json"), line: 3, brand: "prism" },
                BrandHit { path: String::from("manifest.json"), line: 4, brand: "graphpad" },
            ]
        );

        assert_eq!(check_window_title("desktop_host", "LumenCraft — Study 1"), Vec::new());
        assert_eq!(
            check_window_title("desktop_host", "LumenCraft (prism)"),
            vec![BrandHit { path: String::from("desktop_host"), line: 1, brand: "prism" }]
        );
    }

    #[test]
    fn checks_live_ui_surfaces_clean() {
        assert_eq!(check_ui_surfaces(), Ok(Vec::new()));
    }
}
