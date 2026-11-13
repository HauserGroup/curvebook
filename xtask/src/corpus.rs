//! Corpus verification and optional on-demand NIST StRD `.dat` fetcher (`docs/development.md`
//! §Verified NIST StRD corpus).
//!
//! Offline `cargo xtask corpus` never accesses the network: it verifies any locally cached files
//! under `corpus/nist-strd/` (SHA-256 + byte size manifest + ASCII `.dat` structural parse) and
//! runs the offline format, testkit, and `lumencraft-math` NIST StRD test suites. Passing `--fetch`
//! downloads the 58 public-domain NIST StRD `.dat` files via `curl -fsSL` into `corpus/nist-strd/`
//! and records `corpus/nist-strd/sha256sums.txt`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use lumencraft_testkit::nist_strd::{
    MAX_STRD_BYTES, PINNED_NIST_STRD_CORPUS, parse_anova_dat, parse_linear_regression_dat, parse_nonlinear_dat, parse_univariate_dat,
};
use lumencraft_testkit::{CorpusManifestEntry, load_verified_corpus_file, sha256_hex, verify_blob};

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

const CORPUS_HELP: &str = "\
usage: cargo xtask corpus [--fetch | --fetch-only]

options:
  --fetch       download canonical public-domain NIST StRD .dat files into corpus/nist-strd/ and run verification suites
  --fetch-only  download canonical public-domain NIST StRD .dat files and write sha256sums.txt without running tests
  -h, --help    print this help message
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorpusOptions {
    pub fetch: bool,
    pub fetch_only: bool,
    pub help: bool,
}

pub fn parse_args(args: &[String]) -> Result<CorpusOptions, String> {
    let mut fetch = false;
    let mut fetch_only = false;
    let mut help = false;
    for arg in args {
        match arg.as_str() {
            "--fetch" => fetch = true,
            "--fetch-only" => fetch_only = true,
            "-h" | "--help" => help = true,
            other => return Err(format!("unknown `cargo xtask corpus` flag `{other}`\n\n{CORPUS_HELP}")),
        }
    }
    Ok(CorpusOptions { fetch, fetch_only, help })
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let opts = parse_args(args)?;
    if opts.help {
        print!("{CORPUS_HELP}");
        return Ok(());
    }

    let nist_root = root.join("corpus").join("nist-strd");
    if opts.fetch || opts.fetch_only {
        let fetched = fetch_nist_strd(&nist_root)?;
        println!("corpus: fetched and recorded {fetched} NIST StRD .dat files in {}", nist_root.display());
        if opts.fetch_only {
            return Ok(());
        }
    }
    if nist_root.exists() {
        let verified = verify_corpus_dir(&nist_root)?;
        println!("corpus: verified {verified} cached NIST StRD .dat file(s) in {}", nist_root.display());
    } else {
        println!("corpus: optional external directory {} not present; running offline suites (pass `--fetch` to populate)", nist_root.display());
    }

    let require_corpus = nist_root.exists() || opts.fetch;
    run_cargo_corpus(root, &["test", "-p", "lumencraft-format", "-p", "lumencraft-testkit"], require_corpus)?;
    run_cargo_corpus(root, &["test", "-p", "lumencraft-math", "--test", "nist_strd"], require_corpus)?;
    Ok(())
}

fn run_cargo_corpus(root: &Path, args: &[&str], require_corpus: bool) -> Result<(), String> {
    println!("+ cargo {}", args.join(" "));
    let mut cmd = Command::new("cargo");
    cmd.current_dir(root).args(args);
    if require_corpus {
        cmd.env("LUMENCRAFT_REQUIRE_CORPUS", "1");
    }
    let status = cmd.status().map_err(|e| format!("cargo {}: {e}", args.join(" ")))?;
    if !status.success() {
        return Err(format!("`cargo {}` failed with {status}", args.join(" ")));
    }
    Ok(())
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

fn verify_manifest_file(nist_root: &Path, manifest_path: &Path, verified_paths: &mut BTreeSet<String>) -> Result<(), String> {
    let manifest_text = fs::read_to_string(manifest_path).map_err(|e| format!("read {}: {e}", manifest_path.display()))?;
    for (idx, raw_line) in manifest_text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line_no = idx + 1;
        let parts: Vec<&str> = line.split_whitespace().collect();
        let (rel_path, text) = match parts.as_slice() {
            [sha_hex, size_tok, rel] => {
                let expected_bytes =
                    size_tok.parse::<usize>().map_err(|_| format!("{}:{line_no}: invalid byte count `{size_tok}`", manifest_path.display()))?;
                let entry = CorpusManifestEntry { relative_path: rel, expected_bytes, sha256_hex: sha_hex };
                let loaded =
                    load_verified_corpus_file(nist_root, entry).map_err(|e| format!("{}:{line_no} ({rel}): {e}", manifest_path.display()))?;
                ((*rel).to_owned(), loaded)
            }
            [sha_hex, rel] => {
                let clean_rel = rel.strip_prefix('*').unwrap_or(rel);
                let bytes = fs::read(nist_root.join(clean_rel)).map_err(|e| format!("read {clean_rel}: {e}"))?;
                verify_blob(&bytes, bytes.len(), sha_hex).map_err(|e| format!("{}:{line_no} ({clean_rel}): {e}", manifest_path.display()))?;
                let loaded = String::from_utf8(bytes).map_err(|_| format!("{clean_rel}: invalid UTF-8"))?;
                (clean_rel.to_owned(), loaded)
            }
            _ => {
                return Err(format!("{}:{line_no}: malformed manifest line `{line}`", manifest_path.display()));
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
    if let Ok(text) = fs::read_to_string(manifest_path) {
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

/// Verify all manifest entries and `.dat` files present under `nist_root`.
pub fn verify_corpus_dir(nist_root: &Path) -> Result<usize, String> {
    if !nist_root.exists() {
        return Ok(0);
    }
    let mut verified_paths = BTreeSet::new();
    let manifest_path = nist_root.join(MANIFEST_FILENAME);
    if manifest_path.is_file() {
        verify_manifest_file(nist_root, &manifest_path, &mut verified_paths)?;
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
        let bytes = fs::read(&full_path).map_err(|e| format!("read {}: {e}", full_path.display()))?;
        if bytes.len() > MAX_STRD_BYTES {
            return Err(format!("{rel}: size {} exceeds {MAX_STRD_BYTES} bytes", bytes.len()));
        }
        let digest = sha256_hex(&bytes);
        verify_blob(&bytes, bytes.len(), &digest).map_err(|e| format!("{rel}: {e}"))?;
        let text = String::from_utf8(bytes).map_err(|_| format!("{rel}: corpus .dat file is not valid UTF-8"))?;
        validate_dat_text(&rel, &text)?;
        verified_paths.insert(rel);
    }

    if should_verify_pinned_catalog(nist_root, &manifest_path, &verified_paths) {
        verify_pinned_catalog_entries(nist_root, &verified_paths)?;
    }

    Ok(verified_paths.len())
}

fn fetch_nist_strd(nist_root: &Path) -> Result<usize, String> {
    for suite in [StrdSuite::Univariate, StrdSuite::Anova, StrdSuite::LinearRegression, StrdSuite::NonlinearRegression] {
        let dir = nist_root.join(suite.subdir());
        fs::create_dir_all(&dir).map_err(|e| format!("create_dir_all {}: {e}", dir.display()))?;
    }

    let mut manifest_lines = Vec::with_capacity(CATALOG.len() + 2);
    manifest_lines.push(String::from("# NIST StRD SHA-256 + byte-size manifest generated by `cargo xtask corpus --fetch`"));

    for item in CATALOG {
        let rel = item.relative_path();
        let dest = nist_root.join(&rel);
        let tmp_dest = dest.with_extension("dat.part");

        let mut downloaded: Option<Vec<u8>> = None;
        let mut last_err = String::new();
        for url in item.candidate_urls() {
            println!("+ curl -fsSL {url}");
            let status = Command::new("curl")
                .arg("-fsSL")
                .arg("--max-time")
                .arg("30")
                .arg("-o")
                .arg(&tmp_dest)
                .arg(&url)
                .status()
                .map_err(|e| format!("failed to execute `curl` for {url}: {e}"))?;
            if !status.success() {
                let _ = fs::remove_file(&tmp_dest);
                last_err = format!("`curl -fsSL {url}` failed with {status}");
                continue;
            }
            let bytes = fs::read(&tmp_dest).map_err(|e| format!("read {}: {e}", tmp_dest.display()))?;
            if bytes.len() > MAX_STRD_BYTES {
                let _ = fs::remove_file(&tmp_dest);
                return Err(format!("{rel}: downloaded file ({} bytes) exceeds {MAX_STRD_BYTES}", bytes.len()));
            }
            let text = match String::from_utf8(bytes.clone()) {
                Ok(s) => s,
                Err(_) => {
                    let _ = fs::remove_file(&tmp_dest);
                    last_err = format!("{rel}: downloaded file from {url} is not valid UTF-8");
                    continue;
                }
            };
            if !text.contains("Dataset Name:") {
                let _ = fs::remove_file(&tmp_dest);
                last_err = format!("{rel}: response from {url} does not look like a NIST StRD .dat file");
                continue;
            }
            downloaded = Some(bytes);
            break;
        }

        let Some(bytes) = downloaded else {
            return Err(last_err);
        };

        fs::rename(&tmp_dest, &dest).map_err(|e| format!("rename {} -> {}: {e}", tmp_dest.display(), dest.display()))?;
        let digest = sha256_hex(&bytes);
        manifest_lines.push(format!("{digest}  {}  {rel}", bytes.len()));
    }

    let mut manifest_body = manifest_lines.join("\n");
    manifest_body.push('\n');
    let manifest_path = nist_root.join(MANIFEST_FILENAME);
    fs::write(&manifest_path, manifest_body).map_err(|e| format!("write {}: {e}", manifest_path.display()))?;

    Ok(CATALOG.len())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use lumencraft_testkit::sha256_hex;

    use super::{CATALOG, CorpusOptions, MANIFEST_FILENAME, StrdSuite, parse_args, verify_corpus_dir};

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
    fn parse_args_handles_default_fetch_help_and_unknown_flags() {
        assert_eq!(parse_args(&[]).unwrap(), CorpusOptions { fetch: false, fetch_only: false, help: false });
        assert_eq!(parse_args(&[String::from("--fetch")]).unwrap(), CorpusOptions { fetch: true, fetch_only: false, help: false });
        assert_eq!(parse_args(&[String::from("--fetch-only")]).unwrap(), CorpusOptions { fetch: false, fetch_only: true, help: false });
        assert_eq!(parse_args(&[String::from("--help")]).unwrap(), CorpusOptions { fetch: false, fetch_only: false, help: true });
        assert!(parse_args(&[String::from("--bogus")]).is_err());
    }

    #[test]
    fn verify_corpus_dir_validates_manifest_digests_and_catches_tampering() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let temp_root = std::env::temp_dir().join(format!("lumencraft-xtask-corpus-test-{}-{stamp}", std::process::id()));
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
            sha256_hex(uni_dat.as_bytes()),
            uni_dat.len(),
            sha256_hex(anova_dat.as_bytes()),
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
        let temp_parent = std::env::temp_dir().join(format!("lumencraft-xtask-pinned-test-{}-{stamp}", std::process::id()));
        let nist_dir = temp_parent.join("nist-strd");
        fs::create_dir_all(&nist_dir).unwrap();

        // An empty or incomplete `nist-strd` directory must fail the 58-entry pinned catalog check.
        let err = verify_corpus_dir(&nist_dir).unwrap_err();
        assert!(err.contains("pinned NIST StRD file missing"), "unexpected error: {err}");

        let _ = fs::remove_dir_all(&temp_parent);

        // When the workspace corpus is present on disk, verify_corpus_dir must verify all 58 pinned files.
        let repo_corpus = lumencraft_testkit::nist_strd::default_nist_strd_dir();
        if lumencraft_testkit::nist_strd::corpus_available(&repo_corpus) {
            assert_eq!(verify_corpus_dir(&repo_corpus).unwrap(), 58);
        }
    }
}
