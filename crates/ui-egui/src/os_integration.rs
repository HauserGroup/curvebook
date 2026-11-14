//! Native OS clipboard multi-flavor bundles (`PDF`, `SVG`, `HTML`, `PNG`, `EMF`) and host
//! pasteboard / print-spooler integration (`docs/hardware-parity.md` `G10`, `G11`, `G13`).

#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

use lumencraft_chart::ChartScene;
use lumencraft_doc::{DataSheet, SheetCellRange, SheetId};
use lumencraft_engine::Session;
use lumencraft_format::{CsvOptions, export_datasheet_to_csv, format_cell_token, write_delimited_records};
use lumencraft_pdf::{PdfExportOptions, render_pdf};
#[cfg(not(target_arch = "wasm32"))]
use lumencraft_platform::PlatformError;
#[cfg(not(target_arch = "wasm32"))]
use lumencraft_platform::process::{DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_OUTPUT_BYTES, DEFAULT_WALL_TIMEOUT, ProcessCommand, ProcessOutput};
use lumencraft_render::{RenderTargetSpec, encode_emf, encode_png, render_scene};
use lumencraft_svg::{SvgExportOptions, render_svg};
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
#[cfg(not(target_arch = "wasm32"))]
pub fn write_os_clipboard_bundle(bundle: &ClipboardBundle) -> Result<OsAdapterStatus, String> {
    let flavors = bundle.available_flavors();
    if flavors.is_empty() {
        return Err(String::from("cannot write empty clipboard bundle to OS pasteboard"));
    }
    let primary_text = if !bundle.svg.is_empty() { bundle.svg.as_str() } else { bundle.html.as_str() };
    let candidates = os_pasteboard_write_candidates();
    if candidates.is_empty() {
        return Ok(OsAdapterStatus::unavailable("os-pasteboard", "no desktop clipboard display session is active in this environment"));
    }
    write_os_clipboard_bundle_with(&candidates, bundle, primary_text, &flavors)
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
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(String::from("printer name must not be empty"));
    }
    if trimmed.starts_with('-') {
        return Err(format!("invalid printer name `{trimmed}`: must not start with '-'"));
    }
    Ok(Some(trimmed))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_os_clipboard_bundle_with(
    candidates: &[OsCommandCandidate],
    bundle: &ClipboardBundle,
    primary_text: &str,
    flavors: &[&str],
) -> Result<OsAdapterStatus, String> {
    let fallback_svg_or_html =
        if primary_text.is_empty() { format!("lumencraft-figure {}x{}", bundle.width_px, bundle.height_px) } else { primary_text.to_owned() };
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
        match spawn_candidate_with_stdin(candidate, Some(text.as_bytes()))? {
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
fn run_candidate(candidate: &OsCommandCandidate, stdin_bytes: Option<&[u8]>, bounds: ProcessBounds) -> Result<CandidateOutcome, String> {
    let mut cmd = ProcessCommand::new(&candidate.program)
        .args(&candidate.args)
        .wall_timeout(bounds.wall_timeout)
        .idle_timeout(bounds.idle_timeout)
        .max_output_bytes(bounds.max_output_bytes);
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
<body><div class=\"lumencraft-figure\" style=\"width:{width_pt:.3}pt;height:{height_pt:.3}pt;\">{svg}</div></body></html>"
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
    use lumencraft_chart::{ChartScene, PointPt, RectPt, ScenePrimitive, StrokePaint};
    use lumencraft_palette::Rgba8;

    use super::{
        ClipboardBundle, OsAdapterAvailability, OsCommandCandidate, read_os_pasteboard_with, submit_os_print_bytes_with, submit_os_print_job,
        write_os_clipboard_bundle_with, write_os_pasteboard_with,
    };
    use crate::{PrintJobSpec, PrintSpoolOutput};

    #[test]
    fn os_adapters_handle_missing_tools_and_invalid_inputs_safely() -> Result<(), Box<dyn std::error::Error>> {
        let missing = [OsCommandCandidate::new("lumencraft-nonexistent-os-tool-1", &[])];
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
            let write_fallback = [OsCommandCandidate::new("lumencraft-missing-bin", &[]), OsCommandCandidate::new("sh", &["-c", "cat >/dev/null"])];
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
            path: lumencraft_chart::Path2D::line(PointPt::new(10.0, 10.0), PointPt::new(190.0, 110.0)),
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

        let noisy = OsCommandCandidate::new("sh", &["-c", "yes lumencraft_cap_test"]);
        let cap_bounds = ProcessBounds { wall_timeout: Duration::from_secs(2), idle_timeout: Duration::from_secs(2), max_output_bytes: 512 };
        let cap_err = run_candidate(&noisy, None, cap_bounds).unwrap_err();
        assert!(cap_err.contains("output cap"), "unexpected cap err: {cap_err}");
    }
}
