//! Core data structures, equation category namespaces, hook resolution, and table extraction
//! helpers for `curvebook-fit`.

use std::collections::{BTreeMap, HashMap, HashSet};

use curvebook_doc::{
    AnalysisKind, Cell, CellValue, DataSetColumn, DataSheet, HookSource, InfoSheet, ResultSheet, ResultStatus, SheetId, SummaryModeRefusalReason,
    SummaryModeSupport, TableArchetype, TableKind, VariableColumn, VariableRole, parse_column_title_numeric_hook, summary_mode_row_for_spec,
};

use crate::error::FitError;

/// Built-in equation library namespaces for nonlinear regression.
/// Equation library category used to organize built-in regression models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EquationCategory {
    /// Agonist / stimulatory concentration-response models (e.g. `EC50`, Hill slope).
    DoseResponseStimulation,
    /// Antagonist / inhibitory concentration-response and Schild models (e.g. `IC50`).
    DoseResponseInhibition,
    /// Saturation, competition, and kinetic ligand-receptor binding models.
    ReceptorBinding,
    /// Michaelis–Menten, allosteric, and enzyme inhibition kinetic models.
    EnzymeKinetics,
    /// Exponential growth, association, and one-/multi-phase decay models.
    Exponential,
    /// Linear, quadratic, and higher-order polynomial models.
    Polynomial,
    /// Symmetric bell-shaped Gaussian and log-Gaussian peak models.
    Gaussian,
    /// Periodic sinusoidal and damped-oscillation wave models.
    SineWave,
    /// Gompertz, logistic, and sigmoidal population/cell growth models.
    Growth,
    /// Asymmetric chromatography/spectroscopy peak models.
    Peak,
    /// Standard-curve calibration models (4PL/5PL, hyperbola, polynomial).
    StandardCurve,
    /// Linear–quadratic and survival radiobiology dose models.
    Radiobiology,
    /// NIST Statistical Reference Datasets (StRD) nonlinear regression benchmark models.
    NistBenchmark,
    /// Caller-authored custom explicit, implicit, or ODE equation compiled via the DSL.
    UserDefined,
}

/// Single regression observation supporting 1D or multivariate predictors and optional replicate
/// summary spread (`sd`, `n`) or explicit point weight.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DataPoint {
    /// Independent predictor coordinates (`[x]` for standard XY curves, `[x1, x2, ...]` for multivariate).
    pub x: Vec<f64>,
    /// Dependent observed response `y`.
    pub y: f64,
    /// Within-point sample standard deviation when reconstructed from `Mean/SD/N` pre-summarized tables.
    pub sd: Option<f64>,
    /// Within-point sample size `n` when reconstructed from pre-summarized tables.
    pub n: Option<f64>,
    /// Explicit caller-supplied weight (overrides formulaic `WeightingScheme` when present and requested).
    pub explicit_weight: Option<f64>,
    /// Zero-based raw Y replicate subcolumn of the source observation.
    #[serde(default)]
    pub source_subcolumn: usize,
    /// Originating dataset column index (`0 = A`, `1 = B`, ...).
    pub data_set_index: usize,
    /// Originating table row index.
    pub row: usize,
}

impl DataPoint {
    /// Construct a simple unweighted 1D `(x, y)` observation.
    #[must_use]
    pub fn xy(x: f64, y: f64) -> Self {
        Self { x: vec![x], y, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 0 }
    }

    /// Construct a multivariate `(predictors, y)` observation.
    #[must_use]
    pub fn multi(predictors: Vec<f64>, y: f64) -> Self {
        Self { x: predictors, y, sd: None, n: None, explicit_weight: None, source_subcolumn: 0, data_set_index: 0, row: 0 }
    }

    /// Primary scalar predictor `x[0]`.
    #[must_use]
    pub fn x0(&self) -> f64 {
        self.x.first().copied().unwrap_or(0.0)
    }
}

/// Validate optional summarized-point replicate count (`n`) and sample `sd`, returning
/// `(n_f64, n_usize, (n - 1) * sd^2)` or defaulting to `(1.0, 1, 0.0)` for single raw points.
pub(crate) fn validated_replicate_moments(pt: &DataPoint) -> Result<(f64, usize, f64), FitError> {
    let Some(n_raw) = pt.n else {
        return Ok((1.0, 1, 0.0));
    };
    if !n_raw.is_finite() || !(1.0..=1e9).contains(&n_raw) || (n_raw - n_raw.round()).abs() > 1e-6 {
        return Err(FitError::InvalidParameter { name: "n", reason: format!("replicate count N must be a finite integer in 1..=1e9 (got {n_raw})") });
    }
    let n_usize = n_raw.round() as usize;
    let n_f = n_usize as f64;
    let ss_within = if let Some(sd) = pt.sd {
        if !sd.is_finite() || sd < 0.0 {
            return Err(FitError::InvalidParameter { name: "sd", reason: format!("standard deviation must be finite and non-negative (got {sd})") });
        }
        let ss = (n_f - 1.0) * sd * sd;
        if !ss.is_finite() {
            return Err(FitError::InvalidParameter { name: "sd", reason: String::from("within-replicate SS overflowed f64 range") });
        }
        ss
    } else {
        0.0
    };
    Ok((n_f, n_usize, ss_within))
}

/// Observed response for inverse calibration from a row where X is blank.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum UnknownResponse {
    /// A single response or a response already summarized on the input sheet.
    Single((usize, f64)),
    /// A raw response in a specific replicate subcolumn.
    Replicate {
        /// Zero-based table row index of the unknown observation.
        row: usize,
        /// Zero-based replicate subcolumn index within the dataset column.
        subcol: usize,
        /// Active numeric response value read from `(row, subcol)`.
        value: f64,
    },
}

impl From<(usize, f64)> for UnknownResponse {
    fn from(value: (usize, f64)) -> Self {
        Self::Single(value)
    }
}

impl UnknownResponse {
    /// Return the observed response value.
    #[must_use]
    pub const fn value(self) -> f64 {
        match self {
            Self::Single((_, value)) | Self::Replicate { value, .. } => value,
        }
    }
}

/// Extracted dataset series ready for linear or nonlinear regression.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DatasetSeries {
    /// Dataset index (`0 = A`, `1 = B`, ...).
    pub data_set_index: usize,
    /// Human column label (`"A"`, `"Dose 10 nM"`, etc.).
    pub label: String,
    /// Active included `(x, y)` observations.
    pub points: Vec<DataPoint>,
    /// Rows where `X` is blank and `Y` is active, queued for standard-curve inverse interpolation.
    pub unknowns_y: Vec<UnknownResponse>,
    /// Rows where `Y` is blank and `X` is active, queued for forward prediction.
    pub unknowns_x: Vec<(usize, f64)>,
    /// Whether observations came from pre-summarized `(Mean, SD/SEM/%CV, N)` subcolumns.
    pub is_pre_summarized: bool,
    /// Numeric value parsed from the dataset column title (e.g. `[I]` for global inhibition fits).
    pub column_title_value: Option<f64>,
}

/// Derived scalar computed from fitted parameters (e.g. `EC50 = 10^LogEC50`, `t_half = ln(2)/K`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DerivedScalar {
    /// Human-readable quantity name (e.g. `"IC50"`, `"HalfLife"`, `"AUC"`).
    pub name: String,
    /// Computed scalar value in the physical units implied by `units` (`NaN` if undefined at the fit optimum).
    pub value: f64,
    /// Optional physical unit label matching the fitted axis scale (`None` when dimensionless).
    pub units: Option<String>,
}

/// Runtime context for resolving [`HookSource`] references (`InfoConstant`, `ResultScalar`,
/// `ColumnTitleValue`) without cyclic references.
#[derive(Debug, Clone, Default)]
pub struct HookContext<'a> {
    /// Upstream metadata `InfoSheet` slices searched when resolving `InfoConstant` hooks.
    pub info_sheets: &'a [InfoSheet],
    /// Upstream `ResultSheet` slices searched when resolving `ResultScalar` hooks.
    pub result_sheets: &'a [ResultSheet],
    /// Upstream `DataSheet` slices searched when resolving dataset column-title numeric hooks.
    pub data_sheets: &'a [DataSheet],
    /// Preferred primary `InfoSheet` ID searched before scanning remaining `info_sheets`; `None` checks all in order.
    pub primary_info_sheet_id: Option<&'a SheetId>,
    /// Visited hook target keys maintained during resolution to detect and break cyclic references.
    pub visited_sheets: HashSet<String>,
}

impl<'a> HookContext<'a> {
    /// Resolve a [`HookSource`] to a finite `f64` scalar.
    ///
    /// # Errors
    /// Returns [`FitError::HookResolutionError`] if the referenced sheet, constant, or scalar is
    /// missing, non-numeric, non-finite, or introduces a cycle.
    pub fn resolve(&self, source: &HookSource) -> Result<f64, FitError> {
        match source {
            HookSource::InfoConstant { info_sheet_id, constant_name } => self.resolve_info_constant(info_sheet_id.as_ref(), constant_name),
            HookSource::ResultScalar { result_sheet_id, parameter_key, data_set_index } => {
                self.resolve_result_scalar(result_sheet_id, parameter_key, *data_set_index)
            }
            HookSource::ColumnTitleValue { data_sheet_id, data_set_index } => self.resolve_column_title_value(data_sheet_id, *data_set_index),
        }
    }

    fn resolve_info_constant(&self, info_sheet_id: Option<&SheetId>, constant_name: &str) -> Result<f64, FitError> {
        let target_id = info_sheet_id.or(self.primary_info_sheet_id);
        let sheet = if let Some(id) = target_id { self.info_sheets.iter().find(|s| &s.id == id) } else { self.info_sheets.first() };
        let Some(sheet) = sheet else {
            return Err(FitError::HookResolutionError { message: format!("info sheet for constant `{constant_name}` not found") });
        };
        let Some(entry) = sheet.get_constant(constant_name) else {
            return Err(FitError::HookResolutionError { message: format!("constant `{constant_name}` not found on info sheet `{}`", sheet.id) });
        };
        entry
            .value
            .as_finite_f64()
            .ok_or_else(|| FitError::HookResolutionError { message: format!("info constant `{constant_name}` is not a finite number") })
    }

    fn resolve_result_scalar(&self, result_sheet_id: &SheetId, parameter_key: &str, data_set_index: Option<usize>) -> Result<f64, FitError> {
        if self.visited_sheets.contains(result_sheet_id.as_str()) {
            return Err(FitError::HookResolutionError { message: format!("cyclic hook reference involving result sheet `{result_sheet_id}`") });
        }
        let Some(rs) = self.result_sheets.iter().find(|r| &r.id == result_sheet_id) else {
            return Err(FitError::HookResolutionError { message: format!("result sheet `{result_sheet_id}` not found") });
        };
        if let ResultStatus::Failed { message, .. } = &rs.status {
            return Err(FitError::HookResolutionError { message: format!("upstream result sheet `{result_sheet_id}` failed: {message}") });
        }
        if rs.is_imported() {
            return Err(FitError::HookResolutionError {
                message: format!("upstream result sheet `{result_sheet_id}` is an imported snapshot and cannot supply computed hook scalars"),
            });
        }
        rs.lookup_scalar(parameter_key, data_set_index).ok_or_else(|| FitError::HookResolutionError {
            message: format!("scalar `{parameter_key}` missing or non-finite on `{result_sheet_id}`"),
        })
    }

    fn resolve_column_title_value(&self, data_sheet_id: &SheetId, data_set_index: usize) -> Result<f64, FitError> {
        let Some(ds_sheet) = self.data_sheets.iter().find(|d| &d.id == data_sheet_id) else {
            return Err(FitError::HookResolutionError { message: format!("data sheet `{data_sheet_id}` not found") });
        };
        let title = match &ds_sheet.archetype {
            TableArchetype::Xy(xy) => xy.data_sets.get(data_set_index).map(|c| c.title.as_str()),
            TableArchetype::Grouped(g) => g.data_sets.get(data_set_index).map(|c| c.title.as_str()),
            TableArchetype::Column(c) => c.data_sets.get(data_set_index).map(|col| col.title.as_str()),
            _ => None,
        };
        let Some(title) = title else {
            return Err(FitError::HookResolutionError { message: format!("column index {data_set_index} out of bounds on `{data_sheet_id}`") });
        };
        parse_column_title_numeric_hook(title).map_err(|err| FitError::HookResolutionError {
            message: format!("column title `{title}` does not contain a finite numeric value: {err}"),
        })
    }
}

/// Strictly parse an unambiguous finite numeric scalar from a column title
/// (e.g. `"10"`, `"10 nM"`, or `"[I] = 10 nM"` -> `Some(10.0)`), rejecting ambiguous
/// identifiers with embedded hyphens/digits (`"Inhibitor-10 nM"`, `"IL-6 10 ng/mL"`).
#[must_use]
pub fn parse_leading_number(text: &str) -> Option<f64> {
    parse_column_title_numeric_hook(text).ok()
}

/// Stable lookup key for a per-dataset value in a multi-dataset fit, such as `"AntagConc [DS1]"`.
///
/// `data_set_index` is the zero-based source column index, shown one-based. Fixed-constant names,
/// published scalar keys, and equation-annotation lookups all use this key, so it must stay
/// independent of column titles. Result tables show titles through their own display labels.
pub(crate) fn per_dataset_parameter_key(name: &str, data_set_index: usize) -> String {
    format!("{name} [DS{}]", data_set_index.saturating_add(1))
}

/// Column selection & multi-factor Cartesian grouping options for `MultipleVariables` tables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MultivarExtractionSpec<'a> {
    /// Zero-based grouping column indices whose distinct level tuples partition observations into separate series; empty puts all observations into one series.
    pub group_columns: &'a [usize],
    /// Zero-based numeric predictor column indices mapped into `DataPoint::x`; empty falls back to auto-detected numeric X columns.
    pub x_columns: &'a [usize],
    /// Zero-based dependent response column index mapped into `DataPoint::y`; `None` auto-selects the primary response column.
    pub y_column: Option<usize>,
}

/// Extract regression datasets from a [`DataSheet`] (`Xy`, `Grouped`, or `MultipleVariables`).
///
/// # Errors
/// Returns [`FitError::IncompatibleTable`] for unsupported table archetypes or [`FitError::EmptyInput`]
/// if no active finite observations exist across the selected data sets.
pub fn extract_datasets_from_sheet(sheet: &DataSheet, selected_data_sets: &[usize]) -> Result<Vec<DatasetSeries>, FitError> {
    extract_datasets_from_sheet_with_options(sheet, selected_data_sets, &MultivarExtractionSpec::default())
}

/// Extract regression datasets from a [`DataSheet`] with explicit `MultipleVariables` factor,
/// predictor, and response column mappings.
///
/// # Errors
/// Returns [`FitError::IncompatibleTable`] for unsupported table archetypes,
/// [`FitError::InvalidParameter`] if multivariate column indices are out of range, or
/// [`FitError::EmptyInput`] if no active finite observations exist.
pub fn extract_datasets_from_sheet_with_options(
    sheet: &DataSheet,
    selected_data_sets: &[usize],
    mv_spec: &MultivarExtractionSpec<'_>,
) -> Result<Vec<DatasetSeries>, FitError> {
    let y_cols: Vec<usize> = mv_spec.y_column.into_iter().collect();
    extract_datasets_from_sheet_multi(sheet, selected_data_sets, mv_spec.group_columns, mv_spec.x_columns, &y_cols)
}

/// Extract regression datasets from a [`DataSheet`] supporting multi-response `MultipleVariables`
/// assignments (`M` continuous response columns x `G` categorical grouping combinations).
///
/// # Errors
/// Returns [`FitError::IncompatibleTable`] for unsupported table archetypes,
/// [`FitError::InvalidParameter`] if multivariate column indices are out of range or overlap, or
/// [`FitError::EmptyInput`] if no active finite observations exist.
pub fn extract_datasets_from_sheet_multi(
    sheet: &DataSheet,
    selected_data_sets: &[usize],
    group_columns: &[usize],
    x_columns: &[usize],
    y_columns: &[usize],
) -> Result<Vec<DatasetSeries>, FitError> {
    match &sheet.archetype {
        TableArchetype::Xy(xy) => extract_from_xy(xy, selected_data_sets),
        TableArchetype::Grouped(g) => extract_from_grouped(g, selected_data_sets),
        TableArchetype::MultipleVariables(mv) => extract_from_multivar_multi(mv, selected_data_sets, group_columns, x_columns, y_columns),
        other => Err(FitError::IncompatibleTable { kind: other.kind() }),
    }
}

/// Collapse each dataset's observations within identical table rows to their unweighted sample
/// mean with `1` observation per row (`ReplicateFitMode::RowMeansOnly`; Motulsky & Christopoulos 2004).
///
/// Pre-summarized `(Mean, SD, n)` points drop their within-row `sd` and `n` moments (`is_pre_summarized = false`)
/// so each row contributes a single unweighted mean observation.
pub fn collapse_datasets_to_row_means(datasets: &mut [DatasetSeries]) {
    for ds in datasets.iter_mut() {
        if ds.is_pre_summarized {
            for pt in &mut ds.points {
                pt.sd = None;
                pt.n = None;
            }
            ds.is_pre_summarized = false;
            continue;
        }
        let mut by_row: BTreeMap<usize, Vec<&DataPoint>> = BTreeMap::new();
        for pt in &ds.points {
            by_row.entry(pt.row).or_default().push(pt);
        }
        let mut collapsed = Vec::with_capacity(by_row.len());
        for (row, pts) in by_row {
            let Some(first) = pts.first() else {
                continue;
            };
            let n = pts.len() as f64;
            let mean_y = pts.iter().map(|p| p.y).sum::<f64>() / n;
            collapsed.push(DataPoint {
                x: first.x.clone(),
                y: mean_y,
                sd: None,
                n: None,
                explicit_weight: None,
                source_subcolumn: 0,
                data_set_index: first.data_set_index,
                row,
            });
        }
        ds.points = collapsed;
    }
}

/// Filter dataset observations against optional `X` and `Y` inclusive bounds (`Don't fit points when...`),
/// dropping any datasets that retain zero points.
pub fn filter_datasets_by_range(
    datasets: &mut Vec<DatasetSeries>,
    x_range: Option<(Option<f64>, Option<f64>)>,
    y_range: Option<(Option<f64>, Option<f64>)>,
) {
    if x_range.is_none() && y_range.is_none() {
        return;
    }
    for ds in datasets.iter_mut() {
        ds.points.retain(|pt| {
            let x0 = pt.x0();
            let x_ok = x_range.is_none_or(|(lo, hi)| lo.is_none_or(|v| !v.is_finite() || x0 >= v) && hi.is_none_or(|v| !v.is_finite() || x0 <= v));
            let y_ok =
                y_range.is_none_or(|(lo, hi)| lo.is_none_or(|v| !v.is_finite() || pt.y >= v) && hi.is_none_or(|v| !v.is_finite() || pt.y <= v));
            x_ok && y_ok
        });
    }
    datasets.retain(|ds| !ds.points.is_empty());
}

fn resolve_selected_indices(total: usize, selected: &[usize]) -> Result<Vec<usize>, FitError> {
    if selected.is_empty() {
        return Ok((0..total).collect());
    }
    let mut seen = HashSet::with_capacity(selected.len());
    for &idx in selected {
        if idx >= total {
            return Err(FitError::InvalidParameter {
                name: "selected_data_sets",
                reason: format!("selected dataset index {idx} out of range (expected < {total})"),
            });
        }
        if !seen.insert(idx) {
            return Err(FitError::InvalidParameter {
                name: "selected_data_sets",
                reason: format!("duplicate dataset index {idx} in `selected_data_sets`"),
            });
        }
    }
    Ok(selected.to_vec())
}

/// Enforce the canonical `curvebook-doc` summary-mode support matrix ([`curvebook_doc::SUMMARY_MODE_MATRIX`])
/// for linear and nonlinear regression analyses against `Xy` or `Grouped` tables.
///
/// Leaves incompatible table archetypes (`!kind.is_compatible_with(table_kind)`) to fail with
/// [`FitError::IncompatibleTable`] downstream, and returns [`FitError::RequiresRawReplicates`] or
/// [`FitError::UnsupportedSubcolumnMode`] for every refused summary cell.
pub fn enforce_summary_mode_matrix(sheet: &DataSheet, kind: &AnalysisKind) -> Result<(), FitError> {
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
        SummaryModeSupport::Refused(SummaryModeRefusalReason::RequiresRawReplicates) => Err(FitError::RequiresRawReplicates),
        SummaryModeSupport::Refused(reason) => {
            Err(FitError::UnsupportedSubcolumnMode { analysis: row.label, table_kind, mode, reason: reason.message() })
        }
    }
}

fn extract_from_xy(xy: &curvebook_doc::XyTable, selected_data_sets: &[usize]) -> Result<Vec<DatasetSeries>, FitError> {
    let indices = resolve_selected_indices(xy.data_sets.len(), selected_data_sets)?;
    let mut series_list = Vec::with_capacity(indices.len());
    for ds_idx in indices {
        let Some(ds) = xy.data_sets.get(ds_idx) else {
            continue;
        };
        let mut series = DatasetSeries {
            data_set_index: ds_idx,
            label: ds.title.clone(),
            points: Vec::new(),
            unknowns_y: Vec::new(),
            unknowns_x: Vec::new(),
            is_pre_summarized: xy.y_subcolumn_mode.is_pre_summarized(),
            column_title_value: parse_leading_number(&ds.title),
        };
        populate_xy_column(xy, ds, ds_idx, &mut series)?;
        if !series.points.is_empty() {
            series_list.push(series);
        }
    }
    if series_list.is_empty() {
        return Err(FitError::EmptyInput);
    }
    Ok(series_list)
}

fn x_row_is_blank_unknown(xy: &curvebook_doc::XyTable, row: usize) -> bool {
    match &xy.x_mode {
        curvebook_doc::XColumnMode::Numbers { .. } | curvebook_doc::XColumnMode::DateOrElapsedTime { .. } => {
            xy.x_cells.get(row).is_some_and(|subs| !subs.is_empty() && subs.iter().all(|cell| !cell.excluded && cell.is_empty()))
        }
        curvebook_doc::XColumnMode::GeneratedSequence { .. } => false,
    }
}

fn y_cells_are_blank_unknown(subcells: &[Cell]) -> bool {
    !subcells.is_empty() && subcells.iter().all(|cell| !cell.excluded && cell.is_empty())
}

fn populate_xy_column(xy: &curvebook_doc::XyTable, ds: &DataSetColumn, ds_idx: usize, series: &mut DatasetSeries) -> Result<(), FitError> {
    for (row, subcells) in ds.cells.iter().enumerate() {
        let x_opt = xy.x_summary_at(row).map_err(|e| FitError::InvalidParameter { name: "pre_summarized_x", reason: e.to_string() })?;
        let x_blank = x_opt.is_none() && x_row_is_blank_unknown(xy, row);
        // Excluded (or non-blank non-numeric) X rows must be dropped outright rather than
        // masquerading as blank X cells queued for inverse interpolation.
        if x_opt.is_none() && !x_blank {
            continue;
        }

        if xy.y_subcolumn_mode.supports_parametric_moments() {
            if x_blank {
                if let Some(y) = subcells.first().and_then(Cell::active_number) {
                    series.unknowns_y.push((row, y).into());
                }
                continue;
            }
            let pt_opt =
                xy.summarized_y_point(row, ds_idx).map_err(|e| FitError::InvalidParameter { name: "pre_summarized_y", reason: e.to_string() })?;
            match (x_opt, pt_opt) {
                (Some(x), Some(pt)) => {
                    series.points.push(DataPoint {
                        x: vec![x],
                        y: pt.mean,
                        sd: Some(pt.sd),
                        n: Some(pt.n),
                        explicit_weight: None,
                        source_subcolumn: 0,
                        data_set_index: ds_idx,
                        row,
                    });
                }
                (Some(x), None) if y_cells_are_blank_unknown(subcells) => series.unknowns_x.push((row, x)),
                _ => {}
            }
        } else if matches!(xy.y_subcolumn_mode, curvebook_doc::SubcolumnMode::MeanSd | curvebook_doc::SubcolumnMode::MeanSem) {
            return Err(FitError::UnsupportedSubcolumnMode {
                analysis: "Regression fitting",
                table_kind: TableKind::Xy,
                mode: xy.y_subcolumn_mode,
                reason: SummaryModeRefusalReason::MissingSampleSizeOrVariance.message(),
            });
        } else if xy.y_subcolumn_mode.is_pre_summarized() {
            match (x_opt, subcells.first().and_then(Cell::active_number)) {
                (Some(x), Some(y)) => {
                    series.points.push(DataPoint {
                        x: vec![x],
                        y,
                        sd: None,
                        n: None,
                        explicit_weight: None,
                        source_subcolumn: 0,
                        data_set_index: ds_idx,
                        row,
                    });
                }
                (None, Some(y)) if x_blank => series.unknowns_y.push((row, y).into()),
                (Some(x), None) if y_cells_are_blank_unknown(subcells) => series.unknowns_x.push((row, x)),
                _ => {}
            }
        } else {
            let mut had_y = false;
            let multiple_replicates = matches!(xy.y_subcolumn_mode, curvebook_doc::SubcolumnMode::RawReplicates(k) if k > 1);
            for (subcol, cell) in subcells.iter().enumerate() {
                if let Some(y) = cell.active_number() {
                    had_y = true;
                    if let Some(x) = x_opt {
                        series.points.push(DataPoint {
                            x: vec![x],
                            y,
                            sd: None,
                            n: None,
                            explicit_weight: None,
                            source_subcolumn: subcol,
                            data_set_index: ds_idx,
                            row,
                        });
                    } else if x_blank {
                        let response = if multiple_replicates {
                            UnknownResponse::Replicate { row, subcol, value: y }
                        } else {
                            UnknownResponse::Single((row, y))
                        };
                        series.unknowns_y.push(response);
                    }
                }
            }
            if !had_y
                && let Some(x) = x_opt
                && y_cells_are_blank_unknown(subcells)
            {
                series.unknowns_x.push((row, x));
            }
        }
    }
    Ok(())
}

fn extract_from_grouped(g: &curvebook_doc::GroupedTable, selected_data_sets: &[usize]) -> Result<Vec<DatasetSeries>, FitError> {
    let indices = resolve_selected_indices(g.data_sets.len(), selected_data_sets)?;
    let mut series_list = Vec::with_capacity(indices.len());
    for ds_idx in indices {
        let Some(ds) = g.data_sets.get(ds_idx) else {
            continue;
        };
        let mut points = Vec::new();
        for (row, subcells) in ds.cells.iter().enumerate() {
            let x_val = g
                .row_numeric_spacing
                .as_ref()
                .and_then(|v| v.get(row).copied().flatten())
                .or_else(|| g.row_titles.get(row).and_then(|t| parse_leading_number(t)))
                .unwrap_or(row as f64);
            if g.subcolumn_mode.supports_parametric_moments() {
                let pt_opt = curvebook_doc::SummarizedPoint::reconstruct(g.subcolumn_mode, subcells)
                    .map_err(|e| FitError::InvalidParameter { name: "pre_summarized_y", reason: e.to_string() })?;
                if let Some(pt) = pt_opt {
                    points.push(DataPoint {
                        x: vec![x_val],
                        y: pt.mean,
                        sd: Some(pt.sd),
                        n: Some(pt.n),
                        explicit_weight: None,
                        source_subcolumn: 0,
                        data_set_index: ds_idx,
                        row,
                    });
                }
            } else if matches!(g.subcolumn_mode, curvebook_doc::SubcolumnMode::MeanSd | curvebook_doc::SubcolumnMode::MeanSem) {
                return Err(FitError::UnsupportedSubcolumnMode {
                    analysis: "Regression fitting",
                    table_kind: TableKind::Grouped,
                    mode: g.subcolumn_mode,
                    reason: SummaryModeRefusalReason::MissingSampleSizeOrVariance.message(),
                });
            } else if g.subcolumn_mode.is_pre_summarized() {
                if let Some(y) = subcells.first().and_then(Cell::active_number) {
                    points.push(DataPoint {
                        x: vec![x_val],
                        y,
                        sd: None,
                        n: None,
                        explicit_weight: None,
                        source_subcolumn: 0,
                        data_set_index: ds_idx,
                        row,
                    });
                }
            } else {
                for (subcol, cell) in subcells.iter().enumerate() {
                    if let Some(y) = cell.active_number() {
                        points.push(DataPoint {
                            x: vec![x_val],
                            y,
                            sd: None,
                            n: None,
                            explicit_weight: None,
                            source_subcolumn: subcol,
                            data_set_index: ds_idx,
                            row,
                        });
                    }
                }
            }
        }
        if !points.is_empty() {
            series_list.push(DatasetSeries {
                data_set_index: ds_idx,
                label: ds.title.clone(),
                points,
                unknowns_y: Vec::new(),
                unknowns_x: Vec::new(),
                is_pre_summarized: g.subcolumn_mode.is_pre_summarized(),
                column_title_value: parse_leading_number(&ds.title),
            });
        }
    }
    if series_list.is_empty() {
        return Err(FitError::EmptyInput);
    }
    Ok(series_list)
}

fn mv_group_token(cell: &Cell) -> Option<String> {
    if cell.excluded {
        return None;
    }
    match &cell.value {
        CellValue::Empty => None,
        CellValue::Text(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        CellValue::Number(v) if v.is_finite() => Some(format!("{v}")),
        CellValue::Number(_) => None,
    }
}

fn is_mv_numeric_role(var: &VariableColumn) -> bool {
    !matches!(var.role, VariableRole::Label | VariableRole::Categorical)
}

fn pick_default_mv_numeric_col(mv: &curvebook_doc::MultipleVariablesTable, candidates: impl Iterator<Item = usize>) -> Option<usize> {
    let mut role_match = None;
    for idx in candidates {
        let Some(var) = mv.variables.get(idx) else {
            continue;
        };
        if !is_mv_numeric_role(var) {
            continue;
        }
        if var.cells.iter().any(|c| c.active_number().is_some()) {
            return Some(idx);
        }
        if role_match.is_none() {
            role_match = Some(idx);
        }
    }
    role_match
}

pub(crate) struct MvResolvedColumns {
    pub(crate) x_cols: Vec<usize>,
    pub(crate) y_cols: Vec<usize>,
    pub(crate) group_cols: Vec<usize>,
}

struct MvGroupEntry {
    y_col: usize,
    y_title: String,
    tokens: Vec<String>,
    points: Vec<DataPoint>,
}

#[cfg(test)]
pub(crate) fn resolve_mv_columns(
    mv: &curvebook_doc::MultipleVariablesTable,
    selected_data_sets: &[usize],
    mv_spec: &MultivarExtractionSpec<'_>,
) -> Result<(Vec<usize>, usize, Vec<usize>), FitError> {
    let y_slice: Vec<usize> = mv_spec.y_column.into_iter().collect();
    let resolved = resolve_mv_columns_multi(mv, selected_data_sets, mv_spec.group_columns, mv_spec.x_columns, &y_slice)?;
    let y_col = resolved.y_cols.first().copied().ok_or(FitError::EmptyInput)?;
    Ok((resolved.x_cols, y_col, resolved.group_cols))
}

pub(crate) fn resolve_mv_columns_multi(
    mv: &curvebook_doc::MultipleVariablesTable,
    selected_data_sets: &[usize],
    group_columns: &[usize],
    x_columns: &[usize],
    y_columns: &[usize],
) -> Result<MvResolvedColumns, FitError> {
    let n_vars = mv.variables.len();
    let mut group_set = HashSet::with_capacity(group_columns.len());
    for &gc in group_columns {
        if gc >= n_vars {
            return Err(FitError::InvalidParameter { name: "mv_group_columns", reason: format!("Column index {gc} >= {n_vars}") });
        }
        if !group_set.insert(gc) {
            return Err(FitError::InvalidParameter { name: "mv_group_columns", reason: format!("Duplicate column index {gc}") });
        }
    }
    let mut x_set = HashSet::with_capacity(x_columns.len());
    for &xc in x_columns {
        if xc >= n_vars {
            return Err(FitError::InvalidParameter { name: "mv_x_columns", reason: format!("Column index {xc} >= {n_vars}") });
        }
        if !x_set.insert(xc) {
            return Err(FitError::InvalidParameter { name: "mv_x_columns", reason: format!("Duplicate column index {xc}") });
        }
        if group_set.contains(&xc) {
            return Err(FitError::InvalidParameter {
                name: "mv_x_columns",
                reason: format!("Column {xc} cannot be both predictor (X) and grouping column"),
            });
        }
        if !is_mv_numeric_role(&mv.variables[xc]) {
            return Err(FitError::InvalidParameter {
                name: "mv_x_columns",
                reason: format!("Predictor column {xc} must be numeric (got {:?})", mv.variables[xc].role),
            });
        }
    }
    let mut y_set = HashSet::with_capacity(y_columns.len());
    let y_param_name = if y_columns.len() > 1 { "mv_y_columns" } else { "mv_y_column" };
    for &yc in y_columns {
        if yc >= n_vars {
            return Err(FitError::InvalidParameter { name: y_param_name, reason: format!("Column index {yc} >= {n_vars}") });
        }
        if !y_set.insert(yc) {
            return Err(FitError::InvalidParameter { name: y_param_name, reason: format!("Duplicate column index {yc}") });
        }
        if group_set.contains(&yc) {
            return Err(FitError::InvalidParameter {
                name: y_param_name,
                reason: format!("Column {yc} cannot be both response (Y) and grouping column"),
            });
        }
        if x_set.contains(&yc) {
            return Err(FitError::InvalidParameter {
                name: y_param_name,
                reason: format!("Column {yc} cannot be both predictor (X) and response (Y)"),
            });
        }
        if !is_mv_numeric_role(&mv.variables[yc]) {
            return Err(FitError::InvalidParameter {
                name: y_param_name,
                reason: format!("Response column {yc} must be numeric (got {:?})", mv.variables[yc].role),
            });
        }
    }

    if x_columns.is_empty() || y_columns.is_empty() {
        let _ = resolve_selected_indices(n_vars, selected_data_sets)?;
    }

    let selected_numeric: Vec<usize> = selected_data_sets
        .iter()
        .copied()
        .filter(|&idx| idx < n_vars && !group_set.contains(&idx) && is_mv_numeric_role(&mv.variables[idx]))
        .collect();

    let resolved_y_cols: Vec<usize> = if !y_columns.is_empty() {
        y_columns.to_vec()
    } else if let Some(&sel) = selected_numeric.iter().rev().find(|idx| !x_set.contains(idx)) {
        vec![sel]
    } else if let Some(def) = pick_default_mv_numeric_col(mv, (0..n_vars).rev().filter(|idx| !x_set.contains(idx) && !group_set.contains(idx))) {
        vec![def]
    } else {
        let numeric_avail = mv.variables.iter().filter(|v| is_mv_numeric_role(v)).count();
        return Err(FitError::InsufficientData { required: 2, actual: numeric_avail });
    };
    let y_used: HashSet<usize> = resolved_y_cols.iter().copied().collect();

    let selected_x: Vec<usize> = selected_numeric.iter().copied().filter(|idx| !y_used.contains(idx)).collect();
    let x_cols = if !x_columns.is_empty() {
        x_columns.to_vec()
    } else if !selected_x.is_empty() {
        selected_x
    } else if let Some(def) = pick_default_mv_numeric_col(mv, (0..n_vars).filter(|idx| !y_used.contains(idx) && !group_set.contains(idx))) {
        vec![def]
    } else {
        let numeric_avail = mv.variables.iter().filter(|v| is_mv_numeric_role(v)).count();
        return Err(FitError::InsufficientData { required: 2, actual: numeric_avail });
    };

    let x_used: HashSet<usize> = x_cols.iter().copied().collect();
    let selected_categorical: Vec<usize> = selected_data_sets
        .iter()
        .copied()
        .filter(|&idx| {
            idx < n_vars
                && !y_used.contains(&idx)
                && !x_used.contains(&idx)
                && mv.variables.get(idx).is_some_and(|v| v.role == VariableRole::Categorical)
        })
        .collect();
    let group_cols = if !group_columns.is_empty() {
        group_columns.to_vec()
    } else if !selected_categorical.is_empty() {
        selected_categorical
    } else if selected_data_sets.len() < 2 {
        mv.variables
            .iter()
            .enumerate()
            .find(|(idx, v)| !y_used.contains(idx) && !x_used.contains(idx) && v.role == VariableRole::Categorical)
            .map(|(idx, _)| vec![idx])
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok(MvResolvedColumns { x_cols, y_cols: resolved_y_cols, group_cols })
}

pub(crate) fn extract_from_multivar_multi_with_y_map(
    mv: &curvebook_doc::MultipleVariablesTable,
    selected_data_sets: &[usize],
    group_columns: &[usize],
    x_columns: &[usize],
    y_columns: &[usize],
) -> Result<(Vec<DatasetSeries>, HashMap<usize, usize>), FitError> {
    if mv.variables.len() < 2 {
        return Err(FitError::InsufficientData { required: 2, actual: mv.variables.len() });
    }
    let MvResolvedColumns { x_cols, y_cols, group_cols } = resolve_mv_columns_multi(mv, selected_data_sets, group_columns, x_columns, y_columns)?;
    let multi_y = y_cols.len() > 1;
    let n_rows = mv.row_count();

    let level_ranks: Vec<HashMap<String, usize>> = group_cols
        .iter()
        .filter_map(|&gc| mv.variables.get(gc))
        .map(|var| var.ordered_levels().into_iter().enumerate().map(|(i, lvl)| (lvl.level, i)).collect())
        .collect();

    let mut all_entries: Vec<MvGroupEntry> = Vec::new();
    for &y_col in &y_cols {
        let Some(y_var) = mv.variables.get(y_col) else {
            continue;
        };
        let mut grouped: BTreeMap<Vec<String>, Vec<DataPoint>> = BTreeMap::new();
        for row in 0..n_rows {
            let Some(y_val) = y_var.cells.get(row).and_then(Cell::active_number) else {
                continue;
            };
            let mut xs = Vec::with_capacity(x_cols.len());
            let mut all_x = true;
            for &xc in &x_cols {
                if let Some(xv) = mv.variables.get(xc).and_then(|v| v.cells.get(row)).and_then(Cell::active_number) {
                    xs.push(xv);
                } else {
                    all_x = false;
                    break;
                }
            }
            if !all_x {
                continue;
            }
            let mut key_tokens = Vec::with_capacity(group_cols.len());
            let mut all_groups = true;
            for &gc in &group_cols {
                if let Some(tok) = mv.variables.get(gc).and_then(|v| v.cells.get(row)).and_then(mv_group_token) {
                    key_tokens.push(tok);
                } else {
                    all_groups = false;
                    break;
                }
            }
            if !all_groups {
                continue;
            }
            grouped.entry(key_tokens).or_default().push(DataPoint {
                x: xs,
                y: y_val,
                sd: None,
                n: None,
                explicit_weight: None,
                source_subcolumn: 0,
                data_set_index: 0,
                row,
            });
        }

        let mut entries: Vec<(Vec<String>, Vec<DataPoint>)> = grouped.into_iter().collect();
        if !group_cols.is_empty() {
            entries.sort_by(|(ka, _), (kb, _)| {
                for (pos, (ta, tb)) in ka.iter().zip(kb.iter()).enumerate() {
                    let ra = level_ranks.get(pos).and_then(|m| m.get(ta)).copied().unwrap_or(usize::MAX);
                    let rb = level_ranks.get(pos).and_then(|m| m.get(tb)).copied().unwrap_or(usize::MAX);
                    let cmp = ra.cmp(&rb).then_with(|| ta.cmp(tb));
                    if cmp != std::cmp::Ordering::Equal {
                        return cmp;
                    }
                }
                ka.cmp(kb)
            });
        }
        for (tokens, points) in entries {
            all_entries.push(MvGroupEntry { y_col, y_title: y_var.title.clone(), tokens, points });
        }
    }

    let explicit_column_mapping = !x_columns.is_empty() && !y_columns.is_empty();
    if explicit_column_mapping && group_cols.is_empty() && !multi_y && !selected_data_sets.is_empty() {
        let _ = resolve_selected_indices(mv.variables.len(), selected_data_sets)?;
    }
    let keep_indices = if explicit_column_mapping && (!group_cols.is_empty() || multi_y) && !selected_data_sets.is_empty() {
        resolve_selected_indices(all_entries.len(), selected_data_sets)?
    } else {
        (0..all_entries.len()).collect()
    };

    let mut indexed: Vec<Option<MvGroupEntry>> = all_entries.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(keep_indices.len());
    let mut y_col_map = HashMap::with_capacity(keep_indices.len());
    for idx in keep_indices {
        let Some(MvGroupEntry { y_col, y_title, tokens, points: mut pts }) = indexed.get_mut(idx).and_then(Option::take) else {
            continue;
        };
        for p in &mut pts {
            p.data_set_index = idx;
        }
        let label = if multi_y {
            if tokens.is_empty() { y_title } else { format!("{y_title} | {}", tokens.join(" | ")) }
        } else if tokens.is_empty() {
            y_title
        } else {
            tokens.join(" | ")
        };
        let title_val = tokens.first().and_then(|t| parse_leading_number(t)).or_else(|| parse_leading_number(&label));
        y_col_map.insert(idx, y_col);
        out.push(DatasetSeries {
            data_set_index: idx,
            label,
            points: pts,
            unknowns_y: Vec::new(),
            unknowns_x: Vec::new(),
            is_pre_summarized: false,
            column_title_value: title_val,
        });
    }
    if out.is_empty() {
        return Err(FitError::EmptyInput);
    }
    Ok((out, y_col_map))
}

fn extract_from_multivar_multi(
    mv: &curvebook_doc::MultipleVariablesTable,
    selected_data_sets: &[usize],
    group_columns: &[usize],
    x_columns: &[usize],
    y_columns: &[usize],
) -> Result<Vec<DatasetSeries>, FitError> {
    extract_from_multivar_multi_with_y_map(mv, selected_data_sets, group_columns, x_columns, y_columns).map(|(out, _)| out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use curvebook_doc::{Cell, DataSheet, MultipleVariablesTable, SheetId, TableArchetype, VariableRole};

    #[test]
    fn resolve_mv_columns_excludes_label_and_categorical_from_x_and_y() {
        let mut mv = MultipleVariablesTable::new(4, 6).unwrap();
        mv.variables[0].title = String::from("Well");
        mv.variables[0].lock_role(VariableRole::Label);
        mv.variables[1].title = String::from("Compound");
        mv.variables[1].lock_role(VariableRole::Categorical);
        mv.variables[2].title = String::from("LogConcM");
        mv.variables[2].lock_role(VariableRole::Continuous);
        mv.variables[3].title = String::from("Response");
        mv.variables[3].lock_role(VariableRole::Continuous);

        let rows = [
            ("A1", "InhibitorA", -9.0, 98.0),
            ("A2", "InhibitorA", -7.0, 50.0),
            ("A3", "InhibitorA", -5.0, 2.0),
            ("B1", "InhibitorB", -9.0, 99.0),
            ("B2", "InhibitorB", -6.0, 76.0),
            ("B3", "InhibitorB", -4.0, 3.0),
        ];
        for (r, &(well, cmpd, x, y)) in rows.iter().enumerate() {
            mv.variables[0].cells[r] = Cell::text(well);
            mv.variables[1].cells[r] = Cell::text(cmpd);
            mv.variables[2].cells[r] = Cell::number(x);
            mv.variables[3].cells[r] = Cell::number(y);
        }

        let (x_cols, y_col, group_cols) = resolve_mv_columns(&mv, &[], &MultivarExtractionSpec::default()).unwrap();
        assert_eq!(x_cols, vec![2]);
        assert_eq!(y_col, 3);
        assert_eq!(group_cols, vec![1]);

        let ds = DataSheet::new(SheetId::new("mv-4pl"), "MV 4PL", TableArchetype::MultipleVariables(mv.clone())).unwrap();
        let series = extract_datasets_from_sheet(&ds, &[]).unwrap();
        assert_eq!(series.len(), 2);
        assert_eq!(series[0].label, "InhibitorA");
        assert_eq!(series[1].label, "InhibitorB");
        assert_eq!(series[0].points.len(), 3);
        assert_eq!(series[0].points[1].x, vec![-7.0]);
        assert_eq!(series[0].points[1].y, 50.0);

        // Even when selected_data_sets includes Label + Categorical indices, numeric X/Y + grouping still resolve cleanly.
        let (x_all, y_all, g_all) = resolve_mv_columns(&mv, &[0, 1, 2, 3], &MultivarExtractionSpec::default()).unwrap();
        assert_eq!(x_all, vec![2]);
        assert_eq!(y_all, 3);
        assert_eq!(g_all, vec![1]);

        // Trailing Label column and numeric Label tokens must also be excluded.
        let mut mv_trailing = MultipleVariablesTable::new(5, 6).unwrap();
        for c in 0..4 {
            mv_trailing.variables[c] = mv.variables[c].clone();
        }
        mv_trailing.variables[0].cells = (0..6).map(|i| Cell::number((100 + i) as f64)).collect();
        mv_trailing.variables[4].title = String::from("Notes");
        mv_trailing.variables[4].lock_role(VariableRole::Label);
        mv_trailing.variables[4].cells = (0..6).map(|i| Cell::number((900 + i) as f64)).collect();

        let (xt, yt, gt) = resolve_mv_columns(&mv_trailing, &[], &MultivarExtractionSpec::default()).unwrap();
        assert_eq!(xt, vec![2]);
        assert_eq!(yt, 3);
        assert_eq!(gt, vec![1]);

        // Explicit 2-column numeric subset remains ungrouped.
        let (x_sub, y_sub, g_sub) = resolve_mv_columns(&mv, &[2, 3], &MultivarExtractionSpec::default()).unwrap();
        assert_eq!(x_sub, vec![2]);
        assert_eq!(y_sub, 3);
        assert!(g_sub.is_empty());
    }
}
