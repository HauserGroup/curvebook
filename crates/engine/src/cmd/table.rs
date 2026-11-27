//! Data-table cell, title, exclusion, paste/link, CSV/project import, and archetype commands (Wave 9B).

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, BaselineOperation, Cell, CellCoord, CellExclusionNote, CellValue, ColumnEqualsFilter, DataSetColumn, DataSheet, GraphKind,
    GraphSpec, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LevelOrderKind, LevelOrderSpec, LinkSource, MAX_PROJECT_CELLS, MAX_SHEET_CELLS,
    MultipleVariablesTable, NestedGroupColumn, NestedTable, PlacementMode, ProjectDocument, ResultSheet, SheetCellRange, SheetId, SubcolumnMode,
    SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, VariableColumn, VariableRole, VariableSource,
    format_linked_paste_tsv_cell, validate_project_cells, validate_rows, validate_title,
};
use curvebook_format::{CsvOptions, export_datasheet_to_csv, try_export_result_sheet_tidy_csv};
use curvebook_transform::{
    ImportPipelineOptions, ImportPipelineReport, ImportSourcePayload, TargetArchetypeKind, apply_import_link_pipeline,
    evaluate_data_sheet_calculated_variables, validate_no_formula_cycle,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::helpers::{
    always_enabled, bad_input, column_subcolumn_count, ensure_sheet_not_frozen, has_data_or_result_sheet, has_data_sheet, has_multivar_sheet,
    has_nested_sheet, parse_sheet_id, prune_shortened_series_overrides, require_data_sheet, require_unfrozen_data_sheet_mut,
    shift_point_overrides_on_insert_rows, snapshot_active_series_lengths, x_subcolumn_count,
};
use super::params::{command_schema, deserialize_strict, parse_params};
use super::project::recalculate_dirty_with_links;
use super::{CommandGroup, CommandSpec, Ctx, Effects, command_specs};
use crate::{CommandOutcome, EngineError, ErrorCode};

mod convert;
mod import;
mod simulate;

use convert::{ConvertArchetypeParams, run_convert_archetype};
use import::{
    ImportCsvParams, ImportPlateMapParams, ImportPrismParams, ImportPzfxParams, ImportTidyLongParams, ImportXlsxParams, PasteLinkParams,
    run_import_csv, run_import_plate_map, run_import_prism, run_import_pzfx, run_import_tidy_long, run_import_xlsx, run_paste_link,
};
use simulate::{
    RandomSampleRowsParams, RandomizeGroupsParams, SimulateDistributionsParams, SimulateXyParams, run_random_sample_rows, run_randomize_groups,
    run_simulate_distributions, run_simulate_xy,
};

/// Registered command specifications for the `table` group (29 commands).
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    SPECS
}

static SPECS: &[CommandSpec] = &command_specs!(
    CommandGroup::Table;
    {
        id: "table.setCell",
        label: "Set Cell Value",
        menu_path: "Data/Set Cell Value",
        summary: "Set or update a single data cell value and optional exclusion flag, then recalculate downstream sheets.",
        beginner: "Enter or edit one cell in a data table. Connected analyses and graphs refresh automatically.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SetCellParams>,
        enabled: has_data_sheet,
        run: run_set_cell,
    },
    {
        id: "table.setCells",
        label: "Set Multiple Cells",
        menu_path: "Data/Set Multiple Cells",
        summary: "Apply a batch of cell edits or a 2D row/column block to a data table, then recalculate.",
        beginner: "Update multiple cells in a data table at once from a list of edits or a 2D grid.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SetCellsParams>,
        enabled: has_data_sheet,
        run: run_set_cells,
    },
    {
        id: "table.setXCells",
        label: "Set X / Time Column",
        menu_path: "Data/Set X Values",
        summary: "Set one or more X or elapsed-time column cells on an XY or Survival table.",
        beginner: "Fill or edit values in the X (or Survival time) column.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SetXCellsParams>,
        enabled: has_data_sheet,
        run: run_set_x_cells,
    },
    {
        id: "table.setRowTitles",
        label: "Set Row Titles",
        menu_path: "Data/Set Row Titles",
        summary: "Set or clear one or more row titles (or PartsOfWhole slice labels) in a data table.",
        beginner: "Label rows along the left edge of a data table.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SetRowTitlesParams>,
        enabled: has_data_sheet,
        run: run_set_row_titles,
    },
    {
        id: "table.setGroupTitles",
        label: "Set Group Column Titles",
        menu_path: "Data/Set Column Titles",
        summary: "Set group/dataset titles, X/time title, factor titles, or row titles on a data table.",
        beginner: "Rename columns, X/time headers, or factor titles in a data table.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SetTitlesParams>,
        enabled: has_data_sheet,
        run: run_set_group_titles,
    },
    {
        id: "table.setTitles",
        label: "Set Column Titles",
        menu_path: "Data/Set Column Titles",
        summary: "Set dataset column titles, X/time title, factor titles, or row titles on a data table.",
        beginner: "Rename column headers, X/time title, or factor titles in a data table.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SetTitlesParams>,
        enabled: has_data_sheet,
        run: run_set_titles,
    },
    {
        id: "table.setSubcolumnTitles",
        label: "Set Subcolumn Titles",
        menu_path: "Data/Set Subcolumn Titles",
        summary: "Set explicit subcolumn titles on a Nested data table across one or all groups.",
        beginner: "Rename nested replicate subcolumns for a specific group or all groups.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SetSubcolumnTitlesParams>,
        enabled: has_nested_sheet,
        run: run_set_subcolumn_titles,
    },
    {
        id: "table.excludeCells",
        label: "Exclude Cells",
        menu_path: "Change/Exclude Values",
        shortcut: "Cmd+E",
        summary: "Toggle the exclusion mask on single cells, cell lists, or ranges (including X/time cells).",
        beginner: "Exclude or include selected values so analyses ignore them without deleting data.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<ExcludeCellsParams>,
        enabled: has_data_sheet,
        run: run_exclude_cells,
    },
    {
        id: "table.excludeCellsWithReason",
        label: "Exclude Cells With Reason",
        menu_path: "Change/Exclude Values With Reason",
        summary: "Exclude cells and attach a structured audit reason tag, note, and UTC timestamp.",
        beginner: "Exclude values while recording a reason tag and audit timestamp for review.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<ExcludeCellsWithReasonParams>,
        enabled: has_data_sheet,
        run: run_exclude_cells_with_reason,
    },
    {
        id: "table.exclusionAuditReport",
        label: "Exclusion Audit Report",
        menu_path: "Data/Exclusion Audit Report",
        summary: "Generate a read-only audit report of excluded cells and their reason annotations.",
        beginner: "Inspect all excluded values and audit reasons across one sheet or the entire project.",
        effects: Effects::NONE.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<ExclusionAuditReportParams>,
        enabled: always_enabled,
        run: run_exclusion_audit_report,
    },
    {
        id: "table.clearCells",
        label: "Clear Cells",
        menu_path: "Edit/Clear Cells",
        summary: "Clear single cells, lists, ranges, or an entire table, optionally breaking live links.",
        beginner: "Erase values from selected cells or clear the entire data table.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA).with(Effects::IDEMPOTENT),
        input_schema: command_schema::<ClearCellsParams>,
        enabled: has_data_sheet,
        run: run_clear_cells,
    },
    {
        id: "table.paste",
        label: "Paste Special",
        menu_path: "Edit/Paste Special",
        shortcut: "Cmd+V",
        summary: "Paste delimited clipboard text or a 2D cell grid into a table with placement, transpose, and filter options.",
        beginner: "Paste tabular data into a table starting at an anchor cell, optionally transposing or filtering.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<PasteParams>,
        enabled: has_data_sheet,
        run: run_paste,
    },
    {
        id: "table.pasteLink",
        label: "Paste Link",
        menu_path: "Edit/Paste Link",
        summary: "Attach a live linked region from an internal sheet or external delimited file and sync immediately.",
        beginner: "Link a range from another sheet or CSV/TSV file so updates flow through automatically.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM),
        input_schema: command_schema::<PasteLinkParams>,
        enabled: has_data_sheet,
        run: run_paste_link,
    },
    {
        id: "table.importCsv",
        label: "Import CSV / TSV",
        menu_path: "File/Import/CSV or TSV",
        summary: "Import CSV or TSV data from a file path or inline text into a new or existing data table.",
        beginner: "Create a new data table or populate an existing one from a CSV or TSV file.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA),
        input_schema: command_schema::<ImportCsvParams>,
        enabled: always_enabled,
        run: run_import_csv,
    },
    {
        id: "table.exportCsv",
        label: "Export Table CSV / TSV",
        menu_path: "File/Export/Table CSV",
        summary: "Export a data or result sheet to CSV/TSV text and optionally write atomically to disk.",
        beginner: "Save a data or result table as CSV or TSV without changing the project.",
        effects: Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<ExportCsvParams>,
        enabled: has_data_or_result_sheet,
        run: run_export_csv,
    },
    {
        id: "table.importPrism",
        label: "Import .prism files",
        menu_path: "File/Import/.prism files",
        summary: "Import .prism files from disk, base64, or raw byte array.",
        beginner: "Open or merge .prism files into the project.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA),
        input_schema: command_schema::<ImportPrismParams>,
        enabled: always_enabled,
        run: run_import_prism,
    },
    {
        id: "table.importPzfx",
        label: "Import .pzfx files",
        menu_path: "File/Import/.pzfx files",
        summary: "Import .pzfx files from disk, inline XML string, or base64 payload.",
        beginner: "Open or merge .pzfx files into the project.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA),
        input_schema: command_schema::<ImportPzfxParams>,
        enabled: always_enabled,
        run: run_import_pzfx,
    },
    {
        id: "table.importXlsx",
        label: "Import XLSX Workbook",
        menu_path: "File/Import/XLSX Workbook",
        summary: "Import one or more worksheets (or list available sheets) from an .xlsx workbook file, base64, or byte array.",
        beginner: "Open or merge worksheets from a spreadsheet workbook (.xlsx) into the project.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA),
        input_schema: command_schema::<ImportXlsxParams>,
        enabled: always_enabled,
        run: run_import_xlsx,
    },
    {
        id: "table.setVariableRole",
        label: "Set Variable Role",
        menu_path: "Data/Variable Properties",
        summary: "Lock or re-infer the analytical role of a column in a MultipleVariables table.",
        beginner: "Mark a variable column as continuous, categorical, label, calculated, or auto-inferred.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SetVariableRoleParams>,
        enabled: has_multivar_sheet,
        run: run_set_variable_role,
    },
    {
        id: "table.recodeLevels",
        label: "Recode Categorical Levels",
        menu_path: "Data/Recode Levels",
        summary: "Map and rewrite categorical values in a MultipleVariables column.",
        beginner: "Rename or merge category labels within a MultipleVariables column.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<RecodeLevelsParams>,
        enabled: has_multivar_sheet,
        run: run_recode_levels,
    },
    {
        id: "table.setCategoricalOrder",
        label: "Set Categorical Order",
        menu_path: "Data/Set Categorical Order",
        summary: "Configure display/reference ordering of categorical levels in a MultipleVariables column.",
        beginner: "Choose how categories are ordered (explicit list, alphabetical, entry order, or frequency).",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SetCategoricalOrderParams>,
        enabled: has_multivar_sheet,
        run: run_set_categorical_order,
    },
    {
        id: "table.setCalculatedVariable",
        label: "Add / Update Calculated Variable",
        menu_path: "Data/Calculated Variable",
        summary: "Create or update a formula-driven variable column in a MultipleVariables table and evaluate it.",
        beginner: "Add a computed column whose values update automatically from other variables via a formula.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SetCalculatedVariableParams>,
        enabled: has_multivar_sheet,
        run: run_set_calculated_variable,
    },
    {
        id: "table.simulateXy",
        label: "Simulate XY Data",
        menu_path: "Insert/Simulate Data",
        summary: "Generate a synthetic XY data sheet from a built-in equation, parameter map, and noise model.",
        beginner: "Create synthetic XY curves with optional Gaussian, relative CV, or Poisson noise.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<SimulateXyParams>,
        enabled: always_enabled,
        run: run_simulate_xy,
    },
    {
        id: "table.convertArchetype",
        label: "Convert Table Format",
        menu_path: "Change/Convert Table Structure",
        summary: "Preview or convert a data table between Column, Grouped, XY, and MultipleVariables archetypes.",
        beginner: "Reshape a data table into another structure, previewing any lossy changes before applying.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA).with(Effects::IDEMPOTENT),
        input_schema: command_schema::<ConvertArchetypeParams>,
        enabled: has_data_sheet,
        run: run_convert_archetype,
    },
    {
        id: "table.importPlateMap",
        label: "Import Plate Map",
        menu_path: "File/Import/Plate Map",
        summary: "Map 96-well or 384-well microplate readings and well roles into a structured data sheet.",
        beginner: "Import 96-well or 384-well plate measurements using a plate template or role map.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<ImportPlateMapParams>,
        enabled: always_enabled,
        run: run_import_plate_map,
    },
    {
        id: "table.importTidyLong",
        label: "Import Tidy Long Table",
        menu_path: "File/Import/Tidy Long Table",
        summary: "Pivot a long-format table (rows/headers or CSV/TSV file) into a Column, Grouped, XY, or MultipleVariables sheet.",
        beginner: "Reshape long/tidy tabular records into a grouped or XY data table using column mappings.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM),
        input_schema: command_schema::<ImportTidyLongParams>,
        enabled: always_enabled,
        run: run_import_tidy_long,
    },
    {
        id: "table.randomizeGroups",
        label: "Randomize Subjects to Groups",
        menu_path: "Data/Randomize Groups",
        summary: "Deterministically allocate subjects across treatment groups (complete-balanced or permuted-block) and upsert ARRIVE constants on an InfoSheet.",
        beginner: "Randomizes subjects into balanced or blocked treatment arms and stores reproducibility metadata on an Info sheet.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<RandomizeGroupsParams>,
        enabled: always_enabled,
        run: run_randomize_groups,
    },
    {
        id: "table.randomSampleRows",
        label: "Random Sample Rows",
        menu_path: "Data/Random Sample Rows",
        summary: "Draw a reproducible Fisher-Yates random sample of rows (optional stratification) into a new data sheet.",
        beginner: "Creates a new table containing a seeded simple or stratified random subset of rows from a source sheet.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<RandomSampleRowsParams>,
        enabled: has_data_or_result_sheet,
        run: run_random_sample_rows,
    },
    {
        id: "table.simulateDistributions",
        label: "Simulate Distributions",
        menu_path: "Insert/Simulate Distributions",
        summary: "Generate a Column or MultipleVariables table with columns sampled from Uniform, Gaussian, LogNormal, Binomial, Poisson, or Exponential distributions.",
        beginner: "Creates a new table of seeded random variates drawn from parametric statistical distributions.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<SimulateDistributionsParams>,
        enabled: always_enabled,
        run: run_simulate_distributions,
    },
);

// ---------------------------------------------------------------------------
// Initial Link Payload Loading & Title Propagation Helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[allow(dead_code)]
struct FlatCoordInput {
    /// Zero-based row index.
    row: Option<usize>,
    /// Zero-based dataset/column index.
    col: Option<usize>,
    /// Zero-based replicate subcolumn index (default 0).
    subcol: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct CoordInput {
    /// Nested coordinate object `{ row, col, subcol? }`.
    #[schemars(with = "Option<FlatCoordInput>")]
    coord: Option<Box<CoordInput>>,
    /// Zero-based row index.
    row: Option<usize>,
    /// Zero-based dataset/column index.
    col: Option<usize>,
    /// Zero-based replicate subcolumn index (default 0).
    subcol: Option<usize>,
}

impl CoordInput {
    fn resolve(&self) -> Result<CellCoord, EngineError> {
        if let Some(inner) = &self.coord {
            return inner.resolve();
        }
        let row = self.row.ok_or_else(|| {
            EngineError::with_code(ErrorCode::BadInput, "missing required coordinate `row`")
                .with_remediation("Provide `{ row, col, subcol? }` or `coord`.")
        })?;
        let col = self.col.ok_or_else(|| {
            EngineError::with_code(ErrorCode::BadInput, "missing required coordinate `col`")
                .with_remediation("Provide `{ row, col, subcol? }` or `coord`.")
        })?;
        Ok(CellCoord::new(row, col, self.subcol.unwrap_or(0)))
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct FlexibleCoordInput {
    /// Anchor row index.
    row: Option<usize>,
    /// Anchor column index.
    col: Option<usize>,
    /// Anchor subcolumn index.
    subcol: Option<usize>,
}

impl FlexibleCoordInput {
    fn resolve(&self) -> CellCoord {
        CellCoord::new(self.row.unwrap_or(0), self.col.unwrap_or(0), self.subcol.unwrap_or(0))
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CellRangeInput {
    /// Range start coordinate `{ row, col, subcol? }`.
    #[serde(default)]
    start: Option<CoordInput>,
    /// Range end coordinate `{ row, col, subcol? }`.
    #[serde(default)]
    end: Option<CoordInput>,
}

impl CellRangeInput {
    fn resolve(&self) -> Result<SheetCellRange, EngineError> {
        let start_in = self.start.as_ref().ok_or_else(|| {
            EngineError::with_code(ErrorCode::BadInput, "range requires `start`").with_remediation("Specify `{ start: { row, col }, end? }`.")
        })?;
        let end_in = self.end.as_ref().unwrap_or(start_in);
        let a = start_in.resolve()?;
        let b = end_in.resolve()?;
        Ok(SheetCellRange::new(
            CellCoord::new(a.row.min(b.row), a.col.min(b.col), a.subcol.min(b.subcol)),
            CellCoord::new(a.row.max(b.row), a.col.max(b.col), a.subcol.max(b.subcol)),
        ))
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
enum ColRef {
    Index(usize),
    Title(#[schemars(length(min = 1))] String),
}

struct CellValueSchema;

impl JsonSchema for CellValueSchema {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("CellValue")
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "description": "Cell scalar value: finite number, text/numeric string, boolean, cell object, or null to clear",
            "anyOf": [
                { "type": "number", "description": "Finite numeric cell value" },
                { "type": "string", "description": "String or numeric token (trailing `*` sets excluded when enabled)" },
                { "type": "boolean", "description": "Boolean text cell" },
                {
                    "type": "object",
                    "properties": {
                        "value": { "description": "Cell scalar value: finite number, text/numeric string, boolean, or null" },
                        "excluded": { "type": ["boolean", "null"], "description": "Optional explicit exclusion flag" },
                        "decimal_separator": { "type": ["string", "null"], "description": "Explicit decimal separator (`.`, `,`, `dot`, `comma`, or `period`)" },
                        "parse_asterisk_exclusion": { "type": ["boolean", "null"], "description": "Interpret trailing `*` in string tokens as excluded (default true)" }
                    },
                    "additionalProperties": false
                },
                { "type": "null" }
            ]
        })
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct CellPayloadFields {
    /// Cell scalar value: finite number, text/numeric string, boolean, cell object, or null to clear.
    #[serde(default)]
    #[schemars(with = "CellValueSchema")]
    value: Value,
    /// Explicit decimal separator (`.`, `,`, `dot`, `comma`, or `period`).
    decimal_separator: Option<String>,
    /// Interpret trailing `*` in string tokens as excluded (default true).
    parse_asterisk_exclusion: Option<bool>,
    /// Optional explicit exclusion flag.
    excluded: Option<bool>,
}

impl CellPayloadFields {
    fn into_cell(self) -> Result<Cell, EngineError> {
        let dec = parse_explicit_decimal_opt(self.decimal_separator.as_deref())?.unwrap_or('.');
        let parse_asterisk = self.parse_asterisk_exclusion.unwrap_or(true);
        let mut cell = parse_cell_value_with_options(&self.value, dec, parse_asterisk)?;
        if let Some(ex) = self.excluded {
            cell.excluded = ex;
        }
        Ok(cell)
    }
}

fn parse_cell_value_with_options(val: &Value, dec: char, parse_asterisk: bool) -> Result<Cell, EngineError> {
    match val {
        Value::Null => Ok(Cell::empty()),
        Value::Bool(b) => Ok(Cell::text(if *b { "true" } else { "false" })),
        Value::Number(n) => {
            let f = n.as_f64().filter(|x| x.is_finite()).ok_or_else(|| {
                EngineError::with_code(ErrorCode::BadInput, "cell numeric value must be finite")
                    .with_remediation("Provide a finite number, string, or null.")
            })?;
            Ok(Cell::number(f))
        }
        Value::String(s) => Ok(Cell::parse_import_token(s, dec, parse_asterisk)),
        Value::Object(_) => {
            let nested: CellPayloadFields = deserialize_strict(val, "/value")?;
            nested.into_cell()
        }
        Value::Array(_) => Err(EngineError::with_code(ErrorCode::BadInput, "cell value cannot be a JSON array")
            .with_remediation("Provide a scalar number, string, boolean, or null.")),
    }
}

fn parse_cell_from_grid_value(val: &Value) -> Result<Cell, EngineError> {
    parse_cell_value_with_options(val, '.', true)
}

fn parse_explicit_decimal_opt(raw: Option<&str>) -> Result<Option<char>, EngineError> {
    let Some(s) = raw else {
        return Ok(None);
    };
    match s.trim().to_ascii_lowercase().as_str() {
        "." | "dot" | "period" => Ok(Some('.')),
        "," | "comma" => Ok(Some(',')),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported decimal separator `{s}` (expected `.` or `,`)"))
            .with_remediation("Supply an explicit `.`/`dot`/`period` or `,`/`comma`.")),
    }
}

fn parse_delimiter_char(raw: &str) -> Result<char, EngineError> {
    match raw {
        "," | "comma" => Ok(','),
        "\t" | "\\t" | "tab" | "tsv" => Ok('\t'),
        ";" | "semicolon" => Ok(';'),
        _ if raw.chars().count() == 1 => raw.chars().next().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "invalid delimiter")),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported delimiter `{raw}`"))
            .with_remediation("Use `,`, `\\t`/`tab`, `;`, `auto`, or a single delimiter character.")),
    }
}

fn resolve_delimiter(delimiter: Option<&str>, sample_text: &str) -> Result<char, EngineError> {
    if let Some(raw) = delimiter
        && !raw.eq_ignore_ascii_case("auto")
    {
        return parse_delimiter_char(raw);
    }
    Ok(CsvOptions::detect(sample_text).delimiter)
}

fn resolve_decimal_char(decimal_opt: Option<&str>, normalize_decimal: bool, sample_text: &str, delimiter: char) -> Result<char, EngineError> {
    if let Some(raw) = decimal_opt {
        if raw.eq_ignore_ascii_case("auto") {
            return Ok(infer_decimal_char(sample_text, delimiter));
        }
        return match raw.to_ascii_lowercase().as_str() {
            "." | "dot" | "period" => Ok('.'),
            "," | "comma" => Ok(','),
            _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown decimal separator `{raw}`"))
                .with_remediation("Use `.` (`dot`), `,` (`comma`), or `auto`.")),
        };
    }
    if normalize_decimal || delimiter != ',' {
        return Ok(infer_decimal_char(sample_text, delimiter));
    }
    Ok('.')
}

fn infer_decimal_char(sample_text: &str, delimiter: char) -> char {
    if delimiter == '\t' {
        return CsvOptions::detect_tsv(sample_text).decimal_separator;
    }
    if delimiter == ',' {
        return '.';
    }
    let detected = CsvOptions::detect(sample_text);
    if detected.delimiter == delimiter {
        return detected.decimal_separator;
    }
    let tokens: Vec<&str> = sample_text.lines().take(32).flat_map(|line| line.split(delimiter)).collect();
    curvebook_doc::infer_number_separators(tokens, delimiter).0
}

fn parse_placement_mode(raw: &str) -> Result<PlacementMode, EngineError> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "overwrite" => Ok(PlacementMode::Overwrite),
        "insert" | "insertrows" => Ok(PlacementMode::InsertRows),
        "append" | "appendrows" => Ok(PlacementMode::AppendRows),
        "appendcolumns" | "appendcols" => Ok(PlacementMode::AppendColumns),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown placement mode `{raw}`"))
            .with_remediation("Use `overwrite`, `insert_rows`, `append_rows`, or `append_columns`.")),
    }
}

fn parse_table_kind(raw: &str) -> Result<TableKind, EngineError> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "xy" => Ok(TableKind::Xy),
        "column" | "col" => Ok(TableKind::Column),
        "grouped" | "twoway" => Ok(TableKind::Grouped),
        "contingency" => Ok(TableKind::Contingency),
        "survival" => Ok(TableKind::Survival),
        "partsofwhole" | "pie" => Ok(TableKind::PartsOfWhole),
        "multiplevariables" | "multivar" => Ok(TableKind::MultipleVariables),
        "nested" => Ok(TableKind::Nested),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown table archetype `{raw}`"))
            .with_remediation("Use XY, Column, Grouped, Contingency, Survival, PartsOfWhole, MultipleVariables, or Nested.")),
    }
}

fn parse_target_archetype_kind(raw: &str) -> Result<TargetArchetypeKind, EngineError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "column" | "columnar" => Ok(TargetArchetypeKind::Column),
        "grouped" => Ok(TargetArchetypeKind::Grouped),
        "xy" => Ok(TargetArchetypeKind::Xy),
        "contingency" => Ok(TargetArchetypeKind::Contingency),
        "multiplevariables" | "multiple_variables" | "multiple-variables" | "multivariate" => Ok(TargetArchetypeKind::MultipleVariables),
        _ => Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("unsupported target_kind `{raw}` (expected Column, Grouped, XY, Contingency, or MultipleVariables)"),
        )
        .with_remediation("Choose one of Column, Grouped, XY, Contingency, or MultipleVariables.")),
    }
}

fn parse_subcolumn_mode_str(s: &str, replicates: usize) -> Result<SubcolumnMode, EngineError> {
    let norm: String = s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "single" => Ok(SubcolumnMode::Single),
        "replicate" | "replicates" | "rawreplicates" => Ok(SubcolumnMode::RawReplicates(replicates.max(1))),
        "meansd" => Ok(SubcolumnMode::MeanSd),
        "meansem" => Ok(SubcolumnMode::MeanSem),
        "meansdn" => Ok(SubcolumnMode::MeanSdN),
        "meansemn" => Ok(SubcolumnMode::MeanSemN),
        "meancvn" => Ok(SubcolumnMode::MeanCvN),
        "meanlowhigh" => Ok(SubcolumnMode::MeanLowHigh),
        "upperlowerlimits" | "meanupperlower" => Ok(SubcolumnMode::UpperLowerLimits),
        "medianiqr" => Ok(SubcolumnMode::MedianIqr),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown subcolumn mode `{s}`")).with_remediation(
            "Use single, replicates, mean_sd, mean_sem, mean_sd_n, mean_sem_n, mean_cv_n, mean_low_high, upper_lower_limits, or median_iqr.",
        )),
    }
}

fn resolve_subcolumn_mode(raw: Option<&Value>, replicates: usize) -> Result<SubcolumnMode, EngineError> {
    if let Some(val) = raw {
        if let Some(s) = val.as_str() {
            return parse_subcolumn_mode_str(s, replicates);
        }
        if let Ok(mode) = serde_json::from_value::<SubcolumnMode>(val.clone()) {
            return Ok(mode);
        }
        return Err(EngineError::with_code(ErrorCode::BadInput, "invalid `subcolumn_mode` specification"));
    }
    if replicates > 1 { Ok(SubcolumnMode::RawReplicates(replicates)) } else { Ok(SubcolumnMode::Single) }
}

fn parse_import_filter_val(val: Option<&Value>) -> Result<ImportFilterSpec, EngineError> {
    let Some(v) = val else {
        return Ok(ImportFilterSpec::default());
    };
    if v.is_null() {
        return Ok(ImportFilterSpec::default());
    }
    if let Ok(parsed) = deserialize_strict::<ImportFilterSpec>(v, "/filter") {
        return Ok(parsed);
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct FilterObj {
        row_start: Option<usize>,
        row_end: Option<usize>,
        col_start: Option<usize>,
        col_end: Option<usize>,
        row_window: Option<(usize, usize)>,
        col_window: Option<(usize, usize)>,
        decimate_every_k: Option<usize>,
        unstack_group_column: Option<usize>,
        parse_asterisk_exclusions: Option<bool>,
        baseline: Option<(usize, BaselineOperation)>,
        exclude_row_predicate: Option<String>,
        row_match: Option<ColumnEqualsFilterSchema>,
    }
    let obj: FilterObj = deserialize_strict(v, "/filter")?;
    let mut filter = ImportFilterSpec::default();
    if let Some(rw) = obj.row_window {
        filter.row_window = Some(rw);
    } else if obj.row_start.is_some() || obj.row_end.is_some() {
        filter.row_window = Some((obj.row_start.unwrap_or(0), obj.row_end.unwrap_or(usize::MAX)));
    }
    if let Some(cw) = obj.col_window {
        filter.col_window = Some(cw);
    } else if obj.col_start.is_some() || obj.col_end.is_some() {
        filter.col_window = Some((obj.col_start.unwrap_or(0), obj.col_end.unwrap_or(usize::MAX)));
    }
    if let Some(k) = obj.decimate_every_k {
        filter.decimate_every_k = Some(k.max(1));
    }
    filter.unstack_group_column = obj.unstack_group_column;
    filter.parse_asterisk_exclusions = obj.parse_asterisk_exclusions.unwrap_or(true);
    filter.baseline = obj.baseline;
    filter.exclude_row_predicate = obj.exclude_row_predicate;
    filter.row_match = obj.row_match.map(ColumnEqualsFilterSchema::into_doc);
    Ok(filter)
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BaselineInput {
    /// Zero-based reference baseline column index (default 0).
    #[serde(default)]
    col: Option<usize>,
    /// Baseline operation: difference, ratio, percent_change, or fractional_change.
    #[serde(default)]
    op: Option<String>,
}

fn resolve_baseline_adjustment(baseline: Option<BaselineInput>) -> Result<Option<(usize, BaselineOperation)>, EngineError> {
    let Some(b) = baseline else {
        return Ok(None);
    };
    let op_str = b.op.as_deref().ok_or_else(|| {
        EngineError::with_code(ErrorCode::BadInput, "baseline adjustment requires `op`")
            .with_remediation("Set `op` to subtract/difference, divide/ratio, percent_change, or fractional_change.")
    })?;
    let norm: String = op_str.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let op = match norm.as_str() {
        "subtract" | "difference" => BaselineOperation::Difference,
        "divide" | "ratio" => BaselineOperation::Ratio,
        "percentchange" | "pctchange" => BaselineOperation::PercentChange,
        "fractionalchange" => BaselineOperation::FractionalChange,
        _ => {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown baseline operation `{op_str}`")));
        }
    };
    Ok(Some((b.col.unwrap_or(0), op)))
}

// ---------------------------------------------------------------------------
// Schema Reusable Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ColumnEqualsFilterSchema {
    /// Zero-based column index whose trimmed text value is matched.
    pub(super) column: usize,
    /// Exact trimmed text value required to retain a row.
    pub(super) equals: String,
    /// Remove the matched discriminator column from the output table (default true).
    pub(super) drop_column: Option<bool>,
}

impl ColumnEqualsFilterSchema {
    pub(super) fn into_doc(self) -> ColumnEqualsFilter {
        ColumnEqualsFilter { column: self.column, equals: self.equals, drop_column: self.drop_column.unwrap_or(true) }
    }
}

#[derive(Debug, JsonSchema)]
#[allow(dead_code)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportFilterSchema {
    /// First zero-based source row to include.
    row_start: Option<usize>,
    /// Last zero-based source row to include.
    row_end: Option<usize>,
    /// First zero-based source column to include.
    col_start: Option<usize>,
    /// Last zero-based source column to include.
    col_end: Option<usize>,
    /// Inclusive zero-based [start_row, end_row] slice.
    row_window: Option<[usize; 2]>,
    /// Inclusive zero-based [start_col, end_col] slice.
    col_window: Option<[usize; 2]>,
    /// Keep every k-th source row (>= 1).
    #[schemars(range(min = 1))]
    decimate_every_k: Option<usize>,
    /// Optional grouping column index for unstacking.
    unstack_group_column: Option<usize>,
    /// Interpret `*` suffix tokens as excluded cells.
    parse_asterisk_exclusions: Option<bool>,
    /// Optional ImportFilterSpec baseline tuple `[col, operation]`.
    baseline: Option<Value>,
    /// Optional row-level exclusion predicate formula.
    exclude_row_predicate: Option<String>,
    /// Optional exact trimmed text-equality row filter targeting one zero-based column.
    row_match: Option<ColumnEqualsFilterSchema>,
}

pub(super) fn validate_project_cell_budget(doc: &ProjectDocument) -> Result<(), EngineError> {
    let total = doc.total_allocated_cells()?;
    validate_project_cells(total, 0)?;
    Ok(())
}

pub(super) fn validate_project_cell_budget_after_change(
    doc: &ProjectDocument,
    replaced_sheet_id: Option<&SheetId>,
    replacement_or_added_cells: usize,
) -> Result<(), EngineError> {
    let mut existing = doc.total_allocated_cells()?;
    if let Some(target_id) = replaced_sheet_id
        && let Some(existing_ds) = doc.data_sheet(target_id)
    {
        existing = existing.saturating_sub(existing_ds.archetype.allocated_cell_slots());
    }
    validate_project_cells(existing, replacement_or_added_cells)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Cell & Title Helpers
// ---------------------------------------------------------------------------

fn set_row_title(arch: &mut TableArchetype, row: usize, title: Option<String>) -> Result<(), EngineError> {
    let row_count = arch.row_count();
    let val = title.unwrap_or_default();
    match arch {
        TableArchetype::Xy(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::Column(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::Grouped(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::Contingency(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::Survival(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::PartsOfWhole(t) => set_row_title_vec(&mut t.slice_labels, row_count, row, val),
        TableArchetype::MultipleVariables(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
        TableArchetype::Nested(t) => set_row_title_vec(&mut t.row_titles, row_count, row, val),
    }
}

fn set_row_title_vec(titles: &mut Vec<String>, row_count: usize, row: usize, val: String) -> Result<(), EngineError> {
    if row >= row_count {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("row {row} out of range (row count {row_count})"))
            .with_remediation(format!("Choose a row index in 0..{row_count}.")));
    }
    let needed = row.checked_add(1).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "row index overflow"))?;
    validate_rows(needed)?;
    validate_title(&val)?;
    if titles.len() < needed {
        titles.resize(needed, String::new());
    }
    if let Some(slot) = titles.get_mut(row) {
        *slot = val;
    }
    Ok(())
}

fn validate_row_title_batch(arch: &TableArchetype, start_row: usize, titles: &[Option<String>]) -> Result<(), EngineError> {
    let row_count = arch.row_count();
    if !titles.is_empty() {
        let end_exclusive =
            start_row.checked_add(titles.len()).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "row title range overflow"))?;
        if end_exclusive > row_count {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("row title range {start_row}..{end_exclusive} exceeds row count {row_count}"),
            ));
        }
    }
    for s in titles.iter().flatten() {
        validate_title(s)?;
    }
    Ok(())
}

fn set_group_title(arch: &mut TableArchetype, col: usize, title: &str) -> Result<(), EngineError> {
    validate_title(title)?;
    match arch {
        TableArchetype::Xy(t) => set_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Column(t) => set_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Grouped(t) => set_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Contingency(t) => set_ds_title(&mut t.data_sets, col, title),
        TableArchetype::PartsOfWhole(t) => set_ds_title(&mut t.data_sets, col, title),
        TableArchetype::Survival(t) => set_ds_title(&mut t.cohorts, col, title),
        TableArchetype::MultipleVariables(t) => {
            let var = t.variables.get_mut(col).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("variable {col} out of range")))?;
            var.title = title.to_owned();
            Ok(())
        }
        TableArchetype::Nested(t) => {
            let grp = t.groups.get_mut(col).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("group {col} out of range")))?;
            grp.title = title.to_owned();
            Ok(())
        }
    }
}

fn set_ds_title(sets: &mut [DataSetColumn], col: usize, title: &str) -> Result<(), EngineError> {
    validate_title(title)?;
    let ds = sets.get_mut(col).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("column {col} out of range")))?;
    ds.title = title.to_owned();
    Ok(())
}

fn set_nested_subcol_title(nested: &mut NestedTable, col_opt: Option<usize>, subcol: usize, title: &str) -> Result<(), EngineError> {
    validate_title(title)?;
    if let Some(col) = col_opt {
        let grp = nested.groups.get_mut(col).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("group {col} out of range")))?;
        assign_nested_group_subcol_title(grp, subcol, title)?;
    } else {
        for grp in &mut nested.groups {
            assign_nested_group_subcol_title(grp, subcol, title)?;
        }
    }
    Ok(())
}

fn assign_nested_group_subcol_title(grp: &mut NestedGroupColumn, subcol: usize, title: &str) -> Result<(), EngineError> {
    let width = grp.cells.first().map_or(grp.subcolumn_titles.len(), Vec::len);
    assign_subcol_title(&mut grp.subcolumn_titles, width, subcol, title)
}

fn assign_subcol_title(titles: &mut Vec<String>, width: usize, subcol: usize, title: &str) -> Result<(), EngineError> {
    validate_title(title)?;
    if subcol >= width {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("subcolumn {subcol} out of range (width {width})")));
    }
    if titles.len() < width {
        titles.resize(width, String::new());
    }
    if let Some(slot) = titles.get_mut(subcol) {
        *slot = title.to_owned();
    }
    Ok(())
}

fn set_x_or_factor_title(
    arch: &mut TableArchetype,
    x_title: Option<&str>,
    row_factor: Option<&str>,
    col_factor: Option<&str>,
) -> Result<(), EngineError> {
    if let Some(xt) = x_title {
        validate_title(xt)?;
    }
    if let Some(rf) = row_factor {
        validate_title(rf)?;
    }
    if let Some(cf) = col_factor {
        validate_title(cf)?;
    }
    if let Some(xt) = x_title {
        match arch {
            TableArchetype::Xy(t) => t.x_title = xt.to_owned(),
            TableArchetype::Survival(t) => t.time_title = xt.to_owned(),
            _ => return Err(EngineError::with_code(ErrorCode::BadInput, "x_title/time_title applies only to XY or Survival tables")),
        }
    }
    if row_factor.is_some() || col_factor.is_some() {
        match arch {
            TableArchetype::Grouped(t) => {
                if let Some(rf) = row_factor {
                    t.row_factor_title = rf.to_owned();
                }
                if let Some(cf) = col_factor {
                    t.column_factor_title = cf.to_owned();
                }
            }
            TableArchetype::Contingency(t) => {
                if let Some(rf) = row_factor {
                    t.row_factor_title = rf.to_owned();
                }
                if let Some(cf) = col_factor {
                    t.column_factor_title = cf.to_owned();
                }
            }
            _ => return Err(EngineError::with_code(ErrorCode::BadInput, "factor titles apply only to Grouped or Contingency tables")),
        }
    }
    Ok(())
}

fn validate_cell_range_bounds(ds: &DataSheet, range: SheetCellRange, target_x: bool) -> Result<(), EngineError> {
    let rows = ds.archetype.row_count();
    if range.end.row >= rows {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("range end row {} out of range (row count {rows})", range.end.row)));
    }
    if target_x {
        if !matches!(ds.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_)) {
            return Err(EngineError::with_code(ErrorCode::BadInput, "target=`x` requires an XY or Survival sheet"));
        }
        if range.start.col != 0 || range.end.col != 0 {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("range column {}..={} must be 0 when target=`x`", range.start.col, range.end.col),
            )
            .with_remediation("Omit `col`/`col_start`/`col_end` or set them to 0 when targeting the X/time column."));
        }
        let x_subcols = x_subcolumn_count(&ds.archetype);
        if range.end.subcol >= x_subcols {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("range end X subcolumn {} out of range (width {x_subcols})", range.end.subcol),
            ));
        }
        return Ok(());
    }
    let cols = ds.archetype.column_count();
    if range.end.col >= cols {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("range end column {} out of range (column count {cols})", range.end.col)));
    }
    for col in range.start.col..=range.end.col {
        let subcols = column_subcolumn_count(&ds.archetype, col);
        if range.end.subcol >= subcols {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("range end subcolumn {} out of range on column {col} (width {subcols})", range.end.subcol),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 1. table.setCell
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct SetCellParams {
    /// Target data sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    #[serde(flatten)]
    coord: CoordInput,
    #[serde(flatten)]
    cell: CellPayloadFields,
}

fn ensure_mv_edit_preserves_calculated(
    before: &DataSheet,
    after: &DataSheet,
    cols_placed: usize,
    target_range: curvebook_doc::SheetCellRange,
) -> Result<(), EngineError> {
    let TableArchetype::MultipleVariables(before_mv) = &before.archetype else {
        return Ok(());
    };
    if cols_placed > 0 {
        ensure_mv_columns_editable(before, target_range.start.col..=target_range.end.col)?;
    }
    if let TableArchetype::MultipleVariables(after_mv) = &after.archetype {
        let max_len = before_mv.variables.len().max(after_mv.variables.len());
        let touched = (0..max_len).filter(|&col| match (before_mv.variables.get(col), after_mv.variables.get(col)) {
            (Some(b), Some(a)) => matches!(b.source, VariableSource::Calculated { .. }) && (b.title != a.title || b.cells != a.cells),
            _ => false,
        });
        ensure_mv_columns_editable(before, touched)?;
    }
    Ok(())
}

fn ensure_mv_columns_editable<I>(ds: &DataSheet, cols: I) -> Result<(), EngineError>
where
    I: IntoIterator<Item = usize>,
{
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return Ok(());
    };
    for col in cols {
        if let Some(var) = mv.variables.get(col)
            && let VariableSource::Calculated { formula } = &var.source
        {
            return Err(EngineError::with_code(
                ErrorCode::Conflict,
                format!("column {} (`{}`) is a formula-driven calculated variable and cannot be edited manually", col, var.title),
            )
            .with_details(json!({
                "sheet_id": ds.id.as_str(),
                "column": col,
                "title": var.title,
                "formula": formula,
            }))
            .with_remediation("Edit the formula via `table.setCalculatedVariable` or target an entered column."));
        }
    }
    Ok(())
}

pub(crate) struct FastSetCellPrep {
    pub(crate) sheet_id: SheetId,
    pub(crate) coord: CellCoord,
    pub(crate) cell: Cell,
}

/// Parse and pre-validate `table.setCell` when it can execute safely as an O(1) in-place delta
/// without whole-document cloning, DAG recalculation, or checkpoint re-deserialization.
///
/// Returns `Ok(None)` whenever the mutation must use the full general path (row auto-expansion,
/// linked regions, calculated variables, downstream dependents, active graph point overrides,
/// dirty analyses, or non-finite values whose structured error format expects full validation).
pub(crate) fn prepare_fast_set_cell(doc: &ProjectDocument, raw: &Value) -> Result<Option<FastSetCellPrep>, EngineError> {
    if !doc.graph_sheets.iter().all(|g| g.spec.point_overrides.is_empty()) {
        return Ok(None);
    }
    if doc.result_sheets.iter().any(|r| matches!(r.status, curvebook_doc::ResultStatus::Dirty)) {
        return Ok(None);
    }
    let p: SetCellParams = parse_params(&command_schema::<SetCellParams>(), raw.clone())?;
    let sheet_id = parse_sheet_id(&p.sheet_id)?;
    let coord = p.coord.resolve()?;
    let cell = p.cell.into_cell()?;
    if matches!(cell.value, CellValue::Number(v) if !v.is_finite()) {
        return Ok(None);
    }
    let ds = doc.data_sheet(&sheet_id).ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("data sheet `{sheet_id}` not found")))?;
    ensure_sheet_not_frozen(ds)?;
    ensure_mv_columns_editable(ds, std::iter::once(coord.col))?;
    ensure_coord_outside_linked_regions(ds, coord, "edit")?;

    // Only use O(1) delta when slot already exists, table has no links/formulas, and no other
    // sheet has a dependency edge pointing to `sheet_id`.
    if !ds.linked_regions.is_empty()
        || crate::dag::data_sheet_has_calculated_variables(ds)
        || ds.archetype.data_cell(coord).is_none()
        || doc.data_sheets.iter().any(|other| other.linked_regions.iter().any(|r| r.spec.source.upstream_sheet_id() == Some(&sheet_id)))
        || doc.result_sheets.iter().any(|res| doc.upstream_dependencies(&res.id).contains(&sheet_id))
    {
        return Ok(None);
    }
    Ok(Some(FastSetCellPrep { sheet_id, coord, cell }))
}

pub(crate) fn fast_set_cell_outcome(id: &SheetId, coord: CellCoord) -> CommandOutcome {
    CommandOutcome::mutated(
        "table.setCell",
        "updated cell",
        json!({
            "sheet_id": id.as_str(),
            "row": coord.row,
            "col": coord.col,
            "subcol": coord.subcol,
            "recalculated_sheets": 0_usize,
            "refreshed_links": 0_usize,
        }),
    )
}

fn mutate_doc_data_sheet_and_recalc<T>(
    ctx: &mut Ctx<'_>,
    raw_id: &str,
    prune_overrides: bool,
    f: impl FnOnce(&mut ProjectDocument, &SheetId) -> Result<T, EngineError>,
) -> Result<(SheetId, T, super::project::RecalcSummary), EngineError> {
    let id = parse_sheet_id(raw_id)?;
    let doc = &mut ctx.session.doc;
    let before_active = prune_overrides.then(|| snapshot_active_series_lengths(doc, &id));
    let _ = require_unfrozen_data_sheet_mut(doc, &id)?;
    let out = f(doc, &id)?;
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), None, Some(ctx.cancelled))?;
    if let Some(before) = before_active.as_ref() {
        prune_shortened_series_overrides(doc, &id, before);
    }
    Ok((id, out, recalc))
}

fn mutate_data_sheet_and_recalc<T>(
    ctx: &mut Ctx<'_>,
    raw_id: &str,
    prune_overrides: bool,
    f: impl FnOnce(&mut DataSheet) -> Result<T, EngineError>,
) -> Result<(SheetId, T, super::project::RecalcSummary), EngineError> {
    mutate_doc_data_sheet_and_recalc(ctx, raw_id, prune_overrides, |doc, id| {
        let ds = require_unfrozen_data_sheet_mut(doc, id)?;
        f(ds)
    })
}

fn run_set_cell(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetCellParams = parse_params(&command_schema::<SetCellParams>(), raw)?;
    let coord = p.coord.resolve()?;
    let cell = p.cell.into_cell()?;
    let (id, (), recalc) = mutate_data_sheet_and_recalc(ctx, &p.sheet_id, true, |ds| {
        ensure_mv_columns_editable(ds, std::iter::once(coord.col))?;
        ensure_coord_outside_linked_regions(ds, coord, "edit")?;
        ds.set_cell(coord, cell)?;
        Ok(())
    })?;
    Ok(CommandOutcome::mutated(
        "table.setCell",
        "updated cell",
        json!({
            "sheet_id": id.as_str(),
            "row": coord.row,
            "col": coord.col,
            "subcol": coord.subcol,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 2. table.setCells
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct CellEditItem {
    #[serde(flatten)]
    coord: CoordInput,
    #[serde(flatten)]
    cell: CellPayloadFields,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetCellsParams {
    /// Target data sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// List of explicit cell edits.
    edits: Option<Vec<CellEditItem>>,
    /// 2D array of cell values or objects.
    #[schemars(with = "Option<Vec<Vec<CellValueSchema>>>")]
    rows: Option<Vec<Vec<Value>>>,
    /// Starting row for `rows` grid (default 0).
    start_row: Option<usize>,
    /// Starting column for `rows` grid (default 0).
    start_col: Option<usize>,
    /// Subcolumn index for `rows` grid (default 0).
    start_subcol: Option<usize>,
}

fn collect_set_cells_edits(p: SetCellsParams) -> Result<Vec<(CellCoord, Cell)>, EngineError> {
    if let Some(items) = p.edits {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            out.push((item.coord.resolve()?, item.cell.into_cell()?));
        }
        return Ok(out);
    }
    if let Some(rows_arr) = p.rows {
        let start_row = p.start_row.unwrap_or(0);
        let start_col = p.start_col.unwrap_or(0);
        let start_subcol = p.start_subcol.unwrap_or(0);
        let mut out = Vec::new();
        for (r_off, row_cells) in rows_arr.iter().enumerate() {
            let row = start_row.checked_add(r_off).ok_or_else(|| bad_input("row index overflow"))?;
            for (c_off, cell_val) in row_cells.iter().enumerate() {
                let col = start_col.checked_add(c_off).ok_or_else(|| bad_input("column index overflow"))?;
                out.push((CellCoord::new(row, col, start_subcol), parse_cell_from_grid_value(cell_val)?));
            }
        }
        return Ok(out);
    }
    Err(bad_input("table.setCells requires `edits` or `rows`").with_remediation("Supply either an `edits` array or a 2D `rows` array."))
}

fn run_set_cells(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetCellsParams = parse_params(&command_schema::<SetCellsParams>(), raw)?;
    let raw_id = p.sheet_id.clone();
    let edits = collect_set_cells_edits(p)?;
    let (id, edited_count, recalc) = mutate_data_sheet_and_recalc(ctx, &raw_id, true, |ds| {
        ensure_mv_columns_editable(ds, edits.iter().map(|(c, _)| c.col))?;
        ensure_coords_outside_linked_regions(ds, edits.iter().map(|(c, _)| *c), "edit")?;
        for (coord, cell) in &edits {
            ds.set_cell(*coord, cell.clone())?;
        }
        Ok(edits.len())
    })?;
    Ok(CommandOutcome::mutated(
        "table.setCells",
        "updated cells",
        json!({
            "sheet_id": id.as_str(),
            "edited_count": edited_count,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 3. table.setXCells
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct XCellEditItem {
    /// Zero-based row index.
    row: usize,
    /// Zero-based X subcolumn index (default 0).
    subcol: Option<usize>,
    #[serde(flatten)]
    cell: CellPayloadFields,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetXCellsParams {
    /// Target XY or Survival sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// List of X/time cell edits.
    edits: Option<Vec<XCellEditItem>>,
    /// Sequential X/time values starting at `start_row`.
    #[schemars(with = "Option<Vec<CellValueSchema>>")]
    values: Option<Vec<Value>>,
    /// Starting row for `values` batch (default 0).
    start_row: Option<usize>,
    /// Single row index when editing one X cell.
    row: Option<usize>,
    /// X subcolumn index (default 0).
    subcol: Option<usize>,
    #[serde(flatten)]
    cell: CellPayloadFields,
}

fn collect_set_x_updates(p: SetXCellsParams) -> Result<Vec<(usize, usize, Cell)>, EngineError> {
    if let Some(items) = p.edits {
        let mut updates = Vec::with_capacity(items.len());
        for item in items {
            updates.push((item.row, item.subcol.unwrap_or(0), item.cell.into_cell()?));
        }
        return Ok(updates);
    }
    if let Some(vals) = p.values {
        let start_row = p.start_row.unwrap_or(0);
        let subcol = p.subcol.unwrap_or(0);
        let mut updates = Vec::with_capacity(vals.len());
        for (idx, val) in vals.iter().enumerate() {
            let row = start_row.checked_add(idx).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "row index overflow"))?;
            updates.push((row, subcol, parse_cell_from_grid_value(val)?));
        }
        return Ok(updates);
    }
    let row = p.row.ok_or_else(|| {
        EngineError::with_code(ErrorCode::BadInput, "table.setXCells requires `edits`, `values`, or `row`")
            .with_remediation("Provide `values`, `edits`, or a single `{ row, value }`.")
    })?;
    Ok(vec![(row, p.subcol.unwrap_or(0), p.cell.into_cell()?)])
}

fn run_set_x_cells(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetXCellsParams = parse_params(&command_schema::<SetXCellsParams>(), raw)?;
    let raw_id = p.sheet_id.clone();
    let updates = collect_set_x_updates(p)?;
    let (id, edited_count, recalc) = mutate_data_sheet_and_recalc(ctx, &raw_id, true, |ds| {
        for (row, subcol, cell) in &updates {
            ds.set_x_or_time_cell(*row, *subcol, cell.clone())?;
        }
        Ok(updates.len())
    })?;
    Ok(CommandOutcome::mutated(
        "table.setXCells",
        "updated X/time cells",
        json!({
            "sheet_id": id.as_str(),
            "edited_count": edited_count,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 4. table.setRowTitles
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetRowTitlesParams {
    /// Target data sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Batch of row titles (null clears a title).
    titles: Option<Vec<Option<String>>>,
    /// Starting row for `titles` batch (default 0).
    start_row: Option<usize>,
    /// Single row index when `titles` is omitted.
    row: Option<usize>,
    /// Single row title (omit or null to clear).
    title: Option<String>,
}

fn run_set_row_titles(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetRowTitlesParams = parse_params(&command_schema::<SetRowTitlesParams>(), raw)?;
    if let Some(ref title) = p.title {
        validate_title(title)?;
    }
    let (id, updated, recalc) = mutate_data_sheet_and_recalc(ctx, &p.sheet_id, false, |ds| {
        let mut updated = 0usize;
        if let Some(titles) = p.titles {
            let start_row = p.start_row.unwrap_or(0);
            validate_row_title_batch(&ds.archetype, start_row, &titles)?;
            for (offset, title) in titles.into_iter().enumerate() {
                set_row_title(&mut ds.archetype, start_row.saturating_add(offset), title)?;
                updated = updated.saturating_add(1);
            }
        } else {
            let row = p.row.ok_or_else(|| {
                bad_input("table.setRowTitles requires `titles` or `row`").with_remediation("Supply a `titles` array or a single `row` index.")
            })?;
            set_row_title(&mut ds.archetype, row, p.title)?;
            updated = 1;
        }
        Ok(updated)
    })?;
    Ok(CommandOutcome::mutated(
        "table.setRowTitles",
        "updated row titles",
        json!({
            "sheet_id": id.as_str(),
            "updated_count": updated,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 5 & 6. table.setGroupTitles / table.setTitles
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetTitlesParams {
    /// Target data sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// X or Survival time axis title.
    x_title: Option<String>,
    /// Row factor title (Grouped/Contingency).
    row_factor_title: Option<String>,
    /// Column factor title (Grouped/Contingency).
    column_factor_title: Option<String>,
    /// Batch of dataset/column/group titles.
    titles: Option<Vec<String>>,
    /// Starting column index for `titles` batch (default 0).
    start_col: Option<usize>,
    /// Single dataset/column title.
    title: Option<String>,
    /// Column index for single `title` update.
    col: Option<usize>,
    /// Batch of row titles starting at row 0.
    row_titles: Option<Vec<Option<String>>>,
}

fn run_set_group_titles(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_set_titles_with_id("table.setGroupTitles", ctx, raw)
}

fn run_set_titles(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_set_titles_with_id("table.setTitles", ctx, raw)
}

fn propagate_mv_variable_renames(doc: &mut ProjectDocument, sheet_id: &SheetId, old_titles: &[String]) {
    let renames: Vec<(String, String)> = {
        let Some(ds) = doc.data_sheet(sheet_id) else {
            return;
        };
        let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
            return;
        };
        let mut pairs = Vec::new();
        for (idx, old_title) in old_titles.iter().enumerate() {
            let old_t = old_title.trim();
            if old_t.is_empty() || old_titles.get(..idx).is_some_and(|prevs| prevs.iter().any(|prev| prev.trim() == old_t)) {
                continue;
            }
            if let Some(var) = mv.variables.get(idx) {
                let new_t = var.title.trim();
                if !new_t.is_empty() && old_t != new_t {
                    pairs.push((old_t.to_owned(), new_t.to_owned()));
                }
            }
        }
        pairs
    };
    if renames.is_empty() {
        return;
    }
    let rename_refs: Vec<(&str, &str)> = renames.iter().map(|(o, n)| (o.as_str(), n.as_str())).collect();
    if let Some(ds) = doc.data_sheet_mut(sheet_id)
        && let TableArchetype::MultipleVariables(mv) = &mut ds.archetype
    {
        for var in &mut mv.variables {
            if let VariableSource::Calculated { formula } = &mut var.source
                && let Some(rewritten) = curvebook_transform::formula::rewrite_formula_column_refs(formula, &rename_refs)
            {
                *formula = rewritten;
            }
        }
    }
    for rs in &mut doc.result_sheets {
        if rs.analysis.source_sheet_id.as_ref() == Some(sheet_id)
            && let AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, derived_columns, .. }) = &mut rs.analysis.kind
        {
            if let Some(f) = filter_formula.as_mut()
                && let Some(rewritten) = curvebook_transform::formula::rewrite_formula_column_refs(f, &rename_refs)
            {
                *f = rewritten;
            }
            for dc in derived_columns {
                if let Some(rewritten) = curvebook_transform::formula::rewrite_formula_column_refs(&dc.formula, &rename_refs) {
                    dc.formula = rewritten;
                }
            }
        }
    }
}

fn run_set_titles_with_id(command_id: &str, ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetTitlesParams = parse_params(&command_schema::<SetTitlesParams>(), raw)?;
    if p.titles.is_some() && p.title.is_some() {
        return Err(bad_input(format!("cannot combine `titles` with single `title` in `{command_id}`"))
            .with_remediation("Supply either `titles` (with optional `start_col`) or single `title` with `col`, not both.")
            .with_details(json!({ "pointer": "/titles" })));
    }
    if p.title.is_some() && p.col.is_none() {
        return Err(bad_input("missing required `col` for single `title` update")
            .with_remediation("Provide `col` (or pass a `titles` array).")
            .with_details(json!({ "pointer": "/col" })));
    }
    if p.x_title.is_none()
        && p.row_factor_title.is_none()
        && p.column_factor_title.is_none()
        && p.titles.as_ref().is_none_or(Vec::is_empty)
        && p.title.is_none()
        && p.row_titles.as_ref().is_none_or(Vec::is_empty)
    {
        return Err(bad_input(format!(
            "{command_id} requires at least one title update (`title`/`col`, `titles`, `x_title`, `row_factor_title`, `column_factor_title`, or `row_titles`)"
        )));
    }
    if let Some(ref title) = p.title {
        validate_title(title)?;
    }
    let raw_id = p.sheet_id.clone();
    let (id, updated, recalc) = mutate_doc_data_sheet_and_recalc(ctx, &raw_id, false, |doc, id| {
        let (old_mv_titles, updated): (Option<Vec<String>>, usize) = {
            let ds = require_unfrozen_data_sheet_mut(doc, id)?;
            let old = match &ds.archetype {
                TableArchetype::MultipleVariables(mv) => Some(mv.variables.iter().map(|v| v.title.clone()).collect()),
                _ => None,
            };
            if let Some(xt) = p.x_title.as_deref() {
                validate_title(xt)?;
            }
            if let Some(rf) = p.row_factor_title.as_deref() {
                validate_title(rf)?;
            }
            if let Some(cf) = p.column_factor_title.as_deref() {
                validate_title(cf)?;
            }
            if let Some(titles) = p.titles.as_deref() {
                for title in titles {
                    validate_title(title)?;
                }
            }
            if let Some(row_titles) = p.row_titles.as_deref() {
                validate_row_title_batch(&ds.archetype, 0, row_titles)?;
            }

            let mut updated = 0usize;
            if let Some(xt) = p.x_title.as_deref() {
                set_x_or_factor_title(&mut ds.archetype, Some(xt), None, None)?;
                updated = updated.saturating_add(1);
            }
            if p.row_factor_title.is_some() || p.column_factor_title.is_some() {
                set_x_or_factor_title(&mut ds.archetype, None, p.row_factor_title.as_deref(), p.column_factor_title.as_deref())?;
                updated = updated.saturating_add(1);
            }
            if let Some(titles) = p.titles {
                let start_col = p.start_col.unwrap_or(0);
                for (offset, title) in titles.iter().enumerate() {
                    let col = start_col.checked_add(offset).ok_or_else(|| bad_input("column index overflow"))?;
                    set_group_title(&mut ds.archetype, col, title)?;
                    updated = updated.saturating_add(1);
                }
            } else if let Some(title) = p.title.as_deref() {
                let col = p.col.ok_or_else(|| {
                    bad_input("missing required `col` for single `title` update").with_remediation("Provide `col` (or pass a `titles` array).")
                })?;
                set_group_title(&mut ds.archetype, col, title)?;
                updated = updated.saturating_add(1);
            }
            if let Some(row_titles) = p.row_titles {
                for (idx, title) in row_titles.into_iter().enumerate() {
                    set_row_title(&mut ds.archetype, idx, title)?;
                    updated = updated.saturating_add(1);
                }
            }
            if updated == 0 {
                return Err(bad_input(format!("{command_id} applied zero title updates")));
            }
            (old, updated)
        };
        if let Some(old_titles) = old_mv_titles.as_deref() {
            propagate_mv_variable_renames(doc, id, old_titles);
        }
        Ok(updated)
    })?;
    Ok(CommandOutcome::mutated(
        command_id,
        "updated group titles",
        json!({
            "sheet_id": id.as_str(),
            "updated_count": updated,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 7. table.setSubcolumnTitles
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetSubcolumnTitlesParams {
    /// Target Nested data sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Optional group index (omit to update all groups).
    col: Option<usize>,
    /// Batch of subcolumn titles.
    titles: Option<Vec<String>>,
    /// Starting subcolumn index (default 0).
    start_subcol: Option<usize>,
    /// Single subcolumn index when `titles` is omitted.
    subcol: Option<usize>,
    /// Single subcolumn title.
    title: Option<String>,
}

fn run_set_subcolumn_titles(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetSubcolumnTitlesParams = parse_params(&command_schema::<SetSubcolumnTitlesParams>(), raw)?;
    if let Some(ref title) = p.title {
        validate_title(title)?;
    }
    if let Some(ref titles) = p.titles {
        for title in titles {
            validate_title(title)?;
        }
    }
    let raw_id = p.sheet_id.clone();
    let (id, updated, recalc) = mutate_data_sheet_and_recalc(ctx, &raw_id, false, |ds| {
        let TableArchetype::Nested(nested) = &mut ds.archetype else {
            return Err(bad_input(format!("sheet `{}` does not store explicit subcolumn titles (only Nested tables store subcolumn_titles)", ds.id))
                .with_remediation("Select a Nested data table."));
        };

        let mut updated = 0usize;
        if let Some(titles) = p.titles {
            let start_subcol = p.start_subcol.unwrap_or(0);
            for (offset, title) in titles.iter().enumerate() {
                let subcol = start_subcol.checked_add(offset).ok_or_else(|| bad_input("subcolumn index overflow"))?;
                set_nested_subcol_title(nested, p.col, subcol, title)?;
                updated = updated.saturating_add(1);
            }
        } else {
            let subcol = p.subcol.ok_or_else(|| bad_input("missing required `subcol`"))?;
            let title = p.title.as_deref().ok_or_else(|| bad_input("missing required `title`"))?;
            set_nested_subcol_title(nested, p.col, subcol, title)?;
            updated = 1;
        }
        Ok(updated)
    })?;
    Ok(CommandOutcome::mutated(
        "table.setSubcolumnTitles",
        "updated subcolumn titles",
        json!({
            "sheet_id": id.as_str(),
            "updated_count": updated,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 8. table.excludeCells & 11. table.clearCells Range Helper
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct FlatRangeBounds {
    /// Explicit start/end cell range.
    range: Option<CellRangeInput>,
    /// Range start row.
    row_start: Option<usize>,
    /// Range end row.
    row_end: Option<usize>,
    /// Range start column.
    col_start: Option<usize>,
    /// Range end column.
    col_end: Option<usize>,
    /// Range start subcolumn.
    subcol_start: Option<usize>,
    /// Range end subcolumn.
    subcol_end: Option<usize>,
    /// Single cell row index.
    row: Option<usize>,
    /// Single cell column index.
    col: Option<usize>,
    /// Single cell subcolumn index.
    subcol: Option<usize>,
}

impl FlatRangeBounds {
    fn resolve_range(&self) -> Result<Option<SheetCellRange>, EngineError> {
        if let Some(r) = &self.range {
            return r.resolve().map(Some);
        }
        if self.row_start.is_none() && self.row_end.is_none() && self.col_start.is_none() && self.col_end.is_none() {
            return Ok(None);
        }
        let r0 = self.row_start.or(self.row).unwrap_or(0);
        let r1 = self.row_end.unwrap_or(r0);
        let c0 = self.col_start.or(self.col).unwrap_or(0);
        let c1 = self.col_end.unwrap_or(c0);
        let s0 = self.subcol_start.or(self.subcol).unwrap_or(0);
        let s1 = self.subcol_end.unwrap_or(s0);
        Ok(Some(SheetCellRange::new(CellCoord::new(r0.min(r1), c0.min(c1), s0.min(s1)), CellCoord::new(r0.max(r1), c0.max(c1), s0.max(s1)))))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExcludeCellCoordItem {
    #[serde(flatten)]
    coord: CoordInput,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExcludeCellsParams {
    /// Target data sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Whether to exclude (true) or include (false); default true.
    excluded: Option<bool>,
    /// Set to `x` to target X/time column cells.
    target: Option<String>,
    #[serde(flatten)]
    bounds: FlatRangeBounds,
    /// List of cell coordinates.
    cells: Option<Vec<ExcludeCellCoordItem>>,
    /// Single coordinate object `{ row, col, subcol? }`.
    #[schemars(with = "Option<FlatCoordInput>")]
    coord: Option<Box<CoordInput>>,
}

fn run_exclude_cells(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ExcludeCellsParams = parse_params(&command_schema::<ExcludeCellsParams>(), raw)?;
    let excluded = p.excluded.unwrap_or(true);
    let target_x = p.target.as_deref().is_some_and(|t| t.eq_ignore_ascii_case("x"));
    let range_opt = p.bounds.resolve_range()?;
    let raw_id = p.sheet_id.clone();

    let (id, updated, recalc) = mutate_data_sheet_and_recalc(ctx, &raw_id, false, |ds| {
        let mut updated = 0usize;
        let action = if excluded { "exclude" } else { "unexclude" };
        if let Some(range) = range_opt {
            validate_cell_range_bounds(ds, range, target_x)?;
            if target_x {
                for r in range.start.row..=range.end.row {
                    for s in range.start.subcol..=range.end.subcol {
                        ds.set_x_or_time_excluded(r, s, excluded)?;
                        updated = updated.saturating_add(1);
                    }
                }
            } else {
                for r in range.start.row..=range.end.row {
                    for c in range.start.col..=range.end.col {
                        for s in range.start.subcol..=range.end.subcol {
                            ensure_coord_outside_linked_regions(ds, CellCoord::new(r, c, s), action)?;
                        }
                    }
                }
                for r in range.start.row..=range.end.row {
                    for c in range.start.col..=range.end.col {
                        for s in range.start.subcol..=range.end.subcol {
                            ds.set_excluded(CellCoord::new(r, c, s), excluded)?;
                            updated = updated.saturating_add(1);
                        }
                    }
                }
            }
        } else if let Some(coords) = p.cells {
            if target_x {
                let mut x_coords = Vec::with_capacity(coords.len());
                for item in coords {
                    let row = item.coord.row.ok_or_else(|| bad_input("missing required X cell `row`"))?;
                    x_coords.push((row, item.coord.subcol.unwrap_or(0)));
                }
                for (row, subcol) in x_coords {
                    ds.set_x_or_time_excluded(row, subcol, excluded)?;
                    updated = updated.saturating_add(1);
                }
            } else {
                let resolved: Vec<CellCoord> = coords.iter().map(|item| item.coord.resolve()).collect::<Result<_, _>>()?;
                ensure_coords_outside_linked_regions(ds, resolved.iter().copied(), action)?;
                for coord in resolved {
                    ds.set_excluded(coord, excluded)?;
                    updated = updated.saturating_add(1);
                }
            }
        } else if target_x {
            let row = p.bounds.row.ok_or_else(|| bad_input("missing required X cell `row`"))?;
            ds.set_x_or_time_excluded(row, p.bounds.subcol.unwrap_or(0), excluded)?;
            updated = 1;
        } else {
            let coord = CoordInput { coord: p.coord, row: p.bounds.row, col: p.bounds.col, subcol: p.bounds.subcol }.resolve()?;
            ensure_coord_outside_linked_regions(ds, coord, action)?;
            ds.set_excluded(coord, excluded)?;
            updated = 1;
        }
        Ok(updated)
    })?;
    Ok(CommandOutcome::mutated(
        "table.excludeCells",
        "updated exclusion mask",
        json!({
            "sheet_id": id.as_str(),
            "excluded": excluded,
            "updated_count": updated,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 9. table.excludeCellsWithReason
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
enum TupleOrObjCoord {
    Tuple(#[schemars(length(min = 2, max = 3))] Vec<usize>),
    Obj(CoordInput),
}

impl TupleOrObjCoord {
    fn resolve(&self) -> Result<CellCoord, EngineError> {
        match self {
            Self::Tuple(arr) => match arr.as_slice() {
                [row, col] => Ok(CellCoord::new(*row, *col, 0)),
                [row, col, subcol] => Ok(CellCoord::new(*row, *col, *subcol)),
                _ => Err(EngineError::with_code(ErrorCode::BadInput, "coordinate tuple must be `[row, col]` or `[row, col, subcol]`")),
            },
            Self::Obj(obj) => obj.resolve(),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExcludeCellsWithReasonParams {
    /// Target data sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Short non-empty audit reason tag.
    #[schemars(length(min = 1))]
    reason_tag: String,
    /// Optional explanatory audit note.
    note: Option<String>,
    /// Optional explicit UTC timestamp string.
    timestamp_utc: Option<String>,
    /// List of coordinates to exclude.
    #[schemars(length(min = 1))]
    coords: Option<Vec<TupleOrObjCoord>>,
    /// Explicit cell range to exclude.
    range: Option<CellRangeInput>,
    #[serde(flatten)]
    coord: CoordInput,
}

fn current_utc_timestamp() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        String::from("unix:0")
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |dur| dur.as_secs());
        format!("unix:{secs}")
    }
}

fn expand_exclusion_range(range: SheetCellRange, arch: &TableArchetype) -> Result<Vec<CellCoord>, EngineError> {
    let max_row = arch.row_count();
    let max_col = arch.column_count();
    if range.end.row >= max_row || range.end.col >= max_col {
        return Err(EngineError::with_code(ErrorCode::BadInput, "exclusion range exceeds table bounds"));
    }
    let mut coords = Vec::new();
    for row in range.start.row..=range.end.row {
        for col in range.start.col..=range.end.col {
            let max_sub = column_subcolumn_count(arch, col);
            if range.end.subcol >= max_sub {
                return Err(EngineError::with_code(ErrorCode::BadInput, "exclusion range subcolumn exceeds table bounds"));
            }
            for subcol in range.start.subcol..=range.end.subcol {
                coords.push(CellCoord::new(row, col, subcol));
            }
        }
    }
    Ok(coords)
}

fn resolve_exclusion_targets(p: &ExcludeCellsWithReasonParams, arch: &TableArchetype) -> Result<Vec<CellCoord>, EngineError> {
    if let Some(arr) = &p.coords {
        if arr.is_empty() {
            return Err(EngineError::with_code(ErrorCode::BadInput, "table.excludeCellsWithReason `coords` cannot be empty"));
        }
        return arr.iter().map(TupleOrObjCoord::resolve).collect();
    }
    if let Some(r) = &p.range {
        return expand_exclusion_range(r.resolve()?, arch);
    }
    if p.coord.row.is_some() || p.coord.coord.is_some() {
        return Ok(vec![p.coord.resolve()?]);
    }
    Err(EngineError::with_code(ErrorCode::BadInput, "table.excludeCellsWithReason requires `coords`, `range`, or `row`/`col`")
        .with_remediation("Provide target cell coordinates via `coords`, `range`, or `row`/`col`."))
}

fn run_exclude_cells_with_reason(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ExcludeCellsWithReasonParams = parse_params(&command_schema::<ExcludeCellsWithReasonParams>(), raw)?;
    let _ = parse_sheet_id(&p.sheet_id)?;
    if p.reason_tag.trim().is_empty() {
        return Err(bad_input("table.excludeCellsWithReason `reason_tag` cannot be empty")
            .with_remediation("Supply a non-empty `reason_tag` (e.g. `outlier` or `pipetting_error`)."));
    }
    let timestamp_utc = match &p.timestamp_utc {
        Some(ts) if ts.trim().is_empty() => {
            return Err(bad_input("`timestamp_utc` cannot be empty"));
        }
        Some(ts) => ts.clone(),
        None => current_utc_timestamp(),
    };
    let note = p.note.clone().unwrap_or_default();
    validate_title(&p.reason_tag)?;
    validate_title(&note)?;
    validate_title(&timestamp_utc)?;
    let raw_id = p.sheet_id.clone();

    let (sheet_id, count, recalc) = mutate_data_sheet_and_recalc(ctx, &raw_id, false, |ds| {
        let coords = resolve_exclusion_targets(&p, &ds.archetype)?;
        for coord in &coords {
            if ds.archetype.data_cell(*coord).is_none() {
                return Err(bad_input(format!("cell ({}, {}, {}) is out of bounds for `{}`", coord.row, coord.col, coord.subcol, ds.id.as_str())));
            }
        }
        ensure_coords_outside_linked_regions(ds, coords.iter().copied(), "exclude")?;
        let mut notes_by_coord: BTreeMap<CellCoord, CellExclusionNote> =
            std::mem::take(&mut ds.exclusion_notes).into_iter().map(|entry| (entry.coord, entry)).collect();
        for &coord in &coords {
            ds.set_excluded(coord, true)?;
            notes_by_coord.insert(
                coord,
                CellExclusionNote { coord, reason_tag: p.reason_tag.clone(), note: note.clone(), timestamp_utc: timestamp_utc.clone() },
            );
        }
        ds.exclusion_notes = notes_by_coord.into_values().collect();
        Ok(coords.len())
    })?;
    Ok(CommandOutcome::mutated(
        "table.excludeCellsWithReason",
        format!("excluded {count} cell(s) with reason `{}`", p.reason_tag),
        json!({
            "sheet_id": sheet_id.as_str(),
            "count": count,
            "reason_tag": p.reason_tag,
            "note": note,
            "timestamp_utc": timestamp_utc,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 10. table.exclusionAuditReport
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExclusionAuditReportParams {
    /// Optional sheet ID to filter report.
    #[schemars(length(min = 1))]
    sheet_id: Option<String>,
}

fn collect_sheet_exclusions(ds: &DataSheet, entries: &mut Vec<Value>, annotated_count: &mut usize) {
    match &ds.archetype {
        TableArchetype::Xy(xy) => {
            for (row, sub_cells) in xy.x_cells.iter().enumerate() {
                for (subcol, cell) in sub_cells.iter().enumerate() {
                    if cell.excluded {
                        entries.push(json!({
                            "sheet_id": ds.id.as_str(),
                            "sheet_title": ds.title,
                            "target": "x",
                            "row": row,
                            "col": Value::Null,
                            "subcol": subcol,
                            "coord": { "row": row, "subcol": subcol, "target": "x" },
                            "has_reason": false,
                            "reason_tag": Value::Null,
                            "note": Value::Null,
                            "timestamp_utc": Value::Null,
                        }));
                    }
                }
            }
        }
        TableArchetype::Survival(surv) => {
            for (row, cell) in surv.times.iter().enumerate() {
                if cell.excluded {
                    entries.push(json!({
                        "sheet_id": ds.id.as_str(),
                        "sheet_title": ds.title,
                        "target": "x",
                        "row": row,
                        "col": Value::Null,
                        "subcol": 0,
                        "coord": { "row": row, "subcol": 0, "target": "x" },
                        "has_reason": false,
                        "reason_tag": Value::Null,
                        "note": Value::Null,
                        "timestamp_utc": Value::Null,
                    }));
                }
            }
        }
        _ => {}
    }

    let rows = ds.archetype.row_count();
    let cols = ds.archetype.column_count();
    for row in 0..rows {
        for col in 0..cols {
            let subcols = column_subcolumn_count(&ds.archetype, col);
            for subcol in 0..subcols {
                let coord = CellCoord::new(row, col, subcol);
                if !ds.archetype.data_cell(coord).is_some_and(|cell| cell.excluded) {
                    continue;
                }
                let note_opt = ds.exclusion_note(coord);
                if note_opt.is_some() {
                    *annotated_count += 1;
                }
                entries.push(json!({
                    "sheet_id": ds.id.as_str(),
                    "sheet_title": ds.title,
                    "row": row,
                    "col": col,
                    "subcol": subcol,
                    "coord": { "row": row, "col": col, "subcol": subcol },
                    "has_reason": note_opt.is_some(),
                    "reason_tag": note_opt.map(|n| n.reason_tag.as_str()),
                    "note": note_opt.map(|n| n.note.as_str()),
                    "timestamp_utc": note_opt.map(|n| n.timestamp_utc.as_str()),
                }));
            }
        }
    }
}

fn run_exclusion_audit_report(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ExclusionAuditReportParams = parse_params(&command_schema::<ExclusionAuditReportParams>(), raw)?;
    let target_id = p.sheet_id.as_deref().map(parse_sheet_id).transpose()?;
    let doc = ctx.session.document();
    let mut entries = Vec::new();
    let mut annotated_count = 0usize;

    if let Some(ref sid) = target_id {
        let ds = require_data_sheet(doc, sid)?;
        collect_sheet_exclusions(ds, &mut entries, &mut annotated_count);
    } else {
        for ds in &doc.data_sheets {
            collect_sheet_exclusions(ds, &mut entries, &mut annotated_count);
        }
    }

    let total_excluded = entries.len();
    Ok(CommandOutcome::read_only(
        "table.exclusionAuditReport",
        format!("audited {total_excluded} excluded cell(s)"),
        json!({
            "sheet_id": target_id.as_ref().map(SheetId::as_str),
            "total_excluded": total_excluded,
            "annotated_count": annotated_count,
            "unannotated_count": total_excluded - annotated_count,
            "exclusions": entries,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 11. table.clearCells
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
struct ClearCellsParams {
    /// Target data sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Clear all data cells across the sheet.
    all: Option<bool>,
    /// When `all` is true, also clear X/time cells.
    clear_x: Option<bool>,
    /// Detach live linked regions on this sheet (all regions unless `region_id` is supplied).
    unlink: Option<bool>,
    /// When `unlink` is true, detach only the linked region matching this identifier.
    #[schemars(length(min = 1))]
    region_id: Option<String>,
    #[serde(flatten)]
    bounds: FlatRangeBounds,
    /// List of cell coordinates to clear.
    cells: Option<Vec<CoordInput>>,
    /// Single cell coordinate `{ row, col, subcol? }`.
    #[schemars(with = "Option<FlatCoordInput>")]
    coord: Option<Box<CoordInput>>,
}

fn clear_all_cells_in_sheet(ds: &mut DataSheet, clear_x: bool) -> Result<usize, EngineError> {
    let rows = ds.archetype.row_count();
    let cols = ds.archetype.column_count();
    let mut cleared = 0usize;
    for r in 0..rows {
        if clear_x && matches!(ds.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_)) {
            let x_subcols = x_subcolumn_count(&ds.archetype);
            for s in 0..x_subcols {
                ds.set_x_or_time_cell(r, s, Cell::empty())?;
                cleared = cleared.saturating_add(1);
            }
        }
        for c in 0..cols {
            let is_calc_mv = matches!(
                &ds.archetype,
                TableArchetype::MultipleVariables(mv)
                    if mv.variables.get(c).is_some_and(|v| matches!(v.source, VariableSource::Calculated { .. }))
            );
            if is_calc_mv {
                continue;
            }
            let subcols = column_subcolumn_count(&ds.archetype, c);
            for s in 0..subcols {
                ds.set_cell(CellCoord::new(r, c, s), Cell::empty())?;
                cleared = cleared.saturating_add(1);
            }
        }
    }
    Ok(cleared)
}

fn run_clear_cells(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ClearCellsParams = parse_params(&command_schema::<ClearCellsParams>(), raw)?;
    let range_opt = p.bounds.resolve_range()?;
    let raw_id = p.sheet_id.clone();

    let (id, cleared, recalc) = mutate_data_sheet_and_recalc(ctx, &raw_id, true, |ds| {
        let unlink = p.unlink.unwrap_or(false);
        let target_region_id = p.region_id.as_deref();
        if unlink
            && let Some(rid) = target_region_id
            && !ds.linked_regions.iter().any(|r| r.id == rid)
        {
            return Err(EngineError::with_code(ErrorCode::NotFound, format!("linked region `{rid}` not found on `{}`", ds.id)));
        }
        let check_remaining_links = !unlink || target_region_id.is_some();
        let skip_region_id = if unlink { target_region_id } else { None };
        let mut cleared = 0usize;
        if p.all.unwrap_or(false) {
            if check_remaining_links && let Some(region) = ds.linked_regions.iter().find(|r| skip_region_id != Some(r.id.as_str())) {
                return Err(linked_region_conflict_error(&ds.id, region, "clear", None));
            }
            cleared = clear_all_cells_in_sheet(ds, p.clear_x.unwrap_or(false))?;
        } else if let Some(range) = range_opt {
            validate_cell_range_bounds(ds, range, false)?;
            ensure_mv_columns_editable(ds, range.start.col..=range.end.col)?;
            if check_remaining_links {
                for r in range.start.row..=range.end.row {
                    for c in range.start.col..=range.end.col {
                        for s in range.start.subcol..=range.end.subcol {
                            ensure_coord_outside_linked_regions_except(ds, CellCoord::new(r, c, s), "clear", skip_region_id)?;
                        }
                    }
                }
            }
            for r in range.start.row..=range.end.row {
                for c in range.start.col..=range.end.col {
                    for s in range.start.subcol..=range.end.subcol {
                        ds.set_cell(CellCoord::new(r, c, s), Cell::empty())?;
                        cleared = cleared.saturating_add(1);
                    }
                }
            }
        } else if let Some(coords) = p.cells {
            let resolved: Vec<CellCoord> = coords.iter().map(CoordInput::resolve).collect::<Result<_, _>>()?;
            ensure_mv_columns_editable(ds, resolved.iter().map(|c| c.col))?;
            if check_remaining_links {
                for coord in resolved.iter().copied() {
                    ensure_coord_outside_linked_regions_except(ds, coord, "clear", skip_region_id)?;
                }
            }
            for coord in resolved {
                ds.set_cell(coord, Cell::empty())?;
                cleared = cleared.saturating_add(1);
            }
        } else {
            let coord = CoordInput { coord: p.coord, row: p.bounds.row, col: p.bounds.col, subcol: p.bounds.subcol }.resolve()?;
            ensure_mv_columns_editable(ds, std::iter::once(coord.col))?;
            if check_remaining_links {
                ensure_coord_outside_linked_regions_except(ds, coord, "clear", skip_region_id)?;
            }
            ds.set_cell(coord, Cell::empty())?;
            cleared = 1;
        }

        if unlink {
            match target_region_id {
                Some(rid) => ds.linked_regions.retain(|r| r.id != rid),
                None => ds.linked_regions.clear(),
            }
        }
        Ok(cleared)
    })?;
    Ok(CommandOutcome::mutated(
        "table.clearCells",
        "cleared cells",
        json!({
            "sheet_id": id.as_str(),
            "cleared_count": cleared,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 12. table.paste
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct PlacementFields {
    /// Target anchor coordinate.
    anchor: Option<FlexibleCoordInput>,
    /// Anchor row index (default 0).
    start_row: Option<usize>,
    /// Anchor column index (default 0).
    start_col: Option<usize>,
    /// Anchor subcolumn index (default 0).
    start_subcol: Option<usize>,
    /// Placement mode: overwrite, insert_rows, append_rows, append_columns.
    mode: Option<String>,
    /// Propagate source titles to target headers.
    propagate_title_header: Option<bool>,
    /// Skip empty runs when placing cells.
    skip_contiguous_blanks: Option<bool>,
    /// Transpose rows and columns.
    transpose: Option<bool>,
}

impl PlacementFields {
    fn resolve_anchor(&self) -> CellCoord {
        if let Some(a) = &self.anchor {
            return a.resolve();
        }
        CellCoord::new(self.start_row.unwrap_or(0), self.start_col.unwrap_or(0), self.start_subcol.unwrap_or(0))
    }

    fn to_placement_spec(&self, default_propagate: bool) -> Result<ImportPlacementSpec, EngineError> {
        let transpose = self.transpose.unwrap_or(false);
        Ok(ImportPlacementSpec {
            mode: parse_placement_mode(self.mode.as_deref().unwrap_or("overwrite"))?,
            anchor: self.resolve_anchor(),
            propagate_title_header: self.propagate_title_header.unwrap_or(default_propagate),
            skip_contiguous_blanks: self.skip_contiguous_blanks.unwrap_or(false),
            transpose,
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PasteParams {
    /// Target data sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Delimited clipboard text (TSV/CSV).
    text: Option<String>,
    /// 2D array of pasted cell values.
    #[schemars(with = "Option<Vec<Vec<CellValueSchema>>>")]
    rows: Option<Vec<Vec<Value>>>,
    /// Column delimiter (`auto`, `,`, `\\t`/`tab`, `;`, or single character).
    delimiter: Option<String>,
    /// Decimal separator (`.`, `,`, `dot`, `comma`, `period`, or `auto`).
    decimal_separator: Option<String>,
    /// Infer European comma decimal separator when ambiguous.
    normalize_decimal: Option<bool>,
    /// Optional import filter specification.
    #[schemars(with = "Option<ImportFilterSchema>")]
    filter: Option<Value>,
    #[serde(flatten)]
    placement: PlacementFields,
}

fn serialize_2d_rows_as_tsv(rows_arr: &[Vec<Value>], decimal_separator: char) -> Result<String, EngineError> {
    let mut lines = Vec::with_capacity(rows_arr.len());
    for row_cells in rows_arr {
        let mut tokens = Vec::with_capacity(row_cells.len());
        for cell_val in row_cells {
            let cell = parse_cell_value_with_options(cell_val, decimal_separator, true)?;
            tokens.push(format_linked_paste_tsv_cell(&cell));
        }
        lines.push(tokens.join("\t"));
    }
    Ok(lines.join("\n"))
}

fn apply_paste_payload(
    ds: &mut DataSheet,
    spec: &ImportLinkSpec,
    payload: ImportSourcePayload<'_>,
    opts: &ImportPipelineOptions,
    other_project_cells: usize,
) -> Result<ImportPipelineReport, EngineError> {
    let has_calc_mv = matches!(
        &ds.archetype,
        TableArchetype::MultipleVariables(mv) if mv.variables.iter().any(|v| matches!(v.source, VariableSource::Calculated { .. }))
    );
    let needs_budget_stage = other_project_cells > MAX_PROJECT_CELLS.saturating_sub(MAX_SHEET_CELLS);
    if !has_calc_mv && ds.linked_regions.is_empty() && !needs_budget_stage {
        let report = apply_import_link_pipeline(ds, spec, payload, opts, None)?;
        ds.prune_stale_exclusion_notes();
        ds.prune_stale_plate_well_origins();
        return Ok(report);
    }
    let mut staged = ds.clone();
    let report = apply_import_link_pipeline(&mut staged, spec, payload, opts, None)?;
    staged.prune_stale_exclusion_notes();
    staged.prune_stale_plate_well_origins();
    validate_project_cells(other_project_cells, staged.archetype.allocated_cell_slots())?;
    if has_calc_mv {
        ensure_mv_edit_preserves_calculated(ds, &staged, report.cols_placed, report.target_range)?;
    }
    if report.rows_placed > 0 && report.cols_placed > 0 {
        ensure_range_outside_linked_regions(ds, report.target_range, "paste")?;
        if matches!(spec.placement.mode, curvebook_doc::PlacementMode::InsertRows) {
            let shift_span = curvebook_doc::SheetCellRange::new(
                CellCoord::new(spec.placement.anchor.row, report.target_range.start.col, report.target_range.start.subcol),
                CellCoord::new(usize::MAX, report.target_range.end.col, report.target_range.end.subcol),
            );
            ensure_range_outside_linked_regions(ds, shift_span, "insert rows into")?;
        }
    }
    *ds = staged;
    Ok(report)
}

fn run_paste(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: PasteParams = parse_params(&command_schema::<PasteParams>(), raw)?;
    let filter = parse_import_filter_val(p.filter.as_ref())?;
    let placement = p.placement.to_placement_spec(false)?;
    let spec = ImportLinkSpec { source: LinkSource::Clipboard, filter, placement };
    let raw_id = p.sheet_id.clone();

    let (id, report, recalc) = mutate_doc_data_sheet_and_recalc(ctx, &raw_id, true, |doc, id| {
        let existing_rows_before = doc.data_sheet(id).map_or(0, |d| d.archetype.row_count());
        let other_project_cells = doc.total_allocated_cells()?.saturating_sub(doc.data_sheet(id).map_or(0, |d| d.archetype.allocated_cell_slots()));
        let ds = require_unfrozen_data_sheet_mut(doc, id)?;
        let old_cols = ds.archetype.column_count();
        let existing_links = ds.linked_regions.len();
        let opts = ImportPipelineOptions::default();

        let report = if let Some(text) = p.text.as_deref() {
            let delimiter = resolve_delimiter(p.delimiter.as_deref(), text)?;
            let decimal_separator = resolve_decimal_char(p.decimal_separator.as_deref(), p.normalize_decimal.unwrap_or(false), text, delimiter)?;
            let payload = ImportSourcePayload::DelimitedText { text, delimiter, decimal_separator };
            apply_paste_payload(ds, &spec, payload, &opts, other_project_cells)?
        } else if let Some(rows_arr) = p.rows.as_ref() {
            let dec = parse_explicit_decimal_opt(p.decimal_separator.as_deref())?.unwrap_or('.');
            let tsv = serialize_2d_rows_as_tsv(rows_arr, dec)?;
            let payload = ImportSourcePayload::DelimitedText { text: &tsv, delimiter: '\t', decimal_separator: '.' };
            apply_paste_payload(ds, &spec, payload, &opts, other_project_cells)?
        } else {
            return Err(bad_input("table.paste requires `text` or `rows`").with_remediation("Supply delimited `text` or a 2D `rows` array."));
        };

        ds.linked_regions.truncate(existing_links);
        let new_cols = ds.archetype.column_count();
        let appended_ds = (spec.placement.mode == PlacementMode::AppendColumns && new_cols > old_cols).then(|| ds.clone());
        if let Some(updated_ds) = appended_ds {
            let default_spec = GraphSpec::default_for_data_sheet(&updated_ds);
            for g in &mut doc.graph_sheets {
                if g.spec.primary_source_sheet_id == *id
                    && matches!(g.spec.kind, GraphKind::Xy | GraphKind::Column | GraphKind::Grouped | GraphKind::Survival)
                    && g.spec.series.len() == old_cols
                    && g.spec.series.iter().enumerate().all(|(i, s)| s.data_set_index == i)
                    && let Some(new_series) = default_spec.series.get(old_cols..new_cols)
                {
                    g.spec.series.extend(new_series.iter().cloned());
                }
            }
        }
        if spec.placement.mode == PlacementMode::InsertRows && spec.placement.anchor.row < existing_rows_before && report.rows_placed > 0 {
            shift_point_overrides_on_insert_rows(doc, id, spec.placement.anchor.row, report.rows_placed);
        }
        Ok(report)
    })?;
    Ok(CommandOutcome::mutated(
        "table.paste",
        "pasted table data",
        json!({
            "sheet_id": id.as_str(),
            "rows_placed": report.rows_placed,
            "cols_placed": report.cols_placed,
            "target_range": &report.target_range,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 13. table.pasteLink
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportCsvParams {
    /// Data or Result sheet identifier to export.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Optional output file path.
    #[schemars(length(min = 1))]
    path: Option<String>,
    /// Delimiter character (`,`, `\\t`/`tab`, `;`, or 1 char).
    delimiter: Option<String>,
    /// Explicit decimal separator (`.`, `,`, `dot`, `comma`, or `period`).
    decimal_separator: Option<String>,
}

fn run_export_csv(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ExportCsvParams = parse_params(&command_schema::<ExportCsvParams>(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let mut csv_opts = CsvOptions::csv();
    if let Some(delim) = p.delimiter.as_deref() {
        csv_opts.delimiter = parse_delimiter_char(delim)?;
    }
    if let Some(dec) = parse_explicit_decimal_opt(p.decimal_separator.as_deref())? {
        csv_opts.decimal_separator = dec;
    }

    let doc = ctx.session.document();
    let csv_text = if let Some(ds) = doc.data_sheet(&id) {
        export_datasheet_to_csv(ds, &csv_opts)?
    } else if let Some(res) = doc.result_sheet(&id) {
        try_export_result_sheet_tidy_csv(res, &csv_opts)?
    } else {
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("data or result sheet `{id}` not found"))
            .with_remediation("Provide an existing data or result sheet ID."));
    };

    if let Some(path) = p.path.as_deref() {
        ctx.session.path_policy.write_atomic_bytes(path, csv_text.as_bytes())?;
    }
    Ok(CommandOutcome::read_only("table.exportCsv", "exported CSV", json!({ "sheet_id": id.as_str(), "path": p.path, "csv": csv_text })))
}

// ---------------------------------------------------------------------------
// 16. table.importPrism
// ---------------------------------------------------------------------------

fn resolve_mv_variable_index(mv: &MultipleVariablesTable, col_ref: Option<&ColRef>, fallback_title: Option<&str>) -> Result<usize, EngineError> {
    match col_ref {
        Some(ColRef::Index(idx)) => Ok(*idx),
        Some(ColRef::Title(title)) => {
            mv.find_variable(title).ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("variable `{title}` not found")))
        }
        None => {
            if let Some(title) = fallback_title {
                mv.find_variable(title).ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("variable `{title}` not found")))
            } else {
                Err(bad_input("missing required `col` variable index or title")
                    .with_remediation("Identify the variable by zero-based index or title."))
            }
        }
    }
}

fn require_mv_table_mut<'a>(doc: &'a mut ProjectDocument, id: &SheetId) -> Result<&'a mut MultipleVariablesTable, EngineError> {
    let ds = require_unfrozen_data_sheet_mut(doc, id)?;
    ds_mv_table_mut(ds)
}

fn ds_mv_table_mut(ds: &mut DataSheet) -> Result<&mut MultipleVariablesTable, EngineError> {
    match &mut ds.archetype {
        TableArchetype::MultipleVariables(mv) => Ok(mv),
        _ => Err(bad_input(format!("sheet `{}` is not a MultipleVariables table", ds.id)).with_remediation("Select a MultipleVariables data sheet.")),
    }
}

fn parse_variable_role(raw: &str) -> Result<VariableRole, EngineError> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "continuous" | "numeric" => Ok(VariableRole::Continuous),
        "categorical" | "category" | "factor" => Ok(VariableRole::Categorical),
        "label" | "identifier" | "id" | "text" => Ok(VariableRole::Label),
        "calculated" | "formula" => Ok(VariableRole::Calculated),
        _ => Err(bad_input(format!("unknown variable role `{raw}`")).with_remediation("Use continuous, categorical, label, calculated, or auto.")),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetVariableRoleParams {
    /// Target MultipleVariables sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Zero-based variable index or column title.
    col: Option<ColRef>,
    /// Fallback variable column title.
    title: Option<String>,
    /// Role: continuous, categorical, label, calculated, or auto.
    #[schemars(length(min = 1))]
    role: String,
}

fn run_set_variable_role(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetVariableRoleParams = parse_params(&command_schema::<SetVariableRoleParams>(), raw)?;
    let raw_id = p.sheet_id.clone();
    let (id, (col_idx, final_role, role_inferred), recalc) = mutate_data_sheet_and_recalc(ctx, &raw_id, false, |ds| {
        let col_idx = {
            let TableArchetype::MultipleVariables(ref mv) = ds.archetype else {
                return Err(EngineError::with_code(
                    ErrorCode::Conflict,
                    format!("sheet `{}` is `{}` rather than MultipleVariables", ds.id.as_str(), ds.kind().label()),
                ));
            };
            let col_idx = resolve_mv_variable_index(mv, p.col.as_ref(), p.title.as_deref())?;
            ensure_mv_columns_editable(ds, std::iter::once(col_idx))?;
            col_idx
        };
        let is_auto = p.role.eq_ignore_ascii_case("auto") || p.role.eq_ignore_ascii_case("inferred");
        let requested_role = if is_auto {
            None
        } else {
            let parsed_role = parse_variable_role(&p.role)?;
            if parsed_role == VariableRole::Calculated {
                return Err(bad_input("cannot assign role `Calculated` directly on an entered variable; use `table.setCalculatedVariable`")
                    .with_remediation(
                        "Set a calculated formula via `table.setCalculatedVariable`, or choose `continuous`, `categorical`, `label`, or `auto`.",
                    ));
            }
            Some(parsed_role)
        };
        let mv = ds_mv_table_mut(ds)?;
        let var = mv.variables.get_mut(col_idx).ok_or_else(|| bad_input(format!("variable column {col_idx} out of range")))?;

        if let Some(role) = requested_role {
            var.lock_role(role);
        } else {
            var.role_inferred = true;
            var.refresh_inferred_role();
        }
        Ok((col_idx, var.role, var.role_inferred))
    })?;
    Ok(CommandOutcome::mutated(
        "table.setVariableRole",
        "updated variable role",
        json!({
            "sheet_id": id.as_str(),
            "column": col_idx,
            "role": format!("{final_role:?}"),
            "role_inferred": role_inferred,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecodeEntryInput {
    /// Existing level text.
    from: String,
    /// Replacement level text.
    to: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
enum RecodeMappingsInput {
    Pairs(Vec<RecodeEntryInput>),
    Map(BTreeMap<String, String>),
}

impl RecodeMappingsInput {
    fn into_pairs(self) -> Vec<(String, String)> {
        match self {
            Self::Pairs(items) => items.into_iter().map(|i| (i.from, i.to)).collect(),
            Self::Map(map) => map.into_iter().collect(),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecodeLevelsParams {
    /// Target MultipleVariables sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Zero-based variable index or column title.
    col: Option<ColRef>,
    /// Fallback variable column title.
    title: Option<String>,
    /// Recode mapping: object `{ old: new }` or array of `{ from, to }`.
    mappings: RecodeMappingsInput,
}

struct NormalizedRangeBounds {
    min_row: usize,
    max_row: usize,
    min_cs: (usize, usize),
    max_cs: (usize, usize),
}

fn normalize_range_bounds(range: curvebook_doc::SheetCellRange) -> NormalizedRangeBounds {
    let min_row = range.start.row.min(range.end.row);
    let max_row = range.start.row.max(range.end.row);
    let a = (range.start.col, range.start.subcol);
    let b = (range.end.col, range.end.subcol);
    NormalizedRangeBounds { min_row, max_row, min_cs: a.min(b), max_cs: a.max(b) }
}

fn coord_in_sheet_range(range: curvebook_doc::SheetCellRange, coord: CellCoord) -> bool {
    let bounds = normalize_range_bounds(range);
    let cs = (coord.col, coord.subcol);
    coord.row >= bounds.min_row && coord.row <= bounds.max_row && cs >= bounds.min_cs && cs <= bounds.max_cs
}

fn ranges_overlap(a: curvebook_doc::SheetCellRange, b: curvebook_doc::SheetCellRange) -> bool {
    let ab = normalize_range_bounds(a);
    let bb = normalize_range_bounds(b);
    ab.min_row <= bb.max_row && bb.min_row <= ab.max_row && ab.min_cs <= bb.max_cs && bb.min_cs <= ab.max_cs
}

fn linked_region_conflict_error(sheet_id: &SheetId, region: &curvebook_doc::LinkedDataRegion, action: &str, coord: Option<CellCoord>) -> EngineError {
    let detail = match coord {
        Some(c) => json!({
            "sheet_id": sheet_id.as_str(),
            "region_id": region.id,
            "row": c.row,
            "col": c.col,
            "subcol": c.subcol,
        }),
        None => json!({
            "sheet_id": sheet_id.as_str(),
            "region_id": region.id,
        }),
    };
    EngineError::with_code(
        ErrorCode::Conflict,
        format!("cannot {action} cells inside live linked region `{}` on `{sheet_id}`", region.id),
    )
    .with_remediation("Detach the link first (`table.clearCells` with `unlink: true` or re-import with `link: false`) before editing cells inside the linked region.")
    .with_details(detail)
}

fn ensure_coord_outside_linked_regions_except(
    ds: &DataSheet,
    coord: CellCoord,
    action: &str,
    skip_region_id: Option<&str>,
) -> Result<(), EngineError> {
    for region in &ds.linked_regions {
        if skip_region_id == Some(region.id.as_str()) {
            continue;
        }
        if coord_in_sheet_range(region.target_range, coord) {
            return Err(linked_region_conflict_error(&ds.id, region, action, Some(coord)));
        }
    }
    Ok(())
}

fn ensure_coord_outside_linked_regions(ds: &DataSheet, coord: CellCoord, action: &str) -> Result<(), EngineError> {
    ensure_coord_outside_linked_regions_except(ds, coord, action, None)
}

fn ensure_coords_outside_linked_regions(ds: &DataSheet, coords: impl IntoIterator<Item = CellCoord>, action: &str) -> Result<(), EngineError> {
    if ds.linked_regions.is_empty() {
        return Ok(());
    }
    for coord in coords {
        ensure_coord_outside_linked_regions(ds, coord, action)?;
    }
    Ok(())
}

fn ensure_range_outside_linked_regions(ds: &DataSheet, range: curvebook_doc::SheetCellRange, action: &str) -> Result<(), EngineError> {
    for region in &ds.linked_regions {
        if ranges_overlap(region.target_range, range) {
            return Err(linked_region_conflict_error(&ds.id, region, action, None));
        }
    }
    Ok(())
}

fn run_recode_levels(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: RecodeLevelsParams = parse_params(&command_schema::<RecodeLevelsParams>(), raw)?;
    let raw_id = p.sheet_id.clone();
    let pairs = p.mappings.into_pairs();
    let (id, (col_idx, recoded_count, updated_reference_levels), recalc) = mutate_doc_data_sheet_and_recalc(ctx, &raw_id, false, |doc, id| {
        if let Some(ds) = doc.data_sheet(id)
            && let TableArchetype::MultipleVariables(ref mv) = ds.archetype
            && let Ok(col_idx) = resolve_mv_variable_index(mv, p.col.as_ref(), p.title.as_deref())
        {
            ensure_mv_columns_editable(ds, std::iter::once(col_idx))?;
            let rows = ds.archetype.row_count();
            if rows > 0 {
                let col_range = curvebook_doc::SheetCellRange::new(CellCoord::new(0, col_idx, 0), CellCoord::new(rows - 1, col_idx, 0));
                ensure_range_outside_linked_regions(ds, col_range, "recode")?;
            }
        }
        let mv = require_mv_table_mut(doc, id)?;
        let col_idx = resolve_mv_variable_index(mv, p.col.as_ref(), p.title.as_deref())?;
        let var = mv.variables.get_mut(col_idx).ok_or_else(|| bad_input(format!("variable column {col_idx} out of range")))?;
        let recoded_count = var.recode_levels(&pairs);
        for gs in &mut doc.graph_sheets {
            gs.spec.recode_variable_levels(id, col_idx, &pairs);
        }
        let updated_reference_levels = recode_analysis_reference_levels(&mut doc.result_sheets, id, col_idx, &pairs);
        Ok((col_idx, recoded_count, updated_reference_levels))
    })?;
    Ok(CommandOutcome::mutated(
        "table.recodeLevels",
        "recoded variable levels",
        json!({
            "sheet_id": id.as_str(),
            "column": col_idx,
            "recoded_count": recoded_count,
            "updated_reference_levels": updated_reference_levels,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

fn recode_reference_pairs(reference_levels: &mut [(usize, String)], col_idx: usize, mappings: &[(String, String)]) -> usize {
    let mut updated = 0;
    for (ref_col, level) in reference_levels.iter_mut() {
        if *ref_col != col_idx {
            continue;
        }
        for (from, to) in mappings {
            if (*level == *from || level.trim() == from.trim()) && from != to {
                let canonical = to.trim();
                *level = if canonical.is_empty() { to.clone() } else { canonical.to_owned() };
                updated += 1;
                break;
            }
        }
    }
    updated
}

fn recode_analysis_reference_levels(
    result_sheets: &mut [ResultSheet],
    source_sheet_id: &SheetId,
    col_idx: usize,
    mappings: &[(String, String)],
) -> usize {
    if mappings.is_empty() {
        return 0;
    }
    let mut updated = 0;
    for rs in result_sheets {
        if rs.analysis.source_sheet_id.as_ref() != Some(source_sheet_id) {
            continue;
        }
        match &mut rs.analysis.kind {
            AnalysisKind::GeneralizedLinearModel(spec) => {
                updated += recode_reference_pairs(&mut spec.reference_levels, col_idx, mappings);
            }
            AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { reference_levels, .. }) => {
                updated += recode_reference_pairs(reference_levels, col_idx, mappings);
            }
            _ => {}
        }
    }
    updated
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
enum OrderParamValue {
    Explicit(Vec<String>),
    Kind(String),
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetCategoricalOrderParams {
    /// Target MultipleVariables sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Zero-based variable index or column title.
    col: Option<ColRef>,
    /// Fallback variable column title.
    title: Option<String>,
    /// Serialized LevelOrderSpec.
    #[schemars(with = "Option<Map<String, Value>>")]
    order_spec: Option<Value>,
    /// Explicit category order list.
    levels: Option<Vec<String>>,
    /// Explicit array of levels or order mode (`entry_order`, `alphabetical`, `frequency_descending`, `frequency_ascending`, `explicit`).
    order: Option<OrderParamValue>,
    /// Reverse resulting level ordering.
    reversed: Option<bool>,
    /// Optional reference category label.
    reference_level: Option<String>,
}

fn parse_level_order_from_params(p: &SetCategoricalOrderParams) -> Result<LevelOrderSpec, EngineError> {
    if let Some(spec_val) = &p.order_spec {
        if let Ok(parsed) = serde_json::from_value::<LevelOrderSpec>(spec_val.clone()) {
            return Ok(parsed);
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct NestedOrderSpec {
            levels: Option<Vec<String>>,
            order: Option<OrderParamValue>,
            reversed: Option<bool>,
        }
        let nested: NestedOrderSpec = serde_json::from_value(spec_val.clone()).map_err(|err| bad_input(format!("invalid `order_spec`: {err}")))?;
        return resolve_level_order_fields(nested.levels.as_deref(), nested.order.as_ref(), nested.reversed.unwrap_or(false));
    }
    resolve_level_order_fields(p.levels.as_deref(), p.order.as_ref(), p.reversed.unwrap_or(false))
}

fn resolve_level_order_fields(levels: Option<&[String]>, order: Option<&OrderParamValue>, reversed: bool) -> Result<LevelOrderSpec, EngineError> {
    if let Some(arr) = levels {
        return Ok(LevelOrderSpec { order: LevelOrderKind::Explicit(arr.to_vec()), reversed });
    }
    if let Some(OrderParamValue::Explicit(arr)) = order {
        return Ok(LevelOrderSpec { order: LevelOrderKind::Explicit(arr.clone()), reversed });
    }
    let Some(OrderParamValue::Kind(kind_str)) = order else {
        return Err(bad_input("table.setCategoricalOrder requires `order`, `levels`, or `order_spec`")
            .with_remediation("Provide an explicit `order` array or an order mode string."));
    };
    let norm: String = kind_str.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let kind = match norm.as_str() {
        "entryorder" | "entry" => LevelOrderKind::EntryOrder,
        "alphabetical" | "alpha" => LevelOrderKind::Alphabetical,
        "frequencydescending" | "freqdesc" => LevelOrderKind::FrequencyDescending,
        "frequencyascending" | "freqasc" => LevelOrderKind::FrequencyAscending,
        "explicit" => {
            return Err(bad_input("explicit categorical order requires `levels` array"));
        }
        _ => {
            return Err(bad_input(format!("unknown level order `{kind_str}`")));
        }
    };
    Ok(LevelOrderSpec { order: kind, reversed })
}

fn run_set_categorical_order(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetCategoricalOrderParams = parse_params(&command_schema::<SetCategoricalOrderParams>(), raw)?;
    let _ = parse_sheet_id(&p.sheet_id)?;
    let ref_level = match p.reference_level.as_deref() {
        Some(raw_ref) => {
            let trimmed = raw_ref.trim();
            if trimmed.is_empty() {
                return Err(bad_input("`reference_level` cannot be empty"));
            }
            Some(trimmed.to_owned())
        }
        None => None,
    };
    let explicit_spec = if p.order_spec.is_some() || p.levels.is_some() || p.order.is_some() {
        Some(parse_level_order_from_params(&p)?)
    } else {
        if ref_level.is_none() {
            return Err(bad_input("table.setCategoricalOrder requires `order`, `levels`, `order_spec`, or `reference_level`")
                .with_remediation("Provide an explicit `order`, `levels`, `order_spec`, or `reference_level`."));
        }
        None
    };

    let raw_id = p.sheet_id.clone();
    let (id, (col_idx, applied_levels), recalc) = mutate_data_sheet_and_recalc(ctx, &raw_id, false, |ds| {
        let mv = ds_mv_table_mut(ds)?;
        let col_idx = resolve_mv_variable_index(mv, p.col.as_ref(), p.title.as_deref())?;
        let var = mv.variables.get_mut(col_idx).ok_or_else(|| bad_input(format!("variable column {col_idx} out of range")))?;

        let base_spec = match explicit_spec {
            Some(spec) => spec,
            None => {
                let mut spec = var.level_order.clone();
                if let Some(rev) = p.reversed {
                    spec.reversed = rev;
                }
                spec
            }
        };
        var.level_order = if let Some(ref_lvl) = ref_level {
            let mut levels: Vec<String> = match base_spec.order {
                LevelOrderKind::Explicit(mut list) => {
                    if base_spec.reversed {
                        list.reverse();
                    }
                    for seen in var.ordered_levels_with(&LevelOrderSpec::default()) {
                        if !list.contains(&seen.level) {
                            list.push(seen.level);
                        }
                    }
                    list
                }
                _ => var.ordered_levels_with(&base_spec).into_iter().map(|s| s.level).collect(),
            };
            levels.retain(|lvl| lvl != &ref_lvl);
            levels.insert(0, ref_lvl);
            LevelOrderSpec { order: LevelOrderKind::Explicit(levels), reversed: false }
        } else {
            base_spec
        };

        let applied_levels: Vec<String> = var.ordered_levels().into_iter().map(|s| s.level).collect();
        Ok((col_idx, applied_levels))
    })?;
    Ok(CommandOutcome::mutated(
        "table.setCategoricalOrder",
        "updated categorical level order",
        json!({
            "sheet_id": id.as_str(),
            "column": col_idx,
            "levels": applied_levels,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetCalculatedVariableParams {
    /// Target MultipleVariables sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Existing variable index or name (optional when `title` creates or matches a column).
    col: Option<ColRef>,
    /// Variable column title (creates a new column if not found and `col` is omitted).
    title: Option<String>,
    /// Formula expression, e.g. `= log10([Dose])` or `log10(ConcRaw)`.
    formula: String,
}

fn run_set_calculated_variable(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetCalculatedVariableParams = parse_params(&command_schema::<SetCalculatedVariableParams>(), raw)?;
    if let Some(ref title) = p.title {
        validate_title(title)?;
    }
    let raw_id = p.sheet_id.clone();
    let formula = p.formula.clone();

    let (id, col_idx, recalc) = mutate_doc_data_sheet_and_recalc(ctx, &raw_id, false, |doc, id| {
        let other_cells = doc.total_allocated_cells()?.saturating_sub(doc.data_sheet(id).map_or(0, |d| d.archetype.allocated_cell_slots()));
        let info_snapshot = doc.data_sheet(id).and_then(|ds| ds.info_sheet_id.as_ref()).and_then(|iid| doc.info_sheet(iid)).cloned();
        let ds = require_unfrozen_data_sheet_mut(doc, id)?;
        let (resolved_col_idx, created_new, row_count, cur_slots) = {
            let cur_slots = ds.archetype.allocated_cell_slots();
            let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
                return Err(
                    bad_input(format!("sheet `{id}` is not a MultipleVariables table")).with_remediation("Select a MultipleVariables data sheet.")
                );
            };
            if let Some(ref col_ref) = p.col {
                (resolve_mv_variable_index(mv, Some(col_ref), None)?, false, mv.row_count(), cur_slots)
            } else if let Some(ref title) = p.title {
                if let Some(existing) = mv.find_variable(title) {
                    (existing, false, mv.row_count(), cur_slots)
                } else {
                    (mv.variables.len(), true, mv.row_count(), cur_slots)
                }
            } else {
                return Err(bad_input("table.setCalculatedVariable requires `col` or `title`")
                    .with_remediation("Specify a target column index or a `title` for the calculated variable."));
            }
        };
        if !created_new {
            ensure_range_outside_linked_regions(
                ds,
                SheetCellRange::new(CellCoord::new(0, resolved_col_idx, 0), CellCoord::new(row_count.saturating_sub(1), resolved_col_idx, 0)),
                "convert to calculated variable",
            )?;
        } else {
            validate_project_cells(other_cells, cur_slots.saturating_add(row_count))?;
        }

        let TableArchetype::MultipleVariables(mv) = &mut ds.archetype else {
            return Err(bad_input(format!("sheet `{id}` is not a MultipleVariables table")));
        };
        let col_idx = if created_new {
            let title = p.title.as_deref().ok_or_else(|| bad_input("missing `title` for new calculated variable"))?;
            let col = VariableColumn::new(title, row_count)?;
            mv.variables.push(col);
            mv.variables.len().saturating_sub(1)
        } else {
            resolved_col_idx
        };

        if col_idx >= mv.variables.len() {
            return Err(bad_input(format!("variable column {col_idx} out of range")));
        }
        let prev_title = if let Some(title) = &p.title {
            let var = mv.variables.get_mut(col_idx).ok_or_else(|| bad_input(format!("variable column {col_idx} out of range")))?;
            let old = var.title.clone();
            var.title = title.clone();
            Some(old)
        } else {
            None
        };
        if let Err(err) = validate_no_formula_cycle(mv, col_idx, &p.formula) {
            if created_new {
                mv.variables.pop();
            } else if let Some(old) = prev_title
                && let Some(var) = mv.variables.get_mut(col_idx)
            {
                var.title = old;
            }
            return Err(err.into());
        }
        if let Some(var) = mv.variables.get_mut(col_idx) {
            var.set_calculated_formula(&p.formula);
        }
        ds.plate_well_origins.retain(|o| o.coord.col != col_idx);

        evaluate_data_sheet_calculated_variables(ds, info_snapshot.as_ref())?;
        ds.prune_stale_plate_well_origins();
        Ok(col_idx)
    })?;
    Ok(CommandOutcome::mutated(
        "table.setCalculatedVariable",
        "updated calculated variable",
        json!({
            "sheet_id": id.as_str(),
            "column": col_idx,
            "formula": formula,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 22. table.simulateXy
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::Session;
    use crate::cmd::{Caller, build_registry};

    fn seeded_session() -> Session {
        let mut s = Session::new();
        for p in [
            json!({ "sheet_id": "xy-1", "archetype": "xy", "cols": 2, "rows": 4 }),
            json!({ "sheet_id": "col-1", "archetype": "column", "cols": 2, "rows": 4 }),
            json!({ "sheet_id": "nest-1", "archetype": "nested", "cols": 2, "rows": 3, "replicates": 3 }),
            json!({
                "sheet_id": "mv-1", "archetype": "multiple_variables", "cols": 2, "rows": 3,
                "variables": [{ "title": "Arm", "role": "Categorical" }, { "title": "Dose", "role": "Continuous" }]
            }),
        ] {
            s.execute("sheet.createData", p).expect("seed sheet");
        }
        s
    }

    #[test]
    fn enabled_predicates_gate_empty_and_typed_sessions() {
        let (empty, rich) = (Session::new(), seeded_session());
        let reg = build_registry().expect("registry");
        for (id, empty_ok, rich_ok) in [
            ("table.setCell", false, true),
            ("table.exportCsv", false, true),
            ("table.importCsv", true, true),
            ("table.exclusionAuditReport", true, true),
            ("table.setSubcolumnTitles", false, true),
            ("table.setVariableRole", false, true),
        ] {
            let pred = reg.get(id).expect("spec").enabled;
            assert_eq!((pred(&empty), pred(&rich)), (empty_ok, rich_ok), "{id}");
        }
    }

    #[test]
    fn schema_validation_returns_bad_input_and_pointer_on_bad_params() {
        let mut s = seeded_session();
        let cancel = AtomicBool::new(false);
        let err =
            s.execute_with_caller("table.setCell", json!({ "row": 0, "col": 0, "value": 1 }), Caller::Mcp, &cancel).expect_err("missing sheet_id");
        assert_eq!(err.code, ErrorCode::BadInput);
        assert_eq!(err.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str), Some("/sheet_id"));

        let err2 = s
            .execute_with_caller(
                "table.excludeCellsWithReason",
                json!({ "sheet_id": "xy-1", "reason_tag": "   ", "row": 0, "col": 0 }),
                Caller::Mcp,
                &cancel,
            )
            .expect_err("blank reason_tag");
        assert_eq!(err2.code, ErrorCode::BadInput);
        assert!(err2.remediation.is_some() && err2.to_string().contains("reason_tag"));
    }

    #[test]
    fn frozen_sheet_returns_frozen_error_and_preview_convert_is_read_only() {
        let mut s = seeded_session();
        s.execute("sheet.freeze", json!({ "sheet_id": "xy-1" })).expect("freeze");
        assert_eq!(
            s.execute("table.setCell", json!({ "sheet_id": "xy-1", "row": 0, "col": 0, "value": 10.0 })).expect_err("frozen").code,
            ErrorCode::FrozenSheet
        );

        let depth_before = s.undo_depth();
        let preview = s
            .execute("table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "Column", "allow_lossy": true, "preview_only": true }))
            .expect("preview on frozen");
        assert!(!preview.mutated && s.undo_depth() == depth_before);
        assert_eq!(
            s.execute("table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "Column", "allow_lossy": true }))
                .expect_err("mutating convert on frozen")
                .code,
            ErrorCode::FrozenSheet
        );
    }

    #[test]
    fn nested_subcolumn_titles_preserve_biological_unit_width_and_checkpoint_roundtrip() -> Result<(), String> {
        let mut s = Session::new();
        for (cmd, p) in [
            ("sheet.createData", json!({ "sheet_id": "nest-tall", "kind": "nested", "cols": 2, "rows": 4, "replicates": 2 })),
            ("table.setSubcolumnTitles", json!({ "sheet_id": "nest-tall", "col": 0, "titles": ["Unit 1", "Unit 2"] })),
            ("table.setSubcolumnTitles", json!({ "sheet_id": "nest-tall", "subcol": 1, "title": "Shared Bio 2" })),
        ] {
            s.execute(cmd, p).map_err(|e| e.to_string())?;
        }

        let oob = s.execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-tall", "col": 0, "subcol": 2, "title": "Bad" }));
        assert!(oob.as_ref().is_err_and(|e| e.code == ErrorCode::BadInput));
        let oversized = "T".repeat(curvebook_doc::MAX_TITLE_BYTES + 1);
        assert!(s.execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-tall", "col": 0, "subcol": 0, "title": oversized })).is_err());

        fn nested_of<'a>(sess: &'a Session, id: &str) -> Result<&'a curvebook_doc::NestedTable, String> {
            match &sess.document().data_sheet(&SheetId::new(id)).ok_or_else(|| format!("missing {id}"))?.archetype {
                TableArchetype::Nested(nt) => Ok(nt),
                _ => Err(String::from("expected Nested")),
            }
        }
        let nt = nested_of(&s, "nest-tall")?;
        nt.validate().map_err(|e| e.to_string())?;
        assert_eq!(nt.groups[0].subcolumn_titles, vec!["Unit 1", "Shared Bio 2"]);
        assert_eq!((nt.groups[1].subcolumn_titles.len(), nt.groups[1].subcolumn_titles[1].as_str()), (2, "Shared Bio 2"));

        for (cmd, p) in [
            ("project.createCheckpoint", json!({ "name": "cp-nested" })),
            ("table.setSubcolumnTitles", json!({ "sheet_id": "nest-tall", "col": 0, "subcol": 0, "title": "Temp" })),
            ("project.restoreCheckpoint", json!({ "name": "cp-nested" })),
        ] {
            s.execute(cmd, p).map_err(|e| e.to_string())?;
        }
        assert_eq!(nested_of(&s, "nest-tall")?.groups[0].subcolumn_titles, vec!["Unit 1", "Shared Bio 2"]);

        s.execute("sheet.createData", json!({ "sheet_id": "nest-wide", "archetype": "nested", "cols": 1, "rows": 2, "replicates": 4 }))
            .map_err(|e| e.to_string())?;
        s.execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-wide", "col": 0, "subcol": 3, "title": "Unit 4" }))
            .map_err(|e| e.to_string())?;
        let nt_wide = nested_of(&s, "nest-wide")?;
        nt_wide.validate().map_err(|e| e.to_string())?;
        assert_eq!((nt_wide.groups[0].subcolumn_titles.len(), nt_wide.groups[0].subcolumn_titles[3].as_str()), (4, "Unit 4"));
        Ok(())
    }

    #[test]
    fn mv_set_titles_rewrites_calculated_formulas_reevaluates_and_undoes_atomically() -> Result<(), String> {
        let mut s = Session::new();
        for (cmd, p) in [
            ("sheet.createData", json!({ "sheet_id": "mv-1", "archetype": "multiple_variables", "cols": 3, "rows": 3 })),
            ("table.setTitles", json!({ "sheet_id": "mv-1", "titles": ["Dose", "Weight", "Ratio"] })),
            (
                "table.setCells",
                json!({
                    "sheet_id": "mv-1",
                    "edits": [
                        { "row": 0, "col": 0, "value": 10.0 }, { "row": 1, "col": 0, "value": 20.0 }, { "row": 2, "col": 0, "value": 30.0 },
                        { "row": 0, "col": 1, "value": 2.0 }, { "row": 1, "col": 1, "value": 4.0 }, { "row": 2, "col": 1, "value": 5.0 },
                    ]
                }),
            ),
            ("table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "col": 2, "formula": "[Dose] / [Weight]" })),
        ] {
            s.execute(cmd, p).map_err(|e| e.to_string())?;
        }

        fn mv_of(sess: &Session) -> Result<&curvebook_doc::MultipleVariablesTable, String> {
            match &sess.document().data_sheet(&SheetId::new("mv-1")).ok_or("missing mv-1")?.archetype {
                TableArchetype::MultipleVariables(mv) => Ok(mv),
                _ => Err(String::from("expected MultipleVariables")),
            }
        }

        let undo_before = s.undo_depth();
        s.execute("table.setTitles", json!({ "sheet_id": "mv-1", "col": 0, "title": "Dose (mg/kg)" })).map_err(|e| e.to_string())?;
        assert_eq!(s.undo_depth(), undo_before + 1);
        let mv = mv_of(&s)?;
        assert_eq!(mv.variables[0].title, "Dose (mg/kg)");
        assert_eq!(mv.variables[2].source, VariableSource::Calculated { formula: String::from("[Dose (mg/kg)] / [Weight]") });
        let vals: Vec<Option<f64>> = mv.variables[2].cells.iter().map(Cell::active_number).collect();
        assert_eq!(vals, vec![Some(5.0), Some(5.0), Some(6.0)]);

        s.execute("table.setCell", json!({ "sheet_id": "mv-1", "row": 0, "col": 0, "value": 14.0 })).map_err(|e| e.to_string())?;
        assert_eq!(mv_of(&s)?.variables[2].cells[0].active_number(), Some(7.0));

        s.execute("edit.undo", json!({})).map_err(|e| e.to_string())?;
        s.execute("edit.undo", json!({})).map_err(|e| e.to_string())?;
        let mv_u = mv_of(&s)?;
        assert_eq!(mv_u.variables[0].title, "Dose");
        assert_eq!(mv_u.variables[2].source, VariableSource::Calculated { formula: String::from("[Dose] / [Weight]") });
        assert_eq!(mv_u.variables[2].cells[0].active_number(), Some(5.0));
        Ok(())
    }

    #[test]
    fn clear_cells_unlink_region_id_detaches_only_target_and_validates_before_mutating() -> Result<(), String> {
        let mut s = Session::new();
        for (cmd, p) in [
            ("sheet.createData", json!({ "sheet_id": "src-a", "kind": "column", "cols": 1, "rows": 2 })),
            ("sheet.createData", json!({ "sheet_id": "src-b", "kind": "column", "cols": 1, "rows": 2 })),
            (
                "table.setCells",
                json!({ "sheet_id": "src-a", "edits": [{ "row": 0, "col": 0, "value": 10.0 }, { "row": 1, "col": 0, "value": 11.0 }] }),
            ),
            (
                "table.setCells",
                json!({ "sheet_id": "src-b", "edits": [{ "row": 0, "col": 0, "value": 20.0 }, { "row": 1, "col": 0, "value": 21.0 }] }),
            ),
            ("sheet.createData", json!({ "sheet_id": "dst", "kind": "column", "cols": 2, "rows": 2 })),
            ("table.pasteLink", json!({ "target_sheet_id": "dst", "source_sheet_id": "src-a", "anchor": { "row": 0, "col": 0 } })),
            ("table.pasteLink", json!({ "target_sheet_id": "dst", "source_sheet_id": "src-b", "anchor": { "row": 0, "col": 1 } })),
        ] {
            s.execute(cmd, p).map_err(|e| e.to_string())?;
        }

        let region_ids: Vec<String> =
            s.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?.linked_regions.iter().map(|r| r.id.clone()).collect();
        assert_eq!(region_ids.len(), 2);

        let missing_err = s
            .execute("table.clearCells", json!({ "sheet_id": "dst", "row": 0, "col": 0, "unlink": true, "region_id": "missing-region" }))
            .expect_err("unknown region_id must fail");
        assert_eq!(missing_err.code, ErrorCode::NotFound);
        let dst = s.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?;
        assert_eq!(dst.linked_regions.len(), 2);
        assert_eq!(dst.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(10.0));

        s.execute("table.clearCells", json!({ "sheet_id": "dst", "cells": [], "unlink": true, "region_id": region_ids[0] }))
            .map_err(|e| e.to_string())?;
        let dst = s.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?;
        assert_eq!((dst.linked_regions.len(), dst.linked_regions[0].id.as_str()), (1, region_ids[1].as_str()));

        s.execute("table.clearCells", json!({ "sheet_id": "dst", "cells": [], "unlink": true })).map_err(|e| e.to_string())?;
        assert!(s.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?.linked_regions.is_empty());
        Ok(())
    }
}
