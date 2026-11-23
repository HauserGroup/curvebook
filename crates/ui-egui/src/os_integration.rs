//! Native OS clipboard multi-flavor bundles (`PDF`, `SVG`, `HTML`, `PNG`, `EMF`) and host
//! pasteboard / print-spooler integration (`docs/evidence/platforms.md` `G10`, `G11`, `G13`).

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use curvebook_chart::ChartScene;
#[cfg(not(target_arch = "wasm32"))]
use curvebook_doc::MAX_IMAGE_ASSET_BYTES;
use curvebook_doc::{DataSheet, SheetCellRange, SheetId};
use curvebook_engine::Session;
use curvebook_format::{CsvOptions, export_datasheet_to_csv, format_cell_token, write_delimited_records};
use curvebook_pdf::{PdfExportOptions, render_pdf};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_platform::PlatformError;
#[cfg(not(target_arch = "wasm32"))]
use curvebook_platform::process::{DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_OUTPUT_BYTES, DEFAULT_WALL_TIMEOUT, ProcessCommand, ProcessOutput};
use curvebook_render::{RenderTargetSpec, encode_emf, encode_png, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg};
use serde::{Deserialize, Serialize};

const MIN_DPI: u16 = 72;
const MAX_DPI: u16 = 1200;

/// Multi-flavor publication clipboard payload (`PDF`, `SVG`, `HTML`, high-DPI `PNG`, and `EMF`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipboardBundle {
    pub pdf_bytes: Vec<u8>,
    pub svg: String,
    pub html: String,
    pub png_bytes: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emf_bytes: Option<Vec<u8>>,
    pub width_pt: f64,
    pub height_pt: f64,
    pub width_px: u32,
    pub height_px: u32,
    pub dpi: u16,
}

impl ClipboardBundle {
    /// Build a multi-flavor clipboard payload (`PDF`, `SVG`, `HTML`, `PNG`, `EMF`) from a compiled [`ChartScene`].
    ///
    /// # Errors
    /// Returns an error if `dpi` is outside `72..=1200`, scene bounds are invalid, or any vector/raster encoder fails.
    pub fn from_scene(scene: &ChartScene, dpi: u16) -> Result<Self, String> {
        if !(MIN_DPI..=MAX_DPI).contains(&dpi) {
            return Err(format!("clipboard DPI {dpi} out of supported range {MIN_DPI}..={MAX_DPI}"));
        }
        if !scene.width_pt.is_finite() || !scene.height_pt.is_finite() || scene.width_pt <= 0.0 || scene.height_pt <= 0.0 {
            return Err(String::from("scene dimensions must be finite and positive"));
        }

        let scale = f64::from(dpi) / 72.0;
        let width_px = (scene.width_pt * scale).round().clamp(1.0, 8192.0) as u32;
        let height_px = (scene.height_pt * scale).round().clamp(1.0, 8192.0) as u32;

        let pdf_opts = PdfExportOptions { title: Some(scene.title.clone()), ..PdfExportOptions::default() };
        let pdf_bytes = render_pdf(scene, &pdf_opts).map_err(|err| format!("clipboard PDF encode failed: {err}"))?;
        let svg = render_svg(scene, &SvgExportOptions::default()).map_err(|err| format!("clipboard SVG encode failed: {err}"))?;
        let html = build_html_clipboard_wrapper(&scene.title, &svg, scene.width_pt, scene.height_pt);
        let raster =
            render_scene(scene, RenderTargetSpec { width_px, height_px, dpi }).map_err(|err| format!("clipboard raster render failed: {err}"))?;
        let png_bytes = encode_png(&raster).map_err(|err| format!("clipboard PNG encode failed: {err}"))?;
        let emf_bytes = Some(encode_emf(scene).map_err(|err| format!("clipboard EMF encode failed: {err}"))?);

        Ok(Self { pdf_bytes, svg, html, png_bytes, emf_bytes, width_pt: scene.width_pt, height_pt: scene.height_pt, width_px, height_px, dpi })
    }

    /// Compile `sheet_id` from `session` and build its multi-flavor [`ClipboardBundle`].
    ///
    /// # Errors
    /// Returns an error if compiling `sheet_id` or rendering any clipboard flavor fails.
    pub fn from_sheet(session: &Session, sheet_id: &SheetId, dpi: u16) -> Result<Self, String> {
        let scene = session.render_sheet_scene(sheet_id).map_err(|err| err.to_string())?;
        Self::from_scene(&scene, dpi)
    }

    /// Format an entire [`DataSheet`] as a tab-delimited TSV payload preserving `*` exclusions.
    ///
    /// # Errors
    /// Returns an error if TSV formatting fails.
    pub fn tsv_from_datasheet(sheet: &DataSheet, decimal_separator: char) -> Result<String, String> {
        let opts = CsvOptions { decimal_separator, ..CsvOptions::tsv() };
        export_datasheet_to_csv(sheet, &opts).map_err(|err| err.to_string())
    }

    /// Format a rectangular [`SheetCellRange`] of `sheet` as TSV text preserving `*` exclusions and locale decimals.
    #[must_use]
    pub fn tsv_from_datasheet_range(sheet: &DataSheet, range: SheetCellRange, decimal_separator: char) -> String {
        let grid = sheet.extract_cell_grid(range);
        if grid.is_empty() {
            return String::new();
        }
        let opts = CsvOptions { decimal_separator, ..CsvOptions::tsv() };
        let rows: Vec<Vec<String>> = grid.iter().map(|row| row.iter().map(|cell| format_cell_token(cell, &opts)).collect()).collect();
        write_delimited_records(&rows, &opts)
    }

    /// Names of non-empty clipboard flavors encoded inside this bundle.
    #[must_use]
    pub fn available_flavors(&self) -> Vec<&'static str> {
        let mut flavors = Vec::with_capacity(5);
        if !self.pdf_bytes.is_empty() {
            flavors.push("pdf");
        }
        if !self.svg.is_empty() {
            flavors.push("svg");
        }
        if !self.html.is_empty() {
            flavors.push("html");
        }
        if !self.png_bytes.is_empty() {
            flavors.push("png");
        }
        if self.emf_bytes.as_ref().is_some_and(|bytes| !bytes.is_empty()) {
            flavors.push("emf");
        }
        flavors
    }
}

/// Execution / availability state returned by native OS pasteboard and print spool adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OsAdapterAvailability {
    Submitted,
    Read,
    Unavailable,
}

/// Structured status returned by OS pasteboard and print spool integration helpers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsAdapterStatus {
    pub available: bool,
    pub availability: OsAdapterAvailability,
    pub backend: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

impl OsAdapterStatus {
    /// Construct an [`OsAdapterStatus`] representing a missing tool or headless environment.
    #[must_use]
    pub fn unavailable(backend: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            available: false,
            availability: OsAdapterAvailability::Unavailable,
            backend: backend.into(),
            detail: detail.into(),
            text: None,
            job_id: None,
        }
    }
}

/// Injectable platform clipboard adapter used by [`crate::UiApp`] and `EguiHumanPathDriver`.
///
/// Production desktop sessions delegate to [`SystemPlatformClipboardAdapter`], while parallel
/// human-path tests install `StubPlatformClipboardAdapter` (or a custom implementation) to
/// capture multi-flavor [`ClipboardBundle`] payloads without mutating the live OS pasteboard.
pub trait PlatformClipboardAdapter: std::fmt::Debug + Send + Sync {
    /// Write UTF-8 plain/TSV text to the platform clipboard.
    ///
    /// # Errors
    /// Returns a descriptive error string when the clipboard backend fails unexpectedly.
    fn write_text(&self, text: &str) -> Result<OsAdapterStatus, String>;

    /// Write a multi-flavor [`ClipboardBundle`] (`%PDF-1.7`, `SVG`, `HTML`, `PNG`, `EMF`) to the platform clipboard.
    ///
    /// # Errors
    /// Returns a descriptive error string when the bundle is empty or the clipboard backend fails.
    fn write_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String>;

    /// Read plain UTF-8 text from the platform clipboard, if supported by the adapter.
    ///
    /// # Errors
    /// Returns a descriptive error string when the clipboard backend fails unexpectedly.
    fn read_text(&self) -> Result<OsAdapterStatus, String> {
        Ok(OsAdapterStatus::unavailable("platform-clipboard", "clipboard read unsupported on this adapter"))
    }

    /// Read PNG/JPEG/TIFF raster image bytes from the platform clipboard, if present.
    ///
    /// # Errors
    /// Returns a descriptive error string when reading clipboard image data fails unexpectedly.
    fn read_image_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(None)
    }
}

/// Default platform clipboard adapter delegating to native OS pasteboard tools on desktop targets.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPlatformClipboardAdapter;

impl PlatformClipboardAdapter for SystemPlatformClipboardAdapter {
    fn write_text(&self, text: &str) -> Result<OsAdapterStatus, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            write_os_pasteboard(text)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = text;
            Ok(OsAdapterStatus::unavailable("wasm-pasteboard", "browser clipboard requires host DOM bridge"))
        }
    }

    fn write_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            write_os_clipboard_bundle(bundle)
        }
        #[cfg(target_arch = "wasm32")]
        {
            if bundle.available_flavors().is_empty() {
                return Err(String::from("cannot write empty clipboard bundle to pasteboard"));
            }
            Ok(OsAdapterStatus::unavailable("wasm-pasteboard", "browser clipboard requires host DOM bridge"))
        }
    }

    fn read_text(&self) -> Result<OsAdapterStatus, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            read_os_pasteboard()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(OsAdapterStatus::unavailable("wasm-pasteboard", "browser clipboard read requires host DOM bridge"))
        }
    }

    fn read_image_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            read_os_clipboard_image_bytes()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(None)
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Default)]
struct StubClipboardInner {
    bundles: Vec<ClipboardBundle>,
    texts: Vec<String>,
    image_bytes: Option<Vec<u8>>,
    available: bool,
}

/// Deterministic in-memory clipboard adapter stub used by `EguiHumanPathDriver` and tests
/// to capture [`ClipboardBundle`] and UTF-8 text writes with zero live OS pasteboard mutation.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone)]
pub struct StubPlatformClipboardAdapter {
    inner: std::sync::Arc<curvebook_platform::sync::RwLock<StubClipboardInner>>,
}

#[cfg(any(test, feature = "test-support"))]
impl Default for StubPlatformClipboardAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl StubPlatformClipboardAdapter {
    /// Construct an available in-memory clipboard stub with empty capture buffers.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: std::sync::Arc::new(curvebook_platform::sync::RwLock::named(
                "ui.stub_clipboard",
                StubClipboardInner { bundles: Vec::new(), texts: Vec::new(), image_bytes: None, available: true },
            )),
        }
    }

    /// Construct an in-memory clipboard stub simulating a headless environment (`available = false`)
    /// while still recording submitted payloads.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            inner: std::sync::Arc::new(curvebook_platform::sync::RwLock::named(
                "ui.stub_clipboard.unavailable",
                StubClipboardInner { bundles: Vec::new(), texts: Vec::new(), image_bytes: None, available: false },
            )),
        }
    }

    /// Return all captured multi-flavor [`ClipboardBundle`] writes in submission order.
    #[must_use]
    pub fn captured_bundles(&self) -> Vec<ClipboardBundle> {
        self.inner.read_or_recover().map(|guard| guard.bundles.clone()).unwrap_or_default()
    }

    /// Return the most recently captured [`ClipboardBundle`], if any.
    #[must_use]
    pub fn last_bundle(&self) -> Option<ClipboardBundle> {
        self.inner.read_or_recover().ok().and_then(|guard| guard.bundles.last().cloned())
    }

    /// Return all captured plain-text / TSV writes in submission order.
    #[must_use]
    pub fn captured_texts(&self) -> Vec<String> {
        self.inner.read_or_recover().map(|guard| guard.texts.clone()).unwrap_or_default()
    }

    /// Return the most recently captured plain-text / TSV write, if any.
    #[must_use]
    pub fn last_text(&self) -> Option<String> {
        self.inner.read_or_recover().ok().and_then(|guard| guard.texts.last().cloned())
    }

    /// Seed plain UTF-8 / TSV text into the stub clipboard so raw `Cmd/Ctrl+V` reads it.
    pub fn seed_text(&self, text: impl Into<String>) {
        if let Ok(mut guard) = self.inner.write_or_recover() {
            guard.image_bytes = None;
            guard.texts.push(text.into());
        }
    }

    /// Seed encoded PNG/JPEG/TIFF bytes into the stub clipboard for image paste testing.
    pub fn seed_image_bytes(&self, bytes: Vec<u8>) {
        if let Ok(mut guard) = self.inner.write_or_recover() {
            guard.image_bytes = Some(bytes);
        }
    }

    /// Clear all captured bundle, text, and seeded image entries.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.write_or_recover() {
            guard.bundles.clear();
            guard.texts.clear();
            guard.image_bytes = None;
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl PlatformClipboardAdapter for StubPlatformClipboardAdapter {
    fn write_text(&self, text: &str) -> Result<OsAdapterStatus, String> {
        let mut guard = self.inner.write_or_recover().map_err(|err| err.to_string())?;
        guard.image_bytes = None;
        guard.texts.push(text.to_owned());
        if guard.available {
            Ok(OsAdapterStatus {
                available: true,
                availability: OsAdapterAvailability::Submitted,
                backend: String::from("stub-pasteboard"),
                detail: format!("captured {} UTF-8 bytes", text.len()),
                text: None,
                job_id: None,
            })
        } else {
            Ok(OsAdapterStatus::unavailable("stub-pasteboard", "stubbed pasteboard marked unavailable"))
        }
    }

    fn write_bundle(&self, bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
        let flavors = bundle.available_flavors();
        if flavors.is_empty() {
            return Err(String::from("cannot write empty clipboard bundle to pasteboard"));
        }
        let mut guard = self.inner.write_or_recover().map_err(|err| err.to_string())?;
        guard.bundles.push(bundle.clone());
        if guard.available {
            Ok(OsAdapterStatus {
                available: true,
                availability: OsAdapterAvailability::Submitted,
                backend: String::from("stub-pasteboard"),
                detail: format!("captured {} pasteboard flavors ({})", flavors.len(), flavors.join(", ")),
                text: None,
                job_id: None,
            })
        } else {
            Ok(OsAdapterStatus::unavailable("stub-pasteboard", "stubbed pasteboard marked unavailable"))
        }
    }

    fn read_text(&self) -> Result<OsAdapterStatus, String> {
        let guard = self.inner.read_or_recover().map_err(|err| err.to_string())?;
        if !guard.available {
            return Ok(OsAdapterStatus::unavailable("stub-pasteboard", "stubbed pasteboard marked unavailable"));
        }
        let text = guard.texts.last().cloned();
        Ok(OsAdapterStatus {
            available: true,
            availability: OsAdapterAvailability::Submitted,
            backend: String::from("stub-pasteboard"),
            detail: format!("read {} UTF-8 bytes", text.as_deref().map_or(0, str::len)),
            text,
            job_id: None,
        })
    }

    fn read_image_bytes(&self) -> Result<Option<Vec<u8>>, String> {
        let guard = self.inner.read_or_recover().map_err(|err| err.to_string())?;
        if !guard.available {
            return Ok(None);
        }
        Ok(guard.image_bytes.clone())
    }
}

/// Injectable platform URL opener used by [`crate::UiApp`] for allowlisted GitHub issue links.
///
/// Interactive desktop sessions install [`SystemPlatformUrlOpener::live`], interactive browser
/// sessions install `WebPlatformUrlOpener`, and headless / test sessions keep the default
/// [`SystemPlatformUrlOpener::guarded`] or install `StubPlatformUrlOpener`.
pub trait PlatformUrlOpener: std::fmt::Debug + Send + Sync {
    /// Open an allowlisted CurveBook GitHub new-issue URL in the user's browser.
    ///
    /// # Errors
    /// Returns an error if `url` fails [`crate::feedback::validate_github_issue_url`] or if an
    /// active OS launcher fails.
    fn open_external_url(&self, url: &str) -> Result<OsAdapterStatus, String>;
}

/// Production URL opener delegating to the host OS browser launcher (`open` on macOS,
/// `xdg-open` on Unix, PowerShell `Start-Process` via stdin on Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SystemPlatformUrlOpener {
    allow_live_launch: bool,
}

impl SystemPlatformUrlOpener {
    /// Construct a guarded opener that only launches an OS browser when
    /// `CURVEBOOK_ALLOW_OS_GUI_DIALOG` is set (safe default for tests and headless sessions).
    #[must_use]
    pub const fn guarded() -> Self {
        Self { allow_live_launch: false }
    }

    /// Construct a live interactive opener for user-driven desktop windows.
    #[must_use]
    pub const fn live() -> Self {
        Self { allow_live_launch: true }
    }
}

impl PlatformUrlOpener for SystemPlatformUrlOpener {
    fn open_external_url(&self, url: &str) -> Result<OsAdapterStatus, String> {
        open_github_issue_url(url, self.allow_live_launch)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Default)]
struct StubUrlOpenerInner {
    urls: Vec<String>,
    available: bool,
}

/// In-memory [`PlatformUrlOpener`] test double recording allowlisted URL launches with zero OS side effects.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone)]
pub struct StubPlatformUrlOpener {
    inner: std::sync::Arc<curvebook_platform::sync::RwLock<StubUrlOpenerInner>>,
}

#[cfg(any(test, feature = "test-support"))]
impl Default for StubPlatformUrlOpener {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl StubPlatformUrlOpener {
    /// Construct an available URL opener stub.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: std::sync::Arc::new(curvebook_platform::sync::RwLock::named(
                "ui.stub_url_opener",
                StubUrlOpenerInner { urls: Vec::new(), available: true },
            )),
        }
    }

    /// Construct an unavailable URL opener stub that still records submitted allowlisted URLs.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            inner: std::sync::Arc::new(curvebook_platform::sync::RwLock::named(
                "ui.stub_url_opener.unavailable",
                StubUrlOpenerInner { urls: Vec::new(), available: false },
            )),
        }
    }

    /// Return all captured URLs in submission order.
    #[must_use]
    pub fn captured_urls(&self) -> Vec<String> {
        self.inner.read_or_recover().map(|guard| guard.urls.clone()).unwrap_or_default()
    }

    /// Return the most recently captured URL, if any.
    #[must_use]
    pub fn last_url(&self) -> Option<String> {
        self.inner.read_or_recover().ok().and_then(|guard| guard.urls.last().cloned())
    }

    /// Clear all captured URLs.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.write_or_recover() {
            guard.urls.clear();
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl PlatformUrlOpener for StubPlatformUrlOpener {
    fn open_external_url(&self, url: &str) -> Result<OsAdapterStatus, String> {
        crate::feedback::validate_github_issue_url(url)?;
        let mut guard = self.inner.write_or_recover().map_err(|err| err.to_string())?;
        guard.urls.push(url.to_owned());
        if guard.available {
            Ok(OsAdapterStatus {
                available: true,
                availability: OsAdapterAvailability::Submitted,
                backend: String::from("stub-browser"),
                detail: format!("captured {} URL bytes", url.len()),
                text: Some(url.to_owned()),
                job_id: None,
            })
        } else {
            Ok(OsAdapterStatus::unavailable("stub-browser", "stubbed browser launcher marked unavailable"))
        }
    }
}

#[cfg(any(all(not(target_arch = "wasm32"), target_os = "windows"), all(not(target_arch = "wasm32"), test)))]
fn build_windows_url_opener_candidate(stage_dir: &std::path::Path) -> Result<OsCommandCandidate, String> {
    const PS_OPEN_URL: &str =
        "$url = [Console]::In.ReadToEnd().Trim()\nif (-not [string]::IsNullOrWhiteSpace($url)) { Start-Process -FilePath $url }\n";
    let ps1_path = stage_dir.join("open_url.ps1");
    curvebook_platform::write_atomic(&ps1_path, PS_OPEN_URL.as_bytes()).map_err(|err| format!("failed to stage PowerShell URL launcher: {err}"))?;
    Ok(OsCommandCandidate {
        program: String::from("powershell"),
        args: vec![
            String::from("-NoProfile"),
            String::from("-NonInteractive"),
            String::from("-ExecutionPolicy"),
            String::from("Bypass"),
            String::from("-File"),
            ps1_path.to_string_lossy().into_owned(),
        ],
    })
}

/// Validate `url` against [`crate::feedback::validate_github_issue_url`] and open it in the
/// host OS browser when `allow_live_launch` is `true` or `CURVEBOOK_ALLOW_OS_GUI_DIALOG` is set.
///
/// # Errors
/// Returns an error if `url` fails allowlist validation or if spawning the host launcher fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_github_issue_url(url: &str, allow_live_launch: bool) -> Result<OsAdapterStatus, String> {
    crate::feedback::validate_github_issue_url(url)?;
    if !allow_live_launch && std::env::var_os("CURVEBOOK_ALLOW_OS_GUI_DIALOG").is_none() {
        return Ok(OsAdapterStatus::unavailable(
            "os-browser",
            "browser launch disabled in headless/test mode; copy Markdown or open the GitHub URL manually",
        ));
    }

    #[cfg(target_os = "macos")]
    let (candidate, stdin_payload, _stage_guard): (OsCommandCandidate, Option<&[u8]>, Option<StageDirGuard>) =
        (OsCommandCandidate { program: String::from("/usr/bin/open"), args: vec![url.to_owned()] }, None, None);
    #[cfg(all(unix, not(target_os = "macos")))]
    let (candidate, stdin_payload, _stage_guard): (OsCommandCandidate, Option<&[u8]>, Option<StageDirGuard>) =
        (OsCommandCandidate { program: String::from("xdg-open"), args: vec![url.to_owned()] }, None, None);
    #[cfg(target_os = "windows")]
    let (candidate, stdin_payload, _stage_guard): (OsCommandCandidate, Option<&[u8]>, Option<StageDirGuard>) = {
        let stage_dir = StageDirGuard::create("open-url")?;
        let cand = build_windows_url_opener_candidate(stage_dir.path())?;
        (cand, Some(url.as_bytes()), Some(stage_dir))
    };

    match run_candidate_with_stdio(&candidate, stdin_payload, ProcessBounds::default(), false)? {
        CandidateOutcome::Completed(_output) => Ok(OsAdapterStatus {
            available: true,
            availability: OsAdapterAvailability::Submitted,
            backend: candidate.program,
            detail: String::from("opened GitHub issue form in default browser"),
            text: Some(url.to_owned()),
            job_id: None,
        }),
        CandidateOutcome::NotFound => {
            Ok(OsAdapterStatus::unavailable(candidate.program, "OS browser launcher not found; copy Markdown or open the GitHub URL manually"))
        }
    }
}

/// Validate `url` on `wasm32` targets; browser tabs are opened via `WebPlatformUrlOpener`
/// installed by `curvebook-web`.
///
/// # Errors
/// Returns an error if `url` fails allowlist validation.
#[cfg(target_arch = "wasm32")]
pub fn open_github_issue_url(url: &str, _allow_live_launch: bool) -> Result<OsAdapterStatus, String> {
    crate::feedback::validate_github_issue_url(url)?;
    Ok(OsAdapterStatus::unavailable("wasm-browser", "no browser window opener installed; copy Markdown or open the GitHub URL manually"))
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
struct OsCommandCandidate {
    program: String,
    args: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl OsCommandCandidate {
    fn new(program: impl Into<String>, args: &[&str]) -> Self {
        Self { program: program.into(), args: args.iter().map(|&s| s.to_owned()).collect() }
    }
}

/// Write UTF-8 text (`TSV`, `SVG`, or `HTML`) to the host OS clipboard using platform tools
/// (`pbcopy` on macOS, `wl-copy`/`xclip` on Linux, `clip`/`powershell` on Windows).
///
/// Missing OS clipboard binaries or headless display environments return
/// `Ok(OsAdapterStatus { available: false, availability: OsAdapterAvailability::Unavailable, .. })`
/// without panicking.
///
/// # Errors
/// Returns a structured error string if spawning or executing an available clipboard adapter fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_os_pasteboard(text: &str) -> Result<OsAdapterStatus, String> {
    let candidates = os_pasteboard_write_candidates();
    if candidates.is_empty() {
        return Ok(OsAdapterStatus::unavailable("os-pasteboard", "no desktop clipboard display session is active in this environment"));
    }
    write_os_pasteboard_with(&candidates, text)
}

/// Write a multi-flavor [`ClipboardBundle`] (`PDF`, `SVG`, `HTML`, `PNG`, `EMF`) to the host OS
/// clipboard using platform pasteboard integration.
///
/// # Errors
/// Returns an error if the bundle contains no valid payload or if an available clipboard adapter fails.
/// Build a Windows `CF_HTML` clipboard payload (`Version:0.9`) with 10-digit UTF-8 byte offsets.
#[must_use]
pub fn build_cf_html_payload(html_fragment: &str) -> String {
    const START_MARK: &str = "<!--StartFragment-->";
    const END_MARK: &str = "<!--EndFragment-->";
    const HEADER_TEMPLATE_LEN: usize = 105;

    let (body_html, frag_start_rel, frag_end_rel) = if let Some(s) = html_fragment.find(START_MARK)
        && let Some(after_start) = html_fragment.get(s + START_MARK.len()..)
        && let Some(e_rel) = after_start.find(END_MARK)
    {
        let fs = s + START_MARK.len();
        (html_fragment.to_owned(), fs, fs + e_rel)
    } else {
        let lower = html_fragment.to_ascii_lowercase();
        if let Some(body_open) = lower.find("<body")
            && let Some(after_body) = lower.get(body_open..)
            && let Some(gt_rel) = after_body.find('>')
            && let Some(body_close) = lower.rfind("</body>")
            && body_open + gt_rel < body_close
            && let Some(before) = html_fragment.get(..body_open + gt_rel + 1)
            && let Some(inner) = html_fragment.get(body_open + gt_rel + 1..body_close)
            && let Some(after) = html_fragment.get(body_close..)
        {
            let insert_start = body_open + gt_rel + 1;
            let wrapped = format!("{before}{START_MARK}{inner}{END_MARK}{after}");
            let fs = insert_start + START_MARK.len();
            let fe = fs + inner.len();
            (wrapped, fs, fe)
        } else {
            let prefix = "<!DOCTYPE html>\r\n<html>\r\n<body>\r\n<!--StartFragment-->";
            let suffix = "<!--EndFragment-->\r\n</body>\r\n</html>";
            let wrapped = format!("{prefix}{html_fragment}{suffix}");
            let fs = prefix.len();
            let fe = fs + html_fragment.len();
            (wrapped, fs, fe)
        }
    };

    let start_html = HEADER_TEMPLATE_LEN;
    let end_html = start_html + body_html.len();
    let start_fragment = start_html + frag_start_rel;
    let end_fragment = start_html + frag_end_rel;
    let header = format!(
        "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n"
    );
    debug_assert_eq!(header.len(), HEADER_TEMPLATE_LEN);
    format!("{header}{body_html}")
}

#[cfg(not(target_arch = "wasm32"))]
static STAGE_DIR_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(not(target_arch = "wasm32"))]
const MAX_STAGE_DIR_CREATE_ATTEMPTS: usize = 32;

#[cfg(not(target_arch = "wasm32"))]
const STALE_PREVIEW_MAX_AGE_SECS: u64 = 3600;

#[cfg(not(target_arch = "wasm32"))]
fn stage_dir_nonce(seq: u64) -> u64 {
    let time_ns =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0_u64, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
    let stack_probe = (&seq as *const u64) as usize as u64;
    let pid = u64::from(std::process::id());
    let mut x = time_ns ^ stack_probe.rotate_left(17) ^ pid.rotate_left(33) ^ seq.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(not(target_arch = "wasm32"))]
fn cleanup_stale_stage_dirs(base: &std::path::Path, prefix: &str) {
    let full_prefix = format!("curvebook-{prefix}-");
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if !name_str.starts_with(&full_prefix) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        if let Ok(modified) = meta.modified()
            && let Ok(age) = now.duration_since(modified)
            && age.as_secs() >= STALE_PREVIEW_MAX_AGE_SECS
        {
            let _ = curvebook_platform::fs::remove_dir_all_if_not_symlink(&path);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct StageDirGuard {
    path: std::path::PathBuf,
    disarmed: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl StageDirGuard {
    fn create(prefix: &str) -> Result<Self, String> {
        let base = std::env::temp_dir();
        if prefix == "print-preview" {
            cleanup_stale_stage_dirs(&base, prefix);
        }
        Self::create_in(&base, prefix)
    }

    fn create_in(base: &std::path::Path, prefix: &str) -> Result<Self, String> {
        let pid = std::process::id();
        let mut last_err = String::new();
        for _ in 0..MAX_STAGE_DIR_CREATE_ATTEMPTS {
            let seq = STAGE_DIR_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let nonce = stage_dir_nonce(seq);
            let path = base.join(format!("curvebook-{prefix}-{pid}-{nonce:016x}"));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => {
                    let meta =
                        std::fs::symlink_metadata(&path).map_err(|err| format!("failed to inspect staging dir '{}': {err}", path.display()))?;
                    if meta.file_type().is_symlink() || !meta.is_dir() {
                        return Err(format!("staging path '{}' is not a regular directory", path.display()));
                    }
                    return Ok(Self { path, disarmed: false });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_err = err.to_string();
                }
                Err(err) => {
                    return Err(format!("failed to create staging dir '{}': {err}", path.display()));
                }
            }
        }
        Err(format!("failed to allocate unique staging dir after {MAX_STAGE_DIR_CREATE_ATTEMPTS} attempts: {last_err}"))
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for StageDirGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            let _ = curvebook_platform::fs::remove_dir_all_if_not_symlink(&self.path);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn write_os_clipboard_bundle(bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
    let flavors = bundle.available_flavors();
    if flavors.is_empty() {
        return Err(String::from("cannot write empty clipboard bundle to OS pasteboard"));
    }
    let primary_text = if !bundle.svg.is_empty() { bundle.svg.as_str() } else { bundle.html.as_str() };
    write_os_clipboard_bundle_platform(bundle, primary_text, &flavors)
}

#[cfg(target_os = "macos")]
fn write_os_clipboard_bundle_platform(bundle: &ClipboardBundle, primary_text: &str, flavors: &[&str]) -> Result<OsAdapterStatus, String> {
    const JXA_WRITE_PASTEBOARD: &str = "\
ObjC.import('Foundation');
ObjC.import('AppKit');
function run(argv) {
  var mapping = [
    ['com.adobe.pdf', argv[0]],
    ['public.png', argv[1]],
    ['public.svg-image', argv[2]],
    ['public.html', argv[3]],
    ['public.utf8-plain-text', argv[4]]
  ];
  var types = $.NSMutableArray.alloc.init;
  var payloads = [];
  for (var i = 0; i < mapping.length; i++) {
    var uti = mapping[i][0];
    var path = mapping[i][1];
    if (!path || path.length === 0) continue;
    var data = $.NSData.dataWithContentsOfFile($(path));
    if (data && !data.isNil() && Number(data.length) > 0) {
      types.addObject($(uti));
      payloads.push([uti, data]);
    }
  }
  if (payloads.length === 0) return '0';
  var pb = $.NSPasteboard.generalPasteboard;
  pb.clearContents;
  pb.declareTypesOwner(types, $());
  var wrote = 0;
  for (var j = 0; j < payloads.length; j++) {
    if (pb.setDataForType(payloads[j][1], $(payloads[j][0]))) {
      wrote++;
    }
  }
  return String(wrote);
}";

    let stage_dir = StageDirGuard::create("pb-write")?;
    let pdf_arg = stage_optional_file(stage_dir.path(), "clip.pdf", &bundle.pdf_bytes)?;
    let png_arg = stage_optional_file(stage_dir.path(), "clip.png", &bundle.png_bytes)?;
    let svg_arg = stage_optional_file(stage_dir.path(), "clip.svg", bundle.svg.as_bytes())?;
    let html_arg = stage_optional_file(stage_dir.path(), "clip.html", bundle.html.as_bytes())?;
    let txt_arg = stage_optional_file(stage_dir.path(), "clip.txt", primary_text.as_bytes())?;

    let candidate = OsCommandCandidate {
        program: String::from("/usr/bin/osascript"),
        args: vec![
            String::from("-l"),
            String::from("JavaScript"),
            String::from("-e"),
            String::from(JXA_WRITE_PASTEBOARD),
            pdf_arg,
            png_arg,
            svg_arg,
            html_arg,
            txt_arg,
        ],
    };
    if let Ok(CandidateOutcome::Completed(output)) = run_candidate(&candidate, None, ProcessBounds::default()) {
        let wrote_count = String::from_utf8_lossy(&output.stdout).trim().parse::<usize>().unwrap_or(0);
        if wrote_count > 0 {
            return Ok(OsAdapterStatus {
                available: true,
                availability: OsAdapterAvailability::Submitted,
                backend: String::from("osascript-nspasteboard"),
                detail: format!("wrote {} pasteboard flavors ({wrote_count} UTIs: {})", flavors.len(), flavors.join(", ")),
                text: None,
                job_id: None,
            });
        }
    }

    drop(stage_dir);
    let fallback_candidates = os_pasteboard_write_candidates();
    if fallback_candidates.is_empty() {
        return Ok(OsAdapterStatus::unavailable("os-pasteboard", "no desktop clipboard display session is active in this environment"));
    }
    write_os_clipboard_bundle_with(&fallback_candidates, bundle, primary_text, flavors)
}

#[cfg(target_os = "windows")]
fn write_os_clipboard_bundle_platform(bundle: &ClipboardBundle, primary_text: &str, flavors: &[&str]) -> Result<OsAdapterStatus, String> {
    const PS_WRITE_PASTEBOARD: &str = r#"param(
  [string]$EmfPath,
  [string]$PngPath,
  [string]$HtmlPath,
  [string]$SvgPath,
  [string]$TxtPath
)
Add-Type -AssemblyName System.Windows.Forms
$dataObj = New-Object System.Windows.Forms.DataObject
$wrote = 0
if ($TxtPath -and (Test-Path -LiteralPath $TxtPath)) {
  $txt = [System.IO.File]::ReadAllText($TxtPath, [System.Text.Encoding]::UTF8)
  $dataObj.SetText($txt, [System.Windows.Forms.TextDataFormat]::UnicodeText)
  $wrote++
}
if ($HtmlPath -and (Test-Path -LiteralPath $HtmlPath)) {
  $htmlBytes = [System.IO.File]::ReadAllBytes($HtmlPath)
  $htmlStream = New-Object System.IO.MemoryStream(, $htmlBytes)
  $dataObj.SetData('HTML Format', $false, $htmlStream)
  $wrote++
}
if ($SvgPath -and (Test-Path -LiteralPath $SvgPath)) {
  $svgBytes = [System.IO.File]::ReadAllBytes($SvgPath)
  $svgStream = New-Object System.IO.MemoryStream(, $svgBytes)
  $dataObj.SetData('image/svg+xml', $false, $svgStream)
  $wrote++
}
if ($PngPath -and (Test-Path -LiteralPath $PngPath)) {
  $pngBytes = [System.IO.File]::ReadAllBytes($PngPath)
  $pngStream = New-Object System.IO.MemoryStream(, $pngBytes)
  $dataObj.SetData('PNG', $false, $pngStream)
  $wrote++
}
if ($wrote -gt 0) {
  [System.Windows.Forms.Clipboard]::SetDataObject($dataObj, $true)
}
if ($EmfPath -and (Test-Path -LiteralPath $EmfPath)) {
  $emfBytes = [System.IO.File]::ReadAllBytes($EmfPath)
  if ($emfBytes.Length -gt 0) {
    $src = @"
using System;
using System.Runtime.InteropServices;
public static class LcEmfClip {
  [DllImport("gdi32.dll")] public static extern IntPtr SetEnhMetaFileBits(uint cbBuffer, byte[] lpData);
  [DllImport("user32.dll")] public static extern bool OpenClipboard(IntPtr hWndNewOwner);
  [DllImport("user32.dll")] public static extern IntPtr SetClipboardData(uint uFormat, IntPtr hMem);
  [DllImport("user32.dll")] public static extern bool CloseClipboard();
}
"@
    Add-Type -TypeDefinition $src -ErrorAction Stop
    $hemf = [LcEmfClip]::SetEnhMetaFileBits([uint32]$emfBytes.Length, $emfBytes)
    if ($hemf -ne [IntPtr]::Zero -and [LcEmfClip]::OpenClipboard([IntPtr]::Zero)) {
      try {
        [void][LcEmfClip]::SetClipboardData(14, $hemf)
        $wrote++
      } finally {
        [void][LcEmfClip]::CloseClipboard()
      }
    }
  }
}
Write-Output $wrote
"#;

    let stage_dir = StageDirGuard::create("pb-win")?;
    let emf_bytes = bundle.emf_bytes.as_deref().unwrap_or(&[]);
    let emf_arg = stage_optional_file(stage_dir.path(), "clip.emf", emf_bytes)?;
    let png_arg = stage_optional_file(stage_dir.path(), "clip.png", &bundle.png_bytes)?;
    let cf_html = if bundle.html.is_empty() { String::new() } else { build_cf_html_payload(&bundle.html) };
    let html_arg = stage_optional_file(stage_dir.path(), "clip.html", cf_html.as_bytes())?;
    let svg_arg = stage_optional_file(stage_dir.path(), "clip.svg", bundle.svg.as_bytes())?;
    let txt_arg = stage_optional_file(stage_dir.path(), "clip.txt", primary_text.as_bytes())?;
    let ps1_path = stage_dir.path().join("write_bundle.ps1");
    curvebook_platform::write_atomic(&ps1_path, PS_WRITE_PASTEBOARD.as_bytes())
        .map_err(|err| format!("failed to stage PowerShell clipboard helper: {err}"))?;

    let candidate = OsCommandCandidate {
        program: String::from("powershell"),
        args: vec![
            String::from("-NoProfile"),
            String::from("-NonInteractive"),
            String::from("-STA"),
            String::from("-ExecutionPolicy"),
            String::from("Bypass"),
            String::from("-File"),
            ps1_path.to_string_lossy().into_owned(),
            emf_arg,
            png_arg,
            html_arg,
            svg_arg,
            txt_arg,
        ],
    };
    if let Ok(CandidateOutcome::Completed(output)) = run_candidate(&candidate, None, ProcessBounds::default()) {
        let wrote_count = String::from_utf8_lossy(&output.stdout).trim().parse::<usize>().unwrap_or(0);
        if wrote_count > 0 {
            return Ok(OsAdapterStatus {
                available: true,
                availability: OsAdapterAvailability::Submitted,
                backend: String::from("powershell-win32-clipboard"),
                detail: format!("wrote {} clipboard flavors ({wrote_count} formats: {})", flavors.len(), flavors.join(", ")),
                text: None,
                job_id: None,
            });
        }
    }

    drop(stage_dir);
    let fallback_candidates = vec![OsCommandCandidate::new("clip", &[])];
    write_os_clipboard_bundle_with(&fallback_candidates, bundle, primary_text, flavors)
}

#[cfg(all(not(target_arch = "wasm32"), all(unix, not(target_os = "macos"))))]
fn write_os_clipboard_bundle_platform(bundle: &ClipboardBundle, primary_text: &str, flavors: &[&str]) -> Result<OsAdapterStatus, String> {
    let fallback_candidates = os_pasteboard_write_candidates();
    if fallback_candidates.is_empty() {
        return Ok(OsAdapterStatus::unavailable("os-pasteboard", "no desktop clipboard display session is active in this environment"));
    }

    if !bundle.pdf_bytes.is_empty() {
        let mut pdf_candidates = Vec::new();
        if has_nonempty_env("WAYLAND_DISPLAY") {
            pdf_candidates.push(OsCommandCandidate::new("wl-copy", &["--type", "application/pdf"]));
        }
        if has_nonempty_env("DISPLAY") {
            pdf_candidates.push(OsCommandCandidate::new("xclip", &["-selection", "clipboard", "-t", "application/pdf"]));
        }
        for candidate in &pdf_candidates {
            if let Ok(Some(_output)) = spawn_candidate_discard_output(candidate, Some(&bundle.pdf_bytes)) {
                return Ok(OsAdapterStatus {
                    available: true,
                    availability: OsAdapterAvailability::Submitted,
                    backend: candidate.program.clone(),
                    detail: format!("wrote {} bytes as application/pdf (flavors: {})", bundle.pdf_bytes.len(), flavors.join(", ")),
                    text: None,
                    job_id: None,
                });
            }
        }
    }

    if !bundle.svg.is_empty() {
        let mut svg_candidates = Vec::new();
        if has_nonempty_env("WAYLAND_DISPLAY") {
            svg_candidates.push(OsCommandCandidate::new("wl-copy", &["--type", "image/svg+xml"]));
        }
        if has_nonempty_env("DISPLAY") {
            svg_candidates.push(OsCommandCandidate::new("xclip", &["-selection", "clipboard", "-t", "image/svg+xml"]));
        }
        for candidate in &svg_candidates {
            if let Ok(Some(_output)) = spawn_candidate_discard_output(candidate, Some(bundle.svg.as_bytes())) {
                return Ok(OsAdapterStatus {
                    available: true,
                    availability: OsAdapterAvailability::Submitted,
                    backend: candidate.program.clone(),
                    detail: format!("wrote {} bytes as image/svg+xml (flavors: {})", bundle.svg.len(), flavors.join(", ")),
                    text: None,
                    job_id: None,
                });
            }
        }
    }

    write_os_clipboard_bundle_with(&fallback_candidates, bundle, primary_text, flavors)
}

#[cfg(all(not(target_arch = "wasm32"), any(target_os = "macos", target_os = "windows")))]
fn stage_optional_file(dir: &std::path::Path, filename: &str, bytes: &[u8]) -> Result<String, String> {
    if bytes.is_empty() {
        return Ok(String::new());
    }
    let path = dir.join(filename);
    curvebook_platform::write_atomic(&path, bytes).map_err(|err| format!("failed to stage '{filename}': {err}"))?;
    Ok(path.to_string_lossy().into_owned())
}

/// Read binary `%PDF-` bytes from the host OS clipboard when a PDF representation is available.
///
/// # Errors
/// Returns a descriptive error string when the clipboard lacks PDF data or the host OS backend is unavailable.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_os_clipboard_pdf_bytes() -> Result<Vec<u8>, String> {
    #[cfg(target_os = "macos")]
    {
        const JXA_READ_PDF: &str = "\
ObjC.import('Foundation');
ObjC.import('AppKit');
function run(argv) {
  var outPath = argv[0];
  var pb = $.NSPasteboard.generalPasteboard;
  var data = pb.dataForType($('com.adobe.pdf'));
  if (!data || data.isNil() || Number(data.length) === 0) return 'EMPTY';
  return data.writeToFileAtomically($(outPath), true) ? String(data.length) : 'ERR';
}";
        let stage_dir = StageDirGuard::create("pb-read-pdf")?;
        let out_path = stage_dir.path().join("read.pdf");
        let candidate = OsCommandCandidate {
            program: String::from("/usr/bin/osascript"),
            args: vec![
                String::from("-l"),
                String::from("JavaScript"),
                String::from("-e"),
                String::from(JXA_READ_PDF),
                out_path.to_string_lossy().into_owned(),
            ],
        };
        let CandidateOutcome::Completed(output) = run_candidate(&candidate, None, ProcessBounds::default())? else {
            return Err(String::from("macOS osascript clipboard reader is unavailable"));
        };
        let marker = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if marker == "EMPTY" {
            return Err(String::from("OS pasteboard does not contain a `com.adobe.pdf` entry"));
        }
        if marker == "ERR" {
            return Err(String::from("failed to write pasteboard PDF bytes to staging file"));
        }
        const MAX_PASTEBOARD_PDF_BYTES: usize = 128 * 1024 * 1024;
        let bytes = curvebook_platform::fs::read_regular_file_bytes(&out_path, MAX_PASTEBOARD_PDF_BYTES)
            .map_err(|err| format!("failed to read staged pasteboard PDF: {err}"))?;
        if bytes.is_empty() {
            return Err(String::from("staged pasteboard `com.adobe.pdf` entry was empty"));
        }
        Ok(bytes)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut candidates = Vec::new();
        if has_nonempty_env("WAYLAND_DISPLAY") {
            candidates.push(OsCommandCandidate::new("wl-paste", &["--type", "application/pdf"]));
        }
        if has_nonempty_env("DISPLAY") {
            candidates.push(OsCommandCandidate::new("xclip", &["-selection", "clipboard", "-t", "application/pdf", "-o"]));
        }
        for candidate in &candidates {
            if let Ok(Some(output)) = spawn_candidate_with_stdin(candidate, None)
                && !output.stdout.is_empty()
            {
                return Ok(output.stdout);
            }
        }
        Err(String::from("OS clipboard does not contain `application/pdf` data or no display session is active"))
    }
    #[cfg(target_os = "windows")]
    {
        Err(String::from("reading binary PDF clipboard entries is not supported on Windows without a host PDF consumer"))
    }
}

#[cfg(target_arch = "wasm32")]
pub fn read_os_clipboard_pdf_bytes() -> Result<Vec<u8>, String> {
    Err(String::from("OS clipboard PDF read is unavailable in wasm32 builds"))
}

/// Read PNG, JPEG, or TIFF raster image bytes from the host OS clipboard when available (`WF-57`).
///
/// # Errors
/// Returns a descriptive error string only when a host clipboard backend errors unexpectedly;
/// returns `Ok(None)` when the clipboard holds no raster image or no display session is active.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_os_clipboard_image_bytes() -> Result<Option<Vec<u8>>, String> {
    #[cfg(target_os = "macos")]
    {
        const JXA_READ_IMG: &str = "\
ObjC.import('Foundation');
ObjC.import('AppKit');
function run(argv) {
  var outPath = argv[0];
  var pb = $.NSPasteboard.generalPasteboard;
  var types = ['public.png', 'public.jpeg', 'public.tiff'];
  for (var i = 0; i < types.length; i++) {
    var data = pb.dataForType($(types[i]));
    if (data && !data.isNil() && Number(data.length) > 0) {
      return data.writeToFileAtomically($(outPath), true) ? String(data.length) : 'ERR';
    }
  }
  return 'EMPTY';
}";
        let stage_dir = StageDirGuard::create("pb-read-img")?;
        let out_path = stage_dir.path().join("clipboard-image.bin");
        let candidate = OsCommandCandidate {
            program: String::from("/usr/bin/osascript"),
            args: vec![
                String::from("-l"),
                String::from("JavaScript"),
                String::from("-e"),
                String::from(JXA_READ_IMG),
                out_path.to_string_lossy().into_owned(),
            ],
        };
        let Ok(CandidateOutcome::Completed(output)) = run_candidate(&candidate, None, ProcessBounds::default()) else {
            return Ok(None);
        };
        let marker = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if marker == "EMPTY" || marker == "ERR" || !out_path.exists() {
            return Ok(None);
        }
        let bytes = curvebook_platform::fs::read_regular_file_bytes(&out_path, MAX_IMAGE_ASSET_BYTES)
            .map_err(|err| format!("failed to read staged pasteboard image: {err}"))?;
        Ok((!bytes.is_empty()).then_some(bytes))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut candidates = Vec::new();
        if has_nonempty_env("WAYLAND_DISPLAY") {
            candidates.push(OsCommandCandidate::new("wl-paste", &["--type", "image/png"]));
        }
        if has_nonempty_env("DISPLAY") {
            candidates.push(OsCommandCandidate::new("xclip", &["-selection", "clipboard", "-t", "image/png", "-o"]));
        }
        for candidate in &candidates {
            if let Ok(Some(output)) = spawn_candidate_with_stdin(candidate, None)
                && !output.stdout.is_empty()
                && output.stdout.len() <= MAX_IMAGE_ASSET_BYTES
            {
                return Ok(Some(output.stdout));
            }
        }
        Ok(None)
    }
    #[cfg(target_os = "windows")]
    {
        let stage_dir = StageDirGuard::create("pb-read-img")?;
        let out_path = stage_dir.path().join("clipboard-image.png");
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; $img = [System.Windows.Forms.Clipboard]::GetImage(); if ($null -ne $img) {{ $img.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png); 'OK' }} else {{ 'EMPTY' }}",
            out_path.to_string_lossy().replace('\'', "''")
        );
        let candidate = OsCommandCandidate::new("powershell", &["-NoProfile", "-NonInteractive", "-STA", "-Command", &script]);
        let Ok(CandidateOutcome::Completed(output)) = run_candidate(&candidate, None, ProcessBounds::default()) else {
            return Ok(None);
        };
        let marker = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if marker != "OK" || !out_path.exists() {
            return Ok(None);
        }
        let bytes = curvebook_platform::fs::read_regular_file_bytes(&out_path, MAX_IMAGE_ASSET_BYTES)
            .map_err(|err| format!("failed to read staged pasteboard image: {err}"))?;
        Ok((!bytes.is_empty()).then_some(bytes))
    }
}

/// Read UTF-8 plain-text payload from the host OS clipboard.
///
/// # Errors
/// Returns a descriptive error string if the OS clipboard adapter is unavailable or holds no UTF-8 text.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_os_clipboard_text() -> Result<String, String> {
    let status = read_os_pasteboard()?;
    if status.availability != OsAdapterAvailability::Read {
        return Err(status.detail);
    }
    status.text.ok_or_else(|| String::from("OS clipboard returned no UTF-8 text payload"))
}

#[cfg(target_arch = "wasm32")]
pub fn read_os_clipboard_text() -> Result<String, String> {
    Err(String::from("OS clipboard text read is unavailable in wasm32 builds"))
}

/// Discover configured OS print queue names (`lpstat -a` on Unix/macOS, `Get-Printer` on Windows),
/// returning an empty `Vec` gracefully when no spooler is available.
#[must_use]
pub fn discover_os_printers() -> Vec<String> {
    #[cfg(all(not(target_arch = "wasm32"), unix))]
    {
        let bounds =
            ProcessBounds { wall_timeout: Duration::from_secs(3), idle_timeout: Duration::from_secs(2), max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES };
        for prog in ["lpstat", "/usr/bin/lpstat"] {
            let candidate = OsCommandCandidate::new(prog, &["-a"]);
            if let Ok(CandidateOutcome::Completed(output)) = run_candidate(&candidate, None, bounds) {
                let parsed = parse_lpstat_printers(&String::from_utf8_lossy(&output.stdout));
                if !parsed.is_empty() {
                    return parsed;
                }
                return Vec::new();
            }
        }
        Vec::new()
    }
    #[cfg(all(not(target_arch = "wasm32"), target_os = "windows"))]
    {
        let bounds =
            ProcessBounds { wall_timeout: Duration::from_secs(4), idle_timeout: Duration::from_secs(3), max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES };
        let candidate =
            OsCommandCandidate::new("powershell", &["-NoProfile", "-NonInteractive", "-Command", "Get-Printer | ForEach-Object { $_.Name }"]);
        if let Ok(CandidateOutcome::Completed(output)) = run_candidate(&candidate, None, bounds) {
            return parse_windows_printers(&String::from_utf8_lossy(&output.stdout));
        }
        Vec::new()
    }
    #[cfg(target_arch = "wasm32")]
    {
        Vec::new()
    }
}

#[cfg(any(all(not(target_arch = "wasm32"), unix), test))]
fn parse_lpstat_printers(stdout: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("lpstat:") || lower.starts_with("no destinations") {
            continue;
        }
        if let Some(name) = trimmed.split_whitespace().next()
            && validate_printer_name_value(name).is_ok()
            && !out.iter().any(|existing: &String| existing == name)
        {
            out.push(name.to_owned());
        }
    }
    out
}

#[cfg(any(all(not(target_arch = "wasm32"), target_os = "windows"), test))]
fn parse_windows_printers(stdout: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let name = line.trim();
        if !name.is_empty() && validate_printer_name_value(name).is_ok() && !out.iter().any(|existing: &String| existing == name) {
            out.push(name.to_owned());
        }
    }
    out
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn validate_printer_name_value(name: &str) -> Result<&str, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(String::from("printer name must not be empty"));
    }
    if trimmed.starts_with('-') {
        return Err(String::from("printer name must not begin with '-'"));
    }
    if trimmed.chars().any(|ch| ch.is_control() || matches!(ch, '\n' | '\r' | '\0')) {
        return Err(String::from("printer name contains invalid control characters"));
    }
    Ok(trimmed)
}

const WINDOWS_RESERVED_STEMS: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3",
    "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn is_windows_reserved_stem(stem: &str) -> bool {
    WINDOWS_RESERVED_STEMS.iter().any(|&res| stem.eq_ignore_ascii_case(res))
}

const fn is_forbidden_filename_char(ch: char) -> bool {
    ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0')
}

/// Sanitize an arbitrary sheet or project title into a cross-platform filename stem,
/// preserving Unicode letters/digits (`char::is_alphanumeric()`) plus `-` and `_`,
/// stripping path separators and control characters, guarding Windows reserved device names,
/// and capping length cleanly on UTF-8 character boundaries.
#[must_use]
pub fn sanitize_filename_stem(raw: &str, separator: char, max_chars: usize, fallback: &str) -> String {
    let sep = if matches!(separator, '-' | '_') { separator } else { '_' };
    let char_cap = max_chars.clamp(1, 128);
    let byte_cap = char_cap.saturating_mul(2).clamp(16, 120);
    let mut out = String::with_capacity(raw.len().min(byte_cap));
    let mut count = 0usize;

    for ch in raw.trim().chars() {
        if count >= char_cap {
            break;
        }
        if is_forbidden_filename_char(ch) {
            if !out.is_empty() && !out.ends_with(sep) && out.len() + sep.len_utf8() <= byte_cap {
                out.push(sep);
                count += 1;
            }
            continue;
        }
        if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
            if out.len() + ch.len_utf8() > byte_cap {
                break;
            }
            out.push(ch);
            count += 1;
        } else if (ch.is_whitespace() || ch == '.') && !out.is_empty() && !out.ends_with(sep) {
            if out.len() + sep.len_utf8() > byte_cap {
                break;
            }
            out.push(sep);
            count += 1;
        }
    }

    let trimmed = out.trim_matches(['-', '_', '.']);
    if trimmed.is_empty() {
        String::from(fallback)
    } else if is_windows_reserved_stem(trimmed) {
        format!("_{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

#[cfg(any(not(target_arch = "wasm32"), test))]
fn sanitize_print_title(raw: &str) -> String {
    sanitize_filename_stem(raw, '-', 64, "curvebook-print")
}

/// Stage a print spool PDF cleanly via `curvebook_platform::write_atomic` and launch OS print preview
/// only when `CURVEBOOK_ALLOW_OS_GUI_DIALOG` is set in the environment.
///
/// # Errors
/// Returns an error if `spool` does not contain a valid `%PDF-` payload or if atomic staging fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_os_print_preview_dialog(spool: &crate::PrintSpoolOutput) -> Result<OsAdapterStatus, String> {
    open_os_print_preview_dialog_titled(spool, "curvebook-print")
}

#[cfg(any(all(not(target_arch = "wasm32"), target_os = "windows"), all(not(target_arch = "wasm32"), test)))]
fn build_windows_preview_candidate(stage_dir: &std::path::Path, pdf_path: &std::path::Path) -> Result<OsCommandCandidate, String> {
    const PS_OPEN_PREVIEW: &str = "param([Parameter(Mandatory=$true)][string]$PdfPath)\nStart-Process -FilePath $PdfPath\n";
    let ps1_path = stage_dir.join("open_preview.ps1");
    curvebook_platform::write_atomic(&ps1_path, PS_OPEN_PREVIEW.as_bytes())
        .map_err(|err| format!("failed to stage PowerShell preview launcher: {err}"))?;
    Ok(OsCommandCandidate {
        program: String::from("powershell"),
        args: vec![
            String::from("-NoProfile"),
            String::from("-NonInteractive"),
            String::from("-ExecutionPolicy"),
            String::from("Bypass"),
            String::from("-File"),
            ps1_path.to_string_lossy().into_owned(),
            String::from("-PdfPath"),
            pdf_path.to_string_lossy().into_owned(),
        ],
    })
}

/// Stage a print spool PDF cleanly under `<safe_title>.pdf` and open host print preview when enabled.
///
/// # Errors
/// Returns an error if `spool` does not contain a valid `%PDF-` payload or if atomic staging fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_os_print_preview_dialog_titled(spool: &crate::PrintSpoolOutput, title: &str) -> Result<OsAdapterStatus, String> {
    if !spool.pdf_bytes.starts_with(b"%PDF-") {
        return Err(String::from("print preview payload must start with `%PDF-` signature"));
    }
    let safe_title = sanitize_print_title(title);
    let mut stage_dir = StageDirGuard::create("print-preview")?;
    let pdf_path = stage_dir.path().join(format!("{safe_title}.pdf"));
    curvebook_platform::write_atomic(&pdf_path, &spool.pdf_bytes)
        .map_err(|err| format!("failed to stage print preview PDF '{}': {err}", pdf_path.display()))?;

    if std::env::var_os("CURVEBOOK_ALLOW_OS_GUI_DIALOG").is_none() {
        return Ok(OsAdapterStatus::unavailable(
            "os-print-preview",
            format!(
                "staged {} PDF bytes as `{safe_title}.pdf` (GUI launch disabled unless CURVEBOOK_ALLOW_OS_GUI_DIALOG is set)",
                spool.pdf_bytes.len()
            ),
        ));
    }

    #[cfg(target_os = "macos")]
    let candidate = {
        let path_str = pdf_path.to_string_lossy().into_owned();
        OsCommandCandidate { program: String::from("/usr/bin/open"), args: vec![String::from("-a"), String::from("Preview"), path_str] }
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let candidate = {
        let path_str = pdf_path.to_string_lossy().into_owned();
        OsCommandCandidate { program: String::from("xdg-open"), args: vec![path_str] }
    };
    #[cfg(target_os = "windows")]
    let candidate = build_windows_preview_candidate(stage_dir.path(), &pdf_path)?;

    match run_candidate(&candidate, None, ProcessBounds::default())? {
        CandidateOutcome::Completed(_output) => {
            stage_dir.disarm();
            Ok(OsAdapterStatus {
                available: true,
                availability: OsAdapterAvailability::Submitted,
                backend: candidate.program,
                detail: format!("opened print preview `{safe_title}.pdf` ({} bytes)", spool.pdf_bytes.len()),
                text: None,
                job_id: None,
            })
        }
        CandidateOutcome::NotFound => {
            Ok(OsAdapterStatus::unavailable(candidate.program, format!("OS print preview launcher not found after staging `{safe_title}.pdf`")))
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn open_os_print_preview_dialog(_spool: &crate::PrintSpoolOutput) -> Result<OsAdapterStatus, String> {
    Ok(OsAdapterStatus::unavailable("wasm-print-preview", "OS print preview is unavailable in wasm32 builds"))
}

#[cfg(target_arch = "wasm32")]
pub fn open_os_print_preview_dialog_titled(_spool: &crate::PrintSpoolOutput, _title: &str) -> Result<OsAdapterStatus, String> {
    Ok(OsAdapterStatus::unavailable("wasm-print-preview", "OS print preview is unavailable in wasm32 builds"))
}

/// Read UTF-8 text from the host OS clipboard using platform tools (`pbpaste` on macOS,
/// `wl-paste`/`xclip` on Linux, `powershell Get-Clipboard` on Windows).
///
/// Missing OS clipboard binaries or headless display environments return
/// `Ok(OsAdapterStatus { available: false, availability: OsAdapterAvailability::Unavailable, .. })`
/// without panicking.
///
/// # Errors
/// Returns a structured error string if an available clipboard adapter fails or returns non-UTF-8
/// output.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_os_pasteboard() -> Result<OsAdapterStatus, String> {
    let candidates = os_pasteboard_read_candidates();
    if candidates.is_empty() {
        return Ok(OsAdapterStatus::unavailable("os-pasteboard", "no desktop clipboard display session is active in this environment"));
    }
    read_os_pasteboard_with(&candidates)
}

/// Submit a physical-scale [`crate::PrintSpoolOutput`] PDF payload to the host OS print spooler
/// (`lp` / `lpr`).
///
/// Missing print-spool binaries return
/// `Ok(OsAdapterStatus { available: false, availability: OsAdapterAvailability::Unavailable, .. })`
/// without panicking.
///
/// # Errors
/// Returns a structured error string if `spool.pdf_bytes` is not a valid PDF payload, `printer` is
/// invalid, or an available print-spool binary rejects the job.
#[cfg(not(target_arch = "wasm32"))]
pub fn submit_os_print_job(spool: &crate::PrintSpoolOutput, printer: Option<&str>) -> Result<OsAdapterStatus, String> {
    submit_os_print_pdf_bytes(&spool.pdf_bytes, printer)
}

/// Submit raw `%PDF-` bytes to the host OS print spooler (`lp` / `lpr`).
///
/// # Errors
/// Returns a structured error string if `pdf_bytes` is not a valid PDF payload, `printer` is
/// invalid, or an available print-spool binary fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn submit_os_print_pdf_bytes(pdf_bytes: &[u8], printer: Option<&str>) -> Result<OsAdapterStatus, String> {
    let clean_printer = validate_printer_name(printer)?;
    let candidates = os_print_candidates(clean_printer);
    submit_os_print_bytes_with(&candidates, pdf_bytes)
}

#[cfg(not(target_arch = "wasm32"))]
fn validate_printer_name(printer: Option<&str>) -> Result<Option<&str>, String> {
    let Some(raw) = printer else {
        return Ok(None);
    };
    validate_printer_name_value(raw).map(Some)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_os_clipboard_bundle_with(
    candidates: &[OsCommandCandidate],
    bundle: &ClipboardBundle,
    primary_text: &str,
    flavors: &[&str],
) -> Result<OsAdapterStatus, String> {
    let fallback_svg_or_html =
        if primary_text.is_empty() { format!("curvebook-figure {}x{}", bundle.width_px, bundle.height_px) } else { primary_text.to_owned() };
    let mut status = write_os_pasteboard_with(candidates, &fallback_svg_or_html)?;
    if status.available {
        status.detail = format!("wrote clipboard bundle ({}) via {}", flavors.join(", "), status.backend);
    }
    Ok(status)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_os_pasteboard_with(candidates: &[OsCommandCandidate], text: &str) -> Result<OsAdapterStatus, String> {
    let mut missing = Vec::new();
    for candidate in candidates {
        match spawn_candidate_discard_output(candidate, Some(text.as_bytes()))? {
            Some(output) => {
                let detail = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                return Ok(OsAdapterStatus {
                    available: true,
                    availability: OsAdapterAvailability::Submitted,
                    backend: candidate.program.clone(),
                    detail: if detail.is_empty() { format!("wrote {} bytes to OS pasteboard", text.len()) } else { detail },
                    text: None,
                    job_id: None,
                });
            }
            None => missing.push(candidate.program.as_str()),
        }
    }
    Ok(OsAdapterStatus::unavailable(missing.join("/"), "no OS clipboard adapter binary found on PATH"))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_os_pasteboard_with(candidates: &[OsCommandCandidate]) -> Result<OsAdapterStatus, String> {
    let mut missing = Vec::new();
    for candidate in candidates {
        match spawn_candidate_with_stdin(candidate, None)? {
            Some(output) => {
                let text = String::from_utf8(output.stdout).map_err(|err| format!("OS pasteboard returned non-UTF-8 text: {err}"))?;
                return Ok(OsAdapterStatus {
                    available: true,
                    availability: OsAdapterAvailability::Read,
                    backend: candidate.program.clone(),
                    detail: format!("read {} bytes from OS pasteboard", text.len()),
                    text: Some(text),
                    job_id: None,
                });
            }
            None => missing.push(candidate.program.as_str()),
        }
    }
    Ok(OsAdapterStatus::unavailable(missing.join("/"), "no OS clipboard adapter binary found on PATH"))
}

#[cfg(not(target_arch = "wasm32"))]
fn submit_os_print_bytes_with(candidates: &[OsCommandCandidate], pdf_bytes: &[u8]) -> Result<OsAdapterStatus, String> {
    if !pdf_bytes.starts_with(b"%PDF-") {
        return Err(String::from("print spool payload must start with `%PDF-` signature"));
    }
    let mut missing = Vec::new();
    for candidate in candidates {
        match spawn_candidate_with_stdin(candidate, Some(pdf_bytes))? {
            Some(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                let job_id = (!stdout.is_empty()).then_some(stdout.clone());
                return Ok(OsAdapterStatus {
                    available: true,
                    availability: OsAdapterAvailability::Submitted,
                    backend: candidate.program.clone(),
                    detail: if stdout.is_empty() { format!("submitted {} PDF bytes to print spool", pdf_bytes.len()) } else { stdout },
                    text: None,
                    job_id,
                });
            }
            None => missing.push(candidate.program.as_str()),
        }
    }
    Ok(OsAdapterStatus::unavailable(missing.join("/"), "no OS print spooler binary (`lp`/`lpr`) found on PATH"))
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcessBounds {
    wall_timeout: Duration,
    idle_timeout: Duration,
    max_output_bytes: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for ProcessBounds {
    fn default() -> Self {
        Self { wall_timeout: DEFAULT_WALL_TIMEOUT, idle_timeout: DEFAULT_IDLE_TIMEOUT, max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum CandidateOutcome {
    NotFound,
    Completed(ProcessOutput),
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_candidate_with_stdin(candidate: &OsCommandCandidate, stdin_bytes: Option<&[u8]>) -> Result<Option<ProcessOutput>, String> {
    match run_candidate(candidate, stdin_bytes, ProcessBounds::default())? {
        CandidateOutcome::NotFound => Ok(None),
        CandidateOutcome::Completed(output) => Ok(Some(output)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_candidate_discard_output(candidate: &OsCommandCandidate, stdin_bytes: Option<&[u8]>) -> Result<Option<ProcessOutput>, String> {
    match run_candidate_with_stdio(candidate, stdin_bytes, ProcessBounds::default(), false)? {
        CandidateOutcome::NotFound => Ok(None),
        CandidateOutcome::Completed(output) => Ok(Some(output)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn run_candidate(candidate: &OsCommandCandidate, stdin_bytes: Option<&[u8]>, bounds: ProcessBounds) -> Result<CandidateOutcome, String> {
    run_candidate_with_stdio(candidate, stdin_bytes, bounds, true)
}

#[cfg(not(target_arch = "wasm32"))]
fn run_candidate_with_stdio(
    candidate: &OsCommandCandidate,
    stdin_bytes: Option<&[u8]>,
    bounds: ProcessBounds,
    capture_output: bool,
) -> Result<CandidateOutcome, String> {
    let mut cmd = ProcessCommand::new(&candidate.program)
        .args(&candidate.args)
        .wall_timeout(bounds.wall_timeout)
        .idle_timeout(bounds.idle_timeout)
        .max_output_bytes(bounds.max_output_bytes);
    if !capture_output {
        cmd = cmd.discard_output();
    }
    if let Some(bytes) = stdin_bytes {
        cmd = cmd.stdin_bytes(bytes);
    }

    let output = match cmd.run() {
        Ok(output) => output,
        Err(PlatformError::ProgramNotFound { .. }) => return Ok(CandidateOutcome::NotFound),
        Err(PlatformError::TimedOut { program, kind, limit_ms }) => {
            return Err(format!("`{program}` timed out after {limit_ms}ms ({kind} deadline exceeded)"));
        }
        Err(PlatformError::OutputLimitExceeded { program, stream, limit_bytes }) => {
            return Err(format!("`{program}` exceeded {stream} output cap of {limit_bytes} bytes"));
        }
        Err(err) => return Err(format!("failed to run `{}`: {err}", candidate.program)),
    };

    if !output.success {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = if stderr.is_empty() { String::from("no stderr output") } else { stderr };
        let status = match output.status_code {
            Some(code) => format!("exit code {code}"),
            None => String::from("signal termination"),
        };
        return Err(format!("`{}` exited with {status}: {detail}", candidate.program));
    }
    Ok(CandidateOutcome::Completed(output))
}

#[cfg(not(target_arch = "wasm32"))]
fn os_pasteboard_write_candidates() -> Vec<OsCommandCandidate> {
    #[cfg(target_os = "macos")]
    {
        vec![OsCommandCandidate::new("pbcopy", &[]), OsCommandCandidate::new("/usr/bin/pbcopy", &[])]
    }
    #[cfg(target_os = "windows")]
    {
        vec![
            OsCommandCandidate::new("clip", &[]),
            OsCommandCandidate::new("powershell", &["-NoProfile", "-NonInteractive", "-Command", "$input | Set-Clipboard"]),
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut out = Vec::new();
        if has_nonempty_env("WAYLAND_DISPLAY") {
            out.push(OsCommandCandidate::new("wl-copy", &[]));
        }
        if has_nonempty_env("DISPLAY") {
            out.push(OsCommandCandidate::new("xclip", &["-selection", "clipboard"]));
        }
        out
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn os_pasteboard_read_candidates() -> Vec<OsCommandCandidate> {
    #[cfg(target_os = "macos")]
    {
        vec![OsCommandCandidate::new("pbpaste", &[]), OsCommandCandidate::new("/usr/bin/pbpaste", &[])]
    }
    #[cfg(target_os = "windows")]
    {
        vec![OsCommandCandidate::new("powershell", &["-NoProfile", "-NonInteractive", "-Command", "Get-Clipboard"])]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut out = Vec::new();
        if has_nonempty_env("WAYLAND_DISPLAY") {
            out.push(OsCommandCandidate::new("wl-paste", &["--no-newline"]));
        }
        if has_nonempty_env("DISPLAY") {
            out.push(OsCommandCandidate::new("xclip", &["-selection", "clipboard", "-o"]));
        }
        out
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn os_print_candidates(printer: Option<&str>) -> Vec<OsCommandCandidate> {
    let lp = match printer {
        Some(p) => OsCommandCandidate::new("lp", &["-d", p]),
        None => OsCommandCandidate::new("lp", &[]),
    };
    let lpr = match printer {
        Some(p) => OsCommandCandidate::new("lpr", &["-P", p]),
        None => OsCommandCandidate::new("lpr", &[]),
    };
    #[cfg(target_os = "windows")]
    {
        vec![lpr, lp]
    }
    #[cfg(not(target_os = "windows"))]
    {
        vec![lp, lpr]
    }
}

#[cfg(all(not(target_arch = "wasm32"), all(unix, not(target_os = "macos"))))]
fn has_nonempty_env(key: &str) -> bool {
    std::env::var_os(key).is_some_and(|v| !v.is_empty())
}

fn build_html_clipboard_wrapper(title: &str, svg: &str, width_pt: f64, height_pt: f64) -> String {
    let escaped_title = escape_html(title);
    format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>{escaped_title}</title></head>\
<body><div class=\"curvebook-figure\" style=\"width:{width_pt:.3}pt;height:{height_pt:.3}pt;\">{svg}</div></body></html>"
    )
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use curvebook_chart::{ChartScene, PointPt, RectPt, ScenePrimitive, StrokePaint};
    use curvebook_palette::Rgba8;

    use super::{
        ClipboardBundle, OsAdapterAvailability, OsCommandCandidate, read_os_pasteboard_with, submit_os_print_bytes_with, submit_os_print_job,
        write_os_clipboard_bundle_with, write_os_pasteboard_with,
    };
    use crate::{PrintJobSpec, PrintSpoolOutput};

    #[test]
    fn os_adapters_handle_missing_tools_and_invalid_inputs_safely() -> Result<(), Box<dyn std::error::Error>> {
        let missing = [OsCommandCandidate::new("curvebook-nonexistent-os-tool-1", &[])];
        let write_unavail = write_os_pasteboard_with(&missing, "hello")?;
        assert!(!write_unavail.available);
        assert_eq!(write_unavail.availability, OsAdapterAvailability::Unavailable);

        let read_unavail = read_os_pasteboard_with(&missing)?;
        assert!(!read_unavail.available);
        assert_eq!(read_unavail.availability, OsAdapterAvailability::Unavailable);
        assert!(read_unavail.text.is_none());

        let print_unavail = submit_os_print_bytes_with(&missing, b"%PDF-1.7\n%%EOF\n")?;
        assert!(!print_unavail.available);
        assert_eq!(print_unavail.availability, OsAdapterAvailability::Unavailable);

        let bad_spool = PrintSpoolOutput { pdf_bytes: b"not-a-pdf".to_vec(), page_count: 0, page_sizes_pt: Vec::new(), scale_percent: 100.0 };
        assert!(submit_os_print_job(&bad_spool, None).is_err());

        let valid_spool = PrintSpoolOutput {
            pdf_bytes: b"%PDF-1.7\n%%EOF\n".to_vec(),
            page_count: 1,
            page_sizes_pt: vec![(100.0, 100.0)],
            scale_percent: PrintJobSpec::default().scale_percent,
        };
        assert!(submit_os_print_job(&valid_spool, Some("-malicious-flag")).is_err());
        assert!(submit_os_print_job(&valid_spool, Some("   ")).is_err());

        #[cfg(unix)]
        {
            let write_fallback = [OsCommandCandidate::new("curvebook-missing-bin", &[]), OsCommandCandidate::new("sh", &["-c", "cat >/dev/null"])];
            let ok_write = write_os_pasteboard_with(&write_fallback, "x\ty\n1\t2\n")?;
            assert!(ok_write.available);
            assert_eq!(ok_write.availability, OsAdapterAvailability::Submitted);

            let read_mock = [OsCommandCandidate::new("printf", &["alpha\tbeta"])];
            let ok_read = read_os_pasteboard_with(&read_mock)?;
            assert!(ok_read.available);
            assert_eq!(ok_read.availability, OsAdapterAvailability::Read);
            assert_eq!(ok_read.text.as_deref(), Some("alpha\tbeta"));

            let print_mock = [OsCommandCandidate::new("sh", &["-c", "cat >/dev/null && printf 'request id is Mock-1'"])];
            let ok_print = submit_os_print_bytes_with(&print_mock, &valid_spool.pdf_bytes)?;
            assert!(ok_print.available);
            assert_eq!(ok_print.availability, OsAdapterAvailability::Submitted);
            assert_eq!(ok_print.job_id.as_deref(), Some("request id is Mock-1"));

            let failing = [OsCommandCandidate::new("sh", &["-c", "echo 'printer offline' >&2; exit 2"])];
            match submit_os_print_bytes_with(&failing, &valid_spool.pdf_bytes) {
                Err(err) => assert!(err.contains("printer offline")),
                Ok(other) => return Err(format!("expected failure status, got {other:?}").into()),
            }
        }
        Ok(())
    }

    #[test]
    fn clipboard_bundle_populates_valid_emf_and_writes_bundle_status() -> Result<(), Box<dyn std::error::Error>> {
        let mut scene = ChartScene::new("s1", "Bundle Test", 200.0, 120.0, None);
        scene.primitives.push(ScenePrimitive::Path {
            path: curvebook_chart::Path2D::line(PointPt::new(10.0, 10.0), PointPt::new(190.0, 110.0)),
            fill: None,
            stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
            clip: None,
        });
        let bundle = ClipboardBundle::from_scene(&scene, 144)?;
        let emf = bundle.emf_bytes.as_deref().ok_or("missing emf_bytes")?;
        assert!(emf.len() >= 88);
        assert_eq!(&emf[0..4], &1u32.to_le_bytes());
        assert_eq!(&emf[40..44], &0x464D_4520u32.to_le_bytes());
        assert!(bundle.available_flavors().contains(&"emf"));

        #[cfg(unix)]
        {
            let write_mock = [OsCommandCandidate::new("sh", &["-c", "cat >/dev/null"])];
            let status = write_os_clipboard_bundle_with(&write_mock, &bundle, &bundle.svg, &bundle.available_flavors())?;
            assert!(status.available);
            assert!(status.detail.contains("emf"));
        }
        let _ = RectPt::new(0.0, 0.0, 1.0, 1.0);
        Ok(())
    }

    #[cfg(all(unix, not(target_arch = "wasm32")))]
    #[test]
    fn os_command_enforces_timeouts_and_output_caps() {
        use std::time::Duration;

        use super::{ProcessBounds, run_candidate};

        let sleepy = OsCommandCandidate::new("sh", &["-c", "sleep 2"]);
        let timeout_bounds = ProcessBounds { wall_timeout: Duration::from_millis(80), idle_timeout: Duration::from_secs(2), max_output_bytes: 4096 };
        let err = run_candidate(&sleepy, None, timeout_bounds).unwrap_err();
        assert!(err.contains("timed out"), "unexpected timeout err: {err}");

        let noisy = OsCommandCandidate::new("sh", &["-c", "yes curvebook_cap_test"]);
        let cap_bounds = ProcessBounds { wall_timeout: Duration::from_secs(2), idle_timeout: Duration::from_secs(2), max_output_bytes: 512 };
        let cap_err = run_candidate(&noisy, None, cap_bounds).unwrap_err();
        assert!(cap_err.contains("output cap"), "unexpected cap err: {cap_err}");
    }

    #[test]
    fn build_cf_html_payload_computes_exact_utf8_byte_offsets() {
        let frag = "<b>IC<sub>50</sub> αβγ 日本語</b>";
        let payload = super::build_cf_html_payload(frag);
        assert!(payload.starts_with("Version:0.9\r\n"));

        let parse_offset = |label: &str| -> usize {
            let line = payload.lines().find(|l| l.starts_with(label)).unwrap_or_else(|| panic!("missing {label}"));
            let digits = line.trim_start_matches(label).trim();
            assert_eq!(digits.len(), 10, "expected 10-digit zero-padded offset for {label}, got {digits:?}");
            digits.parse::<usize>().expect("valid usize offset")
        };

        let start_html = parse_offset("StartHTML:");
        let end_html = parse_offset("EndHTML:");
        let start_frag = parse_offset("StartFragment:");
        let end_frag = parse_offset("EndFragment:");

        assert_eq!(end_html, payload.len());
        let html_slice = payload.get(start_html..end_html).expect("valid StartHTML..EndHTML");
        assert!(html_slice.starts_with("<!DOCTYPE html>"));
        assert!(html_slice.ends_with("</html>"));
        assert_eq!(payload.get(start_frag..end_frag), Some(frag));

        let wrapped_doc = "<!DOCTYPE html><html><head></head><body><div>α</div></body></html>";
        let wrapped_cf = super::build_cf_html_payload(wrapped_doc);
        let ws = wrapped_cf
            .lines()
            .find(|l| l.starts_with("StartFragment:"))
            .and_then(|l| l.trim_start_matches("StartFragment:").trim().parse::<usize>().ok())
            .expect("StartFragment");
        let we = wrapped_cf
            .lines()
            .find(|l| l.starts_with("EndFragment:"))
            .and_then(|l| l.trim_start_matches("EndFragment:").trim().parse::<usize>().ok())
            .expect("EndFragment");
        assert_eq!(wrapped_cf.get(ws..we), Some("<div>α</div>"));
    }

    #[test]
    fn printer_discovery_parsers_and_print_preview_staging_are_clean() {
        let lp_sample = "\
lpstat: warning
Laser_Lab_01 accepting requests since Mon Nov 14 10:00:00 2026
PDF_Printer accepting requests since Mon Nov 14 10:00:00 2026
Laser_Lab_01 accepting requests since Mon Nov 14 10:00:00 2026
-badflag accepting requests
";
        assert_eq!(super::parse_lpstat_printers(lp_sample), vec!["Laser_Lab_01", "PDF_Printer"]);
        assert_eq!(
            super::parse_windows_printers("Microsoft Print to PDF\r\nCanon Pro\r\nMicrosoft Print to PDF\r\n"),
            vec!["Microsoft Print to PDF", "Canon Pro"]
        );
        assert_eq!(super::sanitize_print_title("  ../../Figure 1 (Final).pdf  "), "Figure-1-Final-pdf");

        let _ = super::discover_os_printers();

        let bad_spool = PrintSpoolOutput { pdf_bytes: b"bad".to_vec(), page_count: 1, page_sizes_pt: vec![(612.0, 792.0)], scale_percent: 100.0 };
        assert!(super::open_os_print_preview_dialog(&bad_spool).is_err());

        let valid_spool = PrintSpoolOutput {
            pdf_bytes: b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\n%%EOF\n".to_vec(),
            page_count: 1,
            page_sizes_pt: vec![(612.0, 792.0)],
            scale_percent: 100.0,
        };
        if std::env::var_os("CURVEBOOK_ALLOW_OS_GUI_DIALOG").is_none() {
            let status = super::open_os_print_preview_dialog_titled(&valid_spool, "Dose Response Figure").expect("stage print preview");
            assert_eq!(status.availability, OsAdapterAvailability::Unavailable);
            assert!(status.detail.contains("Dose-Response-Figure.pdf"));
        }
    }

    #[cfg(all(unix, not(target_arch = "wasm32")))]
    #[test]
    fn r1_s11_08_write_os_pasteboard_with_does_not_wait_on_backgrounded_child_stdio() -> Result<(), String> {
        let candidates =
            [OsCommandCandidate { program: String::from("sh"), args: vec![String::from("-c"), String::from("cat >/dev/null; (sleep 5) &")] }];
        let start = std::time::Instant::now();
        let status = super::write_os_pasteboard_with(&candidates, "hello clipboard")?;
        let elapsed = start.elapsed();
        if status.availability != OsAdapterAvailability::Submitted {
            return Err(format!("expected Submitted status, got {:?}", status));
        }
        if elapsed >= std::time::Duration::from_secs(2) {
            return Err(format!("pasteboard write blocked {elapsed:?} waiting for backgrounded child"));
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn r2_s10_11_12_stage_dir_guard_and_windows_ps1_preview_launcher() -> Result<(), String> {
        let root = std::env::temp_dir().join(format!("curvebook-r2s10-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).map_err(|e| format!("create root: {e}"))?;
        let d1_path = {
            let g1 = super::StageDirGuard::create_in(&root, "r2s10")?;
            let g2 = super::StageDirGuard::create_in(&root, "r2s10")?;
            if g1.path() == g2.path() {
                return Err(String::from("expected distinct randomized staging directories"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(g1.path()).map_err(|e| format!("stat: {e}"))?.permissions().mode() & 0o777;
                if mode != 0o700 {
                    return Err(format!("expected 0o700 mode on staging dir, got {mode:o}"));
                }
            }
            let pdf = g1.path().join("figure.pdf");
            let cand = super::build_windows_preview_candidate(g1.path(), &pdf)?;
            if cand.program != "powershell" || !cand.args.windows(2).any(|w| w[0] == "-File" && w[1].ends_with("open_preview.ps1")) {
                return Err(format!("expected -File open_preview.ps1 launcher, got {:?}", cand.args));
            }
            if cand.args.iter().any(|a| a == "-Command" || a.contains("$args[0]")) {
                return Err(format!("unexpected -Command string interpolation in {:?}", cand.args));
            }
            let ps1 = std::fs::read_to_string(g1.path().join("open_preview.ps1")).map_err(|e| format!("read ps1: {e}"))?;
            if !ps1.contains("param(") || !ps1.contains("-FilePath $PdfPath") {
                return Err(format!("unexpected open_preview.ps1 content: {ps1}"));
            }
            g1.path().to_path_buf()
        };
        if d1_path.exists() {
            return Err(format!("StageDirGuard did not remove '{}' on drop", d1_path.display()));
        }
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn rv53_sanitize_print_title_preserves_unicode_and_guards_traversal_and_reserved_stems() {
        assert_eq!(super::sanitize_print_title("数据"), "数据");
        assert_eq!(super::sanitize_print_title("图表"), "图表");
        assert_ne!(super::sanitize_print_title("数据"), super::sanitize_print_title("图表"));
        assert_eq!(super::sanitize_print_title("График доза"), "График-доза");
        assert_eq!(super::sanitize_print_title("منحنى الاستجابة"), "منحنى-الاستجابة");
        assert_eq!(super::sanitize_print_title("עקומת מינון"), "עקומת-מינון");
        assert_eq!(super::sanitize_print_title("../foo"), "foo");
        assert_eq!(super::sanitize_print_title("..\\..\\foo:bar*baz?\0"), "foo-bar-baz");
        assert_eq!(super::sanitize_print_title("CON"), "_CON");
        assert_eq!(super::sanitize_print_title("con"), "_con");
        assert_eq!(super::sanitize_print_title("LPT1"), "_LPT1");
        assert_eq!(super::sanitize_print_title("COM9..."), "_COM9");
        assert_eq!(super::sanitize_print_title("   ///...   "), "curvebook-print");

        let long_cjk = "数据图表".repeat(40);
        let capped = super::sanitize_print_title(&long_cjk);
        assert!(capped.chars().count() <= 64 && capped.len() <= 120 && !capped.is_empty());
    }

    #[test]
    fn url_opener_enforces_github_allowlist_and_guards_headless_launch() -> Result<(), String> {
        use super::{OsAdapterAvailability, PlatformUrlOpener, StubPlatformUrlOpener, SystemPlatformUrlOpener};

        let valid = "https://github.com/HauserGroup/curvebook/issues/new?template=bug_report.yml&title=Test";
        let stub = StubPlatformUrlOpener::new();
        assert!(stub.open_external_url("https://example.com/evil").is_err());
        assert!(stub.captured_urls().is_empty());

        let status = stub.open_external_url(valid)?;
        assert!(status.available);
        assert_eq!(status.availability, OsAdapterAvailability::Submitted);
        assert_eq!(stub.last_url().as_deref(), Some(valid));

        let guarded = SystemPlatformUrlOpener::guarded();
        assert!(guarded.open_external_url("javascript:alert(1)").is_err());
        if std::env::var_os("CURVEBOOK_ALLOW_OS_GUI_DIALOG").is_none() {
            let guarded_status = guarded.open_external_url(valid)?;
            assert!(!guarded_status.available);
            assert_eq!(guarded_status.availability, OsAdapterAvailability::Unavailable);
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let root = std::env::temp_dir().join(format!("curvebook-url-ps1-test-{}", std::process::id()));
            std::fs::create_dir_all(&root).map_err(|e| format!("create root: {e}"))?;
            let dir = super::StageDirGuard::create_in(&root, "url")?;
            let cand = super::build_windows_url_opener_candidate(dir.path())?;
            assert_eq!(cand.program, "powershell");
            let ps1 = std::fs::read_to_string(dir.path().join("open_url.ps1")).map_err(|e| format!("read ps1: {e}"))?;
            assert!(ps1.contains("[Console]::In.ReadToEnd()"));
            assert!(ps1.contains("Start-Process -FilePath $url"));
            drop(dir);
            let _ = std::fs::remove_dir_all(&root);
        }
        Ok(())
    }
}
