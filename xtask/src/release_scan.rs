//! `cargo xtask release-scan`: unpack Beta/1.0 release assets in isolated temporary trees via
//! `curvebook-platform` bounded subprocesses, verify `SHA256SUMS.txt` parity, and run the
//! `brands`, `cleanroom`, and `assets` distribution scanners over every unpacked container (`R0.26`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
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
const MAX_CONTAINER_FILE_BYTES: usize = 1024 * 1024 * 1024;
const MAX_APPIMAGE_RUNTIME_SCAN_BYTES: usize = 32 * 1024 * 1024;
const MAX_MOUNTED_FILE_BYTES: usize = 512 * 1024 * 1024;
const USAGE: &str = "usage: cargo xtask release-scan [--require-checksums | --no-checksums] [--expect-beta-set <version>] [--cross-os-reports <dir>] [--report <json-out>] <assets-dir>";
const CHECKSUMS_FILE: &str = "SHA256SUMS.txt";
const LICENSES_HTML_FILE: &str = "THIRD-PARTY-LICENSES.html";
const SBOM_EXT: &str = ".cdx.json";
const SUBPROCESS_WALL_TIMEOUT: Duration = Duration::from_secs(180);
const SUBPROCESS_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const SUBPROCESS_STREAM_CAP_BYTES: usize = 64 * 1024 * 1024;
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
    Zip,
    TarGz,
    AppImage,
    Dmg,
    Msi,
}

impl AssetKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Checksums => "checksums",
            Self::HtmlCompliance => "html",
            Self::Sbom => "sbom",
            Self::Zip => "zip",
            Self::TarGz => "tar.gz",
            Self::AppImage => "AppImage",
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
        let items: Vec<_> = self
            .assets
            .iter()
            .map(|a| {
                serde_json::json!({
                    "name": a.name, "kind": a.kind, "size_bytes": a.size_bytes, "sha256": a.sha256,
                    "unpacked_files": a.unpacked_files, "nested_archive_entries": a.nested_archive_entries, "checked_assets": a.checked_assets,
                })
            })
            .collect();
        serde_json::json!({
            "ok": self.ok, "host_os": self.host_os, "checksum_verified": self.checksum_verified, "asset_count": self.asset_count,
            "total_unpacked_files": self.total_unpacked_files, "total_nested_archive_entries": self.total_nested_archive_entries,
            "total_checked_assets": self.total_checked_assets, "assets": items,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ToolConfig {
    pub path_env: Option<String>,
    pub tar_bin: String,
    pub bsdtar_bin: String,
    pub unsquashfs_bin: String,
    pub hdiutil_bin: String,
    pub msiexec_bin: String,
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
            unsquashfs_bin: String::from("unsquashfs"),
            hdiutil_bin: String::from("/usr/bin/hdiutil"),
            msiexec_bin: String::from("msiexec"),
            allow_dmg: cfg!(target_os = "macos"),
            allow_msi: cfg!(windows),
        }
    }
}

impl ToolConfig {
    fn from_env() -> Self {
        let mut tools = Self::default();
        if let Some(msiexec) = std::env::var_os("CURVEBOOK_RELEASE_SCAN_MSIEXEC").filter(|v| !v.is_empty()) {
            (tools.msiexec_bin, tools.allow_msi) = (msiexec.to_string_lossy().into_owned(), true);
        }
        tools
    }
}

struct ScratchDirGuard(PathBuf);
impl ScratchDirGuard {
    fn new(label: &str) -> Result<Self, String> {
        let seq = SCRATCH_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("cb-release-scan-{label}-{}-{nanos}-{seq}", std::process::id()));
        std::fs::create_dir_all(&path).map_err(|e| format!("create scratch dir {}: {e}", path.display()))?;
        Ok(Self(path))
    }
}
impl Drop for ScratchDirGuard {
    fn drop(&mut self) {
        make_tree_writable_best_effort(&self.0);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn make_tree_writable_best_effort(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = std::fs::symlink_metadata(dir) else { return };
    if meta.file_type().is_symlink() {
        return;
    }
    let mut perms = meta.permissions();
    perms.set_mode(perms.mode() | if meta.is_dir() { 0o700 } else { 0o600 });
    let _ = std::fs::set_permissions(dir, perms);
    if meta.is_dir()
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        entries.flatten().for_each(|e| make_tree_writable_best_effort(&e.path()));
    }
}
#[cfg(not(unix))]
fn make_tree_writable_best_effort(_dir: &Path) {}

struct DmgMountGuard<'a> {
    tools: &'a ToolConfig,
    mountpoint: PathBuf,
    mounted: bool,
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
    if build_cmd(tools, &tools.hdiutil_bin).args(["detach", mnt.as_ref()]).run().is_ok_and(|o| o.success) {
        return Ok(());
    }
    let forced = build_cmd(tools, &tools.hdiutil_bin)
        .args(["detach", "-force", mnt.as_ref()])
        .run()
        .map_err(|e| format!("hdiutil detach -force {mnt}: {e}"))?;
    if forced.success {
        Ok(())
    } else {
        Err(format!("hdiutil detach -force {mnt} failed ({:?}): {}", forced.status_code, String::from_utf8_lossy(&forced.stderr).trim()))
    }
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    if matches!(args.first().map(String::as_str), Some("-h" | "--help" | "help")) {
        println!("{USAGE}");
        return Ok(());
    }
    let options = parse_args(args)?;
    let report = scan_release_dir_with_tools(root, &options, &ToolConfig::from_env())?;
    if let Some(report_path) = &options.report_path {
        if let Some(parent) = report_path.parent().filter(|p| !p.as_os_str().is_empty()) {
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
    let (mut assets_dir, mut report_path, mut expect_beta_set, mut cross_os_reports_dir) = (None, None, None, None);
    let (mut saw_require, mut saw_no_checksums, mut it) = (false, false, args.iter());
    let set_uniq = |slot: &mut Option<PathBuf>, flag: &str, v: Option<&String>| {
        let val = v.ok_or_else(|| format!("missing value after `{flag}`\n\n{USAGE}"))?;
        if slot.replace(PathBuf::from(val)).is_some() { Err(format!("duplicate `{flag}` argument\n\n{USAGE}")) } else { Ok(()) }
    };
    while let Some(arg) = it.next().map(String::as_str) {
        match arg {
            "--require-checksums" => saw_require = true,
            "--no-checksums" => saw_no_checksums = true,
            "--report" => set_uniq(&mut report_path, "--report", it.next())?,
            "--cross-os-reports" => set_uniq(&mut cross_os_reports_dir, "--cross-os-reports", it.next())?,
            "--expect-beta-set" => {
                let raw = it.next().ok_or_else(|| format!("missing value after `--expect-beta-set`\n\n{USAGE}"))?;
                if expect_beta_set.replace(normalize_release_version(raw)?).is_some() {
                    return Err(format!("duplicate `--expect-beta-set` argument\n\n{USAGE}"));
                }
            }
            other if other.starts_with('-') => return Err(format!("unknown release-scan flag `{other}`\n\n{USAGE}")),
            positional if assets_dir.replace(PathBuf::from(positional)).is_some() => {
                return Err(format!("unexpected extra argument `{positional}`\n\n{USAGE}"));
            }
            _ => {}
        }
    }
    if saw_require && saw_no_checksums {
        return Err(format!("cannot combine `--require-checksums` and `--no-checksums`\n\n{USAGE}"));
    }
    if expect_beta_set.is_some() && saw_no_checksums {
        return Err(format!("`--expect-beta-set` requires `{CHECKSUMS_FILE}` verification (cannot combine with `--no-checksums`)\n\n{USAGE}"));
    }
    Ok(ScanOptions {
        assets_dir: assets_dir.ok_or_else(|| USAGE.to_owned())?,
        checksum_mode: if saw_no_checksums { ChecksumMode::Disabled } else { ChecksumMode::Required },
        report_path,
        expect_beta_set,
        cross_os_reports_dir,
    })
}

pub(crate) fn normalize_release_version(raw: &str) -> Result<String, String> {
    let ver = raw.trim().strip_prefix('v').unwrap_or(raw.trim());
    semver::Version::parse(ver).map_err(|err| format!("invalid release version `{raw}` for `--expect-beta-set`: {err}"))?;
    Ok(ver.to_owned())
}

#[must_use]
pub(crate) fn expected_beta_asset_names(version: &str) -> Vec<String> {
    [
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
    .into()
}

fn verify_expected_beta_set(assets_dir: &Path, version: &str, actual_names: &[String]) -> Result<(), String> {
    let (expected, actual): (BTreeSet<String>, BTreeSet<String>) =
        (expected_beta_asset_names(version).into_iter().collect(), actual_names.iter().cloned().collect());
    let (missing, extra): (Vec<_>, Vec<_>) =
        (expected.difference(&actual).map(String::as_str).collect(), actual.difference(&expected).map(String::as_str).collect());
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

struct CrossOsRecord {
    host_os: String,
    kind: String,
    rec: ScannedAssetRecord,
}

#[derive(Default)]
struct CrossOsReportCatalog(Vec<CrossOsRecord>);

impl CrossOsReportCatalog {
    fn load(dir: &Path) -> Result<Self, String> {
        if !std::fs::symlink_metadata(dir).map_err(|e| format!("stat `--cross-os-reports` dir `{}`: {e}", dir.display()))?.is_dir() {
            return Err(format!("`--cross-os-reports` path `{}` is not a directory", dir.display()));
        }
        let paths: Vec<_> = read_dir_paths_sorted(dir)?.into_iter().filter(|p| p.is_file() && has_ext_ci(p, &["json"])).collect();
        if paths.is_empty() {
            return Err(format!("`--cross-os-reports` directory `{}` contains no `.json` reports", dir.display()));
        }
        let mut records = Vec::new();
        for path in paths {
            parse_cross_os_report_file(&path, &mut records)?;
        }
        Ok(Self(records))
    }

    fn match_native_asset(&self, expected_host_os: &str, item: &DiscoveredAsset) -> Option<ScannedAssetRecord> {
        self.0
            .iter()
            .find(|r| {
                r.host_os == expected_host_os
                    && r.rec.name == item.name
                    && r.kind == item.kind.label()
                    && r.rec.size_bytes == item.size_bytes
                    && r.rec.sha256 == item.sha256
                    && r.rec.unpacked_files > 0
            })
            .map(|r| ScannedAssetRecord { kind: item.kind.label(), ..r.rec.clone() })
    }
}

fn parse_cross_os_report_file(path: &Path, out: &mut Vec<CrossOsRecord>) -> Result<(), String> {
    let raw = String::from_utf8(read_non_symlink_regular_bytes(path, MAX_REPORT_JSON_BYTES, "cross-OS report")?)
        .map_err(|_| format!("cross-OS report `{}` is not valid UTF-8", path.display()))?;
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
    for e in assets {
        let name = e
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("cross-OS report `{}` has asset entry missing `name`", path.display()))?;
        let req_s = |k: &str| {
            e.get(k).and_then(serde_json::Value::as_str).ok_or_else(|| format!("cross-OS report `{}` asset `{name}` missing `{k}`", path.display()))
        };
        let req_u = |k: &str| {
            e.get(k).and_then(serde_json::Value::as_u64).ok_or_else(|| format!("cross-OS report `{}` asset `{name}` missing `{k}`", path.display()))
        };
        let opt_z = |k: &str| usize::try_from(e.get(k).and_then(serde_json::Value::as_u64).unwrap_or(0)).unwrap_or(usize::MAX);
        out.push(CrossOsRecord {
            host_os: host_os.to_owned(),
            kind: req_s("kind")?.to_owned(),
            rec: ScannedAssetRecord {
                name: name.to_owned(),
                kind: "",
                size_bytes: req_u("size_bytes")?,
                sha256: req_s("sha256")?.to_ascii_lowercase(),
                unpacked_files: usize::try_from(req_u("unpacked_files")?).unwrap_or(usize::MAX),
                nested_archive_entries: opt_z("nested_archive_entries"),
                checked_assets: opt_z("checked_assets"),
            },
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
    if !std::fs::symlink_metadata(&options.assets_dir).map_err(|e| format!("stat `{}`: {e}", options.assets_dir.display()))?.is_dir() {
        return Err(format!("release-scan target `{}` is not a directory", options.assets_dir.display()));
    }
    let discovered = discover_release_assets(&options.assets_dir, options.expect_beta_set.as_deref())?;
    let checksum_verified = options.checksum_mode == ChecksumMode::Required;
    if checksum_verified {
        verify_checksum_manifest(&options.assets_dir, &discovered)?;
    }
    let cross_os_reports = options.cross_os_reports_dir.as_deref().map(CrossOsReportCatalog::load).transpose()?;
    let asset_catalog = crate::assets::load_repo_asset_catalog(root)?;
    let workspace_version = std::fs::read_to_string(root.join("Cargo.toml")).ok().and_then(|t| crate::version::read_workspace_version(&t).ok());
    let scratch = ScratchDirGuard::new("batch")?;
    let (mut records, mut violations) = (Vec::with_capacity(discovered.len()), Vec::new());
    let ctx = AssetScanContext {
        asset_catalog: &asset_catalog,
        tools,
        cross_os_reports: cross_os_reports.as_ref(),
        expected_beta_version: options.expect_beta_set.as_deref(),
        workspace_version: workspace_version.as_deref(),
    };
    for (idx, item) in discovered.iter().enumerate() {
        violations.extend(
            crate::brands::scan_dist_label(&item.name, &item.name)
                .into_iter()
                .map(|h| format!("{}:{}: [brands] filename mentions `{}`", h.path, h.line, h.brand)),
        );
        let item_scratch = scratch.0.join(format!("item-{idx}"));
        std::fs::create_dir_all(&item_scratch).map_err(|e| format!("create staging dir for `{}`: {e}", item.name))?;
        records.push(scan_single_asset(&ctx, item, &item_scratch, &mut violations)?);
    }
    if !violations.is_empty() {
        return Err(format!("release-scan found {} violation(s):\n  {}", violations.len(), violations.join("\n  ")));
    }
    Ok(ReleaseScanReport {
        ok: true,
        host_os: std::env::consts::OS.to_owned(),
        checksum_verified,
        asset_count: records.len(),
        total_unpacked_files: checked_sum_usize(records.iter().map(|r| r.unpacked_files), "total_unpacked_files")?,
        total_nested_archive_entries: checked_sum_usize(records.iter().map(|r| r.nested_archive_entries), "total_nested_archive_entries")?,
        total_checked_assets: checked_sum_usize(records.iter().map(|r| r.checked_assets), "total_checked_assets")?,
        assets: records,
    })
}

fn checked_sum_usize(mut iter: impl Iterator<Item = usize>, field: &str) -> Result<usize, String> {
    iter.try_fold(0usize, usize::checked_add).ok_or_else(|| format!("release-scan report integer overflow while summing `{field}`"))
}

fn discover_release_assets(assets_dir: &Path, expect_beta_set: Option<&str>) -> Result<Vec<DiscoveredAsset>, String> {
    let paths = read_dir_paths_sorted(assets_dir)?;
    if paths.is_empty() {
        return Err(format!("release assets directory `{}` is empty", assets_dir.display()));
    }
    let mut raw_entries = Vec::with_capacity(paths.len());
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
        if meta.len() == 0 {
            return Err(format!("`{name}`: release asset is empty (0 bytes)"));
        }
        raw_entries.push((name, path, meta.len()));
    }
    if let Some(beta_ver) = expect_beta_set {
        let names: Vec<String> = raw_entries.iter().map(|(n, _, _)| n.clone()).collect();
        verify_expected_beta_set(assets_dir, beta_ver, &names)?;
    }
    let (mut out, mut non_checksum) = (Vec::with_capacity(raw_entries.len()), 0usize);
    for (name, path, size_bytes) in raw_entries {
        let kind = classify_asset_name(&name)?;
        non_checksum += usize::from(kind != AssetKind::Checksums);
        out.push(DiscoveredAsset { name, sha256: sha256_file_hex(&path)?, path, kind, size_bytes });
    }
    if non_checksum == 0 {
        Err(format!("release assets directory `{}` contains no release packages beyond `{CHECKSUMS_FILE}`", assets_dir.display()))
    } else {
        Ok(out)
    }
}

fn classify_asset_name(name: &str) -> Result<AssetKind, String> {
    if name == CHECKSUMS_FILE {
        return Ok(AssetKind::Checksums);
    }
    if name == LICENSES_HTML_FILE {
        return Ok(AssetKind::HtmlCompliance);
    }
    let lower = name.to_ascii_lowercase();
    [
        (SBOM_EXT, AssetKind::Sbom),
        (".tar.gz", AssetKind::TarGz),
        (".tgz", AssetKind::TarGz),
        (".zip", AssetKind::Zip),
        (".appimage", AssetKind::AppImage),
        (".dmg", AssetKind::Dmg),
        (".msi", AssetKind::Msi),
    ]
    .into_iter()
    .find_map(|(s, k)| lower.ends_with(s).then_some(k))
    .ok_or_else(|| format!("unsupported or unknown release asset format `{name}` (failing closed)"))
}

fn verify_checksum_manifest(assets_dir: &Path, discovered: &[DiscoveredAsset]) -> Result<(), String> {
    let raw_bytes = read_non_symlink_regular_bytes(&assets_dir.join(CHECKSUMS_FILE), MAX_CHECKSUMS_BYTES, CHECKSUMS_FILE)
        .map_err(|e| format!("missing or unreadable `{CHECKSUMS_FILE}` in `{}`: {e}", assets_dir.display()))?;
    let expected =
        parse_sha256sums(&String::from_utf8(raw_bytes).map_err(|_| format!("`{CHECKSUMS_FILE}` in `{}` is not valid UTF-8", assets_dir.display()))?)?;
    let actual: BTreeMap<&str, &str> =
        discovered.iter().filter(|i| i.kind != AssetKind::Checksums).map(|i| (i.name.as_str(), i.sha256.as_str())).collect();
    let mut errors = Vec::new();
    for (&name, &actual_hex) in &actual {
        match expected.get(name) {
            None => errors.push(format!("`{name}` is present on disk but missing from `{CHECKSUMS_FILE}`")),
            Some(exp_hex) if exp_hex != actual_hex => {
                errors.push(format!("`{name}` SHA-256 mismatch: `{CHECKSUMS_FILE}` has {exp_hex}, computed {actual_hex}"))
            }
            Some(_) => {}
        }
    }
    for name in expected.keys().filter(|n| !actual.contains_key(n.as_str())) {
        errors.push(format!("`{CHECKSUMS_FILE}` lists `{name}`, which does not exist in `{}`", assets_dir.display()));
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
        let (line_no, trimmed) = (idx + 1, line.trim_end_matches('\r'));
        if trimmed.trim().is_empty() {
            continue;
        }
        let Some((digest_raw, rest)) = trimmed
            .get(..64)
            .filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()))
            .and_then(|d| trimmed.get(64..).and_then(|r| r.strip_prefix("  ").or_else(|| r.strip_prefix(" *"))).map(|f| (d, f)))
        else {
            return Err(format!("{CHECKSUMS_FILE}:{line_no}: invalid sha256sum line `{trimmed}`"));
        };
        if rest.is_empty() || rest == "." || rest == ".." || rest.contains(['/', '\\', '\0']) {
            return Err(format!("{CHECKSUMS_FILE}:{line_no}: unsafe or nested asset filename `{rest}`"));
        }
        if rest == CHECKSUMS_FILE {
            return Err(format!("{CHECKSUMS_FILE}:{line_no}: `{CHECKSUMS_FILE}` must not list itself"));
        }
        if entries.insert(rest.to_owned(), digest_raw.to_ascii_lowercase()).is_some() {
            return Err(format!("{CHECKSUMS_FILE}:{line_no}: duplicate entry for `{rest}`"));
        }
    }
    if entries.is_empty() { Err(format!("{CHECKSUMS_FILE} contains zero checksum entries")) } else { Ok(entries) }
}

struct AssetScanContext<'a> {
    asset_catalog: &'a crate::assets::RepoAssetCatalog,
    tools: &'a ToolConfig,
    cross_os_reports: Option<&'a CrossOsReportCatalog>,
    expected_beta_version: Option<&'a str>,
    workspace_version: Option<&'a str>,
}

fn scan_single_asset(
    ctx: &AssetScanContext<'_>,
    item: &DiscoveredAsset,
    item_scratch: &Path,
    violations: &mut Vec<String>,
) -> Result<ScannedAssetRecord, String> {
    if matches!(item.kind, AssetKind::Checksums | AssetKind::HtmlCompliance | AssetKind::Sbom) {
        let bytes = read_non_symlink_regular_bytes(&item.path, MAX_COMPLIANCE_TEXT_BYTES, &item.name)?;
        let text = std::str::from_utf8(&bytes).map_err(|e| format!("`{}` is not valid UTF-8: {e}", item.name))?;
        violations.extend(
            crate::brands::scan_compliance_text(&item.name, text).into_iter().map(|h| format!("{}:{}: [brands] `{}`", h.path, h.line, h.brand)),
        );
        violations.extend(
            crate::cleanroom::scan_compliance_bytes(&item.name, &bytes)
                .into_iter()
                .map(|h| format!("{}:{}: [cleanroom] matched `{}`", h.path, h.line, h.signature)),
        );
        if item.kind == AssetKind::Sbom {
            let exp = ctx.expected_beta_version.or_else(|| filename_sbom_version(&item.name)).or(ctx.workspace_version);
            if let Err(err) = validate_cyclonedx_sbom_json(&item.name, text, exp) {
                violations.push(format!("{}: [sbom] {err}", item.name));
            }
            let mut secret_hits = Vec::new();
            crate::secrets::scan_text(&item.name, text, &mut secret_hits);
            violations.extend(secret_hits.into_iter().map(|h| format!("{}:{}: [secrets] {}", h.location, h.line, h.kind)));
        }
        return Ok(ScannedAssetRecord {
            name: item.name.clone(),
            kind: item.kind.label(),
            size_bytes: item.size_bytes,
            sha256: item.sha256.clone(),
            unpacked_files: 1,
            nested_archive_entries: 0,
            checked_assets: 0,
        });
    }
    let cross_os_host = match item.kind {
        AssetKind::Dmg if !ctx.tools.allow_dmg => Some(("macos", "macOS")),
        AssetKind::Msi if !ctx.tools.allow_msi => Some(("windows", "Windows")),
        _ => None,
    };
    if let (Some((host_id, host_label)), Some(reports)) = (cross_os_host, ctx.cross_os_reports) {
        return reports.match_native_asset(host_id, item).ok_or_else(|| {
            format!(
                "`{}`: missing matching verified {host_label} native scan report (`host_os={host_id}`, exact size/sha256, `unpacked_files > 0`)",
                item.name
            )
        });
    }
    let stage_dir = item_scratch.join("unpacked");
    std::fs::create_dir_all(&stage_dir).map_err(|e| format!("create unpacked dir for `{}`: {e}", item.name))?;
    match item.kind {
        AssetKind::Zip => unpack_zip(&item.name, &item.path, &stage_dir)?,
        AssetKind::TarGz => unpack_tarball(ctx.tools, &item.name, &item.path, &stage_dir)?,
        AssetKind::AppImage => unpack_appimage(ctx.tools, &item.name, &item.path, &stage_dir, violations)?,
        AssetKind::Dmg => unpack_dmg(ctx.tools, &item.name, &item.path, item_scratch, &stage_dir, violations)?,
        AssetKind::Msi => unpack_msi(ctx.tools, &item.name, &item.path, &stage_dir)?,
        AssetKind::Checksums | AssetKind::HtmlCompliance | AssetKind::Sbom => {}
    }
    scan_unpacked_tree(ctx.asset_catalog, item, &stage_dir, violations)
}

fn filename_sbom_version(asset_name: &str) -> Option<&str> {
    asset_name.strip_prefix("curvebook-").and_then(|r| r.strip_suffix(SBOM_EXT)).filter(|v| !v.trim().is_empty())
}

fn str_trim<'a>(m: &'a serde_json::Map<String, serde_json::Value>, k: &str) -> Option<&'a str> {
    m.get(k).and_then(serde_json::Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn is_valid_cyclonedx_serial_number(s: &str) -> bool {
    let Some(uuid) = s.strip_prefix("urn:uuid:") else {
        return false;
    };
    let parts: Vec<&str> = uuid.split('-').collect();
    parts.len() == 5 && parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12]) && parts.iter().all(|p| p.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn validate_cyclonedx_sbom_json(asset_name: &str, text: &str, expected_version: Option<&str>) -> Result<(), String> {
    let val: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("`{asset_name}`: invalid CycloneDX JSON: {e}"))?;
    let obj = val.as_object().ok_or_else(|| format!("`{asset_name}`: CycloneDX SBOM root must be a JSON object"))?;
    if obj.get("bomFormat").and_then(serde_json::Value::as_str) != Some("CycloneDX") {
        return Err(format!("`{asset_name}`: expected CycloneDX SBOM with `\"bomFormat\": \"CycloneDX\"`"));
    }
    if str_trim(obj, "specVersion").is_none() {
        return Err(format!("`{asset_name}`: CycloneDX SBOM is missing non-empty `specVersion`"));
    }
    if !str_trim(obj, "serialNumber").is_some_and(is_valid_cyclonedx_serial_number) {
        return Err(format!("`{asset_name}`: CycloneDX SBOM is missing valid `serialNumber` (`urn:uuid:...`) required by `actions/attest`"));
    }
    let components = obj
        .get("components")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("`{asset_name}`: CycloneDX SBOM is missing `components` array"))?;
    if components.is_empty() {
        return Err(format!("`{asset_name}`: CycloneDX SBOM has empty `components` array"));
    }
    // Map each component name to every declared version (`None` when missing/empty) so duplicates fail closed (`R0.36`).
    let mut comp_versions: BTreeMap<&str, Vec<Option<&str>>> = BTreeMap::new();
    for (idx, comp) in components.iter().enumerate() {
        let co = comp.as_object().ok_or_else(|| format!("`{asset_name}`: CycloneDX `components[{idx}]` must be a JSON object"))?;
        let name =
            str_trim(co, "name").ok_or_else(|| format!("`{asset_name}`: CycloneDX `components[{idx}]` is missing a non-empty string `name`"))?;
        comp_versions.entry(name).or_default().push(str_trim(co, "version"));
    }
    let meta_comp = obj
        .get("metadata")
        .and_then(|m| m.get("component"))
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| format!("`{asset_name}`: CycloneDX SBOM is missing `metadata.component` object"))?;
    let root_name = str_trim(meta_comp, "name").unwrap_or("");
    if root_name != "curvebook" {
        return Err(format!("`{asset_name}`: CycloneDX `metadata.component.name` must be `\"curvebook\"`, found `{root_name}`"));
    }
    let root_ver =
        str_trim(meta_comp, "version").ok_or_else(|| format!("`{asset_name}`: CycloneDX `metadata.component.version` is missing or empty"))?;
    semver::Version::parse(root_ver)
        .map_err(|err| format!("`{asset_name}`: CycloneDX `metadata.component.version` `{root_ver}` is not valid SemVer: {err}"))?;
    if let Some(fv) = filename_sbom_version(asset_name)
        && root_ver != fv
    {
        return Err(format!("`{asset_name}`: CycloneDX `metadata.component.version` `{root_ver}` does not match SBOM filename version `{fv}`"));
    }
    if let Some(ev) = expected_version.map(str::trim).filter(|v| !v.is_empty())
        && root_ver != ev
    {
        return Err(format!("`{asset_name}`: CycloneDX `metadata.component.version` `{root_ver}` does not match expected version `{ev}`"));
    }
    // Exactly one versioned `curvebook-cli` entry is accepted (`R0.36`): missing version or duplicate entries fail closed.
    let cli_versions = comp_versions.get("curvebook-cli").map(Vec::as_slice).unwrap_or_default();
    let cli_ver = match cli_versions {
        [] => return Err(format!("`{asset_name}`: CycloneDX SBOM `components` is missing shipped binary component `curvebook-cli`")),
        [single] => *single,
        many => {
            return Err(format!(
                "`{asset_name}`: CycloneDX SBOM `components` lists shipped binary component `curvebook-cli` {} times; expected exactly one",
                many.len()
            ));
        }
    };
    let Some(ver) = cli_ver else {
        return Err(format!("`{asset_name}`: CycloneDX component `curvebook-cli` is missing a non-empty `version`"));
    };
    if ver != root_ver {
        return Err(format!("`{asset_name}`: CycloneDX component `curvebook-cli` version `{ver}` does not match root version `{root_ver}`"));
    }
    Ok(())
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
    violations.extend(brand_hits.into_iter().map(|h| format!("{}!{}:{}: [brands] `{}`", item.name, h.path, h.line, h.brand)));
    violations.extend(cleanroom_hits.into_iter().map(|h| format!("{}!{}:{}: [cleanroom] matched `{}`", item.name, h.path, h.line, h.signature)));
    violations.extend(asset_violations.into_iter().map(|v| format!("{}!{v}", item.name)));
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
    let src = format!("{asset_name}!{rel_norm}");
    violations.extend(crate::brands::scan_dist_label(&src, rel_norm).into_iter().map(|h| format!("{}:{}: [brands] `{}`", h.path, h.line, h.brand)));
    violations.extend(
        crate::brands::scan_dist_label(&src, &target.to_string_lossy())
            .into_iter()
            .map(|h| format!("{}:{}: [brands] symlink target `{}`", h.path, h.line, h.brand)),
    );
    Ok(())
}

fn scan_tree_symlinks(asset_name: &str, root: &Path, dir: &Path, violations: &mut Vec<String>) -> Result<(), String> {
    for path in read_dir_paths_sorted(dir)? {
        let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            scan_symlink_entry(asset_name, &rel_slash(root, &path)?, &path, violations)?;
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
    let (mut seen_folded, mut expected_files) = (BTreeSet::new(), 0usize);
    for (entry_rel, payload) in zip.iter() {
        let safe_rel = validate_safe_archive_member_path(asset_name, entry_rel)?;
        if !seen_folded.insert(safe_rel.to_string_lossy().replace('\\', "/").to_ascii_lowercase()) {
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

fn unpack_tarball(tools: &ToolConfig, asset_name: &str, archive_path: &Path, stage_dir: &Path) -> Result<(), String> {
    if read_prefix_bytes(archive_path, 2).map_err(|e| format!("read header `{asset_name}`: {e}"))?.as_slice() != [0x1f, 0x8b] {
        return Err(format!("`{asset_name}`: expected gzip header `1f 8b`"));
    }
    let (archive_str, stage_str) = (archive_path.to_string_lossy(), stage_dir.to_string_lossy());
    let (mut errors, mut listed) = (Vec::new(), None);
    for cand in [tools.tar_bin.as_str(), tools.bsdtar_bin.as_str()] {
        match build_cmd(tools, cand).args(["--numeric-owner", "-tvzf", archive_str.as_ref()]).run() {
            Ok(out) if out.success => {
                listed = Some((cand, out.stdout));
                break;
            }
            Ok(out) => errors.push(format!("{cand}: exit {:?} ({})", out.status_code, String::from_utf8_lossy(&out.stderr).trim())),
            Err(err) => errors.push(format!("{cand}: {err}")),
        }
    }
    let (chosen_tar, list_out) = listed.ok_or_else(|| format!("`{asset_name}`: cannot list entries via tar/bsdtar ({})", errors.join("; ")))?;
    let regular_files = validate_verbose_archive_listing(asset_name, &String::from_utf8_lossy(&list_out))?;
    run_checked_tool(tools, chosen_tar, &["-xzf", archive_str.as_ref(), "-C", stage_str.as_ref()], asset_name, "extract tarball")?;
    verify_unpacked_regular_file_count(asset_name, stage_dir, regular_files)
}

#[must_use]
pub(crate) fn find_squashfs_offset(bytes: &[u8]) -> Option<usize> {
    let mut search = 0usize;
    while let Some(rel) = bytes.get(search..).and_then(|s| s.windows(4).position(|w| w == b"hsqs")) {
        let off = search + rel;
        if let Some(sb) = bytes.get(off..off + 96) {
            let block_size = u32::from_le_bytes([sb[12], sb[13], sb[14], sb[15]]);
            let (comp_id, block_log) = (u16::from_le_bytes([sb[20], sb[21]]), u16::from_le_bytes([sb[22], sb[23]]));
            let (s_major, s_minor) = (u16::from_le_bytes([sb[28], sb[29]]), u16::from_le_bytes([sb[30], sb[31]]));
            if s_major == 4 && s_minor == 0 && (12..=20).contains(&block_log) && (1u32 << block_log) == block_size && (1..=6).contains(&comp_id) {
                return Some(off);
            }
        }
        search = off + 1;
    }
    None
}

fn unpack_appimage(tools: &ToolConfig, asset_name: &str, appimage_path: &Path, stage_dir: &Path, violations: &mut Vec<String>) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(appimage_path).map_err(|e| format!("stat `{asset_name}`: {e}"))?;
    if meta.file_type().is_symlink() || !meta.file_type().is_file() {
        return Err(format!("`{asset_name}`: must be a regular non-symlink file"));
    }
    let header_bytes =
        read_prefix_bytes(appimage_path, usize::try_from(meta.len()).unwrap_or(MAX_APPIMAGE_RUNTIME_SCAN_BYTES).min(MAX_APPIMAGE_RUNTIME_SCAN_BYTES))
            .map_err(|e| format!("read `{asset_name}` header: {e}"))?;
    let Some(off) = find_squashfs_offset(&header_bytes) else {
        return Err(format!(
            "`{asset_name}`: cannot unpack `.AppImage` (no embedded SquashFS 4.0 (`hsqs`) superblock found; `--appimage-extract` execution is disabled)"
        ));
    };
    if off > 0 {
        violations.extend(
            crate::cleanroom::scan_compliance_bytes(&format!("{asset_name}#runtime"), &header_bytes[..off])
                .into_iter()
                .map(|h| format!("{}:{}: [cleanroom] matched `{}`", h.path, h.line, h.signature)),
        );
    }
    let (dest, off_s) = (stage_dir.join("squashfs-root"), off.to_string());
    let (dest_s, app_s) = (dest.to_string_lossy(), appimage_path.to_string_lossy());
    let mut cmd = build_cmd(tools, &tools.unsquashfs_bin).arg("-no-progress");
    if off > 0 {
        cmd = cmd.args(["-o", off_s.as_str()]);
    }
    match cmd.args(["-d", dest_s.as_ref(), app_s.as_ref()]).run() {
        Ok(out) if out.success => Ok(()),
        Ok(out) => Err(format!(
            "`{asset_name}`: cannot unpack `.AppImage` (unsquashfs: exit {:?}: {}; `--appimage-extract` execution is disabled)",
            out.status_code,
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(err) => Err(format!("`{asset_name}`: cannot unpack `.AppImage` (unsquashfs: {err}; `--appimage-extract` execution is disabled)")),
    }
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
    let (mnt_s, dmg_s) = (mountpoint.to_string_lossy(), dmg_path.to_string_lossy());
    run_checked_tool(
        tools,
        &tools.hdiutil_bin,
        &["attach", "-readonly", "-nobrowse", "-noautoopen", "-mountpoint", mnt_s.as_ref(), dmg_s.as_ref()],
        asset_name,
        "attach `.dmg` read-only via hdiutil",
    )?;
    let mut guard = DmgMountGuard { tools, mountpoint: mountpoint.clone(), mounted: true };
    copy_mounted_tree_no_symlinks(asset_name, &mountpoint, &mountpoint, stage_dir, violations)?;
    guard.mounted = false;
    detach_dmg_mount(tools, &mountpoint)
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

fn strip_windows_verbatim_prefix(path: &Path) -> PathBuf {
    match path.to_str() {
        Some(raw) if raw.starts_with(r"\\?\UNC\") => PathBuf::from(format!(r"\\{}", raw.trim_start_matches(r"\\?\UNC\"))),
        Some(raw) if raw.starts_with(r"\\?\") => PathBuf::from(raw.trim_start_matches(r"\\?\")),
        _ => path.to_path_buf(),
    }
}

pub(crate) fn absolute_tool_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err(String::from("cannot resolve empty tool path"));
    }
    Ok(strip_windows_verbatim_prefix(&std::path::absolute(path).map_err(|e| format!("resolve absolute path `{}`: {e}", path.display()))?))
}

fn unpack_msi(tools: &ToolConfig, asset_name: &str, msi_path: &Path, stage_dir: &Path) -> Result<(), String> {
    if !tools.allow_msi {
        return Err(format!("`{asset_name}`: cannot unpack Windows `.msi` installer on non-Windows host (run `release-scan` on Windows)"));
    }
    let (abs_msi, abs_stage) = (absolute_tool_path(msi_path)?, absolute_tool_path(stage_dir)?);
    let (msi_s, target_arg) = (abs_msi.to_string_lossy(), format!("TARGETDIR={}", abs_stage.to_string_lossy()));
    run_checked_tool(
        tools,
        &tools.msiexec_bin,
        &["/a", msi_s.as_ref(), "/qn", target_arg.as_str()],
        asset_name,
        "extract `.msi` administratively via `msiexec /a`",
    )?;
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
            let detail = if !stderr.is_empty() { stderr } else { String::from_utf8_lossy(&out.stdout).trim().to_owned() };
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
    if meta.len() > u64::try_from(max_bytes).unwrap_or(u64::MAX) {
        return Err(format!("`{label}`: size {} exceeds bounded scan limit {max_bytes} bytes", meta.len()));
    }
    read_regular_file_bytes(path, max_bytes).map_err(|e| format!("read `{label}`: {e}"))
}

fn validate_verbose_archive_listing(asset_name: &str, listing: &str) -> Result<usize, String> {
    let (mut seen_folded, mut total_entries, mut regular_files, mut total_bytes) = (BTreeSet::new(), 0usize, 0usize, 0u64);
    for line in listing.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty()) {
        total_entries += 1;
        if total_entries > MAX_ARCHIVE_ENTRIES {
            return Err(format!("`{asset_name}`: archive entry count exceeds cap {MAX_ARCHIVE_ENTRIES}"));
        }
        let (is_regular, size_bytes, path) = parse_verbose_listing_entry(asset_name, line)?;
        if size_bytes > MAX_ARCHIVE_ENTRY_BYTES {
            return Err(format!("`{asset_name}`: archive member `{path}` size {size_bytes} exceeds per-entry cap {MAX_ARCHIVE_ENTRY_BYTES}"));
        }
        total_bytes = total_bytes
            .checked_add(size_bytes)
            .filter(|&sum| sum <= MAX_ARCHIVE_UNCOMPRESSED_BYTES)
            .ok_or_else(|| format!("`{asset_name}`: archive aggregate uncompressed size exceeds cap {MAX_ARCHIVE_UNCOMPRESSED_BYTES}"))?;
        let norm = validate_safe_archive_member_path(asset_name, path)?.to_string_lossy().replace('\\', "/");
        if !norm.is_empty() && !seen_folded.insert(norm.to_ascii_lowercase()) {
            return Err(format!("`{asset_name}`: duplicate or case-colliding archive member path `{path}`"));
        }
        if is_regular {
            if norm.is_empty() {
                return Err(format!("`{asset_name}`: regular file archive member has empty normalized path `{path}`"));
            }
            regular_files += 1;
        }
    }
    if total_entries == 0 { Err(format!("`{asset_name}`: archive contains zero entries")) } else { Ok(regular_files) }
}

fn parse_verbose_listing_entry<'a>(asset_name: &str, line: &'a str) -> Result<(bool, u64, &'a str), String> {
    let (b, mut tokens, mut i) = (line.as_bytes(), Vec::new(), 0usize);
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let s = i;
        while i < b.len() && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        if s < i
            && let Some(tok) = line.get(s..i)
        {
            tokens.push((s, tok));
        }
    }
    let Some((_, mode)) = tokens.first().copied() else { return Err(format!("`{asset_name}`: empty archive listing entry")) };
    let mb = mode.as_bytes();
    if mb.len() < 10 || !mb[1..10].iter().all(|c| matches!(c, b'r' | b'w' | b'x' | b'-' | b's' | b'S' | b't' | b'T')) {
        return Err(format!("`{asset_name}`: unexpected archive listing format `{line}`"));
    }
    let is_regular = match mb[0] {
        b'-' => true,
        b'd' => false,
        b'l' => return Err(format!("`{asset_name}`: symbolic link archive member rejected: `{line}`")),
        b'h' => return Err(format!("`{asset_name}`: hard link archive member rejected: `{line}`")),
        _ => return Err(format!("`{asset_name}`: special/unsupported archive member rejected: `{line}`")),
    };
    let (sz, path_off) =
        locate_listing_path_anchor(&tokens).ok_or_else(|| format!("`{asset_name}`: cannot locate archive member path in listing `{line}`"))?;
    let rest = line.get(path_off..).ok_or_else(|| format!("`{asset_name}`: invalid path slice in `{line}`"))?;
    if rest.contains(" -> ") {
        return Err(format!("`{asset_name}`: symbolic link archive member rejected: `{line}`"));
    }
    if rest.contains(" link to ") {
        return Err(format!("`{asset_name}`: hard link archive member rejected: `{line}`"));
    }
    Ok((is_regular, sz, rest))
}

fn locate_listing_path_anchor(tokens: &[(usize, &str)]) -> Option<(u64, usize)> {
    const MONTHS: &str = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec";
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let parse_sz = |t: &str| digits(t).then(|| t.parse::<u64>().ok()).flatten();
    let is_iso = |t: &str| {
        let b = t.as_bytes();
        b.len() == 10 && b[4] == b'-' && b[7] == b'-' && b[..4].iter().chain(&b[5..7]).chain(&b[8..10]).all(u8::is_ascii_digit)
    };
    let is_ty = |t: &str| {
        let b = t.as_bytes();
        (b.len() == 4 && digits(t))
            || (b.len() == 5 && b[2] == b':' && b[..2].iter().chain(&b[3..5]).all(u8::is_ascii_digit))
            || (b.len() == 8 && b[2] == b':' && b[5] == b':' && b[..2].iter().chain(&b[3..5]).chain(&b[6..8]).all(u8::is_ascii_digit))
    };
    if tokens.len() >= 6
        && tokens[1].1.contains('/')
        && let Some(sz) = parse_sz(tokens[2].1)
        && is_iso(tokens[3].1)
        && is_ty(tokens[4].1)
    {
        return Some((sz, tokens[5].0));
    }
    parse_sz(tokens.get(1)?.1)?;
    for idx in 5..tokens.len() {
        let Some(sz) = parse_sz(tokens[idx - 1].1) else { continue };
        if idx + 2 < tokens.len() && is_iso(tokens[idx].1) && is_ty(tokens[idx + 1].1) {
            return Some((sz, tokens[idx + 2].0));
        }
        if idx + 3 < tokens.len()
            && MONTHS.split_ascii_whitespace().any(|m| m == tokens[idx].1)
            && digits(tokens[idx + 1].1)
            && is_ty(tokens[idx + 2].1)
        {
            return Some((sz, tokens[idx + 3].0));
        }
    }
    None
}

fn validate_safe_archive_member_path(asset_name: &str, raw_entry: &str) -> Result<PathBuf, String> {
    if raw_entry.contains(['\0', '\\']) || raw_entry.starts_with('/') {
        return Err(format!("`{asset_name}`: unsafe archive member path `{raw_entry}`"));
    }
    let trimmed = raw_entry.strip_prefix("./").unwrap_or(raw_entry).trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "." {
        return Ok(PathBuf::new());
    }
    let mut out = PathBuf::new();
    for (idx, comp) in Path::new(trimmed).components().enumerate() {
        match comp {
            Component::Normal(seg) => {
                let s = seg.to_string_lossy();
                if s == ".." || (idx == 0 && s.contains(':')) {
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
    let mut buf = Vec::with_capacity(count);
    std::fs::File::open(path)
        .map_err(|e| format!("open `{}`: {e}", path.display()))?
        .take(count as u64)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read `{}`: {e}", path.display()))?;
    Ok(buf)
}

#[must_use]
pub(crate) fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    curvebook_platform::sha256::sha256_bytes(bytes)
}

fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let (mut file, mut hasher, mut buf) = (
        std::fs::File::open(path).map_err(|e| format!("open `{}`: {e}", path.display()))?,
        curvebook_platform::sha256::Sha256::new(),
        vec![0u8; 64 * 1024],
    );
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("read `{}`: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let mut out = String::with_capacity(64);
    for b in hasher.finalize() {
        let _ = write!(out, "{b:02x}");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan_util::test_support::TestDir;
    use curvebook_platform::sha256::sha256_hex;
    use std::path::{Path, PathBuf};

    fn build_squashfs_v4_superblock(block_size: u32, block_log: u16, comp_id: u16) -> [u8; 96] {
        let mut sb = [0u8; 96];
        sb[..4].copy_from_slice(b"hsqs");
        sb[12..16].copy_from_slice(&block_size.to_le_bytes());
        sb[20..22].copy_from_slice(&comp_id.to_le_bytes());
        sb[22..24].copy_from_slice(&block_log.to_le_bytes());
        sb[28..30].copy_from_slice(&4u16.to_le_bytes());
        sb
    }

    #[cfg(unix)]
    fn write_exec_script(dir: &TestDir, rel: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.write(rel, body);
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[cfg(unix)]
    fn write_checksums(dir: &Path) {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n != CHECKSUMS_FILE).collect();
        names.sort();
        let body: String = names.iter().map(|n| format!("{}  {n}\n", sha256_hex(&std::fs::read(dir.join(n)).unwrap()))).collect();
        std::fs::write(dir.join(CHECKSUMS_FILE), body).unwrap();
    }

    #[cfg(unix)]
    fn build_minimal_clean_tar_gz(entry_path: &str, data: &[u8]) -> Vec<u8> {
        let (mut tar, mut hdr) = (Vec::new(), [0u8; 512]);
        hdr[..entry_path.len()].copy_from_slice(entry_path.as_bytes());
        for (range, val) in [
            (100..108, &b"0000644\0"[..]),
            (108..116, b"0000000\0"),
            (116..124, b"0000000\0"),
            (136..148, b"00000000000\0"),
            (257..265, b"ustar\x0000"),
        ] {
            hdr[range].copy_from_slice(val);
        }
        hdr[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        hdr[148..156].fill(b' ');
        hdr[156] = b'0';
        let chksum: u32 = hdr.iter().map(|&b| u32::from(b)).sum();
        hdr[148..156].copy_from_slice(format!("{chksum:06o}\0 ").as_bytes());
        tar.extend_from_slice(&hdr);
        tar.extend_from_slice(data);
        tar.resize(tar.len() + ((512 - (data.len() % 512)) % 512) + 1024, 0);
        let (mut gz, len) = (vec![0x1f, 0x8b, 0x08, 0, 0, 0, 0, 0, 0, 3, 1], tar.len() as u16);
        for chunk in [&len.to_le_bytes()[..], &(!len).to_le_bytes(), &tar, &crc32fast::hash(&tar).to_le_bytes(), &(tar.len() as u32).to_le_bytes()] {
            gz.extend_from_slice(chunk);
        }
        gz
    }

    #[test]
    fn cli_paths_sha256_squashfs_formats_sbom_and_listing_guards() {
        assert_eq!(
            (sha256_hex(b""), sha256_hex(b"abc")),
            (
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()
            )
        );
        let mut img = vec![0x7f, b'E', b'L', b'F'];
        img.resize(256, 0);
        img.extend_from_slice(b"hsqs_decoy_invalid_superblock___________________________________");
        img.resize(4096, 0);
        let off = img.len();
        img.extend_from_slice(&build_squashfs_v4_superblock(131_072, 17, 1));
        assert_eq!((find_squashfs_offset(&img), find_squashfs_offset(b"plain")), (Some(off), None));

        let beta = parse_args(&["--expect-beta-set", "v0.1.0-beta.1", "--cross-os-reports", "dist/r", "dist/rel"].map(String::from)).unwrap();
        assert_eq!((beta.expect_beta_set.as_deref(), beta.checksum_mode), (Some("0.1.0-beta.1"), ChecksumMode::Required));
        for bad in [
            &[][..],
            &["--require-checksums", "--no-checksums", "d"],
            &["--expect-beta-set", "0.1.0", "--no-checksums", "d"],
            &["--expect-beta-set", "../evil", "d"],
            &["--unknown", "d"],
            &["--report"],
            &["a", "b"],
        ] {
            assert!(parse_args(&bad.iter().map(|&s| s.to_owned()).collect::<Vec<_>>()).is_err());
        }
        for r in ["a.deb", "a.rpm", "a.flatpak", "a.AppImage.zsync", "a.exe"] {
            assert!(classify_asset_name(r).is_err(), "{r}");
        }
        for m in ["../escape.txt", "/etc/passwd", "C:\\win.ini", "dir\\file.txt"] {
            assert!(validate_safe_archive_member_path("p.tar.gz", m).is_err());
        }
        assert_eq!(validate_safe_archive_member_path("p.tar.gz", "./usr/bin/curvebook").unwrap(), PathBuf::from("usr/bin/curvebook"));
        let h64 = "a".repeat(64);
        assert_eq!(parse_sha256sums(&format!("{h64}  a.zip\n")).unwrap().len(), 1);
        for bad in ["", "bad  a.zip\n", &format!("{h64}  ../x.zip\n"), &format!("{h64}  SHA256SUMS.txt\n")] {
            assert!(parse_sha256sums(bad).is_err());
        }
        assert_eq!(validate_verbose_archive_listing("ok.tar.gz", "drwxr-xr-x 0 u g 0 Nov 23 04:50 ./usr/\n-rw-r--r-- 0 u g 12 Nov 23 04:50 ./usr/Read Me.txt\n-rw-r--r-- 0/0 45 2026-11-23 04:50 usr/bin/cb\n").unwrap(), 2);
        for bad in [
            "lrwxr-xr-x 0 0 0 0 Nov 23 04:50 sym -> /etc/passwd",
            "hrw-r--r-- 0 0 0 0 Nov 23 04:50 hard link to /etc/passwd",
            "-rw-r--r-- 0 0 0 7 Nov 23 04:50 ../escape.txt",
            "-rw-r--r-- 0 2026-01-01 00:00 safe 6 Jan 1 1970 /abs/escaped.txt",
            "-rw-r--r-- 0/0 600000000 2026-11-23 04:50 usr/bin/huge",
            "-rw-r--r-- 0 0 0 10 Nov 23 04:50 doc/A.txt\n-rw-r--r-- 0 0 0 10 Nov 23 04:50 doc/a.txt",
        ] {
            assert!(validate_verbose_archive_listing("bad.tar.gz", bad).is_err(), "{bad}");
        }
        assert!(checked_sum_usize([usize::MAX, 1].into_iter(), "x").is_err() && absolute_tool_path(Path::new("")).is_err());
        assert_eq!(
            (strip_windows_verbatim_prefix(Path::new(r"\\?\C:\a.msi")), strip_windows_verbatim_prefix(Path::new(r"\\?\UNC\s\sh\a.msi"))),
            (PathBuf::from(r"C:\a.msi"), PathBuf::from(r"\\s\sh\a.msi"))
        );

        let sbom = |fmt: &str, name: &str, ver: &str, cli_n: &str, cli_v: &str| {
            format!(
                r#"{{"bomFormat":"{fmt}","specVersion":"1.5","serialNumber":"urn:uuid:01234567-89ab-5cde-8f01-23456789abcd","metadata":{{"component":{{"name":"{name}","version":"{ver}"}}}},"components":[{{"name":"{cli_n}","version":"{cli_v}"}}]}}"#
            )
        };
        assert!(
            validate_cyclonedx_sbom_json(
                "curvebook-0.1.0.cdx.json",
                &sbom("CycloneDX", "curvebook", "0.1.0", "curvebook-cli", "0.1.0"),
                Some("0.1.0")
            )
            .is_ok()
        );
        for (payload, ev, needle) in [
            ("not-json".into(), Some("0.1.0"), "invalid CycloneDX JSON"),
            ("[]".into(), Some("0.1.0"), "must be a JSON object"),
            (sbom("SPDX", "curvebook", "0.1.0", "curvebook-cli", "0.1.0"), Some("0.1.0"), "bomFormat"),
            (r#"{"bomFormat":"CycloneDX","components":[{"name":"serde"}]}"#.into(), Some("0.1.0"), "specVersion"),
            (r#"{"bomFormat":"CycloneDX","specVersion":"1.5","components":[{"name":"serde"}]}"#.into(), Some("0.1.0"), "serialNumber"),
            (r#"{"bomFormat":"CycloneDX","specVersion":"1.5","serialNumber":"not-a-uuid","components":[{"name":"serde"}]}"#.into(), Some("0.1.0"), "serialNumber"),
            (r#"{"bomFormat":"CycloneDX","specVersion":"1.5","serialNumber":"urn:uuid:01234567-89ab-5cde-8f01-23456789abcd","components":[]}"#.into(), Some("0.1.0"), "empty `components`"),
            (r#"{"bomFormat":"CycloneDX","specVersion":"1.5","serialNumber":"urn:uuid:01234567-89ab-5cde-8f01-23456789abcd","components":[{"name":"curvebook-cli"}]}"#.into(), Some("0.1.0"), "metadata.component"),
            (sbom("CycloneDX", "other", "0.1.0", "curvebook-cli", "0.1.0"), Some("0.1.0"), "metadata.component.name"),
            (sbom("CycloneDX", "curvebook", "bad", "curvebook-cli", "bad"), None, "not valid SemVer"),
            (sbom("CycloneDX", "curvebook", "0.2.0", "curvebook-cli", "0.2.0"), None, "SBOM filename version"),
            (sbom("CycloneDX", "curvebook", "0.1.0", "curvebook-cli", "0.1.0"), Some("0.9.0"), "expected version"),
            (sbom("CycloneDX", "curvebook", "0.1.0", "serde", "0.1.0"), Some("0.1.0"), "curvebook-cli"),
            (sbom("CycloneDX", "curvebook", "0.1.0", "curvebook-cli", "0.2.0"), Some("0.1.0"), "curvebook-cli"),
        ] {
            assert!(validate_cyclonedx_sbom_json("curvebook-0.1.0.cdx.json", &payload, ev).unwrap_err().contains(needle), "{needle}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn tool_dispatch_and_expect_beta_set_verify_containers_detach_dmg_and_cross_os_reports() {
        let (root, dir) = (crate::workspace_root().unwrap(), TestDir::new("relscan-dispatch"));
        let (detach_log, planted) = (dir.join("hdiutil-detach.log"), concat!("Ori", "gin", "Pro"));
        let unsquashfs = write_exec_script(
            &dir,
            "mock-bin/unsquashfs",
            &format!(
                "#!/bin/sh\nset -eu\noff=\"\"; dest=\"\"; src=\"\"\nwhile [ $# -gt 0 ]; do case \"$1\" in -no-progress) shift ;; -o) off=\"$2\"; shift 2 ;; -d) dest=\"$2\"; shift 2 ;; *) src=\"$1\"; shift ;; esac; done\n[ \"$off\" = \"512\" ] || exit 2\nmkdir -p \"$dest/usr/share/doc/curvebook\"\ncase \"$src\" in *dirty*) printf '{planted}\\n' > \"$dest/usr/share/doc/curvebook/README.txt\" ;; *) printf 'CurveBook clean\\n' > \"$dest/usr/share/doc/curvebook/README.txt\" ;; esac\n"
            ),
        );
        let hdiutil = write_exec_script(
            &dir,
            "mock-bin/hdiutil",
            &format!(
                "#!/bin/sh\nset -eu\ncase \"$1\" in attach) mkdir -p \"$6/CurveBook.app/Contents/Resources\"; ln -s /Applications \"$6/Applications\"; cp \"$7\" \"$6/CurveBook.app/Contents/Resources/README.txt\" ;; detach) printf 'detached\\n' >> \"{}\" ;; *) exit 2 ;; esac\n",
                detach_log.display()
            ),
        );
        let msiexec = write_exec_script(
            &dir,
            "mock-bin/msiexec",
            "#!/bin/sh\nset -eu\n[ \"$1\" = \"/a\" ] && [ \"$3\" = \"/qn\" ]\ntarget=\"${4#TARGETDIR=}\"\ncp \"$2\" \"$target/$(basename \"$2\")\"\nmkdir -p \"$target/PFiles/CurveBook\"\ncp \"$2\" \"$target/PFiles/CurveBook/README.txt\"\n",
        );
        let tools = ToolConfig {
            unsquashfs_bin: unsquashfs.to_string_lossy().into_owned(),
            hdiutil_bin: hdiutil.to_string_lossy().into_owned(),
            msiexec_bin: msiexec.to_string_lossy().into_owned(),
            allow_dmg: true,
            allow_msi: true,
            ..ToolConfig::default()
        };
        let mut app_bytes = vec![0x7f, b'E', b'L', b'F'];
        app_bytes.resize(512, 0);
        app_bytes.extend_from_slice(&build_squashfs_v4_superblock(131_072, 17, 1));
        let mk_opts = |assets_dir: PathBuf| ScanOptions {
            assets_dir,
            checksum_mode: ChecksumMode::Required,
            report_path: None,
            expect_beta_set: None,
            cross_os_reports_dir: None,
        };

        dir.write_bytes("clean/cb.AppImage", &app_bytes);
        dir.write("clean/cb.dmg", "CurveBook DMG\n");
        dir.write("clean/cb.msi", "CurveBook MSI\n");
        write_checksums(&dir.join("clean"));
        assert_eq!(
            (
                scan_release_dir_with_tools(&root, &mk_opts(dir.join("clean")), &tools).unwrap().asset_count,
                std::fs::read_to_string(&detach_log).unwrap().lines().count()
            ),
            (4, 1)
        );

        dir.write_bytes("dirty/cb-dirty.AppImage", &app_bytes);
        dir.write("dirty/cb-dirty.dmg", &format!("{planted}\n"));
        dir.write("dirty/cb-dirty.msi", &format!("{planted}\n"));
        write_checksums(&dir.join("dirty"));
        let err = scan_release_dir_with_tools(&root, &mk_opts(dir.join("dirty")), &tools).unwrap_err();
        assert!(
            err.contains("cb-dirty.AppImage!")
                && err.contains("cb-dirty.dmg!")
                && err.contains("cb-dirty.msi!")
                && std::fs::read_to_string(&detach_log).unwrap().lines().count() == 2
        );

        let (ver, dmg_b, x64_b, arm_b) = ("0.1.0-beta.1", b"DMG\n", b"MSIx64\n", b"MSIarm\n");
        let tgz = build_minimal_clean_tar_gz("cb/README.txt", b"CurveBook FreeBSD\n");
        let sbom = serde_json::to_vec(&serde_json::json!({"bomFormat":"CycloneDX","specVersion":"1.5","serialNumber":"urn:uuid:01234567-89ab-5cde-8f01-23456789abcd","metadata":{"component":{"name":"curvebook","version":ver}},"components":[{"name":"curvebook-cli","version":ver},{"name":"serde","version":"1.0.219"}]})).unwrap();
        let (dmg_n, x64_n, arm_n) = (
            format!("curvebook-{ver}-macos-universal.dmg"),
            format!("curvebook-{ver}-windows-x64.msi"),
            format!("curvebook-{ver}-windows-arm64.msi"),
        );
        for (name, bytes) in [
            (dmg_n.as_str(), &dmg_b[..]),
            (x64_n.as_str(), &x64_b[..]),
            (arm_n.as_str(), &arm_b[..]),
            (&format!("curvebook-{ver}-linux-x86_64.AppImage"), &app_bytes[..]),
            (&format!("curvebook-{ver}-linux-aarch64.AppImage"), &app_bytes[..]),
            (&format!("curvebook-{ver}-freebsd-x86_64.tar.gz"), &tgz[..]),
            (&format!("curvebook-{ver}.cdx.json"), &sbom[..]),
            (LICENSES_HTML_FILE, b"<html><title>CurveBook</title></html>\n"),
        ] {
            dir.write_bytes(&format!("beta/{name}"), bytes);
        }
        write_checksums(&dir.join("beta"));
        let mk_rep = |host: &str, items: &[(&str, &str, &[u8])]| {
            let a: Vec<_> = items
                .iter()
                .map(|&(n, k, b)| serde_json::json!({"name":n,"kind":k,"size_bytes":b.len(),"sha256":sha256_hex(b),"unpacked_files":4}))
                .collect();
            serde_json::to_vec(&serde_json::json!({"ok":true,"host_os":host,"assets":a})).unwrap()
        };
        dir.write_bytes("reports/mac.json", &mk_rep("macos", &[(&dmg_n, "dmg", dmg_b)]));
        dir.write_bytes("reports/win.json", &mk_rep("windows", &[(&x64_n, "msi", x64_b), (&arm_n, "msi", arm_b)]));
        let linux_tools =
            ToolConfig { unsquashfs_bin: unsquashfs.to_string_lossy().into_owned(), allow_dmg: false, allow_msi: false, ..ToolConfig::default() };
        let opts = ScanOptions {
            assets_dir: dir.join("beta"),
            checksum_mode: ChecksumMode::Required,
            report_path: None,
            expect_beta_set: Some(ver.to_owned()),
            cross_os_reports_dir: Some(dir.join("reports")),
        };
        assert_eq!(scan_release_dir_with_tools(&root, &opts, &linux_tools).unwrap().asset_count, 9);
        assert!(scan_release_dir_with_tools(&root, &ScanOptions { cross_os_reports_dir: None, ..opts.clone() }, &linux_tools).is_err());
        dir.write_bytes("reports/mac.json", &mk_rep("linux", &[(&dmg_n, "dmg", dmg_b)]));
        assert!(scan_release_dir_with_tools(&root, &opts, &linux_tools).unwrap_err().contains("host_os=macos"));
    }
}
