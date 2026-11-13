//! Core data structures, equation category namespaces, hook resolution, and table extraction
//! helpers for `lumencraft-fit`.

use std::collections::{BTreeMap, HashSet};

use lumencraft_doc::{Cell, DataSetColumn, DataSheet, HookSource, InfoSheet, ResultSheet, SheetId, TableArchetype};

use crate::error::FitError;

/// Built-in equation library namespaces for nonlinear regression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EquationCategory {
    DoseResponseStimulation,
    DoseResponseInhibition,
    ReceptorBinding,
    EnzymeKinetics,
    Exponential,
    Polynomial,
    Gaussian,
    SineWave,
    Growth,
    Peak,
    StandardCurve,
    Radiobiology,
    NistBenchmark,
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
    /// Originating dataset column index (`0 = A`, `1 = B`, ...).
    pub data_set_index: usize,
    /// Originating table row index.
    pub row: usize,
}

impl DataPoint {
    /// Construct a simple unweighted 1D `(x, y)` observation.
    #[must_use]
    pub fn xy(x: f64, y: f64) -> Self {
        Self { x: vec![x], y, sd: None, n: None, explicit_weight: None, data_set_index: 0, row: 0 }
    }

    /// Construct a multivariate `(predictors, y)` observation.
    #[must_use]
    pub fn multi(predictors: Vec<f64>, y: f64) -> Self {
        Self { x: predictors, y, sd: None, n: None, explicit_weight: None, data_set_index: 0, row: 0 }
    }

    /// Primary scalar predictor `x[0]`.
    #[must_use]
    pub fn x0(&self) -> f64 {
        self.x.first().copied().unwrap_or(0.0)
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
    pub unknowns_y: Vec<(usize, f64)>,
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
    pub name: String,
    pub value: f64,
    pub units: Option<String>,
}

/// Runtime context for resolving [`HookSource`] references (`InfoConstant`, `ResultScalar`,
/// `ColumnTitleValue`) without cyclic references.
#[derive(Debug, Clone, Default)]
pub struct HookContext<'a> {
    pub info_sheets: &'a [InfoSheet],
    pub result_sheets: &'a [ResultSheet],
    pub data_sheets: &'a [DataSheet],
    pub primary_info_sheet_id: Option<&'a SheetId>,
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
        parse_leading_number(title)
            .ok_or_else(|| FitError::HookResolutionError { message: format!("column title `{title}` does not contain a finite numeric value") })
    }
}

/// Extract the first finite numeric token from a column title (e.g. `"[I] = 10 nM"` -> `10.0`).
#[must_use]
pub fn parse_leading_number(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if let Ok(v) = trimmed.parse::<f64>()
        && v.is_finite()
    {
        return Some(v);
    }
    for tok in trimmed.split(|c: char| c.is_whitespace() || c == '=' || c == ':' || c == ',') {
        let clean = tok.trim_matches(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+' && c != 'e' && c != 'E');
        if !clean.is_empty()
            && let Ok(v) = clean.parse::<f64>()
            && v.is_finite()
        {
            return Some(v);
        }
    }
    None
}

/// Extract regression datasets from a [`DataSheet`] (`Xy`, `Grouped`, or `MultipleVariables`).
///
/// # Errors
/// Returns [`FitError::IncompatibleTable`] for unsupported table archetypes or [`FitError::EmptyInput`]
/// if no active finite observations exist across the selected data sets.
pub fn extract_datasets_from_sheet(sheet: &DataSheet, selected_data_sets: &[usize]) -> Result<Vec<DatasetSeries>, FitError> {
    match &sheet.archetype {
        TableArchetype::Xy(xy) => extract_from_xy(xy, selected_data_sets),
        TableArchetype::Grouped(g) => extract_from_grouped(g, selected_data_sets),
        TableArchetype::MultipleVariables(mv) => extract_from_multivar(mv, selected_data_sets),
        other => Err(FitError::IncompatibleTable { kind: other.kind() }),
    }
}

fn resolve_selected_indices(total: usize, selected: &[usize]) -> Vec<usize> {
    if selected.is_empty() { (0..total).collect() } else { selected.iter().copied().filter(|&i| i < total).collect() }
}

fn extract_from_xy(xy: &lumencraft_doc::XyTable, selected_data_sets: &[usize]) -> Result<Vec<DatasetSeries>, FitError> {
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

fn populate_xy_column(xy: &lumencraft_doc::XyTable, ds: &DataSetColumn, ds_idx: usize, series: &mut DatasetSeries) -> Result<(), FitError> {
    let x_sd = |row: usize| -> Option<f64> {
        let lumencraft_doc::XColumnMode::Numbers { subcolumn_mode } = xy.x_mode else {
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
                        data_set_index: ds_idx,
                        row,
                    });
                }
                (None, Some(pt)) => series.unknowns_y.push((row, pt.mean)),
                (Some(x), None) => series.unknowns_x.push((row, x)),
                (None, None) => {}
            }
        } else if xy.y_subcolumn_mode.is_pre_summarized() {
            if let (Some(x), Some(y)) = (x_opt, subcells.first().and_then(Cell::active_number)) {
                series.points.push(DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, data_set_index: ds_idx, row });
            }
        } else {
            let mut had_y = false;
            for cell in subcells {
                if let Some(y) = cell.active_number() {
                    had_y = true;
                    if let Some(x) = x_opt {
                        series.points.push(DataPoint { x: vec![x], y, sd: None, n: None, explicit_weight: None, data_set_index: ds_idx, row });
                    } else {
                        series.unknowns_y.push((row, y));
                    }
                }
            }
            if !had_y && let Some(x) = x_opt {
                series.unknowns_x.push((row, x));
            }
        }
    }
    Ok(())
}

fn extract_from_grouped(g: &lumencraft_doc::GroupedTable, selected_data_sets: &[usize]) -> Result<Vec<DatasetSeries>, FitError> {
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
                if let Ok(Some(pt)) = lumencraft_doc::SummarizedPoint::reconstruct(g.subcolumn_mode, subcells) {
                    points.push(DataPoint {
                        x: vec![x_val],
                        y: pt.mean,
                        sd: Some(pt.sd),
                        n: Some(pt.n),
                        explicit_weight: None,
                        data_set_index: ds_idx,
                        row,
                    });
                }
            } else {
                for cell in subcells {
                    if let Some(y) = cell.active_number() {
                        points.push(DataPoint { x: vec![x_val], y, sd: None, n: None, explicit_weight: None, data_set_index: ds_idx, row });
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

fn extract_from_multivar(mv: &lumencraft_doc::MultipleVariablesTable, selected_data_sets: &[usize]) -> Result<Vec<DatasetSeries>, FitError> {
    // Convention: selected_data_sets if length >= 2 gives [x_col..., y_col], or empty uses col 0 as X and col 1 as Y.
    if mv.variables.len() < 2 {
        return Err(FitError::InsufficientData { required: 2, actual: mv.variables.len() });
    }
    let (x_cols, y_col, group_col): (Vec<usize>, usize, Option<usize>) = if selected_data_sets.len() >= 2 {
        let last = selected_data_sets.len() - 1;
        (selected_data_sets[..last].to_vec(), selected_data_sets[last], None)
    } else {
        let y_idx = selected_data_sets.first().copied().unwrap_or(1);
        let x_idx = if y_idx == 0 { 1 } else { 0 };
        let grp = mv
            .variables
            .iter()
            .enumerate()
            .find(|(idx, v)| *idx != x_idx && *idx != y_idx && v.role == lumencraft_doc::VariableRole::Categorical)
            .map(|(idx, _)| idx);
        (vec![x_idx], y_idx, grp)
    };

    let Some(y_var) = mv.variables.get(y_col) else {
        return Err(FitError::EmptyInput);
    };
    let n_rows = mv.row_count();
    let mut grouped: BTreeMap<String, Vec<DataPoint>> = BTreeMap::new();
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
        let label = group_col
            .and_then(|gc| mv.variables.get(gc))
            .and_then(|v| v.cells.get(row))
            .and_then(Cell::active_text)
            .unwrap_or(&y_var.title)
            .to_owned();
        grouped.entry(label).or_default().push(DataPoint { x: xs, y: y_val, sd: None, n: None, explicit_weight: None, data_set_index: 0, row });
    }

    let mut out = Vec::with_capacity(grouped.len());
    for (idx, (label, mut pts)) in grouped.into_iter().enumerate() {
        for p in &mut pts {
            p.data_set_index = idx;
        }
        out.push(DatasetSeries {
            data_set_index: idx,
            label: label.clone(),
            points: pts,
            unknowns_y: Vec::new(),
            unknowns_x: Vec::new(),
            is_pre_summarized: false,
            column_title_value: parse_leading_number(&label),
        });
    }
    if out.is_empty() {
        return Err(FitError::EmptyInput);
    }
    Ok(out)
}
