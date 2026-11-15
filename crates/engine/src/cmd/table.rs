//! Data-table cell, title, exclusion, paste/link, CSV/project import, and archetype commands (Wave 9B).

use std::collections::BTreeMap;
use std::path::Path;

use lumencraft_doc::{
    BaselineOperation, Cell, CellCoord, CellValue, ConstantValue, DataSetColumn, DataSheet, GraphSheet, ImportFilterSpec, ImportLinkSpec,
    ImportPlacementSpec, InfoSheet, InternalLinkKind, LevelOrderKind, LevelOrderSpec, LinkSource, MultipleVariablesTable, NestedTable, PlacementMode,
    ProjectDocument, ResultSheet, SheetCellRange, SheetId, SheetSection, SimulationNoise, SubcolumnMode, TableArchetype, TableKind, VariableColumn,
    VariableRole, VariableSource, validate_rows, validate_title,
};
use lumencraft_format::{
    CsvOptions, export_datasheet_to_csv, export_result_sheet_tidy_csv, import_csv_to_datasheet, import_prism_bytes, import_pzfx_bytes,
    import_pzfx_str, parse_delimited_records,
};
use lumencraft_transform::{
    AllocationScheme, DistributionSimulationConfig, GroupAllocationWeight, ImportPipelineOptions, ImportPipelineReport, ImportSourcePayload,
    PlateGeometry, PlateMapTemplate, PlateWellRole, RowSampleConfig, SimulatedColumnSpec, SimulatedDistribution, SubjectRandomizationConfig,
    TargetArchetypeKind, TidyColumnMapping, apply_import_link_pipeline, convert_datasheet_archetype, evaluate_data_sheet_calculated_variables,
    map_plate_to_datasheet, pivot_tidy_long_to_datasheet, preview_datasheet_conversion, random_sample_rows, randomize_subjects_to_groups,
    simulate_distribution_columns, simulate_xy_sheet, validate_no_formula_cycle,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::helpers::{
    column_subcolumn_count, decode_base64, ensure_sheet_not_frozen, locate_target_and_info_indices, next_unique_sheet_id,
    next_unique_sheet_id_excluding, parse_sheet_id, require_analyzable_data_sheet, require_data_sheet, require_info_sheet_mut,
    require_unfrozen_data_sheet_mut, select_result_tab, sheet_id_exists, x_subcolumn_count,
};
use super::params::{
    any_json_schema, array_schema, bool_schema, enum_str_schema, int_schema, non_empty_str_schema, nullable_schema, num_schema, obj_schema,
    obj_schema_with_aliases, parse_params, str_schema,
};
use super::{CommandGroup, CommandSpec, Ctx, Effects};
use crate::{CommandOutcome, EngineError, ErrorCode, Session, dag};

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
// Shared Reactive Link Refresh & Title Propagation
// ---------------------------------------------------------------------------

struct RecalcSummary {
    recalculated_sheets: usize,
    refreshed_links: usize,
}

enum OwnedSourcePayload {
    Data(Box<DataSheet>),
    Result(Box<ResultSheet>),
    Delimited { text: String, delimiter: char, decimal_separator: char },
}

fn recalculate_with_links(doc: &mut ProjectDocument, include_external_links: bool) -> Result<RecalcSummary, EngineError> {
    let mut refreshed = refresh_linked_regions(doc, include_external_links)?;
    let mut recalculated = dag::recalculate_project(doc)?;
    let max_hops = doc.data_sheets.len().saturating_add(1);
    for _ in 0..max_hops {
        let hop_refreshed = refresh_linked_regions(doc, include_external_links)?;
        if hop_refreshed == 0 {
            break;
        }
        refreshed = refreshed.saturating_add(hop_refreshed);
        recalculated = recalculated.saturating_add(dag::recalculate_project(doc)?);
    }
    Ok(RecalcSummary { recalculated_sheets: recalculated, refreshed_links: refreshed })
}

fn refresh_linked_regions(doc: &mut ProjectDocument, include_external_links: bool) -> Result<usize, EngineError> {
    let order = dag::topological_order(doc)?;
    let mut updated_count = 0usize;

    for target_id in order {
        let Some(target_idx) = doc.data_sheets.iter().position(|s| s.id == target_id) else {
            continue;
        };
        if doc.data_sheets.get(target_idx).is_none_or(|ds| ds.frozen) {
            continue;
        }
        let regions = doc.data_sheets.get(target_idx).map(|ds| ds.linked_regions.clone()).unwrap_or_default();
        if regions.is_empty() {
            continue;
        }

        let mut target_changed = false;
        for region in regions {
            let Some(source_payload) = resolve_link_source_payload(doc, &region.spec.source, include_external_links)? else {
                continue;
            };
            let Some(target_ds) = doc.data_sheets.get_mut(target_idx) else {
                continue;
            };
            if apply_refresh_to_target(target_ds, &region.spec, &source_payload)? {
                target_changed = true;
                updated_count = updated_count.saturating_add(1);
            }
        }
        if target_changed {
            let info_snapshot = doc.data_sheets.get(target_idx).and_then(|ds| ds.info_sheet_id.as_ref()).and_then(|iid| doc.info_sheet(iid)).cloned();
            if let Some(target_ds) = doc.data_sheets.get_mut(target_idx) {
                evaluate_data_sheet_calculated_variables(target_ds, info_snapshot.as_ref())?;
            }
        }
    }
    Ok(updated_count)
}

fn resolve_link_source_payload(
    doc: &ProjectDocument,
    source: &LinkSource,
    include_external_links: bool,
) -> Result<Option<OwnedSourcePayload>, EngineError> {
    match source {
        LinkSource::Clipboard => Ok(None),
        LinkSource::InternalSheet { sheet_id, kind, .. } => match kind {
            InternalLinkKind::DataSheet => Ok(doc.data_sheet(sheet_id).cloned().map(Box::new).map(OwnedSourcePayload::Data)),
            InternalLinkKind::AnalyzableResultSheet | InternalLinkKind::TerminalResultSheet => {
                Ok(doc.result_sheet(sheet_id).cloned().map(Box::new).map(OwnedSourcePayload::Result))
            }
        },
        LinkSource::ExternalDelimitedFile { path, delimiter, decimal_separator } => {
            if !include_external_links {
                return Ok(None);
            }
            let text = read_utf8_file(path, "linked file")?;
            Ok(Some(OwnedSourcePayload::Delimited { text, delimiter: *delimiter, decimal_separator: *decimal_separator }))
        }
    }
}

fn apply_refresh_to_target(target_ds: &mut DataSheet, spec: &ImportLinkSpec, payload: &OwnedSourcePayload) -> Result<bool, EngineError> {
    let before = target_ds.clone();
    let existing_links = target_ds.linked_regions.len();
    let mut refresh_spec = spec.clone();
    refresh_spec.placement.mode = PlacementMode::Overwrite;

    let opts = ImportPipelineOptions::default();
    run_import_pipeline(target_ds, &refresh_spec, payload, &opts, None)?;
    propagate_internal_source_titles(target_ds, &refresh_spec, payload);
    target_ds.linked_regions.truncate(existing_links);
    Ok(target_ds != &before)
}

fn propagate_internal_source_titles(target_ds: &mut DataSheet, spec: &ImportLinkSpec, payload: &OwnedSourcePayload) {
    if !spec.placement.propagate_title_header {
        return;
    }
    let LinkSource::InternalSheet { range, .. } = &spec.source else {
        return;
    };
    let OwnedSourcePayload::Data(src_ds) = payload else {
        return;
    };
    let base_col = spec.placement.anchor.col;
    if spec.placement.transpose {
        for (offset, row) in (range.start.row..=range.end.row).enumerate() {
            if let Some(title) = src_row_title(&src_ds.archetype, row)
                && !title.is_empty()
                && base_col.saturating_add(offset) < target_ds.archetype.column_count()
            {
                let _ = set_group_title(&mut target_ds.archetype, base_col.saturating_add(offset), title);
            }
        }
    } else {
        for (offset, col) in (range.start.col..=range.end.col).enumerate() {
            if let Some(title) = src_group_title(&src_ds.archetype, col)
                && !title.is_empty()
                && base_col.saturating_add(offset) < target_ds.archetype.column_count()
            {
                let _ = set_group_title(&mut target_ds.archetype, base_col.saturating_add(offset), title);
            }
        }
    }
}

fn src_row_title(arch: &TableArchetype, row: usize) -> Option<&str> {
    let slice = match arch {
        TableArchetype::Xy(t) => &t.row_titles,
        TableArchetype::Column(t) => &t.row_titles,
        TableArchetype::Grouped(t) => &t.row_titles,
        TableArchetype::Contingency(t) => &t.row_titles,
        TableArchetype::Survival(t) => &t.row_titles,
        TableArchetype::PartsOfWhole(t) => &t.slice_labels,
        TableArchetype::MultipleVariables(t) => &t.row_titles,
        TableArchetype::Nested(t) => &t.row_titles,
    };
    slice.get(row).map(String::as_str)
}

fn src_group_title(arch: &TableArchetype, col: usize) -> Option<&str> {
    match arch {
        TableArchetype::Xy(t) => t.data_sets.get(col).map(|d| d.title.as_str()),
        TableArchetype::Column(t) => t.data_sets.get(col).map(|d| d.title.as_str()),
        TableArchetype::Grouped(t) => t.data_sets.get(col).map(|d| d.title.as_str()),
        TableArchetype::Contingency(t) => t.data_sets.get(col).map(|d| d.title.as_str()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col).map(|d| d.title.as_str()),
        TableArchetype::Survival(t) => t.cohorts.get(col).map(|d| d.title.as_str()),
        TableArchetype::MultipleVariables(t) => t.variables.get(col).map(|v| v.title.as_str()),
        TableArchetype::Nested(t) => t.groups.get(col).map(|g| g.title.as_str()),
    }
}

fn run_import_pipeline(
    target_ds: &mut DataSheet,
    spec: &ImportLinkSpec,
    payload: &OwnedSourcePayload,
    opts: &ImportPipelineOptions,
    info_sheet: Option<&mut InfoSheet>,
) -> Result<ImportPipelineReport, EngineError> {
    match payload {
        OwnedSourcePayload::Data(src) => Ok(apply_import_link_pipeline(target_ds, spec, ImportSourcePayload::InternalData(src), opts, info_sheet)?),
        OwnedSourcePayload::Result(src) => {
            Ok(apply_import_link_pipeline(target_ds, spec, ImportSourcePayload::InternalResult(src), opts, info_sheet)?)
        }
        OwnedSourcePayload::Delimited { text, delimiter, decimal_separator } => {
            let src = ImportSourcePayload::DelimitedText { text, delimiter: *delimiter, decimal_separator: *decimal_separator };
            Ok(apply_import_link_pipeline(target_ds, spec, src, opts, info_sheet)?)
        }
    }
}

fn apply_pipeline_with_split_info(
    doc: &mut ProjectDocument,
    target_idx: usize,
    info_idx: Option<usize>,
    spec: &ImportLinkSpec,
    payload: &OwnedSourcePayload,
    opts: &ImportPipelineOptions,
) -> Result<ImportPipelineReport, EngineError> {
    let ProjectDocument { data_sheets, info_sheets, .. } = doc;
    let target_ds =
        data_sheets.get_mut(target_idx).ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "target data sheet index out of bounds"))?;
    let target_info = info_idx.and_then(|idx| info_sheets.get_mut(idx));
    let report = run_import_pipeline(target_ds, spec, payload, opts, target_info)?;
    propagate_internal_source_titles(target_ds, spec, payload);
    Ok(report)
}

// ---------------------------------------------------------------------------
// Platform File Helpers
// ---------------------------------------------------------------------------

fn read_utf8_file(path: &str, label: &str) -> Result<String, EngineError> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Err(EngineError::with_code(ErrorCode::UnsupportedFormat, format!("reading {label} `{path}` from filesystem is unavailable on wasm32"))
            .with_remediation("Supply inline text content or run on a native target."))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read_to_string(Path::new(path)).map_err(|err| {
            EngineError::with_code(ErrorCode::IoError, format!("failed reading {label} `{path}`: {err}"))
                .with_remediation("Verify the file path exists and is readable.")
        })
    }
}

fn read_binary_file(path: &str, label: &str) -> Result<Vec<u8>, EngineError> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Err(EngineError::with_code(ErrorCode::UnsupportedFormat, format!("reading {label} `{path}` from filesystem is unavailable on wasm32"))
            .with_remediation("Supply `bytes_base64` or inline content on wasm32."))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::read(Path::new(path)).map_err(|err| {
            EngineError::with_code(ErrorCode::IoError, format!("failed reading {label} `{path}`: {err}"))
                .with_remediation("Verify the file path exists and is readable.")
        })
    }
}

fn write_atomic_text_file(path: &str, text: &str) -> Result<(), EngineError> {
    if path.trim().is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`path` cannot be empty").with_remediation("Provide a non-empty output file path."));
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = text;
        Err(EngineError::with_code(ErrorCode::UnsupportedFormat, "writing CSV files directly to disk is unavailable on wasm32")
            .with_remediation("Omit `path` and consume the returned `csv` string directly."))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let target = Path::new(path);
        if let Some(parent) = target.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|err| {
                EngineError::with_code(ErrorCode::IoError, format!("failed creating parent directory for `{path}`: {err}"))
                    .with_remediation("Ensure the output directory path is valid and writable.")
            })?;
        }
        lumencraft_platform::fs::write_atomic(target, text.as_bytes()).map_err(|err| {
            EngineError::with_code(ErrorCode::IoError, format!("failed writing CSV `{path}`: {err}"))
                .with_remediation("Verify disk space and write permissions for the target path.")
        })
    }
}

// ---------------------------------------------------------------------------
// Shared Deserialization & Parsing Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct CoordInput {
    #[serde(alias = "coordinate")]
    coord: Option<Box<CoordInput>>,
    row: Option<usize>,
    #[serde(alias = "data_set", alias = "dataSet", alias = "column")]
    col: Option<usize>,
    #[serde(alias = "subcolumn", alias = "replicate")]
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
    #[serde(alias = "target_row", alias = "targetRow")]
    row: Option<usize>,
    #[serde(alias = "target_col", alias = "targetCol", alias = "data_set", alias = "dataSet", alias = "column")]
    col: Option<usize>,
    #[serde(alias = "target_subcol", alias = "targetSubcol", alias = "subcolumn", alias = "replicate")]
    subcol: Option<usize>,
}

impl FlexibleCoordInput {
    fn resolve(&self) -> CellCoord {
        CellCoord::new(self.row.unwrap_or(0), self.col.unwrap_or(0), self.subcol.unwrap_or(0))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct CellRangeInput {
    #[serde(alias = "from")]
    start: Option<CoordInput>,
    #[serde(alias = "to")]
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
    #[serde(alias = "decimalSeparator", alias = "decimal")]
    decimal_separator: Option<String>,
    #[serde(alias = "parseAsteriskExclusion")]
    parse_asterisk_exclusion: Option<bool>,
    excluded: Option<bool>,
}

impl CellPayloadFields {
    fn into_cell(self) -> Result<Cell, EngineError> {
        let dec = parse_decimal_opt(self.decimal_separator.as_deref()).unwrap_or('.');
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
            let nested: CellPayloadFields = serde_json::from_value(val.clone())
                .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid cell value object: {err}")))?;
            nested.into_cell()
        }
        Value::Array(_) => Err(EngineError::with_code(ErrorCode::BadInput, "cell value cannot be a JSON array")
            .with_remediation("Provide a scalar number, string, boolean, or null.")),
    }
}

fn parse_cell_from_grid_value(val: &Value) -> Result<Cell, EngineError> {
    parse_cell_value_with_options(val, '.', true)
}

fn parse_decimal_opt(raw: Option<&str>) -> Option<char> {
    match raw?.to_ascii_lowercase().as_str() {
        "." | "dot" | "period" => Some('.'),
        "," | "comma" => Some(','),
        _ => None,
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
    if let Ok(parsed) = serde_json::from_value::<ImportFilterSpec>(v.clone()) {
        return Ok(parsed);
    }
    #[derive(Deserialize)]
    struct FilterObj {
        #[serde(alias = "rowStart")]
        row_start: Option<usize>,
        #[serde(alias = "rowEnd")]
        row_end: Option<usize>,
        #[serde(alias = "colStart")]
        col_start: Option<usize>,
        #[serde(alias = "colEnd")]
        col_end: Option<usize>,
        #[serde(alias = "decimateEveryK", alias = "every_nth_row", alias = "everyNthRow")]
        decimate_every_k: Option<usize>,
        #[serde(alias = "unstackGroupColumn")]
        unstack_group_column: Option<usize>,
        #[serde(alias = "parseAsteriskExclusions", alias = "parse_asterisk_exclusion", alias = "parseAsteriskExclusion")]
        parse_asterisk_exclusions: Option<bool>,
    }
    let obj: FilterObj =
        serde_json::from_value(v.clone()).map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid import filter: {err}")))?;
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
    #[serde(alias = "column", alias = "reference_col", alias = "referenceCol")]
    col: Option<usize>,
    #[serde(alias = "operation", alias = "mode")]
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
    obj_schema_with_aliases(
        vec![
            ("coord", obj_schema(vec![], &[], true)),
            ("row", int_schema("Zero-based row index", Some(0), None)),
            ("col", int_schema("Zero-based dataset/column index", Some(0), None)),
            ("subcol", int_schema("Zero-based replicate subcolumn index (default 0)", Some(0), None)),
        ],
        &[],
        vec![("coord", &["coordinate"]), ("col", &["data_set", "dataSet", "column"]), ("subcol", &["subcolumn", "replicate"])],
        true,
    )
}

fn cell_range_schema() -> Value {
    let endpoint = coord_object_schema();
    obj_schema_with_aliases(vec![("start", endpoint.clone()), ("end", endpoint)], &["start"], vec![("start", &["from"]), ("end", &["to"])], true)
}

fn cell_value_schema() -> Value {
    json!({
        "description": "Cell scalar value: finite number, text/numeric string, boolean, cell object, or null to clear",
        "anyOf": [
            num_schema("Finite numeric cell value", None, None),
            str_schema("String or numeric token (trailing `*` sets excluded when enabled)"),
            bool_schema("Boolean text cell"),
            { "type": "object" },
            { "type": "null" }
        ]
    })
}

fn anchor_schema() -> Value {
    obj_schema_with_aliases(
        vec![
            ("row", int_schema("Anchor row index", Some(0), None)),
            ("col", int_schema("Anchor column index", Some(0), None)),
            ("subcol", int_schema("Anchor subcolumn index", Some(0), None)),
        ],
        &[],
        vec![
            ("row", &["target_row", "targetRow"]),
            ("col", &["target_col", "targetCol", "data_set", "dataSet", "column"]),
            ("subcol", &["target_subcol", "targetSubcol", "subcolumn", "replicate"]),
        ],
        true,
    )
}

fn delimiter_schema() -> Value {
    nullable_schema(str_schema("Column delimiter (`auto`, `,`, `\\t`/`tab`, `;`, or single character)"))
}

fn decimal_sep_schema() -> Value {
    nullable_schema(str_schema("Decimal separator (`.`, `,`, `dot`, `comma`, `period`, or `auto`)"))
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
    let ds = sets.get_mut(col).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("column {col} out of range")))?;
    ds.title = title.to_owned();
    Ok(())
}

fn set_nested_subcol_title(nested: &mut NestedTable, col_opt: Option<usize>, subcol: usize, title: &str) -> Result<(), EngineError> {
    if let Some(col) = col_opt {
        let grp = nested.groups.get_mut(col).ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("group {col} out of range")))?;
        assign_subcol_title(&mut grp.subcolumn_titles, grp.cells.len(), subcol, title)?;
    } else {
        for grp in &mut nested.groups {
            assign_subcol_title(&mut grp.subcolumn_titles, grp.cells.len(), subcol, title)?;
        }
    }
    Ok(())
}

fn assign_subcol_title(titles: &mut Vec<String>, width: usize, subcol: usize, title: &str) -> Result<(), EngineError> {
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
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("coord", coord_object_schema()),
            ("row", int_schema("Zero-based row index", Some(0), None)),
            ("col", int_schema("Zero-based column/dataset index", Some(0), None)),
            ("subcol", int_schema("Zero-based subcolumn index (default 0)", Some(0), None)),
            ("value", cell_value_schema()),
            ("excluded", bool_schema("Optional explicit exclusion flag")),
            ("decimal_separator", decimal_sep_schema()),
            ("parse_asterisk_exclusion", bool_schema("Interpret trailing `*` in string tokens as excluded (default true)")),
        ],
        &["sheet_id"],
        vec![
            ("sheet_id", &["sheetId"]),
            ("coord", &["coordinate"]),
            ("col", &["data_set", "dataSet", "column"]),
            ("subcol", &["subcolumn", "replicate"]),
            ("decimal_separator", &["decimalSeparator", "decimal"]),
            ("parse_asterisk_exclusion", &["parseAsteriskExclusion"]),
        ],
        true,
    )
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
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;
    ensure_mv_columns_editable(ds, std::iter::once(coord.col))?;
    ds.set_cell(coord, cell)?;
    let recalc = recalculate_with_links(doc, false)?;
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
    let edit_item = obj_schema_with_aliases(
        vec![
            ("coord", coord_object_schema()),
            ("row", int_schema("Zero-based row index", Some(0), None)),
            ("col", int_schema("Zero-based column index", Some(0), None)),
            ("subcol", int_schema("Zero-based subcolumn index", Some(0), None)),
            ("value", cell_value_schema()),
            ("excluded", bool_schema("Optional explicit exclusion flag")),
            ("decimal_separator", decimal_sep_schema()),
            ("parse_asterisk_exclusion", bool_schema("Parse trailing `*` exclusion marker")),
        ],
        &[],
        vec![
            ("coord", &["coordinate"]),
            ("col", &["data_set", "dataSet", "column"]),
            ("subcol", &["subcolumn", "replicate"]),
            ("decimal_separator", &["decimalSeparator", "decimal"]),
            ("parse_asterisk_exclusion", &["parseAsteriskExclusion"]),
        ],
        true,
    );
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("edits", array_schema("List of explicit cell edits", edit_item, None, None)),
            ("rows", array_schema("2D array of cell values or objects", array_schema("Row of cells", cell_value_schema(), None, None), None, None)),
            ("start_row", int_schema("Starting row for `rows` grid (default 0)", Some(0), None)),
            ("start_col", int_schema("Starting column for `rows` grid (default 0)", Some(0), None)),
            ("start_subcol", int_schema("Subcolumn index for `rows` grid (default 0)", Some(0), None)),
        ],
        &["sheet_id"],
        vec![
            ("sheet_id", &["sheetId"]),
            ("edits", &["cells"]),
            ("rows", &["grid"]),
            ("start_row", &["startRow"]),
            ("start_col", &["startCol"]),
            ("start_subcol", &["startSubcol"]),
        ],
        true,
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
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;
    ensure_mv_columns_editable(ds, edits.iter().map(|(c, _)| c.col))?;
    for (coord, cell) in &edits {
        ds.set_cell(*coord, cell.clone())?;
    }
    let recalc = recalculate_with_links(doc, false)?;
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
    #[serde(alias = "subcolumn", alias = "replicate")]
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
    let x_edit = obj_schema_with_aliases(
        vec![
            ("row", int_schema("Zero-based row index", Some(0), None)),
            ("subcol", int_schema("Zero-based X subcolumn index (default 0)", Some(0), None)),
            ("value", cell_value_schema()),
            ("excluded", bool_schema("Optional explicit exclusion flag")),
            ("decimal_separator", decimal_sep_schema()),
            ("parse_asterisk_exclusion", bool_schema("Parse trailing `*` exclusion marker")),
        ],
        &["row"],
        vec![
            ("subcol", &["subcolumn", "replicate"]),
            ("decimal_separator", &["decimalSeparator", "decimal"]),
            ("parse_asterisk_exclusion", &["parseAsteriskExclusion"]),
        ],
        true,
    );
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target XY or Survival sheet identifier")),
            ("edits", array_schema("List of X/time cell edits", x_edit, None, None)),
            ("values", array_schema("Sequential X/time values starting at `start_row`", cell_value_schema(), None, None)),
            ("start_row", int_schema("Starting row for `values` batch (default 0)", Some(0), None)),
            ("row", int_schema("Single row index when editing one X cell", Some(0), None)),
            ("subcol", int_schema("X subcolumn index (default 0)", Some(0), None)),
            ("value", cell_value_schema()),
            ("excluded", bool_schema("Optional explicit exclusion flag")),
            ("decimal_separator", decimal_sep_schema()),
            ("parse_asterisk_exclusion", bool_schema("Parse trailing `*` exclusion marker")),
        ],
        &["sheet_id"],
        vec![
            ("sheet_id", &["sheetId"]),
            ("edits", &["cells"]),
            ("values", &["x"]),
            ("start_row", &["startRow"]),
            ("subcol", &["subcolumn", "replicate"]),
            ("decimal_separator", &["decimalSeparator", "decimal"]),
            ("parse_asterisk_exclusion", &["parseAsteriskExclusion"]),
        ],
        true,
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
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;
    for (row, subcol, cell) in &updates {
        ds.set_x_or_time_cell(*row, *subcol, cell.clone())?;
    }
    let recalc = recalculate_with_links(doc, false)?;
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
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("titles", array_schema("Batch of row titles (null clears a title)", nullable_schema(str_schema("Row title")), None, None)),
            ("start_row", int_schema("Starting row for `titles` batch (default 0)", Some(0), None)),
            ("row", int_schema("Single row index when `titles` is omitted", Some(0), None)),
            ("title", nullable_schema(str_schema("Single row title (omit or null to clear)"))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["sheetId"]), ("titles", &["row_titles", "rowTitles"]), ("start_row", &["startRow"])],
        true,
    )
}

fn run_set_row_titles(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetRowTitlesParams = parse_params(&schema_set_row_titles(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
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

    let recalc = recalculate_with_links(doc, false)?;
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
    obj_schema_with_aliases(
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
        vec![
            ("sheet_id", &["sheetId"]),
            ("x_title", &["xTitle", "time_title", "timeTitle"]),
            ("row_factor_title", &["rowFactorTitle"]),
            ("column_factor_title", &["columnFactorTitle"]),
            ("titles", &["group_titles", "groupTitles", "column_titles", "columnTitles", "data_set_titles", "dataSetTitles"]),
            ("start_col", &["startCol"]),
            ("col", &["group", "data_set", "dataSet", "column", "index"]),
            ("row_titles", &["rowTitles"]),
        ],
        true,
    )
}

fn run_set_group_titles(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_set_titles_with_id("table.setGroupTitles", ctx, raw)
}

fn run_set_titles(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_set_titles_with_id("table.setTitles", ctx, raw)
}

fn run_set_titles_with_id(command_id: &str, ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetTitlesParams = parse_params(&schema_set_titles(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let doc = &mut ctx.session.doc;
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;

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
        validate_row_title_batch(&ds.archetype, 0, &row_titles)?;
        for (idx, title) in row_titles.into_iter().enumerate() {
            set_row_title(&mut ds.archetype, idx, title)?;
            updated = updated.saturating_add(1);
        }
    }

    let recalc = recalculate_with_links(doc, false)?;
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
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target Nested data sheet identifier")),
            ("col", nullable_schema(int_schema("Optional group index (omit to update all groups)", Some(0), None))),
            ("titles", array_schema("Batch of subcolumn titles", str_schema("Subcolumn title"), None, None)),
            ("start_subcol", int_schema("Starting subcolumn index (default 0)", Some(0), None)),
            ("subcol", int_schema("Single subcolumn index when `titles` is omitted", Some(0), None)),
            ("title", str_schema("Single subcolumn title")),
        ],
        &["sheet_id"],
        vec![
            ("sheet_id", &["sheetId"]),
            ("col", &["group", "data_set", "dataSet", "column"]),
            ("titles", &["subcolumn_titles", "subcolumnTitles"]),
            ("start_subcol", &["startSubcol"]),
            ("subcol", &["subcolumn", "replicate", "index"]),
        ],
        true,
    )
}

fn run_set_subcolumn_titles(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetSubcolumnTitlesParams = parse_params(&schema_set_subcolumn_titles(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
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

    let recalc = recalculate_with_links(doc, false)?;
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
    obj_schema_with_aliases(
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
        vec![
            ("sheet_id", &["sheetId"]),
            ("range", &["selection"]),
            ("cells", &["coords"]),
            ("coord", &["coordinate"]),
            ("col", &["data_set", "dataSet", "column"]),
            ("subcol", &["subcolumn", "replicate"]),
            ("row_start", &["rowStart"]),
            ("row_end", &["rowEnd"]),
            ("col_start", &["colStart"]),
            ("col_end", &["colEnd"]),
            ("subcol_start", &["subcolStart"]),
            ("subcol_end", &["subcolEnd"]),
        ],
        true,
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
    if let Some(range) = range_opt {
        validate_cell_range_bounds(ds, range, target_x)?;
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
        for item in coords {
            if target_x {
                let row = item.coord.row.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing required X cell `row`"))?;
                ds.set_x_or_time_excluded(row, item.coord.subcol.unwrap_or(0), excluded)?;
            } else {
                ds.set_excluded(item.coord.resolve()?, excluded)?;
            }
            updated = updated.saturating_add(1);
        }
    } else if target_x {
        let row = p.bounds.row.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing required X cell `row`"))?;
        ds.set_x_or_time_excluded(row, p.bounds.subcol.unwrap_or(0), excluded)?;
        updated = 1;
    } else {
        let coord = CoordInput { coord: p.coord, row: p.bounds.row, col: p.bounds.col, subcol: p.bounds.subcol }.resolve()?;
        ds.set_excluded(coord, excluded)?;
        updated = 1;
    }

    let recalc = recalculate_with_links(doc, false)?;
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
    obj_schema_with_aliases(
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
        vec![
            ("sheet_id", &["sheetId", "id"]),
            ("reason_tag", &["reasonTag", "reason"]),
            ("note", &["comment"]),
            ("timestamp_utc", &["timestampUtc", "created_utc", "createdUtc"]),
            ("coords", &["cells"]),
            ("coord", &["coordinate"]),
            ("col", &["data_set", "dataSet", "column"]),
            ("subcol", &["subcolumn", "replicate"]),
        ],
        true,
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
    for &coord in &coords {
        ds.set_excluded_with_reason(coord, &p.reason_tag, &note, &timestamp_utc)?;
    }
    let recalc = recalculate_with_links(doc, false)?;
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
    obj_schema_with_aliases(
        vec![("sheet_id", nullable_schema(non_empty_str_schema("Optional sheet ID to filter report")))],
        &[],
        vec![("sheet_id", &["sheetId", "id"])],
        true,
    )
}

fn collect_sheet_exclusions(ds: &DataSheet, entries: &mut Vec<Value>, annotated_count: &mut usize) {
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
    #[serde(flatten)]
    bounds: FlatRangeBounds,
    cells: Option<Vec<CoordInput>>,
    coord: Option<Box<CoordInput>>,
}

fn schema_clear_cells() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("all", bool_schema("Clear all data cells across the sheet")),
            ("clear_x", bool_schema("When `all` is true, also clear X/time cells")),
            ("unlink", bool_schema("Also detach all live linked regions on this sheet")),
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
        vec![
            ("sheet_id", &["sheetId"]),
            ("clear_x", &["clearX", "include_x", "includeX"]),
            ("unlink", &["break_links", "breakLinks"]),
            ("range", &["selection"]),
            ("cells", &["coords"]),
            ("coord", &["coordinate"]),
            ("col", &["data_set", "dataSet", "column"]),
            ("subcol", &["subcolumn", "replicate"]),
            ("row_start", &["rowStart"]),
            ("row_end", &["rowEnd"]),
            ("col_start", &["colStart"]),
            ("col_end", &["colEnd"]),
            ("subcol_start", &["subcolStart"]),
            ("subcol_end", &["subcolEnd"]),
        ],
        true,
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
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;

    let mut cleared = 0usize;
    if p.all.unwrap_or(false) {
        cleared = clear_all_cells_in_sheet(ds, p.clear_x.unwrap_or(false))?;
    } else if let Some(range) = range_opt {
        validate_cell_range_bounds(ds, range, false)?;
        ensure_mv_columns_editable(ds, range.start.col..=range.end.col)?;
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
        for coord in resolved {
            ds.set_cell(coord, Cell::empty())?;
            cleared = cleared.saturating_add(1);
        }
    } else {
        let coord = CoordInput { coord: p.coord, row: p.bounds.row, col: p.bounds.col, subcol: p.bounds.subcol }.resolve()?;
        ensure_mv_columns_editable(ds, std::iter::once(coord.col))?;
        ds.set_cell(coord, Cell::empty())?;
        cleared = 1;
    }

    if p.unlink.unwrap_or(false) {
        ds.linked_regions.clear();
    }
    let recalc = recalculate_with_links(doc, false)?;
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
    #[serde(alias = "startRow", alias = "target_row", alias = "targetRow", alias = "row")]
    start_row: Option<usize>,
    #[serde(alias = "startCol", alias = "target_col", alias = "targetCol", alias = "col", alias = "data_set", alias = "dataSet")]
    start_col: Option<usize>,
    #[serde(alias = "startSubcol", alias = "target_subcol", alias = "targetSubcol", alias = "subcol", alias = "subcolumn")]
    start_subcol: Option<usize>,
    #[serde(alias = "placement_mode", alias = "placementMode")]
    mode: Option<String>,
    #[serde(alias = "propagateTitleHeader")]
    propagate_title_header: Option<bool>,
    #[serde(alias = "skipContiguousBlanks")]
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
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target data sheet identifier")),
            ("text", str_schema("Delimited clipboard text (TSV/CSV)")),
            ("rows", array_schema("2D array of pasted cell values", array_schema("Row of cell values", cell_value_schema(), None, None), None, None)),
            ("delimiter", delimiter_schema()),
            ("decimal_separator", decimal_sep_schema()),
            ("normalize_decimal", bool_schema("Infer European comma decimal separator when ambiguous")),
            ("filter", nullable_schema(json!({ "type": "object", "description": "Optional row/col/decimation filter" }))),
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
        vec![
            ("sheet_id", &["sheetId", "target_sheet_id", "targetSheetId"]),
            ("text", &["tsv", "csv", "clipboard"]),
            ("rows", &["grid"]),
            ("decimal_separator", &["decimalSeparator", "decimal"]),
            ("normalize_decimal", &["normalizeDecimal"]),
            ("filter", &["slice"]),
            ("anchor", &["start"]),
            ("start_row", &["startRow", "target_row", "targetRow", "row"]),
            ("start_col", &["startCol", "target_col", "targetCol", "col", "data_set", "dataSet"]),
            ("start_subcol", &["startSubcol", "target_subcol", "targetSubcol", "subcol", "subcolumn"]),
            ("mode", &["placement_mode", "placementMode"]),
            ("propagate_title_header", &["propagateTitleHeader"]),
            ("skip_contiguous_blanks", &["skipContiguousBlanks"]),
        ],
        true,
    )
}

fn serialize_2d_rows_as_tsv(rows_arr: &[Vec<Value>], decimal_separator: char) -> Result<String, EngineError> {
    let mut lines = Vec::with_capacity(rows_arr.len());
    for row_cells in rows_arr {
        let mut tokens = Vec::with_capacity(row_cells.len());
        for cell_val in row_cells {
            let cell = parse_cell_value_with_options(cell_val, decimal_separator, true)?;
            tokens.push(format_cell_as_tsv_token(&cell));
        }
        lines.push(tokens.join("\t"));
    }
    Ok(lines.join("\n"))
}

fn format_cell_as_tsv_token(cell: &Cell) -> String {
    let base = match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::Number(v) => v.to_string(),
        CellValue::Text(s) => s.replace(['\t', '\n', '\r'], " "),
    };
    if cell.excluded && !base.is_empty() { format!("{base}*") } else { base }
}

fn apply_paste_payload(
    ds: &mut DataSheet,
    spec: &ImportLinkSpec,
    payload: ImportSourcePayload<'_>,
    opts: &ImportPipelineOptions,
) -> Result<ImportPipelineReport, EngineError> {
    let has_calc_mv = matches!(
        &ds.archetype,
        TableArchetype::MultipleVariables(mv) if mv.variables.iter().any(|v| matches!(v.source, VariableSource::Calculated { .. }))
    );
    if !has_calc_mv {
        return Ok(apply_import_link_pipeline(ds, spec, payload, opts, None)?);
    }
    let mut staged = ds.clone();
    let report = apply_import_link_pipeline(&mut staged, spec, payload, opts, None)?;
    if report.rows_placed > 0 && report.cols_placed > 0 {
        ensure_mv_columns_editable(ds, report.target_range.start.col..=report.target_range.end.col)?;
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
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;
    let existing_links = ds.linked_regions.len();
    let opts = ImportPipelineOptions::default();

    let report = if let Some(text) = p.text.as_deref() {
        let delimiter = resolve_delimiter(p.delimiter.as_deref(), text)?;
        let decimal_separator = resolve_decimal_char(p.decimal_separator.as_deref(), p.normalize_decimal.unwrap_or(false), text, delimiter)?;
        let payload = ImportSourcePayload::DelimitedText { text, delimiter, decimal_separator };
        apply_paste_payload(ds, &spec, payload, &opts)?
    } else if let Some(rows_arr) = p.rows.as_ref() {
        let dec = parse_decimal_opt(p.decimal_separator.as_deref()).unwrap_or('.');
        let tsv = serialize_2d_rows_as_tsv(rows_arr, dec)?;
        let payload = ImportSourcePayload::DelimitedText { text: &tsv, delimiter: '\t', decimal_separator: '.' };
        apply_paste_payload(ds, &spec, payload, &opts)?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.paste requires `text`/`tsv`/`csv` or `rows`/`grid`")
            .with_remediation("Supply delimited `text` or a 2D `rows` array."));
    };

    ds.linked_regions.truncate(existing_links);
    let recalc = recalculate_with_links(doc, false)?;
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
struct PasteLinkParams {
    target_sheet_id: String,
    link_spec: Option<Value>,
    source_sheet_id: Option<String>,
    source_kind: Option<String>,
    tab_id: Option<String>,
    source_range: Option<CellRangeInput>,
    source_path: Option<String>,
    delimiter: Option<String>,
    decimal_separator: Option<String>,
    normalize_decimal: Option<bool>,
    filter: Option<Value>,
    baseline: Option<BaselineInput>,
    exclude_row_predicate: Option<String>,
    link_id: Option<String>,
    provenance: Option<String>,
    #[serde(flatten)]
    placement: PlacementFields,
}

fn schema_paste_link() -> Value {
    obj_schema_with_aliases(
        vec![
            ("target_sheet_id", non_empty_str_schema("Destination data sheet identifier")),
            ("link_spec", nullable_schema(json!({ "type": "object", "description": "Direct ImportLinkSpec payload" }))),
            ("source_sheet_id", nullable_schema(non_empty_str_schema("Internal source Data or Result sheet ID"))),
            ("source_kind", nullable_schema(str_schema("Explicit source kind: data, result, analyzable_result, terminal_result"))),
            ("tab_id", nullable_schema(str_schema("Result sheet tab ID or title"))),
            ("source_range", nullable_schema(cell_range_schema())),
            ("source_path", nullable_schema(non_empty_str_schema("External delimited file path"))),
            ("delimiter", delimiter_schema()),
            ("decimal_separator", decimal_sep_schema()),
            ("normalize_decimal", bool_schema("Infer European decimal separator when ambiguous")),
            ("filter", nullable_schema(json!({ "type": "object" }))),
            ("baseline", nullable_schema(json!({ "type": "object" }))),
            ("exclude_row_predicate", nullable_schema(str_schema("Optional predicate excluding matching linked rows"))),
            ("link_id", nullable_schema(str_schema("Custom link identifier"))),
            ("provenance", nullable_schema(str_schema("Custom provenance note recorded in linked InfoSheet"))),
            ("anchor", anchor_schema()),
            ("start_row", int_schema("Target anchor row", Some(0), None)),
            ("start_col", int_schema("Target anchor column", Some(0), None)),
            ("start_subcol", int_schema("Target anchor subcolumn", Some(0), None)),
            ("mode", str_schema("Placement mode: overwrite, insert_rows, append_rows, append_columns")),
            ("propagate_title_header", bool_schema("Propagate source titles to target headers")),
            ("skip_contiguous_blanks", bool_schema("Skip contiguous blanks")),
            ("transpose", bool_schema("Transpose source matrix")),
        ],
        &["target_sheet_id"],
        vec![
            ("target_sheet_id", &["targetSheetId", "sheet_id", "sheetId"]),
            ("link_spec", &["linkSpec"]),
            ("source_sheet_id", &["sourceSheetId"]),
            ("source_kind", &["sourceKind"]),
            ("tab_id", &["tabId", "source_tab_id", "sourceTabId"]),
            ("source_range", &["sourceRange", "range"]),
            ("source_path", &["sourcePath", "path", "file_path", "filePath"]),
            ("decimal_separator", &["decimalSeparator", "decimal"]),
            ("normalize_decimal", &["normalizeDecimal"]),
            ("filter", &["slice"]),
            ("exclude_row_predicate", &["excludeRowPredicate"]),
            ("link_id", &["linkId"]),
            ("provenance", &["provenance_summary"]),
            ("anchor", &["start"]),
            ("start_row", &["startRow", "target_row", "targetRow", "row"]),
            ("start_col", &["startCol", "target_col", "targetCol", "col", "data_set", "dataSet"]),
            ("start_subcol", &["startSubcol", "target_subcol", "targetSubcol", "subcol", "subcolumn"]),
            ("mode", &["placement_mode", "placementMode"]),
            ("propagate_title_header", &["propagateTitleHeader"]),
            ("skip_contiguous_blanks", &["skipContiguousBlanks"]),
        ],
        true,
    )
}

fn summarize_link_source(source: &LinkSource) -> String {
    match source {
        LinkSource::Clipboard => String::from("Clipboard paste"),
        LinkSource::InternalSheet { sheet_id, kind, .. } => format!("Link from {kind:?} `{sheet_id}`"),
        LinkSource::ExternalDelimitedFile { path, .. } => format!("Link from `{path}`"),
    }
}

fn resolve_internal_link_kind(doc: &ProjectDocument, src_id: &SheetId, explicit: Option<&str>) -> Result<InternalLinkKind, EngineError> {
    if let Some(raw) = explicit {
        let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        return match norm.as_str() {
            "data" | "datasheet" => Ok(InternalLinkKind::DataSheet),
            "analyzableresult" | "analyzableresultsheet" => Ok(InternalLinkKind::AnalyzableResultSheet),
            "terminalresult" | "terminalresultsheet" => Ok(InternalLinkKind::TerminalResultSheet),
            "result" | "resultsheet" => {
                let res = doc
                    .result_sheet(src_id)
                    .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("result sheet `{src_id}` not found")))?;
                Ok(if res.analyzable_data.is_some() { InternalLinkKind::AnalyzableResultSheet } else { InternalLinkKind::TerminalResultSheet })
            }
            _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown link source kind `{raw}`"))),
        };
    }
    match doc.sheet_section(src_id) {
        Some(SheetSection::Data) => Ok(InternalLinkKind::DataSheet),
        Some(SheetSection::Analyses) => {
            let res =
                doc.result_sheet(src_id).ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("result sheet `{src_id}` not found")))?;
            Ok(if res.analyzable_data.is_some() { InternalLinkKind::AnalyzableResultSheet } else { InternalLinkKind::TerminalResultSheet })
        }
        Some(other) => Err(EngineError::with_code(ErrorCode::BadInput, format!("sheet `{src_id}` in section {other:?} cannot be link source"))),
        None => Err(EngineError::with_code(ErrorCode::NotFound, format!("source sheet `{src_id}` not found"))),
    }
}

fn default_full_source_range(
    doc: &ProjectDocument,
    src_id: &SheetId,
    kind: InternalLinkKind,
    tab_id: Option<&str>,
) -> Result<SheetCellRange, EngineError> {
    match kind {
        InternalLinkKind::DataSheet => {
            let ds = require_data_sheet(doc, src_id)?;
            let max_r = ds.archetype.row_count().saturating_sub(1);
            let max_c = ds.archetype.column_count().saturating_sub(1);
            let max_s = (0..ds.archetype.column_count()).map(|c| column_subcolumn_count(&ds.archetype, c)).max().unwrap_or(1).saturating_sub(1);
            Ok(SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(max_r, max_c, max_s)))
        }
        InternalLinkKind::AnalyzableResultSheet | InternalLinkKind::TerminalResultSheet => {
            let res = doc
                .result_sheet(src_id)
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("source result sheet `{src_id}` not found")))?;
            let tab = select_result_tab(res, tab_id)?;
            let max_r = tab.rows.len().saturating_sub(1);
            let max_c = tab.column_headers.len().max(tab.rows.first().map_or(0, Vec::len)).saturating_sub(1);
            Ok(SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(max_r, max_c, 0)))
        }
    }
}

fn build_paste_link_spec(doc: &ProjectDocument, target_id: &SheetId, p: &PasteLinkParams) -> Result<ImportLinkSpec, EngineError> {
    if let Some(raw_spec) = &p.link_spec
        && let Ok(mut parsed) = serde_json::from_value::<ImportLinkSpec>(raw_spec.clone())
    {
        if p.placement.anchor.is_some() {
            parsed.placement.anchor = p.placement.resolve_anchor();
        }
        if let Some(t) = p.placement.transpose {
            parsed.placement.transpose = t;
        }
        if let Some(prop) = p.placement.propagate_title_header {
            parsed.placement.propagate_title_header = prop;
        }
        return Ok(parsed);
    }

    let anchor = p.placement.resolve_anchor();
    let transpose = p.placement.transpose.unwrap_or(false);
    let default_prop = transpose && anchor.row == 0;
    let placement = p.placement.to_placement_spec(default_prop)?;
    let filter = parse_import_filter_val(p.filter.as_ref())?;

    let source = if let Some(src_str) = p.source_sheet_id.as_deref() {
        let src_id = parse_sheet_id(src_str)?;
        if &src_id == target_id {
            return Err(EngineError::with_code(ErrorCode::CycleDetected, format!("self-link on `{target_id}` would create a DAG cycle"))
                .with_remediation("Link from a different upstream data or result sheet."));
        }
        let kind = resolve_internal_link_kind(doc, &src_id, p.source_kind.as_deref())?;
        let tab_id = p.tab_id.clone();
        let range = match &p.source_range {
            Some(r) => r.resolve()?,
            None => default_full_source_range(doc, &src_id, kind, tab_id.as_deref())?,
        };
        LinkSource::InternalSheet { sheet_id: src_id, kind, tab_id, range }
    } else if let Some(path) = p.source_path.as_deref() {
        let preview_text = read_utf8_file(path, "external link file")?;
        let delimiter = resolve_delimiter(p.delimiter.as_deref(), &preview_text)?;
        let decimal_separator = resolve_decimal_char(p.decimal_separator.as_deref(), p.normalize_decimal.unwrap_or(false), &preview_text, delimiter)?;
        LinkSource::ExternalDelimitedFile { path: path.to_owned(), delimiter, decimal_separator }
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.pasteLink requires `link_spec`, `source_sheet_id`, or `path`")
            .with_remediation("Specify an internal `source_sheet_id` or an external `path`."));
    };

    Ok(ImportLinkSpec { source, filter, placement })
}

fn run_paste_link(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: PasteLinkParams = parse_params(&schema_paste_link(), raw)?;
    let target_id = parse_sheet_id(&p.target_sheet_id)?;
    let doc = &mut ctx.session.doc;

    let info_id = {
        let target_ds = require_data_sheet(doc, &target_id)?;
        ensure_sheet_not_frozen(target_ds)?;
        target_ds.info_sheet_id.clone()
    };

    let spec = build_paste_link_spec(doc, &target_id, &p)?;
    let payload = resolve_link_source_payload(doc, &spec.source, true)?
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "table.pasteLink requires an internal sheet or external delimited file source"))?;
    let (target_idx, info_idx) = locate_target_and_info_indices(doc, &target_id, info_id.as_ref())?;
    let prev_ds = doc.data_sheets.get(target_idx).cloned();
    let prev_info = info_idx.and_then(|idx| doc.info_sheets.get(idx).cloned());

    let link_seq = prev_ds.as_ref().map_or(1, |d| d.linked_regions.len().saturating_add(1));
    let link_id = p.link_id.unwrap_or_else(|| format!("link-{}-{link_seq}", target_id.as_str()));
    let provenance_summary = p.provenance.unwrap_or_else(|| summarize_link_source(&spec.source));
    let opts = ImportPipelineOptions {
        baseline: resolve_baseline_adjustment(p.baseline)?,
        exclude_coords: Vec::new(),
        exclude_row_predicate: p.exclude_row_predicate,
        link_id: Some(link_id),
        provenance_summary: Some(provenance_summary),
    };

    let report = match (|| -> Result<ImportPipelineReport, EngineError> {
        let rep = apply_pipeline_with_split_info(doc, target_idx, info_idx, &spec, &payload, &opts)?;
        let _ = dag::topological_order(doc)?;
        Ok(rep)
    })() {
        Ok(r) => r,
        Err(e) => {
            if let (Some(slot), Some(old)) = (doc.data_sheets.get_mut(target_idx), prev_ds) {
                *slot = old;
            }
            if let (Some(idx), Some(old_info)) = (info_idx, prev_info)
                && let Some(slot) = doc.info_sheets.get_mut(idx)
            {
                *slot = old_info;
            }
            return Err(e);
        }
    };

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        "table.pasteLink",
        "created live linked region",
        json!({
            "target_sheet_id": target_id.as_str(),
            "rows_placed": report.rows_placed,
            "cols_placed": report.cols_placed,
            "target_range": &report.target_range,
            "linked_regions": doc.data_sheet(&target_id).map_or(0, |d| d.linked_regions.len()),
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 14. table.importCsv
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ImportCsvParams {
    sheet_id: Option<String>,
    csv: Option<String>,
    path: Option<String>,
    delimiter: Option<String>,
    decimal_separator: Option<String>,
    normalize_decimal: Option<bool>,
    has_header: Option<bool>,
    has_row_titles: Option<bool>,
    replicates: Option<usize>,
    subcolumn_mode: Option<Value>,
    replace: Option<bool>,
    title: Option<String>,
    archetype: Option<String>,
    filter: Option<Value>,
    link: Option<bool>,
    #[serde(flatten)]
    placement: PlacementFields,
}

fn schema_import_csv() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional target sheet ID (auto-generated if omitted)"))),
            ("csv", nullable_schema(str_schema("Inline CSV/TSV content"))),
            ("path", nullable_schema(non_empty_str_schema("Filesystem path to CSV/TSV file"))),
            ("delimiter", delimiter_schema()),
            ("decimal_separator", decimal_sep_schema()),
            ("normalize_decimal", bool_schema("Infer European decimal separator when ambiguous")),
            ("has_header", bool_schema("Treat first row as column headers")),
            ("has_row_titles", bool_schema("Treat first column as row titles")),
            ("replicates", int_schema("Replicate count per group (default 1)", Some(1), None)),
            ("subcolumn_mode", nullable_schema(any_json_schema("Subcolumn mode name or object"))),
            ("replace", bool_schema("Replace existing target sheet instead of placing into region")),
            ("title", nullable_schema(str_schema("Title for imported sheet"))),
            (
                "archetype",
                nullable_schema(str_schema("Target archetype (XY, Column, Grouped, Contingency, Survival, PartsOfWhole, MultipleVariables, Nested)")),
            ),
            ("filter", nullable_schema(json!({ "type": "object" }))),
            ("link", bool_schema("When importing from `path` into an existing sheet, keep a live file link")),
            ("anchor", anchor_schema()),
            ("start_row", int_schema("Target anchor row", Some(0), None)),
            ("start_col", int_schema("Target anchor column", Some(0), None)),
            ("start_subcol", int_schema("Target anchor subcolumn", Some(0), None)),
            ("mode", str_schema("Placement mode: overwrite, insert_rows, append_rows, append_columns")),
            ("propagate_title_header", bool_schema("Propagate header row titles")),
            ("skip_contiguous_blanks", bool_schema("Skip contiguous blanks")),
            ("transpose", bool_schema("Transpose imported table")),
        ],
        &[],
        vec![
            ("sheet_id", &["sheetId"]),
            ("csv", &["tsv", "text", "content"]),
            ("path", &["file_path", "filePath"]),
            ("decimal_separator", &["decimalSeparator", "decimal"]),
            ("normalize_decimal", &["normalizeDecimal"]),
            ("has_header", &["hasHeader", "first_row_titles", "firstRowTitles"]),
            ("has_row_titles", &["hasRowTitles"]),
            ("replicates", &["k"]),
            ("subcolumn_mode", &["subcolumnMode"]),
            ("archetype", &["kind", "table_kind", "tableKind"]),
            ("filter", &["slice"]),
            ("link", &["live_link", "liveLink"]),
            ("anchor", &["start"]),
            ("start_row", &["startRow", "target_row", "targetRow", "row"]),
            ("start_col", &["startCol", "target_col", "targetCol", "col", "data_set", "dataSet"]),
            ("start_subcol", &["startSubcol", "target_subcol", "targetSubcol", "subcol", "subcolumn"]),
            ("mode", &["placement_mode", "placementMode"]),
            ("propagate_title_header", &["propagateTitleHeader"]),
            ("skip_contiguous_blanks", &["skipContiguousBlanks"]),
        ],
        true,
    )
}

fn build_csv_options(delimiter: char, decimal_separator: char) -> CsvOptions {
    let mut opts = match (delimiter, decimal_separator) {
        ('\t', _) => CsvOptions::tsv(),
        (';', ',') => CsvOptions::european_csv(),
        _ => CsvOptions::csv(),
    };
    opts.delimiter = delimiter;
    opts.decimal_separator = decimal_separator;
    opts
}

fn import_into_existing_sheet(
    doc: &mut ProjectDocument,
    id: &SheetId,
    p: &ImportCsvParams,
    text: String,
    csv_opts: &CsvOptions,
    subcolumn_mode: SubcolumnMode,
) -> Result<(), EngineError> {
    let delimiter = csv_opts.delimiter;
    let decimal_separator = csv_opts.decimal_separator;
    let (info_id, cur_title, cur_kind) = {
        let ds = require_data_sheet(doc, id)?;
        ensure_sheet_not_frozen(ds)?;
        (ds.info_sheet_id.clone(), ds.title.clone(), ds.archetype.kind())
    };
    if p.replace.unwrap_or(false) {
        let title = p.title.as_deref().unwrap_or(&cur_title).to_owned();
        let kind = match p.archetype.as_deref() {
            Some(k) => parse_table_kind(k)?,
            None => cur_kind,
        };
        let mut imported = import_csv_to_datasheet(id.clone(), title, &text, kind, subcolumn_mode, csv_opts)?;
        imported.info_sheet_id = info_id;
        if let Some(slot) = doc.data_sheet_mut(id) {
            *slot = imported;
        }
        return Ok(());
    }
    let filter = parse_import_filter_val(p.filter.as_ref())?;
    let placement = p.placement.to_placement_spec(false)?;
    let keep_link = p.link.unwrap_or(false);
    let source = match p.path.as_deref() {
        Some(path) if keep_link => LinkSource::ExternalDelimitedFile { path: path.to_owned(), delimiter, decimal_separator },
        _ => LinkSource::Clipboard,
    };
    let link_spec = ImportLinkSpec { source, filter, placement };
    let (target_idx, info_idx) = locate_target_and_info_indices(doc, id, info_id.as_ref())?;
    let opts = ImportPipelineOptions {
        link_id: if keep_link { Some(format!("csv-link-{}", id.as_str())) } else { None },
        provenance_summary: if keep_link { Some(String::from("Imported CSV link")) } else { None },
        ..ImportPipelineOptions::default()
    };
    let link_before = doc.data_sheets.get(target_idx).map_or(0, |d| d.linked_regions.len());
    let payload = OwnedSourcePayload::Delimited { text, delimiter, decimal_separator };
    apply_pipeline_with_split_info(doc, target_idx, info_idx, &link_spec, &payload, &opts)?;
    if !keep_link && let Some(target_ds) = doc.data_sheets.get_mut(target_idx) {
        target_ds.linked_regions.truncate(link_before);
    }
    Ok(())
}

fn run_import_csv(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportCsvParams = parse_params(&schema_import_csv(), raw)?;
    let doc = &mut ctx.session.doc;
    let id = match p.sheet_id.as_deref() {
        Some(raw_id) => parse_sheet_id(raw_id)?,
        None => next_unique_sheet_id(doc, "data")?,
    };
    let text = if let Some(inline) = &p.csv {
        inline.clone()
    } else if let Some(path) = &p.path {
        read_utf8_file(path, "CSV")?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importCsv requires `path` or `csv`/`text`")
            .with_remediation("Provide a CSV/TSV file `path` or inline `csv` text."));
    };

    let delimiter = resolve_delimiter(p.delimiter.as_deref(), &text)?;
    let decimal_separator = resolve_decimal_char(p.decimal_separator.as_deref(), p.normalize_decimal.unwrap_or(false), &text, delimiter)?;
    let mut csv_opts = build_csv_options(delimiter, decimal_separator);
    if let Some(first_row_titles) = p.has_header {
        csv_opts.header_rows = usize::from(first_row_titles);
    }
    if let Some(has_row_titles) = p.has_row_titles {
        csv_opts.has_row_titles = has_row_titles;
    }
    let subcolumn_mode = resolve_subcolumn_mode(p.subcolumn_mode.as_ref(), p.replicates.unwrap_or(1))?;

    if doc.data_sheet(&id).is_some() {
        import_into_existing_sheet(doc, &id, &p, text, &csv_opts, subcolumn_mode)?;
    } else {
        let title = p.title.as_deref().unwrap_or("Imported CSV");
        let kind = parse_table_kind(p.archetype.as_deref().unwrap_or("xy"))?;
        let imported = import_csv_to_datasheet(id.clone(), title, &text, kind, subcolumn_mode, &csv_opts)?;
        doc.add_data_sheet(imported)?;
    }

    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        "table.importCsv",
        "imported CSV into data sheet",
        json!({
            "sheet_id": id.as_str(),
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 15. table.exportCsv
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ExportCsvParams {
    sheet_id: String,
    path: Option<String>,
    delimiter: Option<String>,
    decimal_separator: Option<String>,
}

fn schema_export_csv() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Data or Result sheet identifier to export")),
            ("path", nullable_schema(non_empty_str_schema("Optional output file path"))),
            ("delimiter", nullable_schema(str_schema("Delimiter character (`,`, `\\t`/`tab`, `;`, or 1 char)"))),
            ("decimal_separator", decimal_sep_schema()),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["sheetId"]), ("path", &["file_path", "filePath"]), ("decimal_separator", &["decimalSeparator", "decimal"])],
        true,
    )
}

fn run_export_csv(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ExportCsvParams = parse_params(&schema_export_csv(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let mut csv_opts = CsvOptions::csv();
    if let Some(delim) = p.delimiter.as_deref() {
        csv_opts.delimiter = parse_delimiter_char(delim)?;
    }
    if let Some(dec) = parse_decimal_opt(p.decimal_separator.as_deref()) {
        csv_opts.decimal_separator = dec;
    }

    let doc = ctx.session.document();
    let csv_text = if let Some(ds) = doc.data_sheet(&id) {
        export_datasheet_to_csv(ds, &csv_opts)?
    } else if let Some(res) = doc.result_sheet(&id) {
        export_result_sheet_tidy_csv(res, &csv_opts)
    } else {
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("data or result sheet `{id}` not found"))
            .with_remediation("Provide an existing data or result sheet ID."));
    };

    if let Some(path) = p.path.as_deref() {
        write_atomic_text_file(path, &csv_text)?;
    }
    Ok(CommandOutcome::read_only("table.exportCsv", "exported CSV", json!({ "sheet_id": id.as_str(), "path": p.path, "csv": csv_text })))
}

// ---------------------------------------------------------------------------
// 16. table.importPrism
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ImportPrismParams {
    path: Option<String>,
    bytes_base64: Option<String>,
    bytes: Option<Vec<u8>>,
    merge: Option<bool>,
}

fn schema_import_prism() -> Value {
    obj_schema_with_aliases(
        vec![
            ("path", nullable_schema(non_empty_str_schema("Filesystem path to external archive"))),
            ("bytes_base64", nullable_schema(non_empty_str_schema("Base64-encoded external archive bytes"))),
            ("bytes", array_schema("Raw byte array of archive contents", int_schema("Byte value 0..=255", Some(0), Some(255)), None, None)),
            ("merge", bool_schema("Merge imported sheets into existing document instead of replacing (default false)")),
        ],
        &[],
        vec![("path", &["file_path", "filePath"]), ("bytes_base64", &["bytesBase64", "archive_base64"])],
        true,
    )
}

fn merge_imported_document(doc: &mut ProjectDocument, imported: ProjectDocument) -> Result<(), EngineError> {
    for ds in imported.data_sheets {
        doc.add_data_sheet(ds)?;
    }
    for info in imported.info_sheets {
        doc.add_info_sheet(info)?;
    }
    for res in imported.result_sheets {
        doc.add_result_sheet(res)?;
    }
    for gs in imported.graph_sheets {
        doc.add_graph_sheet(gs)?;
    }
    for ls in imported.layout_sheets {
        doc.add_layout_sheet(ls)?;
    }
    Ok(())
}

fn run_import_prism(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportPrismParams = parse_params(&schema_import_prism(), raw)?;
    let bytes = if let Some(path) = p.path.as_deref() {
        read_binary_file(path, "archive")?
    } else if let Some(b64) = p.bytes_base64.as_deref() {
        decode_base64(b64)?
    } else if let Some(raw_bytes) = p.bytes {
        raw_bytes
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importPrism requires `path` or `bytes_base64`")
            .with_remediation("Supply `path`, `bytes_base64`, or `bytes`."));
    };

    let report = import_prism_bytes(&bytes)?;
    let doc = &mut ctx.session.doc;
    if p.merge.unwrap_or(false) {
        merge_imported_document(doc, report.document)?;
    } else {
        *doc = report.document;
    }
    Ok(CommandOutcome::mutated(
        "table.importPrism",
        "imported external archive",
        json!({
            "title": doc.title,
            "sheet_count": doc.total_sheet_count(),
            "warnings": report.warnings,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 17. table.importPzfx
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ImportPzfxParams {
    xml: Option<String>,
    path: Option<String>,
    bytes_base64: Option<String>,
    merge: Option<bool>,
}

fn schema_import_pzfx() -> Value {
    obj_schema_with_aliases(
        vec![
            ("xml", nullable_schema(str_schema("Inline XML project string"))),
            ("path", nullable_schema(non_empty_str_schema("Filesystem path to XML project file"))),
            ("bytes_base64", nullable_schema(non_empty_str_schema("Base64-encoded XML project bytes"))),
            ("merge", bool_schema("Merge imported sheets into existing document instead of replacing (default false)")),
        ],
        &[],
        vec![("xml", &["text", "content"]), ("path", &["file_path", "filePath"]), ("bytes_base64", &["bytesBase64"])],
        true,
    )
}

fn run_import_pzfx(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportPzfxParams = parse_params(&schema_import_pzfx(), raw)?;
    let report = if let Some(xml) = p.xml.as_deref() {
        import_pzfx_str(xml)?
    } else if let Some(path) = p.path.as_deref() {
        let bytes = read_binary_file(path, "XML file")?;
        import_pzfx_bytes(&bytes)?
    } else if let Some(b64) = p.bytes_base64.as_deref() {
        let bytes = decode_base64(b64)?;
        import_pzfx_bytes(&bytes)?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importPzfx requires `path`, `xml`/`text`, or `bytes_base64`")
            .with_remediation("Provide an XML `path`, inline `xml` string, or `bytes_base64`."));
    };

    let doc = &mut ctx.session.doc;
    if p.merge.unwrap_or(false) {
        merge_imported_document(doc, report.document)?;
    } else {
        *doc = report.document;
    }
    Ok(CommandOutcome::mutated(
        "table.importPzfx",
        "imported XML project",
        json!({
            "title": doc.title,
            "sheet_count": doc.total_sheet_count(),
            "warnings": report.warnings,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 18-21. MultipleVariables Commands
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
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target MultipleVariables sheet identifier")),
            ("col", col_ref_schema("Zero-based variable index or column title")),
            ("title", nullable_schema(str_schema("Fallback variable column title"))),
            ("role", non_empty_str_schema("Role: continuous, categorical, label, calculated, or auto")),
        ],
        &["sheet_id", "role"],
        vec![("sheet_id", &["sheetId"]), ("col", &["column", "column_index", "columnIndex", "index", "variable"])],
        true,
    )
}

fn run_set_variable_role(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetVariableRoleParams = parse_params(&schema_set_variable_role(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let doc = &mut ctx.session.doc;
    let mv = require_mv_table_mut(doc, &id)?;
    let col_idx = resolve_mv_variable_index(mv, p.col.as_ref(), p.title.as_deref())?;
    let var = mv
        .variables
        .get_mut(col_idx)
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("variable column {col_idx} out of range")))?;

    if p.role.eq_ignore_ascii_case("auto") || p.role.eq_ignore_ascii_case("inferred") {
        var.refresh_inferred_role();
    } else {
        var.lock_role(parse_variable_role(&p.role)?);
    }
    let final_role = var.role;
    let role_inferred = var.role_inferred;

    let recalc = recalculate_with_links(doc, false)?;
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
    #[serde(alias = "old", alias = "source")]
    from: String,
    #[serde(alias = "new", alias = "target")]
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
    let pair_item = obj_schema_with_aliases(
        vec![("from", str_schema("Existing level text")), ("to", str_schema("Replacement level text"))],
        &["from", "to"],
        vec![("from", &["old", "source"]), ("to", &["new", "target"])],
        true,
    );
    let mappings_schema = json!({
        "description": "Recode mapping: object `{ old: new }` or array of `{ from, to }`",
        "anyOf": [
            array_schema("Array of {from, to} recode rules", pair_item, None, None),
            { "type": "object", "additionalProperties": { "type": "string" } }
        ]
    });
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target MultipleVariables sheet identifier")),
            ("col", col_ref_schema("Zero-based variable index or column title")),
            ("title", nullable_schema(str_schema("Fallback variable column title"))),
            ("mappings", mappings_schema),
        ],
        &["sheet_id", "mappings"],
        vec![
            ("sheet_id", &["sheetId"]),
            ("col", &["column", "column_index", "columnIndex", "index", "variable"]),
            ("mappings", &["mapping", "map", "recode"]),
        ],
        true,
    )
}

fn run_recode_levels(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: RecodeLevelsParams = parse_params(&schema_recode_levels(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let pairs = p.mappings.into_pairs();
    let doc = &mut ctx.session.doc;
    let mv = require_mv_table_mut(doc, &id)?;
    let col_idx = resolve_mv_variable_index(mv, p.col.as_ref(), p.title.as_deref())?;
    let var = mv
        .variables
        .get_mut(col_idx)
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("variable column {col_idx} out of range")))?;
    let recoded_count = var.recode_levels(&pairs);
    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        "table.recodeLevels",
        "recoded variable levels",
        json!({
            "sheet_id": id.as_str(),
            "column": col_idx,
            "recoded_count": recoded_count,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    ))
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
}

fn schema_set_categorical_order() -> Value {
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
        vec![
            ("sheet_id", &["sheetId"]),
            ("col", &["column", "column_index", "columnIndex", "index", "variable"]),
            ("order_spec", &["orderSpec"]),
            ("levels", &["explicit"]),
            ("order", &["kind", "mode"]),
        ],
        true,
    )
}

fn parse_level_order_from_params(p: &SetCategoricalOrderParams) -> Result<LevelOrderSpec, EngineError> {
    if let Some(spec_val) = &p.order_spec {
        if let Ok(parsed) = serde_json::from_value::<LevelOrderSpec>(spec_val.clone()) {
            return Ok(parsed);
        }
        #[derive(Deserialize)]
        struct NestedOrderSpec {
            levels: Option<Vec<String>>,
            #[serde(alias = "kind", alias = "mode")]
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
    let spec = parse_level_order_from_params(&p)?;
    let doc = &mut ctx.session.doc;
    let mv = require_mv_table_mut(doc, &id)?;
    let col_idx = resolve_mv_variable_index(mv, p.col.as_ref(), p.title.as_deref())?;
    let var = mv
        .variables
        .get_mut(col_idx)
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("variable column {col_idx} out of range")))?;
    var.level_order = spec;
    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        "table.setCategoricalOrder",
        "updated categorical level order",
        json!({
            "sheet_id": id.as_str(),
            "column": col_idx,
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
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target MultipleVariables sheet identifier")),
            ("col", col_ref_schema("Existing variable index or name (optional when `title` creates or matches a column)")),
            ("title", nullable_schema(str_schema("Variable column title (creates a new column if not found and `col` is omitted)"))),
            ("formula", str_schema("Formula expression, e.g. `= log10([Dose])` or `log10(ConcRaw)`")),
        ],
        &["sheet_id", "formula"],
        vec![
            ("sheet_id", &["sheetId"]),
            ("col", &["column", "column_index", "columnIndex", "index", "variable"]),
            ("title", &["name"]),
            ("formula", &["expression"]),
        ],
        true,
    )
}

fn run_set_calculated_variable(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: SetCalculatedVariableParams = parse_params(&schema_set_calculated_variable(), raw)?;
    let id = parse_sheet_id(&p.sheet_id)?;
    let doc = &mut ctx.session.doc;

    let info_snapshot = doc.data_sheet(&id).and_then(|ds| ds.info_sheet_id.as_ref()).and_then(|iid| doc.info_sheet(iid)).cloned();
    let ds = require_unfrozen_data_sheet_mut(doc, &id)?;

    let TableArchetype::MultipleVariables(mv) = &mut ds.archetype else {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("sheet `{id}` is not a MultipleVariables table"))
            .with_remediation("Select a MultipleVariables data sheet."));
    };

    let (col_idx, created_new) = if let Some(ref col_ref) = p.col {
        (resolve_mv_variable_index(mv, Some(col_ref), None)?, false)
    } else if let Some(ref title) = p.title {
        if let Some(existing) = mv.find_variable(title) {
            (existing, false)
        } else {
            let row_count = mv.row_count();
            let col = VariableColumn::new(title.as_str(), row_count)?;
            mv.variables.push(col);
            (mv.variables.len().saturating_sub(1), true)
        }
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.setCalculatedVariable requires `column`/`col` or `title`")
            .with_remediation("Specify a target column index or a `title` for the calculated variable."));
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
    let recalc = recalculate_with_links(doc, false)?;
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
    #[serde(alias = "key")]
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
    obj_schema_with_aliases(
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
        vec![
            ("sheet_id", &["sheetId"]),
            ("title", &["name"]),
            ("row_count", &["rows", "rowCount"]),
            ("replicates", &["k", "subcolumns"]),
            ("x_min", &["xMin"]),
            ("x_max", &["xMax"]),
            ("equation_id", &["equationId", "equation"]),
            ("parameters", &["params"]),
            ("noise_kind", &["noiseKind", "noise"]),
            ("noise_sd", &["noiseSd", "sd"]),
            ("cv_percent", &["cvPercent"]),
            ("auto_info", &["autoInfo"]),
            ("graph_sheet_id", &["graphSheetId"]),
        ],
        true,
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
        let existing_info = doc.data_sheet(&id).and_then(|d| d.info_sheet_id.clone());
        let slot = require_unfrozen_data_sheet_mut(doc, &id)?;
        if let Some(info_id) = existing_info {
            simulated.info_sheet_id = Some(info_id);
        }
        *slot = simulated;
    } else {
        doc.add_data_sheet(simulated)?;
        if p.auto_info.unwrap_or(false) {
            let iid = next_unique_sheet_id(doc, "info")?;
            if let Some(ds) = doc.data_sheet_mut(&id) {
                ds.info_sheet_id = Some(iid.clone());
            }
            doc.add_info_sheet(InfoSheet::new_preseeded(iid, format!("{title} Info"))?)?;
        }
        if let Some(graph_id) = p.graph_sheet_id.as_deref()
            && let Some(ds) = doc.data_sheet(&id).cloned()
        {
            doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data(parse_sheet_id(graph_id)?, &ds)?)?;
        }
        doc.select_in_navigator(&id)?;
    }

    let recalc = recalculate_with_links(doc, false)?;
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
    obj_schema_with_aliases(
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
        vec![
            ("sheet_id", &["sheetId", "id"]),
            ("target_kind", &["targetKind", "kind"]),
            ("allow_lossy", &["allowLossy"]),
            ("preview_only", &["previewOnly", "preview", "dry_run", "dryRun"]),
        ],
        true,
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
        let preview = preview_datasheet_conversion(&doc.data_sheets[idx], target_kind)?;
        return Ok(CommandOutcome::read_only(
            "table.convertArchetype",
            format!("previewed conversion of `{}`", sheet_id.as_str()),
            json!({
                "sheet_id": sheet_id.as_str(),
                "preview_only": true,
                "preview": preview,
            }),
        ));
    }

    ensure_sheet_not_frozen(&doc.data_sheets[idx])?;
    let (mut converted, preview) = convert_datasheet_archetype(&doc.data_sheets[idx], target_kind, allow_lossy)?;
    converted.id = sheet_id.clone();
    doc.data_sheets[idx] = converted;
    let recalc = recalculate_with_links(doc, false)?;
    Ok(CommandOutcome::mutated(
        "table.convertArchetype",
        format!("converted `{}` to {:?}", sheet_id.as_str(), target_kind),
        json!({
            "sheet_id": sheet_id.as_str(),
            "preview_only": false,
            "preview": preview,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 24. table.importPlateMap
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PlateGeometryInput {
    Count(u64),
    Name(String),
}

impl PlateGeometryInput {
    fn resolve(&self) -> Result<PlateGeometry, EngineError> {
        match self {
            Self::Count(96) => Ok(PlateGeometry::Wells96),
            Self::Count(384) => Ok(PlateGeometry::Wells384),
            Self::Count(n) => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported plate well count `{n}` (expected 96 or 384)"))),
            Self::Name(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                "wells96" | "wells_96" | "96" | "96-well" | "96well" => Ok(PlateGeometry::Wells96),
                "wells384" | "wells_384" | "384" | "384-well" | "384well" => Ok(PlateGeometry::Wells384),
                _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported plate geometry `{raw}`"))),
            },
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PlateValuesInput {
    Flat(Vec<Option<f64>>),
    Grid(Vec<Vec<Option<f64>>>),
}

impl PlateValuesInput {
    fn flatten(self) -> Result<Vec<Option<f64>>, EngineError> {
        let flat = match self {
            Self::Flat(v) => v,
            Self::Grid(rows) => rows.into_iter().flatten().collect(),
        };
        for item in &flat {
            if let Some(num) = item
                && !num.is_finite()
            {
                return Err(EngineError::with_code(ErrorCode::BadInput, "plate well values cannot be NaN or infinite"));
            }
        }
        Ok(flat)
    }
}

#[derive(Debug, Deserialize)]
struct ImportPlateMapParams {
    sheet_id: Option<String>,
    title: Option<String>,
    geometry: Option<PlateGeometryInput>,
    values: PlateValuesInput,
    template: Option<Value>,
    roles: Option<Vec<Value>>,
    template_name: Option<String>,
    x_title: Option<String>,
    target_kind: Option<String>,
    subtract_blank_mean: Option<bool>,
}

fn schema_import_plate_map() -> Value {
    let geom_schema = nullable_schema(json!({
        "description": "Plate geometry: 96, 384, `Wells96`, or `Wells384`",
        "anyOf": [
            { "type": "integer", "enum": [96, 384] },
            non_empty_str_schema("Plate geometry string")
        ]
    }));
    let values_schema = json!({
        "description": "Flat (96/384) or 2D (8x12 / 16x24) plate readings array of finite numbers or nulls",
        "anyOf": [
            array_schema("1D array of well readings", nullable_schema(num_schema("Well reading", None, None)), None, None),
            array_schema(
                "2D plate row/col readings",
                array_schema("Row of well readings", nullable_schema(num_schema("Well reading", None, None)), None, None),
                None,
                None,
            )
        ]
    });
    obj_schema_with_aliases(
        vec![
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional target sheet ID"))),
            ("title", nullable_schema(str_schema("Imported sheet title (default `Plate Data`)"))),
            ("geometry", geom_schema),
            ("values", values_schema),
            ("template", nullable_schema(any_json_schema("PlateMapTemplate object or `.lctemplate` JSON string"))),
            ("roles", nullable_schema(array_schema("Array of per-well role definitions", any_json_schema("PlateWellRole"), None, None))),
            ("template_name", nullable_schema(str_schema("Template name when building from `roles`"))),
            ("x_title", nullable_schema(str_schema("X/concentration axis title (default `Concentration`)"))),
            ("target_kind", nullable_schema(str_schema("Target archetype (Grouped, XY, Column, MultipleVariables)"))),
            ("subtract_blank_mean", bool_schema("Subtract mean of Blank wells from Treatment readings")),
        ],
        &["values"],
        vec![
            ("sheet_id", &["sheetId", "id"]),
            ("title", &["name"]),
            ("geometry", &["plate_geometry", "plateGeometry"]),
            ("values", &["readings", "well_values"]),
            ("template", &["plate_template", "plateTemplate"]),
            ("roles", &["wells"]),
            ("template_name", &["templateName"]),
            ("x_title", &["xTitle"]),
            ("target_kind", &["targetKind", "kind"]),
            ("subtract_blank_mean", &["subtractBlankMean"]),
        ],
        true,
    )
}

fn parse_plate_well_role(val: &Value) -> Result<PlateWellRole, EngineError> {
    if let Some(tag) = val.as_str() {
        return match tag.trim().to_ascii_lowercase().as_str() {
            "blank" => Ok(PlateWellRole::Blank),
            "positivecontrol" | "positive_control" | "positive-control" | "pos_ctrl" | "positive" => Ok(PlateWellRole::PositiveControl),
            "excluded" | "unused" | "skip" => Ok(PlateWellRole::Excluded),
            _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown plate well role `{tag}`"))),
        };
    }
    if let Ok(role) = serde_json::from_value::<PlateWellRole>(val.clone()) {
        return Ok(role);
    }
    #[derive(Deserialize)]
    struct RoleObj {
        #[serde(alias = "type", alias = "kind")]
        role: Option<String>,
        #[serde(alias = "groupName", alias = "group")]
        group_name: Option<String>,
        #[serde(alias = "dilutionX", alias = "x", alias = "concentration")]
        dilution_x: Option<f64>,
        #[serde(alias = "rep")]
        replicate: Option<usize>,
    }
    let obj: RoleObj =
        serde_json::from_value(val.clone()).map_err(|_| EngineError::with_code(ErrorCode::BadInput, "plate well role must be a string or object"))?;
    if let Some(kind) = obj.role.as_deref() {
        match kind.trim().to_ascii_lowercase().as_str() {
            "blank" => return Ok(PlateWellRole::Blank),
            "positivecontrol" | "positive_control" | "positive-control" | "pos_ctrl" | "positive" => {
                return Ok(PlateWellRole::PositiveControl);
            }
            "excluded" | "unused" | "skip" => return Ok(PlateWellRole::Excluded),
            "treatment" => {}
            _ => return Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown plate well role `{kind}`"))),
        }
    }
    let group_name = obj.group_name.ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "treatment well role requires `group_name`"))?;
    let dilution_x = obj
        .dilution_x
        .filter(|x| x.is_finite())
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "treatment well role requires finite `dilution_x`"))?;
    Ok(PlateWellRole::Treatment { group_name, dilution_x, replicate: obj.replicate.unwrap_or(1) })
}

fn parse_plate_template_obj(val: &Value, default_name: &str) -> Result<PlateMapTemplate, EngineError> {
    if let Ok(tpl) = serde_json::from_value::<PlateMapTemplate>(val.clone()) {
        tpl.validate()?;
        return Ok(tpl);
    }
    #[derive(Deserialize)]
    struct TplObj {
        #[serde(alias = "plate_geometry", alias = "plateGeometry")]
        geometry: PlateGeometryInput,
        #[serde(alias = "title")]
        name: Option<String>,
        #[serde(alias = "xTitle")]
        x_title: Option<String>,
        #[serde(alias = "roles")]
        wells: Vec<Value>,
    }
    let obj: TplObj = serde_json::from_value(val.clone())
        .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid plate `template` object: {err}")))?;
    let geometry = obj.geometry.resolve()?;
    let wells: Vec<PlateWellRole> = obj.wells.iter().map(parse_plate_well_role).collect::<Result<_, _>>()?;
    PlateMapTemplate::from_wells(obj.name.as_deref().unwrap_or(default_name), geometry, obj.x_title.as_deref().unwrap_or("Concentration"), wells)
        .map_err(EngineError::from)
}

fn resolve_plate_template(p: &ImportPlateMapParams, default_name: &str) -> Result<(PlateGeometry, PlateMapTemplate), EngineError> {
    let geom_override = p.geometry.as_ref().map(PlateGeometryInput::resolve).transpose()?;
    if let Some(tpl_val) = &p.template {
        let tpl = if let Some(json_str) = tpl_val.as_str() {
            PlateMapTemplate::from_lctemplate_json(json_str)?
        } else {
            parse_plate_template_obj(tpl_val, default_name)?
        };
        let geometry = geom_override.unwrap_or(tpl.geometry);
        return Ok((geometry, tpl));
    }
    let geometry = geom_override
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "table.importPlateMap requires `geometry` when `template` is omitted"))?;
    let roles_arr =
        p.roles.as_ref().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "table.importPlateMap requires `template` or `roles`"))?;
    let wells: Vec<PlateWellRole> = roles_arr.iter().map(parse_plate_well_role).collect::<Result<_, _>>()?;
    let tpl_name = p.template_name.as_deref().unwrap_or(default_name);
    let x_title = p.x_title.as_deref().unwrap_or("Concentration");
    let tpl = PlateMapTemplate::from_wells(tpl_name, geometry, x_title, wells)?;
    Ok((geometry, tpl))
}

fn assign_and_add_imported_sheet(
    doc: &mut ProjectDocument,
    mut sheet: DataSheet,
    explicit_sheet_id: Option<&str>,
) -> Result<(SheetId, String, usize, usize), EngineError> {
    if let Some(raw_id) = explicit_sheet_id {
        sheet.id = parse_sheet_id(raw_id)?;
    } else {
        sheet.id = next_unique_sheet_id_excluding(doc, sheet.id.as_str(), &[])?;
    }
    let sheet_id = sheet.id.clone();
    let title = sheet.title.clone();
    let rows = sheet.archetype.row_count();
    let cols = sheet.archetype.column_count();
    doc.add_data_sheet(sheet)?;
    Ok((sheet_id, title, rows, cols))
}

fn run_import_plate_map(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportPlateMapParams = parse_params(&schema_import_plate_map(), raw)?;
    let title = p.title.as_deref().unwrap_or("Plate Data");
    validate_title(title)?;
    let (geometry, template) = resolve_plate_template(&p, title)?;
    let values = p.values.flatten()?;
    let target_kind = match p.target_kind.as_deref() {
        Some(raw_kind) => parse_target_archetype_kind(raw_kind)?,
        None => TargetArchetypeKind::Grouped,
    };
    let subtract_blank_mean = p.subtract_blank_mean.unwrap_or(false);
    let imported = map_plate_to_datasheet(title, geometry, &values, &template, target_kind, subtract_blank_mean)?;
    let doc = &mut ctx.session.doc;
    let (sheet_id, sheet_title, rows, cols) = assign_and_add_imported_sheet(doc, imported, p.sheet_id.as_deref())?;

    Ok(CommandOutcome::mutated(
        "table.importPlateMap",
        format!("imported plate map into `{}`", sheet_id.as_str()),
        json!({
            "sheet_id": sheet_id.as_str(),
            "title": sheet_title,
            "geometry": geometry,
            "target_kind": target_kind,
            "subtract_blank_mean": subtract_blank_mean,
            "rows": rows,
            "cols": cols,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 25. table.importTidyLong
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum TidyColRef {
    Index(usize),
    Name(String),
}

impl TidyColRef {
    fn resolve(&self, headers: &[String], key_label: &str) -> Result<usize, EngineError> {
        match self {
            Self::Index(idx) => Ok(*idx),
            Self::Name(name) => {
                if let Some(pos) = headers.iter().position(|h| h == name) {
                    return Ok(pos);
                }
                let needle = name.trim().to_ascii_lowercase();
                headers
                    .iter()
                    .position(|h| h.trim().to_ascii_lowercase() == needle)
                    .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("column header `{name}` for `{key_label}` not found")))
            }
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct TidyMappingInput {
    #[serde(alias = "xCol")]
    x_col: Option<TidyColRef>,
    #[serde(alias = "groupCol")]
    group_col: Option<TidyColRef>,
    #[serde(alias = "replicateCol")]
    replicate_col: Option<TidyColRef>,
    #[serde(alias = "valueCol")]
    value_col: Option<TidyColRef>,
    #[serde(alias = "excludedCol")]
    excluded_col: Option<TidyColRef>,
}

#[derive(Debug, Deserialize)]
struct ImportTidyLongParams {
    sheet_id: Option<String>,
    title: Option<String>,
    target_kind: Option<String>,
    headers: Option<Vec<String>>,
    rows: Option<Vec<Vec<Value>>>,
    csv: Option<String>,
    path: Option<String>,
    delimiter: Option<String>,
    decimal_sep: Option<String>,
    normalize_decimal: Option<bool>,
    mapping: Option<TidyMappingInput>,
    #[serde(flatten)]
    flat_mapping: TidyMappingInput,
}

fn schema_import_tidy_long() -> Value {
    let tidy_col_schema = nullable_schema(col_ref_schema("Zero-based header index or header name"));
    let mapping_schema = obj_schema_with_aliases(
        vec![
            ("x_col", tidy_col_schema.clone()),
            ("group_col", tidy_col_schema.clone()),
            ("replicate_col", tidy_col_schema.clone()),
            ("value_col", tidy_col_schema.clone()),
            ("excluded_col", tidy_col_schema.clone()),
        ],
        &[],
        vec![
            ("x_col", &["xCol"]),
            ("group_col", &["groupCol"]),
            ("replicate_col", &["replicateCol"]),
            ("value_col", &["valueCol"]),
            ("excluded_col", &["excludedCol"]),
        ],
        true,
    );
    obj_schema_with_aliases(
        vec![
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional target sheet ID"))),
            ("title", nullable_schema(str_schema("Imported sheet title (default `Tidy Import`)"))),
            ("target_kind", nullable_schema(str_schema("Target archetype (Grouped, Column, XY, MultipleVariables)"))),
            ("headers", nullable_schema(array_schema("Column header names when using `rows`", str_schema("Header name"), None, None))),
            (
                "rows",
                nullable_schema(array_schema(
                    "2D tidy record rows",
                    array_schema("Record row", any_json_schema("Scalar cell"), None, None),
                    None,
                    None,
                )),
            ),
            ("csv", nullable_schema(str_schema("Inline delimited tidy table text"))),
            ("path", nullable_schema(non_empty_str_schema("Filesystem path to delimited tidy file"))),
            ("delimiter", delimiter_schema()),
            ("decimal_sep", decimal_sep_schema()),
            ("normalize_decimal", bool_schema("Infer European decimal separator when ambiguous")),
            ("mapping", nullable_schema(mapping_schema)),
            ("x_col", tidy_col_schema.clone()),
            ("group_col", tidy_col_schema.clone()),
            ("replicate_col", tidy_col_schema.clone()),
            ("value_col", tidy_col_schema.clone()),
            ("excluded_col", tidy_col_schema),
        ],
        &[],
        vec![
            ("sheet_id", &["sheetId", "id"]),
            ("title", &["name"]),
            ("target_kind", &["targetKind", "kind"]),
            ("csv", &["text", "tsv", "source"]),
            ("path", &["file_path", "filePath"]),
            ("decimal_sep", &["decimalSep", "decimal_separator", "decimalSeparator"]),
            ("normalize_decimal", &["normalizeDecimal"]),
            ("mapping", &["column_mapping", "columnMapping"]),
            ("x_col", &["xCol"]),
            ("group_col", &["groupCol"]),
            ("replicate_col", &["replicateCol"]),
            ("value_col", &["valueCol"]),
            ("excluded_col", &["excludedCol"]),
        ],
        true,
    )
}

fn json_scalar_to_string(val: &Value) -> Result<String, EngineError> {
    if let Some(s) = val.as_str() {
        return Ok(s.to_owned());
    }
    if val.is_number() {
        return Ok(val.to_string());
    }
    if let Some(b) = val.as_bool() {
        return Ok(b.to_string());
    }
    if val.is_null() {
        return Ok(String::new());
    }
    Err(EngineError::with_code(ErrorCode::BadInput, "tidy table cell values must be JSON scalars"))
}

fn parse_explicit_tidy_decimal(raw: Option<&str>) -> Result<Option<char>, EngineError> {
    let Some(s) = raw else {
        return Ok(None);
    };
    match s.trim() {
        "." | "dot" | "period" => Ok(Some('.')),
        "," | "comma" => Ok(Some(',')),
        "auto" => Ok(None),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported decimal separator `{s}` (expected `.` or `,`)"))),
    }
}

struct TidyInputData {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    decimal_sep: char,
}

fn extract_tidy_data(p: &ImportTidyLongParams) -> Result<TidyInputData, EngineError> {
    let explicit_dec = parse_explicit_tidy_decimal(p.decimal_sep.as_deref())?;
    if let Some(rows_arr) = &p.rows {
        let mut rows = Vec::with_capacity(rows_arr.len());
        for row_val in rows_arr {
            let parsed_row: Vec<String> = row_val.iter().map(json_scalar_to_string).collect::<Result<_, _>>()?;
            rows.push(parsed_row);
        }
        let headers = if let Some(h) = &p.headers {
            h.clone()
        } else if let Some(first_row) = rows.first() {
            (0..first_row.len()).map(|i| format!("col_{i}")).collect()
        } else {
            return Err(EngineError::with_code(ErrorCode::BadInput, "table.importTidyLong requires `headers` when `rows` is empty"));
        };
        return Ok(TidyInputData { headers, rows, decimal_sep: explicit_dec.unwrap_or('.') });
    }

    let text = if let Some(inline) = &p.csv {
        inline.clone()
    } else if let Some(path) = &p.path {
        read_utf8_file(path, "tidy file")?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "table.importTidyLong requires `rows` or `csv`")
            .with_remediation("Supply `rows`, inline `csv` text, or a file `path`."));
    };

    let delimiter = resolve_delimiter(p.delimiter.as_deref(), &text)?;
    let decimal_sep = match explicit_dec {
        Some(c) => c,
        None => resolve_decimal_char(p.decimal_sep.as_deref(), p.normalize_decimal.unwrap_or(false), &text, delimiter)?,
    };
    let opts = CsvOptions {
        delimiter,
        quote_char: '"',
        decimal_separator: decimal_sep,
        thousands_separator: None,
        header_rows: 1,
        has_row_titles: false,
        parse_asterisk_exclusions: false,
        missing_sentinels: Vec::new(),
    };
    let mut records = parse_delimited_records(&text, &opts)?;
    if records.len() < 2 {
        return Err(EngineError::with_code(ErrorCode::InsufficientData, "tidy CSV input must contain a header row and at least one data row"));
    }
    let headers = records.remove(0);
    Ok(TidyInputData { headers, rows: records, decimal_sep })
}

fn resolve_tidy_mapping(p: &ImportTidyLongParams, headers: &[String]) -> Result<TidyColumnMapping, EngineError> {
    let m = p.mapping.as_ref().unwrap_or(&p.flat_mapping);
    let group_col = m
        .group_col
        .as_ref()
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing required tidy column `group_col`"))?
        .resolve(headers, "group_col")?;
    let value_col = m
        .value_col
        .as_ref()
        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "missing required tidy column `value_col`"))?
        .resolve(headers, "value_col")?;
    Ok(TidyColumnMapping {
        x_col: m.x_col.as_ref().map(|c| c.resolve(headers, "x_col")).transpose()?,
        group_col,
        replicate_col: m.replicate_col.as_ref().map(|c| c.resolve(headers, "replicate_col")).transpose()?,
        value_col,
        excluded_col: m.excluded_col.as_ref().map(|c| c.resolve(headers, "excluded_col")).transpose()?,
    })
}

fn run_import_tidy_long(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ImportTidyLongParams = parse_params(&schema_import_tidy_long(), raw)?;
    let title = p.title.as_deref().unwrap_or("Tidy Import");
    validate_title(title)?;
    let target_kind = match p.target_kind.as_deref() {
        Some(raw_kind) => parse_target_archetype_kind(raw_kind)?,
        None => TargetArchetypeKind::Grouped,
    };
    let tidy = extract_tidy_data(&p)?;
    let mapping = resolve_tidy_mapping(&p, &tidy.headers)?;

    let imported = pivot_tidy_long_to_datasheet(title, &tidy.headers, &tidy.rows, &mapping, target_kind, tidy.decimal_sep)?;
    let doc = &mut ctx.session.doc;
    let (sheet_id, sheet_title, sheet_rows, sheet_cols) = assign_and_add_imported_sheet(doc, imported, p.sheet_id.as_deref())?;

    Ok(CommandOutcome::mutated(
        "table.importTidyLong",
        format!("imported tidy long table into `{}`", sheet_id.as_str()),
        json!({
            "sheet_id": sheet_id.as_str(),
            "title": sheet_title,
            "target_kind": target_kind,
            "rows": sheet_rows,
            "cols": sheet_cols,
            "mapping": mapping,
        }),
    ))
}

// ---------------------------------------------------------------------------
// 26–28. table.randomizeGroups, table.randomSampleRows, table.simulateDistributions
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
    } else if let Some(first) = doc.info_sheets.first() {
        first.id.clone()
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
        #[serde(alias = "name")]
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
    #[serde(default, alias = "n_subjects", alias = "count")]
    subject_count: Option<usize>,
    #[serde(default)]
    scheme: Option<String>,
    #[serde(default)]
    block_sizes: Option<Vec<usize>>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default, alias = "sheetId", alias = "id")]
    sheet_id: Option<String>,
    #[serde(default, alias = "name")]
    title: Option<String>,
    #[serde(default, alias = "info_sheet")]
    info_sheet_id: Option<String>,
}

fn schema_randomize_groups() -> Value {
    let group_item = json!({
        "anyOf": [
            { "type": "string", "minLength": 1 },
            {
                "type": "object",
                "properties": {
                    "label": { "type": "string", "minLength": 1 },
                    "weight": { "type": "integer", "minimum": 1 }
                },
                "required": ["label"],
                "additionalProperties": true
            }
        ]
    });
    obj_schema_with_aliases(
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
        vec![("sheet_id", &["sheetId", "id"]), ("title", &["name"]), ("subject_count", &["n_subjects", "count"]), ("info_sheet_id", &["info_sheet"])],
        true,
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
    let info_id = upsert_info_constants(doc, p.info_sheet_id.as_deref(), None, true, &format!("{title} Info"), &rand_out.arrive_constants)?
        .ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "failed resolving InfoSheet for ARRIVE constants"))?;
    rand_out.data_sheet.info_sheet_id = Some(info_id.clone());
    doc.add_data_sheet(rand_out.data_sheet)?;
    let recalc = recalculate_with_links(doc, false)?;

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
    #[serde(alias = "source_sheet", alias = "data_sheet_id")]
    source_sheet_id: String,
    #[serde(alias = "n_samples", alias = "count")]
    sample_size: usize,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default, alias = "stratify_col")]
    stratify_column: Option<usize>,
    #[serde(default)]
    proportional_allocation: Option<bool>,
    #[serde(default, alias = "sheetId", alias = "id")]
    sheet_id: Option<String>,
    #[serde(default, alias = "name")]
    title: Option<String>,
    #[serde(default, alias = "info_sheet")]
    info_sheet_id: Option<String>,
}

fn schema_random_sample_rows() -> Value {
    obj_schema_with_aliases(
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
        vec![
            ("source_sheet_id", &["source_sheet", "data_sheet_id"]),
            ("sample_size", &["n_samples", "count"]),
            ("stratify_column", &["stratify_col"]),
            ("sheet_id", &["sheetId", "id"]),
            ("title", &["name"]),
            ("info_sheet_id", &["info_sheet"]),
        ],
        true,
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

    let info_id = upsert_info_constants(doc, p.info_sheet_id.as_deref(), src_info_id.as_ref(), false, &format!("{title} Info"), &provenance)?;
    if let Some(ref iid) = info_id {
        sampled.info_sheet_id = Some(iid.clone());
    }
    doc.add_data_sheet(sampled)?;
    let recalc = recalculate_with_links(doc, false)?;

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
    #[serde(alias = "row_count", alias = "rows")]
    n_rows: usize,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default, alias = "multiple_variables")]
    use_multiple_variables: Option<bool>,
    #[serde(default, alias = "sheetId", alias = "id")]
    sheet_id: Option<String>,
    #[serde(default, alias = "name")]
    title: Option<String>,
}

fn find_dist_f64(sources: &[&serde_json::Map<String, Value>], keys: &[&str], field_name: &str, idx: usize) -> Result<f64, EngineError> {
    for src in sources {
        for key in keys {
            if let Some(val) = src.get(*key) {
                return val
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}].{field_name} must be a finite number")));
            }
        }
    }
    Err(EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}] missing numeric parameter `{field_name}`")))
}

fn find_dist_u64(sources: &[&serde_json::Map<String, Value>], keys: &[&str], field_name: &str, idx: usize) -> Result<u64, EngineError> {
    for src in sources {
        for key in keys {
            if let Some(val) = src.get(*key) {
                return val.as_u64().ok_or_else(|| {
                    EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}].{field_name} must be a non-negative integer"))
                });
            }
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
        .or_else(|| obj.get("title"))
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
            .or_else(|| dist_obj.get("type"))
            .or_else(|| dist_obj.get("distribution"))
            .and_then(Value::as_str)
            .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}].distribution object must specify `kind`")))?
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("columns[{idx}].distribution must be a distribution name or object")));
    };
    sources.push(obj);

    let kind_key: String = kind_raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let distribution = match kind_key.as_str() {
        "uniform" => SimulatedDistribution::Uniform {
            min: find_dist_f64(&sources, &["min"], "min", idx)?,
            max: find_dist_f64(&sources, &["max"], "max", idx)?,
        },
        "gaussian" | "normal" => SimulatedDistribution::Gaussian {
            mean: find_dist_f64(&sources, &["mean", "mu"], "mean", idx)?,
            sd: find_dist_f64(&sources, &["sd", "std", "sigma"], "sd", idx)?,
        },
        "lognormal" => SimulatedDistribution::LogNormal {
            mean_log: find_dist_f64(&sources, &["mean_log", "meanlog", "mu"], "mean_log", idx)?,
            sd_log: find_dist_f64(&sources, &["sd_log", "sdlog", "sigma"], "sd_log", idx)?,
        },
        "binomial" => SimulatedDistribution::Binomial {
            trials: find_dist_u64(&sources, &["trials", "n"], "trials", idx)?,
            p: find_dist_f64(&sources, &["p", "prob", "probability"], "p", idx)?,
        },
        "poisson" => SimulatedDistribution::Poisson { lambda: find_dist_f64(&sources, &["lambda", "rate", "mean"], "lambda", idx)? },
        "exponential" | "exp" => SimulatedDistribution::Exponential { rate: find_dist_f64(&sources, &["rate", "lambda"], "rate", idx)? },
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
            }
        },
        "required": ["name", "distribution"],
        "additionalProperties": true
    });
    obj_schema_with_aliases(
        vec![
            ("columns", array_schema("Simulated column distribution specifications", col_schema, Some(1), None)),
            ("n_rows", int_schema("Number of rows to simulate per column", Some(1), None)),
            ("seed", nullable_schema(int_schema("Deterministic 64-bit PRNG seed (default 1)", Some(0), None))),
            ("use_multiple_variables", nullable_schema(bool_schema("Create MultipleVariables table instead of Column table (default false)"))),
            ("sheet_id", nullable_schema(non_empty_str_schema("Optional destination DataSheet ID"))),
            ("title", nullable_schema(str_schema("Optional destination sheet title (default `Simulated Distributions`)"))),
        ],
        &["columns", "n_rows"],
        vec![
            ("n_rows", &["row_count", "rows"]),
            ("use_multiple_variables", &["multiple_variables"]),
            ("sheet_id", &["sheetId", "id"]),
            ("title", &["name"]),
        ],
        true,
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
    doc.add_data_sheet(sheet)?;
    let recalc = recalculate_with_links(doc, false)?;

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
        s.execute("table.setCell", json!({ "sheetId": "xy-1", "coord": { "row": 0, "column": 0 }, "value": 12.5 })).expect("setCell alias");
        s.execute("table.setCells", json!({ "sheet_id": "col-1", "grid": [[1.0, 2.0], [3.0, 4.0]] })).expect("setCells grid");
        s.execute("table.setXCells", json!({ "sheet_id": "xy-1", "values": [1.0, 2.0, 3.0] })).expect("setXCells");
        s.execute("table.setRowTitles", json!({ "sheet_id": "col-1", "titles": ["R1", null] })).expect("setRowTitles");
        let gt_out = s.execute("table.setGroupTitles", json!({ "sheet_id": "col-1", "titles": ["G1", "G2"] })).expect("setGroupTitles");
        assert_eq!(gt_out.command_id, "table.setGroupTitles");
        let st_out = s.execute("table.setTitles", json!({ "sheet_id": "xy-1", "x_title": "Dose", "y_titles": ["Resp"] })).expect("setTitles");
        assert_eq!(st_out.command_id, "table.setTitles");
        s.execute("table.setSubcolumnTitles", json!({ "sheet_id": "nest-1", "col": 0, "titles": ["A", "B", "C"] })).expect("setSubcolumnTitles");

        s.execute("table.excludeCells", json!({ "sheet_id": "col-1", "row": 0, "col": 0 })).expect("exclude");
        s.execute("table.excludeCellsWithReason", json!({ "sheet_id": "col-1", "reason": "outlier", "coords": [[1, 0]] }))
            .expect("excludeWithReason");
        let audit = s.execute("table.exclusionAuditReport", json!({ "sheet_id": "col-1" })).expect("audit");
        assert!(!audit.mutated);
        assert_eq!(audit.payload.get("total_excluded").and_then(Value::as_u64), Some(2));
        assert_eq!(audit.payload.get("annotated_count").and_then(Value::as_u64), Some(1));

        s.execute("table.clearCells", json!({ "sheet_id": "col-1", "row_start": 0, "row_end": 0, "col_start": 1, "col_end": 1 })).expect("clear");
        s.execute("table.paste", json!({ "sheet_id": "col-1", "tsv": "5\t6\n7\t8" })).expect("paste");
        s.execute("table.pasteLink", json!({ "target_sheet_id": "xy-1", "source_sheet_id": "col-1" })).expect("pasteLink");

        let tmp_dir = std::env::temp_dir().join(format!("lc-w9b-table-test-{}", std::process::id()));
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
        s.execute("table.recodeLevels", json!({ "sheet_id": "mv-1", "col": 0, "mapping": { "A": "Alpha" } })).expect("recodeLevels");
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
}
