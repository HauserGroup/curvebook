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
    cleaned.parse::<f64>().map_err(|_| StrdParseError::InvalidNumber { context, token: token.to_owned() })
}

fn parse_usize_token(token: &str, context: &'static str) -> Result<usize, StrdParseError> {
    token.trim().parse::<usize>().map_err(|_| StrdParseError::InvalidNumber { context, token: token.to_owned() })
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

/// Split a NIST `.dat` file into `(header_lines, data_lines_with_1based_line_numbers)`.
fn split_header_and_data(text: &str) -> Result<SplitSections<'_>, StrdParseError> {
    let mut header = String::new();
    let mut in_data = false;
    let mut data_rows = Vec::new();

    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if !in_data {
            let lower = trimmed.to_ascii_lowercase();
            if lower == "data:" || lower.starts_with("data:") && !lower.contains("lines ") || (lower.starts_with("data:") && lower.contains("line")) {
                in_data = true;
                continue;
            }
            header.push_str(line);
            header.push('\n');
        } else if !trimmed.is_empty() && trimmed.as_bytes().first().is_some_and(|b| b.is_ascii_digit() || *b == b'-' || *b == b'+' || *b == b'.') {
            data_rows.push((idx + 1, trimmed));
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

/// Parse a NIST StRD One-Way ANOVA `.dat` file (`SmLs01`-`SmLs09`, `AtmWtAg`, `SiRstv`).
///
/// # Errors
/// Returns [`StrdParseError`] if certified ANOVA table entries or `(group, y)` rows are malformed.
pub fn parse_anova_dat(text: &str) -> Result<AnovaDataset, StrdParseError> {
    let (header, data_rows) = split_header_and_data(text)?;
    let name = extract_dataset_name(&header).ok_or(StrdParseError::MissingField("Dataset Name"))?;
    let difficulty = extract_difficulty(&header);

    let mut between: Option<(usize, f64, f64)> = None;
    let mut within: Option<(usize, f64)> = None;
    let mut r_squared: Option<f64> = None;
    let mut residual_sd: Option<f64> = None;

    for line in header.lines() {
        let lower = line.trim().to_ascii_lowercase();
        if lower.starts_with("between") {
            let nums = trailing_tokens(line, 4).ok_or(StrdParseError::MissingField("Between Treatments row"))?;
            let df = parse_usize_token(nums[0], "df_between")?;
            let ss = parse_f64_token(nums[1], "ss_between")?;
            let f_stat = parse_f64_token(nums[3], "certified_f")?;
            between = Some((df, ss, f_stat));
        } else if lower.starts_with("within") {
            let nums = trailing_tokens(line, 3).ok_or(StrdParseError::MissingField("Within Treatments row"))?;
            let df = parse_usize_token(nums[0], "df_within")?;
            let ss = parse_f64_token(nums[1], "ss_within")?;
            within = Some((df, ss));
        } else if lower.contains("r-squared") {
            if let Some(tok) = line.split_whitespace().last()
                && let Ok(v) = parse_f64_token(tok, "r_squared")
            {
                r_squared = Some(v);
            }
        } else if lower.contains("residual standard deviation")
            && let Some(tok) = line.split_whitespace().last()
            && let Ok(v) = parse_f64_token(tok, "residual_sd")
        {
            residual_sd = Some(v);
        }
    }

    let (certified_df_between, certified_ss_between, certified_f) = between.ok_or(StrdParseError::MissingField("Between Treatments"))?;
    let (certified_df_within, certified_ss_within) = within.ok_or(StrdParseError::MissingField("Within Treatments"))?;
    let certified_r_squared = r_squared.ok_or(StrdParseError::MissingField("R-Squared"))?;
    let certified_residual_sd = residual_sd.ok_or(StrdParseError::MissingField("Residual Standard Deviation"))?;

    let mut observations = Vec::with_capacity(data_rows.len());
    for (line_number, row) in data_rows {
        let parts: Vec<&str> = row.split_whitespace().collect();
        if parts.len() < 2 {
            return Err(StrdParseError::MalformedDataRow { line_number, line: row.to_owned() });
        }
        let group = parse_f64_token(parts[0], "anova group")? as usize;
        let response = parse_f64_token(parts[1], "anova response")?;
        observations.push((group, response));
    }

    Ok(AnovaDataset {
        name,
        difficulty,
        certified_df_between,
        certified_df_within,
        certified_ss_between,
        certified_ss_within,
        certified_f,
        certified_r_squared,
        certified_residual_sd,
        observations,
    })
}

fn trailing_tokens(line: &str, count: usize) -> Option<Vec<&str>> {
    let all: Vec<&str> = line.split_whitespace().collect();
    if all.len() < count {
        return None;
    }
    Some(all[all.len() - count..].to_vec())
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

fn parse_linear_header_summary(header: &str) -> Result<LinearHeaderSummary, StrdParseError> {
    let mut has_b0 = false;
    let mut betas = Vec::new();
    let mut se_betas = Vec::new();
    let mut residual_sd: Option<f64> = None;
    let mut r_squared: Option<f64> = None;
    let mut reg_row: Option<(usize, f64, Option<f64>)> = None;
    let mut res_row: Option<(usize, f64)> = None;

    for line in header.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() == 3
            && let Some(idx) = is_beta_param_label(tokens[0])
            && let Ok(est) = parse_f64_token(tokens[1], "beta")
        {
            let se = if tokens[2].eq_ignore_ascii_case("exact") { 0.0 } else { parse_f64_token(tokens[2], "se_beta")? };
            if idx == 0 {
                has_b0 = true;
            }
            betas.push(est);
            se_betas.push(se);
            continue;
        }
        parse_linear_summary_line(line, &mut residual_sd, &mut r_squared, &mut reg_row, &mut res_row)?;
    }

    if betas.is_empty() {
        return Err(StrdParseError::MissingField("Certified Parameter Estimates (B0/B1...)"));
    }
    let (df_reg, ss_reg, f_stat) = reg_row.ok_or(StrdParseError::MissingField("Model/Regression ANOVA row"))?;
    let (df_res, ss_res) = res_row.ok_or(StrdParseError::MissingField("Residual ANOVA row"))?;

    Ok(LinearHeaderSummary {
        has_intercept: has_b0,
        betas,
        se_betas,
        residual_sd: residual_sd.ok_or(StrdParseError::MissingField("Residual Standard Deviation"))?,
        r_squared: r_squared.ok_or(StrdParseError::MissingField("R-Squared"))?,
        df_reg,
        df_res,
        ss_reg,
        ss_res,
        f_stat,
    })
}

fn parse_linear_summary_line(
    line: &str,
    residual_sd: &mut Option<f64>,
    r_squared: &mut Option<f64>,
    reg_row: &mut Option<(usize, f64, Option<f64>)>,
    res_row: &mut Option<(usize, f64)>,
) -> Result<(), StrdParseError> {
    let lower = line.trim().to_ascii_lowercase();
    if lower.starts_with("residual standard deviation") {
        if let Some(last) = line.split_whitespace().last() {
            *residual_sd = Some(parse_f64_token(last, "residual_sd")?);
        }
    } else if lower.starts_with("r-squared") {
        if let Some(last) = line.split_whitespace().last() {
            *r_squared = Some(parse_f64_token(last, "r_squared")?);
        }
    } else if lower.starts_with("regression") || lower.starts_with("model") {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() >= 5 {
            let df = parse_usize_token(toks[1], "df_reg")?;
            let ss = parse_f64_token(toks[2], "ss_reg")?;
            let f_val = if toks[4].eq_ignore_ascii_case("infinity") { Some(f64::INFINITY) } else { parse_f64_token(toks[4], "f_stat").ok() };
            *reg_row = Some((df, ss, f_val));
        }
    } else if lower.starts_with("residual") {
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() >= 4
            && let Ok(df) = parse_usize_token(toks[1], "df_res")
            && let Ok(ss) = parse_f64_token(toks[2], "ss_res")
        {
            *res_row = Some((df, ss));
        }
    }
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
        if parts.len() < 2 {
            return Err(StrdParseError::MalformedDataRow { line_number, line: row.to_owned() });
        }
        y.push(parse_f64_token(parts[0], "linear y")?);
        let raw_x: Result<Vec<f64>, StrdParseError> = parts[1..].iter().map(|t| parse_f64_token(t, "linear x")).collect();
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
    if raw_x.len() == 1 {
        let x = raw_x[0];
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
            && is_beta_param_label(parts[0]).is_some()
            && let Some(nums) = extract_nonlinear_param_numbers(&parts)
        {
            param_names.push(parts[0].to_owned());
            start_1.push(nums[0]);
            start_2.push(nums[1]);
            certified_betas.push(nums[2]);
            certified_se_betas.push(nums[3]);
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
        if parts.len() < 2 {
            return Err(StrdParseError::MalformedDataRow { line_number, line: row.to_owned() });
        }
        y.push(parse_f64_token(parts[0], "nonlinear y")?);
        let preds: Result<Vec<f64>, StrdParseError> = parts[1..].iter().map(|t| parse_f64_token(t, "nonlinear predictor")).collect();
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
    Some([
        parse_f64_token(tail[0], "start_1").ok()?,
        parse_f64_token(tail[1], "start_2").ok()?,
        parse_f64_token(tail[2], "certified_beta").ok()?,
        parse_f64_token(tail[3], "certified_se").ok()?,
    ])
}

#[cfg(test)]
mod tests {
    use super::{NistDifficulty, parse_anova_dat, parse_linear_regression_dat, parse_nonlinear_dat, parse_univariate_dat};

    #[test]
    fn parses_all_four_nist_strd_ascii_formats() {
        let uni_dat = "\
Dataset Name:  NumAcc1            (NumAcc1.dat)
Level of Difficulty:  Lower
Certified Values:
 Sample Mean                        : 10000002.0
 Sample Standard Deviation          :        1.0
 Sample Autocorrelation Coefficient :        0.0
Data:
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
Level of Difficulty:  Higher
Certified Analysis of Variance Table:
Between Treatments   1   4.00000000000000E+00   4.00000000000000E+00   4.00000000000000E+00
Within Treatments    4   4.00000000000000E+00   1.00000000000000E+00
R-Squared: 5.00000000000000E-01
Residual Standard Deviation: 1.00000000000000E+00
Data:
 1  10000000.0
 1  10000001.0
 1  10000002.0
 2  10000002.0
 2  10000003.0
 2  10000004.0
";
        let anova = parse_anova_dat(anova_dat).unwrap();
        assert_eq!(anova.difficulty, NistDifficulty::Higher);
        assert_eq!(anova.certified_df_between, 1);
        assert_eq!(anova.certified_df_within, 4);
        assert_eq!(anova.observations.len(), 6);

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
}
