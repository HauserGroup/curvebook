//! Table archetype conversion planning, dependent-sheet impact analysis, and execution (`table.convertArchetype`).

use std::collections::HashSet;

use curvebook_doc::{
    AnalysisKind, ColorChannelMapping, DataSetColumn, DataSheet, DocError, HookSource, InternalLinkKind, LinkSource, MAX_DATA_SETS,
    MultipleVariablesTable, MultiplicityMethod, MultivariateSpec, NormalizeAnchor, ParameterScope, PointStyleOverride, ProjectDocument,
    SheetCellRange, SheetId, SizeChannelMapping, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, TableKind, TransformAnalysisSpec, VariableRole,
    XColumnMode,
};
use curvebook_transform::{TargetArchetypeKind, convert_datasheet_archetype, preview_datasheet_conversion};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::super::Ctx;
use super::super::helpers::{bad_input, column_subcolumn_count, ensure_sheet_not_frozen, parse_sheet_id};
use super::super::params::{command_schema, parse_params};
use super::super::project::recalculate_dirty_with_links;
use super::{parse_target_archetype_kind, validate_project_cell_budget_after_change};
use crate::{CommandOutcome, EngineError, ErrorCode, dag};

fn convert_target_kind_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "enum": [
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
            "multivariate"
        ]
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ConvertArchetypeParams {
    /// Target data sheet identifier.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Target table archetype.
    #[schemars(schema_with = "convert_target_kind_schema")]
    target_kind: String,
    /// Permit conversion even if subcolumns or incompatible columns are reshaped/dropped (default false).
    allow_lossy: Option<bool>,
    /// Return conversion preview without mutating document (default false).
    preview_only: Option<bool>,
}

pub(super) fn run_convert_archetype(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let p: ConvertArchetypeParams = parse_params(&command_schema::<ConvertArchetypeParams>(), raw)?;
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

    let target_ds = doc
        .data_sheets
        .get(idx)
        .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("data sheet `{}` not found", sheet_id.as_str())))?;
    if preview_only {
        let base_preview = preview_datasheet_conversion(target_ds, target_kind)?;
        let impact = plan_conversion_impacts(doc, target_ds, target_kind);
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

    ensure_sheet_not_frozen(target_ds)?;
    let old_ds = target_ds.clone();
    let impact = plan_conversion_impacts(doc, &old_ds, target_kind);
    ensure_no_frozen_dependent_link_mutation(doc, &sheet_id, &impact)?;
    let (mut converted, base_preview) = convert_datasheet_archetype(&old_ds, target_kind, allow_lossy)?;
    if impact.incoming_links_lost && !allow_lossy {
        return Err(bad_input(format!(
            "lossy conversion of `{}` drops {} live incoming link(s); pass `allow_lossy: true` to detach them",
            sheet_id.as_str(),
            impact.incoming_links_dropped
        )));
    }
    converted.id = sheet_id.clone();
    validate_project_cell_budget_after_change(doc, Some(&sheet_id), converted.archetype.allocated_cell_slots())?;
    preserve_mappable_incoming_links(&old_ds, &mut converted, &impact.col_plan);
    converted.prune_stale_exclusion_notes();
    if let Some(slot) = doc.data_sheets.get_mut(idx) {
        *slot = converted;
    }
    apply_conversion_dependents(doc, &sheet_id, &impact);
    let mut recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&sheet_id), None, Some(ctx.cancelled))?;
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
        } else if var.role_inferred && var.cells.iter().any(|c| !c.is_empty() && matches!(c.value, curvebook_doc::CellValue::Text(_))) {
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
        if let [only_num] = num_cols.as_slice() {
            if let Some(slot) = cols.get_mut(*only_num) {
                *slot = ColMapTarget::Exact(0);
            }
            return Self {
                cols,
                x_injected_at_col0: false,
                preserves_cell_shape: cat_cols.is_empty() && label_cols.is_empty(),
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: 1,
            };
        }
        if let Some((&first_num, rest_nums)) = num_cols.split_first()
            && !rest_nums.is_empty()
        {
            if let Some(slot) = cols.get_mut(first_num) {
                *slot = ColMapTarget::MovedToX;
            }
            for (yi, &nc) in rest_nums.iter().enumerate() {
                if let Some(slot) = cols.get_mut(nc) {
                    *slot = ColMapTarget::Exact(yi);
                }
            }
            return Self {
                cols,
                x_injected_at_col0: false,
                preserves_cell_shape: cat_cols.is_empty() && label_cols.is_empty(),
                transposes_row_subcol: false,
                target_table_kind,
                converted_col_count: rest_nums.len(),
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
        let is_cross_tab = matches!(
            cat_cols.as_slice(),
            &[c0, c1]
                if mv.variables.get(c0).is_some_and(|v| v.role == VariableRole::Categorical)
                    && mv.variables.get(c1).is_some_and(|v| v.role == VariableRole::Categorical)
        ) && (num_cols.is_empty() || (num_cols.len() == 1 && old_cols == 3 + label_cols.len()));
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
            if let Some(slot) = cols.get_mut(src_c) {
                *slot = ColMapTarget::Exact(dst_c);
            }
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
                curvebook_doc::CellCoord::new(range.start.row, start_col, range.start.subcol),
                curvebook_doc::CellCoord::new(range.end.row, range.end.col, range.end.subcol),
            ));
        }
        let end_c = range.end.col.min(old_cols.saturating_sub(1));
        if range.start.col == end_c
            && let Some(ColMapTarget::Expanded(vec)) = self.cols.get(range.start.col)
            && range.start.subcol == range.end.subcol
            && let Some(&single_col) = vec.get(range.start.subcol)
        {
            return Ok(SheetCellRange::new(
                curvebook_doc::CellCoord::new(range.start.row, single_col, 0),
                curvebook_doc::CellCoord::new(range.end.row, single_col, 0),
            ));
        }
        let mapped = self.map_col_list(&(range.start.col..=end_c).collect::<Vec<_>>(), "linked source range")?;
        let min_c = *mapped.iter().min().ok_or_else(|| String::from("linked source range mapped to zero columns"))?;
        let max_c = *mapped.iter().max().ok_or_else(|| String::from("linked source range mapped to zero columns"))?;
        let contiguous = mapped.len() == max_c - min_c + 1;
        if !contiguous {
            return Err(String::from("linked source column range became non-contiguous after conversion"));
        }
        Ok(SheetCellRange::new(
            curvebook_doc::CellCoord::new(range.start.row, min_c, range.start.subcol),
            curvebook_doc::CellCoord::new(range.end.row, max_c, range.end.subcol),
        ))
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
            curvebook_doc::CellCoord::new(range.start.row, first, range.start.subcol),
            curvebook_doc::CellCoord::new(range.end.row, new_end_col, range.end.subcol),
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

fn ensure_no_frozen_dependent_link_mutation(doc: &ProjectDocument, target_sheet_id: &SheetId, impact: &ConversionImpact) -> Result<(), EngineError> {
    for ds in &doc.data_sheets {
        if ds.id == *target_sheet_id || !ds.frozen {
            continue;
        }
        let mutates_frozen_link = ds.linked_regions.iter().any(|region| {
            impact.broken_link_ids.iter().any(|(sid, rid)| sid == &ds.id && rid == &region.id)
                || matches!(
                    &region.spec.source,
                    LinkSource::InternalSheet { sheet_id, kind: InternalLinkKind::DataSheet, range, .. }
                        if *sheet_id == *target_sheet_id && impact.col_plan.map_source_range(*range).is_ok_and(|mapped| mapped != *range)
                )
        });
        if mutates_frozen_link {
            return Err(EngineError::from(DocError::SheetFrozen { sheet_id: ds.id.clone() }));
        }
    }
    Ok(())
}

fn remap_dunnett_control_column(method: &mut MultiplicityMethod, label: &str, plan: &ConversionColPlan, err: &mut Option<String>) {
    if let MultiplicityMethod::Dunnett { control_column } = method {
        plan.map_one_or_oob(control_column, label, err);
    }
}

fn remap_transform_kind_columns(spec: &mut TransformAnalysisSpec, plan: &ConversionColPlan, err: &mut Option<String>) {
    match spec {
        TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, .. } => {
            if let NormalizeAnchor::ControlColumnMean { column } = zero_anchor {
                plan.map_one_or_oob(column, "Normalize zero_anchor control column", err);
            }
            if let NormalizeAnchor::ControlColumnMean { column } = hundred_anchor {
                plan.map_one_or_oob(column, "Normalize hundred_anchor control column", err);
            }
        }
        TransformAnalysisSpec::RemoveBaseline { baseline_columns, .. } => {
            *baseline_columns = plan.map_col_list_or_oob(baseline_columns, "RemoveBaseline baseline_columns", err);
        }
        TransformAnalysisSpec::RandomSample { stratify_column, .. } => {
            plan.map_opt_or_oob(stratify_column, "RandomSample stratify_column", err);
        }
        TransformAnalysisSpec::Math { .. }
        | TransformAnalysisSpec::Transpose { .. }
        | TransformAnalysisSpec::PruneRows { .. }
        | TransformAnalysisSpec::SelectAndTransform { .. }
        | TransformAnalysisSpec::Simulate { .. } => {}
    }
}

fn remap_survival_kind_columns(spec: &mut SurvivalAnalysisSpec, plan: &ConversionColPlan, err: &mut Option<String>) {
    match spec {
        SurvivalAnalysisSpec::KaplanMeier { multiplicity, mv_time_column, mv_status_column, mv_group_column, .. } => {
            let uses_cohort_columns = mv_group_column.is_none();
            plan.map_opt_or_oob(mv_time_column, "KaplanMeier mv_time_column", err);
            plan.map_opt_or_oob(mv_status_column, "KaplanMeier mv_status_column", err);
            plan.map_opt_or_oob(mv_group_column, "KaplanMeier mv_group_column", err);
            if uses_cohort_columns {
                remap_dunnett_control_column(&mut multiplicity.method, "KaplanMeier Dunnett control_column", plan, err);
            }
        }
        SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, reference_levels, .. } => {
            plan.map_one_or_oob(time_column, "Cox time_column", err);
            plan.map_one_or_oob(status_column, "Cox status_column", err);
            *predictor_columns = plan.map_col_list_or_oob(predictor_columns, "Cox predictor_columns", err);
            for (ref_col, _) in reference_levels.iter_mut() {
                plan.map_one_or_oob(ref_col, "Cox reference_levels column", err);
            }
        }
    }
}

fn remap_multivariate_kind_columns(spec: &mut MultivariateSpec, plan: &ConversionColPlan, err: &mut Option<String>) {
    match spec {
        MultivariateSpec::Pca { pcr_outcome_column, .. } => {
            plan.map_opt_or_oob(pcr_outcome_column, "Pca pcr_outcome_column", err);
        }
        MultivariateSpec::MultipleImputation { response_column, predictor_columns, .. } => {
            plan.map_opt_or_oob(response_column, "MultipleImputation response_column", err);
            *predictor_columns = plan.map_col_list_or_oob(predictor_columns, "MultipleImputation predictor_columns", err);
        }
        MultivariateSpec::CorrelationMatrix { .. } | MultivariateSpec::Clustering { .. } => {}
    }
}

fn remap_analysis_kind_columns(kind: &mut AnalysisKind, plan: &ConversionColPlan, err: &mut Option<String>) {
    match kind {
        AnalysisKind::Transform(spec) => remap_transform_kind_columns(spec, plan, err),
        AnalysisKind::OneWayAnova(spec) => remap_dunnett_control_column(&mut spec.multiplicity.method, "Dunnett control_column", plan, err),
        AnalysisKind::MultipleRowTTests(spec) => {
            remap_dunnett_control_column(&mut spec.multiplicity.method, "MultipleRowTTests Dunnett control_column", plan, err);
        }
        AnalysisKind::NestedComparison(spec) => {
            remap_dunnett_control_column(&mut spec.multiplicity.method, "NestedComparison Dunnett control_column", plan, err);
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
            remap_dunnett_control_column(&mut spec.multiplicity.method, "FactorialAnova Dunnett control_column", plan, err);
        }
        AnalysisKind::Survival(spec) => remap_survival_kind_columns(spec, plan, err),
        AnalysisKind::GeneralizedLinearModel(spec) => {
            plan.map_one_or_oob(&mut spec.response_column, "GLM response_column", err);
            spec.predictor_columns = plan.map_col_list_or_oob(&spec.predictor_columns, "GLM predictor_columns", err);
            for (ref_col, _) in &mut spec.reference_levels {
                plan.map_one_or_oob(ref_col, "GLM reference_levels column", err);
            }
        }
        AnalysisKind::Multivariate(spec) => remap_multivariate_kind_columns(spec, plan, err),
        AnalysisKind::NonlinearRegression(nl) => {
            nl.mv_group_columns = plan.map_col_list_or_oob(&nl.mv_group_columns, "NonlinearRegression mv_group_columns", err);
            nl.mv_x_columns = plan.map_col_list_or_oob(&nl.mv_x_columns, "NonlinearRegression mv_x_columns", err);
            plan.map_opt_or_oob(&mut nl.mv_y_column, "NonlinearRegression mv_y_column", err);
            nl.mv_y_columns = plan.map_col_list_or_oob(&nl.mv_y_columns, "NonlinearRegression mv_y_columns", err);
            for rule in &mut nl.parameter_rules {
                if let ParameterScope::PerDatasetConstant(pairs) = &mut rule.scope {
                    for (col, _) in pairs.iter_mut() {
                        plan.map_one_or_oob(col, "PerDatasetConstant column", err);
                    }
                }
            }
        }
        AnalysisKind::DescriptiveStats(_)
        | AnalysisKind::OutlierDetection(_)
        | AnalysisKind::Contingency(_)
        | AnalysisKind::RocCurve(_)
        | AnalysisKind::LinearRegression(_)
        | AnalysisKind::FitSplineLowess(_)
        | AnalysisKind::PowerAnalysis(_)
        | AnalysisKind::SimpleLogisticRegression(_)
        | AnalysisKind::LeveneTest(_) => {}
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
                    if let Some(&new_slot) =
                        target_slots.iter().find(|&&slot| spec.series.get(slot).is_some_and(|s| s.data_set_index == mapped.series_index))
                    {
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
                        mv.color = ColorChannelMapping::Constant(
                            curvebook_palette::PaletteId::OkabeIto.swatches().first().copied().unwrap_or(curvebook_palette::Rgba8::BLACK),
                        );
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

fn finalize_broken_dependents(doc: &mut ProjectDocument, impact: &ConversionImpact, recalc: &mut super::super::project::RecalcSummary) {
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
