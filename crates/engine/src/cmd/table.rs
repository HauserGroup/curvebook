//! Data-table cell, title, exclusion, paste/link, CSV/project import, and archetype commands (Wave 9B).

use std::collections::{BTreeMap, HashSet};

use curvebook_doc::{
    AnalysisKind, BaselineOperation, Cell, CellCoord, CellValue, ColorChannelMapping, ConstantValue, DataSetColumn, DataSheet, GraphKind, GraphSheet,
    GraphSpec, HookSource, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, LevelOrderKind, LevelOrderSpec,
    LinkSource, MAX_DATA_SETS, MAX_PROJECT_CELLS, MAX_SHEET_CELLS, MultipleVariablesTable, MultiplicityMethod, MultivariateSpec, NestedGroupColumn,
    NestedTable, NormalizeAnchor, ParameterScope, PlacementMode, PointStyleOverride, ProjectDocument, ResultSheet, SheetCellRange, SheetId,
    SimulationNoise, SizeChannelMapping, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, VariableColumn,
    VariableRole, VariableSource, XColumnMode, format_linked_paste_tsv_cell, validate_project_cells, validate_rows, validate_title,
};
use curvebook_format::{CsvOptions, export_datasheet_to_csv, try_export_result_sheet_tidy_csv};
use curvebook_transform::{
    AllocationScheme, DistributionSimulationConfig, GroupAllocationWeight, ImportPipelineOptions, ImportPipelineReport, ImportSourcePayload,
    RowSampleConfig, SimulatedColumnSpec, SimulatedDistribution, SubjectRandomizationConfig, TargetArchetypeKind, apply_import_link_pipeline,
    convert_datasheet_archetype, evaluate_data_sheet_calculated_variables, preview_datasheet_conversion, random_sample_rows,
    randomize_subjects_to_groups, simulate_distribution_columns, simulate_xy_sheet, validate_no_formula_cycle,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::helpers::{
    column_subcolumn_count, ensure_sheet_not_frozen, link_data_sheet_to_info, next_unique_sheet_id, parse_sheet_id, prune_shortened_series_overrides,
    require_analyzable_data_sheet, require_data_sheet, require_info_sheet_mut, require_unfrozen_data_sheet_mut, sheet_id_exists,
    shift_point_overrides_on_insert_rows, snapshot_active_series_lengths, x_subcolumn_count,
};
use super::params::{
    array_schema, bool_schema, deserialize_strict, enum_str_schema, int_schema, non_empty_str_schema, nullable_schema, num_schema, obj_schema,
    obj_schema_with_aliases, parse_params, str_schema,
};
use super::project::recalculate_dirty_with_links;
use super::{CommandGroup, CommandSpec, Ctx, Effects};
use crate::{CommandOutcome, EngineError, ErrorCode, Session, dag};

mod import;

use import::{
    run_import_csv, run_import_plate_map, run_import_prism, run_import_pzfx, run_import_tidy_long, run_paste_link, schema_import_csv,
    schema_import_plate_map, schema_import_prism, schema_import_pzfx, schema_import_tidy_long, schema_paste_link,
};

/// Registered command specifications for the `table` group (28 commands).
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    &SPECS
}

const SPECS: [CommandSpec; 28] = [
    CommandSpec {
        id: "table.setCell",
        label: "Set Cell Value",
        menu_path: "Data/Set Cell Value",
        shortcut: "",
        summary: "Set or update a single data cell value and optional exclusion flag, then recalculate downstream sheets.",
        beginner: "Enter or edit one cell in a data table. Connected analyses and graphs refresh automatically.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: schema_set_cell,
        enabled: has_data_sheet,
        run: run_set_cell,
    },
    CommandSpec {
        id: "table.setCells",
        label: "Set Multiple Cells",
        menu_path: "Data/Set Multiple Cells",
        shortcut: "",
        summary: "Apply a batch of cell edits or a 2D row/column block to a data table, then recalculate.",
        beginner: "Update multiple cells in a data table at once from a list of edits or a 2D grid.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: schema_set_cells,
        enabled: has_data_sheet,
        run: run_set_cells,
    },
    CommandSpec {
        id: "table.setXCells",
        label: "Set X / Time Column",
        menu_path: "Data/Set X Values",
        shortcut: "",
        summary: "Set one or more X or elapsed-time column cells on an XY or Survival table.",
        beginner: "Fill or edit values in the X (or Survival time) column.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: schema_set_x_cells,
        enabled: has_data_sheet,
        run: run_set_x_cells,
    },
    CommandSpec {
        id: "table.setRowTitles",
        label: "Set Row Titles",
        menu_path: "Data/Set Row Titles",
        shortcut: "",
        summary: "Set or clear one or more row titles (or PartsOfWhole slice labels) in a data table.",
        beginner: "Label rows along the left edge of a data table.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: schema_set_row_titles,
        enabled: has_data_sheet,
        run: run_set_row_titles,
    },
    CommandSpec {
        id: "table.setGroupTitles",
        label: "Set Group Column Titles",
        menu_path: "Data/Set Column Titles",
        shortcut: "",
        summary: "Set group/dataset titles, X/time title, factor titles, or row titles on a data table.",
        beginner: "Rename columns, X/time headers, or factor titles in a data table.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: schema_set_titles,
        enabled: has_data_sheet,
        run: run_set_group_titles,
    },
    CommandSpec {
        id: "table.setTitles",
        label: "Set Column Titles",
        menu_path: "Data/Set Column Titles",
        shortcut: "",
        summary: "Set dataset column titles, X/time title, factor titles, or row titles on a data table.",
        beginner: "Rename column headers, X/time title, or factor titles in a data table.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: schema_set_titles,
        enabled: has_data_sheet,
        run: run_set_titles,
    },
    CommandSpec {
        id: "table.setSubcolumnTitles",
        label: "Set Subcolumn Titles",
        menu_path: "Data/Set Subcolumn Titles",
        shortcut: "",
        summary: "Set explicit subcolumn titles on a Nested data table across one or all groups.",
        beginner: "Rename nested replicate subcolumns for a specific group or all groups.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: schema_set_subcolumn_titles,
        enabled: has_nested_sheet,
        run: run_set_subcolumn_titles,
    },
    CommandSpec {
        id: "table.excludeCells",
        label: "Exclude Cells",
        menu_path: "Change/Exclude Values",
        shortcut: "Cmd+E",
        summary: "Toggle the exclusion mask on single cells, cell lists, or ranges (including X/time cells).",
        beginner: "Exclude or include selected values so analyses ignore them without deleting data.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: schema_exclude_cells,
        enabled: has_data_sheet,
        run: run_exclude_cells,
    },
    CommandSpec {
        id: "table.excludeCellsWithReason",
        label: "Exclude Cells With Reason",
        menu_path: "Change/Exclude Values With Reason...",
        shortcut: "",
        summary: "Exclude cells and attach a structured audit reason tag, note, and UTC timestamp.",
        beginner: "Exclude values while recording a reason tag and audit timestamp for review.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_exclude_cells_with_reason,
        enabled: has_data_sheet,
        run: run_exclude_cells_with_reason,
    },
    CommandSpec {
        id: "table.exclusionAuditReport",
        label: "Exclusion Audit Report",
        menu_path: "Data/Exclusion Audit Report...",
        shortcut: "",
        summary: "Generate a read-only audit report of excluded cells and their reason annotations.",
        beginner: "Inspect all excluded values and audit reasons across one sheet or the entire project.",
        group: CommandGroup::Table,
        effects: Effects::NONE.with(Effects::IDEMPOTENT),
        input_schema: schema_exclusion_audit_report,
        enabled: always_enabled,
        run: run_exclusion_audit_report,
    },
    CommandSpec {
        id: "table.clearCells",
        label: "Clear Cells",
        menu_path: "Edit/Clear Cells",
        shortcut: "",
        summary: "Clear single cells, lists, ranges, or an entire table, optionally breaking live links.",
        beginner: "Erase values from selected cells or clear the entire data table.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA).with(Effects::IDEMPOTENT),
        input_schema: schema_clear_cells,
        enabled: has_data_sheet,
        run: run_clear_cells,
    },
    CommandSpec {
        id: "table.paste",
        label: "Paste Special",
        menu_path: "Edit/Paste Special...",
        shortcut: "Cmd+V",
        summary: "Paste delimited clipboard text or a 2D cell grid into a table with placement, transpose, and filter options.",
        beginner: "Paste tabular data into a table starting at an anchor cell, optionally transposing or filtering.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_paste,
        enabled: has_data_sheet,
        run: run_paste,
    },
    CommandSpec {
        id: "table.pasteLink",
        label: "Paste Link",
        menu_path: "Edit/Paste Link",
        shortcut: "",
        summary: "Attach a live linked region from an internal sheet or external delimited file and sync immediately.",
        beginner: "Link a range from another sheet or CSV/TSV file so updates flow through automatically.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM),
        input_schema: schema_paste_link,
        enabled: has_data_sheet,
        run: run_paste_link,
    },
    CommandSpec {
        id: "table.importCsv",
        label: "Import CSV / TSV",
        menu_path: "File/Import/CSV or TSV...",
        shortcut: "",
        summary: "Import CSV or TSV data from a file path or inline text into a new or existing data table.",
        beginner: "Create a new data table or populate an existing one from a CSV or TSV file.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA),
        input_schema: schema_import_csv,
        enabled: always_enabled,
        run: run_import_csv,
    },
    CommandSpec {
        id: "table.exportCsv",
        label: "Export Table CSV / TSV",
        menu_path: "File/Export/Table CSV...",
        shortcut: "",
        summary: "Export a data or result sheet to CSV/TSV text and optionally write atomically to disk.",
        beginner: "Save a data or result table as CSV or TSV without changing the project.",
        group: CommandGroup::Table,
        effects: Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT),
        input_schema: schema_export_csv,
        enabled: has_data_or_result_sheet,
        run: run_export_csv,
    },
    CommandSpec {
        id: "table.importPrism",
        label: "Import External Archive",
        menu_path: "File/Import/External Archive...",
        shortcut: "",
        summary: "Import an external ZIP-based project archive from disk, base64, or raw byte array.",
        beginner: "Open or merge an external ZIP-format statistical archive into the project.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA),
        input_schema: schema_import_prism,
        enabled: always_enabled,
        run: run_import_prism,
    },
    CommandSpec {
        id: "table.importPzfx",
        label: "Import XML Project",
        menu_path: "File/Import/XML Project...",
        shortcut: "",
        summary: "Import an XML project document from disk, inline XML string, or base64 payload.",
        beginner: "Open or merge an XML-format project file into the workspace.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM).with(Effects::MAY_DISCARD_DATA),
        input_schema: schema_import_pzfx,
        enabled: always_enabled,
        run: run_import_pzfx,
    },
    CommandSpec {
        id: "table.setVariableRole",
        label: "Set Variable Role",
        menu_path: "Data/Variable Properties...",
        shortcut: "",
        summary: "Lock or re-infer the analytical role of a column in a MultipleVariables table.",
        beginner: "Mark a variable column as continuous, categorical, label, calculated, or auto-inferred.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: schema_set_variable_role,
        enabled: has_multivar_sheet,
        run: run_set_variable_role,
    },
    CommandSpec {
        id: "table.recodeLevels",
        label: "Recode Categorical Levels",
        menu_path: "Data/Recode Levels...",
        shortcut: "",
        summary: "Map and rewrite categorical values in a MultipleVariables column.",
        beginner: "Rename or merge category labels within a MultipleVariables column.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_recode_levels,
        enabled: has_multivar_sheet,
        run: run_recode_levels,
    },
    CommandSpec {
        id: "table.setCategoricalOrder",
        label: "Set Categorical Order",
        menu_path: "Data/Set Categorical Order...",
        shortcut: "",
        summary: "Configure display/reference ordering of categorical levels in a MultipleVariables column.",
        beginner: "Choose how categories are ordered (explicit list, alphabetical, entry order, or frequency).",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: schema_set_categorical_order,
        enabled: has_multivar_sheet,
        run: run_set_categorical_order,
    },
    CommandSpec {
        id: "table.setCalculatedVariable",
        label: "Add / Update Calculated Variable",
        menu_path: "Data/Calculated Variable...",
        shortcut: "",
        summary: "Create or update a formula-driven variable column in a MultipleVariables table and evaluate it.",
        beginner: "Add a computed column whose values update automatically from other variables via a formula.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: schema_set_calculated_variable,
        enabled: has_multivar_sheet,
        run: run_set_calculated_variable,
    },
    CommandSpec {
        id: "table.simulateXy",
        label: "Simulate XY Data",
        menu_path: "Insert/Simulate Data...",
        shortcut: "",
        summary: "Generate a synthetic XY data sheet from a built-in equation, parameter map, and noise model.",
        beginner: "Create synthetic XY curves with optional Gaussian, relative CV, or Poisson noise.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_simulate_xy,
        enabled: always_enabled,
        run: run_simulate_xy,
    },
    CommandSpec {
        id: "table.convertArchetype",
        label: "Convert Table Archetype",
        menu_path: "Change/Convert Table Structure...",
        shortcut: "",
        summary: "Preview or convert a data table between Column, Grouped, XY, and MultipleVariables archetypes.",
        beginner: "Reshape a data table into another structure, previewing any lossy changes before applying.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA).with(Effects::IDEMPOTENT),
        input_schema: schema_convert_archetype,
        enabled: has_data_sheet,
        run: run_convert_archetype,
    },
    CommandSpec {
        id: "table.importPlateMap",
        label: "Import Plate Map",
        menu_path: "File/Import/Plate Map...",
        shortcut: "",
        summary: "Map 96-well or 384-well microplate readings and well roles into a structured data sheet.",
        beginner: "Import 96-well or 384-well plate measurements using a plate template or role map.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_import_plate_map,
        enabled: always_enabled,
        run: run_import_plate_map,
    },
    CommandSpec {
        id: "table.importTidyLong",
        label: "Import Tidy Long Table",
        menu_path: "File/Import/Tidy Long Table...",
        shortcut: "",
        summary: "Pivot a long-format table (rows/headers or CSV/TSV file) into a Column, Grouped, XY, or MultipleVariables sheet.",
        beginner: "Reshape long/tidy tabular records into a grouped or XY data table using column mappings.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT.with(Effects::READS_FILESYSTEM),
        input_schema: schema_import_tidy_long,
        enabled: always_enabled,
        run: run_import_tidy_long,
    },
    CommandSpec {
        id: "table.randomizeGroups",
        label: "Randomize Subjects to Groups",
        menu_path: "Data/Randomize Groups...",
        shortcut: "",
        summary: "Deterministically allocate subjects across treatment groups (complete-balanced or permuted-block) and upsert ARRIVE constants on an InfoSheet.",
        beginner: "Randomizes subjects into balanced or blocked treatment arms and stores reproducibility metadata on an Info sheet.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_randomize_groups,
        enabled: always_enabled,
        run: run_randomize_groups,
    },
    CommandSpec {
        id: "table.randomSampleRows",
        label: "Random Sample Rows",
        menu_path: "Data/Random Sample Rows...",
        shortcut: "",
        summary: "Draw a reproducible Fisher-Yates random sample of rows (optional stratification) into a new data sheet.",
        beginner: "Creates a new table containing a seeded simple or stratified random subset of rows from a source sheet.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_random_sample_rows,
        enabled: has_data_or_result_sheet,
        run: run_random_sample_rows,
    },
    CommandSpec {
        id: "table.simulateDistributions",
        label: "Simulate Distributions",
        menu_path: "Insert/Simulate Distributions...",
        shortcut: "",
        summary: "Generate a Column or MultipleVariables table with columns sampled from Uniform, Gaussian, LogNormal, Binomial, Poisson, or Exponential distributions.",
        beginner: "Creates a new table of seeded random variates drawn from parametric statistical distributions.",
        group: CommandGroup::Table,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: schema_simulate_distributions,
        enabled: always_enabled,
        run: run_simulate_distributions,
    },
];

// ---------------------------------------------------------------------------
// Enabled Predicates
// ---------------------------------------------------------------------------

const fn always_enabled(_session: &Session) -> bool {
    true
}

fn has_data_sheet(session: &Session) -> bool {
    !session.document().data_sheets.is_empty()
}

fn has_data_or_result_sheet(session: &Session) -> bool {
    let doc = session.document();
    !doc.data_sheets.is_empty() || !doc.result_sheets.is_empty()
}

fn has_nested_sheet(session: &Session) -> bool {
    session.document().data_sheets.iter().any(|ds| matches!(ds.archetype, TableArchetype::Nested(_)))
}

fn has_multivar_sheet(session: &Session) -> bool {
    session.document().data_sheets.iter().any(|ds| matches!(ds.archetype, TableArchetype::MultipleVariables(_)))
}

// ---------------------------------------------------------------------------
// Initial Link Payload Loading & Title Propagation Helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct CoordInput {
    coord: Option<Box<CoordInput>>,
    row: Option<usize>,
    col: Option<usize>,
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

#[derive(Debug, Clone, Deserialize)]
struct FlexibleCoordInput {
    row: Option<usize>,
    col: Option<usize>,
    subcol: Option<usize>,
}

impl FlexibleCoordInput {
    fn resolve(&self) -> CellCoord {
        CellCoord::new(self.row.unwrap_or(0), self.col.unwrap_or(0), self.subcol.unwrap_or(0))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct CellRangeInput {
    start: Option<CoordInput>,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ColRef {
    Index(usize),
    Title(String),
}

#[derive(Debug, Clone, Deserialize)]
struct CellPayloadFields {
    #[serde(default)]
    value: Value,
    decimal_separator: Option<String>,
    parse_asterisk_exclusion: Option<bool>,
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
    if sample_text.contains('\t') {
        return Ok('\t');
    }
    if sample_text.contains(';') {
        return Ok(';');
    }
    Ok(',')
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
    if delimiter == ',' {
        return '.';
    }
    let mut comma_nums = 0usize;
    let mut dot_nums = 0usize;
    for line in sample_text.lines().take(32) {
        for tok in line.split(delimiter) {
            let t = tok.trim().trim_matches('"').trim_matches('*').trim_start_matches(['+', '-']);
            if t.is_empty() {
                continue;
            }
            if t.contains(',') && !t.contains('.') && !looks_like_us_thousands(t) && t.replace(',', ".").parse::<f64>().is_ok() {
                comma_nums = comma_nums.saturating_add(1);
            } else if t.contains('.') && t.parse::<f64>().is_ok() {
                dot_nums = dot_nums.saturating_add(1);
            }
        }
    }
    if delimiter == ';' && dot_nums == 0 {
        return ',';
    }
    if comma_nums > dot_nums { ',' } else { '.' }
}

fn looks_like_us_thousands(token: &str) -> bool {
    let mut parts = token.split(',');
    let Some(first) = parts.next() else {
        return false;
    };
    let rest: Vec<&str> = parts.collect();
    !rest.is_empty()
        && (1..=3).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_digit())
        && rest.iter().all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()))
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
        "meansdn" => Ok(SubcolumnMode::MeanSdN),
        "meansemn" => Ok(SubcolumnMode::MeanSemN),
        "meancvn" => Ok(SubcolumnMode::MeanCvN),
        "meanlowhigh" => Ok(SubcolumnMode::MeanLowHigh),
        "upperlowerlimits" | "meanupperlower" => Ok(SubcolumnMode::UpperLowerLimits),
        "medianiqr" => Ok(SubcolumnMode::MedianIqr),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown subcolumn mode `{s}`"))
            .with_remediation("Use single, replicates, mean_sd_n, mean_sem_n, mean_cv_n, mean_low_high, upper_lower_limits, or median_iqr.")),
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
    struct FilterObj {
        row_start: Option<usize>,
        row_end: Option<usize>,
        col_start: Option<usize>,
        col_end: Option<usize>,
        decimate_every_k: Option<usize>,
        unstack_group_column: Option<usize>,
        parse_asterisk_exclusions: Option<bool>,
    }
    let obj: FilterObj = deserialize_strict(v, "/filter")?;
    let mut filter = ImportFilterSpec::default();
    if obj.row_start.is_some() || obj.row_end.is_some() {
        filter.row_window = Some((obj.row_start.unwrap_or(0), obj.row_end.unwrap_or(usize::MAX)));
    }
    if obj.col_start.is_some() || obj.col_end.is_some() {
        filter.col_window = Some((obj.col_start.unwrap_or(0), obj.col_end.unwrap_or(usize::MAX)));
    }
    if let Some(k) = obj.decimate_every_k {
        filter.decimate_every_k = Some(k.max(1));
    }
    filter.unstack_group_column = obj.unstack_group_column;
    filter.parse_asterisk_exclusions = obj.parse_asterisk_exclusions.unwrap_or(true);
    Ok(filter)
}

#[derive(Debug, Clone, Deserialize)]
struct BaselineInput {
    col: Option<usize>,
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
// Schema Reusable Fragments
// ---------------------------------------------------------------------------

fn coord_object_schema() -> Value {
    let inner_coord = obj_schema(
        vec![
            ("row", int_schema("Zero-based row index", Some(0), None)),
            ("col", int_schema("Zero-based dataset/column index", Some(0), None)),
            ("subcol", int_schema("Zero-based replicate subcolumn index (default 0)", Some(0), None)),
        ],
        &[],
        false,
    );
    obj_schema(
        vec![
            ("coord", inner_coord),
            ("row", int_schema("Zero-based row index", Some(0), None)),
            ("col", int_schema("Zero-based dataset/column index", Some(0), None)),
            ("subcol", int_schema("Zero-based replicate subcolumn index (default 0)", Some(0), None)),
        ],
        &[],
        false,
    )
}

fn cell_range_schema() -> Value {
    let endpoint = coord_object_schema();
    obj_schema(vec![("start", endpoint.clone()), ("end", endpoint)], &["start"], false)
}

fn cell_value_schema() -> Value {
    let dec_schema = explicit_decimal_sep_schema();
    let ast_schema = bool_schema("Interpret trailing `*` in string tokens as excluded (default true)");
    let cell_obj_schema = json!({
        "type": "object",
        "properties": {
            "value": { "description": "Cell scalar value: finite number, text/numeric string, boolean, or null" },
            "excluded": bool_schema("Optional explicit exclusion flag"),
            "decimal_separator": dec_schema,
            "parse_asterisk_exclusion": ast_schema
        },
        "additionalProperties": false
    });
    json!({
        "description": "Cell scalar value: finite number, text/numeric string, boolean, cell object, or null to clear",
        "anyOf": [
            num_schema("Finite numeric cell value", None, None),
            str_schema("String or numeric token (trailing `*` sets excluded when enabled)"),
            bool_schema("Boolean text cell"),
            cell_obj_schema,
            { "type": "null" }
        ]
    })
}

fn anchor_schema() -> Value {
    obj_schema(
        vec![
            ("row", int_schema("Anchor row index", Some(0), None)),
            ("col", int_schema("Anchor column index", Some(0), None)),
            ("subcol", int_schema("Anchor subcolumn index", Some(0), None)),
        ],
        &[],
        false,
    )
}

pub(super) fn import_filter_schema() -> Value {
    obj_schema(
        vec![
            ("row_start", nullable_schema(int_schema("First zero-based source row to include", Some(0), None))),
            ("row_end", nullable_schema(int_schema("Last zero-based source row to include", Some(0), None))),
            ("col_start", nullable_schema(int_schema("First zero-based source column to include", Some(0), None))),
            ("col_end", nullable_schema(int_schema("Last zero-based source column to include", Some(0), None))),
            (
                "row_window",
                nullable_schema(array_schema(
                    "Inclusive zero-based [start_row, end_row] slice",
                    int_schema("Row index", Some(0), None),
                    Some(2),
                    Some(2),
                )),
            ),
            (
                "col_window",
                nullable_schema(array_schema(
                    "Inclusive zero-based [start_col, end_col] slice",
                    int_schema("Column index", Some(0), None),
                    Some(2),
                    Some(2),
                )),
            ),
            ("decimate_every_k", nullable_schema(int_schema("Keep every k-th source row (>= 1)", Some(1), None))),
            ("unstack_group_column", nullable_schema(int_schema("Optional grouping column index for unstacking", Some(0), None))),
            ("parse_asterisk_exclusions", nullable_schema(bool_schema("Interpret `*` suffix tokens as excluded cells"))),
            ("baseline", nullable_schema(json!({ "description": "Optional ImportFilterSpec baseline tuple `[col, operation]`" }))),
            ("exclude_row_predicate", nullable_schema(str_schema("Optional row-level exclusion predicate formula"))),
        ],
        &[],
        false,
    )
}

pub(super) fn baseline_schema() -> Value {
    obj_schema(
        vec![
            ("col", nullable_schema(int_schema("Zero-based reference baseline column index (default 0)", Some(0), None))),
            ("op", nullable_schema(str_schema("Baseline operation: difference, ratio, percent_change, or fractional_change"))),
        ],
        &[],
        false,
    )
}

fn delimiter_schema() -> Value {
    nullable_schema(str_schema("Column delimiter (`auto`, `,`, `\\t`/`tab`, `;`, or single character)"))
}

fn decimal_sep_schema() -> Value {
    nullable_schema(str_schema("Decimal separator (`.`, `,`, `dot`, `comma`, `period`, or `auto`)"))
}

fn explicit_decimal_sep_schema() -> Value {
    nullable_schema(str_schema("Explicit decimal separator (`.`, `,`, `dot`, `comma`, or `period`)"))
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

fn col_ref_schema(desc: &str) -> Value {
    json!({
        "description": desc,
        "anyOf": [
            int_schema("Zero-based variable column index", Some(0), None),
            non_empty_str_schema("Variable column title")
        ]
    })
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

#[derive(Debug, Deserialize)]
struct SetCellParams {
    sheet_id: String,
    #[serde(flatten)]
    coord: CoordInput,
    #[serde(flatten)]
    cell: CellPayloadFields,
}

fn schema_set_cell() -> Value {
    // Retained until Stage 4 updates apps/curvebook-web/tests/sec20_web_fixes.rs
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("coord", coord_object_schema()),
            ("row", int_schema("Zero-based row index", Some(0), None)),
            ("col", int_schema("Zero-based column/dataset index", Some(0), None)),
            ("subcol", int_schema("Zero-based subcolumn index (default 0)", Some(0), None)),
            ("value", cell_value_schema()),
            ("excluded", bool_schema("Optional explicit exclusion flag")),
            ("decimal_separator", explicit_decimal_sep_schema()),
            ("parse_asterisk_exclusion", bool_schema("Interpret trailing `*` in string tokens as excluded (default true)")),
        ],
        &["sheet_id"],
        vec![("value", &["raw"])],
        false,
    )
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

fn run_set_cell(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetCellParams = parse_params(&schema_set_cell(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let coord = p.coord.resolve()?;
    let cell = p.cell.into_cell()?;
    let doc = &mut ctx.session.doc;
    let before_active = snapshot_active_series_lengths(doc, &id);
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;
    ensure_mv_columns_editable(ds, std::iter::once(coord.col))?;
    ensure_coord_outside_linked_regions(ds, coord, "edit")?;
    ds.set_cell(coord, cell)?;
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
    prune_shortened_series_overrides(doc, &id, &before_active);
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

#[derive(Debug, Deserialize)]
struct CellEditItem {
    #[serde(flatten)]
    coord: CoordInput,
    #[serde(flatten)]
    cell: CellPayloadFields,
}

#[derive(Debug, Deserialize)]
struct SetCellsParams {
    sheet_id: String,
    edits: Option<Vec<CellEditItem>>,
    rows: Option<Vec<Vec<Value>>>,
    start_row: Option<usize>,
    start_col: Option<usize>,
    start_subcol: Option<usize>,
}

fn schema_set_cells() -> Value {
    let edit_item = obj_schema(
        vec![
            ("coord", coord_object_schema()),
            ("row", int_schema("Zero-based row index", Some(0), None)),
            ("col", int_schema("Zero-based column index", Some(0), None)),
            ("subcol", int_schema("Zero-based subcolumn index", Some(0), None)),
            ("value", cell_value_schema()),
            ("excluded", bool_schema("Optional explicit exclusion flag")),
            ("decimal_separator", explicit_decimal_sep_schema()),
            ("parse_asterisk_exclusion", bool_schema("Parse trailing `*` exclusion marker")),
        ],
        &[],
        false,
    );
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("edits", array_schema("List of explicit cell edits", edit_item, None, None)),
            ("rows", array_schema("2D array of cell values or objects", array_schema("Row of cells", cell_value_schema(), None, None), None, None)),
            ("start_row", int_schema("Starting row for `rows` grid (default 0)", Some(0), None)),
            ("start_col", int_schema("Starting column for `rows` grid (default 0)", Some(0), None)),
            ("start_subcol", int_schema("Subcolumn index for `rows` grid (default 0)", Some(0), None)),
        ],
        &["sheet_id"],
        false,
    )
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
            let row = start_row.checked_add(r_off).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "row index overflow"))?;
            for (c_off, cell_val) in row_cells.iter().enumerate() {
                let col = start_col.checked_add(c_off).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "column index overflow"))?;
                out.push((CellCoord::new(row, col, start_subcol), parse_cell_from_grid_value(cell_val)?));
            }
        }
        return Ok(out);
    }
    Err(EngineError::with_code(ErrorCode::BadInput, "table.setCells requires `edits`/`cells` or `rows`")
        .with_remediation("Supply either an `edits` array or a 2D `rows` array."))
}

fn run_set_cells(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetCellsParams = parse_params(&schema_set_cells(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let edits = collect_set_cells_edits(p)?;
    let doc = &mut ctx.session.doc;
    let before_active = snapshot_active_series_lengths(doc, &id);
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;
    ensure_mv_columns_editable(ds, edits.iter().map(|(c, _)| c.col))?;
    ensure_coords_outside_linked_regions(ds, edits.iter().map(|(c, _)| *c), "edit")?;
    for (coord, cell) in &edits {
        ds.set_cell(*coord, cell.clone())?;
    }
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
    prune_shortened_series_overrides(doc, &id, &before_active);
    Ok(CommandOutcome::mutated(
        "table.setCells",
        "updated cells",
        json!({
            "sheet_id": id.as_str(),
            "edited_count": edits.len(),
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 3. table.setXCells
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct XCellEditItem {
    row: usize,
    subcol: Option<usize>,
    #[serde(flatten)]
    cell: CellPayloadFields,
}

#[derive(Debug, Deserialize)]
struct SetXCellsParams {
    sheet_id: String,
    edits: Option<Vec<XCellEditItem>>,
    values: Option<Vec<Value>>,
    start_row: Option<usize>,
    row: Option<usize>,
    subcol: Option<usize>,
    #[serde(flatten)]
    cell: CellPayloadFields,
}

fn schema_set_x_cells() -> Value {
    let x_edit = obj_schema(
        vec![
            ("row", int_schema("Zero-based row index", Some(0), None)),
            ("subcol", int_schema("Zero-based X subcolumn index (default 0)", Some(0), None)),
            ("value", cell_value_schema()),
            ("excluded", bool_schema("Optional explicit exclusion flag")),
            ("decimal_separator", explicit_decimal_sep_schema()),
            ("parse_asterisk_exclusion", bool_schema("Parse trailing `*` exclusion marker")),
        ],
        &["row"],
        false,
    );
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target XY or Survival sheet identifier")),
            ("edits", array_schema("List of X/time cell edits", x_edit, None, None)),
            ("values", array_schema("Sequential X/time values starting at `start_row`", cell_value_schema(), None, None)),
            ("start_row", int_schema("Starting row for `values` batch (default 0)", Some(0), None)),
            ("row", int_schema("Single row index when editing one X cell", Some(0), None)),
            ("subcol", int_schema("X subcolumn index (default 0)", Some(0), None)),
            ("value", cell_value_schema()),
            ("excluded", bool_schema("Optional explicit exclusion flag")),
            ("decimal_separator", explicit_decimal_sep_schema()),
            ("parse_asterisk_exclusion", bool_schema("Parse trailing `*` exclusion marker")),
        ],
        &["sheet_id"],
        false,
    )
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
    let p: SetXCellsParams = parse_params(&schema_set_x_cells(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let updates = collect_set_x_updates(p)?;
    let doc = &mut ctx.session.doc;
    let before_active = snapshot_active_series_lengths(doc, &id);
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;
    for (row, subcol, cell) in &updates {
        ds.set_x_or_time_cell(*row, *subcol, cell.clone())?;
    }
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
    prune_shortened_series_overrides(doc, &id, &before_active);
    Ok(CommandOutcome::mutated(
        "table.setXCells",
        "updated X/time cells",
        json!({
            "sheet_id": id.as_str(),
            "edited_count": updates.len(),
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 4. table.setRowTitles
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SetRowTitlesParams {
    sheet_id: String,
    titles: Option<Vec<Option<String>>>,
    start_row: Option<usize>,
    row: Option<usize>,
    title: Option<String>,
}

fn schema_set_row_titles() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("titles", array_schema("Batch of row titles (null clears a title)", nullable_schema(str_schema("Row title")), None, None)),
            ("start_row", int_schema("Starting row for `titles` batch (default 0)", Some(0), None)),
            ("row", int_schema("Single row index when `titles` is omitted", Some(0), None)),
            ("title", nullable_schema(str_schema("Single row title (omit or null to clear)"))),
        ],
        &["sheet_id"],
        false,
    )
}

fn run_set_row_titles(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetRowTitlesParams = parse_params(&schema_set_row_titles(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    if let Some(ref title) = p.title {
        validate_title(title)?;
    }
    let doc = &mut ctx.session.doc;
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;

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
            EngineError::with_code(ErrorCode::BadInput, "table.setRowTitles requires `titles` or `row`")
                .with_remediation("Supply a `titles` array or a single `row` index.")
        })?;
        set_row_title(&mut ds.archetype, row, p.title)?;
        updated = 1;
    }

    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
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

#[derive(Debug, Deserialize)]
struct SetTitlesParams {
    sheet_id: String,
    x_title: Option<String>,
    row_factor_title: Option<String>,
    column_factor_title: Option<String>,
    titles: Option<Vec<String>>,
    start_col: Option<usize>,
    title: Option<String>,
    col: Option<usize>,
    row_titles: Option<Vec<Option<String>>>,
}

fn schema_set_titles() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("x_title", nullable_schema(str_schema("X or Survival time axis title"))),
            ("row_factor_title", nullable_schema(str_schema("Row factor title (Grouped/Contingency)"))),
            ("column_factor_title", nullable_schema(str_schema("Column factor title (Grouped/Contingency)"))),
            ("titles", array_schema("Batch of dataset/column/group titles", str_schema("Column title"), None, None)),
            ("start_col", int_schema("Starting column index for `titles` batch (default 0)", Some(0), None)),
            ("title", nullable_schema(str_schema("Single dataset/column title"))),
            ("col", int_schema("Column index for single `title` update", Some(0), None)),
            ("row_titles", array_schema("Batch of row titles starting at row 0", nullable_schema(str_schema("Row title")), None, None)),
        ],
        &["sheet_id"],
        false,
    )
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
            if old_t.is_empty() || old_titles[..idx].iter().any(|prev| prev.trim() == old_t) {
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
        if rs.analysis.source_sheet_id == *sheet_id
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
    let p: SetTitlesParams = parse_params(&schema_set_titles(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    if p.titles.is_some() && p.title.is_some() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("cannot combine `titles` with single `title` in `{command_id}`"))
            .with_remediation("Supply either `titles` (with optional `start_col`) or single `title` with `col`, not both.")
            .with_details(json!({ "pointer": "/titles" })));
    }
    if p.title.is_some() && p.col.is_none() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "missing required `col` for single `title` update")
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
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!(
                "{command_id} requires at least one title update (`title`/`col`, `titles`, `x_title`, `row_factor_title`, `column_factor_title`, or `row_titles`)"
            ),
        ));
    }
    if let Some(ref title) = p.title {
        validate_title(title)?;
    }
    let doc = &mut ctx.session.doc;
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;
    let old_mv_titles: Option<Vec<String>> = match &ds.archetype {
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
            let col = start_col.checked_add(offset).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "column index overflow"))?;
            set_group_title(&mut ds.archetype, col, title)?;
            updated = updated.saturating_add(1);
        }
    } else if let Some(title) = p.title.as_deref() {
        let col = p.col.ok_or_else(|| {
            EngineError::with_code(ErrorCode::BadInput, "missing required `col` for single `title` update")
                .with_remediation("Provide `col` (or pass a `titles` array).")
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
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("{command_id} applied zero title updates")));
    }

    if let Some(old_titles) = old_mv_titles.as_deref() {
        propagate_mv_variable_renames(doc, &id, old_titles);
    }

    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
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

#[derive(Debug, Deserialize)]
struct SetSubcolumnTitlesParams {
    sheet_id: String,
    col: Option<usize>,
    titles: Option<Vec<String>>,
    start_subcol: Option<usize>,
    subcol: Option<usize>,
    title: Option<String>,
}

fn schema_set_subcolumn_titles() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target Nested data sheet identifier")),
            ("col", nullable_schema(int_schema("Optional group index (omit to update all groups)", Some(0), None))),
            ("titles", array_schema("Batch of subcolumn titles", str_schema("Subcolumn title"), None, None)),
            ("start_subcol", int_schema("Starting subcolumn index (default 0)", Some(0), None)),
            ("subcol", int_schema("Single subcolumn index when `titles` is omitted", Some(0), None)),
            ("title", str_schema("Single subcolumn title")),
        ],
        &["sheet_id"],
        false,
    )
}

fn run_set_subcolumn_titles(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetSubcolumnTitlesParams = parse_params(&schema_set_subcolumn_titles(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    if let Some(ref title) = p.title {
        validate_title(title)?;
    }
    if let Some(ref titles) = p.titles {
        for title in titles {
            validate_title(title)?;
        }
    }
    let doc = &mut ctx.session.doc;
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;

    let TableArchetype::Nested(nested) = &mut ds.archetype else {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("sheet `{id}` does not store explicit subcolumn titles (only Nested tables store subcolumn_titles)"),
        )
        .with_remediation("Select a Nested data table."));
    };

    let mut updated = 0usize;
    if let Some(titles) = p.titles {
        let start_subcol = p.start_subcol.unwrap_or(0);
        for (offset, title) in titles.iter().enumerate() {
            let subcol = start_subcol.checked_add(offset).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "subcolumn index overflow"))?;
            set_nested_subcol_title(nested, p.col, subcol, title)?;
            updated = updated.saturating_add(1);
        }
    } else {
        let subcol = p.subcol.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing required `subcol`"))?;
        let title = p.title.as_deref().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing required `title`"))?;
        set_nested_subcol_title(nested, p.col, subcol, title)?;
        updated = 1;
    }

    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
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

#[derive(Debug, Deserialize)]
struct FlatRangeBounds {
    range: Option<CellRangeInput>,
    row_start: Option<usize>,
    row_end: Option<usize>,
    col_start: Option<usize>,
    col_end: Option<usize>,
    subcol_start: Option<usize>,
    subcol_end: Option<usize>,
    row: Option<usize>,
    col: Option<usize>,
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

#[derive(Debug, Deserialize)]
struct ExcludeCellCoordItem {
    #[serde(flatten)]
    coord: CoordInput,
}

#[derive(Debug, Deserialize)]
struct ExcludeCellsParams {
    sheet_id: String,
    excluded: Option<bool>,
    target: Option<String>,
    #[serde(flatten)]
    bounds: FlatRangeBounds,
    cells: Option<Vec<ExcludeCellCoordItem>>,
    coord: Option<Box<CoordInput>>,
}

fn schema_exclude_cells() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("excluded", bool_schema("Whether to exclude (true) or include (false); default true")),
            ("target", str_schema("Set to `x` to target X/time column cells")),
            ("range", cell_range_schema()),
            ("cells", array_schema("List of cell coordinates", coord_object_schema(), None, None)),
            ("coord", coord_object_schema()),
            ("row", int_schema("Row index", Some(0), None)),
            ("col", int_schema("Column index", Some(0), None)),
            ("subcol", int_schema("Subcolumn index", Some(0), None)),
            ("row_start", int_schema("Range start row", Some(0), None)),
            ("row_end", int_schema("Range end row", Some(0), None)),
            ("col_start", int_schema("Range start column", Some(0), None)),
            ("col_end", int_schema("Range end column", Some(0), None)),
            ("subcol_start", int_schema("Range start subcolumn", Some(0), None)),
            ("subcol_end", int_schema("Range end subcolumn", Some(0), None)),
        ],
        &["sheet_id"],
        false,
    )
}

fn run_exclude_cells(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ExcludeCellsParams = parse_params(&schema_exclude_cells(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let excluded = p.excluded.unwrap_or(true);
    let target_x = p.target.as_deref().is_some_and(|t| t.eq_ignore_ascii_case("x"));
    let range_opt = p.bounds.resolve_range()?;

    let doc = &mut ctx.session.doc;
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;

    let mut updated = 0usize;
    let action = if excluded { "exclude" } else { "unexclude" };
    if let Some(range) = range_opt {
        validate_cell_range_bounds(ds, range, target_x)?;
        if !target_x {
            for r in range.start.row..=range.end.row {
                for c in range.start.col..=range.end.col {
                    for s in range.start.subcol..=range.end.subcol {
                        ensure_coord_outside_linked_regions(ds, CellCoord::new(r, c, s), action)?;
                    }
                }
            }
        }
        for r in range.start.row..=range.end.row {
            for c in range.start.col..=range.end.col {
                for s in range.start.subcol..=range.end.subcol {
                    if target_x {
                        ds.set_x_or_time_excluded(r, s, excluded)?;
                    } else {
                        ds.set_excluded(CellCoord::new(r, c, s), excluded)?;
                    }
                    updated = updated.saturating_add(1);
                }
            }
        }
    } else if let Some(coords) = p.cells {
        if target_x {
            let mut x_coords = Vec::with_capacity(coords.len());
            for item in coords {
                let row = item.coord.row.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing required X cell `row`"))?;
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
        let row = p.bounds.row.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing required X cell `row`"))?;
        ds.set_x_or_time_excluded(row, p.bounds.subcol.unwrap_or(0), excluded)?;
        updated = 1;
    } else {
        let coord = CoordInput { coord: p.coord, row: p.bounds.row, col: p.bounds.col, subcol: p.bounds.subcol }.resolve()?;
        ensure_coord_outside_linked_regions(ds, coord, action)?;
        ds.set_excluded(coord, excluded)?;
        updated = 1;
    }

    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
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

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum TupleOrObjCoord {
    Tuple(Vec<usize>),
    Obj(CoordInput),
}

impl TupleOrObjCoord {
    fn resolve(&self) -> Result<CellCoord, EngineError> {
        match self {
            Self::Tuple(arr) => {
                if arr.len() != 2 && arr.len() != 3 {
                    return Err(EngineError::with_code(ErrorCode::BadInput, "coordinate tuple must be `[row, col]` or `[row, col, subcol]`"));
                }
                Ok(CellCoord::new(arr[0], arr[1], arr.get(2).copied().unwrap_or(0)))
            }
            Self::Obj(obj) => obj.resolve(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ExcludeCellsWithReasonParams {
    sheet_id: String,
    reason_tag: String,
    note: Option<String>,
    timestamp_utc: Option<String>,
    coords: Option<Vec<TupleOrObjCoord>>,
    range: Option<CellRangeInput>,
    #[serde(flatten)]
    coord: CoordInput,
}

fn schema_exclude_cells_with_reason() -> Value {
    let coord_any = json!({
        "description": "Cell coordinate `{ row, col, subcol? }` or `[row, col, subcol?]`",
        "anyOf": [
            array_schema("Coordinate tuple [row, col] or [row, col, subcol]", int_schema("Non-negative coordinate index", Some(0), None), Some(2), Some(3)),
            coord_object_schema()
        ]
    });
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("reason_tag", non_empty_str_schema("Short non-empty audit reason tag")),
            ("note", nullable_schema(str_schema("Optional explanatory audit note"))),
            ("timestamp_utc", nullable_schema(str_schema("Optional explicit UTC timestamp string"))),
            ("coords", array_schema("List of coordinates to exclude", coord_any, Some(1), None)),
            ("range", cell_range_schema()),
            ("coord", coord_object_schema()),
            ("row", int_schema("Single cell row index", Some(0), None)),
            ("col", int_schema("Single cell column index", Some(0), None)),
            ("subcol", int_schema("Single cell subcolumn index", Some(0), None)),
        ],
        &["sheet_id", "reason_tag"],
        false,
    )
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
    let p: ExcludeCellsWithReasonParams = parse_params(&schema_exclude_cells_with_reason(), raw)?;
    let sheet_id = parse_sheet_id(&p.sheet_id)?;
    if p.reason_tag.trim().is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.excludeCellsWithReason `reason_tag` cannot be empty")
            .with_remediation("Supply a non-empty `reason_tag` (e.g. `outlier` or `pipetting_error`)."));
    }
    let timestamp_utc = match &p.timestamp_utc {
        Some(ts) if ts.trim().is_empty() => {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`timestamp_utc` cannot be empty"));
        }
        Some(ts) => ts.clone(),
        None => current_utc_timestamp(),
    };
    let note = p.note.clone().unwrap_or_default();

    let doc = &mut ctx.session.doc;
    let ds = require_unfrozen_data_sheet_mut(doc, &sheet_id)?;
    let coords = resolve_exclusion_targets(&p, &ds.archetype)?;
    for coord in &coords {
        if ds.archetype.data_cell(*coord).is_none() {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("cell ({}, {}, {}) is out of bounds for `{}`", coord.row, coord.col, coord.subcol, sheet_id.as_str()),
            ));
        }
    }
    ensure_coords_outside_linked_regions(ds, coords.iter().copied(), "exclude")?;
    for &coord in &coords {
        ds.set_excluded_with_reason(coord, &p.reason_tag, &note, &timestamp_utc)?;
    }
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&sheet_id), false)?;
    Ok(CommandOutcome::mutated(
        "table.excludeCellsWithReason",
        format!("excluded {} cell(s) with reason `{}`", coords.len(), p.reason_tag),
        json!({
            "sheet_id": sheet_id.as_str(),
            "count": coords.len(),
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

#[derive(Debug, Deserialize)]
struct ExclusionAuditReportParams {
    sheet_id: Option<String>,
}

fn schema_exclusion_audit_report() -> Value {
    obj_schema(vec![("sheet_id", nullable_schema(non_empty_str_schema("Optional sheet ID to filter report")))], &[], false)
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
    let p: ExclusionAuditReportParams = parse_params(&schema_exclusion_audit_report(), raw)?;
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

#[derive(Debug, Deserialize)]
struct ClearCellsParams {
    sheet_id: String,
    all: Option<bool>,
    clear_x: Option<bool>,
    unlink: Option<bool>,
    region_id: Option<String>,
    #[serde(flatten)]
    bounds: FlatRangeBounds,
    cells: Option<Vec<CoordInput>>,
    coord: Option<Box<CoordInput>>,
}

fn schema_clear_cells() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("all", bool_schema("Clear all data cells across the sheet")),
            ("clear_x", bool_schema("When `all` is true, also clear X/time cells")),
            ("unlink", bool_schema("Detach live linked regions on this sheet (all regions unless `region_id` is supplied)")),
            ("region_id", non_empty_str_schema("When `unlink` is true, detach only the linked region matching this identifier")),
            ("range", cell_range_schema()),
            ("cells", array_schema("List of cell coordinates to clear", coord_object_schema(), None, None)),
            ("coord", coord_object_schema()),
            ("row", int_schema("Single cell row index", Some(0), None)),
            ("col", int_schema("Single cell column index", Some(0), None)),
            ("subcol", int_schema("Single cell subcolumn index", Some(0), None)),
            ("row_start", int_schema("Range start row", Some(0), None)),
            ("row_end", int_schema("Range end row", Some(0), None)),
            ("col_start", int_schema("Range start column", Some(0), None)),
            ("col_end", int_schema("Range end column", Some(0), None)),
            ("subcol_start", int_schema("Range start subcolumn", Some(0), None)),
            ("subcol_end", int_schema("Range end subcolumn", Some(0), None)),
        ],
        &["sheet_id"],
        false,
    )
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
    let p: ClearCellsParams = parse_params(&schema_clear_cells(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let range_opt = p.bounds.resolve_range()?;

    let doc = &mut ctx.session.doc;
    let before_active = snapshot_active_series_lengths(doc, &id);
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;

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
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
    prune_shortened_series_overrides(doc, &id, &before_active);
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

#[derive(Debug, Deserialize)]
struct PlacementFields {
    anchor: Option<FlexibleCoordInput>,
    start_row: Option<usize>,
    start_col: Option<usize>,
    start_subcol: Option<usize>,
    mode: Option<String>,
    propagate_title_header: Option<bool>,
    skip_contiguous_blanks: Option<bool>,
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

#[derive(Debug, Deserialize)]
struct PasteParams {
    sheet_id: String,
    text: Option<String>,
    rows: Option<Vec<Vec<Value>>>,
    delimiter: Option<String>,
    decimal_separator: Option<String>,
    normalize_decimal: Option<bool>,
    filter: Option<Value>,
    #[serde(flatten)]
    placement: PlacementFields,
}

fn schema_paste() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("text", str_schema("Delimited clipboard text (TSV/CSV)")),
            ("rows", array_schema("2D array of pasted cell values", array_schema("Row of cell values", cell_value_schema(), None, None), None, None)),
            ("delimiter", delimiter_schema()),
            ("decimal_separator", decimal_sep_schema()),
            ("normalize_decimal", bool_schema("Infer European comma decimal separator when ambiguous")),
            ("filter", nullable_schema(import_filter_schema())),
            ("anchor", anchor_schema()),
            ("start_row", int_schema("Anchor row index (default 0)", Some(0), None)),
            ("start_col", int_schema("Anchor column index (default 0)", Some(0), None)),
            ("start_subcol", int_schema("Anchor subcolumn index (default 0)", Some(0), None)),
            ("mode", str_schema("Placement mode: overwrite, insert_rows, append_rows, append_columns")),
            ("propagate_title_header", bool_schema("Promote first pasted row into column headers")),
            ("skip_contiguous_blanks", bool_schema("Skip empty runs when placing cells")),
            ("transpose", bool_schema("Transpose pasted rows and columns")),
        ],
        &["sheet_id"],
        false,
    )
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
        return Ok(report);
    }
    let mut staged = ds.clone();
    let report = apply_import_link_pipeline(&mut staged, spec, payload, opts, None)?;
    staged.prune_stale_exclusion_notes();
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
    let p: PasteParams = parse_params(&schema_paste(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let filter = parse_import_filter_val(p.filter.as_ref())?;
    let placement = p.placement.to_placement_spec(false)?;
    let spec = ImportLinkSpec { source: LinkSource::Clipboard, filter, placement };

    let doc = &mut ctx.session.doc;
    let before_active = snapshot_active_series_lengths(doc, &id);
    let existing_rows_before = doc.data_sheet(&id).map_or(0, |d| d.archetype.row_count());
    let other_project_cells = doc.total_allocated_cells()?.saturating_sub(doc.data_sheet(&id).map_or(0, |d| d.archetype.allocated_cell_slots()));
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;
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
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.paste requires `text`/`tsv`/`csv` or `rows`/`grid`")
            .with_remediation("Supply delimited `text` or a 2D `rows` array."));
    };

    ds.linked_regions.truncate(existing_links);
    let new_cols = ds.archetype.column_count();
    let appended_ds = (spec.placement.mode == PlacementMode::AppendColumns && new_cols > old_cols).then(|| ds.clone());
    if let Some(updated_ds) = appended_ds {
        let default_spec = GraphSpec::default_for_data_sheet(&updated_ds);
        for g in &mut doc.graph_sheets {
            if g.spec.primary_source_sheet_id == id
                && matches!(g.spec.kind, GraphKind::Xy | GraphKind::Column | GraphKind::Grouped | GraphKind::Survival)
                && g.spec.series.len() == old_cols
                && (0..old_cols).all(|i| g.spec.series[i].data_set_index == i)
                && let Some(new_series) = default_spec.series.get(old_cols..new_cols)
            {
                g.spec.series.extend(new_series.iter().cloned());
            }
        }
    }
    if spec.placement.mode == PlacementMode::InsertRows && spec.placement.anchor.row < existing_rows_before && report.rows_placed > 0 {
        shift_point_overrides_on_insert_rows(doc, &id, spec.placement.anchor.row, report.rows_placed);
    }
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
    prune_shortened_series_overrides(doc, &id, &before_active);
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

#[derive(Debug, Deserialize)]
struct ExportCsvParams {
    sheet_id: String,
    path: Option<String>,
    delimiter: Option<String>,
    decimal_separator: Option<String>,
}

fn schema_export_csv() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Data or Result sheet identifier to export")),
            ("path", nullable_schema(non_empty_str_schema("Optional output file path"))),
            ("delimiter", nullable_schema(str_schema("Delimiter character (`,`, `\\t`/`tab`, `;`, or 1 char)"))),
            ("decimal_separator", explicit_decimal_sep_schema()),
        ],
        &["sheet_id"],
        false,
    )
}

fn run_export_csv(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ExportCsvParams = parse_params(&schema_export_csv(), raw)?;
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
                Err(EngineError::with_code(ErrorCode::BadInput, "missing required `column`/`col` variable index or title")
                    .with_remediation("Identify the variable by zero-based index or title."))
            }
        }
    }
}

fn require_mv_table_mut<'a>(doc: &'a mut ProjectDocument, id: &SheetId) -> Result<&'a mut MultipleVariablesTable, EngineError> {
    let ds = require_unfrozen_data_sheet_mut(doc, id)?;
    match &mut ds.archetype {
        TableArchetype::MultipleVariables(mv) => Ok(mv),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("sheet `{id}` is not a MultipleVariables table"))
            .with_remediation("Select a MultipleVariables data sheet.")),
    }
}

fn parse_variable_role(raw: &str) -> Result<VariableRole, EngineError> {
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "continuous" | "numeric" => Ok(VariableRole::Continuous),
        "categorical" | "category" | "factor" => Ok(VariableRole::Categorical),
        "label" | "identifier" | "id" | "text" => Ok(VariableRole::Label),
        "calculated" | "formula" => Ok(VariableRole::Calculated),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown variable role `{raw}`"))
            .with_remediation("Use continuous, categorical, label, calculated, or auto.")),
    }
}

#[derive(Debug, Deserialize)]
struct SetVariableRoleParams {
    sheet_id: String,
    col: Option<ColRef>,
    title: Option<String>,
    role: String,
}

fn schema_set_variable_role() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target MultipleVariables sheet identifier")),
            ("col", col_ref_schema("Zero-based variable index or column title")),
            ("title", nullable_schema(str_schema("Fallback variable column title"))),
            ("role", non_empty_str_schema("Role: continuous, categorical, label, calculated, or auto")),
        ],
        &["sheet_id", "role"],
        false,
    )
}

fn run_set_variable_role(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetVariableRoleParams = parse_params(&schema_set_variable_role(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let doc = &mut ctx.session.doc;
    let col_idx = {
        let ds = require_data_sheet(doc, &id)?;
        ensure_sheet_not_frozen(ds)?;
        let TableArchetype::MultipleVariables(ref mv) = ds.archetype else {
            return Err(EngineError::with_code(
                ErrorCode::Conflict,
                format!("sheet `{}` is `{}` rather than MultipleVariables", id.as_str(), ds.kind().label()),
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
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                "cannot assign role `Calculated` directly on an entered variable; use `table.setCalculatedVariable`",
            )
            .with_remediation(
                "Set a calculated formula via `table.setCalculatedVariable`, or choose `continuous`, `categorical`, `label`, or `auto`.",
            ));
        }
        Some(parsed_role)
    };
    let mv = require_mv_table_mut(doc, &id)?;
    let var = mv
        .variables
        .get_mut(col_idx)
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("variable column {col_idx} out of range")))?;

    if let Some(role) = requested_role {
        var.lock_role(role);
    } else {
        var.role_inferred = true;
        var.refresh_inferred_role();
    }
    let final_role = var.role;
    let role_inferred = var.role_inferred;

    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
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

#[derive(Debug, Deserialize)]
struct RecodeEntryInput {
    from: String,
    to: String,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
struct RecodeLevelsParams {
    sheet_id: String,
    col: Option<ColRef>,
    title: Option<String>,
    mappings: RecodeMappingsInput,
}

fn schema_recode_levels() -> Value {
    let pair_item =
        obj_schema(vec![("from", str_schema("Existing level text")), ("to", str_schema("Replacement level text"))], &["from", "to"], false);
    let mappings_schema = json!({
        "description": "Recode mapping: object `{ old: new }` or array of `{ from, to }`",
        "anyOf": [
            array_schema("Array of {from, to} recode rules", pair_item, None, None),
            { "type": "object", "additionalProperties": { "type": "string" } }
        ]
    });
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target MultipleVariables sheet identifier")),
            ("col", col_ref_schema("Zero-based variable index or column title")),
            ("title", nullable_schema(str_schema("Fallback variable column title"))),
            ("mappings", mappings_schema),
        ],
        &["sheet_id", "mappings"],
        false,
    )
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
    let p: RecodeLevelsParams = parse_params(&schema_recode_levels(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let pairs = p.mappings.into_pairs();
    let doc = &mut ctx.session.doc;
    if let Some(ds) = doc.data_sheet(&id)
        && let TableArchetype::MultipleVariables(ref mv) = ds.archetype
        && let Ok(col_idx) = resolve_mv_variable_index(mv, p.col.as_ref(), p.title.as_deref())
    {
        ensure_sheet_not_frozen(ds)?;
        ensure_mv_columns_editable(ds, std::iter::once(col_idx))?;
        let rows = ds.archetype.row_count();
        if rows > 0 {
            let col_range = curvebook_doc::SheetCellRange::new(CellCoord::new(0, col_idx, 0), CellCoord::new(rows - 1, col_idx, 0));
            ensure_range_outside_linked_regions(ds, col_range, "recode")?;
        }
    }
    let mv = require_mv_table_mut(doc, &id)?;
    let col_idx = resolve_mv_variable_index(mv, p.col.as_ref(), p.title.as_deref())?;
    let var = mv
        .variables
        .get_mut(col_idx)
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("variable column {col_idx} out of range")))?;
    let recoded_count = var.recode_levels(&pairs);
    for gs in &mut doc.graph_sheets {
        gs.spec.recode_variable_levels(&id, col_idx, &pairs);
    }
    let updated_reference_levels = recode_analysis_reference_levels(&mut doc.result_sheets, &id, col_idx, &pairs);
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
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
        if rs.analysis.source_sheet_id != *source_sheet_id {
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

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OrderParamValue {
    Explicit(Vec<String>),
    Kind(String),
}

#[derive(Debug, Deserialize)]
struct SetCategoricalOrderParams {
    sheet_id: String,
    col: Option<ColRef>,
    title: Option<String>,
    order_spec: Option<Value>,
    levels: Option<Vec<String>>,
    order: Option<OrderParamValue>,
    reversed: Option<bool>,
    reference_level: Option<String>,
}

fn schema_set_categorical_order() -> Value {
    // Retained until Stage 3 updates egui_shell/mod.rs
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target MultipleVariables sheet identifier")),
            ("col", col_ref_schema("Zero-based variable index or column title")),
            ("title", nullable_schema(str_schema("Fallback variable column title"))),
            ("order_spec", nullable_schema(json!({ "type": "object", "description": "Serialized LevelOrderSpec" }))),
            ("levels", array_schema("Explicit category order list", str_schema("Category level"), None, None)),
            (
                "order",
                json!({
                    "description": "Explicit array of levels or order mode (`entry_order`, `alphabetical`, `frequency_descending`, `frequency_ascending`, `explicit`)",
                    "anyOf": [
                        array_schema("Explicit category order list", str_schema("Category level"), None, None),
                        str_schema("Ordering kind name")
                    ]
                }),
            ),
            ("reversed", bool_schema("Reverse resulting level ordering")),
            ("reference_level", nullable_schema(str_schema("Optional reference category label"))),
        ],
        &["sheet_id"],
        vec![("col", &["column"])],
        false,
    )
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
        let nested: NestedOrderSpec = serde_json::from_value(spec_val.clone())
            .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid `order_spec`: {err}")))?;
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
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.setCategoricalOrder requires `order`, `levels`, or `order_spec`")
            .with_remediation("Provide an explicit `order` array or an order mode string."));
    };
    let norm: String = kind_str.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let kind = match norm.as_str() {
        "entryorder" | "entry" => LevelOrderKind::EntryOrder,
        "alphabetical" | "alpha" => LevelOrderKind::Alphabetical,
        "frequencydescending" | "freqdesc" => LevelOrderKind::FrequencyDescending,
        "frequencyascending" | "freqasc" => LevelOrderKind::FrequencyAscending,
        "explicit" => {
            return Err(EngineError::with_code(ErrorCode::BadInput, "explicit categorical order requires `levels` array"));
        }
        _ => {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown level order `{kind_str}`")));
        }
    };
    Ok(LevelOrderSpec { order: kind, reversed })
}

fn run_set_categorical_order(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetCategoricalOrderParams = parse_params(&schema_set_categorical_order(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let ref_level = match p.reference_level.as_deref() {
        Some(raw_ref) => {
            let trimmed = raw_ref.trim();
            if trimmed.is_empty() {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`reference_level` cannot be empty"));
            }
            Some(trimmed.to_owned())
        }
        None => None,
    };
    let explicit_spec = if p.order_spec.is_some() || p.levels.is_some() || p.order.is_some() {
        Some(parse_level_order_from_params(&p)?)
    } else {
        if ref_level.is_none() {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                "table.setCategoricalOrder requires `order`, `levels`, `order_spec`, or `reference_level`",
            )
            .with_remediation("Provide an explicit `order`, `levels`, `order_spec`, or `reference_level`."));
        }
        None
    };

    let doc = &mut ctx.session.doc;
    let mv = require_mv_table_mut(doc, &id)?;
    let col_idx = resolve_mv_variable_index(mv, p.col.as_ref(), p.title.as_deref())?;
    let var = mv
        .variables
        .get_mut(col_idx)
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("variable column {col_idx} out of range")))?;

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
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
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

#[derive(Debug, Deserialize)]
struct SetCalculatedVariableParams {
    sheet_id: String,
    col: Option<ColRef>,
    title: Option<String>,
    formula: String,
}

fn schema_set_calculated_variable() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target MultipleVariables sheet identifier")),
            ("col", col_ref_schema("Existing variable index or name (optional when `title` creates or matches a column)")),
            ("title", nullable_schema(str_schema("Variable column title (creates a new column if not found and `col` is omitted)"))),
            ("formula", str_schema("Formula expression, e.g. `= log10([Dose])` or `log10(ConcRaw)`")),
        ],
        &["sheet_id", "formula"],
        false,
    )
}

fn run_set_calculated_variable(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetCalculatedVariableParams = parse_params(&schema_set_calculated_variable(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    if let Some(ref title) = p.title {
        validate_title(title)?;
    }
    let doc = &mut ctx.session.doc;

    let other_cells = doc.total_allocated_cells()?.saturating_sub(doc.data_sheet(&id).map_or(0, |d| d.archetype.allocated_cell_slots()));
    let info_snapshot = doc.data_sheet(&id).and_then(|ds| ds.info_sheet_id.as_ref()).and_then(|iid| doc.info_sheet(iid)).cloned();
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;
    let (resolved_col_idx, created_new, row_count, cur_slots) = {
        let cur_slots = ds.archetype.allocated_cell_slots();
        let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
            return Err(EngineError::with_code(ErrorCode::BadInput, format!("sheet `{id}` is not a MultipleVariables table"))
                .with_remediation("Select a MultipleVariables data sheet."));
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
            return Err(EngineError::with_code(ErrorCode::BadInput, "table.setCalculatedVariable requires `column`/`col` or `title`")
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
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("sheet `{id}` is not a MultipleVariables table")));
    };
    let col_idx = if created_new {
        let title = p.title.as_deref().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing `title` for new calculated variable"))?;
        let col = VariableColumn::new(title, row_count)?;
        mv.variables.push(col);
        mv.variables.len().saturating_sub(1)
    } else {
        resolved_col_idx
    };

    if col_idx >= mv.variables.len() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("variable column {col_idx} out of range")));
    }
    let prev_title = p.title.as_ref().map(|title| {
        let old = mv.variables[col_idx].title.clone();
        mv.variables[col_idx].title = title.clone();
        old
    });
    if let Err(err) = validate_no_formula_cycle(mv, col_idx, &p.formula) {
        if created_new {
            mv.variables.pop();
        } else if let Some(old) = prev_title {
            mv.variables[col_idx].title = old;
        }
        return Err(err.into());
    }
    mv.variables[col_idx].set_calculated_formula(&p.formula);

    evaluate_data_sheet_calculated_variables(ds, info_snapshot.as_ref())?;
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
    Ok(CommandOutcome::mutated(
        "table.setCalculatedVariable",
        "updated calculated variable",
        json!({
            "sheet_id": id.as_str(),
            "column": col_idx,
            "formula": p.formula,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 22. table.simulateXy
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SimParamObj {
    name: String,
    value: f64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SimParamArrayItem {
    Scalar(f64),
    Pair(String, f64),
    Obj(SimParamObj),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SimParametersInput {
    Map(BTreeMap<String, f64>),
    List(Vec<SimParamArrayItem>),
}

#[derive(Debug, Deserialize)]
struct SimulateXyParams {
    sheet_id: Option<String>,
    title: Option<String>,
    seed: Option<u64>,
    row_count: Option<usize>,
    replicates: Option<usize>,
    x_min: Option<f64>,
    x_max: Option<f64>,
    equation_id: Option<String>,
    parameters: Option<SimParametersInput>,
    noise_kind: Option<String>,
    noise_sd: Option<f64>,
    cv_percent: Option<f64>,
    auto_info: Option<bool>,
    graph_sheet_id: Option<String>,
}

fn schema_simulate_xy() -> Value {
    let params_schema = json!({
        "description": "Simulation equation parameters as `{ Name: value }` map or array",
        "anyOf": [
            { "type": "object", "additionalProperties": { "type": "number" } },
            { "type": "array" }
        ]
    });
    obj_schema(
        vec![
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional destination data sheet ID"))),
            ("title", nullable_schema(str_schema("Simulated sheet title (default `Simulated XY`)"))),
            ("seed", int_schema("PRNG seed (default 1)", Some(0), None)),
            ("row_count", int_schema("Number of simulated X rows (default 12)", Some(0), None)),
            ("replicates", int_schema("Number of replicate Y subcolumns (default 3)", Some(0), None)),
            ("x_min", num_schema("Minimum X value (default 0.0)", None, None)),
            ("x_max", num_schema("Maximum X value (default 10.0)", None, None)),
            ("equation_id", str_schema("Model equation ID (default `hill_4pl`)")),
            ("parameters", nullable_schema(params_schema)),
            ("noise_kind", str_schema("Noise model: none, gaussian, relative_percent/cv, or poisson (default gaussian)")),
            ("noise_sd", num_schema("Gaussian SD noise level (default 0.0)", None, None)),
            ("cv_percent", num_schema("Relative CV% noise level (default 0.0)", None, None)),
            ("auto_info", bool_schema("Auto-create linked InfoSheet for new simulated table")),
            ("graph_sheet_id", nullable_schema(non_empty_str_schema("Optional GraphSheet ID to create for new simulated table"))),
        ],
        &[],
        false,
    )
}

fn resolve_simulation_parameters(raw: Option<SimParametersInput>) -> Result<Vec<(String, f64)>, EngineError> {
    let Some(input) = raw else {
        return Ok(Vec::new());
    };
    let default_names = ["Bottom", "Top", "LogEC50", "HillSlope", "S", "K", "Span", "Rate", "Slope", "Intercept"];
    let pairs: Vec<(String, f64)> = match input {
        SimParametersInput::Map(map) => map.into_iter().collect(),
        SimParametersInput::List(items) => items
            .into_iter()
            .enumerate()
            .map(|(idx, item)| match item {
                SimParamArrayItem::Scalar(num) => (default_names.get(idx).copied().unwrap_or("Param").to_owned(), num),
                SimParamArrayItem::Pair(name, num) => (name, num),
                SimParamArrayItem::Obj(o) => (o.name, o.value),
            })
            .collect(),
    };
    for (_, val) in &pairs {
        if !val.is_finite() {
            return Err(EngineError::with_code(ErrorCode::BadInput, "simulation parameter value must be finite"));
        }
    }
    Ok(pairs)
}

fn resolve_simulation_noise(noise_kind: Option<&str>, noise_sd: Option<f64>, cv_percent: Option<f64>) -> Result<SimulationNoise, EngineError> {
    let kind = noise_kind.unwrap_or("gaussian");
    let norm: String = kind.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    match norm.as_str() {
        "none" | "zero" => Ok(SimulationNoise::None),
        "gaussian" | "normal" => Ok(SimulationNoise::Gaussian { sd: noise_sd.unwrap_or(0.0) }),
        "relative" | "relativepercent" | "cv" => Ok(SimulationNoise::RelativePercent { cv_percent: cv_percent.or(noise_sd).unwrap_or(0.0) }),
        "poisson" => Ok(SimulationNoise::Poisson),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown simulation noise model `{kind}`"))
            .with_remediation("Use none, gaussian, relative_percent/cv, or poisson.")),
    }
}

fn run_simulate_xy(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SimulateXyParams = parse_params(&schema_simulate_xy(), raw)?;
    let sim_params = resolve_simulation_parameters(p.parameters)?;
    let noise = resolve_simulation_noise(p.noise_kind.as_deref(), p.noise_sd, p.cv_percent)?;

    let title = p.title.as_deref().unwrap_or("Simulated XY");
    let seed = p.seed.unwrap_or(1);
    let row_count = p.row_count.unwrap_or(12);
    let replicates = p.replicates.unwrap_or(3);
    let x_min = p.x_min.unwrap_or(0.0);
    let x_max = p.x_max.unwrap_or(10.0);
    let equation_id = p.equation_id.as_deref().unwrap_or("hill_4pl");

    let doc = &mut ctx.session.doc;
    let id = match p.sheet_id.as_deref() {
        Some(raw_id) => parse_sheet_id(raw_id)?,
        None => next_unique_sheet_id(doc, "data")?,
    };

    let mut simulated = simulate_xy_sheet(id.as_str(), title, seed, row_count, replicates, x_min, x_max, equation_id, &sim_params, noise)?;

    if doc.data_sheet(&id).is_some() {
        validate_project_cell_budget_after_change(doc, Some(&id), simulated.archetype.allocated_cell_slots())?;
        let existing_info = doc.data_sheet(&id).and_then(|d| d.info_sheet_id.clone());
        let slot = require_unfrozen_data_sheet_mut(doc, &id)?;
        if let Some(info_id) = existing_info {
            simulated.info_sheet_id = Some(info_id);
        }
        *slot = simulated;
    } else {
        validate_project_cell_budget_after_change(doc, None, simulated.archetype.allocated_cell_slots())?;
        doc.add_data_sheet(simulated)?;
        if p.auto_info.unwrap_or(false) {
            let iid = next_unique_sheet_id(doc, "info")?;
            if let Some(ds) = doc.data_sheet_mut(&id) {
                ds.info_sheet_id = Some(iid.clone());
            }
            doc.add_info_sheet(InfoSheet::new_preseeded(iid.clone(), format!("{title} Info"))?)?;
            link_data_sheet_to_info(doc, &iid, &id);
        }
        if let Some(graph_id) = p.graph_sheet_id.as_deref()
            && let Some(ds) = doc.data_sheet(&id).cloned()
        {
            doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data(parse_sheet_id(graph_id)?, &ds)?)?;
        }
        doc.select_in_navigator(&id)?;
    }

    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
    Ok(CommandOutcome::mutated(
        "table.simulateXy",
        "simulated XY data sheet",
        json!({
            "sheet_id": id.as_str(),
            "equation_id": equation_id,
            "row_count": row_count,
            "replicates": replicates,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 23. table.convertArchetype
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ConvertArchetypeParams {
    sheet_id: String,
    target_kind: String,
    allow_lossy: Option<bool>,
    preview_only: Option<bool>,
}

fn schema_convert_archetype() -> Value {
    obj_schema(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            (
                "target_kind",
                enum_str_schema(
                    "Target table archetype",
                    &[
                        "Column",
                        "column",
                        "columnar",
                        "Grouped",
                        "grouped",
                        "Xy",
                        "XY",
                        "xy",
                        "Contingency",
                        "contingency",
                        "MultipleVariables",
                        "multiple_variables",
                        "multiple-variables",
                        "multivariate",
                    ],
                ),
            ),
            ("allow_lossy", bool_schema("Permit conversion even if subcolumns or incompatible columns are reshaped/dropped (default false)")),
            ("preview_only", bool_schema("Return conversion preview without mutating document (default false)")),
        ],
        &["sheet_id", "target_kind"],
        false,
    )
}

fn run_convert_archetype(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ConvertArchetypeParams = parse_params(&schema_convert_archetype(), raw)?;
    let sheet_id = parse_sheet_id(&p.sheet_id)?;
    let target_kind = parse_target_archetype_kind(&p.target_kind)?;
    let allow_lossy = p.allow_lossy.unwrap_or(false);
    let preview_only = p.preview_only.unwrap_or(false);

    let doc = &mut ctx.session.doc;
    let idx = doc
        .data_sheets
        .iter()
        .position(|ds| ds.id == sheet_id)
        .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("data sheet `{}` not found", sheet_id.as_str())))?;

    if preview_only {
        let base_preview = preview_datasheet_conversion(&doc.data_sheets[idx], target_kind)?;
        let impact = plan_conversion_impacts(doc, &doc.data_sheets[idx], target_kind);
        let preview_val = build_enriched_conversion_preview(&base_preview, &impact);
        return Ok(CommandOutcome::read_only(
            "table.convertArchetype",
            format!("previewed conversion of `{}`", sheet_id.as_str()),
            json!({
                "sheet_id": sheet_id.as_str(),
                "preview_only": true,
                "preview": preview_val,
            }),
        ));
    }

    ensure_sheet_not_frozen(&doc.data_sheets[idx])?;
    let old_ds = doc.data_sheets[idx].clone();
    let impact = plan_conversion_impacts(doc, &old_ds, target_kind);
    let (mut converted, base_preview) = convert_datasheet_archetype(&old_ds, target_kind, allow_lossy)?;
    if impact.incoming_links_lost && !allow_lossy {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!(
                "lossy conversion of `{}` drops {} live incoming link(s); pass `allow_lossy: true` to detach them",
                sheet_id.as_str(),
                impact.incoming_links_dropped
            ),
        ));
    }
    converted.id = sheet_id.clone();
    validate_project_cell_budget_after_change(doc, Some(&sheet_id), converted.archetype.allocated_cell_slots())?;
    preserve_mappable_incoming_links(&old_ds, &mut converted, &impact.col_plan);
    converted.prune_stale_exclusion_notes();
    doc.data_sheets[idx] = converted;
    apply_conversion_dependents(doc, &sheet_id, &impact);
    let mut recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&sheet_id), false)?;
    finalize_broken_dependents(doc, &impact, &mut recalc);
    let preview_val = build_enriched_conversion_preview(&base_preview, &impact);
    Ok(CommandOutcome::mutated(
        "table.convertArchetype",
        format!("converted `{}` to {:?}", sheet_id.as_str(), target_kind),
        json!({
            "sheet_id": sheet_id.as_str(),
            "preview_only": false,
            "preview": preview_val,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
            "failed_count": recalc.failed_result_sheets.len(),
            "failed_result_sheets": recalc.failed_json(),
        }),
    ))
}

#[derive(Debug, Clone)]
enum ColMapTarget {
    Exact(usize),
    Expanded(Vec<usize>),
    MovedToX,
    Unmappable,
}

#[derive(Debug, Clone)]
struct ConversionColPlan {
    cols: Vec<ColMapTarget>,
    x_injected_at_col0: bool,
    preserves_cell_shape: bool,
    transposes_row_subcol: bool,
    target_table_kind: TableKind,
    converted_col_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MvColClass {
    Categorical,
    Label,
    Numeric,
}

fn classify_mv_cols_for_plan(mv: &MultipleVariablesTable) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
    let mut cat = Vec::new();
    let mut label = Vec::new();
    let mut num = Vec::new();
    for (i, var) in mv.variables.iter().enumerate() {
        let cls = if var.role == VariableRole::Categorical {
            MvColClass::Categorical
        } else if var.role == VariableRole::Label {
            MvColClass::Label
        } else if var.role_inferred && var.cells.iter().any(|c| !c.is_empty() && matches!(c.value, CellValue::Text(_))) {
            MvColClass::Categorical
        } else {
            MvColClass::Numeric
        };
        match cls {
            MvColClass::Categorical => cat.push(i),
            MvColClass::Label => label.push(i),
            MvColClass::Numeric => num.push(i),
        }
    }
    (cat, label, num)
}

const fn target_table_kind_from(target: TargetArchetypeKind) -> TableKind {
    match target {
        TargetArchetypeKind::Column => TableKind::Column,
        TargetArchetypeKind::Grouped => TableKind::Grouped,
        TargetArchetypeKind::Xy => TableKind::Xy,
        TargetArchetypeKind::MultipleVariables => TableKind::MultipleVariables,
        TargetArchetypeKind::Contingency => TableKind::Contingency,
    }
}

impl ConversionColPlan {
    fn build(old_ds: &DataSheet, target: TargetArchetypeKind) -> Self {
        let old_cols = old_ds.archetype.column_count();
        let target_table_kind = target_table_kind_from(target);
        if old_ds.kind() == target_table_kind {
            return Self {
                cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                x_injected_at_col0: false,
                preserves_cell_shape: true,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: old_cols,
            };
        }
        match (&old_ds.archetype, target) {
            (TableArchetype::Column(_), TargetArchetypeKind::Grouped) => Self {
                cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                x_injected_at_col0: false,
                preserves_cell_shape: true,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: old_cols,
            },
            (TableArchetype::Grouped(grp), TargetArchetypeKind::Column) => Self::from_grouped_to_column(grp, target_table_kind),
            (TableArchetype::Column(_), TargetArchetypeKind::Xy) => {
                let cols = if old_cols == 1 {
                    vec![ColMapTarget::Exact(0)]
                } else {
                    std::iter::once(ColMapTarget::MovedToX).chain((0..old_cols.saturating_sub(1)).map(ColMapTarget::Exact)).collect()
                };
                Self {
                    cols,
                    x_injected_at_col0: false,
                    preserves_cell_shape: true,
                    transposes_row_subcol: false,
                    target_table_kind,
                    converted_col_count: if old_cols == 1 { 1 } else { old_cols.saturating_sub(1) },
                }
            }
            (TableArchetype::Xy(xy), TargetArchetypeKind::Column) => Self::from_xy_to_column(xy, target_table_kind),
            (TableArchetype::Grouped(_), TargetArchetypeKind::MultipleVariables) => Self {
                cols: vec![ColMapTarget::Unmappable; old_cols],
                x_injected_at_col0: false,
                preserves_cell_shape: false,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: 3,
            },
            (TableArchetype::MultipleVariables(mv), TargetArchetypeKind::Grouped) => Self::from_mv_to_grouped(mv, target_table_kind),
            (TableArchetype::Xy(xy), TargetArchetypeKind::MultipleVariables) => Self::from_xy_to_mv(xy, target_table_kind),
            (TableArchetype::MultipleVariables(mv), TargetArchetypeKind::Xy) => Self::from_mv_to_xy(mv, target_table_kind),
            (TableArchetype::Column(col), TargetArchetypeKind::MultipleVariables) => {
                let subs = col.subcolumn_mode.subcolumn_count();
                let cols = if subs <= 1 {
                    (0..old_cols).map(ColMapTarget::Exact).collect()
                } else {
                    (0..old_cols).map(|c| ColMapTarget::Expanded((c * subs..(c + 1) * subs).collect())).collect()
                };
                Self {
                    cols,
                    x_injected_at_col0: false,
                    preserves_cell_shape: subs <= 1,
                    transposes_row_subcol: false,
                    target_table_kind,
                    converted_col_count: old_cols.saturating_mul(subs),
                }
            }
            (TableArchetype::MultipleVariables(mv), TargetArchetypeKind::Column) => Self::from_mv_to_column(mv, target_table_kind),
            (TableArchetype::Grouped(_), TargetArchetypeKind::Xy) | (TableArchetype::Xy(_), TargetArchetypeKind::Grouped) => Self {
                cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                x_injected_at_col0: false,
                preserves_cell_shape: true,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: old_cols,
            },
            (TableArchetype::Column(_), TargetArchetypeKind::Contingency) => Self {
                cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                x_injected_at_col0: false,
                preserves_cell_shape: true,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: old_cols,
            },
            (TableArchetype::Grouped(grp), TargetArchetypeKind::Contingency) => Self {
                cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                x_injected_at_col0: false,
                preserves_cell_shape: grp.subcolumn_mode.subcolumn_count() <= 1,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: old_cols,
            },
            (TableArchetype::Xy(xy), TargetArchetypeKind::Contingency) => Self {
                cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                x_injected_at_col0: false,
                preserves_cell_shape: xy.y_subcolumn_mode.subcolumn_count() <= 1,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: old_cols,
            },
            (TableArchetype::MultipleVariables(mv), TargetArchetypeKind::Contingency) => Self::from_mv_to_contingency(mv, target_table_kind),
            _ => Self {
                cols: vec![ColMapTarget::Unmappable; old_cols],
                x_injected_at_col0: false,
                preserves_cell_shape: false,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: old_cols,
            },
        }
    }

    fn from_grouped_to_column(grp: &curvebook_doc::GroupedTable, target_table_kind: TableKind) -> Self {
        let old_cols = grp.data_sets.len();
        let rows = grp.data_sets.iter().map(DataSetColumn::row_count).max().unwrap_or(grp.row_titles.len()).max(grp.row_titles.len());
        match grp.subcolumn_mode {
            SubcolumnMode::Single | SubcolumnMode::RawReplicates(1) | SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN | SubcolumnMode::MeanCvN => {
                Self {
                    cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                    x_injected_at_col0: false,
                    preserves_cell_shape: true,
                    transposes_row_subcol: false,
                    target_table_kind,
                    converted_col_count: old_cols,
                }
            }
            SubcolumnMode::RawReplicates(n) if n >= 2 && rows <= 1 => Self {
                cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                x_injected_at_col0: false,
                preserves_cell_shape: false,
                transposes_row_subcol: true,
                target_table_kind,
                converted_col_count: old_cols,
            },
            SubcolumnMode::RawReplicates(n) if n >= 2 => {
                if let Some(total_cols) = old_cols.checked_mul(rows)
                    && total_cols <= MAX_DATA_SETS
                {
                    Self {
                        cols: (0..old_cols).map(|c| ColMapTarget::Expanded((c * rows..(c + 1) * rows).collect())).collect(),
                        x_injected_at_col0: false,
                        preserves_cell_shape: false,
                        transposes_row_subcol: true,
                        target_table_kind,
                        converted_col_count: total_cols,
                    }
                } else {
                    Self {
                        cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                        x_injected_at_col0: false,
                        preserves_cell_shape: false,
                        transposes_row_subcol: false,
                        target_table_kind,
                        converted_col_count: old_cols,
                    }
                }
            }
            _ => Self {
                cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                x_injected_at_col0: false,
                preserves_cell_shape: false,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: old_cols,
            },
        }
    }

    fn from_xy_to_column(xy: &curvebook_doc::XyTable, target_table_kind: TableKind) -> Self {
        let old_cols = xy.data_sets.len();
        if matches!(xy.y_subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr) {
            return Self {
                cols: (0..old_cols).map(|c| ColMapTarget::Exact(1 + c)).collect(),
                x_injected_at_col0: true,
                preserves_cell_shape: false,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: 1 + old_cols,
            };
        }
        if let SubcolumnMode::RawReplicates(n) = xy.y_subcolumn_mode
            && n >= 2
        {
            let needed = old_cols.checked_mul(n).and_then(|v| v.checked_add(1));
            if needed.is_some_and(|tot| tot <= MAX_DATA_SETS) {
                return Self {
                    cols: (0..old_cols).map(|c| ColMapTarget::Expanded((1 + c * n..1 + (c + 1) * n).collect())).collect(),
                    x_injected_at_col0: true,
                    preserves_cell_shape: false,
                    transposes_row_subcol: false,
                    target_table_kind,
                    converted_col_count: 1 + old_cols * n,
                };
            }
            return Self {
                cols: (0..old_cols).map(|c| ColMapTarget::Exact(1 + c)).collect(),
                x_injected_at_col0: true,
                preserves_cell_shape: false,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: 1 + old_cols,
            };
        }
        if xy.y_subcolumn_mode.supports_parametric_moments() {
            let matching_parametric = matches!(&xy.x_mode, XColumnMode::Numbers { subcolumn_mode } if *subcolumn_mode == xy.y_subcolumn_mode);
            if matching_parametric {
                return Self {
                    cols: (0..old_cols).map(|c| ColMapTarget::Exact(1 + c)).collect(),
                    x_injected_at_col0: true,
                    preserves_cell_shape: true,
                    transposes_row_subcol: false,
                    target_table_kind,
                    converted_col_count: 1 + old_cols,
                };
            }
            return Self {
                cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                x_injected_at_col0: false,
                preserves_cell_shape: true,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: old_cols,
            };
        }
        Self {
            cols: (0..old_cols).map(|c| ColMapTarget::Exact(1 + c)).collect(),
            x_injected_at_col0: true,
            preserves_cell_shape: true,
            transposes_row_subcol: false,
            target_table_kind,
            converted_col_count: 1 + old_cols,
        }
    }

    fn from_mv_to_grouped(mv: &MultipleVariablesTable, target_table_kind: TableKind) -> Self {
        let old_cols = mv.variables.len();
        let (cat_cols, _, _) = classify_mv_cols_for_plan(mv);
        if cat_cols.is_empty() {
            Self {
                cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                x_injected_at_col0: false,
                preserves_cell_shape: true,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: old_cols,
            }
        } else {
            Self {
                cols: vec![ColMapTarget::Unmappable; old_cols],
                x_injected_at_col0: false,
                preserves_cell_shape: false,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: 1,
            }
        }
    }

    fn from_xy_to_mv(xy: &curvebook_doc::XyTable, target_table_kind: TableKind) -> Self {
        let old_cols = xy.data_sets.len();
        let x_subs = xy.x_mode.subcolumn_count();
        let y_subs = xy.y_subcolumn_mode.subcolumn_count();
        if xy.y_subcolumn_mode == SubcolumnMode::Single && x_subs <= 1 {
            return Self {
                cols: (0..old_cols).map(|c| ColMapTarget::Exact(1 + c)).collect(),
                x_injected_at_col0: true,
                preserves_cell_shape: true,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: 1 + old_cols,
            };
        }
        if matches!(xy.y_subcolumn_mode, SubcolumnMode::RawReplicates(n) if n >= 2) && x_subs <= 1 {
            return Self {
                cols: vec![ColMapTarget::Unmappable; old_cols],
                x_injected_at_col0: false,
                preserves_cell_shape: false,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: 4,
            };
        }
        let x_vars = if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }) { 1 } else { x_subs };
        let cols = (0..old_cols)
            .map(|c| {
                let start = x_vars + c * y_subs;
                if y_subs <= 1 { ColMapTarget::Exact(start) } else { ColMapTarget::Expanded((start..start + y_subs).collect()) }
            })
            .collect();
        Self {
            cols,
            x_injected_at_col0: x_vars > 0,
            preserves_cell_shape: false,
            transposes_row_subcol: false,
            target_table_kind,
            converted_col_count: x_vars + old_cols * y_subs,
        }
    }

    fn from_mv_to_xy(mv: &MultipleVariablesTable, target_table_kind: TableKind) -> Self {
        let old_cols = mv.variables.len();
        let (cat_cols, label_cols, num_cols) = classify_mv_cols_for_plan(mv);
        if cat_cols.len() == 1 && num_cols.len() >= 2 {
            return Self {
                cols: vec![ColMapTarget::Unmappable; old_cols],
                x_injected_at_col0: false,
                preserves_cell_shape: false,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: 1,
            };
        }
        let mut cols = vec![ColMapTarget::Unmappable; old_cols];
        if num_cols.len() == 1 {
            cols[num_cols[0]] = ColMapTarget::Exact(0);
            return Self {
                cols,
                x_injected_at_col0: false,
                preserves_cell_shape: cat_cols.is_empty() && label_cols.is_empty(),
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: 1,
            };
        }
        if num_cols.len() >= 2 {
            cols[num_cols[0]] = ColMapTarget::MovedToX;
            for (yi, &nc) in num_cols[1..].iter().enumerate() {
                cols[nc] = ColMapTarget::Exact(yi);
            }
            return Self {
                cols,
                x_injected_at_col0: false,
                preserves_cell_shape: cat_cols.is_empty() && label_cols.is_empty(),
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: num_cols.len() - 1,
            };
        }
        Self { cols, x_injected_at_col0: false, preserves_cell_shape: false, transposes_row_subcol: false, target_table_kind, converted_col_count: 1 }
    }

    fn from_mv_to_column(mv: &MultipleVariablesTable, target_table_kind: TableKind) -> Self {
        let old_cols = mv.variables.len();
        let (cat_cols, label_cols, num_cols) = classify_mv_cols_for_plan(mv);
        if cat_cols.len() == 1 && num_cols.len() == 1 && old_cols == 2 {
            Self {
                cols: vec![ColMapTarget::Unmappable; old_cols],
                x_injected_at_col0: false,
                preserves_cell_shape: false,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: 1,
            }
        } else {
            Self {
                cols: (0..old_cols).map(ColMapTarget::Exact).collect(),
                x_injected_at_col0: false,
                preserves_cell_shape: cat_cols.is_empty() && label_cols.is_empty(),
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: old_cols,
            }
        }
    }

    fn from_mv_to_contingency(mv: &MultipleVariablesTable, target_table_kind: TableKind) -> Self {
        let old_cols = mv.variables.len();
        let (cat_cols, label_cols, num_cols) = classify_mv_cols_for_plan(mv);
        let is_cross_tab = cat_cols.len() == 2
            && mv.variables[cat_cols[0]].role == VariableRole::Categorical
            && mv.variables[cat_cols[1]].role == VariableRole::Categorical
            && (num_cols.is_empty() || (num_cols.len() == 1 && old_cols == 3 + label_cols.len()));
        if is_cross_tab {
            return Self {
                cols: vec![ColMapTarget::Unmappable; old_cols],
                x_injected_at_col0: false,
                preserves_cell_shape: false,
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: 2,
            };
        }
        let mv_has_empty_row_titles = mv.row_titles.iter().all(|s| s.trim().is_empty());
        let label_col = label_cols.first().copied().filter(|_| label_cols.len() == 1 && !num_cols.is_empty() && mv_has_empty_row_titles);
        let data_indices: Vec<usize> = (0..old_cols).filter(|&i| Some(i) != label_col).collect();
        let mut cols = vec![ColMapTarget::Unmappable; old_cols];
        for (dst_c, &src_c) in data_indices.iter().enumerate() {
            cols[src_c] = ColMapTarget::Exact(dst_c);
        }
        Self {
            cols,
            x_injected_at_col0: false,
            preserves_cell_shape: false,
            transposes_row_subcol: false,
            target_table_kind,
            converted_col_count: data_indices.len().max(1),
        }
    }

    const fn oob_index(&self) -> usize {
        self.converted_col_count.saturating_add(1)
    }

    fn all_unmappable(&self) -> bool {
        !self.cols.is_empty() && self.cols.iter().all(|c| matches!(c, ColMapTarget::Unmappable))
    }

    fn map_one(&self, old_col: usize, role_desc: &str) -> Result<usize, String> {
        match self.cols.get(old_col) {
            Some(ColMapTarget::Exact(new_c)) => Ok(*new_c),
            Some(ColMapTarget::Expanded(vec)) => vec.first().copied().ok_or_else(|| format!("{role_desc} (col {old_col}) mapped to empty expansion")),
            Some(ColMapTarget::MovedToX) => Err(format!("{role_desc} (col {old_col}) moved into X/row-factor column")),
            Some(ColMapTarget::Unmappable) | None => Err(format!("{role_desc} (col {old_col}) cannot be mapped after conversion")),
        }
    }

    fn map_one_or_oob(&self, col: &mut usize, role_desc: &str, err: &mut Option<String>) {
        match self.map_one(*col, role_desc) {
            Ok(nc) => *col = nc,
            Err(reason) => {
                *col = self.oob_index();
                if err.is_none() {
                    *err = Some(reason);
                }
            }
        }
    }

    fn map_opt_or_oob(&self, opt: &mut Option<usize>, role_desc: &str, err: &mut Option<String>) {
        if let Some(col) = opt.as_mut() {
            self.map_one_or_oob(col, role_desc, err);
        }
    }

    fn map_col_list(&self, cols: &[usize], role_desc: &str) -> Result<Vec<usize>, String> {
        let mut out = Vec::with_capacity(cols.len());
        let mut seen = HashSet::new();
        for &c in cols {
            match self.cols.get(c) {
                Some(ColMapTarget::Exact(new_c)) => {
                    if seen.insert(*new_c) {
                        out.push(*new_c);
                    }
                }
                Some(ColMapTarget::Expanded(vec)) => {
                    for &nc in vec {
                        if seen.insert(nc) {
                            out.push(nc);
                        }
                    }
                }
                Some(ColMapTarget::MovedToX) => return Err(format!("{role_desc} (col {c}) moved into X/row-factor column")),
                Some(ColMapTarget::Unmappable) | None => return Err(format!("{role_desc} (col {c}) cannot be mapped after conversion")),
            }
        }
        Ok(out)
    }

    fn map_col_list_or_oob(&self, cols: &[usize], role_desc: &str, err: &mut Option<String>) -> Vec<usize> {
        let mut out = Vec::with_capacity(cols.len());
        let mut seen = HashSet::new();
        for &c in cols {
            match self.cols.get(c) {
                Some(ColMapTarget::Exact(new_c)) => {
                    if seen.insert(*new_c) {
                        out.push(*new_c);
                    }
                }
                Some(ColMapTarget::Expanded(vec)) => {
                    for &nc in vec {
                        if seen.insert(nc) {
                            out.push(nc);
                        }
                    }
                }
                Some(ColMapTarget::MovedToX) => {
                    if err.is_none() {
                        *err = Some(format!("{role_desc} (col {c}) moved into X/row-factor column"));
                    }
                    out.push(self.oob_index());
                }
                Some(ColMapTarget::Unmappable) | None => {
                    if err.is_none() {
                        *err = Some(format!("{role_desc} (col {c}) cannot be mapped after conversion"));
                    }
                    out.push(self.oob_index());
                }
            }
        }
        out
    }

    fn map_selected_data_sets(&self, selected: &[usize]) -> Result<(Vec<usize>, Option<String>), String> {
        if self.all_unmappable() {
            return Err(String::from("all source columns were pivoted/restructured by conversion"));
        }
        if selected.is_empty() {
            if self.x_injected_at_col0 {
                let expanded = self.map_col_list(&(0..self.cols.len()).collect::<Vec<_>>(), "selected_data_sets")?;
                return Ok((expanded, None));
            }
            let moved_x: Vec<usize> = self.cols.iter().enumerate().filter_map(|(i, m)| matches!(m, ColMapTarget::MovedToX).then_some(i)).collect();
            let note = if moved_x.is_empty() {
                None
            } else {
                Some(format!("column(s) {moved_x:?} moved to X and are excluded from downstream `selected_data_sets: []`"))
            };
            return Ok((Vec::new(), note));
        }
        Ok((self.map_col_list(selected, "selected_data_sets")?, None))
    }

    fn map_source_range(&self, range: SheetCellRange) -> Result<SheetCellRange, String> {
        let old_cols = self.cols.len();
        if range.start.col >= old_cols {
            return Err(format!("linked source start column {} is out of bounds", range.start.col));
        }
        if self.transposes_row_subcol {
            return Err(String::from("conversion transposed replicate subcolumns and rows; linked source range cannot preserve 2D layout"));
        }
        if range.start.col == 0 && range.end.col >= old_cols.saturating_sub(1) && range.end.col >= 1_000_000 {
            if self.all_unmappable() {
                return Err(String::from("open column range cannot survive pivoted table conversion"));
            }
            if self.cols.iter().any(|c| matches!(c, ColMapTarget::MovedToX | ColMapTarget::Unmappable)) {
                return Err(String::from("open column range included column dropped or moved to X"));
            }
            let start_col = usize::from(self.x_injected_at_col0);
            return Ok(SheetCellRange::new(
                CellCoord::new(range.start.row, start_col, range.start.subcol),
                CellCoord::new(range.end.row, range.end.col, range.end.subcol),
            ));
        }
        let end_c = range.end.col.min(old_cols.saturating_sub(1));
        if range.start.col == end_c
            && let Some(ColMapTarget::Expanded(vec)) = self.cols.get(range.start.col)
            && range.start.subcol == range.end.subcol
            && let Some(&single_col) = vec.get(range.start.subcol)
        {
            return Ok(SheetCellRange::new(CellCoord::new(range.start.row, single_col, 0), CellCoord::new(range.end.row, single_col, 0)));
        }
        let mapped = self.map_col_list(&(range.start.col..=end_c).collect::<Vec<_>>(), "linked source range")?;
        let min_c = *mapped.iter().min().ok_or_else(|| String::from("linked source range mapped to zero columns"))?;
        let max_c = *mapped.iter().max().ok_or_else(|| String::from("linked source range mapped to zero columns"))?;
        let contiguous = mapped.len() == max_c - min_c + 1;
        if !contiguous {
            return Err(String::from("linked source column range became non-contiguous after conversion"));
        }
        Ok(SheetCellRange::new(CellCoord::new(range.start.row, min_c, range.start.subcol), CellCoord::new(range.end.row, max_c, range.end.subcol)))
    }

    fn map_incoming_region_range(&self, range: SheetCellRange, old_cols: usize) -> Option<SheetCellRange> {
        if !self.preserves_cell_shape || range.start.col > range.end.col || range.start.col >= old_cols {
            return None;
        }
        let end_bound = range.end.col.min(old_cols.saturating_sub(1));
        let mut mapped = Vec::with_capacity(end_bound - range.start.col + 1);
        for c in range.start.col..=end_bound {
            match self.cols.get(c) {
                Some(ColMapTarget::Exact(new_c)) => mapped.push(*new_c),
                _ => return None,
            }
        }
        let first = *mapped.first()?;
        for (offset, &nc) in mapped.iter().enumerate() {
            if nc != first + offset {
                return None;
            }
        }
        let new_end_col = if range.end.col >= old_cols && range.end.col >= 1_000_000 {
            range.end.col
        } else if range.end.col >= old_cols {
            return None;
        } else {
            *mapped.last()?
        };
        Some(SheetCellRange::new(
            CellCoord::new(range.start.row, first, range.start.subcol),
            CellCoord::new(range.end.row, new_end_col, range.end.subcol),
        ))
    }
}

struct ConversionImpact {
    col_plan: ConversionColPlan,
    warnings: Vec<String>,
    incoming_links_lost: bool,
    incoming_links_dropped: usize,
    broken_result_reasons: Vec<(SheetId, String)>,
    broken_link_ids: Vec<(SheetId, String)>,
}

fn remap_analysis_spec_for_conversion(
    spec: &mut curvebook_doc::AnalysisSpec,
    target_sheet_id: &SheetId,
    plan: &ConversionColPlan,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let mut err: Option<String> = None;
    remap_analysis_kind_column_hooks(&mut spec.kind, target_sheet_id, plan, warnings, &mut err);
    if spec.source_sheet_id != *target_sheet_id {
        return match err {
            Some(reason) => Err(reason),
            None => Ok(()),
        };
    }
    if !spec.kind.is_compatible_with(plan.target_table_kind) {
        return Err(format!(
            "analysis `{}` is incompatible with converted {:?} table `{target_sheet_id}`",
            spec.effective_title(),
            plan.target_table_kind
        ));
    }
    match plan.map_selected_data_sets(&spec.selected_data_sets) {
        Ok((new_sel, sel_note)) => {
            spec.selected_data_sets = new_sel;
            if let Some(note) = sel_note {
                warnings.push(format!("Result sheet `{}`: {note}", spec.effective_title()));
            }
        }
        Err(reason) => {
            spec.selected_data_sets = vec![plan.oob_index()];
            if err.is_none() {
                err = Some(reason);
            }
        }
    }
    remap_analysis_kind_columns(&mut spec.kind, plan, &mut err);
    if let Some(reason) = err {
        spec.selected_data_sets = vec![plan.oob_index()];
        return Err(reason);
    }
    Ok(())
}

fn remap_hook_col_title(
    hook: &mut HookSource,
    target_sheet_id: &SheetId,
    plan: &ConversionColPlan,
    warnings: &mut Vec<String>,
    err: &mut Option<String>,
) {
    if let HookSource::ColumnTitleValue { data_sheet_id, data_set_index } = hook
        && data_sheet_id == target_sheet_id
    {
        if matches!(plan.cols.get(*data_set_index), Some(ColMapTarget::Expanded(_))) {
            warnings.push(format!(
                "Column-title hook on `{target_sheet_id}` col {} targets split replicate columns; verify header numeric value",
                *data_set_index
            ));
        }
        plan.map_one_or_oob(data_set_index, "ColumnTitleValue hook", err);
    }
}

fn remap_analysis_kind_column_hooks(
    kind: &mut AnalysisKind,
    target_sheet_id: &SheetId,
    plan: &ConversionColPlan,
    warnings: &mut Vec<String>,
    err: &mut Option<String>,
) {
    match kind {
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, .. }) => {
            if let NormalizeAnchor::Hooked(hook) = zero_anchor {
                remap_hook_col_title(hook, target_sheet_id, plan, warnings, err);
            }
            if let NormalizeAnchor::Hooked(hook) = hundred_anchor {
                remap_hook_col_title(hook, target_sheet_id, plan, warnings, err);
            }
        }
        AnalysisKind::NonlinearRegression(nl) => {
            for rule in &mut nl.parameter_rules {
                if let ParameterScope::HookedConstant(hook) = &mut rule.scope {
                    remap_hook_col_title(hook, target_sheet_id, plan, warnings, err);
                }
            }
        }
        _ => {}
    }
}

fn remap_analysis_kind_columns(kind: &mut AnalysisKind, plan: &ConversionColPlan, err: &mut Option<String>) {
    match kind {
        AnalysisKind::OneWayAnova(spec) => {
            if let MultiplicityMethod::Dunnett { control_column } = &mut spec.multiplicity.method {
                plan.map_one_or_oob(control_column, "Dunnett control_column", err);
            }
        }
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, .. }) => {
            if let NormalizeAnchor::ControlColumnMean { column } = zero_anchor {
                plan.map_one_or_oob(column, "Normalize zero_anchor control column", err);
            }
            if let NormalizeAnchor::ControlColumnMean { column } = hundred_anchor {
                plan.map_one_or_oob(column, "Normalize hundred_anchor control column", err);
            }
        }
        AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, .. }) => {
            *baseline_columns = plan.map_col_list_or_oob(baseline_columns, "RemoveBaseline baseline_columns", err);
        }
        AnalysisKind::TwoGroupComparison(spec) => {
            plan.map_opt_or_oob(&mut spec.mv_group_column, "mv_group_column", err);
            plan.map_opt_or_oob(&mut spec.mv_response_column, "mv_response_column", err);
        }
        AnalysisKind::BlandAltman(spec) => {
            plan.map_one_or_oob(&mut spec.column_a, "BlandAltman column_a", err);
            plan.map_one_or_oob(&mut spec.column_b, "BlandAltman column_b", err);
        }
        AnalysisKind::PassingBablok(spec) => {
            plan.map_one_or_oob(&mut spec.x_column, "PassingBablok x_column", err);
            plan.map_one_or_oob(&mut spec.y_column, "PassingBablok y_column", err);
        }
        AnalysisKind::PValueStack(spec) => plan.map_opt_or_oob(&mut spec.column, "PValueStack column", err),
        AnalysisKind::FactorialAnova(spec) => {
            spec.multifactor_columns = plan.map_col_list_or_oob(&spec.multifactor_columns, "FactorialAnova multifactor_columns", err);
            plan.map_opt_or_oob(&mut spec.response_column, "FactorialAnova response_column", err);
            if let MultiplicityMethod::Dunnett { control_column } = &mut spec.multiplicity.method {
                plan.map_one_or_oob(control_column, "FactorialAnova Dunnett control_column", err);
            }
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column, mv_status_column, mv_group_column, .. }) => {
            plan.map_opt_or_oob(mv_time_column, "KaplanMeier mv_time_column", err);
            plan.map_opt_or_oob(mv_status_column, "KaplanMeier mv_status_column", err);
            plan.map_opt_or_oob(mv_group_column, "KaplanMeier mv_group_column", err);
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column,
            status_column,
            predictor_columns,
            reference_levels,
            ..
        }) => {
            plan.map_one_or_oob(time_column, "Cox time_column", err);
            plan.map_one_or_oob(status_column, "Cox status_column", err);
            *predictor_columns = plan.map_col_list_or_oob(predictor_columns, "Cox predictor_columns", err);
            for (ref_col, _) in reference_levels.iter_mut() {
                plan.map_one_or_oob(ref_col, "Cox reference_levels column", err);
            }
        }
        AnalysisKind::GeneralizedLinearModel(spec) => {
            plan.map_one_or_oob(&mut spec.response_column, "GLM response_column", err);
            spec.predictor_columns = plan.map_col_list_or_oob(&spec.predictor_columns, "GLM predictor_columns", err);
            for (ref_col, _) in &mut spec.reference_levels {
                plan.map_one_or_oob(ref_col, "GLM reference_levels column", err);
            }
        }
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { response_column, predictor_columns, .. }) => {
            plan.map_opt_or_oob(response_column, "MultipleImputation response_column", err);
            *predictor_columns = plan.map_col_list_or_oob(predictor_columns, "MultipleImputation predictor_columns", err);
        }
        AnalysisKind::NonlinearRegression(nl) => {
            nl.mv_group_columns = plan.map_col_list_or_oob(&nl.mv_group_columns, "NonlinearRegression mv_group_columns", err);
            nl.mv_x_columns = plan.map_col_list_or_oob(&nl.mv_x_columns, "NonlinearRegression mv_x_columns", err);
            plan.map_opt_or_oob(&mut nl.mv_y_column, "NonlinearRegression mv_y_column", err);
            for rule in &mut nl.parameter_rules {
                if let ParameterScope::PerDatasetConstant(pairs) = &mut rule.scope {
                    for (col, _) in pairs.iter_mut() {
                        plan.map_one_or_oob(col, "PerDatasetConstant column", err);
                    }
                }
            }
        }
        _ => {}
    }
}

fn remap_point_override_for_conversion(
    mut ov: PointStyleOverride,
    plan: &ConversionColPlan,
    converted_ds: Option<&DataSheet>,
    warnings: &mut Vec<String>,
    graph_id: &SheetId,
) -> Vec<PointStyleOverride> {
    let row_ok = |point_index: usize| converted_ds.is_none_or(|ds| point_index < ds.archetype.row_count());
    let subcol_ok = |series_index: usize, subcol: Option<usize>| match (converted_ds, subcol) {
        (Some(ds), Some(sc)) => sc < column_subcolumn_count(&ds.archetype, series_index),
        _ => true,
    };
    match plan.cols.get(ov.series_index) {
        Some(ColMapTarget::Exact(new_col)) if !plan.transposes_row_subcol => {
            ov.series_index = *new_col;
            if row_ok(ov.point_index) && subcol_ok(*new_col, ov.subcol) {
                vec![ov]
            } else {
                warnings.push(format!(
                    "Graph `{graph_id}` dropped out-of-bounds point override ({}, {}) after conversion",
                    ov.series_index, ov.point_index
                ));
                Vec::new()
            }
        }
        Some(ColMapTarget::Exact(new_col)) => {
            if ov.point_index != 0 {
                warnings.push(format!("Graph `{graph_id}` dropped point override on out-of-range row {} after conversion", ov.point_index));
                return Vec::new();
            }
            if let Some(sc) = ov.subcol {
                ov.series_index = *new_col;
                ov.point_index = sc;
                ov.subcol = None;
                if row_ok(ov.point_index) && subcol_ok(*new_col, ov.subcol) { vec![ov] } else { Vec::new() }
            } else if let Some(ds) = converted_ds {
                (0..ds.archetype.row_count())
                    .map(|dst_r| {
                        let mut cloned = ov.clone();
                        cloned.series_index = *new_col;
                        cloned.point_index = dst_r;
                        cloned.subcol = None;
                        cloned
                    })
                    .collect()
            } else {
                ov.series_index = *new_col;
                ov.subcol = None;
                vec![ov]
            }
        }
        Some(ColMapTarget::Expanded(cols)) if !plan.transposes_row_subcol && !cols.is_empty() => {
            if let Some(sc) = ov.subcol {
                let Some(&new_col) = cols.get(sc) else {
                    warnings
                        .push(format!("Graph `{graph_id}` dropped point override with out-of-range subcolumn {sc} on column {}", ov.series_index));
                    return Vec::new();
                };
                ov.series_index = new_col;
                ov.subcol = None;
                if row_ok(ov.point_index) && subcol_ok(new_col, ov.subcol) { vec![ov] } else { Vec::new() }
            } else if row_ok(ov.point_index) {
                cols.iter()
                    .filter_map(|&new_col| {
                        let mut cloned = ov.clone();
                        cloned.series_index = new_col;
                        cloned.subcol = None;
                        subcol_ok(new_col, None).then_some(cloned)
                    })
                    .collect()
            } else {
                Vec::new()
            }
        }
        Some(ColMapTarget::Expanded(cols)) if !cols.is_empty() => {
            let Some(&new_col) = cols.get(ov.point_index) else {
                warnings.push(format!(
                    "Graph `{graph_id}` dropped point override with out-of-range row {} on transposed column {}",
                    ov.point_index, ov.series_index
                ));
                return Vec::new();
            };
            if let Some(sc) = ov.subcol {
                ov.series_index = new_col;
                ov.point_index = sc;
                ov.subcol = None;
                if row_ok(ov.point_index) && subcol_ok(new_col, None) { vec![ov] } else { Vec::new() }
            } else if let Some(ds) = converted_ds {
                (0..ds.archetype.row_count())
                    .map(|dst_r| {
                        let mut cloned = ov.clone();
                        cloned.series_index = new_col;
                        cloned.point_index = dst_r;
                        cloned.subcol = None;
                        cloned
                    })
                    .collect()
            } else {
                ov.series_index = new_col;
                ov.subcol = None;
                vec![ov]
            }
        }
        _ => {
            warnings.push(format!("Graph `{graph_id}` dropped point override for unmappable column {}", ov.series_index));
            Vec::new()
        }
    }
}

fn remap_graph_spec_for_conversion(
    spec: &mut curvebook_doc::GraphSpec,
    target_sheet_id: &SheetId,
    plan: &ConversionColPlan,
    converted_ds: Option<&DataSheet>,
    warnings: &mut Vec<String>,
    graph_id: &SheetId,
) {
    let old_series = spec.series.clone();
    let mut kept_series = Vec::with_capacity(spec.series.len());
    let mut slot_targets: Vec<Vec<usize>> = Vec::with_capacity(old_series.len());
    for mut s in std::mem::take(&mut spec.series) {
        if s.source_sheet_id == *target_sheet_id {
            match plan.cols.get(s.data_set_index) {
                Some(ColMapTarget::Exact(new_col)) => {
                    let slot = kept_series.len();
                    s.data_set_index = *new_col;
                    kept_series.push(s);
                    slot_targets.push(vec![slot]);
                }
                Some(ColMapTarget::Expanded(cols)) if !cols.is_empty() => {
                    let mut slots = Vec::with_capacity(cols.len());
                    for &new_col in cols {
                        let slot = kept_series.len();
                        let mut expanded = s.clone();
                        expanded.data_set_index = new_col;
                        kept_series.push(expanded);
                        slots.push(slot);
                    }
                    slot_targets.push(slots);
                }
                _ => {
                    let reason = plan
                        .map_one(s.data_set_index, "graph series")
                        .err()
                        .unwrap_or_else(|| format!("column {} has no target after conversion", s.data_set_index));
                    warnings.push(format!("Graph `{graph_id}` dropped series binding: {reason}"));
                    slot_targets.push(Vec::new());
                }
            }
        } else {
            let slot = kept_series.len();
            kept_series.push(s);
            slot_targets.push(vec![slot]);
        }
    }
    spec.series = kept_series;

    if !spec.point_overrides.is_empty() {
        let mut kept_overrides = Vec::with_capacity(spec.point_overrides.len());
        for mut ov in std::mem::take(&mut spec.point_overrides) {
            if !old_series.is_empty() {
                let old_slot = ov.series_index;
                let Some(prev_series) = old_series.get(old_slot) else {
                    warnings.push(format!("Graph `{graph_id}` dropped orphaned point override on series {old_slot}"));
                    continue;
                };
                let target_slots = slot_targets.get(old_slot).map(Vec::as_slice).unwrap_or(&[]);
                if prev_series.source_sheet_id != *target_sheet_id {
                    if let Some(&new_slot) = target_slots.first() {
                        ov.series_index = new_slot;
                        kept_overrides.push(ov);
                    }
                    continue;
                }
                if target_slots.is_empty() {
                    warnings.push(format!("Graph `{graph_id}` dropped point override for removed series {old_slot}"));
                    continue;
                }
                ov.series_index = prev_series.data_set_index;
                for mut mapped in remap_point_override_for_conversion(ov, plan, converted_ds, warnings, graph_id) {
                    if let Some(&new_slot) = target_slots.iter().find(|&&slot| spec.series[slot].data_set_index == mapped.series_index) {
                        mapped.series_index = new_slot;
                        kept_overrides.push(mapped);
                    }
                }
            } else if spec.primary_source_sheet_id != *target_sheet_id {
                kept_overrides.push(ov);
            } else {
                kept_overrides.extend(remap_point_override_for_conversion(ov, plan, converted_ds, warnings, graph_id));
            }
        }
        spec.point_overrides = kept_overrides;
    }

    if spec.primary_source_sheet_id == *target_sheet_id
        && let Some(mv) = &mut spec.multivariate
    {
        let mut clear_multivariate = false;
        match plan.map_one(mv.x_column, "graph multivariate x_column") {
            Ok(xc) => mv.x_column = xc,
            Err(reason) => {
                warnings.push(format!("Graph `{graph_id}` cleared multivariate encoding: {reason}"));
                clear_multivariate = true;
            }
        }
        if !clear_multivariate {
            match plan.map_one(mv.y_column, "graph multivariate y_column") {
                Ok(yc) => mv.y_column = yc,
                Err(reason) => {
                    warnings.push(format!("Graph `{graph_id}` cleared multivariate encoding: {reason}"));
                    clear_multivariate = true;
                }
            }
        }
        if clear_multivariate {
            spec.multivariate = None;
            return;
        }
        if let Some(lc) = mv.label_column {
            match plan.map_one(lc, "graph multivariate label_column") {
                Ok(nlc) => mv.label_column = Some(nlc),
                Err(reason) => {
                    warnings.push(format!("Graph `{graph_id}` cleared multivariate label_column: {reason}"));
                    mv.label_column = None;
                }
            }
        }
        match &mut mv.color {
            ColorChannelMapping::Categorical { column, .. } | ColorChannelMapping::Continuous { column, .. } => {
                match plan.map_one(*column, "graph multivariate color column") {
                    Ok(nc) => *column = nc,
                    Err(reason) => {
                        warnings.push(format!("Graph `{graph_id}` reset multivariate color channel to constant: {reason}"));
                        mv.color = ColorChannelMapping::Constant(curvebook_palette::PaletteId::OkabeIto.swatches()[0]);
                    }
                }
            }
            ColorChannelMapping::Constant(_) => {}
        }
        if let SizeChannelMapping::Variable { column, .. } = &mut mv.size {
            match plan.map_one(*column, "graph multivariate size column") {
                Ok(nc) => *column = nc,
                Err(reason) => {
                    warnings.push(format!("Graph `{graph_id}` reset multivariate size channel to constant: {reason}"));
                    mv.size = SizeChannelMapping::Constant { radius_pt: 5.0 };
                }
            }
        }
    }
}

fn plan_conversion_impacts(doc: &ProjectDocument, old_ds: &DataSheet, target_kind: TargetArchetypeKind) -> ConversionImpact {
    let col_plan = ConversionColPlan::build(old_ds, target_kind);
    let old_cols = old_ds.archetype.column_count();
    let mut warnings = Vec::new();
    let mut incoming_links_dropped = 0usize;
    if old_ds.kind() != col_plan.target_table_kind {
        for region in &old_ds.linked_regions {
            if col_plan.map_incoming_region_range(region.target_range, old_cols).is_none() {
                incoming_links_dropped += 1;
                warnings.push(format!(
                    "Incoming live link `{}` (col {}..={}) on `{}` cannot be mapped after {:?}->{target_kind:?} conversion and will be detached",
                    region.id,
                    region.target_range.start.col,
                    region.target_range.end.col,
                    old_ds.id,
                    old_ds.kind()
                ));
            }
        }
    }

    let mut broken_result_reasons = Vec::new();
    for rs in &doc.result_sheets {
        let mut staged = rs.analysis.clone();
        if let Err(reason) = remap_analysis_spec_for_conversion(&mut staged, &old_ds.id, &col_plan, &mut warnings) {
            warnings.push(format!("Result sheet `{}` will be marked failed: {reason}", rs.id));
            broken_result_reasons.push((rs.id.clone(), reason));
        }
    }

    let mut broken_link_ids = Vec::new();
    for ds in &doc.data_sheets {
        if ds.id == old_ds.id {
            continue;
        }
        for region in &ds.linked_regions {
            if let LinkSource::InternalSheet { sheet_id, kind: InternalLinkKind::DataSheet, range, .. } = &region.spec.source
                && *sheet_id == old_ds.id
                && let Err(reason) = col_plan.map_source_range(*range)
            {
                warnings.push(format!("Outgoing live link `{}` on `{}` referencing `{}` will be detached: {reason}", region.id, ds.id, old_ds.id));
                broken_link_ids.push((ds.id.clone(), region.id.clone()));
            }
        }
    }

    for gs in &doc.graph_sheets {
        let mut staged_gs = gs.spec.clone();
        remap_graph_spec_for_conversion(&mut staged_gs, &old_ds.id, &col_plan, None, &mut warnings, &gs.id);
    }

    ConversionImpact {
        col_plan,
        warnings,
        incoming_links_lost: incoming_links_dropped > 0,
        incoming_links_dropped,
        broken_result_reasons,
        broken_link_ids,
    }
}

fn build_enriched_conversion_preview(base: &curvebook_transform::ConversionPreview, impact: &ConversionImpact) -> Value {
    let mut val = serde_json::to_value(base).unwrap_or_else(|_| json!({}));
    if let Some(obj) = val.as_object_mut() {
        if impact.incoming_links_lost {
            obj.insert(String::from("lossy"), Value::Bool(true));
        }
        let mut merged_warnings = base.warnings.clone();
        merged_warnings.extend(impact.warnings.iter().cloned());
        obj.insert(String::from("warnings"), Value::Array(merged_warnings.into_iter().map(Value::String).collect()));
    }
    val
}

fn preserve_mappable_incoming_links(old_ds: &DataSheet, converted: &mut DataSheet, plan: &ConversionColPlan) {
    if old_ds.kind() == plan.target_table_kind {
        return;
    }
    converted.linked_regions.clear();
    let old_cols = old_ds.archetype.column_count();
    for region in &old_ds.linked_regions {
        if let Some(mapped_range) = plan.map_incoming_region_range(region.target_range, old_cols) {
            let mut preserved = region.clone();
            preserved.target_range = mapped_range;
            preserved.spec.placement.anchor.col = mapped_range.start.col;
            converted.linked_regions.push(preserved);
        }
    }
}

fn apply_conversion_dependents(doc: &mut ProjectDocument, target_sheet_id: &SheetId, impact: &ConversionImpact) {
    let mut scratch_warnings = Vec::new();
    for rs in &mut doc.result_sheets {
        let _ = remap_analysis_spec_for_conversion(&mut rs.analysis, target_sheet_id, &impact.col_plan, &mut scratch_warnings);
        for outlier in &mut rs.eliminated_outliers {
            if outlier.sheet_id == *target_sheet_id
                && let Ok(new_c) = impact.col_plan.map_one(outlier.col, "outlier reference")
            {
                outlier.col = new_c;
            }
        }
    }
    for ds in &mut doc.data_sheets {
        if ds.id == *target_sheet_id {
            continue;
        }
        ds.linked_regions.retain(|region| {
            let is_broken = impact.broken_link_ids.iter().any(|(sid, rid)| sid == &ds.id && rid == &region.id);
            !is_broken
        });
        for region in &mut ds.linked_regions {
            if let LinkSource::InternalSheet { sheet_id, kind: InternalLinkKind::DataSheet, range, .. } = &mut region.spec.source
                && *sheet_id == *target_sheet_id
                && let Ok(mapped) = impact.col_plan.map_source_range(*range)
            {
                *range = mapped;
            }
        }
    }
    let converted_ds = doc.data_sheet(target_sheet_id).cloned();
    for gs in &mut doc.graph_sheets {
        let mut graph_warnings = Vec::new();
        remap_graph_spec_for_conversion(&mut gs.spec, target_sheet_id, &impact.col_plan, converted_ds.as_ref(), &mut graph_warnings, &gs.id);
    }
}

fn finalize_broken_dependents(doc: &mut ProjectDocument, impact: &ConversionImpact, recalc: &mut super::project::RecalcSummary) {
    for (rid, reason) in &impact.broken_result_reasons {
        if let Some(rs) = doc.result_sheet_mut(rid) {
            dag::clear_failed_result_sheet(rs, ErrorCode::Conflict, reason.clone());
        }
    }
    if !impact.broken_result_reasons.is_empty() {
        let mut all_ids: Vec<SheetId> = recalc.failed_result_sheets.iter().map(|f| f.sheet_id.clone()).collect();
        for (rid, _) in &impact.broken_result_reasons {
            if !all_ids.contains(rid) {
                all_ids.push(rid.clone());
            }
        }
        recalc.failed_result_sheets = dag::collect_failed_result_sheets_for_ids(doc, &all_ids);
    }
}

// ---------------------------------------------------------------------------
// 24. table.importPlateMap
// ---------------------------------------------------------------------------

fn resolve_new_data_sheet_id(doc: &ProjectDocument, raw_id: Option<&str>, prefix: &str) -> Result<SheetId, EngineError> {
    if let Some(raw) = raw_id {
        let id = parse_sheet_id(raw)?;
        if sheet_id_exists(doc, &id) {
            return Err(EngineError::with_code(ErrorCode::Conflict, format!("sheet `{id}` already exists")));
        }
        Ok(id)
    } else {
        next_unique_sheet_id(doc, prefix)
    }
}

fn upsert_info_constants(
    doc: &mut ProjectDocument,
    explicit_info_id: Option<&str>,
    fallback_info_id: Option<&SheetId>,
    create_if_missing: bool,
    default_title: &str,
    constants: &[(String, String)],
) -> Result<Option<SheetId>, EngineError> {
    let target_id = if let Some(raw) = explicit_info_id {
        let iid = parse_sheet_id(raw)?;
        if doc.info_sheet(&iid).is_some() {
            iid
        } else if sheet_id_exists(doc, &iid) {
            return Err(EngineError::with_code(ErrorCode::Conflict, format!("sheet `{iid}` already exists and is not an InfoSheet")));
        } else {
            doc.add_info_sheet(InfoSheet::new_preseeded(iid.clone(), default_title)?)?;
            iid
        }
    } else if let Some(fid) = fallback_info_id.filter(|id| doc.info_sheet(id).is_some()) {
        fid.clone()
    } else if create_if_missing {
        let iid = next_unique_sheet_id(doc, "info")?;
        doc.add_info_sheet(InfoSheet::new_preseeded(iid.clone(), default_title)?)?;
        iid
    } else {
        return Ok(None);
    };

    let info = require_info_sheet_mut(doc, &target_id)?;
    for (name, val) in constants {
        info.set_constant(name.clone(), ConstantValue::Text(val.clone()), None)?;
    }
    Ok(Some(target_id))
}

const fn default_weight_one() -> usize {
    1
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GroupSpecInput {
    Name(String),
    Weighted {
        label: String,
        #[serde(default = "default_weight_one")]
        weight: usize,
    },
}

#[derive(Debug, Deserialize)]
struct RandomizeGroupsParams {
    groups: Vec<GroupSpecInput>,
    #[serde(default)]
    subject_ids: Option<Vec<String>>,
    #[serde(default)]
    subject_count: Option<usize>,
    #[serde(default)]
    scheme: Option<String>,
    #[serde(default)]
    block_sizes: Option<Vec<usize>>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    sheet_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    info_sheet_id: Option<String>,
}

fn schema_randomize_groups() -> Value {
    let group_obj = obj_schema(
        vec![("label", non_empty_str_schema("Treatment arm label")), ("weight", int_schema("Allocation weight", Some(1), None))],
        &["label"],
        false,
    );
    let group_item = json!({
        "anyOf": [
            { "type": "string", "minLength": 1 },
            group_obj
        ]
    });
    obj_schema(
        vec![
            ("groups", array_schema("Treatment arm names or `{label, weight}` allocation objects", group_item, Some(1), None)),
            ("subject_ids", nullable_schema(array_schema("Explicit subject ID list", non_empty_str_schema("Subject ID"), Some(1), None))),
            ("subject_count", nullable_schema(int_schema("Number of subjects (auto-numbered 1..=N when `subject_ids` is omitted)", Some(1), None))),
            (
                "scheme",
                nullable_schema(enum_str_schema(
                    "Allocation scheme (`complete_balanced` or `permuted_block`, default `complete_balanced`)",
                    &["complete_balanced", "permuted_block"],
                )),
            ),
            (
                "block_sizes",
                nullable_schema(array_schema(
                    "Block sizes (each a positive multiple of sum of weights) for `permuted_block`",
                    int_schema("Block size", Some(1), None),
                    Some(1),
                    None,
                )),
            ),
            ("seed", nullable_schema(int_schema("Deterministic 64-bit PRNG seed (default 1)", Some(0), None))),
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional destination DataSheet ID"))),
            ("title", nullable_schema(str_schema("Optional destination sheet title (default `Randomization Schedule`)"))),
            ("info_sheet_id", nullable_schema(non_empty_str_schema("Optional target InfoSheet ID for ARRIVE randomization constants"))),
        ],
        &["groups"],
        false,
    )
}

fn run_randomize_groups(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: RandomizeGroupsParams = parse_params(&schema_randomize_groups(), raw)?;
    let groups: Vec<GroupAllocationWeight> = p
        .groups
        .into_iter()
        .map(|g| match g {
            GroupSpecInput::Name(label) => GroupAllocationWeight::new(label),
            GroupSpecInput::Weighted { label, weight } => GroupAllocationWeight::with_weight(label, weight),
        })
        .collect();
    let sum_weights: usize = groups.iter().fold(0usize, |acc, g| acc.saturating_add(g.weight));

    let norm_scheme =
        p.scheme.as_deref().unwrap_or(if p.block_sizes.as_ref().is_some_and(|v| !v.is_empty()) { "permuted_block" } else { "complete_balanced" });
    let scheme_key: String = norm_scheme.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let scheme = match scheme_key.as_str() {
        "completebalanced" | "complete" | "balanced" => AllocationScheme::CompleteBalanced,
        "permutedblock" | "block" => {
            let block_sizes = match p.block_sizes {
                Some(bs) => bs,
                None => vec![sum_weights.saturating_mul(2).max(2)],
            };
            AllocationScheme::PermutedBlock { block_sizes }
        }
        other => {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("unknown randomization scheme `{other}`; use `complete_balanced` or `permuted_block`"),
            ));
        }
    };

    let doc = &mut ctx.session.doc;
    let id = resolve_new_data_sheet_id(doc, p.sheet_id.as_deref(), "data")?;
    let title = p.title.unwrap_or_else(|| String::from("Randomization Schedule"));
    let seed = p.seed.unwrap_or(1);

    let config = match (p.subject_ids, p.subject_count) {
        (Some(ids), Some(count)) => {
            if ids.len() != count {
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("`subject_ids` length ({}) does not match `subject_count` ({count})", ids.len()),
                ));
            }
            SubjectRandomizationConfig { sheet_id: id.to_string(), title: title.clone(), subject_ids: ids, groups, scheme, seed }
        }
        (Some(ids), None) => SubjectRandomizationConfig { sheet_id: id.to_string(), title: title.clone(), subject_ids: ids, groups, scheme, seed },
        (None, Some(count)) => SubjectRandomizationConfig::from_subject_count(id.to_string(), title.clone(), count, groups, scheme, seed),
        (None, None) => {
            return Err(EngineError::with_code(ErrorCode::BadInput, "table.randomizeGroups requires `subject_ids` or `subject_count`"));
        }
    };

    let subject_count = config.subject_ids.len();
    let mut rand_out = randomize_subjects_to_groups(&config)?;
    validate_project_cell_budget_after_change(doc, None, rand_out.data_sheet.archetype.allocated_cell_slots())?;
    let info_id = upsert_info_constants(doc, p.info_sheet_id.as_deref(), None, true, &format!("{title} Info"), &rand_out.arrive_constants)?
        .ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "failed resolving InfoSheet for ARRIVE constants"))?;
    rand_out.data_sheet.info_sheet_id = Some(info_id.clone());
    doc.add_data_sheet(rand_out.data_sheet)?;
    link_data_sheet_to_info(doc, &info_id, &id);
    let recalc = recalculate_dirty_with_links(doc, &[id.clone(), info_id.clone()], false)?;

    let constants_map: serde_json::Map<String, Value> = rand_out.arrive_constants.into_iter().map(|(k, v)| (k, Value::String(v))).collect();
    Ok(CommandOutcome::mutated(
        "table.randomizeGroups",
        format!("randomized {subject_count} subjects into `{}`", id.as_str()),
        json!({
            "sheet_id": id.as_str(),
            "info_sheet_id": info_id.as_str(),
            "title": title,
            "subject_count": subject_count,
            "seed": seed,
            "arrive_constants": constants_map,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct RandomSampleRowsParams {
    source_sheet_id: String,
    sample_size: usize,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    stratify_column: Option<usize>,
    #[serde(default)]
    proportional_allocation: Option<bool>,
    #[serde(default)]
    sheet_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    info_sheet_id: Option<String>,
}

fn schema_random_sample_rows() -> Value {
    obj_schema(
        vec![
            ("source_sheet_id", non_empty_str_schema("Source DataSheet or analyzable ResultSheet ID")),
            ("sample_size", int_schema("Number of rows to draw without replacement", Some(1), None)),
            ("seed", nullable_schema(int_schema("Deterministic 64-bit PRNG seed (default 42)", Some(0), None))),
            ("stratify_column", nullable_schema(int_schema("Optional zero-based stratification column index", Some(0), None))),
            ("proportional_allocation", nullable_schema(bool_schema("Use proportional stratum allocation when stratified (default true)"))),
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional destination DataSheet ID"))),
            ("title", nullable_schema(str_schema("Optional destination sheet title"))),
            ("info_sheet_id", nullable_schema(non_empty_str_schema("Optional InfoSheet ID for recording sampling provenance constants"))),
        ],
        &["source_sheet_id", "sample_size"],
        false,
    )
}

fn run_random_sample_rows(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: RandomSampleRowsParams = parse_params(&schema_random_sample_rows(), raw)?;
    let src_id = parse_sheet_id(&p.source_sheet_id)?;
    let seed = p.seed.unwrap_or(42);
    let proportional = p.proportional_allocation.unwrap_or(true);

    let doc = &mut ctx.session.doc;
    let src = require_analyzable_data_sheet(doc, &src_id)?.clone();
    let src_info_id = src.info_sheet_id.clone();
    let population_size = src.archetype.row_count();
    let id = resolve_new_data_sheet_id(doc, p.sheet_id.as_deref(), "data")?;
    let title = p.title.unwrap_or_else(|| format!("{} (Sample n={})", src.title, p.sample_size));

    let mut sampled = random_sample_rows(
        &src,
        &RowSampleConfig {
            seed,
            sample_size: p.sample_size,
            stratify_column: p.stratify_column,
            proportional_allocation: proportional,
            output_id: Some(id.to_string()),
            output_title: Some(title.clone()),
        },
    )?;

    let out_rows = sampled.archetype.row_count();
    let out_cols = sampled.archetype.column_count();
    validate_project_cell_budget_after_change(doc, None, sampled.archetype.allocated_cell_slots())?;
    let mut provenance = vec![
        (String::from("sample.source_sheet_id"), src_id.to_string()),
        (String::from("sample.seed"), seed.to_string()),
        (String::from("sample.sample_size"), out_rows.to_string()),
        (String::from("sample.population_size"), population_size.to_string()),
    ];
    if let Some(col) = p.stratify_column {
        provenance.push((String::from("sample.stratify_column"), col.to_string()));
        provenance.push((String::from("sample.allocation"), String::from(if proportional { "proportional" } else { "equal" })));
    }

    let reusable_src_info_id = src_info_id.filter(|iid| {
        doc.info_sheet(iid).is_some_and(|info| !info.constants.iter().any(|c| c.name.starts_with("sample.") || c.name.starts_with("randomization.")))
    });
    let info_id = upsert_info_constants(doc, p.info_sheet_id.as_deref(), reusable_src_info_id.as_ref(), true, &format!("{title} Info"), &provenance)?;
    if let Some(ref iid) = info_id {
        sampled.info_sheet_id = Some(iid.clone());
    }
    doc.add_data_sheet(sampled)?;
    if let Some(ref iid) = info_id {
        link_data_sheet_to_info(doc, iid, &id);
    }
    let mut dirty_seeds = vec![id.clone()];
    if let Some(ref iid) = info_id {
        dirty_seeds.push(iid.clone());
    }
    let recalc = recalculate_dirty_with_links(doc, &dirty_seeds, false)?;

    Ok(CommandOutcome::mutated(
        "table.randomSampleRows",
        format!("sampled {out_rows} row(s) from `{}` into `{}`", src_id.as_str(), id.as_str()),
        json!({
            "sheet_id": id.as_str(),
            "source_sheet_id": src_id.as_str(),
            "info_sheet_id": info_id.as_ref().map(SheetId::as_str),
            "title": title,
            "row_count": out_rows,
            "col_count": out_cols,
            "seed": seed,
            "sample_size": p.sample_size,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct SimulateDistributionsParams {
    columns: Vec<Value>,
    n_rows: usize,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    use_multiple_variables: Option<bool>,
    #[serde(default)]
    sheet_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

fn find_dist_f64(sources: &[&serde_json::Map<String, Value>], field_name: &str, idx: usize) -> Result<f64, EngineError> {
    for src in sources {
        if let Some(val) = src.get(field_name) {
            return val
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}].{field_name} must be a finite number")));
        }
    }
    Err(EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}] missing numeric parameter `{field_name}`")))
}

fn find_dist_u64(sources: &[&serde_json::Map<String, Value>], field_name: &str, idx: usize) -> Result<u64, EngineError> {
    for src in sources {
        if let Some(val) = src.get(field_name) {
            return val
                .as_u64()
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}].{field_name} must be a non-negative integer")));
        }
    }
    Err(EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}] missing integer parameter `{field_name}`")))
}

fn parse_simulated_column_spec(item: &Value, idx: usize) -> Result<SimulatedColumnSpec, EngineError> {
    if let Ok(direct) = serde_json::from_value::<SimulatedColumnSpec>(item.clone()) {
        return Ok(direct);
    }
    let obj = item.as_object().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}] must be an object")))?;
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}] missing non-empty `name`")))?
        .to_owned();

    let dist_val =
        obj.get("distribution").ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}] missing `distribution`")))?;

    if let Ok(dist) = serde_json::from_value::<SimulatedDistribution>(dist_val.clone()) {
        return Ok(SimulatedColumnSpec { name, distribution: dist });
    }

    let mut sources: Vec<&serde_json::Map<String, Value>> = Vec::new();
    if let Some(params_obj) = obj.get("params").and_then(Value::as_object) {
        sources.push(params_obj);
    }
    let kind_raw = if let Some(s) = dist_val.as_str() {
        s
    } else if let Some(dist_obj) = dist_val.as_object() {
        sources.push(dist_obj);
        if let Some(inner_params) = dist_obj.get("params").and_then(Value::as_object) {
            sources.push(inner_params);
        }
        dist_obj
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}].distribution object must specify `kind`")))?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}].distribution must be a distribution name or object")));
    };
    sources.push(obj);

    let kind_key: String = kind_raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let distribution = match kind_key.as_str() {
        "uniform" => SimulatedDistribution::Uniform { min: find_dist_f64(&sources, "min", idx)?, max: find_dist_f64(&sources, "max", idx)? },
        "gaussian" | "normal" => {
            SimulatedDistribution::Gaussian { mean: find_dist_f64(&sources, "mean", idx)?, sd: find_dist_f64(&sources, "sd", idx)? }
        }
        "lognormal" => {
            SimulatedDistribution::LogNormal { mean_log: find_dist_f64(&sources, "mean_log", idx)?, sd_log: find_dist_f64(&sources, "sd_log", idx)? }
        }
        "binomial" => SimulatedDistribution::Binomial { trials: find_dist_u64(&sources, "trials", idx)?, p: find_dist_f64(&sources, "p", idx)? },
        "poisson" => SimulatedDistribution::Poisson { lambda: find_dist_f64(&sources, "lambda", idx)? },
        "exponential" | "exp" => SimulatedDistribution::Exponential { rate: find_dist_f64(&sources, "rate", idx)? },
        other => {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                format!("columns[{idx}] unknown distribution `{other}`; use uniform, gaussian, log_normal, binomial, poisson, or exponential"),
            ));
        }
    };

    Ok(SimulatedColumnSpec { name, distribution })
}

fn schema_simulate_distributions() -> Value {
    let col_schema = json!({
        "type": "object",
        "description": "Column distribution specification `{ name, distribution, ... }` supporting uniform (`min`, `max`), gaussian (`mean`, `sd`), log_normal (`mean_log`, `sd_log`), binomial (`trials`, `p`), poisson (`lambda`), exponential (`rate`)",
        "properties": {
            "name": { "type": "string", "minLength": 1 },
            "distribution": {
                "anyOf": [
                    { "type": "string", "enum": ["uniform", "gaussian", "log_normal", "binomial", "poisson", "exponential"] },
                    { "type": "object" }
                ]
            },
            "params": { "type": "object" },
            "min": num_schema("Minimum bound for uniform distribution", None, None),
            "max": num_schema("Maximum bound for uniform distribution", None, None),
            "mean": num_schema("Mean for gaussian distribution", None, None),
            "sd": num_schema("Standard deviation for gaussian distribution", None, None),
            "mean_log": num_schema("Log-space mean for log_normal distribution", None, None),
            "sd_log": num_schema("Log-space standard deviation for log_normal distribution", None, None),
            "trials": int_schema("Number of trials for binomial distribution", Some(0), None),
            "p": num_schema("Success probability for binomial distribution", None, None),
            "lambda": num_schema("Rate/mean parameter for poisson distribution", None, None),
            "rate": num_schema("Rate parameter for exponential distribution", None, None)
        },
        "required": ["name", "distribution"],
        "additionalProperties": false
    });
    obj_schema(
        vec![
            ("columns", array_schema("Simulated column distribution specifications", col_schema, Some(1), None)),
            ("n_rows", int_schema("Number of rows to simulate per column", Some(1), None)),
            ("seed", nullable_schema(int_schema("Deterministic 64-bit PRNG seed (default 1)", Some(0), None))),
            ("use_multiple_variables", nullable_schema(bool_schema("Create MultipleVariables table instead of Column table (default false)"))),
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional destination DataSheet ID"))),
            ("title", nullable_schema(str_schema("Optional destination sheet title (default `Simulated Distributions`)"))),
        ],
        &["columns", "n_rows"],
        false,
    )
}

fn run_simulate_distributions(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SimulateDistributionsParams = parse_params(&schema_simulate_distributions(), raw)?;
    let columns: Vec<SimulatedColumnSpec> =
        p.columns.iter().enumerate().map(|(i, item)| parse_simulated_column_spec(item, i)).collect::<Result<_, _>>()?;
    let seed = p.seed.unwrap_or(1);
    let use_mv = p.use_multiple_variables.unwrap_or(false);
    let title = p.title.unwrap_or_else(|| String::from("Simulated Distributions"));

    let doc = &mut ctx.session.doc;
    let id = resolve_new_data_sheet_id(doc, p.sheet_id.as_deref(), "data")?;
    let col_count = columns.len();
    let cfg = DistributionSimulationConfig {
        sheet_id: id.to_string(),
        title: title.clone(),
        n_rows: p.n_rows,
        seed,
        columns,
        use_multiple_variables: use_mv,
    };
    let sheet = simulate_distribution_columns(&cfg)?;
    let kind = sheet.archetype.kind();
    validate_project_cell_budget_after_change(doc, None, sheet.archetype.allocated_cell_slots())?;
    doc.add_data_sheet(sheet)?;
    let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;

    Ok(CommandOutcome::mutated(
        "table.simulateDistributions",
        format!("simulated {col_count} column(s) x {} rows into `{}`", p.n_rows, id.as_str()),
        json!({
            "sheet_id": id.as_str(),
            "title": title,
            "kind": kind,
            "n_rows": p.n_rows,
            "n_cols": col_count,
            "seed": seed,
            "use_multiple_variables": use_mv,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::sync::atomic::AtomicBool;

    use curvebook_transform::{PlateGeometry, PlateMapTemplate, PlateWellRole};

    use super::*;
    use crate::cmd::{Caller, build_registry};

    fn seeded_session() -> Session {
        let mut session = Session::new();
        session.execute("sheet.createData", json!({ "sheet_id": "xy-1", "archetype": "xy", "cols": 2, "rows": 4 })).expect("create xy-1");
        session.execute("sheet.createData", json!({ "sheet_id": "col-1", "archetype": "column", "cols": 2, "rows": 4 })).expect("create col-1");
        session
            .execute("sheet.createData", json!({ "sheet_id": "nest-1", "archetype": "nested", "cols": 2, "rows": 3, "replicates": 3 }))
            .expect("create nest-1");
        session
            .execute(
                "sheet.createData",
                json!({
                    "sheet_id": "mv-1",
                    "archetype": "multiple_variables",
                    "cols": 2,
                    "rows": 3,
                    "variables": [{ "title": "Arm", "role": "Categorical" }, { "title": "Dose", "role": "Continuous" }]
                }),
            )
            .expect("create mv-1");
        session
    }

    #[test]
    fn specs_cover_all_28_table_commands_uniquely_and_register() {
        let listed = specs();
        assert_eq!(listed.len(), 28);
        let ids: BTreeSet<&'static str> = listed.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), 28);
        for spec in listed {
            assert_eq!(spec.group, CommandGroup::Table);
            assert!(!spec.label.is_empty());
            assert!(!spec.menu_path.is_empty());
            assert!(!spec.summary.is_empty());
            assert!(!spec.beginner.is_empty());
            let schema = (spec.input_schema)();
            assert_eq!(schema.get("type").and_then(Value::as_str), Some("object"), "{}", spec.id);
        }

        let reg = build_registry().expect("registry build succeeds");
        for id in ids {
            assert!(reg.get(id).is_some(), "missing {id} in registry");
        }
    }

    #[test]
    fn enabled_predicates_gate_empty_and_typed_sessions() {
        let empty = Session::new();
        let reg = build_registry().expect("registry");
        assert!(!(reg.get("table.setCell").expect("spec").enabled)(&empty));
        assert!(!(reg.get("table.exportCsv").expect("spec").enabled)(&empty));
        assert!((reg.get("table.importCsv").expect("spec").enabled)(&empty));
        assert!((reg.get("table.exclusionAuditReport").expect("spec").enabled)(&empty));

        let rich = seeded_session();
        assert!((reg.get("table.setCell").expect("spec").enabled)(&rich));
        assert!((reg.get("table.setSubcolumnTitles").expect("spec").enabled)(&rich));
        assert!((reg.get("table.setVariableRole").expect("spec").enabled)(&rich));
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
        assert!(err2.remediation.is_some());
        assert!(err2.to_string().contains("reason_tag"));
    }

    #[test]
    fn frozen_sheet_returns_frozen_error_and_preview_convert_is_read_only() {
        let mut s = seeded_session();
        s.execute("sheet.freeze", json!({ "sheet_id": "xy-1" })).expect("freeze");
        let err = s.execute("table.setCell", json!({ "sheet_id": "xy-1", "row": 0, "col": 0, "value": 10.0 })).expect_err("frozen");
        assert_eq!(err.code, ErrorCode::FrozenSheet);

        let depth_before = s.undo_depth();
        let preview = s
            .execute("table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "Column", "allow_lossy": true, "preview_only": true }))
            .expect("preview on frozen");
        assert!(!preview.mutated);
        assert_eq!(s.undo_depth(), depth_before);

        let err_conv = s
            .execute("table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "Column", "allow_lossy": true }))
            .expect_err("mutating convert on frozen");
        assert_eq!(err_conv.code, ErrorCode::FrozenSheet);
    }

    #[test]
    fn cell_title_exclusion_paste_export_and_multivar_round_trip() {
        let mut s = seeded_session();
        let cell_out =
            s.execute("table.setCell", json!({ "sheet_id": "xy-1", "coord": { "row": 0, "col": 0 }, "value": 12.5 })).expect("setCell canonical");
        assert!(cell_out.warnings.is_empty(), "canonical params should emit zero warnings");
        s.execute("table.setCells", json!({ "sheet_id": "col-1", "rows": [[1.0, 2.0], [3.0, 4.0]] })).expect("setCells rows");
        s.execute("table.setXCells", json!({ "sheet_id": "xy-1", "values": [1.0, 2.0, 3.0] })).expect("setXCells");
        s.execute("table.setRowTitles", json!({ "sheet_id": "col-1", "titles": ["R1", null] })).expect("setRowTitles");
        let gt_out = s.execute("table.setGroupTitles", json!({ "sheet_id": "col-1", "titles": ["G1", "G2"] })).expect("setGroupTitles");
        assert_eq!(gt_out.command_id, "table.setGroupTitles");
        assert!(gt_out.warnings.is_empty());
        let st_out = s.execute("table.setTitles", json!({ "sheet_id": "xy-1", "x_title": "Dose", "titles": ["Resp"] })).expect("setTitles");
        assert_eq!(st_out.command_id, "table.setTitles");
        assert!(st_out.warnings.is_empty());
        s.execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-1", "col": 0, "titles": ["A", "B", "C"] })).expect("setSubcolumnTitles");

        s.execute("table.excludeCells", json!({ "sheet_id": "col-1", "row": 0, "col": 0 })).expect("exclude");
        s.execute("table.excludeCellsWithReason", json!({ "sheet_id": "col-1", "reason_tag": "outlier", "coords": [[1, 0]] }))
            .expect("excludeWithReason");
        let audit = s.execute("table.exclusionAuditReport", json!({ "sheet_id": "col-1" })).expect("audit");
        assert!(!audit.mutated);
        assert_eq!(audit.payload.get("total_excluded").and_then(Value::as_u64), Some(2));
        assert_eq!(audit.payload.get("annotated_count").and_then(Value::as_u64), Some(1));

        s.execute("table.clearCells", json!({ "sheet_id": "col-1", "row_start": 0, "row_end": 0, "col_start": 1, "col_end": 1 })).expect("clear");
        s.execute("table.paste", json!({ "sheet_id": "col-1", "text": "5\t6\n7\t8" })).expect("paste");
        s.execute("table.pasteLink", json!({ "target_sheet_id": "xy-1", "source_sheet_id": "col-1" })).expect("pasteLink");

        let tmp_dir = std::env::temp_dir().join(format!("cb-w9b-table-test-{}", std::process::id()));
        let csv_path = tmp_dir.join("out.csv");
        let exp = s.execute("table.exportCsv", json!({ "sheet_id": "col-1", "path": csv_path.to_string_lossy() })).expect("exportCsv");
        assert!(!exp.mutated);
        assert!(csv_path.exists());
        let csv_text = exp.payload.get("csv").and_then(Value::as_str).expect("csv payload");
        s.execute("table.importCsv", json!({ "csv": csv_text })).expect("importCsv");
        let _ = std::fs::remove_dir_all(&tmp_dir);

        s.execute("table.setVariableRole", json!({ "sheet_id": "mv-1", "col": 0, "role": "categorical" })).expect("setVariableRole");
        s.execute(
            "table.setCells",
            json!({ "sheet_id": "mv-1", "edits": [{ "row": 0, "col": 0, "value": "A" }, { "row": 0, "col": 1, "value": 10.0 }] }),
        )
        .expect("mv cells");
        s.execute("table.recodeLevels", json!({ "sheet_id": "mv-1", "col": 0, "mappings": { "A": "Alpha" } })).expect("recodeLevels");
        s.execute("table.setCategoricalOrder", json!({ "sheet_id": "mv-1", "col": 0, "order": ["Alpha"] })).expect("setCategoricalOrder");
        s.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "title": "Double", "formula": "= [Dose] * 2" }))
            .expect("setCalculatedVariable");
        s.execute("table.simulateXy", json!({ "row_count": 6, "replicates": 2 })).expect("simulateXy");

        let mut roles = vec![PlateWellRole::Blank, PlateWellRole::PositiveControl, PlateWellRole::Excluded];
        roles.resize(96, PlateWellRole::Treatment { group_name: String::from("Drug"), dilution_x: 1.0, replicate: 1 });
        let tpl = PlateMapTemplate::from_wells("P96", PlateGeometry::Wells96, "Conc", roles).expect("tpl");
        s.execute("table.importPlateMap", json!({ "values": vec![1.0_f64; 96], "template": tpl })).expect("importPlateMap");
        s.execute(
            "table.importTidyLong",
            json!({
                "csv": "grp,dose,val\nCtrl,0,10\nTx,1,20",
                "mapping": { "group_col": "grp", "x_col": "dose", "value_col": "val" }
            }),
        )
        .expect("importTidyLong");
    }

    #[test]
    fn nested_subcolumn_titles_preserve_biological_unit_width_and_checkpoint_roundtrip() -> Result<(), String> {
        let mut s = Session::new();
        // rows (4) > biological_units (2)
        s.execute("sheet.createData", json!({ "sheet_id": "nest-tall", "kind": "nested", "cols": 2, "rows": 4, "replicates": 2 }))
            .map_err(|e| e.to_string())?;
        s.execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-tall", "col": 0, "titles": ["Unit 1", "Unit 2"] }))
            .map_err(|e| e.to_string())?;
        s.execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-tall", "subcol": 1, "title": "Shared Bio 2" })).map_err(|e| e.to_string())?;

        // Out-of-bounds subcolumn must be rejected against biological-unit width (2), not row count (4).
        let oob = s.execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-tall", "col": 0, "subcol": 2, "title": "Bad" }));
        assert!(oob.as_ref().is_err_and(|e| e.code == ErrorCode::BadInput), "expected BadInput for subcol >= width, got {oob:?}");

        // Oversized title must be rejected by validate_title.
        let oversized = "T".repeat(curvebook_doc::MAX_TITLE_BYTES + 1);
        let too_long = s.execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-tall", "col": 0, "subcol": 0, "title": oversized }));
        assert!(too_long.is_err(), "expected oversized subcolumn title to fail validation");

        let ds = s.document().data_sheet(&SheetId::new("nest-tall")).ok_or_else(|| String::from("missing nest-tall"))?;
        let TableArchetype::Nested(nt) = &ds.archetype else {
            return Err(String::from("expected Nested archetype"));
        };
        nt.validate().map_err(|e| e.to_string())?;
        assert_eq!(nt.groups[0].subcolumn_titles, vec!["Unit 1", "Shared Bio 2"]);
        assert_eq!(nt.groups[1].subcolumn_titles.len(), 2);
        assert_eq!(nt.groups[1].subcolumn_titles[1], "Shared Bio 2");

        // Checkpoint save + mutate + restore exercises NestedTable::deserialize -> validate().
        s.execute("project.createCheckpoint", json!({ "name": "cp-nested" })).map_err(|e| e.to_string())?;
        s.execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-tall", "col": 0, "subcol": 0, "title": "Temp" }))
            .map_err(|e| e.to_string())?;
        s.execute("project.restoreCheckpoint", json!({ "name": "cp-nested" })).map_err(|e| e.to_string())?;
        let ds_restored = s.document().data_sheet(&SheetId::new("nest-tall")).ok_or_else(|| String::from("missing restored nest-tall"))?;
        let TableArchetype::Nested(nt_restored) = &ds_restored.archetype else {
            return Err(String::from("expected Nested archetype after restore"));
        };
        assert_eq!(nt_restored.groups[0].subcolumn_titles, vec!["Unit 1", "Shared Bio 2"]);

        // rows (2) < biological_units (4)
        s.execute("sheet.createData", json!({ "sheet_id": "nest-wide", "archetype": "nested", "cols": 1, "rows": 2, "replicates": 4 }))
            .map_err(|e| e.to_string())?;
        s.execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-wide", "col": 0, "subcol": 3, "title": "Unit 4" }))
            .map_err(|e| e.to_string())?;
        let ds_wide = s.document().data_sheet(&SheetId::new("nest-wide")).ok_or_else(|| String::from("missing nest-wide"))?;
        let TableArchetype::Nested(nt_wide) = &ds_wide.archetype else {
            return Err(String::from("expected Nested archetype for nest-wide"));
        };
        nt_wide.validate().map_err(|e| e.to_string())?;
        assert_eq!(nt_wide.groups[0].subcolumn_titles.len(), 4);
        assert_eq!(nt_wide.groups[0].subcolumn_titles[3], "Unit 4");
        Ok(())
    }

    #[test]
    fn mv_set_titles_rewrites_calculated_formulas_reevaluates_and_undoes_atomically() -> Result<(), String> {
        let mut s = Session::new();
        s.execute("sheet.createData", json!({ "sheet_id": "mv-1", "archetype": "multiple_variables", "cols": 3, "rows": 3 }))
            .map_err(|e| e.to_string())?;
        s.execute("table.setTitles", json!({ "sheet_id": "mv-1", "titles": ["Dose", "Weight", "Ratio"] })).map_err(|e| e.to_string())?;
        s.execute(
            "table.setCells",
            json!({
                "sheet_id": "mv-1",
                "edits": [
                    { "row": 0, "col": 0, "value": 10.0 },
                    { "row": 1, "col": 0, "value": 20.0 },
                    { "row": 2, "col": 0, "value": 30.0 },
                    { "row": 0, "col": 1, "value": 2.0 },
                    { "row": 1, "col": 1, "value": 4.0 },
                    { "row": 2, "col": 1, "value": 5.0 },
                ]
            }),
        )
        .map_err(|e| e.to_string())?;
        s.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-1", "col": 2, "formula": "[Dose] / [Weight]" }))
            .map_err(|e| e.to_string())?;

        // Rename `Dose` -> `Dose (mg/kg)`: formula must rewrite & evaluate cleanly in the same command.
        let undo_before = s.undo_depth();
        s.execute("table.setTitles", json!({ "sheet_id": "mv-1", "col": 0, "title": "Dose (mg/kg)" })).map_err(|e| e.to_string())?;
        assert_eq!(s.undo_depth(), undo_before + 1);

        let ds = s.document().data_sheet(&SheetId::new("mv-1")).ok_or_else(|| String::from("missing mv-1"))?;
        let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
            return Err(String::from("expected MultipleVariables"));
        };
        assert_eq!(mv.variables[0].title, "Dose (mg/kg)");
        assert_eq!(mv.variables[2].source, VariableSource::Calculated { formula: String::from("[Dose (mg/kg)] / [Weight]") });
        assert_eq!(mv.variables[2].cells[0].active_number(), Some(5.0));
        assert_eq!(mv.variables[2].cells[1].active_number(), Some(5.0));
        assert_eq!(mv.variables[2].cells[2].active_number(), Some(6.0));

        // Update a Dose cell afterwards to prove ongoing recalculation works with the rewritten formula.
        s.execute("table.setCell", json!({ "sheet_id": "mv-1", "row": 0, "col": 0, "value": 14.0 })).map_err(|e| e.to_string())?;
        let ds2 = s.document().data_sheet(&SheetId::new("mv-1")).ok_or_else(|| String::from("missing mv-1"))?;
        let TableArchetype::MultipleVariables(mv2) = &ds2.archetype else {
            return Err(String::from("expected MultipleVariables"));
        };
        assert_eq!(mv2.variables[2].cells[0].active_number(), Some(7.0));

        // Undo cell edit, then undo rename -> restores BOTH original title `Dose` and `[Dose] / [Weight]`.
        s.execute("edit.undo", json!({})).map_err(|e| e.to_string())?;
        s.execute("edit.undo", json!({})).map_err(|e| e.to_string())?;
        let ds_undo = s.document().data_sheet(&SheetId::new("mv-1")).ok_or_else(|| String::from("missing mv-1"))?;
        let TableArchetype::MultipleVariables(mv_undo) = &ds_undo.archetype else {
            return Err(String::from("expected MultipleVariables"));
        };
        assert_eq!(mv_undo.variables[0].title, "Dose");
        assert_eq!(mv_undo.variables[2].source, VariableSource::Calculated { formula: String::from("[Dose] / [Weight]") });
        assert_eq!(mv_undo.variables[2].cells[0].active_number(), Some(5.0));
        Ok(())
    }

    #[test]
    fn clear_cells_unlink_region_id_detaches_only_target_and_validates_before_mutating() -> Result<(), String> {
        let mut s = Session::new();
        s.execute("sheet.createData", json!({ "sheet_id": "src-a", "kind": "column", "cols": 1, "rows": 2 })).map_err(|e| e.to_string())?;
        s.execute("sheet.createData", json!({ "sheet_id": "src-b", "kind": "column", "cols": 1, "rows": 2 })).map_err(|e| e.to_string())?;
        s.execute(
            "table.setCells",
            json!({ "sheet_id": "src-a", "edits": [{ "row": 0, "col": 0, "value": 10.0 }, { "row": 1, "col": 0, "value": 11.0 }] }),
        )
        .map_err(|e| e.to_string())?;
        s.execute(
            "table.setCells",
            json!({ "sheet_id": "src-b", "edits": [{ "row": 0, "col": 0, "value": 20.0 }, { "row": 1, "col": 0, "value": 21.0 }] }),
        )
        .map_err(|e| e.to_string())?;
        s.execute("sheet.createData", json!({ "sheet_id": "dst", "kind": "column", "cols": 2, "rows": 2 })).map_err(|e| e.to_string())?;
        s.execute("table.pasteLink", json!({ "target_sheet_id": "dst", "source_sheet_id": "src-a", "anchor": { "row": 0, "col": 0 } }))
            .map_err(|e| e.to_string())?;
        s.execute("table.pasteLink", json!({ "target_sheet_id": "dst", "source_sheet_id": "src-b", "anchor": { "row": 0, "col": 1 } }))
            .map_err(|e| e.to_string())?;

        let region_ids: Vec<String> = {
            let ds = s.document().data_sheet(&SheetId::new("dst")).ok_or_else(|| String::from("missing dst"))?;
            ds.linked_regions.iter().map(|r| r.id.clone()).collect()
        };
        assert_eq!(region_ids.len(), 2);
        let first_id = region_ids[0].clone();
        let second_id = region_ids[1].clone();

        // Unknown region_id must fail with NotFound before clearing any cells or detaching existing regions.
        let missing_err = s
            .execute("table.clearCells", json!({ "sheet_id": "dst", "row": 0, "col": 0, "unlink": true, "region_id": "missing-region" }))
            .expect_err("unknown region_id must fail");
        assert_eq!(missing_err.code, ErrorCode::NotFound);
        let dst_after_miss = s.document().data_sheet(&SheetId::new("dst")).ok_or_else(|| String::from("missing dst"))?;
        assert_eq!(dst_after_miss.linked_regions.len(), 2);
        assert_eq!(dst_after_miss.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(10.0));

        // Detaching first_id via cells: [] preserves the second linked region and existing values.
        s.execute("table.clearCells", json!({ "sheet_id": "dst", "cells": [], "unlink": true, "region_id": first_id })).map_err(|e| e.to_string())?;
        let dst_after_detach = s.document().data_sheet(&SheetId::new("dst")).ok_or_else(|| String::from("missing dst"))?;
        assert_eq!(dst_after_detach.linked_regions.len(), 1);
        assert_eq!(dst_after_detach.linked_regions[0].id, second_id);

        // Detach all remaining regions when region_id is omitted.
        s.execute("table.clearCells", json!({ "sheet_id": "dst", "cells": [], "unlink": true })).map_err(|e| e.to_string())?;
        let dst_after_clear_all = s.document().data_sheet(&SheetId::new("dst")).ok_or_else(|| String::from("missing dst"))?;
        assert!(dst_after_clear_all.linked_regions.is_empty());
        Ok(())
    }
}
