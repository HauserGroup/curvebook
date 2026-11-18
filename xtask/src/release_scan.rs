//! `cargo xtask release-scan`: unpack release assets in isolated temporary trees via
//! `curvebook-platform` bounded subprocesses, verify `SHA256SUMS.txt` parity, and run the
//! `brands`, `cleanroom`, and `assets` distribution scanners over every unpacked container (`R0.26`).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use curvebook_format::{ZipArchive, ZipLimits};
use curvebook_platform::process::ProcessCommand;
use curvebook_platform::{PlatformError, write_atomic};

const USAGE: &str = "\
usage: cargo xtask release-scan [--require-checksums | --no-checksums] [--report <json-out>] <assets-dir>

Unpacks every release container in <assets-dir> (.zip, .tar.gz/.tgz, .deb, .rpm, .AppImage,
.flatpak, .dmg on macOS, .msi on Windows) plus plain compliance files (SHA256SUMS.txt,
THIRD-PARTY-LICENSES.html, *.zsync), verifies SHA-256 digests against SHA256SUMS.txt by default,
fails closed on unknown/unextractable formats, and runs brands + cleanroom + assets checks.";

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
            allow_appimage_exec: cfg!(target_os = "linux"),
            allow_dmg: cfg!(target_os = "macos"),
            allow_msi: cfg!(windows),
        }
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
    let tools = ToolConfig::default();
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
                let Some(val) = args.get(idx + 1) else {
                    return Err(format!("missing path after `--report`\n\n{USAGE}"));
                };
                if report_path.is_some() {
                    return Err(format!("duplicate `--report` argument\n\n{USAGE}"));
                }
                report_path = Some(PathBuf::from(val));
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
    let Some(assets_dir) = assets_dir else {
        return Err(USAGE.to_owned());
    };
    let checksum_mode = if saw_no_checksums { ChecksumMode::Disabled } else { ChecksumMode::Required };
    Ok(ScanOptions { assets_dir, checksum_mode, report_path })
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
    let checksum_verified = options.checksum_mode == ChecksumMode::Required;
    if checksum_verified {
        verify_checksum_manifest(&options.assets_dir, &discovered)?;
    }

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
        let outcome = scan_single_asset(&asset_catalog, tools, item, &item_scratch, &mut violations)?;
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
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("missing or unreadable `{CHECKSUMS_FILE}` in `{}`: {e}", assets_dir.display()))?;
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
    item: &DiscoveredAsset,
    item_scratch: &Path,
    violations: &mut Vec<String>,
) -> Result<ScannedAssetRecord, String> {
    match item.kind {
        AssetKind::Checksums | AssetKind::HtmlCompliance => {
            let bytes = std::fs::read(&item.path).map_err(|e| format!("read `{}`: {e}", item.name))?;
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
            let bytes = std::fs::read(&item.path).map_err(|e| format!("read `{}`: {e}", item.name))?;
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
    let bytes = std::fs::read(path).map_err(|e| format!("read `{asset_name}`: {e}"))?;
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
    let list_flag = if require_gzip_magic { "-tzf" } else { "-tf" };
    let extract_flag = if require_gzip_magic { "-xzf" } else { "-xf" };

    let (chosen_tar, list_out) = run_tar_or_bsdtar(tools, &[list_flag, archive_str.as_ref()], asset_name, "list entries")?;
    let listing = String::from_utf8_lossy(&list_out);
    validate_archive_listing(asset_name, &listing)?;

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

    match build_cmd(tools, &tools.ar_bin).current_dir(ar_dir).args(["-x", deb_str.as_ref()]).run() {
        Ok(out) if out.success && dir_has_deb_data_tar(ar_dir) => return Ok(()),
        Ok(out) => errors.push(format!("{}: exit {:?} ({})", tools.ar_bin, out.status_code, String::from_utf8_lossy(&out.stderr).trim())),
        Err(err) => errors.push(format!("{}: {err}", tools.ar_bin)),
    }

    // Fallback for GNU/SVR4 `.deb` archives whose 16-byte member names carry trailing `/`
    // (`data.tar.gz/    `), which macOS BSD `/usr/bin/ar -x` rejects while `bsdtar -xf` unpacks cleanly.
    let ar_dir_str = ar_dir.to_string_lossy();
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

    match build_cmd(tools, &tools.bsdtar_bin).args(["-tf", rpm_str.as_ref()]).run() {
        Ok(list_out) if list_out.success => {
            validate_archive_listing(asset_name, &String::from_utf8_lossy(&list_out.stdout))?;
            let ext_out = build_cmd(tools, &tools.bsdtar_bin)
                .args(["-xf", rpm_str.as_ref(), "-C", stage_str.as_ref()])
                .run()
                .map_err(|e| format!("`{asset_name}`: bsdtar extract failed: {e}"))?;
            if ext_out.success {
                return Ok(());
            }
            errors.push(format!("bsdtar -xf: exit {:?} ({})", ext_out.status_code, String::from_utf8_lossy(&ext_out.stderr).trim()));
        }
        Ok(list_out) => errors.push(format!("bsdtar -tf: exit {:?} ({})", list_out.status_code, String::from_utf8_lossy(&list_out.stderr).trim())),
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
        .arg("-it")
        .stdin_bytes(cpio_stream.clone())
        .run()
        .map_err(|e| format!("`{asset_name}`: cpio -it failed: {e}"))?;
    if !list_cpio.success {
        return Err(format!("`{asset_name}`: cpio -it failed: {}", String::from_utf8_lossy(&list_cpio.stderr).trim()));
    }
    validate_archive_listing(asset_name, &String::from_utf8_lossy(&list_cpio.stdout))?;

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
    let bytes = std::fs::read(appimage_path).map_err(|e| format!("read `{asset_name}`: {e}"))?;
    let sqfs_offset = find_squashfs_offset(&bytes);
    if let Some(off) = sqfs_offset
        && off > 0
    {
        for hit in crate::cleanroom::scan_compliance_bytes(&format!("{asset_name}#runtime"), &bytes[..off]) {
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

    if tools.allow_appimage_exec && bytes.starts_with(b"\x7fELF") {
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
        errors.push(String::from("`--appimage-extract` execution disabled on non-Linux host; install `unsquashfs` or scan on Linux"));
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
            let bytes = std::fs::read(&path).map_err(|e| format!("read mounted `{asset_name}!{rel_norm}`: {e}"))?;
            write_atomic(&dst, &bytes).map_err(|e| format!("stage mounted `{asset_name}!{rel_norm}`: {e}"))?;
        }
    }
    Ok(())
}

fn unpack_msi(tools: &ToolConfig, asset_name: &str, msi_path: &Path, stage_dir: &Path) -> Result<(), String> {
    if !tools.allow_msi {
        return Err(format!("`{asset_name}`: cannot unpack Windows `.msi` installer on non-Windows host (run `release-scan` on Windows)"));
    }
    let msi_str = msi_path.to_string_lossy();
    let target_arg = format!("TARGETDIR={}", stage_dir.to_string_lossy());
    run_checked_tool(
        tools,
        &tools.msiexec_bin,
        &["/a", msi_str.as_ref(), "/qn", target_arg.as_str()],
        asset_name,
        "extract `.msi` administratively via `msiexec /a`",
    )?;

    // `msiexec /a` places a stripped administrative copy of `<name>.msi` at the root of `TARGETDIR`
    // alongside the extracted directory hierarchy; remove that administrative stub before counting payload files.
    let admin_stub = stage_dir.join(asset_name);
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
        .wall_timeout(SUBPROCESS_WALL_TIMEOUT)
        .idle_timeout(SUBPROCESS_IDLE_TIMEOUT)
        .max_output_bytes(SUBPROCESS_STREAM_CAP_BYTES);
    if let Some(path_val) = &tools.path_env {
        cmd = cmd.env("PATH", path_val);
    }
    cmd
}

fn validate_archive_listing(asset_name: &str, listing: &str) -> Result<(), String> {
    let mut saw_entry = false;
    for raw_line in listing.lines() {
        let entry = raw_line.trim_end_matches('\r');
        if entry.is_empty() {
            continue;
        }
        saw_entry = true;
        let _ = validate_safe_archive_member_path(asset_name, entry)?;
    }
    if !saw_entry {
        return Err(format!("`{asset_name}`: archive contains zero entries"));
    }
    Ok(())
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

fn read_prefix_bytes(path: &Path, count: usize) -> Result<Vec<u8>, std::io::Error> {
    let file = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(count);
    file.take(count as u64).read_to_end(&mut buf)?;
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

        let no_cs = parse_args(&[String::from("--no-checksums"), String::from("--report"), String::from("out.json"), String::from("dist")]).unwrap();
        assert_eq!(no_cs.checksum_mode, ChecksumMode::Disabled);
        assert_eq!(no_cs.report_path, Some(PathBuf::from("out.json")));

        for bad in [vec![], vec!["--require-checksums", "--no-checksums", "dist"], vec!["--unknown", "dist"], vec!["--report"], vec!["a", "b"]] {
            let owned: Vec<String> = bad.into_iter().map(String::from).collect();
            assert!(parse_args(&owned).is_err());
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

        // 4. Mock `msiexec`: verifies `/a <msi> /qn TARGETDIR=<dir>` and also drops administrative `<name>.msi` stub.
        let msiexec = write_exec_script(
            &bin_dir,
            "msiexec",
            "#!/bin/sh\nset -eu\n[ \"$1\" = \"/a\" ] && [ \"$3\" = \"/qn\" ]\nmsi=\"$2\"\ntarget=\"${4#TARGETDIR=}\"\ncp \"$msi\" \"$target/$(basename \"$msi\")\"\nmkdir -p \"$target/PFiles/CurveBook\"\ncp \"$msi\" \"$target/PFiles/CurveBook/README.txt\"\n",
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

        let clean_opts = ScanOptions { assets_dir: clean_dir, checksum_mode: ChecksumMode::Required, report_path: None };
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

        let dirty_opts = ScanOptions { assets_dir: dirty_dir, checksum_mode: ChecksumMode::Required, report_path: None };
        let err = scan_release_dir_with_tools(&root, &dirty_opts, &tools).unwrap_err();
        assert!(err.contains("curvebook-dirty.AppImage!"), "{err}");
        assert!(err.contains("curvebook-dirty.flatpak!"), "{err}");
        assert!(err.contains("curvebook-dirty.dmg!"), "{err}");
        assert!(err.contains("curvebook-dirty.msi!"), "{err}");
        assert!(err.contains("curvebook-dirty.rpm!"), "{err}");
        assert_eq!(std::fs::read_to_string(&detach_log).unwrap().lines().count(), 2, "DMG must detach even when scan fails");
    }
}
