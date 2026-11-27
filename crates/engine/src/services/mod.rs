//! Shared application services for figure/tabular export, format registry dispatch,
//! project inspection, and atomic batches.
//!
//! Centralizes domain logic shared by engine commands (`cmd::{export,project}`) and higher-level
//! frontends (MCP server, live control server, headless CLI) without depending on any UI or
//! transport crate.

mod batch;
mod export;
mod fit_diagnostics;
mod formats;
mod inspect;

pub use batch::{BatchCommandStep, BatchStepFailure, command_step_writes_filesystem};
pub use export::{
    DEFAULT_EXPORT_DPI, DEFAULT_PREVIEW_DPI, DocumentExportFormat, EmfSheetKind, ExportArtifactFormat, ExportArtifactRequest, ExportOptionsSpec,
    ExportTarget, ExportedArtifact, MAX_FONT_FILE_BYTES, MAX_RASTER_DPI, MIN_RASTER_DPI, RasterMetadata, RasterSizingPolicy,
    compile_visual_scene_for_sheet, export_document_artifact, find_sheet_id_by_name, load_validated_ttf_font, load_validated_ttf_font_bytes,
    parse_csv_delimiter_opt, parse_dpi, parse_dpi_opt, parse_dpi_str, parse_pdf_color_space, parse_pdf_version, parse_tiff_compression,
    pdf_color_space_str, pdf_version_str, resolve_csv_export_target, resolve_json_target_by_name_or_id, resolve_preview_sheet_id,
    resolve_strict_raster_target, resolve_tabular_sheet_by_name_or_id, resolve_tidy_flag, resolve_visual_sheet_by_name_or_id, tiff_compression_str,
    validate_dpi, validate_ttf_font_bytes,
};
pub use fit_diagnostics::compare_result_vs_horizontal_line;
pub use formats::{
    LoadedProject, dir_size_bytes, has_unpacked_dir_suffix, import_registered_path, is_unpacked_project_path, load_project_from_bytes,
    load_project_from_bytes_with_xlsx_options, load_project_from_bytes_with_xlsx_sheets, load_project_from_path,
    load_project_from_path_with_xlsx_options, load_project_from_path_with_xlsx_sheets, resolve_open_importer, resolve_project_save_exporter,
};
pub use inspect::{
    DataSheetSummary, GraphSheetSummary, InfoSheetSummary, LayoutSheetSummary, MAX_CSV_PREVIEW_BYTES, MAX_CSV_PREVIEW_ROWS,
    MAX_RESULT_PREVIEW_CHECKLIST, MAX_RESULT_PREVIEW_COLS, MAX_RESULT_PREVIEW_FOOTNOTES, MAX_RESULT_PREVIEW_ROWS, MAX_RESULT_PREVIEW_SCALARS,
    MAX_RESULT_PREVIEW_TABS, MAX_RESULT_PREVIEW_TOTAL_CELLS, ProjectInspectSummary, ResultSheetSummary, SheetFocusSummary, build_csv_preview,
    build_csv_preview_with_limit, inspect_document_control_json, inspect_project_summary, inspect_sheet_detail, slice_preview_vec,
    slice_result_tabs_preview, slice_result_tabs_preview_with_opts,
};
