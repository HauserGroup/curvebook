//! Nonlinear regression, dose-response, and curve-fit parameter modal helpers (`R10.3`).
//!
//! Renders the multi-tab curve-fitting parameter interface inside [`build_fit_params_extra_section`]
//! (`Equation`, `Constraints`, `Initial Values`, `Confidence`, `Compare`, `Weights & Range`)
//! plus regression controls for [`AnalysisKind::LinearRegression`] and
//! [`AnalysisKind::PassingBablok`], and handles all [`WidgetAction::Custom`] actions targeting the
//! `"fit_params"` domain.

use std::fmt::Write as _;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ConfidenceLevel, DataSheet, FitMethod, HookSource, LinearRegressionSpec, ModelComparisonSpec, NonlinearFitSpec,
    ParameterRule, ParameterScope, PassingBablokSpec, ProjectDocument, ResultStatus, SheetId, TableArchetype, WeightingScheme,
};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DslEquationKind, EquationCatalogEntry, EquationCategory, ParametricModel as _, extract_datasets_from_sheet,
    format_model_equation_tex, get_builtin_model, list_builtin_equations,
};
use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::grid_view::{format_cell_display, parse_localized_number_input};
use crate::interaction::UiEventOutcome;
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldEditState, TextFieldSpec, ToggleWidgetSpec, emit_button, emit_scrollbars_for_meta, emit_tab,
    emit_text_field, emit_toggle, paint_text_fitted, widget_state,
};
use crate::layout::{
    AccessibilityMeta, FocusScope, LayoutBuilder, LayoutRectPx, NodeSpec, ScrollAxes, ScrollMeta, WidgetAction, WidgetId, WidgetRole,
    WidgetVisualState, ZLayer,
};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::{TextMeasurer as _, UiTextStyle};

use super::a11y::{disclosure_meta, labeled_meta, selectable_meta};

const DOMAIN: &str = "fit_params";
const LINEAR_RANGE_INPUT_PREFIX: &str = "fit.lr.range.input.";
const LINEAR_RANGE_FIELDS: [(&str, &str); 5] = [
    ("x_min", "Ignore points where X <"),
    ("x_max", "Ignore points where X >"),
    ("plot_min", "Curve start X"),
    ("plot_max", "Curve end X"),
    ("segments", "Curve segments"),
];
const ROW_H_PX: i32 = 24;
const ROW_H_TALL_PX: i32 = 28;
const SCROLLBAR_RESERVE_PX: u32 = 16;

const TEMPLATE_EXPLICIT: &str = "Y = Bottom + (Top - Bottom) / (1 + (X / EC50)^HillSlope)";
const TEMPLATE_IMPLICIT: &str = "Y + K * Y^2 = A * X + B";
const TEMPLATE_ODE: &str = "dY/dX = -k * Y\nY(0) = Y0";

/// Transient view state owned by the curve-fit parameter modal shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FitParamsViewState {
    pub model_search: String,
    pub active_fit_tab: String,
    #[serde(default)]
    pub linear_range_selected: bool,
    #[serde(default)]
    pub expanded_categories: Vec<String>,
    #[serde(default)]
    pub tree_touched: bool,
    #[serde(default)]
    pub custom_dsl_enabled: bool,
    #[serde(default)]
    pub dsl_mode: String,
    #[serde(default)]
    pub dsl_status: String,
    #[serde(default)]
    pub selected_param_idx: usize,
    #[serde(default)]
    pub unstable_flag_disabled: bool,
}

impl FitParamsViewState {
    /// Whether ambiguous / unstable fit diagnostics should be surfaced.
    #[must_use]
    pub fn report_unstable_fits(&self) -> bool {
        !self.unstable_flag_disabled
    }

    fn effective_tab(&self) -> FitTab {
        FitTab::from_id(&self.active_fit_tab)
    }

    fn effective_dsl_mode(&self) -> &str {
        match self.dsl_mode.as_str() {
            "implicit" | "ode" => &self.dsl_mode,
            _ => "explicit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FitTab {
    Equation,
    Constraints,
    InitialValues,
    Confidence,
    Compare,
    WeightsRange,
}

impl FitTab {
    const ALL: [Self; 6] = [Self::Equation, Self::Constraints, Self::InitialValues, Self::Confidence, Self::Compare, Self::WeightsRange];

    fn from_id(id: &str) -> Self {
        match id {
            "constraints" => Self::Constraints,
            "initial_values" => Self::InitialValues,
            "confidence" => Self::Confidence,
            "compare" => Self::Compare,
            "weights_range" => Self::WeightsRange,
            _ => Self::Equation,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Equation => "model",
            Self::Constraints => "constraints",
            Self::InitialValues => "initial_values",
            Self::Confidence => "confidence",
            Self::Compare => "compare",
            Self::WeightsRange => "weights_range",
        }
    }

    fn i18n_key(self) -> &'static str {
        match self {
            Self::Equation => "fit_params.tab.equation",
            Self::Constraints => "fit_params.tab.constraints",
            Self::InitialValues => "fit_params.tab.initial_values",
            Self::Confidence => "fit_params.tab.confidence",
            Self::Compare => "fit_params.tab.compare",
            Self::WeightsRange => "fit_params.tab.weights_range",
        }
    }

    fn fallback_label(self) -> &'static str {
        match self {
            Self::Equation => "Model",
            Self::Constraints => "Constraints",
            Self::InitialValues => "Initial Values",
            Self::Confidence => "Confidence",
            Self::Compare => "Compare",
            Self::WeightsRange => "Weights & Range",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CatMeta {
    cat: EquationCategory,
    key: &'static str,
    label: &'static str,
}

const CATEGORIES: [CatMeta; 14] = [
    CatMeta { cat: EquationCategory::DoseResponseStimulation, key: "dose_stim", label: "Dose-Response — Stimulation" },
    CatMeta { cat: EquationCategory::DoseResponseInhibition, key: "dose_inhib", label: "Dose-Response — Inhibition" },
    CatMeta { cat: EquationCategory::ReceptorBinding, key: "receptor", label: "Receptor Binding" },
    CatMeta { cat: EquationCategory::EnzymeKinetics, key: "enzyme", label: "Enzyme Kinetics" },
    CatMeta { cat: EquationCategory::Exponential, key: "exponential", label: "Exponential" },
    CatMeta { cat: EquationCategory::Growth, key: "growth", label: "Classic Growth" },
    CatMeta { cat: EquationCategory::Gaussian, key: "gaussian", label: "Gaussian & Bell" },
    CatMeta { cat: EquationCategory::Peak, key: "peak", label: "Peaks & Spectra" },
    CatMeta { cat: EquationCategory::SineWave, key: "sine", label: "Sine Waves" },
    CatMeta { cat: EquationCategory::Polynomial, key: "polynomial", label: "Polynomial & Lines" },
    CatMeta { cat: EquationCategory::StandardCurve, key: "standard", label: "Standard Curves" },
    CatMeta { cat: EquationCategory::Radiobiology, key: "radiobiology", label: "Radiobiology" },
    CatMeta { cat: EquationCategory::NistBenchmark, key: "nist", label: "NIST Benchmark" },
    CatMeta { cat: EquationCategory::UserDefined, key: "user", label: "Custom / User" },
];

fn category_key(cat: EquationCategory) -> &'static str {
    CATEGORIES.iter().find(|m| m.cat == cat).map_or("user", |m| m.key)
}

fn fit_tr<'a>(locale: &str, key: &str, fallback: &'a str) -> &'a str {
    crate::i18n::try_tr(locale, key).or_else(|| crate::i18n::try_tr("en", key)).unwrap_or(fallback)
}

/// Convert TeX math returned by `curvebook_fit::format_model_equation_tex` into readable
/// single-line proportional math using glyphs supported by the bundled UI fonts.
#[must_use]
pub fn latex_to_display(tex: &str) -> String {
    let mut s = tex.trim().to_owned();
    for macro_name in ["\\mathrm", "\\text", "\\mathit", "\\operatorname", "\\left", "\\right"] {
        s = s.replace(macro_name, "");
    }
    for _ in 0..8 {
        let Some(next) = replace_frac_once(&s) else { break };
        s = next;
    }
    for _ in 0..8 {
        let Some(next) = replace_unary_macro_once(&s, "\\sqrt", "√") else { break };
        s = next;
    }
    let replacements = [
        ("\\cdot", " × "),
        ("\\times", " × "),
        ("\\log_{10}", "log10"),
        ("\\alpha", "α"),
        ("\\beta", "β"),
        ("\\gamma", "γ"),
        ("\\sigma", "σ"),
        ("\\Delta", "Δ"),
        ("\\mu", "μ"),
        ("\\sum", "∑"),
        ("\\le", "≤"),
        ("\\ge", "≥"),
        ("\\pm", "±"),
        ("\\approx", "≈"),
        ("\\neq", "≠"),
        ("\\ln", "ln"),
        ("\\log", "log"),
        ("\\exp", "exp"),
        ("\\sin", "sin"),
        ("\\cos", "cos"),
        ("\\tan", "tan"),
        ("\\,", " "),
        ("\\;", " "),
        ("\\:", " "),
        ("\\!", ""),
        ("$", ""),
        ("\\", ""),
    ];
    for (from, to) in replacements {
        s = s.replace(from, to);
    }
    s = simplify_scripts(&s);
    collapse_spaces(&s)
}

fn find_brace_group(s: &str, open_idx: usize) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    if bytes.get(open_idx).copied() != Some(b'{') {
        return None;
    }
    let mut depth = 0_i32;
    for (offset, b) in bytes[open_idx..].iter().enumerate() {
        match *b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open_idx, open_idx + offset));
                }
            }
            _ => {}
        }
    }
    None
}

fn replace_frac_once(s: &str) -> Option<String> {
    let idx = s.find("\\frac")?;
    let mut cursor = idx + "\\frac".len();
    while s.as_bytes().get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    let (n_open, n_close) = find_brace_group(s, cursor)?;
    let mut d_cursor = n_close + 1;
    while s.as_bytes().get(d_cursor).is_some_and(u8::is_ascii_whitespace) {
        d_cursor += 1;
    }
    let (d_open, d_close) = find_brace_group(s, d_cursor)?;
    let num = s[n_open + 1..n_close].trim();
    let den = s[d_open + 1..d_close].trim();
    let n_str = if is_simple_math_atom(num) { num.to_owned() } else { format!("({num})") };
    let d_str = if is_simple_math_atom(den) { den.to_owned() } else { format!("({den})") };
    let mut out = String::with_capacity(s.len());
    out.push_str(&s[..idx]);
    let _ = write!(out, "{n_str} / {d_str}");
    out.push_str(&s[d_close + 1..]);
    Some(out)
}

fn replace_unary_macro_once(s: &str, name: &str, prefix: &str) -> Option<String> {
    let idx = s.find(name)?;
    let mut cursor = idx + name.len();
    while s.as_bytes().get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    let (open, close) = find_brace_group(s, cursor)?;
    let inner = s[open + 1..close].trim();
    let mut out = String::with_capacity(s.len());
    out.push_str(&s[..idx]);
    let _ = write!(out, "{prefix}({inner})");
    out.push_str(&s[close + 1..]);
    Some(out)
}

fn is_simple_math_atom(s: &str) -> bool {
    !s.is_empty() && !s.contains('+') && !s.contains('-') && !s.contains('/') && !s.contains(' ')
}

fn simplify_scripts(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if (ch == '_' || ch == '^')
            && chars.get(i + 1) == Some(&'{')
            && let Some((end_brace, inner)) = collect_brace_chars(&chars, i + 1)
        {
            if ch == '_' {
                out.push_str(&inner);
            } else if inner.chars().all(|c| c.is_ascii_alphanumeric()) {
                out.push('^');
                out.push_str(&inner);
            } else {
                out.push_str("^(");
                out.push_str(&inner);
                out.push(')');
            }
            i = end_brace + 1;
            continue;
        }
        if ch != '{' && ch != '}' {
            out.push(ch);
        }
        i += 1;
    }
    out
}

fn collect_brace_chars(chars: &[char], open_idx: usize) -> Option<(usize, String)> {
    let mut depth = 0_i32;
    let mut inner = String::new();
    for (idx, &ch) in chars.iter().enumerate().skip(open_idx) {
        match ch {
            '{' => {
                if depth > 0 {
                    inner.push(ch);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((idx, inner));
                }
                inner.push(ch);
            }
            _ => inner.push(ch),
        }
    }
    None
}

fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_owned()
}

fn resolve_data_sheet<'a>(doc: &'a ProjectDocument, source_id: &SheetId) -> Option<&'a DataSheet> {
    doc.data_sheet(source_id)
        .or_else(|| doc.result_sheet(source_id).and_then(|rs| rs.analyzable_data.as_ref()))
        .or_else(|| doc.result_sheet(source_id).and_then(|rs| doc.data_sheet(&rs.analysis.source_sheet_id)))
}

fn active_sheet_points(doc: &ProjectDocument, source_sheet_id: &SheetId) -> Vec<DataPoint> {
    let Some(sheet) = resolve_data_sheet(doc, source_sheet_id) else {
        return Vec::new();
    };
    let Ok(datasets) = extract_datasets_from_sheet(sheet, &[]) else {
        return Vec::new();
    };
    datasets.into_iter().find(|ds| !ds.points.is_empty()).map_or_else(Vec::new, |ds| ds.points)
}

fn draft_points(app: &UiApp) -> Vec<DataPoint> {
    let Some(draft) = app.canvas_dialogs.edit_params.draft.as_ref() else {
        return Vec::new();
    };
    active_sheet_points(app.document(), &draft.source_sheet_id)
}

fn model_param_names(nl: &NonlinearFitSpec) -> Vec<String> {
    if let Some(dsl) = nl.custom_dsl.as_deref().map(str::trim).filter(|s| !s.is_empty())
        && let Ok(compiled) = CompiledDslModel::compile("custom", dsl)
    {
        return compiled.param_names().to_vec();
    }
    if let Some(model) = get_builtin_model(&nl.equation_id) {
        return model.param_names().iter().map(ToString::to_string).collect();
    }
    nl.parameter_rules.iter().map(|r| r.name.clone()).collect()
}

fn model_initial_guesses(nl: &NonlinearFitSpec, points: &[DataPoint]) -> Vec<f64> {
    if let Some(dsl) = nl.custom_dsl.as_deref().map(str::trim).filter(|s| !s.is_empty())
        && let Ok(compiled) = CompiledDslModel::compile("custom", dsl)
    {
        return compiled.initial_guess(points);
    }
    get_builtin_model(&nl.equation_id).map_or_else(Vec::new, |model| model.initial_guess(points))
}

fn model_default_bounds(nl: &NonlinearFitSpec) -> Vec<(Option<f64>, Option<f64>)> {
    get_builtin_model(&nl.equation_id).map_or_else(Vec::new, |model| model.default_bounds())
}

fn model_default_rules(nl: &NonlinearFitSpec, points: &[DataPoint]) -> Vec<ParameterRule> {
    get_builtin_model(&nl.equation_id).map_or_else(Vec::new, |model| model.default_parameter_rules(points))
}

fn find_rule<'a>(nl: &'a NonlinearFitSpec, name: &str) -> Option<&'a ParameterRule> {
    nl.parameter_rules.iter().find(|r| r.name == name)
}

fn ensure_rule_mut<'a>(nl: &'a mut NonlinearFitSpec, name: &str) -> &'a mut ParameterRule {
    if let Some(pos) = nl.parameter_rules.iter().position(|r| r.name == name) {
        return &mut nl.parameter_rules[pos];
    }
    nl.parameter_rules.push(ParameterRule {
        name: name.to_owned(),
        scope: ParameterScope::Local,
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    let last = nl.parameter_rules.len().saturating_sub(1);
    &mut nl.parameter_rules[last]
}

fn dsl_starter_for_mode(mode: &str) -> &'static str {
    match mode {
        "implicit" => TEMPLATE_IMPLICIT,
        "ode" => TEMPLATE_ODE,
        _ => TEMPLATE_EXPLICIT,
    }
}

fn dsl_kind_label(kind: DslEquationKind) -> &'static str {
    match kind {
        DslEquationKind::Explicit => "Explicit",
        DslEquationKind::Implicit => "Implicit",
        DslEquationKind::OdeFirstOrder => "ODE (1st-order)",
    }
}

fn robust_or_rout(method: FitMethod) -> bool {
    matches!(method, FitMethod::RobustCauchy | FitMethod::RoutOutlierElimination { .. })
}

fn profile_ci_enablement(nl: &NonlinearFitSpec, locale: &str) -> ControlEnablement {
    if robust_or_rout(nl.method) {
        ControlEnablement::disabled(fit_tr(locale, "fit_params.dis.robust_profile_ci", "Profile-likelihood CIs are disabled in Robust or ROUT mode."))
    } else {
        ControlEnablement::enabled()
    }
}

fn covariance_enablement(nl: &NonlinearFitSpec) -> ControlEnablement {
    if matches!(nl.method, FitMethod::RobustCauchy) {
        ControlEnablement::disabled("Covariance matrix output is disabled in Robust Cauchy mode.")
    } else {
        ControlEnablement::enabled()
    }
}

fn compare_enablement(nl: &NonlinearFitSpec, locale: &str) -> ControlEnablement {
    if robust_or_rout(nl.method) {
        ControlEnablement::disabled(fit_tr(
            locale,
            "fit_params.dis.robust_compare",
            "Extra-SS F and AICc model comparisons are unavailable in Robust mode.",
        ))
    } else {
        ControlEnablement::enabled()
    }
}

fn is_fixed_scope(scope: &ParameterScope) -> bool {
    matches!(
        scope,
        ParameterScope::Constant(_) | ParameterScope::ColumnTitleConstant | ParameterScope::HookedConstant(_) | ParameterScope::PerDatasetConstant(_)
    )
}

fn bounds_enablement(rule: Option<&ParameterRule>) -> ControlEnablement {
    if let Some(r) = rule {
        if matches!(r.scope, ParameterScope::HookedConstant(_)) {
            return ControlEnablement::disabled("Hooked constant parameters are linked externally; switch scope to Local to set bounds.");
        }
        if is_fixed_scope(&r.scope) {
            return ControlEnablement::disabled("Bounds and initial overrides are disabled while this parameter is fixed as a constant.");
        }
    }
    ControlEnablement::enabled()
}

fn format_compact_num(v: f64) -> String {
    if !v.is_finite() {
        return String::from("NaN");
    }
    let abs = v.abs();
    if abs > 0.0 && !(1e-3..1e5).contains(&abs) {
        format!("{v:.3e}")
    } else if (v - v.round()).abs() < 1e-6 && abs < 1e7 {
        format!("{v:.0}")
    } else {
        let mut s = format!("{v:.4}");
        while s.contains('.') && s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
        s
    }
}

fn scope_label(scope: &ParameterScope) -> String {
    match scope {
        ParameterScope::Local => String::from("Local"),
        ParameterScope::SharedGlobal => String::from("Shared Global"),
        ParameterScope::Constant(v) => format!("Constant ({})", format_compact_num(*v)),
        ParameterScope::HookedConstant(HookSource::InfoConstant { info_sheet_id, constant_name }) => {
            let sid = info_sheet_id.as_ref().map_or("auto", SheetId::as_str);
            format!("Hooked ({sid}:{constant_name})")
        }
        ParameterScope::HookedConstant(HookSource::ResultScalar { result_sheet_id, parameter_key, .. }) => {
            format!("Hooked ({}:{parameter_key})", result_sheet_id.as_str())
        }
        ParameterScope::HookedConstant(HookSource::ColumnTitleValue { data_sheet_id, data_set_index }) => {
            format!("Hooked ({}:col {data_set_index})", data_sheet_id.as_str())
        }
        ParameterScope::PerDatasetConstant(pairs) => format!("Per-Dataset ({})", pairs.len()),
        ParameterScope::ColumnTitleConstant => String::from("Column Title"),
    }
}

fn set_percent_clamped(level: &mut ConfidenceLevel, pct: f64) {
    if let Ok(next) = ConfidenceLevel::from_percent(pct.clamp(0.01, 99.99)) {
        *level = next;
    }
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `fit_params` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut FitParamsViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if let Some(tab_id) = payload.strip_prefix("tab:") {
        state.active_fit_tab = FitTab::from_id(tab_id).id().to_owned();
        return Ok(handled_outcome(payload));
    }

    if matches!(payload, "lr_show_model" | "lr_show_range")
        && matches!(app.canvas_dialogs.edit_params.draft.as_ref().map(|spec| &spec.kind), Some(AnalysisKind::LinearRegression(_)))
    {
        state.linear_range_selected = payload == "lr_show_range";
        return Ok(handled_outcome(payload));
    }

    let Some(mut spec) = app.canvas_dialogs.edit_params.draft.clone() else {
        return Ok(UiEventOutcome::default());
    };

    let handled = match &mut spec.kind {
        AnalysisKind::NonlinearRegression(nl) => handle_nonlinear_action(app, state, nl, &mut spec.confidence_level, payload),
        AnalysisKind::LinearRegression(lr) => handle_linear_action(lr, &mut spec.confidence_level, payload),
        AnalysisKind::PassingBablok(pb) => handle_passing_bablok_action(pb, &mut spec.confidence_level, payload),
        _ => return Ok(UiEventOutcome::default()),
    };

    if !handled {
        return Ok(UiEventOutcome::default());
    }

    let updated_kind = spec.kind.clone();
    if let Some(draft) = app.canvas_dialogs.edit_params.draft.as_mut() {
        draft.confidence_level = spec.confidence_level;
        draft.update_kind(updated_kind);
    }
    Ok(handled_outcome(payload))
}

fn handled_outcome(payload: &str) -> UiEventOutcome {
    UiEventOutcome { handled: true, action: format!("{DOMAIN}:{payload}"), command_id: None, command_result: None }
}

fn handle_nonlinear_action(
    app: &mut UiApp,
    state: &mut FitParamsViewState,
    nl: &mut NonlinearFitSpec,
    confidence_level: &mut ConfidenceLevel,
    payload: &str,
) -> bool {
    handle_equation_action(app, state, nl, payload)
        || handle_constraints_action(app, state, nl, payload)
        || handle_initial_values_action(app, state, nl, payload)
        || handle_confidence_action(app, state, nl, confidence_level, payload)
        || handle_compare_action(app, nl, payload)
        || handle_weights_range_action(app, nl, payload)
}

fn ensure_tree_expanded_baseline(state: &mut FitParamsViewState, active_eq_id: &str) {
    if !state.tree_touched {
        state.tree_touched = true;
        if state.expanded_categories.is_empty()
            && let Some(entry) = list_builtin_equations().into_iter().find(|e| e.id == active_eq_id)
        {
            state.expanded_categories.push(category_key(entry.category).to_owned());
        }
    }
}

fn clear_param_text_buffers(app: &mut UiApp) {
    for key in ["fit.con.lo.input", "fit.con.hi.input", "fit.con.const.input", "fit.init.val.input"] {
        app.interaction.text_field_buffers.remove(key);
    }
}

fn handle_equation_action(app: &mut UiApp, state: &mut FitParamsViewState, nl: &mut NonlinearFitSpec, payload: &str) -> bool {
    if payload == "eq_clear_search" {
        state.model_search.clear();
        app.interaction.text_field_buffers.remove("fit.eq.search");
        return true;
    }
    if payload == "eq_expand_all" {
        state.tree_touched = true;
        state.expanded_categories = CATEGORIES.iter().map(|c| c.key.to_owned()).collect();
        return true;
    }
    if payload == "eq_collapse_all" {
        state.tree_touched = true;
        state.expanded_categories.clear();
        return true;
    }
    if let Some(cat_key) = payload.strip_prefix("toggle_cat:") {
        ensure_tree_expanded_baseline(state, &nl.equation_id);
        if let Some(pos) = state.expanded_categories.iter().position(|k| k == cat_key) {
            state.expanded_categories.remove(pos);
        } else {
            state.expanded_categories.push(cat_key.to_owned());
        }
        return true;
    }
    if let Some(col_raw) = payload.strip_prefix("mv_x:")
        && let Ok(col) = col_raw.parse::<usize>()
    {
        nl.mv_x_columns = vec![col];
        return true;
    }
    if let Some(col_raw) = payload.strip_prefix("mv_y:")
        && let Ok(col) = col_raw.parse::<usize>()
    {
        nl.mv_y_column = Some(col);
        return true;
    }
    if let Some(col_raw) = payload.strip_prefix("mv_group:")
        && let Ok(col) = col_raw.parse::<usize>()
    {
        if let Some(pos) = nl.mv_group_columns.iter().position(|&c| c == col) {
            nl.mv_group_columns.remove(pos);
        } else {
            nl.mv_group_columns.push(col);
            nl.mv_group_columns.sort_unstable();
        }
        return true;
    }
    if let Some(eq_id) = payload.strip_prefix("select_eq:") {
        select_builtin_equation(app, state, nl, eq_id);
        return true;
    }
    handle_dsl_action(app, state, nl, payload)
}

fn select_builtin_equation(app: &mut UiApp, state: &mut FitParamsViewState, nl: &mut NonlinearFitSpec, eq_id: &str) {
    let Some(model) = get_builtin_model(eq_id) else {
        return;
    };
    let valid_params: Vec<String> = model.param_names().iter().map(ToString::to_string).collect();
    nl.equation_id = eq_id.to_owned();
    nl.custom_dsl = None;
    nl.parameter_rules.retain(|r| valid_params.iter().any(|p| p == &r.name));
    state.custom_dsl_enabled = false;
    state.dsl_status.clear();
    state.selected_param_idx = 0;
    clear_param_text_buffers(app);
    ensure_tree_expanded_baseline(state, eq_id);
    if let Some(entry) = list_builtin_equations().into_iter().find(|e| e.id == eq_id) {
        let k = category_key(entry.category).to_owned();
        if !state.expanded_categories.contains(&k) {
            state.expanded_categories.push(k);
        }
    }
}

fn handle_dsl_action(app: &mut UiApp, state: &mut FitParamsViewState, nl: &mut NonlinearFitSpec, payload: &str) -> bool {
    if payload == "toggle_custom_dsl" {
        let next = !(state.custom_dsl_enabled || nl.custom_dsl.is_some());
        state.custom_dsl_enabled = next;
        if next {
            let starter = app
                .interaction
                .text_field_buffers
                .get("fit.dsl.input")
                .map(|b| b.text.clone())
                .filter(|s| !s.trim().is_empty())
                .or_else(|| nl.custom_dsl.clone())
                .unwrap_or_else(|| dsl_starter_for_mode(state.effective_dsl_mode()).to_owned());
            apply_dsl_source(app, state, nl, &starter);
        } else {
            nl.custom_dsl = None;
            state.dsl_status = String::from("Using selected built-in equation.");
        }
        return true;
    }
    if let Some(mode) = payload.strip_prefix("dsl_mode:") {
        state.dsl_mode = mode.to_owned();
        state.custom_dsl_enabled = true;
        let starter = dsl_starter_for_mode(mode).to_owned();
        app.interaction.text_field_buffers.insert(String::from("fit.dsl.input"), TextFieldEditState::new(starter.clone(), true));
        apply_dsl_source(app, state, nl, &starter);
        return true;
    }
    if payload == "apply_dsl" {
        state.custom_dsl_enabled = true;
        let src = app
            .interaction
            .text_field_buffers
            .get("fit.dsl.input")
            .map(|b| b.text.clone())
            .or_else(|| nl.custom_dsl.clone())
            .unwrap_or_else(|| dsl_starter_for_mode(state.effective_dsl_mode()).to_owned());
        apply_dsl_source(app, state, nl, &src);
        return true;
    }
    if payload == "clear_dsl" {
        state.custom_dsl_enabled = false;
        nl.custom_dsl = None;
        state.dsl_status.clear();
        return true;
    }
    false
}

fn apply_dsl_source(app: &mut UiApp, state: &mut FitParamsViewState, nl: &mut NonlinearFitSpec, src: &str) {
    let trimmed = src.trim();
    if trimmed.is_empty() {
        state.dsl_status = String::from("DSL equation cannot be empty.");
        return;
    }
    app.interaction.text_field_buffers.insert(String::from("fit.dsl.input"), TextFieldEditState::new(src.to_owned(), true));
    match CompiledDslModel::compile("custom_dsl", trimmed) {
        Ok(compiled) => {
            let valid = compiled.param_names().to_vec();
            nl.custom_dsl = Some(trimmed.to_owned());
            nl.parameter_rules.retain(|r| valid.iter().any(|p| p == &r.name));
            state.selected_param_idx = 0;
            clear_param_text_buffers(app);
            state.dsl_status = format!("Compiled OK: {} ({} params)", dsl_kind_label(compiled.kind()), valid.len());
        }
        Err(err) => {
            state.dsl_status = format!("Syntax error: {err}");
        }
    }
}

fn handle_constraints_action(app: &mut UiApp, state: &mut FitParamsViewState, nl: &mut NonlinearFitSpec, payload: &str) -> bool {
    let params = model_param_names(nl);
    if handle_constraints_row_action(app, state, nl, &params, payload) {
        return true;
    }
    let Some(selected_name) = params.get(state.selected_param_idx.min(params.len().saturating_sub(1))).cloned() else {
        return false;
    };
    handle_constraints_selected_action(app, state, nl, &selected_name, payload)
}

fn handle_constraints_row_action(
    app: &mut UiApp,
    state: &mut FitParamsViewState,
    nl: &mut NonlinearFitSpec,
    params: &[String],
    payload: &str,
) -> bool {
    if payload == "share_all" {
        for name in params {
            ensure_rule_mut(nl, name).scope = ParameterScope::SharedGlobal;
        }
        return true;
    }
    if payload == "unshare_all" {
        for name in params {
            ensure_rule_mut(nl, name).scope = ParameterScope::Local;
        }
        return true;
    }
    if let Some(idx_str) = payload.strip_prefix("select_param:") {
        if let Ok(idx) = idx_str.parse::<usize>()
            && idx < params.len()
        {
            state.selected_param_idx = idx;
            clear_param_text_buffers(app);
        }
        return true;
    }
    if let Some(idx_str) = payload.strip_prefix("toggle_shared:") {
        if let Ok(idx) = idx_str.parse::<usize>()
            && let Some(name) = params.get(idx)
        {
            let rule = ensure_rule_mut(nl, name);
            rule.scope = if matches!(rule.scope, ParameterScope::SharedGlobal) { ParameterScope::Local } else { ParameterScope::SharedGlobal };
            state.selected_param_idx = idx;
        }
        return true;
    }
    if let Some(idx_str) = payload.strip_prefix("cycle_scope:") {
        if let Ok(idx) = idx_str.parse::<usize>()
            && let Some(name) = params.get(idx)
        {
            let points = draft_points(app);
            let default_c = model_initial_guesses(nl, &points).get(idx).copied().filter(|v| v.is_finite()).unwrap_or(1.0);
            let rule = ensure_rule_mut(nl, name);
            rule.scope = match &rule.scope {
                ParameterScope::Local => ParameterScope::SharedGlobal,
                ParameterScope::SharedGlobal => ParameterScope::Constant(rule.initial_override.unwrap_or(default_c)),
                ParameterScope::Constant(_) => ParameterScope::ColumnTitleConstant,
                ParameterScope::ColumnTitleConstant | ParameterScope::HookedConstant(_) | ParameterScope::PerDatasetConstant(_) => {
                    ParameterScope::Local
                }
            };
            state.selected_param_idx = idx;
            clear_param_text_buffers(app);
        }
        return true;
    }
    false
}

fn handle_constraints_selected_action(
    app: &mut UiApp,
    state: &FitParamsViewState,
    nl: &mut NonlinearFitSpec,
    selected_name: &str,
    payload: &str,
) -> bool {
    if let Some(scope_tag) = payload.strip_prefix("set_scope:") {
        let points = draft_points(app);
        let default_c = model_initial_guesses(nl, &points).get(state.selected_param_idx).copied().filter(|v| v.is_finite()).unwrap_or(1.0);
        let rule = ensure_rule_mut(nl, selected_name);
        rule.scope = match scope_tag {
            "shared" => ParameterScope::SharedGlobal,
            "constant" => ParameterScope::Constant(rule.initial_override.unwrap_or(default_c)),
            "col_title" => ParameterScope::ColumnTitleConstant,
            _ => ParameterScope::Local,
        };
        clear_param_text_buffers(app);
        return true;
    }
    if payload == "apply_param_rule" {
        apply_selected_param_rule_inputs(app, nl, selected_name);
        return true;
    }
    if payload == "clear_param_bounds" {
        let rule = ensure_rule_mut(nl, selected_name);
        if !is_fixed_scope(&rule.scope) {
            rule.lower_bound = None;
            rule.upper_bound = None;
            app.interaction.text_field_buffers.remove("fit.con.lo.input");
            app.interaction.text_field_buffers.remove("fit.con.hi.input");
        }
        return true;
    }
    if let Some(preset) = payload.strip_prefix("bound_preset:") {
        apply_bound_preset(app, nl, selected_name, preset);
        return true;
    }
    if let Some(delta_spec) = payload.strip_prefix("step_bound:") {
        return handle_bound_stepper(app, nl, selected_name, delta_spec);
    }
    false
}

fn apply_selected_param_rule_inputs(app: &UiApp, nl: &mut NonlinearFitSpec, selected_name: &str) {
    let rule = ensure_rule_mut(nl, selected_name);
    if matches!(rule.scope, ParameterScope::Constant(_)) {
        if let Some(buf) = app.interaction.text_field_buffers.get("fit.con.const.input")
            && let Ok(val) = buf.text.trim().parse::<f64>()
            && val.is_finite()
        {
            rule.scope = ParameterScope::Constant(val);
        }
        return;
    }
    if is_fixed_scope(&rule.scope) {
        return;
    }
    if let Some(buf) = app.interaction.text_field_buffers.get("fit.con.lo.input") {
        let txt = buf.text.trim();
        if txt.is_empty() || txt.eq_ignore_ascii_case("-inf") || txt.eq_ignore_ascii_case("none") {
            rule.lower_bound = None;
        } else if let Ok(v) = txt.parse::<f64>()
            && v.is_finite()
        {
            rule.lower_bound = Some(v);
        }
    }
    if let Some(buf) = app.interaction.text_field_buffers.get("fit.con.hi.input") {
        let txt = buf.text.trim();
        if txt.is_empty() || txt.eq_ignore_ascii_case("+inf") || txt.eq_ignore_ascii_case("inf") || txt.eq_ignore_ascii_case("none") {
            rule.upper_bound = None;
        } else if let Ok(v) = txt.parse::<f64>()
            && v.is_finite()
        {
            rule.upper_bound = Some(v);
        }
    }
    if let (Some(lo), Some(hi)) = (rule.lower_bound, rule.upper_bound)
        && lo > hi
    {
        rule.upper_bound = Some(lo);
    }
}

fn apply_bound_preset(app: &mut UiApp, nl: &mut NonlinearFitSpec, selected_name: &str, preset: &str) {
    let rule = ensure_rule_mut(nl, selected_name);
    if is_fixed_scope(&rule.scope) {
        return;
    }
    match preset {
        "ge0" => rule.lower_bound = Some(0.0),
        "gt0" => rule.lower_bound = Some(1e-9),
        "unit" => {
            rule.lower_bound = Some(0.0);
            rule.upper_bound = Some(100.0);
        }
        _ => {
            rule.lower_bound = None;
            rule.upper_bound = None;
        }
    }
    app.interaction.text_field_buffers.remove("fit.con.lo.input");
    app.interaction.text_field_buffers.remove("fit.con.hi.input");
}

fn handle_bound_stepper(app: &mut UiApp, nl: &mut NonlinearFitSpec, selected_name: &str, delta_spec: &str) -> bool {
    let Some((which, sign_str)) = delta_spec.split_once(':') else {
        return false;
    };
    let Ok(delta) = sign_str.parse::<f64>() else {
        return false;
    };
    let rule = ensure_rule_mut(nl, selected_name);
    match which {
        "const" => {
            if let ParameterScope::Constant(v) = &mut rule.scope {
                let next = (*v + delta).clamp(-1e12, 1e12);
                *v = next;
                app.interaction
                    .text_field_buffers
                    .insert(String::from("fit.con.const.input"), TextFieldEditState::new(format_compact_num(next), false));
            }
        }
        "lo" if !is_fixed_scope(&rule.scope) => {
            let next = rule.lower_bound.unwrap_or(0.0) + delta;
            let clamped = rule.upper_bound.map_or(next, |hi| next.min(hi));
            rule.lower_bound = Some(clamped);
            app.interaction.text_field_buffers.insert(String::from("fit.con.lo.input"), TextFieldEditState::new(format_compact_num(clamped), false));
        }
        "hi" if !is_fixed_scope(&rule.scope) => {
            let next = rule.upper_bound.unwrap_or(100.0) + delta;
            let clamped = rule.lower_bound.map_or(next, |lo| next.max(lo));
            rule.upper_bound = Some(clamped);
            app.interaction.text_field_buffers.insert(String::from("fit.con.hi.input"), TextFieldEditState::new(format_compact_num(clamped), false));
        }
        _ => return false,
    }
    true
}

fn handle_initial_values_action(app: &mut UiApp, state: &mut FitParamsViewState, nl: &mut NonlinearFitSpec, payload: &str) -> bool {
    let params = model_param_names(nl);
    if payload == "compute_auto_initials" {
        let points = draft_points(app);
        let guesses = model_initial_guesses(nl, &points);
        let default_rules = model_default_rules(nl, &points);
        for (idx, name) in params.iter().enumerate() {
            let rule = ensure_rule_mut(nl, name);
            if is_fixed_scope(&rule.scope) {
                continue;
            }
            if let Some(g) = guesses.get(idx).copied().filter(|v| v.is_finite()) {
                rule.initial_override = Some(g);
            } else if let Some(dr) = default_rules.iter().find(|r| r.name == *name) {
                rule.initial_override = dr.initial_override;
            }
        }
        clear_param_text_buffers(app);
        return true;
    }
    if payload == "clear_initial_overrides" {
        for name in &params {
            ensure_rule_mut(nl, name).initial_override = None;
        }
        app.interaction.text_field_buffers.remove("fit.init.val.input");
        return true;
    }
    if let Some(idx_str) = payload.strip_prefix("init_clear_one:") {
        if let Ok(idx) = idx_str.parse::<usize>()
            && let Some(name) = params.get(idx)
        {
            ensure_rule_mut(nl, name).initial_override = None;
            state.selected_param_idx = idx;
            app.interaction.text_field_buffers.remove("fit.init.val.input");
        }
        return true;
    }
    if let Some(rest) = payload.strip_prefix("step_init:") {
        return step_initial_value(app, state, nl, &params, rest);
    }
    if payload == "apply_init_input" {
        let Some(name) = params.get(state.selected_param_idx.min(params.len().saturating_sub(1))).cloned() else {
            return false;
        };
        let rule = ensure_rule_mut(nl, &name);
        if !is_fixed_scope(&rule.scope)
            && let Some(buf) = app.interaction.text_field_buffers.get("fit.init.val.input")
        {
            let txt = buf.text.trim();
            if txt.is_empty() || txt.eq_ignore_ascii_case("auto") {
                rule.initial_override = None;
            } else if let Ok(val) = txt.parse::<f64>()
                && val.is_finite()
            {
                rule.initial_override = Some(val);
            }
        }
        return true;
    }
    false
}

fn step_initial_value(app: &mut UiApp, state: &mut FitParamsViewState, nl: &mut NonlinearFitSpec, params: &[String], rest: &str) -> bool {
    let Some((idx_str, sign_str)) = rest.split_once(':') else {
        return false;
    };
    let (Ok(idx), Ok(step)) = (idx_str.parse::<usize>(), sign_str.parse::<f64>()) else {
        return false;
    };
    let Some(name) = params.get(idx) else {
        return false;
    };
    let points = draft_points(app);
    let base = find_rule(nl, name).and_then(|r| r.initial_override).or_else(|| model_initial_guesses(nl, &points).get(idx).copied()).unwrap_or(1.0);
    let rule = ensure_rule_mut(nl, name);
    if !is_fixed_scope(&rule.scope) {
        let delta = if base.abs() >= 1.0 { base.abs() * 0.25 * step } else { 0.5 * step };
        let next = base + delta;
        rule.initial_override = Some(next);
        state.selected_param_idx = idx;
        app.interaction.text_field_buffers.insert(String::from("fit.init.val.input"), TextFieldEditState::new(format_compact_num(next), false));
    }
    true
}

fn handle_confidence_action(
    app: &mut UiApp,
    state: &mut FitParamsViewState,
    nl: &mut NonlinearFitSpec,
    confidence_level: &mut ConfidenceLevel,
    payload: &str,
) -> bool {
    if let Some(mode) = payload.strip_prefix("ci_method:") {
        if mode == "profile" && robust_or_rout(nl.method) {
            return false;
        }
        nl.profile_likelihood_ci = mode == "profile";
        return true;
    }
    if let Some(lvl_str) = payload.strip_prefix("ci_level:") {
        if let Ok(pct) = lvl_str.parse::<f64>()
            && pct.is_finite()
        {
            set_percent_clamped(confidence_level, pct);
            app.interaction
                .text_field_buffers
                .insert(String::from("fit.ci.lvl.input"), TextFieldEditState::new(format_compact_num(confidence_level.percent()), false));
        }
        return true;
    }
    if let Some(delta_str) = payload.strip_prefix("step_ci_level:") {
        if let Ok(delta_pct) = delta_str.parse::<f64>() {
            let next_pct = (confidence_level.percent() + delta_pct).clamp(50.0, 99.9);
            set_percent_clamped(confidence_level, next_pct);
            app.interaction
                .text_field_buffers
                .insert(String::from("fit.ci.lvl.input"), TextFieldEditState::new(format_compact_num(confidence_level.percent()), false));
        }
        return true;
    }
    if payload == "apply_ci_level" {
        if let Some(buf) = app.interaction.text_field_buffers.get("fit.ci.lvl.input")
            && let Ok(pct) = buf.text.trim().parse::<f64>()
            && pct.is_finite()
        {
            set_percent_clamped(confidence_level, pct);
        }
        return true;
    }
    match payload {
        "toggle_conf_bands" => {
            nl.confidence_bands = !nl.confidence_bands;
            return true;
        }
        "toggle_pred_bands" => {
            nl.prediction_bands = !nl.prediction_bands;
            return true;
        }
        "toggle_emit_cov" => {
            if matches!(nl.method, FitMethod::RobustCauchy) {
                return false;
            }
            nl.emit_covariance_tab = !nl.emit_covariance_tab;
            return true;
        }
        "toggle_flag_unstable" => {
            state.unstable_flag_disabled = !state.unstable_flag_disabled;
            return true;
        }
        _ => {}
    }
    if let Some(iters_str) = payload.strip_prefix("set_max_iters:") {
        if let Ok(iters) = iters_str.parse::<usize>() {
            nl.max_iters = iters.clamp(10, 10_000);
        }
        return true;
    }
    if let Some(delta_str) = payload.strip_prefix("step_max_iters:") {
        if let Ok(delta) = delta_str.parse::<i32>() {
            let next = (nl.max_iters as i32).saturating_add(delta).clamp(10, 10_000);
            nl.max_iters = next as usize;
        }
        return true;
    }
    false
}

fn default_alternative_equation(primary: &str) -> String {
    match primary {
        "log_agonist_vs_response_4pl" => String::from("log_agonist_vs_response_3pl"),
        "log_inhibitor_vs_response_4pl" => String::from("log_inhibitor_vs_response_3pl"),
        "two_phase_decay" => String::from("one_phase_decay"),
        "straight_line" => String::from("horizontal_line"),
        _ => String::from("straight_line"),
    }
}

fn current_alt_equation_id(nl: &NonlinearFitSpec) -> String {
    match &nl.comparison {
        Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id })
        | Some(ModelComparisonSpec::Aicc { alternative_equation_id })
        | Some(ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id }) => alternative_equation_id.clone(),
        None => default_alternative_equation(&nl.equation_id),
    }
}

fn handle_compare_action(app: &mut UiApp, nl: &mut NonlinearFitSpec, payload: &str) -> bool {
    if payload == "cmp_clear_search" {
        app.interaction.text_field_buffers.remove("fit.cmp.alt.search");
        return true;
    }
    if let Some(mode) = payload.strip_prefix("cmp_mode:") {
        if mode != "none" && robust_or_rout(nl.method) {
            return false;
        }
        let alt = current_alt_equation_id(nl);
        nl.comparison = match mode {
            "f" => Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: alt }),
            "aicc" => Some(ModelComparisonSpec::Aicc { alternative_equation_id: alt }),
            "lrt" => Some(ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: alt }),
            _ => None,
        };
        return true;
    }
    if let Some(alt_id) = payload.strip_prefix("cmp_alt:") {
        if robust_or_rout(nl.method) {
            return false;
        }
        let next_alt = alt_id.to_owned();
        nl.comparison = Some(match &nl.comparison {
            Some(ModelComparisonSpec::ExtraSumOfSquaresF { .. }) => ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: next_alt },
            Some(ModelComparisonSpec::LikelihoodRatioTest { .. }) => ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: next_alt },
            _ => ModelComparisonSpec::Aicc { alternative_equation_id: next_alt },
        });
        return true;
    }
    false
}

fn handle_weights_range_action(app: &mut UiApp, nl: &mut NonlinearFitSpec, payload: &str) -> bool {
    if handle_method_and_weight_action(nl, payload) {
        return true;
    }
    match payload {
        "toggle_interpolate" => nl.interpolate_unknowns = !nl.interpolate_unknowns,
        "toggle_lof" => nl.replicates_lack_of_fit_test = !nl.replicates_lack_of_fit_test,
        "toggle_normality" => nl.normality_of_residuals = !nl.normality_of_residuals,
        "toggle_homoscedasticity" => nl.homoscedasticity_test = !nl.homoscedasticity_test,
        "toggle_residuals_tab" => nl.emit_residuals_tab = !nl.emit_residuals_tab,
        "toggle_plot_only" => nl.plot_function_only = !nl.plot_function_only,
        "apply_range" => {
            let xmin = parse_optional_bound(app, "fit.wr.xmin.input");
            let xmax = parse_optional_bound(app, "fit.wr.xmax.input");
            nl.x_range_filter = if xmin.is_none() && xmax.is_none() { None } else { Some((xmin, xmax)) };
            if nl.plot_function_only {
                nl.plot_x_min = xmin;
                nl.plot_x_max = xmax;
            }
        }
        "clear_range" => {
            nl.x_range_filter = None;
            nl.y_range_filter = None;
            nl.plot_x_min = None;
            nl.plot_x_max = None;
            app.interaction.text_field_buffers.remove("fit.wr.xmin.input");
            app.interaction.text_field_buffers.remove("fit.wr.xmax.input");
        }
        _ => return false,
    }
    true
}

fn handle_method_and_weight_action(nl: &mut NonlinearFitSpec, payload: &str) -> bool {
    if let Some(m) = payload.strip_prefix("method:") {
        nl.method = match m {
            "poisson" => FitMethod::PoissonDeviance,
            "robust" => FitMethod::RobustCauchy,
            "rout" => FitMethod::RoutOutlierElimination {
                fdr_q_percent: match nl.method {
                    FitMethod::RoutOutlierElimination { fdr_q_percent } => fdr_q_percent,
                    _ => 1.0,
                },
            },
            _ => FitMethod::LeastSquares,
        };
        return true;
    }
    if let Some(q_str) = payload.strip_prefix("rout_q:") {
        if let Ok(q) = q_str.parse::<f64>()
            && q.is_finite()
        {
            nl.method = FitMethod::RoutOutlierElimination { fdr_q_percent: q.clamp(0.05, 25.0) };
        }
        return true;
    }
    if let Some(delta_str) = payload.strip_prefix("step_rout_q:") {
        if let Ok(delta) = delta_str.parse::<f64>()
            && let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut nl.method
        {
            *fdr_q_percent = (*fdr_q_percent + delta).clamp(0.1, 25.0);
        }
        return true;
    }
    if let Some(w) = payload.strip_prefix("weight:") {
        nl.weighting = match w {
            "rel_sq" => WeightingScheme::RelativeSquare,
            "rel" => WeightingScheme::Relative,
            "power" => WeightingScheme::GeneralPowerY(match nl.weighting {
                WeightingScheme::GeneralPowerY(k) => k,
                _ => 1.5,
            }),
            "inv_x" => WeightingScheme::InverseX,
            "inv_x2" => WeightingScheme::InverseX2,
            "obs_sd" => WeightingScheme::ObservedSd,
            _ => WeightingScheme::None,
        };
        return true;
    }
    if let Some(delta_str) = payload.strip_prefix("step_weight_k:") {
        if let Ok(delta) = delta_str.parse::<f64>()
            && let WeightingScheme::GeneralPowerY(k) = &mut nl.weighting
        {
            *k = (*k + delta).clamp(0.25, 4.0);
        }
        return true;
    }
    false
}

fn parse_optional_bound(app: &UiApp, id: &str) -> Option<f64> {
    let txt = app.interaction.text_field_buffers.get(id)?.text.trim();
    if txt.is_empty() { None } else { txt.parse::<f64>().ok().filter(|v| v.is_finite()) }
}

fn linear_range_field_text<'a>(app: &'a UiApp, field: &str) -> Option<&'a str> {
    app.interaction.text_field_buffers.get(&format!("{LINEAR_RANGE_INPUT_PREFIX}{field}")).map(|buffer| buffer.text.trim())
}

fn read_linear_range_inputs(app: &UiApp, mut fit: LinearRegressionSpec) -> Result<LinearRegressionSpec, String> {
    let bound = |(field, label), current| match linear_range_field_text(app, field) {
        None => Ok(current),
        Some("") => Ok(None),
        Some(text) => parse_localized_number_input(text, &app.locale_preset).map(Some).ok_or_else(|| format!("Enter a finite number for “{label}”.")),
    };
    let (lo, hi) = fit.x_range_filter.unwrap_or_default();
    let [min_field, max_field, plot_min_field, plot_max_field, (segments_field, _)] = LINEAR_RANGE_FIELDS;
    let lo = bound(min_field, lo)?;
    let hi = bound(max_field, hi)?;
    if matches!((lo, hi), (Some(lo), Some(hi)) if lo > hi) {
        return Err("The lower X filter bound must be at or below the upper bound.".to_owned());
    }
    fit.x_range_filter = (lo.is_some() || hi.is_some()).then_some((lo, hi));
    fit.plot_x_min = bound(plot_min_field, fit.plot_x_min)?;
    fit.plot_x_max = bound(plot_max_field, fit.plot_x_max)?;
    if matches!((fit.plot_x_min.or(lo), fit.plot_x_max.or(hi)), (Some(start), Some(end)) if start > end) {
        return Err("The curve start X must be at or below the curve end X.".to_owned());
    }
    if let Some(text) = linear_range_field_text(app, segments_field) {
        fit.plot_segments = if text.is_empty() {
            None
        } else {
            let max = LinearRegressionSpec::MAX_PLOT_SEGMENTS;
            let segments = text.parse::<usize>().ok().filter(|value| (1..=max).contains(value));
            Some(segments.ok_or_else(|| format!("Curve segments needs a whole number from 1 to {max}."))?)
        };
    }
    Ok(fit)
}

pub(crate) fn sync_linear_range_inputs(app: &mut UiApp) -> Result<(), String> {
    let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() else {
        return Ok(());
    };
    let AnalysisKind::LinearRegression(fit) = spec.kind else {
        return Ok(());
    };
    let fit = read_linear_range_inputs(app, fit).inspect_err(|_| {
        app.view_shards.fit_params.linear_range_selected = true;
    })?;
    if let Some(draft) = app.canvas_dialogs.edit_params.draft.as_mut() {
        draft.update_kind(AnalysisKind::LinearRegression(fit));
    }
    Ok(())
}

pub(crate) fn clear_linear_range_inputs(app: &mut UiApp) {
    app.interaction.text_field_buffers.retain(|id, _| !id.starts_with(LINEAR_RANGE_INPUT_PREFIX));
    app.view_shards.fit_params.linear_range_selected = false;
}

fn handle_linear_action(lr: &mut LinearRegressionSpec, confidence_level: &mut ConfidenceLevel, payload: &str) -> bool {
    match payload {
        "lr_toggle_deming" => lr.deming_model_ii = !lr.deming_model_ii,
        "lr_toggle_origin" => lr.force_through_origin = !lr.force_through_origin,
        "lr_toggle_conf" => lr.confidence_bands = !lr.confidence_bands,
        "lr_toggle_pred" => lr.prediction_bands = !lr.prediction_bands,
        "lr_toggle_interp" => lr.interpolate_unknowns = !lr.interpolate_unknowns,
        _ => {
            if let Some(lvl_str) = payload.strip_prefix("ci_level:")
                && let Ok(pct) = lvl_str.parse::<f64>()
                && pct.is_finite()
            {
                set_percent_clamped(confidence_level, pct);
                return true;
            }
            return false;
        }
    }
    true
}

fn handle_passing_bablok_action(pb: &mut PassingBablokSpec, confidence_level: &mut ConfidenceLevel, payload: &str) -> bool {
    match payload {
        "step_pb_x:-1" => pb.x_column = pb.x_column.saturating_sub(1),
        "step_pb_x:1" => pb.x_column = pb.x_column.saturating_add(1).min(63),
        "step_pb_y:-1" => pb.y_column = pb.y_column.saturating_sub(1),
        "step_pb_y:1" => pb.y_column = pb.y_column.saturating_add(1).min(63),
        _ => {
            if let Some(lvl_str) = payload.strip_prefix("ci_level:")
                && let Ok(pct) = lvl_str.parse::<f64>()
                && pct.is_finite()
            {
                set_percent_clamped(confidence_level, pct);
                return true;
            }
            return false;
        }
    }
    true
}

/// Return `true` when the active `edit_params.draft` targets a regression / curve-fit model.
#[must_use]
pub fn is_fit_analysis_draft(app: &UiApp) -> bool {
    app.canvas_dialogs
        .edit_params
        .draft
        .as_ref()
        .is_some_and(|d| matches!(d.kind, AnalysisKind::NonlinearRegression(_) | AnalysisKind::LinearRegression(_) | AnalysisKind::PassingBablok(_)))
}

/// Render curve-fit parameter controls inside `rect` cleanly within modal bounds.
pub fn build_fit_params_extra_section(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    _controls: &[crate::inspect::UiWidgetNode],
) {
    let Some(spec) = builder.app.canvas_dialogs.edit_params.draft.as_ref().cloned() else {
        return;
    };
    if rect.width < 120 || rect.height < 64 {
        return;
    }

    match &spec.kind {
        AnalysisKind::NonlinearRegression(nl) => build_nonlinear_section(builder, parent, rect, &spec, nl),
        AnalysisKind::LinearRegression(lr) => build_linear_regression_section(builder, parent, rect, &spec, lr),
        AnalysisKind::PassingBablok(pb) => build_passing_bablok_section(builder, parent, rect, &spec, pb),
        _ => {}
    }
}

fn custom_action(payload: impl Into<String>) -> WidgetAction {
    WidgetAction::Custom { domain: String::from(DOMAIN), payload: payload.into() }
}

struct FitBtn<'a> {
    id: &'a str,
    rect: LayoutRectPx,
    label: &'a str,
    kb_path: &'a str,
    selected: bool,
    primary: bool,
    enablement: ControlEnablement,
    payload: &'a str,
}

fn emit_fit_btn(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, cfg: FitBtn<'_>) {
    if cfg.rect.width < 8 || cfg.rect.height < 8 {
        return;
    }
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: cfg.id,
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: cfg.rect,
            label: cfg.label,
            keyboard_path: cfg.kb_path,
            focus_scope: FocusScope::ModalDialog,
            tab_order: None,
            primary: cfg.primary,
            selected: cfg.selected,
            shortcut: None,
            enablement: cfg.enablement,
            action: custom_action(cfg.payload),
        },
    );
}

struct FitToggle<'a> {
    id: &'a str,
    rect: LayoutRectPx,
    label: &'a str,
    kb_path: &'a str,
    checked: bool,
    is_radio: bool,
    enablement: ControlEnablement,
    scroll_container: Option<&'a str>,
    payload: &'a str,
}

fn emit_fit_tgl(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, cfg: FitToggle<'_>) {
    if cfg.rect.width < 16 || cfg.rect.height < 12 {
        return;
    }
    emit_toggle(
        builder,
        ToggleWidgetSpec {
            id: cfg.id,
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: cfg.rect,
            label: cfg.label,
            keyboard_path: cfg.kb_path,
            focus_scope: FocusScope::ModalDialog,
            tab_order: None,
            checked: cfg.checked,
            is_radio: cfg.is_radio,
            enablement: cfg.enablement,
            scroll_container: cfg.scroll_container,
            action: custom_action(cfg.payload),
        },
    );
}

struct FitInput<'a> {
    id: &'a str,
    rect: LayoutRectPx,
    label: &'a str,
    fallback_text: &'a str,
    kb_path: &'a str,
    multiline: bool,
    enablement: ControlEnablement,
}

fn emit_fit_input(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, cfg: FitInput<'_>) {
    if cfg.rect.width < 16 || cfg.rect.height < 12 {
        return;
    }
    emit_text_field(
        builder,
        TextFieldSpec {
            id: cfg.id,
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: cfg.rect,
            label: cfg.label,
            fallback_text: cfg.fallback_text,
            keyboard_path: cfg.kb_path,
            focus_scope: FocusScope::ModalDialog,
            tab_order: None,
            multiline: cfg.multiline,
            enablement: cfg.enablement,
        },
    );
}

fn text_style_px(size_px: f64, bold: bool) -> UiTextStyle {
    if bold { UiTextStyle::ui_bold(size_px) } else { UiTextStyle::ui_sans(size_px) }
}

fn paint_card_box(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, rect: LayoutRectPx, fill: Rgba8, border: Rgba8) {
    builder.paint_fill(ZLayer::ModalDialog, Some(owner), rect, fill);
    builder.paint_stroke(ZLayer::ModalDialog, Some(owner), rect, border, 1);
}

struct LabelLine<'a> {
    rect: LayoutRectPx,
    text: &'a str,
    size_px: f64,
    bold: bool,
    color: Rgba8,
    scroll_container: Option<&'a str>,
}

fn paint_label_line(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, cfg: LabelLine<'_>) {
    if cfg.rect.width < 6 || cfg.rect.height < 8 || cfg.text.is_empty() {
        return;
    }
    let style = text_style_px(cfg.size_px, cfg.bold);
    paint_text_fitted(
        builder,
        owner,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: cfg.rect,
            pad_x: 4,
            text: cfg.text,
            style: &style,
            color: cfg.color,
            scroll_container: cfg.scroll_container,
        },
    );
}

fn emit_scroll_container_start(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    scroll_id: &str,
    scroll_rect: LayoutRectPx,
    total_content_h: u32,
    a11y_label: &str,
) -> (WidgetId, LayoutRectPx, f64) {
    let inner_w = scroll_rect.width.saturating_sub(SCROLLBAR_RESERVE_PX);
    let list_clip = LayoutRectPx::new(scroll_rect.x + 2, scroll_rect.y + 2, inner_w, scroll_rect.height.saturating_sub(4));
    let scroll_meta = ScrollMeta::new(
        scroll_id,
        scroll_rect,
        [inner_w.max(1), total_content_h.max(list_clip.height)],
        builder.scroll_offset(scroll_id),
        ScrollAxes::Vertical,
    );
    let scroll_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(scroll_id),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: scroll_rect,
        a11y: labeled_meta(WidgetRole::Banner, a11y_label, FocusScope::ModalDialog, "dialog.scroll"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_card_box(builder, &scroll_wid, scroll_rect, builder.palette.surface, builder.palette.border);
    builder.attach_scroll(&scroll_wid, scroll_meta.clone());
    let scroll_y = scroll_meta.scroll_offset_px[1];
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(scroll_wid.clone()));
    builder.push_clip(list_clip);
    (scroll_wid, list_clip, scroll_y)
}

struct ScrollRowCfg<'a> {
    scroll_id: &'a str,
    id: &'a str,
    rect: LayoutRectPx,
    label: &'a str,
    a11y: AccessibilityMeta,
    selected: bool,
    enablement: ControlEnablement,
    action: WidgetAction,
}

fn emit_scroll_row_widget(builder: &mut LayoutBuilder<'_>, scroll_wid: &WidgetId, cfg: ScrollRowCfg<'_>) {
    let st = widget_state(builder, cfg.id, cfg.enablement);
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(cfg.id),
        parent: Some(scroll_wid.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: cfg.rect,
        a11y: cfg.a11y,
        state: st.clone(),
        action: cfg.action,
    });
    let fill = if cfg.selected {
        builder.palette.header
    } else if st.hovered && st.enablement.enabled {
        builder.palette.panel
    } else {
        builder.palette.surface
    };
    builder.paint_fill(ZLayer::ModalDialog, Some(&wid), cfg.rect, fill);
    let border = if cfg.selected || st.focused { builder.palette.accent } else { builder.palette.border };
    builder.paint_stroke(ZLayer::ModalDialog, Some(&wid), cfg.rect, border, 1);
    let text_color = if st.enablement.enabled { builder.palette.text } else { builder.palette.muted };
    let size_px = (12.0 * builder.scale()).clamp(10.0, 15.0);
    let style = text_style_px(size_px, cfg.selected);
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: cfg.rect,
            pad_x: 6,
            text: cfg.label,
            style: &style,
            color: text_color,
            scroll_container: Some(cfg.scroll_id),
        },
    );
}

fn build_nonlinear_section(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, spec: &AnalysisSpec, nl: &NonlinearFitSpec) {
    let state = &builder.app.view_shards.fit_params;
    let locale = builder.tree.locale.clone();
    let active_tab = state.effective_tab();

    let tab_h = 24_u32;
    let mut tab_x = rect.x;
    for tab in FitTab::ALL {
        let label = fit_tr(&locale, tab.i18n_key(), tab.fallback_label());
        let style = text_style_px((11.5 * builder.scale()).clamp(10.0, 15.0), true);
        let measured = builder.measurer.measure(label, &style, &locale).width_px.ceil() as u32;
        let max_avail = (rect.right() - tab_x).max(0) as u32;
        let tab_w = (measured + 20).min(max_avail);
        if tab_w < 28 {
            break;
        }
        let tab_rect = LayoutRectPx::new(tab_x, rect.y, tab_w, tab_h);
        let id = format!("fit.tab.{}", tab.id());
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id: &id,
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: tab_rect,
                label,
                keyboard_path: "dialog.fit.tabs",
                focus_scope: FocusScope::ModalDialog,
                tab_order: None,
                primary: false,
                selected: active_tab == tab,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: custom_action(format!("tab:{}", tab.id())),
            },
        );
        tab_x += tab_w as i32 + 4;
    }

    let body = LayoutRectPx::new(rect.x, rect.y + tab_h as i32 + 4, rect.width, rect.height.saturating_sub(tab_h + 4));
    match active_tab {
        FitTab::Equation => build_equation_tab(builder, parent, body, spec, nl, state, &locale),
        FitTab::Constraints => build_constraints_tab(builder, parent, body, nl, state, &locale),
        FitTab::InitialValues => build_initial_values_tab(builder, parent, body, spec, nl, state, &locale),
        FitTab::Confidence => build_confidence_tab(builder, parent, body, spec, nl, state, &locale),
        FitTab::Compare => build_compare_tab(builder, parent, body, nl, &locale),
        FitTab::WeightsRange => build_weights_range_tab(builder, parent, body, nl),
    }
}

#[derive(Debug, Clone)]
enum EqTreeRow {
    Category { meta: CatMeta, count: usize, expanded: bool },
    Equation(EquationCatalogEntry),
}

fn build_equation_tree_rows(state: &FitParamsViewState, nl: &NonlinearFitSpec, search: &str) -> Vec<EqTreeRow> {
    let entries = list_builtin_equations();
    let q = search.trim().to_ascii_lowercase();
    let active_cat = entries.iter().find(|e| e.id == nl.equation_id).map(|e| category_key(e.category));
    let mut rows = Vec::new();
    for meta in CATEGORIES {
        let cat_entries: Vec<EquationCatalogEntry> = entries
            .iter()
            .filter(|e| e.category == meta.cat)
            .filter(|e| {
                q.is_empty()
                    || e.name.to_ascii_lowercase().contains(&q)
                    || e.id.to_ascii_lowercase().contains(&q)
                    || meta.label.to_ascii_lowercase().contains(&q)
            })
            .cloned()
            .collect();
        if cat_entries.is_empty() {
            continue;
        }
        let expanded = if !q.is_empty() {
            true
        } else if state.tree_touched {
            state.expanded_categories.iter().any(|k| k == meta.key)
        } else {
            active_cat == Some(meta.key) || meta.cat == EquationCategory::DoseResponseInhibition
        };
        rows.push(EqTreeRow::Category { meta, count: cat_entries.len(), expanded });
        if expanded {
            for entry in cat_entries {
                rows.push(EqTreeRow::Equation(entry));
            }
        }
    }
    rows
}

struct MvSelectorRow<'a> {
    prefix: &'a str,
    row_lbl: &'a str,
    action_prefix: &'a str,
    titles: &'a [String],
}

fn emit_mv_column_selector_row(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    row: MvSelectorRow<'_>,
    is_selected: impl Fn(usize) -> bool,
) -> u32 {
    if row.titles.is_empty() || rect.width < 120 {
        return 0;
    }
    let lbl_w = 50_u32;
    let max_per_row = ((rect.width.saturating_sub(lbl_w)) / 72).max(1) as usize;
    let per_row = row.titles.len().clamp(1, max_per_row.min(6));
    let avail_w = rect.width.saturating_sub(lbl_w);
    let btn_w = (avail_w.saturating_sub((per_row as u32 - 1) * 4) / per_row as u32).max(40);
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(rect.x, rect.y, lbl_w, 20),
            text: row.row_lbl,
            size_px: 11.5,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    let mut rows = 0_u32;
    for (row_idx, chunk) in row.titles.chunks(per_row).enumerate() {
        let ry = rect.y + row_idx as i32 * 22;
        for (i, title) in chunk.iter().enumerate() {
            let col = row_idx * per_row + i;
            let bx = rect.x + lbl_w as i32 + i as i32 * (btn_w as i32 + 4);
            let id = format!("{}.{col}", row.prefix);
            emit_fit_btn(
                builder,
                parent,
                FitBtn {
                    id: &id,
                    rect: LayoutRectPx::new(bx, ry, btn_w, 20),
                    label: title,
                    kb_path: "dialog.fit.mv_col",
                    selected: is_selected(col),
                    primary: false,
                    enablement: ControlEnablement::enabled(),
                    payload: &format!("{}:{col}", row.action_prefix),
                },
            );
        }
        rows += 1;
    }
    rows.saturating_mul(22)
}

fn build_equation_tab(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    body: LayoutRectPx,
    spec: &AnalysisSpec,
    nl: &NonlinearFitSpec,
    state: &FitParamsViewState,
    locale: &str,
) {
    let mv_titles: Vec<String> = builder
        .app
        .document()
        .data_sheet(&spec.source_sheet_id)
        .and_then(|ds| match &ds.archetype {
            curvebook_doc::TableArchetype::MultipleVariables(mv) => Some(mv.variables.iter().map(|v| v.title.clone()).collect()),
            _ => None,
        })
        .unwrap_or_default();

    let mut eq_y = body.y;
    if !mv_titles.is_empty() && body.height >= 160 {
        let h_x = emit_mv_column_selector_row(
            builder,
            parent,
            LayoutRectPx::new(body.x, eq_y, body.width, 44),
            MvSelectorRow { prefix: "fit.mv.x", row_lbl: "X col:", action_prefix: "mv_x", titles: &mv_titles },
            |c| nl.mv_x_columns.contains(&c),
        );
        eq_y += h_x as i32 + 2;
        let h_y = emit_mv_column_selector_row(
            builder,
            parent,
            LayoutRectPx::new(body.x, eq_y, body.width, 44),
            MvSelectorRow { prefix: "fit.mv.y", row_lbl: "Y col:", action_prefix: "mv_y", titles: &mv_titles },
            |c| nl.mv_y_column == Some(c),
        );
        eq_y += h_y as i32 + 2;
        let h_g = emit_mv_column_selector_row(
            builder,
            parent,
            LayoutRectPx::new(body.x, eq_y, body.width, 44),
            MvSelectorRow { prefix: "fit.mv.group", row_lbl: "Group:", action_prefix: "mv_group", titles: &mv_titles },
            |c| nl.mv_group_columns.contains(&c),
        );
        eq_y += h_g as i32 + 6;
    }
    let used_h = u32::try_from((eq_y - body.y).max(0)).unwrap_or(0);
    let eq_body = LayoutRectPx::new(body.x, eq_y, body.width, body.height.saturating_sub(used_h));

    let left_w = ((eq_body.width * 50) / 100).max(240);
    let gap = 8_u32;
    let right_x = eq_body.x + left_w as i32 + gap as i32;
    let right_w = eq_body.width.saturating_sub(left_w + gap);

    let btn_w = 40_u32;
    let search_w = left_w.saturating_sub(btn_w * 3 + 12);
    let search_ph = fit_tr(locale, "fit_params.eq.search_placeholder", "Search equations…");
    emit_fit_input(
        builder,
        parent,
        FitInput {
            id: "fit.eq.search",
            rect: LayoutRectPx::new(eq_body.x, eq_body.y, search_w, 24),
            label: search_ph,
            fallback_text: &state.model_search,
            kb_path: "dialog.fit.eq.search",
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );
    let bx = eq_body.x + search_w as i32 + 4;
    for (i, (id, lbl, payload)) in [
        ("fit.eq.search_clear", "Clear", "eq_clear_search"),
        ("fit.eq.expand_all", "All", "eq_expand_all"),
        ("fit.eq.collapse_all", "None", "eq_collapse_all"),
    ]
    .iter()
    .enumerate()
    {
        emit_fit_btn(
            builder,
            parent,
            FitBtn {
                id,
                rect: LayoutRectPx::new(bx + i as i32 * (btn_w as i32 + 4), eq_body.y, btn_w, 24),
                label: lbl,
                kb_path: "dialog.fit.eq.toolbar",
                selected: false,
                primary: false,
                enablement: ControlEnablement::enabled(),
                payload,
            },
        );
    }

    let search_query = builder.app.interaction.text_field_buffers.get("fit.eq.search").map_or(state.model_search.as_str(), |b| b.text.as_str());
    let rows = build_equation_tree_rows(state, nl, search_query);
    let tree_rect = LayoutRectPx::new(eq_body.x, eq_body.y + 28, left_w, eq_body.height.saturating_sub(28));
    let total_h = (rows.len() as u32).saturating_mul(ROW_H_PX as u32);
    let (scroll_wid, list_clip, scroll_y) =
        emit_scroll_container_start(builder, parent, "fit.eq.tree.scroll", tree_rect, total_h, "Equation catalog tree");

    if rows.is_empty() {
        paint_label_line(
            builder,
            &scroll_wid,
            LabelLine {
                rect: LayoutRectPx::new(list_clip.x, list_clip.y + 8, list_clip.width, 22),
                text: "No equations match the current filter.",
                size_px: 12.0,
                bold: false,
                color: builder.palette.muted,
                scroll_container: Some("fit.eq.tree.scroll"),
            },
        );
    } else {
        emit_equation_tree_rows(builder, &scroll_wid, list_clip, scroll_y, &rows, nl);
    }
    builder.pop_clip();

    build_equation_right_pane(builder, parent, LayoutRectPx::new(right_x, eq_body.y, right_w, eq_body.height), spec, nl, state, locale);
}

fn emit_equation_tree_rows(
    builder: &mut LayoutBuilder<'_>,
    scroll_wid: &WidgetId,
    list_clip: LayoutRectPx,
    scroll_y: f64,
    rows: &[EqTreeRow],
    nl: &NonlinearFitSpec,
) {
    for (idx, row) in rows.iter().enumerate() {
        let y = list_clip.y + idx as i32 * ROW_H_PX - scroll_y.round() as i32;
        if y < list_clip.y || y + ROW_H_PX > list_clip.bottom() {
            continue;
        }
        match row {
            EqTreeRow::Category { meta, count, expanded } => {
                let arrow = if *expanded { "↓" } else { "→" };
                let text = format!("{arrow} {} ({count})", meta.label);
                let wid = format!("fit.eq.cat.{}", meta.key);
                emit_scroll_row_widget(
                    builder,
                    scroll_wid,
                    ScrollRowCfg {
                        scroll_id: "fit.eq.tree.scroll",
                        id: &wid,
                        rect: LayoutRectPx::new(list_clip.x, y, list_clip.width, ROW_H_PX as u32 - 2),
                        label: &text,
                        a11y: disclosure_meta(&text, FocusScope::ModalDialog, "dialog.fit.eq.tree", *expanded),
                        selected: false,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action(format!("toggle_cat:{}", meta.key)),
                    },
                );
            }
            EqTreeRow::Equation(entry) => {
                let selected = nl.custom_dsl.is_none() && nl.equation_id == entry.id;
                let text = format!("{} ({}p)", entry.name, entry.param_names.len());
                let wid = format!("fit.eq.item.{}", entry.id);
                emit_scroll_row_widget(
                    builder,
                    scroll_wid,
                    ScrollRowCfg {
                        scroll_id: "fit.eq.tree.scroll",
                        id: &wid,
                        rect: LayoutRectPx::new(list_clip.x + 10, y, list_clip.width.saturating_sub(10), ROW_H_PX as u32 - 2),
                        label: &text,
                        a11y: selectable_meta(WidgetRole::TreeItem, &entry.name, FocusScope::ModalDialog, "dialog.fit.eq.tree", selected),
                        selected,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action(format!("select_eq:{}", entry.id)),
                    },
                );
            }
        }
    }
}

fn build_equation_right_pane(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    pane: LayoutRectPx,
    spec: &AnalysisSpec,
    nl: &NonlinearFitSpec,
    state: &FitParamsViewState,
    locale: &str,
) {
    let dsl_active = state.custom_dsl_enabled || nl.custom_dsl.is_some();
    let dsl_label = fit_tr(locale, "fit_params.eq.custom_dsl_toggle", "Use Custom DSL Equation");
    emit_fit_tgl(
        builder,
        parent,
        FitToggle {
            id: "fit.eq.dsl_toggle",
            rect: LayoutRectPx::new(pane.x, pane.y, (pane.width * 56 / 100).max(160), 22),
            label: dsl_label,
            kb_path: "dialog.fit.eq.dsl_toggle",
            checked: dsl_active,
            is_radio: false,
            enablement: ControlEnablement::enabled(),
            scroll_container: None,
            payload: "toggle_custom_dsl",
        },
    );

    let mode_w = pane.width.saturating_sub(12) / 3;
    let mode_y = pane.y + 26;
    for (idx, (key, lbl)) in [("explicit", "Explicit"), ("implicit", "Implicit"), ("ode", "ODE (dY/dX)")].iter().enumerate() {
        let sel = dsl_active && state.effective_dsl_mode() == *key;
        emit_fit_btn(
            builder,
            parent,
            FitBtn {
                id: &format!("fit.eq.dsl_mode.{key}"),
                rect: LayoutRectPx::new(pane.x + idx as i32 * (mode_w as i32 + 6), mode_y, mode_w, 22),
                label: lbl,
                kb_path: "dialog.fit.eq.dsl_mode",
                selected: sel,
                primary: false,
                enablement: ControlEnablement::enabled(),
                payload: &format!("dsl_mode:{key}"),
            },
        );
    }

    let preview_y = if dsl_active {
        let dsl_fallback = nl.custom_dsl.as_deref().unwrap_or_else(|| dsl_starter_for_mode(state.effective_dsl_mode()));
        emit_fit_input(
            builder,
            parent,
            FitInput {
                id: "fit.dsl.input",
                rect: LayoutRectPx::new(pane.x, pane.y + 52, pane.width, 40),
                label: "Custom DSL expression",
                fallback_text: dsl_fallback,
                kb_path: "dialog.fit.eq.dsl_input",
                multiline: true,
                enablement: ControlEnablement::enabled(),
            },
        );
        let half_w = pane.width.saturating_sub(6) / 2;
        emit_fit_btn(
            builder,
            parent,
            FitBtn {
                id: "fit.eq.dsl_apply",
                rect: LayoutRectPx::new(pane.x, pane.y + 96, half_w, 24),
                label: fit_tr(locale, "fit_params.eq.dsl_apply", "Compile & Apply DSL"),
                kb_path: "dialog.fit.eq.dsl_apply",
                selected: false,
                primary: true,
                enablement: ControlEnablement::enabled(),
                payload: "apply_dsl",
            },
        );
        emit_fit_btn(
            builder,
            parent,
            FitBtn {
                id: "fit.eq.dsl_clear",
                rect: LayoutRectPx::new(pane.x + half_w as i32 + 6, pane.y + 96, half_w, 24),
                label: "Use Built-in Model",
                kb_path: "dialog.fit.eq.dsl_clear",
                selected: false,
                primary: false,
                enablement: ControlEnablement::enabled(),
                payload: "clear_dsl",
            },
        );
        pane.y + 124
    } else {
        pane.y + 52
    };

    paint_formula_preview_card(
        builder,
        parent,
        LayoutRectPx::new(pane.x, preview_y, pane.width, (pane.bottom() - preview_y).max(60) as u32),
        spec,
        nl,
        state,
        locale,
    );
}

fn paint_formula_preview_card(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    card_rect: LayoutRectPx,
    spec: &AnalysisSpec,
    nl: &NonlinearFitSpec,
    state: &FitParamsViewState,
    locale: &str,
) {
    let card_id = builder.push_node(NodeSpec {
        id: WidgetId::from("fit.eq.preview_card"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: card_rect,
        a11y: labeled_meta(
            WidgetRole::Label,
            fit_tr(locale, "fit_params.eq.preview_heading", "Model Formula Preview"),
            FocusScope::ModalDialog,
            "dialog.fit.eq.preview",
        ),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_card_box(builder, &card_id, card_rect, builder.palette.surface, builder.palette.border);

    let title = if let Some(dsl) = nl.custom_dsl.as_deref() {
        CompiledDslModel::compile("custom", dsl)
            .map_or_else(|_| String::from("Custom DSL (Draft)"), |m| format!("Custom DSL ({})", dsl_kind_label(m.kind())))
    } else {
        get_builtin_model(&nl.equation_id).map_or_else(|| nl.equation_id.clone(), |m| m.name().to_owned())
    };
    let points = active_sheet_points(builder.app.document(), &spec.source_sheet_id);
    let params = model_param_names(nl);
    let guesses = model_initial_guesses(nl, &points);
    let param_pairs: Vec<(String, f64)> = params.iter().enumerate().map(|(i, p)| (p.clone(), guesses.get(i).copied().unwrap_or(1.0))).collect();
    let tex_formula = format_model_equation_tex(&nl.equation_id, nl.custom_dsl.as_deref(), &param_pairs, false, 4);
    let display_formula = latex_to_display(&tex_formula);
    let params_line =
        if params.is_empty() { String::from("Parameters: none") } else { format!("Parameters ({}): {}", params.len(), params.join(", ")) };
    let status_line = if !state.dsl_status.is_empty() {
        state.dsl_status.clone()
    } else {
        format!("Model ID: {} | Rules: {} | Points: {}", nl.equation_id, nl.parameter_rules.len(), points.len())
    };
    let heading = fit_tr(locale, "fit_params.eq.preview_heading", "Model Formula Preview");
    let lines = [
        (heading, 11.5, true, builder.palette.muted),
        (title.as_str(), 12.5, true, builder.palette.text),
        (display_formula.as_str(), 12.0, false, builder.palette.accent),
        (params_line.as_str(), 11.0, false, builder.palette.text),
        (status_line.as_str(), 11.0, false, builder.palette.muted),
    ];
    let mut ly = card_rect.y + 6;
    for (text, size_px, bold, color) in lines {
        if ly + 18 > card_rect.bottom() {
            break;
        }
        paint_label_line(
            builder,
            &card_id,
            LabelLine {
                rect: LayoutRectPx::new(card_rect.x + 6, ly, card_rect.width.saturating_sub(12), 18),
                text,
                size_px,
                bold,
                color,
                scroll_container: None,
            },
        );
        ly += 20;
    }
}

fn build_constraints_tab(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    body: LayoutRectPx,
    nl: &NonlinearFitSpec,
    state: &FitParamsViewState,
    locale: &str,
) {
    let params = model_param_names(nl);
    let sel_idx = state.selected_param_idx.min(params.len().saturating_sub(1));

    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.con.share_all",
            rect: LayoutRectPx::new(body.x, body.y, 152, 24),
            label: fit_tr(locale, "fit_params.con.share_all", "Share All Globally"),
            kb_path: "dialog.fit.con.share_all",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "share_all",
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.con.unshare_all",
            rect: LayoutRectPx::new(body.x + 158, body.y, 134, 24),
            label: fit_tr(locale, "fit_params.con.unshare_all", "Reset to Local"),
            kb_path: "dialog.fit.con.unshare_all",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "unshare_all",
        },
    );

    let shared_count = nl.parameter_rules.iter().filter(|r| matches!(r.scope, ParameterScope::SharedGlobal)).count();
    let const_count = nl.parameter_rules.iter().filter(|r| is_fixed_scope(&r.scope)).count();
    let summary = format!("Params: {} | Shared: {shared_count} | Fixed: {const_count}", params.len());
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(body.x + 300, body.y + 2, body.width.saturating_sub(300), 20),
            text: &summary,
            size_px: 11.5,
            bold: false,
            color: builder.palette.muted,
            scroll_container: None,
        },
    );

    let inspector_h = 104_u32;
    let scroll_rect = LayoutRectPx::new(body.x, body.y + 28, body.width, body.height.saturating_sub(28 + inspector_h + 6));
    let total_h = (params.len() as u32).saturating_mul(ROW_H_TALL_PX as u32);
    let (scroll_wid, list_clip, scroll_y) =
        emit_scroll_container_start(builder, parent, "fit.con.scroll", scroll_rect, total_h, "Parameter constraints");

    let defaults = model_default_bounds(nl);
    for (idx, pname) in params.iter().enumerate() {
        let y = list_clip.y + idx as i32 * ROW_H_TALL_PX - scroll_y.round() as i32;
        if y < list_clip.y || y + ROW_H_TALL_PX > list_clip.bottom() {
            continue;
        }
        let rule = find_rule(nl, pname);
        let shared = rule.is_some_and(|r| matches!(r.scope, ParameterScope::SharedGlobal));
        let selected = idx == sel_idx;
        let def_b = defaults.get(idx).copied().unwrap_or((None, None));
        let lo_str = rule.and_then(|r| r.lower_bound).or(def_b.0).map_or_else(|| String::from("-inf"), format_compact_num);
        let hi_str = rule.and_then(|r| r.upper_bound).or(def_b.1).map_or_else(|| String::from("+inf"), format_compact_num);
        let scope_txt = rule.map_or_else(|| String::from("Local"), |r| scope_label(&r.scope));
        let row_label = format!("{pname}  [{lo_str}, {hi_str}]");

        let name_w = 210_u32.min(list_clip.width / 3);
        let shared_w = 122_u32;
        let scope_w = list_clip.width.saturating_sub(name_w + shared_w + 12);
        emit_scroll_row_widget(
            builder,
            &scroll_wid,
            ScrollRowCfg {
                scroll_id: "fit.con.scroll",
                id: &format!("fit.con.row.{idx}"),
                rect: LayoutRectPx::new(list_clip.x, y + 1, name_w, ROW_H_TALL_PX as u32 - 4),
                label: &row_label,
                a11y: selectable_meta(WidgetRole::TreeItem, pname, FocusScope::ModalDialog, "dialog.fit.con.list", selected),
                selected,
                enablement: ControlEnablement::enabled(),
                action: custom_action(format!("select_param:{idx}")),
            },
        );
        emit_fit_tgl(
            builder,
            &scroll_wid,
            FitToggle {
                id: &format!("fit.con.shared.{idx}"),
                rect: LayoutRectPx::new(list_clip.x + name_w as i32 + 6, y + 2, shared_w, ROW_H_TALL_PX as u32 - 6),
                label: "Shared Global",
                kb_path: "dialog.fit.con.shared",
                checked: shared,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: Some("fit.con.scroll"),
                payload: &format!("toggle_shared:{idx}"),
            },
        );
        emit_scroll_row_widget(
            builder,
            &scroll_wid,
            ScrollRowCfg {
                scroll_id: "fit.con.scroll",
                id: &format!("fit.con.scope.{idx}"),
                rect: LayoutRectPx::new(list_clip.x + name_w as i32 + shared_w as i32 + 12, y + 1, scope_w, ROW_H_TALL_PX as u32 - 4),
                label: &format!("Scope: {scope_txt}"),
                a11y: labeled_meta(WidgetRole::Button, format!("Cycle scope for {pname}"), FocusScope::ModalDialog, "dialog.fit.con.scope"),
                selected: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action(format!("cycle_scope:{idx}")),
            },
        );
    }
    builder.pop_clip();

    build_constraint_inspector(
        builder,
        parent,
        LayoutRectPx::new(body.x, body.bottom() - inspector_h as i32, body.width, inspector_h),
        nl,
        &params,
        sel_idx,
        &defaults,
        locale,
    );
}

#[allow(clippy::too_many_arguments)]
fn build_constraint_inspector(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    insp_rect: LayoutRectPx,
    nl: &NonlinearFitSpec,
    params: &[String],
    sel_idx: usize,
    defaults: &[(Option<f64>, Option<f64>)],
    locale: &str,
) {
    let insp_id = builder.push_node(NodeSpec {
        id: WidgetId::from("fit.con.inspector"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: insp_rect,
        a11y: labeled_meta(WidgetRole::Label, "Selected parameter rule", FocusScope::ModalDialog, "dialog.fit.con.inspector"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_card_box(builder, &insp_id, insp_rect, builder.palette.panel, builder.palette.border);

    let Some(sel_name) = params.get(sel_idx) else {
        return;
    };
    let sel_rule = find_rule(nl, sel_name);
    let b_en = bounds_enablement(sel_rule);
    let rule_title = format!("Editing: {sel_name} ({})", sel_rule.map_or_else(|| String::from("Local"), |r| scope_label(&r.scope)));
    paint_label_line(
        builder,
        &insp_id,
        LabelLine {
            rect: LayoutRectPx::new(insp_rect.x + 8, insp_rect.y + 6, 300, 20),
            text: &rule_title,
            size_px: 12.0,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    let scopes = [("local", "Local"), ("shared", "Shared"), ("constant", "Constant"), ("col_title", "Col Title")];
    for (i, (k, lbl)) in scopes.iter().enumerate() {
        let is_cur = matches!(
            (*k, sel_rule.map(|r| &r.scope)),
            ("local", None | Some(ParameterScope::Local))
                | ("shared", Some(ParameterScope::SharedGlobal))
                | ("constant", Some(ParameterScope::Constant(_)))
                | ("col_title", Some(ParameterScope::ColumnTitleConstant))
        );
        emit_fit_btn(
            builder,
            &insp_id,
            FitBtn {
                id: &format!("fit.con.set_scope.{k}"),
                rect: LayoutRectPx::new(insp_rect.x + 318 + i as i32 * 86, insp_rect.y + 5, 80, 22),
                label: lbl,
                kb_path: "dialog.fit.con.set_scope",
                selected: is_cur,
                primary: false,
                enablement: ControlEnablement::enabled(),
                payload: &format!("set_scope:{k}"),
            },
        );
    }

    let r2_y = insp_rect.y + 34;
    if let Some(ParameterScope::Constant(cval)) = sel_rule.map(|r| &r.scope) {
        emit_fit_btn(
            builder,
            &insp_id,
            FitBtn {
                id: "fit.con.const.dec",
                rect: LayoutRectPx::new(insp_rect.x + 8, r2_y, 28, 24),
                label: "-",
                kb_path: "dialog.fit.con.dec",
                selected: false,
                primary: false,
                enablement: ControlEnablement::enabled(),
                payload: "step_bound:const:-1",
            },
        );
        emit_fit_input(
            builder,
            &insp_id,
            FitInput {
                id: "fit.con.const.input",
                rect: LayoutRectPx::new(insp_rect.x + 40, r2_y, 120, 24),
                label: "Constant value",
                fallback_text: &format_compact_num(*cval),
                kb_path: "dialog.fit.con.const",
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
        emit_fit_btn(
            builder,
            &insp_id,
            FitBtn {
                id: "fit.con.const.inc",
                rect: LayoutRectPx::new(insp_rect.x + 164, r2_y, 28, 24),
                label: "+",
                kb_path: "dialog.fit.con.inc",
                selected: false,
                primary: false,
                enablement: ControlEnablement::enabled(),
                payload: "step_bound:const:1",
            },
        );
        emit_fit_btn(
            builder,
            &insp_id,
            FitBtn {
                id: "fit.con.apply",
                rect: LayoutRectPx::new(insp_rect.x + 202, r2_y, 150, 24),
                label: fit_tr(locale, "fit_params.con.apply_bounds", "Apply Parameter Rule"),
                kb_path: "dialog.fit.con.apply",
                selected: false,
                primary: true,
                enablement: ControlEnablement::enabled(),
                payload: "apply_param_rule",
            },
        );
    } else {
        let def_b = defaults.get(sel_idx).copied().unwrap_or((None, None));
        let lo_fb = sel_rule.and_then(|r| r.lower_bound).or(def_b.0).map_or_else(|| String::from("-inf"), format_compact_num);
        let hi_fb = sel_rule.and_then(|r| r.upper_bound).or(def_b.1).map_or_else(|| String::from("+inf"), format_compact_num);
        emit_fit_btn(
            builder,
            &insp_id,
            FitBtn {
                id: "fit.con.lo.dec",
                rect: LayoutRectPx::new(insp_rect.x + 8, r2_y, 26, 24),
                label: "-",
                kb_path: "dialog.fit.con.lo_dec",
                selected: false,
                primary: false,
                enablement: b_en.clone(),
                payload: "step_bound:lo:-1",
            },
        );
        emit_fit_input(
            builder,
            &insp_id,
            FitInput {
                id: "fit.con.lo.input",
                rect: LayoutRectPx::new(insp_rect.x + 38, r2_y, 92, 24),
                label: "Lower bound",
                fallback_text: &lo_fb,
                kb_path: "dialog.fit.con.lo",
                multiline: false,
                enablement: b_en.clone(),
            },
        );
        emit_fit_btn(
            builder,
            &insp_id,
            FitBtn {
                id: "fit.con.lo.inc",
                rect: LayoutRectPx::new(insp_rect.x + 134, r2_y, 26, 24),
                label: "+",
                kb_path: "dialog.fit.con.lo_inc",
                selected: false,
                primary: false,
                enablement: b_en.clone(),
                payload: "step_bound:lo:1",
            },
        );
        emit_fit_btn(
            builder,
            &insp_id,
            FitBtn {
                id: "fit.con.hi.dec",
                rect: LayoutRectPx::new(insp_rect.x + 174, r2_y, 26, 24),
                label: "-",
                kb_path: "dialog.fit.con.hi_dec",
                selected: false,
                primary: false,
                enablement: b_en.clone(),
                payload: "step_bound:hi:-1",
            },
        );
        emit_fit_input(
            builder,
            &insp_id,
            FitInput {
                id: "fit.con.hi.input",
                rect: LayoutRectPx::new(insp_rect.x + 204, r2_y, 92, 24),
                label: "Upper bound",
                fallback_text: &hi_fb,
                kb_path: "dialog.fit.con.hi",
                multiline: false,
                enablement: b_en.clone(),
            },
        );
        emit_fit_btn(
            builder,
            &insp_id,
            FitBtn {
                id: "fit.con.hi.inc",
                rect: LayoutRectPx::new(insp_rect.x + 300, r2_y, 26, 24),
                label: "+",
                kb_path: "dialog.fit.con.hi_inc",
                selected: false,
                primary: false,
                enablement: b_en.clone(),
                payload: "step_bound:hi:1",
            },
        );
        emit_fit_btn(
            builder,
            &insp_id,
            FitBtn {
                id: "fit.con.apply",
                rect: LayoutRectPx::new(insp_rect.x + 340, r2_y, 150, 24),
                label: fit_tr(locale, "fit_params.con.apply_bounds", "Apply Parameter Rule"),
                kb_path: "dialog.fit.con.apply",
                selected: false,
                primary: true,
                enablement: b_en.clone(),
                payload: "apply_param_rule",
            },
        );
        emit_fit_btn(
            builder,
            &insp_id,
            FitBtn {
                id: "fit.con.clear",
                rect: LayoutRectPx::new(insp_rect.x + 496, r2_y, 76, 24),
                label: "Clear",
                kb_path: "dialog.fit.con.clear",
                selected: false,
                primary: false,
                enablement: b_en.clone(),
                payload: "clear_param_bounds",
            },
        );
    }

    let r3_y = insp_rect.y + 68;
    paint_label_line(
        builder,
        &insp_id,
        LabelLine {
            rect: LayoutRectPx::new(insp_rect.x + 8, r3_y + 2, 120, 20),
            text: "Initial-Value Rule:",
            size_px: 11.0,
            bold: false,
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
    for (i, (k, lbl)) in [("ge0", "Lower >= 0"), ("gt0", "Lower > 0"), ("unit", "0 .. 100"), ("none", "Unbounded")].iter().enumerate() {
        emit_fit_btn(
            builder,
            &insp_id,
            FitBtn {
                id: &format!("fit.con.preset.{k}"),
                rect: LayoutRectPx::new(insp_rect.x + 132 + i as i32 * 96, r3_y, 90, 22),
                label: lbl,
                kb_path: "dialog.fit.con.preset",
                selected: false,
                primary: false,
                enablement: b_en.clone(),
                payload: &format!("bound_preset:{k}"),
            },
        );
    }
}

fn build_initial_values_tab(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    body: LayoutRectPx,
    spec: &AnalysisSpec,
    nl: &NonlinearFitSpec,
    state: &FitParamsViewState,
    locale: &str,
) {
    let params = model_param_names(nl);
    let points = active_sheet_points(builder.app.document(), &spec.source_sheet_id);
    let guesses = model_initial_guesses(nl, &points);
    let sel_idx = state.selected_param_idx.min(params.len().saturating_sub(1));

    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.init.auto_guess",
            rect: LayoutRectPx::new(body.x, body.y, 214, 24),
            label: fit_tr(locale, "fit_params.init.auto_guess", "Compute Data-Driven Initials"),
            kb_path: "dialog.fit.init.auto_guess",
            selected: false,
            primary: true,
            enablement: ControlEnablement::enabled(),
            payload: "compute_auto_initials",
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.init.clear",
            rect: LayoutRectPx::new(body.x + 220, body.y, 136, 24),
            label: "Use Auto Rules",
            kb_path: "dialog.fit.init.clear",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "clear_initial_overrides",
        },
    );
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(body.x + 366, body.y + 2, body.width.saturating_sub(366), 20),
            text: &format!("Data points sampled: {}", points.len()),
            size_px: 11.5,
            bold: false,
            color: builder.palette.muted,
            scroll_container: None,
        },
    );

    let bar_h = 46_u32;
    let scroll_rect = LayoutRectPx::new(body.x, body.y + 28, body.width, body.height.saturating_sub(28 + bar_h + 6));
    let total_h = (params.len() as u32).saturating_mul(ROW_H_TALL_PX as u32);
    let (scroll_wid, list_clip, scroll_y) =
        emit_scroll_container_start(builder, parent, "fit.init.scroll", scroll_rect, total_h, "Initial parameter estimates");

    for (idx, pname) in params.iter().enumerate() {
        let y = list_clip.y + idx as i32 * ROW_H_TALL_PX - scroll_y.round() as i32;
        if y < list_clip.y || y + ROW_H_TALL_PX > list_clip.bottom() {
            continue;
        }
        let rule = find_rule(nl, pname);
        let en = bounds_enablement(rule);
        let guess_str = guesses.get(idx).copied().filter(|v| v.is_finite()).map_or_else(|| String::from("auto"), format_compact_num);
        let override_str =
            rule.and_then(|r| r.initial_override).map_or_else(|| format!("Auto ({guess_str})"), |v| format!("Override: {}", format_compact_num(v)));
        let label = format!("{pname}: {override_str}");
        let main_w = list_clip.width.saturating_sub(152);
        emit_scroll_row_widget(
            builder,
            &scroll_wid,
            ScrollRowCfg {
                scroll_id: "fit.init.scroll",
                id: &format!("fit.init.row.{idx}"),
                rect: LayoutRectPx::new(list_clip.x, y + 1, main_w, ROW_H_TALL_PX as u32 - 4),
                label: &label,
                a11y: selectable_meta(WidgetRole::TreeItem, pname, FocusScope::ModalDialog, "dialog.fit.init.list", idx == sel_idx),
                selected: idx == sel_idx,
                enablement: ControlEnablement::enabled(),
                action: custom_action(format!("select_param:{idx}")),
            },
        );
        let bx = list_clip.x + main_w as i32 + 6;
        emit_fit_btn(
            builder,
            &scroll_wid,
            FitBtn {
                id: &format!("fit.init.dec.{idx}"),
                rect: LayoutRectPx::new(bx, y + 2, 30, 22),
                label: "-",
                kb_path: "dialog.fit.init.dec",
                selected: false,
                primary: false,
                enablement: en.clone(),
                payload: &format!("step_init:{idx}:-1"),
            },
        );
        emit_fit_btn(
            builder,
            &scroll_wid,
            FitBtn {
                id: &format!("fit.init.inc.{idx}"),
                rect: LayoutRectPx::new(bx + 34, y + 2, 30, 22),
                label: "+",
                kb_path: "dialog.fit.init.inc",
                selected: false,
                primary: false,
                enablement: en.clone(),
                payload: &format!("step_init:{idx}:1"),
            },
        );
        emit_fit_btn(
            builder,
            &scroll_wid,
            FitBtn {
                id: &format!("fit.init.rst.{idx}"),
                rect: LayoutRectPx::new(bx + 68, y + 2, 74, 22),
                label: "Reset",
                kb_path: "dialog.fit.init.rst",
                selected: false,
                primary: false,
                enablement: en,
                payload: &format!("init_clear_one:{idx}"),
            },
        );
    }
    builder.pop_clip();

    let bar_rect = LayoutRectPx::new(body.x, body.bottom() - bar_h as i32, body.width, bar_h);
    let bar_id = builder.push_node(NodeSpec {
        id: WidgetId::from("fit.init.editor_bar"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: bar_rect,
        a11y: labeled_meta(WidgetRole::Label, "Selected initial value editor", FocusScope::ModalDialog, "dialog.fit.init.bar"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_card_box(builder, &bar_id, bar_rect, builder.palette.panel, builder.palette.border);
    if let Some(sel_name) = params.get(sel_idx) {
        let rule = find_rule(nl, sel_name);
        let en = bounds_enablement(rule);
        let fallback =
            rule.and_then(|r| r.initial_override).or_else(|| guesses.get(sel_idx).copied()).map_or_else(|| String::from("1"), format_compact_num);
        paint_label_line(
            builder,
            &bar_id,
            LabelLine {
                rect: LayoutRectPx::new(bar_rect.x + 8, bar_rect.y + 12, 180, 22),
                text: &format!("Initial value ({sel_name}):"),
                size_px: 12.0,
                bold: true,
                color: builder.palette.text,
                scroll_container: None,
            },
        );
        emit_fit_input(
            builder,
            &bar_id,
            FitInput {
                id: "fit.init.val.input",
                rect: LayoutRectPx::new(bar_rect.x + 194, bar_rect.y + 11, 140, 24),
                label: "Initial value override",
                fallback_text: &fallback,
                kb_path: "dialog.fit.init.input",
                multiline: false,
                enablement: en.clone(),
            },
        );
        emit_fit_btn(
            builder,
            &bar_id,
            FitBtn {
                id: "fit.init.apply",
                rect: LayoutRectPx::new(bar_rect.x + 342, bar_rect.y + 11, 128, 24),
                label: "Apply Initial",
                kb_path: "dialog.fit.init.apply",
                selected: false,
                primary: true,
                enablement: en,
                payload: "apply_init_input",
            },
        );
    }
}

fn build_confidence_tab(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    body: LayoutRectPx,
    spec: &AnalysisSpec,
    nl: &NonlinearFitSpec,
    state: &FitParamsViewState,
    locale: &str,
) {
    let col_w = body.width.saturating_sub(14) / 2;
    let right_x = body.x + col_w as i32 + 14;
    let profile_en = profile_ci_enablement(nl, locale);
    let cov_en = covariance_enablement(nl);
    let ci_pct = spec.confidence_level.percent();

    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(body.x, body.y, col_w, 20),
            text: "Confidence Interval Method",
            size_px: 12.0,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    emit_fit_tgl(
        builder,
        parent,
        FitToggle {
            id: "fit.ci.asymptotic",
            rect: LayoutRectPx::new(body.x, body.y + 24, col_w, 24),
            label: fit_tr(locale, "fit_params.ci.asymptotic", "Asymptotic (Wald SE)"),
            kb_path: "dialog.fit.ci.asymp",
            checked: !nl.profile_likelihood_ci,
            is_radio: true,
            enablement: ControlEnablement::enabled(),
            scroll_container: None,
            payload: "ci_method:asymptotic",
        },
    );
    emit_fit_tgl(
        builder,
        parent,
        FitToggle {
            id: "fit.ci.profile",
            rect: LayoutRectPx::new(body.x, body.y + 52, col_w, 24),
            label: fit_tr(locale, "fit_params.ci.profile", "Profile Likelihood CI"),
            kb_path: "dialog.fit.ci.profile",
            checked: nl.profile_likelihood_ci,
            is_radio: true,
            enablement: profile_en,
            scroll_container: None,
            payload: "ci_method:profile",
        },
    );

    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(body.x, body.y + 88, col_w, 20),
            text: "Confidence Level (%)",
            size_px: 12.0,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    for (i, (val, lbl)) in [("90", "90%"), ("95", "95%"), ("99", "99%"), ("99.9", "99.9%")].iter().enumerate() {
        let target: f64 = val.parse().unwrap_or(95.0);
        emit_fit_btn(
            builder,
            parent,
            FitBtn {
                id: &format!("fit.ci.lvl.{val}"),
                rect: LayoutRectPx::new(body.x + i as i32 * 64, body.y + 112, 58, 24),
                label: lbl,
                kb_path: "dialog.fit.ci.lvl",
                selected: (ci_pct - target).abs() < 0.05,
                primary: false,
                enablement: ControlEnablement::enabled(),
                payload: &format!("ci_level:{val}"),
            },
        );
    }
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.ci.lvl.dec",
            rect: LayoutRectPx::new(body.x, body.y + 144, 28, 24),
            label: "-",
            kb_path: "dialog.fit.ci.lvl_dec",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "step_ci_level:-1",
        },
    );
    emit_fit_input(
        builder,
        parent,
        FitInput {
            id: "fit.ci.lvl.input",
            rect: LayoutRectPx::new(body.x + 34, body.y + 144, 96, 24),
            label: "Confidence level percent",
            fallback_text: &format_compact_num(ci_pct),
            kb_path: "dialog.fit.ci.lvl_input",
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.ci.lvl.inc",
            rect: LayoutRectPx::new(body.x + 136, body.y + 144, 28, 24),
            label: "+",
            kb_path: "dialog.fit.ci.lvl_inc",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "step_ci_level:1",
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.ci.lvl.apply",
            rect: LayoutRectPx::new(body.x + 170, body.y + 144, 84, 24),
            label: "Apply",
            kb_path: "dialog.fit.ci.lvl_apply",
            selected: false,
            primary: true,
            enablement: ControlEnablement::enabled(),
            payload: "apply_ci_level",
        },
    );
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(body.x, body.y + 176, col_w, 20),
            text: &format!(
                "Active level: {}% ({})",
                format_compact_num(ci_pct),
                if nl.profile_likelihood_ci { "Profile Likelihood" } else { "Asymptotic SE" }
            ),
            size_px: 11.5,
            bold: false,
            color: builder.palette.muted,
            scroll_container: None,
        },
    );

    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(right_x, body.y, col_w, 20),
            text: "Curve Bands & Diagnostics",
            size_px: 12.0,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    emit_fit_tgl(
        builder,
        parent,
        FitToggle {
            id: "fit.ci.conf_bands",
            rect: LayoutRectPx::new(right_x, body.y + 24, col_w, 24),
            label: "Plot Confidence Bands",
            kb_path: "dialog.fit.ci.conf_bands",
            checked: nl.confidence_bands,
            is_radio: false,
            enablement: ControlEnablement::enabled(),
            scroll_container: None,
            payload: "toggle_conf_bands",
        },
    );
    emit_fit_tgl(
        builder,
        parent,
        FitToggle {
            id: "fit.ci.pred_bands",
            rect: LayoutRectPx::new(right_x, body.y + 52, col_w, 24),
            label: "Plot Prediction Bands",
            kb_path: "dialog.fit.ci.pred_bands",
            checked: nl.prediction_bands,
            is_radio: false,
            enablement: ControlEnablement::enabled(),
            scroll_container: None,
            payload: "toggle_pred_bands",
        },
    );
    emit_fit_tgl(
        builder,
        parent,
        FitToggle {
            id: "fit.ci.emit_cov",
            rect: LayoutRectPx::new(right_x, body.y + 80, col_w, 24),
            label: "Emit Covariance Matrix Tab",
            kb_path: "dialog.fit.ci.emit_cov",
            checked: nl.emit_covariance_tab,
            is_radio: false,
            enablement: cov_en,
            scroll_container: None,
            payload: "toggle_emit_cov",
        },
    );

    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(right_x, body.y + 114, col_w, 20),
            text: "Ambiguous / Unstable Fit Flagging",
            size_px: 12.0,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    emit_fit_tgl(
        builder,
        parent,
        FitToggle {
            id: "fit.ci.flag_unstable",
            rect: LayoutRectPx::new(right_x, body.y + 138, col_w, 24),
            label: fit_tr(locale, "fit_params.ci.flag_unstable", "Report Ambiguous / Unstable Fits"),
            kb_path: "dialog.fit.ci.flag_unstable",
            checked: state.report_unstable_fits(),
            is_radio: false,
            enablement: ControlEnablement::enabled(),
            scroll_container: None,
            payload: "toggle_flag_unstable",
        },
    );

    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(right_x, body.y + 170, col_w, 20),
            text: &format!("Max Iterations: {}", nl.max_iters),
            size_px: 11.5,
            bold: false,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    for (i, n) in [100_usize, 250, 500, 1000].iter().enumerate() {
        emit_fit_btn(
            builder,
            parent,
            FitBtn {
                id: &format!("fit.ci.iters.{n}"),
                rect: LayoutRectPx::new(right_x + i as i32 * 56, body.y + 194, 50, 22),
                label: &n.to_string(),
                kb_path: "dialog.fit.ci.iters",
                selected: nl.max_iters == *n,
                primary: false,
                enablement: ControlEnablement::enabled(),
                payload: &format!("set_max_iters:{n}"),
            },
        );
    }
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.ci.iters.dec",
            rect: LayoutRectPx::new(right_x + 228, body.y + 194, 34, 22),
            label: "-50",
            kb_path: "dialog.fit.ci.iters_dec",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "step_max_iters:-50",
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.ci.iters.inc",
            rect: LayoutRectPx::new(right_x + 266, body.y + 194, 34, 22),
            label: "+50",
            kb_path: "dialog.fit.ci.iters_inc",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "step_max_iters:50",
        },
    );

    let pts = active_sheet_points(builder.app.document(), &spec.source_sheet_id).len();
    let free_p = model_param_names(nl).into_iter().filter(|name| !find_rule(nl, name).is_some_and(|r| is_fixed_scope(&r.scope))).count();
    let ambiguous_rs =
        builder.app.canvas_dialogs.edit_params.result_sheet_id.as_ref().and_then(|id| builder.app.document().result_sheet(id)).is_some_and(|rs| {
            matches!(rs.status, ResultStatus::Failed { .. })
                || rs.scalars.iter().any(|s| s.key.contains("ambiguous"))
                || rs.floating_notes.iter().any(|n| n.body_markdown.to_ascii_lowercase().contains("ambiguous"))
        });
    let diag_msg = if ambiguous_rs {
        String::from("Active result flagged warning/ambiguous; review bounds or initials.")
    } else if pts > 0 && free_p >= pts {
        format!("Warning: {free_p} free parameters for {pts} points (potential ambiguous fit).")
    } else if state.report_unstable_fits() {
        format!("Identifiability check active: {free_p} free params vs {pts} data points.")
    } else {
        String::from("Unstable fit highlight muted in preview (solver warnings still logged).")
    };
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(right_x, body.y + 224, col_w, 22),
            text: &diag_msg,
            size_px: 11.0,
            bold: false,
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
}

fn build_compare_tab(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body: LayoutRectPx, nl: &NonlinearFitSpec, locale: &str) {
    let left_w = 344_u32;
    let right_x = body.x + left_w as i32 + 12;
    let right_w = body.width.saturating_sub(left_w + 12);
    let cmp_en = compare_enablement(nl, locale);

    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(body.x, body.y, left_w, 20),
            text: "Model Comparison Method",
            size_px: 12.0,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    let (is_none, is_f, is_aicc, is_lrt) = match &nl.comparison {
        None => (true, false, false, false),
        Some(ModelComparisonSpec::ExtraSumOfSquaresF { .. }) => (false, true, false, false),
        Some(ModelComparisonSpec::Aicc { .. }) => (false, false, true, false),
        Some(ModelComparisonSpec::LikelihoodRatioTest { .. }) => (false, false, false, true),
    };
    emit_fit_tgl(
        builder,
        parent,
        FitToggle {
            id: "fit.cmp.none",
            rect: LayoutRectPx::new(body.x, body.y + 24, left_w, 24),
            label: fit_tr(locale, "fit_params.cmp.none", "None (Single Model)"),
            kb_path: "dialog.fit.cmp.none",
            checked: is_none,
            is_radio: true,
            enablement: ControlEnablement::enabled(),
            scroll_container: None,
            payload: "cmp_mode:none",
        },
    );
    emit_fit_tgl(
        builder,
        parent,
        FitToggle {
            id: "fit.cmp.f",
            rect: LayoutRectPx::new(body.x, body.y + 52, left_w, 24),
            label: fit_tr(locale, "fit_params.cmp.extra_ss_f", "Extra Sum-of-Squares F Test"),
            kb_path: "dialog.fit.cmp.f",
            checked: is_f,
            is_radio: true,
            enablement: cmp_en.clone(),
            scroll_container: None,
            payload: "cmp_mode:f",
        },
    );
    emit_fit_tgl(
        builder,
        parent,
        FitToggle {
            id: "fit.cmp.aicc",
            rect: LayoutRectPx::new(body.x, body.y + 80, left_w, 24),
            label: fit_tr(locale, "fit_params.cmp.aicc", "AICc Model Selection"),
            kb_path: "dialog.fit.cmp.aicc",
            checked: is_aicc,
            is_radio: true,
            enablement: cmp_en.clone(),
            scroll_container: None,
            payload: "cmp_mode:aicc",
        },
    );
    emit_fit_tgl(
        builder,
        parent,
        FitToggle {
            id: "fit.cmp.lrt",
            rect: LayoutRectPx::new(body.x, body.y + 108, left_w, 24),
            label: "Likelihood Ratio Test (LRT)",
            kb_path: "dialog.fit.cmp.lrt",
            checked: is_lrt,
            is_radio: true,
            enablement: cmp_en.clone(),
            scroll_container: None,
            payload: "cmp_mode:lrt",
        },
    );

    let alt_id = current_alt_equation_id(nl);
    let card = LayoutRectPx::new(body.x, body.y + 144, left_w, 80);
    let card_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("fit.cmp.summary_card"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: card,
        a11y: labeled_meta(WidgetRole::Label, "Comparison summary", FocusScope::ModalDialog, "dialog.fit.cmp.summary"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_card_box(builder, &card_wid, card, builder.palette.panel, builder.palette.border);
    paint_label_line(
        builder,
        &card_wid,
        LabelLine {
            rect: LayoutRectPx::new(card.x + 6, card.y + 8, card.width - 12, 20),
            text: &format!("Primary: {}", nl.equation_id),
            size_px: 11.5,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    paint_label_line(
        builder,
        &card_wid,
        LabelLine {
            rect: LayoutRectPx::new(card.x + 6, card.y + 32, card.width - 12, 20),
            text: &format!("Alternative: {alt_id}"),
            size_px: 11.5,
            bold: false,
            color: builder.palette.accent,
            scroll_container: None,
        },
    );

    let alt_en = if !cmp_en.enabled {
        cmp_en
    } else if nl.comparison.is_none() {
        ControlEnablement::disabled("Select Extra-SS F, AICc, or LRT first to choose an alternative model.")
    } else {
        ControlEnablement::enabled()
    };
    let search_w = right_w.saturating_sub(54);
    emit_fit_input(
        builder,
        parent,
        FitInput {
            id: "fit.cmp.alt.search",
            rect: LayoutRectPx::new(right_x, body.y, search_w, 24),
            label: "Search alternative model…",
            fallback_text: "",
            kb_path: "dialog.fit.cmp.search",
            multiline: false,
            enablement: alt_en.clone(),
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.cmp.alt.clear",
            rect: LayoutRectPx::new(right_x + search_w as i32 + 4, body.y, 48, 24),
            label: "Clear",
            kb_path: "dialog.fit.cmp.clear",
            selected: false,
            primary: false,
            enablement: alt_en.clone(),
            payload: "cmp_clear_search",
        },
    );

    let q = builder.app.interaction.text_field_buffers.get("fit.cmp.alt.search").map_or("", |b| b.text.as_str()).trim().to_ascii_lowercase();
    let entries: Vec<EquationCatalogEntry> = list_builtin_equations()
        .into_iter()
        .filter(|e| e.id != nl.equation_id && (q.is_empty() || e.name.to_ascii_lowercase().contains(&q) || e.id.to_ascii_lowercase().contains(&q)))
        .collect();
    let scroll_rect = LayoutRectPx::new(right_x, body.y + 28, right_w, body.height.saturating_sub(28));
    let total_h = (entries.len() as u32).saturating_mul(ROW_H_PX as u32);
    let (scroll_wid, list_clip, scroll_y) =
        emit_scroll_container_start(builder, parent, "fit.cmp.alt.scroll", scroll_rect, total_h, "Alternative comparison models");
    for (idx, entry) in entries.iter().enumerate() {
        let y = list_clip.y + idx as i32 * ROW_H_PX - scroll_y.round() as i32;
        if y < list_clip.y || y + ROW_H_PX > list_clip.bottom() {
            continue;
        }
        let sel = nl.comparison.is_some() && alt_id == entry.id;
        emit_scroll_row_widget(
            builder,
            &scroll_wid,
            ScrollRowCfg {
                scroll_id: "fit.cmp.alt.scroll",
                id: &format!("fit.cmp.alt.{}", entry.id),
                rect: LayoutRectPx::new(list_clip.x, y, list_clip.width, ROW_H_PX as u32 - 2),
                label: &format!("{} ({}p)", entry.name, entry.param_names.len()),
                a11y: selectable_meta(WidgetRole::TreeItem, &entry.name, FocusScope::ModalDialog, "dialog.fit.cmp.list", sel),
                selected: sel,
                enablement: alt_en.clone(),
                action: custom_action(format!("cmp_alt:{}", entry.id)),
            },
        );
    }
    builder.pop_clip();
}

fn build_weights_range_tab(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body: LayoutRectPx, nl: &NonlinearFitSpec) {
    let col_w = body.width.saturating_sub(20) / 3;
    let c2_x = body.x + col_w as i32 + 10;
    let c3_x = c2_x + col_w as i32 + 10;

    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(body.x, body.y, col_w, 20),
            text: "Fitting Method",
            size_px: 12.0,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    for (i, (k, lbl, checked)) in [
        ("ls", "Least Squares", matches!(nl.method, FitMethod::LeastSquares)),
        ("poisson", "Poisson Deviance", matches!(nl.method, FitMethod::PoissonDeviance)),
        ("robust", "Robust Cauchy", matches!(nl.method, FitMethod::RobustCauchy)),
        ("rout", "ROUT Outlier (Q)", matches!(nl.method, FitMethod::RoutOutlierElimination { .. })),
    ]
    .iter()
    .enumerate()
    {
        emit_fit_tgl(
            builder,
            parent,
            FitToggle {
                id: &format!("fit.wr.method.{k}"),
                rect: LayoutRectPx::new(body.x, body.y + 24 + i as i32 * 26, col_w, 24),
                label: lbl,
                kb_path: "dialog.fit.wr.method",
                checked: *checked,
                is_radio: true,
                enablement: ControlEnablement::enabled(),
                scroll_container: None,
                payload: &format!("method:{k}"),
            },
        );
    }
    let rout_en = if matches!(nl.method, FitMethod::RoutOutlierElimination { .. }) {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Enable ROUT Outlier (Q) fitting to adjust false-discovery rate Q%.")
    };
    let q_val = match nl.method {
        FitMethod::RoutOutlierElimination { fdr_q_percent } => fdr_q_percent,
        _ => 1.0,
    };
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(body.x, body.y + 136, col_w, 20),
            text: &format!("ROUT Q: {}%", format_compact_num(q_val)),
            size_px: 11.5,
            bold: false,
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.wr.rout_dec",
            rect: LayoutRectPx::new(body.x, body.y + 160, 44, 24),
            label: "-0.5",
            kb_path: "dialog.fit.wr.rout_dec",
            selected: false,
            primary: false,
            enablement: rout_en.clone(),
            payload: "step_rout_q:-0.5",
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.wr.rout_1",
            rect: LayoutRectPx::new(body.x + 50, body.y + 160, 44, 24),
            label: "1%",
            kb_path: "dialog.fit.wr.rout_1",
            selected: false,
            primary: false,
            enablement: rout_en.clone(),
            payload: "rout_q:1.0",
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.wr.rout_inc",
            rect: LayoutRectPx::new(body.x + 100, body.y + 160, 44, 24),
            label: "+0.5",
            kb_path: "dialog.fit.wr.rout_inc",
            selected: false,
            primary: false,
            enablement: rout_en,
            payload: "step_rout_q:0.5",
        },
    );

    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(c2_x, body.y, col_w, 20),
            text: "Weighting Scheme",
            size_px: 12.0,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    for (i, (k, lbl, checked)) in [
        ("none", "No Weighting (Equal)", matches!(nl.weighting, WeightingScheme::None)),
        ("rel_sq", "1 / Y^2 (Relative Sq)", matches!(nl.weighting, WeightingScheme::RelativeSquare)),
        ("rel", "1 / |Y| (Relative)", matches!(nl.weighting, WeightingScheme::Relative)),
        ("power", "General 1 / |Y|^k", matches!(nl.weighting, WeightingScheme::GeneralPowerY(_))),
        ("inv_x", "1 / |X|", matches!(nl.weighting, WeightingScheme::InverseX)),
        ("inv_x2", "1 / X^2", matches!(nl.weighting, WeightingScheme::InverseX2)),
        ("obs_sd", "1 / SD^2 (Observed)", matches!(nl.weighting, WeightingScheme::ObservedSd)),
    ]
    .iter()
    .enumerate()
    {
        emit_fit_tgl(
            builder,
            parent,
            FitToggle {
                id: &format!("fit.wr.weight.{k}"),
                rect: LayoutRectPx::new(c2_x, body.y + 24 + i as i32 * 25, col_w, 23),
                label: lbl,
                kb_path: "dialog.fit.wr.weight",
                checked: *checked,
                is_radio: true,
                enablement: ControlEnablement::enabled(),
                scroll_container: None,
                payload: &format!("weight:{k}"),
            },
        );
    }
    let pow_en = if matches!(nl.weighting, WeightingScheme::GeneralPowerY(_)) {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Select General 1 / |Y|^k weighting to adjust exponent k.")
    };
    let k_val = match nl.weighting {
        WeightingScheme::GeneralPowerY(k) => k,
        _ => 1.5,
    };
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(c2_x, body.y + 206, col_w, 20),
            text: &format!("Power k: {}", format_compact_num(k_val)),
            size_px: 11.5,
            bold: false,
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.wr.pow_dec",
            rect: LayoutRectPx::new(c2_x, body.y + 228, 52, 24),
            label: "-0.25",
            kb_path: "dialog.fit.wr.pow_dec",
            selected: false,
            primary: false,
            enablement: pow_en.clone(),
            payload: "step_weight_k:-0.25",
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.wr.pow_inc",
            rect: LayoutRectPx::new(c2_x + 58, body.y + 228, 52, 24),
            label: "+0.25",
            kb_path: "dialog.fit.wr.pow_inc",
            selected: false,
            primary: false,
            enablement: pow_en,
            payload: "step_weight_k:0.25",
        },
    );

    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(c3_x, body.y, col_w, 20),
            text: "Diagnostics & Range",
            size_px: 12.0,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    for (i, (k, lbl, checked, act)) in [
        ("interp", "Interpolate Unknowns", nl.interpolate_unknowns, "toggle_interpolate"),
        ("lof", "Replicates Lack-of-Fit", nl.replicates_lack_of_fit_test, "toggle_lof"),
        ("norm", "Normality of Residuals", nl.normality_of_residuals, "toggle_normality"),
        ("homo", "Homoscedasticity Test", nl.homoscedasticity_test, "toggle_homoscedasticity"),
        ("res_tab", "Emit Residuals Tab", nl.emit_residuals_tab, "toggle_residuals_tab"),
        ("plot_only", "Evaluate Function Only", nl.plot_function_only, "toggle_plot_only"),
    ]
    .iter()
    .enumerate()
    {
        emit_fit_tgl(
            builder,
            parent,
            FitToggle {
                id: &format!("fit.wr.opt.{k}"),
                rect: LayoutRectPx::new(c3_x, body.y + 24 + i as i32 * 25, col_w, 23),
                label: lbl,
                kb_path: "dialog.fit.wr.opts",
                checked: *checked,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: None,
                payload: act,
            },
        );
    }
    let (xmin_fb, xmax_fb) = match nl.x_range_filter {
        Some((lo, hi)) => (lo.map_or_else(String::new, format_compact_num), hi.map_or_else(String::new, format_compact_num)),
        None => (String::new(), String::new()),
    };
    let rx_y = body.y + 184;
    let half_w = col_w.saturating_sub(6) / 2;
    emit_fit_input(
        builder,
        parent,
        FitInput {
            id: "fit.wr.xmin.input",
            rect: LayoutRectPx::new(c3_x, rx_y, half_w, 24),
            label: "X min filter",
            fallback_text: &xmin_fb,
            kb_path: "dialog.fit.wr.xmin",
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );
    emit_fit_input(
        builder,
        parent,
        FitInput {
            id: "fit.wr.xmax.input",
            rect: LayoutRectPx::new(c3_x + half_w as i32 + 6, rx_y, half_w, 24),
            label: "X max filter",
            fallback_text: &xmax_fb,
            kb_path: "dialog.fit.wr.xmax",
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.wr.range_apply",
            rect: LayoutRectPx::new(c3_x, rx_y + 30, half_w, 24),
            label: "Apply Range",
            kb_path: "dialog.fit.wr.range_apply",
            selected: false,
            primary: true,
            enablement: ControlEnablement::enabled(),
            payload: "apply_range",
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.wr.range_clear",
            rect: LayoutRectPx::new(c3_x + half_w as i32 + 6, rx_y + 30, half_w, 24),
            label: "Clear",
            kb_path: "dialog.fit.wr.range_clear",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "clear_range",
        },
    );
}

fn build_linear_regression_section(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    spec: &AnalysisSpec,
    lr: &LinearRegressionSpec,
) {
    let range_selected = builder.app.view_shards.fit_params.linear_range_selected;
    let tab_width = (rect.width.saturating_sub(4) / 2).min(80);
    for (index, (id, label, payload, selected)) in
        [("fit.lr.tab.model", "Model", "lr_show_model", !range_selected), ("fit.lr.tab.range", "Range", "lr_show_range", range_selected)]
            .into_iter()
            .enumerate()
    {
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id,
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(rect.x + index as i32 * (tab_width as i32 + 4), rect.y, tab_width, 24),
                label,
                keyboard_path: "dialog.fit.lr.tabs",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(600 + index as u32),
                primary: false,
                selected,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: custom_action(payload),
            },
        );
    }
    let body = LayoutRectPx::new(rect.x, rect.y + 28, rect.width, rect.height.saturating_sub(28));
    if range_selected {
        build_linear_range_tab(builder, parent, body, lr);
    } else {
        build_linear_model_tab(builder, parent, body, spec, lr);
    }
}

fn build_linear_range_tab(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, body: LayoutRectPx, lr: &LinearRegressionSpec) {
    let (min, max) = lr.x_range_filter.unwrap_or_default();
    let locale = &builder.app.locale_preset;
    let values =
        [min, max, lr.plot_x_min, lr.plot_x_max].map(|value| value.map_or_else(String::new, |n| format_cell_display(&Cell::number(n), locale)));
    let segments = lr.plot_segments.map_or_else(String::new, |n| n.to_string());
    let col_width = body.width.saturating_sub(12) / 2;
    let values = values.iter().chain(std::iter::once(&segments));
    for (index, ((id, label), value)) in LINEAR_RANGE_FIELDS.into_iter().zip(values).enumerate() {
        let x = body.x + (index % 2) as i32 * (col_width as i32 + 12);
        let y = body.y + (index / 2) as i32 * 48;
        paint_label_line(
            builder,
            parent,
            LabelLine {
                rect: LayoutRectPx::new(x, y, col_width, 20),
                text: label,
                size_px: 12.0,
                bold: false,
                color: builder.palette.text,
                scroll_container: None,
            },
        );
        emit_text_field(
            builder,
            TextFieldSpec {
                id: &format!("{LINEAR_RANGE_INPUT_PREFIX}{id}"),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(x, y + 20, col_width, 24),
                label,
                fallback_text: value,
                keyboard_path: "dialog.fit.lr.range",
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(610 + index as u32),
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
    }
    let help = if let Err(message) = read_linear_range_inputs(builder.app, *lr) {
        message
    } else {
        format!("Blank curve bounds follow the fitted range. The default is {} segments.", LinearRegressionSpec::DEFAULT_PLOT_SEGMENTS)
    };
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(body.x, body.y + 146, body.width, 22),
            text: &help,
            size_px: 12.0,
            bold: false,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
}

fn build_linear_model_tab(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, spec: &AnalysisSpec, lr: &LinearRegressionSpec) {
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(rect.x, rect.y, rect.width, 22),
            text: "Simple Linear / Deming Regression Parameters",
            size_px: 13.0,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    let col_w = rect.width.saturating_sub(16) / 2;
    for (i, (k, lbl, checked, act)) in [
        ("deming", "Deming (Model II) Regression", lr.deming_model_ii, "lr_toggle_deming"),
        ("origin", "Force Regression Through Origin", lr.force_through_origin, "lr_toggle_origin"),
        ("conf", "Plot Confidence Bands", lr.confidence_bands, "lr_toggle_conf"),
        ("pred", "Plot Prediction Bands", lr.prediction_bands, "lr_toggle_pred"),
        ("interp", "Interpolate Unknowns", lr.interpolate_unknowns, "lr_toggle_interp"),
    ]
    .iter()
    .enumerate()
    {
        emit_fit_tgl(
            builder,
            parent,
            FitToggle {
                id: &format!("fit.lr.{k}"),
                rect: LayoutRectPx::new(rect.x, rect.y + 30 + i as i32 * 28, col_w, 24),
                label: lbl,
                kb_path: "dialog.fit.lr",
                checked: *checked,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: None,
                payload: act,
            },
        );
    }
    let rx = rect.x + col_w as i32 + 16;
    let ci_pct = spec.confidence_level.percent();
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(rx, rect.y + 30, col_w, 22),
            text: &format!("Confidence Level: {}%", format_compact_num(ci_pct)),
            size_px: 12.0,
            bold: false,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    for (i, (val, lbl)) in [("90", "90%"), ("95", "95%"), ("99", "99%")].iter().enumerate() {
        let target: f64 = val.parse().unwrap_or(95.0);
        emit_fit_btn(
            builder,
            parent,
            FitBtn {
                id: &format!("fit.lr.ci.{val}"),
                rect: LayoutRectPx::new(rx + i as i32 * 64, rect.y + 58, 58, 24),
                label: lbl,
                kb_path: "dialog.fit.lr.ci",
                selected: (ci_pct - target).abs() < 0.05,
                primary: false,
                enablement: ControlEnablement::enabled(),
                payload: &format!("ci_level:{val}"),
            },
        );
    }
}

fn build_passing_bablok_section(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, spec: &AnalysisSpec, pb: &PassingBablokSpec) {
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(rect.x, rect.y, rect.width, 22),
            text: "Passing-Bablok Method Comparison Parameters",
            size_px: 13.0,
            bold: true,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    let col_titles = column_titles_for_sheet(builder.app.document(), &spec.source_sheet_id);
    let x_name = col_titles.get(pb.x_column).map_or_else(|| format!("Col {}", pb.x_column + 1), Clone::clone);
    let y_name = col_titles.get(pb.y_column).map_or_else(|| format!("Col {}", pb.y_column + 1), Clone::clone);

    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(rect.x, rect.y + 34, 260, 22),
            text: &format!("Reference X Column: #{}, {x_name}", pb.x_column + 1),
            size_px: 12.0,
            bold: false,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.pb.x.dec",
            rect: LayoutRectPx::new(rect.x + 270, rect.y + 32, 32, 24),
            label: "-",
            kb_path: "dialog.fit.pb.x_dec",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "step_pb_x:-1",
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.pb.x.inc",
            rect: LayoutRectPx::new(rect.x + 308, rect.y + 32, 32, 24),
            label: "+",
            kb_path: "dialog.fit.pb.x_inc",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "step_pb_x:1",
        },
    );

    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(rect.x, rect.y + 68, 260, 22),
            text: &format!("Test Y Column: #{}, {y_name}", pb.y_column + 1),
            size_px: 12.0,
            bold: false,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.pb.y.dec",
            rect: LayoutRectPx::new(rect.x + 270, rect.y + 66, 32, 24),
            label: "-",
            kb_path: "dialog.fit.pb.y_dec",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "step_pb_y:-1",
        },
    );
    emit_fit_btn(
        builder,
        parent,
        FitBtn {
            id: "fit.pb.y.inc",
            rect: LayoutRectPx::new(rect.x + 308, rect.y + 66, 32, 24),
            label: "+",
            kb_path: "dialog.fit.pb.y_inc",
            selected: false,
            primary: false,
            enablement: ControlEnablement::enabled(),
            payload: "step_pb_y:1",
        },
    );

    let ci_pct = spec.confidence_level.percent();
    paint_label_line(
        builder,
        parent,
        LabelLine {
            rect: LayoutRectPx::new(rect.x, rect.y + 106, 220, 22),
            text: &format!("Confidence Level: {}%", format_compact_num(ci_pct)),
            size_px: 12.0,
            bold: false,
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    for (i, (val, lbl)) in [("90", "90%"), ("95", "95%"), ("99", "99%")].iter().enumerate() {
        let target: f64 = val.parse().unwrap_or(95.0);
        emit_fit_btn(
            builder,
            parent,
            FitBtn {
                id: &format!("fit.pb.ci.{val}"),
                rect: LayoutRectPx::new(rect.x + 230 + i as i32 * 64, rect.y + 104, 58, 24),
                label: lbl,
                kb_path: "dialog.fit.pb.ci",
                selected: (ci_pct - target).abs() < 0.05,
                primary: false,
                enablement: ControlEnablement::enabled(),
                payload: &format!("ci_level:{val}"),
            },
        );
    }
}

fn column_titles_for_sheet(doc: &ProjectDocument, sid: &SheetId) -> Vec<String> {
    let Some(sheet) = resolve_data_sheet(doc, sid) else {
        return Vec::new();
    };
    sheet_column_titles(sheet)
}

fn sheet_column_titles(sheet: &DataSheet) -> Vec<String> {
    match &sheet.archetype {
        TableArchetype::Xy(xy) => {
            let mut out = vec![xy.x_title.clone()];
            out.extend(xy.data_sets.iter().map(|c| c.title.clone()));
            out
        }
        TableArchetype::Column(c) => c.data_sets.iter().map(|col| col.title.clone()).collect(),
        TableArchetype::Grouped(g) => g.data_sets.iter().map(|col| col.title.clone()).collect(),
        TableArchetype::Contingency(ct) => ct.data_sets.iter().map(|col| col.title.clone()).collect(),
        TableArchetype::Survival(s) => s.cohorts.iter().map(|col| col.title.clone()).collect(),
        TableArchetype::PartsOfWhole(p) => p.data_sets.iter().map(|col| col.title.clone()).collect(),
        TableArchetype::MultipleVariables(mv) => mv.variables.iter().map(|v| v.title.clone()).collect(),
        TableArchetype::Nested(n) => n.groups.iter().map(|g| g.title.clone()).collect(),
    }
}
