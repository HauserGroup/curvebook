//! Execution engine for all eight [`TransformAnalysisSpec`] operations (`docs/implementation/data-tables-and-transforms.md` §5, `docs/planning/calculators-and-quick-stats.md` §4).

pub mod fraction_of_total;
pub mod math;
pub mod normalize_baseline;
pub mod reshape_select;
pub mod row_stats;

pub use fraction_of_total::{clopper_pearson_ci, execute_fraction_of_total, wilson_score_ci};
pub use math::{MathTransformOp, MathTransformOptions, execute_math_op, execute_math_transform, execute_math_transform_with_options};
pub use normalize_baseline::{
    BaselineKind, execute_normalize, execute_remove_baseline, execute_remove_baseline_kind, execute_remove_baseline_with_options,
};
pub use reshape_select::{
    PruneMethod, SelectRearrangeOptions, SelectRule, execute_prune, execute_select, execute_select_and_rearrange, execute_select_with_derived,
    execute_transpose,
};
pub use row_stats::{execute_row_statistics, quantile_type6_sorted};

use std::collections::HashSet;

use curvebook_doc::error::MAX_SHEET_ID_BYTES;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellExclusionNote, ChecklistItem, ChecklistStatus, DataSheet, DocError, HookSource, InfoSheet, MAX_TITLE_BYTES,
    PartsOfWholeTable, PharmacologyTransformPreset, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
    SimulationMode, SummaryModeSupport, TableArchetype, TableKind, TransformAnalysisSpec, VariableRole, VariableSource,
    parse_column_title_numeric_hook, summary_mode_row_for_spec,
};

use crate::error::TransformError;
use crate::randomize::{RowSampleConfig, random_sample_rows};
use crate::simulate::{simulate_column_sheet, simulate_contingency_2x2, simulate_xy_sheet};

/// External context (linked `InfoSheet`, upstream `ResultSheet`s, `DataSheet`s, and column slice)
/// supplied when executing a derived data transformation.
#[derive(Debug, Clone, Default)]
pub struct TransformContext<'a> {
    /// Optional primary paired [`InfoSheet`] resolving untargeted [`HookSource::InfoConstant`] hooks and `Info("Key")` formula references.
    pub info_sheet: Option<&'a InfoSheet>,
    /// Optional project [`InfoSheet`] slice resolving targeted [`HookSource::InfoConstant`] hooks across multiple info sheets.
    pub info_sheets: &'a [InfoSheet],
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

    pub(crate) fn primary_info_sheet(&self) -> Option<&'a InfoSheet> {
        self.info_sheet.or_else(|| self.info_sheets.first())
    }

    fn resolve_info_sheet_for_hook(
        &self,
        info_sheet_id: Option<&curvebook_doc::SheetId>,
        constant_name: &str,
    ) -> Result<&'a InfoSheet, TransformError> {
        if let Some(expected_id) = info_sheet_id {
            if let Some(info) =
                self.info_sheet.filter(|info| &info.id == expected_id).or_else(|| self.info_sheets.iter().find(|info| &info.id == expected_id))
            {
                return Ok(info);
            }
            if self.info_sheet.is_none() && self.info_sheets.is_empty() {
                return Err(TransformError::MissingHook { description: format!("no InfoSheet provided to resolve `{constant_name}`") });
            }
            if let Some(primary) = self.info_sheet
                && self.info_sheets.is_empty()
            {
                return Err(TransformError::MissingHook { description: format!("InfoSheet `{}` does not match hooked `{expected_id}`", primary.id) });
            }
            return Err(TransformError::MissingHook { description: format!("InfoSheet `{expected_id}` not found to resolve `{constant_name}`") });
        }
        self.primary_info_sheet()
            .ok_or_else(|| TransformError::MissingHook { description: format!("no InfoSheet provided to resolve `{constant_name}`") })
    }

    /// Resolve a [`HookSource`] into a finite scalar `f64`.
    pub fn resolve_hook(&self, hook: &HookSource, source_sheet: &DataSheet) -> Result<f64, TransformError> {
        match hook {
            HookSource::InfoConstant { info_sheet_id, constant_name } => {
                let info = self.resolve_info_sheet_for_hook(info_sheet_id.as_ref(), constant_name)?;
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
                match &res.status {
                    ResultStatus::UpToDate => {}
                    ResultStatus::Dirty => {
                        return Err(TransformError::MissingHook {
                            description: format!("upstream ResultSheet `{result_sheet_id}` is dirty / not up to date"),
                        });
                    }
                    ResultStatus::Failed { message, .. } => {
                        return Err(TransformError::MissingHook {
                            description: format!("upstream ResultSheet `{result_sheet_id}` failed: {message}"),
                        });
                    }
                    ResultStatus::Imported | ResultStatus::ImportedStale => {
                        return Err(TransformError::MissingHook {
                            description: format!(
                                "upstream ResultSheet `{result_sheet_id}` is an imported snapshot and cannot supply computed hook scalars"
                            ),
                        });
                    }
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
        let id = self.output_id.clone().unwrap_or_else(|| bounded_suffixed_str(source.id.as_str(), &format!("-{suffix}"), MAX_SHEET_ID_BYTES));
        let title = self.output_title.clone().unwrap_or_else(|| bounded_suffixed_str(&source.title, &format!(" ({title_suffix})"), MAX_TITLE_BYTES));
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

/// Enforce the canonical `curvebook-doc` summary-mode support matrix ([`curvebook_doc::SUMMARY_MODE_MATRIX`])
/// for transform, spline/LOWESS, and AUC operations invoked against `Column`, `Xy`, or `Grouped` tables.
///
/// Leaves incompatible table archetypes (`!kind.is_compatible_with(table_kind)`) to return
/// [`TransformError::IncompatibleTable`] downstream, and returns [`TransformError::UnsupportedSubcolumnMode`]
/// for every refused pre-summarized cell.
pub fn enforce_summary_mode_matrix(sheet: &DataSheet, kind: &AnalysisKind) -> Result<(), TransformError> {
    let table_kind = sheet.archetype.kind();
    if !kind.is_compatible_with(table_kind) {
        return Ok(());
    }
    let Some(mode) = sheet.archetype.primary_subcolumn_mode() else {
        return Ok(());
    };
    if !mode.is_pre_summarized() {
        return Ok(());
    }
    let Some(row) = summary_mode_row_for_spec(kind, table_kind) else {
        return Ok(());
    };
    let Some(verdict) = row.support_for(mode) else {
        return Ok(());
    };
    match verdict {
        SummaryModeSupport::Supported(_) => Ok(()),
        SummaryModeSupport::Refused(reason) => {
            Err(TransformError::UnsupportedSubcolumnMode { operation: row.label, table_kind, mode, reason: reason.message() })
        }
    }
}

/// Execute any [`TransformAnalysisSpec`] against `source` without mutating `source`.
pub fn execute_transform(source: &DataSheet, spec: &TransformAnalysisSpec, ctx: &TransformContext<'_>) -> Result<TransformOutput, TransformError> {
    enforce_summary_mode_matrix(source, &AnalysisKind::Transform(spec.clone()))?;
    match spec {
        TransformAnalysisSpec::Math {
            x_transform,
            y_transform,
            error_propagation,
            pharmacology_preset,
            interchange_xy,
            zero_x_substitute,
            asymmetric_ci_policy,
        } => {
            let x_op = x_transform.map(MathTransformOp::Preset);
            let y_op = y_transform.map(MathTransformOp::Preset);
            let opts = MathTransformOptions {
                pharmacology_preset: *pharmacology_preset,
                interchange_xy: *interchange_xy,
                zero_x_substitute: *zero_x_substitute,
                asymmetric_ci_policy: *asymmetric_ci_policy,
            };
            execute_math_transform_with_options(source, x_op.as_ref(), y_op.as_ref(), *error_propagation, &opts, ctx)
        }
        TransformAnalysisSpec::Normalize { zero_anchor, hundred_anchor, output_percent, per_data_set } => {
            execute_normalize(source, zero_anchor, hundred_anchor, *output_percent, *per_data_set, ctx)
        }
        TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation, baseline_kind, retain_unselected_columns } => {
            if let Some(kind) = baseline_kind {
                execute_remove_baseline_with_options(source, kind, *operation, *retain_unselected_columns, ctx)
            } else {
                if baseline_columns.is_empty() {
                    return Err(TransformError::InvalidSpec { message: String::from("baseline_columns must not be empty") });
                }
                let kind = BaselineKind::ColumnsRowWise(baseline_columns.clone());
                execute_remove_baseline_with_options(source, &kind, *operation, *retain_unselected_columns, ctx)
            }
        }
        TransformAnalysisSpec::Transpose { swap_row_and_column_titles } => execute_transpose(source, *swap_row_and_column_titles, ctx),
        TransformAnalysisSpec::PruneRows { every_k_rows, average_window } => {
            let method = if *average_window { PruneMethod::AverageBins(*every_k_rows) } else { PruneMethod::EveryNth(*every_k_rows) };
            execute_prune(source, method, ctx)
        }
        TransformAnalysisSpec::SelectAndTransform {
            filter_formula,
            target_archetype,
            derived_columns,
            complete_rows_only,
            subject_column,
            row_title_column,
            contingency_count_column,
            include_variable_name_prefixes,
        } => {
            let mut rules = Vec::new();
            if let Some(formula) = filter_formula
                && !formula.trim().is_empty()
            {
                rules.push(SelectRule::Predicate(formula.clone()));
            }
            let opts = SelectRearrangeOptions {
                complete_rows_only: *complete_rows_only,
                subject_column: *subject_column,
                row_title_column: *row_title_column,
                contingency_count_column: *contingency_count_column,
                include_variable_name_prefixes: *include_variable_name_prefixes,
            };
            execute_select_and_rearrange(source, &rules, derived_columns, *target_archetype, &opts, ctx)
        }
        TransformAnalysisSpec::Simulate { seed, row_count, replicates, x_min, x_max, equation_id, parameters, noise, simulation_mode } => {
            let (id, title) = ctx.derived_ids(source, "simulate", "Simulated");
            match simulation_mode.as_ref().unwrap_or(&SimulationMode::XyCurve) {
                SimulationMode::XyCurve => {
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
                SimulationMode::Contingency2x2(cspec) => {
                    let data_sheet = simulate_contingency_2x2(&id, &title, *seed, cspec)?;
                    let summary_tab = build_summary_tab(
                        "Contingency simulation summary",
                        vec![("Design", ResultCell::Text(format!("{:?}", cspec.design))), ("Seed", ResultCell::Text(seed.to_string()))],
                        Vec::new(),
                    );
                    Ok(TransformOutput { data_sheet, summary_tab, scalars: Vec::new(), warnings: Vec::new() })
                }
                SimulationMode::Column(col_spec) => {
                    let n_cols = col_spec.column_means.len().max(col_spec.hooked_means.len());
                    let mut resolved = Vec::with_capacity(n_cols);
                    for i in 0..n_cols {
                        if let Some(Some(hook)) = col_spec.hooked_means.get(i) {
                            resolved.push(ctx.resolve_hook(hook, source)?);
                        } else if let Some(&m) = col_spec.column_means.get(i) {
                            resolved.push(m);
                        } else {
                            return Err(TransformError::InvalidSpec {
                                message: format!("column mean[{i}] missing and no hooked_means entry provided"),
                            });
                        }
                    }
                    let data_sheet = simulate_column_sheet(&id, &title, *seed, *row_count, col_spec, Some(&resolved))?;
                    let summary_tab = build_summary_tab(
                        "Column simulation summary",
                        vec![
                            ("Columns", ResultCell::Number { value: resolved.len() as f64, prefix: None, units: None }),
                            ("Rows", ResultCell::Number { value: *row_count as f64, prefix: None, units: None }),
                            ("Within SD", ResultCell::Number { value: col_spec.within_sd, prefix: None, units: None }),
                            ("Seed", ResultCell::Text(seed.to_string())),
                        ],
                        Vec::new(),
                    );
                    Ok(TransformOutput { data_sheet, summary_tab, scalars: Vec::new(), warnings: Vec::new() })
                }
            }
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
            let mut scalars = vec![
                ResultScalarEntry { key: String::from("sample_size"), data_set_index: None, value: *sample_size as f64, units: None },
                ResultScalarEntry { key: String::from("population_size"), data_set_index: None, value: population_size as f64, units: None },
            ];
            if *seed <= (1_u64 << 53) {
                #[allow(clippy::cast_precision_loss)]
                scalars.push(ResultScalarEntry { key: String::from("seed"), data_set_index: None, value: *seed as f64, units: None });
            }
            Ok(TransformOutput { data_sheet, summary_tab, scalars, warnings: Vec::new() })
        }
    }
}

/// Execute an [`AnalysisSpec`] whose kind is [`AnalysisKind::Transform`],
/// [`AnalysisKind::FitSplineLowess`], or [`AnalysisKind::AreaUnderCurve`] and populate a [`ResultSheet`].
pub fn execute_analysis_transform(source: &DataSheet, analysis: &AnalysisSpec, ctx: &TransformContext<'_>) -> Result<ResultSheet, TransformError> {
    if matches!(analysis.kind, AnalysisKind::Transform(_) | AnalysisKind::FitSplineLowess(_) | AnalysisKind::AreaUnderCurve(_)) {
        enforce_summary_mode_matrix(source, &analysis.kind)?;
    }
    match &analysis.kind {
        AnalysisKind::Transform(spec) => {
            let mut local_ctx = ctx.clone();
            if local_ctx.selected_data_sets.is_empty() && !analysis.selected_data_sets.is_empty() {
                local_ctx.selected_data_sets = analysis.selected_data_sets.clone();
            }
            let output = execute_transform(source, spec, &local_ctx)?;
            let mut sheet = output.into_result_sheet(analysis.clone())?;
            if let TransformAnalysisSpec::Math { pharmacology_preset: Some(PharmacologyTransformPreset::LineweaverBurk), .. } = spec {
                append_lineweaver_burk_diagnostics(&mut sheet, &local_ctx.selected_data_sets);
            }
            Ok(sheet)
        }
        AnalysisKind::FitSplineLowess(spec) => crate::spline::execute_spline_lowess(source, analysis, spec),
        AnalysisKind::AreaUnderCurve(spec) => crate::curve::execute_area_under_curve(source, analysis, spec),
        AnalysisKind::RowStatistics(spec) => execute_row_statistics(source, analysis, spec, ctx),
        AnalysisKind::FractionOfTotal(spec) => execute_fraction_of_total(source, analysis, spec, ctx),
        AnalysisKind::CurveCalculus(spec) => crate::curve::execute_curve_calculus(source, analysis, spec, ctx),
        _ => Err(TransformError::InvalidSpec {
            message: format!("analysis `{}` is not a supported transform analysis specification", analysis.effective_title()),
        }),
    }
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

/// Append unweighted double-reciprocal OLS diagnostics and a prominent error-distortion caution to a
/// Lineweaver–Burk transform `ResultSheet` (Lineweaver & Burk 1934; Cornish-Bowden 2012).
fn append_lineweaver_burk_diagnostics(sheet: &mut ResultSheet, selected_data_sets: &[usize]) {
    let Some(TableArchetype::Xy(xy)) = sheet.analyzable_data.as_ref().map(|d| &d.archetype) else {
        return;
    };
    let active_cols: Vec<usize> = if selected_data_sets.is_empty() {
        (0..xy.data_sets.len()).collect()
    } else {
        selected_data_sets.iter().copied().filter(|&c| c < xy.data_sets.len()).collect()
    };

    let mut row_labels = Vec::with_capacity(active_cols.len());
    let mut rows = Vec::with_capacity(active_cols.len());
    for &c in &active_cols {
        let ds = &xy.data_sets[c];
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for r in 0..xy.row_count() {
            let Some(xv) = xy.x_summary_at(r).ok().flatten().filter(|v| v.is_finite()) else {
                continue;
            };
            let Some(subs) = ds.cells.get(r) else {
                continue;
            };
            if xy.y_subcolumn_mode.is_pre_summarized() || xy.y_subcolumn_mode == curvebook_doc::SubcolumnMode::Single {
                if let Some(yv) = subs.first().and_then(Cell::active_number).filter(|v| v.is_finite()) {
                    xs.push(xv);
                    ys.push(yv);
                }
            } else {
                for cell in subs {
                    if let Some(yv) = cell.active_number().filter(|v| v.is_finite()) {
                        xs.push(xv);
                        ys.push(yv);
                    }
                }
            }
        }
        let n = xs.len();
        let (slope, intercept, r_sq, app_vmax, app_km) = if n >= 2 {
            let nf = n as f64;
            let mx = curvebook_math::sum::sum(&xs) / nf;
            let my = curvebook_math::sum::sum(&ys) / nf;
            let mut sxx = curvebook_math::sum::NeumaierSum::new();
            let mut sxy = curvebook_math::sum::NeumaierSum::new();
            let mut syy = curvebook_math::sum::NeumaierSum::new();
            for (&x, &y) in xs.iter().zip(ys.iter()) {
                let dx = x - mx;
                let dy = y - my;
                sxx.add(dx * dx);
                sxy.add(dx * dy);
                syy.add(dy * dy);
            }
            let sxx_v = sxx.total();
            let sxy_v = sxy.total();
            let syy_v = syy.total();
            if sxx_v > 0.0 && sxx_v.is_finite() {
                let b = sxy_v / sxx_v;
                let a = my - b * mx;
                let r2 = if syy_v > 0.0 { ((sxy_v * sxy_v) / (sxx_v * syy_v)).clamp(0.0, 1.0) } else { 1.0 };
                let vmax = if a.is_finite() && a.abs() > 1e-15 { Some(1.0 / a) } else { None };
                let km = if a.is_finite() && a.abs() > 1e-15 && b.is_finite() { Some(b / a) } else { None };
                (Some(b), Some(a), Some(r2), vmax, km)
            } else {
                (None, None, None, None, None)
            }
        } else {
            (None, None, None, None, None)
        };
        let opt_cell = |v: Option<f64>| match v.filter(|x| x.is_finite()) {
            Some(val) => ResultCell::Number { value: val, prefix: None, units: None },
            None => ResultCell::Empty,
        };
        row_labels.push(ds.title.clone());
        rows.push(vec![
            ResultCell::Number { value: n as f64, prefix: None, units: None },
            opt_cell(slope),
            opt_cell(intercept),
            opt_cell(r_sq),
            opt_cell(app_vmax),
            opt_cell(app_km),
        ]);
        if let Some(b) = slope.filter(|v| v.is_finite()) {
            sheet.scalars.push(ResultScalarEntry { key: String::from("Lineweaver-Burk slope"), data_set_index: Some(c), value: b, units: None });
        }
        if let Some(a) = intercept.filter(|v| v.is_finite()) {
            sheet.scalars.push(ResultScalarEntry {
                key: String::from("Lineweaver-Burk Y-intercept"),
                data_set_index: Some(c),
                value: a,
                units: None,
            });
        }
        if let Some(r2) = r_sq.filter(|v| v.is_finite()) {
            sheet.scalars.push(ResultScalarEntry { key: String::from("Lineweaver-Burk R squared"), data_set_index: Some(c), value: r2, units: None });
        }
        if let Some(vmax) = app_vmax.filter(|v| v.is_finite()) {
            sheet.scalars.push(ResultScalarEntry { key: String::from("Apparent Vmax"), data_set_index: Some(c), value: vmax, units: None });
        }
        if let Some(km) = app_km.filter(|v| v.is_finite()) {
            sheet.scalars.push(ResultScalarEntry { key: String::from("Apparent Km"), data_set_index: Some(c), value: km, units: None });
        }
    }

    sheet.tabs.push(ResultTab {
        id: String::from("lineweaver-burk-summary"),
        title: String::from("Lineweaver-Burk double-reciprocal linear diagnostics (1/Y vs 1/X)"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![
            String::from("Finite pairs (n)"),
            String::from("Slope (Km / Vmax)"),
            String::from("Y-intercept (1 / Vmax)"),
            String::from("R squared"),
            String::from("Apparent Vmax"),
            String::from("Apparent Km"),
        ],
        row_labels,
        rows,
        footnotes: vec![String::from(
            "Warning (Lineweaver & Burk 1934; Cornish-Bowden 2012): Taking reciprocals (1/X, 1/Y) distorts experimental error structure and disproportionately weights small-concentration observations. Use nonlinear Michaelis-Menten regression on untransformed (X, Y) data for parameter estimation; use Lineweaver-Burk plots for visual inspection only.",
        )],
    });
    sheet.checklist.push(ChecklistItem {
        assumption: String::from("Lineweaver-Burk reciprocal error distortion"),
        status: ChecklistStatus::Warn,
        diagnostic_metric: Some(String::from("Unweighted double-reciprocal (1/X, 1/Y) transform")),
        explanation: String::from(
            "Unweighted linear regression on (1/X, 1/Y) severely amplifies measurement noise at small substrate concentrations and yields biased Vmax/Km estimates. Fit nonlinear Michaelis-Menten regression to the raw (X, Y) data for parameter estimation, and reserve the Lineweaver-Burk plot for visual diagnostics.",
        ),
        remediation: None,
    });
}

pub(crate) fn swap_error_bound_cells(row_cells: &mut [Cell]) -> bool {
    if row_cells.len() >= 3 {
        row_cells.swap(1, 2);
        true
    } else {
        false
    }
}

pub(crate) fn bounded_suffixed_str(base: &str, suffix: &str, max_bytes: usize) -> String {
    if base.len().saturating_add(suffix.len()) <= max_bytes {
        return format!("{base}{suffix}");
    }
    if suffix.len() >= max_bytes {
        let mut out = String::with_capacity(max_bytes);
        for ch in suffix.chars() {
            if out.len() + ch.len_utf8() > max_bytes {
                break;
            }
            out.push(ch);
        }
        return out;
    }
    let prefix_budget = max_bytes - suffix.len();
    let mut out = String::with_capacity(max_bytes);
    for ch in base.chars() {
        if out.len() + ch.len_utf8() > prefix_budget {
            break;
        }
        out.push(ch);
    }
    out.push_str(suffix);
    out
}

pub(crate) fn snapshot_calculated_variables(arch: &mut TableArchetype) {
    if let TableArchetype::MultipleVariables(mv) = arch {
        for var in &mut mv.variables {
            if matches!(var.source, VariableSource::Calculated { .. }) || var.role == VariableRole::Calculated {
                var.source = VariableSource::Entered;
                var.role = VariableRole::Continuous;
                var.role_inferred = true;
                var.refresh_inferred_role();
            }
        }
    }
}

pub(crate) fn validate_parts_of_whole_non_negative(table: &PartsOfWholeTable) -> Result<(), TransformError> {
    for (col, ds) in table.data_sets.iter().enumerate() {
        for (row, subs) in ds.cells.iter().enumerate() {
            if let Some(val) = subs.first().and_then(Cell::active_number)
                && val < 0.0
            {
                return Err(TransformError::Doc(DocError::InvalidCellValue {
                    archetype: TableKind::PartsOfWhole,
                    row,
                    col,
                    reason: "parts-of-whole slice values must be >= 0",
                }));
            }
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub(crate) fn remap_swapped_error_notes(notes: &mut [CellExclusionNote], swapped_rows: &[usize], col: usize) {
    if notes.is_empty() || swapped_rows.is_empty() {
        return;
    }
    let row_set: HashSet<usize> = swapped_rows.iter().copied().collect();
    let mut changed = false;
    for note in notes.iter_mut() {
        if note.coord.col == col && (note.coord.subcol == 1 || note.coord.subcol == 2) && row_set.contains(&note.coord.row) {
            note.coord.subcol = if note.coord.subcol == 1 { 2 } else { 1 };
            changed = true;
        }
    }
    if changed {
        notes.sort_by_key(|n| n.coord);
    }
}

pub(crate) fn remap_swapped_error_notes_batch(notes: &mut [CellExclusionNote], swapped: &[(usize, usize)]) {
    if notes.is_empty() || swapped.is_empty() {
        return;
    }
    let swapped_set: HashSet<(usize, usize)> = swapped.iter().copied().collect();
    let mut changed = false;
    for note in notes.iter_mut() {
        if (note.coord.subcol == 1 || note.coord.subcol == 2) && swapped_set.contains(&(note.coord.row, note.coord.col)) {
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

#[cfg(test)]
mod tests {
    use curvebook_doc::error::MAX_SHEET_ID_BYTES;
    use curvebook_doc::{CellCoord, ColumnDesignHint, ColumnTable, DataSheet, MAX_TITLE_BYTES, TableArchetype};

    use super::{TransformContext, bounded_suffixed_str, remap_swapped_error_notes, remap_swapped_error_notes_batch};
    use crate::test_helpers as th;

    #[test]
    fn aud04_38_batched_remap_swapped_error_notes_handles_large_tables() {
        let n = 10_000_usize;
        let mut notes: Vec<_> = (0..n).map(|r| th::excl_note(r, r % 2, if r % 3 == 0 { 1 } else { 2 }, "qc", "note")).collect();
        let swapped: Vec<(usize, usize)> = (0..n).step_by(2).map(|r| (r, r % 2)).collect();
        remap_swapped_error_notes_batch(&mut notes, &swapped);
        assert!(notes.len() == n && notes.windows(2).all(|w| w[0].coord <= w[1].coord));
        for note in &notes {
            let orig_sub = if note.coord.row % 3 == 0 { 1 } else { 2 };
            assert_eq!(note.coord.subcol, if note.coord.row % 2 == 0 { 3 - orig_sub } else { orig_sub });
        }
        let mut single = vec![th::excl_note(4, 0, 1, "qc", "n")];
        remap_swapped_error_notes(&mut single, &[4], 0);
        assert_eq!(single[0].coord, CellCoord::new(4, 0, 2));
    }

    #[test]
    fn aud04_45_derived_ids_bounds_auto_suffixes_preserving_utf8() {
        let long_title = "µ".repeat(MAX_TITLE_BYTES / 2);
        assert_eq!(long_title.len(), MAX_TITLE_BYTES);
        let sheet = DataSheet::new(
            "s".repeat(MAX_SHEET_ID_BYTES),
            long_title,
            TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap()),
        )
        .unwrap();
        let (id, title) = TransformContext::default().derived_ids(&sheet, "normalize", "Normalized");
        assert!(id.len() <= MAX_SHEET_ID_BYTES && id.ends_with("-normalize"));
        assert!(title.len() <= MAX_TITLE_BYTES && title.ends_with(" (Normalized)"));
        let tiny = bounded_suffixed_str("abc", "µµµ", 3);
        assert!(tiny.len() <= 3 && tiny == "µ");
    }

    #[test]
    fn lineweaver_burk_guards_zeros_labels_axes_and_emits_distortion_diagnostics() -> Result<(), Box<dyn std::error::Error>> {
        use curvebook_doc::{
            AnalysisKind, AnalysisSpec, Cell, ChecklistStatus, ErrorPropagationMode, PharmacologyTransformPreset, SheetId, SubcolumnMode,
            TransformAnalysisSpec, XyTable,
        };

        // Exact Michaelis-Menten curve: Vmax = 100, Km = 2 => v = 100 * S / (2 + S), plus S = 0 row.
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 5)?;
        xy.x_title = String::from("[S]");
        xy.data_sets[0].title = String::from("Velocity");
        let s_vals = [0.0, 1.0, 2.0, 4.0, 8.0];
        for (r, &s) in s_vals.iter().enumerate() {
            xy.set_x_cell(r, Cell::number(s))?;
            let v = if s == 0.0 { 0.0 } else { 100.0 * s / (2.0 + s) };
            xy.data_sets[0].cells[r][0] = Cell::number(v);
        }
        let sheet = DataSheet::new(SheetId::new("xy-lb"), "Enzyme Kinetics", TableArchetype::Xy(xy))?;
        let spec = AnalysisSpec::new(
            SheetId::new("res-lb"),
            SheetId::new("xy-lb"),
            AnalysisKind::Transform(TransformAnalysisSpec::Math {
                x_transform: None,
                y_transform: None,
                error_propagation: ErrorPropagationMode::FirstOrderDeltaMethod,
                pharmacology_preset: Some(PharmacologyTransformPreset::LineweaverBurk),
                interchange_xy: false,
                zero_x_substitute: None,
                asymmetric_ci_policy: Default::default(),
            }),
        )?;
        let rs = super::execute_analysis_transform(&sheet, &spec, &TransformContext::default())?;
        let derived = rs.analyzable_data.as_ref().ok_or("missing derived XY table")?;
        let TableArchetype::Xy(out_xy) = &derived.archetype else {
            return Err("expected XY".into());
        };
        assert_eq!(out_xy.x_title, "1 / [S]");
        // Row 0 (S=0, v=0) must be blanked cleanly (never Inf/NaN).
        assert!(out_xy.x_summary_at(0)?.is_none());
        assert!(out_xy.data_sets[0].cells[0][0].active_number().is_none());
        // Exact apparent Vmax = 100, Km = 2 on noiseless Michaelis-Menten data.
        assert!((rs.lookup_scalar("Apparent Vmax", Some(0)).ok_or("vmax")? - 100.0).abs() < 1e-10);
        assert!((rs.lookup_scalar("Apparent Km", Some(0)).ok_or("km")? - 2.0).abs() < 1e-10);
        assert!((rs.lookup_scalar("Lineweaver-Burk R squared", Some(0)).ok_or("r2")? - 1.0).abs() < 1e-12);
        assert!(rs.tabs.iter().any(|t| t.id == "lineweaver-burk-summary"));
        assert!(rs.checklist.iter().any(|c| c.status == ChecklistStatus::Warn && c.assumption.contains("distortion")));
        Ok(())
    }
}
