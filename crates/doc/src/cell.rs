//! Typed cell values, non-destructive cell exclusions (`excluded: bool`), inline `*` exclusion
//! token parsing, replicate subcolumn modes, and pre-summarized `(Mean, SD/SEM/%CV, N)` moment
//! reconstruction (`docs/implementation/data-tables-and-transforms.md` §1.2, §2, §2.1, §6.1).

use crate::error::{DocError, SummaryPointError, validate_subcolumns};

/// Typed content of a single grid cell (`docs/implementation/data-tables-and-transforms.md` §1.2).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum CellValue {
    /// Unpopulated cell (missing observation; non-finite `NaN`/`±Inf` inputs also normalize to `Empty` via [`Self::from_finite_or_empty`]).
    #[default]
    Empty,
    /// Numeric scalar observation in the column's data units (finite `f64` when constructed via [`Self::from_finite_or_empty`] or [`Cell::number`]).
    Number(f64),
    /// Categorical label, row/group identifier, or free-form string token.
    Text(String),
}

impl CellValue {
    /// Construct a finite numeric cell value, mapping `NaN` or `±Inf` cleanly to [`Self::Empty`]
    /// (`docs/implementation/data-tables-and-transforms.md` §4.1).
    #[must_use]
    pub fn from_finite_or_empty(value: f64) -> Self {
        if value.is_finite() { Self::Number(value) } else { Self::Empty }
    }

    /// True if the cell holds no value (`Empty`, or whitespace-only text).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Empty => true,
            Self::Number(_) => false,
            Self::Text(s) => s.trim().is_empty(),
        }
    }

    /// Return `Some(x)` iff this cell holds a finite IEEE-754 `f64`.
    #[must_use]
    pub fn finite_f64(&self) -> Option<f64> {
        match self {
            Self::Number(v) if v.is_finite() => Some(*v),
            _ => None,
        }
    }
}

/// Grid cell pairing a [`CellValue`] with a non-destructive exclusion mask (`excluded: bool`).
///
/// Excluded cells remain visible in the spreadsheet (rendered in blue italics with an `*` badge)
/// and persist in `.curvebook` archives while downstream statistical tests, fits, transforms, and
/// graphs mask them out without mutating raw observations (`docs/implementation/data-tables-and-transforms.md` §1.2).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    /// Underlying empty, numeric, or text payload stored in the cell.
    pub value: CellValue,
    /// When `true`, the observation is retained visibly in the sheet (`*` badge) but masked out of [`Self::active_number`], [`Self::active_text`], statistical tests, fits, transforms, and plotted series.
    pub excluded: bool,
}

impl Cell {
    /// Empty, unexcluded cell.
    #[must_use]
    pub const fn empty() -> Self {
        Self { value: CellValue::Empty, excluded: false }
    }

    /// Finite numeric cell (`NaN` / `±Inf` mapped safely to `Empty`).
    #[must_use]
    pub fn number(value: f64) -> Self {
        Self { value: CellValue::from_finite_or_empty(value), excluded: false }
    }

    /// Excluded numeric cell (`45.6*`).
    #[must_use]
    pub fn excluded_number(value: f64) -> Self {
        Self { value: CellValue::from_finite_or_empty(value), excluded: true }
    }

    /// Text / categorical token cell.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self { value: CellValue::Text(value.into()), excluded: false }
    }

    /// Excluded text / categorical token cell.
    #[must_use]
    pub fn excluded_text(value: impl Into<String>) -> Self {
        Self { value: CellValue::Text(value.into()), excluded: true }
    }

    /// True when the underlying value is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Return `Some(x)` iff `!self.excluded` and `self.value` is a finite `f64`.
    #[must_use]
    pub fn active_number(&self) -> Option<f64> {
        if self.excluded { None } else { self.value.finite_f64() }
    }

    /// Return `Some(&str)` iff `!self.excluded` and `self.value` is non-empty text.
    #[must_use]
    pub fn active_text(&self) -> Option<&str> {
        if self.excluded {
            return None;
        }
        match &self.value {
            CellValue::Text(s) if !s.trim().is_empty() => Some(s.as_str()),
            _ => None,
        }
    }

    /// Parse an imported CSV/TSV/clipboard token, honoring inline asterisk exclusion syntax
    /// (`45.6*` or `*45.6` -> `CellValue::Number(45.6)` with `excluded = true`) and regional
    /// decimal separators (`.` vs `,`) (`docs/implementation/data-tables-and-transforms.md` §6.1).
    #[must_use]
    pub fn parse_import_token(raw: &str, decimal_separator: char, parse_asterisk_exclusion: bool) -> Self {
        Self::parse_import_token_with_thousands(raw, decimal_separator, None, parse_asterisk_exclusion)
    }

    /// Parse an imported token with explicit decimal and optional thousands-grouping separators.
    #[must_use]
    pub fn parse_import_token_with_thousands(
        raw: &str,
        decimal_separator: char,
        thousands_separator: Option<char>,
        parse_asterisk_exclusion: bool,
    ) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("na") || trimmed.eq_ignore_ascii_case("nan") {
            return Self::empty();
        }

        let (core, starred) = if parse_asterisk_exclusion {
            if let Some(stripped) = trimmed.strip_suffix('*') {
                (stripped.trim(), true)
            } else if let Some(stripped) = trimmed.strip_prefix('*') {
                (stripped.trim(), true)
            } else {
                (trimmed, false)
            }
        } else {
            (trimmed, false)
        };

        if core.is_empty() {
            return Self::text(trimmed);
        }

        if let Some(num) = parse_import_number(core, decimal_separator, thousands_separator) {
            return Self { value: CellValue::Number(num), excluded: starred };
        }

        Self { value: CellValue::Text(if starred { core.to_owned() } else { trimmed.to_owned() }), excluded: starred }
    }
}

/// Parse a numeric token respecting explicit decimal separator (`.` or `,`), optional thousands
/// grouping separator, Unicode minus (`\u{2212}`), and scientific notation (`e`/`E`).
#[must_use]
pub fn parse_import_number(raw: &str, decimal_separator: char, thousands_separator: Option<char>) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (neg, body) = split_numeric_sign(trimmed)?;
    if body.is_empty() {
        return None;
    }

    let (mantissa, exp_suffix) = split_exponent(body)?;
    let thousands = thousands_separator.filter(|&sep| sep != decimal_separator);
    let clean_mantissa = normalize_numeric_mantissa(mantissa, decimal_separator, thousands)?;
    let mut assembled = String::with_capacity(clean_mantissa.len() + exp_suffix.as_ref().map_or(0, String::len) + usize::from(neg));
    if neg {
        assembled.push('-');
    }
    assembled.push_str(&clean_mantissa);
    if let Some(exp) = exp_suffix {
        assembled.push('e');
        assembled.push_str(&exp);
    }

    let num = assembled.parse::<f64>().ok()?;
    num.is_finite().then_some(num)
}

/// Infer `(decimal_separator, thousands_separator)` from a sample of trimmed cell tokens under
/// an active column delimiter.
#[must_use]
pub fn infer_number_separators<'a, I>(tokens: I, delimiter: char) -> (char, Option<char>)
where
    I: IntoIterator<Item = &'a str>,
{
    let mut us_thousands_hits = 0_usize;
    let mut eu_thousands_hits = 0_usize;
    let mut comma_decimal_candidates = 0_usize;
    let mut comma_non3_decimal_hits = 0_usize;
    let mut dot_decimal_candidates = 0_usize;
    let mut dot_non3_decimal_hits = 0_usize;

    for raw in tokens {
        let token = raw.trim().trim_matches('*').trim();
        if token.is_empty() {
            continue;
        }
        let Some((_, body)) = split_numeric_sign(token) else {
            continue;
        };
        let Some((mantissa, _)) = split_exponent(body) else {
            continue;
        };
        if mantissa.is_empty() || !mantissa.chars().all(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | ' ' | '\u{00a0}' | '\u{202f}')) {
            continue;
        }
        let comma_count = mantissa.chars().filter(|&c| c == ',').count();
        let dot_count = mantissa.chars().filter(|&c| c == '.').count();

        if comma_count >= 1
            && dot_count == 1
            && let Some((int_raw, frac_raw)) = mantissa.split_once('.')
            && !frac_raw.is_empty()
            && frac_raw.bytes().all(|b| b.is_ascii_digit())
            && strip_thousands_grouping(int_raw, &[',']).is_some()
        {
            us_thousands_hits = us_thousands_hits.saturating_add(1);
            dot_non3_decimal_hits = dot_non3_decimal_hits.saturating_add(1);
            continue;
        }
        if dot_count >= 1
            && comma_count == 1
            && let Some((int_raw, frac_raw)) = mantissa.split_once(',')
            && !frac_raw.is_empty()
            && frac_raw.bytes().all(|b| b.is_ascii_digit())
            && strip_thousands_grouping(int_raw, &['.']).is_some()
        {
            eu_thousands_hits = eu_thousands_hits.saturating_add(1);
            comma_non3_decimal_hits = comma_non3_decimal_hits.saturating_add(1);
            continue;
        }
        if comma_count > 1 && dot_count == 0 && strip_thousands_grouping(mantissa, &[',']).is_some() {
            us_thousands_hits = us_thousands_hits.saturating_add(1);
            continue;
        }
        if dot_count > 1 && comma_count == 0 && strip_thousands_grouping(mantissa, &['.']).is_some() {
            eu_thousands_hits = eu_thousands_hits.saturating_add(1);
            continue;
        }
        if comma_count == 1
            && dot_count == 0
            && let Some((int_raw, frac_raw)) = mantissa.split_once(',')
            && !frac_raw.is_empty()
            && int_raw.bytes().all(|b| b.is_ascii_digit())
            && frac_raw.bytes().all(|b| b.is_ascii_digit())
        {
            comma_decimal_candidates = comma_decimal_candidates.saturating_add(1);
            if int_raw.is_empty() || int_raw.len() > 3 || int_raw.starts_with('0') || frac_raw.len() != 3 {
                comma_non3_decimal_hits = comma_non3_decimal_hits.saturating_add(1);
            }
            continue;
        }
        if dot_count == 1
            && comma_count == 0
            && let Some((int_raw, frac_raw)) = mantissa.split_once('.')
            && !frac_raw.is_empty()
            && int_raw.bytes().all(|b| b.is_ascii_digit())
            && frac_raw.bytes().all(|b| b.is_ascii_digit())
        {
            dot_decimal_candidates = dot_decimal_candidates.saturating_add(1);
            if int_raw.is_empty() || int_raw.len() > 3 || int_raw.starts_with('0') || frac_raw.len() != 3 {
                dot_non3_decimal_hits = dot_non3_decimal_hits.saturating_add(1);
            }
        }
    }

    if eu_thousands_hits > 0 && us_thousands_hits == 0 {
        return (',', Some('.'));
    }
    if us_thousands_hits > 0 && eu_thousands_hits == 0 {
        return ('.', Some(','));
    }
    if delimiter == ',' {
        if comma_non3_decimal_hits > 0 && dot_decimal_candidates == 0 && us_thousands_hits == 0 {
            return (',', None);
        }
        let thousands = (us_thousands_hits > 0 || (comma_decimal_candidates > 0 && comma_non3_decimal_hits == 0)).then_some(',');
        return ('.', thousands);
    }
    if comma_non3_decimal_hits > dot_non3_decimal_hits || (comma_decimal_candidates > 0 && dot_decimal_candidates == 0 && dot_non3_decimal_hits == 0)
    {
        let thousands = (eu_thousands_hits > 0 || (dot_decimal_candidates > 0 && dot_non3_decimal_hits == 0)).then_some('.');
        return (',', thousands);
    }
    if dot_decimal_candidates > 0 || us_thousands_hits > 0 {
        let thousands = (us_thousands_hits > 0 || (comma_decimal_candidates > 0 && comma_non3_decimal_hits == 0)).then_some(',');
        return ('.', thousands);
    }
    if delimiter == ';' && comma_decimal_candidates > 0 {
        return (',', None);
    }
    ('.', None)
}

fn split_numeric_sign(raw: &str) -> Option<(bool, &str)> {
    if let Some(rest) = raw.strip_prefix('-') {
        Some((true, rest.trim_start()))
    } else if let Some(rest) = raw.strip_prefix('\u{2212}') {
        Some((true, rest.trim_start()))
    } else if let Some(rest) = raw.strip_prefix('+') {
        Some((false, rest.trim_start()))
    } else {
        Some((false, raw))
    }
}

fn split_exponent(body: &str) -> Option<(&str, Option<String>)> {
    let Some(exp_pos) = body.find(['e', 'E']) else {
        return Some((body, None));
    };
    let mantissa = body.get(..exp_pos)?;
    let exp_raw = body.get(exp_pos + 1..)?;
    if mantissa.is_empty() || exp_raw.is_empty() {
        return None;
    }
    let (exp_neg, digits) = split_numeric_sign(exp_raw)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let exp = if exp_neg { format!("-{digits}") } else { digits.to_owned() };
    Some((mantissa, Some(exp)))
}

fn normalize_numeric_mantissa(mantissa: &str, decimal_separator: char, thousands_separator: Option<char>) -> Option<String> {
    let dec_count = mantissa.chars().filter(|&c| c == decimal_separator).count();
    if dec_count == 1 {
        let (int_raw, frac_raw) = mantissa.split_once(decimal_separator)?;
        if (int_raw.is_empty() && frac_raw.is_empty()) || !frac_raw.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let clean_int = if int_raw.is_empty() {
            String::new()
        } else {
            let mut seps = Vec::with_capacity(5);
            if let Some(t) = thousands_separator {
                seps.push(t);
            }
            for fallback in [if decimal_separator == ',' { '.' } else { ',' }, ' ', '\u{00a0}', '\u{202f}'] {
                if fallback != decimal_separator && !seps.contains(&fallback) {
                    seps.push(fallback);
                }
            }
            strip_thousands_grouping(int_raw, &seps)?
        };
        return Some(format!("{clean_int}.{frac_raw}"));
    }

    if dec_count > 1 {
        return None;
    }

    if mantissa.bytes().all(|b| b.is_ascii_digit()) {
        return (!mantissa.is_empty()).then(|| mantissa.to_owned());
    }

    let mut plain_seps = Vec::with_capacity(4);
    if let Some(t) = thousands_separator {
        plain_seps.push(t);
    }
    for space in [' ', '\u{00a0}', '\u{202f}'] {
        if !plain_seps.contains(&space) {
            plain_seps.push(space);
        }
    }
    if let Some(clean_int) = strip_thousands_grouping(mantissa, &plain_seps) {
        return Some(clean_int);
    }

    // Without explicit thousands separator, allow multi-group comma integers in dot mode (e.g. `1,234,567`)
    // and multi-group dot integers in comma mode (e.g. `1.234.567`), plus ungrouped invariant `.` decimals
    // in comma mode (`12.34`) when dot is not the configured thousands separator.
    let opposite = if decimal_separator == ',' { '.' } else { ',' };
    if thousands_separator != Some(opposite) {
        let opp_count = mantissa.chars().filter(|&c| c == opposite).count();
        if opp_count > 1 && strip_thousands_grouping(mantissa, &[opposite]).is_some() {
            return strip_thousands_grouping(mantissa, &[opposite]);
        }
        if decimal_separator == ','
            && opp_count == 1
            && let Some((int_raw, frac_raw)) = mantissa.split_once('.')
            && !(int_raw.is_empty() && frac_raw.is_empty())
            && int_raw.bytes().all(|b| b.is_ascii_digit())
            && frac_raw.bytes().all(|b| b.is_ascii_digit())
        {
            return Some(mantissa.to_owned());
        }
    }

    None
}

/// Strip strict 3-digit thousands grouping separators (`1,234`, `12,345,678`) from `int_part`.
#[must_use]
pub fn strip_thousands_grouping(int_part: &str, allowed_seps: &[char]) -> Option<String> {
    if int_part.is_empty() {
        return None;
    }
    if int_part.bytes().all(|b| b.is_ascii_digit()) {
        return Some(int_part.to_owned());
    }
    for &sep in allowed_seps {
        if !int_part.contains(sep) {
            continue;
        }
        let mut parts = int_part.split(sep);
        let first = parts.next()?;
        if first.is_empty() || first.len() > 3 || first.starts_with('0') || !first.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let mut out = String::from(first);
        let mut group_count = 0_usize;
        for part in parts {
            if part.len() != 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            out.push_str(part);
            group_count += 1;
        }
        if group_count > 0 {
            return Some(out);
        }
    }
    None
}

/// Subcolumn structure within each data-set column (`docs/implementation/data-tables-and-transforms.md` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SubcolumnMode {
    /// Single value per row (`1` subcolumn).
    #[default]
    Single,
    /// `n` raw replicate subcolumns (`Y1 .. Yn`, `1 <= n <= MAX_SUBCOLUMNS`).
    RawReplicates(usize),
    /// Pre-summarized `[Mean, SD]` (`2` subcolumns, no sample size `N`).
    MeanSd,
    /// Pre-summarized `[Mean, SEM]` (`2` subcolumns, no sample size `N`).
    MeanSem,
    /// Pre-summarized `[Mean, SD, N]` (`3` subcolumns).
    MeanSdN,
    /// Pre-summarized `[Mean, SEM, N]` (`3` subcolumns).
    MeanSemN,
    /// Pre-summarized `[Mean, %CV, N]` (`3` subcolumns).
    MeanCvN,
    /// Pre-summarized `[Mean, -Error, +Error]` (`3` subcolumns).
    MeanLowHigh,
    /// Pre-summarized `[Mean, LowerLimit, UpperLimit]` (`3` subcolumns).
    UpperLowerLimits,
    /// Pre-summarized `[Median, Q1, Q3]` (`3` subcolumns).
    MedianIqr,
}

impl SubcolumnMode {
    /// Number of physical subcolumns per data-set column.
    #[must_use]
    pub const fn subcolumn_count(self) -> usize {
        match self {
            Self::Single => 1,
            Self::RawReplicates(n) => n,
            Self::MeanSd | Self::MeanSem => 2,
            Self::MeanSdN | Self::MeanSemN | Self::MeanCvN | Self::MeanLowHigh | Self::UpperLowerLimits | Self::MedianIqr => 3,
        }
    }

    /// Validate that replicate counts lie within `1..=MAX_SUBCOLUMNS`.
    pub fn validate(self) -> Result<(), DocError> {
        match self {
            Self::RawReplicates(n) => validate_subcolumns(n),
            _ => Ok(()),
        }
    }

    /// True if values represent pre-computed summary statistics rather than raw observations.
    ///
    /// Nonparametric rank tests, normality tests, outlier tests, and robust regression refuse
    /// pre-summarized tables with `RequiresRawReplicates` (`docs/implementation/data-tables-and-transforms.md` §2.1).
    #[must_use]
    pub const fn is_pre_summarized(self) -> bool {
        matches!(
            self,
            Self::MeanSd
                | Self::MeanSem
                | Self::MeanSdN
                | Self::MeanSemN
                | Self::MeanCvN
                | Self::MeanLowHigh
                | Self::UpperLowerLimits
                | Self::MedianIqr
        )
    }

    /// True if parametric `(Mean, SD, N)` within-point sum of squares can be reconstructed rigorously.
    #[must_use]
    pub const fn supports_parametric_moments(self) -> bool {
        matches!(self, Self::MeanSdN | Self::MeanSemN | Self::MeanCvN)
    }

    /// Second-tier subcolumn header labels for the spreadsheet grid.
    #[must_use]
    pub fn subcolumn_labels(self) -> Vec<String> {
        match self {
            Self::Single => vec![String::from("Y")],
            Self::RawReplicates(n) => (1..=n).map(|i| format!("Y{i}")).collect(),
            Self::MeanSd => vec![String::from("Mean"), String::from("SD")],
            Self::MeanSem => vec![String::from("Mean"), String::from("SEM")],
            Self::MeanSdN => vec![String::from("Mean"), String::from("SD"), String::from("N")],
            Self::MeanSemN => vec![String::from("Mean"), String::from("SEM"), String::from("N")],
            Self::MeanCvN => vec![String::from("Mean"), String::from("%CV"), String::from("N")],
            Self::MeanLowHigh => vec![String::from("Mean"), String::from("-Error"), String::from("+Error")],
            Self::UpperLowerLimits => vec![String::from("Mean"), String::from("Lower"), String::from("Upper")],
            Self::MedianIqr => vec![String::from("Median"), String::from("Q1"), String::from("Q3")],
        }
    }

    /// Resolve `(center, lower_endpoint, upper_endpoint)` in data-axis coordinates from a
    /// pre-summarized subcolumn slice (`MeanSd`, `MeanSem`, `MeanSdN`, `MeanSemN`, `MeanCvN`,
    /// `MeanLowHigh`, `UpperLowerLimits`, or `MedianIqr`).
    #[must_use]
    pub fn explicit_bounds(self, subcells: &[Cell]) -> Option<ExplicitErrorBounds> {
        let center = subcells.first()?.active_number()?;
        if !center.is_finite() {
            return None;
        }
        match self {
            Self::Single | Self::RawReplicates(_) => None,
            Self::MeanSd | Self::MeanSem | Self::MeanSdN | Self::MeanSemN => {
                let d = subcells.get(1)?.active_number()?;
                if !d.is_finite() || d < 0.0 {
                    return None;
                }
                let lower = center - d;
                let upper = center + d;
                (lower.is_finite() && upper.is_finite()).then_some(ExplicitErrorBounds { center, lower, upper })
            }
            Self::MeanCvN => {
                let cv = subcells.get(1)?.active_number()?;
                if !cv.is_finite() || cv < 0.0 || center == 0.0 {
                    return None;
                }
                let sd = (cv / 100.0) * center.abs();
                let lower = center - sd;
                let upper = center + sd;
                (sd.is_finite() && lower.is_finite() && upper.is_finite()).then_some(ExplicitErrorBounds { center, lower, upper })
            }
            Self::MeanLowHigh => {
                let down = subcells.get(1)?.active_number()?;
                let up = subcells.get(2)?.active_number()?;
                if !down.is_finite() || !up.is_finite() || down < 0.0 || up < 0.0 {
                    return None;
                }
                let lower = center - down;
                let upper = center + up;
                (lower.is_finite() && upper.is_finite()).then_some(ExplicitErrorBounds { center, lower, upper })
            }
            Self::UpperLowerLimits | Self::MedianIqr => {
                let low = subcells.get(1)?.active_number()?;
                let high = subcells.get(2)?.active_number()?;
                if !low.is_finite() || !high.is_finite() {
                    return None;
                }
                let (lower, upper) = if low <= high { (low, high) } else { (high, low) };
                Some(ExplicitErrorBounds { center, lower, upper })
            }
        }
    }
}

/// Plottable `(center, lower_endpoint, upper_endpoint)` resolved from pre-summarized subcolumns.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExplicitErrorBounds {
    /// Central point location in data-axis units (`Mean` for parametric/interval modes or `Median` for [`SubcolumnMode::MedianIqr`]).
    pub center: f64,
    /// Lower error-bar endpoint in data-axis units (`center - SD`, `center - SEM`, `center - (%CV/100)*|center|`, `center - (-Error)`, `LowerLimit`, or `Q1`).
    pub lower: f64,
    /// Upper error-bar endpoint in data-axis units (`center + SD`, `center + SEM`, `center + (%CV/100)*|center|`, `center + (+Error)`, `UpperLimit`, or `Q3`).
    pub upper: f64,
}

/// Visual layout of replicate subcolumns in a `Grouped` table (`docs/implementation/data-tables-and-transforms.md` §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ReplicateStacking {
    /// Display replicate subcolumns (`Y1 .. Yn`) horizontally side-by-side within each row (`row = factor A level`, `subcolumn = subject/replicate`).
    #[default]
    SideBySide,
    /// Display replicates stacked vertically across consecutive rows under a single column.
    Stacked,
}

/// Rigorously reconstructed parametric summary observation from `MeanSdN`, `MeanSemN`, or
/// `MeanCvN` subcolumns (`docs/implementation/data-tables-and-transforms.md` §2.1):
///
/// `SS_within = (n - 1) * SD^2`, where `SD = SEM * sqrt(n)` or `(%CV / 100) * |Mean|`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SummarizedPoint {
    /// Sample arithmetic mean in data units.
    pub mean: f64,
    /// Reconstructed sample standard deviation (`>= 0`, finite) in data units (`SD`, `SEM * sqrt(n)`, or `(%CV / 100) * |mean|`).
    pub sd: f64,
    /// Integer replicate count `n >= 1.0` stored as `f64` for exact moment algebra.
    pub n: f64,
    /// Reconstructed within-cell sum of squared deviations `(n - 1) * sd^2` (`0.0` when `n == 1.0`).
    pub ss_within: f64,
    /// Within-cell degrees of freedom `n - 1.0` (`>= 0.0`).
    pub df_within: f64,
}

impl SummarizedPoint {
    /// Reconstruct `(mean, sd, n, ss_within, df_within)` from a 3-cell subcolumn slice.
    ///
    /// Returns `Ok(None)` when the mean cell is empty or excluded. Flags missing `N`, `N < 1`,
    /// fractional `N`, negative/non-finite spread, undefined `%CV` at `mean == 0`, or overflowing
    /// `sd`/`ss_within` as explicit [`SummaryPointError`] diagnostics rather than silently assuming `N = 1`.
    pub fn reconstruct(mode: SubcolumnMode, subcells: &[Cell]) -> Result<Option<Self>, SummaryPointError> {
        if !mode.supports_parametric_moments() {
            return Err(SummaryPointError::NotParametricSummaryMode { mode });
        }
        let Some(mean) = subcells.first().and_then(Cell::active_number) else {
            return Ok(None);
        };
        let spread = subcells.get(1).and_then(Cell::active_number).ok_or(SummaryPointError::MissingSpread)?;
        if !spread.is_finite() || spread < 0.0 {
            return Err(SummaryPointError::NegativeSpread { spread });
        }
        let n = subcells.get(2).and_then(Cell::active_number).ok_or(SummaryPointError::MissingSampleSize)?;
        if !n.is_finite() || n < 1.0 || n.fract() != 0.0 {
            return Err(SummaryPointError::SampleSizeBelowOne { n });
        }
        let sd = match mode {
            SubcolumnMode::MeanSdN => spread,
            SubcolumnMode::MeanSemN => spread * n.sqrt(),
            SubcolumnMode::MeanCvN => {
                if mean == 0.0 {
                    return Err(SummaryPointError::MissingSpread);
                }
                (spread / 100.0) * mean.abs()
            }
            _ => return Err(SummaryPointError::NotParametricSummaryMode { mode }),
        };
        let df_within = n - 1.0;
        let sd_sq = sd * sd;
        let ss_within = df_within * sd_sq;
        if !sd.is_finite() || !sd_sq.is_finite() || !df_within.is_finite() || !ss_within.is_finite() || ss_within < 0.0 {
            return Err(SummaryPointError::NegativeSpread { spread });
        }
        Ok(Some(Self { mean, sd, n, ss_within, df_within }))
    }
}
