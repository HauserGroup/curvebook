//! Locale-aware numeric formatting for scientific axis ticks and *p*-value labels.

use lumencraft_math::{AxisScale, PValueFormat};
use serde::{Deserialize, Serialize};

/// Axis numeric tick formatting style (`docs/graphing-and-layouts.md` §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AxisTickFormat {
    #[default]
    Decimal,
    PowerOfTenSuperscript,
    ScientificE,
    Antilog,
    EngineeringSi,
}

/// Numeric formatting policy controlling decimal separator, digit grouping, and minus sign.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocaleNumberPolicy {
    pub decimal_separator: char,
    pub thousands_separator: Option<char>,
    pub minus_sign: String,
}

impl LocaleNumberPolicy {
    /// Standard publication policy using Unicode minus (`U+2212`) and period decimals.
    #[must_use]
    pub fn publication() -> Self {
        Self { decimal_separator: '.', thousands_separator: None, minus_sign: String::from("\u{2212}") }
    }

    /// Publication English locale policy with comma thousands grouping and Unicode minus (`U+2212`).
    #[must_use]
    pub fn english_grouped() -> Self {
        Self { decimal_separator: '.', thousands_separator: Some(','), minus_sign: String::from("\u{2212}") }
    }

    /// European decimal-comma policy with dot thousands grouping and Unicode minus (`U+2212`).
    #[must_use]
    pub fn european() -> Self {
        Self { decimal_separator: ',', thousands_separator: Some('.'), minus_sign: String::from("\u{2212}") }
    }

    /// Return a copy of this policy with the specified thousands grouping separator.
    #[must_use]
    pub fn with_thousands_separator(mut self, thousands_separator: Option<char>) -> Self {
        self.thousands_separator = thousands_separator;
        self
    }
}

impl Default for LocaleNumberPolicy {
    fn default() -> Self {
        Self { decimal_separator: '.', thousands_separator: None, minus_sign: String::from("-") }
    }
}

/// Format an axis tick value according to its [`AxisScale`] and [`LocaleNumberPolicy`].
#[must_use]
pub fn format_axis_tick(value: f64, scale: AxisScale, policy: &LocaleNumberPolicy) -> String {
    if let Some(special) = format_non_finite(value, policy) {
        return special;
    }
    let clean = if value == 0.0 { 0.0 } else { value };
    let abs = clean.abs();
    let style = match scale {
        AxisScale::Log10 => {
            if abs > 0.0 && !(1e-3..1e5).contains(&abs) {
                AxisTickFormat::PowerOfTenSuperscript
            } else {
                AxisTickFormat::Decimal
            }
        }
        AxisScale::Log2 | AxisScale::Ln => {
            if abs > 0.0 && !(1e-3..1e5).contains(&abs) {
                AxisTickFormat::ScientificE
            } else {
                AxisTickFormat::Decimal
            }
        }
        AxisScale::Linear | AxisScale::Logit(_) | AxisScale::Probit | AxisScale::Reciprocal => {
            if abs > 0.0 && !(1e-4..1e6).contains(&abs) {
                AxisTickFormat::ScientificE
            } else {
                AxisTickFormat::Decimal
            }
        }
    };
    format_axis_tick_with_format(clean, scale, style, policy)
}

/// Format an axis tick value using an explicit [`AxisTickFormat`] and [`LocaleNumberPolicy`].
#[must_use]
pub fn format_axis_tick_with_format(value: f64, _scale: AxisScale, tick_format: AxisTickFormat, policy: &LocaleNumberPolicy) -> String {
    if let Some(special) = format_non_finite(value, policy) {
        return special;
    }
    let clean = if value == 0.0 { 0.0 } else { value };
    if clean == 0.0 {
        return String::from("0");
    }
    match tick_format {
        AxisTickFormat::Decimal => format_decimal_value(clean, 10, -9, 12, policy),
        AxisTickFormat::Antilog => format_decimal_value(clean, 10, -12, 12, policy),
        AxisTickFormat::PowerOfTenSuperscript => format_power_of_ten(clean, policy),
        AxisTickFormat::ScientificE => format_scientific_e(clean, policy),
        AxisTickFormat::EngineeringSi => format_engineering_si(clean, policy),
    }
}

/// Format a raw `p`-value probability into a publication string (optionally with `P = ` prefix and stars).
#[must_use]
pub fn format_pvalue_label(p: f64, format: PValueFormat, include_prefix: bool, include_stars: bool, policy: &LocaleNumberPolicy) -> String {
    let raw = if p.is_finite() && (0.0..=1.0).contains(&p) {
        let clean_p = if p == 0.0 { 0.0 } else { p };
        format.format_p(clean_p)
    } else {
        String::from("NaN")
    };
    let formatted_num = apply_policy_simple(&raw, policy);
    let mut out = String::new();
    if include_prefix {
        if formatted_num.starts_with('<') || formatted_num.starts_with('>') {
            out.push_str("P ");
            out.push_str(&formatted_num);
        } else {
            out.push_str("P = ");
            out.push_str(&formatted_num);
        }
    } else {
        out.push_str(&formatted_num);
    }

    if include_stars && p.is_finite() && (0.0..=1.0).contains(&p) {
        let stars = format.stars(p);
        out.push(' ');
        out.push_str(stars);
    }
    out
}

fn format_non_finite(value: f64, policy: &LocaleNumberPolicy) -> Option<String> {
    if value.is_nan() {
        return Some(String::from("NaN"));
    }
    if value.is_infinite() {
        return if value.is_sign_negative() { Some(format!("{}Inf", policy.minus_sign)) } else { Some(String::from("Inf")) };
    }
    None
}

fn decompose_scientific_abs(abs: f64) -> Option<(String, i32)> {
    let sci = format!("{abs:.9e}");
    let (mantissa, exp_str) = sci.split_once('e')?;
    let exp = exp_str.parse::<i32>().ok()?;
    let mut digits = String::with_capacity(mantissa.len());
    for ch in mantissa.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        }
    }
    while digits.len() > 1 && digits.ends_with('0') {
        digits.pop();
    }
    if digits.is_empty() {
        digits.push('0');
    }
    Some((digits, exp))
}

fn format_decimal_value(value: f64, _sig_digits: usize, min_fixed_exp: i32, max_fixed_exp: i32, policy: &LocaleNumberPolicy) -> String {
    let is_neg = value.is_sign_negative();
    let abs = value.abs();
    let Some((digits, exp)) = decompose_scientific_abs(abs) else {
        return String::from("NaN");
    };
    if exp < min_fixed_exp || exp > max_fixed_exp {
        return format_scientific_e(value, policy);
    }

    let (int_digits, frac_digits) = place_decimal_digits(&digits, exp);
    assemble_decimal(is_neg, &int_digits, &frac_digits, policy)
}

fn place_decimal_digits(digits: &str, exp: i32) -> (String, String) {
    if exp >= 0 {
        let int_len = (exp as usize) + 1;
        if digits.len() <= int_len {
            let mut int_part = String::with_capacity(int_len);
            int_part.push_str(digits);
            for _ in digits.len()..int_len {
                int_part.push('0');
            }
            (int_part, String::new())
        } else {
            (digits[..int_len].to_owned(), digits[int_len..].to_owned())
        }
    } else {
        let leading_zeros = ((-exp) as usize).saturating_sub(1);
        let mut frac = String::with_capacity(leading_zeros + digits.len());
        for _ in 0..leading_zeros {
            frac.push('0');
        }
        frac.push_str(digits);
        (String::from("0"), frac)
    }
}

fn assemble_decimal(is_neg: bool, int_digits: &str, frac_digits: &str, policy: &LocaleNumberPolicy) -> String {
    if int_digits == "0" && frac_digits.is_empty() {
        return String::from("0");
    }
    let mut out = String::with_capacity(int_digits.len() + frac_digits.len() + 8);
    if is_neg {
        out.push_str(&policy.minus_sign);
    }
    push_grouped_integer(&mut out, int_digits, policy.thousands_separator);
    if !frac_digits.is_empty() {
        out.push(policy.decimal_separator);
        out.push_str(frac_digits);
    }
    out
}

fn push_grouped_integer(out: &mut String, int_digits: &str, separator: Option<char>) {
    let Some(sep) = separator else {
        out.push_str(int_digits);
        return;
    };
    let len = int_digits.len();
    if len <= 3 {
        out.push_str(int_digits);
        return;
    }
    for (idx, ch) in int_digits.chars().enumerate() {
        if idx > 0 && (len - idx).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(ch);
    }
}

fn format_power_of_ten(value: f64, policy: &LocaleNumberPolicy) -> String {
    let is_neg = value.is_sign_negative();
    let abs = value.abs();
    let Some((digits, exp)) = decompose_scientific_abs(abs) else {
        return String::from("NaN");
    };
    let mut out = String::new();
    if is_neg {
        out.push_str(&policy.minus_sign);
    }
    if digits != "1" {
        let (int_digits, frac_digits) = place_decimal_digits(&digits, 0);
        out.push_str(&int_digits);
        if !frac_digits.is_empty() {
            out.push(policy.decimal_separator);
            out.push_str(&frac_digits);
        }
        out.push('\u{00D7}');
    }
    out.push_str("10^{");
    if exp < 0 {
        out.push_str(&policy.minus_sign);
        out.push_str(&exp.unsigned_abs().to_string());
    } else {
        out.push_str(&exp.to_string());
    }
    out.push('}');
    out
}

fn format_scientific_e(value: f64, policy: &LocaleNumberPolicy) -> String {
    let is_neg = value.is_sign_negative();
    let abs = value.abs();
    let Some((digits, exp)) = decompose_scientific_abs(abs) else {
        return String::from("NaN");
    };
    let (int_digits, frac_digits) = place_decimal_digits(&digits, 0);
    let mut out = String::new();
    if is_neg {
        out.push_str(&policy.minus_sign);
    }
    out.push_str(&int_digits);
    if !frac_digits.is_empty() {
        out.push(policy.decimal_separator);
        out.push_str(&frac_digits);
    }
    out.push('e');
    if exp < 0 {
        out.push_str(&policy.minus_sign);
        out.push_str(&exp.unsigned_abs().to_string());
    } else {
        out.push_str(&exp.to_string());
    }
    out
}

fn format_engineering_si(value: f64, policy: &LocaleNumberPolicy) -> String {
    let is_neg = value.is_sign_negative();
    let abs = value.abs();
    let Some((digits, exp)) = decompose_scientific_abs(abs) else {
        return String::from("NaN");
    };
    let exp3 = exp.div_euclid(3) * 3;
    let shift = exp - exp3;
    let (int_digits, frac_digits) = place_decimal_digits(&digits, shift);
    let mut out = assemble_decimal(is_neg, &int_digits, &frac_digits, policy);
    if let Some(prefix) = si_prefix(exp3) {
        out.push_str(prefix);
    } else if exp3 < 0 {
        out.push('e');
        out.push_str(&policy.minus_sign);
        out.push_str(&exp3.unsigned_abs().to_string());
    } else {
        out.push('e');
        out.push_str(&exp3.to_string());
    }
    out
}

const fn si_prefix(exp3: i32) -> Option<&'static str> {
    match exp3 {
        30 => Some("Q"),
        27 => Some("R"),
        24 => Some("Y"),
        21 => Some("Z"),
        18 => Some("E"),
        15 => Some("P"),
        12 => Some("T"),
        9 => Some("G"),
        6 => Some("M"),
        3 => Some("k"),
        0 => Some(""),
        -3 => Some("m"),
        -6 => Some("\u{03BC}"),
        -9 => Some("n"),
        -12 => Some("p"),
        -15 => Some("f"),
        -18 => Some("a"),
        -21 => Some("z"),
        -24 => Some("y"),
        -27 => Some("r"),
        -30 => Some("q"),
        _ => None,
    }
}

fn apply_policy_simple(raw: &str, policy: &LocaleNumberPolicy) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    for ch in raw.chars() {
        match ch {
            '-' => out.push_str(&policy.minus_sign),
            '.' => out.push(policy.decimal_separator),
            _ => out.push(ch),
        }
    }
    out
}
