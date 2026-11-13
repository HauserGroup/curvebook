//! Clean-room ASCII `.dat` parser for all four NIST Statistical Reference Datasets (StRD) suites:
//! 1. Univariate summary statistics (`parse_univariate_dat`),
//! 2. One-way ANOVA (`parse_anova_dat`),
//! 3. Linear regression (`parse_linear_regression_dat`),
//! 4. Nonlinear least-squares regression (`parse_nonlinear_dat`).
//!
//! Reference:
//! - NIST Information Technology Laboratory, Statistical Engineering Division:
//!   *Statistical Reference Datasets (StRD)*, <https://www.itl.nist.gov/div898/strd/> (public domain).

use std::fmt;

/// Maximum input payload accepted by the NIST StRD ASCII parser (`16 MiB`).
pub const MAX_STRD_BYTES: usize = 16 * 1024 * 1024;

/// Maximum number of observation rows accepted in a single NIST StRD `.dat` file.
pub const MAX_STRD_ROWS: usize = 1_000_000;

/// NIST StRD numerical stiffness / ill-conditioning difficulty tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NistDifficulty {
    Lower,
    Average,
    Higher,
}

/// Errors returned when parsing a NIST StRD `.dat` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrdParseError {
    MissingField(&'static str),
    InvalidNumber { context: &'static str, token: String },
    EmptyDataSection,
    MalformedDataRow { line_number: usize, line: String },
    InputTooLarge { kind: &'static str, actual: usize, max: usize },
}

impl fmt::Display for StrdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing NIST StRD field: {field}"),
            Self::InvalidNumber { context, token } => {
                write!(f, "invalid numeric token '{token}' while parsing {context}")
            }
            Self::EmptyDataSection => write!(f, "NIST StRD Data section contains no observations"),
            Self::MalformedDataRow { line_number, line } => {
                write!(f, "malformed NIST StRD data row at line {line_number}: {line}")
            }
            Self::InputTooLarge { kind, actual, max } => {
                write!(f, "NIST StRD {kind} ({actual}) exceeds maximum bound ({max})")
            }
        }
    }
}

impl std::error::Error for StrdParseError {}

/// NIST StRD Univariate Summary Statistics dataset (`NumAcc1`-`NumAcc4`, `Lew`, `Lottery`, `Mavro`, `Michelso`, `PiDigits`).
#[derive(Debug, Clone)]
pub struct UnivariateDataset {
    pub name: String,
    pub difficulty: NistDifficulty,
    pub certified_mean: f64,
    pub certified_sd: f64,
    pub certified_r1: f64,
    pub values: Vec<f64>,
}

/// NIST StRD One-Way Analysis of Variance dataset (`SiRstv`, `AtmWtAg`, `SmLs01`-`SmLs09`).
#[derive(Debug, Clone)]
pub struct AnovaDataset {
    pub name: String,
    pub difficulty: NistDifficulty,
    pub certified_df_between: usize,
    pub certified_df_within: usize,
    pub certified_ss_between: f64,
    pub certified_ss_within: f64,
    pub certified_f: f64,
    pub certified_r_squared: f64,
    pub certified_residual_sd: f64,
    /// `(group_label, response)` rows in file order.
    pub observations: Vec<(usize, f64)>,
}

/// NIST StRD Linear Least-Squares dataset (`Norris`, `Pontius`, `NoInt1`, `NoInt2`, `Filip`, `Longley`, `Wampler1`-`Wampler5`).
#[derive(Debug, Clone)]
pub struct LinearRegressionDataset {
    pub name: String,
    pub difficulty: NistDifficulty,
    pub has_intercept: bool,
    pub certified_betas: Vec<f64>,
    pub certified_se_betas: Vec<f64>,
    pub certified_residual_sd: f64,
    pub certified_r_squared: f64,
    pub certified_df_reg: usize,
    pub certified_df_res: usize,
    pub certified_ss_reg: f64,
    pub certified_ss_res: f64,
    pub certified_f: Option<f64>,
    pub y: Vec<f64>,
    /// Expanded design matrix rows matching `certified_betas` column-for-column.
    pub design_rows: Vec<Vec<f64>>,
}

/// NIST StRD Nonlinear Least-Squares dataset (`Misra1a`, `Eckerle4`, `Rat43`, `Thurber`, `Bennett5`, `Nelson`, etc.).
#[derive(Debug, Clone)]
pub struct NonlinearDataset {
    pub name: String,
    pub difficulty: NistDifficulty,
    pub param_names: Vec<String>,
    pub start_1: Vec<f64>,
    pub start_2: Vec<f64>,
    pub certified_betas: Vec<f64>,
    pub certified_se_betas: Vec<f64>,
    pub certified_rss: f64,
    pub certified_residual_sd: f64,
    pub certified_df: usize,
    pub y: Vec<f64>,
    pub predictors: Vec<Vec<f64>>,
}

fn normalize_fortran_float(token: &str) -> String {
    if token.contains('D') || token.contains('d') { token.replace(['D', 'd'], "E") } else { token.to_owned() }
}

fn parse_f64_token(token: &str, context: &'static str) -> Result<f64, StrdParseError> {
    let cleaned = normalize_fortran_float(token.trim());
    let value = cleaned.parse::<f64>().map_err(|_| StrdParseError::InvalidNumber { context, token: token.to_owned() })?;
    if !value.is_finite() {
        return Err(StrdParseError::InvalidNumber { context, token: token.to_owned() });
    }
    Ok(value)
}

fn parse_usize_token(token: &str, context: &'static str) -> Result<usize, StrdParseError> {
    token.trim().parse::<usize>().map_err(|_| StrdParseError::InvalidNumber { context, token: token.to_owned() })
}

fn parse_positive_group_token(token: &str, context: &'static str) -> Result<usize, StrdParseError> {
    let trimmed = token.trim();
    if let Ok(group) = trimmed.parse::<usize>() {
        if group >= 1 {
            return Ok(group);
        }
        return Err(StrdParseError::InvalidNumber { context, token: token.to_owned() });
    }
    // Accept exact positive integer floats (e.g. `1.0`, `2.0000000E+00`) while rejecting <= 0, fractional, NaN, or Inf.
    const MAX_EXACT_F64_INT: f64 = 9_007_199_254_740_992.0; // 2^53
    let value = parse_f64_token(trimmed, context)?;
    if !(1.0..=MAX_EXACT_F64_INT).contains(&value) || value.fract() != 0.0 {
        return Err(StrdParseError::InvalidNumber { context, token: token.to_owned() });
    }
    usize::try_from(value as u64).map_err(|_| StrdParseError::InvalidNumber { context, token: token.to_owned() })
}

fn extract_dataset_name(header: &str) -> Option<String> {
    for line in header.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Dataset Name:") {
            let name = rest.split_whitespace().next()?;
            return Some(name.to_owned());
        }
    }
    None
}

fn extract_difficulty(header: &str) -> NistDifficulty {
    for line in header.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("level of difficulty") {
            if lower.contains("higher") {
                return NistDifficulty::Higher;
            }
            if lower.contains("average") {
                return NistDifficulty::Average;
            }
            return NistDifficulty::Lower;
        }
    }
    NistDifficulty::Lower
}

type NumberedLine<'a> = (usize, &'a str);
type SplitSections<'a> = (String, Vec<NumberedLine<'a>>);

fn is_data_section_header(trimmed_lower: &str) -> bool {
    let Some(rest) = trimmed_lower.strip_prefix("data:") else {
        return false;
    };
    let rest = rest.trim();
    if rest.is_empty() {
        return true;
    }
    // Header Table-of-Contents entries look like `Data: (lines 61 to 81)` or `Data: y, x (line 61)`.
    !rest.contains("lines ") && !rest.contains("(lines") && !rest.contains("(line") && !rest.starts_with("line ")
}

fn is_data_row_candidate(trimmed: &str) -> bool {
    let Some(first) = trimmed.as_bytes().first() else {
        return false;
    };
    if !(first.is_ascii_digit() || *first == b'-' || *first == b'+' || *first == b'.') {
        return false;
    }
    // Skip decorative separator lines such as `----------` under `Data: Y` in univariate `.dat` files.
    !trimmed.bytes().all(|b| b == b'-')
}

/// Split a NIST `.dat` file into `(header_lines, data_lines_with_1based_line_numbers)`.
fn split_header_and_data(text: &str) -> Result<SplitSections<'_>, StrdParseError> {
    if text.len() > MAX_STRD_BYTES {
        return Err(StrdParseError::InputTooLarge { kind: "input bytes", actual: text.len(), max: MAX_STRD_BYTES });
    }

    let lines: Vec<&str> = text.lines().collect();
    let mut last_data_idx = None;
    for (idx, line) in lines.iter().enumerate() {
        let lower = line.trim().to_ascii_lowercase();
        if is_data_section_header(&lower) {
            last_data_idx = Some(idx);
        }
    }
    let Some(data_header_idx) = last_data_idx else {
        return Err(StrdParseError::EmptyDataSection);
    };

    let mut header = String::new();
    for line in lines.get(..data_header_idx).unwrap_or(&[]) {
        header.push_str(line);
        header.push('\n');
    }

    let mut data_rows = Vec::new();
    for (offset, line) in lines.get(data_header_idx + 1..).unwrap_or(&[]).iter().enumerate() {
        let trimmed = line.trim();
        if is_data_row_candidate(trimmed) {
            if data_rows.len() >= MAX_STRD_ROWS {
                return Err(StrdParseError::InputTooLarge { kind: "data rows", actual: data_rows.len().saturating_add(1), max: MAX_STRD_ROWS });
            }
            data_rows.push((data_header_idx + 2 + offset, trimmed));
        }
    }

    if data_rows.is_empty() {
        return Err(StrdParseError::EmptyDataSection);
    }
    Ok((header, data_rows))
}

fn find_colon_float(header: &str, needle_lower: &str, context: &'static str) -> Result<f64, StrdParseError> {
    for line in header.lines() {
        if line.to_ascii_lowercase().contains(needle_lower)
            && let Some((_, rhs)) = line.rsplit_once(':')
            && let Some(tok) = rhs.split_whitespace().next()
        {
            return parse_f64_token(tok, context);
        }
    }
    Err(StrdParseError::MissingField(context))
}

/// Parse a NIST StRD Univariate Summary Statistics `.dat` file.
///
/// # Errors
/// Returns [`StrdParseError`] if certified fields or numeric observations are missing or malformed.
pub fn parse_univariate_dat(text: &str) -> Result<UnivariateDataset, StrdParseError> {
    let (header, data_rows) = split_header_and_data(text)?;
    let name = extract_dataset_name(&header).ok_or(StrdParseError::MissingField("Dataset Name"))?;
    let difficulty = extract_difficulty(&header);
    let certified_mean = find_colon_float(&header, "sample mean", "Sample Mean")?;
    let certified_sd = find_colon_float(&header, "sample standard deviation", "Sample Standard Deviation")?;
    let certified_r1 = find_colon_float(&header, "sample autocorrelation", "Sample Autocorrelation Coefficient")?;

    let mut values = Vec::with_capacity(data_rows.len());
    for (line_number, row) in data_rows {
        let Some(tok) = row.split_whitespace().next() else {
            return Err(StrdParseError::MalformedDataRow { line_number, line: row.to_owned() });
        };
        values.push(parse_f64_token(tok, "univariate observation")?);
    }

    Ok(UnivariateDataset { name, difficulty, certified_mean, certified_sd, certified_r1, values })
}

struct AnovaHeaderSummary {
    df_between: usize,
    ss_between: f64,
    f_stat: f64,
    df_within: usize,
    ss_within: f64,
    r_squared: f64,
    residual_sd: f64,
}

fn parse_anova_header_summary(header: &str) -> Result<AnovaHeaderSummary, StrdParseError> {
    let mut between: Option<(usize, f64, f64)> = None;
    let mut within: Option<(usize, f64)> = None;
    let mut r_squared: Option<f64> = None;
    let mut residual_sd: Option<f64> = None;
    let mut pending_residual_prefix = false;

    for line in header.lines() {
        let lower = line.trim().to_ascii_lowercase();
        if lower.is_empty() {
            continue;
        }
        if lower.starts_with("between") {
            let nums = trailing_tokens(line, 4).ok_or(StrdParseError::MissingField("Between Treatments row"))?;
            let [df_tok, ss_tok, _, f_tok] = nums.as_slice() else {
                return Err(StrdParseError::MissingField("Between Treatments row"));
            };
            between =
                Some((parse_usize_token(df_tok, "df_between")?, parse_f64_token(ss_tok, "ss_between")?, parse_f64_token(f_tok, "certified_f")?));
        } else if lower.starts_with("within") {
            let nums = trailing_tokens(line, 3).ok_or(StrdParseError::MissingField("Within Treatments row"))?;
            let [df_tok, ss_tok, _] = nums.as_slice() else {
                return Err(StrdParseError::MissingField("Within Treatments row"));
            };
            within = Some((parse_usize_token(df_tok, "df_within")?, parse_f64_token(ss_tok, "ss_within")?));
        } else if lower.contains("r-squared") {
            if let Some(tok) = line.split_whitespace().last()
                && let Ok(v) = parse_f64_token(tok, "r_squared")
            {
                r_squared = Some(v);
            }
        } else if (lower.contains("residual standard deviation") || (pending_residual_prefix && lower.starts_with("standard deviation")))
            && let Some(tok) = line.split_whitespace().last()
            && let Ok(v) = parse_f64_token(tok, "residual_sd")
        {
            residual_sd = Some(v);
        }
        pending_residual_prefix = lower == "certified residual" || lower == "residual";
    }

    let (df_between, ss_between, f_stat) = between.ok_or(StrdParseError::MissingField("Between Treatments"))?;
    let (df_within, ss_within) = within.ok_or(StrdParseError::MissingField("Within Treatments"))?;
    Ok(AnovaHeaderSummary {
        df_between,
        ss_between,
        f_stat,
        df_within,
        ss_within,
        r_squared: r_squared.ok_or(StrdParseError::MissingField("R-Squared"))?,
        residual_sd: residual_sd.ok_or(StrdParseError::MissingField("Residual Standard Deviation"))?,
    })
}

/// Parse a NIST StRD One-Way ANOVA `.dat` file (`SmLs01`-`SmLs09`, `AtmWtAg`, `SiRstv`).
///
/// NIST StRD ANOVA `.dat` files may store `(group, response)` (`Data: Treatment  Response`) or
/// `(response, group)` (`Data: y  x`) in each data row; this parser normalizes both into
/// `observations: Vec<(group, response)>` (`group >= 1`).
///
/// # Errors
/// Returns [`StrdParseError`] if certified ANOVA table entries or `(y, group)` rows are malformed.
pub fn parse_anova_dat(text: &str) -> Result<AnovaDataset, StrdParseError> {
    let (header, data_rows) = split_header_and_data(text)?;
    let name = extract_dataset_name(&header).ok_or(StrdParseError::MissingField("Dataset Name"))?;
    let difficulty = extract_difficulty(&header);
    let summary = parse_anova_header_summary(&header)?;

    let expected_groups = summary.df_between.saturating_add(1);
    let mut observations = parse_anova_rows(&data_rows, false);
    let needs_group_first = match &observations {
        Err(_) => true,
        Ok(obs) => !anova_groups_cover_range(obs, expected_groups),
    };
    if needs_group_first
        && let Ok(alt) = parse_anova_rows(&data_rows, true)
        && anova_groups_cover_range(&alt, expected_groups)
    {
        observations = Ok(alt);
    }
    let observations = observations?;

    Ok(AnovaDataset {
        name,
        difficulty,
        certified_df_between: summary.df_between,
        certified_df_within: summary.df_within,
        certified_ss_between: summary.ss_between,
        certified_ss_within: summary.ss_within,
        certified_f: summary.f_stat,
        certified_r_squared: summary.r_squared,
        certified_residual_sd: summary.residual_sd,
        observations,
    })
}

fn parse_anova_rows(data_rows: &[NumberedLine<'_>], group_first: bool) -> Result<Vec<(usize, f64)>, StrdParseError> {
    let mut observations = Vec::with_capacity(data_rows.len());
    for &(line_number, row) in data_rows {
        let parts: Vec<&str> = row.split_whitespace().collect();
        let [col0, col1, ..] = parts.as_slice() else {
            return Err(StrdParseError::MalformedDataRow { line_number, line: row.to_owned() });
        };
        let (group_tok, y_tok) = if group_first { (col0, col1) } else { (col1, col0) };
        let group = parse_positive_group_token(group_tok, "anova group")?;
        let response = parse_f64_token(y_tok, "anova response")?;
        observations.push((group, response));
    }
    Ok(observations)
}

fn anova_groups_cover_range(observations: &[(usize, f64)], expected_groups: usize) -> bool {
    if expected_groups == 0 || observations.is_empty() {
        return false;
    }
    let mut seen = vec![false; expected_groups];
    for &(group, _) in observations {
        let Some(slot) = group.checked_sub(1).and_then(|idx| seen.get_mut(idx)) else {
            return false;
        };
        *slot = true;
    }
    seen.into_iter().all(|present| present)
}

fn trailing_tokens(line: &str, count: usize) -> Option<Vec<&str>> {
    let all: Vec<&str> = line.split_whitespace().collect();
    let start = all.len().checked_sub(count)?;
    all.get(start..).map(<[&str]>::to_vec)
}

fn is_beta_param_label(tok: &str) -> Option<usize> {
    let rest = tok.strip_prefix('B').or_else(|| tok.strip_prefix('b'))?;
    rest.parse::<usize>().ok()
}

struct LinearHeaderSummary {
    has_intercept: bool,
    betas: Vec<f64>,
    se_betas: Vec<f64>,
    residual_sd: f64,
    r_squared: f64,
    df_reg: usize,
    df_res: usize,
    ss_reg: f64,
    ss_res: f64,
    f_stat: Option<f64>,
}

#[derive(Default)]
struct LinearSummaryAccum {
    residual_sd: Option<f64>,
    r_squared: Option<f64>,
    reg_row: Option<(usize, f64, Option<f64>)>,
    res_row: Option<(usize, f64)>,
    pending_residual_prefix: bool,
}

fn parse_linear_header_summary(header: &str) -> Result<LinearHeaderSummary, StrdParseError> {
    let mut has_b0 = false;
    let mut betas = Vec::new();
    let mut se_betas = Vec::new();
    let mut acc = LinearSummaryAccum::default();

    for line in header.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if let [label_tok, est_tok, se_tok] = tokens.as_slice()
            && let Some(idx) = is_beta_param_label(label_tok)
            && let Ok(est) = parse_f64_token(est_tok, "beta")
        {
            let se = if se_tok.eq_ignore_ascii_case("exact") { 0.0 } else { parse_f64_token(se_tok, "se_beta")? };
            if idx == 0 {
                has_b0 = true;
            }
            betas.push(est);
            se_betas.push(se);
            acc.pending_residual_prefix = false;
            continue;
        }
        parse_linear_summary_line(line, &mut acc)?;
    }

    if betas.is_empty() {
        return Err(StrdParseError::MissingField("Certified Parameter Estimates (B0/B1...)"));
    }
    let (df_reg, ss_reg, f_stat) = acc.reg_row.ok_or(StrdParseError::MissingField("Model/Regression ANOVA row"))?;
    let (df_res, ss_res) = acc.res_row.ok_or(StrdParseError::MissingField("Residual ANOVA row"))?;

    Ok(LinearHeaderSummary {
        has_intercept: has_b0,
        betas,
        se_betas,
        residual_sd: acc.residual_sd.ok_or(StrdParseError::MissingField("Residual Standard Deviation"))?,
        r_squared: acc.r_squared.ok_or(StrdParseError::MissingField("R-Squared"))?,
        df_reg,
        df_res,
        ss_reg,
        ss_res,
        f_stat,
    })
}

fn parse_linear_summary_line(line: &str, acc: &mut LinearSummaryAccum) -> Result<(), StrdParseError> {
    let lower = line.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Ok(());
    }
    if lower.starts_with("residual standard deviation") || (acc.pending_residual_prefix && lower.starts_with("standard deviation")) {
        if let Some(last) = line.split_whitespace().last() {
            acc.residual_sd = Some(parse_f64_token(last, "residual_sd")?);
        }
    } else if lower.starts_with("r-squared") {
        if let Some(last) = line.split_whitespace().last() {
            acc.r_squared = Some(parse_f64_token(last, "r_squared")?);
        }
    } else if lower.starts_with("regression") || lower.starts_with("model") {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if let [_, df_tok, ss_tok, _, f_tok, ..] = toks.as_slice() {
            let df = parse_usize_token(df_tok, "df_reg")?;
            let ss = parse_f64_token(ss_tok, "ss_reg")?;
            let f_val = if f_tok.eq_ignore_ascii_case("infinity") { Some(f64::INFINITY) } else { parse_f64_token(f_tok, "f_stat").ok() };
            acc.reg_row = Some((df, ss, f_val));
        }
    } else if lower.starts_with("residual") {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if let [_, df_tok, ss_tok, _, ..] = toks.as_slice()
            && let Ok(df) = parse_usize_token(df_tok, "df_res")
            && let Ok(ss) = parse_f64_token(ss_tok, "ss_res")
        {
            acc.res_row = Some((df, ss));
        }
    }
    acc.pending_residual_prefix = lower == "residual" || lower == "certified residual";
    Ok(())
}

/// Parse a NIST StRD Linear Regression `.dat` file (`Norris`, `Pontius`, `NoInt1`, `NoInt2`, `Filip`, `Longley`, `Wampler1`-`Wampler5`).
///
/// Automatically constructs polynomial Vandermonde rows `[1, x, x^2, ..., x^d]` when a single predictor
/// column `x` is paired with `d + 1` certified `B0..Bd` coefficients.
///
/// # Errors
/// Returns [`StrdParseError`] if certified fields or data rows are missing or malformed.
pub fn parse_linear_regression_dat(text: &str) -> Result<LinearRegressionDataset, StrdParseError> {
    let (header, data_rows) = split_header_and_data(text)?;
    let name = extract_dataset_name(&header).ok_or(StrdParseError::MissingField("Dataset Name"))?;
    let difficulty = extract_difficulty(&header);
    let summary = parse_linear_header_summary(&header)?;
    let p = summary.betas.len();

    let mut y = Vec::with_capacity(data_rows.len());
    let mut design_rows = Vec::with_capacity(data_rows.len());

    for (line_number, row) in data_rows {
        let parts: Vec<&str> = row.split_whitespace().collect();
        let [y_tok, x_toks @ ..] = parts.as_slice() else {
            return Err(StrdParseError::MalformedDataRow { line_number, line: row.to_owned() });
        };
        if x_toks.is_empty() {
            return Err(StrdParseError::MalformedDataRow { line_number, line: row.to_owned() });
        }
        y.push(parse_f64_token(y_tok, "linear y")?);
        let raw_x: Result<Vec<f64>, StrdParseError> = x_toks.iter().map(|t| parse_f64_token(t, "linear x")).collect();
        let raw_x = raw_x?;

        let design_row = expand_linear_design_row(&raw_x, summary.has_intercept, p, line_number, row)?;
        design_rows.push(design_row);
    }

    Ok(LinearRegressionDataset {
        name,
        difficulty,
        has_intercept: summary.has_intercept,
        certified_betas: summary.betas,
        certified_se_betas: summary.se_betas,
        certified_residual_sd: summary.residual_sd,
        certified_r_squared: summary.r_squared,
        certified_df_reg: summary.df_reg,
        certified_df_res: summary.df_res,
        certified_ss_reg: summary.ss_reg,
        certified_ss_res: summary.ss_res,
        certified_f: summary.f_stat,
        y,
        design_rows,
    })
}

fn expand_linear_design_row(
    raw_x: &[f64],
    has_intercept: bool,
    target_p: usize,
    line_number: usize,
    raw_line: &str,
) -> Result<Vec<f64>, StrdParseError> {
    let expected_raw_cols = target_p.saturating_sub(usize::from(has_intercept));
    if raw_x.len() == expected_raw_cols {
        let mut row = Vec::with_capacity(target_p);
        if has_intercept {
            row.push(1.0);
        }
        row.extend_from_slice(raw_x);
        return Ok(row);
    }
    // Single-predictor polynomial model: [1, x, x^2, ..., x^d] (or [x, x^2, ..., x^p] without intercept).
    if let &[x] = raw_x {
        let mut row = Vec::with_capacity(target_p);
        let start_pow = i32::from(!has_intercept);
        for k in 0..target_p {
            row.push(x.powi(start_pow + (k as i32)));
        }
        return Ok(row);
    }
    Err(StrdParseError::MalformedDataRow { line_number, line: raw_line.to_owned() })
}

/// Parse a NIST StRD Nonlinear Least-Squares `.dat` file (`Misra1a`, `Rat43`, `Eckerle4`, `Thurber`, `Nelson`, etc.).
///
/// # Errors
/// Returns [`StrdParseError`] if starting values, certified parameters, or data columns are malformed.
pub fn parse_nonlinear_dat(text: &str) -> Result<NonlinearDataset, StrdParseError> {
    let (header, data_rows) = split_header_and_data(text)?;
    let name = extract_dataset_name(&header).ok_or(StrdParseError::MissingField("Dataset Name"))?;
    let difficulty = extract_difficulty(&header);

    let mut param_names = Vec::new();
    let mut start_1 = Vec::new();
    let mut start_2 = Vec::new();
    let mut certified_betas = Vec::new();
    let mut certified_se_betas = Vec::new();
    let mut certified_rss: Option<f64> = None;
    let mut certified_residual_sd: Option<f64> = None;
    let mut certified_df: Option<usize> = None;

    for line in header.lines() {
        let trimmed = line.trim();
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        // Parameter rows look like: `b1 = 500.0  250.0  2.3894212918E+02  1.2156585877E+01` or without `=`.
        if (parts.len() == 5 || parts.len() == 6)
            && let Some(&first) = parts.first()
            && is_beta_param_label(first).is_some()
            && let Some([s1, s2, cb, cse]) = extract_nonlinear_param_numbers(&parts)
        {
            param_names.push(first.to_owned());
            start_1.push(s1);
            start_2.push(s2);
            certified_betas.push(cb);
            certified_se_betas.push(cse);
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("residual sum of squares")
            && let Some(tok) = parts.last()
        {
            certified_rss = Some(parse_f64_token(tok, "certified_rss")?);
        } else if lower.starts_with("residual standard deviation")
            && let Some(tok) = parts.last()
        {
            certified_residual_sd = Some(parse_f64_token(tok, "certified_residual_sd")?);
        } else if lower.starts_with("degrees of freedom")
            && let Some(tok) = parts.last()
        {
            certified_df = Some(parse_usize_token(tok, "certified_df")?);
        }
    }

    if param_names.is_empty() {
        return Err(StrdParseError::MissingField("Nonlinear parameter rows (b1, b2, ...)"));
    }

    let mut y = Vec::with_capacity(data_rows.len());
    let mut predictors = Vec::with_capacity(data_rows.len());
    for (line_number, row) in data_rows {
        let parts: Vec<&str> = row.split_whitespace().collect();
        let [y_tok, pred_toks @ ..] = parts.as_slice() else {
            return Err(StrdParseError::MalformedDataRow { line_number, line: row.to_owned() });
        };
        if pred_toks.is_empty() {
            return Err(StrdParseError::MalformedDataRow { line_number, line: row.to_owned() });
        }
        y.push(parse_f64_token(y_tok, "nonlinear y")?);
        let preds: Result<Vec<f64>, StrdParseError> = pred_toks.iter().map(|t| parse_f64_token(t, "nonlinear predictor")).collect();
        predictors.push(preds?);
    }

    Ok(NonlinearDataset {
        name,
        difficulty,
        param_names,
        start_1,
        start_2,
        certified_betas,
        certified_se_betas,
        certified_rss: certified_rss.ok_or(StrdParseError::MissingField("Residual Sum of Squares"))?,
        certified_residual_sd: certified_residual_sd.ok_or(StrdParseError::MissingField("Residual Standard Deviation"))?,
        certified_df: certified_df.ok_or(StrdParseError::MissingField("Degrees of Freedom"))?,
        y,
        predictors,
    })
}

fn extract_nonlinear_param_numbers(parts: &[&str]) -> Option<[f64; 4]> {
    let tail = if parts.get(1) == Some(&"=") { parts.get(2..6)? } else { parts.get(1..5)? };
    let [s1, s2, cb, cse] = tail else {
        return None;
    };
    Some([
        parse_f64_token(s1, "start_1").ok()?,
        parse_f64_token(s2, "start_2").ok()?,
        parse_f64_token(cb, "certified_beta").ok()?,
        parse_f64_token(cse, "certified_se").ok()?,
    ])
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_STRD_BYTES, NistDifficulty, StrdParseError, parse_anova_dat, parse_linear_regression_dat, parse_nonlinear_dat, parse_univariate_dat,
    };

    #[test]
    fn parses_all_four_nist_strd_ascii_formats() {
        let uni_dat = "\
Dataset Name:  NumAcc1            (NumAcc1.dat)
File Format:   ASCII
               Data:              (lines 12 to 14)
Level of Difficulty:  Lower
Certified Values:
 Sample Mean                        : 10000002.0
 Sample Standard Deviation          :        1.0
 Sample Autocorrelation Coefficient :        0.0
Data:  y
 10000001
 10000002
 10000003
";
        let uni = parse_univariate_dat(uni_dat).unwrap();
        assert_eq!(uni.name, "NumAcc1");
        assert_eq!(uni.difficulty, NistDifficulty::Lower);
        assert_eq!(uni.values, vec![10_000_001.0, 10_000_002.0, 10_000_003.0]);

        let anova_dat = "\
Dataset Name:  MiniAnova          (MiniAnova.dat)
File Format:   ASCII
               Certified Values:  (lines 6 to 10)
               Data: y, x         (lines 12 to 17)
Level of Difficulty:  Higher
Certified Analysis of Variance Table:
Between Treatments   1   4.00000000000000E+00   4.00000000000000E+00   4.00000000000000E+00
Within Treatments    4   4.00000000000000E+00   1.00000000000000E+00
R-Squared: 5.00000000000000E-01
Residual Standard Deviation: 1.00000000000000E+00
Data:  y          x
 10000000.0   1
 10000001.0   1
 10000002.0   1
 10000002.0   2
 10000003.0   2
 10000004.0   2.0
";
        let anova = parse_anova_dat(anova_dat).unwrap();
        assert_eq!(anova.difficulty, NistDifficulty::Higher);
        assert_eq!(anova.certified_df_between, 1);
        assert_eq!(anova.certified_df_within, 4);
        assert_eq!(
            anova.observations,
            vec![(1, 10_000_000.0), (1, 10_000_001.0), (1, 10_000_002.0), (2, 10_000_002.0), (2, 10_000_003.0), (2, 10_000_004.0),]
        );

        let lin_dat = "\
Dataset Name:  Wampler1           (Wampler1.dat)
Level of Difficulty:  Lower
Certified Regression Statistics:
  B0   1.00000000000000E+00   0.00000000000000E+00
  B1   1.00000000000000E+00   0.00000000000000E+00
  B2   1.00000000000000E+00   0.00000000000000E+00
Residual Standard Deviation   0.00000000000000E+00
R-Squared                     1.00000000000000E+00
Model       2   9.80000000000000E+01   4.90000000000000E+01   Infinity
Residual    1   0.00000000000000E+00   0.00000000000000E+00
Data:
  1.0   0.0
  3.0   1.0
  7.0   2.0
 13.0   3.0
";
        let lin = parse_linear_regression_dat(lin_dat).unwrap();
        assert!(lin.has_intercept);
        assert_eq!(lin.design_rows[2], vec![1.0, 2.0, 4.0]);

        let nl_dat = "\
Dataset Name:  Misra1a            (Misra1a.dat)
Level of Difficulty:  Lower
  b1 =   500.0   250.0   2.3894212918E+02   1.2156585877E+01
  b2 =  0.0001  0.0005   5.5015643181E-04   3.6568478596E-05
Residual Sum of Squares:                    1.2455138894E-01
Residual Standard Deviation:                1.0187948793E-01
Degrees of Freedom:                                12
Data:
  10.07   77.6
  14.73  114.9
";
        let nl = parse_nonlinear_dat(nl_dat).unwrap();
        assert_eq!(nl.param_names, vec!["b1", "b2"]);
        assert_eq!(nl.certified_df, 12);
        assert_eq!(nl.predictors.len(), 2);
    }

    #[test]
    fn anova_parser_rejects_invalid_group_labels_and_oversized_inputs() {
        let base_header = "\
Dataset Name:  BadAnova           (BadAnova.dat)
Level of Difficulty:  Lower
Certified Analysis of Variance Table:
Between Treatments   1   4.0   4.0   4.0
Within Treatments    4   4.0   1.0
R-Squared: 0.5
Residual Standard Deviation: 1.0
Data:  y   x
";
        for bad_group in ["0", "-1", "1.5", "NaN", "inf"] {
            let text = format!("{base_header} 10.0   {bad_group}\n");
            let err = parse_anova_dat(&text).unwrap_err();
            assert!(
                matches!(err, StrdParseError::InvalidNumber { context: "anova group", .. }),
                "expected InvalidNumber for {bad_group}, got {err:?}"
            );
        }

        let oversized = "x".repeat(MAX_STRD_BYTES + 1);
        assert!(matches!(parse_univariate_dat(&oversized), Err(StrdParseError::InputTooLarge { kind: "input bytes", .. })));
    }
}
