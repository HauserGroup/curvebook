//! Sheet creation, renaming, deletion, duplication, pipeline cloning, freezing, reordering, and selection commands.

use std::collections::{BTreeSet, HashMap, HashSet};

use curvebook_doc::{
    AnalysisKind, CellCoord, CloneDataSheetOptions, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, DocError, ErrorCode, GraphAnnotation,
    GraphSheet, GroupedTable, HookSource, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, LayoutItem,
    LayoutSheet, LinkSource, MultipleVariablesTable, NestedTable, PartsOfWholeTable, PlacementMode, ProjectDocument, ResultCell, ResultSheet,
    ResultStatus, ResultTab, SheetCellRange, SheetFolderKind, SheetId, SheetSection, SubcolumnMode, SurvivalTable, TableArchetype, TableKind,
    VariableRole, XColumnMode, XyTable, extract_template_hooks, validate_cell_grid, validate_data_sets, validate_project_cells, validate_rows,
    validate_sheet_cells, validate_subcolumns, validate_title,
};
use curvebook_transform::{ImportPipelineOptions, ImportSourcePayload, apply_import_link_pipeline};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cmd::helpers::{
    always_enabled, apply_result_tab_metadata_to_sheet, bad_input, bad_input_ptr, clip_safe_title, ensure_sheet_not_frozen, has_result_sheet,
    link_data_sheet_to_info, parse_sheet_id, remap_analysis_hooks, remap_graph_spec_references, remap_layout_spec_references, require_data_sheet_mut,
    require_result_sheet, sanitize_title_controls, select_result_tab, sheet_id_exists, sync_info_linked_data_sheets,
};
use crate::cmd::params::{NonEmptyStrSchema, command_schema, command_schema_with_aliases, parse_params};
use crate::cmd::project::{RecalcSummary, recalculate_dirty_with_links};
use crate::cmd::{CommandGroup, CommandSpec, Ctx, Effects, command_specs};
use crate::{CommandOutcome, EngineError};

fn next_unique_id_with_reserved(doc: &ProjectDocument, base: &str, reserved: &HashSet<String>) -> Result<SheetId, EngineError> {
    let sanitized: String =
        base.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' }).collect();
    let clean_base = sanitized.trim_matches('-');
    let prefix = if clean_base.is_empty() { "sheet" } else { clean_base };
    let truncated = if prefix.len() > 40 { prefix.get(..40).unwrap_or(prefix) } else { prefix };

    for n in 1..10_000_usize {
        let candidate_str = format!("{truncated}-{n}");
        if reserved.contains(&candidate_str) {
            continue;
        }
        let Ok(candidate) = SheetId::try_new(candidate_str) else {
            continue;
        };
        if !sheet_id_exists(doc, &candidate) {
            return Ok(candidate);
        }
    }
    Err(EngineError::with_code(ErrorCode::ResourceLimit, "unable to allocate unique sheet identifier"))
}

fn next_unique_sheet_id_hyphen(doc: &ProjectDocument, prefix: &str) -> Result<SheetId, EngineError> {
    next_unique_id_with_reserved(doc, prefix, &HashSet::new())
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
enum VariableInitInput {
    Name(String),
    Object { title: Option<String>, role: Option<String> },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateDataParams {
    /// Optional explicit DataSheet ID (auto-generated if omitted).
    #[schemars(length(min = 1))]
    sheet_id: Option<String>,
    /// Optional human-readable table title.
    title: Option<String>,
    /// Table archetype (`xy`, `column`, `grouped`, `contingency`, `survival`, `parts_of_whole`, `multiple_variables`, `nested`).
    kind: Option<String>,
    /// Initial row count.
    #[schemars(range(min = 1))]
    rows: Option<usize>,
    /// Initial column / data-set count.
    #[schemars(range(min = 1))]
    cols: Option<usize>,
    /// Raw replicate subcolumns (or biological units for nested).
    #[schemars(range(min = 1))]
    replicates: Option<usize>,
    /// Replicate subcolumn mode string or structured SubcolumnMode object.
    subcolumn_mode: Option<Value>,
    /// Optional X-column mode string or structured XColumnMode object.
    x_mode: Option<Value>,
    /// Optional X-axis column title for XY tables.
    x_title: Option<String>,
    /// Column table design hint (`unpaired`, `paired`, `precomputed`).
    design_hint: Option<Value>,
    /// Row factor title for Grouped/Contingency tables.
    row_factor_title: Option<String>,
    /// Column factor title for Grouped/Contingency tables.
    column_factor_title: Option<String>,
    /// Elapsed time title for Survival tables.
    time_title: Option<String>,
    /// Numeric status code denoting Event rows on Survival tables.
    event_code: Option<i64>,
    /// Numeric status code denoting Censored rows on Survival tables.
    censored_code: Option<i64>,
    /// Variable definitions for MultipleVariables tables.
    variables: Option<Vec<VariableInitInput>>,
    /// Auto-create both linked InfoSheet and uninitialized GraphSheet.
    auto_family: Option<bool>,
    /// Auto-create or attach paired InfoSheet.
    auto_info: Option<bool>,
    /// Auto-create uninitialized GraphSheet for this table.
    auto_graph: Option<bool>,
    /// Optional explicit InfoSheet ID to link or create.
    #[schemars(length(min = 1))]
    info_sheet_id: Option<String>,
    /// Optional explicit GraphSheet ID to create.
    #[schemars(length(min = 1))]
    graph_sheet_id: Option<String>,
}

fn create_data_schema() -> Value {
    // Retained until Stage 4 retires r16_5_cli_and_callers.rs
    command_schema_with_aliases::<CreateDataParams>(&[("kind", &["archetype"])])
}

fn parse_table_kind(raw: &str) -> Result<TableKind, EngineError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "xy" => Ok(TableKind::Xy),
        "column" | "col" => Ok(TableKind::Column),
        "grouped" | "group" => Ok(TableKind::Grouped),
        "contingency" => Ok(TableKind::Contingency),
        "survival" | "kaplan_meier" | "kaplanmeier" => Ok(TableKind::Survival),
        "parts_of_whole" | "partsofwhole" | "parts-of-whole" | "pie" => Ok(TableKind::PartsOfWhole),
        "multiple_variables" | "multiplevariables" | "multiple-variables" | "multi" | "multivariate" => Ok(TableKind::MultipleVariables),
        "nested" => Ok(TableKind::Nested),
        other => Err(bad_input(format!("unsupported table archetype `{other}`"))
            .with_remediation("Use one of: xy, column, grouped, contingency, survival, parts_of_whole, multiple_variables, nested.")),
    }
}

fn parse_subcolumn_mode(raw: Option<&Value>, replicates: usize) -> Result<SubcolumnMode, EngineError> {
    let Some(val) = raw else {
        return Ok(if replicates <= 1 { SubcolumnMode::Single } else { SubcolumnMode::RawReplicates(replicates) });
    };
    if val.is_null() {
        return Ok(if replicates <= 1 { SubcolumnMode::Single } else { SubcolumnMode::RawReplicates(replicates) });
    }
    if let Some(s) = val.as_str() {
        return match s.trim().to_ascii_lowercase().as_str() {
            "single" => Ok(SubcolumnMode::Single),
            "raw" | "raw_replicates" | "rawreplicates" | "replicate" | "replicates" => {
                Ok(if replicates <= 1 { SubcolumnMode::Single } else { SubcolumnMode::RawReplicates(replicates) })
            }
            "mean_sd" | "mean_sd_n" | "meansdn" => Ok(SubcolumnMode::MeanSdN),
            "mean_sem" | "mean_sem_n" | "meansemn" => Ok(SubcolumnMode::MeanSemN),
            "mean_cv" | "mean_cv_n" | "meancvn" => Ok(SubcolumnMode::MeanCvN),
            "mean_low_high" | "meanlowhigh" => Ok(SubcolumnMode::MeanLowHigh),
            "upper_lower" | "upper_lower_limits" | "upperlowerlimits" | "mean_upper_lower" | "meanupperlower" => Ok(SubcolumnMode::UpperLowerLimits),
            "median_iqr" | "medianiqr" => Ok(SubcolumnMode::MedianIqr),
            other => Err(bad_input(format!("unsupported subcolumn mode `{other}`"))),
        };
    }
    serde_json::from_value::<SubcolumnMode>(val.clone()).map_err(|err| bad_input(format!("invalid subcolumn_mode: {err}")))
}

fn parse_column_design_hint(raw: Option<&Value>) -> Result<ColumnDesignHint, EngineError> {
    let Some(val) = raw else {
        return Ok(ColumnDesignHint::UnpairedReplicates);
    };
    if val.is_null() {
        return Ok(ColumnDesignHint::UnpairedReplicates);
    }
    if let Some(s) = val.as_str() {
        return match s.trim().to_ascii_lowercase().as_str() {
            "unpaired" | "unpaired_replicates" | "unpairedreplicates" => Ok(ColumnDesignHint::UnpairedReplicates),
            "paired" | "repeated" | "paired_repeated_measures" | "pairedrepeatedmeasures" => Ok(ColumnDesignHint::PairedRepeatedMeasures),
            "precomputed" | "precomputed_error" | "precomputederror" => Ok(ColumnDesignHint::PrecomputedError),
            other => Err(bad_input(format!("unsupported design_hint `{other}`"))),
        };
    }
    serde_json::from_value::<ColumnDesignHint>(val.clone()).map_err(|err| bad_input(format!("invalid design_hint: {err}")))
}

fn parse_x_mode(raw: Option<&Value>) -> Result<XColumnMode, EngineError> {
    let Some(val) = raw else {
        return Ok(XColumnMode::default());
    };
    if val.is_null() {
        return Ok(XColumnMode::default());
    }
    if let Some(s) = val.as_str() {
        return match s.trim().to_ascii_lowercase().as_str() {
            "numbers" | "numeric" => Ok(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }),
            "date" | "time" | "elapsed" | "date_or_elapsed_time" | "dateorelapsedtime" => {
                Ok(XColumnMode::DateOrElapsedTime { unit_label: String::from("Time") })
            }
            "sequence" | "generated" | "generated_sequence" => {
                Ok(XColumnMode::GeneratedSequence { start: 0.0, step: 1.0, progression: curvebook_doc::SequenceProgression::Arithmetic })
            }
            other => Err(bad_input(format!("unsupported x_mode `{other}`"))),
        };
    }
    serde_json::from_value::<XColumnMode>(val.clone()).map_err(|err| bad_input(format!("invalid x_mode: {err}")))
}

fn parse_variable_role(raw: &str) -> Result<VariableRole, EngineError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "continuous" | "numeric" | "number" => Ok(VariableRole::Continuous),
        "categorical" | "category" | "factor" => Ok(VariableRole::Categorical),
        "label" | "identifier" | "id" | "text" => Ok(VariableRole::Label),
        "calculated" | "formula" => Ok(VariableRole::Calculated),
        other => Err(bad_input(format!("unsupported variable role `{other}`"))),
    }
}

fn document_used_cells(doc: &ProjectDocument) -> usize {
    doc.data_sheets.iter().fold(0usize, |acc, sheet| acc.saturating_add(sheet.archetype.allocated_cell_slots()))
}

fn validate_sheet_and_project_budget(
    existing_cells: usize,
    rows: usize,
    cols: usize,
    subcolumns: usize,
    extra_cols_per_row: usize,
) -> Result<(), EngineError> {
    validate_rows(rows)?;
    validate_data_sets(cols)?;
    let sheet_cells = validate_cell_grid(rows, cols, subcolumns, extra_cols_per_row)?;
    validate_project_cells(existing_cells, sheet_cells)?;
    Ok(())
}

fn projected_cloned_sheet_cells(arch: &TableArchetype, override_replicates: Option<usize>) -> Result<usize, DocError> {
    if let Some(n) = override_replicates {
        validate_subcolumns(n)?;
        return match arch {
            TableArchetype::Xy(t) => {
                let rows = t.x_cells.len().max(t.data_sets.first().map_or(0, |d| d.cells.len()));
                validate_cell_grid(rows, t.data_sets.len(), n, t.x_mode.subcolumn_count())
            }
            TableArchetype::Grouped(t) => {
                let rows = t.data_sets.first().map_or(0, |d| d.cells.len());
                validate_cell_grid(rows, t.data_sets.len(), n, 0)
            }
            TableArchetype::Nested(t) => {
                let rows = t.groups.first().map_or(0, |g| g.cells.len());
                validate_cell_grid(rows, t.groups.len(), n, 0)
            }
            _ => {
                let cells = arch.allocated_cell_slots();
                validate_sheet_cells(cells)?;
                Ok(cells)
            }
        };
    }
    let cells = arch.allocated_cell_slots();
    validate_sheet_cells(cells)?;
    Ok(cells)
}

fn build_archetype_from_params(parsed: &CreateDataParams, existing_cells: usize) -> Result<TableArchetype, EngineError> {
    let kind = parse_table_kind(parsed.kind.as_deref().unwrap_or("xy"))?;
    let default_rows = match kind {
        TableKind::Contingency => 2,
        TableKind::PartsOfWhole => 6,
        _ => 12,
    };
    let rows = parsed.rows.unwrap_or(default_rows);
    let default_cols = match kind {
        TableKind::PartsOfWhole => 1,
        TableKind::MultipleVariables => parsed.variables.as_ref().map_or(2, |v| v.len().max(1)),
        _ => 2,
    };
    let cols = parsed.cols.unwrap_or(default_cols);
    let replicates = parsed.replicates.unwrap_or(if kind == TableKind::Nested { 3 } else { 1 });

    match kind {
        TableKind::Xy => {
            let submode = parse_subcolumn_mode(parsed.subcolumn_mode.as_ref(), replicates)?;
            let x_mode = parse_x_mode(parsed.x_mode.as_ref())?;
            validate_sheet_and_project_budget(existing_cells, rows, cols, submode.subcolumn_count(), x_mode.subcolumn_count())?;
            let mut xy = XyTable::new_with_x_mode(x_mode, submode, cols, rows)?;
            if let Some(ref xt) = parsed.x_title {
                validate_title(xt)?;
                xy.x_title = xt.clone();
            }
            Ok(TableArchetype::Xy(xy))
        }
        TableKind::Column => {
            let design = parse_column_design_hint(parsed.design_hint.as_ref())?;
            let col = if parsed.subcolumn_mode.is_some() || replicates > 1 {
                let submode = parse_subcolumn_mode(parsed.subcolumn_mode.as_ref(), replicates)?;
                validate_sheet_and_project_budget(existing_cells, rows, cols, submode.subcolumn_count(), 0)?;
                ColumnTable::new_with_subcolumns(design, submode, cols, rows)?
            } else {
                validate_sheet_and_project_budget(existing_cells, rows, cols, 1, 0)?;
                ColumnTable::new(design, cols, rows)?
            };
            Ok(TableArchetype::Column(col))
        }
        TableKind::Grouped => {
            let submode = parse_subcolumn_mode(parsed.subcolumn_mode.as_ref(), replicates)?;
            validate_sheet_and_project_budget(existing_cells, rows, cols, submode.subcolumn_count(), 0)?;
            let mut grp = GroupedTable::new(submode, cols, rows)?;
            if let Some(ref rf) = parsed.row_factor_title {
                validate_title(rf)?;
                grp.row_factor_title = rf.clone();
            }
            if let Some(ref cf) = parsed.column_factor_title {
                validate_title(cf)?;
                grp.column_factor_title = cf.clone();
            }
            Ok(TableArchetype::Grouped(grp))
        }
        TableKind::Contingency => {
            validate_sheet_and_project_budget(existing_cells, rows, cols, 1, 0)?;
            let mut cont = ContingencyTable::new(cols, rows)?;
            if let Some(ref rf) = parsed.row_factor_title {
                validate_title(rf)?;
                cont.row_factor_title = rf.clone();
            }
            if let Some(ref cf) = parsed.column_factor_title {
                validate_title(cf)?;
                cont.column_factor_title = cf.clone();
            }
            Ok(TableArchetype::Contingency(cont))
        }
        TableKind::Survival => {
            validate_sheet_and_project_budget(existing_cells, rows, cols, 1, 1)?;
            let mut surv = SurvivalTable::new(cols, rows)?;
            if let Some(ref tt) = parsed.time_title {
                validate_title(tt)?;
                surv.time_title = tt.clone();
            }
            if let Some(ev) = parsed.event_code {
                surv.event_code = ev;
            }
            if let Some(cen) = parsed.censored_code {
                surv.censored_code = cen;
            }
            if surv.event_code == surv.censored_code {
                return Err(bad_input_ptr(
                    "/censored_code",
                    format!("Survival `event_code` ({}) and `censored_code` ({}) must be distinct", surv.event_code, surv.censored_code),
                )
                .with_remediation("Supply distinct integer codes for `event_code` and `censored_code` (for example `1` and `0`)."));
            }
            Ok(TableArchetype::Survival(surv))
        }
        TableKind::PartsOfWhole => {
            validate_sheet_and_project_budget(existing_cells, rows, cols, 1, 0)?;
            Ok(TableArchetype::PartsOfWhole(PartsOfWholeTable::new(cols, rows)?))
        }
        TableKind::MultipleVariables => build_multivariate_archetype(cols, rows, parsed.variables.as_deref(), existing_cells),
        TableKind::Nested => {
            validate_subcolumns(replicates)?;
            validate_sheet_and_project_budget(existing_cells, rows, cols, replicates, 0)?;
            Ok(TableArchetype::Nested(NestedTable::new(cols, replicates, rows)?))
        }
    }
}

fn build_multivariate_archetype(
    cols: usize,
    rows: usize,
    vars: Option<&[VariableInitInput]>,
    existing_cells: usize,
) -> Result<TableArchetype, EngineError> {
    let count = vars.map_or(cols, |v| cols.max(v.len()));
    validate_sheet_and_project_budget(existing_cells, rows, count, 1, 0)?;
    let mut mv = MultipleVariablesTable::new(count, rows)?;
    if let Some(items) = vars {
        for (idx, item) in items.iter().enumerate() {
            if let Some(slot) = mv.variables.get_mut(idx) {
                match item {
                    VariableInitInput::Name(name) => {
                        validate_title(name)?;
                        slot.title = name.clone();
                    }
                    VariableInitInput::Object { title, role } => {
                        if let Some(t) = title {
                            validate_title(t)?;
                            slot.title = t.clone();
                        }
                        if let Some(r) = role {
                            slot.lock_role(parse_variable_role(r)?);
                        }
                    }
                }
            }
        }
    }
    Ok(TableArchetype::MultipleVariables(mv))
}

fn provision_paired_info_sheet(
    doc: &mut ProjectDocument,
    reserved: &mut HashSet<String>,
    explicit_id: Option<&str>,
    data_id: &SheetId,
    data_title: &str,
) -> Result<(SheetId, Option<InfoSheet>), EngineError> {
    if let Some(raw) = explicit_id {
        let iid = parse_sheet_id(raw)?;
        if reserved.contains(iid.as_str()) {
            return Err(EngineError::with_code(ErrorCode::AlreadyExists, format!("info_sheet_id `{}` conflicts with new sheet ID", iid.as_str())));
        }
        if doc.info_sheet(&iid).is_some() {
            return Ok((iid, None));
        }
        if sheet_id_exists(doc, &iid) {
            return Err(EngineError::with_code(ErrorCode::AlreadyExists, format!("sheet id `{}` already exists in another section", iid.as_str())));
        }
        let info_title = format!("{data_title} Info");
        validate_title(&info_title)?;
        let info = InfoSheet::new_preseeded(iid.clone(), info_title)?;
        reserved.insert(iid.as_str().to_owned());
        return Ok((iid, Some(info)));
    }
    let iid = next_unique_id_with_reserved(doc, &format!("info-{}", data_id.as_str()), reserved)?;
    let info_title = format!("{data_title} Info");
    validate_title(&info_title)?;
    let info = InfoSheet::new_preseeded(iid.clone(), info_title)?;
    reserved.insert(iid.as_str().to_owned());
    Ok((iid, Some(info)))
}

fn run_sheet_create_data(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: CreateDataParams = parse_params(&create_data_schema(), params)?;
    let id = match parsed.sheet_id.as_deref() {
        Some(raw) => {
            let sid = parse_sheet_id(raw)?;
            if sheet_id_exists(&ctx.session.doc, &sid) {
                return Err(EngineError::with_code(ErrorCode::AlreadyExists, format!("sheet `{}` already exists", sid.as_str()))
                    .with_remediation("Choose a distinct `sheet_id` or omit it to auto-generate."));
            }
            sid
        }
        None => next_unique_sheet_id_hyphen(&ctx.session.doc, "data")?,
    };

    let default_title = format!("Data {}", ctx.session.doc.data_sheets.len() + 1);
    let title = parsed.title.as_deref().unwrap_or(&default_title).to_owned();
    let existing_cells = document_used_cells(&ctx.session.doc);
    let archetype = build_archetype_from_params(&parsed, existing_cells)?;
    let mut sheet = DataSheet::new(id.clone(), title.clone(), archetype)?;

    let mut reserved = HashSet::new();
    reserved.insert(id.as_str().to_owned());

    let auto_family = parsed.auto_family.unwrap_or(false);
    let auto_info = parsed.auto_info.unwrap_or(auto_family || parsed.info_sheet_id.is_some());
    let auto_graph = parsed.auto_graph.unwrap_or(auto_family || parsed.graph_sheet_id.is_some());

    let mut new_info: Option<InfoSheet> = None;
    let attached_info_id = if auto_info {
        let (iid, info_opt) = provision_paired_info_sheet(&mut ctx.session.doc, &mut reserved, parsed.info_sheet_id.as_deref(), &id, &title)?;
        sheet.info_sheet_id = Some(iid.clone());
        new_info = info_opt;
        Some(iid)
    } else {
        None
    };

    let new_graph = if auto_graph {
        let gid = match parsed.graph_sheet_id.as_deref() {
            Some(raw) => {
                let g = parse_sheet_id(raw)?;
                if reserved.contains(g.as_str()) || sheet_id_exists(&ctx.session.doc, &g) {
                    return Err(EngineError::with_code(ErrorCode::AlreadyExists, format!("graph sheet `{}` already exists", g.as_str())));
                }
                g
            }
            None => next_unique_id_with_reserved(&ctx.session.doc, &format!("graph-{}", id.as_str()), &reserved)?,
        };
        reserved.insert(gid.as_str().to_owned());
        Some(GraphSheet::new_uninitialized_for_data(gid, &sheet)?)
    } else {
        None
    };

    let kind_label = format!("{:?}", sheet.kind());
    if let Some(info) = new_info {
        ctx.session.doc.add_info_sheet(info)?;
    }
    ctx.session.doc.add_data_sheet(sheet)?;
    if let Some(ref iid) = attached_info_id {
        link_data_sheet_to_info(&mut ctx.session.doc, iid, &id);
    }
    let created_graph_id = if let Some(graph) = new_graph {
        let gid = graph.id.clone();
        ctx.session.doc.add_graph_sheet(graph)?;
        Some(gid)
    } else {
        None
    };
    ctx.session.doc.select_in_navigator(&id)?;

    Ok(CommandOutcome::mutated(
        "sheet.createData",
        "created data table sheet",
        json!({
            "sheet_id": id.as_str(),
            "title": title,
            "kind": kind_label,
            "info_sheet_id": attached_info_id.as_ref().map(SheetId::as_str),
            "graph_sheet_id": created_graph_id.as_ref().map(SheetId::as_str),
        }),
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenameSheetParams {
    /// ID of sheet to rename.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// New title for the sheet.
    #[schemars(length(min = 1))]
    title: String,
}

fn rename_sheet_schema() -> Value {
    // Retained until Stage 4 retires r16_5_cli_and_callers.rs
    command_schema_with_aliases::<RenameSheetParams>(&[("sheet_id", &["id"]), ("title", &["new_title"])])
}

fn run_sheet_rename(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: RenameSheetParams = parse_params(&rename_sheet_schema(), params)?;
    let id = parse_sheet_id(&parsed.sheet_id)?;
    let title = parsed.title.trim().to_owned();
    if title.is_empty() {
        return Err(bad_input("sheet `title` must not be empty").with_remediation("Provide a non-empty sheet title."));
    }
    validate_title(&title)?;

    let section = ctx
        .session
        .doc
        .sheet_section(&id)
        .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", id.as_str())))?;

    match section {
        SheetSection::Data => {
            {
                let ds = require_data_sheet_mut(&mut ctx.session.doc, &id)?;
                ensure_sheet_not_frozen(ds)?;
                ds.set_title(title.clone())?;
            }
            let _ = recalculate_dirty_with_links(&mut ctx.session.doc, std::slice::from_ref(&id), None, Some(ctx.cancelled))?;
        }
        SheetSection::Info => {
            let info = ctx
                .session
                .doc
                .info_sheet_mut(&id)
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("info sheet `{}` not found", id.as_str())))?;
            info.title.clone_from(&title);
        }
        SheetSection::Analyses => {
            let rs = ctx
                .session
                .doc
                .result_sheet_mut(&id)
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("result sheet `{}` not found", id.as_str())))?;
            rs.analysis.set_custom_title(Some(title.clone()))?;
        }
        SheetSection::Graphs => {
            let gs = ctx
                .session
                .doc
                .graph_sheet_mut(&id)
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("graph sheet `{}` not found", id.as_str())))?;
            gs.title.clone_from(&title);
        }
        SheetSection::Layouts => {
            let ls = ctx
                .session
                .doc
                .layout_sheet_mut(&id)
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("layout sheet `{}` not found", id.as_str())))?;
            ls.title.clone_from(&title);
        }
    }

    Ok(CommandOutcome::mutated(
        "sheet.rename",
        "renamed sheet",
        json!({
            "sheet_id": id.as_str(),
            "title": title,
            "section": format!("{section:?}"),
        }),
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteSheetParams {
    /// ID of the sheet to delete.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Delete downstream dependents (true) or refuse when dependents exist (false).
    cascade: Option<bool>,
}

fn has_live_dependents(doc: &ProjectDocument, id: &SheetId, section: SheetSection) -> Result<(), EngineError> {
    let dependent_data =
        doc.data_sheets.iter().find(|ds| ds.id != *id && ds.linked_regions.iter().any(|r| r.spec.source.upstream_sheet_id() == Some(id)));
    if let Some(ds) = dependent_data {
        let code = if ds.frozen { ErrorCode::FrozenSheet } else { ErrorCode::Conflict };
        return Err(EngineError::with_code(code, format!("cannot delete `{}` while data sheet `{}` links to it", id.as_str(), ds.id.as_str()))
            .with_remediation("Delete or unlink dependent sheets first, or pass `cascade: true` on unfrozen sheets."));
    }
    if matches!(section, SheetSection::Data | SheetSection::Analyses)
        && let Some(rs) =
            doc.result_sheets.iter().find(|r| r.id != *id && (r.analysis.source_sheet_id == *id || result_hooks_data_or_result_sheet(r, id)))
    {
        return Err(EngineError::with_code(
            ErrorCode::Conflict,
            format!("cannot delete `{}` while analysis `{}` depends on it", id.as_str(), rs.id.as_str()),
        )
        .with_remediation("Pass `cascade: true` or remove dependent analyses first."));
    }
    if let Some(gs) = doc.graph_sheets.iter().find(|g| g.id != *id && graph_references_sheet(doc, g, id)) {
        return Err(EngineError::with_code(
            ErrorCode::Conflict,
            format!("cannot delete `{}` while graph `{}` depends on it", id.as_str(), gs.id.as_str()),
        ));
    }
    if let Some(ls) = doc.layout_sheets.iter().find(|l| l.id != *id && layout_references_sheet(doc, l, id)) {
        return Err(EngineError::with_code(
            ErrorCode::Conflict,
            format!("cannot delete `{}` while layout `{}` embeds it", id.as_str(), ls.id.as_str()),
        ));
    }
    if section == SheetSection::Info {
        if let Some(ds) = doc.data_sheets.iter().find(|d| d.info_sheet_id.as_ref() == Some(id)) {
            return Err(EngineError::with_code(
                ErrorCode::Conflict,
                format!("cannot delete info sheet `{}` while data sheet `{}` references it", id.as_str(), ds.id.as_str()),
            ));
        }
        if let Some(rs) = doc.result_sheets.iter().find(|r| result_hooks_info_sheet(doc, r, id)) {
            return Err(EngineError::with_code(
                ErrorCode::Conflict,
                format!("cannot delete info sheet `{}` while analysis `{}` hooks constants from it", id.as_str(), rs.id.as_str()),
            ));
        }
    }
    Ok(())
}

fn result_origin_info_sheet_id<'a>(doc: &'a ProjectDocument, rs: &'a ResultSheet) -> Option<&'a SheetId> {
    let mut cursor = &rs.analysis.source_sheet_id;
    let mut seen: HashSet<&SheetId> = HashSet::new();
    while seen.insert(cursor) {
        if let Some(ds) = doc.data_sheet(cursor) {
            return ds.info_sheet_id.as_ref();
        }
        if let Some(upstream) = doc.result_sheet(cursor) {
            cursor = &upstream.analysis.source_sheet_id;
        } else {
            break;
        }
    }
    None
}

fn result_hooks_data_or_result_sheet(rs: &ResultSheet, target_id: &SheetId) -> bool {
    rs.analysis.kind.hooked_sources().into_iter().any(|hook| match hook {
        HookSource::ResultScalar { result_sheet_id, .. } => &result_sheet_id == target_id,
        HookSource::ColumnTitleValue { data_sheet_id, .. } => &data_sheet_id == target_id,
        HookSource::InfoConstant { .. } => false,
    })
}

fn result_hooks_info_sheet(doc: &ProjectDocument, rs: &ResultSheet, info_id: &SheetId) -> bool {
    if let AnalysisKind::PowerAnalysis(p) = &rs.analysis.kind
        && p.info_sheet_id.as_ref() == Some(info_id)
    {
        return true;
    }
    rs.analysis.kind.hooked_sources().into_iter().any(|hook| match hook {
        HookSource::InfoConstant { info_sheet_id: Some(iid), .. } => &iid == info_id,
        HookSource::InfoConstant { info_sheet_id: None, .. } => result_origin_info_sheet_id(doc, rs) == Some(info_id),
        _ => false,
    })
}

fn graph_references_sheet(doc: &ProjectDocument, gs: &GraphSheet, id: &SheetId) -> bool {
    doc.upstream_dependencies(&gs.id).contains(id)
}

fn layout_references_sheet(doc: &ProjectDocument, ls: &LayoutSheet, id: &SheetId) -> bool {
    doc.upstream_dependencies(&ls.id).contains(id)
}

fn collect_downstream_for_delete(doc: &ProjectDocument, root_id: &SheetId, include_paired_info: bool, to_delete: &mut HashSet<SheetId>) {
    let mut changed = true;
    while changed {
        changed = false;
        for rs in &doc.result_sheets {
            if !to_delete.contains(&rs.id) && to_delete.contains(&rs.analysis.source_sheet_id) {
                to_delete.insert(rs.id.clone());
                changed = true;
            }
        }
        for ds in &doc.data_sheets {
            if !to_delete.contains(&ds.id)
                && ds.linked_regions.iter().any(|r| r.spec.source.upstream_sheet_id().is_some_and(|u| to_delete.contains(u)))
            {
                to_delete.insert(ds.id.clone());
                changed = true;
            }
        }
    }

    for gs in &doc.graph_sheets {
        if !to_delete.contains(&gs.id)
            && (to_delete.contains(&gs.spec.primary_source_sheet_id) || gs.spec.series.iter().any(|s| to_delete.contains(&s.source_sheet_id)))
        {
            to_delete.insert(gs.id.clone());
        }
    }

    if include_paired_info {
        for ds in &doc.data_sheets {
            if to_delete.contains(&ds.id)
                && let Some(ref iid) = ds.info_sheet_id
            {
                let shared_elsewhere =
                    doc.data_sheets.iter().any(|other| !to_delete.contains(&other.id) && other.info_sheet_id.as_ref() == Some(iid))
                        || doc.result_sheets.iter().any(|rs| {
                            !to_delete.contains(&rs.id) && (result_hooks_info_sheet(doc, rs, iid) || doc.upstream_dependencies(&rs.id).contains(iid))
                        })
                        || doc.graph_sheets.iter().any(|gs| !to_delete.contains(&gs.id) && graph_references_sheet(doc, gs, iid))
                        || doc.layout_sheets.iter().any(|ls| !to_delete.contains(&ls.id) && layout_references_sheet(doc, ls, iid));
                if !shared_elsewhere && !to_delete.contains(iid) && iid != root_id {
                    to_delete.insert(iid.clone());
                }
            }
        }
    }
}

fn run_sheet_delete(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: DeleteSheetParams = parse_params(&command_schema::<DeleteSheetParams>(), params)?;
    let id = parse_sheet_id(&parsed.sheet_id)?;
    let section = ctx
        .session
        .doc
        .sheet_section(&id)
        .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", id.as_str())))?;

    if section == SheetSection::Data
        && let Some(ds) = ctx.session.doc.data_sheet(&id)
    {
        ds.ensure_mutable()?;
    }

    let mut to_delete: HashSet<SheetId> = HashSet::new();
    to_delete.insert(id.clone());

    if parsed.cascade == Some(false) {
        has_live_dependents(&ctx.session.doc, &id, section)?;
    } else if matches!(section, SheetSection::Data | SheetSection::Analyses) {
        collect_downstream_for_delete(&ctx.session.doc, &id, parsed.cascade == Some(true), &mut to_delete);
    }

    for ds in &ctx.session.doc.data_sheets {
        if !ds.frozen {
            continue;
        }
        let mutates_frozen_sheet = to_delete.contains(&ds.id)
            || ds.info_sheet_id.as_ref().is_some_and(|iid| to_delete.contains(iid))
            || ds.linked_regions.iter().any(|r| r.spec.source.upstream_sheet_id().is_some_and(|up| to_delete.contains(up)));
        if mutates_frozen_sheet {
            return Err(EngineError::from(DocError::SheetFrozen { sheet_id: ds.id.clone() }));
        }
    }

    let deleted_any_info = to_delete.iter().any(|sid| ctx.session.doc.info_sheet(sid).is_some());
    let surviving_dependents: Vec<SheetId> = ctx
        .session
        .doc
        .data_sheets
        .iter()
        .map(|s| &s.id)
        .chain(ctx.session.doc.result_sheets.iter().map(|s| &s.id))
        .filter(|sid| !to_delete.contains(*sid))
        .filter(|sid| ctx.session.doc.upstream_dependencies(sid).iter().any(|up| to_delete.contains(up)))
        .cloned()
        .collect();
    let deleted_ids = prune_deleted_sheets(&mut ctx.session.doc, &to_delete);
    if deleted_any_info || matches!(section, SheetSection::Data | SheetSection::Analyses) {
        let _ = recalculate_dirty_with_links(&mut ctx.session.doc, &surviving_dependents, None, Some(ctx.cancelled))?;
    }

    Ok(CommandOutcome::mutated(
        "sheet.delete",
        "deleted sheet",
        json!({
            "sheet_id": id.as_str(),
            "deleted_ids": deleted_ids,
        }),
    ))
}

fn prune_deleted_sheets(doc: &mut ProjectDocument, to_delete: &HashSet<SheetId>) -> Vec<String> {
    let mut deleted_ids: Vec<String> = to_delete.iter().map(|sid| sid.as_str().to_owned()).collect();
    deleted_ids.sort();

    doc.data_sheets.retain(|ds| !to_delete.contains(&ds.id));
    doc.info_sheets.retain(|info| !to_delete.contains(&info.id));
    doc.result_sheets.retain(|rs| !to_delete.contains(&rs.id));
    doc.graph_sheets.retain(|gs| !to_delete.contains(&gs.id));
    doc.layout_sheets.retain(|ls| !to_delete.contains(&ls.id));

    for ds in &mut doc.data_sheets {
        if ds.info_sheet_id.as_ref().is_some_and(|iid| to_delete.contains(iid)) {
            ds.info_sheet_id = None;
        }
        ds.linked_regions.retain(|r| r.spec.source.upstream_sheet_id().is_none_or(|up| !to_delete.contains(up)));
    }
    sync_info_linked_data_sheets(doc, to_delete);
    for rs in &mut doc.result_sheets {
        if let AnalysisKind::PowerAnalysis(p) = &mut rs.analysis.kind
            && p.info_sheet_id.as_ref().is_some_and(|iid| to_delete.contains(iid))
        {
            p.info_sheet_id = None;
        }
    }
    for gs in &mut doc.graph_sheets {
        gs.spec.series.retain(|s| !to_delete.contains(&s.source_sheet_id));
        for series in &mut gs.spec.series {
            if series.regression_overlay.as_ref().is_some_and(|ov| to_delete.contains(&ov.result_sheet_id)) {
                series.regression_overlay = None;
            }
        }
        gs.spec.comparisons.retain(|cmp| !to_delete.contains(&cmp.result_sheet_id));
        if gs.spec.compact_letter_display.as_ref().is_some_and(|cld| to_delete.contains(&cld.result_sheet_id)) {
            gs.spec.compact_letter_display = None;
            gs.spec.pairwise_display_mode = curvebook_doc::PairwiseDisplayMode::BracketsOnly;
        }
        gs.spec.annotations.retain(|ann| match ann {
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                !to_delete.contains(result_sheet_id)
            }
            GraphAnnotation::TextBox { .. }
            | GraphAnnotation::LineOrArrow { .. }
            | GraphAnnotation::Shape { .. }
            | GraphAnnotation::EmbeddedImage { .. } => true,
        });
    }
    for ls in &mut doc.layout_sheets {
        ls.spec.items.retain(|item| match item {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => !to_delete.contains(graph_sheet_id),
            LayoutItem::LiveTable { source_sheet_id, .. } => !to_delete.contains(source_sheet_id),
            LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => true,
        });
    }
    if doc.active_sheet_id.as_ref().is_some_and(|sid| to_delete.contains(sid)) {
        doc.active_sheet_id = None;
    }
    if doc.family_anchor_sheet_id.as_ref().is_some_and(|sid| to_delete.contains(sid)) {
        doc.family_anchor_sheet_id = None;
    }
    deleted_ids
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DuplicateSheetParams {
    /// Source sheet ID to duplicate.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Optional explicit ID for the duplicated sheet.
    #[schemars(length(min = 1))]
    new_sheet_id: Option<String>,
    /// Optional title override for the duplicate.
    title: Option<String>,
    /// Preserve or set frozen flag on duplicated DataSheet (defaults to false).
    frozen: Option<bool>,
}

fn allocate_new_sheet_id(doc: &ProjectDocument, explicit: Option<&str>, fallback_prefix: &str) -> Result<SheetId, EngineError> {
    match explicit {
        Some(raw) => {
            let sid = parse_sheet_id(raw)?;
            if sheet_id_exists(doc, &sid) {
                return Err(EngineError::with_code(ErrorCode::AlreadyExists, format!("sheet `{}` already exists", sid.as_str())));
            }
            Ok(sid)
        }
        None => next_unique_sheet_id_hyphen(doc, fallback_prefix),
    }
}

fn run_sheet_duplicate(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: DuplicateSheetParams = parse_params(&command_schema::<DuplicateSheetParams>(), params)?;
    let id = parse_sheet_id(&parsed.sheet_id)?;
    let section = ctx
        .session
        .doc
        .sheet_section(&id)
        .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", id.as_str())))?;

    let new_id = allocate_new_sheet_id(&ctx.session.doc, parsed.new_sheet_id.as_deref(), &format!("{}-copy", id.as_str()))?;
    if let Some(ref title) = parsed.title {
        validate_title(title)?;
    }

    match section {
        SheetSection::Data => {
            let existing_cells = document_used_cells(&ctx.session.doc);
            let src = ctx
                .session
                .doc
                .data_sheet(&id)
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("data sheet `{}` not found", id.as_str())))?;
            let needed = src.archetype.allocated_cell_slots();
            validate_sheet_cells(needed)?;
            validate_project_cells(existing_cells, needed)?;
            let mut copy = src.clone();
            copy.id = new_id.clone();
            let title = parsed.title.unwrap_or_else(|| format!("{} (Copy)", copy.title));
            validate_title(&title)?;
            copy.title = title;
            copy.frozen = parsed.frozen.unwrap_or(false);
            let linked_info = copy.info_sheet_id.clone();
            ctx.session.doc.add_data_sheet(copy)?;
            if let Some(ref iid) = linked_info {
                link_data_sheet_to_info(&mut ctx.session.doc, iid, &new_id);
            }
            let _ = recalculate_dirty_with_links(&mut ctx.session.doc, std::slice::from_ref(&new_id), None, Some(ctx.cancelled))?;
        }
        SheetSection::Info => {
            let mut copy = ctx
                .session
                .doc
                .info_sheet(&id)
                .cloned()
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("info sheet `{}` not found", id.as_str())))?;
            copy.id = new_id.clone();
            copy.linked_data_sheets.clear();
            let title = parsed.title.unwrap_or_else(|| format!("{} (Copy)", copy.title));
            validate_title(&title)?;
            copy.title = title;
            ctx.session.doc.add_info_sheet(copy)?;
        }
        SheetSection::Analyses => {
            let mut copy = ctx
                .session
                .doc
                .result_sheet(&id)
                .cloned()
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("result sheet `{}` not found", id.as_str())))?;
            copy.id = new_id.clone();
            copy.analysis.id = new_id.clone();
            let title = parsed.title.unwrap_or_else(|| format!("{} (Copy)", copy.analysis.effective_title()));
            validate_title(&title)?;
            copy.analysis.custom_title = Some(title);
            if let Some(ref mut ad) = copy.analyzable_data {
                ad.id = new_id.clone();
            }
            ctx.session.doc.add_result_sheet(copy)?;
        }
        SheetSection::Graphs => {
            let mut copy = ctx
                .session
                .doc
                .graph_sheet(&id)
                .cloned()
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("graph sheet `{}` not found", id.as_str())))?;
            copy.id = new_id.clone();
            let title = parsed.title.unwrap_or_else(|| format!("{} (Copy)", copy.title));
            validate_title(&title)?;
            copy.title = title;
            ctx.session.doc.add_graph_sheet(copy)?;
        }
        SheetSection::Layouts => {
            let mut copy = ctx
                .session
                .doc
                .layout_sheet(&id)
                .cloned()
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("layout sheet `{}` not found", id.as_str())))?;
            copy.id = new_id.clone();
            let title = parsed.title.unwrap_or_else(|| format!("{} (Copy)", copy.title));
            validate_title(&title)?;
            copy.title = title;
            ctx.session.doc.add_layout_sheet(copy)?;
        }
    }

    ctx.session.doc.select_in_navigator(&new_id)?;
    Ok(CommandOutcome::mutated(
        "sheet.duplicate",
        "duplicated sheet",
        json!({
            "source_sheet_id": id.as_str(),
            "sheet_id": new_id.as_str(),
            "section": format!("{section:?}"),
        }),
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DuplicateFamilyParams {
    /// Root sheet ID whose family should be duplicated.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// ID prefix for duplicated sheets.
    #[schemars(length(min = 1))]
    prefix: Option<String>,
    /// Clear Y data and reset results when true (defaults to false).
    clear_data: Option<bool>,
    /// Optional explicit ID for duplicated DataSheet.
    #[schemars(length(min = 1))]
    new_sheet_id: Option<String>,
    /// Optional title override for duplicated DataSheet.
    title: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ClonePipelineParams {
    /// Source root DataSheet (or family member) ID to clone.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Optional explicit ID for the cloned DataSheet.
    #[schemars(length(min = 1))]
    new_sheet_id: Option<String>,
    /// Optional ID prefix for cloned sheets.
    #[schemars(length(min = 1))]
    prefix: Option<String>,
    /// Optional title for cloned DataSheet.
    title: Option<String>,
    /// Convenience flag overriding keep_x_values / clear_y_values.
    clear_data: Option<bool>,
    /// Preserve X values when cloning (defaults to true).
    keep_x_values: Option<bool>,
    /// Clear Y values when cloning pipeline (defaults to true).
    clear_y_values: Option<bool>,
    /// Clear dataset column titles (defaults to false).
    clear_column_titles: Option<bool>,
    /// Clear row titles (defaults to false).
    clear_row_titles: Option<bool>,
    /// Optional new replicate subcolumn count.
    #[schemars(range(min = 1))]
    override_replicates: Option<usize>,
    /// Clone downstream ResultSheets (defaults to true).
    clone_analyses: Option<bool>,
    /// Clone downstream GraphSheets (defaults to true).
    clone_graphs: Option<bool>,
    /// Clone downstream LayoutSheets (defaults to true).
    clone_layouts: Option<bool>,
    /// Skip immediate recalculation and leave cloned ResultSheets stale.
    clear_results: Option<bool>,
}

struct FamilyCloneOptions<'a> {
    command_id: &'a str,
    raw_sheet_id: &'a str,
    new_sheet_id: Option<&'a str>,
    prefix: Option<&'a str>,
    title: Option<&'a str>,
    suffix_label: &'a str,
    keep_x_values: bool,
    clear_y_values: bool,
    clear_column_titles: bool,
    clear_row_titles: bool,
    override_replicates: Option<usize>,
    clone_analyses: bool,
    clone_graphs: bool,
    clone_layouts: bool,
    clear_results: bool,
}

fn resolve_family_root_data_id(doc: &ProjectDocument, raw_id: &SheetId) -> Result<SheetId, EngineError> {
    let mut active = BTreeSet::new();
    let mut visited = BTreeSet::new();
    resolve_family_root_data_id_with_visited(doc, raw_id, &mut active, &mut visited)
}

fn resolve_family_root_data_id_with_visited(
    doc: &ProjectDocument,
    raw_id: &SheetId,
    active: &mut BTreeSet<SheetId>,
    visited: &mut BTreeSet<SheetId>,
) -> Result<SheetId, EngineError> {
    if !active.insert(raw_id.clone()) {
        return Err(EngineError::with_code(ErrorCode::CycleDetected, format!("circular sheet reference detected at `{}`", raw_id.as_str())));
    }
    if !visited.insert(raw_id.clone()) {
        active.remove(raw_id);
        return Err(bad_input(format!("sheet `{}` cannot trace back to a data sheet", raw_id.as_str())));
    }

    let outcome = resolve_family_root_data_id_inner(doc, raw_id, active, visited);
    active.remove(raw_id);
    outcome
}

fn resolve_family_root_data_id_inner(
    doc: &ProjectDocument,
    raw_id: &SheetId,
    active: &mut BTreeSet<SheetId>,
    visited: &mut BTreeSet<SheetId>,
) -> Result<SheetId, EngineError> {
    let section =
        doc.sheet_section(raw_id).ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", raw_id.as_str())))?;

    match section {
        SheetSection::Data => Ok(raw_id.clone()),
        SheetSection::Info => doc
            .data_sheets
            .iter()
            .find(|ds| ds.info_sheet_id.as_ref() == Some(raw_id))
            .map(|ds| ds.id.clone())
            .ok_or_else(|| bad_input(format!("info sheet `{}` has no owning data sheet", raw_id.as_str()))),
        SheetSection::Analyses => {
            let Some(rs) = doc.result_sheet(raw_id) else {
                return Err(EngineError::with_code(ErrorCode::NotFound, format!("result sheet `{}` not found", raw_id.as_str())));
            };
            let up = &rs.analysis.source_sheet_id;
            if doc.data_sheet(up).is_some() {
                return Ok(up.clone());
            }
            if doc.sheet_section(up).is_none() {
                return Err(bad_input(format!("result sheet `{}` cannot trace back to a data sheet", raw_id.as_str())));
            }
            resolve_family_root_data_id_with_visited(doc, up, active, visited)
        }
        SheetSection::Graphs => {
            let gs = doc
                .graph_sheet(raw_id)
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("graph sheet `{}` not found", raw_id.as_str())))?;
            let primary = &gs.spec.primary_source_sheet_id;
            if doc.data_sheet(primary).is_some() {
                return Ok(primary.clone());
            }
            resolve_family_root_data_id_with_visited(doc, primary, active, visited)
        }
        SheetSection::Layouts => {
            let ls = doc
                .layout_sheet(raw_id)
                .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("layout sheet `{}` not found", raw_id.as_str())))?;
            let mut cycle_err: Option<EngineError> = None;
            for item in &ls.spec.items {
                let target_id = match item {
                    LayoutItem::LiveTable { source_sheet_id, .. } => {
                        if doc.data_sheet(source_sheet_id).is_some() {
                            return Ok(source_sheet_id.clone());
                        }
                        source_sheet_id
                    }
                    LayoutItem::LiveGraph { graph_sheet_id, .. } => graph_sheet_id,
                    LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => continue,
                };
                match resolve_family_root_data_id_with_visited(doc, target_id, active, visited) {
                    Ok(found) => return Ok(found),
                    Err(err) if err.code == ErrorCode::CycleDetected => {
                        if cycle_err.is_none() {
                            cycle_err = Some(err);
                        }
                    }
                    Err(_) => {}
                }
            }
            Err(cycle_err.unwrap_or_else(|| bad_input(format!("layout `{}` has no data sheet family", raw_id.as_str()))))
        }
    }
}

fn clone_family_into_doc(
    doc: &mut ProjectDocument,
    req: &FamilyCloneOptions<'_>,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<CommandOutcome, EngineError> {
    let raw_id = parse_sheet_id(req.raw_sheet_id)?;
    let root_data_id = resolve_family_root_data_id(doc, &raw_id)?;
    let existing_cells = document_used_cells(doc);
    let src_ds = {
        let src_ref = doc
            .data_sheet(&root_data_id)
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("data sheet `{}` not found", root_data_id.as_str())))?;
        let projected_cells = projected_cloned_sheet_cells(&src_ref.archetype, req.override_replicates)?;
        validate_project_cells(existing_cells, projected_cells)?;
        src_ref.clone()
    };

    let mut reserved: HashSet<String> = HashSet::new();
    let id_prefix = req.prefix.unwrap_or(root_data_id.as_str());
    let new_data_id = match req.new_sheet_id {
        Some(raw_new) => {
            let sid = parse_sheet_id(raw_new)?;
            if sheet_id_exists(doc, &sid) {
                return Err(EngineError::with_code(ErrorCode::AlreadyExists, format!("sheet `{}` already exists", sid.as_str())));
            }
            sid
        }
        None => next_unique_id_with_reserved(doc, id_prefix, &reserved)?,
    };
    reserved.insert(new_data_id.as_str().to_owned());

    let new_title = req.title.map_or_else(|| format!("{} ({})", src_ds.title, req.suffix_label), str::to_owned);
    let clone_opts = CloneDataSheetOptions {
        title: new_title.clone(),
        keep_x_values: req.keep_x_values,
        clear_y_values: req.clear_y_values,
        clear_column_titles: req.clear_column_titles,
        clear_row_titles: req.clear_row_titles,
        override_replicates: req.override_replicates,
    };
    let mut cloned_ds = src_ds.clone_structure(new_data_id.clone(), &clone_opts)?;

    let mut id_map: HashMap<SheetId, SheetId> = HashMap::new();
    id_map.insert(root_data_id.clone(), new_data_id.clone());

    let new_info_id = clone_paired_info(doc, &src_ds, &mut cloned_ds, &mut reserved, id_prefix, req.prefix.is_some())?;
    if let (Some(old_info), Some(new_info)) = (&src_ds.info_sheet_id, &new_info_id) {
        id_map.insert(old_info.clone(), new_info.clone());
    }
    doc.add_data_sheet(cloned_ds)?;

    let result_ids = if req.clone_analyses {
        clone_downstream_results(doc, &mut id_map, &mut reserved, id_prefix, req.clear_results, req.suffix_label)?
    } else {
        Vec::new()
    };
    let graph_ids =
        if req.clone_graphs { clone_downstream_graphs(doc, &mut id_map, &mut reserved, id_prefix, req.suffix_label)? } else { Vec::new() };
    let layout_ids = if req.clone_layouts { clone_downstream_layouts(doc, &id_map, &mut reserved, id_prefix, req.suffix_label)? } else { Vec::new() };

    if !req.clear_results {
        let dirty_seeds: Vec<SheetId> = id_map.values().cloned().collect();
        let _ = recalculate_dirty_with_links(doc, &dirty_seeds, None, Some(cancelled))?;
    }
    doc.select_in_navigator(&new_data_id)?;

    Ok(CommandOutcome::mutated(
        req.command_id,
        "cloned sheet pipeline",
        json!({
            "source_sheet_id": root_data_id.as_str(),
            "sheet_id": new_data_id.as_str(),
            "title": new_title,
            "info_sheet_id": new_info_id.as_ref().map(SheetId::as_str),
            "result_sheet_ids": result_ids,
            "graph_sheet_ids": graph_ids,
            "layout_sheet_ids": layout_ids,
        }),
    ))
}

fn clone_paired_info(
    doc: &mut ProjectDocument,
    src_ds: &DataSheet,
    cloned_ds: &mut DataSheet,
    reserved: &mut HashSet<String>,
    id_prefix: &str,
    has_custom_prefix: bool,
) -> Result<Option<SheetId>, EngineError> {
    let Some(ref src_info_id) = src_ds.info_sheet_id else {
        cloned_ds.info_sheet_id = None;
        return Ok(None);
    };
    let Some(mut cloned_info) = doc.info_sheet(src_info_id).cloned() else {
        cloned_ds.info_sheet_id = None;
        return Ok(None);
    };
    let candidate = if has_custom_prefix { format!("{id_prefix}-info") } else { format!("info-{}", cloned_ds.id.as_str()) };
    let new_info_id = next_unique_id_with_reserved(doc, &candidate, reserved)?;
    reserved.insert(new_info_id.as_str().to_owned());
    cloned_info.id = new_info_id.clone();
    cloned_info.linked_data_sheets = vec![cloned_ds.id.clone()];
    let info_title = format!("{} Info", cloned_ds.title);
    validate_title(&info_title)?;
    cloned_info.title = info_title;
    doc.add_info_sheet(cloned_info)?;
    cloned_ds.info_sheet_id = Some(new_info_id.clone());
    Ok(Some(new_info_id))
}

fn clone_downstream_results(
    doc: &mut ProjectDocument,
    id_map: &mut HashMap<SheetId, SheetId>,
    reserved: &mut HashSet<String>,
    id_prefix: &str,
    clear_results: bool,
    suffix_label: &str,
) -> Result<Vec<String>, EngineError> {
    let mut created = Vec::new();
    let original_results = doc.result_sheets.clone();
    loop {
        let mut progress = false;
        for rs in &original_results {
            if id_map.contains_key(&rs.id) || !id_map.contains_key(&rs.analysis.source_sheet_id) {
                continue;
            }
            let new_res_id = next_unique_id_with_reserved(doc, &format!("{id_prefix}-res"), reserved)?;
            reserved.insert(new_res_id.as_str().to_owned());
            let new_rs = remap_result_sheet(rs.clone(), new_res_id.clone(), id_map, clear_results, suffix_label)?;
            id_map.insert(rs.id.clone(), new_res_id.clone());
            created.push(new_res_id.as_str().to_owned());
            doc.add_result_sheet(new_rs)?;
            progress = true;
        }
        if !progress {
            break;
        }
    }
    for created_id_str in &created {
        let created_id = SheetId::new(created_id_str.as_str());
        if let Some(rs) = doc.result_sheets.iter_mut().find(|r| r.id == created_id) {
            remap_analysis_hooks(&mut rs.analysis, id_map);
        }
    }
    Ok(created)
}

fn remap_result_sheet(
    mut res: ResultSheet,
    new_id: SheetId,
    id_map: &HashMap<SheetId, SheetId>,
    clear_results: bool,
    suffix_label: &str,
) -> Result<ResultSheet, EngineError> {
    let orig_title = res.title().to_owned();
    res.id = new_id.clone();
    res.analysis.id = new_id;
    if let Some(mapped) = id_map.get(&res.analysis.source_sheet_id) {
        res.analysis.source_sheet_id = mapped.clone();
    }
    let next_title = if let Some(custom) = res.analysis.custom_title.take() {
        format!("{custom} ({suffix_label})")
    } else {
        format!("{orig_title} ({suffix_label})")
    };
    validate_title(&next_title)?;
    res.analysis.custom_title = Some(next_title);
    remap_analysis_hooks(&mut res.analysis, id_map);
    for outlier in &mut res.eliminated_outliers {
        if let Some(mapped) = id_map.get(&outlier.sheet_id) {
            outlier.sheet_id = mapped.clone();
        }
    }
    if clear_results {
        res.status = ResultStatus::Dirty;
        res.analyzable_data = None;
        res.tabs.clear();
        res.scalars.clear();
        res.pairwise_comparisons.clear();
        res.fitted_curves.clear();
        res.eliminated_outliers.clear();
        res.checklist.clear();
    } else if let Some(analyzable) = &mut res.analyzable_data {
        analyzable.id = res.id.clone();
    }
    Ok(res)
}

fn text_hooks_any_sheet(text: &str, id_map: &HashMap<SheetId, SheetId>) -> bool {
    if extract_template_hooks(text).iter().any(|hook| hook.upstream_sheet_id().is_some_and(|id| id_map.contains_key(id))) {
        return true;
    }
    let mut rest = text;
    while let Some((_, after_open)) = rest.split_once("{{") {
        let Some((inner, tail)) = after_open.split_once("}}") else {
            break;
        };
        let parts: Vec<&str> = inner.trim().split(':').map(str::trim).collect();
        if let ["scalar", sheet, rest_parts @ ..] = parts.as_slice()
            && !sheet.is_empty()
            && !rest_parts.is_empty()
            && id_map.contains_key(&SheetId::new(*sheet))
        {
            return true;
        }
        rest = tail;
    }
    false
}

fn graph_references_family(gs: &GraphSheet, id_map: &HashMap<SheetId, SheetId>) -> bool {
    let spec = &gs.spec;
    id_map.contains_key(&spec.primary_source_sheet_id)
        || spec.series.iter().any(|s| {
            id_map.contains_key(&s.source_sheet_id)
                || s.regression_overlay.as_ref().is_some_and(|ov| id_map.contains_key(&ov.result_sheet_id))
                || s.legend_label.as_deref().is_some_and(|lbl| text_hooks_any_sheet(lbl, id_map))
        })
        || spec.compact_letter_display.as_ref().is_some_and(|cld| id_map.contains_key(&cld.result_sheet_id))
        || spec.comparisons.iter().any(|cmp| id_map.contains_key(&cmp.result_sheet_id))
        || spec.annotations.iter().any(|ann| match ann {
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                id_map.contains_key(result_sheet_id)
            }
            GraphAnnotation::TextBox { markdown, .. } => text_hooks_any_sheet(markdown, id_map),
            GraphAnnotation::LineOrArrow { .. } | GraphAnnotation::Shape { .. } | GraphAnnotation::EmbeddedImage { .. } => false,
        })
        || spec.inline_hooks().iter().any(|hook| hook.upstream_sheet_id().is_some_and(|id| id_map.contains_key(id)))
        || text_hooks_any_sheet(&spec.title, id_map)
        || text_hooks_any_sheet(&spec.x_axis.title, id_map)
        || text_hooks_any_sheet(&spec.y_axis.title, id_map)
        || spec.y2_axis.as_ref().is_some_and(|y2| text_hooks_any_sheet(&y2.title, id_map))
}

fn clone_downstream_graphs(
    doc: &mut ProjectDocument,
    id_map: &mut HashMap<SheetId, SheetId>,
    reserved: &mut HashSet<String>,
    id_prefix: &str,
    suffix_label: &str,
) -> Result<Vec<String>, EngineError> {
    let mut created = Vec::new();
    let original_graphs = doc.graph_sheets.clone();
    for gs in &original_graphs {
        if !graph_references_family(gs, id_map) {
            continue;
        }
        let new_graph_id = next_unique_id_with_reserved(doc, &format!("{id_prefix}-graph"), reserved)?;
        reserved.insert(new_graph_id.as_str().to_owned());
        let mut cloned = gs.clone();
        cloned.id = new_graph_id.clone();
        let next_title = format!("{} ({suffix_label})", gs.title);
        validate_title(&next_title)?;
        cloned.title = next_title;
        remap_graph_spec_references(&mut cloned.spec, id_map);
        id_map.insert(gs.id.clone(), new_graph_id.clone());
        created.push(new_graph_id.as_str().to_owned());
        doc.add_graph_sheet(cloned)?;
    }
    Ok(created)
}

fn clone_downstream_layouts(
    doc: &mut ProjectDocument,
    id_map: &HashMap<SheetId, SheetId>,
    reserved: &mut HashSet<String>,
    id_prefix: &str,
    suffix_label: &str,
) -> Result<Vec<String>, EngineError> {
    let mut created = Vec::new();
    let original_layouts = doc.layout_sheets.clone();
    for ls in &original_layouts {
        let references_items = ls.spec.items.iter().any(|item| match item {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => id_map.contains_key(graph_sheet_id),
            LayoutItem::LiveTable { source_sheet_id, .. } => id_map.contains_key(source_sheet_id),
            LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => false,
        });
        let references_hooks = ls.spec.inline_hooks().iter().any(|hook| hook.upstream_sheet_id().is_some_and(|id| id_map.contains_key(id)));
        if !references_items && !references_hooks {
            continue;
        }
        let new_layout_id = next_unique_id_with_reserved(doc, &format!("{id_prefix}-layout"), reserved)?;
        reserved.insert(new_layout_id.as_str().to_owned());
        let mut cloned = ls.clone();
        cloned.id = new_layout_id.clone();
        let next_title = format!("{} ({suffix_label})", ls.title);
        validate_title(&next_title)?;
        cloned.title = next_title;
        remap_layout_spec_references(&mut cloned.spec, id_map);
        created.push(new_layout_id.as_str().to_owned());
        doc.add_layout_sheet(cloned)?;
    }
    Ok(created)
}

fn run_sheet_duplicate_family(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: DuplicateFamilyParams = parse_params(&command_schema::<DuplicateFamilyParams>(), params)?;
    let clear_data = parsed.clear_data.unwrap_or(false);
    let req = FamilyCloneOptions {
        command_id: "sheet.duplicateFamily",
        raw_sheet_id: &parsed.sheet_id,
        new_sheet_id: parsed.new_sheet_id.as_deref(),
        prefix: parsed.prefix.as_deref(),
        title: parsed.title.as_deref(),
        suffix_label: "Copy",
        keep_x_values: !clear_data,
        clear_y_values: clear_data,
        clear_column_titles: false,
        clear_row_titles: false,
        override_replicates: None,
        clone_analyses: true,
        clone_graphs: true,
        clone_layouts: true,
        clear_results: clear_data,
    };
    clone_family_into_doc(&mut ctx.session.doc, &req, ctx.cancelled)
}

fn run_sheet_clone_pipeline(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: ClonePipelineParams = parse_params(&command_schema::<ClonePipelineParams>(), params)?;
    let (keep_x, clear_y) = match parsed.clear_data {
        Some(clear_all) => (!clear_all, clear_all),
        None => (parsed.keep_x_values.unwrap_or(true), parsed.clear_y_values.unwrap_or(true)),
    };
    let req = FamilyCloneOptions {
        command_id: "sheet.clonePipeline",
        raw_sheet_id: &parsed.sheet_id,
        new_sheet_id: parsed.new_sheet_id.as_deref(),
        prefix: parsed.prefix.as_deref(),
        title: parsed.title.as_deref(),
        suffix_label: "Clone",
        keep_x_values: keep_x,
        clear_y_values: clear_y,
        clear_column_titles: parsed.clear_column_titles.unwrap_or(false),
        clear_row_titles: parsed.clear_row_titles.unwrap_or(false),
        override_replicates: parsed.override_replicates,
        clone_analyses: parsed.clone_analyses.unwrap_or(true),
        clone_graphs: parsed.clone_graphs.unwrap_or(true),
        clone_layouts: parsed.clone_layouts.unwrap_or(true),
        clear_results: parsed.clear_results.unwrap_or(clear_y),
    };
    clone_family_into_doc(&mut ctx.session.doc, &req, ctx.cancelled)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SheetIdParams {
    /// Target DataSheet ID.
    #[schemars(length(min = 1))]
    sheet_id: String,
}

fn run_sheet_freeze(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    set_sheet_frozen(ctx, "sheet.freeze", params, true)
}

fn run_sheet_unfreeze(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    set_sheet_frozen(ctx, "sheet.unfreeze", params, false)
}

fn set_sheet_frozen(ctx: &mut Ctx<'_>, command_id: &str, params: Value, frozen: bool) -> Result<CommandOutcome, EngineError> {
    let parsed: SheetIdParams = parse_params(&command_schema::<SheetIdParams>(), params)?;
    let id = parse_sheet_id(&parsed.sheet_id)?;
    {
        let ds = require_data_sheet_mut(&mut ctx.session.doc, &id)?;
        ds.frozen = frozen;
    }

    let recalc = if frozen {
        RecalcSummary::default()
    } else {
        recalculate_dirty_with_links(&mut ctx.session.doc, std::slice::from_ref(&id), None, Some(ctx.cancelled))?
    };
    let message = if frozen { "froze data sheet" } else { "unfroze data sheet" };
    Ok(CommandOutcome::mutated(
        command_id,
        message,
        json!({
            "sheet_id": id.as_str(),
            "frozen": frozen,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReorderSheetParams {
    /// Sheet folder (`data`, `info`, `results`, `graphs`, `layouts`).
    folder: Option<String>,
    /// Navigator section name (`data`, `info`, `results`, `analyses`, `graphs`, `layouts`).
    section: Option<String>,
    /// Source zero-based index for index swap/move.
    from_index: Option<usize>,
    /// Destination zero-based index.
    to_index: Option<usize>,
    /// Target sheet ID to move to `index`.
    #[schemars(length(min = 1))]
    sheet_id: Option<String>,
    /// Target zero-based index when reordering by `sheet_id`.
    index: Option<usize>,
    /// Complete ordered list of sheet IDs for `section`.
    #[schemars(with = "Option<Vec<NonEmptyStrSchema>>")]
    order: Option<Vec<String>>,
}

fn parse_sheet_folder_str(raw: &str) -> Result<SheetFolderKind, EngineError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "data" | "tables" => Ok(SheetFolderKind::Data),
        "info" => Ok(SheetFolderKind::Info),
        "results" | "analyses" | "analysis" => Ok(SheetFolderKind::Results),
        "graphs" | "graph" => Ok(SheetFolderKind::Graphs),
        "layouts" | "layout" => Ok(SheetFolderKind::Layouts),
        other => Err(bad_input(format!("unsupported sheet folder `{other}`")).with_remediation("Use one of: data, info, results, graphs, layouts.")),
    }
}

fn reorder_by_explicit_order(doc: &mut ProjectDocument, folder: SheetFolderKind, order_strs: &[String]) -> Result<Vec<SheetId>, EngineError> {
    let mut parsed_order = Vec::with_capacity(order_strs.len());
    for raw in order_strs {
        parsed_order.push(parse_sheet_id(raw)?);
    }
    match folder {
        SheetFolderKind::Data => apply_id_permutation(&mut doc.data_sheets, &parsed_order, |ds| &ds.id)?,
        SheetFolderKind::Info => apply_id_permutation(&mut doc.info_sheets, &parsed_order, |info| &info.id)?,
        SheetFolderKind::Results => apply_id_permutation(&mut doc.result_sheets, &parsed_order, |rs| &rs.id)?,
        SheetFolderKind::Graphs => apply_id_permutation(&mut doc.graph_sheets, &parsed_order, |gs| &gs.id)?,
        SheetFolderKind::Layouts => apply_id_permutation(&mut doc.layout_sheets, &parsed_order, |ls| &ls.id)?,
    }
    Ok(parsed_order)
}

fn apply_id_permutation<T>(items: &mut Vec<T>, order: &[SheetId], id_of: fn(&T) -> &SheetId) -> Result<(), EngineError> {
    if order.len() != items.len() {
        return Err(bad_input(format!("`order` length ({}) does not match section sheet count ({})", order.len(), items.len())));
    }
    let mut by_id: HashMap<SheetId, T> = HashMap::with_capacity(items.len());
    for item in items.drain(..) {
        by_id.insert(id_of(&item).clone(), item);
    }
    for sid in order {
        let Some(item) = by_id.remove(sid) else {
            // Restore drained items on error isn't needed (session rolls back), return structured error cleanly.
            return Err(bad_input(format!("`order` references unknown or duplicate sheet `{}`", sid.as_str())));
        };
        items.push(item);
    }
    Ok(())
}

fn move_sheet_id_to_index(doc: &mut ProjectDocument, id: &SheetId, section: SheetSection, target_index: usize) -> Result<(), EngineError> {
    match section {
        SheetSection::Data => move_item_to_index(&mut doc.data_sheets, id, target_index, |ds| &ds.id),
        SheetSection::Info => move_item_to_index(&mut doc.info_sheets, id, target_index, |info| &info.id),
        SheetSection::Analyses => move_item_to_index(&mut doc.result_sheets, id, target_index, |rs| &rs.id),
        SheetSection::Graphs => move_item_to_index(&mut doc.graph_sheets, id, target_index, |gs| &gs.id),
        SheetSection::Layouts => move_item_to_index(&mut doc.layout_sheets, id, target_index, |ls| &ls.id),
    }
}

fn move_item_to_index<T>(items: &mut Vec<T>, id: &SheetId, target_index: usize, id_of: fn(&T) -> &SheetId) -> Result<(), EngineError> {
    let from = items
        .iter()
        .position(|item| id_of(item) == id)
        .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found in section", id.as_str())))?;
    if target_index >= items.len() {
        return Err(bad_input(format!("target index {target_index} out of bounds for {} sheets", items.len())));
    }
    let item = items.remove(from);
    items.insert(target_index, item);
    Ok(())
}

fn run_sheet_reorder(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: ReorderSheetParams = parse_params(&command_schema::<ReorderSheetParams>(), params)?;
    let section_raw = parsed.folder.as_deref().or(parsed.section.as_deref());

    if let Some(ref order_strs) = parsed.order {
        let raw_folder = section_raw.ok_or_else(|| bad_input("`sheet.reorder` with `order` requires `section` or `folder`"))?;
        let folder = parse_sheet_folder_str(raw_folder)?;
        let ordered = reorder_by_explicit_order(&mut ctx.session.doc, folder, order_strs)?;
        let section = folder.section();
        let ordered_strs: Vec<&str> = ordered.iter().map(SheetId::as_str).collect();
        return Ok(CommandOutcome::mutated(
            "sheet.reorder",
            "reordered section sheets",
            json!({ "section": format!("{section:?}"), "order": ordered_strs }),
        ));
    }

    if parsed.sheet_id.is_none() && (parsed.folder.is_some() || parsed.from_index.is_some()) {
        let raw_folder = section_raw.ok_or_else(|| bad_input("`sheet.reorder` missing `folder`"))?;
        let folder = parse_sheet_folder_str(raw_folder)?;
        let from_index = parsed.from_index.ok_or_else(|| bad_input("`sheet.reorder` requires `from_index`"))?;
        let to_index = parsed.to_index.or(parsed.index).ok_or_else(|| bad_input("`sheet.reorder` requires `to_index`"))?;
        ctx.session.doc.reorder_sheet(folder, from_index, to_index)?;
        return Ok(CommandOutcome::mutated(
            "sheet.reorder",
            "reordered sheet",
            json!({ "folder": format!("{folder:?}"), "from_index": from_index, "to_index": to_index }),
        ));
    }

    let raw_sheet =
        parsed.sheet_id.as_deref().ok_or_else(|| bad_input("`sheet.reorder` requires `sheet_id` or (`folder`, `from_index`, `to_index`)"))?;
    let id = parse_sheet_id(raw_sheet)?;
    let target_index = parsed.index.or(parsed.to_index).ok_or_else(|| bad_input("`sheet.reorder` requires `index` or `to_index`"))?;
    let section = ctx
        .session
        .doc
        .sheet_section(&id)
        .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", id.as_str())))?;

    if let Some(raw_folder) = section_raw {
        let expected_section = parse_sheet_folder_str(raw_folder)?.section();
        if expected_section != section {
            return Err(bad_input(format!("sheet `{}` belongs to `{section:?}`, not `{expected_section:?}`", id.as_str())));
        }
    }

    move_sheet_id_to_index(&mut ctx.session.doc, &id, section, target_index)?;
    Ok(CommandOutcome::mutated(
        "sheet.reorder",
        "reordered sheet",
        json!({ "sheet_id": id.as_str(), "section": format!("{section:?}"), "index": target_index }),
    ))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SelectSheetParams {
    /// Sheet ID to select.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Selection pane (`navigator` or `family`, defaults to `navigator`).
    mode: Option<String>,
}

fn run_sheet_select(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: SelectSheetParams = parse_params(&command_schema::<SelectSheetParams>(), params)?;
    let id = parse_sheet_id(&parsed.sheet_id)?;
    let mode = parsed.mode.as_deref().unwrap_or("navigator").trim();
    if mode.eq_ignore_ascii_case("family") {
        ctx.session.doc.select_in_family_pane(&id)?;
    } else if mode.eq_ignore_ascii_case("navigator") {
        ctx.session.doc.select_in_navigator(&id)?;
    } else {
        return Err(bad_input(format!("unsupported selection mode `{mode}`")).with_remediation("Use `navigator` or `family`."));
    }

    Ok(CommandOutcome::read_only(
        "sheet.select",
        "selected sheet",
        json!({
            "sheet_id": id.as_str(),
            "mode": mode,
            "active_sheet_id": ctx.session.doc.active_sheet_id.as_ref().map(SheetId::as_str),
            "family_anchor_sheet_id": ctx.session.doc.family_anchor_sheet_id.as_ref().map(SheetId::as_str),
        }),
    ))
}

fn extract_archetype_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "anyOf": [
            { "type": "string", "enum": ["auto", "column", "multiple_variables", "grouped"] },
            { "type": "null" }
        ]
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExtractResultTabParams {
    /// Source ResultSheet ID whose tab should be extracted.
    #[schemars(length(min = 1))]
    result_sheet_id: String,
    /// ResultTab ID or exact title (defaults to first tab).
    tab_id: Option<String>,
    /// Optional destination DataSheet ID (auto-generated when omitted).
    #[schemars(length(min = 1))]
    sheet_id: Option<String>,
    /// Optional title for the extracted linked DataSheet.
    title: Option<String>,
    /// Target table archetype: auto (default), column, multiple_variables, or grouped.
    #[serde(default)]
    #[schemars(schema_with = "extract_archetype_schema")]
    archetype: Option<String>,
    /// When true, also create an uninitialized GraphSheet linked to the extracted DataSheet.
    auto_graph: Option<bool>,
}

fn tab_has_non_numeric_cells(tab: &ResultTab) -> bool {
    tab.rows.iter().flatten().any(|cell| matches!(cell, ResultCell::Text(_) | ResultCell::Boolean(_) | ResultCell::ConfidenceInterval { .. }))
}

fn resolve_extracted_table_kind(tab: &ResultTab, raw: Option<&str>) -> Result<TableKind, EngineError> {
    match raw.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
        "auto" => {
            if tab_has_non_numeric_cells(tab) {
                Ok(TableKind::MultipleVariables)
            } else {
                Ok(TableKind::Column)
            }
        }
        "column" => Ok(TableKind::Column),
        "multiple_variables" | "multivariate" => Ok(TableKind::MultipleVariables),
        "grouped" => Ok(TableKind::Grouped),
        other => Err(bad_input(format!(
            "unsupported `sheet.extractResultTab` archetype `{other}` (expected auto, column, multiple_variables, or grouped)"
        ))),
    }
}

fn run_sheet_extract_result_tab(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: ExtractResultTabParams = parse_params(&command_schema::<ExtractResultTabParams>(), params)?;
    let res_id = parse_sheet_id(&parsed.result_sheet_id)?;

    let (res, tab) = {
        let doc = &ctx.session.doc;
        let res = require_result_sheet(doc, &res_id)?;
        let tab = select_result_tab(res, parsed.tab_id.as_deref())?;
        (res.clone(), tab.clone())
    };

    if tab.rows.is_empty() {
        return Err(EngineError::with_code(ErrorCode::Conflict, format!("result tab `{}` on `{res_id}` has no rows to extract", tab.id))
            .with_remediation("Select a tabular ResultTab with at least one data row."));
    }

    let row_count = tab.rows.len();
    let col_count = tab.rows.iter().map(Vec::len).max().unwrap_or(0).max(tab.column_headers.len()).max(1);
    let target_kind = resolve_extracted_table_kind(&tab, parsed.archetype.as_deref())?;
    let existing_cells = document_used_cells(&ctx.session.doc);
    validate_sheet_and_project_budget(existing_cells, row_count, col_count, 1, 0)?;

    let target_id = allocate_new_sheet_id(&ctx.session.doc, parsed.sheet_id.as_deref(), "data")?;
    let title = match parsed.title.as_deref() {
        Some(t) => {
            let clean = sanitize_title_controls(t);
            validate_title(&clean)?;
            clean
        }
        None => clip_safe_title(&format!("{} - {}", res.title(), tab.title)),
    };

    let archetype = match target_kind {
        TableKind::Column => TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, col_count, row_count)?),
        TableKind::Grouped => TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(1), col_count, row_count)?),
        TableKind::MultipleVariables => TableArchetype::MultipleVariables(MultipleVariablesTable::new(col_count, row_count)?),
        other => {
            return Err(bad_input(format!("unsupported target archetype `{other:?}` for `sheet.extractResultTab`")));
        }
    };

    let mut sheet = DataSheet::new(target_id.clone(), title.clone(), archetype)?;
    let link_kind = if res.analyzable_data.is_some() { InternalLinkKind::AnalyzableResultSheet } else { InternalLinkKind::TerminalResultSheet };
    let link_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: res_id.clone(),
            kind: link_kind,
            tab_id: Some(tab.id.clone()),
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1_000_000, 1_000_000, 0)),
        },
        filter: ImportFilterSpec { parse_asterisk_exclusions: false, ..ImportFilterSpec::default() },
        placement: ImportPlacementSpec {
            mode: PlacementMode::Overwrite,
            anchor: CellCoord::new(0, 0, 0),
            propagate_title_header: false,
            skip_contiguous_blanks: false,
            transpose: false,
        },
    };
    let opts = ImportPipelineOptions {
        link_id: Some(format!("result-tab-{}-{}", res_id.as_str(), target_id.as_str())),
        provenance_summary: Some(format!("Linked tab `{}` from {}", tab.title, res_id)),
        ..ImportPipelineOptions::default()
    };
    let report = apply_import_link_pipeline(&mut sheet, &link_spec, ImportSourcePayload::InternalResult(&res), &opts, None)?;
    sheet.prune_stale_exclusion_notes();
    sheet.prune_stale_plate_well_origins();
    apply_result_tab_metadata_to_sheet(&mut sheet, &tab);

    let mut reserved = HashSet::new();
    reserved.insert(target_id.as_str().to_owned());
    let new_graph = if parsed.auto_graph.unwrap_or(false) {
        let gid = next_unique_id_with_reserved(&ctx.session.doc, "graph", &reserved)?;
        Some(GraphSheet::new_uninitialized_for_data(gid, &sheet)?)
    } else {
        None
    };
    let graph_id = new_graph.as_ref().map(|g| g.id.as_str().to_owned());

    validate_project_cells(existing_cells, sheet.archetype.allocated_cell_slots())?;
    ctx.session.doc.add_data_sheet(sheet)?;
    if let Some(graph) = new_graph {
        ctx.session.doc.add_graph_sheet(graph)?;
    }
    let _ = crate::dag::topological_order(&ctx.session.doc)?;

    Ok(CommandOutcome::mutated(
        "sheet.extractResultTab",
        format!("extracted tab `{}` from `{res_id}` into `{target_id}`", tab.id),
        json!({
            "sheet_id": target_id.as_str(),
            "title": title,
            "result_sheet_id": res_id.as_str(),
            "tab_id": tab.id,
            "tab_title": tab.title,
            "archetype": target_kind,
            "rows_placed": report.rows_placed,
            "cols_placed": report.cols_placed,
            "graph_sheet_id": graph_id,
        }),
    ))
}

static SPECS: &[CommandSpec] = &command_specs!(
    CommandGroup::Sheet;
    {
        id: "sheet.createData",
        label: "New Data Table",
        menu_path: "Insert/New Data Table",
        shortcut: "Cmd+T",
        summary: "Create a typed data table sheet and optional paired info/graph sheets.",
        beginner: "Creates one of the eight scientific table archetypes (XY, Column, Grouped, Contingency, Survival, Parts of Whole, Multiple Variables, or Nested).",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: create_data_schema,
        enabled: always_enabled,
        run: run_sheet_create_data,
    },
    {
        id: "sheet.rename",
        label: "Rename Sheet",
        menu_path: "Change/Rename Sheet",
        summary: "Rename any sheet in Data, Info, Results, Graphs, or Layouts.",
        beginner: "Updates the display title of a sheet (refusing frozen DataSheets) and refreshes dependent labels.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: rename_sheet_schema,
        enabled: always_enabled,
        run: run_sheet_rename,
    },
    {
        id: "sheet.delete",
        label: "Delete Sheet",
        menu_path: "Edit/Delete Sheet",
        summary: "Delete a sheet and optionally cascade to its dependent family.",
        beginner: "Removes a sheet from the project, blocking deletion on frozen sheets or when `cascade: false` with active dependents.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA),
        input_schema: command_schema::<DeleteSheetParams>,
        enabled: always_enabled,
        run: run_sheet_delete,
    },
    {
        id: "sheet.duplicate",
        label: "Duplicate Sheet",
        menu_path: "Edit/Duplicate Sheet",
        summary: "Duplicate a single sheet within its section.",
        beginner: "Creates an independent copy of one Data, Info, Result, Graph, or Layout sheet.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<DuplicateSheetParams>,
        enabled: always_enabled,
        run: run_sheet_duplicate,
    },
    {
        id: "sheet.duplicateFamily",
        label: "Duplicate Family",
        menu_path: "Edit/Duplicate Family",
        summary: "Duplicate a data sheet together with its paired info, analyses, graphs, and layouts.",
        beginner: "Clones a complete linked family of sheets so all formulas, analyses, and figures remain wired to the copied table.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<DuplicateFamilyParams>,
        enabled: always_enabled,
        run: run_sheet_duplicate_family,
    },
    {
        id: "sheet.clonePipeline",
        label: "Clone Pipeline",
        menu_path: "Change/Clone Pipeline...",
        summary: "Clone a table structure and downstream analysis/graph/layout pipeline for a new experimental run.",
        beginner: "Creates a new table preserving X coordinates, analyses, graphs, and layouts while clearing Y replicates for new measurements.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<ClonePipelineParams>,
        enabled: always_enabled,
        run: run_sheet_clone_pipeline,
    },
    {
        id: "sheet.freeze",
        label: "Freeze Sheet",
        menu_path: "Change/Freeze Sheet",
        summary: "Lock a DataSheet against accidental cell, link, or structural edits.",
        beginner: "Prevents mutations, link refreshes, renaming, and deletion on a DataSheet while allowing downstream reads and graphs.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SheetIdParams>,
        enabled: always_enabled,
        run: run_sheet_freeze,
    },
    {
        id: "sheet.unfreeze",
        label: "Unfreeze Sheet",
        menu_path: "Change/Unfreeze Sheet",
        summary: "Unlock a frozen DataSheet and recalculate linked dependents.",
        beginner: "Restores mutability to a frozen DataSheet and triggers immediate link/DAG recalculation.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SheetIdParams>,
        enabled: always_enabled,
        run: run_sheet_unfreeze,
    },
    {
        id: "sheet.reorder",
        label: "Reorder Sheet",
        menu_path: "Change/Reorder Sheet",
        summary: "Reorder sheets within a project navigator folder.",
        beginner: "Moves a sheet to a new index by (`sheet_id`, `index`), by (`folder`, `from_index`, `to_index`), or by explicit `order` array.",
        effects: Effects::MUTATES_DOCUMENT.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<ReorderSheetParams>,
        enabled: always_enabled,
        run: run_sheet_reorder,
    },
    {
        id: "sheet.select",
        label: "Select Sheet",
        menu_path: "View/Select Sheet",
        summary: "Select the active sheet in the navigator or family pane.",
        beginner: "Focuses a sheet in the navigator (`mode: \"navigator\"`) or sub-sheet family view (`mode: \"family\"`).",
        effects: Effects::CHANGES_UI_STATE.with(Effects::IDEMPOTENT),
        input_schema: command_schema::<SelectSheetParams>,
        enabled: always_enabled,
        run: run_sheet_select,
    },
    {
        id: "sheet.extractResultTab",
        label: "Extract Result Tab to Data Table",
        menu_path: "Insert/Extract Result Tab...",
        summary: "Create a reactive linked DataSheet from a ResultSheet sub-tab (such as PC scores, residuals, or cleaned data).",
        beginner: "Copies a computed result table tab into a new linked data table so you can analyze or graph it.",
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: command_schema::<ExtractResultTabParams>,
        enabled: has_result_sheet,
        run: run_sheet_extract_result_tab,
    },
);

/// Registered command specifications for the `sheet` group.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    SPECS
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use serde_json::json;

    use crate::{Session, cmd::Caller};

    use super::*;

    fn find_spec(id: &str) -> &'static CommandSpec {
        specs().iter().find(|s| s.id == id).unwrap()
    }

    fn new_session(title: &str) -> Session {
        Session::from_document(ProjectDocument::new(title).unwrap())
    }

    fn exec_cmd(session: &mut Session, id: &str, caller: Caller, params: Value) -> Result<CommandOutcome, EngineError> {
        let cancel = AtomicBool::new(false);
        session.execute_registered_spec(find_spec(id), params, caller, &cancel)
    }

    #[test]
    fn create_select_rename_freeze_unfreeze_duplicate_and_reorder() {
        let mut session = new_session("Sheet Ops");
        let created = exec_cmd(
            &mut session,
            "sheet.createData",
            Caller::Ui,
            json!({ "sheet_id": "d1", "title": "Dose", "archetype": "xy", "cols": 2, "rows": 4, "auto_family": true }),
        )
        .unwrap();
        assert!(created.mutated);
        assert_eq!(created.payload["kind"], json!("Xy"));
        let info_id = created.payload["info_sheet_id"].as_str().unwrap().to_owned();
        let graph_id = created.payload["graph_sheet_id"].as_str().unwrap().to_owned();
        assert_eq!(session.document().family_anchor_sheet_id.as_ref().map(SheetId::as_str), Some("d1"));

        exec_cmd(&mut session, "sheet.select", Caller::Ui, json!({ "sheet_id": graph_id, "mode": "family" })).unwrap();
        assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some(graph_id.as_str()));
        assert_eq!(session.document().family_anchor_sheet_id.as_ref().map(SheetId::as_str), Some("d1"));

        exec_cmd(&mut session, "sheet.freeze", Caller::Ui, json!({ "sheet_id": "d1" })).unwrap();
        let frozen_rename = exec_cmd(&mut session, "sheet.rename", Caller::Mcp, json!({ "sheet_id": "d1", "title": "Nope" })).unwrap_err();
        assert_eq!(frozen_rename.code, ErrorCode::FrozenSheet);
        let frozen_delete = exec_cmd(&mut session, "sheet.delete", Caller::Mcp, json!({ "sheet_id": "d1" })).unwrap_err();
        assert_eq!(frozen_delete.code, ErrorCode::FrozenSheet);

        exec_cmd(&mut session, "sheet.unfreeze", Caller::Ui, json!({ "sheet_id": "d1" })).unwrap();
        exec_cmd(&mut session, "sheet.rename", Caller::Ui, json!({ "sheet_id": "d1", "title": "Renamed Dose" })).unwrap();
        assert_eq!(session.document().data_sheet(&SheetId::new("d1")).unwrap().title, "Renamed Dose");

        let fam = exec_cmd(&mut session, "sheet.duplicateFamily", Caller::Ui, json!({ "sheet_id": "d1", "prefix": "fam" })).unwrap();
        let fam_id = fam.payload["sheet_id"].as_str().unwrap().to_owned();
        assert!(fam_id.starts_with("fam"));
        assert_eq!(fam.payload["graph_sheet_ids"].as_array().unwrap().len(), 1);

        exec_cmd(&mut session, "sheet.reorder", Caller::Ui, json!({ "folder": SheetFolderKind::Data, "from_index": 0, "to_index": 1 })).unwrap();
        assert_eq!(session.document().data_sheets[1].id, SheetId::new("d1"));

        exec_cmd(&mut session, "sheet.reorder", Caller::Mcp, json!({ "section": "data", "order": ["d1", fam_id] })).unwrap();
        assert_eq!(session.document().data_sheets[0].id, SheetId::new("d1"));

        exec_cmd(&mut session, "sheet.delete", Caller::Ui, json!({ "sheet_id": "d1", "cascade": true })).unwrap();
        assert!(session.document().data_sheet(&SheetId::new("d1")).is_none());
        assert!(session.document().info_sheet(&SheetId::new(&info_id)).is_none());
        assert!(session.document().graph_sheet(&SheetId::new(&graph_id)).is_none());
    }

    #[test]
    fn duplicate_and_clone_pipeline_handle_multivariate_and_cascade_refusal() {
        let mut session = new_session("Pipeline Ops");
        exec_cmd(
            &mut session,
            "sheet.createData",
            Caller::Mcp,
            json!({
                "sheet_id": "mv-1",
                "archetype": "multiple_variables",
                "rows": 3,
                "variables": ["Score", { "title": "Group", "role": "categorical" }],
                "auto_family": true
            }),
        )
        .unwrap();
        let dup = exec_cmd(&mut session, "sheet.duplicate", Caller::Ui, json!({ "sheet_id": "mv-1", "new_sheet_id": "mv-copy" })).unwrap();
        assert_eq!(dup.payload["sheet_id"], json!("mv-copy"));

        let no_cascade_err = exec_cmd(&mut session, "sheet.delete", Caller::Mcp, json!({ "sheet_id": "mv-1", "cascade": false })).unwrap_err();
        assert_eq!(no_cascade_err.code, ErrorCode::Conflict);

        let cloned =
            exec_cmd(&mut session, "sheet.clonePipeline", Caller::Ui, json!({ "sheet_id": "mv-1", "new_sheet_id": "mv-run2", "title": "Run 2" }))
                .unwrap();
        assert_eq!(cloned.payload["sheet_id"], json!("mv-run2"));
        assert_eq!(cloned.payload["graph_sheet_ids"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn create_data_accepts_replicate_and_mean_upper_lower_subcolumn_aliases() {
        let mut session = new_session("Subcolumn Aliases");
        exec_cmd(
            &mut session,
            "sheet.createData",
            Caller::Cli,
            json!({
                "sheet_id": "xy_rep",
                "kind": "xy",
                "rows": 4,
                "cols": 1,
                "replicates": 3,
                "subcolumn_mode": "replicate",
                "auto_graph": false
            }),
        )
        .unwrap();
        exec_cmd(
            &mut session,
            "sheet.createData",
            Caller::Cli,
            json!({
                "sheet_id": "xy_mul",
                "kind": "xy",
                "rows": 4,
                "cols": 1,
                "subcolumn_mode": "mean_upper_lower",
                "auto_graph": false
            }),
        )
        .unwrap();
        exec_cmd(
            &mut session,
            "sheet.createData",
            Caller::Cli,
            json!({
                "sheet_id": "grp_mul",
                "kind": "grouped",
                "rows": 2,
                "cols": 2,
                "subcolumn_mode": "meanupperlower",
                "auto_graph": false
            }),
        )
        .unwrap();

        match &session.document().data_sheet(&SheetId::new("xy_rep")).unwrap().archetype {
            TableArchetype::Xy(xy) => assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::RawReplicates(3)),
            other => panic!("expected Xy table, got {other:?}"),
        }
        match &session.document().data_sheet(&SheetId::new("xy_mul")).unwrap().archetype {
            TableArchetype::Xy(xy) => assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::UpperLowerLimits),
            other => panic!("expected Xy table, got {other:?}"),
        }
        match &session.document().data_sheet(&SheetId::new("grp_mul")).unwrap().archetype {
            TableArchetype::Grouped(grp) => assert_eq!(grp.subcolumn_mode, SubcolumnMode::UpperLowerLimits),
            other => panic!("expected Grouped table, got {other:?}"),
        }
    }

    #[test]
    fn extract_result_tab_and_analyzable_chaining_propagate_reactively() {
        let mut session = Session::from_document(ProjectDocument::new("Chained Analysis").unwrap());
        session
            .execute(
                "sheet.createData",
                json!({
                    "sheet_id": "xy-1",
                    "kind": "xy",
                    "cols": 1,
                    "rows": 6,
                    "x_title": "Dose",
                    "title": "Raw Curves"
                }),
            )
            .unwrap();
        session.execute("table.setXCells", json!({ "sheet_id": "xy-1", "values": [1.0, 2.0, 3.0, 4.0, 5.0, 6.0] })).unwrap();
        for (r, y) in [10.0, 22.0, 55.0, 88.0, 96.0, 99.0].iter().enumerate() {
            session.execute("table.setCell", json!({ "sheet_id": "xy-1", "row": r, "col": 0, "value": y })).unwrap();
        }

        // Direct chaining on an analyzable Transform ResultSheet (`analyzable_data`):
        session
            .execute(
                "transform.run",
                json!({
                    "sheet_id": "norm-1",
                    "source_sheet_id": "xy-1",
                    "kind": "normalize"
                }),
            )
            .unwrap();
        session
            .execute(
                "fit.run",
                json!({
                    "sheet_id": "norm-stats",
                    "source_sheet_id": "norm-1",
                    "kind": "linear_regression"
                }),
            )
            .unwrap();
        session
            .execute(
                "graph.create",
                json!({
                    "sheet_id": "g-norm",
                    "data_sheet_id": "norm-1",
                    "kind": "xy"
                }),
            )
            .unwrap();
        assert_eq!(session.document().result_sheet(&SheetId::new("norm-stats")).unwrap().status, ResultStatus::UpToDate);

        // Chaining via `sheet.extractResultTab` on a non-analyzable nonlinear fit residuals tab:
        session
            .execute(
                "fit.run",
                json!({
                    "sheet_id": "fit-1",
                    "source_sheet_id": "xy-1",
                    "kind": "nonlinear_regression",
                    "emit_residuals_tab": true
                }),
            )
            .unwrap();

        let extracted = session
            .execute(
                "sheet.extractResultTab",
                json!({
                    "result_sheet_id": "fit-1",
                    "tab_id": "nonlinear_residuals",
                    "sheet_id": "res-data",
                    "archetype": "column",
                    "auto_graph": true
                }),
            )
            .unwrap();
        assert_eq!(extracted.payload["sheet_id"], json!("res-data"));
        assert_eq!(extracted.payload["tab_id"], json!("nonlinear_residuals"));

        let initial_cells = match &session.document().data_sheet(&SheetId::new("res-data")).unwrap().archetype {
            TableArchetype::Column(col) => col.data_sets.iter().map(|ds| ds.cells.clone()).collect::<Vec<_>>(),
            other => panic!("expected Column table, got {other:?}"),
        };

        session
            .execute(
                "stats.run",
                json!({
                    "sheet_id": "res-desc",
                    "source_sheet_id": "res-data",
                    "kind": "descriptive_stats"
                }),
            )
            .unwrap();
        assert_eq!(session.document().result_sheet(&SheetId::new("res-desc")).unwrap().status, ResultStatus::UpToDate);

        let initial_desc_tabs = session.document().result_sheet(&SheetId::new("res-desc")).unwrap().tabs.clone();

        // Edit upstream input cell: reactive DAG recomputes `fit-1` -> refreshes `res-data` -> recomputes `res-desc`.
        let set_out = session.execute("table.setCell", json!({ "sheet_id": "xy-1", "row": 0, "col": 0, "value": 25.0 })).unwrap();
        assert!(set_out.payload["recalculated_sheets"].as_u64().unwrap() >= 3);

        let updated_cells = match &session.document().data_sheet(&SheetId::new("res-data")).unwrap().archetype {
            TableArchetype::Column(col) => col.data_sets.iter().map(|ds| ds.cells.clone()).collect::<Vec<_>>(),
            other => panic!("expected Column table, got {other:?}"),
        };
        assert_ne!(initial_cells, updated_cells, "upstream cell edit must refresh extracted ResultTab DataSheet");
        let updated_desc = session.document().result_sheet(&SheetId::new("res-desc")).unwrap();
        assert_eq!(updated_desc.status, ResultStatus::UpToDate);
        assert_ne!(initial_desc_tabs, updated_desc.tabs, "downstream analysis on extracted DataSheet must recompute");
    }
}
