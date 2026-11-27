//! Shared linear-predictor formula compiler (`ModelFormulaSpec` -> dense design matrix and
//! column-to-term span mapping) used by selective-term multifactor ANOVA and reusable by
//! generalized linear models and Cox proportional hazards regression.
//!
//! # References
//!
//! - Wilkinson, G. N., & Rogers, C. E. (1973). Symbolic description of factorial models for analysis
//!   of variance. *Journal of the Royal Statistical Society, Series C (Applied Statistics)*, 22(3),
//!   392–399. `doi:10.2307/2346786`.
//! - Searle, S. R. (1987). *Linear Models for Unbalanced Data*. Wiley.
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.
//! - McCullagh, P., & Nelder, J. A. (1989). *Generalized Linear Models* (2nd ed.). Chapman &
//!   Hall/CRC.

use curvebook_doc::{Cell, CellValue, FormulaTerm, ModelFormulaSpec, VariableColumn, VariableRole};
use curvebook_math::linalg::matrix::MAX_MATRIX_ELEMENTS;
use curvebook_math::linalg::{DenseMatrix, svd};

use crate::error::StatsError;

/// Maximum number of expanded design-matrix columns allowed by [`compile_formula_design`].
pub const MAX_FORMULA_DESIGN_COLS: usize = 4_096;

/// Categorical contrast coding convention used when expanding categorical predictors in a formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoricalCoding {
    /// Sum-to-zero (deviation / effect) coding with the last level equal to `-1` across all
    /// `L - 1` contrast columns, yielding orthogonal Type III ANOVA hypotheses on unweighted means
    /// (Searle 1987; Maxwell & Delaney 2004).
    SumToZero,
    /// Reference (treatment / dummy) coding with level `0` as the omitted baseline and indicator
    /// columns for levels `1..L`, standard for GLMs with intercept and Cox PH regression
    /// (McCullagh & Nelder 1989).
    ReferenceFirst,
    /// Wilkinson–Rogers treatment coding for no-intercept GLMs: behaves identically to
    /// [`Self::ReferenceFirst`] when `include_intercept == true`, and expands the first
    /// categorical main-effect term into all `L` level indicators when `include_intercept == false`
    /// while keeping subsequent categorical terms reference-coded (Wilkinson & Rogers 1973).
    ReferenceFirstNoInterceptFull,
}

/// Predictor column descriptor supplied to [`compile_formula_design`].
#[derive(Debug, Clone, Copy)]
pub enum FormulaPredictorColumn<'a> {
    /// Continuous numeric predictor with finite values (`len == n_rows`).
    Continuous {
        /// Predictor column title.
        name: &'a str,
        /// Per-row finite observations.
        values: &'a [f64],
    },
    /// Categorical factor predictor with zero-based level indices in `0..levels.len()`.
    Categorical {
        /// Factor title.
        name: &'a str,
        /// Ordered distinct level labels.
        levels: &'a [String],
        /// Per-row zero-based level indices (`len == n_rows`).
        level_indices: &'a [usize],
    },
}

impl<'a> FormulaPredictorColumn<'a> {
    /// Display name of this predictor column.
    #[must_use]
    pub const fn name(&self) -> &'a str {
        match self {
            Self::Continuous { name, .. } | Self::Categorical { name, .. } => name,
        }
    }

    /// Number of observation rows in this predictor column.
    #[must_use]
    pub const fn len(&self) -> usize {
        match self {
            Self::Continuous { values, .. } => values.len(),
            Self::Categorical { level_indices, .. } => level_indices.len(),
        }
    }

    /// True when the predictor has zero rows.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Column slice within a [`CompiledDesignMatrix`] belonging to one [`FormulaTerm`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledTermSpan {
    /// Original symbolic term from [`ModelFormulaSpec`].
    pub term: FormulaTerm,
    /// Human-readable term label (e.g. `"Dose"`, `"Drug × Dose"`, `"Age^2"`, `"sqrt(Concentration)"`).
    pub name: String,
    /// Zero-based starting column index inside [`CompiledDesignMatrix::matrix`].
    pub col_start: usize,
    /// Number of contiguous columns occupied by this term.
    pub col_count: usize,
}

/// Compiled design matrix produced by [`compile_formula_design`].
#[derive(Debug, Clone)]
pub struct CompiledDesignMatrix {
    /// Dense `n_rows x n_cols` design matrix.
    pub matrix: DenseMatrix,
    /// Per-column human-readable parameter names (`len == matrix.ncols()`).
    pub column_names: Vec<String>,
    /// Ordered spans mapping each [`FormulaTerm`] to its column range inside `matrix`.
    pub term_spans: Vec<CompiledTermSpan>,
    /// Whether column `0` is an intercept column of ones.
    pub has_intercept: bool,
}

impl CompiledDesignMatrix {
    /// Compute the numerical column rank of the compiled design matrix via singular value
    /// decomposition (`svd`).
    pub fn numerical_rank(&self) -> Result<usize, StatsError> {
        Ok(svd(&self.matrix)?.rank())
    }
}

fn encode_single_column(
    pred: &FormulaPredictorColumn<'_>,
    coding: CategoricalCoding,
    full_indicators: bool,
) -> Result<Vec<(String, Vec<f64>)>, StatsError> {
    match *pred {
        FormulaPredictorColumn::Continuous { name, values } => {
            if let Some(&bad) = values.iter().find(|v| !v.is_finite()) {
                return Err(StatsError::InvalidParameter { name: "formula_predictor", value: bad });
            }
            Ok(vec![(name.to_owned(), values.to_vec())])
        }
        FormulaPredictorColumn::Categorical { name, levels, level_indices } => {
            let n_levels = levels.len();
            if n_levels == 0 || (!full_indicators && n_levels < 2) {
                return Ok(Vec::new());
            }
            let n_rows = level_indices.len();
            for &idx in level_indices {
                if idx >= n_levels {
                    return Err(StatsError::InvalidParameter { name: "categorical_level_index", value: idx as f64 });
                }
            }
            if full_indicators {
                let mut encoded = Vec::with_capacity(n_levels);
                for (k, lvl_name) in levels.iter().enumerate() {
                    let col_name = format!("{name}[{lvl_name}]");
                    let col: Vec<f64> = level_indices.iter().map(|&lvl| if lvl == k { 1.0 } else { 0.0 }).collect();
                    encoded.push((col_name, col));
                }
                return Ok(encoded);
            }
            let n_cols = n_levels - 1;
            let mut encoded = Vec::with_capacity(n_cols);
            match coding {
                CategoricalCoding::SumToZero => {
                    for (k, lvl_name) in levels.iter().enumerate().take(n_cols) {
                        let col_name = format!("{name}[{lvl_name}]");
                        let mut col = Vec::with_capacity(n_rows);
                        for &lvl in level_indices {
                            let val = if lvl == k {
                                1.0
                            } else if lvl == n_levels - 1 {
                                -1.0
                            } else {
                                0.0
                            };
                            col.push(val);
                        }
                        encoded.push((col_name, col));
                    }
                }
                CategoricalCoding::ReferenceFirst | CategoricalCoding::ReferenceFirstNoInterceptFull => {
                    let ref_lvl = &levels[0];
                    for (k, lvl_name) in levels.iter().enumerate().skip(1) {
                        let col_name = format!("{name}[{lvl_name} vs {ref_lvl}]");
                        let col: Vec<f64> = level_indices.iter().map(|&lvl| if lvl == k { 1.0 } else { 0.0 }).collect();
                        encoded.push((col_name, col));
                    }
                }
            }
            Ok(encoded)
        }
    }
}

fn kronecker_columns(left: &[(String, Vec<f64>)], right: &[(String, Vec<f64>)]) -> Result<Vec<(String, Vec<f64>)>, StatsError> {
    let total = left.len().saturating_mul(right.len());
    if total > MAX_FORMULA_DESIGN_COLS {
        return Err(StatsError::InvalidParameter { name: "formula_design_columns", value: total as f64 });
    }
    let mut out = Vec::with_capacity(total);
    for (ln, lv) in left {
        for (rn, rv) in right {
            let prod: Vec<f64> = lv.iter().zip(rv.iter()).map(|(&a, &b)| a * b).collect();
            if let Some(&bad) = prod.iter().find(|v| !v.is_finite()) {
                return Err(StatsError::InvalidParameter { name: "formula_interaction", value: bad });
            }
            out.push((format!("{ln}:{rn}"), prod));
        }
    }
    Ok(out)
}

/// Compile a [`ModelFormulaSpec`] against typed predictor columns into a dense design matrix
/// with deterministic column names and column-to-term span ranges (Wilkinson & Rogers 1973;
/// Searle 1987; McCullagh & Nelder 1989).
pub fn compile_formula_design(
    predictors: &[FormulaPredictorColumn<'_>],
    formula: &ModelFormulaSpec,
    coding: CategoricalCoding,
) -> Result<CompiledDesignMatrix, StatsError> {
    if predictors.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let n_rows = predictors[0].len();
    if n_rows == 0 {
        return Err(StatsError::EmptyInput);
    }
    for p in predictors {
        if p.len() != n_rows {
            return Err(StatsError::DimensionMismatch { expected: n_rows, actual: p.len() });
        }
    }

    let mut cols: Vec<Vec<f64>> = Vec::new();
    let mut column_names: Vec<String> = Vec::new();
    let mut term_spans: Vec<CompiledTermSpan> = Vec::with_capacity(formula.terms.len());

    if formula.include_intercept {
        cols.push(vec![1.0_f64; n_rows]);
        column_names.push(String::from("Intercept"));
    }

    let get_pred =
        |idx: usize| -> Result<&FormulaPredictorColumn<'_>, StatsError> { predictors.get(idx).ok_or(StatsError::MissingColumn { index: idx }) };
    let mut expanded_first_cat_full = false;

    for &term in &formula.terms {
        let col_start = cols.len();
        let (term_name, expanded) = match term {
            FormulaTerm::Main(c) => {
                let p = get_pred(c)?;
                let use_full = !formula.include_intercept
                    && coding == CategoricalCoding::ReferenceFirstNoInterceptFull
                    && !expanded_first_cat_full
                    && matches!(p, FormulaPredictorColumn::Categorical { levels, .. } if levels.len() >= 2);
                if use_full {
                    expanded_first_cat_full = true;
                }
                (p.name().to_owned(), encode_single_column(p, coding, use_full)?)
            }
            FormulaTerm::Interaction2(a, b) => {
                if a == b {
                    return Err(StatsError::InvalidParameter { name: "interaction_duplicate_column", value: a as f64 });
                }
                let pa = get_pred(a)?;
                let pb = get_pred(b)?;
                let ea = encode_single_column(pa, coding, false)?;
                let eb = encode_single_column(pb, coding, false)?;
                (format!("{} × {}", pa.name(), pb.name()), kronecker_columns(&ea, &eb)?)
            }
            FormulaTerm::Interaction3(a, b, c) => {
                if a == b || a == c || b == c {
                    return Err(StatsError::InvalidParameter { name: "interaction_duplicate_column", value: a as f64 });
                }
                let pa = get_pred(a)?;
                let pb = get_pred(b)?;
                let pc = get_pred(c)?;
                let ea = encode_single_column(pa, coding, false)?;
                let eb = encode_single_column(pb, coding, false)?;
                let ec = encode_single_column(pc, coding, false)?;
                let eab = kronecker_columns(&ea, &eb)?;
                (format!("{} × {} × {}", pa.name(), pb.name(), pc.name()), kronecker_columns(&eab, &ec)?)
            }
            FormulaTerm::Power { column, degree } => {
                if degree < 2 {
                    return Err(StatsError::InvalidParameter { name: "power_degree", value: f64::from(degree) });
                }
                let p = get_pred(column)?;
                let FormulaPredictorColumn::Continuous { name, values } = *p else {
                    return Err(StatsError::IncompatibleAnalysisSpec { expected: "continuous predictor column for Power formula term" });
                };
                let exp_i = i32::from(degree);
                let mut powered = Vec::with_capacity(n_rows);
                for &v in values {
                    let pv = v.powi(exp_i);
                    if !pv.is_finite() {
                        return Err(StatsError::InvalidParameter { name: "power_term_value", value: pv });
                    }
                    powered.push(pv);
                }
                let label = format!("{name}^{degree}");
                (label.clone(), vec![(label, powered)])
            }
            FormulaTerm::Sqrt(column) => {
                let p = get_pred(column)?;
                let FormulaPredictorColumn::Continuous { name, values } = *p else {
                    return Err(StatsError::IncompatibleAnalysisSpec { expected: "continuous predictor column for Sqrt formula term" });
                };
                let mut roots = Vec::with_capacity(n_rows);
                for &v in values {
                    if !v.is_finite() || v < 0.0 {
                        return Err(StatsError::InvalidParameter { name: "sqrt_term_value", value: v });
                    }
                    roots.push(v.sqrt());
                }
                let label = format!("sqrt({name})");
                (label.clone(), vec![(label, roots)])
            }
        };

        let col_count = expanded.len();
        let next_total = cols.len().saturating_add(col_count);
        if next_total > MAX_FORMULA_DESIGN_COLS {
            return Err(StatsError::InvalidParameter { name: "formula_design_columns", value: next_total as f64 });
        }
        for (cname, col_vals) in expanded {
            column_names.push(cname);
            cols.push(col_vals);
        }
        term_spans.push(CompiledTermSpan { term, name: term_name, col_start, col_count });
    }

    let n_cols = cols.len();
    if n_cols == 0 {
        return Err(StatsError::EmptyInput);
    }
    let total_elems = n_rows.checked_mul(n_cols).ok_or(StatsError::InvalidParameter { name: "matrix_elements", value: f64::INFINITY })?;
    if total_elems > MAX_MATRIX_ELEMENTS {
        return Err(StatsError::InvalidParameter { name: "matrix_elements", value: total_elems as f64 });
    }
    let mut data = vec![0.0_f64; total_elems];
    for (c, col) in cols.iter().enumerate() {
        for (r, &v) in col.iter().enumerate() {
            data[r * n_cols + c] = v;
        }
    }
    let matrix = DenseMatrix::from_row_major(n_rows, n_cols, data)?;
    Ok(CompiledDesignMatrix { matrix, column_names, term_spans, has_intercept: formula.include_intercept })
}

/// Resolve a [`ModelFormulaSpec`] of categorical factorial terms (`Main`, `Interaction2`,
/// `Interaction3`) into deduplicated local factor-index subsets (`0..factor_count`) for
/// selective-term factorial ANOVA (Searle 1987; Maxwell & Delaney 2004).
///
/// When `source_columns` is non-empty (e.g. `FactorialAnovaSpec::multifactor_columns` on a
/// `MultipleVariablesTable`), each term's column index is matched against `source_columns` first
/// (mapping table column indices to local factor indices `0..source_columns.len()`). When
/// `source_columns` is empty (e.g. `GroupedTable` factorial factors), indices are interpreted
/// directly as local factor indices `0..factor_count`.
pub fn resolve_factorial_formula_subsets(
    formula: &ModelFormulaSpec,
    factor_count: usize,
    source_columns: &[usize],
) -> Result<Vec<Vec<usize>>, StatsError> {
    if formula.terms.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let map_col = |col: usize| -> Result<usize, StatsError> {
        if !source_columns.is_empty() {
            source_columns.iter().position(|&c| c == col).ok_or(StatsError::MissingColumn { index: col })
        } else if col < factor_count {
            Ok(col)
        } else {
            Err(StatsError::MissingColumn { index: col })
        }
    };

    let mut subsets: Vec<Vec<usize>> = Vec::with_capacity(formula.terms.len());
    for &term in &formula.terms {
        let mut sub = match term {
            FormulaTerm::Main(a) => vec![map_col(a)?],
            FormulaTerm::Interaction2(a, b) => {
                let fa = map_col(a)?;
                let fb = map_col(b)?;
                if fa == fb {
                    return Err(StatsError::InvalidParameter { name: "interaction_duplicate_factor", value: a as f64 });
                }
                vec![fa, fb]
            }
            FormulaTerm::Interaction3(a, b, c) => {
                let fa = map_col(a)?;
                let fb = map_col(b)?;
                let fc = map_col(c)?;
                if fa == fb || fa == fc || fb == fc {
                    return Err(StatsError::InvalidParameter { name: "interaction_duplicate_factor", value: a as f64 });
                }
                vec![fa, fb, fc]
            }
            FormulaTerm::Power { .. } | FormulaTerm::Sqrt(_) => {
                return Err(StatsError::IncompatibleAnalysisSpec {
                    expected: "categorical Main, Interaction2, or Interaction3 terms for factorial ANOVA formula",
                });
            }
        };
        sub.sort_unstable();
        if !subsets.contains(&sub) {
            subsets.push(sub);
        }
    }
    Ok(subsets)
}

/// Remap column indices inside a [`ModelFormulaSpec`] into local `0..source_columns.len()`
/// indices against `source_columns` (e.g. `GlmSpec::predictor_columns` or
/// `CoxProportionalHazards::predictor_columns`).
///
/// If every referenced column index exists inside `source_columns`, indices are interpreted as
/// table column indices and mapped to their position in `source_columns`. Otherwise, if every
/// referenced index is already in `0..source_columns.len()`, local indices are accepted directly.
pub fn remap_formula_columns(formula: &ModelFormulaSpec, source_columns: &[usize]) -> Result<ModelFormulaSpec, StatsError> {
    if source_columns.is_empty() || formula.terms.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let referenced = collect_formula_columns(formula);
    let all_in_source = referenced.iter().all(|c| source_columns.contains(c));
    let all_local = referenced.iter().all(|&c| c < source_columns.len());
    if !all_in_source && !all_local {
        let bad = referenced.into_iter().find(|c| !source_columns.contains(c)).unwrap_or(0);
        return Err(StatsError::MissingColumn { index: bad });
    }
    let map_idx = |c: usize| -> Result<usize, StatsError> {
        if all_in_source { source_columns.iter().position(|&sc| sc == c).ok_or(StatsError::MissingColumn { index: c }) } else { Ok(c) }
    };
    let mut terms = Vec::with_capacity(formula.terms.len());
    for &t in &formula.terms {
        let mapped = match t {
            FormulaTerm::Main(a) => FormulaTerm::Main(map_idx(a)?),
            FormulaTerm::Interaction2(a, b) => FormulaTerm::Interaction2(map_idx(a)?, map_idx(b)?),
            FormulaTerm::Interaction3(a, b, c) => FormulaTerm::Interaction3(map_idx(a)?, map_idx(b)?, map_idx(c)?),
            FormulaTerm::Power { column, degree } => FormulaTerm::Power { column: map_idx(column)?, degree },
            FormulaTerm::Sqrt(a) => FormulaTerm::Sqrt(map_idx(a)?),
        };
        terms.push(mapped);
    }
    Ok(ModelFormulaSpec { include_intercept: formula.include_intercept, terms })
}

/// Collect the deduplicated ordered list of predictor column indices referenced by a [`ModelFormulaSpec`].
#[must_use]
pub fn collect_formula_columns(formula: &ModelFormulaSpec) -> Vec<usize> {
    let mut cols = Vec::new();
    let mut push_unique = |c: usize| {
        if !cols.contains(&c) {
            cols.push(c);
        }
    };
    for &t in &formula.terms {
        match t {
            FormulaTerm::Main(a) | FormulaTerm::Power { column: a, .. } | FormulaTerm::Sqrt(a) => push_unique(a),
            FormulaTerm::Interaction2(a, b) => {
                push_unique(a);
                push_unique(b);
            }
            FormulaTerm::Interaction3(a, b, c) => {
                push_unique(a);
                push_unique(b);
                push_unique(c);
            }
        }
    }
    cols
}

/// Binary outcome encoding resolved from a numeric (`0`/`1`) or 2-level text variable column
/// for logistic regression.
#[derive(Debug, Clone, PartialEq)]
pub struct BinaryOutcomeEncoding {
    /// Per-row binary value (`Some(0.0)` or `Some(1.0)`), `None` if cell is blank, excluded, or invalid.
    pub values: Vec<Option<f64>>,
    /// Label representing `y = 0` (reference/negative class).
    pub negative_label: String,
    /// Label representing `y = 1` (event/positive class).
    pub positive_label: String,
    /// True when the column was encoded from 2-level text labels rather than numeric `0`/`1`.
    pub from_text: bool,
}

/// Encode a `VariableColumn` as a binary `{0.0, 1.0}` logistic outcome, supporting both numeric
/// `{0, 1}` columns and 2-level categorical/text columns (ordered via `level_order`, with level 0
/// mapped to `0.0` and level 1 mapped to `1.0`).
pub fn encode_binary_outcome_column(var: &VariableColumn, row_count: usize) -> Result<BinaryOutcomeEncoding, StatsError> {
    let mut has_text = false;
    let mut has_num = false;
    for r in 0..row_count {
        if let Some(cell) = var.cells.get(r)
            && !cell.excluded
        {
            match &cell.value {
                CellValue::Text(s) if !s.trim().is_empty() => has_text = true,
                CellValue::Number(v) if v.is_finite() => has_num = true,
                _ => {}
            }
        }
    }
    if var.role == VariableRole::Categorical || (has_text && !has_num) {
        let mut active_cells = Vec::new();
        for r in 0..row_count {
            if let Some(cell) = var.cells.get(r)
                && !cell.excluded
            {
                match &cell.value {
                    CellValue::Text(s) if !s.trim().is_empty() => active_cells.push(Cell::text(s.trim())),
                    CellValue::Number(v) if v.is_finite() => active_cells.push(Cell::text(format!("{v}"))),
                    _ => {}
                }
            }
        }
        let temp_col = VariableColumn {
            title: var.title.clone(),
            role: VariableRole::Categorical,
            role_inferred: false,
            source: var.source.clone(),
            level_order: var.level_order.clone(),
            cells: active_cells,
            display_decimals: None,
        };
        let levels: Vec<String> = temp_col.ordered_levels_with(&var.level_order).into_iter().map(|s| s.level).collect();
        if levels.len() != 2 {
            return Err(StatsError::InvalidParameter { name: "logistic_text_levels", value: levels.len() as f64 });
        }
        let neg_lbl = levels[0].clone();
        let pos_lbl = levels[1].clone();
        let mut values = Vec::with_capacity(row_count);
        for r in 0..row_count {
            let val = var.cells.get(r).and_then(|c| {
                if c.excluded {
                    return None;
                }
                let tok = match &c.value {
                    CellValue::Text(s) => {
                        let t = s.trim();
                        if t.is_empty() {
                            return None;
                        }
                        t.to_owned()
                    }
                    CellValue::Number(v) if v.is_finite() => format!("{v}"),
                    _ => return None,
                };
                if tok == neg_lbl {
                    Some(0.0)
                } else if tok == pos_lbl {
                    Some(1.0)
                } else {
                    None
                }
            });
            values.push(val);
        }
        return Ok(BinaryOutcomeEncoding { values, negative_label: neg_lbl, positive_label: pos_lbl, from_text: has_text || !has_num });
    }
    let mut values = Vec::with_capacity(row_count);
    for r in 0..row_count {
        values.push(var.cells.get(r).and_then(Cell::active_number));
    }
    Ok(BinaryOutcomeEncoding { values, negative_label: String::from("0"), positive_label: String::from("1"), from_text: false })
}
