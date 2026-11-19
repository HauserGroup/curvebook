//! Execution engine for all eight [`TransformAnalysisSpec`] operations (`docs/implementation/data-tables-and-transforms.md` §5, `docs/planning/calculators-and-quick-stats.md` §4).

pub mod math;
pub mod normalize_baseline;
pub mod reshape_select;

pub use math::{MathTransformOp, execute_math_op, execute_math_transform};
pub use normalize_baseline::{BaselineKind, execute_normalize, execute_remove_baseline, execute_remove_baseline_kind};
pub use reshape_select::{PruneMethod, SelectRule, execute_prune, execute_select, execute_select_with_derived, execute_transpose};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellExclusionNote, DataSheet, HookSource, InfoSheet, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus,
    ResultTab, ResultTabKind, TableArchetype, TransformAnalysisSpec, parse_column_title_numeric_hook,
};

use crate::error::TransformError;
use crate::randomize::{RowSampleConfig, random_sample_rows};
use crate::simulate::simulate_xy_sheet;

/// External context (linked `InfoSheet`, upstream `ResultSheet`s, `DataSheet`s, and column slice)
/// supplied when executing a derived data transformation.
#[derive(Debug, Clone, Default)]
pub struct TransformContext<'a> {
    /// Optional paired [`InfoSheet`] resolving [`HookSource::InfoConstant`] hooks and `Info("Key")` formula references.
    pub info_sheet: Option<&'a InfoSheet>,
    /// Upstream [`ResultSheet`] slice resolving [`HookSource::ResultScalar`] hooks.
    pub result_sheets: &'a [ResultSheet],
    /// Project [`DataSheet`] slice resolving cross-sheet [`HookSource::ColumnTitleValue`] hooks.
    pub data_sheets: &'a [DataSheet],
    /// Zero-based dataset column indices to include (`empty` selects all columns on `source`).
    pub selected_data_sets: Vec<usize>,
    /// Optional explicit identifier for the derived output [`DataSheet`]; `None` appends an operation suffix to `source.id`.
    pub output_id: Option<String>,
    /// Optional explicit display title for the derived output [`DataSheet`]; `None` appends an operation suffix to `source.title`.
    pub output_title: Option<String>,
}

impl<'a> TransformContext<'a> {
    /// Construct a `TransformContext` pre-bound to `info_sheet` with default empty upstream slices.
    #[must_use]
    pub fn with_info(info_sheet: &'a InfoSheet) -> Self {
        Self { info_sheet: Some(info_sheet), ..Self::default() }
    }

    /// Resolve a [`HookSource`] into a finite scalar `f64`.
    pub fn resolve_hook(&self, hook: &HookSource, source_sheet: &DataSheet) -> Result<f64, TransformError> {
        match hook {
            HookSource::InfoConstant { info_sheet_id, constant_name } => {
                let info = self
                    .info_sheet
                    .ok_or_else(|| TransformError::MissingHook { description: format!("no InfoSheet provided to resolve `{constant_name}`") })?;
                if let Some(expected_id) = info_sheet_id
                    && &info.id != expected_id
                {
                    return Err(TransformError::MissingHook {
                        description: format!("InfoSheet `{}` does not match hooked `{}`", info.id, expected_id),
                    });
                }
                let entry = info.get_constant(constant_name).ok_or_else(|| TransformError::MissingHook {
                    description: format!("constant `{constant_name}` not found in InfoSheet `{}`", info.id),
                })?;
                entry
                    .value
                    .as_finite_f64()
                    .ok_or_else(|| TransformError::MissingHook { description: format!("constant `{constant_name}` is not a finite numeric value") })
            }
            HookSource::ResultScalar { result_sheet_id, parameter_key, data_set_index } => {
                let res = self
                    .result_sheets
                    .iter()
                    .find(|r| &r.id == result_sheet_id)
                    .ok_or_else(|| TransformError::MissingHook { description: format!("ResultSheet `{result_sheet_id}` not found") })?;
                if let ResultStatus::Failed { message, .. } = &res.status {
                    return Err(TransformError::MissingHook { description: format!("upstream ResultSheet `{result_sheet_id}` failed: {message}") });
                }
                res.lookup_scalar(parameter_key, *data_set_index).ok_or_else(|| TransformError::MissingHook {
                    description: format!("scalar `{parameter_key}` not found on ResultSheet `{result_sheet_id}`"),
                })
            }
            HookSource::ColumnTitleValue { data_sheet_id, data_set_index } => {
                let target = if &source_sheet.id == data_sheet_id {
                    source_sheet
                } else {
                    self.data_sheets
                        .iter()
                        .find(|s| &s.id == data_sheet_id)
                        .ok_or_else(|| TransformError::MissingHook { description: format!("DataSheet `{data_sheet_id}` not found") })?
                };
                let title = column_title_at(&target.archetype, *data_set_index).ok_or_else(|| TransformError::MissingHook {
                    description: format!("column {data_set_index} not found on DataSheet `{data_sheet_id}`"),
                })?;
                parse_column_title_numeric_hook(title).map_err(|err| TransformError::MissingHook { description: err.to_string() })
            }
        }
    }

    pub(crate) fn derived_ids(&self, source: &DataSheet, suffix: &str, title_suffix: &str) -> (String, String) {
        let id = self.output_id.clone().unwrap_or_else(|| format!("{}-{suffix}", source.id.as_str()));
        let title = self.output_title.clone().unwrap_or_else(|| format!("{} ({title_suffix})", source.title));
        (id, title)
    }

    pub(crate) fn active_columns(&self, total_cols: usize) -> Result<Vec<usize>, TransformError> {
        if self.selected_data_sets.is_empty() {
            return Ok((0..total_cols).collect());
        }
        let mut out = Vec::with_capacity(self.selected_data_sets.len().min(total_cols));
        for &idx in &self.selected_data_sets {
            if idx >= total_cols {
                return Err(TransformError::InvalidSpec { message: format!("selected dataset index {idx} exceeds column count {total_cols}") });
            }
            if !out.contains(&idx) {
                out.push(idx);
            }
        }
        Ok(out)
    }
}

/// Complete output produced by executing a data transformation.
#[derive(Debug, Clone, PartialEq)]
pub struct TransformOutput {
    /// Derived [`DataSheet`] suitable for [`ResultSheet::analyzable_data`], downstream analyses, curve fits, and graphs.
    pub data_sheet: DataSheet,
    /// Audit [`ResultTab`] summarizing row/column counts, anchors, or parameters applied by the transform.
    pub summary_tab: ResultTab,
    /// Hookable scalar outputs (such as computed normalization anchors or baseline means).
    pub scalars: Vec<ResultScalarEntry>,
    /// Non-fatal domain warnings (such as out-of-domain `ln(<= 0)` cells mapped to empty).
    pub warnings: Vec<String>,
}

impl TransformOutput {
    /// Wrap this transform output into an analyzable [`ResultSheet`].
    pub fn into_result_sheet(self, analysis: AnalysisSpec) -> Result<ResultSheet, TransformError> {
        let mut sheet = ResultSheet::new(analysis)?;
        sheet.status = ResultStatus::UpToDate;
        sheet.analyzable_data = Some(self.data_sheet);
        sheet.tabs.push(self.summary_tab);
        sheet.scalars = self.scalars;
        Ok(sheet)
    }
}

/// Execute any [`TransformAnalysisSpec`] against `source` without mutating `source`.
pub fn execute_transform(source: &DataSheet, spec: &TransformAnalysisSpec, ctx: &TransformContext<'_>) -> Result<TransformOutput, TransformError> {
    match spec {
        TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation } => {
            let x_op = x_transform.map(MathTransformOp::Preset);
            let y_op = y_transform.map(MathTransformOp::Preset);
            execute_math_transform(source, x_op.as_ref(), y_op.as_ref(), *error_propagation, ctx)
        }
        TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, output_percent, per_data_set } => {
            execute_normalize(source, zero_anchor, hundred_anchor, *output_percent, *per_data_set, ctx)
        }
        TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation } => execute_remove_baseline(source, baseline_columns, *operation, ctx),
        TransformAnalysisSpec::Transpose { swap_row_and_column_titles } => execute_transpose(source, *swap_row_and_column_titles, ctx),
        TransformAnalysisSpec::PruneRows { every_k_rows, average_window } => {
            let method = if *average_window { PruneMethod::AverageBins(*every_k_rows) } else { PruneMethod::EveryNth(*every_k_rows) };
            execute_prune(source, method, ctx)
        }
        TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype, derived_columns } => {
            let mut rules = Vec::new();
            if let Some(formula) = filter_formula
                && !formula.trim().is_empty()
            {
                rules.push(SelectRule::Predicate(formula.clone()));
            }
            execute_select_with_derived(source, &rules, derived_columns, *target_archetype, ctx)
        }
        TransformAnalysisSpec::Simulate { seed, row_count, replicates, x_min, x_max, equation_id, parameters, noise } => {
            let (id, title) = ctx.derived_ids(source, "simulate", "Simulated");
            let data_sheet = simulate_xy_sheet(&id, &title, *seed, *row_count, *replicates, *x_min, *x_max, equation_id, parameters, *noise)?;
            let summary_tab = build_summary_tab(
                "Simulation summary",
                vec![
                    ("Equation", ResultCell::Text(equation_id.clone())),
                    ("Rows", ResultCell::Number { value: *row_count as f64, prefix: None, units: None }),
                    ("Replicates", ResultCell::Number { value: *replicates as f64, prefix: None, units: None }),
                    ("Seed", ResultCell::Text(seed.to_string())),
                ],
                Vec::new(),
            );
            Ok(TransformOutput { data_sheet, summary_tab, scalars: Vec::new(), warnings: Vec::new() })
        }
        TransformAnalysisSpec::RandomSample { seed, sample_size, stratify_column, proportional_allocation } => {
            let (id, title) = ctx.derived_ids(source, "sample", "Random sample");
            let population_size = source.archetype.row_count();
            let config = RowSampleConfig {
                seed: *seed,
                sample_size: *sample_size,
                stratify_column: *stratify_column,
                proportional_allocation: *proportional_allocation,
                output_id: Some(id),
                output_title: Some(title),
            };
            let data_sheet = random_sample_rows(source, &config)?;
            let stratify_label = stratify_column
                .map_or_else(|| String::from("None"), |c| column_title_at(&source.archetype, c).map_or_else(|| format!("Column {c}"), str::to_owned));
            let summary_tab = build_summary_tab(
                "Random sample summary",
                vec![
                    ("Sample size", ResultCell::Number { value: *sample_size as f64, prefix: None, units: None }),
                    ("Population size", ResultCell::Number { value: population_size as f64, prefix: None, units: None }),
                    ("Seed", ResultCell::Text(seed.to_string())),
                    ("Stratify column", ResultCell::Text(stratify_label)),
                    ("Proportional allocation", ResultCell::Boolean(*proportional_allocation)),
                ],
                Vec::new(),
            );
            let scalars = vec![
                ResultScalarEntry { key: String::from("sample_size"), data_set_index: None, value: *sample_size as f64, units: None },
                ResultScalarEntry { key: String::from("population_size"), data_set_index: None, value: population_size as f64, units: None },
                ResultScalarEntry { key: String::from("seed"), data_set_index: None, value: *seed as f64, units: None },
            ];
            Ok(TransformOutput { data_sheet, summary_tab, scalars, warnings: Vec::new() })
        }
    }
}

/// Execute an [`AnalysisSpec`] whose kind is [`AnalysisKind::Transform`] and populate a [`ResultSheet`].
pub fn execute_analysis_transform(source: &DataSheet, analysis: &AnalysisSpec, ctx: &TransformContext<'_>) -> Result<ResultSheet, TransformError> {
    let AnalysisKind::Transform(spec) = &analysis.kind else {
        return Err(TransformError::InvalidSpec { message: format!("analysis `{}` is not a Transform specification", analysis.effective_title()) });
    };
    let mut local_ctx = ctx.clone();
    if local_ctx.selected_data_sets.is_empty() && !analysis.selected_data_sets.is_empty() {
        local_ctx.selected_data_sets = analysis.selected_data_sets.clone();
    }
    let output = execute_transform(source, spec, &local_ctx)?;
    output.into_result_sheet(analysis.clone())
}

pub(crate) fn build_summary_tab(title: &str, entries: Vec<(&str, ResultCell)>, footnotes: Vec<String>) -> ResultTab {
    let mut row_labels = Vec::with_capacity(entries.len());
    let mut rows = Vec::with_capacity(entries.len());
    for (label, cell) in entries {
        row_labels.push(label.to_owned());
        rows.push(vec![cell]);
    }
    ResultTab {
        id: String::from("summary"),
        title: title.to_owned(),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Value")],
        row_labels,
        rows,
        footnotes,
    }
}

pub(crate) fn swap_error_bound_cells(row_cells: &mut [Cell]) -> bool {
    if row_cells.len() >= 3 {
        row_cells.swap(1, 2);
        true
    } else {
        false
    }
}

pub(crate) fn remap_swapped_error_notes(notes: &mut [CellExclusionNote], swapped_rows: &[usize], col: usize) {
    if notes.is_empty() || swapped_rows.is_empty() {
        return;
    }
    let mut changed = false;
    for note in notes.iter_mut() {
        if note.coord.col == col && (note.coord.subcol == 1 || note.coord.subcol == 2) && swapped_rows.contains(&note.coord.row) {
            note.coord.subcol = if note.coord.subcol == 1 { 2 } else { 1 };
            changed = true;
        }
    }
    if changed {
        notes.sort_by_key(|n| n.coord);
    }
}

fn column_title_at(arch: &TableArchetype, idx: usize) -> Option<&str> {
    match arch {
        TableArchetype::Xy(t) => t.data_sets.get(idx).map(|d| d.title.as_str()),
        TableArchetype::Column(t) => t.data_sets.get(idx).map(|d| d.title.as_str()),
        TableArchetype::Grouped(t) => t.data_sets.get(idx).map(|d| d.title.as_str()),
        TableArchetype::Contingency(t) => t.data_sets.get(idx).map(|d| d.title.as_str()),
        TableArchetype::Survival(t) => t.cohorts.get(idx).map(|d| d.title.as_str()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(idx).map(|d| d.title.as_str()),
        TableArchetype::MultipleVariables(t) => t.variables.get(idx).map(|v| v.title.as_str()),
        TableArchetype::Nested(t) => t.groups.get(idx).map(|g| g.title.as_str()),
    }
}
