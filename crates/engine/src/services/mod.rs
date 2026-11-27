//! Shared application services for figure/tabular export, format registry dispatch,
//! project inspection, and atomic batches.
//!
//! Centralizes domain logic shared by engine commands (`cmd::{export,project}`) and higher-level
//! frontends (MCP server, live control server, headless CLI) without depending on any UI or
//! transport crate.

mod batch;
pub mod equation_library;
mod export;
mod fit_diagnostics;
mod formats;
mod inspect;

pub use batch::{BatchCommandStep, BatchStepFailure, command_step_writes_filesystem};
pub use equation_library::{
    MAX_USER_EQUATION_LIBRARY_BYTES, MAX_USER_EQUATIONS, SaveUserEquationRequest, USER_EQUATION_LIBRARY_FILE_NAME,
    USER_EQUATION_LIBRARY_SCHEMA_VERSION, UserEquationEntry, UserEquationLibrary, UserEquationStoreFile, ValidatedUserEquation,
    clone_builtin_equation_template, normalize_user_equation_id, validate_user_equation_dsl, validate_user_equation_id,
};
pub use export::{
    DEFAULT_EXPORT_DPI, DEFAULT_PREVIEW_DPI, DocumentExportFormat, EmfSheetKind, ExportArtifactFormat, ExportArtifactRequest, ExportOptionsSpec,
    ExportTarget, ExportedArtifact, MAX_FONT_FILE_BYTES, MAX_RASTER_DPI, MIN_RASTER_DPI, PZFX_EXPORT_DATA_ONLY_NOTICE, PZFX_EXPORT_STRUCTURE_NOTICE,
    RasterMetadata, RasterSizingPolicy, compile_visual_scene_for_sheet, export_document_artifact, find_sheet_id_by_name,
    format_import_or_archive_warning, format_pzfx_export_warning, load_validated_ttf_font, load_validated_ttf_font_bytes, parse_csv_delimiter_opt,
    parse_dpi, parse_dpi_opt, parse_dpi_str, parse_pdf_color_space, parse_pdf_version, parse_tiff_compression, pdf_color_space_str, pdf_version_str,
    resolve_csv_export_target, resolve_json_target_by_name_or_id, resolve_preview_sheet_id, resolve_strict_raster_target,
    resolve_tabular_sheet_by_name_or_id, resolve_tidy_flag, resolve_visual_sheet_by_name_or_id, tiff_compression_str, validate_dpi,
    validate_ttf_font_bytes,
};
pub use fit_diagnostics::compare_result_vs_horizontal_line;
pub use formats::{
    LoadedProject, dir_size_bytes, finalize_import_report, finalize_promoted_imported_analyses, has_unpacked_dir_suffix, import_registered_path,
    is_unpacked_project_path, load_project_from_bytes, load_project_from_bytes_with_xlsx_options, load_project_from_bytes_with_xlsx_sheets,
    load_project_from_path, load_project_from_path_with_xlsx_options, load_project_from_path_with_xlsx_sheets, resolve_open_importer,
    resolve_project_save_exporter,
};
pub use inspect::{
    DataSheetSummary, GraphSheetSummary, InfoSheetSummary, LayoutSheetSummary, MAX_CSV_PREVIEW_BYTES, MAX_CSV_PREVIEW_ROWS,
    MAX_RESULT_PREVIEW_CHECKLIST, MAX_RESULT_PREVIEW_COLS, MAX_RESULT_PREVIEW_FOOTNOTES, MAX_RESULT_PREVIEW_ROWS, MAX_RESULT_PREVIEW_SCALARS,
    MAX_RESULT_PREVIEW_TABS, MAX_RESULT_PREVIEW_TOTAL_CELLS, ProjectInspectSummary, ResultSheetSummary, SheetFocusSummary, build_csv_preview,
    build_csv_preview_with_limit, inspect_document_control_json, inspect_project_summary, inspect_sheet_detail, slice_preview_vec,
    slice_result_tabs_preview, slice_result_tabs_preview_with_opts,
};
