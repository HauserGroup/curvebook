//! Nonlinear regression, dose-response, and curve-fit parameter modal helpers (`R10.3`).
//!
//! Holds curve-fitting state and action helpers backing `egui_shell::fit_modal`
//! (`Equation`, `Constraints`, `Initial Values`, `Confidence`, `Compare`, `Weights & Range`)
//! plus regression controls for [`AnalysisKind::LinearRegression`] and
//! [`AnalysisKind::PassingBablok`], and handles custom actions targeting the `"fit_params"` domain.

use std::fmt::Write as _;

use curvebook_doc::{
    AnalysisKind, ConfidenceLevel, DataSheet, FitMethod, LinearRegressionSpec, ModelComparisonSpec, NonlinearFitSpec, ParameterRule, ParameterScope,
    PassingBablokSpec, ProjectDocument, SheetId, WeightingScheme,
};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DslEquationKind, EquationCategory, ParametricModel as _, extract_datasets_from_sheet, get_builtin_model,
    list_builtin_equations,
};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::grid_view::parse_localized_number_input;
use crate::interaction::{TextFieldEditState, UiEventOutcome};

const DOMAIN: &str = "fit_params";
const LINEAR_RANGE_INPUT_PREFIX: &str = "fit.lr.range.input.";
const R_SQUARED_THRESHOLD_INPUT: &str = "fit.diagnostics.r2.input";
const LINEAR_RANGE_FIELDS: [(&str, &str); 5] = [
    ("x_min", "Ignore points where X <"),
    ("x_max", "Ignore points where X >"),
    ("plot_min", "Curve start X"),
    ("plot_max", "Curve end X"),
    ("segments", "Curve segments"),
];

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
}

impl FitParamsViewState {
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
    Diagnostics,
}

impl FitTab {
    fn from_id(id: &str) -> Self {
        match id {
            "constraints" => Self::Constraints,
            "initial_values" => Self::InitialValues,
            "confidence" => Self::Confidence,
            "compare" => Self::Compare,
            "weights_range" => Self::WeightsRange,
            "diagnostics" => Self::Diagnostics,
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
            Self::Diagnostics => "diagnostics",
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

fn is_fixed_scope(scope: &ParameterScope) -> bool {
    matches!(
        scope,
        ParameterScope::Constant(_) | ParameterScope::ColumnTitleConstant | ParameterScope::HookedConstant(_) | ParameterScope::PerDatasetConstant(_)
    )
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
        AnalysisKind::NonlinearRegression(nl) => handle_nonlinear_action(app, state, nl, &mut spec.confidence_level, payload)?,
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

fn parse_finite_input(app: &UiApp, label: &str, raw: &str) -> Result<f64, String> {
    parse_localized_number_input(raw, &app.locale_preset).ok_or_else(|| format!("Enter a finite number for “{label}”."))
}

fn handle_nonlinear_action(
    app: &mut UiApp,
    state: &mut FitParamsViewState,
    nl: &mut NonlinearFitSpec,
    confidence_level: &mut ConfidenceLevel,
    payload: &str,
) -> Result<bool, String> {
    if handle_equation_action(app, state, nl, payload) {
        return Ok(true);
    }
    if handle_constraints_action(app, state, nl, payload)? {
        return Ok(true);
    }
    if handle_initial_values_action(app, state, nl, payload)? {
        return Ok(true);
    }
    if handle_confidence_action(app, nl, confidence_level, payload)? {
        return Ok(true);
    }
    if handle_compare_action(app, nl, payload) {
        return Ok(true);
    }
    handle_weights_range_action(app, nl, payload)
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

fn handle_constraints_action(app: &mut UiApp, state: &mut FitParamsViewState, nl: &mut NonlinearFitSpec, payload: &str) -> Result<bool, String> {
    let params = model_param_names(nl);
    if handle_constraints_row_action(app, state, nl, &params, payload) {
        return Ok(true);
    }
    let Some(selected_name) = params.get(state.selected_param_idx.min(params.len().saturating_sub(1))).cloned() else {
        return Ok(false);
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
) -> Result<bool, String> {
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
        return Ok(true);
    }
    if payload == "apply_param_rule" {
        apply_selected_param_rule_inputs(app, nl, selected_name)?;
        return Ok(true);
    }
    if payload == "clear_param_bounds" {
        let rule = ensure_rule_mut(nl, selected_name);
        if !is_fixed_scope(&rule.scope) {
            rule.lower_bound = None;
            rule.upper_bound = None;
            app.interaction.text_field_buffers.remove("fit.con.lo.input");
            app.interaction.text_field_buffers.remove("fit.con.hi.input");
        }
        return Ok(true);
    }
    if let Some(preset) = payload.strip_prefix("bound_preset:") {
        apply_bound_preset(app, nl, selected_name, preset);
        return Ok(true);
    }
    if let Some(delta_spec) = payload.strip_prefix("step_bound:") {
        return Ok(handle_bound_stepper(app, nl, selected_name, delta_spec));
    }
    Ok(false)
}

fn apply_selected_param_rule_inputs(app: &UiApp, nl: &mut NonlinearFitSpec, selected_name: &str) -> Result<(), String> {
    let existing_scope = nl.parameter_rules.iter().find(|r| r.name == selected_name).map(|r| r.scope.clone());
    if matches!(existing_scope, Some(ParameterScope::Constant(_))) {
        if let Some(buf) = app.interaction.text_field_buffers.get("fit.con.const.input") {
            let val = parse_finite_input(app, "Constant value", buf.text.trim())?;
            ensure_rule_mut(nl, selected_name).scope = ParameterScope::Constant(val);
        }
        return Ok(());
    }
    if existing_scope.as_ref().is_some_and(is_fixed_scope) {
        return Ok(());
    }
    let next_lo = if let Some(buf) = app.interaction.text_field_buffers.get("fit.con.lo.input") {
        let txt = buf.text.trim();
        if txt.is_empty() || txt.eq_ignore_ascii_case("-inf") || txt.eq_ignore_ascii_case("none") {
            Some(None)
        } else {
            Some(Some(parse_finite_input(app, "Lower bound", txt)?))
        }
    } else {
        None
    };
    let next_hi = if let Some(buf) = app.interaction.text_field_buffers.get("fit.con.hi.input") {
        let txt = buf.text.trim();
        if txt.is_empty() || txt.eq_ignore_ascii_case("+inf") || txt.eq_ignore_ascii_case("inf") || txt.eq_ignore_ascii_case("none") {
            Some(None)
        } else {
            Some(Some(parse_finite_input(app, "Upper bound", txt)?))
        }
    } else {
        None
    };
    let rule = ensure_rule_mut(nl, selected_name);
    if let Some(lo) = next_lo {
        rule.lower_bound = lo;
    }
    if let Some(hi) = next_hi {
        rule.upper_bound = hi;
    }
    if let (Some(lo), Some(hi)) = (rule.lower_bound, rule.upper_bound)
        && lo > hi
    {
        rule.upper_bound = Some(lo);
    }
    Ok(())
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

fn handle_initial_values_action(app: &mut UiApp, state: &mut FitParamsViewState, nl: &mut NonlinearFitSpec, payload: &str) -> Result<bool, String> {
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
        return Ok(true);
    }
    if payload == "clear_initial_overrides" {
        for name in &params {
            ensure_rule_mut(nl, name).initial_override = None;
        }
        app.interaction.text_field_buffers.remove("fit.init.val.input");
        return Ok(true);
    }
    if let Some(idx_str) = payload.strip_prefix("init_clear_one:") {
        if let Ok(idx) = idx_str.parse::<usize>()
            && let Some(name) = params.get(idx)
        {
            ensure_rule_mut(nl, name).initial_override = None;
            state.selected_param_idx = idx;
            app.interaction.text_field_buffers.remove("fit.init.val.input");
        }
        return Ok(true);
    }
    if let Some(rest) = payload.strip_prefix("step_init:") {
        return Ok(step_initial_value(app, state, nl, &params, rest));
    }
    if payload == "apply_init_input" {
        let Some(name) = params.get(state.selected_param_idx.min(params.len().saturating_sub(1))).cloned() else {
            return Ok(false);
        };
        let is_fixed = nl.parameter_rules.iter().find(|r| r.name == name).is_some_and(|r| is_fixed_scope(&r.scope));
        if !is_fixed && let Some(buf) = app.interaction.text_field_buffers.get("fit.init.val.input") {
            let txt = buf.text.trim();
            let next_val =
                if txt.is_empty() || txt.eq_ignore_ascii_case("auto") { None } else { Some(parse_finite_input(app, "Initial value", txt)?) };
            ensure_rule_mut(nl, &name).initial_override = next_val;
        }
        return Ok(true);
    }
    Ok(false)
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
    nl: &mut NonlinearFitSpec,
    confidence_level: &mut ConfidenceLevel,
    payload: &str,
) -> Result<bool, String> {
    if let Some(mode) = payload.strip_prefix("ci_method:") {
        if mode == "profile" && robust_or_rout(nl.method) {
            return Err(crate::i18n::tr(&app.locale, "fit_params.dis.robust_profile_ci").to_owned());
        }
        nl.profile_likelihood_ci = mode == "profile";
        return Ok(true);
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
        return Ok(true);
    }
    if let Some(delta_str) = payload.strip_prefix("step_ci_level:") {
        if let Ok(delta_pct) = delta_str.parse::<f64>() {
            let next_pct = (confidence_level.percent() + delta_pct).clamp(50.0, 99.9);
            set_percent_clamped(confidence_level, next_pct);
            app.interaction
                .text_field_buffers
                .insert(String::from("fit.ci.lvl.input"), TextFieldEditState::new(format_compact_num(confidence_level.percent()), false));
        }
        return Ok(true);
    }
    if payload == "apply_ci_level" {
        if let Some(buf) = app.interaction.text_field_buffers.get("fit.ci.lvl.input") {
            let pct = parse_finite_input(app, "Confidence level", buf.text.trim())?;
            set_percent_clamped(confidence_level, pct);
        }
        return Ok(true);
    }
    match payload {
        "toggle_conf_bands" => {
            nl.confidence_bands = !nl.confidence_bands;
            return Ok(true);
        }
        "toggle_pred_bands" => {
            nl.prediction_bands = !nl.prediction_bands;
            return Ok(true);
        }
        "toggle_emit_cov" => {
            if matches!(nl.method, FitMethod::RobustCauchy) {
                return Err(String::from("Covariance matrix is unavailable for Robust nonlinear regression."));
            }
            nl.emit_covariance_tab = !nl.emit_covariance_tab;
            return Ok(true);
        }
        "toggle_flag_unstable" => {
            nl.identify_unstable_parameters = !nl.identify_unstable_parameters;
            return Ok(true);
        }
        _ => {}
    }
    if let Some(iters_str) = payload.strip_prefix("set_max_iters:") {
        if let Ok(iters) = iters_str.parse::<usize>() {
            nl.max_iters = iters.clamp(10, 10_000);
        }
        return Ok(true);
    }
    if let Some(delta_str) = payload.strip_prefix("step_max_iters:") {
        if let Ok(delta) = delta_str.parse::<i32>() {
            let next = (nl.max_iters as i32).saturating_add(delta).clamp(10, 10_000);
            nl.max_iters = next as usize;
        }
        return Ok(true);
    }
    Ok(false)
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

fn handle_weights_range_action(app: &mut UiApp, nl: &mut NonlinearFitSpec, payload: &str) -> Result<bool, String> {
    if handle_method_and_weight_action(nl, payload) {
        return Ok(true);
    }
    match payload {
        "toggle_interpolate" => nl.interpolate_unknowns = !nl.interpolate_unknowns,
        "toggle_lof" => nl.replicates_lack_of_fit_test = !nl.replicates_lack_of_fit_test,
        "toggle_normality" => nl.normality_of_residuals = !nl.normality_of_residuals,
        "toggle_homoscedasticity" => nl.homoscedasticity_test = !nl.homoscedasticity_test,
        "toggle_residuals_tab" => nl.emit_residuals_tab = !nl.emit_residuals_tab,
        "toggle_plot_only" => nl.plot_function_only = !nl.plot_function_only,
        "apply_range" => {
            let xmin = parse_optional_bound(app, "X range min", "fit.wr.xmin.input")?;
            let xmax = parse_optional_bound(app, "X range max", "fit.wr.xmax.input")?;
            if matches!((xmin, xmax), (Some(lo), Some(hi)) if lo > hi) {
                return Err(String::from("The lower X range bound must be at or below the upper bound."));
            }
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
        _ => return Ok(false),
    }
    Ok(true)
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

fn parse_optional_bound(app: &UiApp, label: &str, id: &str) -> Result<Option<f64>, String> {
    let Some(buf) = app.interaction.text_field_buffers.get(id) else {
        return Ok(None);
    };
    let txt = buf.text.trim();
    if txt.is_empty() { Ok(None) } else { parse_finite_input(app, label, txt).map(Some) }
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

fn read_r_squared_threshold(app: &UiApp, current: Option<f64>) -> Result<Option<f64>, String> {
    let Some(text) = app.interaction.text_field_buffers.get(R_SQUARED_THRESHOLD_INPUT).map(|buf| buf.text.trim()) else {
        return Ok(current);
    };
    if text.is_empty() {
        return Ok(None);
    }
    parse_localized_number_input(text, &app.locale_preset)
        .filter(|value| (0.0..=1.0).contains(value))
        .map(Some)
        .ok_or_else(|| "Enter an R² threshold from 0 to 1 or leave it blank.".to_owned())
}

pub(crate) fn sync_fit_parameter_inputs(app: &mut UiApp) -> Result<(), String> {
    let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() else {
        return Ok(());
    };
    let kind = match &spec.kind {
        AnalysisKind::LinearRegression(fit) => {
            let fit = read_linear_range_inputs(app, *fit).inspect_err(|_| {
                app.view_shards.fit_params.linear_range_selected = true;
            })?;
            AnalysisKind::LinearRegression(fit)
        }
        AnalysisKind::NonlinearRegression(fit) if app.interaction.text_field_buffers.contains_key(R_SQUARED_THRESHOLD_INPUT) => {
            let mut fit = fit.clone();
            fit.flag_r_squared_below = read_r_squared_threshold(app, fit.flag_r_squared_below).inspect_err(|_| {
                app.view_shards.fit_params.active_fit_tab = FitTab::Diagnostics.id().to_owned();
            })?;
            AnalysisKind::NonlinearRegression(fit)
        }
        _ => return Ok(()),
    };
    if let Some(draft) = app.canvas_dialogs.edit_params.draft.as_mut() {
        draft.update_kind(kind);
    }
    Ok(())
}

pub(crate) fn clear_fit_parameter_inputs(app: &mut UiApp) {
    app.interaction.text_field_buffers.retain(|id, _| id != R_SQUARED_THRESHOLD_INPUT && !id.starts_with(LINEAR_RANGE_INPUT_PREFIX));
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
