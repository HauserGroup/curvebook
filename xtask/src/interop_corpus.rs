//! External `.pzfx` / `.prism` interop corpus harness, structural observer, and conformance
//! profile machinery (§1.1, §1.2, §1.3, §6.3).
//!
//! Safety invariants:
//! - Refuses to run if a repo-internal corpus path is not gitignored (`git check-ignore`), has any
//!   tracked files (`git ls-files`), or contains symlinks escaping the corpus root.
//! - Verifies SHA-256 checksums against `provenance.tsv`, skipping dotfiles and non-format files.
//! - Emits aggregate counts only on stdout (never prints filenames, titles, cell values, or vendor
//!   strings). `--observe` writes structural-only observations into a gitignored ledger directory.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use curvebook_platform::sha256::sha256_hex;
use curvebook_platform::write_atomic;
use flate2::read::DeflateDecoder;
use serde_json::{Map, Value, json};

use crate::scan_util::{lower_ext, rel_slash};
use crate::text_decode::{decode_utf16, detect_bomless_utf16};

/// Default relative path to the local gitignored external interop corpus.
pub(crate) const DEFAULT_CORPUS_DIR: &str = "corpus/third-party-prism";

const CORPUS_ENV_VAR: &str = "CURVEBOOK_EXTERNAL_INTEROP_CORPUS";
const DEFAULT_LEDGER_REL: &str = "plan/format-observations";
const MAX_CORPUS_FILE_BYTES: usize = 256 * 1024 * 1024;
const MAX_ZIP_ENTRIES: usize = 100_000;
const MAX_ZIP_JSON_BYTES: usize = 32 * 1024 * 1024;
const MAX_ZIP_CSV_PROBE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ZIP_TOTAL_READ_BYTES: usize = 128 * 1024 * 1024;
const MAX_XML_DEPTH: usize = 256;

/// Minimum corpus file support required for a writer feature to be `Conformant` (§6.3).
pub const MIN_CONFORMANCE_FILES: usize = 3;
/// Minimum distinct source records required for a writer feature to be `Conformant` (§6.3).
pub const MIN_CONFORMANCE_SOURCES: usize = 2;
/// Minimum distinct `.prism` files required before §5 / §6.3 `.prism` design (§1.3).
pub const MIN_ARCHIVE_GATE_FILES: usize = 20;
/// Minimum distinct major versions required before §5 / §6.3 `.prism` design (§1.3).
pub const MIN_ARCHIVE_GATE_MAJOR_VERSIONS: usize = 2;

const USAGE: &str = "\
usage: cargo xtask interop-corpus [options]

options:
  --strict              fail loudly on missing/mismatched provenance entries or unexpected import errors
  --observe             write structural-only observations and dialect profiles to the gitignored ledger
  --corpus <dir>        override corpus root (else CURVEBOOK_EXTERNAL_INTEROP_CORPUS, else default local corpus)
  --ledger-dir <dir>    override gitignored output directory for --observe (default: plan/format-observations)
  --structural-only     run provenance and structural observation without CurveBook importer/engine pipeline
  -h, --help            print this help message
";

/// Parsed CLI options for `cargo xtask interop-corpus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteropCorpusOptions {
    pub strict: bool,
    pub observe: bool,
    pub structural_only: bool,
    pub help: bool,
    pub corpus_override: Option<PathBuf>,
    pub ledger_dir_override: Option<PathBuf>,
}

/// Parse CLI arguments for `cargo xtask interop-corpus`.
pub fn parse_args(args: &[String]) -> Result<InteropCorpusOptions, String> {
    let mut strict = false;
    let mut observe = false;
    let mut structural_only = false;
    let mut help = false;
    let mut corpus_override = None;
    let mut ledger_dir_override = None;
    let mut idx = 0usize;
    while let Some(arg) = args.get(idx) {
        match arg.as_str() {
            "--strict" => strict = true,
            "--observe" => observe = true,
            "--structural-only" => structural_only = true,
            "-h" | "--help" => help = true,
            "--corpus" => {
                idx += 1;
                let val = args.get(idx).ok_or_else(|| format!("missing value for `--corpus`\n\n{USAGE}"))?;
                corpus_override = Some(PathBuf::from(val));
            }
            "--ledger-dir" => {
                idx += 1;
                let val = args.get(idx).ok_or_else(|| format!("missing value for `--ledger-dir`\n\n{USAGE}"))?;
                ledger_dir_override = Some(PathBuf::from(val));
            }
            other => return Err(format!("unknown `cargo xtask interop-corpus` flag `{other}`\n\n{USAGE}")),
        }
        idx += 1;
    }
    Ok(InteropCorpusOptions { strict, observe, structural_only, help, corpus_override, ledger_dir_override })
}

/// Entry point invoked by `cargo xtask interop-corpus`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let opts = parse_args(args)?;
    if opts.help {
        print!("{USAGE}");
        return Ok(());
    }
    #[cfg(not(feature = "docs-site"))]
    if !opts.structural_only && std::env::var_os("CURVEBOOK_INTEROP_NO_DELEGATE").is_none() {
        return delegate_with_engine_pipeline(root, args);
    }
    let summary = run_with_options(root, &opts)?;
    print_aggregate_summary(&summary);
    if opts.strict {
        enforce_strict_gates(&summary)?;
    }
    Ok(())
}

#[cfg(not(feature = "docs-site"))]
fn delegate_with_engine_pipeline(root: &Path, args: &[String]) -> Result<(), String> {
    let mut cmd_args = vec!["run", "--locked", "-p", "xtask", "--features", "docs-site", "--", "interop-corpus"];
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    cmd_args.extend(arg_refs);
    let mut cmd = crate::dev_tool_command("cargo")
        .current_dir(root)
        .env("CURVEBOOK_INTEROP_NO_DELEGATE", "1")
        .args(cmd_args)
        .wall_timeout(std::time::Duration::from_secs(1800))
        .idle_timeout(std::time::Duration::from_secs(600));
    if let Ok(val) = std::env::var(CORPUS_ENV_VAR)
        && !val.is_empty()
    {
        cmd = cmd.env(CORPUS_ENV_VAR, val);
    }
    let out = cmd.run().map_err(|e| format!("delegate interop-corpus with docs-site pipeline: {e}"))?;
    if !out.stdout.is_empty() {
        print!("{}", String::from_utf8_lossy(&out.stdout));
    }
    if !out.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
    }
    if !out.success {
        return Err(format!("`interop-corpus` exited with status {:?}", out.status_code));
    }
    Ok(())
}

fn resolve_corpus_dir(root: &Path, opts: &InteropCorpusOptions) -> PathBuf {
    if let Some(dir) = &opts.corpus_override {
        return if dir.is_absolute() { dir.clone() } else { root.join(dir) };
    }
    if let Ok(raw) = std::env::var(CORPUS_ENV_VAR)
        && !raw.trim().is_empty()
    {
        let p = PathBuf::from(raw.trim());
        return if p.is_absolute() { p } else { root.join(p) };
    }
    root.join(DEFAULT_CORPUS_DIR)
}

fn resolve_ledger_dir(root: &Path, opts: &InteropCorpusOptions) -> PathBuf {
    match &opts.ledger_dir_override {
        Some(dir) if dir.is_absolute() => dir.clone(),
        Some(dir) => root.join(dir),
        None => root.join(DEFAULT_LEDGER_REL),
    }
}

/// Validate that a directory is safe to use as an external interop corpus:
/// - must exist and be a directory
/// - if located inside any git repository (including `workspace_root`), `git check-ignore` must
///   confirm it is ignored AND `git ls-files` must report zero tracked files beneath it.
pub fn validate_corpus_root(workspace_root: &Path, corpus_dir: &Path) -> Result<PathBuf, String> {
    if !corpus_dir.exists() {
        return Err(format!("interop corpus directory does not exist (set {CORPUS_ENV_VAR} or populate the local gitignored corpus)"));
    }
    let canon_corpus = std::fs::canonicalize(corpus_dir).map_err(|e| format!("canonicalize corpus root: {e}"))?;
    if !canon_corpus.is_dir() {
        return Err(String::from("interop corpus path is not a directory"));
    }
    verify_untracked_gitignored_if_in_repo(workspace_root, &canon_corpus, "corpus")?;
    Ok(canon_corpus)
}

/// Validate that an `--observe` ledger output directory is gitignored and untracked whenever it
/// falls inside a git repository.
pub fn validate_ledger_dir(workspace_root: &Path, ledger_dir: &Path) -> Result<(), String> {
    let probe = find_existing_ancestor(ledger_dir)?;
    let canon_probe = std::fs::canonicalize(&probe).map_err(|e| format!("canonicalize ledger ancestor: {e}"))?;
    let suffix = ledger_dir.strip_prefix(&probe).map_err(|e| format!("ledger suffix: {e}"))?;
    let full_target = canon_probe.join(suffix);
    verify_untracked_gitignored_if_in_repo(workspace_root, &full_target, "observation ledger")
}

fn find_existing_ancestor(path: &Path) -> Result<PathBuf, String> {
    let mut cur = path.to_path_buf();
    loop {
        if cur.exists() {
            return Ok(cur);
        }
        let Some(parent) = cur.parent() else {
            return Err(String::from("path has no existing ancestor directory"));
        };
        cur = parent.to_path_buf();
    }
}

fn verify_untracked_gitignored_if_in_repo(workspace_root: &Path, target: &Path, label: &str) -> Result<(), String> {
    let mut repos_to_check = Vec::new();
    if let Ok(canon_ws) = std::fs::canonicalize(workspace_root)
        && target.starts_with(&canon_ws)
    {
        repos_to_check.push(canon_ws);
    }
    let probe_dir = if target.is_dir() { target.to_path_buf() } else { find_existing_ancestor(target)? };
    if let Some(git_top) = git_toplevel(&probe_dir)
        && let Ok(canon_top) = std::fs::canonicalize(&git_top)
        && !repos_to_check.contains(&canon_top)
    {
        repos_to_check.push(canon_top);
    }

    for repo_root in repos_to_check {
        let Ok(rel) = target.strip_prefix(&repo_root) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if rel_str.is_empty() {
            return Err(format!("refusing to use git repository root as {label} path"));
        }
        let tracked = Command::new("git")
            .current_dir(&repo_root)
            .args(["ls-files", "-z", "--", &rel_str])
            .output()
            .map_err(|e| format!("git ls-files for {label}: {e}"))?;
        if !tracked.status.success() {
            return Err(format!("git ls-files failed while validating {label} path"));
        }
        if !tracked.stdout.is_empty() {
            return Err(format!("refusing to run: repo-internal {label} path contains tracked git files"));
        }
        let ignored = Command::new("git")
            .current_dir(&repo_root)
            .args(["check-ignore", "-q", "--", &rel_str])
            .status()
            .map_err(|e| format!("git check-ignore for {label}: {e}"))?;
        if !ignored.success() {
            return Err(format!("refusing to run: repo-internal {label} path is not gitignored (`git check-ignore` failed)"));
        }
    }
    Ok(())
}

fn git_toplevel(dir: &Path) -> Option<PathBuf> {
    let out = Command::new("git").current_dir(dir).args(["rev-parse", "--show-toplevel"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = std::str::from_utf8(&out.stdout).ok()?.trim();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// Format class of a discovered corpus file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InteropFormat {
    Pzfx,
    ZipArchive,
    BinaryPzf,
}

impl InteropFormat {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pzfx => ".pzfx",
            Self::ZipArchive => ".prism",
            Self::BinaryPzf => ".pzf (PCFF binary)",
        }
    }
}

/// Single parsed entry from `provenance.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceEntry {
    pub source: String,
    pub record_id: String,
    pub licence: String,
    pub expected_bytes: Option<u64>,
    pub expected_sha256: String,
}

/// Parse `provenance.tsv` content (supporting LF and CRLF) keyed by normalized relative path.
pub fn parse_provenance_tsv(text: &str) -> Result<BTreeMap<String, ProvenanceEntry>, String> {
    let mut lines = text.lines();
    let Some(header_line) = lines.next() else {
        return Ok(BTreeMap::new());
    };
    let headers: Vec<String> = header_line.split('\t').map(|h| h.trim().to_ascii_lowercase()).collect();
    let col_idx = |names: &[&str]| -> Option<usize> { headers.iter().position(|h| names.iter().any(|n| h == n)) };
    let source_i = col_idx(&["source"]).ok_or_else(|| String::from("provenance.tsv missing `source` column"))?;
    let path_i = col_idx(&["path"]).ok_or_else(|| String::from("provenance.tsv missing `path` column"))?;
    let sha_i = col_idx(&["sha256", "sha-256"]).ok_or_else(|| String::from("provenance.tsv missing `sha256` column"))?;
    let id_i = col_idx(&["id", "record_id"]);
    let doi_i = col_idx(&["doi"]);
    let licence_i = col_idx(&["licence", "license"]);
    let bytes_i = col_idx(&["bytes", "size"]);

    let mut out = BTreeMap::new();
    for (idx, line) in lines.enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let get_col = |opt: Option<usize>| -> &str { opt.and_then(|i| cols.get(i)).map_or("", |s| s.trim()) };
        let source = get_col(Some(source_i));
        let raw_path = get_col(Some(path_i));
        let sha = get_col(Some(sha_i)).to_ascii_lowercase();
        if raw_path.is_empty() {
            return Err(format!("provenance.tsv line {}: empty path column", idx + 2));
        }
        let norm_path = normalize_rel_path(raw_path);
        let id = get_col(id_i);
        let doi = get_col(doi_i);
        let record_id = if !id.is_empty() {
            format!("{source}:{id}")
        } else if !doi.is_empty() {
            format!("{source}:{doi}")
        } else {
            source.to_owned()
        };
        let licence_raw = get_col(licence_i);
        let licence = if licence_raw.is_empty() { String::from("unspecified") } else { licence_raw.to_owned() };
        let expected_bytes = {
            let b = get_col(bytes_i);
            if b.is_empty() { None } else { b.parse::<u64>().ok() }
        };
        out.insert(norm_path, ProvenanceEntry { source: source.to_owned(), record_id, licence, expected_bytes, expected_sha256: sha });
    }
    Ok(out)
}

fn normalize_rel_path(raw: &str) -> String {
    let slash = raw.replace('\\', "/");
    slash.trim_start_matches("./").to_owned()
}

/// Discovered file ready for verification and observation.
#[derive(Debug, Clone)]
pub struct DiscoveredFile {
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub declared_format: InteropFormat,
}

/// Result of recursively scanning the corpus root directory.
#[derive(Debug, Clone, Default)]
pub struct DiscoveredCorpus {
    pub files: Vec<DiscoveredFile>,
    pub skipped_dotfiles: usize,
    pub skipped_manifests_and_logs: usize,
    pub skipped_partial_downloads: usize,
    pub skipped_other_non_format: usize,
}

/// Walk `canon_root`, rejecting any symlink escaping `canon_root` (or broken symlinks), skipping
/// dotfiles and non-format files.
pub fn discover_corpus_files(canon_root: &Path) -> Result<DiscoveredCorpus, String> {
    let mut out = DiscoveredCorpus::default();
    let mut visited_dirs = BTreeSet::new();
    visited_dirs.insert(canon_root.to_path_buf());
    walk_corpus_dir(canon_root, canon_root, &mut visited_dirs, &mut out)?;
    out.files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(out)
}

fn walk_corpus_dir(canon_root: &Path, dir: &Path, visited_dirs: &mut BTreeSet<PathBuf>, out: &mut DiscoveredCorpus) -> Result<(), String> {
    let mut children = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read corpus dir: {e}"))?;
    for entry in entries {
        children.push(entry.map_err(|e| format!("corpus dir entry: {e}"))?.path());
    }
    children.sort();
    for path in children {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat corpus entry: {e}"))?;
        let is_symlink = meta.file_type().is_symlink();
        let resolved_meta = if is_symlink {
            let target = std::fs::canonicalize(&path).map_err(|_| String::from("refusing to run: corpus contains broken or unresolvable symlink"))?;
            if !target.starts_with(canon_root) {
                return Err(String::from("refusing to run: corpus contains symlink escaping corpus root"));
            }
            std::fs::metadata(&target).map_err(|e| format!("stat symlink target: {e}"))?
        } else {
            meta
        };

        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name.starts_with('.') {
            out.skipped_dotfiles += 1;
            continue;
        }
        if resolved_meta.is_dir() {
            let canon_dir = std::fs::canonicalize(&path).map_err(|e| format!("canonicalize corpus subdir: {e}"))?;
            if !canon_dir.starts_with(canon_root) {
                return Err(String::from("refusing to run: corpus subdirectory escapes corpus root"));
            }
            if visited_dirs.insert(canon_dir) {
                walk_corpus_dir(canon_root, &path, visited_dirs, out)?;
            }
            continue;
        }
        if !resolved_meta.is_file() {
            continue;
        }
        let rel_path = rel_slash(canon_root, &path)?;
        let ext = lower_ext(&rel_path);
        if rel_path.to_ascii_lowercase().ends_with(".prism") {
            out.files.push(DiscoveredFile { rel_path, abs_path: path, declared_format: InteropFormat::ZipArchive });
            continue;
        }
        match ext.as_str() {
            "pzfx" => out.files.push(DiscoveredFile { rel_path, abs_path: path, declared_format: InteropFormat::Pzfx }),
            "pzf" => out.files.push(DiscoveredFile { rel_path, abs_path: path, declared_format: InteropFormat::BinaryPzf }),
            "part" | "tmp" | "crdownload" | "download" | "partial" => out.skipped_partial_downloads += 1,
            "tsv" | "csv" | "log" | "txt" | "json" | "md" | "sh" | "py" | "yaml" | "yml" => out.skipped_manifests_and_logs += 1,
            _ => out.skipped_other_non_format += 1,
        }
    }
    Ok(())
}

fn read_bounded_bytes(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open corpus file: {e}"))?;
    let mut buf = Vec::new();
    let cap = u64::try_from(limit.saturating_add(1)).unwrap_or(u64::MAX);
    file.take(cap).read_to_end(&mut buf).map_err(|e| format!("read corpus file: {e}"))?;
    if buf.len() > limit {
        return Err(format!("file exceeds {limit}-byte safety limit"));
    }
    Ok(buf)
}

/// Aggregate numeric distribution percentiles (min, p50, p95, max).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SizeDistribution {
    pub count: usize,
    pub min: u64,
    pub p50: u64,
    pub p95: u64,
    pub max: u64,
    pub total: u64,
}

impl SizeDistribution {
    #[must_use]
    pub fn from_samples(samples: &mut [u64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        samples.sort_unstable();
        let n = samples.len();
        let p50_idx = (n - 1) / 2;
        let p95_idx = ((n - 1) * 95) / 100;
        let total = samples.iter().copied().fold(0u64, u64::saturating_add);
        Self {
            count: n,
            min: samples.first().copied().unwrap_or(0),
            p50: samples.get(p50_idx).copied().unwrap_or(0),
            p95: samples.get(p95_idx).copied().unwrap_or(0),
            max: samples.last().copied().unwrap_or(0),
            total,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "count": self.count,
            "min": self.min,
            "p50": self.p50,
            "p95": self.p95,
            "max": self.max,
            "total": self.total,
        })
    }
}

/// Structural CV 2-subcolumn diagnostic summary computed purely from aggregate statistics (never
/// stores raw cell values).
#[derive(Debug, Clone, Default)]
pub struct CvDiagnosticStats {
    pub cv_tables: usize,
    pub cv_y_columns: usize,
    pub declared_subcolumns_hist: BTreeMap<String, usize>,
    pub actual_subcolumns_hist: BTreeMap<usize, usize>,
    pub table_type_hist: BTreeMap<String, usize>,
    pub ev_format_hist: BTreeMap<String, usize>,
    pub numeric_pairs: usize,
    pub s1_gt_abs_s0_count: usize,
    pub s1_le_100_when_abs_s0_lt_0_1: usize,
    pub s1_gt_100_when_abs_s0_gt_1000: usize,
    pub s1_values_in_0_to_100: usize,
    pub log10_abs_s0_sum: f64,
    pub log10_s1_sum: f64,
    pub log10_s0_sq_sum: f64,
    pub log10_s1_sq_sum: f64,
    pub log10_cross_sum: f64,
    pub log10_pairs: usize,
    pub ratios_s1_over_abs_s0: Vec<f64>,
    pub abs_s0_samples: Vec<f64>,
    pub s1_samples: Vec<f64>,
}

impl CvDiagnosticStats {
    fn record_pair(&mut self, s0: f64, s1: f64) {
        if !s0.is_finite() || !s1.is_finite() {
            return;
        }
        self.numeric_pairs += 1;
        let a0 = s0.abs();
        self.abs_s0_samples.push(a0);
        self.s1_samples.push(s1);
        if s1 > a0 {
            self.s1_gt_abs_s0_count += 1;
        }
        if (0.0..=100.0).contains(&s1) {
            self.s1_values_in_0_to_100 += 1;
        }
        if a0 > 0.0 && a0 < 0.1 && s1 > 0.0 && s1 <= 100.0 {
            self.s1_le_100_when_abs_s0_lt_0_1 += 1;
        }
        if a0 > 1000.0 && s1 > 100.0 {
            self.s1_gt_100_when_abs_s0_gt_1000 += 1;
        }
        if a0 > 0.0 {
            self.ratios_s1_over_abs_s0.push(s1 / a0);
            if s1 > 0.0 {
                let lx = a0.log10();
                let ly = s1.log10();
                self.log10_abs_s0_sum += lx;
                self.log10_s1_sum += ly;
                self.log10_s0_sq_sum += lx * lx;
                self.log10_s1_sq_sum += ly * ly;
                self.log10_cross_sum += lx * ly;
                self.log10_pairs += 1;
            }
        }
    }

    fn merge(&mut self, other: &Self) {
        self.cv_tables += other.cv_tables;
        self.cv_y_columns += other.cv_y_columns;
        merge_counts(&mut self.declared_subcolumns_hist, &other.declared_subcolumns_hist);
        merge_counts(&mut self.actual_subcolumns_hist, &other.actual_subcolumns_hist);
        merge_counts(&mut self.table_type_hist, &other.table_type_hist);
        merge_counts(&mut self.ev_format_hist, &other.ev_format_hist);
        self.numeric_pairs += other.numeric_pairs;
        self.s1_gt_abs_s0_count += other.s1_gt_abs_s0_count;
        self.s1_le_100_when_abs_s0_lt_0_1 += other.s1_le_100_when_abs_s0_lt_0_1;
        self.s1_gt_100_when_abs_s0_gt_1000 += other.s1_gt_100_when_abs_s0_gt_1000;
        self.s1_values_in_0_to_100 += other.s1_values_in_0_to_100;
        self.log10_abs_s0_sum += other.log10_abs_s0_sum;
        self.log10_s1_sum += other.log10_s1_sum;
        self.log10_s0_sq_sum += other.log10_s0_sq_sum;
        self.log10_s1_sq_sum += other.log10_s1_sq_sum;
        self.log10_cross_sum += other.log10_cross_sum;
        self.log10_pairs += other.log10_pairs;
        self.ratios_s1_over_abs_s0.extend_from_slice(&other.ratios_s1_over_abs_s0);
        self.abs_s0_samples.extend_from_slice(&other.abs_s0_samples);
        self.s1_samples.extend_from_slice(&other.s1_samples);
    }

    fn log_log_slope_and_r(&self) -> (Option<f64>, Option<f64>) {
        if self.log10_pairs < 3 {
            return (None, None);
        }
        let n = self.log10_pairs as f64;
        let sxx = self.log10_s0_sq_sum - (self.log10_abs_s0_sum * self.log10_abs_s0_sum) / n;
        let syy = self.log10_s1_sq_sum - (self.log10_s1_sum * self.log10_s1_sum) / n;
        let sxy = self.log10_cross_sum - (self.log10_abs_s0_sum * self.log10_s1_sum) / n;
        let slope = if sxx > 1e-12 { Some(sxy / sxx) } else { None };
        let r = if sxx > 1e-12 && syy > 1e-12 { Some(sxy / (sxx * syy).sqrt()) } else { None };
        (slope, r)
    }

    fn to_json(&self) -> Value {
        let (slope, corr) = self.log_log_slope_and_r();
        json!({
            "cv_tables": self.cv_tables,
            "cv_y_columns": self.cv_y_columns,
            "declared_subcolumns": self.declared_subcolumns_hist,
            "actual_subcolumns": self.actual_subcolumns_hist,
            "table_types": self.table_type_hist,
            "ev_formats": self.ev_format_hist,
            "numeric_pairs": self.numeric_pairs,
            "s1_gt_abs_s0_count": self.s1_gt_abs_s0_count,
            "s1_in_0_100_count": self.s1_values_in_0_to_100,
            "s1_le_100_when_abs_s0_lt_0_1": self.s1_le_100_when_abs_s0_lt_0_1,
            "s1_gt_100_when_abs_s0_gt_1000": self.s1_gt_100_when_abs_s0_gt_1000,
            "abs_s0_quantiles": f64_quantiles_json(&self.abs_s0_samples),
            "s1_quantiles": f64_quantiles_json(&self.s1_samples),
            "ratio_s1_over_abs_s0_quantiles": f64_quantiles_json(&self.ratios_s1_over_abs_s0),
            "log10_slope_s1_vs_abs_s0": slope,
            "log10_pearson_r": corr,
        })
    }
}

fn f64_quantiles_json(values: &[f64]) -> Value {
    if values.is_empty() {
        return json!(null);
    }
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return json!(null);
    }
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let at = |pct: usize| sorted.get(((n - 1) * pct) / 100).copied().unwrap_or(0.0);
    json!({
        "min": at(0),
        "p25": at(25),
        "p50": at(50),
        "p75": at(75),
        "max": at(100),
    })
}

/// Structural observation of a single `.pzfx` XML file (zero content/titles/filenames).
#[derive(Debug, Clone, Default)]
pub struct PzfxObservation {
    pub file_id: String,
    pub observed_at: String,
    pub source: String,
    pub record_id: String,
    pub licence: String,
    pub bytes: u64,
    pub encoding: String,
    pub root_shape: String,
    pub xml_schema_version: Option<String>,
    pub parse_error_class: Option<String>,
    pub has_doctype: bool,
    pub trailing_raw_bytes: usize,
    pub trailing_non_ws_bytes: usize,
    pub trailing_class: String,
    pub template_elements: usize,
    pub template_total_bytes: usize,
    pub template_body_classes: BTreeMap<String, usize>,
    pub top_level_children: Vec<String>,
    pub element_counts: BTreeMap<String, usize>,
    pub parent_child_pairs: BTreeSet<(String, String)>,
    pub element_attrs: BTreeMap<String, BTreeSet<String>>,
    pub parent_child_orders: BTreeMap<String, Vec<String>>,
    pub table_count: usize,
    pub huge_table_count: usize,
    pub table_type_hist: BTreeMap<String, usize>,
    pub y_format_hist: BTreeMap<String, usize>,
    pub y_format_subcols_hist: BTreeMap<String, usize>,
    pub x_format_hist: BTreeMap<String, usize>,
    pub x_ev_format_hist: BTreeMap<String, usize>,
    pub x_ev_subcols_hist: BTreeMap<String, usize>,
    pub replicates_hist: BTreeMap<String, usize>,
    pub color_attr_names: BTreeMap<String, usize>,
    pub x_progression_attr_names: BTreeMap<String, usize>,
    pub excluded_attr_values: BTreeMap<String, usize>,
    pub rich_text_tags: BTreeMap<String, usize>,
    pub has_symbol_font: bool,
    pub has_subcolumn_titles: bool,
    pub has_decimals_attr: bool,
    pub table_seq_ref_count: usize,
    pub table_seq_direct_table_count: usize,
    pub table_seq_selected_values: BTreeMap<String, usize>,
    pub info_seq_ref_count: usize,
    pub info_seq_direct_info_count: usize,
    pub info_seq_selected_values: BTreeMap<String, usize>,
    pub info_constant_shapes: BTreeMap<String, usize>,
    pub huge_table_dims: Vec<(usize, usize, usize, String)>,
    pub max_regular_table_ycols: usize,
    pub max_regular_table_subcols: usize,
    pub cv_diagnostics: CvDiagnosticStats,
}

impl PzfxObservation {
    fn to_ledger_json(&self) -> Value {
        json!({
            "file_id": self.file_id,
            "observed_at": self.observed_at,
            "source": self.source,
            "record_id": self.record_id,
            "licence": self.licence,
            "bytes": self.bytes,
            "encoding": self.encoding,
            "root_shape": self.root_shape,
            "xml_schema_version": self.xml_schema_version,
            "parse_error_class": self.parse_error_class,
            "has_doctype": self.has_doctype,
            "trailing_raw_bytes": self.trailing_raw_bytes,
            "trailing_non_ws_bytes": self.trailing_non_ws_bytes,
            "trailing_class": self.trailing_class,
            "template_elements": self.template_elements,
            "template_total_bytes": self.template_total_bytes,
            "template_body_classes": self.template_body_classes,
            "top_level_children": self.top_level_children,
            "table_count": self.table_count,
            "huge_table_count": self.huge_table_count,
            "table_types": self.table_type_hist,
            "y_formats": self.y_format_hist,
            "y_format_subcols": self.y_format_subcols_hist,
            "x_formats": self.x_format_hist,
            "x_ev_formats": self.x_ev_format_hist,
            "table_seq_refs": self.table_seq_ref_count,
            "table_seq_direct_tables": self.table_seq_direct_table_count,
            "table_seq_selected": self.table_seq_selected_values,
            "info_seq_refs": self.info_seq_ref_count,
            "info_seq_direct_infos": self.info_seq_direct_info_count,
            "has_symbol_font": self.has_symbol_font,
            "has_subcolumn_titles": self.has_subcolumn_titles,
            "has_decimals_attr": self.has_decimals_attr,
            "excluded_attr_values": self.excluded_attr_values,
            "rich_text_tags": self.rich_text_tags,
            "info_constant_shapes": self.info_constant_shapes,
        })
    }
}

/// Inspect a `.pzfx` payload structurally (pure Rust, zero content leakage).
/// Returns `None` if `bytes` begins with `PCFF` (legacy binary `.pzf`, unsupported by design).
#[must_use]
pub fn observe_pzfx_bytes(bytes: &[u8], file_id: &str, observed_at: &str, source: &str, record_id: &str, licence: &str) -> Option<PzfxObservation> {
    if bytes.starts_with(b"PCFF") {
        return None;
    }
    let mut obs = PzfxObservation {
        file_id: file_id.to_owned(),
        observed_at: observed_at.to_owned(),
        source: source.to_owned(),
        record_id: record_id.to_owned(),
        licence: licence.to_owned(),
        bytes: bytes.len() as u64,
        trailing_class: String::from("none"),
        ..PzfxObservation::default()
    };
    let (encoding, text) = decode_xml_for_observation(bytes);
    obs.encoding = encoding;
    scan_pzfx_xml_structure(&text, &mut obs);
    Some(obs)
}

fn decode_xml_for_observation(bytes: &[u8]) -> (String, String) {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF])
        && let Ok(s) = std::str::from_utf8(rest)
    {
        return (String::from("utf-8-bom"), s.to_owned());
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return (String::from("utf-16-le-bom"), decode_utf16(rest, true));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return (String::from("utf-16-be-bom"), decode_utf16(rest, false));
    }
    if let Some(le) = detect_bomless_utf16(bytes) {
        let label = if le { "utf-16-le-nobom" } else { "utf-16-be-nobom" };
        return (String::from(label), decode_utf16(bytes, le));
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        let decl = peek_xml_declared_encoding(bytes);
        let label = decl.unwrap_or_else(|| String::from("utf-8"));
        return (label, s.to_owned());
    }
    let declared = peek_xml_declared_encoding(bytes).unwrap_or_else(|| String::from("non-utf8-unknown"));
    let decoded = match declared.as_str() {
        "windows-1252" | "cp1252" => decode_windows_1252(bytes),
        _ => bytes.iter().map(|&b| char::from(b)).collect(),
    };
    (declared, decoded)
}

fn peek_xml_declared_encoding(bytes: &[u8]) -> Option<String> {
    let head_len = bytes.len().min(256);
    let head: String = bytes.get(..head_len)?.iter().map(|&b| char::from(b)).collect();
    let decl_start = head.find("<?xml")?;
    let decl_end = head.get(decl_start..)?.find("?>")?;
    let decl = head.get(decl_start..decl_start + decl_end)?;
    let lower = decl.to_ascii_lowercase();
    let enc_pos = lower.find("encoding")?;
    let after = decl.get(enc_pos + 8..)?.trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let quote = after.chars().next().filter(|&c| c == '"' || c == '\'')?;
    let rest = after.get(1..)?;
    let end = rest.find(quote)?;
    let val = rest.get(..end)?.trim().to_ascii_lowercase();
    (!val.is_empty()).then_some(val)
}

fn decode_windows_1252(bytes: &[u8]) -> String {
    const CP1252_HIGH: [char; 32] = [
        '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}',
        '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}', '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}',
        '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
    ];
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if (0x80..=0x9F).contains(&b) {
            out.push(CP1252_HIGH.get(usize::from(b - 0x80)).copied().unwrap_or(char::from(b)));
        } else {
            out.push(char::from(b));
        }
    }
    out
}

#[derive(Debug, Default)]
struct ActiveTableState {
    is_huge: bool,
    table_type: String,
    ev_format: String,
    subcolumns_attr: String,
    x_ev_format: String,
    y_col_count: usize,
    in_y_col: bool,
    in_x_col: bool,
    cur_y_subcols: usize,
    cur_x_subcols: usize,
    max_subcols: usize,
    max_rows: usize,
    cur_subcol_rows: usize,
    is_cv_table: bool,
    cur_y_col_subcol_nums: Vec<Vec<Option<f64>>>,
    cur_subcol_idx: Option<usize>,
    in_d_cell: bool,
    d_buf: String,
}

#[derive(Debug, Default)]
struct ActiveConstantState {
    in_constant: bool,
    in_value: bool,
    value_buf: String,
    attr_value: Option<String>,
}

struct PzfxScanState<'a> {
    obs: &'a mut PzfxObservation,
    stack: Vec<String>,
    root_raw_name: Option<String>,
    table: Option<ActiveTableState>,
    constant: ActiveConstantState,
    in_title_or_note: usize,
}

fn scan_pzfx_xml_structure(text: &str, obs: &mut PzfxObservation) {
    let bytes = text.as_bytes();
    let mut state =
        PzfxScanState { obs, stack: Vec::new(), root_raw_name: None, table: None, constant: ActiveConstantState::default(), in_title_or_note: 0 };
    let mut pos = 0usize;
    let mut root_closed_at: Option<usize> = None;

    while pos < bytes.len() {
        let Some(rel_lt) = text.get(pos..).and_then(|s| s.find('<')) else {
            if let Some(t) = state.table.as_mut()
                && t.in_d_cell
            {
                t.d_buf.push_str(text.get(pos..).unwrap_or(""));
            }
            if state.constant.in_value {
                state.constant.value_buf.push_str(text.get(pos..).unwrap_or(""));
            }
            break;
        };
        let lt = pos + rel_lt;
        if lt > pos {
            let chunk = text.get(pos..lt).unwrap_or("");
            if let Some(t) = state.table.as_mut()
                && t.in_d_cell
            {
                t.d_buf.push_str(chunk);
            }
            if state.constant.in_value {
                state.constant.value_buf.push_str(chunk);
            }
        }
        let tail = text.get(lt..).unwrap_or("");
        if tail.starts_with("<!--") {
            let Some(end_rel) = tail.find("-->") else {
                state.obs.parse_error_class = Some(String::from("unclosed_comment"));
                return;
            };
            pos = lt + end_rel + 3;
            continue;
        }
        if tail.starts_with("<![CDATA[") {
            let Some(end_rel) = tail.find("]]>") else {
                state.obs.parse_error_class = Some(String::from("unclosed_cdata"));
                return;
            };
            let body = tail.get(9..end_rel).unwrap_or("");
            if let Some(t) = state.table.as_mut()
                && t.in_d_cell
            {
                t.d_buf.push_str(body);
            }
            if state.constant.in_value {
                state.constant.value_buf.push_str(body);
            }
            pos = lt + end_rel + 3;
            continue;
        }
        if tail.starts_with("<?") {
            let Some(end_rel) = tail.find("?>") else {
                state.obs.parse_error_class = Some(String::from("unclosed_pi"));
                return;
            };
            pos = lt + end_rel + 2;
            continue;
        }
        if tail.starts_with("<!") {
            let head_upper = tail.get(..tail.len().min(16)).unwrap_or("").to_ascii_uppercase();
            if head_upper.starts_with("<!DOCTYPE") || head_upper.starts_with("<!ENTITY") {
                state.obs.has_doctype = true;
            }
            let Some(gt_rel) = find_xml_tag_end(tail) else {
                state.obs.parse_error_class = Some(String::from("unclosed_decl"));
                return;
            };
            pos = lt + gt_rel + 1;
            continue;
        }
        let Some(gt_rel) = find_xml_tag_end(tail) else {
            state.obs.parse_error_class = Some(String::from("unclosed_tag"));
            return;
        };
        let tag_inner = tail.get(1..gt_rel).unwrap_or("").trim();
        let next_pos = lt + gt_rel + 1;
        if let Some(close_name_raw) = tag_inner.strip_prefix('/') {
            let close_name = close_name_raw.trim();
            handle_end_tag(&mut state, close_name);
            pos = next_pos;
            if state.root_raw_name.is_some() && state.stack.is_empty() {
                root_closed_at = Some(pos);
                break;
            }
            continue;
        }
        let self_closing = tag_inner.ends_with('/');
        let header = if self_closing { tag_inner.trim_end_matches('/').trim_end() } else { tag_inner };
        let (raw_name, attrs) = parse_tag_name_and_attrs(header);
        if raw_name.is_empty() {
            state.obs.parse_error_class = Some(String::from("empty_tag_name"));
            return;
        }
        if state.stack.len() >= MAX_XML_DEPTH {
            state.obs.parse_error_class = Some(String::from("max_depth_exceeded"));
            return;
        }
        let local = local_xml_name(&raw_name);
        let skip_template = local == "Template" && !self_closing;
        handle_start_tag(&mut state, &raw_name, local, &attrs, self_closing);
        if skip_template {
            if let Some((after_close, body_len, body_class)) = skip_opaque_template_body(text, next_pos) {
                state.obs.template_elements += 1;
                state.obs.template_total_bytes = state.obs.template_total_bytes.saturating_add(body_len);
                *state.obs.template_body_classes.entry(body_class).or_insert(0) += 1;
                handle_end_tag(&mut state, &raw_name);
                pos = after_close;
            } else {
                state.obs.parse_error_class = Some(String::from("unclosed_template"));
                return;
            }
        } else {
            pos = next_pos;
        }
        if state.root_raw_name.is_some() && state.stack.is_empty() {
            root_closed_at = Some(pos);
            break;
        }
    }

    let missing_root = state.root_raw_name.is_none();
    drop(state);

    match root_closed_at {
        Some(end_pos) => {
            let trailing = text.get(end_pos..).unwrap_or("");
            obs.trailing_raw_bytes = trailing.len();
            let non_ws = trailing.bytes().filter(|b| !b.is_ascii_whitespace()).count();
            obs.trailing_non_ws_bytes = non_ws;
            obs.trailing_class = classify_trailing_slice(trailing.as_bytes(), non_ws);
        }
        None => {
            if obs.parse_error_class.is_none() {
                obs.parse_error_class = Some(if missing_root { String::from("missing_root_element") } else { String::from("unclosed_root_element") });
            }
        }
    }
}

fn classify_trailing_slice(trailing: &[u8], non_ws: usize) -> String {
    if trailing.is_empty() {
        return String::from("none");
    }
    if non_ws == 0 {
        return String::from("whitespace_only");
    }
    if trailing.iter().any(|&b| b == 0 || (b < 0x20 && !b.is_ascii_whitespace())) { String::from("binary") } else { String::from("text") }
}

fn skip_opaque_template_body(text: &str, body_start: usize) -> Option<(usize, usize, String)> {
    let rest = text.get(body_start..)?;
    let close_idx = rest.find("</Template>")?;
    let body = rest.get(..close_idx)?;
    let trimmed = body.trim();
    let class = if trimmed.is_empty() {
        "empty"
    } else if trimmed.starts_with('<') {
        "xml_markup"
    } else if trimmed.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'\r' | b'\n' | b' ' | b'\t')) {
        "base64_or_ascii_token"
    } else if body.bytes().any(|b| b == 0 || b >= 0x80) {
        "binary_or_non_ascii"
    } else {
        "other_text"
    };
    Some((body_start + close_idx + "</Template>".len(), body.len(), String::from(class)))
}

fn find_xml_tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (idx, b) in s.bytes().enumerate() {
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'>' => return Some(idx),
                _ => {}
            }
        }
    }
    None
}

fn parse_tag_name_and_attrs(header: &str) -> (String, Vec<(String, String)>) {
    let bytes = header.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let name_start = i;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' {
        i += 1;
    }
    let name = header.get(name_start..i).unwrap_or("").to_owned();
    let mut attrs = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let k_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        let key = header.get(k_start..i).unwrap_or("");
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) != Some(&b'=') {
            break;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let Some(&q) = bytes.get(i).filter(|&&b| b == b'"' || b == b'\'') else {
            break;
        };
        i += 1;
        let v_start = i;
        while i < bytes.len() && bytes[i] != q {
            i += 1;
        }
        let val = header.get(v_start..i).unwrap_or("");
        if i < bytes.len() {
            i += 1;
        }
        if !key.is_empty() {
            attrs.push((key.to_owned(), val.to_owned()));
        }
    }
    (name, attrs)
}

fn local_xml_name(raw: &str) -> &str {
    raw.rsplit_once(':').map_or(raw, |(_, local)| local)
}

/// Inline mixed-content rich-text parents whose child tag order reflects user prose rather than
/// XML container schema order (for example `H<SUB>2</SUB>O <I>in vitro</I>` vs `<I>V</I><SUB>max</SUB>`).
fn is_mixed_content_rich_text_parent(parent: &str) -> bool {
    matches!(
        parent,
        "Title"
            | "d"
            | "FloatingNote"
            | "Font"
            | "Notes"
            | "Note"
            | "Value"
            | "Description"
            | "B"
            | "I"
            | "U"
            | "SUP"
            | "SUB"
            | "b"
            | "i"
            | "u"
            | "sup"
            | "sub"
    )
}

fn attr_val<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| local_xml_name(k) == key).map(|(_, v)| v.as_str())
}

fn handle_start_tag(state: &mut PzfxScanState<'_>, raw_name: &str, local: &str, attrs: &[(String, String)], self_closing: bool) {
    let is_root = state.root_raw_name.is_none();
    let canonical_tag = if is_root {
        state.root_raw_name = Some(raw_name.to_owned());
        state.obs.root_shape = if local == "PrismFile" {
            String::from("bare_root")
        } else if local.ends_with("PrismFile") {
            String::from("prefixed_standard_root")
        } else if local.eq_ignore_ascii_case("stylesheet") {
            String::from("xslt_wrapped_root")
        } else {
            String::from("other_root")
        };
        if let Some(ver) = attr_val(attrs, "PrismXMLVersion").or_else(|| attr_val(attrs, "SchemaVersion")).or_else(|| attr_val(attrs, "Version")) {
            state.obs.xml_schema_version = Some(ver.trim().to_owned());
        }
        String::from("<Root>")
    } else if local.ends_with("PrismFile") {
        if state.obs.xml_schema_version.is_none()
            && let Some(ver) = attr_val(attrs, "PrismXMLVersion").or_else(|| attr_val(attrs, "SchemaVersion")).or_else(|| attr_val(attrs, "Version"))
        {
            state.obs.xml_schema_version = Some(ver.trim().to_owned());
        }
        String::from("<InnerRoot>")
    } else {
        local.to_owned()
    };

    if let Some(parent) = state.stack.last() {
        if state.stack.len() == 1 {
            state.obs.top_level_children.push(canonical_tag.clone());
        }
        state.obs.parent_child_pairs.insert((parent.clone(), canonical_tag.clone()));
        if !is_mixed_content_rich_text_parent(parent) {
            let order = state.obs.parent_child_orders.entry(parent.clone()).or_default();
            if !order.contains(&canonical_tag) {
                order.push(canonical_tag.clone());
            }
        }
    }
    *state.obs.element_counts.entry(canonical_tag.clone()).or_insert(0) += 1;
    let attr_set = state.obs.element_attrs.entry(canonical_tag.clone()).or_default();
    for (k, _) in attrs {
        attr_set.insert(local_xml_name(k).to_owned());
    }

    if local == "Template" && self_closing {
        state.obs.template_elements += 1;
        *state.obs.template_body_classes.entry(String::from("empty")).or_insert(0) += 1;
    }

    observe_sequence_start(state, local, attrs);
    observe_table_start(state, local, attrs, self_closing);
    observe_rich_text_and_info_start(state, local, attrs, self_closing);

    if !self_closing {
        state.stack.push(canonical_tag);
    }
}

fn observe_sequence_start(state: &mut PzfxScanState<'_>, local: &str, attrs: &[(String, String)]) {
    let parent = state.stack.last().map(String::as_str);
    match (parent, local) {
        (Some("TableSequence" | "HugeTableSequence"), "Ref") => {
            state.obs.table_seq_ref_count += 1;
            if let Some(sel) = attr_val(attrs, "Selected") {
                *state.obs.table_seq_selected_values.entry(sel.trim().to_owned()).or_insert(0) += 1;
            }
        }
        (Some("TableSequence" | "HugeTableSequence"), "Table" | "HugeTable") => {
            state.obs.table_seq_direct_table_count += 1;
        }
        (Some("InfoSequence"), "Ref") => {
            state.obs.info_seq_ref_count += 1;
            if let Some(sel) = attr_val(attrs, "Selected") {
                *state.obs.info_seq_selected_values.entry(sel.trim().to_owned()).or_insert(0) += 1;
            }
        }
        (Some("InfoSequence"), "Info") => {
            state.obs.info_seq_direct_info_count += 1;
        }
        _ => {}
    }
}

fn observe_table_start(state: &mut PzfxScanState<'_>, local: &str, attrs: &[(String, String)], self_closing: bool) {
    if matches!(local, "Table" | "HugeTable" | "Table1024") {
        begin_table_observation(state, local, attrs, self_closing);
        return;
    }
    observe_table_child_start(state, local, attrs, self_closing);
}

fn begin_table_observation(state: &mut PzfxScanState<'_>, local: &str, attrs: &[(String, String)], self_closing: bool) {
    let is_huge = matches!(local, "HugeTable" | "Table1024");
    if is_huge {
        state.obs.huge_table_count += 1;
    } else {
        state.obs.table_count += 1;
    }
    let base_table_type = attr_val(attrs, "TableType").unwrap_or("<missing>").trim();
    let table_type = match attr_val(attrs, "ExtTableType").map(str::trim).filter(|s| !s.is_empty()) {
        Some(ext) => format!("{base_table_type}|Ext={ext}"),
        None => base_table_type.to_owned(),
    };
    let y_format = attr_val(attrs, "YFormat").unwrap_or("<missing>").trim().to_owned();
    let ev_format = attr_val(attrs, "EVFormat").unwrap_or("<missing>").trim().to_owned();
    let x_format = attr_val(attrs, "XFormat").unwrap_or("<missing>").trim().to_owned();
    let x_ev_format = attr_val(attrs, "XEVFormat").unwrap_or("<missing>").trim().to_owned();
    let subcols = attr_val(attrs, "Subcolumns").unwrap_or("<missing>").trim().to_owned();
    let reps = attr_val(attrs, "Replicates").unwrap_or("<missing>").trim().to_owned();

    *state.obs.table_type_hist.entry(table_type.clone()).or_insert(0) += 1;
    *state.obs.y_format_hist.entry(y_format.clone()).or_insert(0) += 1;
    *state.obs.y_format_subcols_hist.entry(format!("{y_format}|Subcolumns={subcols}|Replicates={reps}")).or_insert(0) += 1;
    if x_format != "<missing>" {
        *state.obs.x_format_hist.entry(x_format).or_insert(0) += 1;
    }
    if x_ev_format != "<missing>" {
        *state.obs.x_ev_format_hist.entry(x_ev_format.clone()).or_insert(0) += 1;
    }
    if reps != "<missing>" {
        *state.obs.replicates_hist.entry(reps).or_insert(0) += 1;
    }
    for (k, _) in attrs {
        let lk = local_xml_name(k);
        if lk.to_ascii_lowercase().contains("color") {
            *state.obs.color_attr_names.entry(lk.to_owned()).or_insert(0) += 1;
        }
        if matches!(lk, "XStart" | "XInterval" | "XStep" | "XIncrement" | "XAdvanced" | "XUnits" | "Units") {
            *state.obs.x_progression_attr_names.entry(lk.to_owned()).or_insert(0) += 1;
        }
        if lk == "Decimals" {
            state.obs.has_decimals_attr = true;
        }
    }
    let is_cv = y_format.eq_ignore_ascii_case("CV") || y_format.eq_ignore_ascii_case("CVN");
    let tbl =
        ActiveTableState { is_huge, table_type, ev_format, subcolumns_attr: subcols, x_ev_format, is_cv_table: is_cv, ..ActiveTableState::default() };
    if !self_closing {
        state.table = Some(tbl);
    }
}

fn observe_table_child_start(state: &mut PzfxScanState<'_>, local: &str, attrs: &[(String, String)], self_closing: bool) {
    let Some(t) = state.table.as_mut() else {
        return;
    };
    for (k, _) in attrs {
        let lk = local_xml_name(k);
        if lk == "Decimals" {
            state.obs.has_decimals_attr = true;
        }
        if lk.to_ascii_lowercase().contains("color") {
            *state.obs.color_attr_names.entry(format!("{local}@{lk}")).or_insert(0) += 1;
        }
        if matches!(lk, "XStart" | "XInterval" | "XStep" | "XIncrement" | "XAdvanced" | "XUnits" | "Units") {
            *state.obs.x_progression_attr_names.entry(format!("{local}@{lk}")).or_insert(0) += 1;
        }
    }
    match local {
        "YColumn" => {
            t.y_col_count += 1;
            if !self_closing {
                t.in_y_col = true;
                t.cur_y_subcols = 0;
                t.cur_y_col_subcol_nums.clear();
            }
        }
        "XColumn" | "XAdvancedColumn" => {
            if !self_closing {
                t.in_x_col = true;
                t.cur_x_subcols = 0;
            }
        }
        "Subcolumn" => {
            if t.in_y_col {
                t.cur_y_subcols += 1;
                t.cur_subcol_rows = 0;
                if t.is_cv_table && !self_closing {
                    let idx = t.cur_y_col_subcol_nums.len();
                    t.cur_y_col_subcol_nums.push(Vec::new());
                    t.cur_subcol_idx = Some(idx);
                }
            } else if t.in_x_col {
                t.cur_x_subcols += 1;
                t.cur_subcol_rows = 0;
            }
        }
        "SubColumnTitles" => {
            state.obs.has_subcolumn_titles = true;
        }
        "Title" if state.stack.last().map(String::as_str) == Some("Subcolumn") => {
            state.obs.has_subcolumn_titles = true;
        }
        "d" => {
            t.cur_subcol_rows += 1;
            if t.cur_subcol_rows > t.max_rows {
                t.max_rows = t.cur_subcol_rows;
            }
            if let Some(ex) = attr_val(attrs, "Excluded") {
                *state.obs.excluded_attr_values.entry(ex.trim().to_owned()).or_insert(0) += 1;
            }
            if t.is_cv_table && t.cur_subcol_idx.is_some() {
                if self_closing {
                    if let Some(idx) = t.cur_subcol_idx
                        && let Some(vec) = t.cur_y_col_subcol_nums.get_mut(idx)
                    {
                        vec.push(None);
                    }
                } else {
                    t.in_d_cell = true;
                    t.d_buf.clear();
                }
            }
        }
        _ => {}
    }
}

fn observe_rich_text_and_info_start(state: &mut PzfxScanState<'_>, local: &str, attrs: &[(String, String)], self_closing: bool) {
    if matches!(local, "Title" | "Note" | "Notes" | "Description") && !self_closing {
        state.in_title_or_note += 1;
    } else if state.in_title_or_note > 0 && matches!(local, "b" | "i" | "u" | "sup" | "sub" | "Font" | "br" | "B" | "I" | "U" | "Sup" | "Sub") {
        *state.obs.rich_text_tags.entry(local.to_owned()).or_insert(0) += 1;
        if local == "Font"
            && let Some(face) = attr_val(attrs, "Face")
            && face.trim().eq_ignore_ascii_case("symbol")
        {
            state.obs.has_symbol_font = true;
        }
    }

    if local == "Constant" {
        let attr_v = attr_val(attrs, "Value").map(str::to_owned);
        if self_closing {
            if let Some(v) = attr_v {
                record_info_constant_shape(state.obs, &v);
            }
        } else {
            state.constant = ActiveConstantState { in_constant: true, in_value: false, value_buf: String::new(), attr_value: attr_v };
        }
    } else if state.constant.in_constant && local == "Value" && !self_closing {
        state.constant.in_value = true;
        state.constant.value_buf.clear();
    }
}

fn handle_end_tag(state: &mut PzfxScanState<'_>, close_raw: &str) {
    let local = local_xml_name(close_raw);
    if matches!(local, "Title" | "Note" | "Notes" | "Description") {
        state.in_title_or_note = state.in_title_or_note.saturating_sub(1);
    }

    if local == "Value" && state.constant.in_value {
        state.constant.in_value = false;
        let v = std::mem::take(&mut state.constant.value_buf);
        record_info_constant_shape(state.obs, &v);
        state.constant.attr_value = None;
    } else if local == "Constant" && state.constant.in_constant {
        if let Some(v) = state.constant.attr_value.take() {
            record_info_constant_shape(state.obs, &v);
        }
        state.constant.in_constant = false;
    }

    if let Some(t) = state.table.as_mut() {
        match local {
            "d" if t.in_d_cell => {
                t.in_d_cell = false;
                let parsed = parse_lenient_f64(t.d_buf.trim());
                if let Some(idx) = t.cur_subcol_idx
                    && let Some(vec) = t.cur_y_col_subcol_nums.get_mut(idx)
                {
                    vec.push(parsed);
                }
                t.d_buf.clear();
            }
            "Subcolumn" => {
                t.cur_subcol_idx = None;
            }
            "YColumn" if t.in_y_col => {
                t.in_y_col = false;
                if t.cur_y_subcols > t.max_subcols {
                    t.max_subcols = t.cur_y_subcols;
                }
                if t.is_cv_table {
                    state.obs.cv_diagnostics.cv_y_columns += 1;
                    *state.obs.cv_diagnostics.actual_subcolumns_hist.entry(t.cur_y_subcols).or_insert(0) += 1;
                    if let (Some(c0), Some(c1)) = (t.cur_y_col_subcol_nums.first(), t.cur_y_col_subcol_nums.get(1)) {
                        for (a, b) in c0.iter().zip(c1.iter()) {
                            if let (Some(v0), Some(v1)) = (*a, *b) {
                                state.obs.cv_diagnostics.record_pair(v0, v1);
                            }
                        }
                    }
                    t.cur_y_col_subcol_nums.clear();
                }
            }
            "XColumn" | "XAdvancedColumn" if t.in_x_col => {
                t.in_x_col = false;
                if t.x_ev_format != "<missing>" {
                    let key = format!("{}|actual_subcols={}", t.x_ev_format, t.cur_x_subcols);
                    *state.obs.x_ev_subcols_hist.entry(key).or_insert(0) += 1;
                }
            }
            "Table" | "HugeTable" | "Table1024" => {
                if let Some(finished) = state.table.take() {
                    if finished.is_cv_table {
                        state.obs.cv_diagnostics.cv_tables += 1;
                        *state.obs.cv_diagnostics.declared_subcolumns_hist.entry(finished.subcolumns_attr.clone()).or_insert(0) += 1;
                        *state.obs.cv_diagnostics.table_type_hist.entry(finished.table_type.clone()).or_insert(0) += 1;
                        *state.obs.cv_diagnostics.ev_format_hist.entry(finished.ev_format.clone()).or_insert(0) += 1;
                    }
                    if finished.is_huge {
                        state.obs.huge_table_dims.push((finished.y_col_count, finished.max_subcols, finished.max_rows, finished.table_type));
                    } else {
                        state.obs.max_regular_table_ycols = state.obs.max_regular_table_ycols.max(finished.y_col_count);
                        state.obs.max_regular_table_subcols = state.obs.max_regular_table_subcols.max(finished.max_subcols);
                    }
                }
            }
            _ => {}
        }
    }

    if state.stack.len() == 1 {
        if state.root_raw_name.as_deref().is_some_and(|r| local_xml_name(r) == local) {
            state.stack.pop();
        }
    } else if state.stack.last().is_some_and(|top| top == local || (local.ends_with("PrismFile") && top == "<InnerRoot>")) {
        state.stack.pop();
    } else if let Some(pos) = state.stack.iter().rposition(|top| top == local) {
        state.stack.truncate(pos);
    }
}

fn record_info_constant_shape(obs: &mut PzfxObservation, raw: &str) {
    let trimmed = raw.trim();
    let shape = if trimmed.is_empty() {
        "empty"
    } else if is_strict_iso_date(trimmed) {
        "iso_date"
    } else if parse_lenient_f64(trimmed).is_some() {
        "number"
    } else {
        "other_text"
    };
    *obs.info_constant_shapes.entry(String::from(shape)).or_insert(0) += 1;
}

fn is_strict_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b.get(4) == Some(&b'-') && b.get(7) == Some(&b'-') && b.iter().enumerate().all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
}

fn parse_lenient_f64(s: &str) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    if let Ok(v) = s.parse::<f64>()
        && v.is_finite()
    {
        return Some(v);
    }
    if s.contains(',') && !s.contains('.') {
        let normalized = s.replace(',', ".");
        if let Ok(v) = normalized.parse::<f64>()
            && v.is_finite()
        {
            return Some(v);
        }
    }
    None
}

// ================================================================================================
// Lazy ZIP64 + JSON Structural Observer for `.prism` containers
// ================================================================================================

const ZIP_EOCD_SIG: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
const ZIP_CD_SIG: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
const ZIP_LFH_SIG: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
const ZIP64_LOCATOR_SIG: [u8; 4] = [0x50, 0x4b, 0x06, 0x07];
const ZIP64_EOCD_SIG: [u8; 4] = [0x50, 0x4b, 0x06, 0x06];

#[derive(Debug, Clone)]
struct LazyZipEntry {
    normalized_name: String,
    method: u16,
    expected_crc: u32,
    comp_size: u64,
    uncomp_size: u64,
    lfh_offset: u64,
    is_dir: bool,
}

#[derive(Debug)]
struct LazyZipMeta {
    entries: Vec<LazyZipEntry>,
    uses_zip64: bool,
    has_backslash_paths: bool,
    has_non_utf8_names: bool,
    compression_methods: BTreeSet<u16>,
}

/// Structural observation of a single `.prism` ZIP container (zero content/titles/filenames).
#[derive(Debug, Clone, Default)]
pub struct ArchiveObservation {
    pub file_id: String,
    pub observed_at: String,
    pub source: String,
    pub record_id: String,
    pub licence: String,
    pub bytes: u64,
    pub parse_error_class: Option<String>,
    pub entry_count: usize,
    pub total_uncompressed_bytes: u64,
    pub max_entry_uncompressed_bytes: u64,
    pub uses_zip64: bool,
    pub has_backslash_paths: bool,
    pub has_non_utf8_names: bool,
    pub compression_methods: BTreeSet<u16>,
    pub csv_entries: usize,
    pub json_entries: usize,
    pub image_entries: usize,
    pub xml_entries: usize,
    pub opaque_entries: usize,
    pub opaque_extensions: BTreeMap<String, usize>,
    pub path_templates: BTreeMap<String, usize>,
    pub dialect: String,
    pub version_keys: BTreeMap<String, String>,
    pub app_version: Option<String>,
    pub major_version: Option<u32>,
    pub document_json_keys: BTreeMap<String, String>,
    pub sheets_subkeys: BTreeMap<String, usize>,
    pub sheet_attrs_map_keys: BTreeSet<String>,
    pub role_json_keys: BTreeMap<String, BTreeSet<String>>,
    pub data_formats: BTreeMap<String, usize>,
    pub table_types: BTreeMap<String, usize>,
    pub analysis_classes: BTreeMap<String, usize>,
    pub csv_header_probe: BTreeMap<String, usize>,
    pub x_vs_row_titles_probe: BTreeMap<String, usize>,
    pub analysis_result_shapes: BTreeMap<String, usize>,
    pub has_analysis_params_json: bool,
    pub has_graph_previews: bool,
    pub has_exclusions_key: bool,
    pub has_info_sheets: bool,
    pub high_low_probe: HighLowOffsetProbe,
    pub zip_cv_probe: CvDiagnosticStats,
}

/// Aggregate probe checking whether `y_high_low` stores offsets or absolute bounds and subcol order.
#[derive(Debug, Clone, Default)]
pub struct HighLowOffsetProbe {
    pub tables: usize,
    pub numeric_triples: usize,
    pub both_non_negative: usize,
    pub c1_lt_mean_and_c2_gt_mean: usize,
    pub c1_gt_c2: usize,
    pub c1_lt_c2: usize,
    pub c1_eq_c2: usize,
    pub c1_gt_mean: usize,
    pub c2_lt_mean: usize,
}

impl HighLowOffsetProbe {
    fn record(&mut self, mean: f64, c1: f64, c2: f64) {
        if !mean.is_finite() || !c1.is_finite() || !c2.is_finite() {
            return;
        }
        self.numeric_triples += 1;
        if c1 >= 0.0 && c2 >= 0.0 {
            self.both_non_negative += 1;
        }
        if c1 <= mean && c2 >= mean {
            self.c1_lt_mean_and_c2_gt_mean += 1;
        }
        if c1 > c2 {
            self.c1_gt_c2 += 1;
        } else if c1 < c2 {
            self.c1_lt_c2 += 1;
        } else {
            self.c1_eq_c2 += 1;
        }
        if c1 > mean {
            self.c1_gt_mean += 1;
        }
        if c2 < mean {
            self.c2_lt_mean += 1;
        }
    }

    fn merge(&mut self, other: &Self) {
        self.tables += other.tables;
        self.numeric_triples += other.numeric_triples;
        self.both_non_negative += other.both_non_negative;
        self.c1_lt_mean_and_c2_gt_mean += other.c1_lt_mean_and_c2_gt_mean;
        self.c1_gt_c2 += other.c1_gt_c2;
        self.c1_lt_c2 += other.c1_lt_c2;
        self.c1_eq_c2 += other.c1_eq_c2;
        self.c1_gt_mean += other.c1_gt_mean;
        self.c2_lt_mean += other.c2_lt_mean;
    }

    fn to_json(&self) -> Value {
        json!({
            "tables": self.tables,
            "numeric_triples": self.numeric_triples,
            "both_non_negative": self.both_non_negative,
            "c1_le_mean_and_c2_ge_mean": self.c1_lt_mean_and_c2_gt_mean,
            "c1_gt_c2": self.c1_gt_c2,
            "c1_lt_c2": self.c1_lt_c2,
            "c1_eq_c2": self.c1_eq_c2,
            "c1_gt_mean": self.c1_gt_mean,
            "c2_lt_mean": self.c2_lt_mean,
        })
    }
}

impl ArchiveObservation {
    fn to_ledger_json(&self) -> Value {
        json!({
            "file_id": self.file_id,
            "observed_at": self.observed_at,
            "source": self.source,
            "record_id": self.record_id,
            "licence": self.licence,
            "bytes": self.bytes,
            "parse_error_class": self.parse_error_class,
            "entry_count": self.entry_count,
            "total_uncompressed_bytes": self.total_uncompressed_bytes,
            "max_entry_uncompressed_bytes": self.max_entry_uncompressed_bytes,
            "uses_zip64": self.uses_zip64,
            "has_backslash_paths": self.has_backslash_paths,
            "has_non_utf8_names": self.has_non_utf8_names,
            "compression_methods": self.compression_methods,
            "csv_entries": self.csv_entries,
            "json_entries": self.json_entries,
            "image_entries": self.image_entries,
            "xml_entries": self.xml_entries,
            "opaque_entries": self.opaque_entries,
            "opaque_extensions": self.opaque_extensions,
            "dialect": self.dialect,
            "version_keys": self.version_keys,
            "major_version": self.major_version,
            "document_json_keys": self.document_json_keys,
            "sheets_subkeys": self.sheets_subkeys,
            "sheet_attrs_map_keys": self.sheet_attrs_map_keys,
            "data_formats": self.data_formats,
            "table_types": self.table_types,
            "csv_header_probe": self.csv_header_probe,
            "x_vs_row_titles_probe": self.x_vs_row_titles_probe,
            "analysis_result_shapes": self.analysis_result_shapes,
            "has_analysis_params_json": self.has_analysis_params_json,
            "has_graph_previews": self.has_graph_previews,
            "has_exclusions_key": self.has_exclusions_key,
            "has_info_sheets": self.has_info_sheets,
        })
    }
}

/// Inspect a `.prism` ZIP container structurally (handles ZIP64, backslashes, lazy reads).
#[must_use]
pub fn observe_archive_bytes(bytes: &[u8], file_id: &str, observed_at: &str, source: &str, record_id: &str, licence: &str) -> ArchiveObservation {
    let mut obs = ArchiveObservation {
        file_id: file_id.to_owned(),
        observed_at: observed_at.to_owned(),
        source: source.to_owned(),
        record_id: record_id.to_owned(),
        licence: licence.to_owned(),
        bytes: bytes.len() as u64,
        dialect: String::from("unparsed"),
        ..ArchiveObservation::default()
    };
    let meta = match parse_lazy_zip_cd(bytes) {
        Ok(m) => m,
        Err(class) => {
            obs.parse_error_class = Some(class);
            return obs;
        }
    };
    obs.uses_zip64 = meta.uses_zip64;
    obs.has_backslash_paths = meta.has_backslash_paths;
    obs.has_non_utf8_names = meta.has_non_utf8_names;
    obs.compression_methods = meta.compression_methods;

    let file_entries: Vec<&LazyZipEntry> = meta.entries.iter().filter(|e| !e.is_dir).collect();
    obs.entry_count = file_entries.len();
    for entry in &file_entries {
        obs.total_uncompressed_bytes = obs.total_uncompressed_bytes.saturating_add(entry.uncomp_size);
        obs.max_entry_uncompressed_bytes = obs.max_entry_uncompressed_bytes.max(entry.uncomp_size);
        classify_archive_entry(entry, &mut obs);
    }

    let mut read_budget = MAX_ZIP_TOTAL_READ_BYTES;
    inspect_archive_json_and_csv(bytes, &file_entries, &mut read_budget, &mut obs);
    obs
}

fn classify_archive_entry(entry: &LazyZipEntry, obs: &mut ArchiveObservation) {
    let ext = lower_ext(&entry.normalized_name);
    match ext.as_str() {
        "csv" | "tsv" => obs.csv_entries += 1,
        "json" => obs.json_entries += 1,
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "svg" | "ico" | "avif" | "heic" => {
            obs.image_entries += 1;
            let low = entry.normalized_name.to_ascii_lowercase();
            if low.contains("graph") || low.contains("preview") || low.contains("thumb") {
                obs.has_graph_previews = true;
            }
        }
        "xml" | "pzfx" => obs.xml_entries += 1,
        _ => {
            obs.opaque_entries += 1;
            let key = if ext.is_empty() { String::from("<no-ext>") } else { format!(".{ext}") };
            *obs.opaque_extensions.entry(key).or_insert(0) += 1;
        }
    }
    let tpl = normalize_structural_path_template(&entry.normalized_name);
    *obs.path_templates.entry(tpl).or_insert(0) += 1;
}

fn normalize_structural_path_template(path: &str) -> String {
    const KNOWN_DIRS: &[&str] = &[
        "data",
        "sheets",
        "tables",
        "datasets",
        "analyses",
        "analysis",
        "results",
        "graphs",
        "layouts",
        "info",
        "notes",
        "assets",
        "images",
        "previews",
        "thumbnails",
        "metadata",
        "styles",
        "resources",
        "content",
        "project",
        "columns",
        "rows",
        "sets",
        "files",
        "objects",
        "cache",
        "caches",
        "templates",
        "fonts",
    ];
    const KNOWN_FILES: &[&str] = &[
        "document.json",
        "manifest.json",
        "sheet.json",
        "data.csv",
        "parameters.json",
        "results.json",
        "metadata.json",
        "info.json",
        "graph.json",
        "layout.json",
        "table.json",
        "index.json",
        "settings.json",
        "properties.json",
        "version.json",
        "schema.json",
        "dataset.json",
        "analysis.json",
        "result.json",
    ];

    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let last_idx = segs.len().saturating_sub(1);
    let mut out = Vec::with_capacity(segs.len());
    for (idx, seg) in segs.iter().enumerate() {
        let low = seg.to_ascii_lowercase();
        if idx == last_idx {
            if KNOWN_FILES.contains(&low.as_str()) {
                out.push(low);
            } else {
                let ext = lower_ext(seg);
                let stem = seg.strip_suffix(&format!(".{ext}")).unwrap_or(seg);
                let stem_tpl = if looks_like_uid(stem) {
                    "<uid>"
                } else if stem.bytes().all(|b| b.is_ascii_digit()) && !stem.is_empty() {
                    "<num>"
                } else {
                    "<file>"
                };
                if ext.is_empty() {
                    out.push(stem_tpl.to_owned());
                } else {
                    out.push(format!("{stem_tpl}.{ext}"));
                }
            }
        } else if KNOWN_DIRS.contains(&low.as_str()) {
            out.push(low);
        } else if looks_like_uid(seg) {
            out.push(String::from("<uid>"));
        } else if seg.bytes().all(|b| b.is_ascii_digit()) && !seg.is_empty() {
            out.push(String::from("<num>"));
        } else {
            out.push(String::from("<dir>"));
        }
    }
    out.join("/")
}

fn looks_like_uid(s: &str) -> bool {
    if s.len() < 8 {
        return false;
    }
    let hex_chars = s.bytes().filter(u8::is_ascii_hexdigit).count();
    let sep_chars = s.bytes().filter(|&b| matches!(b, b'-' | b'_')).count();
    hex_chars >= 8 && hex_chars + sep_chars == s.len()
}

fn inspect_archive_json_and_csv(bytes: &[u8], entries: &[&LazyZipEntry], read_budget: &mut usize, obs: &mut ArchiveObservation) {
    let doc_entry = entries.iter().copied().find(|e| file_name_ci(&e.normalized_name, "document.json"));
    let manifest_entry = entries.iter().copied().find(|e| file_name_ci(&e.normalized_name, "manifest.json"));

    let mut role_ids: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut analysis_preferred_results: BTreeSet<String> = BTreeSet::new();

    if let Some(de) = doc_entry {
        match read_lazy_entry(bytes, de, MAX_ZIP_JSON_BYTES, read_budget) {
            Ok(raw) => match serde_json::from_slice::<Value>(&raw) {
                Ok(val) => {
                    obs.dialect = classify_document_json_dialect(&val);
                    inspect_document_json(&val, obs, &mut role_ids);
                }
                Err(_) => {
                    obs.dialect = String::from("invalid_document_json");
                }
            },
            Err(class) => {
                obs.dialect = format!("unreadable_document_json:{class}");
            }
        }
    } else if manifest_entry.is_some() {
        obs.dialect = String::from("manifest_json_generic");
    } else {
        obs.dialect = String::from("no_document_json");
    }

    // Inspect remaining JSON entries (capped by read_budget).
    let mut data_table_probes: Vec<ArchiveTableProbe> = Vec::new();
    for &entry in entries {
        if lower_ext(&entry.normalized_name) != "json" {
            continue;
        }
        if file_name_ci(&entry.normalized_name, "document.json") {
            continue;
        }
        let Ok(raw) = read_lazy_entry(bytes, entry, MAX_ZIP_JSON_BYTES, read_budget) else {
            continue;
        };
        let Ok(val) = serde_json::from_slice::<Value>(&raw) else {
            continue;
        };
        extract_version_fields(&entry.normalized_name, &val, obs);
        let role = classify_json_entry_role(&entry.normalized_name, &val, &role_ids, &analysis_preferred_results);
        let key_set = obs.role_json_keys.entry(role.clone()).or_default();
        collect_json_key_paths(&val, "", 0, 3, key_set);
        if key_set.iter().any(|k| k.to_ascii_lowercase().contains("exclu")) {
            obs.has_exclusions_key = true;
        }
        if let Some(obj) = val.as_object() {
            if let Some(pref) = obj.get("preferredResultSheet").and_then(Value::as_str) {
                analysis_preferred_results.insert(pref.to_owned());
                *obs.analysis_result_shapes.entry(String::from("has_preferredResultSheet")).or_insert(0) += 1;
            }
            if obj.get("parameters").is_some() || obj.get("analysisParameters").is_some() || file_name_ci(&entry.normalized_name, "parameters.json") {
                obs.has_analysis_params_json = true;
            }
            if obj.get("results").is_some() || file_name_ci(&entry.normalized_name, "results.json") {
                *obs.analysis_result_shapes.entry(String::from("has_results_json")).or_insert(0) += 1;
            }
            if let Some(ac) = obj.get("analysisClass").and_then(Value::as_str)
                && !ac.is_empty()
                && file_name_ci(&entry.normalized_name, "sheet.json")
            {
                *obs.analysis_classes.entry(ac.to_owned()).or_insert(0) += 1;
            }
            if let Some(table) = obj.get("table").and_then(Value::as_object) {
                observe_archive_table_obj(table, obs, &mut data_table_probes);
            } else if obj.contains_key("dataFormat") {
                observe_archive_table_obj(obj, obs, &mut data_table_probes);
            }
        }
    }

    // Probe CSVs linked to observed data tables for headerless vs headered, X vs row-title order,
    // and high_low / cv storage diagnostics.
    for probe in data_table_probes {
        let csv_entry =
            entries.iter().copied().find(|e| lower_ext(&e.normalized_name) == "csv" && path_has_segment_or_stem(&e.normalized_name, &probe.uid));
        let Some(ce) = csv_entry else {
            continue;
        };
        let Ok(raw_csv) = read_lazy_entry(bytes, ce, MAX_ZIP_CSV_PROBE_BYTES, read_budget) else {
            continue;
        };
        let Ok(csv_text) = std::str::from_utf8(&raw_csv) else {
            continue;
        };
        probe_archive_csv(csv_text, &probe.data_format, probe.has_x, probe.has_row_titles, obs);
    }
}

struct ArchiveTableProbe {
    uid: String,
    data_format: String,
    has_x: bool,
    has_row_titles: bool,
}

fn observe_archive_table_obj(table: &Map<String, Value>, obs: &mut ArchiveObservation, probes: &mut Vec<ArchiveTableProbe>) {
    let data_format = table.get("dataFormat").and_then(Value::as_str).unwrap_or("<missing>").to_owned();
    *obs.data_formats.entry(data_format.clone()).or_insert(0) += 1;
    for k in ["format", "xDataSetFormat", "tableType", "type", "kind", "tableKind", "archetype"] {
        if let Some(s) = table.get(k).and_then(Value::as_str) {
            *obs.table_types.entry(format!("{k}={s}")).or_insert(0) += 1;
        }
    }
    let has_x = table.get("xDataSet").is_some_and(|v| !v.is_null() && v.as_str() != Some(""));
    let has_row_titles = table.get("rowTitlesDataSet").is_some_and(|v| !v.is_null() && v.as_str() != Some(""));
    if let Some(uid) = table.get("uid").and_then(Value::as_str)
        && !uid.is_empty()
    {
        probes.push(ArchiveTableProbe { uid: uid.to_owned(), data_format, has_x, has_row_titles });
    }
}

fn probe_archive_csv(csv_text: &str, data_fmt: &str, has_x: bool, has_row_titles: bool, obs: &mut ArchiveObservation) {
    let rows: Vec<Vec<&str>> =
        csv_text.lines().take(32).filter(|l| !l.trim().is_empty()).map(|l| l.split(',').map(|c| c.trim().trim_matches('"')).collect()).collect();
    let Some(first_row) = rows.first() else {
        return;
    };
    let prefix_cols = usize::from(has_x) + usize::from(has_row_titles);
    let data_cells = first_row.get(prefix_cols..).unwrap_or(&[]);
    let non_empty: Vec<&&str> = data_cells.iter().filter(|c| !c.is_empty()).collect();
    if !non_empty.is_empty() {
        let numeric = non_empty.iter().filter(|c| parse_lenient_f64(c).is_some()).count();
        let class = if numeric == non_empty.len() {
            "row0_all_numeric_headerless"
        } else if numeric == 0 {
            "row0_non_numeric_possible_header"
        } else {
            "row0_mixed"
        };
        *obs.csv_header_probe.entry(String::from(class)).or_insert(0) += 1;
    }

    if has_x && has_row_titles {
        let mut c0_num = 0usize;
        let mut c0_txt = 0usize;
        let mut c1_num = 0usize;
        let mut c1_txt = 0usize;
        for row in &rows {
            if let Some(v0) = row.first().copied().filter(|s| !s.is_empty()) {
                if parse_lenient_f64(v0).is_some() {
                    c0_num += 1;
                } else {
                    c0_txt += 1;
                }
            }
            if let Some(v1) = row.get(1).copied().filter(|s| !s.is_empty()) {
                if parse_lenient_f64(v1).is_some() {
                    c1_num += 1;
                } else {
                    c1_txt += 1;
                }
            }
        }
        let order = if c0_num > 0 && c0_txt == 0 && c1_txt > 0 {
            "col0_x_numeric_col1_row_titles_text"
        } else if c1_num > 0 && c1_txt == 0 && c0_txt > 0 {
            "col0_row_titles_text_col1_x_numeric"
        } else {
            "ambiguous_both_numeric_or_empty"
        };
        *obs.x_vs_row_titles_probe.entry(String::from(order)).or_insert(0) += 1;
    }

    if data_fmt == "y_high_low" {
        obs.high_low_probe.tables += 1;
        for row in &rows {
            let mut idx = prefix_cols;
            while idx + 2 < row.len() {
                if let (Some(m), Some(a), Some(b)) = (
                    row.get(idx).and_then(|s| parse_lenient_f64(s)),
                    row.get(idx + 1).and_then(|s| parse_lenient_f64(s)),
                    row.get(idx + 2).and_then(|s| parse_lenient_f64(s)),
                ) {
                    obs.high_low_probe.record(m, a, b);
                }
                idx += 3;
            }
        }
    } else if matches!(data_fmt, "y_cv" | "y_cv_n") {
        let stride = if data_fmt == "y_cv_n" { 3 } else { 2 };
        obs.zip_cv_probe.cv_tables += 1;
        for row in &rows {
            let mut idx = prefix_cols;
            while idx + 1 < row.len() {
                if let (Some(m), Some(err)) = (row.get(idx).and_then(|s| parse_lenient_f64(s)), row.get(idx + 1).and_then(|s| parse_lenient_f64(s))) {
                    obs.zip_cv_probe.record_pair(m, err);
                }
                idx += stride;
            }
        }
    }
}

fn classify_document_json_dialect(val: &Value) -> String {
    let Some(obj) = val.as_object() else {
        return String::from("non_object_document_json");
    };
    if obj.contains_key("data_sheets") {
        return String::from("curvebook_native");
    }
    let has_sheets = obj
        .get("sheets")
        .and_then(Value::as_object)
        .is_some_and(|s| s.get("data").is_some_and(Value::is_array) || s.get("analyses").is_some_and(Value::is_array));
    if has_sheets || obj.contains_key("sheetAttributesMap") {
        return String::from("external_id_indexed");
    }
    String::from("other_document_json")
}

fn inspect_document_json(val: &Value, obs: &mut ArchiveObservation, role_ids: &mut BTreeMap<String, BTreeSet<String>>) {
    let Some(obj) = val.as_object() else {
        return;
    };
    for (k, v) in obj {
        obs.document_json_keys.insert(k.clone(), json_type_name(v).to_owned());
        if matches!(k.as_str(), "createdBy" | "modifiedBy")
            && let Some(sub) = v.as_object()
        {
            for (sk, sv) in sub {
                obs.document_json_keys.insert(format!("{k}.{}", sanitize_key_label(sk)), json_type_name(sv).to_owned());
            }
        }
    }
    extract_version_fields("document.json", val, obs);
    if let Some(sheets) = obj.get("sheets").and_then(Value::as_object) {
        for (sk, sv) in sheets {
            if let Some(arr) = sv.as_array() {
                obs.sheets_subkeys.insert(sk.clone(), arr.len());
                if sk.to_ascii_lowercase().contains("info") && !arr.is_empty() {
                    obs.has_info_sheets = true;
                }
                let set = role_ids.entry(sk.clone()).or_default();
                for item in arr {
                    if let Some(id) = item.as_str() {
                        set.insert(id.to_owned());
                    }
                }
            } else {
                obs.sheets_subkeys.insert(format!("{sk}({})", json_type_name(sv)), 1);
            }
        }
    }
    if let Some(attr_map) = obj.get("sheetAttributesMap").and_then(Value::as_object) {
        for entry_val in attr_map.values() {
            if let Some(entry_obj) = entry_val.as_object() {
                for ek in entry_obj.keys() {
                    obs.sheet_attrs_map_keys.insert(ek.clone());
                }
            }
        }
    }
}

fn extract_version_fields(source_path: &str, val: &Value, obs: &mut ArchiveObservation) {
    let Some(obj) = val.as_object() else {
        return;
    };
    let file_tag = Path::new(source_path).file_name().and_then(|n| n.to_str()).unwrap_or("json");
    let is_doc_json = file_tag.eq_ignore_ascii_case("document.json");

    if is_doc_json {
        // Prefer creator/modifier application version over formatVersion/min*Version.
        for actor_key in ["modifiedBy", "createdBy"] {
            if let Some(actor_obj) = obj.get(actor_key).and_then(Value::as_object) {
                for (ak, av) in actor_obj {
                    let akl = ak.to_ascii_lowercase();
                    if akl.contains("version")
                        && let Some(sanitized) = sanitize_version_value(av)
                    {
                        let key_name = format!("document.json:{actor_key}.{}", sanitize_key_label(ak));
                        obs.version_keys.insert(key_name, sanitized.clone());
                        if obs.app_version.is_none() {
                            obs.major_version = parse_major_version_u32(&sanitized);
                            obs.app_version = Some(sanitized);
                        }
                    }
                }
            }
        }
    }

    for (k, v) in obj {
        let kl = k.to_ascii_lowercase();
        if !kl.contains("version") && kl != "format" {
            continue;
        }
        let Some(sanitized) = sanitize_version_value(v) else {
            continue;
        };
        let key_name = format!("{file_tag}:{}", sanitize_key_label(k));
        if is_doc_json && !matches!(kl.as_str(), "formatversion" | "minformatversion" | "format") && obs.app_version.is_none() {
            obs.major_version = parse_major_version_u32(&sanitized);
            obs.app_version = Some(sanitized.clone());
        }
        obs.version_keys.insert(key_name, sanitized);
    }
}

fn sanitize_key_label(key: &str) -> String {
    if crate::brands::contains_vendor_brand_word(key) { String::from("<vendor_key>") } else { key.to_owned() }
}

/// Extract numeric version tokens (e.g. `10.2.3`) from a JSON scalar, stripping any alphabetic words
/// so vendor/product names are never emitted.
fn sanitize_version_value(val: &Value) -> Option<String> {
    match val {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => {
            let mut best: Option<String> = None;
            for tok in s.split(|c: char| !c.is_ascii_digit() && c != '.') {
                let t = tok.trim_matches('.');
                if t.is_empty() || !t.bytes().any(|b| b.is_ascii_digit()) {
                    continue;
                }
                if best.as_ref().is_none_or(|b| t.len() > b.len()) {
                    best = Some(t.to_owned());
                }
            }
            best
        }
        Value::Object(map) => {
            let major = map.get("major").and_then(Value::as_u64)?;
            let minor = map.get("minor").and_then(Value::as_u64);
            let patch = map.get("patch").and_then(Value::as_u64);
            Some(match (minor, patch) {
                (Some(mi), Some(pa)) => format!("{major}.{mi}.{pa}"),
                (Some(mi), None) => format!("{major}.{mi}"),
                _ => major.to_string(),
            })
        }
        _ => None,
    }
}

fn parse_major_version_u32(sanitized: &str) -> Option<u32> {
    sanitized.split('.').next()?.parse::<u32>().ok()
}

fn classify_json_entry_role(path: &str, val: &Value, role_ids: &BTreeMap<String, BTreeSet<String>>, preferred_results: &BTreeSet<String>) -> String {
    for id in preferred_results {
        if path_has_segment_or_stem(path, id) {
            return String::from("preferred_result_json");
        }
    }
    for (role, ids) in role_ids {
        if ids.iter().any(|id| path_has_segment_or_stem(path, id)) {
            let fname = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("other.json");
            let suffix = if fname.eq_ignore_ascii_case("sheet.json") { "sheet.json" } else { "other.json" };
            return format!("{role}:{suffix}");
        }
    }
    if let Some(obj) = val.as_object() {
        if obj.contains_key("table") || obj.contains_key("dataFormat") {
            return String::from("table_or_sheet_json");
        }
        if obj.contains_key("title") && obj.len() <= 6 {
            return String::from("dataset_or_meta_json");
        }
    }
    String::from("other_json")
}

fn collect_json_key_paths(val: &Value, prefix: &str, depth: usize, max_depth: usize, out: &mut BTreeSet<String>) {
    if depth >= max_depth {
        return;
    }
    match val {
        Value::Object(map) => {
            for (k, v) in map {
                let seg = if looks_like_uid(k) { "<uid>" } else { k.as_str() };
                let safe_seg = sanitize_key_label(seg);
                let path = if prefix.is_empty() { safe_seg } else { format!("{prefix}.{safe_seg}") };
                out.insert(format!("{path}:{}", json_type_name(v)));
                collect_json_key_paths(v, &path, depth + 1, max_depth, out);
            }
        }
        Value::Array(arr) => {
            if let Some(first) = arr.first() {
                let path = format!("{prefix}[]");
                collect_json_key_paths(first, &path, depth + 1, max_depth, out);
            }
        }
        _ => {}
    }
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn file_name_ci(path: &str, target: &str) -> bool {
    Path::new(path).file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(target))
}

fn path_has_segment_or_stem(path: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    for seg in path.split('/') {
        if seg == id {
            return true;
        }
        if let Some(stem) = Path::new(seg).file_stem().and_then(|s| s.to_str())
            && stem == id
        {
            return true;
        }
    }
    false
}

fn parse_lazy_zip_cd(bytes: &[u8]) -> Result<LazyZipMeta, String> {
    let eocd = find_zip_eocd(bytes)?;
    let mut total_entries = u64::from(zip_u16(bytes, eocd + 10)?);
    let mut cd_size = u64::from(zip_u32(bytes, eocd + 12)?);
    let mut cd_offset = u64::from(zip_u32(bytes, eocd + 16)?);
    let mut uses_zip64 = false;

    if total_entries == u64::from(u16::MAX) || cd_size == u64::from(u32::MAX) || cd_offset == u64::from(u32::MAX) {
        uses_zip64 = true;
        if eocd < 20 {
            return Err(String::from("zip64_missing_locator"));
        }
        let loc = eocd - 20;
        if bytes.get(loc..loc + 4) != Some(ZIP64_LOCATOR_SIG.as_slice()) {
            return Err(String::from("zip64_invalid_locator"));
        }
        let z64_off = usize::try_from(zip_u64(bytes, loc + 8)?).map_err(|_| String::from("zip64_offset_overflow"))?;
        if bytes.get(z64_off..z64_off + 4) != Some(ZIP64_EOCD_SIG.as_slice()) {
            return Err(String::from("zip64_invalid_eocd"));
        }
        total_entries = zip_u64(bytes, z64_off + 32)?;
        cd_size = zip_u64(bytes, z64_off + 40)?;
        cd_offset = zip_u64(bytes, z64_off + 48)?;
    }

    let entry_count = usize::try_from(total_entries).map_err(|_| String::from("zip_entry_count_overflow"))?;
    if entry_count > MAX_ZIP_ENTRIES {
        return Err(String::from("zip_entry_limit_exceeded"));
    }
    let cd_start = usize::try_from(cd_offset).map_err(|_| String::from("zip_cd_offset_overflow"))?;
    let cd_len = usize::try_from(cd_size).map_err(|_| String::from("zip_cd_size_overflow"))?;
    let cd_end = cd_start.checked_add(cd_len).ok_or_else(|| String::from("zip_cd_bounds_overflow"))?;
    if cd_end > bytes.len() {
        return Err(String::from("zip_cd_out_of_bounds"));
    }

    let mut cursor = cd_start;
    let mut entries = Vec::with_capacity(entry_count.min(1024));
    let mut has_backslash_paths = false;
    let mut has_non_utf8_names = false;
    let mut compression_methods = BTreeSet::new();

    for _ in 0..entry_count {
        if cursor.checked_add(46).is_none_or(|e| e > cd_end) || bytes.get(cursor..cursor + 4) != Some(ZIP_CD_SIG.as_slice()) {
            return Err(String::from("zip_cd_corrupt_header"));
        }
        let flags = zip_u16(bytes, cursor + 8)?;
        let method = zip_u16(bytes, cursor + 10)?;
        let expected_crc = zip_u32(bytes, cursor + 16)?;
        let mut comp_size = u64::from(zip_u32(bytes, cursor + 20)?);
        let mut uncomp_size = u64::from(zip_u32(bytes, cursor + 24)?);
        let name_len = usize::from(zip_u16(bytes, cursor + 28)?);
        let extra_len = usize::from(zip_u16(bytes, cursor + 30)?);
        let comment_len = usize::from(zip_u16(bytes, cursor + 32)?);
        let mut lfh_offset = u64::from(zip_u32(bytes, cursor + 42)?);

        let name_start = cursor + 46;
        let name_end = name_start.checked_add(name_len).ok_or_else(|| String::from("zip_name_overflow"))?;
        let extra_end = name_end.checked_add(extra_len).ok_or_else(|| String::from("zip_extra_overflow"))?;
        let next_cursor = extra_end.checked_add(comment_len).ok_or_else(|| String::from("zip_record_overflow"))?;
        if next_cursor > cd_end {
            return Err(String::from("zip_cd_entry_oob"));
        }

        let extra_bytes = bytes.get(name_end..extra_end).unwrap_or(&[]);
        if uncomp_size == u64::from(u32::MAX) || comp_size == u64::from(u32::MAX) || lfh_offset == u64::from(u32::MAX) {
            uses_zip64 = true;
            apply_zip64_extra_field(extra_bytes, &mut uncomp_size, &mut comp_size, &mut lfh_offset)?;
        }

        let raw_name_bytes = bytes.get(name_start..name_end).unwrap_or(&[]);
        let raw_name = if (flags & 0x0800) != 0 {
            std::str::from_utf8(raw_name_bytes).map_or_else(
                |_| {
                    has_non_utf8_names = true;
                    String::from_utf8_lossy(raw_name_bytes).into_owned()
                },
                str::to_owned,
            )
        } else {
            match std::str::from_utf8(raw_name_bytes) {
                Ok(s) => s.to_owned(),
                Err(_) => {
                    has_non_utf8_names = true;
                    raw_name_bytes.iter().map(|&b| char::from(b)).collect()
                }
            }
        };
        if raw_name.contains('\\') {
            has_backslash_paths = true;
        }
        let normalized_name = raw_name.replace('\\', "/");
        let is_dir = normalized_name.ends_with('/');
        if !is_dir {
            compression_methods.insert(method);
        }
        entries.push(LazyZipEntry { normalized_name, method, expected_crc, comp_size, uncomp_size, lfh_offset, is_dir });
        cursor = next_cursor;
    }

    Ok(LazyZipMeta { entries, uses_zip64, has_backslash_paths, has_non_utf8_names, compression_methods })
}

fn apply_zip64_extra_field(extra: &[u8], uncomp: &mut u64, comp: &mut u64, lfh_off: &mut u64) -> Result<(), String> {
    let mut pos = 0usize;
    while pos + 4 <= extra.len() {
        let header_id = zip_u16(extra, pos)?;
        let data_size = usize::from(zip_u16(extra, pos + 2)?);
        let data_start = pos + 4;
        let data_end = data_start.checked_add(data_size).ok_or_else(|| String::from("zip64_extra_overflow"))?;
        if data_end > extra.len() {
            break;
        }
        if header_id == 0x0001 {
            let mut cur = data_start;
            if *uncomp == u64::from(u32::MAX) && cur + 8 <= data_end {
                *uncomp = zip_u64(extra, cur)?;
                cur += 8;
            }
            if *comp == u64::from(u32::MAX) && cur + 8 <= data_end {
                *comp = zip_u64(extra, cur)?;
                cur += 8;
            }
            if *lfh_off == u64::from(u32::MAX) && cur + 8 <= data_end {
                *lfh_off = zip_u64(extra, cur)?;
            }
            return Ok(());
        }
        pos = data_end;
    }
    Ok(())
}

fn read_lazy_entry(bytes: &[u8], entry: &LazyZipEntry, max_entry: usize, budget: &mut usize) -> Result<Vec<u8>, String> {
    let uncomp = usize::try_from(entry.uncomp_size).map_err(|_| String::from("entry_size_overflow"))?;
    if uncomp > max_entry || uncomp > *budget {
        return Err(String::from("entry_budget_exceeded"));
    }
    let lfh = usize::try_from(entry.lfh_offset).map_err(|_| String::from("lfh_offset_overflow"))?;
    if lfh.checked_add(30).is_none_or(|e| e > bytes.len()) || bytes.get(lfh..lfh + 4) != Some(ZIP_LFH_SIG.as_slice()) {
        return Err(String::from("invalid_lfh"));
    }
    let lfh_name_len = usize::from(zip_u16(bytes, lfh + 26)?);
    let lfh_extra_len = usize::from(zip_u16(bytes, lfh + 28)?);
    let data_start = lfh
        .checked_add(30)
        .and_then(|v| v.checked_add(lfh_name_len))
        .and_then(|v| v.checked_add(lfh_extra_len))
        .ok_or_else(|| String::from("lfh_data_overflow"))?;
    let comp_len = usize::try_from(entry.comp_size).map_err(|_| String::from("comp_size_overflow"))?;
    let data_end = data_start.checked_add(comp_len).ok_or_else(|| String::from("data_end_overflow"))?;
    let compressed = bytes.get(data_start..data_end).ok_or_else(|| String::from("payload_truncated"))?;

    let out = match entry.method {
        0 => compressed.to_vec(),
        8 => {
            let cap = u64::try_from(uncomp.saturating_add(1)).unwrap_or(u64::MAX);
            let mut dec = DeflateDecoder::new(compressed).take(cap);
            let mut buf = Vec::with_capacity(uncomp.min(1024 * 1024));
            dec.read_to_end(&mut buf).map_err(|_| String::from("deflate_error"))?;
            buf
        }
        _ => return Err(String::from("unsupported_compression")),
    };
    if out.len() > *budget {
        return Err(String::from("entry_budget_exceeded"));
    }
    if crc32fast::hash(&out) != entry.expected_crc {
        return Err(String::from("crc_mismatch"));
    }
    *budget = budget.saturating_sub(out.len());
    Ok(out)
}

fn find_zip_eocd(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() < 22 {
        return Err(String::from("too_small_for_zip"));
    }
    let min_pos = bytes.len().saturating_sub(22 + 65_535);
    let max_pos = bytes.len() - 22;
    for pos in (min_pos..=max_pos).rev() {
        if bytes.get(pos..pos + 4) == Some(ZIP_EOCD_SIG.as_slice()) {
            let comment_len = usize::from(zip_u16(bytes, pos + 20)?);
            if pos + 22 + comment_len == bytes.len() {
                return Ok(pos);
            }
        }
    }
    Err(String::from("missing_zip_eocd"))
}

fn zip_u16(bytes: &[u8], off: usize) -> Result<u16, String> {
    let slice = bytes.get(off..off.checked_add(2).ok_or_else(|| String::from("zip_u16_overflow"))?).ok_or_else(|| String::from("zip_eof"))?;
    let arr: [u8; 2] = slice.try_into().map_err(|_| String::from("zip_u16_slice"))?;
    Ok(u16::from_le_bytes(arr))
}

fn zip_u32(bytes: &[u8], off: usize) -> Result<u32, String> {
    let slice = bytes.get(off..off.checked_add(4).ok_or_else(|| String::from("zip_u32_overflow"))?).ok_or_else(|| String::from("zip_eof"))?;
    let arr: [u8; 4] = slice.try_into().map_err(|_| String::from("zip_u32_slice"))?;
    Ok(u32::from_le_bytes(arr))
}

fn zip_u64(bytes: &[u8], off: usize) -> Result<u64, String> {
    let slice = bytes.get(off..off.checked_add(8).ok_or_else(|| String::from("zip_u64_overflow"))?).ok_or_else(|| String::from("zip_eof"))?;
    let arr: [u8; 8] = slice.try_into().map_err(|_| String::from("zip_u64_slice"))?;
    Ok(u64::from_le_bytes(arr))
}

// ================================================================================================
// §6.3 Conformance Profile Machinery (Dialect Profile, Exclusion List, Outcomes, Table Sync)
// ================================================================================================

/// Documented exclusion item that CurveBook writers must NEVER copy even when present in 100% of
/// corpus files (§6.3 item 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileExclusion {
    pub format: &'static str,
    pub feature: &'static str,
    pub reason: &'static str,
}

/// Tracked, documented §6.3 exclusion list.
pub const EXCLUSION_LIST: &[ProfileExclusion] = &[
    ProfileExclusion {
        format: ".pzfx",
        feature: "attr:OriginalVersion@CreatedByProgram",
        reason: "vendor-identifying creator program string must never be forged",
    },
    ProfileExclusion {
        format: ".pzfx",
        feature: "attr:OriginalVersion@CreatedByVersion",
        reason: "vendor-identifying creator application version must never be forged",
    },
    ProfileExclusion { format: ".pzfx", feature: "attr:OriginalVersion@Login", reason: "depositor login name reveals personal user identity" },
    ProfileExclusion {
        format: ".pzfx",
        feature: "attr:OriginalVersion@DateTime",
        reason: "wall-clock creation timestamp breaks deterministic byte-identical export",
    },
    ProfileExclusion { format: ".pzfx", feature: "attr:OriginalVersion@Machine", reason: "depositor machine hostname reveals user environment" },
    ProfileExclusion { format: ".pzfx", feature: "attr:OriginalVersion@FilePath", reason: "local filesystem path reveals user directory structure" },
    ProfileExclusion {
        format: ".pzfx",
        feature: "attr:OriginalVersion@RegisteredTo",
        reason: "software registration owner reveals user/institution identity",
    },
    ProfileExclusion {
        format: ".pzfx",
        feature: "attr:MostRecentVersion@CreatedByProgram",
        reason: "vendor-identifying creator program string must never be forged",
    },
    ProfileExclusion {
        format: ".pzfx",
        feature: "attr:MostRecentVersion@CreatedByVersion",
        reason: "vendor-identifying creator application version must never be forged",
    },
    ProfileExclusion { format: ".pzfx", feature: "attr:MostRecentVersion@Login", reason: "depositor login name reveals personal user identity" },
    ProfileExclusion {
        format: ".pzfx",
        feature: "attr:MostRecentVersion@DateTime",
        reason: "wall-clock modification timestamp breaks deterministic byte-identical export",
    },
    ProfileExclusion {
        format: ".pzfx",
        feature: "attr:MostRecentVersion@RegisteredTo",
        reason: "software registration owner reveals user/institution identity",
    },
    ProfileExclusion { format: ".pzfx", feature: "elem:<Root>/Template", reason: "opaque binary/template blob is not CurveBook-authored data" },
    ProfileExclusion { format: ".prism", feature: "key:document.json:createdBy", reason: "vendor/creator identity field must never be copied" },
    ProfileExclusion { format: ".prism", feature: "key:document.json:modifiedBy", reason: "vendor/modifier identity field must never be copied" },
    ProfileExclusion {
        format: ".prism",
        feature: "key:document.json:creationDate",
        reason: "wall-clock creation timestamp breaks deterministic export",
    },
    ProfileExclusion {
        format: ".prism",
        feature: "key:document.json:modificationDate",
        reason: "wall-clock modification timestamp breaks deterministic export",
    },
    ProfileExclusion { format: ".prism", feature: "key:document.json:createdAt", reason: "wall-clock timestamp breaks deterministic export" },
    ProfileExclusion { format: ".prism", feature: "key:document.json:updatedAt", reason: "wall-clock timestamp breaks deterministic export" },
    ProfileExclusion { format: ".prism", feature: "key:document.json:author", reason: "depositor identity metadata must never be copied" },
    ProfileExclusion { format: ".prism", feature: "key:document.json:filePath", reason: "local user filesystem path must never be emitted" },
];

/// Returns the exclusion reason if `(format, feature)` appears on [`EXCLUSION_LIST`].
#[must_use]
pub fn exclusion_reason(format: &str, feature: &str) -> Option<&'static str> {
    EXCLUSION_LIST.iter().find(|e| e.format == format && e.feature == feature).map(|e| e.reason)
}

/// Corpus support for a single structural feature across distinct files and source records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FeatureSupport {
    pub file_count: usize,
    pub source_count: usize,
}

impl FeatureSupport {
    #[must_use]
    pub const fn meets_threshold(&self) -> bool {
        self.file_count >= MIN_CONFORMANCE_FILES && self.source_count >= MIN_CONFORMANCE_SOURCES
    }
}

/// Observed ordering support between two sibling child elements `(earlier, later)` under `parent`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderSupport {
    pub files_ab: usize,
    pub sources_ab: usize,
    pub files_ba: usize,
    pub sources_ba: usize,
}

/// Structural dialect profile derived from real corpus files (§6.3 item 1).
#[derive(Debug, Clone, Default)]
pub struct DialectProfile {
    pub format: String,
    pub total_files: usize,
    pub total_sources: usize,
    pub features: BTreeMap<String, FeatureSupport>,
    pub orders: BTreeMap<(String, String, String), OrderSupport>,
}

impl DialectProfile {
    /// Compute the set of non-excluded structural features present in 100% of profiled files
    /// (provided the corpus itself meets the minimum `k >= 3` files across `>= 2` sources gate).
    #[must_use]
    pub fn required_features(&self) -> BTreeSet<String> {
        if self.total_files < MIN_CONFORMANCE_FILES || self.total_sources < MIN_CONFORMANCE_SOURCES {
            return BTreeSet::new();
        }
        self.features
            .iter()
            .filter(|(feat, sup)| sup.file_count == self.total_files && exclusion_reason(&self.format, feat).is_none())
            .map(|(feat, _)| feat.clone())
            .collect()
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let feats: Map<String, Value> =
            self.features.iter().map(|(k, v)| (k.clone(), json!({ "files": v.file_count, "sources": v.source_count }))).collect();
        let req: Vec<String> = self.required_features().into_iter().collect();
        let mut ord_list = Vec::new();
        for ((parent, a, b), s) in &self.orders {
            if s.files_ab >= MIN_CONFORMANCE_FILES && s.sources_ab >= MIN_CONFORMANCE_SOURCES && s.files_ba == 0 {
                ord_list.push(json!({
                    "parent": parent,
                    "before": a,
                    "after": b,
                    "files": s.files_ab,
                    "sources": s.sources_ab,
                }));
            }
        }
        json!({
            "format": self.format,
            "total_files": self.total_files,
            "total_sources": self.total_sources,
            "min_conformance_files": MIN_CONFORMANCE_FILES,
            "min_conformance_sources": MIN_CONFORMANCE_SOURCES,
            "required_non_excluded_features": req,
            "unanimous_child_orders": ord_list,
            "features": feats,
        })
    }
}

/// Accumulator that builds a [`DialectProfile`] from observed per-file feature sets.
#[derive(Debug, Default)]
pub struct DialectProfileBuilder {
    format: String,
    file_ids: BTreeSet<String>,
    source_ids: BTreeSet<String>,
    feature_files: BTreeMap<String, BTreeSet<String>>,
    feature_sources: BTreeMap<String, BTreeSet<String>>,
    order_files: BTreeMap<(String, String, String), BTreeSet<String>>,
    order_sources: BTreeMap<(String, String, String), BTreeSet<String>>,
}

impl DialectProfileBuilder {
    #[must_use]
    pub fn new(format: impl Into<String>) -> Self {
        Self { format: format.into(), ..Self::default() }
    }

    /// Record one observed corpus file's structural features and per-parent child orders.
    pub fn record_file(&mut self, file_id: &str, record_id: &str, features: &BTreeSet<String>, parent_orders: &BTreeMap<String, Vec<String>>) {
        self.file_ids.insert(file_id.to_owned());
        self.source_ids.insert(record_id.to_owned());
        for feat in features {
            self.feature_files.entry(feat.clone()).or_default().insert(file_id.to_owned());
            self.feature_sources.entry(feat.clone()).or_default().insert(record_id.to_owned());
        }
        for (parent, seq) in parent_orders {
            for (i, earlier) in seq.iter().enumerate() {
                for later in seq.iter().skip(i + 1) {
                    let key = (parent.clone(), earlier.clone(), later.clone());
                    self.order_files.entry(key.clone()).or_default().insert(file_id.to_owned());
                    self.order_sources.entry(key).or_default().insert(record_id.to_owned());
                }
            }
        }
    }

    #[must_use]
    pub fn build(self) -> DialectProfile {
        let mut features = BTreeMap::new();
        for (feat, files) in self.feature_files {
            let sources = self.feature_sources.get(&feat).map_or(0, BTreeSet::len);
            features.insert(feat, FeatureSupport { file_count: files.len(), source_count: sources });
        }
        let mut orders = BTreeMap::new();
        for ((parent, a, b), files_ab) in &self.order_files {
            let sources_ab = self.order_sources.get(&(parent.clone(), a.clone(), b.clone())).map_or(0, BTreeSet::len);
            let rev_key = (parent.clone(), b.clone(), a.clone());
            let files_ba = self.order_files.get(&rev_key).map_or(0, BTreeSet::len);
            let sources_ba = self.order_sources.get(&rev_key).map_or(0, BTreeSet::len);
            orders.insert((parent.clone(), a.clone(), b.clone()), OrderSupport { files_ab: files_ab.len(), sources_ab, files_ba, sources_ba });
        }
        DialectProfile { format: self.format, total_files: self.file_ids.len(), total_sources: self.source_ids.len(), features, orders }
    }
}

/// Structural features emitted by a candidate writer output for §6.3 conformance evaluation.
#[derive(Debug, Clone, Default)]
pub struct CandidateStructure {
    pub emitted_features: BTreeSet<String>,
    pub parent_child_orders: BTreeMap<String, Vec<String>>,
    /// High-level feature coverage keys (e.g. `archetype=XY|mode=MeanSd|attr=YFormat:SD`).
    pub coverage_features: BTreeSet<String>,
}

/// Three §6.3 conformance outcomes per feature or structural check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureOutcome {
    Conformant,
    NonConformant { reason: String },
    InsufficientCorpusCoverage { files: usize, sources: usize },
}

/// Aggregate result of checking a writer candidate against a [`DialectProfile`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConformanceReport {
    pub feature_outcomes: BTreeMap<String, FeatureOutcome>,
    pub violations: Vec<String>,
}

impl ConformanceReport {
    #[must_use]
    pub fn has_non_conformant(&self) -> bool {
        !self.violations.is_empty() || self.feature_outcomes.values().any(|o| matches!(o, FeatureOutcome::NonConformant { .. }))
    }
}

/// Evaluate a candidate writer output against `profile` following §6.3 rules:
/// 1. Any emitted feature on [`EXCLUSION_LIST`] is `NonConformant`.
/// 2. Any missing feature from `profile.required_features()` is `NonConformant`.
/// 3. Any reversed unanimous child order (`>= k` files, `>= 2` sources, 0 reversed in corpus) is `NonConformant`.
/// 4. Each emitted structural feature and coverage feature gets `Conformant`, `NonConformant`, or
///    `InsufficientCorpusCoverage`.
#[must_use]
pub fn check_conformance(profile: &DialectProfile, candidate: &CandidateStructure) -> ConformanceReport {
    check_conformance_with_warned_gaps(profile, candidate, &BTreeSet::new())
}

/// Evaluate a candidate writer output against `profile` while treating zero-file features that
/// were explicitly acknowledged by a runtime `InsufficientCorpusCoverage` export warning as
/// [`FeatureOutcome::InsufficientCorpusCoverage`] rather than [`FeatureOutcome::NonConformant`] (§6.3 item 4).
#[must_use]
pub fn check_conformance_with_warned_gaps(
    profile: &DialectProfile,
    candidate: &CandidateStructure,
    warned_insufficient: &BTreeSet<String>,
) -> ConformanceReport {
    let mut report = ConformanceReport::default();

    for req in profile.required_features() {
        if !candidate.emitted_features.contains(&req) {
            let reason = format!("missing required non-excluded feature `{req}` present in 100% of corpus files");
            report.violations.push(reason.clone());
            report.feature_outcomes.insert(req, FeatureOutcome::NonConformant { reason });
        }
    }

    for feat in &candidate.emitted_features {
        if let Some(reason) = exclusion_reason(&profile.format, feat) {
            let msg = format!("emitted excluded feature `{feat}` ({reason})");
            report.violations.push(msg.clone());
            report.feature_outcomes.insert(feat.clone(), FeatureOutcome::NonConformant { reason: msg });
            continue;
        }
        match profile.features.get(feat) {
            Some(sup) if sup.meets_threshold() => {
                report.feature_outcomes.insert(feat.clone(), FeatureOutcome::Conformant);
            }
            Some(sup) => {
                report
                    .feature_outcomes
                    .insert(feat.clone(), FeatureOutcome::InsufficientCorpusCoverage { files: sup.file_count, sources: sup.source_count });
            }
            None if warned_insufficient.contains(feat) => {
                report.feature_outcomes.insert(feat.clone(), FeatureOutcome::InsufficientCorpusCoverage { files: 0, sources: 0 });
            }
            None => {
                let reason = format!("emitted feature `{feat}` never observed in corpus profile");
                report.violations.push(reason.clone());
                report.feature_outcomes.insert(feat.clone(), FeatureOutcome::NonConformant { reason });
            }
        }
    }

    for feat in &candidate.coverage_features {
        let sup = profile.features.get(feat).cloned().unwrap_or_default();
        let outcome = if sup.meets_threshold() {
            FeatureOutcome::Conformant
        } else {
            FeatureOutcome::InsufficientCorpusCoverage { files: sup.file_count, sources: sup.source_count }
        };
        report.feature_outcomes.insert(feat.clone(), outcome);
    }

    for (parent, seq) in &candidate.parent_child_orders {
        for (i, earlier) in seq.iter().enumerate() {
            for later in seq.iter().skip(i + 1) {
                let rev_key = (parent.clone(), later.clone(), earlier.clone());
                if let Some(ord) = profile.orders.get(&rev_key)
                    && ord.files_ab >= MIN_CONFORMANCE_FILES
                    && ord.sources_ab >= MIN_CONFORMANCE_SOURCES
                    && ord.files_ba == 0
                {
                    let reason = format!(
                        "reversed unanimous child order under `{parent}`: emitted `{earlier}` before `{later}`, corpus always orders `{later}` before `{earlier}`"
                    );
                    report.violations.push(reason.clone());
                    report.feature_outcomes.insert(format!("order:{parent}:{earlier}<{later}"), FeatureOutcome::NonConformant { reason });
                }
            }
        }
    }

    report
}

/// Static coverage status declared next to a writer (`crates/format/src/pzfx/export.rs`, §6.3 item 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclaredCoverage {
    Conformant,
    InsufficientCoverage,
}

/// Single row of a writer's tracked static feature-coverage table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticFeatureCoverage {
    pub feature: &'static str,
    pub declared: DeclaredCoverage,
}

/// Verify that a writer's tracked static feature-coverage table does not over-claim `Conformant`
/// for any feature that fails the corpus profile's `k >= 3` files across `>= 2` sources gate.
pub fn verify_static_coverage_table(profile: &DialectProfile, table: &[StaticFeatureCoverage]) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    for row in table {
        let sup = profile.features.get(row.feature).cloned().unwrap_or_default();
        if row.declared == DeclaredCoverage::Conformant && !sup.meets_threshold() {
            errors.push(format!(
                "feature `{}` is statically marked Conformant but corpus profile has only {} file(s) across {} source(s) (need >= {MIN_CONFORMANCE_FILES} files, >= {MIN_CONFORMANCE_SOURCES} sources)",
                row.feature, sup.file_count, sup.source_count
            ));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

// ================================================================================================
// Corpus Execution & Aggregation
// ================================================================================================

/// Aggregate §6.3 `.pzfx` corpus conformance and static coverage-table verification results.
#[derive(Debug, Clone, Default)]
pub struct PzfxConformanceSummary {
    pub evaluated: bool,
    pub static_table_rows_total: usize,
    pub static_table_conformant_rows: usize,
    pub static_table_insufficient_rows: usize,
    pub static_table_sync_errors: Vec<String>,
    pub synthetic_cases_total: usize,
    pub synthetic_determinism_ok: usize,
    pub synthetic_roundtrip_ok: usize,
    pub synthetic_roundtrip_diffs: usize,
    pub synthetic_conformant_cases: usize,
    pub synthetic_non_conformant_cases: usize,
    pub synthetic_violations: BTreeMap<String, usize>,
    pub synthetic_insufficient_features: BTreeMap<String, usize>,
    pub reexported_files_checked: usize,
    pub reexported_conformant_files: usize,
    pub reexported_non_conformant_files: usize,
    pub reexported_violations: BTreeMap<String, usize>,
    pub reexported_insufficient_features: BTreeMap<String, usize>,
}

impl PzfxConformanceSummary {
    fn to_json(&self) -> Value {
        json!({
            "evaluated": self.evaluated,
            "static_table_rows_total": self.static_table_rows_total,
            "static_table_conformant_rows": self.static_table_conformant_rows,
            "static_table_insufficient_rows": self.static_table_insufficient_rows,
            "static_table_sync_errors": self.static_table_sync_errors,
            "synthetic_cases_total": self.synthetic_cases_total,
            "synthetic_determinism_ok": self.synthetic_determinism_ok,
            "synthetic_roundtrip_ok": self.synthetic_roundtrip_ok,
            "synthetic_roundtrip_diffs": self.synthetic_roundtrip_diffs,
            "synthetic_conformant_cases": self.synthetic_conformant_cases,
            "synthetic_non_conformant_cases": self.synthetic_non_conformant_cases,
            "synthetic_violations": self.synthetic_violations,
            "synthetic_insufficient_features": self.synthetic_insufficient_features,
            "reexported_files_checked": self.reexported_files_checked,
            "reexported_conformant_files": self.reexported_conformant_files,
            "reexported_non_conformant_files": self.reexported_non_conformant_files,
            "reexported_violations": self.reexported_violations,
            "reexported_insufficient_features": self.reexported_insufficient_features,
        })
    }
}

/// Aggregate Step 7 cached-vs-recomputed diagnostic summary for one external `.prism` `analysisClass`.
#[derive(Debug, Clone, Default)]
pub struct AnalysisClassDiagnosticSummary {
    pub mapping_status: String,
    pub curvebook_kind: String,
    pub analyses_observed: usize,
    pub translated_clean: usize,
    pub translated_blocked: usize,
    pub blocking_reason_classes: BTreeMap<String, usize>,
    pub incompatible_or_unresolved_source: usize,
    pub recomputed_ok: usize,
    pub recomputed_failed: usize,
    pub recompute_panics: usize,
    pub cached_scalars_total: usize,
    pub recomputed_scalars_total: usize,
    pub scalars_compared: usize,
    pub scalars_agreed: usize,
    pub scalars_disagreed: usize,
    pub disagreed_scalar_keys: BTreeMap<String, usize>,
    pub compared_scalar_keys: BTreeMap<String, usize>,
    /// Per `experimentalDesign` (or `<default>`): `[recomputed_ok, analyses_compared, analyses_fully_agreed]`.
    pub variants: BTreeMap<String, [usize; 3]>,
}

impl AnalysisClassDiagnosticSummary {
    fn to_json(&self) -> Value {
        json!({
            "mapping_status": self.mapping_status,
            "curvebook_kind": self.curvebook_kind,
            "analyses_observed": self.analyses_observed,
            "translated_clean": self.translated_clean,
            "translated_blocked": self.translated_blocked,
            "blocking_reason_classes": self.blocking_reason_classes,
            "incompatible_or_unresolved_source": self.incompatible_or_unresolved_source,
            "recomputed_ok": self.recomputed_ok,
            "recomputed_failed": self.recomputed_failed,
            "recompute_panics": self.recompute_panics,
            "cached_scalars_total": self.cached_scalars_total,
            "recomputed_scalars_total": self.recomputed_scalars_total,
            "scalars_compared": self.scalars_compared,
            "scalars_agreed": self.scalars_agreed,
            "scalars_disagreed": self.scalars_disagreed,
            "disagreed_scalar_keys": self.disagreed_scalar_keys,
            "compared_scalar_keys": self.compared_scalar_keys,
            "variants": self.variants.iter().map(|(k, v)| (k.clone(), json!({"recomputed_ok": v[0], "analyses_compared": v[1], "analyses_fully_agreed": v[2]}))).collect::<Map<String, Value>>(),
        })
    }
}

/// Aggregate CurveBook 7-step import/export/conformance/diagnostic metrics collected when compiled
/// with `docs-site`.
#[derive(Debug, Clone, Default)]
pub struct EnginePipelineSummary {
    pub enabled: bool,
    pub attempted: usize,
    pub import_ok: usize,
    pub import_err: usize,
    pub panics: usize,
    pub pzfx_import_ok: usize,
    pub pzfx_import_err: usize,
    pub pzfx_valid_import_err: usize,
    pub pzfx_known_corrupt_err: usize,
    pub pzfx_safety_cap_err: usize,
    pub pzfx_panics: usize,
    pub archive_import_ok: usize,
    pub archive_import_err: usize,
    pub archive_valid_import_err: usize,
    pub archive_known_corrupt_err: usize,
    pub archive_safety_cap_err: usize,
    pub archive_panics: usize,
    pub parse_failure_classes: BTreeMap<String, usize>,
    pub parse_failure_subclasses: BTreeMap<String, usize>,
    pub corrupt_file_classes: BTreeMap<String, usize>,
    pub safety_cap_refusal_classes: BTreeMap<String, usize>,
    pub warning_kinds: BTreeMap<String, usize>,
    pub dropped_sections: BTreeMap<String, usize>,
    pub archetypes: BTreeMap<String, usize>,
    pub subcolumn_modes: BTreeMap<String, usize>,
    pub engine_recalc_ok: usize,
    pub engine_recalc_err: usize,
    pub curvebook_roundtrip_ok: usize,
    pub curvebook_roundtrip_err: usize,
    pub curvebook_roundtrip_unexpected_err: usize,
    pub curvebook_roundtrip_safety_cap_err: usize,
    pub curvebook_roundtrip_err_classes: BTreeMap<String, usize>,
    pub curvebook_roundtrip_diffs: usize,
    pub pzfx_export_ok: usize,
    pub pzfx_export_err: usize,
    pub pzfx_export_panics: usize,
    pub pzfx_export_warning_kinds: BTreeMap<String, usize>,
    pub pzfx_export_dropped_kinds: BTreeMap<String, usize>,
    pub pzfx_export_insufficient_features: BTreeMap<String, usize>,
    pub pzfx_reimport_ok: usize,
    pub pzfx_reimport_err: usize,
    pub pzfx_reimport_err_classes: BTreeMap<String, usize>,
    pub pzfx_reimport_panics: usize,
    pub pzfx_reexport_identical: usize,
    pub pzfx_reexport_diff_files: usize,
    pub pzfx_reexport_diffs_total: usize,
    pub pzfx_reexport_tables_compared: usize,
    pub pzfx_reexport_cells_compared: usize,
    pub pzfx_reexport_info_sheets_compared: usize,
    pub pzfx_reexport_diff_classes: BTreeMap<String, usize>,
    pub pzfx_reexport_diff_subclasses: BTreeMap<String, usize>,
    pub archive_fidelity_files_checked: usize,
    pub archive_fidelity_tables_checked: usize,
    pub archive_fidelity_cells_checked: usize,
    pub archive_fidelity_tables_skipped_with_warning: usize,
    pub archive_fidelity_diff_files: usize,
    pub archive_fidelity_diffs_total: usize,
    pub archive_fidelity_diff_classes: BTreeMap<String, usize>,
    pub pzfx_conformance: PzfxConformanceSummary,
    pub archive_analysis_diagnostics: BTreeMap<String, AnalysisClassDiagnosticSummary>,
    pub import_ms_dist: SizeDistribution,
}

impl EnginePipelineSummary {
    fn to_json(&self) -> Value {
        let diag_map: Map<String, Value> = self.archive_analysis_diagnostics.iter().map(|(k, v)| (k.clone(), v.to_json())).collect();
        let mut map = Map::new();
        map.insert(String::from("enabled"), json!(self.enabled));
        map.insert(String::from("attempted"), json!(self.attempted));
        map.insert(String::from("import_ok"), json!(self.import_ok));
        map.insert(String::from("import_err"), json!(self.import_err));
        map.insert(String::from("panics"), json!(self.panics));
        map.insert(
            String::from("pzfx_import"),
            json!({
                "ok": self.pzfx_import_ok,
                "err": self.pzfx_import_err,
                "valid_unexpected_err": self.pzfx_valid_import_err,
                "known_corrupt_err": self.pzfx_known_corrupt_err,
                "safety_cap_refused_err": self.pzfx_safety_cap_err,
                "panics": self.pzfx_panics,
            }),
        );
        map.insert(
            String::from("archive_import"),
            json!({
                "ok": self.archive_import_ok,
                "err": self.archive_import_err,
                "valid_unexpected_err": self.archive_valid_import_err,
                "known_corrupt_err": self.archive_known_corrupt_err,
                "safety_cap_refused_err": self.archive_safety_cap_err,
                "panics": self.archive_panics,
            }),
        );
        map.insert(String::from("parse_failure_classes"), json!(self.parse_failure_classes));
        map.insert(String::from("parse_failure_subclasses"), json!(self.parse_failure_subclasses));
        map.insert(String::from("corrupt_file_classes"), json!(self.corrupt_file_classes));
        map.insert(String::from("safety_cap_refusal_classes"), json!(self.safety_cap_refusal_classes));
        map.insert(String::from("warning_kinds"), json!(self.warning_kinds));
        map.insert(String::from("dropped_sections"), json!(self.dropped_sections));
        map.insert(String::from("archetypes"), json!(self.archetypes));
        map.insert(String::from("subcolumn_modes"), json!(self.subcolumn_modes));
        map.insert(String::from("engine_recalc_ok"), json!(self.engine_recalc_ok));
        map.insert(String::from("engine_recalc_err"), json!(self.engine_recalc_err));
        map.insert(String::from("curvebook_roundtrip_ok"), json!(self.curvebook_roundtrip_ok));
        map.insert(String::from("curvebook_roundtrip_err"), json!(self.curvebook_roundtrip_err));
        map.insert(String::from("curvebook_roundtrip_unexpected_err"), json!(self.curvebook_roundtrip_unexpected_err));
        map.insert(String::from("curvebook_roundtrip_safety_cap_err"), json!(self.curvebook_roundtrip_safety_cap_err));
        map.insert(String::from("curvebook_roundtrip_err_classes"), json!(self.curvebook_roundtrip_err_classes));
        map.insert(String::from("curvebook_roundtrip_diffs"), json!(self.curvebook_roundtrip_diffs));
        map.insert(
            String::from("pzfx_reexport_fidelity"),
            json!({
                "export_ok": self.pzfx_export_ok,
                "export_err": self.pzfx_export_err,
                "export_panics": self.pzfx_export_panics,
                "export_warning_kinds": self.pzfx_export_warning_kinds,
                "export_dropped_kinds": self.pzfx_export_dropped_kinds,
                "export_insufficient_features": self.pzfx_export_insufficient_features,
                "reimport_ok": self.pzfx_reimport_ok,
                "reimport_err": self.pzfx_reimport_err,
                "reimport_err_classes": self.pzfx_reimport_err_classes,
                "reimport_panics": self.pzfx_reimport_panics,
                "identical_files": self.pzfx_reexport_identical,
                "diff_files": self.pzfx_reexport_diff_files,
                "diffs_total": self.pzfx_reexport_diffs_total,
                "tables_compared": self.pzfx_reexport_tables_compared,
                "cells_compared": self.pzfx_reexport_cells_compared,
                "info_sheets_compared": self.pzfx_reexport_info_sheets_compared,
                "diff_classes": self.pzfx_reexport_diff_classes,
                "diff_subclasses": self.pzfx_reexport_diff_subclasses,
            }),
        );
        map.insert(
            String::from("archive_data_layer_fidelity"),
            json!({
                "files_checked": self.archive_fidelity_files_checked,
                "tables_checked": self.archive_fidelity_tables_checked,
                "cells_checked": self.archive_fidelity_cells_checked,
                "tables_skipped_with_warning": self.archive_fidelity_tables_skipped_with_warning,
                "diff_files": self.archive_fidelity_diff_files,
                "diffs_total": self.archive_fidelity_diffs_total,
                "diff_classes": self.archive_fidelity_diff_classes,
            }),
        );
        map.insert(String::from("pzfx_conformance"), self.pzfx_conformance.to_json());
        map.insert(String::from("archive_analysis_diagnostics"), Value::Object(diag_map));
        map.insert(String::from("import_duration_ms"), self.import_ms_dist.to_json());
        Value::Object(map)
    }
}

/// Complete aggregate report emitted by `run_with_options` (contains ZERO filenames, titles, cell
/// values, or vendor strings).
#[derive(Debug, Clone, Default)]
pub struct CorpusRunSummary {
    pub observed_at: String,
    pub provenance_present: bool,
    pub provenance_verified: usize,
    pub provenance_missing: usize,
    pub provenance_hash_mismatch: usize,
    pub provenance_size_mismatch: usize,
    pub provenance_missing_on_disk: usize,
    pub skipped_dotfiles: usize,
    pub skipped_manifests_and_logs: usize,
    pub skipped_partial_downloads: usize,
    pub skipped_other_non_format: usize,
    pub read_errors: usize,
    pub counts_by_source: BTreeMap<String, usize>,
    pub counts_by_licence: BTreeMap<String, usize>,
    pub counts_by_format: BTreeMap<String, usize>,
    pub pcff_binary_total: usize,
    pub pcff_with_pzfx_ext: usize,
    pub pcff_with_archive_ext: usize,
    pub pzf_ext_total: usize,
    pub pzfx_xml_total: usize,
    pub archive_total: usize,
    pub pzfx_size_dist: SizeDistribution,
    pub pcff_size_dist: SizeDistribution,
    pub archive_size_dist: SizeDistribution,
    pub archive_entry_count_dist: SizeDistribution,
    pub pzfx_encodings: BTreeMap<String, usize>,
    pub pzfx_xml_versions: BTreeMap<String, usize>,
    pub pzfx_root_shapes: BTreeMap<String, usize>,
    pub pzfx_parse_errors: BTreeMap<String, usize>,
    pub pzfx_with_doctype: usize,
    pub pzfx_with_trailing_non_ws: usize,
    pub pzfx_trailing_classes: BTreeMap<String, usize>,
    pub pzfx_trailing_bytes_dist: SizeDistribution,
    pub pzfx_with_template: usize,
    pub pzfx_template_elements_total: usize,
    pub pzfx_template_body_classes: BTreeMap<String, usize>,
    pub pzfx_template_bytes_dist: SizeDistribution,
    pub pzfx_tables_total: usize,
    pub pzfx_huge_tables_total: usize,
    pub pzfx_files_with_huge_table: usize,
    pub pzfx_huge_table_min_ycols: Option<usize>,
    pub pzfx_huge_table_max_ycols: usize,
    pub pzfx_huge_table_min_subcols: Option<usize>,
    pub pzfx_huge_table_max_subcols: usize,
    pub pzfx_huge_table_types: BTreeMap<String, usize>,
    pub pzfx_max_regular_ycols: usize,
    pub pzfx_max_regular_subcols: usize,
    pub pzfx_table_types: BTreeMap<String, usize>,
    pub pzfx_y_formats: BTreeMap<String, usize>,
    pub pzfx_y_format_subcols: BTreeMap<String, usize>,
    pub pzfx_x_formats: BTreeMap<String, usize>,
    pub pzfx_x_ev_formats: BTreeMap<String, usize>,
    pub pzfx_x_ev_subcols: BTreeMap<String, usize>,
    pub pzfx_table_seq_refs: usize,
    pub pzfx_table_seq_direct_tables: usize,
    pub pzfx_table_seq_selected: BTreeMap<String, usize>,
    pub pzfx_info_seq_refs: usize,
    pub pzfx_info_seq_direct_infos: usize,
    pub pzfx_info_seq_selected: BTreeMap<String, usize>,
    pub pzfx_color_attrs: BTreeMap<String, usize>,
    pub pzfx_x_prog_attrs: BTreeMap<String, usize>,
    pub pzfx_excluded_values: BTreeMap<String, usize>,
    pub pzfx_rich_text_tags: BTreeMap<String, usize>,
    pub pzfx_files_with_symbol_font: usize,
    pub pzfx_files_with_subcol_titles: usize,
    pub pzfx_files_with_decimals: usize,
    pub pzfx_info_constant_shapes: BTreeMap<String, usize>,
    pub pzfx_cv_diagnostics: CvDiagnosticStats,
    pub archive_parse_errors: BTreeMap<String, usize>,
    pub archive_zip64_count: usize,
    pub archive_backslash_count: usize,
    pub archive_non_utf8_count: usize,
    pub archive_compression_methods: BTreeMap<u16, usize>,
    pub archive_csv_entries_total: usize,
    pub archive_json_entries_total: usize,
    pub archive_image_entries_total: usize,
    pub archive_xml_entries_total: usize,
    pub archive_opaque_entries_total: usize,
    pub archive_files_with_opaque: usize,
    pub archive_opaque_extensions: BTreeMap<String, usize>,
    pub archive_path_templates: BTreeMap<String, usize>,
    pub archive_dialects: BTreeMap<String, usize>,
    pub archive_app_versions: BTreeMap<String, usize>,
    pub archive_major_versions: BTreeMap<u32, usize>,
    pub archive_version_keys: BTreeMap<String, BTreeMap<String, usize>>,
    pub archive_document_json_keys: BTreeMap<String, usize>,
    pub archive_sheets_subkeys: BTreeMap<String, usize>,
    pub archive_sheet_attrs_map_keys: BTreeMap<String, usize>,
    pub archive_role_json_keys: BTreeMap<String, BTreeMap<String, usize>>,
    pub archive_data_formats: BTreeMap<String, usize>,
    pub archive_table_types: BTreeMap<String, usize>,
    pub archive_analysis_classes: BTreeMap<String, usize>,
    pub archive_csv_header_probe: BTreeMap<String, usize>,
    pub archive_x_vs_row_titles_probe: BTreeMap<String, usize>,
    pub archive_analysis_result_shapes: BTreeMap<String, usize>,
    pub archive_files_with_analysis_params: usize,
    pub archive_files_with_graph_previews: usize,
    pub archive_files_with_exclusions: usize,
    pub archive_files_with_info_sheets: usize,
    pub archive_high_low_probe: HighLowOffsetProbe,
    pub archive_cv_probe: CvDiagnosticStats,
    pub archive_gate_met: bool,
    pub pipeline: EnginePipelineSummary,
}

impl CorpusRunSummary {
    fn to_json(&self) -> Value {
        json!({
            "observed_at": self.observed_at,
            "provenance": {
                "present": self.provenance_present,
                "verified": self.provenance_verified,
                "missing": self.provenance_missing,
                "hash_mismatch": self.provenance_hash_mismatch,
                "size_mismatch": self.provenance_size_mismatch,
                "missing_on_disk": self.provenance_missing_on_disk,
            },
            "skipped": {
                "dotfiles": self.skipped_dotfiles,
                "manifests_and_logs": self.skipped_manifests_and_logs,
                "partial_downloads": self.skipped_partial_downloads,
                "other_non_format": self.skipped_other_non_format,
                "read_errors": self.read_errors,
            },
            "counts": {
                "by_source": self.counts_by_source,
                "by_licence": self.counts_by_licence,
                "by_format": self.counts_by_format,
                "pzfx_xml": self.pzfx_xml_total,
                "pcff_binary_unsupported_by_design": self.pcff_binary_total,
                "pcff_with_pzfx_ext": self.pcff_with_pzfx_ext,
                "pcff_with_archive_ext": self.pcff_with_archive_ext,
                "pzf_ext_total": self.pzf_ext_total,
                "archive_containers": self.archive_total,
            },
            "distributions": {
                "pzfx_bytes": self.pzfx_size_dist.to_json(),
                "pcff_bytes": self.pcff_size_dist.to_json(),
                "archive_bytes": self.archive_size_dist.to_json(),
                "archive_entry_counts": self.archive_entry_count_dist.to_json(),
            },
            "pzfx_observations": self.pzfx_observations_json(),
            "archive_observations": self.archive_observations_json(),
            "curvebook_pipeline": self.pipeline.to_json(),
        })
    }

    fn pzfx_observations_json(&self) -> Value {
        let mut map = Map::new();
        map.insert(String::from("encodings"), json!(self.pzfx_encodings));
        map.insert(String::from("xml_schema_versions"), json!(self.pzfx_xml_versions));
        map.insert(String::from("root_shapes"), json!(self.pzfx_root_shapes));
        map.insert(String::from("parse_errors"), json!(self.pzfx_parse_errors));
        map.insert(String::from("with_doctype"), json!(self.pzfx_with_doctype));
        map.insert(String::from("with_trailing_non_ws"), json!(self.pzfx_with_trailing_non_ws));
        map.insert(String::from("trailing_classes"), json!(self.pzfx_trailing_classes));
        map.insert(String::from("trailing_non_ws_bytes"), self.pzfx_trailing_bytes_dist.to_json());
        map.insert(String::from("with_template"), json!(self.pzfx_with_template));
        map.insert(String::from("template_elements_total"), json!(self.pzfx_template_elements_total));
        map.insert(String::from("template_body_classes"), json!(self.pzfx_template_body_classes));
        map.insert(String::from("template_bytes"), self.pzfx_template_bytes_dist.to_json());
        map.insert(String::from("tables_total"), json!(self.pzfx_tables_total));
        map.insert(String::from("huge_tables_total"), json!(self.pzfx_huge_tables_total));
        map.insert(String::from("files_with_huge_table"), json!(self.pzfx_files_with_huge_table));
        map.insert(String::from("huge_table_ycols_range"), json!([self.pzfx_huge_table_min_ycols, self.pzfx_huge_table_max_ycols]));
        map.insert(String::from("huge_table_subcols_range"), json!([self.pzfx_huge_table_min_subcols, self.pzfx_huge_table_max_subcols]));
        map.insert(String::from("huge_table_types"), json!(self.pzfx_huge_table_types));
        map.insert(String::from("regular_table_max_ycols"), json!(self.pzfx_max_regular_ycols));
        map.insert(String::from("regular_table_max_subcols"), json!(self.pzfx_max_regular_subcols));
        map.insert(String::from("table_types"), json!(self.pzfx_table_types));
        map.insert(String::from("y_formats"), json!(self.pzfx_y_formats));
        map.insert(String::from("y_format_subcols"), json!(self.pzfx_y_format_subcols));
        map.insert(String::from("x_formats"), json!(self.pzfx_x_formats));
        map.insert(String::from("x_ev_formats"), json!(self.pzfx_x_ev_formats));
        map.insert(String::from("x_ev_subcols"), json!(self.pzfx_x_ev_subcols));
        map.insert(String::from("table_seq_refs"), json!(self.pzfx_table_seq_refs));
        map.insert(String::from("table_seq_direct_tables"), json!(self.pzfx_table_seq_direct_tables));
        map.insert(String::from("table_seq_selected"), json!(self.pzfx_table_seq_selected));
        map.insert(String::from("info_seq_refs"), json!(self.pzfx_info_seq_refs));
        map.insert(String::from("info_seq_direct_infos"), json!(self.pzfx_info_seq_direct_infos));
        map.insert(String::from("info_seq_selected"), json!(self.pzfx_info_seq_selected));
        map.insert(String::from("color_attrs"), json!(self.pzfx_color_attrs));
        map.insert(String::from("x_progression_attrs"), json!(self.pzfx_x_prog_attrs));
        map.insert(String::from("excluded_attr_values"), json!(self.pzfx_excluded_values));
        map.insert(String::from("rich_text_tags"), json!(self.pzfx_rich_text_tags));
        map.insert(String::from("files_with_symbol_font"), json!(self.pzfx_files_with_symbol_font));
        map.insert(String::from("files_with_subcolumn_titles"), json!(self.pzfx_files_with_subcol_titles));
        map.insert(String::from("files_with_decimals_attr"), json!(self.pzfx_files_with_decimals));
        map.insert(String::from("info_constant_shapes"), json!(self.pzfx_info_constant_shapes));
        map.insert(String::from("cv_2subcol_diagnostics"), self.pzfx_cv_diagnostics.to_json());
        Value::Object(map)
    }

    fn archive_observations_json(&self) -> Value {
        let mut map = Map::new();
        map.insert(String::from("gate_1_3_met"), json!(self.archive_gate_met));
        map.insert(String::from("gate_min_files"), json!(MIN_ARCHIVE_GATE_FILES));
        map.insert(String::from("gate_min_major_versions"), json!(MIN_ARCHIVE_GATE_MAJOR_VERSIONS));
        map.insert(String::from("parse_errors"), json!(self.archive_parse_errors));
        map.insert(String::from("zip64_files"), json!(self.archive_zip64_count));
        map.insert(String::from("backslash_path_files"), json!(self.archive_backslash_count));
        map.insert(String::from("non_utf8_name_files"), json!(self.archive_non_utf8_count));
        map.insert(String::from("compression_methods"), json!(self.archive_compression_methods));
        map.insert(String::from("csv_entries_total"), json!(self.archive_csv_entries_total));
        map.insert(String::from("json_entries_total"), json!(self.archive_json_entries_total));
        map.insert(String::from("image_entries_total"), json!(self.archive_image_entries_total));
        map.insert(String::from("xml_entries_total"), json!(self.archive_xml_entries_total));
        map.insert(String::from("opaque_entries_total"), json!(self.archive_opaque_entries_total));
        map.insert(String::from("files_with_opaque_entries"), json!(self.archive_files_with_opaque));
        map.insert(String::from("opaque_extensions"), json!(self.archive_opaque_extensions));
        map.insert(String::from("path_templates"), json!(self.archive_path_templates));
        map.insert(String::from("dialects"), json!(self.archive_dialects));
        map.insert(String::from("app_versions"), json!(self.archive_app_versions));
        map.insert(String::from("major_versions"), json!(self.archive_major_versions));
        map.insert(String::from("version_keys"), json!(self.archive_version_keys));
        map.insert(String::from("document_json_keys"), json!(self.archive_document_json_keys));
        map.insert(String::from("sheets_subkeys"), json!(self.archive_sheets_subkeys));
        map.insert(String::from("sheet_attrs_map_keys"), json!(self.archive_sheet_attrs_map_keys));
        map.insert(String::from("role_json_keys"), json!(self.archive_role_json_keys));
        map.insert(String::from("data_formats"), json!(self.archive_data_formats));
        map.insert(String::from("table_types"), json!(self.archive_table_types));
        map.insert(String::from("analysis_classes"), json!(self.archive_analysis_classes));
        map.insert(String::from("csv_header_probe"), json!(self.archive_csv_header_probe));
        map.insert(String::from("x_vs_row_titles_probe"), json!(self.archive_x_vs_row_titles_probe));
        map.insert(String::from("analysis_result_shapes"), json!(self.archive_analysis_result_shapes));
        map.insert(String::from("files_with_analysis_params_json"), json!(self.archive_files_with_analysis_params));
        map.insert(String::from("files_with_graph_previews"), json!(self.archive_files_with_graph_previews));
        map.insert(String::from("files_with_exclusions_key"), json!(self.archive_files_with_exclusions));
        map.insert(String::from("files_with_info_sheets"), json!(self.archive_files_with_info_sheets));
        map.insert(String::from("high_low_probe"), self.archive_high_low_probe.to_json());
        map.insert(String::from("cv_probe"), self.archive_cv_probe.to_json());
        Value::Object(map)
    }
}

/// Execute the corpus verification, structural observation, and optional CurveBook pipeline.
pub fn run_with_options(workspace_root: &Path, opts: &InteropCorpusOptions) -> Result<CorpusRunSummary, String> {
    let corpus_dir = resolve_corpus_dir(workspace_root, opts);
    let canon_corpus = validate_corpus_root(workspace_root, &corpus_dir)?;
    let ledger_dir = if opts.observe {
        let dir = resolve_ledger_dir(workspace_root, opts);
        validate_ledger_dir(workspace_root, &dir)?;
        Some(dir)
    } else {
        None
    };

    let now_secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let observed_at = format!("unix:{now_secs}");

    let prov_path = canon_corpus.join("provenance.tsv");
    let (provenance_present, provenance_map) = if prov_path.is_file() {
        let raw = read_bounded_bytes(&prov_path, 64 * 1024 * 1024)?;
        let text = std::str::from_utf8(&raw).map_err(|e| format!("provenance.tsv is not valid UTF-8: {e}"))?;
        (true, parse_provenance_tsv(text)?)
    } else {
        (false, BTreeMap::new())
    };

    let discovered = discover_corpus_files(&canon_corpus)?;
    let mut summary = CorpusRunSummary {
        observed_at: observed_at.clone(),
        provenance_present,
        skipped_dotfiles: discovered.skipped_dotfiles,
        skipped_manifests_and_logs: discovered.skipped_manifests_and_logs,
        skipped_partial_downloads: discovered.skipped_partial_downloads,
        skipped_other_non_format: discovered.skipped_other_non_format,
        ..CorpusRunSummary::default()
    };

    let mut seen_rel_paths = BTreeSet::new();
    let mut pzfx_sizes = Vec::new();
    let mut pcff_sizes = Vec::new();
    let mut archive_sizes = Vec::new();
    let mut archive_entries = Vec::new();
    let mut trailing_sizes = Vec::new();
    let mut template_sizes = Vec::new();
    let mut import_durations_ms = Vec::new();

    let mut pzfx_obs_records = Vec::new();
    let mut archive_obs_records = Vec::new();
    let mut pzfx_profile_builder = DialectProfileBuilder::new(".pzfx");
    let mut archive_profile_builder = DialectProfileBuilder::new(".prism");

    #[cfg(feature = "docs-site")]
    let mut reexported_pzfx_candidates: Vec<(CandidateStructure, BTreeSet<String>)> = Vec::new();

    #[cfg(feature = "docs-site")]
    {
        summary.pipeline.enabled = !opts.structural_only;
        if summary.pipeline.enabled {
            seed_analysis_mapping_diagnostics(&mut summary.pipeline.archive_analysis_diagnostics);
        }
    }

    for item in &discovered.files {
        seen_rel_paths.insert(item.rel_path.clone());
        let bytes = match read_bounded_bytes(&item.abs_path, MAX_CORPUS_FILE_BYTES) {
            Ok(b) => b,
            Err(_) => {
                summary.read_errors += 1;
                continue;
            }
        };
        let actual_sha = sha256_hex(&bytes);
        let file_id = format!("sha256:{actual_sha}");
        let (source, record_id, licence) = match provenance_map.get(&item.rel_path) {
            Some(entry) => {
                let mut ok = true;
                if actual_sha != entry.expected_sha256 {
                    summary.provenance_hash_mismatch += 1;
                    ok = false;
                }
                if entry.expected_bytes.is_some_and(|exp| exp != bytes.len() as u64) {
                    summary.provenance_size_mismatch += 1;
                    ok = false;
                }
                if ok {
                    summary.provenance_verified += 1;
                }
                (entry.source.as_str(), entry.record_id.as_str(), entry.licence.as_str())
            }
            None => {
                summary.provenance_missing += 1;
                ("unrecorded", "unrecorded:unknown", "unrecorded")
            }
        };

        *summary.counts_by_source.entry(source.to_owned()).or_insert(0) += 1;
        *summary.counts_by_licence.entry(licence.to_owned()).or_insert(0) += 1;

        match item.declared_format {
            InteropFormat::BinaryPzf => {
                summary.pcff_binary_total += 1;
                summary.pzf_ext_total += 1;
                pcff_sizes.push(bytes.len() as u64);
                *summary.counts_by_format.entry(InteropFormat::BinaryPzf.label().to_owned()).or_insert(0) += 1;
            }
            InteropFormat::Pzfx => {
                if bytes.starts_with(b"PCFF") {
                    summary.pcff_binary_total += 1;
                    summary.pcff_with_pzfx_ext += 1;
                    pcff_sizes.push(bytes.len() as u64);
                    *summary.counts_by_format.entry(InteropFormat::BinaryPzf.label().to_owned()).or_insert(0) += 1;
                    continue;
                }
                summary.pzfx_xml_total += 1;
                pzfx_sizes.push(bytes.len() as u64);
                *summary.counts_by_format.entry(InteropFormat::Pzfx.label().to_owned()).or_insert(0) += 1;
                let mut pzfx_known_corrupt = false;
                if let Some(obs) = observe_pzfx_bytes(&bytes, &file_id, &observed_at, source, record_id, licence) {
                    pzfx_known_corrupt = obs.parse_error_class.is_some();
                    fold_pzfx_observation(&obs, &mut summary, &mut trailing_sizes, &mut template_sizes, &mut pzfx_profile_builder);
                    if ledger_dir.is_some() {
                        pzfx_obs_records.push(obs.to_ledger_json());
                    }
                }
                let _ = pzfx_known_corrupt;
                #[cfg(feature = "docs-site")]
                if !opts.structural_only {
                    run_engine_pipeline_for_file(
                        InteropFormat::Pzfx,
                        &bytes,
                        pzfx_known_corrupt,
                        &mut summary.pipeline,
                        &mut import_durations_ms,
                        &mut reexported_pzfx_candidates,
                    );
                }
            }
            InteropFormat::ZipArchive => {
                if bytes.starts_with(b"PCFF") {
                    summary.pcff_binary_total += 1;
                    summary.pcff_with_archive_ext += 1;
                    pcff_sizes.push(bytes.len() as u64);
                    *summary.counts_by_format.entry(InteropFormat::BinaryPzf.label().to_owned()).or_insert(0) += 1;
                    continue;
                }
                summary.archive_total += 1;
                archive_sizes.push(bytes.len() as u64);
                *summary.counts_by_format.entry(InteropFormat::ZipArchive.label().to_owned()).or_insert(0) += 1;
                let obs = observe_archive_bytes(&bytes, &file_id, &observed_at, source, record_id, licence);
                let archive_known_corrupt =
                    obs.parse_error_class.is_some() || obs.dialect == "invalid_document_json" || obs.dialect.starts_with("unreadable_document_json:");
                archive_entries.push(obs.entry_count as u64);
                fold_archive_observation(&obs, &mut summary, &mut archive_profile_builder);
                if ledger_dir.is_some() {
                    archive_obs_records.push(obs.to_ledger_json());
                }
                let _ = archive_known_corrupt;
                #[cfg(feature = "docs-site")]
                if !opts.structural_only {
                    run_engine_pipeline_for_file(
                        InteropFormat::ZipArchive,
                        &bytes,
                        archive_known_corrupt,
                        &mut summary.pipeline,
                        &mut import_durations_ms,
                        &mut reexported_pzfx_candidates,
                    );
                }
            }
        }
    }

    for prov_rel in provenance_map.keys() {
        let ext = lower_ext(prov_rel);
        if matches!(ext.as_str(), "pzfx" | "prism" | "pzf") && !seen_rel_paths.contains(prov_rel) {
            summary.provenance_missing_on_disk += 1;
        }
    }

    summary.pzfx_size_dist = SizeDistribution::from_samples(&mut pzfx_sizes);
    summary.pcff_size_dist = SizeDistribution::from_samples(&mut pcff_sizes);
    summary.archive_size_dist = SizeDistribution::from_samples(&mut archive_sizes);
    summary.archive_entry_count_dist = SizeDistribution::from_samples(&mut archive_entries);
    summary.pzfx_trailing_bytes_dist = SizeDistribution::from_samples(&mut trailing_sizes);
    summary.pzfx_template_bytes_dist = SizeDistribution::from_samples(&mut template_sizes);
    summary.pipeline.import_ms_dist = SizeDistribution::from_samples(&mut import_durations_ms);
    summary.archive_gate_met =
        summary.archive_total >= MIN_ARCHIVE_GATE_FILES && summary.archive_major_versions.len() >= MIN_ARCHIVE_GATE_MAJOR_VERSIONS;

    let pzfx_profile = pzfx_profile_builder.build();
    let archive_profile = archive_profile_builder.build();

    #[cfg(feature = "docs-site")]
    if !opts.structural_only {
        evaluate_pzfx_corpus_conformance(&pzfx_profile, &reexported_pzfx_candidates, &mut summary.pipeline.pzfx_conformance);
    }

    if let Some(dir) = ledger_dir {
        write_observation_ledger(&dir, &summary, &pzfx_obs_records, &archive_obs_records, &pzfx_profile, &archive_profile)?;
    }

    Ok(summary)
}

fn fold_pzfx_observation(
    obs: &PzfxObservation,
    summary: &mut CorpusRunSummary,
    trailing_sizes: &mut Vec<u64>,
    template_sizes: &mut Vec<u64>,
    profile_builder: &mut DialectProfileBuilder,
) {
    *summary.pzfx_encodings.entry(obs.encoding.clone()).or_insert(0) += 1;
    let ver = obs.xml_schema_version.clone().unwrap_or_else(|| String::from("<missing>"));
    *summary.pzfx_xml_versions.entry(ver).or_insert(0) += 1;
    *summary.pzfx_root_shapes.entry(obs.root_shape.clone()).or_insert(0) += 1;
    if let Some(err) = &obs.parse_error_class {
        *summary.pzfx_parse_errors.entry(err.clone()).or_insert(0) += 1;
    }
    if obs.has_doctype {
        summary.pzfx_with_doctype += 1;
    }
    if obs.trailing_non_ws_bytes > 0 {
        summary.pzfx_with_trailing_non_ws += 1;
        trailing_sizes.push(obs.trailing_non_ws_bytes as u64);
    }
    *summary.pzfx_trailing_classes.entry(obs.trailing_class.clone()).or_insert(0) += 1;
    if obs.template_elements > 0 {
        summary.pzfx_with_template += 1;
        summary.pzfx_template_elements_total += obs.template_elements;
        template_sizes.push(obs.template_total_bytes as u64);
    }
    merge_counts(&mut summary.pzfx_template_body_classes, &obs.template_body_classes);
    summary.pzfx_tables_total += obs.table_count;
    summary.pzfx_huge_tables_total += obs.huge_table_count;
    if obs.huge_table_count > 0 {
        summary.pzfx_files_with_huge_table += 1;
    }
    for (ycols, subcols, _rows, tt) in &obs.huge_table_dims {
        summary.pzfx_huge_table_min_ycols = Some(summary.pzfx_huge_table_min_ycols.map_or(*ycols, |m| m.min(*ycols)));
        summary.pzfx_huge_table_max_ycols = summary.pzfx_huge_table_max_ycols.max(*ycols);
        summary.pzfx_huge_table_min_subcols = Some(summary.pzfx_huge_table_min_subcols.map_or(*subcols, |m| m.min(*subcols)));
        summary.pzfx_huge_table_max_subcols = summary.pzfx_huge_table_max_subcols.max(*subcols);
        *summary.pzfx_huge_table_types.entry(tt.clone()).or_insert(0) += 1;
    }
    summary.pzfx_max_regular_ycols = summary.pzfx_max_regular_ycols.max(obs.max_regular_table_ycols);
    summary.pzfx_max_regular_subcols = summary.pzfx_max_regular_subcols.max(obs.max_regular_table_subcols);
    merge_counts(&mut summary.pzfx_table_types, &obs.table_type_hist);
    merge_counts(&mut summary.pzfx_y_formats, &obs.y_format_hist);
    merge_counts(&mut summary.pzfx_y_format_subcols, &obs.y_format_subcols_hist);
    merge_counts(&mut summary.pzfx_x_formats, &obs.x_format_hist);
    merge_counts(&mut summary.pzfx_x_ev_formats, &obs.x_ev_format_hist);
    merge_counts(&mut summary.pzfx_x_ev_subcols, &obs.x_ev_subcols_hist);
    summary.pzfx_table_seq_refs += obs.table_seq_ref_count;
    summary.pzfx_table_seq_direct_tables += obs.table_seq_direct_table_count;
    merge_counts(&mut summary.pzfx_table_seq_selected, &obs.table_seq_selected_values);
    summary.pzfx_info_seq_refs += obs.info_seq_ref_count;
    summary.pzfx_info_seq_direct_infos += obs.info_seq_direct_info_count;
    merge_counts(&mut summary.pzfx_info_seq_selected, &obs.info_seq_selected_values);
    merge_counts(&mut summary.pzfx_color_attrs, &obs.color_attr_names);
    merge_counts(&mut summary.pzfx_x_prog_attrs, &obs.x_progression_attr_names);
    merge_counts(&mut summary.pzfx_excluded_values, &obs.excluded_attr_values);
    merge_counts(&mut summary.pzfx_rich_text_tags, &obs.rich_text_tags);
    if obs.has_symbol_font {
        summary.pzfx_files_with_symbol_font += 1;
    }
    if obs.has_subcolumn_titles {
        summary.pzfx_files_with_subcol_titles += 1;
    }
    if obs.has_decimals_attr {
        summary.pzfx_files_with_decimals += 1;
    }
    merge_counts(&mut summary.pzfx_info_constant_shapes, &obs.info_constant_shapes);
    summary.pzfx_cv_diagnostics.merge(&obs.cv_diagnostics);

    if obs.parse_error_class.is_none() && obs.root_shape != "xslt_wrapped_root" {
        let features = pzfx_observation_feature_set(obs);
        profile_builder.record_file(&obs.file_id, &obs.record_id, &features, &obs.parent_child_orders);
    }
}

fn pzfx_observation_feature_set(obs: &PzfxObservation) -> BTreeSet<String> {
    let mut features = BTreeSet::new();
    for (parent, child) in &obs.parent_child_pairs {
        features.insert(format!("elem:{parent}/{child}"));
    }
    for (elem, attrs) in &obs.element_attrs {
        for attr in attrs {
            features.insert(format!("attr:{elem}@{attr}"));
        }
    }
    for tt in obs.table_type_hist.keys() {
        features.insert(format!("table_type:{tt}"));
    }
    for yf in obs.y_format_subcols_hist.keys() {
        features.insert(format!("y_format:{yf}"));
    }
    features
}

/// Convert a parsed [`PzfxObservation`] into a [`CandidateStructure`] for §6.3 conformance checks.
#[must_use]
pub fn candidate_from_pzfx_observation(obs: &PzfxObservation) -> CandidateStructure {
    CandidateStructure {
        emitted_features: pzfx_observation_feature_set(obs),
        parent_child_orders: obs.parent_child_orders.clone(),
        coverage_features: BTreeSet::new(),
    }
}

/// Normalize a `PzfxDataLayerDifference::location` into a structural-only class with all sheet IDs
/// and numeric indices stripped so zero user identifiers leak into summaries or ledgers.
#[must_use]
pub fn classify_pzfx_diff_location(location: &str) -> String {
    let mut out = String::with_capacity(location.len());
    let mut depth = 0usize;
    for ch in location.chars() {
        match ch {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    if out.is_empty() { String::from("unknown") } else { out }
}

#[cfg(feature = "docs-site")]
fn extract_backtick_feature(detail: &str) -> Option<String> {
    let rest = detail.strip_prefix("feature `")?;
    let end = rest.find('`')?;
    rest.get(..end).filter(|s| !s.is_empty()).map(str::to_owned)
}

#[cfg(feature = "docs-site")]
fn classify_blocking_option(raw: &str) -> String {
    if raw.starts_with("unmapped nonlinear equation ") {
        return String::from("unmapped_nonlinear_equation");
    }
    if let Some((prefix, rest)) = raw.split_once('`')
        && let Some((tok, _)) = rest.split_once('`')
    {
        let clean_prefix = prefix.trim().replace(' ', "_");
        let is_enum = !tok.is_empty() && tok.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        return if is_enum { format!("{clean_prefix}:{tok}") } else { format!("{clean_prefix}:<other>") };
    }
    raw.trim().replace(' ', "_")
}

fn fold_archive_observation(obs: &ArchiveObservation, summary: &mut CorpusRunSummary, profile_builder: &mut DialectProfileBuilder) {
    if let Some(err) = &obs.parse_error_class {
        *summary.archive_parse_errors.entry(err.clone()).or_insert(0) += 1;
        return;
    }
    if obs.uses_zip64 {
        summary.archive_zip64_count += 1;
    }
    if obs.has_backslash_paths {
        summary.archive_backslash_count += 1;
    }
    if obs.has_non_utf8_names {
        summary.archive_non_utf8_count += 1;
    }
    for &m in &obs.compression_methods {
        *summary.archive_compression_methods.entry(m).or_insert(0) += 1;
    }
    summary.archive_csv_entries_total += obs.csv_entries;
    summary.archive_json_entries_total += obs.json_entries;
    summary.archive_image_entries_total += obs.image_entries;
    summary.archive_xml_entries_total += obs.xml_entries;
    summary.archive_opaque_entries_total += obs.opaque_entries;
    if obs.opaque_entries > 0 {
        summary.archive_files_with_opaque += 1;
    }
    merge_counts(&mut summary.archive_opaque_extensions, &obs.opaque_extensions);
    merge_counts(&mut summary.archive_path_templates, &obs.path_templates);
    *summary.archive_dialects.entry(obs.dialect.clone()).or_insert(0) += 1;
    if let Some(av) = &obs.app_version {
        *summary.archive_app_versions.entry(av.clone()).or_insert(0) += 1;
    }
    if let Some(maj) = obs.major_version {
        *summary.archive_major_versions.entry(maj).or_insert(0) += 1;
    }
    for (k, v) in &obs.version_keys {
        *summary.archive_version_keys.entry(k.clone()).or_default().entry(v.clone()).or_insert(0) += 1;
    }
    for (k, ty) in &obs.document_json_keys {
        *summary.archive_document_json_keys.entry(format!("{k}:{ty}")).or_insert(0) += 1;
    }
    for (k, count) in &obs.sheets_subkeys {
        *summary.archive_sheets_subkeys.entry(k.clone()).or_insert(0) += *count;
    }
    for k in &obs.sheet_attrs_map_keys {
        *summary.archive_sheet_attrs_map_keys.entry(k.clone()).or_insert(0) += 1;
    }
    for (role, keys) in &obs.role_json_keys {
        let entry = summary.archive_role_json_keys.entry(role.clone()).or_default();
        for k in keys {
            *entry.entry(k.clone()).or_insert(0) += 1;
        }
    }
    merge_counts(&mut summary.archive_data_formats, &obs.data_formats);
    merge_counts(&mut summary.archive_table_types, &obs.table_types);
    merge_counts(&mut summary.archive_analysis_classes, &obs.analysis_classes);
    merge_counts(&mut summary.archive_csv_header_probe, &obs.csv_header_probe);
    merge_counts(&mut summary.archive_x_vs_row_titles_probe, &obs.x_vs_row_titles_probe);
    merge_counts(&mut summary.archive_analysis_result_shapes, &obs.analysis_result_shapes);
    if obs.has_analysis_params_json {
        summary.archive_files_with_analysis_params += 1;
    }
    if obs.has_graph_previews {
        summary.archive_files_with_graph_previews += 1;
    }
    if obs.has_exclusions_key {
        summary.archive_files_with_exclusions += 1;
    }
    if obs.has_info_sheets {
        summary.archive_files_with_info_sheets += 1;
    }
    summary.archive_high_low_probe.merge(&obs.high_low_probe);
    summary.archive_cv_probe.merge(&obs.zip_cv_probe);

    let mut features = BTreeSet::new();
    for k in obs.document_json_keys.keys() {
        features.insert(format!("key:document.json:{k}"));
    }
    for tpl in obs.path_templates.keys() {
        features.insert(format!("zip_path:{tpl}"));
    }
    for fmt in obs.data_formats.keys() {
        features.insert(format!("dataFormat:{fmt}"));
    }
    profile_builder.record_file(&obs.file_id, &obs.record_id, &features, &BTreeMap::new());
}

#[cfg(feature = "docs-site")]
fn seed_analysis_mapping_diagnostics(map: &mut BTreeMap<String, AnalysisClassDiagnosticSummary>) {
    use curvebook_format::{ANALYSIS_MAPPINGS, AnalysisVerificationStatus};
    for row in ANALYSIS_MAPPINGS {
        let status = match row.verification {
            AnalysisVerificationStatus::CorpusVerified => "CorpusVerified",
            AnalysisVerificationStatus::Unverified => "Unverified",
        };
        map.entry(row.foreign_class.to_owned()).or_insert_with(|| AnalysisClassDiagnosticSummary {
            mapping_status: status.to_owned(),
            curvebook_kind: row.curvebook_kind_label.to_owned(),
            ..AnalysisClassDiagnosticSummary::default()
        });
    }
}

#[cfg(feature = "docs-site")]
fn run_engine_pipeline_for_file(
    fmt: InteropFormat,
    bytes: &[u8],
    known_corrupt: bool,
    pipeline: &mut EnginePipelineSummary,
    durations_ms: &mut Vec<u64>,
    reexported_pzfx_candidates: &mut Vec<(CandidateStructure, BTreeSet<String>)>,
) {
    use curvebook_doc::TableArchetype;
    use curvebook_format::{
        FormatError, ImportWarningKind, PzfxExportWarningKind, compare_pzfx_data_layer, export_pzfx_bytes, import_prism_bytes, import_pzfx_bytes,
        open_project_bytes, save_project_bytes,
    };

    pipeline.attempted += 1;
    let start = std::time::Instant::now();
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match fmt {
        InteropFormat::Pzfx => import_pzfx_bytes(bytes),
        InteropFormat::ZipArchive => import_prism_bytes(bytes),
        InteropFormat::BinaryPzf => Err(FormatError::UnsupportedFormat { reason: String::from("binary") }),
    }));
    let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    durations_ms.push(elapsed_ms);

    let report = match res {
        Err(_) => {
            pipeline.panics += 1;
            match fmt {
                InteropFormat::Pzfx => pipeline.pzfx_panics += 1,
                InteropFormat::ZipArchive => pipeline.archive_panics += 1,
                InteropFormat::BinaryPzf => {}
            }
            return;
        }
        Ok(Err(err)) => {
            pipeline.import_err += 1;
            let cls = format!("{}:{}", fmt.label(), format_error_class(&err));
            *pipeline.parse_failure_classes.entry(cls).or_insert(0) += 1;
            let subcls = format!("{}:{}", fmt.label(), format_error_subclass(&err));
            *pipeline.parse_failure_subclasses.entry(subcls.clone()).or_insert(0) += 1;
            let is_safety_cap = is_documented_safety_cap_error(&err);
            match fmt {
                InteropFormat::Pzfx => {
                    pipeline.pzfx_import_err += 1;
                    if known_corrupt {
                        pipeline.pzfx_known_corrupt_err += 1;
                        *pipeline.corrupt_file_classes.entry(subcls).or_insert(0) += 1;
                    } else if is_safety_cap {
                        pipeline.pzfx_safety_cap_err += 1;
                        *pipeline.safety_cap_refusal_classes.entry(subcls).or_insert(0) += 1;
                    } else {
                        pipeline.pzfx_valid_import_err += 1;
                    }
                }
                InteropFormat::ZipArchive => {
                    pipeline.archive_import_err += 1;
                    if known_corrupt {
                        pipeline.archive_known_corrupt_err += 1;
                        *pipeline.corrupt_file_classes.entry(subcls).or_insert(0) += 1;
                    } else if is_safety_cap {
                        pipeline.archive_safety_cap_err += 1;
                        *pipeline.safety_cap_refusal_classes.entry(subcls).or_insert(0) += 1;
                    } else {
                        pipeline.archive_valid_import_err += 1;
                    }
                }
                InteropFormat::BinaryPzf => {}
            }
            return;
        }
        Ok(Ok(rep)) => rep,
    };

    pipeline.import_ok += 1;
    match fmt {
        InteropFormat::Pzfx => pipeline.pzfx_import_ok += 1,
        InteropFormat::ZipArchive => pipeline.archive_import_ok += 1,
        InteropFormat::BinaryPzf => {}
    }

    for w in &report.warnings {
        let kind_label = format!("{:?}", w.kind);
        *pipeline.warning_kinds.entry(kind_label.clone()).or_insert(0) += 1;
        if matches!(
            w.kind,
            ImportWarningKind::IgnoredSection
                | ImportWarningKind::UnsupportedAnalysis
                | ImportWarningKind::UnsupportedEquation
                | ImportWarningKind::UnsupportedGraphOrLayout
                | ImportWarningKind::UnsupportedScript
                | ImportWarningKind::UnsupportedChunk
                | ImportWarningKind::UnsupportedTableType
        ) {
            *pipeline.dropped_sections.entry(kind_label).or_insert(0) += 1;
        }
    }

    for ds in &report.document.data_sheets {
        let arch_label = format!("{:?}", ds.archetype.kind());
        *pipeline.archetypes.entry(arch_label).or_insert(0) += 1;
        match &ds.archetype {
            TableArchetype::Xy(t) => {
                *pipeline.subcolumn_modes.entry(format!("{:?}", t.y_subcolumn_mode)).or_insert(0) += 1;
            }
            TableArchetype::Column(t) => {
                *pipeline.subcolumn_modes.entry(format!("{:?}", t.subcolumn_mode)).or_insert(0) += 1;
            }
            TableArchetype::Grouped(t) => {
                *pipeline.subcolumn_modes.entry(format!("{:?}", t.subcolumn_mode)).or_insert(0) += 1;
            }
            _ => {}
        }
    }

    // Step 4 & §6.2: `.pzfx` export -> re-import -> data-layer fidelity + §6.3 candidate capture.
    if fmt == InteropFormat::Pzfx {
        let exp_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| export_pzfx_bytes(&report.document)));
        match exp_res {
            Err(_) => pipeline.pzfx_export_panics += 1,
            Ok(Err(_)) => pipeline.pzfx_export_err += 1,
            Ok(Ok(export_rep)) => {
                pipeline.pzfx_export_ok += 1;
                let mut warned_insufficient = BTreeSet::new();
                for w in &export_rep.warnings {
                    *pipeline.pzfx_export_warning_kinds.entry(format!("{:?}", w.kind)).or_insert(0) += 1;
                    if w.kind == PzfxExportWarningKind::InsufficientCorpusCoverage
                        && let Some(feat) = extract_backtick_feature(&w.message)
                    {
                        *pipeline.pzfx_export_insufficient_features.entry(feat.clone()).or_insert(0) += 1;
                        warned_insufficient.insert(feat);
                    }
                }
                for group in &export_rep.preflight.dropped {
                    *pipeline.pzfx_export_dropped_kinds.entry(format!("{:?}", group.kind)).or_insert(0) += 1;
                }
                if let Some(reexp_obs) = observe_pzfx_bytes(&export_rep.bytes, "reexport", "unix:0", "reexport", "reexport:0", "internal") {
                    reexported_pzfx_candidates.push((candidate_from_pzfx_observation(&reexp_obs), warned_insufficient));
                }

                let reimp_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| import_pzfx_bytes(&export_rep.bytes)));
                match reimp_res {
                    Err(_) => pipeline.pzfx_reimport_panics += 1,
                    Ok(Err(err)) => {
                        pipeline.pzfx_reimport_err += 1;
                        let subcls = format_error_subclass(&err);
                        *pipeline.pzfx_reimport_err_classes.entry(subcls).or_insert(0) += 1;
                    }
                    Ok(Ok(reimported)) => {
                        pipeline.pzfx_reimport_ok += 1;
                        let diff = compare_pzfx_data_layer(&report.document, &reimported.document);
                        pipeline.pzfx_reexport_tables_compared = pipeline.pzfx_reexport_tables_compared.saturating_add(diff.tables_compared);
                        pipeline.pzfx_reexport_cells_compared = pipeline.pzfx_reexport_cells_compared.saturating_add(diff.cells_compared);
                        pipeline.pzfx_reexport_info_sheets_compared =
                            pipeline.pzfx_reexport_info_sheets_compared.saturating_add(diff.info_sheets_compared);
                        if diff.is_identical() {
                            pipeline.pzfx_reexport_identical += 1;
                        } else {
                            pipeline.pzfx_reexport_diff_files += 1;
                            pipeline.pzfx_reexport_diffs_total = pipeline.pzfx_reexport_diffs_total.saturating_add(diff.differences.len());
                            for d in &diff.differences {
                                let cls = classify_pzfx_diff_location(&d.location);
                                let subcls = classify_pzfx_diff_subclass(&cls, &d.expected, &d.actual);
                                *pipeline.pzfx_reexport_diff_classes.entry(cls).or_insert(0) += 1;
                                *pipeline.pzfx_reexport_diff_subclasses.entry(subcls).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // §6.2 & Step 7 for `.prism`: CSV/JSON data-layer fidelity + cached-result diagnostic comparison.
    if fmt == InteropFormat::ZipArchive {
        verify_archive_data_layer_and_analyses(bytes, &report, pipeline);
    }

    // Step 5: Engine recalculation.
    let recalc_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut session = curvebook_engine::Session::from_document(report.document.clone());
        session.recalculate_all()
    }));
    match recalc_res {
        Ok(Ok(())) => pipeline.engine_recalc_ok += 1,
        Ok(Err(_)) | Err(_) => pipeline.engine_recalc_err += 1,
    }

    // Step 6: Native `.curvebook` round-trip.
    match save_project_bytes(&report.document) {
        Ok(saved) => match open_project_bytes(&saved) {
            Ok(reopened) => {
                if reopened.data_sheets == report.document.data_sheets {
                    pipeline.curvebook_roundtrip_ok += 1;
                } else {
                    pipeline.curvebook_roundtrip_diffs += 1;
                }
            }
            Err(err) => {
                pipeline.curvebook_roundtrip_err += 1;
                let cls = format!("open:{}", format_error_subclass(&err));
                *pipeline.curvebook_roundtrip_err_classes.entry(cls.clone()).or_insert(0) += 1;
                if is_documented_safety_cap_error(&err) {
                    pipeline.curvebook_roundtrip_safety_cap_err += 1;
                    *pipeline.safety_cap_refusal_classes.entry(format!(".curvebook:{cls}")).or_insert(0) += 1;
                } else {
                    pipeline.curvebook_roundtrip_unexpected_err += 1;
                }
            }
        },
        Err(err) => {
            pipeline.curvebook_roundtrip_err += 1;
            let cls = format!("save:{}", format_error_subclass(&err));
            *pipeline.curvebook_roundtrip_err_classes.entry(cls.clone()).or_insert(0) += 1;
            if is_documented_safety_cap_error(&err) {
                pipeline.curvebook_roundtrip_safety_cap_err += 1;
                *pipeline.safety_cap_refusal_classes.entry(format!(".curvebook:{cls}")).or_insert(0) += 1;
            } else {
                pipeline.curvebook_roundtrip_unexpected_err += 1;
            }
        }
    }
}

#[cfg(feature = "docs-site")]
fn is_documented_safety_cap_error(err: &curvebook_format::FormatError) -> bool {
    use curvebook_doc::DocError;
    use curvebook_format::FormatError;

    match err {
        FormatError::FileTooLarge { .. } | FormatError::XmlDepthExceeded { .. } | FormatError::ZipBomb { .. } => true,
        FormatError::XmlParse { .. } => format_error_subclass(err) == "XmlParse:limit_exceeded",
        FormatError::CsvParse { .. } => format_error_subclass(err) == "CsvParse:limit_exceeded",
        FormatError::Doc(
            DocError::TooManyRows { .. }
            | DocError::TooManyDataSets { .. }
            | DocError::TooManySubcolumns { .. }
            | DocError::TooManyCells { .. }
            | DocError::TooManySheets { .. }
            | DocError::TooManyLayoutItems { .. }
            | DocError::TooManyImageAssets { .. },
        ) => true,
        _ => false,
    }
}

#[cfg(feature = "docs-site")]
fn classify_pzfx_diff_subclass(cls: &str, expected: &str, actual: &str) -> String {
    if cls == "data.x_mode" {
        return if expected.starts_with("DateOrElapsedTime") && actual.starts_with("DateOrElapsedTime") {
            String::from("data.x_mode:date_elapsed_unit_label")
        } else {
            String::from("data.x_mode:variant_change")
        };
    }
    if expected.trim() == actual.trim() {
        return format!("{cls}:leading_or_trailing_ws");
    }
    if expected.replace("\r\n", "\n") == actual.replace("\r\n", "\n") {
        return format!("{cls}:crlf_vs_lf");
    }
    if expected.is_empty() && !actual.is_empty() {
        return format!("{cls}:empty_became_non_empty");
    }
    if !expected.is_empty() && actual.is_empty() {
        return format!("{cls}:non_empty_became_empty");
    }
    let has_markup_trigger = |s: &str| s.contains('*') || s.contains('$') || s.contains('^') || s.contains('~') || s.contains('\\');
    if has_markup_trigger(expected) || has_markup_trigger(actual) {
        return format!("{cls}:markup_or_escape_roundtrip");
    }
    if expected.contains('\n') || actual.contains('\n') {
        return format!("{cls}:multiline_newline_diff");
    }
    format!("{cls}:other_text_diff")
}

#[cfg(feature = "docs-site")]
fn format_error_subclass(err: &curvebook_format::FormatError) -> String {
    use curvebook_doc::DocError;
    use curvebook_format::FormatError;

    match err {
        FormatError::Doc(de) => {
            let sub = match de {
                DocError::EmptyIdentifier => "EmptyIdentifier",
                DocError::InvalidIdentifier { .. } => "InvalidIdentifier",
                DocError::TooManyRows { .. } => "TooManyRows",
                DocError::TooManyDataSets { .. } => "TooManyDataSets",
                DocError::TooManySubcolumns { .. } => "TooManySubcolumns",
                DocError::TooManyCells { .. } => "TooManyCells",
                DocError::TooManyLayoutItems { .. } => "TooManyLayoutItems",
                DocError::TooManySheets { .. } => "TooManySheets",
                DocError::TooManyImageAssets { .. } => "TooManyImageAssets",
                DocError::TitleTooLong { .. } => "TitleTooLong",
                DocError::InvalidSubcolumnCount { .. } => "InvalidSubcolumnCount",
                DocError::IncompatibleSubcolumnMode { .. } => "IncompatibleSubcolumnMode",
                DocError::SheetFrozen { .. } => "SheetFrozen",
                DocError::CellOutOfBounds { .. } => "CellOutOfBounds",
                DocError::InvalidCellValue { .. } => "InvalidCellValue",
                DocError::SheetNotFound { .. } => "SheetNotFound",
                DocError::SheetIndexOutOfBounds { .. } => "SheetIndexOutOfBounds",
                DocError::DuplicateSheetId { .. } => "DuplicateSheetId",
                DocError::InvalidConfidenceLevel { .. } => "InvalidConfidenceLevel",
                DocError::InvalidAxisSegments { .. } => "InvalidAxisSegments",
                DocError::IncompatibleRetargetTable { .. } => "IncompatibleRetargetTable",
            };
            format!("Doc:{sub}")
        }
        FormatError::ZipBomb { reason } => {
            let low = reason.to_ascii_lowercase();
            let sub = if low.contains("entries") || low.contains("entry count") {
                "entry_count_cap"
            } else if low.contains("ratio") {
                "compression_ratio_cap"
            } else if low.contains("total") || low.contains("uncompressed") || low.contains("budget") {
                "uncompressed_bytes_cap"
            } else {
                "other"
            };
            format!("ZipBomb:{sub}")
        }
        FormatError::XmlParse { reason, .. } => {
            let low = reason.to_ascii_lowercase();
            let sub = if low.contains("exceed") || low.contains("limit") || low.contains("max") || low.contains("node") {
                "limit_exceeded"
            } else if low.contains("entity") || low.contains("character reference") || low.contains("amp") {
                "entity_or_charref"
            } else if low.contains("unclosed") || low.contains("eof") || low.contains("unexpected end") {
                "unclosed_or_eof"
            } else if low.contains("mismatch") || low.contains("closing") {
                "mismatched_tag"
            } else if low.contains("control") || low.contains("invalid char") || low.contains("utf") || low.contains("encoding") {
                "invalid_char_or_encoding"
            } else if low.contains("attr") || low.contains("quote") {
                "attribute_syntax"
            } else {
                "other_syntax"
            };
            format!("XmlParse:{sub}")
        }
        FormatError::CsvParse { reason, .. } => {
            let low = reason.to_ascii_lowercase();
            let sub = if low.contains("quote") {
                "quote_syntax"
            } else if low.contains("limit") || low.contains("max") || low.contains("budget") {
                "limit_exceeded"
            } else {
                "other_syntax"
            };
            format!("CsvParse:{sub}")
        }
        FormatError::FileTooLarge { .. } => String::from("FileTooLarge"),
        other => String::from(format_error_class(other)),
    }
}

#[cfg(feature = "docs-site")]
fn verify_archive_data_layer_and_analyses(bytes: &[u8], report: &curvebook_format::ImportReport, pipeline: &mut EnginePipelineSummary) {
    use curvebook_format::prism::import::external_archive_zip_limits;
    use curvebook_format::prism::schema::{
        ArchiveIdIndex, ExternalDataSetJson, ExternalDataSheetJson, ExternalDocumentIndex, IdLookup, is_external_indexed_document,
    };
    use curvebook_format::zip::{ZipIndex, ZipReadOptions};

    let Ok(mut zip_index) = ZipIndex::from_bytes_with_options(bytes, external_archive_zip_limits(), ZipReadOptions::external_interop()) else {
        return;
    };
    let doc_path = zip_index
        .entry_names()
        .find(|p| p.eq_ignore_ascii_case("document.json"))
        .or_else(|| zip_index.entry_names().find(|p| !p.starts_with("__MACOSX/") && file_name_ci(p, "document.json")))
        .map(str::to_owned);
    let Some(doc_path) = doc_path else {
        return;
    };
    let Ok(Some(raw_doc)) = zip_index.read_entry(&doc_path) else {
        return;
    };
    let clean_doc = raw_doc.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&raw_doc);
    let Ok(doc_val) = serde_json::from_slice::<Value>(clean_doc) else {
        return;
    };
    if !is_external_indexed_document(&doc_val) {
        return;
    }

    let id_index = ArchiveIdIndex::from_zip_index(&zip_index);
    let doc_index = ExternalDocumentIndex::from_json(&doc_val);

    pipeline.archive_fidelity_files_checked += 1;
    let mut file_diffs = 0usize;
    let mut imported_ds_idx = 0usize;
    let mut data_uid_to_sheet_id: BTreeMap<String, curvebook_doc::SheetId> = BTreeMap::new();
    let mut data_set_positions = curvebook_format::ExternalDataSetPositions::new();

    for sheet_uid in &doc_index.data_sheet_ids {
        let IdLookup::Found(sheet_path) = id_index.find_data_sheet_json(sheet_uid) else {
            pipeline.archive_fidelity_tables_skipped_with_warning += 1;
            continue;
        };
        let Ok(Some(sheet_bytes)) = zip_index.read_entry(&sheet_path) else {
            pipeline.archive_fidelity_tables_skipped_with_warning += 1;
            continue;
        };
        let Ok(sheet_val) = serde_json::from_slice::<Value>(&sheet_bytes) else {
            pipeline.archive_fidelity_tables_skipped_with_warning += 1;
            continue;
        };
        let Some(sheet_meta) = ExternalDataSheetJson::from_json(&sheet_val) else {
            continue;
        };
        if sheet_meta.is_analysis_view() {
            continue;
        }
        let Some(table_uid) = sheet_meta.table.uid.as_deref().filter(|s| !s.trim().is_empty()) else {
            pipeline.archive_fidelity_tables_skipped_with_warning += 1;
            continue;
        };
        let IdLookup::Found(csv_path) = id_index.find_table_csv(table_uid) else {
            pipeline.archive_fidelity_tables_skipped_with_warning += 1;
            continue;
        };
        let Some(x_set) = load_opt_dataset_for_check(&mut zip_index, &id_index, sheet_meta.table.x_data_set.as_deref()) else {
            pipeline.archive_fidelity_tables_skipped_with_warning += 1;
            continue;
        };
        if sheet_meta.table.row_titles_data_set.is_some()
            && load_opt_dataset_for_check(&mut zip_index, &id_index, sheet_meta.table.row_titles_data_set.as_deref()).is_none()
        {
            pipeline.archive_fidelity_tables_skipped_with_warning += 1;
            continue;
        }
        let mut y_sets: Vec<ExternalDataSetJson> = Vec::with_capacity(sheet_meta.table.data_sets.len());
        let mut missing_y = false;
        for ds_uid in &sheet_meta.table.data_sets {
            match load_opt_dataset_for_check(&mut zip_index, &id_index, Some(ds_uid.as_str())) {
                Some(Some(ds)) => y_sets.push(ds),
                _ => {
                    missing_y = true;
                    break;
                }
            }
        }
        if missing_y {
            pipeline.archive_fidelity_tables_skipped_with_warning += 1;
            continue;
        }
        if !is_supported_external_table_plan(&sheet_meta, &y_sets) {
            pipeline.archive_fidelity_tables_skipped_with_warning += 1;
            continue;
        }
        let Ok(Some(csv_bytes)) = zip_index.read_entry(&csv_path) else {
            pipeline.archive_fidelity_tables_skipped_with_warning += 1;
            continue;
        };
        let Ok(csv_text) = std::str::from_utf8(&csv_bytes) else {
            pipeline.archive_fidelity_tables_skipped_with_warning += 1;
            continue;
        };

        let Some(imported_ds) = report.document.data_sheets.get(imported_ds_idx) else {
            file_diffs += 1;
            *pipeline.archive_fidelity_diff_classes.entry(String::from("missing_imported_data_sheet")).or_insert(0) += 1;
            break;
        };
        imported_ds_idx += 1;
        data_uid_to_sheet_id.insert(sheet_uid.clone(), imported_ds.id.clone());
        for (pos, ds_uid) in sheet_meta.table.data_sets.iter().enumerate() {
            data_set_positions.insert(ds_uid.clone(), (sheet_uid.clone(), pos));
        }

        let (cells_checked, table_diffs) = compare_external_csv_against_datasheet(
            &sheet_meta,
            x_set.as_ref(),
            &y_sets,
            csv_text,
            imported_ds,
            &mut pipeline.archive_fidelity_diff_classes,
        );
        pipeline.archive_fidelity_tables_checked += 1;
        pipeline.archive_fidelity_cells_checked = pipeline.archive_fidelity_cells_checked.saturating_add(cells_checked);
        file_diffs = file_diffs.saturating_add(table_diffs);
    }

    if imported_ds_idx != report.document.data_sheets.len() {
        file_diffs += 1;
        *pipeline.archive_fidelity_diff_classes.entry(String::from("extra_imported_data_sheet")).or_insert(0) += 1;
    }

    if file_diffs > 0 {
        pipeline.archive_fidelity_diff_files += 1;
        pipeline.archive_fidelity_diffs_total = pipeline.archive_fidelity_diffs_total.saturating_add(file_diffs);
    }

    // Step 7: `.prism` cached-result diagnostic comparison per `analysisClass`.
    let source_maps = ArchiveSourceMaps { data_uid_to_sheet_id: &data_uid_to_sheet_id, data_set_positions: &data_set_positions };
    run_archive_analysis_diagnostics(&mut zip_index, &id_index, &doc_index, &source_maps, &report.document, pipeline);
}

#[cfg(feature = "docs-site")]
fn load_opt_dataset_for_check(
    zip_index: &mut curvebook_format::zip::ZipIndex<'_>,
    id_index: &curvebook_format::prism::schema::ArchiveIdIndex,
    uid_opt: Option<&str>,
) -> Option<Option<curvebook_format::prism::schema::ExternalDataSetJson>> {
    use curvebook_format::prism::schema::{ExternalDataSetJson, IdLookup};
    let Some(uid) = uid_opt else {
        return Some(None);
    };
    let IdLookup::Found(path) = id_index.find_dataset_json(uid) else {
        return None;
    };
    let Ok(Some(bytes)) = zip_index.read_entry(&path) else {
        return None;
    };
    let Ok(val) = serde_json::from_slice::<Value>(&bytes) else {
        return None;
    };
    ExternalDataSetJson::from_json(&val).map(Some)
}

#[cfg(feature = "docs-site")]
fn is_supported_external_table_plan(
    sheet_meta: &curvebook_format::prism::schema::ExternalDataSheetJson,
    y_sets: &[curvebook_format::prism::schema::ExternalDataSetJson],
) -> bool {
    use curvebook_doc::{SubcolumnMode, TableKind};
    let kind = match sheet_meta.table.format.as_deref().map(str::trim) {
        Some(f) if f.eq_ignore_ascii_case("xy") => TableKind::Xy,
        Some(f) if f.eq_ignore_ascii_case("column") => TableKind::Column,
        Some(f) if f.eq_ignore_ascii_case("grouped") => TableKind::Grouped,
        Some(f) if f.eq_ignore_ascii_case("survival") => TableKind::Survival,
        Some(f) if f.eq_ignore_ascii_case("contingency") => TableKind::Contingency,
        Some(f) if f.eq_ignore_ascii_case("multivariable") => TableKind::MultipleVariables,
        Some(f) if f.eq_ignore_ascii_case("nested") => TableKind::Nested,
        Some(f) if f.eq_ignore_ascii_case("parts_of_whole") => TableKind::PartsOfWhole,
        None | Some("") => {
            if sheet_meta.table.x_data_set.is_some() {
                TableKind::Xy
            } else {
                TableKind::Column
            }
        }
        Some(_) => return false,
    };
    let raw_fmt = sheet_meta.table.data_format.as_deref().map(str::trim).unwrap_or("y_single");
    let max_ds_reps = y_sets.iter().map(|ds| ds.replicates.len()).max().unwrap_or(0);
    let mode = match raw_fmt.to_ascii_lowercase().as_str() {
        "y_single" | "text" => {
            if matches!(kind, TableKind::Grouped | TableKind::Nested) {
                SubcolumnMode::RawReplicates(1)
            } else {
                SubcolumnMode::Single
            }
        }
        "y_replicates" | "text_replicates" => {
            let reps = match sheet_meta.table.replicates_count {
                Some(0) => 0,
                Some(n) => n.max(max_ds_reps),
                None => max_ds_reps,
            };
            if reps == 0 {
                return false;
            }
            if reps == 1 && !matches!(kind, TableKind::Grouped | TableKind::Nested) && !kind.allows_subcolumn_mode(SubcolumnMode::RawReplicates(1)) {
                SubcolumnMode::Single
            } else {
                SubcolumnMode::RawReplicates(reps)
            }
        }
        "y_sd_n" | "y_cv_n" => SubcolumnMode::MeanSdN,
        "y_se_n" => SubcolumnMode::MeanSemN,
        "y_sd" | "y_cv" => SubcolumnMode::MeanSd,
        "y_se" => SubcolumnMode::MeanSem,
        "y_high_low" | "y_plus_minus" => SubcolumnMode::MeanLowHigh,
        _ => return false,
    };
    kind.allows_subcolumn_mode(mode)
        || (kind == TableKind::Column && TableKind::Grouped.allows_subcolumn_mode(mode))
        || (matches!(kind, TableKind::Grouped | TableKind::Nested) && mode == SubcolumnMode::Single)
}

#[cfg(feature = "docs-site")]
fn compare_external_csv_against_datasheet(
    sheet_meta: &curvebook_format::prism::schema::ExternalDataSheetJson,
    x_set: Option<&curvebook_format::prism::schema::ExternalDataSetJson>,
    y_sets: &[curvebook_format::prism::schema::ExternalDataSetJson],
    csv_text: &str,
    imported_ds: &curvebook_doc::DataSheet,
    diff_classes: &mut BTreeMap<String, usize>,
) -> (usize, usize) {
    use curvebook_doc::{Cell, CellValue, DataSetColumn, MAX_TITLE_BYTES, TableArchetype, XColumnMode};
    use curvebook_format::csv::{CsvOptions, parse_cell_token, parse_delimited_records};

    let csv_opts = CsvOptions { header_rows: 0, has_row_titles: false, parse_asterisk_exclusions: true, ..CsvOptions::csv() };
    let Ok(records) = parse_delimited_records(csv_text, &csv_opts) else {
        *diff_classes.entry(String::from("csv_parse_error")).or_insert(0) += 1;
        return (0, 1);
    };

    let mut cells_checked = 0usize;
    let mut diffs = 0usize;
    let has_rt = sheet_meta.table.row_titles_data_set.is_some();
    let rt_cols = usize::from(has_rt);

    let x_fmt = sheet_meta
        .table
        .x_data_set_format
        .as_deref()
        .or_else(|| x_set.and_then(|s| s.format.as_deref()))
        .unwrap_or("y_single")
        .trim()
        .to_ascii_lowercase();
    let x_csv_cols = if sheet_meta.table.x_data_set.is_none() {
        0
    } else {
        match x_fmt.as_str() {
            "series" => 0,
            "y_errorbar" | "date_se" => 2,
            _ => 1,
        }
    };
    let y_start = rt_cols + x_csv_cols;

    // 1. Verify row titles when present.
    if has_rt {
        let imported_rt: &[String] = match &imported_ds.archetype {
            TableArchetype::Xy(t) => &t.row_titles,
            TableArchetype::Column(t) => &t.row_titles,
            TableArchetype::Grouped(t) => &t.row_titles,
            TableArchetype::Contingency(t) => &t.row_titles,
            TableArchetype::Survival(t) => &t.row_titles,
            TableArchetype::PartsOfWhole(t) => &t.slice_labels,
            TableArchetype::MultipleVariables(t) => &t.row_titles,
            TableArchetype::Nested(t) => &t.row_titles,
        };
        for (r, rec) in records.iter().enumerate() {
            let expected = clamp_utf8_bytes(rec.first().map(String::as_str).unwrap_or("").trim(), MAX_TITLE_BYTES);
            let actual = imported_rt.get(r).map(String::as_str).unwrap_or("");
            if expected != actual {
                diffs += 1;
                *diff_classes.entry(String::from("row_title_mismatch")).or_insert(0) += 1;
            }
        }
    }

    // 2. Verify X / survival time columns.
    match &imported_ds.archetype {
        TableArchetype::Xy(xy) if x_csv_cols > 0 => {
            let x_subs = xy.x_mode.subcolumn_count();
            for (r, rec) in records.iter().enumerate() {
                for s in 0..x_subs {
                    let raw = rec.get(rt_cols + s).map(String::as_str).unwrap_or("");
                    let mut exp = parse_cell_token(raw, &csv_opts);
                    if x_set.and_then(|xs| xs.replicates.get(s)).is_some_and(|rep| rep.excluded_rows.contains(&r)) {
                        exp.excluded = true;
                    }
                    let act = xy.x_cells.get(r).and_then(|row| row.get(s)).cloned().unwrap_or_else(Cell::empty);
                    cells_checked += 1;
                    check_cell_fidelity(&exp, &act, "x_cell", &mut diffs, diff_classes);
                }
            }
        }
        TableArchetype::Xy(xy) if sheet_meta.table.x_data_set.is_some() && x_fmt == "series" => {
            let rep = x_set.and_then(|s| s.replicates.first());
            let exp_start = rep.and_then(|r| r.start_value).unwrap_or(0.0);
            let exp_step = rep.and_then(|r| r.interval).unwrap_or(1.0);
            match xy.x_mode {
                XColumnMode::GeneratedSequence { start, step, .. } => {
                    if !floats_close_1e12(start, exp_start) || !floats_close_1e12(step, exp_step) {
                        diffs += 1;
                        *diff_classes.entry(String::from("x_series_params_mismatch")).or_insert(0) += 1;
                    }
                }
                _ => {
                    diffs += 1;
                    *diff_classes.entry(String::from("x_series_mode_mismatch")).or_insert(0) += 1;
                }
            }
        }
        TableArchetype::Survival(surv) if sheet_meta.table.x_data_set.is_some() => {
            if x_fmt == "series" {
                let rep = x_set.and_then(|s| s.replicates.first());
                let exp_start = rep.and_then(|r| r.start_value).unwrap_or(0.0);
                let exp_step = rep.and_then(|r| r.interval).unwrap_or(1.0);
                for r in 0..records.len().max(1) {
                    let val = exp_start + (r as f64) * exp_step;
                    let exp = if val.is_finite() { Cell::number(val) } else { Cell::empty() };
                    let act = surv.times.get(r).cloned().unwrap_or_else(Cell::empty);
                    cells_checked += 1;
                    check_cell_fidelity(&exp, &act, "survival_time", &mut diffs, diff_classes);
                }
            } else {
                for (r, rec) in records.iter().enumerate() {
                    let raw = rec.get(rt_cols).map(String::as_str).unwrap_or("");
                    let mut exp = parse_cell_token(raw, &csv_opts);
                    if x_set.and_then(|xs| xs.replicates.first()).is_some_and(|rep| rep.excluded_rows.contains(&r)) {
                        exp.excluded = true;
                    }
                    let act = surv.times.get(r).cloned().unwrap_or_else(Cell::empty);
                    cells_checked += 1;
                    check_cell_fidelity(&exp, &act, "survival_time", &mut diffs, diff_classes);
                }
            }
        }
        _ => {}
    }

    // 3. Verify Y datasets.
    let raw_fmt = sheet_meta.table.data_format.as_deref().map(str::trim).unwrap_or("y_single").to_ascii_lowercase();
    let is_text_mode = matches!(raw_fmt.as_str(), "text" | "text_replicates");
    let default_stride = match raw_fmt.as_str() {
        "y_sd_n" | "y_se_n" | "y_cv_n" | "y_high_low" => 3,
        "y_sd" | "y_se" | "y_cv" | "y_plus_minus" => 2,
        "y_replicates" | "text_replicates" => {
            let max_ds = y_sets.iter().map(|ds| ds.replicates.len()).max().unwrap_or(1);
            sheet_meta.table.replicates_count.unwrap_or(max_ds).max(max_ds).max(1)
        }
        _ => 1,
    };

    let mut verify_ds_columns = |cols: &[DataSetColumn], target_subs: usize, censored_code: Option<i64>| {
        let mut csv_col_base = y_start;
        for (col_idx, ds_meta) in y_sets.iter().enumerate() {
            let Some(act_col) = cols.get(col_idx) else {
                diffs += 1;
                *diff_classes.entry(String::from("missing_y_column")).or_insert(0) += 1;
                break;
            };
            let ds_reps = ds_meta.replicates.len();
            let ds_width = if ds_reps > 0 { ds_reps } else { default_stride };
            let use_swap_3 = raw_fmt == "y_high_low" || (raw_fmt == "y_plus_minus" && ds_reps >= 3);
            let use_dup_2 = raw_fmt == "y_plus_minus" && !use_swap_3;

            for (r, rec) in records.iter().enumerate() {
                if use_swap_3 {
                    let c0 = parse_ext_cell_for_check(rec, csv_col_base, r, 0, ds_meta, false, &csv_opts);
                    let c_plus = parse_ext_cell_for_check(rec, csv_col_base + 1, r, 1, ds_meta, false, &csv_opts);
                    let c_minus = parse_ext_cell_for_check(rec, csv_col_base + 2, r, 2, ds_meta, false, &csv_opts);
                    for (s, exp) in [(0, &c0), (1, &c_minus), (2, &c_plus)] {
                        let act = act_col.cell(r, s).cloned().unwrap_or_else(Cell::empty);
                        cells_checked += 1;
                        check_cell_fidelity(exp, &act, "y_cell", &mut diffs, diff_classes);
                    }
                } else if use_dup_2 {
                    let c0 = parse_ext_cell_for_check(rec, csv_col_base, r, 0, ds_meta, false, &csv_opts);
                    let c_err = parse_ext_cell_for_check(rec, csv_col_base + 1, r, 1, ds_meta, false, &csv_opts);
                    for (s, exp) in [(0, &c0), (1, &c_err), (2, &c_err)] {
                        let act = act_col.cell(r, s).cloned().unwrap_or_else(Cell::empty);
                        cells_checked += 1;
                        check_cell_fidelity(exp, &act, "y_cell", &mut diffs, diff_classes);
                    }
                } else {
                    for s in 0..ds_width.min(target_subs) {
                        let mut exp = parse_ext_cell_for_check(rec, csv_col_base + s, r, s, ds_meta, is_text_mode, &csv_opts);
                        if s == 0
                            && exp.is_empty()
                            && let Some(cc) = censored_code
                            && ds_meta.replicates.first().is_some_and(|rep| rep.censored_rows.contains(&r))
                        {
                            exp.value = CellValue::Number(cc as f64);
                        }
                        let act = act_col.cell(r, s).cloned().unwrap_or_else(Cell::empty);
                        cells_checked += 1;
                        check_cell_fidelity(&exp, &act, "y_cell", &mut diffs, diff_classes);
                    }
                }
            }
            csv_col_base = csv_col_base.saturating_add(ds_width);
        }
    };

    match &imported_ds.archetype {
        TableArchetype::Xy(t) => verify_ds_columns(&t.data_sets, t.y_subcolumn_mode.subcolumn_count(), None),
        TableArchetype::Column(t) => verify_ds_columns(&t.data_sets, t.subcolumn_mode.subcolumn_count(), None),
        TableArchetype::Grouped(t) => verify_ds_columns(&t.data_sets, t.subcolumn_mode.subcolumn_count(), None),
        TableArchetype::Contingency(t) => verify_ds_columns(&t.data_sets, 1, None),
        TableArchetype::PartsOfWhole(t) => verify_ds_columns(&t.data_sets, 1, None),
        TableArchetype::Survival(t) => verify_ds_columns(&t.cohorts, 1, Some(t.censored_code)),
        TableArchetype::MultipleVariables(mv) => {
            let mut csv_col_base = y_start;
            for (col_idx, ds_meta) in y_sets.iter().enumerate() {
                let Some(act_var) = mv.variables.get(col_idx) else {
                    diffs += 1;
                    *diff_classes.entry(String::from("missing_mv_variable")).or_insert(0) += 1;
                    break;
                };
                let is_str = ds_meta.replicates.first().and_then(|r| r.value_type.as_deref()).is_some_and(|vt| vt.eq_ignore_ascii_case("string"));
                for (r, rec) in records.iter().enumerate() {
                    let exp = parse_ext_cell_for_check(rec, csv_col_base, r, 0, ds_meta, is_str, &csv_opts);
                    let act = act_var.cells.get(r).cloned().unwrap_or_else(Cell::empty);
                    cells_checked += 1;
                    check_cell_fidelity(&exp, &act, "mv_cell", &mut diffs, diff_classes);
                }
                csv_col_base = csv_col_base.saturating_add(ds_meta.replicates.len().max(1));
            }
        }
        TableArchetype::Nested(nested) => {
            let units = nested.groups.first().map_or(1, curvebook_doc::NestedGroupColumn::biological_unit_count).max(1);
            let mut csv_col_base = y_start;
            for (col_idx, ds_meta) in y_sets.iter().enumerate() {
                let Some(act_grp) = nested.groups.get(col_idx) else {
                    diffs += 1;
                    *diff_classes.entry(String::from("missing_nested_group")).or_insert(0) += 1;
                    break;
                };
                let ds_width = if ds_meta.replicates.is_empty() { units } else { ds_meta.replicates.len() };
                for (r, rec) in records.iter().enumerate() {
                    for s in 0..ds_width.min(units) {
                        let exp = parse_ext_cell_for_check(rec, csv_col_base + s, r, s, ds_meta, false, &csv_opts);
                        let act = act_grp.cells.get(r).and_then(|row| row.get(s)).cloned().unwrap_or_else(Cell::empty);
                        cells_checked += 1;
                        check_cell_fidelity(&exp, &act, "nested_cell", &mut diffs, diff_classes);
                    }
                }
                csv_col_base = csv_col_base.saturating_add(ds_width);
            }
        }
    }

    (cells_checked, diffs)
}

#[cfg(feature = "docs-site")]
fn parse_ext_cell_for_check(
    rec: &[String],
    csv_col: usize,
    row_idx: usize,
    rep_idx: usize,
    ds_meta: &curvebook_format::prism::schema::ExternalDataSetJson,
    force_text: bool,
    csv_opts: &curvebook_format::csv::CsvOptions,
) -> curvebook_doc::Cell {
    use curvebook_doc::Cell;
    use curvebook_format::csv::parse_cell_token;

    let raw = rec.get(csv_col).map(String::as_str).unwrap_or("");
    let mut cell = if force_text {
        let trimmed = raw.trim();
        if trimmed.is_empty() { Cell::empty() } else { Cell::text(trimmed) }
    } else {
        parse_cell_token(raw, csv_opts)
    };
    if ds_meta.replicates.get(rep_idx).is_some_and(|rep| rep.excluded_rows.contains(&row_idx)) {
        cell.excluded = true;
    }
    cell
}

#[cfg(feature = "docs-site")]
fn check_cell_fidelity(
    exp: &curvebook_doc::Cell,
    act: &curvebook_doc::Cell,
    prefix: &str,
    diffs: &mut usize,
    diff_classes: &mut BTreeMap<String, usize>,
) {
    use curvebook_doc::CellValue;

    if exp.excluded != act.excluded {
        *diffs += 1;
        *diff_classes.entry(format!("{prefix}_excluded_mismatch")).or_insert(0) += 1;
    }
    let vals_eq = match (&exp.value, &act.value) {
        (CellValue::Empty, CellValue::Empty) => true,
        (CellValue::Number(a), CellValue::Number(b)) => floats_close_1e12(*a, *b),
        (CellValue::Text(a), CellValue::Text(b)) => a == b,
        _ => false,
    };
    if !vals_eq {
        *diffs += 1;
        *diff_classes.entry(format!("{prefix}_value_mismatch")).or_insert(0) += 1;
    }
}

#[cfg(feature = "docs-site")]
fn floats_close_1e12(a: f64, b: f64) -> bool {
    let a = if a == 0.0 { 0.0 } else { a };
    let b = if b == 0.0 { 0.0 } else { b };
    let tol = 1e-12 * a.abs().max(b.abs()).max(1.0);
    (a - b).abs() <= tol
}

#[cfg(feature = "docs-site")]
fn clamp_utf8_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

/// Per-design grouping key for agreement counts: `experimentalDesign`, a one-tailed marker, and
/// the post-test procedure, so promotion can be decided per design rather than per class.
#[cfg(feature = "docs-site")]
fn analysis_variant_key(params_val: &Value) -> String {
    let content = params_val.get("content");
    let mut key = content.and_then(|c| c.get("experimentalDesign")).and_then(Value::as_str).unwrap_or("<default>").to_owned();
    if content.and_then(|c| c.pointer("/options/pValueTailType")).and_then(Value::as_str) == Some("P_VALUE_ONE_TAILED") {
        key.push_str(" | one-tailed");
    }
    if let Some(mc) = content.and_then(|c| c.pointer("/postTests/pairwiseTests/multipleComparisons"))
        && content.and_then(|c| c.pointer("/postTests/type")).and_then(Value::as_str) != Some("NO_TESTS")
    {
        let kind = mc.get("type").and_then(Value::as_str).unwrap_or("?");
        let test = match kind {
            "STATISTICAL" => mc.pointer("/statistical/test").and_then(Value::as_str),
            "FDR" => mc.pointer("/fdr/test").and_then(Value::as_str),
            _ => None,
        };
        key.push_str(" | ");
        key.push_str(test.unwrap_or(kind));
    }
    key
}

/// Per-archive lookup tables shared by the analysis diagnostics pass.
#[cfg(feature = "docs-site")]
struct ArchiveSourceMaps<'a> {
    data_uid_to_sheet_id: &'a BTreeMap<String, curvebook_doc::SheetId>,
    data_set_positions: &'a curvebook_format::ExternalDataSetPositions,
}

#[cfg(feature = "docs-site")]
fn run_archive_analysis_diagnostics(
    zip_index: &mut curvebook_format::zip::ZipIndex<'_>,
    id_index: &curvebook_format::prism::schema::ArchiveIdIndex,
    doc_index: &curvebook_format::prism::schema::ExternalDocumentIndex,
    source_maps: &ArchiveSourceMaps<'_>,
    imported_doc: &curvebook_doc::ProjectDocument,
    pipeline: &mut EnginePipelineSummary,
) {
    use curvebook_doc::{AnalysisSpec, ProjectDocument, ResultSheet, ResultStatus, SheetId};
    use curvebook_format::prism::schema::IdLookup;
    use curvebook_format::{
        AnalysisVerificationStatus, CACHED_SCALAR_LINKS, cached_values_for_link, check_control_in_selection,
        compare_recomputed_against_cached_results, find_analysis_mapping, primary_input_data_sheet_uid, resolve_selected_data_sets,
        translate_analysis_parameters,
    };

    for analysis_uid in &doc_index.analysis_sheet_ids {
        let IdLookup::Found(sheet_path) = id_index.find_analysis_entry(analysis_uid, "sheet.json") else {
            continue;
        };
        let Ok(Some(sheet_bytes)) = zip_index.read_entry(&sheet_path) else {
            continue;
        };
        let Ok(sheet_val) = serde_json::from_slice::<Value>(&sheet_bytes) else {
            continue;
        };
        let Some(sheet_obj) = sheet_val.as_object() else {
            continue;
        };

        let raw_class = sheet_obj.get("analysisClass").and_then(Value::as_str).unwrap_or("UNKNOWN").trim();
        let class_key = if raw_class.is_empty() { String::from("UNKNOWN") } else { raw_class.to_owned() };
        let mapping_row = find_analysis_mapping(&class_key);

        let diag_entry = pipeline.archive_analysis_diagnostics.entry(class_key.clone()).or_insert_with(|| {
            let (status, kind_label) = match mapping_row {
                Some(row) => {
                    let st = match row.verification {
                        AnalysisVerificationStatus::CorpusVerified => "CorpusVerified",
                        AnalysisVerificationStatus::Unverified => "Unverified",
                    };
                    (st, row.curvebook_kind_label)
                }
                None => ("Unmapped", "<unmapped>"),
            };
            AnalysisClassDiagnosticSummary {
                mapping_status: status.to_owned(),
                curvebook_kind: kind_label.to_owned(),
                ..AnalysisClassDiagnosticSummary::default()
            }
        });
        diag_entry.analyses_observed += 1;

        let Some(row) = mapping_row else {
            continue;
        };

        let params_val = match id_index.find_analysis_entry(analysis_uid, "parameters.json") {
            IdLookup::Found(p) => zip_index.read_entry(&p).ok().flatten().and_then(|b| serde_json::from_slice::<Value>(&b).ok()),
            _ => None,
        };
        let Some(params_val) = params_val else {
            diag_entry.translated_blocked += 1;
            *diag_entry.blocking_reason_classes.entry(String::from("missing_parameters_json")).or_insert(0) += 1;
            continue;
        };
        let Some(mut translated) = translate_analysis_parameters(&class_key, &params_val) else {
            continue;
        };
        let source_uid = primary_input_data_sheet_uid(sheet_obj, source_maps.data_uid_to_sheet_id);
        let selected_data_sets = match resolve_selected_data_sets(sheet_obj, source_uid, source_maps.data_set_positions) {
            Ok(sel) => sel,
            Err(reason) => {
                translated.blocking_options.push(reason);
                Vec::new()
            }
        };
        if let Err(reason) = check_control_in_selection(&translated.kind, &selected_data_sets) {
            translated.blocking_options.push(reason);
        }
        if !translated.blocking_options.is_empty() {
            diag_entry.translated_blocked += 1;
            for opt in &translated.blocking_options {
                let cls = classify_blocking_option(opt);
                *diag_entry.blocking_reason_classes.entry(cls).or_insert(0) += 1;
            }
            continue;
        }
        diag_entry.translated_clean += 1;

        // Resolve the primary input DataSheet exactly as the importer does.
        let source_ds = source_uid
            .and_then(|uid| source_maps.data_uid_to_sheet_id.get(uid))
            .and_then(|sid| imported_doc.data_sheet(sid))
            .filter(|ds| translated.kind.is_compatible_with(ds.archetype.kind()));
        let Some(source_ds) = source_ds else {
            diag_entry.incompatible_or_unresolved_source += 1;
            continue;
        };

        let results_val = match id_index.find_analysis_entry(analysis_uid, "results.json") {
            IdLookup::Found(p) => zip_index.read_entry(&p).ok().flatten().and_then(|b| serde_json::from_slice::<Value>(&b).ok()),
            _ => None,
        }
        .unwrap_or(Value::Null);
        let cached_link_values: usize = CACHED_SCALAR_LINKS
            .iter()
            .filter(|l| l.foreign_class.eq_ignore_ascii_case(&class_key))
            .map(|l| cached_values_for_link(l, &results_val).len())
            .sum();
        diag_entry.cached_scalars_total = diag_entry.cached_scalars_total.saturating_add(cached_link_values);

        let recompute_outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<ResultSheet, String> {
            let mut tmp_doc = ProjectDocument::new("Diagnostic").map_err(|e| e.to_string())?;
            if let Some(info_id) = &source_ds.info_sheet_id
                && let Some(info) = imported_doc.info_sheet(info_id)
            {
                tmp_doc.add_info_sheet(info.clone()).map_err(|e| e.to_string())?;
            }
            tmp_doc.add_data_sheet(source_ds.clone()).map_err(|e| e.to_string())?;
            let mut spec = AnalysisSpec::new("diag-res", source_ds.id.clone(), translated.kind.clone()).map_err(|e| e.to_string())?;
            spec.selected_data_sets = selected_data_sets.clone();
            let rs = ResultSheet::new(spec).map_err(|e| e.to_string())?;
            tmp_doc.add_result_sheet(rs).map_err(|e| e.to_string())?;
            let _ = curvebook_engine::dag::recalculate_project_with_report(&mut tmp_doc).map_err(|e| e.to_string())?;
            tmp_doc.result_sheet(&SheetId::new("diag-res")).cloned().ok_or_else(|| String::from("missing diagnostic result sheet"))
        }));

        match recompute_outcome {
            Err(_) => diag_entry.recompute_panics += 1,
            Ok(Err(_)) => diag_entry.recomputed_failed += 1,
            Ok(Ok(recomputed)) => {
                if recomputed.status == ResultStatus::UpToDate {
                    diag_entry.recomputed_ok += 1;
                    diag_entry.recomputed_scalars_total = diag_entry.recomputed_scalars_total.saturating_add(recomputed.scalars.len());
                    let cmp = compare_recomputed_against_cached_results(
                        &recomputed,
                        &class_key,
                        &results_val,
                        row.absolute_tolerance,
                        row.relative_tolerance,
                    );
                    let variant = analysis_variant_key(&params_val);
                    let counts = diag_entry.variants.entry(variant).or_insert([0; 3]);
                    counts[0] += 1;
                    if cmp.compared_count > 0 {
                        counts[1] += 1;
                        if cmp.disagreements.is_empty() {
                            counts[2] += 1;
                        }
                    }
                    for (key, n) in &cmp.compared_by_key {
                        *diag_entry.compared_scalar_keys.entry(key.clone()).or_insert(0) += n;
                    }
                    diag_entry.scalars_compared = diag_entry.scalars_compared.saturating_add(cmp.compared_count);
                    diag_entry.scalars_disagreed = diag_entry.scalars_disagreed.saturating_add(cmp.disagreements.len());
                    diag_entry.scalars_agreed = diag_entry.scalars_agreed.saturating_add(cmp.compared_count.saturating_sub(cmp.disagreements.len()));
                    for d in &cmp.disagreements {
                        *diag_entry.disagreed_scalar_keys.entry(d.key.clone()).or_insert(0) += 1;
                    }
                } else {
                    diag_entry.recomputed_failed += 1;
                }
            }
        }
    }
}

#[cfg(feature = "docs-site")]
fn evaluate_pzfx_corpus_conformance(
    pzfx_profile: &DialectProfile,
    reexported_candidates: &[(CandidateStructure, BTreeSet<String>)],
    out: &mut PzfxConformanceSummary,
) {
    use curvebook_format::{
        PZFX_FEATURE_CORPUS_COVERAGE, PzfxExportWarningKind, PzfxFeatureCoverageStatus, compare_pzfx_data_layer, export_pzfx_bytes, import_pzfx_bytes,
    };

    out.static_table_rows_total = PZFX_FEATURE_CORPUS_COVERAGE.len();
    for row in PZFX_FEATURE_CORPUS_COVERAGE {
        match row.status {
            PzfxFeatureCoverageStatus::Conformant => out.static_table_conformant_rows += 1,
            PzfxFeatureCoverageStatus::InsufficientCoverage => out.static_table_insufficient_rows += 1,
        }
    }

    if pzfx_profile.total_files < MIN_CONFORMANCE_FILES || pzfx_profile.total_sources < MIN_CONFORMANCE_SOURCES {
        return;
    }
    out.evaluated = true;

    // 1. Verify tracked `PZFX_FEATURE_CORPUS_COVERAGE` static table sync against real corpus profile.
    let static_rows: Vec<StaticFeatureCoverage> = PZFX_FEATURE_CORPUS_COVERAGE
        .iter()
        .map(|row| StaticFeatureCoverage {
            feature: row.feature,
            declared: match row.status {
                PzfxFeatureCoverageStatus::Conformant => DeclaredCoverage::Conformant,
                PzfxFeatureCoverageStatus::InsufficientCoverage => DeclaredCoverage::InsufficientCoverage,
            },
        })
        .collect();
    if let Err(sync_errs) = verify_static_coverage_table(pzfx_profile, &static_rows) {
        out.static_table_sync_errors = sync_errs;
    }

    // 2. Broad synthetic CurveBook project suite covering every archetype, subcolumn mode, X mode,
    //    metadata feature, and table dimension tier (§6.3 items 3–5).
    let synthetic_docs = build_synthetic_pzfx_conformance_suite();
    out.synthetic_cases_total = synthetic_docs.len();
    for (case_label, doc) in synthetic_docs {
        let Ok(rep1) = export_pzfx_bytes(&doc) else {
            out.synthetic_non_conformant_cases += 1;
            *out.synthetic_violations.entry(format!("synthetic_export_failed:{case_label}")).or_insert(0) += 1;
            continue;
        };
        let Ok(rep2) = export_pzfx_bytes(&doc) else {
            out.synthetic_non_conformant_cases += 1;
            *out.synthetic_violations.entry(format!("synthetic_export2_failed:{case_label}")).or_insert(0) += 1;
            continue;
        };
        if rep1.bytes == rep2.bytes {
            out.synthetic_determinism_ok += 1;
        }
        match import_pzfx_bytes(&rep1.bytes) {
            Ok(reimp) => {
                let diff = compare_pzfx_data_layer(&doc, &reimp.document);
                if diff.is_identical() {
                    out.synthetic_roundtrip_ok += 1;
                } else {
                    out.synthetic_roundtrip_diffs += 1;
                    for d in &diff.differences {
                        let cls = classify_pzfx_diff_location(&d.location);
                        *out.synthetic_violations.entry(format!("synthetic_roundtrip_diff:{case_label}:{cls}")).or_insert(0) += 1;
                    }
                }
            }
            Err(_) => {
                out.synthetic_roundtrip_diffs += 1;
                *out.synthetic_violations.entry(format!("synthetic_reimport_failed:{case_label}")).or_insert(0) += 1;
            }
        }

        let mut warned_insufficient = BTreeSet::new();
        for w in &rep1.warnings {
            if w.kind == PzfxExportWarningKind::InsufficientCorpusCoverage
                && let Some(feat) = extract_backtick_feature(&w.message)
            {
                warned_insufficient.insert(feat);
            }
        }
        let Some(obs) = observe_pzfx_bytes(&rep1.bytes, "synthetic", "unix:0", "synthetic", "synthetic:0", "internal") else {
            out.synthetic_non_conformant_cases += 1;
            *out.synthetic_violations.entry(format!("synthetic_observe_failed:{case_label}")).or_insert(0) += 1;
            continue;
        };
        let cand = candidate_from_pzfx_observation(&obs);
        let conf = check_conformance_with_warned_gaps(pzfx_profile, &cand, &warned_insufficient);
        if conf.has_non_conformant() {
            out.synthetic_non_conformant_cases += 1;
            for v in &conf.violations {
                *out.synthetic_violations.entry(v.clone()).or_insert(0) += 1;
            }
        } else {
            out.synthetic_conformant_cases += 1;
        }
        for (feat, outcome) in &conf.feature_outcomes {
            if matches!(outcome, FeatureOutcome::InsufficientCorpusCoverage { .. }) {
                *out.synthetic_insufficient_features.entry(feat.clone()).or_insert(0) += 1;
            }
        }
    }

    // 3. Re-exported real corpus `.pzfx` files checked against `pzfx_profile`.
    out.reexported_files_checked = reexported_candidates.len();
    for (cand, warned_insufficient) in reexported_candidates {
        let conf = check_conformance_with_warned_gaps(pzfx_profile, cand, warned_insufficient);
        if conf.has_non_conformant() {
            out.reexported_non_conformant_files += 1;
            for v in &conf.violations {
                *out.reexported_violations.entry(v.clone()).or_insert(0) += 1;
            }
        } else {
            out.reexported_conformant_files += 1;
        }
        for (feat, outcome) in &conf.feature_outcomes {
            if matches!(outcome, FeatureOutcome::InsufficientCorpusCoverage { .. }) {
                *out.reexported_insufficient_features.entry(feat.clone()).or_insert(0) += 1;
            }
        }
    }
}

#[cfg(feature = "docs-site")]
fn build_synthetic_pzfx_conformance_suite() -> Vec<(&'static str, curvebook_doc::ProjectDocument)> {
    let mut cases = Vec::new();
    push_synthetic_xy_and_column_cases(&mut cases);
    push_synthetic_grouped_and_other_archetypes(&mut cases);
    push_synthetic_metadata_and_wide_cases(&mut cases);
    cases
}

#[cfg(feature = "docs-site")]
fn push_synthetic_xy_and_column_cases(cases: &mut Vec<(&'static str, curvebook_doc::ProjectDocument)>) {
    use curvebook_doc::{
        Cell, ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SequenceProgression, SubcolumnMode, TableArchetype, XColumnMode, XyTable,
    };

    let all_subcol_modes: &[(&'static str, SubcolumnMode)] = &[
        ("single", SubcolumnMode::Single),
        ("rep3", SubcolumnMode::RawReplicates(3)),
        ("mean_sd", SubcolumnMode::MeanSd),
        ("mean_sem", SubcolumnMode::MeanSem),
        ("mean_sd_n", SubcolumnMode::MeanSdN),
        ("mean_sem_n", SubcolumnMode::MeanSemN),
        ("mean_cv_n", SubcolumnMode::MeanCvN),
        ("mean_low_high", SubcolumnMode::MeanLowHigh),
        ("upper_lower_limits", SubcolumnMode::UpperLowerLimits),
        ("median_iqr", SubcolumnMode::MedianIqr),
    ];

    for &(tag, mode) in all_subcol_modes {
        if let Some(doc) = make_synthetic_xy_doc(mode) {
            cases.push((tag, doc));
        }
        if !matches!(mode, SubcolumnMode::RawReplicates(_))
            && let Some(doc) = make_synthetic_column_doc(mode)
        {
            cases.push((tag, doc));
        }
    }

    let x_mode_specs: &[(&'static str, XColumnMode)] = &[
        ("xy_x_seq", XColumnMode::GeneratedSequence { start: 0.0, step: 2.5, progression: SequenceProgression::Arithmetic }),
        ("xy_x_time", XColumnMode::DateOrElapsedTime { unit_label: String::from("Time") }),
        ("xy_x_err_sd", XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSd }),
        ("xy_x_err_sem", XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSem }),
    ];
    for (tag, x_mode) in x_mode_specs {
        if let Ok(mut doc) = ProjectDocument::new("Synthetic XY XMode")
            && let Ok(mut xy) = XyTable::new_with_x_mode(x_mode.clone(), SubcolumnMode::Single, 1, 3)
        {
            xy.x_title = String::from("Time");
            for r in 0..3 {
                for s in 0..x_mode.subcolumn_count() {
                    if let Some(slot) = xy.x_cells.get_mut(r).and_then(|row| row.get_mut(s)) {
                        *slot = Cell::number((r + 1) as f64 + 0.1 * (s as f64));
                    }
                }
                if let Some(slot) = xy.data_sets.first_mut().and_then(|c| c.cell_mut(r, 0)) {
                    *slot = Cell::number(10.0 + (r as f64));
                }
            }
            if let Ok(sheet) = DataSheet::new("Table0", "XY XMode", TableArchetype::Xy(xy))
                && doc.add_data_sheet(sheet).is_ok()
            {
                cases.push((tag, doc));
            }
        }
    }

    fn make_synthetic_xy_doc(mode: SubcolumnMode) -> Option<ProjectDocument> {
        let mut doc = ProjectDocument::new("Synthetic XY").ok()?;
        let mut xy = XyTable::new(mode, 2, 3).ok()?;
        xy.x_title = String::from("Dose");
        xy.x_display_decimals = Some(2);
        for r in 0..3 {
            if let Some(slot) = xy.x_cells.get_mut(r).and_then(|row| row.first_mut()) {
                *slot = Cell::number((r + 1) as f64);
            }
        }
        for (c, ds) in xy.data_sets.iter_mut().enumerate() {
            ds.title = format!("Series {}", c + 1);
            ds.display_decimals = Some(3);
            for r in 0..3 {
                for s in 0..mode.subcolumn_count() {
                    if let Some(cell) = ds.cell_mut(r, s) {
                        *cell = if r == 2 && s == 0 && c == 0 {
                            Cell::excluded_number(10.0 + (r + c + s) as f64)
                        } else {
                            Cell::number(5.0 + (r * 2 + c + s) as f64)
                        };
                    }
                }
            }
        }
        doc.add_data_sheet(DataSheet::new("Table0", "XY Study", TableArchetype::Xy(xy)).ok()?).ok()?;
        Some(doc)
    }

    fn make_synthetic_column_doc(mode: SubcolumnMode) -> Option<ProjectDocument> {
        let hint = if mode.is_pre_summarized() { ColumnDesignHint::PrecomputedError } else { ColumnDesignHint::UnpairedReplicates };
        let mut doc = ProjectDocument::new("Synthetic Column").ok()?;
        let mut col = ColumnTable::new_with_subcolumns(hint, mode, 2, 3).ok()?;
        for (c, ds) in col.data_sets.iter_mut().enumerate() {
            ds.title = format!("Group {}", c + 1);
            for r in 0..3 {
                for s in 0..mode.subcolumn_count() {
                    if let Some(slot) = ds.cell_mut(r, s) {
                        *slot = Cell::number(20.0 + (r + c + s) as f64);
                    }
                }
            }
        }
        doc.add_data_sheet(DataSheet::new("Table0", "Column Study", TableArchetype::Column(col)).ok()?).ok()?;
        Some(doc)
    }
}

#[cfg(feature = "docs-site")]
fn push_synthetic_grouped_and_other_archetypes(cases: &mut Vec<(&'static str, curvebook_doc::ProjectDocument)>) {
    use curvebook_doc::{
        Cell, ContingencyTable, DataSheet, GroupedTable, MultipleVariablesTable, NestedTable, PartsOfWholeTable, ProjectDocument, SubcolumnMode,
        SurvivalTable, TableArchetype, VariableRole,
    };

    for &(tag, mode) in &[
        ("grouped_rep1", SubcolumnMode::RawReplicates(1)),
        ("grouped_rep3", SubcolumnMode::RawReplicates(3)),
        ("grouped_sdn", SubcolumnMode::MeanSdN),
        ("grouped_sen", SubcolumnMode::MeanSemN),
    ] {
        if let Some(doc) = make_grouped_doc(mode) {
            cases.push((tag, doc));
        }
    }
    if let Some(doc) = make_contingency_doc() {
        cases.push(("contingency", doc));
    }
    if let Some(doc) = make_survival_doc() {
        cases.push(("survival", doc));
    }
    if let Some(doc) = make_parts_doc() {
        cases.push(("parts_of_whole", doc));
    }
    if let Some(doc) = make_mv_doc() {
        cases.push(("multivariable", doc));
    }
    if let Some(doc) = make_nested_doc() {
        cases.push(("nested", doc));
    }

    fn make_grouped_doc(mode: SubcolumnMode) -> Option<ProjectDocument> {
        let mut doc = ProjectDocument::new("Synthetic Grouped").ok()?;
        let mut grp = GroupedTable::new(mode, 2, 2).ok()?;
        grp.row_titles = vec![String::from("Control"), String::from("Treated")];
        for (c, ds) in grp.data_sets.iter_mut().enumerate() {
            ds.title = format!("Dose {}", c + 1);
            if mode.subcolumn_count() > 1 {
                ds.subcolumn_titles = (0..mode.subcolumn_count()).map(|s| format!("Sub {}", s + 1)).collect();
            }
            for r in 0..2 {
                for s in 0..mode.subcolumn_count() {
                    if let Some(slot) = ds.cell_mut(r, s) {
                        *slot = Cell::number(30.0 + (r + c + s) as f64);
                    }
                }
            }
        }
        doc.add_data_sheet(DataSheet::new("Table0", "Grouped Study", TableArchetype::Grouped(grp)).ok()?).ok()?;
        Some(doc)
    }

    fn make_contingency_doc() -> Option<ProjectDocument> {
        let mut doc = ProjectDocument::new("Synthetic Contingency").ok()?;
        let mut cont = ContingencyTable::new(2, 2).ok()?;
        cont.row_titles = vec![String::from("Positive"), String::from("Negative")];
        for (c, ds) in cont.data_sets.iter_mut().enumerate() {
            ds.title = format!("Arm {}", c + 1);
            for r in 0..2 {
                if let Some(slot) = ds.cell_mut(r, 0) {
                    *slot = Cell::number((10 + c * 5 + r * 3) as f64);
                }
            }
        }
        doc.add_data_sheet(DataSheet::new("Table0", "Contingency", TableArchetype::Contingency(cont)).ok()?).ok()?;
        Some(doc)
    }

    fn make_survival_doc() -> Option<ProjectDocument> {
        let mut doc = ProjectDocument::new("Synthetic Survival").ok()?;
        let mut surv = SurvivalTable::new(2, 3).ok()?;
        surv.time_title = String::from("Days");
        for (r, t) in [5.0, 12.0, 20.0].into_iter().enumerate() {
            if let Some(slot) = surv.times.get_mut(r) {
                *slot = Cell::number(t);
            }
        }
        if let Some(slot) = surv.cohorts.first_mut().and_then(|c| c.cell_mut(0, 0)) {
            *slot = Cell::number(1.0);
        }
        if let Some(slot) = surv.cohorts.get_mut(1).and_then(|c| c.cell_mut(1, 0)) {
            *slot = Cell::number(0.0);
        }
        doc.add_data_sheet(DataSheet::new("Table0", "Survival", TableArchetype::Survival(surv)).ok()?).ok()?;
        Some(doc)
    }

    fn make_parts_doc() -> Option<ProjectDocument> {
        let mut doc = ProjectDocument::new("Synthetic PartsOfWhole").ok()?;
        let mut pow = PartsOfWholeTable::new(1, 3).ok()?;
        pow.slice_labels = vec![String::from("A"), String::from("B"), String::from("C")];
        for (r, v) in [25.0, 50.0, 25.0].into_iter().enumerate() {
            if let Some(slot) = pow.data_sets.first_mut().and_then(|c| c.cell_mut(r, 0)) {
                *slot = Cell::number(v);
            }
        }
        doc.add_data_sheet(DataSheet::new("Table0", "Parts", TableArchetype::PartsOfWhole(pow)).ok()?).ok()?;
        Some(doc)
    }

    fn make_mv_doc() -> Option<ProjectDocument> {
        let mut doc = ProjectDocument::new("Synthetic MV").ok()?;
        let mut mv = MultipleVariablesTable::new(2, 2).ok()?;
        if let Some(v0) = mv.variables.first_mut() {
            v0.title = String::from("Age");
            v0.role = VariableRole::Continuous;
            if let Some(c0) = v0.cells.first_mut() {
                *c0 = Cell::number(34.0);
            }
            if let Some(c1) = v0.cells.get_mut(1) {
                *c1 = Cell::number(41.0);
            }
        }
        if let Some(v1) = mv.variables.get_mut(1) {
            v1.title = String::from("Cohort");
            v1.role = VariableRole::Categorical;
            if let Some(c0) = v1.cells.first_mut() {
                *c0 = Cell::text("Alpha");
            }
            if let Some(c1) = v1.cells.get_mut(1) {
                *c1 = Cell::text("Beta");
            }
        }
        doc.add_data_sheet(DataSheet::new("Table0", "Multivariable", TableArchetype::MultipleVariables(mv)).ok()?).ok()?;
        Some(doc)
    }

    fn make_nested_doc() -> Option<ProjectDocument> {
        let mut doc = ProjectDocument::new("Synthetic Nested").ok()?;
        let mut nested = NestedTable::new(2, 3, 2).ok()?;
        for (g, grp) in nested.groups.iter_mut().enumerate() {
            grp.title = format!("Arm {}", g + 1);
            grp.subcolumn_titles = vec![format!("A{g}U1"), format!("A{g}U2"), format!("A{g}U3")];
            for r in 0..2 {
                for s in 0..3 {
                    if let Some(slot) = grp.cells.get_mut(r).and_then(|row| row.get_mut(s)) {
                        *slot = Cell::number((10 + g * 5 + r + s) as f64);
                    }
                }
            }
        }
        doc.add_data_sheet(DataSheet::new("Table0", "Nested", TableArchetype::Nested(nested)).ok()?).ok()?;
        Some(doc)
    }
}

#[cfg(feature = "docs-site")]
fn push_synthetic_metadata_and_wide_cases(cases: &mut Vec<(&'static str, curvebook_doc::ProjectDocument)>) {
    use curvebook_doc::{
        Cell, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, FloatingNote, InfoConstant, InfoSheet, ProjectDocument, RectPt, SheetId,
        SubcolumnMode, TableArchetype, XyTable,
    };
    use curvebook_palette::Rgba8;

    if let Some(doc) = make_rich_metadata_doc() {
        cases.push(("rich_metadata", doc));
    }
    if let Some(doc) = make_wide_column_doc("Synthetic HugeTable", "Huge", 105) {
        cases.push(("huge_table", doc));
    }
    if let Some(doc) = make_wide_column_doc("Synthetic Table1024", "Wide", 257) {
        cases.push(("table1024", doc));
    }

    fn make_rich_metadata_doc() -> Option<ProjectDocument> {
        let mut doc = ProjectDocument::new("Synthetic Rich Metadata").ok()?;
        let mut info0 = InfoSheet::new_preseeded("Info0", "Project Info").ok()?;
        let mut info1 = InfoSheet::new_preseeded("Info1", "Table Info").ok()?;
        let mut xy = XyTable::new(SubcolumnMode::MeanSd, 1, 2).ok()?;
        info0.notes_markdown = String::from("Primary notes");
        info0.constants.clear();
        info0.constants.push(InfoConstant::new("Experiment Date", ConstantValue::Date(String::from("2026-11-26")), None).ok()?);
        info0.constants.push(InfoConstant::new("Lot", ConstantValue::Number(42.0), None).ok()?);
        info1.notes_markdown = String::from("Embedded info notes");
        info1.constants.clear();
        info1.constants.push(InfoConstant::new("Protocol", ConstantValue::Text(String::from("Standard")), None).ok()?);
        if let Some(slot) = xy.x_cells.first_mut().and_then(|row| row.first_mut()) {
            *slot = Cell::number(1.0);
        }
        if let Some(slot) = xy.data_sets.first_mut().and_then(|c| c.cell_mut(0, 0)) {
            *slot = Cell::number(12.0);
        }
        if let Some(slot) = xy.data_sets.first_mut().and_then(|c| c.cell_mut(0, 1)) {
            *slot = Cell::number(1.5);
        }
        let mut sheet = DataSheet::new("Table0", "**Bold** *Italic* H~2~O x^2^", TableArchetype::Xy(xy)).ok()?;
        sheet.color_tag = Some(Rgba8::rgb(255, 255, 0));
        sheet.info_sheet_id = Some(info1.id.clone());
        sheet.floating_notes.push(FloatingNote::new("Note0", RectPt::new(10.0, 20.0, 180.0, 60.0), "**Note** line1\n*line2*"));
        doc.add_info_sheet(info0).ok()?;
        doc.add_info_sheet(info1).ok()?;
        doc.add_data_sheet(sheet).ok()?;
        let _ = doc.select_in_navigator(&SheetId::new("Info0"));
        Some(doc)
    }

    fn make_wide_column_doc(proj_title: &str, sheet_title: &str, col_count: usize) -> Option<ProjectDocument> {
        let mut doc = ProjectDocument::new(proj_title).ok()?;
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, col_count, 1).ok()?;
        for (idx, ds) in col.data_sets.iter_mut().enumerate() {
            if let Some(slot) = ds.cell_mut(0, 0) {
                *slot = Cell::number(idx as f64);
            }
        }
        doc.add_data_sheet(DataSheet::new("Table0", sheet_title, TableArchetype::Column(col)).ok()?).ok()?;
        Some(doc)
    }
}

#[cfg(feature = "docs-site")]
fn format_error_class(err: &curvebook_format::FormatError) -> &'static str {
    use curvebook_format::FormatError;
    match err {
        FormatError::Io(_) => "Io",
        FormatError::Json(_) => "Json",
        FormatError::Doc(_) => "Doc",
        FormatError::InvalidZip { .. } => "InvalidZip",
        FormatError::CorruptCrc { .. } => "CorruptCrc",
        FormatError::UnsupportedCompression { .. } => "UnsupportedCompression",
        FormatError::ZipBomb { .. } => "ZipBomb",
        FormatError::FileTooLarge { .. } => "FileTooLarge",
        FormatError::UnsafePath { .. } => "UnsafePath",
        FormatError::DuplicateEntry { .. } => "DuplicateEntry",
        FormatError::MissingArchiveEntry { .. } => "MissingArchiveEntry",
        FormatError::UnsupportedSchemaVersion { .. } => "UnsupportedSchemaVersion",
        FormatError::InvalidArchiveKind { .. } => "InvalidArchiveKind",
        FormatError::DependencyCycle { .. } => "DependencyCycle",
        FormatError::NonFiniteFloat { .. } => "NonFiniteFloat",
        FormatError::CsvParse { .. } => "CsvParse",
        FormatError::XmlParse { .. } => "XmlParse",
        FormatError::XmlDepthExceeded { .. } => "XmlDepthExceeded",
        FormatError::UnsupportedFormat { .. } => "UnsupportedFormat",
        FormatError::InvalidPipeline { .. } => "InvalidPipeline",
    }
}

fn write_observation_ledger(
    dir: &Path,
    summary: &CorpusRunSummary,
    pzfx_records: &[Value],
    archive_records: &[Value],
    pzfx_profile: &DialectProfile,
    archive_profile: &DialectProfile,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create ledger dir: {e}"))?;
    let mut summary_txt = serde_json::to_string_pretty(&summary.to_json()).map_err(|e| format!("serialize summary: {e}"))?;
    summary_txt.push('\n');
    write_atomic(&dir.join("summary.json"), summary_txt.as_bytes()).map_err(|e| format!("write summary.json: {e}"))?;

    write_jsonl(&dir.join("pzfx-observations.jsonl"), pzfx_records)?;
    write_jsonl(&dir.join("archive-observations.jsonl"), archive_records)?;

    let mut pzfx_prof_txt = serde_json::to_string_pretty(&pzfx_profile.to_json()).map_err(|e| format!("serialize pzfx profile: {e}"))?;
    pzfx_prof_txt.push('\n');
    write_atomic(&dir.join("pzfx-dialect-profile.json"), pzfx_prof_txt.as_bytes()).map_err(|e| format!("write pzfx-dialect-profile.json: {e}"))?;

    let mut arc_prof_txt = serde_json::to_string_pretty(&archive_profile.to_json()).map_err(|e| format!("serialize archive profile: {e}"))?;
    arc_prof_txt.push('\n');
    write_atomic(&dir.join("archive-dialect-profile.json"), arc_prof_txt.as_bytes())
        .map_err(|e| format!("write archive-dialect-profile.json: {e}"))?;
    Ok(())
}

fn write_jsonl(path: &Path, records: &[Value]) -> Result<(), String> {
    let mut out = String::new();
    for rec in records {
        let line = serde_json::to_string(rec).map_err(|e| format!("serialize jsonl record: {e}"))?;
        out.push_str(&line);
        out.push('\n');
    }
    write_atomic(path, out.as_bytes()).map_err(|e| format!("write ledger jsonl: {e}"))
}

fn enforce_strict_gates(summary: &CorpusRunSummary) -> Result<(), String> {
    let mut failures = Vec::new();
    if !summary.provenance_present {
        failures.push(String::from("provenance.tsv is missing"));
    }
    if summary.provenance_missing > 0 {
        failures.push(format!("{} format file(s) missing provenance.tsv entry", summary.provenance_missing));
    }
    if summary.provenance_hash_mismatch > 0 {
        failures.push(format!("{} file(s) failed SHA-256 verification", summary.provenance_hash_mismatch));
    }
    if summary.provenance_size_mismatch > 0 {
        failures.push(format!("{} file(s) failed byte-size verification", summary.provenance_size_mismatch));
    }
    if summary.provenance_missing_on_disk > 0 {
        failures.push(format!("{} provenance.tsv format entries missing on disk", summary.provenance_missing_on_disk));
    }
    if summary.read_errors > 0 {
        failures.push(format!("{} file read error(s)", summary.read_errors));
    }
    let p = &summary.pipeline;
    if p.panics > 0 || p.pzfx_export_panics > 0 || p.pzfx_reimport_panics > 0 {
        failures.push(format!(
            "panic(s) during CurveBook pipeline (import={}, pzfx_export={}, pzfx_reimport={})",
            p.panics, p.pzfx_export_panics, p.pzfx_reimport_panics
        ));
    }
    let analysis_panics: usize = p.archive_analysis_diagnostics.values().map(|d| d.recompute_panics).sum();
    if analysis_panics > 0 {
        failures.push(format!("{analysis_panics} panic(s) during .prism analysis recomputation"));
    }
    if p.enabled {
        if p.pzfx_valid_import_err > 0 {
            failures.push(format!("{} unexpected .pzfx import error(s) on valid XML files", p.pzfx_valid_import_err));
        }
        if p.archive_valid_import_err > 0 {
            failures.push(format!("{} unexpected .prism import error(s) on valid archives", p.archive_valid_import_err));
        }
        if p.pzfx_export_err > 0 {
            failures.push(format!("{} .pzfx export error(s)", p.pzfx_export_err));
        }
        if p.pzfx_reimport_err > 0 {
            failures.push(format!("{} .pzfx re-import error(s)", p.pzfx_reimport_err));
        }
        if p.pzfx_reexport_diff_files > 0 {
            failures.push(format!(
                "{} .pzfx file(s) had {} data-layer diff(s) after re-export ({})",
                p.pzfx_reexport_diff_files,
                p.pzfx_reexport_diffs_total,
                format_map(&p.pzfx_reexport_diff_classes)
            ));
        }
        if p.engine_recalc_err > 0 {
            failures.push(format!("{} engine recalculation error(s)", p.engine_recalc_err));
        }
        if p.curvebook_roundtrip_unexpected_err > 0 || p.curvebook_roundtrip_diffs > 0 {
            failures.push(format!(
                ".curvebook round-trip failures (unexpected_err={}, safety_cap_err={}, diffs={})",
                p.curvebook_roundtrip_unexpected_err, p.curvebook_roundtrip_safety_cap_err, p.curvebook_roundtrip_diffs
            ));
        }
        #[cfg(feature = "docs-site")]
        {
            let missing_mappings: Vec<&str> = curvebook_format::ANALYSIS_MAPPINGS
                .iter()
                .map(|row| row.foreign_class)
                .filter(|cls| !p.archive_analysis_diagnostics.contains_key(*cls))
                .collect();
            if !missing_mappings.is_empty() {
                failures.push(format!(
                    "{} ANALYSIS_MAPPINGS row(s) missing from Step 7 diagnostics ({})",
                    missing_mappings.len(),
                    missing_mappings.join(", ")
                ));
            }
        }
        if p.archive_fidelity_diff_files > 0 {
            failures.push(format!(
                "{} .prism file(s) had {} data-layer diff(s) against CSV/JSON ({})",
                p.archive_fidelity_diff_files,
                p.archive_fidelity_diffs_total,
                format_map(&p.archive_fidelity_diff_classes)
            ));
        }
        let conf = &p.pzfx_conformance;
        if conf.evaluated {
            if !conf.static_table_sync_errors.is_empty() {
                failures.push(format!(
                    "PZFX_FEATURE_CORPUS_COVERAGE sync failed with {} error(s): {}",
                    conf.static_table_sync_errors.len(),
                    conf.static_table_sync_errors.join("; ")
                ));
            }
            if conf.synthetic_determinism_ok < conf.synthetic_cases_total {
                failures
                    .push(format!(".pzfx synthetic determinism failed ({}/{} identical)", conf.synthetic_determinism_ok, conf.synthetic_cases_total));
            }
            if conf.synthetic_roundtrip_diffs > 0 {
                failures.push(format!("{} .pzfx synthetic case(s) had round-trip data-layer diffs", conf.synthetic_roundtrip_diffs));
            }
            if conf.synthetic_non_conformant_cases > 0 {
                failures.push(format!(
                    "{} .pzfx synthetic case(s) failed §6.3 conformance ({})",
                    conf.synthetic_non_conformant_cases,
                    format_map(&conf.synthetic_violations)
                ));
            }
            if conf.reexported_non_conformant_files > 0 {
                failures.push(format!(
                    "{} re-exported corpus .pzfx file(s) failed §6.3 conformance ({})",
                    conf.reexported_non_conformant_files,
                    format_map(&conf.reexported_violations)
                ));
            }
        }
    }
    if failures.is_empty() { Ok(()) } else { Err(format!("interop-corpus --strict failed:\n  {}", failures.join("\n  "))) }
}

fn print_aggregate_summary(s: &CorpusRunSummary) {
    println!("interop-corpus aggregate summary ({}):", s.observed_at);
    println!(
        "  provenance: present={}, verified={}, missing={}, hash_mismatch={}, size_mismatch={}, missing_on_disk={}",
        s.provenance_present,
        s.provenance_verified,
        s.provenance_missing,
        s.provenance_hash_mismatch,
        s.provenance_size_mismatch,
        s.provenance_missing_on_disk
    );
    println!(
        "  skipped: dotfiles={}, manifests_and_logs={}, partial_downloads={}, other_non_format={}, read_errors={}",
        s.skipped_dotfiles, s.skipped_manifests_and_logs, s.skipped_partial_downloads, s.skipped_other_non_format, s.read_errors
    );
    println!("  by_source: {}", format_map(&s.counts_by_source));
    println!("  by_licence: {}", format_map(&s.counts_by_licence));
    println!("  by_format: {}", format_map(&s.counts_by_format));
    println!(
        "  pcff_binary (unsupported by design): total={} (with .pzfx ext={}, with .prism ext={}, with .pzf ext={})",
        s.pcff_binary_total, s.pcff_with_pzfx_ext, s.pcff_with_archive_ext, s.pzf_ext_total
    );
    println!(
        "  size_bytes: .pzfx={:?}, pcff={:?}, .prism={:?}",
        (s.pzfx_size_dist.min, s.pzfx_size_dist.p50, s.pzfx_size_dist.p95, s.pzfx_size_dist.max),
        (s.pcff_size_dist.min, s.pcff_size_dist.p50, s.pcff_size_dist.p95, s.pcff_size_dist.max),
        (s.archive_size_dist.min, s.archive_size_dist.p50, s.archive_size_dist.p95, s.archive_size_dist.max)
    );
    println!(
        "  .pzfx structure: xml_files={}, encodings={}, xml_versions={}, trailing_non_ws={}, template_files={} (elems={}), tables={}, huge_tables={} (files={})",
        s.pzfx_xml_total,
        format_map(&s.pzfx_encodings),
        format_map(&s.pzfx_xml_versions),
        s.pzfx_with_trailing_non_ws,
        s.pzfx_with_template,
        s.pzfx_template_elements_total,
        s.pzfx_tables_total,
        s.pzfx_huge_tables_total,
        s.pzfx_files_with_huge_table
    );
    println!(
        "  .pzfx tables: types={}, y_formats={}, x_formats={}, x_ev_formats={}",
        format_map(&s.pzfx_table_types),
        format_map(&s.pzfx_y_formats),
        format_map(&s.pzfx_x_formats),
        format_map(&s.pzfx_x_ev_formats)
    );
    println!(
        "  .pzfx sequences: TableSequence(refs={}, direct_tables={}, selected={}), InfoSequence(refs={}, direct_infos={}, selected={})",
        s.pzfx_table_seq_refs,
        s.pzfx_table_seq_direct_tables,
        format_map(&s.pzfx_table_seq_selected),
        s.pzfx_info_seq_refs,
        s.pzfx_info_seq_direct_infos,
        format_map(&s.pzfx_info_seq_selected)
    );
    println!(
        "  .pzfx cv_diagnostics: tables={}, y_cols={}, pairs={}, s1>abs(s0)={}, s1_in_0..100={}, log10_slope_r={:?}",
        s.pzfx_cv_diagnostics.cv_tables,
        s.pzfx_cv_diagnostics.cv_y_columns,
        s.pzfx_cv_diagnostics.numeric_pairs,
        s.pzfx_cv_diagnostics.s1_gt_abs_s0_count,
        s.pzfx_cv_diagnostics.s1_values_in_0_to_100,
        s.pzfx_cv_diagnostics.log_log_slope_and_r()
    );
    println!(
        "  .prism structure: files={}, gate_1_3_met={}, major_versions={}, app_versions={}, dialects={}, entry_count(min/p50/p95/max)={:?}, opaque_entries={} (files={}, exts={})",
        s.archive_total,
        s.archive_gate_met,
        format_map(&s.archive_major_versions),
        format_map(&s.archive_app_versions),
        format_map(&s.archive_dialects),
        (s.archive_entry_count_dist.min, s.archive_entry_count_dist.p50, s.archive_entry_count_dist.p95, s.archive_entry_count_dist.max),
        s.archive_opaque_entries_total,
        s.archive_files_with_opaque,
        format_map(&s.archive_opaque_extensions)
    );
    println!(
        "  .prism data: data_formats={}, table_types={}, analysis_classes={}, csv_header_probe={}, x_vs_row_titles={}",
        format_map(&s.archive_data_formats),
        format_map(&s.archive_table_types),
        format_map(&s.archive_analysis_classes),
        format_map(&s.archive_csv_header_probe),
        format_map(&s.archive_x_vs_row_titles_probe)
    );
    if s.pipeline.enabled {
        let p = &s.pipeline;
        println!(
            "  curvebook pipeline: attempted={}, import_ok={}, import_err={}, panics={}, recalc_ok={}, recalc_err={}, roundtrip_ok={}, roundtrip_unexpected_err={}, roundtrip_safety_cap_err={}, roundtrip_diffs={}",
            p.attempted,
            p.import_ok,
            p.import_err,
            p.panics,
            p.engine_recalc_ok,
            p.engine_recalc_err,
            p.curvebook_roundtrip_ok,
            p.curvebook_roundtrip_unexpected_err,
            p.curvebook_roundtrip_safety_cap_err,
            p.curvebook_roundtrip_diffs
        );
        println!(
            "  pipeline per-format import: .pzfx(ok={}, err={}, valid_err={}, corrupt_err={}, safety_cap_err={}, panics={}), .prism(ok={}, err={}, valid_err={}, corrupt_err={}, safety_cap_err={}, panics={})",
            p.pzfx_import_ok,
            p.pzfx_import_err,
            p.pzfx_valid_import_err,
            p.pzfx_known_corrupt_err,
            p.pzfx_safety_cap_err,
            p.pzfx_panics,
            p.archive_import_ok,
            p.archive_import_err,
            p.archive_valid_import_err,
            p.archive_known_corrupt_err,
            p.archive_safety_cap_err,
            p.archive_panics
        );
        if !p.corrupt_file_classes.is_empty() {
            println!("  pipeline corrupt_third_party_files: {}", format_map(&p.corrupt_file_classes));
        }
        if !p.safety_cap_refusal_classes.is_empty() {
            println!("  pipeline safety_cap_refusals: {}", format_map(&p.safety_cap_refusal_classes));
        }
        println!("  pipeline parse_failure_classes: {}", format_map(&p.parse_failure_classes));
        if !p.parse_failure_subclasses.is_empty() {
            println!("  pipeline parse_failure_subclasses: {}", format_map(&p.parse_failure_subclasses));
        }
        if !p.curvebook_roundtrip_err_classes.is_empty() {
            println!("  pipeline curvebook_roundtrip_err_classes: {}", format_map(&p.curvebook_roundtrip_err_classes));
        }
        println!("  pipeline warning_kinds: {}", format_map(&p.warning_kinds));
        println!("  pipeline dropped_sections: {}", format_map(&p.dropped_sections));
        println!("  pipeline archetypes: {}", format_map(&p.archetypes));
        println!("  pipeline subcolumn_modes: {}", format_map(&p.subcolumn_modes));
        println!(
            "  .pzfx re-export fidelity (§6.2): export_ok={}, export_err={}, export_panics={}, reimport_ok={}, reimport_err={}, reimport_panics={}, identical={}, diff_files={}, diffs_total={}, tables={}, cells={}, info_sheets={}, diff_classes={}",
            p.pzfx_export_ok,
            p.pzfx_export_err,
            p.pzfx_export_panics,
            p.pzfx_reimport_ok,
            p.pzfx_reimport_err,
            p.pzfx_reimport_panics,
            p.pzfx_reexport_identical,
            p.pzfx_reexport_diff_files,
            p.pzfx_reexport_diffs_total,
            p.pzfx_reexport_tables_compared,
            p.pzfx_reexport_cells_compared,
            p.pzfx_reexport_info_sheets_compared,
            format_map(&p.pzfx_reexport_diff_classes)
        );
        if !p.pzfx_reimport_err_classes.is_empty() {
            println!("  .pzfx reimport_err_classes: {}", format_map(&p.pzfx_reimport_err_classes));
        }
        if !p.pzfx_reexport_diff_subclasses.is_empty() {
            println!("  .pzfx reexport_diff_subclasses: {}", format_map(&p.pzfx_reexport_diff_subclasses));
        }
        println!(
            "  .pzfx export warnings: kinds={}, dropped={}, insufficient_features={}",
            format_map(&p.pzfx_export_warning_kinds),
            format_map(&p.pzfx_export_dropped_kinds),
            format_map(&p.pzfx_export_insufficient_features)
        );
        println!(
            "  .prism data-layer fidelity (§6.2): files_checked={}, tables_checked={}, cells_checked={}, skipped_with_warning={}, diff_files={}, diffs_total={}, diff_classes={}",
            p.archive_fidelity_files_checked,
            p.archive_fidelity_tables_checked,
            p.archive_fidelity_cells_checked,
            p.archive_fidelity_tables_skipped_with_warning,
            p.archive_fidelity_diff_files,
            p.archive_fidelity_diffs_total,
            format_map(&p.archive_fidelity_diff_classes)
        );
        let c = &p.pzfx_conformance;
        println!(
            "  .pzfx conformance (§6.3): evaluated={}, static_table(rows={}, conformant={}, insufficient={}, sync_errors={}), synthetic(cases={}, det={}, rt_ok={}, rt_diffs={}, conformant={}, non_conformant={}), reexported(checked={}, conformant={}, non_conformant={})",
            c.evaluated,
            c.static_table_rows_total,
            c.static_table_conformant_rows,
            c.static_table_insufficient_rows,
            c.static_table_sync_errors.len(),
            c.synthetic_cases_total,
            c.synthetic_determinism_ok,
            c.synthetic_roundtrip_ok,
            c.synthetic_roundtrip_diffs,
            c.synthetic_conformant_cases,
            c.synthetic_non_conformant_cases,
            c.reexported_files_checked,
            c.reexported_conformant_files,
            c.reexported_non_conformant_files
        );
        if !c.static_table_sync_errors.is_empty() {
            println!("  .pzfx static_table_sync_errors: {:?}", c.static_table_sync_errors);
        }
        if !c.synthetic_violations.is_empty() {
            println!("  .pzfx synthetic_violations: {}", format_map(&c.synthetic_violations));
        }
        if !c.reexported_violations.is_empty() {
            println!("  .pzfx reexported_violations: {}", format_map(&c.reexported_violations));
        }
        if !c.synthetic_insufficient_features.is_empty() || !c.reexported_insufficient_features.is_empty() {
            println!(
                "  .pzfx insufficient_corpus_coverage: synthetic={}, reexported={}",
                format_map(&c.synthetic_insufficient_features),
                format_map(&c.reexported_insufficient_features)
            );
        }
        println!("  .prism analysis diagnostics (Step 7, {} classes):", p.archive_analysis_diagnostics.len());
        for (cls, d) in &p.archive_analysis_diagnostics {
            println!(
                "    {cls}: status={}, kind={}, observed={}, clean={}, blocked={}, unresolv={}, recomp_ok={}, recomp_fail={}, panics={}, scalars(cached={}, recomp={}, compared={}, agreed={}, disagreed={})",
                d.mapping_status,
                d.curvebook_kind,
                d.analyses_observed,
                d.translated_clean,
                d.translated_blocked,
                d.incompatible_or_unresolved_source,
                d.recomputed_ok,
                d.recomputed_failed,
                d.recompute_panics,
                d.cached_scalars_total,
                d.recomputed_scalars_total,
                d.scalars_compared,
                d.scalars_agreed,
                d.scalars_disagreed
            );
        }
    } else {
        println!("  curvebook pipeline: skipped (--structural-only)");
    }
}

fn format_map<K: std::fmt::Display, V: std::fmt::Display>(map: &BTreeMap<K, V>) -> String {
    if map.is_empty() {
        return String::from("{}");
    }
    let parts: Vec<String> = map.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{{{}}}", parts.join(", "))
}

fn merge_counts<K: Ord + Clone>(dst: &mut BTreeMap<K, usize>, src: &BTreeMap<K, usize>) {
    for (k, v) in src {
        *dst.entry(k.clone()).or_insert(0) += *v;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use curvebook_platform::sha256::sha256_hex;

    use super::{
        CandidateStructure, DeclaredCoverage, DialectProfileBuilder, FeatureOutcome, InteropCorpusOptions, StaticFeatureCoverage, check_conformance,
        discover_corpus_files, enforce_strict_gates, observe_archive_bytes, observe_pzfx_bytes, parse_args, parse_provenance_tsv, run_with_options,
        validate_corpus_root, verify_static_coverage_table,
    };
    use crate::scan_util::test_support::TestDir;
    use crate::zip_read::TestZipWriter;

    fn sample_pzfx_xml(y_format: &str, subcols: usize, trailing: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <PrismFile PrismXMLVersion=\"6.00\">\n\
             <Created><OriginalVersion CreatedByProgram=\"Synthetic\" Login=\"tester\" DateTime=\"2026-01-01\"/></Created>\n\
             <InfoSequence><Ref ID=\"Info0\" Selected=\"1\"/></InfoSequence>\n\
             <TableSequence><Ref ID=\"Table0\" Selected=\"1\"/></TableSequence>\n\
             <Info ID=\"Info0\"><Title>Meta</Title><Constant><Name>Date</Name><Value>2026-11-26</Value></Constant></Info>\n\
             <Table ID=\"Table0\" TableType=\"XY\" XFormat=\"numbers\" YFormat=\"{y_format}\" Subcolumns=\"{subcols}\">\n\
             <Title>Synthetic <Font Face=\"Symbol\">a</Font><sup>2</sup></Title>\n\
             <XColumn Decimals=\"2\"><Title>X</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>\n\
             <YColumn Decimals=\"2\"><Title>Y</Title>\
             <Subcolumn><Title>Mean</Title><d>10</d><d Excluded=\"1\">20</d></Subcolumn>\
             <Subcolumn><Title>Err</Title><d>1.5</d><d>2.5</d></Subcolumn>\
             </YColumn>\n\
             </Table>\n\
             <Template>QUJDRA==</Template>\n\
             </PrismFile>{trailing}"
        )
    }

    fn build_synthetic_archive(version: &str, with_opaque: bool) -> Vec<u8> {
        let mut zw = TestZipWriter::new();
        let doc = format!(
            "{{\"formatVersion\":\"1\",\"minFormatVersion\":\"1\",\"createdBy\":{{\"version\":\"{version}\"}},\"sheets\":{{\"data\":[\"11111111-1111-1111-1111-111111111111\"],\"analyses\":[\"22222222-2222-2222-2222-222222222222\"]}},\"sheetAttributesMap\":{{\"11111111-1111-1111-1111-111111111111\":{{\"title\":\"Data 1\"}}}}}}"
        );
        zw.add_file("document.json", doc.into_bytes()).unwrap();
        let sheet =
            r#"{"title":"Data 1","table":{"uid":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","dataFormat":"y_sd","xDataSet":"x1","dataSets":["ds1"]}}"#;
        zw.add_file("data/sheets/11111111-1111-1111-1111-111111111111/sheet.json", sheet.as_bytes().to_vec()).unwrap();
        zw.add_file("data/tables/aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa/data.csv", b"1,10.0,1.2\n2,20.0,2.4\n".to_vec()).unwrap();
        let analysis = r#"{"title":"Fit","preferredResultSheet":"33333333-3333-3333-3333-333333333333","parameters":{"method":"ols"}}"#;
        zw.add_file("analyses/22222222-2222-2222-2222-222222222222/sheet.json", analysis.as_bytes().to_vec()).unwrap();
        zw.add_file("graphs/preview.png", b"\x89PNG\r\n\x1a\n".to_vec()).unwrap();
        if with_opaque {
            zw.add_file("cache/blob.bin", b"\x00\x01\x02\x03".to_vec()).unwrap();
        }
        zw.finish().unwrap()
    }

    #[test]
    fn parse_args_handles_flags_and_rejects_unknown() {
        let parsed = parse_args(&[
            String::from("--strict"),
            String::from("--observe"),
            String::from("--structural-only"),
            String::from("--corpus"),
            String::from("my-dir"),
            String::from("--ledger-dir"),
            String::from("plan/format-observations"),
        ])
        .unwrap();
        assert!(parsed.strict && parsed.observe && parsed.structural_only && !parsed.help);
        assert_eq!(parsed.corpus_override.as_deref(), Some(std::path::Path::new("my-dir")));
        assert!(parse_args(&[String::from("--bogus")]).is_err());
        assert!(parse_args(&[String::from("--corpus")]).is_err());
    }

    #[test]
    fn repo_internal_corpus_requires_gitignored_and_untracked() {
        // 1. Non-gitignored directory inside a git repo must be refused.
        let repo = TestDir::git_repo("interop-unignored");
        repo.write("local-corpus/a.pzfx", &sample_pzfx_xml("SD", 2, ""));
        let err = validate_corpus_root(repo.path(), &repo.join("local-corpus")).unwrap_err();
        assert!(err.contains("not gitignored"), "{err}");

        // 2. Gitignored directory with a force-tracked file must be refused.
        repo.write(".gitignore", "/local-corpus\n");
        repo.run_git(&["add", "-f", "--", "local-corpus/a.pzfx"]);
        let err_tracked = validate_corpus_root(repo.path(), &repo.join("local-corpus")).unwrap_err();
        assert!(err_tracked.contains("tracked git files"), "{err_tracked}");

        // 3. Gitignored directory with zero tracked files must succeed.
        repo.run_git(&["rm", "--cached", "--", "local-corpus/a.pzfx"]);
        assert!(validate_corpus_root(repo.path(), &repo.join("local-corpus")).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_escaping_corpus_root_and_broken_symlinks() {
        use std::os::unix::fs::symlink;

        let outside = TestDir::new("interop-outside");
        let secret = outside.write("secret.pzfx", &sample_pzfx_xml("SD", 2, ""));

        let corpus = TestDir::new("interop-sym-escape");
        symlink(&secret, corpus.join("escape.pzfx")).unwrap();
        let canon = std::fs::canonicalize(corpus.path()).unwrap();
        let err = discover_corpus_files(&canon).unwrap_err();
        assert!(err.contains("symlink escaping corpus root"), "{err}");

        let broken = TestDir::new("interop-sym-broken");
        symlink("/nonexistent/missing.pzfx", broken.join("dangling.pzfx")).unwrap();
        let canon_broken = std::fs::canonicalize(broken.path()).unwrap();
        let err_broken = discover_corpus_files(&canon_broken).unwrap_err();
        assert!(err_broken.contains("broken or unresolvable symlink"), "{err_broken}");
    }

    #[test]
    fn provenance_verification_filtering_and_strict_mode() {
        let repo = TestDir::git_repo("interop-prov-e2e");
        repo.write(".gitignore", "/corpus\n/plan/*\n");
        repo.git_add_all();

        let xml_bytes = sample_pzfx_xml("SD", 2, "TRAILING_TAIL").into_bytes();
        let pcff_bytes = b"PCFF\x00\x01\x02legacy_binary".to_vec();
        let zip_bytes = build_synthetic_archive("10.2.1", true);

        repo.write_bytes("corpus/ext/rec1/a.pzfx", &xml_bytes);
        repo.write_bytes("corpus/ext/rec1/legacy.pzfx", &pcff_bytes);
        repo.write_bytes("corpus/ext/rec2/b.prism", &zip_bytes);
        repo.write("corpus/ext/.DS_Store", "mac");
        repo.write("corpus/ext/fetch.log", "log");
        repo.write("corpus/ext/inprogress.pzfx.part", "partial");

        // CRLF provenance.tsv with valid SHA-256 hashes.
        let tsv = format!(
            "source\tid\tdoi\tlicence\turl\tpath\tbytes\tsha256\r\n\
             zenodo\t101\t10.5281/zenodo.101\tCC-BY-4.0\thttps://example.org/1\trec1/a.pzfx\t{}\t{}\r\n\
             zenodo\t101\t10.5281/zenodo.101\tCC-BY-4.0\thttps://example.org/2\trec1/legacy.pzfx\t{}\t{}\r\n\
             dryad\t202\t10.5061/dryad.202\tCC0-1.0\thttps://example.org/3\trec2/b.prism\t{}\t{}\r\n",
            xml_bytes.len(),
            sha256_hex(&xml_bytes),
            pcff_bytes.len(),
            sha256_hex(&pcff_bytes),
            zip_bytes.len(),
            sha256_hex(&zip_bytes),
        );
        repo.write("corpus/ext/provenance.tsv", &tsv);

        let parsed_prov = parse_provenance_tsv(&tsv).unwrap();
        assert_eq!(parsed_prov.len(), 3);

        let opts = InteropCorpusOptions {
            strict: true,
            observe: true,
            structural_only: true,
            help: false,
            corpus_override: Some(repo.join("corpus/ext")),
            ledger_dir_override: Some(repo.join("plan/format-observations")),
        };
        let summary = run_with_options(repo.path(), &opts).unwrap();
        assert!(enforce_strict_gates(&summary).is_ok(), "{summary:?}");
        assert_eq!(summary.provenance_verified, 3);
        assert_eq!(summary.provenance_missing, 0);
        assert_eq!(summary.provenance_hash_mismatch, 0);
        assert_eq!(summary.skipped_dotfiles, 1);
        assert_eq!(summary.skipped_partial_downloads, 1);
        assert_eq!(summary.skipped_manifests_and_logs, 2); // provenance.tsv + fetch.log
        assert_eq!(summary.pzfx_xml_total, 1);
        assert_eq!(summary.pcff_binary_total, 1);
        assert_eq!(summary.pcff_with_pzfx_ext, 1);
        assert_eq!(summary.archive_total, 1);
        assert_eq!(summary.pzfx_with_trailing_non_ws, 1);
        assert_eq!(summary.pzfx_with_template, 1);
        assert_eq!(summary.pzfx_files_with_symbol_font, 1);
        assert_eq!(summary.pzfx_table_seq_refs, 1);
        assert_eq!(summary.archive_opaque_entries_total, 1);
        assert_eq!(summary.archive_major_versions.get(&10), Some(&1));

        // Ledger outputs were written into gitignored plan/format-observations/.
        assert!(repo.join("plan/format-observations/summary.json").is_file());
        assert!(repo.join("plan/format-observations/pzfx-observations.jsonl").is_file());
        assert!(repo.join("plan/format-observations/archive-observations.jsonl").is_file());
        assert!(repo.join("plan/format-observations/pzfx-dialect-profile.json").is_file());
        assert!(repo.join("plan/format-observations/archive-dialect-profile.json").is_file());

        // Tampering with a file must trigger SHA-256 mismatch and fail --strict.
        repo.write_bytes("corpus/ext/rec1/a.pzfx", b"<PrismFile PrismXMLVersion=\"6.00\"/>");
        let bad_summary = run_with_options(repo.path(), &opts).unwrap();
        assert_eq!(bad_summary.provenance_hash_mismatch, 1);
        assert!(enforce_strict_gates(&bad_summary).unwrap_err().contains("SHA-256"));

        // Unrecorded extra file must trigger provenance_missing and fail --strict.
        repo.write_bytes("corpus/ext/rec1/untracked.pzfx", &xml_bytes);
        let miss_summary = run_with_options(repo.path(), &opts).unwrap();
        assert_eq!(miss_summary.provenance_missing, 1);
        assert!(enforce_strict_gates(&miss_summary).unwrap_err().contains("missing provenance.tsv"));
    }

    #[test]
    fn structural_observers_skip_pcff_and_capture_xml_and_archive_details() {
        assert!(observe_pzfx_bytes(b"PCFFbinary", "id", "t", "s", "r", "l").is_none());
        let xml = sample_pzfx_xml("CV", 2, "\n\x00\x01bin");
        let obs = observe_pzfx_bytes(xml.as_bytes(), "sha256:abc", "unix:1", "zenodo", "zenodo:1", "CC-BY-4.0").unwrap();
        assert_eq!(obs.xml_schema_version.as_deref(), Some("6.00"));
        assert_eq!(obs.trailing_class, "binary");
        assert_eq!(obs.template_elements, 1);
        assert_eq!(obs.cv_diagnostics.cv_tables, 1);
        assert_eq!(obs.cv_diagnostics.numeric_pairs, 2);
        assert_eq!(obs.info_constant_shapes.get("iso_date"), Some(&1));
        assert!(!obs.parent_child_orders.contains_key("Title"), "mixed-content rich-text parents must not impose child order");

        let arc_bytes = build_synthetic_archive("11.0.2", false);
        let arc_obs = observe_archive_bytes(&arc_bytes, "sha256:def", "unix:1", "dryad", "dryad:1", "CC0-1.0");
        assert_eq!(arc_obs.dialect, "external_id_indexed");
        assert_eq!(arc_obs.major_version, Some(11));
        assert_eq!(arc_obs.opaque_entries, 0);
        assert!(arc_obs.has_analysis_params_json);
        assert!(arc_obs.has_graph_previews);
        assert_eq!(arc_obs.csv_header_probe.get("row0_all_numeric_headerless"), Some(&1));
    }

    #[test]
    fn conformance_profile_honors_exclusion_list_outcomes_and_static_table_sync() {
        let mut builder = DialectProfileBuilder::new(".pzfx");
        let order = BTreeMap::from([(String::from("<Root>"), vec![String::from("Created"), String::from("Info"), String::from("Table")])]);

        // 3 files across 2 distinct sources share core features + an excluded Login attr.
        // A rare feature appears in 3 files from ONLY 1 source -> InsufficientCorpusCoverage.
        // A two-file feature appears in 2 files from 2 sources -> InsufficientCorpusCoverage.
        for (fid, rec, extra) in [
            ("f1", "srcA:1", &["feat:single_source_only", "feat:two_files_only"][..]),
            ("f2", "srcA:1", &["feat:single_source_only"][..]),
            ("f3", "srcA:1", &["feat:single_source_only"][..]),
            ("f4", "srcB:2", &["feat:two_files_only"][..]),
        ] {
            let mut feats = BTreeSet::from([
                String::from("elem:<Root>/Table"),
                String::from("attr:Table@YFormat"),
                String::from("attr:OriginalVersion@Login"), // on EXCLUSION_LIST despite 100% presence!
                String::from("y_format:SD|Subcolumns=2"),
            ]);
            for &e in extra {
                feats.insert(e.to_owned());
            }
            builder.record_file(fid, rec, &feats, &order);
        }
        let profile = builder.build();

        // Excluded 100%-present attribute MUST NOT appear in required_features().
        let req = profile.required_features();
        assert!(req.contains("elem:<Root>/Table"));
        assert!(!req.contains("attr:OriginalVersion@Login"), "excluded feature must never be required");

        // 1. Conformant candidate (omits excluded Login, includes required features, respects order).
        let good = CandidateStructure {
            emitted_features: BTreeSet::from([
                String::from("elem:<Root>/Table"),
                String::from("attr:Table@YFormat"),
                String::from("y_format:SD|Subcolumns=2"),
            ]),
            parent_child_orders: BTreeMap::from([(String::from("<Root>"), vec![String::from("Info"), String::from("Table")])]),
            coverage_features: BTreeSet::from([
                String::from("y_format:SD|Subcolumns=2"),
                String::from("feat:single_source_only"),
                String::from("feat:two_files_only"),
            ]),
        };
        let good_report = check_conformance(&profile, &good);
        assert!(!good_report.has_non_conformant(), "{good_report:?}");
        assert_eq!(good_report.feature_outcomes.get("y_format:SD|Subcolumns=2"), Some(&FeatureOutcome::Conformant));
        assert_eq!(
            good_report.feature_outcomes.get("feat:single_source_only"),
            Some(&FeatureOutcome::InsufficientCorpusCoverage { files: 3, sources: 1 })
        );
        assert_eq!(
            good_report.feature_outcomes.get("feat:two_files_only"),
            Some(&FeatureOutcome::InsufficientCorpusCoverage { files: 2, sources: 2 })
        );

        // 2. Non-conformant candidate: emits excluded attr, misses required feature, reverses order.
        let bad = CandidateStructure {
            emitted_features: BTreeSet::from([
                String::from("elem:<Root>/Table"),
                String::from("attr:OriginalVersion@Login"),
                String::from("attr:Table@InventedAttr"),
            ]),
            parent_child_orders: BTreeMap::from([(String::from("<Root>"), vec![String::from("Table"), String::from("Info")])]),
            coverage_features: BTreeSet::new(),
        };
        let bad_report = check_conformance(&profile, &bad);
        assert!(bad_report.has_non_conformant());
        assert!(bad_report.violations.iter().any(|v| v.contains("excluded")), "{bad_report:?}");
        assert!(bad_report.violations.iter().any(|v| v.contains("missing required")), "{bad_report:?}");
        assert!(bad_report.violations.iter().any(|v| v.contains("never observed")), "{bad_report:?}");
        assert!(bad_report.violations.iter().any(|v| v.contains("reversed unanimous child order")), "{bad_report:?}");

        // 3. Static coverage table sync gate rejects over-claimed Conformant rows.
        let valid_table = [
            StaticFeatureCoverage { feature: "y_format:SD|Subcolumns=2", declared: DeclaredCoverage::Conformant },
            StaticFeatureCoverage { feature: "feat:single_source_only", declared: DeclaredCoverage::InsufficientCoverage },
        ];
        assert!(verify_static_coverage_table(&profile, &valid_table).is_ok());

        let overclaimed_table = [StaticFeatureCoverage { feature: "feat:single_source_only", declared: DeclaredCoverage::Conformant }];
        let sync_errs = verify_static_coverage_table(&profile, &overclaimed_table).unwrap_err();
        assert_eq!(sync_errs.len(), 1);
        assert!(sync_errs[0].contains("feat:single_source_only"), "{sync_errs:?}");

        // 4. Warned zero-file gaps classify as InsufficientCorpusCoverage instead of NonConformant.
        let gap_cand = CandidateStructure {
            emitted_features: BTreeSet::from([
                String::from("elem:<Root>/Table"),
                String::from("attr:Table@YFormat"),
                String::from("y_format:SD|Subcolumns=2"),
                String::from("y_format:CVN|Subcolumns=<missing>|Replicates=<missing>"),
            ]),
            parent_child_orders: BTreeMap::new(),
            coverage_features: BTreeSet::new(),
        };
        let warned = BTreeSet::from([String::from("y_format:CVN|Subcolumns=<missing>|Replicates=<missing>")]);
        let gap_rep = super::check_conformance_with_warned_gaps(&profile, &gap_cand, &warned);
        assert!(!gap_rep.has_non_conformant(), "{gap_rep:?}");
        assert_eq!(
            gap_rep.feature_outcomes.get("y_format:CVN|Subcolumns=<missing>|Replicates=<missing>"),
            Some(&FeatureOutcome::InsufficientCorpusCoverage { files: 0, sources: 0 })
        );
        assert_eq!(super::classify_pzfx_diff_location("data[0:SecretTable].col[2][4][1].excluded"), "data.col.excluded");
    }

    #[cfg(feature = "docs-site")]
    #[test]
    fn full_pipeline_verifies_pzfx_roundtrip_archive_fidelity_and_analysis_diagnostics() {
        let repo = TestDir::git_repo("interop-full-pipeline");
        repo.write(".gitignore", "/corpus\n/plan/*\n");
        repo.git_add_all();

        let pzfx_xml = sample_pzfx_xml("SD", 2, "");
        let pzfx_bytes = pzfx_xml.as_bytes().to_vec();

        // Build an external ID-indexed `.prism` archive with a 2-column Column table and a
        // T_TESTS_AND_NONPARAMETRIC analysis (parameters + cached results.json).
        let mut zw = TestZipWriter::new();
        let doc = r#"{
            "formatVersion":"1-6-0",
            "minFormatVersion":"1-0-0",
            "createdBy":{"version":"10.4.1.627"},
            "sheets":{"data":["11111111-1111-1111-1111-111111111111"],"analyses":["22222222-2222-2222-2222-222222222222"]},
            "sheetAttributesMap":{"11111111-1111-1111-1111-111111111111":{"title":"Groups"}}
        }"#;
        zw.add_file("document.json", doc.as_bytes().to_vec()).unwrap();
        let sheet = r#"{
            "title":"Groups",
            "table":{
                "uid":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "format":"column",
                "dataFormat":"y_single",
                "dataSets":["ds1","ds2"]
            }
        }"#;
        zw.add_file("data/sheets/11111111-1111-1111-1111-111111111111/sheet.json", sheet.as_bytes().to_vec()).unwrap();
        let ds1 =
            r#"{"title":"Ctrl","format":"y_single","replicates":[{"valueType":"real","cellAttributes":[{"rows":"2","attributes":["EXCLUDED"]}]}]}"#;
        let ds2 = r#"{"title":"Treat","format":"y_single","replicates":[{"valueType":"real"}]}"#;
        zw.add_file("data/sets/ds1.json", ds1.as_bytes().to_vec()).unwrap();
        zw.add_file("data/sets/ds2.json", ds2.as_bytes().to_vec()).unwrap();
        zw.add_file("data/tables/aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa/data.csv", b"10,20\n12,22\n99,24\n14,26\n".to_vec()).unwrap();
        let an_sheet = r#"{
            "title":"Unpaired t",
            "analysisClass":"T_TESTS_AND_NONPARAMETRIC",
            "inputSheets":[{"uid":"11111111-1111-1111-1111-111111111111","title":"Groups"}]
        }"#;
        let an_params = r#"{"content":{"experimentalDesign":"UNPAIRED_T_TEST","options":{"pValueTailType":"P_VALUE_TWO_TAILED"}}}"#;
        let an_results = r#"{"content":{"status":{"error":"","warning":[]},"pValue":0.001}}"#;
        zw.add_file("analyses/22222222-2222-2222-2222-222222222222/sheet.json", an_sheet.as_bytes().to_vec()).unwrap();
        zw.add_file("analyses/22222222-2222-2222-2222-222222222222/parameters.json", an_params.as_bytes().to_vec()).unwrap();
        zw.add_file("analyses/22222222-2222-2222-2222-222222222222/results.json", an_results.as_bytes().to_vec()).unwrap();
        let arc_bytes = zw.finish().unwrap();

        repo.write_bytes("corpus/ext/rec1/a.pzfx", &pzfx_bytes);
        repo.write_bytes("corpus/ext/rec2/b.prism", &arc_bytes);
        let tsv = format!(
            "source\tid\tdoi\tlicence\turl\tpath\tbytes\tsha256\n\
             zenodo\t101\t10.5281/zenodo.101\tCC-BY-4.0\thttps://example.org/1\trec1/a.pzfx\t{}\t{}\n\
             zenodo\t102\t10.5281/zenodo.102\tCC0-1.0\thttps://example.org/2\trec2/b.prism\t{}\t{}\n",
            pzfx_bytes.len(),
            sha256_hex(&pzfx_bytes),
            arc_bytes.len(),
            sha256_hex(&arc_bytes),
        );
        repo.write("corpus/ext/provenance.tsv", &tsv);

        let opts = InteropCorpusOptions {
            strict: true,
            observe: false,
            structural_only: false,
            help: false,
            corpus_override: Some(repo.join("corpus/ext")),
            ledger_dir_override: None,
        };
        let summary = run_with_options(repo.path(), &opts).unwrap();
        assert!(enforce_strict_gates(&summary).is_ok(), "{summary:?}");
        assert_eq!(summary.pipeline.pzfx_import_ok, 1);
        assert_eq!(summary.pipeline.pzfx_export_ok, 1);
        assert_eq!(summary.pipeline.pzfx_reimport_ok, 1);
        assert_eq!(summary.pipeline.pzfx_reexport_identical, 1);
        assert_eq!(summary.pipeline.pzfx_reexport_diff_files, 0);
        assert_eq!(summary.pipeline.archive_import_ok, 1);
        assert_eq!(summary.pipeline.archive_fidelity_files_checked, 1);
        assert_eq!(summary.pipeline.archive_fidelity_tables_checked, 1);
        assert_eq!(summary.pipeline.archive_fidelity_diff_files, 0);
        assert!(summary.pipeline.archive_fidelity_cells_checked >= 8);
        assert!(summary.pipeline.archive_analysis_diagnostics.len() >= 11);
        let ttest = summary.pipeline.archive_analysis_diagnostics.get("T_TESTS_AND_NONPARAMETRIC").unwrap();
        assert_eq!(ttest.analyses_observed, 1);
        assert_eq!(ttest.translated_clean, 1);
        assert_eq!(ttest.recomputed_ok, 1);
        assert_eq!(ttest.recompute_panics, 0);
    }

    #[cfg(feature = "docs-site")]
    #[test]
    fn pcff_named_archive_corrupt_and_safety_cap_strict_classification() {
        let repo = TestDir::git_repo("interop-strict-classes");
        repo.write(".gitignore", "/corpus\n/plan/*\n");
        repo.git_add_all();

        let pzfx_bytes = sample_pzfx_xml("SD", 2, "").into_bytes();
        let pcff_archive_bytes = b"PCFF\x00\x01\x02legacy_binary_with_archive_ext".to_vec();
        let corrupt_zip_bytes = b"PK\x03\x04truncated_without_eocd".to_vec();

        // Construct a valid `.prism` archive whose table declares > MAX_DATA_SETS columns so it
        // cleanly trips the documented `Doc:TooManyDataSets` safety cap.
        let mut zw = TestZipWriter::new();
        let doc = r#"{
            "formatVersion":"1-6-0",
            "createdBy":{"version":"10.4.1.627"},
            "sheets":{"data":["11111111-1111-1111-1111-111111111111"]},
            "sheetAttributesMap":{"11111111-1111-1111-1111-111111111111":{"title":"TooWide"}}
        }"#;
        zw.add_file("document.json", doc.as_bytes().to_vec()).unwrap();
        let over_cap = curvebook_doc::MAX_DATA_SETS + 1;
        let ds_ids = vec!["\"ds-0\""; over_cap];
        let sheet = format!(
            "{{\"title\":\"TooWide\",\"table\":{{\"uid\":\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\",\"format\":\"column\",\"dataFormat\":\"y_single\",\"dataSets\":[{}]}}}}",
            ds_ids.join(",")
        );
        zw.add_file("data/sheets/11111111-1111-1111-1111-111111111111/sheet.json", sheet.into_bytes()).unwrap();
        zw.add_file("data/sets/ds-0.json", br#"{"title":"Y","replicates":[{"valueType":"real"}]}"#.to_vec()).unwrap();
        zw.add_file("data/tables/aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa/data.csv", b"1\n".to_vec()).unwrap();
        let cap_archive_bytes = zw.finish().unwrap();

        repo.write_bytes("corpus/ext/rec1/valid.pzfx", &pzfx_bytes);
        repo.write_bytes("corpus/ext/rec1/legacy.prism", &pcff_archive_bytes);
        repo.write_bytes("corpus/ext/rec2/corrupt.prism", &corrupt_zip_bytes);
        repo.write_bytes("corpus/ext/rec2/overcap.prism", &cap_archive_bytes);

        let tsv = format!(
            "source\tid\tdoi\tlicence\turl\tpath\tbytes\tsha256\n\
             zenodo\t1\t10.5281/zenodo.1\tCC-BY-4.0\thttps://example.org/1\trec1/valid.pzfx\t{}\t{}\n\
             zenodo\t1\t10.5281/zenodo.1\tCC-BY-4.0\thttps://example.org/2\trec1/legacy.prism\t{}\t{}\n\
             zenodo\t2\t10.5281/zenodo.2\tCC0-1.0\thttps://example.org/3\trec2/corrupt.prism\t{}\t{}\n\
             zenodo\t2\t10.5281/zenodo.2\tCC0-1.0\thttps://example.org/4\trec2/overcap.prism\t{}\t{}\n",
            pzfx_bytes.len(),
            sha256_hex(&pzfx_bytes),
            pcff_archive_bytes.len(),
            sha256_hex(&pcff_archive_bytes),
            corrupt_zip_bytes.len(),
            sha256_hex(&corrupt_zip_bytes),
            cap_archive_bytes.len(),
            sha256_hex(&cap_archive_bytes),
        );
        repo.write("corpus/ext/provenance.tsv", &tsv);

        let opts = InteropCorpusOptions {
            strict: true,
            observe: false,
            structural_only: false,
            help: false,
            corpus_override: Some(repo.join("corpus/ext")),
            ledger_dir_override: None,
        };
        let summary = run_with_options(repo.path(), &opts).unwrap();
        assert!(enforce_strict_gates(&summary).is_ok(), "{summary:?}");
        assert_eq!(summary.pcff_binary_total, 1);
        assert_eq!(summary.pcff_with_archive_ext, 1);
        assert_eq!(summary.archive_total, 2);
        assert_eq!(summary.pipeline.pzfx_import_ok, 1);
        assert_eq!(summary.pipeline.archive_known_corrupt_err, 1);
        assert_eq!(summary.pipeline.corrupt_file_classes.get(".prism:InvalidZip"), Some(&1));
        assert_eq!(summary.pipeline.archive_safety_cap_err, 1);
        assert_eq!(summary.pipeline.safety_cap_refusal_classes.get(".prism:Doc:TooManyDataSets"), Some(&1));
        assert_eq!(summary.pipeline.archive_valid_import_err, 0);
        assert_eq!(summary.pipeline.pzfx_valid_import_err, 0);

        // Strict gates must reject unexpected valid errors, engine recalc errors, unexpected
        // round-trip errors, and missing ANALYSIS_MAPPINGS rows.
        let mut bad_recalc = summary.clone();
        bad_recalc.pipeline.engine_recalc_err = 1;
        assert!(enforce_strict_gates(&bad_recalc).unwrap_err().contains("engine recalculation"));

        let mut bad_rt = summary.clone();
        bad_rt.pipeline.curvebook_roundtrip_unexpected_err = 1;
        assert!(enforce_strict_gates(&bad_rt).unwrap_err().contains(".curvebook round-trip"));

        let mut bad_mapping = summary;
        bad_mapping.pipeline.archive_analysis_diagnostics.remove("T_TESTS_AND_NONPARAMETRIC");
        assert!(enforce_strict_gates(&bad_mapping).unwrap_err().contains("ANALYSIS_MAPPINGS"));
    }
}
