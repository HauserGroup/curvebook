//! Corpus verification and optional on-demand NIST StRD `.dat` fetcher (`docs/development.md`
//! §Verified NIST StRD corpus).
#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use curvebook_platform::process::ProcessCommand;
use curvebook_platform::{read_regular_file_bytes, write_atomic};
use curvebook_testkit::nist_strd::{
    MAX_STRD_BYTES, PINNED_NIST_STRD_CORPUS, PinnedStrdSpec, parse_anova_dat, parse_linear_regression_dat, parse_nonlinear_dat, parse_univariate_dat,
};
use curvebook_testkit::{CorpusManifestEntry, load_verified_corpus_file, verify_blob};

const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

const NIST_STRD_BASE_URL: &str = "https://www.itl.nist.gov/div898/strd";
const MANIFEST_FILENAME: &str = "sha256sums.txt";
const MAX_CORPUS_FILES: usize = 512;

/// NIST StRD benchmark suite category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrdSuite {
    Univariate,
    Anova,
    LinearRegression,
    NonlinearRegression,
}

impl StrdSuite {
    const fn subdir(self) -> &'static str {
        match self {
            Self::Univariate => "univ",
            Self::Anova => "anova",
            Self::LinearRegression => "lls",
            Self::NonlinearRegression => "nls",
        }
    }

    const fn url_prefixes(self) -> &'static [&'static str] {
        match self {
            Self::Univariate => &["univ/data", "univ/data/LINKS/DATA", "univ"],
            Self::Anova => &["anova", "anova/data/LINKS/DATA", "anova/data"],
            Self::LinearRegression => &["lls/data", "lls/data/LINKS/DATA", "lls"],
            Self::NonlinearRegression => &["nls/data/LINKS/DATA", "nls/data", "nls"],
        }
    }
}

/// Canonical NIST StRD `.dat` dataset descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrdDatasetSpec {
    pub suite: StrdSuite,
    pub name: &'static str,
}

impl StrdDatasetSpec {
    #[must_use]
    pub fn relative_path(self) -> String {
        format!("{}/{}.dat", self.suite.subdir(), self.name)
    }

    #[cfg(test)]
    #[must_use]
    pub fn remote_url(self) -> String {
        self.candidate_urls().into_iter().next().unwrap_or_default()
    }

    #[must_use]
    pub fn candidate_urls(self) -> Vec<String> {
        self.suite.url_prefixes().iter().map(|prefix| format!("{NIST_STRD_BASE_URL}/{prefix}/{}.dat", self.name)).collect()
    }
}

const fn spec(suite: StrdSuite, name: &'static str) -> StrdDatasetSpec {
    StrdDatasetSpec { suite, name }
}

/// All 58 public-domain NIST StRD ASCII `.dat` datasets across the four reference suites.
pub const CATALOG: &[StrdDatasetSpec] = &[
    // Univariate summary statistics (9)
    spec(StrdSuite::Univariate, "PiDigits"),
    spec(StrdSuite::Univariate, "Lottery"),
    spec(StrdSuite::Univariate, "Lew"),
    spec(StrdSuite::Univariate, "Mavro"),
    spec(StrdSuite::Univariate, "Michelso"),
    spec(StrdSuite::Univariate, "NumAcc1"),
    spec(StrdSuite::Univariate, "NumAcc2"),
    spec(StrdSuite::Univariate, "NumAcc3"),
    spec(StrdSuite::Univariate, "NumAcc4"),
    // One-way ANOVA (11)
    spec(StrdSuite::Anova, "SiRstv"),
    spec(StrdSuite::Anova, "AtmWtAg"),
    spec(StrdSuite::Anova, "SmLs01"),
    spec(StrdSuite::Anova, "SmLs02"),
    spec(StrdSuite::Anova, "SmLs03"),
    spec(StrdSuite::Anova, "SmLs04"),
    spec(StrdSuite::Anova, "SmLs05"),
    spec(StrdSuite::Anova, "SmLs06"),
    spec(StrdSuite::Anova, "SmLs07"),
    spec(StrdSuite::Anova, "SmLs08"),
    spec(StrdSuite::Anova, "SmLs09"),
    // Linear least-squares regression (11)
    spec(StrdSuite::LinearRegression, "Norris"),
    spec(StrdSuite::LinearRegression, "Pontius"),
    spec(StrdSuite::LinearRegression, "NoInt1"),
    spec(StrdSuite::LinearRegression, "NoInt2"),
    spec(StrdSuite::LinearRegression, "Filip"),
    spec(StrdSuite::LinearRegression, "Longley"),
    spec(StrdSuite::LinearRegression, "Wampler1"),
    spec(StrdSuite::LinearRegression, "Wampler2"),
    spec(StrdSuite::LinearRegression, "Wampler3"),
    spec(StrdSuite::LinearRegression, "Wampler4"),
    spec(StrdSuite::LinearRegression, "Wampler5"),
    // Nonlinear least-squares regression (27)
    spec(StrdSuite::NonlinearRegression, "Misra1a"),
    spec(StrdSuite::NonlinearRegression, "Chwirut2"),
    spec(StrdSuite::NonlinearRegression, "Chwirut1"),
    spec(StrdSuite::NonlinearRegression, "Lanczos3"),
    spec(StrdSuite::NonlinearRegression, "Gauss1"),
    spec(StrdSuite::NonlinearRegression, "Gauss2"),
    spec(StrdSuite::NonlinearRegression, "DanWood"),
    spec(StrdSuite::NonlinearRegression, "Misra1b"),
    spec(StrdSuite::NonlinearRegression, "Kirby2"),
    spec(StrdSuite::NonlinearRegression, "Hahn1"),
    spec(StrdSuite::NonlinearRegression, "Nelson"),
    spec(StrdSuite::NonlinearRegression, "MGH17"),
    spec(StrdSuite::NonlinearRegression, "Lanczos1"),
    spec(StrdSuite::NonlinearRegression, "Lanczos2"),
    spec(StrdSuite::NonlinearRegression, "Gauss3"),
    spec(StrdSuite::NonlinearRegression, "Misra1c"),
    spec(StrdSuite::NonlinearRegression, "Misra1d"),
    spec(StrdSuite::NonlinearRegression, "Roszman1"),
    spec(StrdSuite::NonlinearRegression, "ENSO"),
    spec(StrdSuite::NonlinearRegression, "MGH09"),
    spec(StrdSuite::NonlinearRegression, "Thurber"),
    spec(StrdSuite::NonlinearRegression, "BoxBOD"),
    spec(StrdSuite::NonlinearRegression, "Rat42"),
    spec(StrdSuite::NonlinearRegression, "MGH10"),
    spec(StrdSuite::NonlinearRegression, "Eckerle4"),
    spec(StrdSuite::NonlinearRegression, "Rat43"),
    spec(StrdSuite::NonlinearRegression, "Bennett5"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusOptions {
    pub fetch: bool,
    pub root: Option<PathBuf>,
}

pub fn parse_args(args: &[String]) -> Result<CorpusOptions, String> {
    let mut fetch = false;
    let mut root: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--fetch" => fetch = true,
            "--root" => {
                let path = iter.next().ok_or_else(|| String::from("missing path after `--root`"))?;
                root = Some(PathBuf::from(path));
            }
            other => return Err(format!("unknown `fetch_corpus` flag `{other}`")),
        }
    }
    Ok(CorpusOptions { fetch, root })
}

fn default_workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| String::from("failed to resolve workspace root from CARGO_MANIFEST_DIR"))
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Err(err) = run(&args) {
        eprintln!("fetch_corpus: {err}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

pub fn run(args: &[String]) -> Result<(), String> {
    let opts = parse_args(args)?;
    let root = match opts.root {
        Some(path) => path,
        None => default_workspace_root()?,
    };
    let nist_root = root.join("corpus").join("nist-strd");
    if opts.fetch {
        let FetchSummary { reused, downloaded } = fetch_nist_strd(&nist_root)?;
        println!(
            "corpus: recorded {} NIST StRD .dat files in {} ({reused} already verified, {downloaded} downloaded)",
            reused + downloaded,
            nist_root.display()
        );
    }
    if nist_root.exists() {
        let verified = verify_corpus_dir(&nist_root)?;
        println!("corpus: verified {verified} cached NIST StRD .dat file(s) in {}", nist_root.display());
    } else {
        println!("corpus: optional external directory {} not present; running offline suites (pass `--fetch` to populate)", nist_root.display());
    }
    Ok(())
}

fn validate_manifest_rel_path(nist_root: &Path, rel: &str, context: &str) -> Result<PathBuf, String> {
    if rel.is_empty()
        || rel.contains(['\\', '\0', ':'])
        || rel.starts_with('/')
        || rel.ends_with('/')
        || rel.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(format!("{context}: invalid relative corpus path `{rel}`"));
    }
    let mut full = nist_root.to_path_buf();
    let mut count = 0_usize;
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) => {
                count += 1;
                full.push(part);
                match fs::symlink_metadata(&full) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        return Err(format!("{context}: corpus path `{rel}` must not be or traverse a symlink"));
                    }
                    Ok(_) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(format!("{context}: inspect `{rel}`: {err}")),
                }
            }
            Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("{context}: corpus path `{rel}` must contain only normal path components"));
            }
        }
    }
    if count == 0 {
        return Err(format!("{context}: empty corpus path"));
    }
    Ok(full)
}

fn read_non_symlink_regular_bytes(path: &Path, max_bytes: usize, label: &str) -> Result<Vec<u8>, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("inspect {label} ({}): {e}", path.display()))?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(format!("{label}: `{}` must be a regular non-symlink file", path.display()));
    }
    let max_u64 = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    if meta.len() > max_u64 {
        return Err(format!("{label}: size {} exceeds {max_bytes} bytes", meta.len()));
    }
    read_regular_file_bytes(path, max_bytes).map_err(|e| format!("read {label} ({}): {e}", path.display()))
}

fn validate_dat_by_suite(suite: StrdSuite, text: &str, label: &str) -> Result<(), String> {
    match suite {
        StrdSuite::Univariate => parse_univariate_dat(text).map(|_| ()).map_err(|e| format!("{label}: {e}")),
        StrdSuite::Anova => parse_anova_dat(text).map(|_| ()).map_err(|e| format!("{label}: {e}")),
        StrdSuite::LinearRegression => parse_linear_regression_dat(text).map(|_| ()).map_err(|e| format!("{label}: {e}")),
        StrdSuite::NonlinearRegression => parse_nonlinear_dat(text).map(|_| ()).map_err(|e| format!("{label}: {e}")),
    }
}

fn infer_suite_for_relative_path(rel_path: &str) -> Option<StrdSuite> {
    if rel_path.starts_with("univ/") {
        return Some(StrdSuite::Univariate);
    }
    if rel_path.starts_with("anova/") {
        return Some(StrdSuite::Anova);
    }
    if rel_path.starts_with("lls/") {
        return Some(StrdSuite::LinearRegression);
    }
    if rel_path.starts_with("nls/") {
        return Some(StrdSuite::NonlinearRegression);
    }
    let stem = Path::new(rel_path).file_stem()?.to_str()?;
    CATALOG.iter().find(|entry| entry.name.eq_ignore_ascii_case(stem)).map(|entry| entry.suite)
}

fn validate_dat_text(rel_path: &str, text: &str) -> Result<(), String> {
    if let Some(suite) = infer_suite_for_relative_path(rel_path) {
        return validate_dat_by_suite(suite, text, rel_path);
    }
    if parse_univariate_dat(text).is_ok()
        || parse_anova_dat(text).is_ok()
        || parse_linear_regression_dat(text).is_ok()
        || parse_nonlinear_dat(text).is_ok()
    {
        return Ok(());
    }
    Err(format!("{rel_path}: file did not match any NIST StRD ASCII suite format"))
}

fn verify_manifest_file(nist_root: &Path, manifest_path: &Path, enforce_pinned: bool, verified_paths: &mut BTreeSet<String>) -> Result<(), String> {
    let manifest_bytes = read_non_symlink_regular_bytes(manifest_path, MAX_MANIFEST_BYTES, "corpus manifest")?;
    let manifest_text = String::from_utf8(manifest_bytes).map_err(|_| format!("read {}: invalid UTF-8", manifest_path.display()))?;
    for (idx, raw_line) in manifest_text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line_no = idx + 1;
        let loc = format!("{}:{line_no}", manifest_path.display());
        let parts: Vec<&str> = line.split_whitespace().collect();
        let (rel_path, text) = match parts.as_slice() {
            [sha_hex, size_tok, rel] => {
                let _full = validate_manifest_rel_path(nist_root, rel, &loc)?;
                if enforce_pinned {
                    if !rel.ends_with(".dat") {
                        return Err(format!("{rel}: unexpected non-`.dat` manifest entry is not in `PINNED_NIST_STRD_CORPUS`"));
                    }
                    if pinned_spec_for_rel(rel).is_none() {
                        return Err(format!("{rel}: unexpected `.dat` file is not in `PINNED_NIST_STRD_CORPUS`"));
                    }
                }
                let expected_bytes = size_tok.parse::<usize>().map_err(|_| format!("{loc}: invalid byte count `{size_tok}`"))?;
                if expected_bytes > MAX_STRD_BYTES {
                    return Err(format!("{loc} ({rel}): declared size {expected_bytes} exceeds {MAX_STRD_BYTES} bytes"));
                }
                let entry = CorpusManifestEntry { relative_path: rel, expected_bytes, sha256_hex: sha_hex };
                let loaded = load_verified_corpus_file(nist_root, entry).map_err(|e| format!("{loc} ({rel}): {e}"))?;
                ((*rel).to_owned(), loaded)
            }
            [sha_hex, rel] => {
                let clean_rel = rel.strip_prefix('*').unwrap_or(rel);
                let full_path = validate_manifest_rel_path(nist_root, clean_rel, &loc)?;
                if enforce_pinned && !clean_rel.ends_with(".dat") {
                    return Err(format!("{clean_rel}: unexpected non-`.dat` manifest entry is not in `PINNED_NIST_STRD_CORPUS`"));
                }
                let pinned = pinned_spec_for_rel(clean_rel);
                if enforce_pinned && pinned.is_none() {
                    return Err(format!("{clean_rel}: unexpected `.dat` file is not in `PINNED_NIST_STRD_CORPUS`"));
                }
                let bytes = read_non_symlink_regular_bytes(&full_path, MAX_STRD_BYTES, clean_rel)?;
                if let Some(spec) = pinned {
                    verify_blob(&bytes, spec.manifest.expected_bytes, sha_hex).map_err(|e| format!("{loc} ({clean_rel}): {e}"))?;
                } else {
                    let meta_len = bytes.len();
                    verify_blob(&bytes, meta_len, sha_hex).map_err(|e| format!("{loc} ({clean_rel}): {e}"))?;
                }
                let loaded = String::from_utf8(bytes).map_err(|_| format!("{clean_rel}: invalid UTF-8"))?;
                (clean_rel.to_owned(), loaded)
            }
            _ => {
                return Err(format!("{loc}: malformed manifest line `{line}`"));
            }
        };
        if rel_path.ends_with(".dat") {
            validate_dat_text(&rel_path, &text)?;
            verified_paths.insert(rel_path);
        }
    }
    Ok(())
}

fn collect_dat_files(root: &Path, current: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(current).map_err(|e| format!("read_dir {}: {e}", current.display()))?;
    for entry_res in entries {
        if out.len() >= MAX_CORPUS_FILES {
            return Err(format!("corpus directory {} exceeds {MAX_CORPUS_FILES} files", root.display()));
        }
        let entry = entry_res.map_err(|e| format!("dir entry in {}: {e}", current.display()))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| format!("stat {}: {e}", path.display()))?;
        if file_type.is_dir() {
            collect_dat_files(root, &path, out)?;
        } else if file_type.is_file() && path.extension().and_then(|s| s.to_str()) == Some("dat") {
            out.push(path);
        }
    }
    Ok(())
}

fn should_verify_pinned_catalog(nist_root: &Path, manifest_path: &Path, verified_paths: &BTreeSet<String>) -> bool {
    if nist_root.file_name().and_then(|s| s.to_str()) == Some("nist-strd") || verified_paths.len() >= PINNED_NIST_STRD_CORPUS.len() {
        return true;
    }
    if let Ok(bytes) = read_non_symlink_regular_bytes(manifest_path, MAX_MANIFEST_BYTES, "corpus manifest")
        && let Ok(text) = String::from_utf8(bytes)
    {
        return text.contains("NIST StRD SHA-256") || PINNED_NIST_STRD_CORPUS.iter().any(|spec| text.contains(spec.manifest.sha256_hex));
    }
    false
}

fn verify_pinned_catalog_entries(nist_root: &Path, verified_paths: &BTreeSet<String>) -> Result<(), String> {
    for spec in PINNED_NIST_STRD_CORPUS {
        let rel = spec.manifest.relative_path;
        if !verified_paths.contains(rel) {
            return Err(format!("pinned NIST StRD file missing from {}: {rel}", nist_root.display()));
        }
        load_verified_corpus_file(nist_root, spec.manifest).map_err(|e| format!("pinned NIST StRD check failed for {rel}: {e}"))?;
    }
    Ok(())
}

fn pinned_spec_for_rel(rel_path: &str) -> Option<&'static PinnedStrdSpec> {
    PINNED_NIST_STRD_CORPUS.iter().find(|spec| spec.manifest.relative_path == rel_path)
}

/// Verify all manifest entries and `.dat` files present under `nist_root`.
pub fn verify_corpus_dir(nist_root: &Path) -> Result<usize, String> {
    if !nist_root.exists() {
        return Ok(0);
    }
    let mut verified_paths = BTreeSet::new();
    let manifest_path = nist_root.join(MANIFEST_FILENAME);
    let enforce_pinned = nist_root.file_name().and_then(|s| s.to_str()) == Some("nist-strd");
    if manifest_path.is_file() {
        verify_manifest_file(nist_root, &manifest_path, enforce_pinned, &mut verified_paths)?;
    }

    let mut dat_files = Vec::new();
    collect_dat_files(nist_root, nist_root, &mut dat_files)?;
    dat_files.sort();

    for full_path in dat_files {
        let rel =
            full_path.strip_prefix(nist_root).map_err(|e| format!("strip_prefix {}: {e}", full_path.display()))?.to_string_lossy().replace('\\', "/");
        if verified_paths.contains(&rel) {
            continue;
        }
        let Some(spec) = pinned_spec_for_rel(&rel) else {
            return Err(format!("{rel}: unmanifested `.dat` file is not in `PINNED_NIST_STRD_CORPUS`"));
        };
        let bytes = read_non_symlink_regular_bytes(&full_path, MAX_STRD_BYTES, &rel)?;
        verify_blob(&bytes, spec.manifest.expected_bytes, spec.manifest.sha256_hex).map_err(|e| format!("{rel}: {e}"))?;
        let text = String::from_utf8(bytes).map_err(|_| format!("{rel}: corpus .dat file is not valid UTF-8"))?;
        validate_dat_text(&rel, &text)?;
        verified_paths.insert(rel);
    }

    if should_verify_pinned_catalog(nist_root, &manifest_path, &verified_paths) {
        for rel in &verified_paths {
            if pinned_spec_for_rel(rel).is_none() {
                return Err(format!("{rel}: unexpected `.dat` file is not in `PINNED_NIST_STRD_CORPUS`"));
            }
        }
        verify_pinned_catalog_entries(nist_root, &verified_paths)?;
    }

    Ok(verified_paths.len())
}

fn verify_candidate_dat(item: StrdDatasetSpec, bytes: &[u8]) -> Result<(), String> {
    let rel = item.relative_path();
    if bytes.len() > MAX_STRD_BYTES {
        return Err(format!("{rel}: downloaded file ({} bytes) exceeds {MAX_STRD_BYTES}", bytes.len()));
    }
    let Some(pinned) = pinned_spec_for_rel(&rel) else {
        return Err(format!("{rel}: missing pinned SHA-256 entry in `PINNED_NIST_STRD_CORPUS`"));
    };
    verify_blob(bytes, pinned.manifest.expected_bytes, pinned.manifest.sha256_hex).map_err(|e| format!("{rel}: pinned verification failed: {e}"))?;
    let text = String::from_utf8(bytes.to_vec()).map_err(|_| format!("{rel}: downloaded file is not valid UTF-8"))?;
    validate_dat_by_suite(item.suite, &text, &rel)
}

fn commit_verified_download(tmp_dest: &Path, dest: &Path, item: StrdDatasetSpec) -> Result<(), String> {
    let bytes = match read_non_symlink_regular_bytes(tmp_dest, MAX_STRD_BYTES, &item.relative_path()) {
        Ok(b) => b,
        Err(err) => {
            let _ = fs::remove_file(tmp_dest);
            return Err(err);
        }
    };
    if let Err(err) = verify_candidate_dat(item, &bytes) {
        let _ = fs::remove_file(tmp_dest);
        return Err(err);
    }
    fs::rename(tmp_dest, dest).map_err(|e| format!("rename {} -> {}: {e}", tmp_dest.display(), dest.display()))
}

fn fetch_single_dataset(item: StrdDatasetSpec, dest: &Path, max_filesize: &str) -> Result<(), String> {
    let rel = item.relative_path();
    let tmp_dest = dest.with_extension("dat.part");
    let tmp_str = tmp_dest.to_str().ok_or_else(|| format!("non-UTF-8 path {}", tmp_dest.display()))?;
    let mut last_err = format!("{rel}: no candidate download URLs configured");

    for url in item.candidate_urls() {
        println!("+ curl -fsSL {url}");
        let out = ProcessCommand::new("curl")
            .args([
                "-fsSL",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                "30",
                "--connect-timeout",
                "15",
                // Retries only transient failures (timeouts and HTTP 408/429/500/502/503/504); a
                // 404 still falls through to the next candidate URL immediately.
                "--retry",
                CURL_RETRIES,
                "--max-filesize",
                max_filesize,
                "-o",
                tmp_str,
                &url,
            ])
            .wall_timeout(Duration::from_secs(120))
            .idle_timeout(Duration::from_secs(30))
            .run()
            .map_err(|e| format!("failed to execute `curl` for {url}: {e}"))?;
        if !out.success {
            let _ = fs::remove_file(&tmp_dest);
            last_err = format!("`curl -fsSL {url}` failed with exit code {:?}", out.status_code);
            continue;
        }
        match commit_verified_download(&tmp_dest, dest, item) {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_err = format!("{err} (from {url})");
            }
        }
    }

    Err(last_err)
}

fn build_pinned_manifest_body() -> String {
    let mut lines = Vec::with_capacity(PINNED_NIST_STRD_CORPUS.len() + 1);
    lines.push(String::from("# NIST StRD SHA-256 + byte-size manifest generated by `cargo xtask corpus --fetch`"));
    for spec in PINNED_NIST_STRD_CORPUS {
        lines.push(format!("{}  {}  {}", spec.manifest.sha256_hex, spec.manifest.expected_bytes, spec.manifest.relative_path));
    }
    let mut body = lines.join("\n");
    body.push('\n');
    body
}

/// Attempts `curl --retry` makes after a transient failure, per candidate URL.
const CURL_RETRIES: &str = "4";

/// Outcome of [`fetch_nist_strd`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FetchSummary {
    /// Files already on disk that passed the pinned check and were kept.
    reused: usize,
    /// Files downloaded and verified in this run.
    downloaded: usize,
}

/// `true` when `dest` already holds exactly the pinned bytes for `item` (SHA-256, byte size, and
/// suite structure), so it can be kept instead of downloaded again. Any read or verification
/// failure means "download it".
fn is_verified_on_disk(item: StrdDatasetSpec, dest: &Path) -> bool {
    read_non_symlink_regular_bytes(dest, MAX_STRD_BYTES, &item.relative_path()).is_ok_and(|bytes| verify_candidate_dat(item, &bytes).is_ok())
}

fn fetch_nist_strd(nist_root: &Path) -> Result<FetchSummary, String> {
    for suite in [StrdSuite::Univariate, StrdSuite::Anova, StrdSuite::LinearRegression, StrdSuite::NonlinearRegression] {
        let dir = nist_root.join(suite.subdir());
        fs::create_dir_all(&dir).map_err(|e| format!("create_dir_all {}: {e}", dir.display()))?;
    }

    let max_filesize = MAX_STRD_BYTES.to_string();
    let mut summary = FetchSummary { reused: 0, downloaded: 0 };
    for item in CATALOG {
        let dest = nist_root.join(item.relative_path());
        if is_verified_on_disk(*item, &dest) {
            summary.reused += 1;
            continue;
        }
        fetch_single_dataset(*item, &dest, &max_filesize)?;
        summary.downloaded += 1;
    }

    let manifest_path = nist_root.join(MANIFEST_FILENAME);
    write_atomic(&manifest_path, build_pinned_manifest_body().as_bytes()).map_err(|e| format!("write {}: {e}", manifest_path.display()))?;

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        CATALOG, CorpusOptions, MANIFEST_FILENAME, MAX_STRD_BYTES, PINNED_NIST_STRD_CORPUS, StrdSuite, build_pinned_manifest_body,
        commit_verified_download, is_verified_on_disk, parse_args, verify_candidate_dat, verify_corpus_dir,
    };

    #[test]
    fn catalog_covers_all_58_nist_strd_datasets_with_unique_paths() {
        assert_eq!(CATALOG.len(), 58);
        let univ = CATALOG.iter().filter(|d| d.suite == StrdSuite::Univariate).count();
        let anova = CATALOG.iter().filter(|d| d.suite == StrdSuite::Anova).count();
        let lls = CATALOG.iter().filter(|d| d.suite == StrdSuite::LinearRegression).count();
        let nls = CATALOG.iter().filter(|d| d.suite == StrdSuite::NonlinearRegression).count();
        assert_eq!((univ, anova, lls, nls), (9, 11, 11, 27));

        let mut paths = BTreeSet::new();
        let mut urls = BTreeSet::new();
        for item in CATALOG {
            assert!(paths.insert(item.relative_path()), "duplicate path: {}", item.relative_path());
            assert!(urls.insert(item.remote_url()), "duplicate url: {}", item.remote_url());
        }
    }

    #[test]
    fn parse_args_handles_fetch_root_and_unknown_flags() {
        assert_eq!(parse_args(&[]).unwrap(), CorpusOptions { fetch: false, root: None });
        assert_eq!(
            parse_args(&[String::from("--fetch"), String::from("--root"), String::from("/work")]).unwrap(),
            CorpusOptions { fetch: true, root: Some(PathBuf::from("/work")) }
        );
        assert!(parse_args(&[String::from("--bogus")]).is_err());
    }

    #[test]
    fn verify_corpus_dir_validates_manifest_digests_and_catches_tampering() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let temp_root = std::env::temp_dir().join(format!("curvebook-xtask-corpus-test-{}-{stamp}", std::process::id()));
        fs::create_dir_all(temp_root.join("univ")).unwrap();
        fs::create_dir_all(temp_root.join("anova")).unwrap();

        let uni_dat = "\
Dataset Name:  NumAcc1            (NumAcc1.dat)
Level of Difficulty:  Lower
Certified Values:
 Sample Mean                        : 10000002.0
 Sample Standard Deviation          :        1.0
 Sample Autocorrelation Coefficient :        0.0
Data:  y
 10000001
 10000002
 10000003
";
        let anova_dat = "\
Dataset Name:  SiRstv             (SiRstv.dat)
File Format:   ASCII
               Data: y, x         (lines 10 to 13)
Level of Difficulty:  Lower
Certified Analysis of Variance Table:
Between Treatments   1   4.00000000000000E+00   4.00000000000000E+00   4.00000000000000E+00
Within Treatments    2   2.00000000000000E+00   1.00000000000000E+00
R-Squared: 6.66666666666667E-01
Residual Standard Deviation: 1.00000000000000E+00
Data:  y   x
 10.0   1
 12.0   1
 12.0   2
 14.0   2
";
        let uni_path = temp_root.join("univ/NumAcc1.dat");
        let anova_path = temp_root.join("anova/SiRstv.dat");
        fs::write(&uni_path, uni_dat).unwrap();
        fs::write(&anova_path, anova_dat).unwrap();

        let manifest = format!(
            "# test manifest\n{}  {}  univ/NumAcc1.dat\n{}  {}  anova/SiRstv.dat\n",
            curvebook_testkit::sha256_hex(uni_dat.as_bytes()),
            uni_dat.len(),
            curvebook_testkit::sha256_hex(anova_dat.as_bytes()),
            anova_dat.len(),
        );
        fs::write(temp_root.join(MANIFEST_FILENAME), manifest).unwrap();

        assert_eq!(verify_corpus_dir(&temp_root).unwrap(), 2);

        // Tampering with a byte must fail SHA-256 / size verification.
        let tampered = uni_dat.replace("10000003", "10000004");
        fs::write(&uni_path, tampered).unwrap();
        assert!(verify_corpus_dir(&temp_root).is_err());

        let _ = fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn verify_corpus_dir_enforces_pinned_catalog_on_nist_strd_directory() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let temp_parent = std::env::temp_dir().join(format!("curvebook-xtask-pinned-test-{}-{stamp}", std::process::id()));
        let nist_dir = temp_parent.join("nist-strd");
        fs::create_dir_all(&nist_dir).unwrap();

        // An empty or incomplete `nist-strd` directory must fail the 58-entry pinned catalog check.
        let err = verify_corpus_dir(&nist_dir).unwrap_err();
        assert!(err.contains("pinned NIST StRD file missing"), "unexpected error: {err}");

        let _ = fs::remove_dir_all(&temp_parent);

        // When the workspace corpus is present on disk, verify_corpus_dir must verify all 58 pinned files.
        let repo_corpus = curvebook_testkit::nist_strd::default_nist_strd_dir();
        if curvebook_testkit::nist_strd::corpus_available(&repo_corpus) {
            assert_eq!(verify_corpus_dir(&repo_corpus).unwrap(), 58);
        }
    }

    #[test]
    fn pinned_catalog_matches_fetch_catalog_and_manifest_builder() {
        assert_eq!(CATALOG.len(), PINNED_NIST_STRD_CORPUS.len());
        let pinned_paths: BTreeSet<&str> = PINNED_NIST_STRD_CORPUS.iter().map(|s| s.manifest.relative_path).collect();
        for item in CATALOG {
            assert!(pinned_paths.contains(item.relative_path().as_str()), "missing pinned entry for {}", item.relative_path());
        }

        let body = build_pinned_manifest_body();
        let data_lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).collect();
        assert_eq!(data_lines.len(), PINNED_NIST_STRD_CORPUS.len());
        for (line, spec) in data_lines.iter().zip(PINNED_NIST_STRD_CORPUS.iter()) {
            let expected = format!("{}  {}  {}", spec.manifest.sha256_hex, spec.manifest.expected_bytes, spec.manifest.relative_path);
            assert_eq!(*line, expected);
        }
    }

    #[test]
    fn commit_verified_download_rejects_mismatch_and_preserves_existing_file() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("curvebook-xtask-fetch-pin-{}-{stamp}", std::process::id()));
        fs::create_dir_all(dir.join("univ")).unwrap();

        let dest = dir.join("univ/NumAcc1.dat");
        let tmp = dest.with_extension("dat.part");
        let original = b"preserved-original-corpus-bytes";
        fs::write(&dest, original).unwrap();

        // Syntactically valid NIST header, wrong SHA-256 digest vs pinned NumAcc1.dat.
        let spoofed = b"Dataset Name:  NumAcc1\nData:  y\n 1\n 2\n";
        fs::write(&tmp, spoofed).unwrap();

        let err = commit_verified_download(&tmp, &dest, CATALOG[5]).unwrap_err();
        assert!(err.contains("pinned verification failed"), "unexpected error: {err}");
        assert!(!tmp.exists(), "staged temp file should be deleted on pin mismatch");
        assert_eq!(fs::read(&dest).unwrap(), original, "existing destination file must not be overwritten");

        // Oversized buffer rejected before touching filesystem.
        let oversized = vec![b' '; MAX_STRD_BYTES + 1];
        assert!(verify_candidate_dat(CATALOG[5], &oversized).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cached_files_are_reused_only_when_they_match_the_pinned_bytes() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("curvebook-xtask-cache-reuse-{}-{stamp}", std::process::id()));
        fs::create_dir_all(dir.join("univ")).unwrap();
        let item = CATALOG[5];
        let dest = dir.join(item.relative_path());

        assert!(!is_verified_on_disk(item, &dest), "a missing file must be downloaded");

        // A stale or tampered cache entry (valid-looking header, wrong digest) must be downloaded again.
        fs::write(&dest, b"Dataset Name:  NumAcc1\nData:  y\n 1\n 2\n").unwrap();
        assert!(!is_verified_on_disk(item, &dest), "a tampered file must be downloaded again");

        // An oversized cache entry is rejected without being trusted.
        fs::write(&dest, vec![b' '; MAX_STRD_BYTES + 1]).unwrap();
        assert!(!is_verified_on_disk(item, &dest), "an oversized file must be downloaded again");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_corpus_dir_rejects_unmanifested_stray_dat_and_tampered_unmanifested_pinned_dat() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("curvebook-xtask-stray-dat-{}-{stamp}", std::process::id()));
        fs::create_dir_all(dir.join("anova")).unwrap();

        // Unmanifested stray .dat file even in a non-nist-strd folder must fail instead of passing a self-digest tautology.
        let stray = dir.join("anova/stray.dat");
        fs::write(&stray, "Dataset Name:  Stray\nCertified Analysis of Variance Table:\nBetween Treatments   999999999   1.0   1.0   1.0\n").unwrap();
        let err = verify_corpus_dir(&dir).unwrap_err();
        assert!(err.contains("not in `PINNED_NIST_STRD_CORPUS`"), "unexpected error: {err}");
        let _ = fs::remove_file(&stray);

        // Even inside an `nist-strd` directory with a matching manifest line, unpinned `.dat` entries are rejected.
        let nist_stray_dir = dir.join("nist-strd");
        fs::create_dir_all(nist_stray_dir.join("anova")).unwrap();
        let valid_anova = "Dataset Name:  SiRstv\nCertified Analysis of Variance Table:\nBetween Treatments   1   4.0   4.0   4.0\nWithin Treatments    2   2.0   1.0\nR-Squared: 0.66666666666667\nResidual Standard Deviation: 1.0\nData:  y   x\n 10.0   1\n 12.0   1\n 12.0   2\n 14.0   2\n";
        fs::write(nist_stray_dir.join("anova/stray.dat"), valid_anova).unwrap();
        let manifest =
            format!("# NIST StRD SHA-256\n{}  {}  anova/stray.dat\n", curvebook_testkit::sha256_hex(valid_anova.as_bytes()), valid_anova.len(),);
        fs::write(nist_stray_dir.join(MANIFEST_FILENAME), manifest).unwrap();
        let err_manifested = verify_corpus_dir(&nist_stray_dir).unwrap_err();
        assert!(
            err_manifested.contains("anova/stray.dat: unexpected `.dat` file is not in `PINNED_NIST_STRD_CORPUS`"),
            "unexpected error: {err_manifested}"
        );

        // An unmanifested file whose relative path IS in PINNED_NIST_STRD_CORPUS still must match its pinned SHA-256.
        let unmanifested_pinned = dir.join("anova/SiRstv.dat");
        fs::write(&unmanifested_pinned, "Dataset Name:  SiRstv\nData: y x\n 1 1\n").unwrap();
        let err2 = verify_corpus_dir(&dir).unwrap_err();
        assert!(err2.contains("anova/SiRstv.dat"), "unexpected error: {err2}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fa_sec23_02_manifest_rejects_traversal_unknown_non_dat_and_two_column_size_bypass() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let base = std::env::temp_dir().join(format!("cb-xtask-sec23-02-{}-{stamp}", std::process::id()));
        let nist_dir = base.join("nist-strd");
        fs::create_dir_all(nist_dir.join("univ")).unwrap();

        // 1. Traversal relative path rejected cleanly before read.
        let bad_rel = format!("{}  3  ../escape.dat\n", curvebook_testkit::sha256_hex(b"abc"));
        fs::write(nist_dir.join(MANIFEST_FILENAME), bad_rel).unwrap();
        assert!(verify_corpus_dir(&nist_dir).unwrap_err().contains("invalid relative corpus path"));

        // 2. Non-.dat two-column entry rejected in pinned `nist-strd` manifest.
        let extra = nist_dir.join("README.txt");
        fs::write(&extra, b"arbitrary").unwrap();
        let non_dat_manifest = format!("{}  README.txt\n", curvebook_testkit::sha256_hex(b"arbitrary"));
        fs::write(nist_dir.join(MANIFEST_FILENAME), non_dat_manifest).unwrap();
        assert!(verify_corpus_dir(&nist_dir).unwrap_err().contains("non-`.dat`"));

        // 3. Two-column pinned `.dat` entry checks against pinned `expected_bytes` rather than `bytes.len()`.
        let pinned_numacc1 = PINNED_NIST_STRD_CORPUS.iter().find(|s| s.manifest.relative_path == "univ/NumAcc1.dat").unwrap();
        let spoofed = b"Dataset Name:  NumAcc1\nData:  y\n 1\n 2\n";
        fs::write(nist_dir.join(pinned_numacc1.manifest.relative_path), spoofed).unwrap();
        let two_col = format!("{}  univ/NumAcc1.dat\n", curvebook_testkit::sha256_hex(spoofed));
        fs::write(nist_dir.join(MANIFEST_FILENAME), two_col).unwrap();
        let err = verify_corpus_dir(&nist_dir).unwrap_err();
        assert!(err.contains("size mismatch"), "expected pinned size mismatch, got {err}");

        let _ = fs::remove_dir_all(&base);
    }
}
