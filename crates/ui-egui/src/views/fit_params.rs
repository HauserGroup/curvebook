//! Nonlinear regression, dose-response, and curve-fit parameter modal helpers (`R10.3`).

use curvebook_doc::{
    AnalysisKind, ConfidenceLevel, DataSheet, FitMethod, LinearRegressionSpec, NonlinearFitSpec, ParameterRule, ParameterScope, ProjectDocument,
    SheetId,
};
use curvebook_fit::{CompiledDslModel, DataPoint, ParametricModel as _, extract_datasets_from_sheet, get_builtin_model};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::grid_view::parse_localized_number_input;
use crate::interaction::UiEventOutcome;

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

fn robust_or_rout(method: FitMethod) -> bool {
    matches!(method, FitMethod::RobustCauchy | FitMethod::RoutOutlierElimination { .. })
}

fn is_fixed_scope(scope: &ParameterScope) -> bool {
    matches!(
        scope,
        ParameterScope::Constant(_) | ParameterScope::ColumnTitleConstant | ParameterScope::HookedConstant(_) | ParameterScope::PerDatasetConstant(_)
    )
}

fn set_percent_clamped(level: &mut ConfidenceLevel, pct: f64) {
    if let Ok(next) = ConfidenceLevel::from_percent(pct.clamp(0.01, 99.99)) {
        *level = next;
    }
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `fit_params` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut FitParamsViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let Some(mut spec) = app.canvas_dialogs.edit_params.draft.clone() else {
        return Ok(UiEventOutcome::default());
    };

    let handled = match &mut spec.kind {
        AnalysisKind::NonlinearRegression(nl) => handle_nonlinear_action(app, state, nl, &mut spec.confidence_level, payload)?,
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
    Ok(UiEventOutcome { handled: true, action: format!("{DOMAIN}:{payload}"), command_id: None, command_result: None })
}

fn parse_finite_input(app: &UiApp, label: &str, raw: &str) -> Result<f64, String> {
    parse_localized_number_input(raw, &app.locale_preset).ok_or_else(|| format!("Enter a finite number for “{label}”."))
}

fn clear_param_text_buffers(app: &mut UiApp) {
    for key in ["fit.con.lo.input", "fit.con.hi.input", "fit.con.const.input", "fit.init.val.input"] {
        app.interaction.text_field_buffers.remove(key);
    }
}

fn handle_nonlinear_action(
    app: &mut UiApp,
    state: &FitParamsViewState,
    nl: &mut NonlinearFitSpec,
    confidence_level: &mut ConfidenceLevel,
    payload: &str,
) -> Result<bool, String> {
    if handle_constraints_and_init_action(app, state, nl, payload)? {
        return Ok(true);
    }
    if handle_confidence_action(app, nl, confidence_level, payload)? {
        return Ok(true);
    }
    handle_method_and_range_action(app, nl, payload)
}

fn handle_constraints_and_init_action(app: &mut UiApp, state: &FitParamsViewState, nl: &mut NonlinearFitSpec, payload: &str) -> Result<bool, String> {
    let params = model_param_names(nl);
    let Some(selected_name) = params.get(state.selected_param_idx.min(params.len().saturating_sub(1))).cloned() else {
        return Ok(false);
    };
    if let Some(scope_tag) = payload.strip_prefix("set_scope:") {
        let points = draft_points(app);
        let default_c = model_initial_guesses(nl, &points).get(state.selected_param_idx).copied().filter(|v| v.is_finite()).unwrap_or(1.0);
        let rule = ensure_rule_mut(nl, &selected_name);
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
        apply_selected_param_rule_inputs(app, nl, &selected_name)?;
        return Ok(true);
    }
    if payload == "apply_init_input" {
        let is_fixed = nl.parameter_rules.iter().find(|r| r.name == selected_name).is_some_and(|r| is_fixed_scope(&r.scope));
        if !is_fixed && let Some(buf) = app.interaction.text_field_buffers.get("fit.init.val.input") {
            let txt = buf.text.trim();
            let next_val =
                if txt.is_empty() || txt.eq_ignore_ascii_case("auto") { None } else { Some(parse_finite_input(app, "Initial value", txt)?) };
            ensure_rule_mut(nl, &selected_name).initial_override = next_val;
        }
        return Ok(true);
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

fn handle_confidence_action(app: &UiApp, nl: &mut NonlinearFitSpec, confidence_level: &mut ConfidenceLevel, payload: &str) -> Result<bool, String> {
    if let Some(mode) = payload.strip_prefix("ci_method:") {
        if mode == "profile" && robust_or_rout(nl.method) {
            return Err(crate::i18n::tr(&app.locale, "fit_params.dis.robust_profile_ci").to_owned());
        }
        nl.profile_likelihood_ci = mode == "profile";
        return Ok(true);
    }
    if payload == "apply_ci_level" {
        if let Some(buf) = app.interaction.text_field_buffers.get("fit.ci.lvl.input") {
            let pct = parse_finite_input(app, "Confidence level", buf.text.trim())?;
            set_percent_clamped(confidence_level, pct);
        }
        return Ok(true);
    }
    if payload == "toggle_emit_cov" {
        if matches!(nl.method, FitMethod::RobustCauchy) {
            return Err(String::from("Covariance matrix is unavailable for Robust nonlinear regression."));
        }
        nl.emit_covariance_tab = !nl.emit_covariance_tab;
        return Ok(true);
    }
    Ok(false)
}

fn handle_method_and_range_action(app: &UiApp, nl: &mut NonlinearFitSpec, payload: &str) -> Result<bool, String> {
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
        return Ok(true);
    }
    if payload == "apply_range" {
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
        return Ok(true);
    }
    Ok(false)
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
                app.view_shards.fit_params.active_fit_tab = String::from("diagnostics");
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
