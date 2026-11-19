//! Core data structures, equation category namespaces, hook resolution, and table extraction
//! helpers for `curvebook-fit`.

use std::collections::{BTreeMap, HashMap, HashSet};

use curvebook_doc::{
    Cell, CellValue, DataSetColumn, DataSheet, HookSource, InfoSheet, ResultSheet, ResultStatus, SheetId, TableArchetype, VariableColumn,
    VariableRole, parse_column_title_numeric_hook,
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
    match &sheet.archetype {
        TableArchetype::Xy(xy) => extract_from_xy(xy, selected_data_sets),
        TableArchetype::Grouped(g) => extract_from_grouped(g, selected_data_sets),
        TableArchetype::MultipleVariables(mv) => extract_from_multivar(mv, selected_data_sets, mv_spec),
        other => Err(FitError::IncompatibleTable { kind: other.kind() }),
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

fn resolve_selected_indices(total: usize, selected: &[usize]) -> Vec<usize> {
    if selected.is_empty() { (0..total).collect() } else { selected.iter().copied().filter(|&i| i < total).collect() }
}

fn extract_from_xy(xy: &curvebook_doc::XyTable, selected_data_sets: &[usize]) -> Result<Vec<DatasetSeries>, FitError> {
    let indices = resolve_selected_indices(xy.data_sets.len(), selected_data_sets);
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
            xy.x_cells.get(row).and_then(|subs| subs.first()).is_some_and(|cell| !cell.excluded && cell.is_empty())
        }
        curvebook_doc::XColumnMode::GeneratedSequence { .. } => false,
    }
}

fn y_cells_are_blank_unknown(subcells: &[Cell]) -> bool {
    !subcells.is_empty() && subcells.iter().all(|cell| !cell.excluded && cell.is_empty())
}

fn populate_xy_column(xy: &curvebook_doc::XyTable, ds: &DataSetColumn, ds_idx: usize, series: &mut DatasetSeries) -> Result<(), FitError> {
    let x_sd = |row: usize| -> Option<f64> {
        let curvebook_doc::XColumnMode::Numbers { subcolumn_mode } = xy.x_mode else {
            return None;
        };
        if !subcolumn_mode.supports_parametric_moments() {
            return None;
        }
        xy.summarized_x_point(row).ok().flatten().map(|pt| pt.sd)
    };
    let _ = x_sd;

    for (row, subcells) in ds.cells.iter().enumerate() {
        let x_opt = xy.x_value_at(row);
        let x_blank = x_opt.is_none() && x_row_is_blank_unknown(xy, row);
        // Excluded (or non-blank non-numeric) X rows must be dropped outright rather than
        // masquerading as blank X cells queued for inverse interpolation.
        if x_opt.is_none() && !x_blank {
            continue;
        }

        if xy.y_subcolumn_mode.supports_parametric_moments() {
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
                (None, Some(pt)) if x_blank => series.unknowns_y.push((row, pt.mean).into()),
                (Some(x), None) if y_cells_are_blank_unknown(subcells) => series.unknowns_x.push((row, x)),
                _ => {}
            }
        } else if xy.y_subcolumn_mode.is_pre_summarized() {
            if let (Some(x), Some(y)) = (x_opt, subcells.first().and_then(Cell::active_number)) {
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
    let indices = resolve_selected_indices(g.data_sets.len(), selected_data_sets);
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

pub(crate) fn resolve_mv_columns(
    mv: &curvebook_doc::MultipleVariablesTable,
    selected_data_sets: &[usize],
    mv_spec: &MultivarExtractionSpec<'_>,
) -> Result<(Vec<usize>, usize, Vec<usize>), FitError> {
    let n_vars = mv.variables.len();
    for &gc in mv_spec.group_columns {
        if gc >= n_vars {
            return Err(FitError::InvalidParameter { name: "mv_group_columns", reason: format!("Column index {gc} >= {n_vars}") });
        }
    }
    for &xc in mv_spec.x_columns {
        if xc >= n_vars {
            return Err(FitError::InvalidParameter { name: "mv_x_columns", reason: format!("Column index {xc} >= {n_vars}") });
        }
    }
    if let Some(yc) = mv_spec.y_column
        && yc >= n_vars
    {
        return Err(FitError::InvalidParameter { name: "mv_y_column", reason: format!("Column index {yc} >= {n_vars}") });
    }

    let group_set: HashSet<usize> = mv_spec.group_columns.iter().copied().collect();
    let x_set: HashSet<usize> = mv_spec.x_columns.iter().copied().collect();
    let selected_numeric: Vec<usize> = selected_data_sets
        .iter()
        .copied()
        .filter(|&idx| idx < n_vars && !group_set.contains(&idx) && mv.variables.get(idx).is_some_and(is_mv_numeric_role))
        .collect();

    let y_col = if let Some(yc) = mv_spec.y_column {
        yc
    } else if let Some(&sel) = selected_numeric.iter().rev().find(|idx| !x_set.contains(idx)) {
        sel
    } else {
        pick_default_mv_numeric_col(mv, (0..n_vars).rev().filter(|idx| !x_set.contains(idx) && !group_set.contains(idx)))
            .or_else(|| (0..n_vars).rev().find(|idx| !x_set.contains(idx) && !group_set.contains(idx)))
            .unwrap_or_else(|| selected_data_sets.first().copied().unwrap_or(1))
    };

    let selected_x: Vec<usize> = selected_numeric.iter().copied().filter(|&idx| idx != y_col).collect();
    let x_cols = if !mv_spec.x_columns.is_empty() {
        mv_spec.x_columns.to_vec()
    } else if !selected_x.is_empty() {
        selected_x
    } else {
        let fallback = pick_default_mv_numeric_col(mv, (0..n_vars).filter(|&idx| idx != y_col && !group_set.contains(&idx)))
            .or_else(|| (0..n_vars).find(|&idx| idx != y_col && !group_set.contains(&idx)))
            .unwrap_or(usize::from(y_col == 0));
        vec![fallback]
    };

    let x_used: HashSet<usize> = x_cols.iter().copied().collect();
    let selected_categorical: Vec<usize> = selected_data_sets
        .iter()
        .copied()
        .filter(|&idx| {
            idx < n_vars && idx != y_col && !x_used.contains(&idx) && mv.variables.get(idx).is_some_and(|v| v.role == VariableRole::Categorical)
        })
        .collect();
    let group_cols = if !mv_spec.group_columns.is_empty() {
        mv_spec.group_columns.to_vec()
    } else if !selected_categorical.is_empty() {
        selected_categorical
    } else if selected_data_sets.len() < 2 {
        mv.variables
            .iter()
            .enumerate()
            .find(|(idx, v)| *idx != y_col && !x_used.contains(idx) && v.role == VariableRole::Categorical)
            .map(|(idx, _)| vec![idx])
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok((x_cols, y_col, group_cols))
}

fn extract_from_multivar(
    mv: &curvebook_doc::MultipleVariablesTable,
    selected_data_sets: &[usize],
    mv_spec: &MultivarExtractionSpec<'_>,
) -> Result<Vec<DatasetSeries>, FitError> {
    if mv.variables.len() < 2 {
        return Err(FitError::InsufficientData { required: 2, actual: mv.variables.len() });
    }
    let (x_cols, y_col, group_cols) = resolve_mv_columns(mv, selected_data_sets, mv_spec)?;

    let Some(y_var) = mv.variables.get(y_col) else {
        return Err(FitError::EmptyInput);
    };
    let n_rows = mv.row_count();
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

    let level_ranks: Vec<HashMap<String, usize>> = group_cols
        .iter()
        .filter_map(|&gc| mv.variables.get(gc))
        .map(|var| var.ordered_levels().into_iter().enumerate().map(|(i, lvl)| (lvl.level, i)).collect())
        .collect();

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

    let explicit_column_mapping = !mv_spec.x_columns.is_empty() && mv_spec.y_column.is_some();
    let keep_indices = if explicit_column_mapping && !group_cols.is_empty() && !selected_data_sets.is_empty() {
        resolve_selected_indices(entries.len(), selected_data_sets)
    } else {
        (0..entries.len()).collect()
    };

    let mut indexed: Vec<Option<(Vec<String>, Vec<DataPoint>)>> = entries.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(keep_indices.len());
    for idx in keep_indices {
        let Some((tokens, mut pts)) = indexed.get_mut(idx).and_then(Option::take) else {
            continue;
        };
        for p in &mut pts {
            p.data_set_index = idx;
        }
        let label = if tokens.is_empty() { y_var.title.clone() } else { tokens.join(" | ") };
        let title_val = tokens.first().and_then(|t| parse_leading_number(t)).or_else(|| parse_leading_number(&label));
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
    Ok(out)
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
