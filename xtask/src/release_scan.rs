//! `cargo xtask release-scan`: unpack release assets in isolated temporary trees via
//! `curvebook-platform` bounded subprocesses, verify `SHA256SUMS.txt` parity, and run the
//! `brands`, `cleanroom`, and `assets` distribution scanners over every unpacked container (`R0.26`).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::scan_util::{has_ext_ci, read_dir_paths_sorted, rel_slash};
use crate::zip_read::{ZipArchive, ZipLimits};
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
THIRD-PARTY-LICENSES.html, *.cdx.json, *.zsync), verifies SHA-256 digests against SHA256SUMS.txt
by default, optionally checks the exact R0.28/R14.2 Beta asset set (--expect-beta-set <version>)
and verifies cross-OS .dmg/.msi builds via native scan reports (--cross-os-reports <dir>), fails
closed on unknown/unextractable formats or malformed CycloneDX SBOMs, and runs brands + cleanroom
+ assets (plus secrets on SBOMs) checks.";

const CHECKSUMS_FILE: &str = "SHA256SUMS.txt";
const LICENSES_HTML_FILE: &str = "THIRD-PARTY-LICENSES.html";
const SBOM_EXT: &str = ".cdx.json";

const SUBPROCESS_WALL_TIMEOUT: Duration = Duration::from_secs(180);
const SUBPROCESS_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const SUBPROCESS_STREAM_CAP_BYTES: usize = 64 * 1024 * 1024;
const CPIO_STREAM_CAP_BYTES: usize = 512 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 16_384;
const MAX_ARCHIVE_UNCOMPRESSED_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRY_BYTES: u64 = 512 * 1024 * 1024;

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
    Sbom,
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
            Self::Sbom => "sbom",
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

fn set_unique_flag<T>(slot: &mut Option<T>, flag: &str, value: T) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("duplicate `{flag}` argument\n\n{USAGE}"));
    }
    *slot = Some(value);
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
    while let Some(arg) = args.get(idx).map(String::as_str) {
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
                set_unique_flag(&mut report_path, "--report", PathBuf::from(take_flag_value(args, idx, "--report")?))?;
                idx += 2;
            }
            "--expect-beta-set" => {
                let ver = normalize_release_version(take_flag_value(args, idx, "--expect-beta-set")?)?;
                set_unique_flag(&mut expect_beta_set, "--expect-beta-set", ver)?;
                idx += 2;
            }
            "--cross-os-reports" => {
                set_unique_flag(&mut cross_os_reports_dir, "--cross-os-reports", PathBuf::from(take_flag_value(args, idx, "--cross-os-reports")?))?;
                idx += 2;
            }
            other if other.starts_with('-') => return Err(format!("unknown release-scan flag `{other}`\n\n{USAGE}")),
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
    let assets_dir = assets_dir.ok_or_else(|| USAGE.to_owned())?;
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
        format!("curvebook-{version}{SBOM_EXT}"),
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
        let paths: Vec<PathBuf> = read_dir_paths_sorted(dir)?.into_iter().filter(|p| p.is_file() && has_ext_ci(p, &["json"])).collect();
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
    let host_os = val
        .get("host_os")
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("cross-OS report `{}` missing `host_os`", path.display()))?;
    let assets = val
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("cross-OS report `{}` missing `assets` array", path.display()))?;
    for entry in assets {
        let name = entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("cross-OS report `{}` has asset entry missing `name`", path.display()))?;
        let req_str = |k: &str| {
            entry
                .get(k)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("cross-OS report `{}` asset `{name}` missing `{k}`", path.display()))
        };
        let req_u64 = |k: &str| {
            entry
                .get(k)
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| format!("cross-OS report `{}` asset `{name}` missing `{k}`", path.display()))
        };
        let opt_usize = |k: &str| usize::try_from(entry.get(k).and_then(serde_json::Value::as_u64).unwrap_or(0)).unwrap_or(usize::MAX);
        out.push(CrossOsVerifiedRecord {
            host_os: host_os.to_owned(),
            name: name.to_owned(),
            kind: req_str("kind")?.to_owned(),
            size_bytes: req_u64("size_bytes")?,
            sha256: req_str("sha256")?.to_ascii_lowercase(),
            unpacked_files: usize::try_from(req_u64("unpacked_files")?).unwrap_or(usize::MAX),
            nested_archive_entries: opt_usize("nested_archive_entries"),
            checked_assets: opt_usize("checked_assets"),
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

    let total_unpacked_files = checked_sum_usize(records.iter().map(|r| r.unpacked_files), "total_unpacked_files")?;
    let total_nested_archive_entries = checked_sum_usize(records.iter().map(|r| r.nested_archive_entries), "total_nested_archive_entries")?;
    let total_checked_assets = checked_sum_usize(records.iter().map(|r| r.checked_assets), "total_checked_assets")?;
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

fn checked_sum_usize(mut iter: impl Iterator<Item = usize>, field: &str) -> Result<usize, String> {
    iter.try_fold(0usize, usize::checked_add).ok_or_else(|| format!("release-scan report integer overflow while summing `{field}`"))
}

fn discover_release_assets(assets_dir: &Path) -> Result<Vec<DiscoveredAsset>, String> {
    let paths = read_dir_paths_sorted(assets_dir)?;
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
    const SUFFIXES: &[(&str, AssetKind)] = &[
        (SBOM_EXT, AssetKind::Sbom),
        (".zsync", AssetKind::Zsync),
        (".tar.gz", AssetKind::TarGz),
        (".tgz", AssetKind::TarGz),
        (".zip", AssetKind::Zip),
        (".deb", AssetKind::Deb),
        (".rpm", AssetKind::Rpm),
        (".appimage", AssetKind::AppImage),
        (".flatpak", AssetKind::Flatpak),
        (".dmg", AssetKind::Dmg),
        (".msi", AssetKind::Msi),
    ];
    let lower = name.to_ascii_lowercase();
    SUFFIXES
        .iter()
        .find_map(|&(suffix, kind)| lower.ends_with(suffix).then_some(kind))
        .ok_or_else(|| format!("unsupported or unknown release asset format `{name}` (failing closed)"))
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
    let compliance_record = || ScannedAssetRecord {
        name: item.name.clone(),
        kind: item.kind.label(),
        size_bytes: item.size_bytes,
        sha256: item.sha256.clone(),
        unpacked_files: 1,
        nested_archive_entries: 0,
        checked_assets: 0,
    };
    match item.kind {
        AssetKind::Checksums | AssetKind::HtmlCompliance => {
            let bytes = read_non_symlink_regular_bytes(&item.path, MAX_COMPLIANCE_TEXT_BYTES, &item.name)?;
            let text = std::str::from_utf8(&bytes).map_err(|e| format!("`{}` is not valid UTF-8: {e}", item.name))?;
            collect_compliance_hits(&item.name, text, &bytes, violations);
            Ok(compliance_record())
        }
        AssetKind::Sbom => {
            let bytes = read_non_symlink_regular_bytes(&item.path, MAX_COMPLIANCE_TEXT_BYTES, &item.name)?;
            let text = std::str::from_utf8(&bytes).map_err(|e| format!("`{}` is not valid UTF-8: {e}", item.name))?;
            validate_cyclonedx_sbom_json(&item.name, text)?;
            collect_compliance_hits(&item.name, text, &bytes, violations);
            let mut secret_hits = Vec::new();
            crate::secrets::scan_text(&item.name, text, &mut secret_hits);
            for hit in secret_hits {
                violations.push(format!("{}:{}: [secrets] {}", hit.location, hit.line, hit.kind));
            }
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
            Ok(compliance_record())
        }
        _ => {
            let cross_os_host = match item.kind {
                AssetKind::Dmg if !tools.allow_dmg => Some(("macos", "macOS")),
                AssetKind::Msi if !tools.allow_msi => Some(("windows", "Windows")),
                _ => None,
            };
            if let (Some((host_id, host_label)), Some(reports)) = (cross_os_host, cross_os_reports) {
                return reports.match_native_asset(host_id, item).ok_or_else(|| {
                    format!(
                        "`{}`: missing matching verified {host_label} native scan report (`host_os={host_id}`, exact size/sha256, `unpacked_files > 0`)",
                        item.name
                    )
                });
            }
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

fn validate_cyclonedx_sbom_json(asset_name: &str, text: &str) -> Result<(), String> {
    let val: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("`{asset_name}`: invalid CycloneDX JSON: {e}"))?;
    let Some(obj) = val.as_object() else {
        return Err(format!("`{asset_name}`: CycloneDX SBOM root must be a JSON object"));
    };
    let bom_format = obj.get("bomFormat").and_then(serde_json::Value::as_str);
    if bom_format != Some("CycloneDX") {
        return Err(format!("`{asset_name}`: expected CycloneDX SBOM with `\"bomFormat\": \"CycloneDX\"`, found {bom_format:?}"));
    }
    let spec_version = obj.get("specVersion").and_then(serde_json::Value::as_str).map(str::trim).unwrap_or("");
    if spec_version.is_empty() {
        return Err(format!("`{asset_name}`: CycloneDX SBOM is missing non-empty `specVersion`"));
    }
    let Some(components) = obj.get("components").and_then(serde_json::Value::as_array) else {
        return Err(format!("`{asset_name}`: CycloneDX SBOM is missing `components` array"));
    };
    if components.is_empty() {
        return Err(format!("`{asset_name}`: CycloneDX SBOM has empty `components` array"));
    }
    for (idx, comp) in components.iter().enumerate() {
        let has_name = comp.as_object().and_then(|c| c.get("name")).and_then(serde_json::Value::as_str).is_some_and(|n| !n.trim().is_empty());
        if !has_name {
            return Err(format!("`{asset_name}`: CycloneDX `components[{idx}]` is missing a non-empty string `name`"));
        }
    }
    Ok(())
}

fn extract_zsync_header_utf8<'a>(asset_name: &str, bytes: &'a [u8]) -> Result<&'a str, String> {
    if let Ok(all_utf8) = std::str::from_utf8(bytes) {
        if all_utf8.trim().is_empty() {
            return Err(format!("`{asset_name}`: zsync file is blank"));
        }
        return Ok(all_utf8);
    }
    let lf_pos = bytes.windows(2).position(|w| w == b"\n\n");
    let crlf_pos = bytes.windows(4).position(|w| w == b"\r\n\r\n");
    let header_end = match (lf_pos, crlf_pos) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => {
            return Err(format!("`{asset_name}`: non-UTF-8 `.zsync` file lacks standard header separator before block checksums"));
        }
    };
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
        AssetKind::Checksums | AssetKind::HtmlCompliance | AssetKind::Sbom | AssetKind::Zsync => Ok(()),
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

fn scan_symlink_entry(asset_name: &str, rel_norm: &str, path: &Path, violations: &mut Vec<String>) -> Result<(), String> {
    let target = std::fs::read_link(path).map_err(|e| format!("read_link `{asset_name}!{rel_norm}`: {e}"))?;
    let target_str = target.to_string_lossy();
    let src = format!("{asset_name}!{rel_norm}");
    for hit in crate::brands::scan_dist_label(&src, rel_norm) {
        violations.push(format!("{}:{}: [brands] `{}`", hit.path, hit.line, hit.brand));
    }
    for hit in crate::brands::scan_dist_label(&src, &target_str) {
        violations.push(format!("{}:{}: [brands] symlink target `{}`", hit.path, hit.line, hit.brand));
    }
    Ok(())
}

fn scan_tree_symlinks(asset_name: &str, root: &Path, dir: &Path, violations: &mut Vec<String>) -> Result<(), String> {
    for path in read_dir_paths_sorted(dir)? {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            let rel = rel_slash(root, &path)?;
            scan_symlink_entry(asset_name, &rel, &path, violations)?;
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
    let mut seen_folded = BTreeSet::new();
    let mut expected_files = 0usize;
    for (entry_rel, payload) in zip.iter() {
        let safe_rel = validate_safe_archive_member_path(asset_name, entry_rel)?;
        let folded = safe_rel.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        if !seen_folded.insert(folded) {
            return Err(format!("`{asset_name}`: duplicate or case-colliding ZIP member path `{entry_rel}`"));
        }
        let out_path = stage_dir.join(&safe_rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir for `{asset_name}!{entry_rel}`: {e}"))?;
        }
        write_atomic(&out_path, payload).map_err(|e| format!("write `{asset_name}!{entry_rel}`: {e}"))?;
        expected_files += 1;
    }
    verify_unpacked_regular_file_count(asset_name, stage_dir, expected_files)
}

fn verify_unpacked_regular_file_count(asset_name: &str, stage_dir: &Path, expected: usize) -> Result<(), String> {
    let mut files = Vec::new();
    crate::scan_util::collect_dist_files(stage_dir, stage_dir, &mut files)?;
    if files.len() != expected {
        return Err(format!(
            "`{asset_name}`: extracted {} regular file(s) but archive listing declared {expected} (possible overwrite or hidden entry)",
            files.len()
        ));
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

    let (chosen_tar, list_out) = run_tar_or_bsdtar(tools, &["--numeric-owner", list_flag, archive_str.as_ref()], asset_name, "list entries")?;
    let listing = String::from_utf8_lossy(&list_out);
    let summary = validate_verbose_archive_listing(asset_name, &listing, false)?;

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
    verify_unpacked_regular_file_count(asset_name, stage_dir, summary.regular_files)
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
    let mut debian_binaries = Vec::new();
    let mut unexpected_members = Vec::new();
    for path in read_dir_paths_sorted(&ar_dir)? {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat deb-ar entry `{asset_name}`: {e}"))?;
        if meta.file_type().is_symlink() || !meta.file_type().is_file() {
            return Err(format!("`{asset_name}`: non-regular or symlink `.deb` `ar` member `{}`", path.display()));
        }
        let Some(fname) = path.file_name().and_then(|n| n.to_str()) else {
            return Err(format!("`{asset_name}`: non-UTF-8 `.deb` `ar` member filename"));
        };
        if is_deb_tar_member(fname, "data.tar") {
            data_tars.push(path);
        } else if is_deb_tar_member(fname, "control.tar") {
            control_tars.push(path);
        } else if fname == "debian-binary" {
            debian_binaries.push(path);
        } else {
            unexpected_members.push(fname.to_owned());
        }
    }
    if !unexpected_members.is_empty() {
        return Err(format!("`{asset_name}`: unexpected extra `.deb` `ar` member(s) rejected (failing closed): {}", unexpected_members.join(", ")));
    }
    let [deb_bin] = debian_binaries.as_slice() else {
        return Err(format!("`{asset_name}`: expected exactly 1 `debian-binary` member in `.deb`, found {}", debian_binaries.len()));
    };
    let deb_bin_bytes = read_non_symlink_regular_bytes(deb_bin, 64, &format!("{asset_name}!debian-binary"))?;
    let deb_bin_str = std::str::from_utf8(&deb_bin_bytes).map_err(|_| format!("`{asset_name}`: `debian-binary` is not valid UTF-8"))?;
    if deb_bin_str.trim() != "2.0" {
        return Err(format!("`{asset_name}`: unexpected `debian-binary` content `{}` (expected `2.0`)", deb_bin_str.trim()));
    }

    let [data_tar] = data_tars.as_slice() else {
        return Err(format!("`{asset_name}`: expected exactly 1 `data.tar*` member in `.deb`, found {}", data_tars.len()));
    };
    let [ctrl_tar] = control_tars.as_slice() else {
        return Err(format!("`{asset_name}`: expected exactly 1 `control.tar*` member in `.deb`, found {}", control_tars.len()));
    };

    let data_stage = stage_dir.join("_deb_data");
    std::fs::create_dir_all(&data_stage).map_err(|e| format!("mkdir _deb_data for `{asset_name}`: {e}"))?;
    unpack_tarball(tools, &format!("{asset_name}!data"), data_tar, &data_stage, false)?;
    let ctrl_stage = stage_dir.join("_deb_control");
    std::fs::create_dir_all(&ctrl_stage).map_err(|e| format!("mkdir _deb_control for `{asset_name}`: {e}"))?;
    unpack_tarball(tools, &format!("{asset_name}!control"), ctrl_tar, &ctrl_stage, false)?;
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
    match build_cmd(tools, &tools.bsdtar_bin).args(["--numeric-owner", "-tvf", deb_str.as_ref()]).run() {
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

    match build_cmd(tools, &tools.bsdtar_bin).args(["--numeric-owner", "-tvf", rpm_str.as_ref()]).run() {
        Ok(list_out) if list_out.success => {
            let summary = validate_verbose_archive_listing(asset_name, &String::from_utf8_lossy(&list_out.stdout), false)?;
            let ext_out = build_cmd(tools, &tools.bsdtar_bin)
                .args(["-xf", rpm_str.as_ref(), "-C", stage_str.as_ref()])
                .run()
                .map_err(|e| format!("`{asset_name}`: bsdtar extract failed: {e}"))?;
            if ext_out.success {
                return verify_unpacked_regular_file_count(asset_name, stage_dir, summary.regular_files);
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
        .arg("-itvn")
        .stdin_bytes(cpio_stream.clone())
        .run()
        .map_err(|e| format!("`{asset_name}`: cpio -itvn failed: {e}"))?;
    if !list_cpio.success {
        return Err(format!("`{asset_name}`: cpio -itvn failed: {}", String::from_utf8_lossy(&list_cpio.stderr).trim()));
    }
    let summary = validate_verbose_archive_listing(asset_name, &String::from_utf8_lossy(&list_cpio.stdout), false)?;

    // Prefer GNU `cpio --no-absolute-filenames`; BSD `bsdcpio` rejects absolute paths by default
    // (unless `--insecure` is passed) and does not recognize `--no-absolute-filenames`.
    let extract_cpio = extract_cpio_no_absolute(tools, asset_name, stage_dir, cpio_stream)?;
    if !extract_cpio.success {
        return Err(format!("`{asset_name}`: cpio -idm failed: {}", String::from_utf8_lossy(&extract_cpio.stderr).trim()));
    }
    verify_unpacked_regular_file_count(asset_name, stage_dir, summary.regular_files)
}

fn extract_cpio_no_absolute(
    tools: &ToolConfig,
    asset_name: &str,
    stage_dir: &Path,
    cpio_stream: Vec<u8>,
) -> Result<curvebook_platform::process::ProcessOutput, String> {
    let first = build_cmd(tools, &tools.cpio_bin)
        .current_dir(stage_dir)
        .args(["-idm", "--no-absolute-filenames"])
        .stdin_bytes(cpio_stream.clone())
        .run()
        .map_err(|e| format!("`{asset_name}`: cpio -idm --no-absolute-filenames failed: {e}"))?;
    if first.success {
        return Ok(first);
    }
    let stderr = String::from_utf8_lossy(&first.stderr).to_ascii_lowercase();
    let is_bsd_unknown_flag = stderr.contains("unrecognized option")
        || stderr.contains("unknown option")
        || stderr.contains("illegal option")
        || stderr.contains("is not supported")
        || stderr.contains("brief usage:")
        || stderr.contains("usage: cpio");
    if !is_bsd_unknown_flag {
        return Ok(first);
    }
    // Confirm this really is BSD `bsdcpio` (which rejects absolute paths unless `--insecure` is set).
    let ver = build_cmd(tools, &tools.cpio_bin).arg("--version").run();
    let is_bsdcpio = ver.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("bsdcpio"));
    if !is_bsdcpio {
        return Ok(first);
    }
    build_cmd(tools, &tools.cpio_bin)
        .current_dir(stage_dir)
        .arg("-idm")
        .stdin_bytes(cpio_stream)
        .run()
        .map_err(|e| format!("`{asset_name}`: cpio -idm failed: {e}"))
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
    for path in read_dir_paths_sorted(current_dir)? {
        let rel = path.strip_prefix(mount_root).map_err(|e| format!("strip mount prefix {}: {e}", path.display()))?;
        let rel_norm = rel.to_string_lossy().replace('\\', "/");
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat mounted `{asset_name}!{rel_norm}`: {e}"))?;
        if meta.file_type().is_symlink() {
            scan_symlink_entry(asset_name, &rel_norm, &path, violations)?;
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
    let mut seen_folded = BTreeSet::new();
    let mut entry_count = 0usize;
    for raw_line in listing.lines() {
        let entry = raw_line.trim_end_matches('\r');
        if entry.is_empty() {
            continue;
        }
        entry_count += 1;
        if entry_count > MAX_ARCHIVE_ENTRIES {
            return Err(format!("`{asset_name}`: `.deb` ar entry count exceeds cap {MAX_ARCHIVE_ENTRIES}"));
        }
        let trimmed = entry.trim_end_matches('/');
        if trimmed.is_empty() || trimmed.contains('/') {
            return Err(format!("`{asset_name}`: unsafe `.deb` ar member path `{entry}`"));
        }
        let safe = validate_safe_archive_member_path(asset_name, trimmed)?;
        let folded = safe.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        if !seen_folded.insert(folded) {
            return Err(format!("`{asset_name}`: duplicate or case-colliding `.deb` ar member `{entry}`"));
        }
    }
    if entry_count == 0 {
        return Err(format!("`{asset_name}`: `.deb` archive contains zero ar members"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ListingSummary {
    regular_files: usize,
    total_entries: usize,
    total_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
struct ParsedListingEntry<'a> {
    is_regular_file: bool,
    size_bytes: u64,
    path: &'a str,
}

fn validate_verbose_archive_listing(asset_name: &str, listing: &str, flat_deb_ar: bool) -> Result<ListingSummary, String> {
    let mut seen_folded = BTreeSet::new();
    let mut total_entries = 0usize;
    let mut regular_files = 0usize;
    let mut total_bytes = 0u64;

    for raw_line in listing.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        total_entries += 1;
        if total_entries > MAX_ARCHIVE_ENTRIES {
            return Err(format!("`{asset_name}`: archive entry count exceeds cap {MAX_ARCHIVE_ENTRIES}"));
        }
        let parsed = parse_verbose_listing_entry(asset_name, line)?;
        if parsed.size_bytes > MAX_ARCHIVE_ENTRY_BYTES {
            return Err(format!(
                "`{asset_name}`: archive member `{}` size {} exceeds per-entry cap {MAX_ARCHIVE_ENTRY_BYTES}",
                parsed.path, parsed.size_bytes
            ));
        }
        total_bytes = total_bytes
            .checked_add(parsed.size_bytes)
            .filter(|&sum| sum <= MAX_ARCHIVE_UNCOMPRESSED_BYTES)
            .ok_or_else(|| format!("`{asset_name}`: archive aggregate uncompressed size exceeds cap {MAX_ARCHIVE_UNCOMPRESSED_BYTES}"))?;

        let safe_path = if flat_deb_ar {
            let trimmed = parsed.path.trim_end_matches('/');
            if trimmed.is_empty() || trimmed.contains('/') {
                return Err(format!("`{asset_name}`: unsafe `.deb` ar member path `{}`", parsed.path));
            }
            validate_safe_archive_member_path(asset_name, trimmed)?
        } else {
            validate_safe_archive_member_path(asset_name, parsed.path)?
        };
        let norm = safe_path.to_string_lossy().replace('\\', "/");
        if !norm.is_empty() && !seen_folded.insert(norm.to_ascii_lowercase()) {
            return Err(format!("`{asset_name}`: duplicate or case-colliding archive member path `{}`", parsed.path));
        }
        if parsed.is_regular_file {
            if norm.is_empty() {
                return Err(format!("`{asset_name}`: regular file archive member has empty normalized path `{}`", parsed.path));
            }
            regular_files += 1;
        }
    }
    if total_entries == 0 {
        return Err(format!("`{asset_name}`: archive contains zero entries"));
    }
    Ok(ListingSummary { regular_files, total_entries, total_bytes })
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

fn parse_size_token(tok: &str) -> Option<u64> {
    if !tok.is_empty() && tok.bytes().all(|b| b.is_ascii_digit()) { tok.parse::<u64>().ok() } else { None }
}

fn parse_verbose_listing_entry<'a>(asset_name: &str, line: &'a str) -> Result<ParsedListingEntry<'a>, String> {
    let tokens = collect_token_spans(line);
    let Some((_, mode)) = tokens.first().copied() else {
        return Err(format!("`{asset_name}`: empty archive listing entry"));
    };
    let mode_bytes = mode.as_bytes();
    let is_mode = mode_bytes.len() >= 10 && mode_bytes[1..10].iter().all(|b| matches!(b, b'r' | b'w' | b'x' | b'-' | b's' | b'S' | b't' | b'T'));
    if !is_mode {
        return Err(format!("`{asset_name}`: unexpected archive listing format `{line}`"));
    }
    let is_regular_file = match mode_bytes[0] {
        b'-' => true,
        b'd' => false,
        b'l' => return Err(format!("`{asset_name}`: symbolic link archive member rejected: `{line}`")),
        b'h' => return Err(format!("`{asset_name}`: hard link archive member rejected: `{line}`")),
        _ => return Err(format!("`{asset_name}`: special/unsupported archive member rejected: `{line}`")),
    };

    // Match strict GNU tar (`mode owner/group SIZE YYYY-MM-DD HH:MM path`) or BSD tar / cpio
    // (`mode nlink owner group SIZE Mon DD HH:MM|YYYY path`) formats, requiring the file size token
    // immediately before the timestamp so spoofed `uname`/`gname` strings cannot shift the path start.
    let parsed_anchor = if tokens.len() >= 6
        && tokens[1].1.contains('/')
        && let Some(sz) = parse_size_token(tokens[2].1)
        && is_iso_date(tokens[3].1)
        && is_time_or_year(tokens[4].1)
    {
        Some((sz, tokens[5].0))
    } else if tokens.len() >= 9
        && parse_size_token(tokens[1].1).is_some()
        && let Some(sz) = parse_size_token(tokens[4].1)
        && is_ascii_month(tokens[5].1)
        && tokens[6].1.bytes().all(|b| b.is_ascii_digit())
        && is_time_or_year(tokens[7].1)
    {
        Some((sz, tokens[8].0))
    } else if tokens.len() >= 8
        && parse_size_token(tokens[1].1).is_some()
        && let Some(sz) = parse_size_token(tokens[4].1)
        && is_iso_date(tokens[5].1)
        && is_time_or_year(tokens[6].1)
    {
        Some((sz, tokens[7].0))
    } else if parse_size_token(tokens.get(1).map_or("", |t| t.1)).is_some() {
        // Fallback when owner/group contains whitespace (non-numeric listing): require `idx >= 5`
        // (past mode, nlink, owner, group, size) and a numeric size token immediately preceding date.
        let mut found = None;
        for idx in 5..tokens.len() {
            let Some(sz) = parse_size_token(tokens[idx - 1].1) else {
                continue;
            };
            if idx + 2 < tokens.len() && is_iso_date(tokens[idx].1) && is_time_or_year(tokens[idx + 1].1) {
                found = Some((sz, tokens[idx + 2].0));
                break;
            }
            if idx + 3 < tokens.len()
                && is_ascii_month(tokens[idx].1)
                && tokens[idx + 1].1.bytes().all(|b| b.is_ascii_digit())
                && is_time_or_year(tokens[idx + 2].1)
            {
                found = Some((sz, tokens[idx + 3].0));
                break;
            }
        }
        found
    } else {
        None
    };

    let Some((size_bytes, start)) = parsed_anchor else {
        return Err(format!("`{asset_name}`: cannot locate archive member path in listing `{line}`"));
    };
    let rest = line.get(start..).ok_or_else(|| format!("`{asset_name}`: invalid path slice in `{line}`"))?;
    if rest.contains(" -> ") {
        return Err(format!("`{asset_name}`: symbolic link archive member rejected: `{line}`"));
    }
    if rest.contains(" link to ") {
        return Err(format!("`{asset_name}`: hard link archive member rejected: `{line}`"));
    }
    Ok(ParsedListingEntry { is_regular_file, size_bytes, path: rest })
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

#[must_use]
pub(crate) fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    curvebook_platform::sha256::sha256_bytes(bytes)
}

#[cfg(test)]
#[must_use]
fn sha256_hex(bytes: &[u8]) -> String {
    curvebook_platform::sha256::sha256_hex(bytes)
}

fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("open `{}`: {e}", path.display()))?;
    let mut hasher = curvebook_platform::sha256::Sha256::new();
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
        hdr[..entry_path.len()].copy_from_slice(entry_path.as_bytes());
        hdr[100..108].copy_from_slice(b"0000644\0");
        hdr[108..116].copy_from_slice(b"0000000\0");
        hdr[116..124].copy_from_slice(b"0000000\0");
        hdr[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        hdr[136..148].copy_from_slice(b"00000000000\0");
        hdr[148..156].fill(b' ');
        hdr[156] = b'0';
        hdr[257..265].copy_from_slice(b"ustar\x0000");
        let chksum: u32 = hdr.iter().map(|&b| u32::from(b)).sum();
        hdr[148..156].copy_from_slice(format!("{chksum:06o}\0 ").as_bytes());
        tar.extend_from_slice(&hdr);
        tar.extend_from_slice(data);
        tar.resize(tar.len() + ((512 - (data.len() % 512)) % 512) + 1024, 0);

        let mut gz = vec![0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x01];
        let len = tar.len() as u16;
        gz.extend_from_slice(&len.to_le_bytes());
        gz.extend_from_slice(&(!len).to_le_bytes());
        gz.extend_from_slice(&tar);
        gz.extend_from_slice(&crc32fast::hash(&tar).to_le_bytes());
        gz.extend_from_slice(&(tar.len() as u32).to_le_bytes());
        gz
    }

    #[cfg(unix)]
    fn write_cross_os_report(path: &Path, host_os: &str, entries: &[(&str, &str, &[u8], usize)]) {
        let assets: Vec<serde_json::Value> = entries
            .iter()
            .map(|&(name, kind, bytes, unpacked)| {
                serde_json::json!({
                    "name": name, "kind": kind, "size_bytes": bytes.len(), "sha256": sha256_hex(bytes),
                    "unpacked_files": unpacked, "nested_archive_entries": 0, "checked_assets": 1
                })
            })
            .collect();
        std::fs::write(path, serde_json::to_vec_pretty(&serde_json::json!({"ok": true, "host_os": host_os, "assets": assets})).unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn expect_beta_set_and_cross_os_reports_verify_exact_9_files_and_fail_closed() {
        let root = crate::workspace_root().unwrap();
        let scratch = ScratchDirGuard::new("beta-set").unwrap();
        let bin_dir = scratch.path.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let unsquashfs = write_exec_script(
            &bin_dir,
            "unsquashfs",
            "#!/bin/sh\nset -eu\ndest=\"\"\nwhile [ $# -gt 0 ]; do case \"$1\" in -no-progress) shift ;; -o) shift 2 ;; -d) dest=\"$2\"; shift 2 ;; *) shift ;; esac; done\nmkdir -p \"$dest/usr/share/doc/curvebook\"\nprintf 'CurveBook clean AppImage\\n' > \"$dest/usr/share/doc/curvebook/README.txt\"\n",
        );

        let ver = "0.1.0-beta.1";
        let rel_dir = scratch.path.join("release");
        std::fs::create_dir_all(&rel_dir).unwrap();
        let mut appimage_bytes = vec![0x7f, b'E', b'L', b'F'];
        appimage_bytes.resize(512, 0);
        appimage_bytes.extend_from_slice(&build_squashfs_v4_superblock(131_072, 17, 1));
        let (dmg_b, x64_b, arm_b) = (b"CurveBook DMG fixture\n", b"CurveBook MSI x64 fixture\n", b"CurveBook MSI arm64 fixture\n");
        let freebsd_tgz =
            build_minimal_clean_tar_gz("curvebook-0.1.0-beta.1-freebsd-x86_64/share/doc/curvebook/README.txt", b"CurveBook FreeBSD clean\n");
        let sbom_json = serde_json::to_vec_pretty(&serde_json::json!({
            "bomFormat": "CycloneDX",
            "specVersion": "1.5",
            "version": 1,
            "components": [{ "type": "library", "name": "serde", "version": "1.0.219" }]
        }))
        .unwrap();
        let dmg_name = format!("curvebook-{ver}-macos-universal.dmg");
        let x64_name = format!("curvebook-{ver}-windows-x64.msi");
        let arm_name = format!("curvebook-{ver}-windows-arm64.msi");
        for (name, bytes) in [
            (dmg_name.as_str(), &dmg_b[..]),
            (x64_name.as_str(), &x64_b[..]),
            (arm_name.as_str(), &arm_b[..]),
            (&format!("curvebook-{ver}-linux-x86_64.AppImage"), &appimage_bytes[..]),
            (&format!("curvebook-{ver}-linux-aarch64.AppImage"), &appimage_bytes[..]),
            (&format!("curvebook-{ver}-freebsd-x86_64.tar.gz"), &freebsd_tgz[..]),
            (&format!("curvebook-{ver}.cdx.json"), &sbom_json[..]),
            ("THIRD-PARTY-LICENSES.html", b"<html><title>CurveBook</title><body>MIT OR Apache-2.0</body></html>\n"),
        ] {
            std::fs::write(rel_dir.join(name), bytes).unwrap();
        }
        write_checksums(&rel_dir);

        let reports_dir = scratch.path.join("reports");
        std::fs::create_dir_all(&reports_dir).unwrap();
        write_cross_os_report(&reports_dir.join("scan-report-macos.json"), "macos", &[(&dmg_name, "dmg", dmg_b, 4)]);
        write_cross_os_report(
            &reports_dir.join("scan-report-windows.json"),
            "windows",
            &[(&x64_name, "msi", x64_b, 5), (&arm_name, "msi", arm_b, 5)],
        );

        let linux_tools =
            ToolConfig { unsquashfs_bin: unsquashfs.to_string_lossy().into_owned(), allow_dmg: false, allow_msi: false, ..ToolConfig::default() };

        // 1. Complete 9-file Beta set (including CycloneDX SBOM) + matching cross-OS reports passes cleanly.
        let ok_opts = ScanOptions {
            assets_dir: rel_dir.clone(),
            checksum_mode: ChecksumMode::Required,
            report_path: None,
            expect_beta_set: Some(ver.to_owned()),
            cross_os_reports_dir: Some(reports_dir.clone()),
        };
        let report = scan_release_dir_with_tools(&root, &ok_opts, &linux_tools).unwrap();
        assert_eq!((report.asset_count, report.host_os.as_str()), (9, std::env::consts::OS));
        assert!(report.assets.iter().any(|a| a.name == format!("curvebook-{ver}.cdx.json") && a.kind == "sbom"));

        assert!(scan_release_dir_with_tools(&root, &ScanOptions { cross_os_reports_dir: None, ..ok_opts.clone() }, &linux_tools).is_err());

        write_cross_os_report(&reports_dir.join("scan-report-macos.json"), "linux", &[(&dmg_name, "dmg", dmg_b, 4)]);
        let bad_host = scan_release_dir_with_tools(&root, &ok_opts, &linux_tools).unwrap_err();
        assert!(bad_host.contains("host_os=macos"), "{bad_host}");

        // 4. Extra dropped asset fails closed even if all 9 files are present.
        std::fs::write(rel_dir.join(format!("curvebook-web-{ver}.zip")), b"PK\x05\x06").unwrap();
        write_checksums(&rel_dir);
        let extra = scan_release_dir_with_tools(&root, &ok_opts, &linux_tools).unwrap_err();
        assert!(extra.contains("unexpected extra file(s)") && extra.contains(&format!("curvebook-web-{ver}.zip")), "{extra}");
    }

    #[test]
    fn cyclonedx_sbom_validator_accepts_valid_and_rejects_malformed() {
        let valid =
            r#"{"bomFormat":"CycloneDX","specVersion":"1.5","version":1,"components":[{"type":"library","name":"serde","version":"1.0.219"}]}"#;
        assert!(super::validate_cyclonedx_sbom_json("curvebook-0.1.0.cdx.json", valid).is_ok());

        for (case, payload, expected_needle) in [
            ("not json", "not-json", "invalid CycloneDX JSON"),
            ("non-object root", "[]", "must be a JSON object"),
            ("wrong bomFormat", r#"{"bomFormat":"SPDX","specVersion":"2.3","components":[{"name":"serde"}]}"#, "bomFormat"),
            ("missing specVersion", r#"{"bomFormat":"CycloneDX","components":[{"name":"serde"}]}"#, "specVersion"),
            ("missing components", r#"{"bomFormat":"CycloneDX","specVersion":"1.5"}"#, "missing `components`"),
            ("empty components", r#"{"bomFormat":"CycloneDX","specVersion":"1.5","components":[]}"#, "empty `components`"),
            (
                "nameless component",
                r#"{"bomFormat":"CycloneDX","specVersion":"1.5","components":[{"type":"library","name":"   "}]}"#,
                "components[0]",
            ),
        ] {
            let err = super::validate_cyclonedx_sbom_json("curvebook-0.1.0.cdx.json", payload).unwrap_err();
            assert!(err.contains(expected_needle), "case `{case}` error `{err}` did not contain `{expected_needle}`");
        }
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

    #[test]
    fn aud_12_17_31_32_47_hardens_duplicates_spoofed_uname_size_caps_deb_members_checked_sum_and_crlf_zsync() {
        // AUD-12-17: exact and ASCII case-folded duplicate entries must be rejected.
        let dup_exact = "-rw-r--r--  0 0 0 10 Nov 23 04:50 doc/README.txt\n-rw-r--r--  0 0 0 10 Nov 23 04:50 doc/README.txt\n";
        assert!(super::validate_verbose_archive_listing("dup.tar.gz", dup_exact, false).unwrap_err().contains("duplicate"));
        let dup_case = "-rw-r--r--  0 0 0 10 Nov 23 04:50 doc/README.txt\n-rw-r--r--  0 0 0 10 Nov 23 04:50 doc/readme.txt\n";
        assert!(super::validate_verbose_archive_listing("case.tar.gz", dup_case, false).unwrap_err().contains("case-colliding"));

        // AUD-12-31: spoofed `uname` containing an ISO date/time must NOT hide `/abs/escaped.txt`.
        let spoofed_uname = "-rw-r--r--  0 2026-01-01 00:00 safe.txt    6 Jan  1  1970 /abs/escaped.txt\n";
        assert!(super::validate_verbose_archive_listing("spoof.tar.gz", spoofed_uname, false).is_err());

        // AUD-12-31: archive member exceeding size cap must be rejected before extraction.
        let bomb_listing = "-rw-r--r-- 0/0 600000000 2026-11-23 04:50 usr/bin/huge\n";
        assert!(super::validate_verbose_archive_listing("bomb.tar.gz", bomb_listing, false).unwrap_err().contains("per-entry cap"));

        // AUD-12-47: checked sum overflows cleanly instead of panicking or wrapping.
        assert!(super::checked_sum_usize([usize::MAX, 1].into_iter(), "checked_assets").is_err());
        assert_eq!(super::checked_sum_usize([3usize, 4usize].into_iter(), "checked_assets").unwrap(), 7);

        // AUD-12-47: CRLF zsync header followed by binary checksums containing `0x0A 0x0A` parses cleanly.
        let mut zsync_crlf = b"zsync: 0.6.2\r\nFilename: curvebook.AppImage\r\n\r\n".to_vec();
        zsync_crlf.extend_from_slice(&[0xff, 0xfe, 0x0a, 0x0a, 0x80, 0x81]);
        let hdr = super::extract_zsync_header_utf8("test.zsync", &zsync_crlf).unwrap();
        assert!(hdr.starts_with("zsync: 0.6.2") && hdr.contains("Filename: curvebook.AppImage"), "{hdr:?}");

        // AUD-12-32: `.deb` with 2 control archives or extra unexpected `ar` member must fail closed.
        #[cfg(unix)]
        {
            fn append_ar_member(out: &mut Vec<u8>, name: &str, body: &[u8]) {
                let mut hdr = format!("{name:<16}0           0     0     100644  {:<10}`\n", body.len()).into_bytes();
                assert_eq!(hdr.len(), 60);
                out.append(&mut hdr);
                out.extend_from_slice(body);
                if body.len() % 2 == 1 {
                    out.push(b'\n');
                }
            }
            let scratch = ScratchDirGuard::new("deb-aud12-32").unwrap();
            let tar_bytes = build_minimal_clean_tar_gz("usr/share/doc/curvebook/README.txt", b"CurveBook\n");
            let ctrl_bytes = build_minimal_clean_tar_gz("control", b"Package: curvebook\n");

            // Two control.tar.* archives must fail closed instead of skipping control scanning.
            let mut two_ctrl = b"!<arch>\n".to_vec();
            append_ar_member(&mut two_ctrl, "debian-binary", b"2.0\n");
            append_ar_member(&mut two_ctrl, "control.tar.gz", &ctrl_bytes);
            append_ar_member(&mut two_ctrl, "control.tar.xz", &ctrl_bytes);
            append_ar_member(&mut two_ctrl, "data.tar.gz", &tar_bytes);
            let deb1 = scratch.path.join("two-ctrl.deb");
            std::fs::write(&deb1, &two_ctrl).unwrap();
            let s1 = scratch.path.join("s1");
            let st1 = scratch.path.join("st1");
            std::fs::create_dir_all(&st1).unwrap();
            let err1 = super::unpack_deb(&ToolConfig::default(), "two-ctrl.deb", &deb1, &s1, &st1).unwrap_err();
            assert!(err1.contains("expected exactly 1 `control.tar*`"), "{err1}");

            // Unexpected extra `ar` member must fail closed.
            let mut extra_ar = b"!<arch>\n".to_vec();
            append_ar_member(&mut extra_ar, "debian-binary", b"2.0\n");
            append_ar_member(&mut extra_ar, "control.tar.gz", &ctrl_bytes);
            append_ar_member(&mut extra_ar, "data.tar.gz", &tar_bytes);
            append_ar_member(&mut extra_ar, "vendor.txt", b"hidden\n");
            let deb2 = scratch.path.join("extra-ar.deb");
            std::fs::write(&deb2, &extra_ar).unwrap();
            let s2 = scratch.path.join("s2");
            let st2 = scratch.path.join("st2");
            std::fs::create_dir_all(&st2).unwrap();
            let err2 = super::unpack_deb(&ToolConfig::default(), "extra-ar.deb", &deb2, &s2, &st2).unwrap_err();
            assert!(err2.contains("unexpected extra `.deb` `ar` member"), "{err2}");

            // AUD-12-17: `data.tar*` containing `_deb_control/control` must NOT be overwritten by `control.tar*`.
            let planted = concat!("Ori", "gin", "Pro");
            let expected_brand = concat!("ori", "gin", "pro");
            let poisoned_data = build_minimal_clean_tar_gz("_deb_control/control", format!("{planted} stealth payload\n").as_bytes());
            let mut colliding_deb = b"!<arch>\n".to_vec();
            append_ar_member(&mut colliding_deb, "debian-binary", b"2.0\n");
            append_ar_member(&mut colliding_deb, "control.tar.gz", &ctrl_bytes);
            append_ar_member(&mut colliding_deb, "data.tar.gz", &poisoned_data);
            let deb3 = scratch.path.join("collide.deb");
            std::fs::write(&deb3, &colliding_deb).unwrap();
            let s3 = scratch.path.join("s3");
            let st3 = scratch.path.join("st3");
            std::fs::create_dir_all(&st3).unwrap();
            super::unpack_deb(&ToolConfig::default(), "collide.deb", &deb3, &s3, &st3).unwrap();
            assert!(st3.join("_deb_data/_deb_control/control").is_file(), "data.tar _deb_control/control must survive unpack");
            assert!(st3.join("_deb_control/control").is_file(), "control.tar control must survive unpack");
            let hits = crate::brands::scan_dist_artefacts(&st3).unwrap();
            assert!(hits.iter().any(|h| h.brand == expected_brand), "poisoned _deb_control/control in data.tar must be scanned: {hits:?}");
        }
    }
}
