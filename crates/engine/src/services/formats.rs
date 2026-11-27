//! Shared `FormatRegistry` import/export dispatch and project-loading services.

#[cfg(not(target_arch = "wasm32"))]
use std::fs;
use std::path::Path;

use curvebook_doc::{
    ErrorCode, GraphSheet, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, TableKind, XColumnMode, validate_project_cells,
    validate_sheet_cells,
};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_format::MAX_ARCHIVE_FILE_BYTES;
use curvebook_format::registry::{Exporter, FormatKind, FormatRegistry, ImportOptions, ImportPayload, Importer};
use curvebook_format::{
    ArchiveKind, ImportReport, XlsxImportOptions, binary_pzf_unsupported_error, import_prism_bytes, import_pzfx_bytes, import_xlsx_bytes,
    is_binary_pzf_bytes, open_archive_bytes_with_warnings,
};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_format::{is_unpacked_project_dir, open_project_unpacked_dir_with_warnings};
#[cfg(not(target_arch = "wasm32"))]
use curvebook_platform::PlatformError;
use serde_json::Value;

use crate::EngineError;

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
                ImportPayload::Report(report) => Ok(load_import_report(report, format_tag)),
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

fn load_import_report(report: ImportReport, format_tag: &'static str) -> LoadedProject {
    let warnings =
        report
            .warnings
            .into_iter()
            .map(|w| {
                if w.source.trim().is_empty() {
                    format!("[{:?}] {}", w.kind, w.message)
                } else {
                    format!("[{:?}] {}: {}", w.kind, w.source, w.message)
                }
            })
            .collect();
    LoadedProject { doc: report.document, format_tag, warnings }
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
            return Ok(load_import_report(report, "xlsx"));
        }
        return Ok(load_import_report(import_prism_bytes(bytes)?, "prism"));
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let trimmed = body.iter().copied().find(|b| !b.is_ascii_whitespace());
    match trimmed {
        Some(b'<') => Ok(load_import_report(import_pzfx_bytes(bytes)?, "pzfx")),
        Some(b'{') => load_project_json(body),
        _ => Err(EngineError::with_code(ErrorCode::UnsupportedFormat, "unrecognized input file format")),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use curvebook_doc::ErrorCode;
    use curvebook_format::{BINARY_PZF_UNSUPPORTED_MESSAGE, FormatRegistry};

    use super::{has_unpacked_dir_suffix, load_project_from_bytes, resolve_open_importer, resolve_project_save_exporter};

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
