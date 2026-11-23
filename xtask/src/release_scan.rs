//! `cargo xtask release-scan`: unpack release assets in isolated temporary trees via
//! `curvebook-platform` bounded subprocesses, verify `SHA256SUMS.txt` parity, and run the
//! `brands`, `cleanroom`, and `assets` distribution scanners over every unpacked container (`R0.26`).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use curvebook_format::{ZipArchive, ZipLimits};
use curvebook_platform::process::ProcessCommand;
use curvebook_platform::{PlatformError, read_regular_file_bytes, write_atomic};

const MAX_REPORT_JSON_BYTES: usize = 16 * 1024 * 1024;
const MAX_CHECKSUMS_BYTES: usize = 4 * 1024 * 1024;
const MAX_COMPLIANCE_TEXT_BYTES: usize = 32 * 1024 * 1024;
const MAX_ZSYNC_BYTES: usize = 64 * 1024 * 1024;
const MAX_CONTAINER_FILE_BYTES: usize = 1024 * 1024 * 1024;
const MAX_APPIMAGE_RUNTIME_SCAN_BYTES: usize = 32 * 1024 * 1024;
const MAX_MOUNTED_FILE_BYTES: usize = 512 * 1024 * 1024;

const USAGE: &str = "\
usage: cargo xtask release-scan [--require-checksums | --no-checksums] [--expect-beta-set <version>] [--cross-os-reports <dir>] [--report <json-out>] <assets-dir>

Unpacks every release container in <assets-dir> (.zip, .tar.gz/.tgz, .deb, .rpm, .AppImage,
.flatpak, .dmg on macOS, .msi on Windows) plus plain compliance files (SHA256SUMS.txt,
THIRD-PARTY-LICENSES.html, *.zsync), verifies SHA-256 digests against SHA256SUMS.txt by default,
optionally checks the exact R0.28 Beta asset set (--expect-beta-set <version>) and verifies
cross-OS .dmg/.msi builds via native scan reports (--cross-os-reports <dir>), fails closed on
unknown/unextractable formats, and runs brands + cleanroom + assets checks.";

const CHECKSUMS_FILE: &str = "SHA256SUMS.txt";
const LICENSES_HTML_FILE: &str = "THIRD-PARTY-LICENSES.html";

const SUBPROCESS_WALL_TIMEOUT: Duration = Duration::from_secs(180);
const SUBPROCESS_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const SUBPROCESS_STREAM_CAP_BYTES: usize = 64 * 1024 * 1024;
const CPIO_STREAM_CAP_BYTES: usize = 512 * 1024 * 1024;

const RELEASE_ZIP_LIMITS: ZipLimits =
    ZipLimits { max_entries: 16_384, max_total_uncompressed_bytes: 1024 * 1024 * 1024, max_entry_uncompressed_bytes: 512 * 1024 * 1024 };

#[cfg(unix)]
const SAFE_TOOL_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin:/opt/homebrew/bin";

static SCRATCH_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChecksumMode {
    Required,
    Disabled,
}

#[derive(Debug, Clone)]
pub(crate) struct ScanOptions {
    pub assets_dir: PathBuf,
    pub checksum_mode: ChecksumMode,
    pub report_path: Option<PathBuf>,
    pub expect_beta_set: Option<String>,
    pub cross_os_reports_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetKind {
    Checksums,
    HtmlCompliance,
    Zsync,
    Zip,
    TarGz,
    Deb,
    Rpm,
    AppImage,
    Flatpak,
    Dmg,
    Msi,
}

impl AssetKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Checksums => "checksums",
            Self::HtmlCompliance => "html",
            Self::Zsync => "zsync",
            Self::Zip => "zip",
            Self::TarGz => "tar.gz",
            Self::Deb => "deb",
            Self::Rpm => "rpm",
            Self::AppImage => "AppImage",
            Self::Flatpak => "flatpak",
            Self::Dmg => "dmg",
            Self::Msi => "msi",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ScannedAssetRecord {
    pub name: String,
    pub kind: &'static str,
    pub size_bytes: u64,
    pub sha256: String,
    pub unpacked_files: usize,
    pub nested_archive_entries: usize,
    pub checked_assets: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct ReleaseScanReport {
    pub ok: bool,
    pub host_os: String,
    pub checksum_verified: bool,
    pub asset_count: usize,
    pub total_unpacked_files: usize,
    pub total_nested_archive_entries: usize,
    pub total_checked_assets: usize,
    pub assets: Vec<ScannedAssetRecord>,
}

impl ReleaseScanReport {
    fn to_json_value(&self) -> serde_json::Value {
        let items: Vec<serde_json::Value> = self
            .assets
            .iter()
            .map(|a| {
                serde_json::json!({
                    "name": a.name,
                    "kind": a.kind,
                    "size_bytes": a.size_bytes,
                    "sha256": a.sha256,
                    "unpacked_files": a.unpacked_files,
                    "nested_archive_entries": a.nested_archive_entries,
                    "checked_assets": a.checked_assets,
                })
            })
            .collect();
        serde_json::json!({
            "ok": self.ok,
            "host_os": self.host_os,
            "checksum_verified": self.checksum_verified,
            "asset_count": self.asset_count,
            "total_unpacked_files": self.total_unpacked_files,
            "total_nested_archive_entries": self.total_nested_archive_entries,
            "total_checked_assets": self.total_checked_assets,
            "assets": items,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ToolConfig {
    pub path_env: Option<String>,
    pub tar_bin: String,
    pub bsdtar_bin: String,
    pub ar_bin: String,
    pub rpm2cpio_bin: String,
    pub cpio_bin: String,
    pub unsquashfs_bin: String,
    pub flatpak_bin: String,
    pub ostree_bin: String,
    pub hdiutil_bin: String,
    pub msiexec_bin: String,
    pub allow_appimage_exec: bool,
    pub allow_dmg: bool,
    pub allow_msi: bool,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            #[cfg(unix)]
            path_env: Some(String::from(SAFE_TOOL_PATH)),
            #[cfg(not(unix))]
            path_env: None,
            tar_bin: String::from("tar"),
            bsdtar_bin: String::from("bsdtar"),
            ar_bin: String::from("ar"),
            rpm2cpio_bin: String::from("rpm2cpio"),
            cpio_bin: String::from("cpio"),
            unsquashfs_bin: String::from("unsquashfs"),
            flatpak_bin: String::from("flatpak"),
            ostree_bin: String::from("ostree"),
            hdiutil_bin: String::from("/usr/bin/hdiutil"),
            msiexec_bin: String::from("msiexec"),
            allow_appimage_exec: false,
            allow_dmg: cfg!(target_os = "macos"),
            allow_msi: cfg!(windows),
        }
    }
}

impl ToolConfig {
    fn from_env() -> Self {
        let mut tools = Self::default();
        if let Some(msiexec) = std::env::var_os("CURVEBOOK_RELEASE_SCAN_MSIEXEC").filter(|v| !v.is_empty()) {
            tools.msiexec_bin = msiexec.to_string_lossy().into_owned();
            tools.allow_msi = true;
        }
        tools
    }
}

struct ScratchDirGuard {
    path: PathBuf,
}

impl ScratchDirGuard {
    fn new(label: &str) -> Result<Self, String> {
        let seq = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-release-scan-{label}-{}-{nanos}-{seq}", std::process::id()));
        std::fs::create_dir_all(&path).map_err(|e| format!("create scratch dir {}: {e}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for ScratchDirGuard {
    fn drop(&mut self) {
        make_tree_writable_best_effort(&self.path);
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(unix)]
fn make_tree_writable_best_effort(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    if meta.is_dir() {
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o700);
        let _ = std::fs::set_permissions(dir, perms);
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                make_tree_writable_best_effort(&entry.path());
            }
        }
    } else if meta.is_file() {
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o600);
        let _ = std::fs::set_permissions(dir, perms);
    }
}

#[cfg(not(unix))]
fn make_tree_writable_best_effort(_dir: &Path) {}

struct DmgMountGuard<'a> {
    tools: &'a ToolConfig,
    mountpoint: PathBuf,
    mounted: bool,
}

impl<'a> DmgMountGuard<'a> {
    fn new(tools: &'a ToolConfig, mountpoint: PathBuf) -> Self {
        Self { tools, mountpoint, mounted: true }
    }

    fn detach_checked(mut self) -> Result<(), String> {
        self.mounted = false;
        detach_dmg_mount(self.tools, &self.mountpoint)
    }
}

impl Drop for DmgMountGuard<'_> {
    fn drop(&mut self) {
        if self.mounted {
            let _ = detach_dmg_mount(self.tools, &self.mountpoint);
        }
    }
}

fn detach_dmg_mount(tools: &ToolConfig, mountpoint: &Path) -> Result<(), String> {
    let mnt = mountpoint.to_string_lossy();
    let first = build_cmd(tools, &tools.hdiutil_bin).args(["detach", mnt.as_ref()]).run();
    if let Ok(out) = &first
        && out.success
    {
        return Ok(());
    }
    let forced = build_cmd(tools, &tools.hdiutil_bin)
        .args(["detach", "-force", mnt.as_ref()])
        .run()
        .map_err(|e| format!("hdiutil detach -force {mnt}: {e}"))?;
    if !forced.success {
        return Err(format!("hdiutil detach -force {mnt} failed ({:?}): {}", forced.status_code, String::from_utf8_lossy(&forced.stderr).trim()));
    }
    Ok(())
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    if matches!(args.first().map(String::as_str), Some("-h" | "--help" | "help")) {
        println!("{USAGE}");
        return Ok(());
    }
    let options = parse_args(args)?;
    let tools = ToolConfig::from_env();
    let report = scan_release_dir_with_tools(root, &options, &tools)?;
    if let Some(report_path) = &options.report_path {
        if let Some(parent) = report_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| format!("create report directory {}: {e}", parent.display()))?;
        }
        let mut json = serde_json::to_vec_pretty(&report.to_json_value()).map_err(|e| format!("serialize release-scan report: {e}"))?;
        json.push(b'\n');
        write_atomic(report_path, &json).map_err(|e| format!("write report {}: {e}", report_path.display()))?;
    }
    println!(
        "release-scan: verified {} asset(s) in {} ({} unpacked files, {} nested archive entries, {} packaged assets, checksums={}): 0 violations",
        report.asset_count,
        options.assets_dir.display(),
        report.total_unpacked_files,
        report.total_nested_archive_entries,
        report.total_checked_assets,
        if report.checksum_verified { "verified" } else { "skipped" }
    );
    Ok(())
}

fn parse_args(args: &[String]) -> Result<ScanOptions, String> {
    let mut assets_dir: Option<PathBuf> = None;
    let mut saw_require = false;
    let mut saw_no_checksums = false;
    let mut report_path: Option<PathBuf> = None;
    let mut expect_beta_set: Option<String> = None;
    let mut cross_os_reports_dir: Option<PathBuf> = None;

    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();
        match arg {
            "--require-checksums" => {
                saw_require = true;
                idx += 1;
            }
            "--no-checksums" => {
                saw_no_checksums = true;
                idx += 1;
            }
            "--report" => {
                let val = take_flag_value(args, idx, "--report")?;
                if report_path.is_some() {
                    return Err(format!("duplicate `--report` argument\n\n{USAGE}"));
                }
                report_path = Some(PathBuf::from(val));
                idx += 2;
            }
            "--expect-beta-set" => {
                let val = take_flag_value(args, idx, "--expect-beta-set")?;
                if expect_beta_set.is_some() {
                    return Err(format!("duplicate `--expect-beta-set` argument\n\n{USAGE}"));
                }
                expect_beta_set = Some(normalize_release_version(val)?);
                idx += 2;
            }
            "--cross-os-reports" => {
                let val = take_flag_value(args, idx, "--cross-os-reports")?;
                if cross_os_reports_dir.is_some() {
                    return Err(format!("duplicate `--cross-os-reports` argument\n\n{USAGE}"));
                }
                cross_os_reports_dir = Some(PathBuf::from(val));
                idx += 2;
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown release-scan flag `{other}`\n\n{USAGE}"));
            }
            positional => {
                if assets_dir.is_some() {
                    return Err(format!("unexpected extra argument `{positional}`\n\n{USAGE}"));
                }
                assets_dir = Some(PathBuf::from(positional));
                idx += 1;
            }
        }
    }

    if saw_require && saw_no_checksums {
        return Err(format!("cannot combine `--require-checksums` and `--no-checksums`\n\n{USAGE}"));
    }
    if expect_beta_set.is_some() && saw_no_checksums {
        return Err(format!("`--expect-beta-set` requires `{CHECKSUMS_FILE}` verification (cannot combine with `--no-checksums`)\n\n{USAGE}"));
    }
    let Some(assets_dir) = assets_dir else {
        return Err(USAGE.to_owned());
    };
    let checksum_mode = if saw_no_checksums { ChecksumMode::Disabled } else { ChecksumMode::Required };
    Ok(ScanOptions { assets_dir, checksum_mode, report_path, expect_beta_set, cross_os_reports_dir })
}

fn take_flag_value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    let Some(val) = args.get(idx + 1) else {
        return Err(format!("missing value after `{flag}`\n\n{USAGE}"));
    };
    Ok(val.as_str())
}

pub(crate) fn normalize_release_version(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let ver = trimmed.strip_prefix('v').unwrap_or(trimmed);
    semver::Version::parse(ver).map_err(|err| format!("invalid release version `{raw}` for `--expect-beta-set`: {err}"))?;
    Ok(ver.to_owned())
}

#[must_use]
pub(crate) fn expected_beta_asset_names(version: &str) -> Vec<String> {
    vec![
        format!("curvebook-{version}-macos-universal.dmg"),
        format!("curvebook-{version}-windows-x64.msi"),
        format!("curvebook-{version}-windows-arm64.msi"),
        format!("curvebook-{version}-linux-x86_64.AppImage"),
        format!("curvebook-{version}-linux-aarch64.AppImage"),
        format!("curvebook-{version}-freebsd-x86_64.tar.gz"),
        String::from(LICENSES_HTML_FILE),
        String::from(CHECKSUMS_FILE),
    ]
}

fn verify_expected_beta_set(assets_dir: &Path, version: &str, discovered: &[DiscoveredAsset]) -> Result<(), String> {
    let expected: BTreeSet<String> = expected_beta_asset_names(version).into_iter().collect();
    let actual: BTreeSet<String> = discovered.iter().map(|d| d.name.clone()).collect();
    let missing: Vec<&str> = expected.difference(&actual).map(String::as_str).collect();
    let extra: Vec<&str> = actual.difference(&expected).map(String::as_str).collect();
    if missing.is_empty() && extra.is_empty() {
        return Ok(());
    }
    let mut parts = Vec::new();
    if !missing.is_empty() {
        parts.push(format!("missing expected file(s): {}", missing.join(", ")));
    }
    if !extra.is_empty() {
        parts.push(format!("unexpected extra file(s): {}", extra.join(", ")));
    }
    Err(format!("Beta release asset set check failed for `{version}` in `{}` ({})", assets_dir.display(), parts.join("; ")))
}

#[derive(Debug, Clone)]
struct CrossOsVerifiedRecord {
    host_os: String,
    name: String,
    kind: String,
    size_bytes: u64,
    sha256: String,
    unpacked_files: usize,
    nested_archive_entries: usize,
    checked_assets: usize,
}

#[derive(Debug, Default)]
struct CrossOsReportCatalog {
    records: Vec<CrossOsVerifiedRecord>,
}

impl CrossOsReportCatalog {
    fn load(dir: &Path) -> Result<Self, String> {
        let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("stat `--cross-os-reports` dir `{}`: {e}", dir.display()))?;
        if !meta.is_dir() {
            return Err(format!("`--cross-os-reports` path `{}` is not a directory", dir.display()));
        }
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(dir).map_err(|e| format!("read_dir `{}`: {e}", dir.display()))? {
            let path = entry.map_err(|e| format!("entry in `{}`: {e}", dir.display()))?.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("json")) {
                paths.push(path);
            }
        }
        paths.sort();
        if paths.is_empty() {
            return Err(format!("`--cross-os-reports` directory `{}` contains no `.json` reports", dir.display()));
        }
        let mut records = Vec::new();
        for path in paths {
            parse_cross_os_report_file(&path, &mut records)?;
        }
        Ok(Self { records })
    }

    fn match_native_asset(&self, expected_host_os: &str, item: &DiscoveredAsset) -> Option<ScannedAssetRecord> {
        self.records
            .iter()
            .find(|rec| {
                rec.host_os == expected_host_os
                    && rec.name == item.name
                    && rec.kind == item.kind.label()
                    && rec.size_bytes == item.size_bytes
                    && rec.sha256 == item.sha256
                    && rec.unpacked_files > 0
            })
            .map(|rec| ScannedAssetRecord {
                name: item.name.clone(),
                kind: item.kind.label(),
                size_bytes: item.size_bytes,
                sha256: item.sha256.clone(),
                unpacked_files: rec.unpacked_files,
                nested_archive_entries: rec.nested_archive_entries,
                checked_assets: rec.checked_assets,
            })
    }
}

fn parse_cross_os_report_file(path: &Path, out: &mut Vec<CrossOsVerifiedRecord>) -> Result<(), String> {
    let raw_bytes = read_non_symlink_regular_bytes(path, MAX_REPORT_JSON_BYTES, "cross-OS report")?;
    let raw = String::from_utf8(raw_bytes).map_err(|_| format!("cross-OS report `{}` is not valid UTF-8", path.display()))?;
    let val: serde_json::Value = serde_json::from_str(&raw).map_err(|e| format!("parse report `{}`: {e}", path.display()))?;
    if val.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(format!("cross-OS report `{}` is not marked `ok: true`", path.display()));
    }
    let Some(host_os) = val.get("host_os").and_then(serde_json::Value::as_str).filter(|s| !s.is_empty()) else {
        return Err(format!("cross-OS report `{}` missing `host_os`", path.display()));
    };
    let Some(assets) = val.get("assets").and_then(serde_json::Value::as_array) else {
        return Err(format!("cross-OS report `{}` missing `assets` array", path.display()));
    };
    for entry in assets {
        let Some(name) = entry.get("name").and_then(serde_json::Value::as_str) else {
            return Err(format!("cross-OS report `{}` has asset entry missing `name`", path.display()));
        };
        let Some(kind) = entry.get("kind").and_then(serde_json::Value::as_str) else {
            return Err(format!("cross-OS report `{}` asset `{name}` missing `kind`", path.display()));
        };
        let Some(size_bytes) = entry.get("size_bytes").and_then(serde_json::Value::as_u64) else {
            return Err(format!("cross-OS report `{}` asset `{name}` missing `size_bytes`", path.display()));
        };
        let Some(sha256) = entry.get("sha256").and_then(serde_json::Value::as_str) else {
            return Err(format!("cross-OS report `{}` asset `{name}` missing `sha256`", path.display()));
        };
        let Some(unpacked_files) = entry.get("unpacked_files").and_then(serde_json::Value::as_u64) else {
            return Err(format!("cross-OS report `{}` asset `{name}` missing `unpacked_files`", path.display()));
        };
        let nested_archive_entries = entry.get("nested_archive_entries").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let checked_assets = entry.get("checked_assets").and_then(serde_json::Value::as_u64).unwrap_or(0);
        out.push(CrossOsVerifiedRecord {
            host_os: host_os.to_owned(),
            name: name.to_owned(),
            kind: kind.to_owned(),
            size_bytes,
            sha256: sha256.to_ascii_lowercase(),
            unpacked_files: usize::try_from(unpacked_files).unwrap_or(usize::MAX),
            nested_archive_entries: usize::try_from(nested_archive_entries).unwrap_or(usize::MAX),
            checked_assets: usize::try_from(checked_assets).unwrap_or(usize::MAX),
        });
    }
    Ok(())
}

struct DiscoveredAsset {
    name: String,
    path: PathBuf,
    kind: AssetKind,
    size_bytes: u64,
    sha256: String,
}

pub(crate) fn scan_release_dir_with_tools(root: &Path, options: &ScanOptions, tools: &ToolConfig) -> Result<ReleaseScanReport, String> {
    let meta = std::fs::symlink_metadata(&options.assets_dir).map_err(|e| format!("stat `{}`: {e}", options.assets_dir.display()))?;
    if !meta.is_dir() {
        return Err(format!("release-scan target `{}` is not a directory", options.assets_dir.display()));
    }

    let discovered = discover_release_assets(&options.assets_dir)?;
    if let Some(beta_ver) = &options.expect_beta_set {
        verify_expected_beta_set(&options.assets_dir, beta_ver, &discovered)?;
    }
    let checksum_verified = options.checksum_mode == ChecksumMode::Required;
    if checksum_verified {
        verify_checksum_manifest(&options.assets_dir, &discovered)?;
    }
    let cross_os_reports = options.cross_os_reports_dir.as_deref().map(CrossOsReportCatalog::load).transpose()?;

    let asset_catalog = crate::assets::load_repo_asset_catalog(root)?;
    let scratch = ScratchDirGuard::new("batch")?;
    let mut records = Vec::with_capacity(discovered.len());
    let mut violations = Vec::new();

    for (idx, item) in discovered.iter().enumerate() {
        for hit in crate::brands::scan_dist_label(&item.name, &item.name) {
            violations.push(format!("{}:{}: [brands] filename mentions `{}`", hit.path, hit.line, hit.brand));
        }
        let item_scratch = scratch.path.join(format!("item-{idx}"));
        std::fs::create_dir_all(&item_scratch).map_err(|e| format!("create staging dir for `{}`: {e}", item.name))?;
        let outcome = scan_single_asset(&asset_catalog, tools, cross_os_reports.as_ref(), item, &item_scratch, &mut violations)?;
        records.push(outcome);
    }

    if !violations.is_empty() {
        return Err(format!("release-scan found {} violation(s):\n  {}", violations.len(), violations.join("\n  ")));
    }

    let total_unpacked_files = records.iter().map(|r| r.unpacked_files).sum();
    let total_nested_archive_entries = records.iter().map(|r| r.nested_archive_entries).sum();
    let total_checked_assets = records.iter().map(|r| r.checked_assets).sum();
    Ok(ReleaseScanReport {
        ok: true,
        host_os: std::env::consts::OS.to_owned(),
        checksum_verified,
        asset_count: records.len(),
        total_unpacked_files,
        total_nested_archive_entries,
        total_checked_assets,
        assets: records,
    })
}

fn discover_release_assets(assets_dir: &Path) -> Result<Vec<DiscoveredAsset>, String> {
    let entries = std::fs::read_dir(assets_dir).map_err(|e| format!("read_dir `{}`: {e}", assets_dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| format!("entry in `{}`: {e}", assets_dir.display()))?.path());
    }
    paths.sort();

    if paths.is_empty() {
        return Err(format!("release assets directory `{}` is empty", assets_dir.display()));
    }

    let mut out = Vec::with_capacity(paths.len());
    let mut non_checksum_count = 0usize;
    for path in paths {
        let name =
            path.file_name().and_then(|n| n.to_str()).ok_or_else(|| format!("non-UTF-8 asset filename under `{}`", assets_dir.display()))?.to_owned();
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat `{}`: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!("`{name}`: symlinks are forbidden in release assets directory"));
        }
        if meta.is_dir() {
            return Err(format!("`{name}`: unexpected subdirectory in flat release assets directory"));
        }
        if !meta.is_file() {
            return Err(format!("`{name}`: non-regular file is forbidden in release assets directory"));
        }
        let size_bytes = meta.len();
        if size_bytes == 0 {
            return Err(format!("`{name}`: release asset is empty (0 bytes)"));
        }
        let kind = classify_asset_name(&name)?;
        if kind != AssetKind::Checksums {
            non_checksum_count += 1;
        }
        let sha256 = sha256_file_hex(&path)?;
        out.push(DiscoveredAsset { name, path, kind, size_bytes, sha256 });
    }

    if non_checksum_count == 0 {
        return Err(format!("release assets directory `{}` contains no release packages beyond `{CHECKSUMS_FILE}`", assets_dir.display()));
    }
    Ok(out)
}

fn classify_asset_name(name: &str) -> Result<AssetKind, String> {
    if name == CHECKSUMS_FILE {
        return Ok(AssetKind::Checksums);
    }
    if name == LICENSES_HTML_FILE {
        return Ok(AssetKind::HtmlCompliance);
    }
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".zsync") {
        return Ok(AssetKind::Zsync);
    }
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        return Ok(AssetKind::TarGz);
    }
    if lower.ends_with(".zip") {
        return Ok(AssetKind::Zip);
    }
    if lower.ends_with(".deb") {
        return Ok(AssetKind::Deb);
    }
    if lower.ends_with(".rpm") {
        return Ok(AssetKind::Rpm);
    }
    if lower.ends_with(".appimage") {
        return Ok(AssetKind::AppImage);
    }
    if lower.ends_with(".flatpak") {
        return Ok(AssetKind::Flatpak);
    }
    if lower.ends_with(".dmg") {
        return Ok(AssetKind::Dmg);
    }
    if lower.ends_with(".msi") {
        return Ok(AssetKind::Msi);
    }
    Err(format!("unsupported or unknown release asset format `{name}` (failing closed)"))
}

fn verify_checksum_manifest(assets_dir: &Path, discovered: &[DiscoveredAsset]) -> Result<(), String> {
    let manifest_path = assets_dir.join(CHECKSUMS_FILE);
    let raw_bytes = read_non_symlink_regular_bytes(&manifest_path, MAX_CHECKSUMS_BYTES, CHECKSUMS_FILE)
        .map_err(|e| format!("missing or unreadable `{CHECKSUMS_FILE}` in `{}`: {e}", assets_dir.display()))?;
    let raw = String::from_utf8(raw_bytes).map_err(|_| format!("`{CHECKSUMS_FILE}` in `{}` is not valid UTF-8", assets_dir.display()))?;
    let expected_by_name = parse_sha256sums(&raw)?;

    let mut actual_siblings: BTreeMap<&str, &str> = BTreeMap::new();
    for item in discovered {
        if item.kind != AssetKind::Checksums {
            actual_siblings.insert(item.name.as_str(), item.sha256.as_str());
        }
    }

    let mut errors = Vec::new();
    for (&name, &actual_hex) in &actual_siblings {
        match expected_by_name.get(name) {
            None => errors.push(format!("`{name}` is present on disk but missing from `{CHECKSUMS_FILE}`")),
            Some(expected_hex) if expected_hex != actual_hex => {
                errors.push(format!("`{name}` SHA-256 mismatch: `{CHECKSUMS_FILE}` has {expected_hex}, computed {actual_hex}"));
            }
            Some(_) => {}
        }
    }
    for name in expected_by_name.keys() {
        if !actual_siblings.contains_key(name.as_str()) {
            errors.push(format!("`{CHECKSUMS_FILE}` lists `{name}`, which does not exist in `{}`", assets_dir.display()));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("`{CHECKSUMS_FILE}` verification failed ({} error(s)):\n  {}", errors.len(), errors.join("\n  ")))
    }
}

fn parse_sha256sums(raw: &str) -> Result<BTreeMap<String, String>, String> {
    let mut entries = BTreeMap::new();
    for (idx, line) in raw.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim_end_matches('\r');
        if trimmed.trim().is_empty() {
            continue;
        }
        let Some((digest_raw, rest)) = parse_sha256_line(trimmed) else {
            return Err(format!("{CHECKSUMS_FILE}:{line_no}: invalid sha256sum line `{trimmed}`"));
        };
        if rest.is_empty() || rest == "." || rest == ".." || rest.contains('/') || rest.contains('\\') || rest.contains('\0') {
            return Err(format!("{CHECKSUMS_FILE}:{line_no}: unsafe or nested asset filename `{rest}`"));
        }
        if rest == CHECKSUMS_FILE {
            return Err(format!("{CHECKSUMS_FILE}:{line_no}: `{CHECKSUMS_FILE}` must not list itself"));
        }
        if entries.insert(rest.to_owned(), digest_raw.to_ascii_lowercase()).is_some() {
            return Err(format!("{CHECKSUMS_FILE}:{line_no}: duplicate entry for `{rest}`"));
        }
    }
    if entries.is_empty() {
        return Err(format!("{CHECKSUMS_FILE} contains zero checksum entries"));
    }
    Ok(entries)
}

fn parse_sha256_line(line: &str) -> Option<(&str, &str)> {
    if line.len() < 66 {
        return None;
    }
    let digest = line.get(..64)?;
    if !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let rest = line.get(64..)?;
    let filename = rest.strip_prefix("  ").or_else(|| rest.strip_prefix(" *"))?;
    Some((digest, filename))
}

fn scan_single_asset(
    asset_catalog: &crate::assets::RepoAssetCatalog,
    tools: &ToolConfig,
    cross_os_reports: Option<&CrossOsReportCatalog>,
    item: &DiscoveredAsset,
    item_scratch: &Path,
    violations: &mut Vec<String>,
) -> Result<ScannedAssetRecord, String> {
    match item.kind {
        AssetKind::Checksums | AssetKind::HtmlCompliance => {
            let bytes = read_non_symlink_regular_bytes(&item.path, MAX_COMPLIANCE_TEXT_BYTES, &item.name)?;
            let text = std::str::from_utf8(&bytes).map_err(|e| format!("`{}` is not valid UTF-8: {e}", item.name))?;
            collect_compliance_hits(&item.name, text, &bytes, violations);
            Ok(ScannedAssetRecord {
                name: item.name.clone(),
                kind: item.kind.label(),
                size_bytes: item.size_bytes,
                sha256: item.sha256.clone(),
                unpacked_files: 1,
                nested_archive_entries: 0,
                checked_assets: 0,
            })
        }
        AssetKind::Zsync => {
            let bytes = read_non_symlink_regular_bytes(&item.path, MAX_ZSYNC_BYTES, &item.name)?;
            let header = extract_zsync_header_utf8(&item.name, &bytes)?;
            collect_compliance_hits(&item.name, header, &bytes, violations);
            Ok(ScannedAssetRecord {
                name: item.name.clone(),
                kind: item.kind.label(),
                size_bytes: item.size_bytes,
                sha256: item.sha256.clone(),
                unpacked_files: 1,
                nested_archive_entries: 0,
                checked_assets: 0,
            })
        }
        AssetKind::Dmg if !tools.allow_dmg => {
            if let Some(reports) = cross_os_reports {
                return reports.match_native_asset("macos", item).ok_or_else(|| {
                    format!(
                        "`{}`: missing matching verified macOS native scan report (`host_os=macos`, exact size/sha256, `unpacked_files > 0`)",
                        item.name
                    )
                });
            }
            let stage_dir = item_scratch.join("unpacked");
            std::fs::create_dir_all(&stage_dir).map_err(|e| format!("create unpacked dir for `{}`: {e}", item.name))?;
            unpack_container_asset(tools, item, item_scratch, &stage_dir, violations)?;
            scan_unpacked_tree(asset_catalog, item, &stage_dir, violations)
        }
        AssetKind::Msi if !tools.allow_msi => {
            if let Some(reports) = cross_os_reports {
                return reports.match_native_asset("windows", item).ok_or_else(|| {
                    format!(
                        "`{}`: missing matching verified Windows native scan report (`host_os=windows`, exact size/sha256, `unpacked_files > 0`)",
                        item.name
                    )
                });
            }
            let stage_dir = item_scratch.join("unpacked");
            std::fs::create_dir_all(&stage_dir).map_err(|e| format!("create unpacked dir for `{}`: {e}", item.name))?;
            unpack_container_asset(tools, item, item_scratch, &stage_dir, violations)?;
            scan_unpacked_tree(asset_catalog, item, &stage_dir, violations)
        }
        _ => {
            let stage_dir = item_scratch.join("unpacked");
            std::fs::create_dir_all(&stage_dir).map_err(|e| format!("create unpacked dir for `{}`: {e}", item.name))?;
            unpack_container_asset(tools, item, item_scratch, &stage_dir, violations)?;
            scan_unpacked_tree(asset_catalog, item, &stage_dir, violations)
        }
    }
}

fn collect_compliance_hits(asset_name: &str, text: &str, raw_bytes: &[u8], violations: &mut Vec<String>) {
    for hit in crate::brands::scan_compliance_text(asset_name, text) {
        violations.push(format!("{}:{}: [brands] `{}`", hit.path, hit.line, hit.brand));
    }
    for hit in crate::cleanroom::scan_compliance_bytes(asset_name, raw_bytes) {
        violations.push(format!("{}:{}: [cleanroom] matched `{}`", hit.path, hit.line, hit.signature));
    }
}

fn extract_zsync_header_utf8<'a>(asset_name: &str, bytes: &'a [u8]) -> Result<&'a str, String> {
    if let Ok(all_utf8) = std::str::from_utf8(bytes) {
        if all_utf8.trim().is_empty() {
            return Err(format!("`{asset_name}`: zsync file is blank"));
        }
        return Ok(all_utf8);
    }
    let header_end = bytes
        .windows(2)
        .position(|w| w == b"\n\n")
        .or_else(|| bytes.windows(4).position(|w| w == b"\r\n\r\n"))
        .ok_or_else(|| format!("`{asset_name}`: non-UTF-8 `.zsync` file lacks standard header separator before block checksums"))?;
    let header = std::str::from_utf8(&bytes[..header_end]).map_err(|e| format!("`{asset_name}`: invalid UTF-8 in `.zsync` header: {e}"))?;
    if !header.starts_with("zsync:") {
        return Err(format!("`{asset_name}`: `.zsync` metadata header does not begin with `zsync:`"));
    }
    Ok(header)
}

fn unpack_container_asset(
    tools: &ToolConfig,
    item: &DiscoveredAsset,
    item_scratch: &Path,
    stage_dir: &Path,
    violations: &mut Vec<String>,
) -> Result<(), String> {
    match item.kind {
        AssetKind::Zip => unpack_zip(&item.name, &item.path, stage_dir),
        AssetKind::TarGz => unpack_tarball(tools, &item.name, &item.path, stage_dir, true),
        AssetKind::Deb => unpack_deb(tools, &item.name, &item.path, item_scratch, stage_dir),
        AssetKind::Rpm => unpack_rpm(tools, &item.name, &item.path, stage_dir),
        AssetKind::AppImage => unpack_appimage(tools, &item.name, &item.path, item_scratch, stage_dir, violations),
        AssetKind::Flatpak => unpack_flatpak(tools, &item.name, &item.path, item_scratch, stage_dir),
        AssetKind::Dmg => unpack_dmg(tools, &item.name, &item.path, item_scratch, stage_dir, violations),
        AssetKind::Msi => unpack_msi(tools, &item.name, &item.path, stage_dir),
        AssetKind::Checksums | AssetKind::HtmlCompliance | AssetKind::Zsync => Ok(()),
    }
}

fn scan_unpacked_tree(
    asset_catalog: &crate::assets::RepoAssetCatalog,
    item: &DiscoveredAsset,
    stage_dir: &Path,
    violations: &mut Vec<String>,
) -> Result<ScannedAssetRecord, String> {
    scan_tree_symlinks(&item.name, stage_dir, stage_dir, violations)?;
    let (brand_stats, brand_hits) =
        crate::brands::scan_dist_artefacts_with_stats(stage_dir).map_err(|e| format!("`{}` brands scan failed: {e}", item.name))?;
    if brand_stats.files == 0 {
        return Err(format!("`{}` unpacked to zero regular files (failing closed)", item.name));
    }
    let (_, _, cleanroom_hits) =
        crate::cleanroom::scan_dist_artefacts_with_stats(stage_dir).map_err(|e| format!("`{}` cleanroom scan failed: {e}", item.name))?;
    let (checked_assets, asset_violations) =
        crate::assets::scan_dist_artefacts(asset_catalog, stage_dir).map_err(|e| format!("`{}` assets scan failed: {e}", item.name))?;

    for hit in brand_hits {
        violations.push(format!("{}!{}:{}: [brands] `{}`", item.name, hit.path, hit.line, hit.brand));
    }
    for hit in cleanroom_hits {
        violations.push(format!("{}!{}:{}: [cleanroom] matched `{}`", item.name, hit.path, hit.line, hit.signature));
    }
    for v in asset_violations {
        violations.push(format!("{}!{v}", item.name));
    }

    Ok(ScannedAssetRecord {
        name: item.name.clone(),
        kind: item.kind.label(),
        size_bytes: item.size_bytes,
        sha256: item.sha256.clone(),
        unpacked_files: brand_stats.files,
        nested_archive_entries: brand_stats.archive_entries,
        checked_assets,
    })
}

fn scan_tree_symlinks(asset_name: &str, root: &Path, dir: &Path, violations: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| format!("dir entry under {}: {e}", dir.display()))?.path());
    }
    paths.sort();
    for path in paths {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            let rel = path.strip_prefix(root).map_err(|e| format!("strip prefix {}: {e}", path.display()))?.to_string_lossy().replace('\\', "/");
            let target = std::fs::read_link(&path).map_err(|e| format!("read_link `{asset_name}!{rel}`: {e}"))?;
            let target_str = target.to_string_lossy();
            let src = format!("{asset_name}!{rel}");
            for hit in crate::brands::scan_dist_label(&src, &rel) {
                violations.push(format!("{}:{}: [brands] `{}`", hit.path, hit.line, hit.brand));
            }
            for hit in crate::brands::scan_dist_label(&src, &target_str) {
                violations.push(format!("{}:{}: [brands] symlink target `{}`", hit.path, hit.line, hit.brand));
            }
        } else if meta.is_dir() {
            scan_tree_symlinks(asset_name, root, &path, violations)?;
        }
    }
    Ok(())
}

fn unpack_zip(asset_name: &str, path: &Path, stage_dir: &Path) -> Result<(), String> {
    let bytes = read_non_symlink_regular_bytes(path, MAX_CONTAINER_FILE_BYTES, asset_name)?;
    let zip = ZipArchive::from_bytes_with_limits(&bytes, RELEASE_ZIP_LIMITS).map_err(|e| format!("unpack `{asset_name}`: {e}"))?;
    if zip.is_empty() {
        return Err(format!("`{asset_name}`: ZIP archive contains zero files"));
    }
    for (entry_rel, payload) in zip.iter() {
        let safe_rel = validate_safe_archive_member_path(asset_name, entry_rel)?;
        let out_path = stage_dir.join(&safe_rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir for `{asset_name}!{entry_rel}`: {e}"))?;
        }
        write_atomic(&out_path, payload).map_err(|e| format!("write `{asset_name}!{entry_rel}`: {e}"))?;
    }
    Ok(())
}

fn unpack_tarball(tools: &ToolConfig, asset_name: &str, archive_path: &Path, stage_dir: &Path, require_gzip_magic: bool) -> Result<(), String> {
    if require_gzip_magic {
        let magic = read_prefix_bytes(archive_path, 2).map_err(|e| format!("read header `{asset_name}`: {e}"))?;
        if magic.as_slice() != [0x1f, 0x8b] {
            return Err(format!("`{asset_name}`: expected gzip header `1f 8b`"));
        }
    }
    let archive_str = archive_path.to_string_lossy();
    let stage_str = stage_dir.to_string_lossy();
    let list_flag = if require_gzip_magic { "-tvzf" } else { "-tvf" };
    let extract_flag = if require_gzip_magic { "-xzf" } else { "-xf" };

    let (chosen_tar, list_out) = run_tar_or_bsdtar(tools, &[list_flag, archive_str.as_ref()], asset_name, "list entries")?;
    let listing = String::from_utf8_lossy(&list_out);
    validate_verbose_archive_listing(asset_name, &listing, false)?;

    let out = build_cmd(tools, chosen_tar)
        .args([extract_flag, archive_str.as_ref(), "-C", stage_str.as_ref()])
        .run()
        .map_err(|e| format!("`{asset_name}`: `{chosen_tar} {extract_flag}` failed: {e}"))?;
    if !out.success {
        return Err(format!(
            "`{asset_name}`: `{chosen_tar} {extract_flag}` exited with {:?}: {}",
            out.status_code,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

fn run_tar_or_bsdtar<'a>(tools: &'a ToolConfig, args: &[&str], asset_name: &str, op: &str) -> Result<(&'a str, Vec<u8>), String> {
    let mut errors = Vec::new();
    for candidate in [tools.tar_bin.as_str(), tools.bsdtar_bin.as_str()] {
        match build_cmd(tools, candidate).args(args).run() {
            Ok(out) if out.success => return Ok((candidate, out.stdout)),
            Ok(out) => errors.push(format!("{candidate}: exit {:?} ({})", out.status_code, String::from_utf8_lossy(&out.stderr).trim())),
            Err(err) => errors.push(format!("{candidate}: {err}")),
        }
    }
    Err(format!("`{asset_name}`: cannot {op} via tar/bsdtar ({})", errors.join("; ")))
}

fn unpack_deb(tools: &ToolConfig, asset_name: &str, deb_path: &Path, item_scratch: &Path, stage_dir: &Path) -> Result<(), String> {
    let magic = read_prefix_bytes(deb_path, 8).map_err(|e| format!("read header `{asset_name}`: {e}"))?;
    if magic.as_slice() != b"!<arch>\n" {
        return Err(format!("`{asset_name}`: invalid `.deb` header (expected `!<arch>\\n`)"));
    }

    let ar_dir = item_scratch.join("deb-ar");
    std::fs::create_dir_all(&ar_dir).map_err(|e| format!("mkdir deb-ar for `{asset_name}`: {e}"))?;
    extract_deb_ar_members(tools, asset_name, deb_path, &ar_dir)?;

    let mut data_tars = Vec::new();
    let mut control_tars = Vec::new();
    for entry in std::fs::read_dir(&ar_dir).map_err(|e| format!("read deb-ar `{asset_name}`: {e}"))? {
        let path = entry.map_err(|e| format!("deb-ar entry `{asset_name}`: {e}"))?.path();
        let Some(fname) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_deb_tar_member(fname, "data.tar") {
            data_tars.push(path);
        } else if is_deb_tar_member(fname, "control.tar") {
            control_tars.push(path);
        }
    }
    data_tars.sort();
    control_tars.sort();

    let [data_tar] = data_tars.as_slice() else {
        return Err(format!("`{asset_name}`: expected exactly 1 `data.tar*` member in `.deb`, found {}", data_tars.len()));
    };
    unpack_tarball(tools, &format!("{asset_name}!data"), data_tar, stage_dir, false)?;

    if let [ctrl_tar] = control_tars.as_slice() {
        let ctrl_stage = stage_dir.join("_deb_control");
        std::fs::create_dir_all(&ctrl_stage).map_err(|e| format!("mkdir _deb_control for `{asset_name}`: {e}"))?;
        unpack_tarball(tools, &format!("{asset_name}!control"), ctrl_tar, &ctrl_stage, false)?;
    }
    Ok(())
}

fn is_deb_tar_member(fname: &str, stem: &str) -> bool {
    fname == stem || fname.starts_with(&format!("{stem}."))
}

fn extract_deb_ar_members(tools: &ToolConfig, asset_name: &str, deb_path: &Path, ar_dir: &Path) -> Result<(), String> {
    let deb_str = deb_path.to_string_lossy();
    let mut errors = Vec::new();

    match build_cmd(tools, &tools.ar_bin).args(["-t", deb_str.as_ref()]).run() {
        Ok(list_out) if list_out.success => {
            validate_deb_ar_listing(asset_name, &String::from_utf8_lossy(&list_out.stdout))?;
            match build_cmd(tools, &tools.ar_bin).current_dir(ar_dir).args(["-x", deb_str.as_ref()]).run() {
                Ok(out) if out.success && dir_has_deb_data_tar(ar_dir) => return Ok(()),
                Ok(out) => errors.push(format!("{}: exit {:?} ({})", tools.ar_bin, out.status_code, String::from_utf8_lossy(&out.stderr).trim())),
                Err(err) => errors.push(format!("{}: {err}", tools.ar_bin)),
            }
        }
        Ok(list_out) => {
            errors.push(format!("{} -t: exit {:?} ({})", tools.ar_bin, list_out.status_code, String::from_utf8_lossy(&list_out.stderr).trim()))
        }
        Err(err) => errors.push(format!("{} -t: {err}", tools.ar_bin)),
    }

    // Fallback for GNU/SVR4 `.deb` archives whose 16-byte member names carry trailing `/`
    // (`data.tar.gz/    `), which macOS BSD `/usr/bin/ar -x` rejects while `bsdtar -xf` unpacks cleanly.
    let ar_dir_str = ar_dir.to_string_lossy();
    match build_cmd(tools, &tools.bsdtar_bin).args(["-tvf", deb_str.as_ref()]).run() {
        Ok(list_out) if list_out.success => {
            validate_verbose_archive_listing(asset_name, &String::from_utf8_lossy(&list_out.stdout), true)?;
            match build_cmd(tools, &tools.bsdtar_bin).args(["-xf", deb_str.as_ref(), "-C", ar_dir_str.as_ref()]).run() {
                Ok(out) if out.success && dir_has_deb_data_tar(ar_dir) => Ok(()),
                Ok(out) => {
                    errors.push(format!("{}: exit {:?} ({})", tools.bsdtar_bin, out.status_code, String::from_utf8_lossy(&out.stderr).trim()));
                    Err(format!("`{asset_name}`: failed to unpack `.deb` `ar` members ({})", errors.join("; ")))
                }
                Err(err) => {
                    errors.push(format!("{}: {err}", tools.bsdtar_bin));
                    Err(format!("`{asset_name}`: failed to unpack `.deb` `ar` members ({})", errors.join("; ")))
                }
            }
        }
        Ok(list_out) => {
            errors.push(format!("{} -tvf: exit {:?} ({})", tools.bsdtar_bin, list_out.status_code, String::from_utf8_lossy(&list_out.stderr).trim()));
            Err(format!("`{asset_name}`: failed to inspect `.deb` `ar` members ({})", errors.join("; ")))
        }
        Err(err) => {
            errors.push(format!("{} -tvf: {err}", tools.bsdtar_bin));
            Err(format!("`{asset_name}`: failed to inspect `.deb` `ar` members ({})", errors.join("; ")))
        }
    }
}

fn dir_has_deb_data_tar(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_str().is_some_and(|n| is_deb_tar_member(n, "data.tar")) && e.path().metadata().is_ok_and(|m| m.len() > 0))
}

fn unpack_rpm(tools: &ToolConfig, asset_name: &str, rpm_path: &Path, stage_dir: &Path) -> Result<(), String> {
    let magic = read_prefix_bytes(rpm_path, 4).map_err(|e| format!("read header `{asset_name}`: {e}"))?;
    if magic.as_slice() != [0xed, 0xab, 0xee, 0xdb] {
        return Err(format!("`{asset_name}`: invalid `.rpm` lead magic"));
    }

    let rpm_str = rpm_path.to_string_lossy();
    let stage_str = stage_dir.to_string_lossy();
    let mut errors = Vec::new();

    match build_cmd(tools, &tools.bsdtar_bin).args(["-tvf", rpm_str.as_ref()]).run() {
        Ok(list_out) if list_out.success => {
            validate_verbose_archive_listing(asset_name, &String::from_utf8_lossy(&list_out.stdout), false)?;
            let ext_out = build_cmd(tools, &tools.bsdtar_bin)
                .args(["-xf", rpm_str.as_ref(), "-C", stage_str.as_ref()])
                .run()
                .map_err(|e| format!("`{asset_name}`: bsdtar extract failed: {e}"))?;
            if ext_out.success {
                return Ok(());
            }
            errors.push(format!("bsdtar -xf: exit {:?} ({})", ext_out.status_code, String::from_utf8_lossy(&ext_out.stderr).trim()));
        }
        Ok(list_out) => errors.push(format!("bsdtar -tvf: exit {:?} ({})", list_out.status_code, String::from_utf8_lossy(&list_out.stderr).trim())),
        Err(err) => errors.push(format!("bsdtar: {err}")),
    }

    let cpio_stream = match build_cmd(tools, &tools.rpm2cpio_bin).max_output_bytes(CPIO_STREAM_CAP_BYTES).arg(rpm_str.as_ref()).run() {
        Ok(out) if out.success && !out.stdout.is_empty() => out.stdout,
        Ok(out) => {
            errors.push(format!("rpm2cpio: exit {:?} ({})", out.status_code, String::from_utf8_lossy(&out.stderr).trim()));
            return Err(format!("`{asset_name}`: cannot unpack `.rpm` ({})", errors.join("; ")));
        }
        Err(err) => {
            errors.push(format!("rpm2cpio: {err}"));
            return Err(format!("`{asset_name}`: cannot unpack `.rpm` ({})", errors.join("; ")));
        }
    };

    let list_cpio = build_cmd(tools, &tools.cpio_bin)
        .arg("-itv")
        .stdin_bytes(cpio_stream.clone())
        .run()
        .map_err(|e| format!("`{asset_name}`: cpio -itv failed: {e}"))?;
    if !list_cpio.success {
        return Err(format!("`{asset_name}`: cpio -itv failed: {}", String::from_utf8_lossy(&list_cpio.stderr).trim()));
    }
    validate_verbose_archive_listing(asset_name, &String::from_utf8_lossy(&list_cpio.stdout), false)?;

    let extract_cpio = build_cmd(tools, &tools.cpio_bin)
        .current_dir(stage_dir)
        .arg("-idm")
        .stdin_bytes(cpio_stream)
        .run()
        .map_err(|e| format!("`{asset_name}`: cpio -idm failed: {e}"))?;
    if !extract_cpio.success {
        return Err(format!("`{asset_name}`: cpio -idm failed: {}", String::from_utf8_lossy(&extract_cpio.stderr).trim()));
    }
    Ok(())
}

/// Locate the byte offset of an embedded SquashFS 4.0 little-endian superblock (`"hsqs"`).
#[must_use]
pub(crate) fn find_squashfs_offset(bytes: &[u8]) -> Option<usize> {
    const SQFS_MAGIC: &[u8; 4] = b"hsqs";
    const SUPERBLOCK_MIN_BYTES: usize = 96;
    let mut search = 0usize;
    while let Some(rel) = bytes.get(search..).and_then(|s| s.windows(4).position(|w| w == SQFS_MAGIC)) {
        let off = search + rel;
        if let Some(sb) = bytes.get(off..off + SUPERBLOCK_MIN_BYTES) {
            let block_size = u32::from_le_bytes([sb[12], sb[13], sb[14], sb[15]]);
            let comp_id = u16::from_le_bytes([sb[20], sb[21]]);
            let block_log = u16::from_le_bytes([sb[22], sb[23]]);
            let s_major = u16::from_le_bytes([sb[28], sb[29]]);
            let s_minor = u16::from_le_bytes([sb[30], sb[31]]);
            let log_matches = (12..=20).contains(&block_log) && (1u32 << block_log) == block_size;
            if s_major == 4 && s_minor == 0 && log_matches && (1..=6).contains(&comp_id) {
                return Some(off);
            }
        }
        search = off + 1;
    }
    None
}

fn unpack_appimage(
    tools: &ToolConfig,
    asset_name: &str,
    appimage_path: &Path,
    item_scratch: &Path,
    stage_dir: &Path,
    violations: &mut Vec<String>,
) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(appimage_path).map_err(|e| format!("stat `{asset_name}`: {e}"))?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(format!("`{asset_name}`: must be a regular non-symlink file"));
    }
    let scan_cap = usize::try_from(meta.len()).unwrap_or(MAX_APPIMAGE_RUNTIME_SCAN_BYTES).min(MAX_APPIMAGE_RUNTIME_SCAN_BYTES);
    let header_bytes = read_prefix_bytes(appimage_path, scan_cap).map_err(|e| format!("read `{asset_name}` header: {e}"))?;
    let sqfs_offset = find_squashfs_offset(&header_bytes);
    if let Some(off) = sqfs_offset
        && off > 0
    {
        for hit in crate::cleanroom::scan_compliance_bytes(&format!("{asset_name}#runtime"), &header_bytes[..off]) {
            violations.push(format!("{}:{}: [cleanroom] matched `{}`", hit.path, hit.line, hit.signature));
        }
    }

    let mut errors = Vec::new();
    if let Some(off) = sqfs_offset {
        let dest = stage_dir.join("squashfs-root");
        let dest_str = dest.to_string_lossy();
        let app_str = appimage_path.to_string_lossy();
        let off_str = off.to_string();
        let mut cmd = build_cmd(tools, &tools.unsquashfs_bin);
        cmd = cmd.arg("-no-progress");
        if off > 0 {
            cmd = cmd.args(["-o", off_str.as_str()]);
        }
        cmd = cmd.args(["-d", dest_str.as_ref(), app_str.as_ref()]);
        match cmd.run() {
            Ok(out) if out.success => return Ok(()),
            Ok(out) => errors.push(format!("unsquashfs: exit {:?} ({})", out.status_code, String::from_utf8_lossy(&out.stderr).trim())),
            Err(err) => errors.push(format!("unsquashfs: {err}")),
        }
    } else {
        errors.push(String::from("no embedded SquashFS 4.0 (`hsqs`) superblock found"));
    }

    if tools.allow_appimage_exec && header_bytes.starts_with(b"\x7fELF") {
        let bytes = read_non_symlink_regular_bytes(appimage_path, MAX_CONTAINER_FILE_BYTES, asset_name)?;
        let exec_work = item_scratch.join("appimage-exec");
        std::fs::create_dir_all(&exec_work).map_err(|e| format!("mkdir appimage-exec for `{asset_name}`: {e}"))?;
        let runner_copy = exec_work.join("runner.AppImage");
        write_atomic(&runner_copy, &bytes).map_err(|e| format!("stage `{asset_name}` for --appimage-extract: {e}"))?;
        set_owner_executable(&runner_copy)?;
        let runner_str = runner_copy.to_string_lossy();
        match build_cmd(tools, runner_str.as_ref()).current_dir(&exec_work).env("APPIMAGE_EXTRACT_AND_RUN", "1").arg("--appimage-extract").run() {
            Ok(out) if out.success && exec_work.join("squashfs-root").is_dir() => {
                std::fs::rename(exec_work.join("squashfs-root"), stage_dir.join("squashfs-root"))
                    .map_err(|e| format!("move extracted squashfs-root for `{asset_name}`: {e}"))?;
                return Ok(());
            }
            Ok(out) => errors.push(format!("--appimage-extract: exit {:?} ({})", out.status_code, String::from_utf8_lossy(&out.stderr).trim())),
            Err(err) => errors.push(format!("--appimage-extract: {err}")),
        }
    } else if !tools.allow_appimage_exec {
        errors.push(String::from("`--appimage-extract` execution is disabled (never execute untrusted AppImages); install `unsquashfs`"));
    }

    Err(format!("`{asset_name}`: cannot unpack `.AppImage` ({})", errors.join("; ")))
}

#[cfg(unix)]
fn set_owner_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).map_err(|e| format!("stat {}: {e}", path.display()))?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| format!("chmod {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn unpack_flatpak(tools: &ToolConfig, asset_name: &str, bundle_path: &Path, item_scratch: &Path, stage_dir: &Path) -> Result<(), String> {
    let repo_dir = item_scratch.join("ostree-repo");
    std::fs::create_dir_all(&repo_dir).map_err(|e| format!("mkdir ostree-repo for `{asset_name}`: {e}"))?;
    let repo_arg = format!("--repo={}", repo_dir.to_string_lossy());
    let repo_str = repo_dir.to_string_lossy();
    let bundle_str = bundle_path.to_string_lossy();

    run_checked_tool(
        tools,
        &tools.ostree_bin,
        &[repo_arg.as_str(), "init", "--mode=archive"],
        asset_name,
        "initialize temporary OSTree repository (install `ostree` and `flatpak`)",
    )?;
    run_checked_tool(
        tools,
        &tools.flatpak_bin,
        &["build-import-bundle", "--no-update-summary", repo_str.as_ref(), bundle_str.as_ref()],
        asset_name,
        "import `.flatpak` bundle (install `flatpak`)",
    )?;
    let refs_stdout = run_checked_tool(tools, &tools.ostree_bin, &[repo_arg.as_str(), "refs"], asset_name, "list imported OSTree refs")?;
    let refs_text = String::from_utf8_lossy(&refs_stdout);
    let refs: Vec<&str> = refs_text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if refs.is_empty() {
        return Err(format!("`{asset_name}`: `flatpak build-import-bundle` imported zero OSTree refs"));
    }

    for (idx, ostree_ref) in refs.iter().enumerate() {
        let checkout_dest = stage_dir.join(format!("ref-{idx}"));
        let dest_str = checkout_dest.to_string_lossy();
        run_checked_tool(
            tools,
            &tools.ostree_bin,
            &[repo_arg.as_str(), "checkout", "--user-mode", ostree_ref, dest_str.as_ref()],
            asset_name,
            "check out imported Flatpak tree",
        )?;
    }
    Ok(())
}

fn unpack_dmg(
    tools: &ToolConfig,
    asset_name: &str,
    dmg_path: &Path,
    item_scratch: &Path,
    stage_dir: &Path,
    violations: &mut Vec<String>,
) -> Result<(), String> {
    if !tools.allow_dmg {
        return Err(format!("`{asset_name}`: cannot unpack macOS `.dmg` disk image on non-macOS host (run `release-scan` on macOS)"));
    }
    let mountpoint = item_scratch.join("dmg-mnt");
    std::fs::create_dir_all(&mountpoint).map_err(|e| format!("mkdir dmg-mnt for `{asset_name}`: {e}"))?;
    let mnt_str = mountpoint.to_string_lossy();
    let dmg_str = dmg_path.to_string_lossy();

    run_checked_tool(
        tools,
        &tools.hdiutil_bin,
        &["attach", "-readonly", "-nobrowse", "-noautoopen", "-mountpoint", mnt_str.as_ref(), dmg_str.as_ref()],
        asset_name,
        "attach `.dmg` read-only via hdiutil",
    )?;
    let guard = DmgMountGuard::new(tools, mountpoint.clone());
    copy_mounted_tree_no_symlinks(asset_name, &mountpoint, &mountpoint, stage_dir, violations)?;
    guard.detach_checked()
}

fn copy_mounted_tree_no_symlinks(
    asset_name: &str,
    mount_root: &Path,
    current_dir: &Path,
    dest_root: &Path,
    violations: &mut Vec<String>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(current_dir).map_err(|e| format!("read mounted `{asset_name}` dir {}: {e}", current_dir.display()))?;
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| format!("mounted `{asset_name}` entry: {e}"))?.path());
    }
    paths.sort();

    for path in paths {
        let rel = path.strip_prefix(mount_root).map_err(|e| format!("strip mount prefix {}: {e}", path.display()))?;
        let rel_norm = rel.to_string_lossy().replace('\\', "/");
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat mounted `{asset_name}!{rel_norm}`: {e}"))?;
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&path).map_err(|e| format!("read_link `{asset_name}!{rel_norm}`: {e}"))?;
            let target_str = target.to_string_lossy();
            let src = format!("{asset_name}!{rel_norm}");
            for hit in crate::brands::scan_dist_label(&src, &rel_norm) {
                violations.push(format!("{}:{}: [brands] `{}`", hit.path, hit.line, hit.brand));
            }
            for hit in crate::brands::scan_dist_label(&src, &target_str) {
                violations.push(format!("{}:{}: [brands] symlink target `{}`", hit.path, hit.line, hit.brand));
            }
        } else if meta.is_dir() {
            std::fs::create_dir_all(dest_root.join(rel)).map_err(|e| format!("mkdir `{asset_name}!{rel_norm}`: {e}"))?;
            copy_mounted_tree_no_symlinks(asset_name, mount_root, &path, dest_root, violations)?;
        } else if meta.is_file() {
            let dst = dest_root.join(rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("mkdir parent `{asset_name}!{rel_norm}`: {e}"))?;
            }
            let bytes = read_non_symlink_regular_bytes(&path, MAX_MOUNTED_FILE_BYTES, &format!("{asset_name}!{rel_norm}"))?;
            write_atomic(&dst, &bytes).map_err(|e| format!("stage mounted `{asset_name}!{rel_norm}`: {e}"))?;
        }
    }
    Ok(())
}

/// Strip a leading Windows verbatim path prefix (`\\?\` or `\\?\UNC\`) when present.
///
/// Windows `msiexec /a` rejects both relative paths (exit code `1619`) and `\\?\`
/// verbatim paths returned by `std::fs::canonicalize`.
fn strip_windows_verbatim_prefix(path: &Path) -> PathBuf {
    let Some(raw) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(stripped) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

/// Resolve `path` to an absolute path using [`std::path::absolute`] and strip any
/// leading Windows `\\?\` verbatim prefix before handing paths to external tools.
pub(crate) fn absolute_tool_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err(String::from("cannot resolve empty tool path"));
    }
    let abs = std::path::absolute(path).map_err(|e| format!("resolve absolute path `{}`: {e}", path.display()))?;
    Ok(strip_windows_verbatim_prefix(&abs))
}

fn unpack_msi(tools: &ToolConfig, asset_name: &str, msi_path: &Path, stage_dir: &Path) -> Result<(), String> {
    if !tools.allow_msi {
        return Err(format!("`{asset_name}`: cannot unpack Windows `.msi` installer on non-Windows host (run `release-scan` on Windows)"));
    }
    let abs_msi = absolute_tool_path(msi_path)?;
    let abs_stage = absolute_tool_path(stage_dir)?;
    let msi_str = abs_msi.to_string_lossy();
    let target_arg = format!("TARGETDIR={}", abs_stage.to_string_lossy());
    run_checked_tool(
        tools,
        &tools.msiexec_bin,
        &["/a", msi_str.as_ref(), "/qn", target_arg.as_str()],
        asset_name,
        "extract `.msi` administratively via `msiexec /a`",
    )?;

    // `msiexec /a` places a stripped administrative copy of `<name>.msi` at the root of `TARGETDIR`
    // alongside the extracted directory hierarchy; remove that administrative stub before counting payload files.
    let admin_stub = abs_stage.join(asset_name);
    if admin_stub.is_file() {
        let _ = std::fs::remove_file(&admin_stub);
    }
    Ok(())
}

fn run_checked_tool(tools: &ToolConfig, program: &str, args: &[&str], asset_name: &str, purpose: &str) -> Result<Vec<u8>, String> {
    match build_cmd(tools, program).args(args).run() {
        Ok(out) if out.success => Ok(out.stdout),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            let detail = if !stderr.is_empty() { stderr } else { stdout };
            Err(format!("`{asset_name}`: failed to {purpose} (`{program}` exited with {:?}: {detail})", out.status_code))
        }
        Err(PlatformError::ProgramNotFound { program: missing }) => {
            Err(format!("`{asset_name}`: required platform tool `{missing}` is unavailable to {purpose} (failing closed)"))
        }
        Err(err) => Err(format!("`{asset_name}`: failed to {purpose}: {err}")),
    }
}

fn build_cmd(tools: &ToolConfig, program: &str) -> ProcessCommand {
    let mut cmd = ProcessCommand::new(program)
        .env("LC_ALL", "C")
        .wall_timeout(SUBPROCESS_WALL_TIMEOUT)
        .idle_timeout(SUBPROCESS_IDLE_TIMEOUT)
        .max_output_bytes(SUBPROCESS_STREAM_CAP_BYTES);
    if let Some(path_val) = &tools.path_env {
        cmd = cmd.env("PATH", path_val);
    }
    cmd
}

fn read_non_symlink_regular_bytes(path: &Path, max_bytes: usize, label: &str) -> Result<Vec<u8>, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("stat `{label}`: {e}"))?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(format!("`{label}`: must be a regular non-symlink file"));
    }
    let max_u64 = u64::try_from(max_bytes).unwrap_or(u64::MAX);
    if meta.len() > max_u64 {
        return Err(format!("`{label}`: size {} exceeds bounded scan limit {max_bytes} bytes", meta.len()));
    }
    read_regular_file_bytes(path, max_bytes).map_err(|e| format!("read `{label}`: {e}"))
}

fn validate_deb_ar_listing(asset_name: &str, listing: &str) -> Result<(), String> {
    let mut saw_entry = false;
    for raw_line in listing.lines() {
        let entry = raw_line.trim_end_matches('\r');
        if entry.is_empty() {
            continue;
        }
        saw_entry = true;
        let trimmed = entry.trim_end_matches('/');
        if trimmed.is_empty() || trimmed.contains('/') {
            return Err(format!("`{asset_name}`: unsafe `.deb` ar member path `{entry}`"));
        }
        let _ = validate_safe_archive_member_path(asset_name, trimmed)?;
    }
    if !saw_entry {
        return Err(format!("`{asset_name}`: `.deb` archive contains zero ar members"));
    }
    Ok(())
}

fn validate_verbose_archive_listing(asset_name: &str, listing: &str, flat_deb_ar: bool) -> Result<(), String> {
    let mut saw_entry = false;
    for raw_line in listing.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        saw_entry = true;
        let entry = parse_verbose_listing_entry(asset_name, line)?;
        if flat_deb_ar {
            let trimmed = entry.trim_end_matches('/');
            if trimmed.is_empty() || trimmed.contains('/') {
                return Err(format!("`{asset_name}`: unsafe `.deb` ar member path `{entry}`"));
            }
            let _ = validate_safe_archive_member_path(asset_name, trimmed)?;
        } else {
            let _ = validate_safe_archive_member_path(asset_name, entry)?;
        }
    }
    if !saw_entry {
        return Err(format!("`{asset_name}`: archive contains zero entries"));
    }
    Ok(())
}

fn collect_token_spans(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= bytes.len() {
            break;
        }
        let start = idx;
        while idx < bytes.len() && !bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if let Some(tok) = line.get(start..idx) {
            out.push((start, tok));
        }
    }
    out
}

fn is_ascii_month(tok: &str) -> bool {
    matches!(tok, "Jan" | "Feb" | "Mar" | "Apr" | "May" | "Jun" | "Jul" | "Aug" | "Sep" | "Oct" | "Nov" | "Dec")
}

fn is_iso_date(tok: &str) -> bool {
    let b = tok.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit)
}

fn is_time_or_year(tok: &str) -> bool {
    let b = tok.as_bytes();
    let is_year = b.len() == 4 && b.iter().all(u8::is_ascii_digit);
    let is_hhmm = b.len() == 5 && b[2] == b':' && b[..2].iter().all(u8::is_ascii_digit) && b[3..5].iter().all(u8::is_ascii_digit);
    let is_hhmmss = b.len() == 8
        && b[2] == b':'
        && b[5] == b':'
        && b[..2].iter().all(u8::is_ascii_digit)
        && b[3..5].iter().all(u8::is_ascii_digit)
        && b[6..8].iter().all(u8::is_ascii_digit);
    is_year || is_hhmm || is_hhmmss
}

fn parse_verbose_listing_entry<'a>(asset_name: &str, line: &'a str) -> Result<&'a str, String> {
    let tokens = collect_token_spans(line);
    let Some((_, mode)) = tokens.first().copied() else {
        return Err(format!("`{asset_name}`: empty archive listing entry"));
    };
    let mode_bytes = mode.as_bytes();
    let is_mode = mode_bytes.len() >= 10 && mode_bytes[1..10].iter().all(|b| matches!(b, b'r' | b'w' | b'x' | b'-' | b's' | b'S' | b't' | b'T'));
    if !is_mode {
        return Err(format!("`{asset_name}`: unexpected archive listing format `{line}`"));
    }
    match mode_bytes[0] {
        b'-' | b'd' => {}
        b'l' => return Err(format!("`{asset_name}`: symbolic link archive member rejected: `{line}`")),
        b'h' => return Err(format!("`{asset_name}`: hard link archive member rejected: `{line}`")),
        _ => return Err(format!("`{asset_name}`: special/unsupported archive member rejected: `{line}`")),
    }

    let name_start = if tokens.len() >= 6 && is_iso_date(tokens[3].1) && is_time_or_year(tokens[4].1) {
        Some(tokens[5].0)
    } else if tokens.len() >= 9 && is_ascii_month(tokens[5].1) && tokens[6].1.bytes().all(|b| b.is_ascii_digit()) && is_time_or_year(tokens[7].1) {
        Some(tokens[8].0)
    } else {
        let mut found = None;
        for idx in 2..tokens.len() {
            if idx + 2 < tokens.len() && is_iso_date(tokens[idx].1) && is_time_or_year(tokens[idx + 1].1) {
                found = Some(tokens[idx + 2].0);
                break;
            }
            if idx + 3 < tokens.len()
                && is_ascii_month(tokens[idx].1)
                && tokens[idx + 1].1.bytes().all(|b| b.is_ascii_digit())
                && is_time_or_year(tokens[idx + 2].1)
            {
                found = Some(tokens[idx + 3].0);
                break;
            }
        }
        found
    };

    let Some(start) = name_start else {
        return Err(format!("`{asset_name}`: cannot locate archive member path in listing `{line}`"));
    };
    let rest = line.get(start..).ok_or_else(|| format!("`{asset_name}`: invalid path slice in `{line}`"))?;
    if rest.contains(" -> ") {
        return Err(format!("`{asset_name}`: symbolic link archive member rejected: `{line}`"));
    }
    if rest.contains(" link to ") {
        return Err(format!("`{asset_name}`: hard link archive member rejected: `{line}`"));
    }
    Ok(rest)
}

fn validate_safe_archive_member_path(asset_name: &str, raw_entry: &str) -> Result<PathBuf, String> {
    if raw_entry.contains('\0') || raw_entry.contains('\\') || raw_entry.starts_with('/') {
        return Err(format!("`{asset_name}`: unsafe archive member path `{raw_entry}`"));
    }
    let stripped = raw_entry.strip_prefix("./").unwrap_or(raw_entry);
    let trimmed = stripped.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        return Ok(PathBuf::new());
    }
    let mut out = PathBuf::new();
    for (idx, comp) in Path::new(trimmed).components().enumerate() {
        match comp {
            Component::Normal(seg) => {
                let seg_str = seg.to_string_lossy();
                if seg_str == ".." || (idx == 0 && seg_str.contains(':')) {
                    return Err(format!("`{asset_name}`: unsafe archive member path `{raw_entry}`"));
                }
                out.push(seg);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("`{asset_name}`: path traversal in archive member `{raw_entry}`"));
            }
        }
    }
    Ok(out)
}

fn read_prefix_bytes(path: &Path, count: usize) -> Result<Vec<u8>, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("stat `{}`: {e}", path.display()))?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(format!("`{}` must be a regular non-symlink file", path.display()));
    }
    let file = std::fs::File::open(path).map_err(|e| format!("open `{}`: {e}", path.display()))?;
    let mut buf = Vec::with_capacity(count);
    file.take(count as u64).read_to_end(&mut buf).map_err(|e| format!("read `{}`: {e}", path.display()))?;
    Ok(buf)
}

// ============================================================================
// Pure-Rust FIPS 180-4 SHA-256 implementation (zero external crate dependencies)
// ============================================================================

const SHA256_H0: [u32; 8] = [0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19];

const SHA256_K: [u32; 64] = [
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

struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    total_bytes: u64,
}

impl Sha256 {
    const fn new() -> Self {
        Self { state: SHA256_H0, block: [0u8; 64], block_len: 0, total_bytes: 0 }
    }

    fn update(&mut self, mut input: &[u8]) {
        self.total_bytes = self.total_bytes.wrapping_add(input.len() as u64);
        if self.block_len > 0 {
            let need = 64 - self.block_len;
            if input.len() < need {
                if let Some(dst) = self.block.get_mut(self.block_len..self.block_len + input.len()) {
                    dst.copy_from_slice(input);
                }
                self.block_len += input.len();
                return;
            }
            if let (Some(dst), Some((head, tail))) = (self.block.get_mut(self.block_len..64), input.split_at_checked(need)) {
                dst.copy_from_slice(head);
                input = tail;
            }
            let block_copy = self.block;
            compress_sha256_block(&mut self.state, &block_copy);
            self.block_len = 0;
        }

        let (chunks, remainder) = input.as_chunks::<64>();
        for chunk in chunks {
            compress_sha256_block(&mut self.state, chunk);
        }
        if !remainder.is_empty()
            && let Some(dst) = self.block.get_mut(..remainder.len())
        {
            dst.copy_from_slice(remainder);
            self.block_len = remainder.len();
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_bytes.wrapping_mul(8);
        if let Some(slot) = self.block.get_mut(self.block_len) {
            *slot = 0x80;
            self.block_len += 1;
        }
        if self.block_len > 56 {
            if let Some(tail) = self.block.get_mut(self.block_len..64) {
                tail.fill(0);
            }
            let block_copy = self.block;
            compress_sha256_block(&mut self.state, &block_copy);
            self.block.fill(0);
            self.block_len = 0;
        } else if let Some(pad) = self.block.get_mut(self.block_len..56) {
            pad.fill(0);
        }
        self.block[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let final_block = self.block;
        compress_sha256_block(&mut self.state, &final_block);

        let mut digest = [0u8; 32];
        for (slot, word) in digest.as_chunks_mut::<4>().0.iter_mut().zip(self.state) {
            *slot = word.to_be_bytes();
        }
        digest
    }
}

fn compress_sha256_block(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (dst, chunk) in w.iter_mut().take(16).zip(block.as_chunks::<4>().0) {
        *dst = u32::from_be_bytes(*chunk);
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
        let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h.wrapping_add(sum1).wrapping_add(ch).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
        let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = sum0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    for (s, delta) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *s = s.wrapping_add(delta);
    }
}

#[must_use]
pub(crate) fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize()
}

#[cfg(test)]
#[must_use]
fn sha256_hex(bytes: &[u8]) -> String {
    encode_lower_hex(&sha256_bytes(bytes))
}

fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("open `{}`: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("read `{}`: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        if let Some(chunk) = buf.get(..n) {
            hasher.update(chunk);
        }
    }
    Ok(encode_lower_hex(&hasher.finalize()))
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0x0f)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{
        ChecksumMode, ScanOptions, ScratchDirGuard, ToolConfig, find_squashfs_offset, parse_args, parse_sha256sums, scan_release_dir_with_tools,
        sha256_hex, validate_safe_archive_member_path,
    };

    fn join(parts: &[&str]) -> String {
        parts.concat()
    }

    fn build_squashfs_v4_superblock(block_size: u32, block_log: u16, comp_id: u16) -> [u8; 96] {
        let mut sb = [0u8; 96];
        sb[..4].copy_from_slice(b"hsqs");
        sb[12..16].copy_from_slice(&block_size.to_le_bytes());
        sb[20..22].copy_from_slice(&comp_id.to_le_bytes());
        sb[22..24].copy_from_slice(&block_log.to_le_bytes());
        sb[28..30].copy_from_slice(&4u16.to_le_bytes());
        sb[30..32].copy_from_slice(&0u16.to_le_bytes());
        sb
    }

    #[test]
    fn sha256_matches_nist_fips_180_4_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        let million_a = vec![b'a'; 1_000_000];
        assert_eq!(sha256_hex(&million_a), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn squashfs_offset_detector_validates_superblock_fields() {
        let mut image = vec![0x7f, b'E', b'L', b'F'];
        image.resize(256, 0x90);
        // Decoy magic with invalid major/minor version must be ignored.
        image.extend_from_slice(b"hsqs_not_a_valid_superblock_____________________________");
        image.resize(4096, 0);
        let expected_offset = image.len();
        image.extend_from_slice(&build_squashfs_v4_superblock(131_072, 17, 1));
        image.resize(expected_offset + 512, 0);

        assert_eq!(find_squashfs_offset(&image), Some(expected_offset));
        assert_eq!(find_squashfs_offset(b"plain binary without squashfs"), None);
    }

    #[test]
    fn cli_parser_and_member_path_guards_reject_invalid_inputs() {
        let ok = parse_args(&[String::from("dist/release")]).unwrap();
        assert_eq!(ok.checksum_mode, ChecksumMode::Required);
        assert!(ok.report_path.is_none());
        assert!(ok.expect_beta_set.is_none());
        assert!(ok.cross_os_reports_dir.is_none());

        let no_cs = parse_args(&[String::from("--no-checksums"), String::from("--report"), String::from("out.json"), String::from("dist")]).unwrap();
        assert_eq!(no_cs.checksum_mode, ChecksumMode::Disabled);
        assert_eq!(no_cs.report_path, Some(PathBuf::from("out.json")));

        let beta = parse_args(&[
            String::from("--expect-beta-set"),
            String::from("v0.1.0-beta.1"),
            String::from("--cross-os-reports"),
            String::from("dist/scan-reports"),
            String::from("dist/release"),
        ])
        .unwrap();
        assert_eq!(beta.expect_beta_set.as_deref(), Some("0.1.0-beta.1"));
        assert_eq!(beta.cross_os_reports_dir, Some(PathBuf::from("dist/scan-reports")));
        assert_eq!(beta.checksum_mode, ChecksumMode::Required);

        for bad in [
            vec![],
            vec!["--require-checksums", "--no-checksums", "dist"],
            vec!["--expect-beta-set", "0.1.0-beta.1", "--no-checksums", "dist"],
            vec!["--expect-beta-set", "../evil", "dist"],
            vec!["--expect-beta-set", "", "dist"],
            vec!["--expect-beta-set"],
            vec!["--cross-os-reports"],
            vec!["--unknown", "dist"],
            vec!["--report"],
            vec!["a", "b"],
        ] {
            let owned: Vec<String> = bad.into_iter().map(String::from).collect();
            assert!(parse_args(&owned).is_err(), "expected failure for {owned:?}");
        }

        for unsafe_member in ["../escape.txt", "a/../../escape.txt", "/etc/passwd", "C:\\Windows\\win.ini", "dir\\file.txt"] {
            assert!(validate_safe_archive_member_path("pkg.tar.gz", unsafe_member).is_err(), "accepted {unsafe_member}");
        }
        assert_eq!(validate_safe_archive_member_path("pkg.tar.gz", "./usr/bin/curvebook").unwrap(), PathBuf::from("usr/bin/curvebook"));

        let good_sums = format!("{}  curvebook-0.1.0-web.zip\n", "a".repeat(64));
        assert_eq!(parse_sha256sums(&good_sums).unwrap().len(), 1);
        for bad_sums in
            ["", "not-a-sha  file.zip\n", &format!("{}  ../escape.zip\n", "a".repeat(64)), &format!("{}  SHA256SUMS.txt\n", "a".repeat(64))]
        {
            assert!(parse_sha256sums(bad_sums).is_err());
        }
    }

    #[cfg(unix)]
    fn write_exec_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[cfg(unix)]
    fn write_checksums(dir: &Path) {
        let mut entries: Vec<String> = Vec::new();
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "SHA256SUMS.txt")
            .collect();
        names.sort();
        for name in names {
            let bytes = std::fs::read(dir.join(&name)).unwrap();
            entries.push(format!("{}  {name}\n", sha256_hex(&bytes)));
        }
        std::fs::write(dir.join("SHA256SUMS.txt"), entries.concat()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn tool_dispatch_unpacks_appimage_flatpak_msi_rpm_fallback_and_guarantees_dmg_detach_on_error() {
        let root = crate::workspace_root().unwrap();
        let scratch = ScratchDirGuard::new("unit-dispatch").unwrap();
        let bin_dir = scratch.path.join("mock-bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let detach_log = scratch.path.join("hdiutil-detach.log");
        let planted_brand = join(&["Ori", "gin", "Pro"]);

        // 1. Mock `unsquashfs`: verifies `-o <offset>` is passed, writes clean or contaminated payload.
        let unsquashfs = write_exec_script(
            &bin_dir,
            "unsquashfs",
            &format!(
                "#!/bin/sh\nset -eu\noffset=\"\"\ndest=\"\"\nsrc=\"\"\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    -no-progress) shift ;;\n    -o) offset=\"$2\"; shift 2 ;;\n    -d) dest=\"$2\"; shift 2 ;;\n    *) src=\"$1\"; shift ;;\n  esac\ndone\n[ \"$offset\" = \"512\" ] || {{ echo \"expected offset 512, got $offset\" >&2; exit 2; }}\nmkdir -p \"$dest/usr/share/doc/curvebook\"\ncase \"$src\" in\n  *dirty*) printf 'Contaminated {planted_brand} payload\\n' > \"$dest/usr/share/doc/curvebook/README.txt\" ;;\n  *) printf 'CurveBook clean AppImage\\n' > \"$dest/usr/share/doc/curvebook/README.txt\" ;;\nesac\n"
            ),
        );

        // 2. Mock `ostree` + `flatpak`: verifies `init --mode=archive`, `build-import-bundle`, `refs`, and `checkout --user-mode`.
        let ostree = write_exec_script(
            &bin_dir,
            "ostree",
            "#!/bin/sh\nset -eu\nrepo=\"${1#--repo=}\"\nshift\ncase \"$1\" in\n  init) mkdir -p \"$repo/objects\" ;;\n  refs) printf 'app/org.curvebook.CurveBook/x86_64/stable\\n' ;;\n  checkout)\n    dest=\"$4\"\n    mkdir -p \"$dest/files/share/doc/curvebook\"\n    cp \"$repo/imported.txt\" \"$dest/files/share/doc/curvebook/README.txt\"\n    ;;\n  *) exit 2 ;;\nesac\n",
        );
        let flatpak = write_exec_script(
            &bin_dir,
            "flatpak",
            "#!/bin/sh\nset -eu\n[ \"$1\" = \"build-import-bundle\" ] && [ \"$2\" = \"--no-update-summary\" ]\ncp \"$4\" \"$3/imported.txt\"\n",
        );

        // 3. Mock `hdiutil`: mounts payload including an `/Applications` symlink, and logs `detach` even when scan fails!
        let hdiutil = write_exec_script(
            &bin_dir,
            "hdiutil",
            &format!(
                "#!/bin/sh\nset -eu\ncase \"$1\" in\n  attach)\n    mnt=\"$6\"\n    dmg=\"$7\"\n    mkdir -p \"$mnt/CurveBook.app/Contents/Resources\"\n    ln -s /Applications \"$mnt/Applications\"\n    cp \"$dmg\" \"$mnt/CurveBook.app/Contents/Resources/README.txt\"\n    ;;\n  detach)\n    printf 'detached:%s\\n' \"$*\" >> \"{}\"\n    ;;\n  *) exit 2 ;;\nesac\n",
                detach_log.display()
            ),
        );

        // 4. Mock `msiexec`: verifies `/a <abs_msi> /qn TARGETDIR=<abs_dir>` and drops administrative `<name>.msi` stub.
        let msiexec = write_exec_script(
            &bin_dir,
            "msiexec",
            "#!/bin/sh\nset -eu\n[ \"$1\" = \"/a\" ] && [ \"$3\" = \"/qn\" ]\nmsi=\"$2\"\ntarget=\"${4#TARGETDIR=}\"\n[ \"$target\" != \"$4\" ]\ncase \"$msi\" in /*) ;; *) echo \"expected absolute /a path, got $msi\" >&2; exit 2 ;; esac\ncase \"$target\" in /*) ;; *) echo \"expected absolute TARGETDIR path, got $target\" >&2; exit 2 ;; esac\ncp \"$msi\" \"$target/$(basename \"$msi\")\"\nmkdir -p \"$target/PFiles/CurveBook\"\ncp \"$msi\" \"$target/PFiles/CurveBook/README.txt\"\n",
        );

        // 5. Mock `rpm2cpio`: emits a valid SVR4 `newc` cpio stream piped into real host `cpio -it` / `cpio -idm`.
        let rpm2cpio = write_exec_script(
            &bin_dir,
            "rpm2cpio",
            &format!(
                "#!/bin/sh\nset -eu\ntmp=\"$(mktemp -d)\"\ntrap 'rm -rf \"$tmp\"' EXIT\nmkdir -p \"$tmp/usr/share/doc/curvebook\"\ncase \"$1\" in\n  *dirty*) printf 'RPM leak {planted_brand}\\n' > \"$tmp/usr/share/doc/curvebook/README.txt\" ;;\n  *) printf 'CurveBook RPM fallback clean\\n' > \"$tmp/usr/share/doc/curvebook/README.txt\" ;;\nesac\n(cd \"$tmp\" && printf './usr\\n./usr/share\\n./usr/share/doc\\n./usr/share/doc/curvebook\\n./usr/share/doc/curvebook/README.txt\\n' | cpio -o -H newc 2>/dev/null)\n"
            ),
        );

        let tools = ToolConfig {
            bsdtar_bin: String::from("nonexistent-bsdtar-for-rpm2cpio-fallback"),
            rpm2cpio_bin: rpm2cpio.to_string_lossy().into_owned(),
            unsquashfs_bin: unsquashfs.to_string_lossy().into_owned(),
            ostree_bin: ostree.to_string_lossy().into_owned(),
            flatpak_bin: flatpak.to_string_lossy().into_owned(),
            hdiutil_bin: hdiutil.to_string_lossy().into_owned(),
            msiexec_bin: msiexec.to_string_lossy().into_owned(),
            allow_dmg: true,
            allow_msi: true,
            ..ToolConfig::default()
        };

        // Clean release directory across AppImage + Flatpak + DMG + MSI + RPM(rpm2cpio+cpio) passes and detaches DMG.
        let clean_dir = scratch.path.join("clean-rel");
        std::fs::create_dir_all(&clean_dir).unwrap();
        let mut appimage_bytes = vec![0x7f, b'E', b'L', b'F'];
        appimage_bytes.resize(512, 0);
        appimage_bytes.extend_from_slice(&build_squashfs_v4_superblock(131_072, 17, 1));
        let rpm_lead = [0xed, 0xab, 0xee, 0xdb, 3, 0, 0, 0];
        std::fs::write(clean_dir.join("curvebook-0.1.0-linux-x86_64.AppImage"), &appimage_bytes).unwrap();
        std::fs::write(clean_dir.join("curvebook-0.1.0-linux-x86_64.flatpak"), "CurveBook Flatpak clean\n").unwrap();
        std::fs::write(clean_dir.join("curvebook-0.1.0-macos-universal.dmg"), "CurveBook DMG clean\n").unwrap();
        std::fs::write(clean_dir.join("curvebook-0.1.0-windows-x64.msi"), "CurveBook MSI clean\n").unwrap();
        std::fs::write(clean_dir.join("curvebook-0.1.0-linux-x86_64.rpm"), rpm_lead).unwrap();
        write_checksums(&clean_dir);

        let clean_opts = ScanOptions {
            assets_dir: clean_dir,
            checksum_mode: ChecksumMode::Required,
            report_path: None,
            expect_beta_set: None,
            cross_os_reports_dir: None,
        };
        let report = scan_release_dir_with_tools(&root, &clean_opts, &tools).unwrap();
        assert_eq!(report.asset_count, 6);
        assert!(std::fs::read_to_string(&detach_log).unwrap().lines().count() == 1);

        // Contaminated AppImage, DMG, Flatpak, MSI, and RPM must ALL be detected with exact container names, AND DMG must still detach!
        let dirty_dir = scratch.path.join("dirty-rel");
        std::fs::create_dir_all(&dirty_dir).unwrap();
        let dirty_note = format!("Exported compatibility guide for {planted_brand}\n");
        std::fs::write(dirty_dir.join("curvebook-dirty.AppImage"), &appimage_bytes).unwrap();
        std::fs::write(dirty_dir.join("curvebook-dirty.flatpak"), &dirty_note).unwrap();
        std::fs::write(dirty_dir.join("curvebook-dirty.dmg"), &dirty_note).unwrap();
        std::fs::write(dirty_dir.join("curvebook-dirty.msi"), &dirty_note).unwrap();
        std::fs::write(dirty_dir.join("curvebook-dirty.rpm"), rpm_lead).unwrap();
        write_checksums(&dirty_dir);

        let dirty_opts = ScanOptions {
            assets_dir: dirty_dir,
            checksum_mode: ChecksumMode::Required,
            report_path: None,
            expect_beta_set: None,
            cross_os_reports_dir: None,
        };
        let err = scan_release_dir_with_tools(&root, &dirty_opts, &tools).unwrap_err();
        assert!(err.contains("curvebook-dirty.AppImage!"), "{err}");
        assert!(err.contains("curvebook-dirty.flatpak!"), "{err}");
        assert!(err.contains("curvebook-dirty.dmg!"), "{err}");
        assert!(err.contains("curvebook-dirty.msi!"), "{err}");
        assert!(err.contains("curvebook-dirty.rpm!"), "{err}");
        assert_eq!(std::fs::read_to_string(&detach_log).unwrap().lines().count(), 2, "DMG must detach even when scan fails");
    }

    #[cfg(unix)]
    fn build_minimal_clean_tar_gz(entry_path: &str, data: &[u8]) -> Vec<u8> {
        let mut tar = Vec::new();
        let mut hdr = [0u8; 512];
        let name_bytes = entry_path.as_bytes();
        hdr[..name_bytes.len()].copy_from_slice(name_bytes);
        hdr[100..108].copy_from_slice(b"0000644\0");
        hdr[108..116].copy_from_slice(b"0000000\0");
        hdr[116..124].copy_from_slice(b"0000000\0");
        let size_octal = format!("{:011o}\0", data.len());
        hdr[124..136].copy_from_slice(size_octal.as_bytes());
        hdr[136..148].copy_from_slice(b"00000000000\0");
        hdr[148..156].fill(b' ');
        hdr[156] = b'0';
        hdr[257..263].copy_from_slice(b"ustar\0");
        hdr[263..265].copy_from_slice(b"00");
        let chksum: u32 = hdr.iter().map(|&b| u32::from(b)).sum();
        hdr[148..156].copy_from_slice(format!("{chksum:06o}\0 ").as_bytes());
        tar.extend_from_slice(&hdr);
        tar.extend_from_slice(data);
        let pad = (512 - (data.len() % 512)) % 512;
        tar.resize(tar.len() + pad, 0);
        tar.resize(tar.len() + 1024, 0);

        let mut gz = vec![0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03];
        let len = tar.len() as u16;
        let nlen = !len;
        gz.push(1);
        gz.extend_from_slice(&len.to_le_bytes());
        gz.extend_from_slice(&nlen.to_le_bytes());
        gz.extend_from_slice(&tar);
        let mut crc = 0xffff_ffffu32;
        for &b in &tar {
            crc ^= u32::from(b);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320u32 & mask);
            }
        }
        gz.extend_from_slice(&(!crc).to_le_bytes());
        gz.extend_from_slice(&(tar.len() as u32).to_le_bytes());
        gz
    }

    #[cfg(unix)]
    #[test]
    fn expect_beta_set_and_cross_os_reports_verify_exact_8_files_and_fail_closed() {
        let root = crate::workspace_root().unwrap();
        let scratch = ScratchDirGuard::new("beta-set").unwrap();
        let bin_dir = scratch.path.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        let unsquashfs = write_exec_script(
            &bin_dir,
            "unsquashfs",
            "#!/bin/sh\nset -eu\ndest=\"\"\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    -no-progress) shift ;;\n    -o) shift 2 ;;\n    -d) dest=\"$2\"; shift 2 ;;\n    *) shift ;;\n  esac\ndone\nmkdir -p \"$dest/usr/share/doc/curvebook\"\nprintf 'CurveBook clean AppImage\\n' > \"$dest/usr/share/doc/curvebook/README.txt\"\n",
        );

        let ver = "0.1.0-beta.1";
        let rel_dir = scratch.path.join("release");
        std::fs::create_dir_all(&rel_dir).unwrap();

        let mut appimage_bytes = vec![0x7f, b'E', b'L', b'F'];
        appimage_bytes.resize(512, 0);
        appimage_bytes.extend_from_slice(&build_squashfs_v4_superblock(131_072, 17, 1));
        let dmg_bytes = b"CurveBook DMG fixture\n";
        let msi_x64_bytes = b"CurveBook MSI x64 fixture\n";
        let msi_arm64_bytes = b"CurveBook MSI arm64 fixture\n";
        let freebsd_tgz =
            build_minimal_clean_tar_gz("curvebook-0.1.0-beta.1-freebsd-x86_64/share/doc/curvebook/README.txt", b"CurveBook FreeBSD clean\n");

        std::fs::write(rel_dir.join(format!("curvebook-{ver}-macos-universal.dmg")), dmg_bytes).unwrap();
        std::fs::write(rel_dir.join(format!("curvebook-{ver}-windows-x64.msi")), msi_x64_bytes).unwrap();
        std::fs::write(rel_dir.join(format!("curvebook-{ver}-windows-arm64.msi")), msi_arm64_bytes).unwrap();
        std::fs::write(rel_dir.join(format!("curvebook-{ver}-linux-x86_64.AppImage")), &appimage_bytes).unwrap();
        std::fs::write(rel_dir.join(format!("curvebook-{ver}-linux-aarch64.AppImage")), &appimage_bytes).unwrap();
        std::fs::write(rel_dir.join(format!("curvebook-{ver}-freebsd-x86_64.tar.gz")), &freebsd_tgz).unwrap();
        std::fs::write(rel_dir.join("THIRD-PARTY-LICENSES.html"), "<html><title>CurveBook</title><body>MIT OR Apache-2.0</body></html>\n").unwrap();
        write_checksums(&rel_dir);

        let reports_dir = scratch.path.join("reports");
        std::fs::create_dir_all(&reports_dir).unwrap();
        std::fs::write(
            reports_dir.join("scan-report-macos.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "ok": true,
                "host_os": "macos",
                "assets": [{
                    "name": format!("curvebook-{ver}-macos-universal.dmg"),
                    "kind": "dmg",
                    "size_bytes": dmg_bytes.len(),
                    "sha256": sha256_hex(dmg_bytes),
                    "unpacked_files": 4,
                    "nested_archive_entries": 0,
                    "checked_assets": 1
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            reports_dir.join("scan-report-windows.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "ok": true,
                "host_os": "windows",
                "assets": [
                    {
                        "name": format!("curvebook-{ver}-windows-x64.msi"),
                        "kind": "msi",
                        "size_bytes": msi_x64_bytes.len(),
                        "sha256": sha256_hex(msi_x64_bytes),
                        "unpacked_files": 5,
                        "nested_archive_entries": 0,
                        "checked_assets": 1
                    },
                    {
                        "name": format!("curvebook-{ver}-windows-arm64.msi"),
                        "kind": "msi",
                        "size_bytes": msi_arm64_bytes.len(),
                        "sha256": sha256_hex(msi_arm64_bytes),
                        "unpacked_files": 5,
                        "nested_archive_entries": 0,
                        "checked_assets": 1
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let linux_host_tools =
            ToolConfig { unsquashfs_bin: unsquashfs.to_string_lossy().into_owned(), allow_dmg: false, allow_msi: false, ..ToolConfig::default() };

        // 1. Complete 8-file Beta set + matching cross-OS reports passes cleanly.
        let ok_opts = ScanOptions {
            assets_dir: rel_dir.clone(),
            checksum_mode: ChecksumMode::Required,
            report_path: None,
            expect_beta_set: Some(ver.to_owned()),
            cross_os_reports_dir: Some(reports_dir.clone()),
        };
        let report = scan_release_dir_with_tools(&root, &ok_opts, &linux_host_tools).unwrap();
        assert_eq!(report.asset_count, 8);
        assert_eq!(report.host_os, std::env::consts::OS);

        // 2. Without cross-os-reports, cross-OS .dmg / .msi STILL fails closed!
        let no_reports_opts = ScanOptions { cross_os_reports_dir: None, ..ok_opts.clone() };
        assert!(scan_release_dir_with_tools(&root, &no_reports_opts, &linux_host_tools).is_err());

        // 3. Forged wrong host_os or mismatched sha256 in cross-OS report fails closed!
        std::fs::write(
            reports_dir.join("scan-report-macos.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "ok": true,
                "host_os": "linux",
                "assets": [{
                    "name": format!("curvebook-{ver}-macos-universal.dmg"),
                    "kind": "dmg",
                    "size_bytes": dmg_bytes.len(),
                    "sha256": sha256_hex(dmg_bytes),
                    "unpacked_files": 4
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let bad_host_err = scan_release_dir_with_tools(&root, &ok_opts, &linux_host_tools).unwrap_err();
        assert!(bad_host_err.contains("host_os=macos"), "{bad_host_err}");

        // 4. Extra dropped asset fails closed even if all 8 files are present.
        std::fs::write(rel_dir.join(format!("curvebook-web-{ver}.zip")), b"PK\x05\x06").unwrap();
        write_checksums(&rel_dir);
        let extra_err = scan_release_dir_with_tools(&root, &ok_opts, &linux_host_tools).unwrap_err();
        assert!(extra_err.contains("unexpected extra file(s)") && extra_err.contains(&format!("curvebook-web-{ver}.zip")), "{extra_err}");
    }

    #[test]
    fn normalize_release_version_accepts_optional_v_and_build_metadata_and_rejects_invalid() {
        for (raw, expected) in [
            ("0.1.0", "0.1.0"),
            ("v0.1.0-beta.1", "0.1.0-beta.1"),
            (" 0.2.0-rc.1+build.001 ", "0.2.0-rc.1+build.001"),
            ("v1.2.3+sha.abcdef", "1.2.3+sha.abcdef"),
        ] {
            assert_eq!(super::normalize_release_version(raw).as_deref(), Ok(expected), "raw={raw}");
        }
        for bad in ["", "v", "vv0.1.0", "../evil", "0.1.0/foo", "0.1.0\\foo", "0.1.0+", "0.1.0+bad..meta", "0.1.0 beta", "1.2", "0.1.0-", "0.1.0-01"]
        {
            assert!(super::normalize_release_version(bad).is_err(), "expected failure for `{bad}`");
        }
    }

    #[test]
    fn absolute_tool_path_resolves_relative_paths_and_strips_windows_verbatim_prefixes() {
        let cwd = std::env::current_dir().unwrap();
        let rel = Path::new("dist").join("release").join("curvebook-0.1.0-windows-x64.msi");
        let abs = super::absolute_tool_path(&rel).unwrap();
        assert!(abs.is_absolute(), "expected absolute path, got {}", abs.display());
        assert!(abs.ends_with(&rel), "expected `{}` to end with `{}`", abs.display(), rel.display());
        assert_eq!(abs, cwd.join(&rel));
        assert!(!abs.to_string_lossy().starts_with(r"\\?\"));

        let dotted = Path::new(".").join("dist").join(".").join("release").join("curvebook.msi");
        assert_eq!(super::absolute_tool_path(&dotted).unwrap(), cwd.join("dist").join("release").join("curvebook.msi"));

        assert!(super::absolute_tool_path(Path::new("")).is_err());

        assert_eq!(super::strip_windows_verbatim_prefix(Path::new(r"\\?\C:\dist\release\app.msi")), PathBuf::from(r"C:\dist\release\app.msi"));
        assert_eq!(
            super::strip_windows_verbatim_prefix(Path::new(r"\\?\UNC\server\share\dist\app.msi")),
            PathBuf::from(r"\\server\share\dist\app.msi")
        );
        assert_eq!(super::strip_windows_verbatim_prefix(Path::new(r"C:\dist\release\app.msi")), PathBuf::from(r"C:\dist\release\app.msi"));
        assert_eq!(super::strip_windows_verbatim_prefix(Path::new("/tmp/dist/app.msi")), PathBuf::from("/tmp/dist/app.msi"));

        #[cfg(windows)]
        {
            assert_eq!(super::absolute_tool_path(Path::new(r"\\?\C:\dist\release\app.msi")).unwrap(), PathBuf::from(r"C:\dist\release\app.msi"));
            assert_eq!(
                super::strip_windows_verbatim_prefix(Path::new(r"\\?\UNC\server\share\dist\app.msi")),
                PathBuf::from(r"\\server\share\dist\app.msi")
            );
        }
    }

    #[test]
    fn fa_sec23_01_appimage_no_exec_default_and_verbose_archive_listing_checks() {
        assert!(!ToolConfig::default().allow_appimage_exec, "AppImage execution fallback must default to false on all OSes");

        // BSD/tar and GNU/tar clean regular + directory entries (including filenames with spaces) pass.
        let clean_listing = "\
drwxr-xr-x  0 user   group        0 Nov 23 04:50 ./usr/
-rw-r--r--  0 user   group      123 Nov 23 04:50 ./usr/share/doc/curvebook/Read Me.txt
-rw-r--r-- root/root       456 2026-11-23 04:50 usr/bin/curvebook
";
        assert!(super::validate_verbose_archive_listing("pkg.tar.gz", clean_listing, false).is_ok());

        // Symlinks, tar hardlinks, cpio hardlinks, special files, and traversal entries are rejected.
        for bad_line in [
            "lrwxr-xr-x  0 root   wheel        0 Nov 23 04:50 usr/bin/sym -> /etc/passwd",
            "hrw-r--r--  0 root   wheel        0 Nov 23 04:50 usr/bin/hard link to /etc/passwd",
            "-rw-r--r--  2 root   wheel       10 Nov 23 04:50 usr/bin/hard link to usr/bin/curvebook",
            "crw-r--r--  0 root   wheel     1, 3 Nov 23 04:50 dev/null",
            "prw-r--r--  0 root   wheel        0 Nov 23 04:50 var/fifo",
            "-rw-r--r--  0 root   wheel        7 Nov 23 04:50 ../escape.txt",
        ] {
            let err = super::validate_verbose_archive_listing("bad.tar.gz", bad_line, false).unwrap_err();
            assert!(!err.is_empty(), "expected rejection for {bad_line}");
        }

        // Debian ar flat listing accepts SVR4 trailing `/` and rejects subdirectories/traversal.
        assert!(super::validate_deb_ar_listing("pkg.deb", "debian-binary/\ncontrol.tar.gz/\ndata.tar.gz/\n").is_ok());
        assert!(super::validate_deb_ar_listing("pkg.deb", "sub/data.tar.gz\n").is_err());
        assert!(super::validate_deb_ar_listing("pkg.deb", "../escape\n").is_err());
    }
}
