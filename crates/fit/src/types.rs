//! Core data structures, equation category namespaces, hook resolution, and table extraction
//! helpers for `lumencraft-fit`.

use std::collections::{BTreeMap, HashMap, HashSet};

use lumencraft_doc::{Cell, CellValue, DataSetColumn, DataSheet, HookSource, InfoSheet, ResultSheet, ResultStatus, SheetId, TableArchetype};

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

/// Column selection & multi-factor Cartesian grouping options for `MultipleVariables` tables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MultivarExtractionSpec<'a> {
    pub group_columns: &'a [usize],
    pub x_columns: &'a [usize],
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

fn x_row_is_blank_unknown(xy: &lumencraft_doc::XyTable, row: usize) -> bool {
    match &xy.x_mode {
        lumencraft_doc::XColumnMode::Numbers { .. } | lumencraft_doc::XColumnMode::DateOrElapsedTime { .. } => {
            xy.x_cells.get(row).and_then(|subs| subs.first()).is_some_and(|cell| !cell.excluded && cell.is_empty())
        }
        lumencraft_doc::XColumnMode::GeneratedSequence { .. } => false,
    }
}

fn y_cells_are_blank_unknown(subcells: &[Cell]) -> bool {
    !subcells.is_empty() && subcells.iter().all(|cell| !cell.excluded && cell.is_empty())
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
                        data_set_index: ds_idx,
                        row,
                    });
                }
                (None, Some(pt)) if x_blank => series.unknowns_y.push((row, pt.mean)),
                (Some(x), None) if y_cells_are_blank_unknown(subcells) => series.unknowns_x.push((row, x)),
                _ => {}
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
                    } else if x_blank {
                        series.unknowns_y.push((row, y));
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
                let pt_opt = lumencraft_doc::SummarizedPoint::reconstruct(g.subcolumn_mode, subcells)
                    .map_err(|e| FitError::InvalidParameter { name: "pre_summarized_y", reason: e.to_string() })?;
                if let Some(pt) = pt_opt {
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

fn resolve_mv_columns(
    mv: &lumencraft_doc::MultipleVariablesTable,
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

    let y_col = if let Some(yc) = mv_spec.y_column {
        yc
    } else if mv_spec.x_columns.is_empty() && selected_data_sets.len() >= 2 {
        selected_data_sets[selected_data_sets.len() - 1]
    } else if let Some(&sel) = selected_data_sets.first()
        && !x_set.contains(&sel)
        && !group_set.contains(&sel)
    {
        sel
    } else {
        (0..n_vars)
            .rev()
            .find(|idx| !x_set.contains(idx) && !group_set.contains(idx))
            .unwrap_or_else(|| selected_data_sets.first().copied().unwrap_or(1))
    };

    let x_cols = if !mv_spec.x_columns.is_empty() {
        mv_spec.x_columns.to_vec()
    } else if selected_data_sets.len() >= 2 {
        selected_data_sets[..selected_data_sets.len() - 1].to_vec()
    } else {
        let fallback = (0..n_vars).find(|&idx| idx != y_col && !group_set.contains(&idx)).unwrap_or(usize::from(y_col == 0));
        vec![fallback]
    };

    let x_used: HashSet<usize> = x_cols.iter().copied().collect();
    let group_cols = if !mv_spec.group_columns.is_empty() {
        mv_spec.group_columns.to_vec()
    } else if selected_data_sets.len() < 2 {
        mv.variables
            .iter()
            .enumerate()
            .find(|(idx, v)| *idx != y_col && !x_used.contains(idx) && v.role == lumencraft_doc::VariableRole::Categorical)
            .map(|(idx, _)| vec![idx])
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Ok((x_cols, y_col, group_cols))
}

fn extract_from_multivar(
    mv: &lumencraft_doc::MultipleVariablesTable,
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
        grouped.entry(key_tokens).or_default().push(DataPoint { x: xs, y: y_val, sd: None, n: None, explicit_weight: None, data_set_index: 0, row });
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
