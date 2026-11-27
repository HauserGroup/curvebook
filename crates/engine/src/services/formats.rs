//! Shared `FormatRegistry` import/export dispatch and project-loading services.

use std::collections::HashSet;
#[cfg(not(target_arch = "wasm32"))]
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use curvebook_doc::{
    AnalysisKind, ErrorCode, GraphSheet, ImportedSnapshotSpec, MAX_TITLE_BYTES, ProjectDocument, ResultProvenance, ResultSheet, ResultStatus,
    SheetId, SubcolumnMode, TableArchetype, TableKind, XColumnMode, validate_project_cells, validate_sheet_cells,
};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_format::MAX_ARCHIVE_FILE_BYTES;
use curvebook_format::registry::{Exporter, FormatKind, FormatRegistry, ImportOptions, ImportPayload, Importer};
use curvebook_format::{
    ArchiveKind, CachedBound, ImportReport, ImportWarning, ImportWarningKind, PromotedAnalysisRef, XlsxImportOptions, binary_pzf_unsupported_error,
    compare_recomputed_against_reference, import_prism_bytes, import_pzfx_bytes, import_xlsx_bytes, is_binary_pzf_bytes,
    open_archive_bytes_with_warnings,
};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_format::{is_unpacked_project_dir, open_project_unpacked_dir_with_warnings};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_platform::PlatformError;
use serde_json::Value;

use crate::{EngineError, dag};

const MAX_DISAGREEMENT_WARNINGS_PER_ANALYSIS: usize = 16;

/// Loaded project document paired with its detected container format tag and import warnings.
#[derive(Debug, Clone)]
pub struct LoadedProject {
    /// Decoded or synthesized project document.
    pub doc: ProjectDocument,
    /// Canonical format descriptor ID (`"curvebook"`, `"curvebook.d"`, `"cbtemplate"`, `"xlsx"`, `.prism`, `"pzfx"`, `"csv"`, `"tsv"`, `"json"`).
    pub format_tag: &'static str,
    /// Non-fatal import warnings collected during ingestion.
    pub warnings: Vec<String>,
}

/// Return `true` when `path` carries a `.curvebook.d` directory suffix (case-insensitive).
#[must_use]
pub fn has_unpacked_dir_suffix(path: &Path) -> bool {
    path.file_name().and_then(std::ffi::OsStr::to_str).is_some_and(|name| name.to_ascii_lowercase().ends_with(".curvebook.d"))
}

/// Return `true` when `path` refers to an unpacked `.curvebook.d` directory (either by
/// `.curvebook.d` suffix or an existing unpacked `manifest.json` marker), without classifying
/// arbitrary directories such as `.`.
#[must_use]
pub fn is_unpacked_project_path(path: &Path) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        has_unpacked_dir_suffix(path) || is_unpacked_project_dir(path)
    }
    #[cfg(target_arch = "wasm32")]
    {
        has_unpacked_dir_suffix(path)
    }
}

/// Compute total file byte size inside an unpacked directory tree.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::IoError`] if directory traversal or metadata lookup fails,
/// or [`ErrorCode::UnsupportedFormat`] on `wasm32`.
pub fn dir_size_bytes(root: &Path) -> Result<usize, EngineError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut total = 0_u64;
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let entries = fs::read_dir(&dir).map_err(|err| {
                EngineError::with_code(ErrorCode::IoError, format!("failed to read `{}`: {err}", dir.display())).with_retryable(true)
            })?;
            for entry in entries {
                let entry = entry.map_err(|err| {
                    EngineError::with_code(ErrorCode::IoError, format!("failed to inspect `{}`: {err}", dir.display())).with_retryable(true)
                })?;
                let meta = entry.metadata().map_err(|err| {
                    EngineError::with_code(ErrorCode::IoError, format!("failed to stat `{}`: {err}", entry.path().display())).with_retryable(true)
                })?;
                if meta.is_dir() {
                    stack.push(entry.path());
                } else {
                    total = total.saturating_add(meta.len());
                }
            }
        }
        Ok(usize::try_from(total).unwrap_or(usize::MAX))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = root;
        Err(EngineError::with_code(ErrorCode::UnsupportedFormat, "directory inspection is not supported on wasm32"))
    }
}

/// Resolve an [`Importer`] from `registry` using an explicit format keyword or path matching.
///
/// When `allow_txt_as_csv` is `true`, `.txt` files fall back to the registered `"csv"` importer.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::UnsupportedFormat`] or [`ErrorCode::BadInput`] if no
/// registered importer matches.
pub fn resolve_open_importer<'a>(
    registry: &'a FormatRegistry,
    explicit: Option<&str>,
    path: &Path,
    allow_txt_as_csv: bool,
) -> Result<&'a dyn Importer, EngineError> {
    if let Some(fmt) = explicit {
        if fmt.trim().trim_start_matches('.').eq_ignore_ascii_case("pzf") {
            return Err(EngineError::from(binary_pzf_unsupported_error()));
        }
        return registry
            .importer_by_format(fmt)
            .ok_or_else(|| EngineError::with_code(ErrorCode::UnsupportedFormat, format!("unsupported open format '{fmt}'")));
    }
    if let Some(found) = registry.importer_for_path(path) {
        return Ok(found);
    }
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("pzf") => Err(EngineError::from(binary_pzf_unsupported_error())),
        Some("txt") if allow_txt_as_csv => {
            registry.importer_by_format("csv").ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "CSV importer unavailable"))
        }
        None | Some("") => Err(EngineError::with_code(ErrorCode::BadInput, "cannot infer file format without extension; specify \"format\"")),
        Some(other) => Err(EngineError::with_code(ErrorCode::UnsupportedFormat, format!("unsupported file extension '.{other}'"))),
    }
}

/// Resolve a project save [`Exporter`] (`curvebook`, `curvebook.d`, or `cbtemplate`) from `registry`.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::UnsupportedFormat`] or [`ErrorCode::BadInput`] if an
/// explicit format is unsupported or `path` is an existing non-project directory.
pub fn resolve_project_save_exporter<'a>(registry: &'a FormatRegistry, explicit: Option<&str>, path: &Path) -> Result<&'a dyn Exporter, EngineError> {
    if let Some(fmt) = explicit {
        if !matches!(fmt, "curvebook" | "curvebook.d" | "cbtemplate") {
            return Err(EngineError::with_code(ErrorCode::UnsupportedFormat, format!("unsupported save format '{fmt}'")));
        }
        return registry
            .exporter_by_format(fmt)
            .ok_or_else(|| EngineError::with_code(ErrorCode::UnsupportedFormat, format!("unsupported save format '{fmt}'")));
    }
    #[cfg(not(target_arch = "wasm32"))]
    if !has_unpacked_dir_suffix(path) && !is_unpacked_project_dir(path) && path.is_dir() {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("save_file target `{}` is a directory without a `.curvebook.d` suffix", path.display()),
        ));
    }
    #[cfg(not(target_arch = "wasm32"))]
    let allow_unpacked = has_unpacked_dir_suffix(path) || is_unpacked_project_dir(path);
    #[cfg(target_arch = "wasm32")]
    let allow_unpacked = has_unpacked_dir_suffix(path);

    if let Some(found) = registry.exporter_for_path(path)
        && matches!(found.descriptor().id, "curvebook" | "curvebook.d" | "cbtemplate")
        && (found.descriptor().id != "curvebook.d" || allow_unpacked)
    {
        return Ok(found);
    }
    registry.exporter_by_format("curvebook").ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "curvebook exporter unavailable"))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_regular_file_for_import(path: &Path, limit: usize) -> Result<Vec<u8>, EngineError> {
    curvebook_platform::fs::read_regular_file_bytes(path, limit).map_err(|err| match err {
        PlatformError::InvalidPath { reason } => EngineError::with_code(ErrorCode::BadInput, format!("invalid file `{}`: {reason}", path.display())),
        other => EngineError::from(other),
    })
}

/// Dispatch an [`Importer`] against an already-resolved filesystem `resolved_path`, returning the
/// decoded [`ImportPayload`] plus optional buffered bytes for non-directory formats.
///
/// # Errors
/// Returns [`EngineError`] on invalid regular files, oversized files, decode failures, or `wasm32`.
pub fn import_registered_path(
    importer: &dyn Importer,
    resolved_path: &Path,
    import_opts: &ImportOptions,
) -> Result<(ImportPayload, Option<Vec<u8>>), EngineError> {
    if importer.descriptor().id == "curvebook.d" {
        let payload = importer.import_path(resolved_path, import_opts)?;
        return Ok((payload, None));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let raw_bytes = read_regular_file_for_import(resolved_path, importer.max_input_bytes())?;
        let payload = importer.import_bytes(&raw_bytes, import_opts)?;
        Ok((payload, Some(raw_bytes)))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (resolved_path, import_opts);
        Err(EngineError::with_code(ErrorCode::IoError, "filesystem operations are not supported on wasm32"))
    }
}

/// Load a [`ProjectDocument`] from a regular file or unpacked `.curvebook.d` directory, returning
/// `(loaded_project, input_bytes)`.
///
/// # Errors
/// Returns [`EngineError`] if reading, regular-file validation, size caps, or format decoding fail,
/// or if a delimited CSV/TSV file is passed with `allow_delimited == false`.
pub fn load_project_from_path(path: &Path, allow_delimited: bool) -> Result<(LoadedProject, usize), EngineError> {
    load_project_from_path_with_xlsx_sheets(path, allow_delimited, None)
}

/// Load a [`ProjectDocument`] from a regular file or unpacked `.curvebook.d` directory, optionally
/// restricting `.xlsx` imports to specific worksheet tab names.
///
/// # Errors
/// Returns [`EngineError`] if reading, regular-file validation, size caps, or format decoding fail,
/// or if a delimited CSV/TSV file is passed with `allow_delimited == false`.
pub fn load_project_from_path_with_xlsx_sheets(
    path: &Path,
    allow_delimited: bool,
    xlsx_sheets: Option<&[String]>,
) -> Result<(LoadedProject, usize), EngineError> {
    let opts = xlsx_sheets.map(|sheets| XlsxImportOptions { sheet_names: Some(sheets.to_vec()), ..XlsxImportOptions::default() });
    load_project_from_path_with_xlsx_options(path, allow_delimited, opts.as_ref())
}

/// Load a [`ProjectDocument`] from a regular file or unpacked `.curvebook.d` directory, optionally
/// passing explicit [`XlsxImportOptions`] for `.xlsx` workbooks.
///
/// # Errors
/// Returns [`EngineError`] if reading, regular-file validation, size caps, or format decoding fail,
/// or if a delimited CSV/TSV file is passed with `allow_delimited == false`.
pub fn load_project_from_path_with_xlsx_options(
    path: &Path,
    allow_delimited: bool,
    xlsx_options: Option<&XlsxImportOptions>,
) -> Result<(LoadedProject, usize), EngineError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if is_unpacked_project_path(path) {
            let (doc, warnings) = open_project_unpacked_dir_with_warnings(path)?;
            let total_bytes = dir_size_bytes(path)?;
            return Ok((LoadedProject { doc, format_tag: "curvebook.d", warnings }, total_bytes));
        }
        let limit = FormatRegistry::default_registry().importer_for_path(path).map_or(MAX_ARCHIVE_FILE_BYTES, |importer| importer.max_input_bytes());
        let bytes = read_regular_file_for_import(path, limit)?;
        let loaded = load_project_from_bytes_with_xlsx_options(path, &bytes, allow_delimited, xlsx_options)?;
        Ok((loaded, bytes.len()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (path, allow_delimited, xlsx_options);
        Err(EngineError::with_code(ErrorCode::UnsupportedFormat, "filesystem project loading is not supported on wasm32"))
    }
}

/// Load a [`ProjectDocument`] from archive, template, `.xlsx`, `.prism`, `.pzfx`, JSON, or
/// optionally CSV/TSV bytes via [`FormatRegistry`].
///
/// # Errors
/// Returns [`EngineError`] if decoding/validation fails or if delimited input is passed with
/// `allow_delimited == false`.
pub fn load_project_from_bytes(path_hint: &Path, bytes: &[u8], allow_delimited: bool) -> Result<LoadedProject, EngineError> {
    load_project_from_bytes_with_xlsx_options(path_hint, bytes, allow_delimited, None)
}

/// Load a [`ProjectDocument`] from bytes, optionally restricting `.xlsx` imports to specific
/// worksheet tab names.
///
/// # Errors
/// Returns [`EngineError`] if decoding/validation fails or if delimited input is passed with
/// `allow_delimited == false`.
pub fn load_project_from_bytes_with_xlsx_sheets(
    path_hint: &Path,
    bytes: &[u8],
    allow_delimited: bool,
    xlsx_sheets: Option<&[String]>,
) -> Result<LoadedProject, EngineError> {
    let opts = xlsx_sheets.map(|sheets| XlsxImportOptions { sheet_names: Some(sheets.to_vec()), ..XlsxImportOptions::default() });
    load_project_from_bytes_with_xlsx_options(path_hint, bytes, allow_delimited, opts.as_ref())
}

/// Load a [`ProjectDocument`] from bytes, optionally passing explicit [`XlsxImportOptions`] for
/// `.xlsx` workbooks.
///
/// # Errors
/// Returns [`EngineError`] if decoding/validation fails or if delimited input is passed with
/// `allow_delimited == false`.
pub fn load_project_from_bytes_with_xlsx_options(
    path_hint: &Path,
    bytes: &[u8],
    allow_delimited: bool,
    xlsx_options: Option<&XlsxImportOptions>,
) -> Result<LoadedProject, EngineError> {
    let registry = FormatRegistry::default_registry();
    if let Some(importer) = registry.importer_for_path(path_hint) {
        let desc = importer.descriptor();
        return import_with_registered_adapter(importer, desc.id, desc.kind, path_hint, bytes, allow_delimited, xlsx_options);
    }
    let ext = path_hint.extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase);
    if ext.as_deref() == Some("pzf") {
        return Err(EngineError::from(binary_pzf_unsupported_error()));
    }
    if ext.as_deref() == Some("json") {
        return load_project_json(bytes);
    }
    sniff_project_bytes(bytes)
}

fn import_with_registered_adapter(
    importer: &dyn Importer,
    format_tag: &'static str,
    kind: FormatKind,
    path_hint: &Path,
    bytes: &[u8],
    allow_delimited: bool,
    xlsx_options: Option<&XlsxImportOptions>,
) -> Result<LoadedProject, EngineError> {
    match kind {
        FormatKind::Template => {
            let (kind, doc, warnings) = open_archive_bytes_with_warnings(bytes)?;
            if kind != ArchiveKind::Template {
                return Err(EngineError::from(curvebook_format::FormatError::InvalidArchiveKind {
                    expected: ArchiveKind::Template.as_str(),
                    found: kind.as_str(),
                }));
            }
            Ok(LoadedProject { doc, format_tag, warnings })
        }
        FormatKind::Archive => {
            let (_, doc, warnings) = open_archive_bytes_with_warnings(bytes)?;
            Ok(LoadedProject { doc, format_tag, warnings })
        }
        FormatKind::ForeignProject => {
            let mut opts = ImportOptions::default();
            if format_tag == "xlsx"
                && let Some(raw_xopts) = xlsx_options
            {
                let mut xopts = raw_xopts.clone();
                if xopts.title.is_none() {
                    xopts.title = path_hint.file_stem().and_then(std::ffi::OsStr::to_str).map(str::to_owned);
                }
                opts.xlsx = Some(xopts);
            }
            match importer.import_bytes(bytes, &opts)? {
                ImportPayload::Report(report) => load_import_report(report, format_tag),
                ImportPayload::Document(doc) => Ok(LoadedProject { doc, format_tag, warnings: Vec::new() }),
                ImportPayload::DataSheet(_) => {
                    Err(EngineError::with_code(ErrorCode::InternalError, format!("importer `{format_tag}` returned unexpected single-sheet payload")))
                }
            }
        }
        FormatKind::DelimitedText => {
            if !allow_delimited {
                return Err(EngineError::with_code(ErrorCode::UnsupportedFormat, "delimited CSV/TSV files are not project archives"));
            }
            load_delimited_with_importer(importer, path_hint, bytes, format_tag)
        }
        FormatKind::ProjectDir | FormatKind::ResultTable | FormatKind::FigureVector | FormatKind::FigureRaster => sniff_project_bytes(bytes),
    }
}

fn load_native_archive(bytes: &[u8]) -> Result<LoadedProject, EngineError> {
    let (kind, doc, warnings) = open_archive_bytes_with_warnings(bytes)?;
    let format_tag = match kind {
        ArchiveKind::Project => "curvebook",
        ArchiveKind::Template => "cbtemplate",
    };
    Ok(LoadedProject { doc, format_tag, warnings })
}

fn truncate_utf8_max_bytes(raw: &str, max_bytes: usize) -> &str {
    if raw.len() <= max_bytes {
        return raw;
    }
    let mut end = max_bytes;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    raw.get(..end).unwrap_or("")
}

fn strip_trailing_fraction_zeros(raw: &str) -> String {
    if let Some((int_part, frac_part)) = raw.split_once('.') {
        let trimmed = frac_part.trim_end_matches('0');
        if trimmed.is_empty() { int_part.to_owned() } else { format!("{int_part}.{trimmed}") }
    } else {
        raw.to_owned()
    }
}

fn format_warning_number(value: f64, sig_figs: usize) -> String {
    if !value.is_finite() {
        return format!("{value}");
    }
    if value == 0.0 {
        return String::from("0");
    }
    let digits = sig_figs.clamp(1, 15);
    let sci = format!("{value:.prec$e}", prec = digits - 1);
    let Some((mantissa_str, exp_str)) = sci.split_once('e') else {
        return sci;
    };
    let rounded_exp = exp_str.parse::<i32>().unwrap_or(0);
    if !(-4..6).contains(&rounded_exp) {
        let mantissa = strip_trailing_fraction_zeros(mantissa_str);
        return format!("{mantissa}e{rounded_exp}");
    }
    let decimals = usize::try_from((digits as i32 - 1 - rounded_exp).max(0)).unwrap_or(0);
    let rounded_val = sci.parse::<f64>().unwrap_or(value);
    let fixed = format!("{rounded_val:.decimals$}");
    strip_trailing_fraction_zeros(&fixed)
}

fn format_warning_pair(recomputed: f64, cached: f64) -> (String, String) {
    for sig_figs in [4_usize, 6, 9, 12] {
        let rec_str = format_warning_number(recomputed, sig_figs);
        let cached_str = format_warning_number(cached, sig_figs);
        if rec_str != cached_str || recomputed == cached {
            return (rec_str, cached_str);
        }
    }
    (format_warning_number(recomputed, 15), format_warning_number(cached, 15))
}

fn failure_message_for_status(status: &ResultStatus) -> String {
    match status {
        ResultStatus::Failed { message, .. } => {
            let trimmed = message.trim();
            if trimmed.is_empty() { String::from("recomputation failed") } else { trimmed.to_owned() }
        }
        ResultStatus::Dirty => String::from("analysis remained dirty after recalculation"),
        ResultStatus::Imported => String::from("analysis was marked imported"),
        ResultStatus::ImportedStale => String::from("analysis was marked stale imported"),
        ResultStatus::UpToDate => String::new(),
    }
}

fn fallback_promoted_result_sheet(rs: &mut ResultSheet, pref: &PromotedAnalysisRef, warnings: &mut Vec<ImportWarning>) {
    let fail_msg = failure_message_for_status(&rs.status);
    let kind_label = truncate_utf8_max_bytes(&pref.fallback.original_kind_label, MAX_TITLE_BYTES).to_owned();
    let snapshot_kind = AnalysisKind::ImportedSnapshot(ImportedSnapshotSpec { original_kind_label: kind_label.clone() });
    rs.analysis.auto_title = snapshot_kind.derive_auto_title();
    rs.analysis.kind = snapshot_kind;
    rs.analysis.selected_data_sets.clear();
    let raw_reason = format!("recomputation failed ({fail_msg}); preserved cached result");
    let reason = truncate_utf8_max_bytes(&raw_reason, MAX_TITLE_BYTES).to_owned();
    let canonical_format = ".prism".trim_start_matches('.').to_owned();
    rs.provenance = ResultProvenance::Imported { source_format: canonical_format, original_kind_label: kind_label, reason };
    rs.status = ResultStatus::Imported;
    rs.tabs = pref.fallback.tabs.clone();
    rs.scalars = pref.fallback.scalars.clone();
    rs.pairwise_comparisons.clear();
    rs.fitted_curves.clear();
    rs.eliminated_outliers.clear();
    rs.checklist.clear();
    rs.cell_details.clear();
    rs.analyzable_data = None;
    warnings.push(ImportWarning::new(
        ImportWarningKind::AnalysisRecomputeFallback,
        format!("analyses/{}", pref.analysis_uid),
        format!("analysis `{}` failed to recompute ({fail_msg}); kept cached snapshot", pref.title),
    ));
}

fn emit_disagreement_warnings(rs: &ResultSheet, pref: &PromotedAnalysisRef, warnings: &mut Vec<ImportWarning>) {
    let diag = compare_recomputed_against_reference(rs, &pref.reference, pref.abs_tol, pref.rel_tol);
    let source = format!("analyses/{}", pref.analysis_uid);
    for diff in diag.disagreements.iter().take(MAX_DISAGREEMENT_WARNINGS_PER_ANALYSIS) {
        let op = match diff.bound {
            CachedBound::Equal => "=",
            CachedBound::LessThan => "<",
            CachedBound::GreaterThan => ">",
        };
        let (rec_str, cached_str) = format_warning_pair(diff.recomputed, diff.cached);
        let msg = format!(
            "analysis `{}`: {} differs (recomputed {} = {}, file stored {} {} {}); CurveBook's value stands",
            pref.title, diff.quantity_label, diff.symbol, rec_str, diff.symbol, op, cached_str
        );
        warnings.push(ImportWarning::new(ImportWarningKind::RecomputedResultDisagreement, source.clone(), msg));
    }
    if diag.disagreements.len() > MAX_DISAGREEMENT_WARNINGS_PER_ANALYSIS {
        let remaining = diag.disagreements.len().saturating_sub(MAX_DISAGREEMENT_WARNINGS_PER_ANALYSIS);
        warnings.push(ImportWarning::new(
            ImportWarningKind::RecomputedResultDisagreement,
            source,
            format!("analysis `{}`: {remaining} additional recomputed value(s) differed from stored results; CurveBook's values stand", pref.title),
        ));
    }
}

/// Recalculate promoted live `.prism` analyses inside `doc`, compare recomputed results against
/// transient cached references (emitting [`ImportWarningKind::RecomputedResultDisagreement`] while
/// always keeping CurveBook's recomputed values), and convert any failing promoted sheet in place
/// back to its preserved [`AnalysisKind::ImportedSnapshot`] fallback with
/// [`ImportWarningKind::AnalysisRecomputeFallback`].
///
/// Returns `Ok(None)` when `promoted` references no live computed result sheets in `doc`.
///
/// # Errors
/// Returns [`EngineError`] if topological DAG recalculation fails or is cancelled.
pub fn finalize_promoted_imported_analyses(
    doc: &mut ProjectDocument,
    promoted: &[PromotedAnalysisRef],
    warnings: &mut Vec<ImportWarning>,
    cancelled: Option<&AtomicBool>,
) -> Result<Option<dag::RecalcReport>, EngineError> {
    let live_ids: Vec<SheetId> = promoted
        .iter()
        .filter_map(|pref| {
            doc.result_sheet(&pref.sheet_id)
                .filter(|rs| matches!(rs.provenance, ResultProvenance::Computed) && !rs.is_imported())
                .map(|rs| rs.id.clone())
        })
        .collect();
    if live_ids.is_empty() {
        return Ok(None);
    }

    let live_id_set: HashSet<SheetId> = live_ids.iter().cloned().collect();
    let (mut recalc_report, _refreshed_links) = dag::recalculate_dirty_with_links_report(doc, &live_ids, None, cancelled)?;
    for pref in promoted {
        if !live_id_set.contains(&pref.sheet_id) {
            continue;
        }
        let Some(rs) = doc.result_sheet_mut(&pref.sheet_id) else {
            continue;
        };
        if matches!(rs.status, ResultStatus::UpToDate) {
            emit_disagreement_warnings(rs, pref, warnings);
        } else {
            fallback_promoted_result_sheet(rs, pref, warnings);
            recalc_report.failed_result_sheets.retain(|id| id != &pref.sheet_id);
        }
    }
    Ok(Some(recalc_report))
}

/// Finalize an [`ImportReport`] by recomputing any promoted live `.prism` analyses and appending
/// any disagreement or snapshot-fallback warnings to the returned warning list.
///
/// # Errors
/// Returns [`EngineError`] if recalculating promoted analyses fails.
pub fn finalize_import_report(mut report: ImportReport) -> Result<(ProjectDocument, Vec<ImportWarning>), EngineError> {
    let _ = finalize_promoted_imported_analyses(&mut report.document, &report.promoted_analyses, &mut report.warnings, None)?;
    Ok((report.document, report.warnings))
}

fn load_import_report(report: ImportReport, format_tag: &'static str) -> Result<LoadedProject, EngineError> {
    let (doc, raw_warnings) = finalize_import_report(report)?;
    let warnings =
        raw_warnings
            .into_iter()
            .map(|w| {
                if w.source.trim().is_empty() {
                    format!("[{:?}] {}", w.kind, w.message)
                } else {
                    format!("[{:?}] {}: {}", w.kind, w.source, w.message)
                }
            })
            .collect();
    Ok(LoadedProject { doc, format_tag, warnings })
}

fn load_project_json(bytes: &[u8]) -> Result<LoadedProject, EngineError> {
    let raw_value: Value =
        serde_json::from_slice(bytes).map_err(|err| EngineError::with_code(ErrorCode::CorruptData, format!("invalid project JSON: {err}")))?;
    if !raw_value.is_object() {
        return Err(EngineError::with_code(ErrorCode::CorruptData, "invalid project JSON: expected top-level JSON object"));
    }
    let doc_version = match raw_value.get("schema_version") {
        None | Some(Value::Null) => 0,
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| EngineError::with_code(ErrorCode::CorruptData, format!("invalid project `schema_version`: {v}")))?,
    };
    let migrated_value = curvebook_format::archive::migrate_document_json(raw_value, doc_version)?;
    let mut doc: ProjectDocument = serde_json::from_value(migrated_value)
        .map_err(|err| EngineError::with_code(ErrorCode::CorruptData, format!("invalid project JSON: {err}")))?;
    let warnings = doc.clear_dangling_selection_ids();
    validate_loaded_project_document(&doc)?;
    Ok(LoadedProject { doc, format_tag: "json", warnings })
}

fn validate_loaded_project_document(doc: &ProjectDocument) -> Result<(), EngineError> {
    curvebook_format::archive::validate_project_document(doc)?;
    let mut total_cells = 0_usize;
    for sheet in &doc.data_sheets {
        if let TableArchetype::Xy(xy) = &sheet.archetype
            && let XColumnMode::Numbers { subcolumn_mode } = xy.x_mode
        {
            subcolumn_mode.validate()?;
        }
        let slots = sheet.archetype.allocated_cell_slots();
        validate_sheet_cells(slots)?;
        total_cells = validate_project_cells(total_cells, slots)?;
    }
    Ok(())
}

fn load_delimited_with_importer(
    importer: &dyn Importer,
    path_hint: &Path,
    bytes: &[u8],
    format_tag: &'static str,
) -> Result<LoadedProject, EngineError> {
    let stem = path_hint.file_stem().and_then(std::ffi::OsStr::to_str).filter(|s| !s.trim().is_empty()).unwrap_or("Imported Table");
    let opts = ImportOptions {
        sheet_id: SheetId::new("data_1"),
        title: stem.to_owned(),
        table_kind: TableKind::Xy,
        subcolumn_mode: SubcolumnMode::Single,
        csv: None,
        xlsx: None,
    };
    let ds = importer
        .import_bytes(bytes, &opts)?
        .into_data_sheet()
        .ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, format!("delimited importer `{format_tag}` did not return a DataSheet")))?;
    let mut doc = ProjectDocument::new(stem)?;
    let mut graph = GraphSheet::new_uninitialized_for_data(SheetId::new("graph_1"), &ds)?;
    graph.initialized = true;
    doc.add_data_sheet(ds)?;
    doc.add_graph_sheet(graph)?;
    Ok(LoadedProject { doc, format_tag, warnings: Vec::new() })
}

fn sniff_project_bytes(bytes: &[u8]) -> Result<LoadedProject, EngineError> {
    if is_binary_pzf_bytes(bytes) {
        return Err(EngineError::from(binary_pzf_unsupported_error()));
    }
    if bytes.starts_with(b"PK\x03\x04") {
        if let Ok(loaded) = load_native_archive(bytes) {
            return Ok(loaded);
        }
        if let Ok(report) = import_xlsx_bytes(bytes) {
            return load_import_report(report, "xlsx");
        }
        return load_import_report(import_prism_bytes(bytes)?, "prism");
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let trimmed = body.iter().copied().find(|b| !b.is_ascii_whitespace());
    match trimmed {
        Some(b'<') => load_import_report(import_pzfx_bytes(bytes)?, "pzfx"),
        Some(b'{') => load_project_json(body),
        _ => Err(EngineError::with_code(ErrorCode::UnsupportedFormat, "unrecognized input file format")),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, ImportedSnapshotSpec, MAX_TITLE_BYTES,
        MultiplicityMethod, MultiplicitySpec, OneWayAnovaSpec, OneWayMethod, ProjectDocument, ResultCell, ResultProvenance, ResultScalarEntry,
        ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetId, SheetOrigin, TableArchetype, TwoGroupMethod, TwoGroupSpec,
    };
    use curvebook_format::{
        BINARY_PZF_UNSUPPORTED_MESSAGE, CachedAnalysisReference, CachedBound, CachedPairwiseRef, CachedScalarRef, CrosswalkComparison,
        FormatRegistry, ImportWarningKind, ImportedAnalysisFallback, PromotedAnalysisRef,
    };
    use curvebook_palette::Rgba8;

    use super::{
        finalize_promoted_imported_analyses, has_unpacked_dir_suffix, load_project_from_bytes, resolve_open_importer, resolve_project_save_exporter,
    };

    fn column_sheet(id: &str, title: &str, groups: &[(&str, &[f64])]) -> DataSheet {
        let max_rows = groups.iter().map(|(_, vals)| vals.len()).max().unwrap_or(1).max(1);
        let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, groups.len(), max_rows).expect("column table");
        for (col_idx, (col_title, vals)) in groups.iter().enumerate() {
            if let Some(ds_col) = table.data_sets.get_mut(col_idx) {
                ds_col.title = (*col_title).to_owned();
                for (row_idx, &v) in vals.iter().enumerate() {
                    if let Some(cell) = ds_col.cells.get_mut(row_idx).and_then(|sub| sub.first_mut()) {
                        *cell = Cell::number(v);
                    }
                }
            }
        }
        DataSheet::new(id, title, TableArchetype::Column(table)).expect("data sheet")
    }

    #[test]
    fn finalize_promoted_imported_analyses_agreeing_analysis_recomputes_live_with_zero_warnings() {
        let mut doc = ProjectDocument::new("Agreeing Study").expect("doc");
        doc.add_data_sheet(column_sheet("data-1", "Groups", &[("Control", &[10.0, 12.0, 14.0]), ("Treated", &[20.0, 22.0, 24.0])])).expect("add ds");

        let mut spec = AnalysisSpec::new(
            "res-ttest",
            "data-1",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, two_tailed: true, ..TwoGroupSpec::default() }),
        )
        .expect("spec");
        spec.set_custom_title(Some(String::from("Unpaired t test"))).expect("title");
        doc.add_result_sheet(ResultSheet::new(spec).expect("rs")).expect("add rs");

        // Compute exact reference values on a scratch copy so the agreeing reference matches within tolerance.
        let mut oracle = doc.clone();
        crate::dag::recalculate_project(&mut oracle).expect("oracle recalc");
        let oracle_rs = oracle.result_sheet(&SheetId::new("res-ttest")).expect("oracle rs");
        let expected_p = oracle_rs.lookup_scalar("PValue", None).expect("PValue");
        let expected_t = oracle_rs.lookup_scalar("t", None).expect("t");

        let pref = PromotedAnalysisRef {
            sheet_id: SheetId::new("res-ttest"),
            analysis_uid: String::from("uid-agree"),
            title: String::from("Unpaired t test"),
            foreign_class: String::from("T_TESTS_AND_NONPARAMETRIC"),
            abs_tol: 1e-9,
            rel_tol: 1e-6,
            reference: CachedAnalysisReference {
                scalars: vec![
                    CachedScalarRef {
                        key: String::from("PValue"),
                        data_set_index: None,
                        value: expected_p,
                        bound: CachedBound::Equal,
                        comparison: CrosswalkComparison::Signed,
                    },
                    CachedScalarRef {
                        key: String::from("t"),
                        data_set_index: None,
                        value: expected_t.abs(),
                        bound: CachedBound::Equal,
                        comparison: CrosswalkComparison::Magnitude,
                    },
                ],
                pairwise: Vec::new(),
                ..CachedAnalysisReference::default()
            },
            fallback: ImportedAnalysisFallback {
                original_kind_label: String::from("T_TESTS_AND_NONPARAMETRIC"),
                tabs: Vec::new(),
                scalars: Vec::new(),
            },
        };

        let mut warnings = Vec::new();
        let report = finalize_promoted_imported_analyses(&mut doc, &[pref], &mut warnings, None).expect("finalize");
        let report = report.expect("expected recalc report");
        assert!(report.failed_result_sheets.is_empty());
        assert!(warnings.is_empty(), "expected zero warnings on agreeing analysis, got {warnings:?}");

        let rs = doc.result_sheet(&SheetId::new("res-ttest")).expect("result sheet");
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert_eq!(rs.provenance, ResultProvenance::Computed);
        assert!((rs.lookup_scalar("PValue", None).expect("recomputed PValue") - expected_p).abs() <= 1e-12);

        // Empty promoted list is a no-op returning Ok(None).
        let noop = finalize_promoted_imported_analyses(&mut doc, &[], &mut warnings, None).expect("noop");
        assert!(noop.is_none());
    }

    #[test]
    fn finalize_promoted_imported_analyses_disagreements_warn_and_keep_curvebook_values() {
        let mut doc = ProjectDocument::new("ANOVA Study").expect("doc");
        doc.add_data_sheet(column_sheet(
            "data-1",
            "Dose Groups",
            &[("Vehicle", &[1.0, 2.0, 3.0]), ("Low", &[5.0, 6.0, 7.0]), ("High", &[20.0, 21.0, 22.0])],
        ))
        .expect("add ds");

        let mut spec = AnalysisSpec::new(
            "res-anova",
            "data-1",
            AnalysisKind::OneWayAnova(OneWayAnovaSpec {
                method: OneWayMethod::OrdinaryAnova,
                multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..MultiplicitySpec::default() },
                ..OneWayAnovaSpec::default()
            }),
        )
        .expect("spec");
        spec.set_custom_title(Some(String::from("Dose ANOVA"))).expect("title");
        doc.add_result_sheet(ResultSheet::new(spec).expect("rs")).expect("add rs");

        let pref = PromotedAnalysisRef {
            sheet_id: SheetId::new("res-anova"),
            analysis_uid: String::from("uid-anova"),
            title: String::from("Dose ANOVA"),
            foreign_class: String::from("ONE_WAY_ANOVA"),
            abs_tol: 1e-9,
            rel_tol: 1e-6,
            reference: CachedAnalysisReference {
                scalars: vec![CachedScalarRef {
                    key: String::from("P value"),
                    data_set_index: None,
                    value: 0.156,
                    bound: CachedBound::Equal,
                    comparison: CrosswalkComparison::Signed,
                }],
                pairwise: vec![CachedPairwiseRef {
                    col_a: 0,
                    col_b: 1,
                    p_or_q: Some((1e-15, CachedBound::LessThan, false)),
                    means_diff: Some(99.0),
                    p_abs_tol: None,
                }],
                ..CachedAnalysisReference::default()
            },
            fallback: ImportedAnalysisFallback { original_kind_label: String::from("ONE_WAY_ANOVA"), tabs: Vec::new(), scalars: Vec::new() },
        };

        let mut warnings = Vec::new();
        let report = finalize_promoted_imported_analyses(&mut doc, &[pref], &mut warnings, None).expect("finalize");
        assert!(report.is_some());

        let rs = doc.result_sheet(&SheetId::new("res-anova")).expect("result sheet");
        assert_eq!(rs.status, ResultStatus::UpToDate);
        assert_eq!(rs.provenance, ResultProvenance::Computed);

        // CurveBook's recomputed values must stand (never overwritten by stale cached references).
        let live_p = rs.lookup_scalar("P value", None).expect("live omnibus P");
        assert!(live_p < 1e-3, "expected significant omnibus P value, got {live_p}");
        let pair = rs.pairwise_comparisons.iter().find(|c| (c.group_a.min(c.group_b), c.group_a.max(c.group_b)) == (0, 1)).expect("Vehicle vs Low");
        assert!(pair.raw_p_or_q > 1e-6, "expected Vehicle vs Low Tukey p > 1e-6, got {}", pair.raw_p_or_q);
        assert!((pair.estimate_diff.expect("mean diff").abs() - 4.0).abs() < 1e-9);

        assert_eq!(warnings.len(), 3, "expected 1 scalar + 2 pairwise warnings, got {warnings:?}");
        assert!(warnings.iter().all(|w| w.kind == ImportWarningKind::RecomputedResultDisagreement && w.source == "analyses/uid-anova"));
        assert!(
            warnings.iter().any(|w| w.message.contains("Dose ANOVA")
                && w.message.contains("P value")
                && w.message.contains("file stored p = 0.156")
                && w.message.contains("CurveBook's value stands")),
            "missing omnibus P disagreement warning: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| {
                w.message.contains("Dose ANOVA")
                    && w.message.contains("Vehicle vs Low adjusted p-value")
                    && w.message.contains("file stored p < 1e-15")
                    && w.message.contains("CurveBook's value stands")
            }),
            "missing P_VALUE_LESS pairwise warning: {warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| {
                w.message.contains("Dose ANOVA")
                    && w.message.contains("Vehicle vs Low mean difference")
                    && w.message.contains("file stored |mean diff| = 99")
                    && w.message.contains("CurveBook's value stands")
            }),
            "missing pairwise mean-diff warning: {warnings:?}"
        );

        // Exceeding MAX_DISAGREEMENT_WARNINGS_PER_ANALYSIS caps individual warnings at 16 + 1 summary.
        let many_refs: Vec<CachedScalarRef> = (0..18)
            .map(|i| CachedScalarRef {
                key: String::from("P value"),
                data_set_index: None,
                value: 0.2 + (i as f64) * 0.01,
                bound: CachedBound::Equal,
                comparison: CrosswalkComparison::Signed,
            })
            .collect();
        let capped_pref = PromotedAnalysisRef {
            sheet_id: SheetId::new("res-anova"),
            analysis_uid: String::from("uid-capped"),
            title: String::from("Dose ANOVA"),
            foreign_class: String::from("ONE_WAY_ANOVA"),
            abs_tol: 1e-9,
            rel_tol: 1e-6,
            reference: CachedAnalysisReference { scalars: many_refs, ..CachedAnalysisReference::default() },
            fallback: ImportedAnalysisFallback { original_kind_label: String::from("ONE_WAY_ANOVA"), tabs: Vec::new(), scalars: Vec::new() },
        };
        let mut capped_warnings = Vec::new();
        let _ = finalize_promoted_imported_analyses(&mut doc, &[capped_pref], &mut capped_warnings, None).expect("capped finalize");
        assert_eq!(capped_warnings.len(), 17, "expected 16 individual + 1 summary warning");
        assert!(capped_warnings.last().is_some_and(|w| w.message.contains("2 additional")));
    }

    #[test]
    fn finalize_promoted_imported_analyses_failing_recompute_falls_back_to_snapshot_preserving_metadata() {
        let mut doc = ProjectDocument::new("Degenerate Study").expect("doc");
        // 1-group table cannot run a two-group unpaired t-test -> recalculation records ResultStatus::Failed.
        doc.add_data_sheet(column_sheet("data-1", "Single Group", &[("OnlyGroup", &[1.0, 2.0, 3.0])])).expect("add ds");

        let mut spec = AnalysisSpec::new(
            "res-degenerate",
            "data-1",
            AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::UnpairedT, two_tailed: true, ..TwoGroupSpec::default() }),
        )
        .expect("spec");
        spec.selected_data_sets = vec![0];
        spec.set_custom_title(Some(String::from("Broken T-Test"))).expect("custom title");

        let expected_color = Some(Rgba8::rgba(12, 34, 56, 255));
        let canonical_format = ".prism".trim_start_matches('.');
        let expected_origin = Some(SheetOrigin::new("study.prism", canonical_format, "unix:1700000000").expect("origin"));
        let mut rs = ResultSheet::new(spec).expect("rs");
        rs.color_tag = expected_color;
        rs.origin = expected_origin.clone();
        doc.add_result_sheet(rs).expect("add rs");

        let cached_tab = ResultTab {
            id: String::from("cached-tab-1"),
            title: String::from("Cached Summary"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("Parameter"), String::from("Value")],
            row_labels: vec![String::from("P value")],
            rows: vec![vec![ResultCell::Text(String::from("P value")), ResultCell::Number { value: 0.042, prefix: None, units: None }]],
            footnotes: Vec::new(),
        };
        let cached_scalar = ResultScalarEntry { key: String::from("PValue"), data_set_index: None, value: 0.042, units: None };
        let pref = PromotedAnalysisRef {
            sheet_id: SheetId::new("res-degenerate"),
            analysis_uid: String::from("uid-bad"),
            title: String::from("Broken T-Test"),
            foreign_class: String::from("T_TESTS_AND_NONPARAMETRIC"),
            abs_tol: 1e-9,
            rel_tol: 1e-6,
            reference: CachedAnalysisReference::default(),
            fallback: ImportedAnalysisFallback {
                original_kind_label: String::from("T_TESTS_AND_NONPARAMETRIC"),
                tabs: vec![cached_tab.clone()],
                scalars: vec![cached_scalar.clone()],
            },
        };

        let mut warnings = Vec::new();
        let report = finalize_promoted_imported_analyses(&mut doc, &[pref], &mut warnings, None).expect("finalize");
        let report = report.expect("expected recalc report");
        assert!(
            !report.failed_result_sheets.contains(&SheetId::new("res-degenerate")),
            "fallen-back sheet must be stripped from failed_result_sheets: {:?}",
            report.failed_result_sheets
        );

        let after = doc.result_sheet(&SheetId::new("res-degenerate")).expect("fallback sheet");
        assert_eq!(after.id, SheetId::new("res-degenerate"));
        assert_eq!(after.origin, expected_origin);
        assert_eq!(after.color_tag, expected_color);
        assert_eq!(after.analysis.source_sheet_id, Some(SheetId::new("data-1")));
        assert_eq!(after.analysis.custom_title.as_deref(), Some("Broken T-Test"));
        assert_eq!(
            after.analysis.kind,
            AnalysisKind::ImportedSnapshot(ImportedSnapshotSpec { original_kind_label: String::from("T_TESTS_AND_NONPARAMETRIC") })
        );
        assert!(after.analysis.selected_data_sets.is_empty());
        assert_eq!(after.status, ResultStatus::Imported);
        match &after.provenance {
            ResultProvenance::Imported { source_format, original_kind_label, reason } => {
                assert_eq!(source_format, canonical_format);
                assert_eq!(original_kind_label, "T_TESTS_AND_NONPARAMETRIC");
                assert!(reason.contains("recomputation failed") && reason.contains("preserved cached result"), "unexpected reason: {reason}");
                assert!(reason.len() <= MAX_TITLE_BYTES);
            }
            ResultProvenance::Computed => panic!("expected Imported provenance after fallback"),
        }
        assert_eq!(after.tabs, vec![cached_tab]);
        assert_eq!(after.scalars, vec![cached_scalar]);
        assert!(after.pairwise_comparisons.is_empty());
        assert!(after.fitted_curves.is_empty());
        assert!(after.checklist.is_empty());
        assert!(after.analyzable_data.is_none());
        assert!(after.validate().is_ok(), "fallback snapshot must satisfy ResultSheet::validate");

        assert_eq!(warnings.len(), 1);
        let warning = warnings.first().expect("fallback warning");
        assert_eq!(warning.kind, ImportWarningKind::AnalysisRecomputeFallback);
        assert_eq!(warning.source, "analyses/uid-bad");
        assert!(
            warning.message.contains("Broken T-Test") && warning.message.contains("kept cached snapshot"),
            "unexpected fallback warning: {}",
            warning.message
        );
    }

    #[test]
    fn open_importer_and_save_exporter_resolution_rejects_legacy_and_supports_txt() {
        let reg = FormatRegistry::default_registry();

        let txt = resolve_open_importer(&reg, None, Path::new("assay.txt"), true).expect("txt falls back to csv");
        assert_eq!(txt.descriptor().id, "csv");

        let no_txt = resolve_open_importer(&reg, None, Path::new("assay.txt"), false).map(|imp| imp.descriptor().id).expect_err("txt disallowed");
        assert_eq!(no_txt.code, ErrorCode::UnsupportedFormat);

        let no_ext = resolve_open_importer(&reg, None, Path::new("README"), true).map(|imp| imp.descriptor().id).expect_err("missing extension");
        assert_eq!(no_ext.code, ErrorCode::BadInput);

        let legacy = resolve_project_save_exporter(&reg, Some("lumencraft"), Path::new("out.curvebook"))
            .map(|exp| exp.descriptor().id)
            .expect_err("legacy rejected");
        assert_eq!(legacy.code, ErrorCode::UnsupportedFormat);

        assert!(has_unpacked_dir_suffix(Path::new("study.CURVEBOOK.D")));
        assert!(!has_unpacked_dir_suffix(Path::new("study.curvebook")));
    }

    #[test]
    fn pzf_extension_and_pcff_bytes_refuse_with_canonical_unsupported_message() {
        let reg = FormatRegistry::default_registry();
        assert!(reg.importer_by_format("pzf").is_none());
        assert!(reg.importer_for_path(Path::new("legacy.pzf")).is_none());

        for (explicit, path) in [
            (None, Path::new("legacy.pzf")),
            (None, Path::new("LEGACY.PZF")),
            (Some("pzf"), Path::new("arbitrary.bin")),
            (Some(".PZF"), Path::new("arbitrary.bin")),
        ] {
            let err = resolve_open_importer(&reg, explicit, path, true).map(|imp| imp.descriptor().id).expect_err("pzf must be refused");
            assert_eq!(err.code, ErrorCode::UnsupportedFormat);
            assert!(err.message().contains(BINARY_PZF_UNSUPPORTED_MESSAGE), "unexpected message: {}", err.message());
        }

        let pcff_non_utf8 = b"PCFF\x00\x01\xFF\xFEbinary-payload";
        let by_pzfx = load_project_from_bytes(Path::new("misnamed.pzfx"), pcff_non_utf8, false).expect_err("PCFF .pzfx refused");
        let by_sniff = load_project_from_bytes(Path::new("blob"), pcff_non_utf8, false).expect_err("PCFF sniff refused");
        let by_pzf_ext = load_project_from_bytes(Path::new("legacy.pzf"), b"\x00\xFFnot-pcff", false).expect_err(".pzf ext refused");
        for err in [&by_pzfx, &by_sniff, &by_pzf_ext] {
            assert_eq!(err.code, ErrorCode::UnsupportedFormat);
            assert!(err.message().contains(BINARY_PZF_UNSUPPORTED_MESSAGE), "unexpected message: {}", err.message());
        }
        assert_eq!(by_pzfx.message(), by_sniff.message());
        assert_eq!(by_pzfx.message(), by_pzf_ext.message());
    }

    #[test]
    fn load_project_from_bytes_supports_delimited_when_allowed_and_refuses_when_disallowed() {
        let csv_bytes = b"X,Y\n1,2\n3,4\n";
        let loaded = load_project_from_bytes(Path::new("dose.csv"), csv_bytes, true).expect("delimited allowed");
        assert_eq!(loaded.format_tag, "csv");
        assert_eq!(loaded.doc.data_sheets.len(), 1);
        assert_eq!(loaded.doc.graph_sheets.len(), 1);

        let denied = load_project_from_bytes(Path::new("dose.csv"), csv_bytes, false).expect_err("delimited denied");
        assert_eq!(denied.code, ErrorCode::UnsupportedFormat);
    }
}
