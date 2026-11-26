//! Canonical TeX-subset equation formatting for fitted linear, polynomial,
//! built-in nonlinear, and custom DSL models (`R9.16` / `R10.19`).
//!
//! Emits clean strings compatible with two-dimensional mathematical typography
//! (`\frac{}{}`, `\sqrt{}`, `\exp`, `\ln`, `\log`, `\sin`, `\cos`, `\tan`,
//! `\sinh`, `\cosh`, `\tanh`, `\arcsin`, `\arccos`, `\arctan`, `\erf`, `\erfc`,
//! `\min`, `\max`, `\cdot`, `\times`, `\text{}`, uppercase and lowercase Greek
//! escapes, fractional powers, and braced `^{}`/`_{}`).

use std::collections::{HashMap, HashSet};

use crate::dsl::{
    BinaryOp, DatasetFilter, DslSection, Expr, MAX_DSL_LINES, MAX_DSL_SOURCE_BYTES, MAX_TOTAL_DATASET_INDICES, UnaryOp, is_valid_ident,
    parse_dataset_prefix, parse_expr_collecting_locs, parse_section_header, split_top_level_assignment,
};
use crate::library::normalize_id;
use crate::types::per_dataset_parameter_key;

/// Formats a floating-point coefficient with `sig_figs` significant figures.
///
/// Avoids `-0` / `-0.0`, uses clean decimals for numbers in `[0.0001, 10^6)`,
/// uses `m \times 10^{e}` outside that range, and renders non-finite values
/// using TeX symbols (`\text{NaN}`, `\infty`, `-\infty`).
pub fn format_number_sig_figs(value: f64, sig_figs: usize) -> String {
    if value.is_nan() {
        return "\\text{NaN}".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() { "-\\infty".to_string() } else { "\\infty".to_string() };
    }
    if value == 0.0 {
        return "0".to_string();
    }

    let figs = sig_figs.clamp(1, 15);
    let negative = value.is_sign_negative();
    let abs = value.abs();
    let sci = format!("{abs:.prec$e}", prec = figs - 1);
    let Some((mantissa_raw, exp_raw)) = sci.split_once('e') else {
        return abs.to_string();
    };
    let exp: i32 = exp_raw.parse().unwrap_or(0);
    let digits: String = mantissa_raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.chars().all(|c| c == '0') {
        return "0".to_string();
    }

    let prefix = if negative { "-" } else { "" };
    if (-4..6).contains(&exp) {
        let body = format_decimal_from_digits(&digits, exp);
        return if body == "0" { body } else { format!("{prefix}{body}") };
    }

    let mantissa = if let Some((head, tail)) = digits.split_at_checked(1)
        && !tail.is_empty()
    {
        trim_decimal_trailing_zeros(format!("{head}.{tail}"))
    } else {
        digits
    };
    format!("{prefix}{mantissa} \\times 10^{{{exp}}}")
}

fn format_decimal_from_digits(digits: &str, exp: i32) -> String {
    if exp < 0 {
        let zeros = "0".repeat((-exp - 1) as usize);
        return trim_decimal_trailing_zeros(format!("0.{zeros}{digits}"));
    }
    let pos = (exp + 1) as usize;
    if pos >= digits.len() {
        let zeros = "0".repeat(pos - digits.len());
        return format!("{digits}{zeros}");
    }
    let (head, tail) = digits.split_at_checked(pos).unwrap_or((digits, ""));
    trim_decimal_trailing_zeros(format!("{head}.{tail}"))
}

fn trim_decimal_trailing_zeros(mut s: String) -> String {
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    s
}

/// Formats a linear or polynomial equation in canonical TeX-subset notation.
///
/// - Symbolic linear (`order <= 1`): `Y = \text{Slope} \cdot X + \text{Intercept}`
///   (or `Y = \text{Slope} \cdot X` when `force_through_origin`).
/// - Symbolic quadratic/polynomial (`order >= 2`): `Y = b_0 + b_1 \cdot X + b_2 \cdot X^2 + \dots`.
/// - Substituted: inserts formatted numbers and normalizes sign artifacts
///   (`+ -2.5` -> `- 2.5`, `- -2.5` -> `+ 2.5`).
pub fn format_polynomial_equation_tex(
    order: usize,
    force_through_origin: bool,
    slope_intercept_or_coeffs: &[(String, f64)],
    substituted: bool,
    sig_figs: usize,
) -> String {
    let eff_substituted = substituted && !slope_intercept_or_coeffs.is_empty();
    if order <= 1 {
        let raw = format_linear_equation_tex(force_through_origin, slope_intercept_or_coeffs, eff_substituted, sig_figs);
        return normalize_tex_signs(&raw);
    }

    let start = usize::from(force_through_origin);
    let max_order = order.min(32);
    let mut terms = Vec::with_capacity(max_order.saturating_sub(start) + 1);
    for k in start..=max_order {
        let term = if eff_substituted {
            let coeff = lookup_poly_coeff(slope_intercept_or_coeffs, k, order, force_through_origin);
            let num = format_number_sig_figs(coeff, sig_figs);
            poly_term_tex(&num, k)
        } else {
            let sym = sub_token("b", k);
            poly_term_tex(&sym, k)
        };
        terms.push(term);
    }
    normalize_tex_signs(&format!("Y = {}", terms.join(" + ")))
}

/// Formats a [`curvebook_doc::LinearRegressionSpec`] equation in canonical TeX-subset notation,
/// preserving the Y-intercept whenever a non-origin `fixed_point` takes precedence over
/// `force_through_origin` or when `deming_model_ii` is active (`AUD-13-18`).
#[must_use]
pub fn format_linear_regression_equation_tex(
    spec: &curvebook_doc::LinearRegressionSpec,
    params: &[(String, f64)],
    substituted: bool,
    sig_figs: usize,
) -> String {
    let through_origin = !spec.deming_model_ii && matches!(spec.effective_fixed_point(), Some((x0, y0)) if x0 == 0.0 && y0 == 0.0);
    format_polynomial_equation_tex(1, through_origin, params, substituted, sig_figs)
}

fn format_linear_equation_tex(force_through_origin: bool, params: &[(String, f64)], substituted: bool, sig_figs: usize) -> String {
    if !substituted || params.is_empty() {
        return if force_through_origin {
            "Y = \\text{Slope} \\cdot X".to_string()
        } else {
            "Y = \\text{Slope} \\cdot X + \\text{Intercept}".to_string()
        };
    }

    let (slope, intercept) = extract_linear_params(params, force_through_origin);
    let slope_str = slope.map_or_else(|| "\\text{Slope}".to_string(), |v| format_number_sig_figs(v, sig_figs));
    if force_through_origin {
        if let Some(b) = intercept.filter(|v| v.is_finite() && *v != 0.0) {
            let intercept_str = format_number_sig_figs(b, sig_figs);
            format!("Y = {slope_str} \\cdot X + {intercept_str}")
        } else {
            format!("Y = {slope_str} \\cdot X")
        }
    } else {
        let intercept_str = intercept.map_or_else(|| "\\text{Intercept}".to_string(), |v| format_number_sig_figs(v, sig_figs));
        format!("Y = {slope_str} \\cdot X + {intercept_str}")
    }
}

fn is_linear_slope_key(key: &str) -> bool {
    matches!(key, "slope" | "m" | "b1" | "b_1" | "a1" | "beta1")
}

fn is_linear_intercept_key(key: &str) -> bool {
    matches!(key, "yintercept" | "y_intercept" | "intercept" | "b0" | "b_0" | "a0" | "beta0" | "c")
}

fn extract_linear_params(params: &[(String, f64)], force_through_origin: bool) -> (Option<f64>, Option<f64>) {
    let mut slope: Option<(usize, f64)> = None;
    let mut intercept: Option<(usize, f64)> = None;
    for (idx, (name, val)) in params.iter().enumerate() {
        let key = normalize_id(name);
        if is_linear_slope_key(&key) {
            slope.get_or_insert((idx, *val));
        } else if is_linear_intercept_key(&key) {
            intercept.get_or_insert((idx, *val));
        }
    }
    let s = slope
        .map(|(_, v)| v)
        .or_else(|| params.iter().enumerate().find(|(idx, _)| intercept.is_none_or(|(i_idx, _)| *idx != i_idx)).map(|(_, (_, v))| *v));
    let b = if force_through_origin {
        // Preserve an explicit non-zero intercept (e.g. non-origin `fixed_point` line) instead of
        // silently truncating `Y = m*X + b` to `Y = m*X` (`AUD-13-18`).
        intercept.map(|(_, v)| v).filter(|v| v.is_finite() && *v != 0.0)
    } else {
        intercept.map(|(_, v)| v).or_else(|| {
            let s_idx = slope
                .map(|(i, _)| i)
                .or_else(|| params.iter().enumerate().find(|(idx, _)| intercept.is_none_or(|(i_idx, _)| *idx != i_idx)).map(|(idx, _)| idx));
            params.iter().enumerate().find(|(idx, _)| Some(*idx) != s_idx).map(|(_, (_, v))| *v)
        })
    };
    (s, b)
}

fn lookup_poly_coeff(coeffs: &[(String, f64)], power: usize, order: usize, force_through_origin: bool) -> f64 {
    let candidates =
        [format!("b{power}"), format!("b_{power}"), format!("a{power}"), format!("a_{power}"), format!("beta{power}"), format!("c{power}")];
    for (name, val) in coeffs {
        let key = normalize_id(name);
        if candidates.iter().any(|c| c == &key)
            || (power == 0 && matches!(key.as_str(), "intercept" | "yintercept" | "y_intercept"))
            || (power == 1 && matches!(key.as_str(), "slope"))
        {
            return *val;
        }
    }
    let idx = if force_through_origin && coeffs.len() == order { power - 1 } else { power };
    coeffs.get(idx).map_or(0.0, |(_, v)| *v)
}

fn poly_term_tex(coeff: &str, power: usize) -> String {
    match power {
        0 => coeff.to_string(),
        1 => format!("{coeff} \\cdot X"),
        _ => format!("{coeff} \\cdot {}", pow_token("X", power)),
    }
}

fn sub_token(base: &str, idx: usize) -> String {
    if idx < 10 { format!("{base}_{idx}") } else { format!("{base}_{{{idx}}}") }
}

fn pow_token(base: &str, exp: usize) -> String {
    if exp < 10 { format!("{base}^{exp}") } else { format!("{base}^{{{exp}}}") }
}

/// Formats any built-in equation ID or custom DSL formula as canonical TeX-subset markup.
pub fn format_model_equation_tex(
    equation_id: &str,
    custom_dsl: Option<&str>,
    params: &[(String, f64)],
    substituted: bool,
    sig_figs: usize,
) -> String {
    format_model_equation_tex_for_dataset(equation_id, custom_dsl, params, None, substituted, sig_figs)
}

/// Formats any built-in equation ID or custom DSL formula for a specific optional `data_set_index`
/// (`<A>`, `<B>`, per-dataset constants such as `AntagConc [DS1]` / `HotNM [DS1]`).
pub fn format_model_equation_tex_for_dataset(
    equation_id: &str,
    custom_dsl: Option<&str>,
    params: &[(String, f64)],
    data_set_index: Option<usize>,
    substituted: bool,
    sig_figs: usize,
) -> String {
    if let Some(dsl_src) = custom_dsl {
        let trimmed = dsl_src.trim();
        if !trimmed.is_empty() {
            return format_custom_dsl_tex(trimmed, params, data_set_index, substituted, sig_figs);
        }
    }

    let id = normalize_id(equation_id);
    let ctx = EqCtx { params, data_set_index, substituted, sig_figs };

    let lookup_builtin = |candidate: &str| {
        format_pharma_equation_tex(candidate, &ctx)
            .or_else(|| format_curve_equation_tex(candidate, &ctx))
            .or_else(|| format_nist_equation_tex(candidate, &ctx))
    };
    let raw = lookup_builtin(&id)
        .or_else(|| {
            crate::library::get_builtin_model(equation_id).and_then(|model| {
                let canonical = normalize_id(model.id());
                lookup_builtin(&canonical)
            })
        })
        .unwrap_or_else(|| format_unknown_fallback(equation_id, params, substituted, sig_figs));

    normalize_tex_signs(&raw)
}

fn format_unknown_fallback(equation_id: &str, params: &[(String, f64)], substituted: bool, sig_figs: usize) -> String {
    if params.is_empty() {
        let id = normalize_id(equation_id);
        if id.is_empty() {
            return "Y = f(X)".to_string();
        }
        let sub = if contains_tex_meta(&id) || id.contains('_') { format!("\\text{{{}}}", escape_tex_text(&id)) } else { id };
        return format!("Y = f_{{{sub}}}(X)");
    }
    let mut parts = Vec::with_capacity(params.len());
    for (name, val) in params {
        let pname = tex_ident(name);
        if substituted {
            let v = format_number_sig_figs(*val, sig_figs);
            parts.push(format!("{pname} = {v}"));
        } else {
            parts.push(pname);
        }
    }
    format!("Y = f(X; {})", parts.join(", "))
}

struct EqCtx<'a> {
    params: &'a [(String, f64)],
    data_set_index: Option<usize>,
    substituted: bool,
    sig_figs: usize,
}

impl<'a> EqCtx<'a> {
    fn find(&self, name: &str) -> Option<f64> {
        if let Some(ds_idx) = self.data_set_index {
            let ds_target = normalize_id(&per_dataset_parameter_key(name, ds_idx));
            for (k, v) in self.params {
                if normalize_id(k) == ds_target {
                    return Some(*v);
                }
            }
        }
        for (k, v) in self.params {
            if k == name {
                return Some(*v);
            }
        }
        let target = normalize_id(name);
        for (k, v) in self.params {
            if normalize_id(k) == target {
                return Some(*v);
            }
        }
        None
    }

    fn val(&self, name: &str, sym: &str) -> String {
        if !self.substituted {
            return sym.to_string();
        }
        match self.find(name) {
            Some(v) => format_number_sig_figs(v, self.sig_figs),
            None => sym.to_string(),
        }
    }

    fn factor(&self, name: &str, sym: &str) -> String {
        let s = self.val(name, sym);
        if self.substituted && s.starts_with('-') { format!("({s})") } else { s }
    }

    fn denom(&self, name: &str, sym: &str) -> String {
        let s = self.val(name, sym);
        if self.substituted && (s.starts_with('-') || s.contains("\\times")) { format!("({s})") } else { s }
    }

    fn pow_base(&self, name: &str, sym: &str) -> String {
        let s = self.val(name, sym);
        if self.substituted && (s.starts_with('-') || s.contains("\\times") || s.contains(' ')) { format!("({s})") } else { s }
    }

    fn num(&self, v: f64) -> String {
        format_number_sig_figs(v, self.sig_figs)
    }

    fn factor_num(&self, v: f64) -> String {
        let s = self.num(v);
        if s.starts_with('-') { format!("({s})") } else { s }
    }
}

fn format_pharma_equation_tex(id: &str, ctx: &EqCtx<'_>) -> Option<String> {
    format_dose_response_tex(id, ctx).or_else(|| format_binding_kinetics_tex(id, ctx)).or_else(|| format_enzyme_kinetics_tex(id, ctx))
}

fn format_dose_response_tex(id: &str, ctx: &EqCtx<'_>) -> Option<String> {
    let s = match id {
        "log_agonist_vs_response_3pl" | "log_agonist_vs_response" | "ec50_3pl" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + 10^{{{} - X}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("LogEC50", "\\log\\text{EC}_{50}"),
        ),
        "log_agonist_vs_response_4pl" | "log_agonist_vs_response_variable_slope" | "4pl" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + 10^{{({} - X) \\cdot {}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("LogEC50", "\\log\\text{EC}_{50}"),
            ctx.factor("HillSlope", "\\text{HillSlope}"),
        ),
        "log_agonist_vs_response_normalized" => {
            format!("Y = \\frac{{100}}{{1 + 10^{{{} - X}}}}", ctx.val("LogEC50", "\\log\\text{EC}_{50}"))
        }
        "log_agonist_vs_response_normalized_var" => format!(
            "Y = \\frac{{100}}{{1 + 10^{{({} - X) \\cdot {}}}}}",
            ctx.val("LogEC50", "\\log\\text{EC}_{50}"),
            ctx.factor("HillSlope", "\\text{HillSlope}"),
        ),
        "agonist_vs_response_3pl" | "agonist_vs_response" => format!(
            "Y = {} + \\frac{{({} - {}) \\cdot X}}{{{} + X}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("EC50", "\\text{EC}_{50}"),
        ),
        "agonist_vs_response_4pl" => {
            let h = ctx.val("HillSlope", "\\text{HillSlope}");
            format!(
                "Y = {} + \\frac{{({} - {}) \\cdot X^{{{h}}}}}{{{}^{{{h}}} + X^{{{h}}}}}",
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.val("Top", "\\text{Top}"),
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.pow_base("EC50", "\\text{EC}_{50}"),
            )
        }
        "agonist_vs_response_normalized" => {
            format!("Y = \\frac{{100 \\cdot X}}{{{} + X}}", ctx.val("EC50", "\\text{EC}_{50}"))
        }
        "agonist_vs_response_normalized_var" => {
            format!("Y = \\frac{{100}}{{1 + ({} / X)^{{{}}}}}", ctx.val("EC50", "\\text{EC}_{50}"), ctx.val("HillSlope", "\\text{HillSlope}"),)
        }
        "log_agonist_vs_response_5pl" | "5pl" => format!(
            "Y = {} + \\frac{{{} - {}}}{{(1 + 10^{{({} - X) \\cdot {}}})^{{{}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("LogXb", "\\log X_b"),
            ctx.factor("HillSlope", "\\text{HillSlope}"),
            ctx.val("S", "S"),
        ),
        "agonist_vs_response_5pl" => {
            let s_val = ctx.val("S", "S");
            format!(
                "Y = {} + \\frac{{{} - {}}}{{(1 + (2^{{1 / {s_val}}} - 1) \\cdot ({} / X)^{{{}}})^{{{s_val}}}}}",
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.val("Top", "\\text{Top}"),
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.val("EC50", "\\text{EC}_{50}"),
                ctx.val("HillSlope", "\\text{HillSlope}"),
            )
        }
        "bell_shaped_dose_response" => {
            let dip = ctx.val("Dip", "\\text{Dip}");
            format!(
                "Y = {dip} + \\frac{{{} - {dip}}}{{1 + 10^{{(X - {}) \\cdot {}}}}} + \\frac{{{} - {dip}}}{{1 + 10^{{({} - X) \\cdot {}}}}}",
                ctx.val("Plateau1", "\\text{Plateau}_1"),
                ctx.val("LogEC50_1", "\\log\\text{EC}_{50,1}"),
                ctx.factor("nH1", "n_{H1}"),
                ctx.val("Plateau2", "\\text{Plateau}_2"),
                ctx.val("LogEC50_2", "\\log\\text{EC}_{50,2}"),
                ctx.factor("nH2", "n_{H2}"),
            )
        }
        "bell_shaped_dose_response_conc" => {
            let dip = ctx.val("Dip", "\\text{Dip}");
            format!(
                "Y = {dip} + \\frac{{{} - {dip}}}{{1 + (X / {})^{{{}}}}} + \\frac{{{} - {dip}}}{{1 + ({} / X)^{{{}}}}}",
                ctx.val("Plateau1", "\\text{Plateau}_1"),
                ctx.denom("EC50_1", "\\text{EC}_{50,1}"),
                ctx.val("nH1", "n_{H1}"),
                ctx.val("Plateau2", "\\text{Plateau}_2"),
                ctx.val("EC50_2", "\\text{EC}_{50,2}"),
                ctx.val("nH2", "n_{H2}"),
            )
        }
        "biphasic_dose_response" => {
            let bot = ctx.val("Bottom", "\\text{Bottom}");
            let top = ctx.val("Top", "\\text{Top}");
            let frac = ctx.val("Frac", "\\text{Frac}");
            format!(
                "Y = {bot} + ({top} - {bot}) \\cdot (\\frac{{{frac}}}{{1 + 10^{{({} - X) \\cdot {}}}}} + \\frac{{1 - {frac}}}{{1 + 10^{{({} - X) \\cdot {}}}}})",
                ctx.val("LogEC50_1", "\\log\\text{EC}_{50,1}"),
                ctx.factor("nH1", "n_{H1}"),
                ctx.val("LogEC50_2", "\\log\\text{EC}_{50,2}"),
                ctx.factor("nH2", "n_{H2}"),
            )
        }
        "biphasic_dose_response_conc" => {
            let bot = ctx.val("Bottom", "\\text{Bottom}");
            let top = ctx.val("Top", "\\text{Top}");
            let frac = ctx.val("Frac", "\\text{Frac}");
            format!(
                "Y = {bot} + ({top} - {bot}) \\cdot (\\frac{{{frac}}}{{1 + ({} / X)^{{{}}}}} + \\frac{{1 - {frac}}}{{1 + ({} / X)^{{{}}}}})",
                ctx.val("EC50_1", "\\text{EC}_{50,1}"),
                ctx.val("nH1", "n_{H1}"),
                ctx.val("EC50_2", "\\text{EC}_{50,2}"),
                ctx.val("nH2", "n_{H2}"),
            )
        }
        "log_agonist_vs_response_ecf" | "ecf" => {
            let f = ctx.val("F", "F");
            let f_den = ctx.denom("F", "F");
            format!(
                "Y = {} + \\frac{{{} - {}}}{{1 + \\frac{{100 - {f}}}{{{f_den}}} \\cdot 10^{{({} - X) \\cdot {}}}}}",
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.val("Top", "\\text{Top}"),
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.val("LogECF", "\\log\\text{EC}_F"),
                ctx.factor("HillSlope", "\\text{HillSlope}"),
            )
        }
        "agonist_vs_response_ecf" => {
            let f = ctx.val("F", "F");
            let f_den = ctx.denom("F", "F");
            format!(
                "Y = {} + \\frac{{{} - {}}}{{1 + \\frac{{100 - {f}}}{{{f_den}}} \\cdot ({} / X)^{{{}}}}}",
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.val("Top", "\\text{Top}"),
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.val("ECF", "\\text{EC}_F"),
                ctx.val("HillSlope", "\\text{HillSlope}"),
            )
        }
        "ec50_shift_gaddum_schild" | "gaddum_schild" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + 10^{{({} + \\log(1 + ({} \\cdot 10^{{{}}})^{{{}}}) - X) \\cdot {}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("LogEC50", "\\log\\text{EC}_{50}"),
            ctx.val("AntagConc", "\\text{AntagConc}"),
            ctx.val("pA2", "\\text{pA}_2"),
            ctx.val("SchildSlope", "\\text{SchildSlope}"),
            ctx.factor("HillSlope", "\\text{HillSlope}"),
        ),
        "ec50_shift_gaddum_schild_conc" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + ({} \\cdot (1 + ({} \\cdot 10^{{{}}})^{{{}}}) / X)^{{{}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("EC50", "\\text{EC}_{50}"),
            ctx.val("AntagConc", "\\text{AntagConc}"),
            ctx.val("pA2", "\\text{pA}_2"),
            ctx.val("SchildSlope", "\\text{SchildSlope}"),
            ctx.val("HillSlope", "\\text{HillSlope}"),
        ),
        "ec50_shift_gaddum_schild_unity_log" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + 10^{{({} + \\log(1 + {} \\cdot 10^{{{}}}) - X) \\cdot {}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("LogEC50", "\\log\\text{EC}_{50}"),
            ctx.val("AntagConc", "\\text{AntagConc}"),
            ctx.val("pKB", "\\text{pK}_B"),
            ctx.factor("HillSlope", "\\text{HillSlope}"),
        ),
        "ec50_shift_gaddum_schild_unity_conc" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + ({} \\cdot (1 + {} \\cdot 10^{{{}}}) / X)^{{{}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("EC50", "\\text{EC}_{50}"),
            ctx.val("AntagConc", "\\text{AntagConc}"),
            ctx.val("pKB", "\\text{pK}_B"),
            ctx.val("HillSlope", "\\text{HillSlope}"),
        ),
        "ec50_shift_ratio_log" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + 10^{{({} + \\log({}) - X) \\cdot {}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("LogEC50Control", "\\log\\text{EC}_{50,\\text{ctrl}}"),
            ctx.val("EC50Ratio", "\\text{EC}_{50\\text{Ratio}}"),
            ctx.factor("HillSlope", "\\text{HillSlope}"),
        ),
        "ec50_shift_ratio_conc" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + ({} \\cdot {} / X)^{{{}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("EC50Control", "\\text{EC}_{50,\\text{ctrl}}"),
            ctx.factor("EC50Ratio", "\\text{EC}_{50\\text{Ratio}}"),
            ctx.val("HillSlope", "\\text{HillSlope}"),
        ),
        "allosteric_ec50_shift" => {
            let mod_c = ctx.val("ModConc", "\\text{ModConc}");
            let kb = ctx.denom("Kb", "K_b");
            let alpha = ctx.val("alpha", "\\alpha");
            format!(
                "Y = {} + \\frac{{{} - {}}}{{1 + 10^{{({} + \\log(\\frac{{1 + {mod_c} / {kb}}}{{1 + {alpha} \\cdot {mod_c} / {kb}}}) - X) \\cdot {}}}}}",
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.val("Top", "\\text{Top}"),
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.val("LogEC50", "\\log\\text{EC}_{50}"),
                ctx.factor("HillSlope", "\\text{HillSlope}"),
            )
        }
        "allosteric_ec50_shift_conc" => {
            let mod_c = ctx.val("ModConc", "\\text{ModConc}");
            let kb = ctx.denom("Kb", "K_b");
            let alpha = ctx.val("alpha", "\\alpha");
            format!(
                "Y = {} + \\frac{{{} - {}}}{{1 + ({} \\cdot \\frac{{1 + {mod_c} / {kb}}}{{1 + {alpha} \\cdot {mod_c} / {kb}}} / X)^{{{}}}}}",
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.val("Top", "\\text{Top}"),
                ctx.val("Bottom", "\\text{Bottom}"),
                ctx.val("EC50", "\\text{EC}_{50}"),
                ctx.val("HillSlope", "\\text{HillSlope}"),
            )
        }
        "operational_model_agonist" => {
            let basal = ctx.val("Basal", "\\text{Basal}");
            let emax = ctx.val("Emax", "E_{\\max}");
            let log_ka = ctx.val("LogKA", "\\log K_A");
            let log_tau = ctx.val("LogTau", "\\log\\tau");
            let n = ctx.val("n", "n");
            format!(
                "Y = {basal} + \\frac{{({emax} - {basal}) \\cdot (10^{{{log_tau} + X}})^{{{n}}}}}{{(10^{{{log_tau} + X}})^{{{n}}} + (10^{{X}} + 10^{{{log_ka}}})^{{{n}}}}}"
            )
        }
        "operational_model_agonist_conc" => {
            let basal = ctx.val("Basal", "\\text{Basal}");
            let emax = ctx.val("Emax", "E_{\\max}");
            let ka = ctx.val("KA", "K_A");
            let tau = ctx.val("Tau", "\\tau");
            let n = ctx.val("n", "n");
            format!("Y = {basal} + \\frac{{{emax} - {basal}}}{{1 + (\\frac{{{ka} + X}}{{{tau} \\cdot X}})^{{{n}}}}}")
        }
        "operational_model_depletion_log" => {
            let basal = ctx.val("Basal", "\\text{Basal}");
            let emax = ctx.val("EffectMax", "\\text{Effect}_{\\max}");
            let log_ka = ctx.val("LogKA", "\\log K_A");
            let log_tau = ctx.val("LogTau", "\\log\\tau");
            let n = ctx.val("n", "n");
            format!(
                "Y = {basal} + \\frac{{({emax} - {basal}) \\cdot (10^{{{log_tau} + X}})^{{{n}}}}}{{(10^{{{log_tau} + X}})^{{{n}}} + (10^{{X}} + 10^{{{log_ka}}})^{{{n}}}}}"
            )
        }
        "operational_model_depletion_conc" => {
            let basal = ctx.val("Basal", "\\text{Basal}");
            let emax = ctx.val("EffectMax", "\\text{Effect}_{\\max}");
            let ka = ctx.val("KA", "K_A");
            let tau = ctx.val("Tau", "\\tau");
            let n = ctx.val("n", "n");
            format!("Y = {basal} + \\frac{{{emax} - {basal}}}{{1 + (\\frac{{{ka} + X}}{{{tau} \\cdot X}})^{{{n}}}}}")
        }
        "log_inhibitor_vs_response_3pl" | "log_inhibitor_vs_response" | "ic50_3pl" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + 10^{{X - {}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("LogIC50", "\\log\\text{IC}_{50}"),
        ),
        "log_inhibitor_vs_response_4pl" | "log_inhibitor_vs_response_variable_slope" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + 10^{{(X - {}) \\cdot {}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("LogIC50", "\\log\\text{IC}_{50}"),
            ctx.factor("HillSlope", "\\text{HillSlope}"),
        ),
        "log_inhibitor_vs_response_normalized" => {
            format!("Y = \\frac{{100}}{{1 + 10^{{X - {}}}}}", ctx.val("LogIC50", "\\log\\text{IC}_{50}"))
        }
        "log_inhibitor_vs_response_normalized_var" => format!(
            "Y = \\frac{{100}}{{1 + 10^{{(X - {}) \\cdot {}}}}}",
            ctx.val("LogIC50", "\\log\\text{IC}_{50}"),
            ctx.factor("HillSlope", "\\text{HillSlope}"),
        ),
        "inhibitor_vs_response_3pl" | "inhibitor_vs_response" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + X / {}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.denom("IC50", "\\text{IC}_{50}"),
        ),
        "inhibitor_vs_response_4pl" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + (X / {})^{{{}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.denom("IC50", "\\text{IC}_{50}"),
            ctx.val("HillSlope", "\\text{HillSlope}"),
        ),
        "inhibitor_vs_response_normalized" => {
            format!("Y = \\frac{{100}}{{1 + X / {}}}", ctx.denom("IC50", "\\text{IC}_{50}"))
        }
        "inhibitor_vs_response_normalized_var" => {
            format!("Y = \\frac{{100}}{{1 + ({} / X)^{{{}}}}}", ctx.val("IC50", "\\text{IC}_{50}"), ctx.val("HillSlope", "\\text{HillSlope}"),)
        }
        "absolute_ic50_4pl" => {
            let bot = ctx.val("Bottom", "\\text{Bottom}");
            let top = ctx.val("Top", "\\text{Top}");
            let c0 = ctx.val("Control0", "\\text{Control}_0");
            let c100 = ctx.val("Control100", "\\text{Control}_{100}");
            let midpoint = format!("0.5 \\cdot ({c0} + {c100})");
            format!(
                "Y = {bot} + \\frac{{{top} - {bot}}}{{1 + \\frac{{{top} - {midpoint}}}{{{midpoint} - {bot}}} \\cdot 10^{{(X - {}) \\cdot {}}}}}",
                ctx.val("LogIC50", "\\log\\text{IC}_{50}"),
                ctx.factor("HillSlope", "\\text{HillSlope}"),
            )
        }
        "absolute_ic50_4pl_conc" => {
            let bot = ctx.val("Bottom", "\\text{Bottom}");
            let top = ctx.val("Top", "\\text{Top}");
            let c0 = ctx.val("Control0", "\\text{Control}_0");
            let c100 = ctx.val("Control100", "\\text{Control}_{100}");
            let midpoint = format!("0.5 \\cdot ({c0} + {c100})");
            format!(
                "Y = {bot} + \\frac{{{top} - {bot}}}{{1 + \\frac{{{top} - {midpoint}}}{{{midpoint} - {bot}}} \\cdot (X / {})^{{{}}}}}",
                ctx.denom("IC50", "\\text{IC}_{50}"),
                ctx.val("HillSlope", "\\text{HillSlope}"),
            )
        }
        _ => return None,
    };
    Some(s)
}

fn format_binding_kinetics_tex(id: &str, ctx: &EqCtx<'_>) -> Option<String> {
    let s = match id {
        "one_site_specific_binding" => format!("Y = \\frac{{{} \\cdot X}}{{{} + X}}", ctx.val("Bmax", "B_{\\max}"), ctx.val("Kd", "K_d")),
        "one_site_specific_hill" => {
            let h = ctx.val("h", "h");
            format!("Y = \\frac{{{} \\cdot X^{{{h}}}}}{{{}^{{{h}}} + X^{{{h}}}}}", ctx.val("Bmax", "B_{\\max}"), ctx.pow_base("Kd", "K_d"))
        }
        "one_site_total_and_ns" | "one_site_total_and_ns_global" => format!(
            "Y = \\frac{{{} \\cdot X}}{{{} + X}} + {} \\cdot X + {}",
            ctx.val("Bmax", "B_{\\max}"),
            ctx.val("Kd", "K_d"),
            ctx.val("NS", "\\text{NS}"),
            ctx.val("Background", "\\text{Background}"),
        ),
        "two_site_total_and_ns_global" => format!(
            "Y = \\frac{{{} \\cdot X}}{{{} + X}} + \\frac{{{} \\cdot X}}{{{} + X}} + {} \\cdot X + {}",
            ctx.val("BmaxHi", "B_{\\max,\\text{Hi}}"),
            ctx.val("KdHi", "K_{d,\\text{Hi}}"),
            ctx.val("BmaxLo", "B_{\\max,\\text{Lo}}"),
            ctx.val("KdLo", "K_{d,\\text{Lo}}"),
            ctx.val("NS", "\\text{NS}"),
            ctx.val("Background", "\\text{Background}"),
        ),
        "one_site_specific_ligand_depletion" => {
            let rt = ctx.val("Rt", "R_t");
            let kd = ctx.val("Kd", "K_d");
            let ns = ctx.val("NS", "\\text{NS}");
            format!(
                "Y = \\frac{{{rt} + X + {kd} \\cdot (1 + {ns}) - \\sqrt{{({kd} \\cdot (1 + {ns}) + {rt} - X)^{{2}} + 4 \\cdot (1 + {ns}) \\cdot {kd} \\cdot X}}}}{{2}}"
            )
        }
        "one_site_total_ligand_depletion" => {
            let bmax = ctx.val("Bmax", "B_{\\max}");
            let kd = ctx.val("Kd", "K_d");
            let ns = ctx.val("NS", "\\text{NS}");
            let sa = ctx.factor("SpecAct", "\\text{SpecAct}");
            let vol = ctx.factor("Vol", "\\text{Vol}");
            let kd_cpm = format!("({kd} \\cdot {vol} \\cdot 1000 \\cdot {sa})");
            let b_term = format!("{kd_cpm} \\cdot (1 + {ns}) + X \\cdot (1 + 2 \\cdot {ns}) + {bmax}");
            let c_term = format!("X \\cdot ({ns} \\cdot {kd_cpm} + {ns} \\cdot X + {bmax})");
            format!("Y = \\frac{{2 \\cdot {c_term}}}{{{b_term} + \\sqrt{{({b_term})^{{2}} - 4 \\cdot (1 + {ns}) \\cdot {c_term}}}}}")
        }
        "one_site_competition_ligand_depletion" => {
            let rt = ctx.val("Rt", "R_t");
            let hot_f = ctx.factor("HotNM", "\\text{HotNM}");
            let kd = ctx.val("KdNM", "K_{d,\\text{nM}}");
            let kd_f = ctx.factor("KdNM", "K_{d,\\text{nM}}");
            let log_ki = ctx.val("LogKi", "\\log K_i");
            let ns = ctx.val("NS", "\\text{NS}");
            let r_free = format!("R({rt}, {log_ki}, X)");
            format!("Y = \\frac{{({r_free} + {ns} \\cdot {kd_f}) \\cdot {hot_f}}}{{{kd} \\cdot (1 + {ns}) + {r_free}}}")
        }
        "allosteric_modulator_saturation" => {
            let mod_c = ctx.val("ModConc", "\\text{ModConc}");
            let kb = ctx.denom("Kb", "K_b");
            let alpha = ctx.val("alpha", "\\alpha");
            format!(
                "Y = \\frac{{{} \\cdot X \\cdot (1 + {alpha} \\cdot {mod_c} / {kb})}}{{{} \\cdot (1 + {mod_c} / {kb}) + X \\cdot (1 + {alpha} \\cdot {mod_c} / {kb})}}",
                ctx.val("Bmax", "B_{\\max}"),
                ctx.val("Kd", "K_d"),
            )
        }
        "allosteric_modulator_competition" => {
            let y0 = ctx.val("Y0", "Y_0");
            let log_kb = ctx.val("LogKB", "\\log K_B");
            let log_alpha = ctx.val("LogAlpha", "\\log\\alpha");
            let hot = ctx.val("RadioligandNM", "[\\text{Hot}]");
            let kd = ctx.val("HotKdNM", "K_{d,\\text{hot}}");
            format!(
                "Y = {y0} \\cdot \\frac{{{hot} + {kd}}}{{{hot} + {kd} \\cdot \\frac{{1 + 10^{{X - {log_kb}}}}}{{1 + 10^{{X - {log_kb} + {log_alpha}}}}}}}"
            )
        }
        "homologous_one_site_competition" => {
            let hot = ctx.val("HotConc", "\\text{HotConc}");
            format!(
                "Y = \\frac{{{} \\cdot {hot}}}{{{hot} + 10^{{X}} + {}}} + {}",
                ctx.val("Bmax", "B_{\\max}"),
                ctx.val("Kd", "K_d"),
                ctx.val("NS", "\\text{NS}"),
            )
        }
        "two_sites_specific_binding" => format!(
            "Y = \\frac{{{} \\cdot X}}{{{} + X}} + \\frac{{{} \\cdot X}}{{{} + X}}",
            ctx.val("BmaxHi", "B_{\\max,\\text{Hi}}"),
            ctx.val("KdHi", "K_{d,\\text{Hi}}"),
            ctx.val("BmaxLo", "B_{\\max,\\text{Lo}}"),
            ctx.val("KdLo", "K_{d,\\text{Lo}}"),
        ),
        "one_site_fit_ki" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + 10^{{X - {} - \\log(1 + {} / {})}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("LogKi", "\\log K_i"),
            ctx.val("HotConc_nM", "\\text{HotConc}_{\\text{nM}}"),
            ctx.denom("HotKd_nM", "\\text{HotKd}_{\\text{nM}}"),
        ),
        "one_site_fit_logic50" => format!(
            "Y = {} + \\frac{{{} - {}}}{{1 + 10^{{X - {}}}}}",
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("Top", "\\text{Top}"),
            ctx.val("Bottom", "\\text{Bottom}"),
            ctx.val("LogIC50", "\\log\\text{IC}_{50}"),
        ),
        "two_sites_fit_logic50" => {
            let bot = ctx.val("Bottom", "\\text{Bottom}");
            let top = ctx.val("Top", "\\text{Top}");
            let f_hi = ctx.val("FractionHi", "\\text{Fraction}_{\\text{Hi}}");
            format!(
                "Y = {bot} + ({top} - {bot}) \\cdot (\\frac{{{f_hi}}}{{1 + 10^{{X - {}}}}} + \\frac{{1 - {f_hi}}}{{1 + 10^{{X - {}}}}})",
                ctx.val("LogIC50_Hi", "\\log\\text{IC}_{50,\\text{Hi}}"),
                ctx.val("LogIC50_Lo", "\\log\\text{IC}_{50,\\text{Lo}}"),
            )
        }
        "two_sites_fit_ki" => {
            let bot = ctx.val("Bottom", "\\text{Bottom}");
            let top = ctx.val("Top", "\\text{Top}");
            let f_hi = ctx.val("FractionHi", "\\text{Fraction}_{\\text{Hi}}");
            let hot = ctx.val("HotConc_nM", "\\text{HotConc}_{\\text{nM}}");
            let kd = ctx.denom("HotKd_nM", "\\text{HotKd}_{\\text{nM}}");
            format!(
                "Y = {bot} + ({top} - {bot}) \\cdot (\\frac{{{f_hi}}}{{1 + 10^{{X - {} - \\log(1 + {hot} / {kd})}}}} + \\frac{{1 - {f_hi}}}{{1 + 10^{{X - {} - \\log(1 + {hot} / {kd})}}}})",
                ctx.val("LogKi_1", "\\log K_{i,1}"),
                ctx.val("LogKi_2", "\\log K_{i,2}"),
            )
        }
        "dissociation_one_phase" => {
            let ns = ctx.val("NS", "\\text{NS}");
            format!("Y = ({} - {ns}) \\cdot \\exp(-{} \\cdot X) + {ns}", ctx.val("Y0", "Y_0"), ctx.val("Koff", "K_{\\text{off}}"),)
        }
        "association_kinetics_one_conc" => {
            format!("Y = {} \\cdot (1 - \\exp(-{} \\cdot X))", ctx.val("Ymax", "Y_{\\max}"), ctx.val("Kobs", "K_{\\text{obs}}"),)
        }
        "association_kinetics_two_conc" => {
            let bmax = ctx.val("Bmax", "B_{\\max}");
            let kon = ctx.val("Kon", "K_{\\text{on}}");
            let kon_f = ctx.factor("Kon", "K_{\\text{on}}");
            let koff = ctx.val("Koff", "K_{\\text{off}}");
            let lig = ctx.factor("LigandConc", "\\text{LigandConc}");
            format!(
                "Y = \\frac{{{bmax} \\cdot {kon_f} \\cdot {lig}}}{{{kon} \\cdot {lig} + {koff}}} \\cdot (1 - \\exp(-({kon} \\cdot {lig} + {koff}) \\cdot X))"
            )
        }
        "assoc_then_dissoc" => {
            let bmax = ctx.val("Bmax", "B_{\\max}");
            let kon = ctx.val("Kon", "K_{\\text{on}}");
            let kon_f = ctx.factor("Kon", "K_{\\text{on}}");
            let koff = ctx.val("Koff", "K_{\\text{off}}");
            let lig = ctx.factor("LigandConc", "\\text{LigandConc}");
            let t0 = ctx.val("Time0", "\\text{Time}_0");
            format!(
                "Y = \\frac{{{bmax} \\cdot {kon_f} \\cdot {lig}}}{{{kon} \\cdot {lig} + {koff}}} \\cdot (1 - \\exp(-({kon} \\cdot {lig} + {koff}) \\cdot \\min(X, {t0}))) \\cdot \\exp(-{koff} \\cdot \\max(0, X - {t0}))"
            )
        }
        "motulsky_mahan_kinetics" => format_motulsky_mahan_tex(ctx),
        _ => return None,
    };
    Some(s)
}

fn format_motulsky_mahan_tex(ctx: &EqCtx<'_>) -> String {
    let bmax = ctx.val("Bmax", "B_{\\max}");
    let kon1 = ctx.factor("kon1", "k_{\\text{on},1}");
    let koff2 = ctx.val("koff2", "k_{\\text{off},2}");
    let l = ctx.factor("L", "L");

    if ctx.substituted
        && let (Some(k1), Some(kr1), Some(k2), Some(kr2), Some(lv), Some(iv)) =
            (ctx.find("kon1"), ctx.find("koff1"), ctx.find("kon2"), ctx.find("koff2"), ctx.find("L"), ctx.find("I"))
    {
        let ka = k1 * lv + kr1;
        let kb = k2 * iv + kr2;
        let disc = ((ka - kb) * (ka - kb) + 4.0 * k1 * k2 * lv * iv).max(0.0).sqrt();
        let kf = 0.5 * (ka + kb + disc);
        let ks = 0.5 * (ka + kb - disc);
        if (kf - ks).abs() > 1e-18 && kf.abs() > 1e-18 && ks.abs() > 1e-18 {
            let kf_s = ctx.num(kf);
            let ks_s = ctx.num(ks);
            let ks_f = ctx.factor_num(ks);
            return format!(
                "Y = \\frac{{{bmax} \\cdot {kon1} \\cdot {l}}}{{{kf_s} - {ks_s}}} \\cdot (\\frac{{{koff2} \\cdot ({kf_s} - {ks_s})}}{{{kf_s} \\cdot {ks_f}}} + \\frac{{{koff2} - {kf_s}}}{{{kf_s}}} \\cdot \\exp(-{kf_s} \\cdot X) - \\frac{{{koff2} - {ks_s}}}{{{ks_s}}} \\cdot \\exp(-{ks_s} \\cdot X))"
            );
        }
    }

    format!(
        "Y = \\frac{{{bmax} \\cdot {kon1} \\cdot {l}}}{{K_F - K_S}} \\cdot (\\frac{{{koff2} \\cdot (K_F - K_S)}}{{K_F \\cdot K_S}} + \\frac{{{koff2} - K_F}}{{K_F}} \\cdot \\exp(-K_F \\cdot X) - \\frac{{{koff2} - K_S}}{{K_S}} \\cdot \\exp(-K_S \\cdot X))"
    )
}

fn format_enzyme_kinetics_tex(id: &str, ctx: &EqCtx<'_>) -> Option<String> {
    let s = match id {
        "michaelis_menten" => format!("Y = \\frac{{{} \\cdot X}}{{{} + X}}", ctx.val("Vmax", "V_{\\max}"), ctx.val("Km", "K_m")),
        "kcat_enzyme_kinetics" | "kcat" => format!(
            "Y = \\frac{{{} \\cdot {} \\cdot X}}{{{} + X}}",
            ctx.val("Et", "E_t"),
            ctx.factor("kcat", "k_{\\text{cat}}"),
            ctx.val("Km", "K_m"),
        ),
        "allosteric_sigmoidal_kinetics" | "allosteric_sigmoidal" => {
            let h = ctx.val("h", "h");
            format!(
                "Y = \\frac{{{} \\cdot X^{{{h}}}}}{{{}^{{{h}}} + X^{{{h}}}}}",
                ctx.val("Vmax", "V_{\\max}"),
                ctx.pow_base("Khalf", "K_{\\text{half}}")
            )
        }
        "substrate_inhibition" => format!(
            "Y = \\frac{{{} \\cdot X}}{{{} + X \\cdot (1 + X / {})}}",
            ctx.val("Vmax", "V_{\\max}"),
            ctx.val("Km", "K_m"),
            ctx.denom("Ki", "K_i"),
        ),
        "competitive_inhibition" => format!(
            "Y = \\frac{{{} \\cdot X}}{{{} \\cdot (1 + {} / {}) + X}}",
            ctx.val("Vmax", "V_{\\max}"),
            ctx.val("Km", "K_m"),
            ctx.val("I", "I"),
            ctx.denom("Ki", "K_i"),
        ),
        "noncompetitive_inhibition" => format!(
            "Y = \\frac{{{} \\cdot X}}{{(1 + {} / {}) \\cdot ({} + X)}}",
            ctx.val("Vmax", "V_{\\max}"),
            ctx.val("I", "I"),
            ctx.denom("Ki", "K_i"),
            ctx.val("Km", "K_m"),
        ),
        "uncompetitive_inhibition" => format!(
            "Y = \\frac{{{} \\cdot X}}{{{} + X \\cdot (1 + {} / {})}}",
            ctx.val("Vmax", "V_{\\max}"),
            ctx.val("Km", "K_m"),
            ctx.val("I", "I"),
            ctx.denom("alphaKi", "\\alpha K_i"),
        ),
        "mixed_model_inhibition" => format!(
            "Y = \\frac{{{} \\cdot X}}{{{} \\cdot (1 + {} / {}) + X \\cdot (1 + {} / ({} \\cdot {}))}}",
            ctx.val("Vmax", "V_{\\max}"),
            ctx.val("Km", "K_m"),
            ctx.val("I", "I"),
            ctx.denom("Ki", "K_i"),
            ctx.val("I", "I"),
            ctx.val("Alpha", "\\alpha"),
            ctx.factor("Ki", "K_i"),
        ),
        "morrison_tight_binding_ki" | "morrison" => {
            let vo = ctx.val("Vo", "V_0");
            let et = ctx.val("Et", "E_t");
            let ki = ctx.val("Ki", "K_i");
            let s_val = ctx.val("S", "S");
            let km = ctx.denom("Km", "K_m");
            let et_f = ctx.factor("Et", "E_t");
            format!(
                "Y = {vo} \\cdot (1 - \\frac{{{et} + X + {ki} \\cdot (1 + {s_val} / {km}) - \\sqrt{{({et} + X + {ki} \\cdot (1 + {s_val} / {km}))^{{2}} - 4 \\cdot {et_f} \\cdot X}}}}{{2 \\cdot {et_f}}})"
            )
        }
        _ => return None,
    };
    Some(s)
}

fn format_curve_equation_tex(id: &str, ctx: &EqCtx<'_>) -> Option<String> {
    format_exponential_growth_tex(id, ctx).or_else(|| format_peak_wave_tex(id, ctx)).or_else(|| format_line_poly_curve_tex(id, ctx))
}

fn format_exponential_growth_tex(id: &str, ctx: &EqCtx<'_>) -> Option<String> {
    let s = match id {
        "one_phase_decay" => {
            let p = ctx.val("Plateau", "\\text{Plateau}");
            format!("Y = ({} - {p}) \\cdot \\exp(-{} \\cdot X) + {p}", ctx.val("Y0", "Y_0"), ctx.val("K", "K"))
        }
        "two_phase_decay" => {
            let y0 = ctx.val("Y0", "Y_0");
            let p = ctx.val("Plateau", "\\text{Plateau}");
            let pf = ctx.val("PercentFast", "\\text{PercentFast}");
            format!(
                "Y = {p} + ({y0} - {p}) \\cdot \\frac{{{pf}}}{{100}} \\cdot \\exp(-{} \\cdot X) + ({y0} - {p}) \\cdot (1 - \\frac{{{pf}}}{{100}}) \\cdot \\exp(-{} \\cdot X)",
                ctx.val("KFast", "K_{\\text{Fast}}"),
                ctx.val("KSlow", "K_{\\text{Slow}}"),
            )
        }
        "three_phase_decay" => {
            let y0 = ctx.val("Y0", "Y_0");
            let p = ctx.val("Plateau", "\\text{Plateau}");
            let pf = ctx.val("PercentFast", "\\text{PercentFast}");
            let ps = ctx.val("PercentSlow", "\\text{PercentSlow}");
            format!(
                "Y = {p} + ({y0} - {p}) \\cdot (\\frac{{{pf}}}{{100}} \\cdot \\exp(-{} \\cdot X) + \\frac{{{ps}}}{{100}} \\cdot \\exp(-{} \\cdot X) + (1 - \\frac{{{pf} + {ps}}}{{100}}) \\cdot \\exp(-{} \\cdot X))",
                ctx.val("KFast", "K_{\\text{Fast}}"),
                ctx.val("KSlow", "K_{\\text{Slow}}"),
                ctx.val("KVerySlow", "K_{\\text{VerySlow}}"),
            )
        }
        "plateau_followed_by_one_phase_decay" => {
            let p = ctx.val("Plateau", "\\text{Plateau}");
            format!("Y = ({} - {p}) \\cdot \\exp(-{} \\cdot \\max(0, X - {})) + {p}", ctx.val("Y0", "Y_0"), ctx.val("K", "K"), ctx.val("X0", "X_0"),)
        }
        "one_phase_association" => {
            let y0 = ctx.val("Y0", "Y_0");
            format!("Y = {y0} + ({} - {y0}) \\cdot (1 - \\exp(-{} \\cdot X))", ctx.val("Plateau", "\\text{Plateau}"), ctx.val("K", "K"))
        }
        "two_phase_association" => {
            let y0 = ctx.val("Y0", "Y_0");
            let p = ctx.val("Plateau", "\\text{Plateau}");
            let pf = ctx.val("PercentFast", "\\text{PercentFast}");
            format!(
                "Y = {y0} + ({p} - {y0}) \\cdot (\\frac{{{pf}}}{{100}} \\cdot (1 - \\exp(-{} \\cdot X)) + (1 - \\frac{{{pf}}}{{100}}) \\cdot (1 - \\exp(-{} \\cdot X)))",
                ctx.val("KFast", "K_{\\text{Fast}}"),
                ctx.val("KSlow", "K_{\\text{Slow}}"),
            )
        }
        "plateau_followed_by_one_phase_association" => {
            let y0 = ctx.val("Y0", "Y_0");
            format!(
                "Y = {y0} + ({} - {y0}) \\cdot (1 - \\exp(-{} \\cdot \\max(0, X - {})))",
                ctx.val("Plateau", "\\text{Plateau}"),
                ctx.val("K", "K"),
                ctx.val("X0", "X_0"),
            )
        }
        "exponential_growth" | "malthusian_growth" | "exponential_growth_malthusian" => {
            format!("Y = {} \\cdot \\exp({} \\cdot X)", ctx.val("Y0", "Y_0"), ctx.val("K", "K"))
        }
        "log_exponential_growth" => {
            format!("Y = {} + {} \\cdot X", ctx.val("LnY0", "\\ln Y_0"), ctx.val("k", "k"))
        }
        "exponential_plateau" => {
            let ym = ctx.val("YM", "Y_M");
            format!("Y = {ym} - ({ym} - {}) \\cdot \\exp(-{} \\cdot X)", ctx.val("Y0", "Y_0"), ctx.val("K", "K"))
        }
        "logistic_growth" => {
            let ym = ctx.val("YM", "Y_M");
            let y0 = ctx.val("Y0", "Y_0");
            format!("Y = \\frac{{{ym} \\cdot {y0}}}{{({ym} - {y0}) \\cdot \\exp(-{} \\cdot X) + {y0}}}", ctx.val("K", "K"))
        }
        "boltzmann_sigmoid" => {
            let bot = ctx.val("Bottom", "\\text{Bottom}");
            format!(
                "Y = {bot} + \\frac{{{} - {bot}}}{{1 + \\exp(\\frac{{{} - X}}{{{}}})}}",
                ctx.val("Top", "\\text{Top}"),
                ctx.val("V50", "V_{50}"),
                ctx.val("Slope", "\\text{Slope}"),
            )
        }
        "gompertz_growth" => {
            let ym = ctx.val("YM", "Y_M");
            let y0 = ctx.val("Y0", "Y_0");
            format!("Y = {ym} \\cdot (\\frac{{{y0}}}{{{ym}}})^{{\\exp(-{} \\cdot X)}}", ctx.val("K", "K"))
        }
        "beta_growth_then_decay" => {
            let ym = ctx.val("Ym", "Y_m");
            let te = ctx.val("te", "t_e");
            let tm = ctx.val("tm", "t_m");
            format!(
                "Y = {ym} \\cdot (1 + \\frac{{{te} - X}}{{{te} - {tm}}}) \\cdot (\\frac{{X}}{{{te}}})^{{\\frac{{{te}}}{{{te} - {tm}}}}} \\quad (0 \\le X \\le {te}); \\quad {ym} \\quad (X > {te})"
            )
        }
        "linear_quadratic_survival" => {
            format!("Y = \\exp(-({} \\cdot D + {} \\cdot D^2))", ctx.val("alpha", "\\alpha"), ctx.val("beta", "\\beta"))
        }
        "linear_quadratic_percent_surviving" => {
            format!("Y = 100 \\cdot \\exp(-({} \\cdot D + {} \\cdot D^2))", ctx.val("alpha", "\\alpha"), ctx.val("beta", "\\beta"))
        }
        "linear_quadratic_number_surviving" => format!(
            "Y = {} \\cdot \\exp(-({} \\cdot D + {} \\cdot D^2))",
            ctx.val("Y0", "Y_0"),
            ctx.val("alpha", "\\alpha"),
            ctx.val("beta", "\\beta"),
        ),
        "linear_quadratic_fraction_dead" => {
            format!("Y = 1 - \\exp(-({} \\cdot D + {} \\cdot D^2))", ctx.val("alpha", "\\alpha"), ctx.val("beta", "\\beta"))
        }
        "linear_quadratic_percent_dead" => {
            format!("Y = 100 \\cdot (1 - \\exp(-({} \\cdot D + {} \\cdot D^2)))", ctx.val("alpha", "\\alpha"), ctx.val("beta", "\\beta"))
        }
        "linear_quadratic_number_dead" => format!(
            "Y = {} \\cdot (1 - \\exp(-({} \\cdot D + {} \\cdot D^2)))",
            ctx.val("Y0", "Y_0"),
            ctx.val("alpha", "\\alpha"),
            ctx.val("beta", "\\beta"),
        ),
        _ => return None,
    };
    Some(s)
}

fn format_peak_wave_tex(id: &str, ctx: &EqCtx<'_>) -> Option<String> {
    let s = match id {
        "gaussian" => format!(
            "Y = {} \\cdot \\exp(-\\frac{{1}}{{2}} \\cdot (\\frac{{X - {}}}{{{}}})^{{2}})",
            ctx.val("Amplitude", "\\text{Amplitude}"),
            ctx.val("Mean", "\\text{Mean}"),
            ctx.val("SD", "\\text{SD}"),
        ),
        "gaussian_baseline" => format!(
            "Y = {} + {} \\cdot \\exp(-\\frac{{1}}{{2}} \\cdot (\\frac{{X - {}}}{{{}}})^{{2}})",
            ctx.val("Baseline", "\\text{Baseline}"),
            ctx.val("Amplitude", "\\text{Amplitude}"),
            ctx.val("Mean", "\\text{Mean}"),
            ctx.val("SD", "\\text{SD}"),
        ),
        "gaussian_area" => {
            let sd = ctx.val("SD", "\\text{SD}");
            format!(
                "Y = \\frac{{{}}}{{{sd} \\cdot \\sqrt{{2\\pi}}}} \\cdot \\exp(-\\frac{{1}}{{2}} \\cdot (\\frac{{X - {}}}{{{sd}}})^{{2}})",
                ctx.val("Area", "\\text{Area}"),
                ctx.val("Mean", "\\text{Mean}"),
            )
        }
        "sum_of_two_gaussians" => format!(
            "Y = {} \\cdot \\exp(-\\frac{{1}}{{2}} \\cdot (\\frac{{X - {}}}{{{}}})^{{2}}) + {} \\cdot \\exp(-\\frac{{1}}{{2}} \\cdot (\\frac{{X - {}}}{{{}}})^{{2}})",
            ctx.val("Amp1", "A_1"),
            ctx.val("Mean1", "\\mu_1"),
            ctx.val("SD1", "\\sigma_1"),
            ctx.val("Amp2", "A_2"),
            ctx.val("Mean2", "\\mu_2"),
            ctx.val("SD2", "\\sigma_2"),
        ),
        "log_gaussian" => format!(
            "Y = {} \\cdot \\exp(-\\frac{{1}}{{2}} \\cdot (\\frac{{\\ln(X / {})}}{{\\ln({})}})^{{2}})",
            ctx.val("Amplitude", "\\text{Amplitude}"),
            ctx.denom("GeoMean", "\\text{GeoMean}"),
            ctx.val("GeoSD", "\\text{GeoSD}"),
        ),
        "lognormal_area" => format!(
            "Y = \\frac{{{}}}{{X}} \\cdot \\exp(-\\frac{{1}}{{2}} \\cdot (\\frac{{\\ln(X / {})}}{{\\ln({})}})^{{2}})",
            ctx.val("A", "A"),
            ctx.denom("GeoMean", "\\text{GeoMean}"),
            ctx.val("GeoSD", "\\text{GeoSD}"),
        ),
        "cumulative_gaussian" => {
            let bot = ctx.val("Bottom", "\\text{Bottom}");
            format!(
                "Y = {bot} + ({} - {bot}) \\cdot \\frac{{1}}{{2}} \\cdot (1 + \\erf(\\frac{{X - {}}}{{{} \\cdot \\sqrt{{2}}}}))",
                ctx.val("Top", "\\text{Top}"),
                ctx.val("Mean", "\\text{Mean}"),
                ctx.val("SD", "\\text{SD}"),
            )
        }
        "lorentzian" => format!(
            "Y = \\frac{{{}}}{{1 + (\\frac{{X - {}}}{{0.5 \\cdot {}}})^{{2}}}}",
            ctx.val("Amplitude", "\\text{Amplitude}"),
            ctx.val("Center", "\\text{Center}"),
            ctx.factor("Width", "\\text{Width}"),
        ),
        "voigt" | "voigt_peak" => format!(
            "Y = {} \\cdot ({} \\cdot \\frac{{1}}{{1 + (\\frac{{X - {}}}{{{}}})^{{2}}}} + (1 - {}) \\cdot \\exp(-\\ln(2) \\cdot (\\frac{{X - {}}}{{{}}})^{{2}}))",
            ctx.val("Amplitude", "\\text{Amplitude}"),
            ctx.val("Eta", "\\eta"),
            ctx.val("Center", "\\text{Center}"),
            ctx.val("Width", "\\text{Width}"),
            ctx.val("Eta", "\\eta"),
            ctx.val("Center", "\\text{Center}"),
            ctx.val("Width", "\\text{Width}"),
        ),
        "weibull_peak" => {
            let shape = ctx.val("Shape", "\\text{Shape}");
            let scale = ctx.val("Scale", "\\text{Scale}");
            format!(
                "Y = {} \\cdot \\frac{{{shape}}}{{{scale}}} \\cdot (\\frac{{X}}{{{scale}}})^{{{shape} - 1}} \\cdot \\exp(-(\\frac{{X}}{{{scale}}})^{{{shape}}})",
                ctx.val("Amplitude", "\\text{Amplitude}"),
            )
        }
        "sine_wave" => format!(
            "Y = {} + {} \\cdot \\sin(2\\pi \\cdot {} \\cdot X + {})",
            ctx.val("Baseline", "\\text{Baseline}"),
            ctx.val("Amplitude", "\\text{Amplitude}"),
            ctx.factor("Frequency", "\\text{Frequency}"),
            ctx.val("PhaseShift", "\\text{PhaseShift}"),
        ),
        "standard_sine_wave" => format!(
            "Y = {} \\cdot \\sin(\\frac{{2\\pi \\cdot X}}{{{}}} + {})",
            ctx.val("Amplitude", "\\text{Amplitude}"),
            ctx.val("Wavelength", "\\text{Wavelength}"),
            ctx.val("PhaseShift", "\\text{PhaseShift}"),
        ),
        "damped_sine_wave" => format!(
            "Y = {} + {} \\cdot \\exp(-{} \\cdot X) \\cdot \\sin(2\\pi \\cdot {} \\cdot X + {})",
            ctx.val("Baseline", "\\text{Baseline}"),
            ctx.val("Amplitude", "\\text{Amplitude}"),
            ctx.val("K", "K"),
            ctx.factor("Frequency", "\\text{Frequency}"),
            ctx.val("PhaseShift", "\\text{PhaseShift}"),
        ),
        "sinc_wave" => {
            let w = ctx.denom("Width", "\\text{Width}");
            let c = ctx.val("Center", "\\text{Center}");
            format!(
                "Y = {} + {} \\cdot \\frac{{\\sin((X - {c}) / {w})}}{{(X - {c}) / {w}}}",
                ctx.val("Baseline", "\\text{Baseline}"),
                ctx.val("Amplitude", "\\text{Amplitude}"),
            )
        }
        _ => return None,
    };
    Some(s)
}

fn format_line_poly_curve_tex(id: &str, ctx: &EqCtx<'_>) -> Option<String> {
    let s = match id {
        "straight_line" => format_polynomial_equation_tex(1, false, ctx.params, ctx.substituted, ctx.sig_figs),
        "line_through_origin" => format_polynomial_equation_tex(1, true, ctx.params, ctx.substituted, ctx.sig_figs),
        "two_lines_intersection" => format!(
            "Y = {} + {} \\cdot (X - {})",
            ctx.val("YCross", "Y_{\\text{cross}}"),
            ctx.val("Slope", "\\text{Slope}"),
            ctx.val("XCross", "X_{\\text{cross}}"),
        ),
        "segmental_linear" => format!(
            "Y = {} + {} \\cdot \\min(X, {}) + {} \\cdot \\max(0, X - {})",
            ctx.val("Intercept1", "\\text{Intercept}_1"),
            ctx.val("Slope1", "\\text{Slope}_1"),
            ctx.val("X0", "X_0"),
            ctx.val("Slope2", "\\text{Slope}_2"),
            ctx.val("X0", "X_0"),
        ),
        "hinge_smooth_hyperbolic" => {
            let s1 = ctx.val("Slope1", "\\text{Slope}_1");
            let s2 = ctx.val("Slope2", "\\text{Slope}_2");
            let x0 = ctx.val("X0", "X_0");
            let delta = ctx.val("Delta", "\\Delta");
            let delta_b = ctx.pow_base("Delta", "\\Delta");
            format!(
                "Y = {} + {s1} \\cdot (X - {x0}) + \\frac{{{s2} - {s1}}}{{2}} \\cdot ((X - {x0}) + \\sqrt{{(X - {x0})^{{2}} + {delta_b}^{{2}}}} - |{delta}|)",
                ctx.val("Y0", "Y_0"),
            )
        }
        "polynomial_2" | "quadratic" => format_polynomial_equation_tex(2, false, ctx.params, ctx.substituted, ctx.sig_figs),
        "polynomial_3" | "cubic" => format_polynomial_equation_tex(3, false, ctx.params, ctx.substituted, ctx.sig_figs),
        "polynomial_4" | "quartic" => format_polynomial_equation_tex(4, false, ctx.params, ctx.substituted, ctx.sig_figs),
        "polynomial_5" => format_polynomial_equation_tex(5, false, ctx.params, ctx.substituted, ctx.sig_figs),
        "polynomial_6" => format_polynomial_equation_tex(6, false, ctx.params, ctx.substituted, ctx.sig_figs),
        "centered_poly_1" | "centered_linear" | "centered_polynomial_1" => format_centered_poly_tex(1, ctx),
        "centered_poly_2" | "centered_quadratic" | "centered_polynomial_2" => format_centered_poly_tex(2, ctx),
        "centered_poly_3" | "centered_cubic" | "centered_polynomial_3" => format_centered_poly_tex(3, ctx),
        "centered_poly_4" | "centered_quartic" | "centered_polynomial_4" => format_centered_poly_tex(4, ctx),
        "centered_poly_5" | "centered_polynomial_5" => format_centered_poly_tex(5, ctx),
        "centered_poly_6" | "centered_polynomial_6" => format_centered_poly_tex(6, ctx),
        "power_law" | "power_series" => format!("Y = {} \\cdot X^{{{}}}", ctx.val("a", "a"), ctx.val("b", "b")),
        "power_series_two_term" => {
            format!("Y = {} \\cdot X^{{{}}} + {} \\cdot X^{{{}}}", ctx.val("A", "A"), ctx.val("B", "B"), ctx.val("C", "C"), ctx.val("D", "D"),)
        }
        "logarithmic_curve" => format!("Y = {} \\cdot \\ln(X) + {}", ctx.val("a", "a"), ctx.val("b", "b")),
        "semi_log_line" | "semilog_line" => {
            format!("Y = {} \\cdot \\log(X) + {}", ctx.val("Slope", "\\text{Slope}"), ctx.val("YIntercept", "\\text{YIntercept}"))
        }
        "log_log_line" | "loglog_line" => {
            format!("Y = 10^{{{} \\cdot \\log(X) + {}}}", ctx.val("Slope", "\\text{Slope}"), ctx.val("LogYIntercept", "\\text{LogYIntercept}"))
        }
        "probit_line" => {
            format!("Y = 100 \\cdot \\Phi({} \\cdot X + {})", ctx.val("Slope", "\\text{Slope}"), ctx.val("Intercept", "\\text{Intercept}"))
        }
        _ => return None,
    };
    Some(s)
}

fn format_centered_poly_tex(order: usize, ctx: &EqCtx<'_>) -> String {
    let xc = ctx.val("XCenter", "X_{\\text{center}}");
    let mut terms = Vec::with_capacity(order + 1);
    for k in 0..=order {
        let cname = format!("b{k}");
        let sym = sub_token("b", k);
        let c = ctx.val(&cname, &sym);
        let term = match k {
            0 => c,
            1 => format!("{c} \\cdot (X - {xc})"),
            _ => format!("{c} \\cdot (X - {xc})^{{{k}}}"),
        };
        terms.push(term);
    }
    format!("Y = {}", terms.join(" + "))
}

fn format_nist_equation_tex(id: &str, ctx: &EqCtx<'_>) -> Option<String> {
    let base = id.strip_prefix("nist_").unwrap_or(id);
    format_nist_part1(base, ctx).or_else(|| format_nist_part2(base, ctx))
}

fn format_nist_part1(id: &str, ctx: &EqCtx<'_>) -> Option<String> {
    let b1 = ctx.val("b1", "b_1");
    let b2 = ctx.val("b2", "b_2");
    let b3 = ctx.val("b3", "b_3");
    let b4 = ctx.val("b4", "b_4");
    let b5 = ctx.val("b5", "b_5");
    let b6 = ctx.val("b6", "b_6");
    let b7 = ctx.val("b7", "b_7");

    let s = match id {
        "misra1a" | "boxbod" => format!("Y = {b1} \\cdot (1 - \\exp(-{b2} \\cdot X))"),
        "chwirut1" | "chwirut2" => format!("Y = \\frac{{\\exp(-{b1} \\cdot X)}}{{{b2} + {b3} \\cdot X}}"),
        "lanczos1" | "lanczos2" | "lanczos3" => {
            format!("Y = {b1} \\cdot \\exp(-{b2} \\cdot X) + {b3} \\cdot \\exp(-{b4} \\cdot X) + {b5} \\cdot \\exp(-{b6} \\cdot X)")
        }
        "gauss1" | "gauss2" | "gauss3" => format!(
            "Y = {b1} \\cdot \\exp(-{b2} \\cdot X) + {b3} \\cdot \\exp(-\\frac{{(X - {b4})^{{2}}}}{{{}^{{2}}}}) + {b6} \\cdot \\exp(-\\frac{{(X - {b7})^{{2}}}}{{{}^{{2}}}})",
            ctx.pow_base("b5", "b_5"),
            ctx.pow_base("b8", "b_8"),
        ),
        "danwood" => format!("Y = {b1} \\cdot X^{{{b2}}}"),
        "misra1b" => format!("Y = {b1} \\cdot (1 - (1 + \\frac{{{b2} \\cdot X}}{{2}})^{{-2}})"),
        "kirby2" => format!("Y = \\frac{{{b1} + {b2} \\cdot X + {b3} \\cdot X^2}}{{1 + {b4} \\cdot X + {b5} \\cdot X^2}}"),
        "hahn1" | "thurber" => {
            format!("Y = \\frac{{{b1} + {b2} \\cdot X + {b3} \\cdot X^2 + {b4} \\cdot X^3}}{{1 + {b5} \\cdot X + {b6} \\cdot X^2 + {b7} \\cdot X^3}}")
        }
        _ => return None,
    };
    Some(s)
}

fn format_nist_part2(id: &str, ctx: &EqCtx<'_>) -> Option<String> {
    let b1 = ctx.val("b1", "b_1");
    let b2 = ctx.val("b2", "b_2");
    let b3 = ctx.val("b3", "b_3");
    let b4 = ctx.val("b4", "b_4");
    let b5 = ctx.val("b5", "b_5");
    let b6 = ctx.val("b6", "b_6");
    let b7 = ctx.val("b7", "b_7");
    let b8 = ctx.val("b8", "b_8");
    let b9 = ctx.val("b9", "b_9");

    let s = match id {
        "nelson" => format!("Y = {b1} - {b2} \\cdot X_1 \\cdot \\exp(-{b3} \\cdot X_2)"),
        "mgh17" => format!("Y = {b1} + {b2} \\cdot \\exp(-{b4} \\cdot X) + {b3} \\cdot \\exp(-{b5} \\cdot X)"),
        "misra1c" => format!("Y = {b1} \\cdot (1 - (1 + 2 \\cdot {} \\cdot X)^{{-\\frac{{1}}{{2}}}})", ctx.factor("b2", "b_2")),
        "misra1d" => format!("Y = \\frac{{{b1} \\cdot {} \\cdot X}}{{1 + {b2} \\cdot X}}", ctx.factor("b2", "b_2")),
        "roszman1" => format!("Y = {b1} - {b2} \\cdot X - \\frac{{\\arctan(\\frac{{{b3}}}{{X - {b4}}})}}{{\\pi}}"),
        "enso" => format!(
            "Y = {b1} + {b2} \\cdot \\cos(\\frac{{2\\pi \\cdot X}}{{12}}) + {b3} \\cdot \\sin(\\frac{{2\\pi \\cdot X}}{{12}}) + {b5} \\cdot \\cos(\\frac{{2\\pi \\cdot X}}{{{b4}}}) + {b6} \\cdot \\sin(\\frac{{2\\pi \\cdot X}}{{{b4}}}) + {b8} \\cdot \\cos(\\frac{{2\\pi \\cdot X}}{{{b7}}}) + {b9} \\cdot \\sin(\\frac{{2\\pi \\cdot X}}{{{b7}}})"
        ),
        "mgh09" => format!("Y = \\frac{{{b1} \\cdot (X^2 + {b2} \\cdot X)}}{{X^2 + {b3} \\cdot X + {b4}}}"),
        "rat42" => format!("Y = \\frac{{{b1}}}{{1 + \\exp({b2} - {b3} \\cdot X)}}"),
        "mgh10" => format!("Y = {b1} \\cdot \\exp(\\frac{{{b2}}}{{X + {b3}}})"),
        "eckerle4" => {
            format!("Y = \\frac{{{b1}}}{{{b2}}} \\cdot \\exp(-\\frac{{(X - {b3})^{{2}}}}{{2 \\cdot {}^{{2}}}})", ctx.pow_base("b2", "b_2"))
        }
        "rat43" => format!("Y = \\frac{{{b1}}}{{(1 + \\exp({b2} - {b3} \\cdot X))^{{\\frac{{1}}{{{}}}}}}}", ctx.denom("b4", "b_4")),
        "bennett5" => format!("Y = {b1} \\cdot ({b2} + X)^{{-\\frac{{1}}{{{}}}}}", ctx.denom("b3", "b_3")),
        _ => return None,
    };
    Some(s)
}

// ---------------------------------------------------------------------------
// Custom DSL TeX formatter backed by `crate::dsl`'s parser/AST (zero recursion)
// ---------------------------------------------------------------------------

const MAX_DSL_INLINED_NODES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OdeLhsKind {
    DyDx,
    DyDt,
    YPrime,
}

impl OdeLhsKind {
    fn to_tex(self) -> &'static str {
        match self {
            Self::DyDx => "\\frac{dY}{dX}",
            Self::DyDt => "\\frac{dY}{dt}",
            Self::YPrime => "Y'",
        }
    }
}

#[derive(Clone, Debug)]
enum DslRawStmt<'a> {
    OdeDerivative { lhs: OdeLhsKind, rhs: &'a str },
    OdeInitialOrLocal { lhs_trim: &'a str, rhs: &'a str },
    ExplicitY { rhs: &'a str },
    Local { name: &'a str, rhs: &'a str },
    Implicit { lhs: &'a str, rhs: &'a str },
    Bare { expr: &'a str },
}

impl DslRawStmt<'_> {
    fn is_output(&self) -> bool {
        matches!(self, Self::OdeDerivative { .. } | Self::ExplicitY { .. } | Self::Implicit { .. } | Self::Bare { .. })
    }
}

fn format_custom_dsl_tex(src: &str, params: &[(String, f64)], data_set_index: Option<usize>, substituted: bool, sig_figs: usize) -> String {
    if src.len() > MAX_DSL_SOURCE_BYTES {
        return "Y = f_{\\text{custom}}(X)".to_string();
    }

    let Some(script) = select_dsl_script(src, data_set_index) else {
        return "Y = f(X)".to_string();
    };

    let ctx = EqCtx { params, data_set_index, substituted, sig_figs };
    let tex = render_selected_dsl_script(&script, &ctx);
    normalize_tex_signs(&tex)
}

struct SelectedDslScript<'a> {
    locals: Vec<(&'a str, &'a str)>,
    target: DslRawStmt<'a>,
}

fn select_dsl_script(src: &str, data_set_index: Option<usize>) -> Option<SelectedDslScript<'_>> {
    let mut parsed: Vec<(DatasetFilter, DslRawStmt<'_>)> = Vec::new();
    let mut total_dataset_indices = 0_usize;
    let mut in_equation = true;
    for (idx, raw_line) in src.lines().enumerate() {
        if idx >= MAX_DSL_LINES {
            break;
        }
        let stripped = strip_dsl_comment(raw_line).trim();
        if stripped.is_empty() {
            continue;
        }
        if stripped.starts_with('[') && stripped.ends_with(']') {
            in_equation = matches!(parse_section_header(stripped, idx + 1), Ok(Some(DslSection::Equation)));
            continue;
        }
        if !in_equation {
            continue;
        }
        let (filter, body) = parse_dataset_prefix(stripped, idx + 1).unwrap_or((DatasetFilter::Any, stripped));
        let filter_len = match &filter {
            DatasetFilter::Any => 0,
            DatasetFilter::Include(v) | DatasetFilter::Exclude(v) => v.len(),
        };
        total_dataset_indices = total_dataset_indices.saturating_add(filter_len);
        if total_dataset_indices > MAX_TOTAL_DATASET_INDICES {
            return None;
        }
        let body = body.trim();
        if !body.is_empty() {
            parsed.push((filter, classify_dsl_line(body)));
        }
    }

    if parsed.is_empty() {
        return None;
    }

    let has_ode = parsed.iter().any(|(_, st)| matches!(st, DslRawStmt::OdeDerivative { .. }));
    let target_ds = data_set_index.unwrap_or(0);

    let chosen_idx = pick_target_stmt_index(&parsed, target_ds)?;
    let chosen_filter = &parsed[chosen_idx].0;
    let active_ds = if chosen_filter.matches(target_ds) {
        Some(target_ds)
    } else if let DatasetFilter::Include(indices) = chosen_filter {
        indices.first().copied()
    } else {
        None
    };

    let mut locals = Vec::new();
    for (filter, stmt) in &parsed[..chosen_idx] {
        let filter_ok = match active_ds {
            Some(ds) => filter.matches(ds),
            None => *filter == DatasetFilter::Any,
        };
        if !filter_ok {
            continue;
        }
        match stmt {
            DslRawStmt::Local { name, rhs } => locals.push((*name, *rhs)),
            DslRawStmt::OdeInitialOrLocal { lhs_trim, rhs } if !has_ode && is_valid_ident(lhs_trim) => {
                locals.push((*lhs_trim, *rhs));
            }
            _ => {}
        }
    }

    Some(SelectedDslScript { locals, target: parsed[chosen_idx].1.clone() })
}

fn pick_target_stmt_index(parsed: &[(DatasetFilter, DslRawStmt<'_>)], target_ds: usize) -> Option<usize> {
    let scoped_match = parsed.iter().rposition(|(f, st)| st.is_output() && *f != DatasetFilter::Any && f.matches(target_ds));
    if scoped_match.is_some() {
        return scoped_match;
    }
    let default_match = parsed.iter().rposition(|(f, st)| st.is_output() && f.matches(target_ds));
    if default_match.is_some() {
        return default_match;
    }
    parsed.iter().rposition(|(_, st)| st.is_output()).or_else(|| parsed.len().checked_sub(1))
}

fn classify_dsl_line(line: &str) -> DslRawStmt<'_> {
    let Some((lhs_raw, rhs_raw)) = split_top_level_assignment(line) else {
        return DslRawStmt::Bare { expr: line.trim() };
    };
    let lhs_trim = lhs_raw.trim();
    let rhs_trim = rhs_raw.trim();
    let lhs_compact: String = lhs_trim.chars().filter(|c| !c.is_whitespace()).collect();
    let lhs_upper = lhs_compact.to_ascii_uppercase();
    match lhs_upper.as_str() {
        "DY/DX" | "D(Y,X)" => DslRawStmt::OdeDerivative { lhs: OdeLhsKind::DyDx, rhs: rhs_trim },
        "DY/DT" | "D(Y,T)" => DslRawStmt::OdeDerivative { lhs: OdeLhsKind::DyDt, rhs: rhs_trim },
        "Y'" => DslRawStmt::OdeDerivative { lhs: OdeLhsKind::YPrime, rhs: rhs_trim },
        "Y" => DslRawStmt::ExplicitY { rhs: rhs_trim },
        "Y0" => DslRawStmt::OdeInitialOrLocal { lhs_trim, rhs: rhs_trim },
        _ if lhs_upper.starts_with("Y(") && lhs_upper.ends_with(')') => DslRawStmt::OdeInitialOrLocal { lhs_trim, rhs: rhs_trim },
        _ if is_valid_ident(lhs_trim) => DslRawStmt::Local { name: lhs_trim, rhs: rhs_trim },
        _ => DslRawStmt::Implicit { lhs: lhs_trim, rhs: rhs_trim },
    }
}

fn strip_dsl_comment(line: &str) -> &str {
    let mut best = line;
    for marker in [";", "#", "//"] {
        if let Some((prefix, _)) = line.split_once(marker)
            && prefix.len() < best.len()
        {
            best = prefix;
        }
    }
    best
}

struct DslRenderEnv<'a> {
    env: HashMap<String, (Expr, usize)>,
    shadowed: HashSet<&'a str>,
    parsed_locals: Vec<(&'a str, &'a str, Option<Expr>)>,
    can_inline: bool,
}

fn render_selected_dsl_script(script: &SelectedDslScript<'_>, ctx: &EqCtx<'_>) -> String {
    let mut env: HashMap<String, (Expr, usize)> = HashMap::new();
    let mut shadowed: HashSet<&str> = HashSet::new();
    let mut parsed_locals: Vec<(&str, &str, Option<Expr>)> = Vec::with_capacity(script.locals.len());
    let mut all_locals_parsed = true;

    for &(name, rhs_src) in &script.locals {
        if let Some(raw_expr) = parse_algebraic_expr(rhs_src) {
            if let Some(resolved) = inline_locals_iterative(&raw_expr, &env, MAX_DSL_INLINED_NODES) {
                env.insert(name.to_string(), resolved);
                shadowed.insert(name);
            } else {
                all_locals_parsed = false;
            }
            parsed_locals.push((name, rhs_src, Some(raw_expr)));
        } else {
            all_locals_parsed = false;
            parsed_locals.push((name, rhs_src, None));
        }
    }

    let render_env = DslRenderEnv { can_inline: all_locals_parsed && !env.is_empty(), env, shadowed, parsed_locals };

    match &script.target {
        DslRawStmt::ExplicitY { rhs } => render_lhs_rhs_dsl("Y", None, rhs, &render_env, ctx),
        DslRawStmt::OdeDerivative { lhs, rhs } => render_lhs_rhs_dsl(lhs.to_tex(), None, rhs, &render_env, ctx),
        DslRawStmt::OdeInitialOrLocal { lhs_trim, rhs } => render_lhs_rhs_dsl(&tex_ident(lhs_trim), None, rhs, &render_env, ctx),
        DslRawStmt::Local { name, rhs } => render_lhs_rhs_dsl(&tex_ident(name), None, rhs, &render_env, ctx),
        DslRawStmt::Implicit { lhs, rhs } => render_lhs_rhs_dsl("", Some(lhs), rhs, &render_env, ctx),
        DslRawStmt::Bare { expr } => {
            if let Some(raw_rhs) = parse_algebraic_expr(expr) {
                if render_env.can_inline
                    && let Some((inlined_rhs, _)) = inline_locals_iterative(&raw_rhs, &render_env.env, MAX_DSL_INLINED_NODES)
                {
                    let rhs_tex = render_dsl_tex_iterative(&inlined_rhs, ctx, None);
                    return if expr_references_y(&inlined_rhs) { format!("{rhs_tex} = 0") } else { format!("Y = {rhs_tex}") };
                }
                let rhs_tex = render_dsl_tex_iterative(&raw_rhs, ctx, Some(&render_env.shadowed));
                let core = if expr_references_y(&raw_rhs) { format!("{rhs_tex} = 0") } else { format!("Y = {rhs_tex}") };
                return append_helper_locals(core, &render_env.parsed_locals, ctx);
            }
            let fb = fallback_custom_dsl_tex("Y", expr, ctx);
            append_helper_locals(fb, &render_env.parsed_locals, ctx)
        }
    }
}

fn render_lhs_rhs_dsl(fixed_lhs_tex: &str, lhs_expr_src: Option<&str>, rhs_src: &str, render_env: &DslRenderEnv<'_>, ctx: &EqCtx<'_>) -> String {
    let raw_lhs = lhs_expr_src.and_then(parse_algebraic_expr);
    let raw_rhs = parse_algebraic_expr(rhs_src);

    if let Some(rhs_expr) = raw_rhs.as_ref()
        && (lhs_expr_src.is_none() || raw_lhs.is_some())
    {
        if render_env.can_inline {
            let inlined_rhs = inline_locals_iterative(rhs_expr, &render_env.env, MAX_DSL_INLINED_NODES);
            let inlined_lhs = raw_lhs.as_ref().and_then(|l| inline_locals_iterative(l, &render_env.env, MAX_DSL_INLINED_NODES));
            if let Some((r_expr, _)) = inlined_rhs
                && (raw_lhs.is_none() || inlined_lhs.is_some())
            {
                let lhs_tex = inlined_lhs.map_or_else(|| fixed_lhs_tex.to_string(), |(l_expr, _)| render_dsl_tex_iterative(&l_expr, ctx, None));
                let rhs_tex = render_dsl_tex_iterative(&r_expr, ctx, None);
                return format!("{lhs_tex} = {rhs_tex}");
            }
        }
        let lhs_tex =
            raw_lhs.as_ref().map_or_else(|| fixed_lhs_tex.to_string(), |l_expr| render_dsl_tex_iterative(l_expr, ctx, Some(&render_env.shadowed)));
        let rhs_tex = render_dsl_tex_iterative(rhs_expr, ctx, Some(&render_env.shadowed));
        return append_helper_locals(format!("{lhs_tex} = {rhs_tex}"), &render_env.parsed_locals, ctx);
    }

    let lhs_tex = if let Some(l_src) = lhs_expr_src {
        raw_lhs.as_ref().map_or_else(|| fallback_lhs_tex(l_src, ctx), |l_expr| render_dsl_tex_iterative(l_expr, ctx, Some(&render_env.shadowed)))
    } else {
        fixed_lhs_tex.to_string()
    };
    let core = fallback_custom_dsl_tex(&lhs_tex, rhs_src, ctx);
    append_helper_locals(core, &render_env.parsed_locals, ctx)
}

fn fallback_lhs_tex(lhs_src: &str, ctx: &EqCtx<'_>) -> String {
    let s = substitute_and_escape_fallback(lhs_src.trim(), ctx);
    sanitize_fallback_tex_specials(&format_fallback_operators(&s))
}

fn append_helper_locals(core: String, parsed_locals: &[(&str, &str, Option<Expr>)], ctx: &EqCtx<'_>) -> String {
    if parsed_locals.is_empty() {
        return core;
    }
    let mut env_names: HashSet<&str> = HashSet::new();
    let mut parts = Vec::with_capacity(parsed_locals.len() + 1);
    parts.push(core);
    for &(name, rhs_src, ref opt_expr) in parsed_locals {
        let lhs_tex = tex_ident(name);
        let stmt = if let Some(expr) = opt_expr {
            let rhs_tex = render_dsl_tex_iterative(expr, ctx, Some(&env_names));
            format!("{lhs_tex} = {rhs_tex}")
        } else {
            fallback_custom_dsl_tex(&lhs_tex, rhs_src, ctx)
        };
        env_names.insert(name);
        parts.push(stmt);
    }
    parts.join("; \\quad ")
}

fn parse_algebraic_expr(src: &str) -> Option<Expr> {
    parse_expr_collecting_locs(src, 1, 0, None).ok()
}

fn inline_locals_iterative(root: &Expr, env: &HashMap<String, (Expr, usize)>, max_nodes: usize) -> Option<(Expr, usize)> {
    enum InlineTask<'a> {
        Visit(&'a Expr),
        BuildUnary(UnaryOp),
        BuildBin(BinaryOp),
        BuildCall(&'a str, usize),
        BuildIf,
    }

    let mut tasks = vec![InlineTask::Visit(root)];
    let mut results: Vec<Expr> = Vec::new();
    let mut count = 0usize;

    while let Some(task) = tasks.pop() {
        match task {
            InlineTask::Visit(node) => match node {
                Expr::Num(v) => {
                    count += 1;
                    if count > max_nodes {
                        return None;
                    }
                    results.push(Expr::Num(*v));
                }
                Expr::Ident(name) => {
                    if let Some((local_expr, local_nodes)) = env.get(name) {
                        count = count.checked_add(*local_nodes)?;
                        if count > max_nodes {
                            return None;
                        }
                        results.push(local_expr.clone());
                    } else {
                        count += 1;
                        if count > max_nodes {
                            return None;
                        }
                        results.push(Expr::Ident(name.clone()));
                    }
                }
                Expr::Unary { op, rhs } => {
                    count += 1;
                    if count > max_nodes {
                        return None;
                    }
                    tasks.push(InlineTask::BuildUnary(*op));
                    tasks.push(InlineTask::Visit(rhs));
                }
                Expr::Binary { op, lhs, rhs } => {
                    count += 1;
                    if count > max_nodes {
                        return None;
                    }
                    tasks.push(InlineTask::BuildBin(*op));
                    tasks.push(InlineTask::Visit(rhs));
                    tasks.push(InlineTask::Visit(lhs));
                }
                Expr::Call { name, args } => {
                    count += 1;
                    if count > max_nodes {
                        return None;
                    }
                    tasks.push(InlineTask::BuildCall(name.as_str(), args.len()));
                    for arg in args.iter().rev() {
                        tasks.push(InlineTask::Visit(arg));
                    }
                }
                Expr::If { cond, then_br, else_br } => {
                    count += 1;
                    if count > max_nodes {
                        return None;
                    }
                    tasks.push(InlineTask::BuildIf);
                    tasks.push(InlineTask::Visit(else_br));
                    tasks.push(InlineTask::Visit(then_br));
                    tasks.push(InlineTask::Visit(cond));
                }
            },
            InlineTask::BuildUnary(op) => {
                let inner = results.pop()?;
                results.push(Expr::Unary { op, rhs: Box::new(inner) });
            }
            InlineTask::BuildBin(op) => {
                let rhs = results.pop()?;
                let lhs = results.pop()?;
                results.push(Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) });
            }
            InlineTask::BuildCall(name, arity) => {
                if results.len() < arity {
                    return None;
                }
                let args = results.split_off(results.len() - arity);
                results.push(Expr::Call { name: name.to_owned(), args });
            }
            InlineTask::BuildIf => {
                let else_br = results.pop()?;
                let then_br = results.pop()?;
                let cond = results.pop()?;
                results.push(Expr::If { cond: Box::new(cond), then_br: Box::new(then_br), else_br: Box::new(else_br) });
            }
        }
    }
    if results.len() == 1 { results.pop().map(|e| (e, count)) } else { None }
}

fn expr_references_y(root: &Expr) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Ident(name) if name.eq_ignore_ascii_case("Y") => return true,
            Expr::Num(_) | Expr::Ident(_) => {}
            Expr::Unary { rhs, .. } => stack.push(rhs),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(rhs);
                stack.push(lhs);
            }
            Expr::Call { args, .. } => stack.extend(args.iter()),
            Expr::If { cond, then_br, else_br } => {
                stack.push(else_br);
                stack.push(then_br);
                stack.push(cond);
            }
        }
    }
    false
}

fn has_leading_dsl_neg(mut expr: &Expr) -> bool {
    loop {
        match expr {
            Expr::Unary { op: UnaryOp::Neg, .. } => return true,
            Expr::Binary { op: BinaryOp::Mul, lhs, .. } => expr = lhs,
            _ => return false,
        }
    }
}

fn has_balanced_outer_parens(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 2 || bytes.first() != Some(&b'(') || bytes.last() != Some(&b')') {
        return false;
    }
    let mut depth = 0_usize;
    for (idx, &byte) in bytes.iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && idx + 1 < bytes.len() {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

enum RenderTask<'a> {
    Visit { expr: &'a Expr, parent_prec: u8, strip_left_neg: bool },
    FinishNeg { parent_prec: u8, need_paren: bool },
    FinishNot { parent_prec: u8, need_paren: bool },
    FinishAdd { parent_prec: u8 },
    FinishSub { parent_prec: u8, r_paren: bool },
    FinishMul { parent_prec: u8 },
    FinishRem { parent_prec: u8 },
    FinishDiv { parent_prec: u8, strip_neg_active: bool, inner_is_add_sub: bool },
    FinishPow { lhs_is_bin_or_neg: bool, lhs_is_pow: bool },
    FinishInfix { op_tex: &'static str, parent_prec: u8 },
    FinishIf { parent_prec: u8 },
    FinishCall { name: &'a str, arg_count: usize, first_is_atom: bool },
}

fn render_dsl_tex_iterative(root: &Expr, ctx: &EqCtx<'_>, shadowed_locals: Option<&HashSet<&str>>) -> String {
    let mut tasks = vec![RenderTask::Visit { expr: root, parent_prec: 0, strip_left_neg: false }];
    let mut results: Vec<String> = Vec::new();

    while let Some(task) = tasks.pop() {
        match task {
            RenderTask::Visit { expr, parent_prec, strip_left_neg } => {
                push_visit_tasks(expr, parent_prec, strip_left_neg, ctx, shadowed_locals, &mut tasks, &mut results);
            }
            RenderTask::FinishNeg { parent_prec, need_paren } => {
                let body = results.pop().unwrap_or_default();
                if !need_paren && let Some(pos_body) = body.strip_prefix('-') {
                    let out = if parent_prec >= 3 && (pos_body.contains("\\times") || pos_body.contains("\\cdot") || pos_body.contains('/')) {
                        format!("({pos_body})")
                    } else {
                        pos_body.to_string()
                    };
                    results.push(out);
                } else {
                    let neg = if need_paren { format!("-({body})") } else { format!("-{body}") };
                    results.push(if parent_prec >= 3 { format!("({neg})") } else { neg });
                }
            }
            RenderTask::FinishNot { parent_prec, need_paren } => {
                let body = results.pop().unwrap_or_default();
                let s = if need_paren { format!("\\neg ({body})") } else { format!("\\neg {body}") };
                results.push(if parent_prec >= 3 { format!("({s})") } else { s });
            }
            RenderTask::FinishAdd { parent_prec } => {
                let r = results.pop().unwrap_or_default();
                let l = results.pop().unwrap_or_default();
                let s = format!("{l} + {r}");
                results.push(if parent_prec > 1 { format!("({s})") } else { s });
            }
            RenderTask::FinishSub { parent_prec, r_paren } => {
                let r = results.pop().unwrap_or_default();
                let l = results.pop().unwrap_or_default();
                let rhs_str = if r_paren { format!("({r})") } else { r };
                let s = format!("{l} - {rhs_str}");
                results.push(if parent_prec > 1 { format!("({s})") } else { s });
            }
            RenderTask::FinishMul { parent_prec } => {
                let r = results.pop().unwrap_or_default();
                let l = results.pop().unwrap_or_default();
                let s = format!("{l} \\cdot {r}");
                results.push(if parent_prec > 2 { format!("({s})") } else { s });
            }
            RenderTask::FinishRem { parent_prec } => {
                let r = results.pop().unwrap_or_default();
                let l = results.pop().unwrap_or_default();
                let s = format!("{l} \\bmod {r}");
                results.push(if parent_prec > 2 { format!("({s})") } else { s });
            }
            RenderTask::FinishDiv { parent_prec, strip_neg_active, inner_is_add_sub } => {
                let d = results.pop().unwrap_or_default();
                let n = results.pop().unwrap_or_default();
                if strip_neg_active {
                    if !inner_is_add_sub && let Some(pos) = n.strip_prefix('-') {
                        let frac = format!("\\frac{{{pos}}}{{{d}}}");
                        results.push(if parent_prec >= 3 && pos.contains("\\times") { format!("({frac})") } else { frac });
                    } else {
                        let frac = format!("-\\frac{{{n}}}{{{d}}}");
                        results.push(if parent_prec >= 3 { format!("({frac})") } else { frac });
                    }
                } else {
                    results.push(format!("\\frac{{{n}}}{{{d}}}"));
                }
            }
            RenderTask::FinishPow { lhs_is_bin_or_neg, lhs_is_pow } => {
                let e = results.pop().unwrap_or_default();
                let mut b = results.pop().unwrap_or_default();
                let already_wrapped = has_balanced_outer_parens(&b);
                let base_needs_paren = lhs_is_pow || lhs_is_bin_or_neg || b.starts_with('-') || b.contains("\\times") || b.starts_with("\\frac");
                if base_needs_paren && !already_wrapped {
                    b = format!("({b})");
                }
                results.push(format!("{b}^{{{e}}}"));
            }
            RenderTask::FinishInfix { op_tex, parent_prec } => {
                let r = results.pop().unwrap_or_default();
                let l = results.pop().unwrap_or_default();
                let s = format!("{l} {op_tex} {r}");
                results.push(if parent_prec > 0 { format!("({s})") } else { s });
            }
            RenderTask::FinishIf { parent_prec } => {
                let e = results.pop().unwrap_or_default();
                let t = results.pop().unwrap_or_default();
                let c = results.pop().unwrap_or_default();
                let s = format!("\\text{{IF}}({c}, {t}, {e})");
                results.push(if parent_prec >= 4 { format!("({s})") } else { s });
            }
            RenderTask::FinishCall { name, arg_count, first_is_atom } => {
                let start = results.len().saturating_sub(arg_count);
                let args = results.split_off(start);
                results.push(render_dsl_call_strings(name, &args, first_is_atom));
            }
        }
    }

    results.pop().unwrap_or_default()
}

fn push_visit_tasks<'a>(
    expr: &'a Expr,
    parent_prec: u8,
    strip_left_neg: bool,
    ctx: &EqCtx<'_>,
    shadowed_locals: Option<&HashSet<&str>>,
    tasks: &mut Vec<RenderTask<'a>>,
    results: &mut Vec<String>,
) {
    match expr {
        Expr::Num(v) => {
            let formatted = format_dsl_num(*v, ctx.sig_figs);
            let out =
                if parent_prec >= 3 && (formatted.starts_with('-') || formatted.contains("\\times")) { format!("({formatted})") } else { formatted };
            results.push(out);
        }
        Expr::Ident(name) => {
            let is_shadowed = shadowed_locals.is_some_and(|env| env.contains(name.as_str()));
            let rendered = if ctx.substituted
                && !is_shadowed
                && let Some(v) = ctx.find(name)
            {
                let s = ctx.num(v);
                if parent_prec >= 3 && (s.starts_with('-') || s.contains("\\times")) { format!("({s})") } else { s }
            } else {
                match name.to_ascii_lowercase().as_str() {
                    "pi" => "\\pi".to_string(),
                    "inf" | "infinity" => "\\infty".to_string(),
                    _ => tex_ident(name),
                }
            };
            results.push(rendered);
        }
        Expr::Unary { op: UnaryOp::Neg, rhs } => {
            if strip_left_neg {
                tasks.push(RenderTask::Visit { expr: rhs, parent_prec, strip_left_neg: false });
            } else {
                let need_paren = matches!(rhs.as_ref(), Expr::Binary { op: BinaryOp::Add | BinaryOp::Sub, .. });
                tasks.push(RenderTask::FinishNeg { parent_prec, need_paren });
                tasks.push(RenderTask::Visit { expr: rhs, parent_prec: 2, strip_left_neg: false });
            }
        }
        Expr::Unary { op: UnaryOp::Not, rhs } => {
            let need_paren = matches!(rhs.as_ref(), Expr::Binary { .. } | Expr::If { .. });
            tasks.push(RenderTask::FinishNot { parent_prec, need_paren });
            tasks.push(RenderTask::Visit { expr: rhs, parent_prec: 0, strip_left_neg: false });
        }
        Expr::Binary { op: BinaryOp::Add, lhs, rhs } => {
            tasks.push(RenderTask::FinishAdd { parent_prec });
            tasks.push(RenderTask::Visit { expr: rhs, parent_prec: 1, strip_left_neg: false });
            tasks.push(RenderTask::Visit { expr: lhs, parent_prec: 1, strip_left_neg: false });
        }
        Expr::Binary { op: BinaryOp::Sub, lhs, rhs } => {
            let r_paren = matches!(rhs.as_ref(), Expr::Binary { op: BinaryOp::Add | BinaryOp::Sub, .. });
            tasks.push(RenderTask::FinishSub { parent_prec, r_paren });
            tasks.push(RenderTask::Visit { expr: rhs, parent_prec: 2, strip_left_neg: false });
            tasks.push(RenderTask::Visit { expr: lhs, parent_prec: 1, strip_left_neg: false });
        }
        Expr::Binary { op: BinaryOp::Mul, lhs, rhs } => {
            tasks.push(RenderTask::FinishMul { parent_prec });
            tasks.push(RenderTask::Visit { expr: rhs, parent_prec: 3, strip_left_neg: false });
            tasks.push(RenderTask::Visit { expr: lhs, parent_prec: 2, strip_left_neg });
        }
        Expr::Binary { op: BinaryOp::Rem, lhs, rhs } => {
            tasks.push(RenderTask::FinishRem { parent_prec });
            tasks.push(RenderTask::Visit { expr: rhs, parent_prec: 3, strip_left_neg: false });
            tasks.push(RenderTask::Visit { expr: lhs, parent_prec: 2, strip_left_neg: false });
        }
        Expr::Binary { op: BinaryOp::Div, lhs, rhs } => {
            let strip_neg_active = has_leading_dsl_neg(lhs);
            let inner_is_add_sub = matches!(
                lhs.as_ref(),
                Expr::Unary { op: UnaryOp::Neg, rhs: inner } if matches!(inner.as_ref(), Expr::Binary { op: BinaryOp::Add | BinaryOp::Sub, .. })
            );
            tasks.push(RenderTask::FinishDiv { parent_prec, strip_neg_active, inner_is_add_sub });
            tasks.push(RenderTask::Visit { expr: rhs, parent_prec: 0, strip_left_neg: false });
            tasks.push(RenderTask::Visit { expr: lhs, parent_prec: 0, strip_left_neg: strip_neg_active });
        }
        Expr::Binary { op: BinaryOp::Pow, lhs, rhs } => {
            let lhs_is_pow = matches!(lhs.as_ref(), Expr::Binary { op: BinaryOp::Pow, .. })
                || matches!(lhs.as_ref(), Expr::Call { name, .. } if name.eq_ignore_ascii_case("pow") || name.eq_ignore_ascii_case("exp10"));
            let lhs_is_bin_or_neg = matches!(lhs.as_ref(), Expr::Binary { .. } | Expr::Unary { .. });
            tasks.push(RenderTask::FinishPow { lhs_is_bin_or_neg, lhs_is_pow });
            tasks.push(RenderTask::Visit { expr: rhs, parent_prec: 0, strip_left_neg: false });
            tasks.push(RenderTask::Visit { expr: lhs, parent_prec: 4, strip_left_neg: false });
        }
        Expr::Binary {
            op: op @ (BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge | BinaryOp::And | BinaryOp::Or),
            lhs,
            rhs,
        } => {
            let (op_tex, child_prec) = match op {
                BinaryOp::Eq => ("=", 1),
                BinaryOp::Ne => ("\\neq", 1),
                BinaryOp::Lt => ("<", 1),
                BinaryOp::Le => ("\\leq", 1),
                BinaryOp::Gt => (">", 1),
                BinaryOp::Ge => ("\\geq", 1),
                BinaryOp::And => ("\\land", 0),
                BinaryOp::Or => ("\\lor", 0),
                _ => ("", 0),
            };
            let l_prec = if matches!(op, BinaryOp::And) && matches!(lhs.as_ref(), Expr::Binary { op: BinaryOp::Or, .. }) { 1 } else { child_prec };
            let r_prec = if matches!(op, BinaryOp::And) && matches!(rhs.as_ref(), Expr::Binary { op: BinaryOp::Or, .. }) { 1 } else { child_prec };
            tasks.push(RenderTask::FinishInfix { op_tex, parent_prec });
            tasks.push(RenderTask::Visit { expr: rhs, parent_prec: r_prec, strip_left_neg: false });
            tasks.push(RenderTask::Visit { expr: lhs, parent_prec: l_prec, strip_left_neg: false });
        }
        Expr::If { cond, then_br, else_br } => {
            tasks.push(RenderTask::FinishIf { parent_prec });
            tasks.push(RenderTask::Visit { expr: else_br, parent_prec: 0, strip_left_neg: false });
            tasks.push(RenderTask::Visit { expr: then_br, parent_prec: 0, strip_left_neg: false });
            tasks.push(RenderTask::Visit { expr: cond, parent_prec: 0, strip_left_neg: false });
        }
        Expr::Call { name, args } => {
            let first_is_atom = matches!(args.first(), Some(Expr::Ident(_) | Expr::Num(_)));
            tasks.push(RenderTask::FinishCall { name: name.as_str(), arg_count: args.len(), first_is_atom });
            for arg in args.iter().rev() {
                tasks.push(RenderTask::Visit { expr: arg, parent_prec: 0, strip_left_neg: false });
            }
        }
    }
}

fn render_dsl_call_strings(name: &str, rendered_args: &[String], first_is_atom: bool) -> String {
    let lower = name.to_ascii_lowercase();
    match (lower.as_str(), rendered_args) {
        ("exp", [a]) => format!("\\exp({a})"),
        ("exp10", [a]) => format!("10^{{{a}}}"),
        ("ln" | "loge", [a]) => format!("\\ln({a})"),
        ("log" | "log10", [a]) => format!("\\log({a})"),
        ("log2", [a]) => format!("\\log_2({a})"),
        ("sqrt", [a]) => format!("\\sqrt{{{a}}}"),
        ("abs", [a]) => format!("|{a}|"),
        ("sin", [a]) => format!("\\sin({a})"),
        ("cos", [a]) => format!("\\cos({a})"),
        ("tan", [a]) => format!("\\tan({a})"),
        ("sinh", [a]) => format!("\\sinh({a})"),
        ("cosh", [a]) => format!("\\cosh({a})"),
        ("tanh", [a]) => format!("\\tanh({a})"),
        ("arcsin" | "asin", [a]) => format!("\\arcsin({a})"),
        ("arccos" | "acos", [a]) => format!("\\arccos({a})"),
        ("arctan" | "atan", [a]) => format!("\\arctan({a})"),
        ("sec", [a]) => format!("\\sec({a})"),
        ("csc", [a]) => format!("\\csc({a})"),
        ("cot", [a]) => format!("\\cot({a})"),
        ("erf", [a]) => format!("\\erf({a})"),
        ("erfc", [a]) => format!("\\erfc({a})"),
        ("min", _) => format!("\\min({})", rendered_args.join(", ")),
        ("max", _) => format!("\\max({})", rendered_args.join(", ")),
        ("pow", [b, e]) => {
            if first_is_atom && !b.starts_with('-') && !b.contains("\\times") {
                format!("{b}^{{{e}}}")
            } else {
                format!("({b})^{{{e}}}")
            }
        }
        _ => format!("\\text{{{}}}({})", escape_tex_text(name), rendered_args.join(", ")),
    }
}

fn format_dsl_num(value: f64, sig_figs: usize) -> String {
    let abs = value.abs();
    if value != 0.0 && value.is_finite() && !(1e-4..1e6).contains(&abs) {
        format_number_sig_figs(value, sig_figs)
    } else {
        format_number_sig_figs(value, 15)
    }
}

fn escape_tex_fallback_char(ch: char, out: &mut String) {
    match ch {
        '\\' => out.push_str("\\\\"),
        '{' => out.push_str("\\{"),
        '}' => out.push_str("\\}"),
        '$' => out.push_str("\\$"),
        '#' => out.push_str("\\#"),
        '%' => out.push_str("\\%"),
        '&' => out.push_str("\\&"),
        '_' => out.push_str("\\_"),
        '<' => out.push_str("\\textless{}"),
        '>' => out.push_str("\\textgreater{}"),
        _ => out.push(ch),
    }
}

fn escape_tex_fallback_into(input: &str, out: &mut String) {
    for ch in input.chars() {
        escape_tex_fallback_char(ch, out);
    }
}

fn format_fallback_operators(input: &str) -> String {
    let with_pow = input.replace("**", "^");
    with_pow.replace('*', " \\cdot ")
}

const MAX_FALLBACK_POWERS: usize = 8;

fn is_fallback_func_open_paren(prefix: &str) -> bool {
    const FUNCS: [&str; 9] = ["\\exp", "\\log", "\\ln", "\\text{sqrt}", "\\sin", "\\cos", "\\tan", "\\min", "\\max"];
    FUNCS.iter().any(|f| prefix.ends_with(f))
}

fn is_fallback_power_break_token(s: &str, pos: usize) -> Option<usize> {
    let rest = s.get(pos..)?;
    for kw in ["\\cdot", "\\textless{}", "\\textgreater{}"] {
        if rest.starts_with(kw) {
            return Some(kw.len());
        }
    }
    None
}

/// Escape chained/excessive/orphan `^` and unmatched `(`, `)`, `[`, `]`, `|` delimiters in the
/// unparseable custom-DSL fallback path (`GAP-01-01`, `GAP-01-02`) without disturbing simple
/// balanced expressions or single recognized function-call prefixes.
fn sanitize_fallback_tex_specials(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut paren_stack: Vec<usize> = Vec::new();
    let mut bracket_stack: Vec<usize> = Vec::new();
    let mut pipe_stack: Vec<(usize, usize, usize)> = Vec::new();
    let mut unmatched = vec![false; bytes.len()];
    let mut unmatched_open_parens: Vec<usize> = Vec::new();
    let mut caret_positions: Vec<(usize, bool)> = Vec::new();
    let mut seen_caret_in_term = false;
    let mut has_operand_before = false;
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'\\' {
            if let Some(kw_len) = is_fallback_power_break_token(input, i) {
                seen_caret_in_term = false;
                has_operand_before = false;
                i += kw_len;
                continue;
            }
            if input.get(i..).is_some_and(|s| s.starts_with("\\text{")) {
                i += "\\text{".len();
                let mut depth = 1usize;
                while i < bytes.len() && depth > 0 {
                    if bytes[i] == b'\\' {
                        i += if i + 1 < bytes.len() { 2 } else { 1 };
                        continue;
                    }
                    if bytes[i] == b'{' {
                        depth += 1;
                    } else if bytes[i] == b'}' {
                        depth -= 1;
                    }
                    i += 1;
                }
                has_operand_before = true;
                continue;
            }
            let next_len = input.get(i + 1..).and_then(|s| s.chars().next()).map_or(0, char::len_utf8);
            i += 1 + next_len;
            has_operand_before = true;
            continue;
        }
        match bytes[i] {
            b'(' => {
                paren_stack.push(i);
                seen_caret_in_term = false;
                has_operand_before = false;
                i += 1;
            }
            b')' => {
                if paren_stack.pop().is_none() {
                    unmatched[i] = true;
                    while let Some((p, _, _)) = pipe_stack.pop() {
                        unmatched[p] = true;
                    }
                } else {
                    while pipe_stack.last().is_some_and(|&(_, pd, _)| pd > paren_stack.len()) {
                        if let Some((p, _, _)) = pipe_stack.pop() {
                            unmatched[p] = true;
                        }
                    }
                }
                has_operand_before = true;
                i += 1;
            }
            b'[' => {
                bracket_stack.push(i);
                seen_caret_in_term = false;
                has_operand_before = false;
                i += 1;
            }
            b']' => {
                if bracket_stack.pop().is_none() {
                    unmatched[i] = true;
                    while let Some((p, _, _)) = pipe_stack.pop() {
                        unmatched[p] = true;
                    }
                } else {
                    while pipe_stack.last().is_some_and(|&(_, _, bd)| bd > bracket_stack.len()) {
                        if let Some((p, _, _)) = pipe_stack.pop() {
                            unmatched[p] = true;
                        }
                    }
                }
                has_operand_before = true;
                i += 1;
            }
            b'|' => {
                let p_depth = paren_stack.len();
                let b_depth = bracket_stack.len();
                if pipe_stack.last().is_some_and(|&(_, pd, bd)| pd == p_depth && bd == b_depth) {
                    pipe_stack.pop();
                } else {
                    pipe_stack.push((i, p_depth, b_depth));
                }
                seen_caret_in_term = false;
                has_operand_before = false;
                i += 1;
            }
            b'^' => {
                let after = input.get(i + 1..).unwrap_or("").trim_start();
                let has_operand_after = after.chars().next().is_some_and(|c| !matches!(c, '+' | '-' | '/' | '=' | ',' | ';' | ')' | ']' | '^'));
                let is_unsafe = seen_caret_in_term || !has_operand_before || !has_operand_after;
                caret_positions.push((i, is_unsafe));
                seen_caret_in_term = true;
                has_operand_before = false;
                i += 1;
            }
            b'+' | b'-' | b'/' | b'=' | b',' | b';' => {
                seen_caret_in_term = false;
                has_operand_before = false;
                i += 1;
            }
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
            }
            _ => {
                let ch_len = input.get(i..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
                has_operand_before = true;
                i += ch_len;
            }
        }
    }

    for pos in paren_stack {
        unmatched[pos] = true;
        unmatched_open_parens.push(pos);
    }
    for pos in bracket_stack {
        unmatched[pos] = true;
    }
    for (pos, _, _) in pipe_stack {
        unmatched[pos] = true;
    }
    if unmatched_open_parens.len() == 1
        && let Some(&pos) = unmatched_open_parens.first()
        && input.get(..pos).is_some_and(is_fallback_func_open_paren)
    {
        unmatched[pos] = false;
    }

    let escape_all_carets = caret_positions.len() > MAX_FALLBACK_POWERS;
    let mut escape_caret = vec![false; bytes.len()];
    for &(pos, is_unsafe) in &caret_positions {
        if escape_all_carets || is_unsafe {
            escape_caret[pos] = true;
        }
    }

    let mut out = String::with_capacity(input.len() + 16);
    for (idx, ch) in input.char_indices() {
        if escape_caret.get(idx).copied().unwrap_or(false) {
            out.push_str("\\^");
        } else if unmatched.get(idx).copied().unwrap_or(false) {
            match ch {
                '(' | ')' | '[' | ']' => {
                    out.push('\\');
                    out.push(ch);
                }
                '|' => out.push_str("\\text{|}"),
                _ => out.push(ch),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn fallback_custom_dsl_tex(lhs: &str, rhs_raw: &str, ctx: &EqCtx<'_>) -> String {
    let mut s = substitute_and_escape_fallback(rhs_raw, ctx);
    for (pat, rep) in [
        ("exp(", "\\exp("),
        ("log10(", "\\log("),
        ("log(", "\\log("),
        ("ln(", "\\ln("),
        ("sqrt(", "\\text{sqrt}("),
        ("sin(", "\\sin("),
        ("cos(", "\\cos("),
        ("tan(", "\\tan("),
        ("min(", "\\min("),
        ("max(", "\\max("),
    ] {
        s = replace_ident_word_prefix(&s, pat, rep);
    }
    s = format_fallback_operators(&s);
    s = sanitize_fallback_tex_specials(&s);
    format!("{lhs} = {}", s.trim())
}

fn substitute_and_escape_fallback(input: &str, ctx: &EqCtx<'_>) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            out.push('^');
            i += 2;
            continue;
        }
        if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            let next_non_space_idx = (i..chars.len()).find(|&k| !chars[k].is_whitespace());
            let next_non_space = next_non_space_idx.map(|k| chars[k]);
            let next_is_call = next_non_space == Some('(');
            if ctx.substituted
                && !next_is_call
                && let Some(val) = ctx.find(&ident)
            {
                let mut num = format_number_sig_figs(val, ctx.sig_figs);
                let prev_non_space = out.chars().rev().find(|c| !c.is_whitespace());
                let next_is_pow =
                    next_non_space == Some('^') || next_non_space_idx.is_some_and(|k| chars[k] == '*' && chars.get(k + 1) == Some(&'*'));
                if prev_non_space == Some('^') {
                    out.push('{');
                    out.push_str(&num);
                    out.push('}');
                } else {
                    if (num.starts_with('-') || num.contains("\\times")) && (matches!(prev_non_space, Some('*' | '/')) || next_is_pow) {
                        num = format!("({num})");
                    }
                    out.push_str(&num);
                }
            } else {
                escape_tex_fallback_into(&ident, &mut out);
            }
        } else {
            escape_tex_fallback_char(chars[i], &mut out);
            i += 1;
        }
    }
    out
}

fn replace_ident_word_prefix(input: &str, pat: &str, rep: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some((before, after)) = rest.split_once(pat) {
        let prefix_ok = before.chars().next_back().is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '\\');
        out.push_str(before);
        if prefix_ok {
            out.push_str(rep);
        } else {
            out.push_str(pat);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

fn greek_tex_escape(ident: &str) -> Option<&'static str> {
    let esc = match ident {
        "alpha" => "\\alpha",
        "beta" => "\\beta",
        "gamma" => "\\gamma",
        "delta" => "\\delta",
        "epsilon" => "\\epsilon",
        "varepsilon" => "\\varepsilon",
        "zeta" => "\\zeta",
        "eta" => "\\eta",
        "theta" => "\\theta",
        "vartheta" => "\\vartheta",
        "iota" => "\\iota",
        "kappa" => "\\kappa",
        "varkappa" => "\\varkappa",
        "lambda" => "\\lambda",
        "mu" => "\\mu",
        "nu" => "\\nu",
        "xi" => "\\xi",
        "omicron" => "\\omicron",
        "pi" => "\\pi",
        "varpi" => "\\varpi",
        "rho" => "\\rho",
        "varrho" => "\\varrho",
        "sigma" => "\\sigma",
        "varsigma" => "\\varsigma",
        "tau" => "\\tau",
        "upsilon" => "\\upsilon",
        "phi" => "\\phi",
        "varphi" => "\\varphi",
        "chi" => "\\chi",
        "psi" => "\\psi",
        "omega" => "\\omega",
        "Alpha" => "\\Alpha",
        "Beta" => "\\Beta",
        "Gamma" => "\\Gamma",
        "Delta" => "\\Delta",
        "Epsilon" => "\\Epsilon",
        "Zeta" => "\\Zeta",
        "Eta" => "\\Eta",
        "Theta" => "\\Theta",
        "Iota" => "\\Iota",
        "Kappa" => "\\Kappa",
        "Lambda" => "\\Lambda",
        "Mu" => "\\Mu",
        "Nu" => "\\Nu",
        "Xi" => "\\Xi",
        "Omicron" => "\\Omicron",
        "Pi" => "\\Pi",
        "Rho" => "\\Rho",
        "Sigma" => "\\Sigma",
        "Tau" => "\\Tau",
        "Upsilon" => "\\Upsilon",
        "Phi" => "\\Phi",
        "Chi" => "\\Chi",
        "Psi" => "\\Psi",
        "Omega" => "\\Omega",
        _ => return None,
    };
    Some(esc)
}

fn escape_tex_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\textbackslash{}"),
            '<' => out.push_str("\\textless{}"),
            '>' => out.push_str("\\textgreater{}"),
            '{' | '}' | '^' | '_' | '*' | '$' | '&' | '%' | '#' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

fn contains_tex_meta(s: &str) -> bool {
    s.chars().any(|ch| ch.is_whitespace() || matches!(ch, '\\' | '{' | '}' | '^' | '*' | '$' | '&' | '%' | '#' | '<' | '>'))
}

fn tex_ident_sub(sub: &str) -> String {
    if let Some(greek) = greek_tex_escape(sub) {
        return greek.to_string();
    }
    if (sub.len() == 1 && sub.chars().all(|c| c.is_ascii_alphanumeric())) || (!sub.is_empty() && sub.chars().all(|c| c.is_ascii_digit())) {
        return sub.to_string();
    }
    let clean = sub.replace('_', ",");
    format!("\\text{{{}}}", escape_tex_text(&clean))
}

fn tex_ident(ident: &str) -> String {
    if let Some(greek) = greek_tex_escape(ident) {
        return greek.to_string();
    }
    if ident.len() == 1 && ident.chars().all(|c| c.is_ascii_alphanumeric()) {
        return ident.to_string();
    }
    if !contains_tex_meta(ident)
        && let Some((head, tail)) = ident.split_once('_')
        && !head.is_empty()
        && !tail.is_empty()
        && !tail.starts_with('_')
        && !tail.ends_with('_')
        && !tail.contains("__")
    {
        return format!("{}_{{{}}}", tex_ident(head), tex_ident_sub(tail));
    }
    format!("\\text{{{}}}", escape_tex_text(ident))
}

// ---------------------------------------------------------------------------
// Sign normalization (`+ -` -> `- `, `- -` -> `+ `, `--` -> `+`, unary cleanup)
// ---------------------------------------------------------------------------

fn normalize_tex_signs(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < chars.len() {
        // Leave `\text{...}` content verbatim so labels like `one-site` or hyphens aren't touched.
        if slice_starts_with(&chars, i, "\\text{") {
            let end = copy_braced_group(&chars, i + 5, &mut out, "\\text");
            i = end;
            continue;
        }

        if chars[i] == '+' || chars[i] == '-' {
            let run_start = i;
            let mut neg_count = 0usize;
            let mut j = i;
            while j < chars.len() {
                if chars[j] == '-' {
                    neg_count += 1;
                    j += 1;
                } else if chars[j] == '+' || chars[j] == ' ' {
                    j += 1;
                } else {
                    break;
                }
            }
            // Trim trailing spaces included at the end of the run; remember if there was space originally.
            while j > run_start && chars[j - 1] == ' ' {
                j -= 1;
            }
            let had_leading_space = out.ends_with(' ');
            while out.ends_with(' ') {
                out.pop();
            }
            let had_trailing_space = chars.get(j) == Some(&' ');
            let prev_char = out.chars().next_back();
            let after_tex_op = ends_with_tex_op(&out);
            let is_unary = after_tex_op || is_unary_prev(prev_char);
            let is_negative = neg_count % 2 == 1;

            if is_unary {
                if after_tex_op || matches!(prev_char, Some('=' | ',' | ';' | '<' | '>')) {
                    out.push(' ');
                }
                if is_negative {
                    out.push('-');
                }
            } else {
                let sign = if is_negative { '-' } else { '+' };
                if prev_char.is_some() && !matches!(prev_char, Some('(' | '{' | '[')) {
                    out.push(' ');
                }
                out.push(sign);
                out.push(' ');
                let _ = (had_leading_space, had_trailing_space);
            }

            i = j;
            while i < chars.len() && chars[i] == ' ' {
                i += 1;
            }
            continue;
        }

        out.push(chars[i]);
        i += 1;
    }

    out.trim().to_string()
}

fn ends_with_tex_op(out: &str) -> bool {
    for kw in ["\\cdot", "\\times", "\\leq", "\\geq", "\\neq", "\\land", "\\lor", "\\neg", "\\bmod"] {
        if let Some(prefix) = out.strip_suffix(kw) {
            let backslashes = prefix.chars().rev().take_while(|&c| c == '\\').count();
            if backslashes % 2 == 0 {
                return true;
            }
        }
    }
    false
}

fn is_unary_prev(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(c) => matches!(c, '=' | '(' | '{' | '[' | ',' | ';' | '<' | '>' | '/' | '^'),
    }
}

fn slice_starts_with(chars: &[char], idx: usize, pat: &str) -> bool {
    (idx..).zip(pat.chars()).all(|(k, pc)| chars.get(k) == Some(&pc))
}

fn copy_braced_group(chars: &[char], brace_idx: usize, out: &mut String, prefix: &str) -> usize {
    out.push_str(prefix);
    let mut depth = 0usize;
    let mut i = brace_idx;
    while i < chars.len() {
        let ch = chars[i];
        out.push(ch);
        i += 1;
        if ch == '\\' {
            if let Some(&next_ch) = chars.get(i) {
                out.push(next_ch);
                i += 1;
            }
            continue;
        }
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                break;
            }
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::list_builtin_equations;

    #[test]
    fn number_formatting_avoids_neg_zero_and_uses_clean_decimals_or_sci() {
        assert_eq!(format_number_sig_figs(0.0, 4), "0");
        assert_eq!(format_number_sig_figs(-0.0, 4), "0");
        assert_eq!(format_number_sig_figs(f64::NAN, 4), "\\text{NaN}");
        assert_eq!(format_number_sig_figs(f64::INFINITY, 4), "\\infty");
        assert_eq!(format_number_sig_figs(f64::NEG_INFINITY, 4), "-\\infty");
        assert_eq!(format_number_sig_figs(12.345, 3), "12.3");
        assert_eq!(format_number_sig_figs(2.500, 4), "2.5");
        assert_eq!(format_number_sig_figs(100.0, 3), "100");
        assert_eq!(format_number_sig_figs(0.0001234, 3), "0.000123");
        assert_eq!(format_number_sig_figs(0.00001234, 3), "1.23 \\times 10^{-5}");
        assert_eq!(format_number_sig_figs(-0.00001234, 3), "-1.23 \\times 10^{-5}");
        assert_eq!(format_number_sig_figs(1_234_567.0, 4), "1.235 \\times 10^{6}");
        assert_eq!(format_number_sig_figs(9.999, 2), "10");
    }

    #[test]
    fn polynomial_formatting_symbolic_and_substituted_normalizes_signs() {
        assert_eq!(format_polynomial_equation_tex(1, false, &[], false, 4), "Y = \\text{Slope} \\cdot X + \\text{Intercept}");
        assert_eq!(format_polynomial_equation_tex(1, true, &[], false, 4), "Y = \\text{Slope} \\cdot X");
        assert_eq!(
            format_polynomial_equation_tex(1, false, &[("Slope".into(), -2.5), ("YIntercept".into(), -1.25)], true, 4,),
            "Y = -2.5 \\cdot X - 1.25"
        );
        assert_eq!(format_polynomial_equation_tex(2, false, &[], false, 4), "Y = b_0 + b_1 \\cdot X + b_2 \\cdot X^2");
        assert_eq!(
            format_polynomial_equation_tex(3, false, &[("b0".into(), 1.5), ("b1".into(), -2.5), ("b2".into(), -0.25), ("b3".into(), 4.0)], true, 4,),
            "Y = 1.5 - 2.5 \\cdot X - 0.25 \\cdot X^2 + 4 \\cdot X^3"
        );
    }

    #[test]
    fn canonical_landmark_equations_match_publication_tex() {
        let mm = format_model_equation_tex("michaelis_menten", None, &[], false, 4);
        assert_eq!(mm, "Y = \\frac{V_{\\max} \\cdot X}{K_m + X}");

        let mm_sub = format_model_equation_tex("michaelis_menten", None, &[("Vmax".into(), 120.0), ("Km".into(), 4.5)], true, 4);
        assert_eq!(mm_sub, "Y = \\frac{120 \\cdot X}{4.5 + X}");

        let hill = format_model_equation_tex("allosteric_sigmoidal_kinetics", None, &[], false, 4);
        assert_eq!(hill, "Y = \\frac{V_{\\max} \\cdot X^{h}}{K_{\\text{half}}^{h} + X^{h}}");

        let bind = format_model_equation_tex("one_site_specific_binding", None, &[], false, 4);
        assert_eq!(bind, "Y = \\frac{B_{\\max} \\cdot X}{K_d + X}");

        let lq = format_model_equation_tex("linear_quadratic_survival", None, &[], false, 4);
        assert_eq!(lq, "Y = \\exp(-(\\alpha \\cdot D + \\beta \\cdot D^2))");

        let nelson = format_model_equation_tex("nist_nelson", None, &[("b1".into(), 2.59), ("b2".into(), -1.75), ("b3".into(), -0.02)], true, 4);
        assert_eq!(nelson, "Y = 2.59 + 1.75 \\cdot X_1 \\cdot \\exp(0.02 \\cdot X_2)");
    }

    fn has_balanced_braces(s: &str) -> bool {
        let mut depth = 0i32;
        for ch in s.chars() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                }
                _ => {}
            }
        }
        depth == 0
    }

    #[test]
    fn every_builtin_equation_formats_cleanly_in_symbolic_and_substituted_modes() {
        let catalog = list_builtin_equations();
        assert!(catalog.len() >= 117, "expected >= 117 built-ins, got {}", catalog.len());

        let mut missing_params = Vec::new();
        for entry in &catalog {
            let sym = format_model_equation_tex(&entry.id, None, &[], false, 4);
            assert!(!sym.is_empty(), "empty symbolic output for {}", entry.id);
            assert!(sym.contains('='), "missing '=' in symbolic output for {}: {sym}", entry.id);
            assert!(has_balanced_braces(&sym), "unbalanced braces in symbolic {}: {sym}", entry.id);
            assert!(!sym.contains("+ -"), "sign artifact '+ -' in symbolic {}: {sym}", entry.id);
            assert!(!sym.contains("- -"), "sign artifact '- -' in symbolic {}: {sym}", entry.id);
            assert!(!sym.starts_with("Y = f(X;"), "hit unknown fallback for built-in {}: {sym}", entry.id);
            assert!(!sym.starts_with("Y = f_{"), "hit empty unknown fallback for built-in {}: {sym}", entry.id);

            let pos_params: Vec<(String, f64)> = entry.param_names.iter().enumerate().map(|(i, p)| (p.clone(), 11.13 + (i as f64) * 3.17)).collect();
            let pos = format_model_equation_tex(&entry.id, None, &pos_params, true, 4);
            assert!(!pos.is_empty() && pos.contains('='), "bad pos substituted {}: {pos}", entry.id);
            assert!(has_balanced_braces(&pos), "unbalanced braces in pos {}: {pos}", entry.id);
            assert!(!pos.contains("+ -") && !pos.contains("- -"), "sign artifact in pos {}: {pos}", entry.id);
            assert!(!pos.contains("NaN"), "NaN in pos {}: {pos}", entry.id);

            if entry.id != "motulsky_mahan_kinetics" {
                for (pname, pval) in &pos_params {
                    let needle = format_number_sig_figs(*pval, 4);
                    if !pos.contains(&needle) {
                        missing_params.push(format!("{}: param `{pname}` ({needle}) not in `{pos}` (all params: {:?})", entry.id, entry.param_names));
                    }
                }
            }

            let neg_params: Vec<(String, f64)> = entry.param_names.iter().enumerate().map(|(i, p)| (p.clone(), -(1.25 + (i as f64) * 0.5))).collect();
            let neg = format_model_equation_tex(&entry.id, None, &neg_params, true, 4);
            assert!(!neg.is_empty() && neg.contains('='), "bad neg substituted {}: {neg}", entry.id);
            assert!(has_balanced_braces(&neg), "unbalanced braces in neg {}: {neg}", entry.id);
            assert!(!neg.contains("+ -") && !neg.contains("- -") && !neg.contains("--"), "sign artifact in neg {}: {neg}", entry.id);
            assert!(!neg.contains("NaN"), "NaN in neg {}: {neg}", entry.id);
        }
        assert!(missing_params.is_empty(), "missing parameter substitutions:\n{}", missing_params.join("\n"));
    }

    #[test]
    fn custom_dsl_parses_and_substitutes_cleanly() {
        let dsl = "# custom enzyme model\nY = (Vmax * X) / (Km + X)\n[InitialValues]\nVmax = 100\n";
        let sym = format_model_equation_tex("custom_dsl", Some(dsl), &[], false, 4);
        assert_eq!(sym, "Y = \\frac{\\text{Vmax} \\cdot X}{\\text{Km} + X}");

        let sub = format_model_equation_tex(
            "custom_dsl",
            Some("Y = a * exp(-k * X) + b"),
            &[("a".into(), 5.5), ("k".into(), -0.25), ("b".into(), -3.0)],
            true,
            4,
        );
        assert_eq!(sub, "Y = 5.5 \\cdot \\exp(0.25 \\cdot X) - 3");

        let boundary = format_model_equation_tex(
            "custom_dsl",
            Some("<A> Y = sqrt(Km^2 + K * X) + log10(K) - ln(X) ; inline comment"),
            &[("K".into(), 2.0), ("Km".into(), 5.0)],
            true,
            4,
        );
        assert_eq!(boundary, "Y = \\sqrt{5^{2} + 2 \\cdot X} + \\log(2) - \\ln(X)");

        let exp10_eq = format_model_equation_tex("custom_dsl", Some("Y = exp10(a - b * X)"), &[], false, 4);
        assert_eq!(exp10_eq, "Y = 10^{a - b \\cdot X}");

        // Aliases resolve cleanly.
        assert_eq!(
            format_model_equation_tex("4pl", None, &[], false, 4),
            format_model_equation_tex("log_agonist_vs_response_4pl", None, &[], false, 4)
        );
        assert_eq!(format_model_equation_tex("boxbod", None, &[], false, 4), format_model_equation_tex("nist_boxbod", None, &[], false, 4));
        assert_eq!(format_model_equation_tex("quadratic", None, &[], false, 4), "Y = b_0 + b_1 \\cdot X + b_2 \\cdot X^2");

        let fb = format_model_equation_tex("totally_unknown_id", None, &[("k1".into(), 2.5), ("k2".into(), -1.0)], true, 4);
        assert_eq!(fb, "Y = f(X; \\text{k1} = 2.5, \\text{k2} = -1)");

        let fb_empty = format_model_equation_tex("totally_unknown_id", None, &[], false, 4);
        assert_eq!(fb_empty, "Y = f_{\\text{totally\\_unknown\\_id}}(X)");
    }

    #[test]
    fn greek_parameters_fractional_exponents_and_operators_format_for_2d_math() {
        let greek_dsl = "Y = Delta * exp(-theta * X / tau_fast) + alpha_1 * sin(kappa * X + phi) + Omega";
        let sym = format_model_equation_tex("custom_dsl", Some(greek_dsl), &[], false, 4);
        assert_eq!(
            sym,
            "Y = \\Delta \\cdot \\exp(-\\frac{\\theta \\cdot X}{\\tau_{\\text{fast}}}) + \\alpha_{1} \\cdot \\sin(\\kappa \\cdot X + \\phi) + \\Omega"
        );

        let sub = format_model_equation_tex(
            "custom_dsl",
            Some(greek_dsl),
            &[("Delta".into(), 12.5), ("theta".into(), 2.0), ("tau_fast".into(), 4.0), ("Omega".into(), -1.5)],
            true,
            4,
        );
        assert!(sub.contains("12.5 \\cdot \\exp(-\\frac{2 \\cdot X}{4})"), "sub={sub}");
        assert!(sub.contains("- 1.5"), "sub={sub}");

        let frac_pow = format_model_equation_tex("custom_dsl", Some("Y = A * (X / t_e)^(-1 / beta) + erf(gamma * X)"), &[], false, 4);
        assert_eq!(frac_pow, "Y = A \\cdot (\\frac{X}{t_{e}})^{-\\frac{1}{\\beta}} + \\erf(\\gamma \\cdot X)");

        let misra1c = format_model_equation_tex("nist_misra1c", None, &[], false, 4);
        assert_eq!(misra1c, "Y = b_1 \\cdot (1 - (1 + 2 \\cdot b_2 \\cdot X)^{-\\frac{1}{2}})");
        let rat43 = format_model_equation_tex("nist_rat43", None, &[], false, 4);
        assert_eq!(rat43, "Y = \\frac{b_1}{(1 + \\exp(b_2 - b_3 \\cdot X))^{\\frac{1}{b_4}}}");
        let roszman = format_model_equation_tex("nist_roszman1", None, &[], false, 4);
        assert_eq!(roszman, "Y = b_1 - b_2 \\cdot X - \\frac{\\arctan(\\frac{b_3}{X - b_4})}{\\pi}");
    }

    #[test]
    fn rv23_and_rv24_tex_text_escapes_and_mul_unary_signs() {
        assert_eq!(escape_tex_text(r"a<b\c>d*e"), r"a\textless{}b\textbackslash{}c\textgreater{}d\*e");
        assert_eq!(normalize_tex_signs(r"a \cdot -3"), r"a \cdot -3");
        assert_eq!(normalize_tex_signs(r"a \cdot - 3"), r"a \cdot -3");
        assert_eq!(normalize_tex_signs(r"a \times -3"), r"a \times -3");
        assert_eq!(normalize_tex_signs(r"a \cdot +X"), r"a \cdot X");
        assert_eq!(normalize_tex_signs(r"t - 3"), r"t - 3");
        assert_eq!(normalize_tex_signs(r"s - 3"), r"s - 3");
    }

    #[test]
    fn custom_dsl_ode_sci_literals_and_unary_pow_precedence_match_dsl_parser() {
        assert_eq!(format_model_equation_tex("custom_dsl", Some("Y0 = A0\ndY/dt = -k * Y"), &[], false, 4), "\\frac{dY}{dt} = -k \\cdot Y");
        assert_eq!(format_model_equation_tex("custom_dsl", Some("Y' = -k * Y"), &[("k".into(), 0.25)], true, 4), "Y' = -0.25 \\cdot Y");
        assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = 1.25e-5 * X + 0.5"), &[], false, 4), "Y = 1.25 \\times 10^{-5} \\cdot X + 0.5");
        assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = 2^-3^2 + X^-2"), &[], false, 4), "Y = 2^{-3^{2}} + X^{-2}");
    }

    #[test]
    fn custom_dsl_enforces_cumulative_dataset_index_cap() {
        // DA-04-01 regression: cumulative dataset indices across lines are capped at MAX_TOTAL_DATASET_INDICES (50_000).
        let within_cap = "<0:9999>Y=X\n".repeat(5);
        assert_eq!(format_model_equation_tex("custom_dsl", Some(&within_cap), &[], false, 4), "Y = X");

        let over_cap = "<0:9999>Y=X\n".repeat(6);
        assert_eq!(format_model_equation_tex("custom_dsl", Some(&over_cap), &[], false, 4), "Y = f(X)");
    }

    #[test]
    fn custom_dsl_left_nested_powers_parenthesize_outer_base() {
        // DA-04-03 regression: left-nested powers whose inner base starts with `(` must not emit double superscripts.
        assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = ((a + b)^2)^3"), &[], false, 4), "Y = ((a + b)^{2})^{3}");
        assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = ((-a)^b)^c"), &[], false, 4), "Y = ((-a)^{b})^{c}");
        assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = (a^b)^c"), &[], false, 4), "Y = (a^{b})^{c}");
        assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = pow(a + b, 2)^3"), &[], false, 4), "Y = ((a + b)^{2})^{3}");
        assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = exp10(X)^2"), &[], false, 4), "Y = (10^{X})^{2}");
        // Single-level parenthesized base stays cleanly unwrapped (no double parens).
        assert_eq!(format_model_equation_tex("custom_dsl", Some("Y = (a + b)^2"), &[], false, 4), "Y = (a + b)^{2}");
    }

    #[test]
    fn gap_01_01_and_02_fallback_escapes_chained_carets_html_and_unmatched_delims() {
        // Exceeds DSL MAX_PARSE_DEPTH (128), so formatting enters `fallback_custom_dsl_tex`.
        let chained = format!("Y = X{}", "^2".repeat(150));
        let tex_chained = format_model_equation_tex("custom_dsl", Some(&chained), &[], false, 4);
        assert!(tex_chained.contains("\\^"), "excessive fallback carets must be escaped: {tex_chained}");

        let short_chain = format_model_equation_tex("custom_dsl", Some("Y = X^2^3 @ 1"), &[], false, 4);
        assert!(short_chain.contains("\\^"), "chained fallback caret must be escaped: {short_chain}");

        let unmatched = format_model_equation_tex("custom_dsl", Some("Y = ((([|(|(<b>x"), &[], false, 4);
        assert!(unmatched.contains("\\(") && unmatched.contains("\\[") && unmatched.contains("\\text{|}"), "got: {unmatched}");
        assert!(unmatched.contains("\\textless{}") && unmatched.contains("\\textgreater{}"), "got: {unmatched}");
        assert!(!unmatched.contains("<b>"), "got: {unmatched}");
    }
}
