//! Statistical, ANOVA/RM/REML, survival, contingency, ROC, and GLM multi-tab parameter modal (`R10.3`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ContingencyMethod, ContingencySpec, CoxTieMethod, DataSheet, FactorialAnovaSpec, GlmFamily, GlmSpec, KappaWeighting,
    MultiplicityMethod, MultivariateSpec, OneWayAnovaSpec, OneWayMethod, OutlierSpec, PValueFormat, PValueStackMethod, RepeatedMeasuresDesign,
    ResidualCovariance, RocCiMethod, RocSpec, SubcolumnAggregation, SumOfSquaresType, SurvivalAnalysisSpec, TableArchetype, TwoGroupMethod,
    TwoGroupSpec,
};
use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::dialogs_canvas::EditAnalysisParametersState;
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::paint::{PaintOp, PaintOpKind};
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{
    ButtonWidgetSpec, DropdownSpec, FittedTextSpec, SpinnerSpec, ToggleWidgetSpec, emit_button, emit_dropdown, emit_numeric_spinner,
    emit_scrollbars_for_meta, emit_tab, emit_toggle, paint_text_fitted, widget_state,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::{
    ControlEnablement, evaluate_dialog_widget, evaluate_edit_params_tab, r1_paired_or_matched_allowed, r1_sphericity_or_reml_control,
    r2_assume_gaussian_allowed, r2_parametric_post_hoc_method, r2_post_hoc_option_enabled, r3_control_column_picker, r4_two_group_graph_differences,
    r4_two_group_nonparametric_options, r4_two_group_welch_correction, r5_contingency_2x2_effect_sizes, r5_contingency_cochran_armitage_trend,
    r6_contingency_ci_selector, r7_correlation_heatmap_toggle, r8_subcolumn_aggregation, r9_pvalue_precision_spinner, source_table_shape,
};
use crate::text_bridge::UiTextStyle;

use super::a11y::labeled_meta;
use super::analyze_picker::scaled_u32;
use super::dialogs_graph_misc::emit_descriptor_controls_grid;
use super::fit_params::{build_fit_params_extra_section, is_fit_analysis_draft};

/// Transient view state owned by the statistical parameters modal shard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatParamsViewState {
    pub active_section: String,
    #[serde(default)]
    pub schematic_mode: String,
    #[serde(default = "default_true_opt")]
    pub diag_qq_plot: bool,
    #[serde(default = "default_true_opt")]
    pub diag_residuals_plot: bool,
    #[serde(default = "default_true_opt")]
    pub diag_homoscedasticity: bool,
    #[serde(default)]
    pub diag_influence_leverage: bool,
    #[serde(default = "default_true_opt")]
    pub diag_assumption_checklist: bool,
}

const fn default_true_opt() -> bool {
    true
}

impl Default for StatParamsViewState {
    fn default() -> Self {
        Self {
            active_section: String::new(),
            schematic_mode: String::from("auto"),
            diag_qq_plot: true,
            diag_residuals_plot: true,
            diag_homoscedasticity: true,
            diag_influence_leverage: false,
            diag_assumption_checklist: true,
        }
    }
}

fn label_i18n(locale: &str, key: &str, fallback: &'static str) -> &'static str {
    crate::i18n::try_tr(locale, key).unwrap_or(fallback)
}

fn custom_action(payload: impl Into<String>) -> WidgetAction {
    WidgetAction::Custom { domain: String::from("stat_params"), payload: payload.into() }
}

// ============================================================================
// Custom Action Dispatcher
// ============================================================================

/// Dispatch custom `WidgetAction::Custom` events targeting the `stat_params` domain.
///
/// # Errors
/// Returns an error if `payload` references an invalid parameter or violates an `R1`–`R9` predicate.
pub fn handle_custom_action(app: &mut UiApp, state: &mut StatParamsViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if payload == "smoke" {
        return Ok(UiEventOutcome::default());
    }
    let handled = match payload {
        "select_all_datasets" => handle_dataset_select_all(app),
        "schematic:toggle" => handle_schematic_toggle(app, state)?,
        "toggle_geisser_greenhouse" => handle_toggle_gg(app)?,
        "toggle_effect_sizes" => handle_toggle_effect_sizes(app),
        "toggle_fit_reml_on_missing" => handle_toggle_reml_missing(app)?,
        "toggle_three_way" => handle_toggle_three_way(app),
        "toggle_two_tailed" => handle_toggle_two_tailed(app),
        "toggle_mult_report_ci" => handle_toggle_mult_ci(app),
        "cycle_control_column" => handle_cycle_control_col(app)?,
        "toggle_pca_std" => handle_toggle_pca_std(app),
        "toggle_corr_spearman" => handle_toggle_corr_spearman(app),
        "toggle_corr_all_pairs" => handle_toggle_corr_all_pairs(app),
        "toggle_corr_heatmap" => handle_toggle_corr_heatmap(app)?,
        "toggle_desc_normality" => handle_toggle_desc_normality(app),
        "toggle_grubbs_iterative" => handle_toggle_grubbs_iterative(app),
        "toggle_ba_percent" => handle_toggle_ba_percent(app),
        "toggle_glm_vif" => handle_toggle_glm_vif(app),
        _ => handle_prefixed_custom_action(app, state, payload)?,
    };
    Ok(UiEventOutcome { handled, action: format!("stat_params:{payload}"), ..Default::default() })
}

fn handle_prefixed_custom_action(app: &mut UiApp, state: &mut StatParamsViewState, payload: &str) -> Result<bool, String> {
    if let Some(rest) = payload.strip_prefix("tab:") {
        let mut map = serde_json::Map::new();
        map.insert(String::from("active_tab"), serde_json::Value::String(rest.to_owned()));
        app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
        state.active_section = rest.to_owned();
        return Ok(true);
    }
    if let Some(rest) = payload.strip_prefix("toggle_dataset:") {
        let idx = rest.parse::<usize>().map_err(|_| format!("invalid dataset index `{rest}`"))?;
        toggle_dataset_index(app, idx);
        return Ok(true);
    }
    if let Some(rest) = payload.strip_prefix("set_one_way_method:") {
        return set_one_way_method(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_rm_design:") {
        return set_factorial_rm_design(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_residual_cov:") {
        return set_factorial_residual_cov(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_ss_type:") {
        return set_factorial_ss_type(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_two_group_method:") {
        return set_two_group_method(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("toggle_two_group_flag:") {
        return toggle_two_group_flag(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_multiplicity:") {
        return set_multiplicity_method(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_pvalue_format:") {
        return set_pvalue_format(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_confidence:") {
        let pct = rest.parse::<f64>().map_err(|_| format!("invalid confidence `{rest}`"))?;
        app.canvas_dialogs.edit_params.set_confidence_percent(pct)?;
        return Ok(true);
    }
    if let Some(rest) = payload.strip_prefix("set_nested_mode:") {
        return set_nested_mode(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("toggle_mrow:") {
        return toggle_mrow_flag(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_desc_agg:") {
        return set_desc_aggregation(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_outlier_mode:") {
        return set_outlier_mode(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_contingency_method:") {
        return set_contingency_method(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_kappa_weighting:") {
        return set_kappa_weighting(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("toggle_contingency_flag:") {
        return toggle_contingency_flag(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("cycle_contingency_ci:") {
        return cycle_contingency_ci(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("toggle_km_flag:") {
        return toggle_km_flag(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("cycle_km_col:") {
        return cycle_km_column(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("cycle_cox_col:") {
        return cycle_cox_column(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("toggle_cox_pred:") {
        let idx = rest.parse::<usize>().map_err(|_| format!("invalid predictor index `{rest}`"))?;
        return toggle_cox_predictor(app, idx);
    }
    if let Some(rest) = payload.strip_prefix("set_cox_tie:") {
        return set_cox_tie_method(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_roc_ci:") {
        return set_roc_ci_method(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_glm_family:") {
        return set_glm_family(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_glm_resp:") {
        let idx = rest.parse::<usize>().map_err(|_| format!("invalid response index `{rest}`"))?;
        return set_glm_response(app, idx);
    }
    if let Some(rest) = payload.strip_prefix("toggle_glm_pred:") {
        let idx = rest.parse::<usize>().map_err(|_| format!("invalid predictor index `{rest}`"))?;
        return toggle_glm_predictor(app, idx);
    }
    if let Some(rest) = payload.strip_prefix("set_fac_resp:") {
        let idx = rest.parse::<usize>().map_err(|_| format!("invalid response index `{rest}`"))?;
        return set_factorial_response(app, idx);
    }
    if let Some(rest) = payload.strip_prefix("toggle_fac_col:") {
        let idx = rest.parse::<usize>().map_err(|_| format!("invalid factor index `{rest}`"))?;
        return toggle_factorial_col(app, idx);
    }
    if let Some(rest) = payload.strip_prefix("set_cluster_dist:") {
        return set_cluster_distance(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_cluster_link:") {
        return set_cluster_linkage(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_pstack_method:") {
        return set_pvalue_stack_method(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("cycle_pstack_col") {
        let _ = rest;
        return cycle_pvalue_stack_col(app);
    }
    if let Some(rest) = payload.strip_prefix("cycle_ba_col:") {
        return cycle_ba_column(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("toggle_diag:") {
        return Ok(toggle_diag_flag(app, state, rest));
    }
    if let Some(rest) = payload.strip_prefix("spinner:") {
        return handle_spinner_step(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("schematic:") {
        state.schematic_mode = rest.to_owned();
        return Ok(true);
    }
    Ok(false)
}

fn source_sheet_col_count(app: &UiApp) -> usize {
    let doc = app.document();
    let Some(src_id) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id) else {
        return 0;
    };
    doc.data_sheet(src_id).or_else(|| doc.result_sheet(src_id).and_then(|rs| rs.analyzable_data.as_ref())).map_or(0, |ds| ds.archetype.column_count())
}

fn handle_dataset_select_all(app: &mut UiApp) -> bool {
    if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft {
        spec.selected_data_sets.clear();
        return true;
    }
    false
}

fn toggle_dataset_index(app: &mut UiApp, idx: usize) {
    let total = source_sheet_col_count(app).max(idx + 1);
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return;
    };
    if spec.selected_data_sets.is_empty() {
        spec.selected_data_sets = (0..total).filter(|&i| i != idx).collect();
    } else if let Some(pos) = spec.selected_data_sets.iter().position(|&i| i == idx) {
        if spec.selected_data_sets.len() > 1 {
            spec.selected_data_sets.remove(pos);
        }
    } else {
        spec.selected_data_sets.push(idx);
        spec.selected_data_sets.sort_unstable();
        if spec.selected_data_sets.len() == total {
            spec.selected_data_sets.clear();
        }
    }
}

fn handle_schematic_toggle(app: &mut UiApp, state: &mut StatParamsViewState) -> Result<bool, String> {
    let Some(kind) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| d.kind.clone()) else {
        return Ok(false);
    };
    match kind {
        AnalysisKind::OneWayAnova(_) | AnalysisKind::TwoGroupComparison(_) => {
            let next = !app.canvas_dialogs.edit_params.paired_or_matched;
            let src = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id);
            if next && !r1_paired_or_matched_allowed(app.document(), src).enabled {
                state.schematic_mode = if state.schematic_mode == "detail" { String::from("auto") } else { String::from("detail") };
                return Ok(true);
            }
            let mut map = serde_json::Map::new();
            map.insert(String::from("paired_or_matched"), serde_json::Value::Bool(next));
            app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
            Ok(true)
        }
        AnalysisKind::FactorialAnova(f) => {
            if f.repeated_measures != RepeatedMeasuresDesign::None {
                let next = if f.residual_covariance == ResidualCovariance::CompoundSymmetry { "unstructured" } else { "compound_symmetry" };
                return set_factorial_residual_cov(app, next);
            }
            let mut map = serde_json::Map::new();
            map.insert(String::from("paired_or_matched"), serde_json::Value::Bool(true));
            app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
            Ok(true)
        }
        _ => {
            state.schematic_mode = if state.schematic_mode == "detail" { String::from("auto") } else { String::from("detail") };
            Ok(true)
        }
    }
}

fn set_one_way_method(app: &mut UiApp, method_name: &str) -> Result<bool, String> {
    let ep = &app.canvas_dialogs.edit_params;
    let src = ep.draft.as_ref().map(|d| &d.source_sheet_id);
    let wants_nonparam = matches!(method_name, "kruskal" | "friedman");
    let wants_paired = matches!(method_name, "rm" | "friedman");
    if wants_nonparam {
        let g = r2_assume_gaussian_allowed(app.document(), src);
        if !g.enabled {
            return Err(g.disabled_reason.unwrap_or_else(|| String::from("Nonparametric tests unavailable")));
        }
    }
    if wants_paired {
        let p = r1_paired_or_matched_allowed(app.document(), src);
        if !p.enabled {
            return Err(p.disabled_reason.unwrap_or_else(|| String::from("Repeated measures unavailable")));
        }
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("anova_method"), serde_json::Value::String(method_name.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    if wants_nonparam && let Some(spec) = &mut app.canvas_dialogs.edit_params.draft {
        let mut kind = spec.kind.clone();
        if let AnalysisKind::OneWayAnova(ref mut a) = kind
            && !matches!(a.multiplicity.method, MultiplicityMethod::None | MultiplicityMethod::DunnRankSum)
        {
            a.multiplicity.method = MultiplicityMethod::DunnRankSum;
            spec.update_kind(kind);
        }
    } else if !wants_nonparam && let Some(spec) = &mut app.canvas_dialogs.edit_params.draft {
        let mut kind = spec.kind.clone();
        if let AnalysisKind::OneWayAnova(ref mut a) = kind
            && a.multiplicity.method == MultiplicityMethod::DunnRankSum
        {
            a.multiplicity.method = MultiplicityMethod::TukeyHsd;
            spec.update_kind(kind);
        }
    }
    Ok(true)
}

fn handle_toggle_gg(app: &mut UiApp) -> Result<bool, String> {
    let ep = &app.canvas_dialogs.edit_params;
    if !ep.paired_or_matched {
        return Err(r1_sphericity_or_reml_control(false).disabled_reason.unwrap_or_default());
    }
    let cur = ep.draft.as_ref().is_none_or(
        |d| !matches!(d.kind, AnalysisKind::OneWayAnova(ref a) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })),
    );
    let mut map = serde_json::Map::new();
    map.insert(String::from("geisser_greenhouse"), serde_json::Value::Bool(!cur));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn handle_toggle_effect_sizes(app: &mut UiApp) -> bool {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return false;
    };
    let mut updated = spec.kind.clone();
    let changed = match &mut updated {
        AnalysisKind::OneWayAnova(a) => {
            a.compute_effect_sizes = !a.compute_effect_sizes;
            true
        }
        AnalysisKind::FactorialAnova(f) => {
            f.compute_effect_sizes = !f.compute_effect_sizes;
            true
        }
        AnalysisKind::TwoGroupComparison(t) => {
            t.compute_effect_sizes = !t.compute_effect_sizes;
            true
        }
        _ => false,
    };
    if changed {
        spec.update_kind(updated);
    }
    changed
}

fn set_factorial_rm_design(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let mut map = serde_json::Map::new();
    map.insert(String::from("repeated_measures"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    app.canvas_dialogs.edit_params.sync_rule_flags_from_kind();
    Ok(true)
}

fn set_factorial_residual_cov(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    if !app.canvas_dialogs.edit_params.paired_or_matched {
        return Err(r1_sphericity_or_reml_control(false).disabled_reason.unwrap_or_default());
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("residual_covariance"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_factorial_ss_type(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let mut map = serde_json::Map::new();
    map.insert(String::from("ss_type"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn handle_toggle_reml_missing(app: &mut UiApp) -> Result<bool, String> {
    if !app.canvas_dialogs.edit_params.paired_or_matched {
        return Err(r1_sphericity_or_reml_control(false).disabled_reason.unwrap_or_default());
    }
    let cur = matches!(
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::FactorialAnova(f)) if f.fit_reml_on_missing
    );
    let mut map = serde_json::Map::new();
    map.insert(String::from("fit_reml_on_missing"), serde_json::Value::Bool(!cur));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn handle_toggle_three_way(app: &mut UiApp) -> bool {
    let cur = matches!(
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::FactorialAnova(f)) if f.three_way
    );
    let mut map = serde_json::Map::new();
    map.insert(String::from("three_way"), serde_json::Value::Bool(!cur));
    app.canvas_dialogs.edit_params.apply_field_patch(&map).is_ok()
}

fn set_factorial_response(app: &mut UiApp, idx: usize) -> Result<bool, String> {
    let mut map = serde_json::Map::new();
    map.insert(String::from("response_column"), serde_json::json!(idx));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn toggle_factorial_col(app: &mut UiApp, idx: usize) -> Result<bool, String> {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return Ok(false);
    };
    let mut updated = spec.kind.clone();
    if let AnalysisKind::FactorialAnova(ref mut f) = updated {
        if let Some(pos) = f.multifactor_columns.iter().position(|&c| c == idx) {
            if f.multifactor_columns.len() > 1 {
                f.multifactor_columns.remove(pos);
            }
        } else {
            f.multifactor_columns.push(idx);
            f.multifactor_columns.sort_unstable();
        }
        spec.update_kind(updated);
        return Ok(true);
    }
    Ok(false)
}

fn set_two_group_method(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let src = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id);
    let wants_paired = matches!(raw, "paired" | "ratio_paired" | "wilcoxon" | "paired_sign_test");
    let wants_nonparam = matches!(raw, "mann_whitney" | "wilcoxon" | "ks" | "one_sample_wilcoxon" | "paired_sign_test");
    if wants_paired {
        let p = r1_paired_or_matched_allowed(app.document(), src);
        if !p.enabled {
            return Err(p.disabled_reason.unwrap_or_default());
        }
    }
    if wants_nonparam {
        let g = r2_assume_gaussian_allowed(app.document(), src);
        if !g.enabled {
            return Err(g.disabled_reason.unwrap_or_default());
        }
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("two_group_method"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn handle_toggle_two_tailed(app: &mut UiApp) -> bool {
    let cur = matches!(
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::TwoGroupComparison(t)) if t.two_tailed
    );
    let mut map = serde_json::Map::new();
    map.insert(String::from("two_tailed"), serde_json::Value::Bool(!cur));
    app.canvas_dialogs.edit_params.apply_field_patch(&map).is_ok()
}

fn toggle_two_group_flag(app: &mut UiApp, flag: &str) -> Result<bool, String> {
    let ep = &app.canvas_dialogs.edit_params;
    let (key, next) = match flag {
        "graph_differences" => ("graph_differences", !ep.graph_differences),
        "graph_ranks" => ("graph_ranks", !ep.graph_ranks),
        "hodges_lehmann" => ("hodges_lehmann_ci", !ep.hodges_lehmann_ci),
        "welch" => ("welch_correction", !ep.welch_correction),
        _ => return Ok(false),
    };
    let mut map = serde_json::Map::new();
    map.insert(key.to_owned(), serde_json::Value::Bool(next));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_multiplicity_method(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let ep = &app.canvas_dialogs.edit_params;
    if matches!(ep.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::MultipleRowTTests(_))) {
        let method = parse_mult_method(raw, ep.control_column)?;
        let nonparam = matches!(ep.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::MultipleRowTTests(m)) if m.nonparametric);
        if !r2_parametric_post_hoc_method(!nonparam, method).enabled {
            return Err(String::from("Parametric multiplicity disabled in nonparametric mode."));
        }
        if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft
            && let AnalysisKind::MultipleRowTTests(mut mr) = spec.kind.clone()
        {
            mr.multiplicity.method = method;
            spec.update_kind(AnalysisKind::MultipleRowTTests(mr));
        }
        return Ok(true);
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("multiplicity_method"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn handle_toggle_mult_ci(app: &mut UiApp) -> bool {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return false;
    };
    let mut updated = spec.kind.clone();
    let changed = match &mut updated {
        AnalysisKind::OneWayAnova(a) => {
            a.multiplicity.report_confidence_intervals = !a.multiplicity.report_confidence_intervals;
            true
        }
        AnalysisKind::FactorialAnova(f) => {
            f.multiplicity.report_confidence_intervals = !f.multiplicity.report_confidence_intervals;
            true
        }
        AnalysisKind::NestedComparison(n) => {
            n.multiplicity.report_confidence_intervals = !n.multiplicity.report_confidence_intervals;
            true
        }
        AnalysisKind::MultipleRowTTests(m) => {
            m.multiplicity.report_confidence_intervals = !m.multiplicity.report_confidence_intervals;
            true
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => {
            multiplicity.report_confidence_intervals = !multiplicity.report_confidence_intervals;
            true
        }
        _ => false,
    };
    if changed {
        spec.update_kind(updated);
    }
    changed
}

fn handle_cycle_control_col(app: &mut UiApp) -> Result<bool, String> {
    let method = current_multiplicity(&app.canvas_dialogs.edit_params).map_or(MultiplicityMethod::None, |m| m.method);
    let gate = r3_control_column_picker(app.canvas_dialogs.edit_params.compare_against_control, method);
    if !gate.enabled {
        return Err(gate.disabled_reason.unwrap_or_default());
    }
    let cols = source_sheet_col_count(app).max(2);
    let next = (app.canvas_dialogs.edit_params.control_column + 1) % cols;
    let mut map = serde_json::Map::new();
    map.insert(String::from("control_column"), serde_json::json!(next));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_pvalue_format(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let mut map = serde_json::Map::new();
    map.insert(String::from("p_value_format"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_nested_mode(app: &mut UiApp, mode: &str) -> Result<bool, String> {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return Ok(false);
    };
    if let AnalysisKind::NestedComparison(mut n) = spec.kind.clone() {
        n.two_group_t_test = mode == "t_test";
        spec.update_kind(AnalysisKind::NestedComparison(n));
        return Ok(true);
    }
    Ok(false)
}

fn toggle_mrow_flag(app: &mut UiApp, flag: &str) -> Result<bool, String> {
    let src = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id);
    if flag == "nonparametric" {
        let g = r2_assume_gaussian_allowed(app.document(), src);
        if !g.enabled {
            return Err(g.disabled_reason.unwrap_or_default());
        }
    }
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return Ok(false);
    };
    if let AnalysisKind::MultipleRowTTests(mut m) = spec.kind.clone() {
        match flag {
            "paired" => m.paired = !m.paired,
            "nonparametric" => m.nonparametric = !m.nonparametric,
            "equal_sd" => m.assume_equal_sd = !m.assume_equal_sd,
            _ => return Ok(false),
        }
        spec.update_kind(AnalysisKind::MultipleRowTTests(m));
        return Ok(true);
    }
    Ok(false)
}

fn set_desc_aggregation(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let doc = app.document();
    let src = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id);
    let (_, _, max_sub) = source_table_shape(doc, src);
    let gate = r8_subcolumn_aggregation(max_sub);
    if !gate.enabled {
        return Err(gate.disabled_reason.unwrap_or_default());
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("subcolumn_agg"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn handle_toggle_desc_normality(app: &mut UiApp) -> bool {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return false;
    };
    if let AnalysisKind::DescriptiveStats(mut d) = spec.kind.clone() {
        d.run_normality_battery = !d.run_normality_battery;
        spec.update_kind(AnalysisKind::DescriptiveStats(d));
        return true;
    }
    false
}

fn set_outlier_mode(app: &mut UiApp, mode: &str) -> Result<bool, String> {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return Ok(false);
    };
    let next = match mode {
        "rout" => OutlierSpec::Rout { fdr_q_percent: 1.0 },
        "grubbs" => OutlierSpec::Grubbs { alpha: 0.05, iterative: true },
        _ => return Ok(false),
    };
    spec.update_kind(AnalysisKind::OutlierDetection(next));
    Ok(true)
}

fn handle_toggle_grubbs_iterative(app: &mut UiApp) -> bool {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return false;
    };
    if let AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative }) = spec.kind {
        spec.update_kind(AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative: !iterative }));
        return true;
    }
    false
}

fn set_contingency_method(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let doc = app.document();
    let src = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id);
    let (rows, cols, _) = source_table_shape(doc, src);
    if matches!(raw, "fisher" | "barnard" | "mcnemar") {
        let g = r5_contingency_2x2_effect_sizes(rows, cols);
        if !g.enabled {
            return Err(g.disabled_reason.unwrap_or_default());
        }
    } else if raw == "cochran_armitage" {
        let g = r5_contingency_cochran_armitage_trend(rows, cols);
        if !g.enabled {
            return Err(g.disabled_reason.unwrap_or_default());
        }
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("contingency_method"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_kappa_weighting(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let mut map = serde_json::Map::new();
    map.insert(String::from("kappa_weighting"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn toggle_contingency_flag(app: &mut UiApp, flag: &str) -> Result<bool, String> {
    let doc = app.document();
    let src = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id);
    let (rows, cols, _) = source_table_shape(doc, src);
    let gate = r5_contingency_2x2_effect_sizes(rows, cols);
    if flag != "phi" && !gate.enabled {
        return Err(gate.disabled_reason.unwrap_or_default());
    }
    let ep = &app.canvas_dialogs.edit_params;
    let cont = match ep.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::Contingency(c)) => c.clone(),
        _ => ContingencySpec::default(),
    };
    let mut map = serde_json::Map::new();
    match flag {
        "or" => {
            map.insert(String::from("compute_odds_and_risk_ratios"), serde_json::Value::Bool(!ep.contingency_odds_ratio_checked));
        }
        "arr" => {
            map.insert(String::from("attributable_risk"), serde_json::Value::Bool(!ep.contingency_attributable_risk_checked));
        }
        "sens" => {
            map.insert(String::from("sensitivity_specificity"), serde_json::Value::Bool(!cont.compute_sensitivity_specificity));
        }
        "phi" => {
            if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft
                && let AnalysisKind::Contingency(mut c) = spec.kind.clone()
            {
                c.compute_phi_and_cramers_v = !c.compute_phi_and_cramers_v;
                spec.update_kind(AnalysisKind::Contingency(c));
            }
            return Ok(true);
        }
        _ => return Ok(false),
    }
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn cycle_contingency_ci(app: &mut UiApp, which: &str) -> Result<bool, String> {
    let doc = app.document();
    let ep = &app.canvas_dialogs.edit_params;
    let (rows, cols, _) = source_table_shape(doc, ep.draft.as_ref().map(|d| &d.source_sheet_id));
    let cont = match ep.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::Contingency(c)) => c.clone(),
        _ => ContingencySpec::default(),
    };
    let (wid_key, checked, label, next_val) = match which {
        "or" => (
            "edit.contingency.ci.baptista_pike",
            ep.contingency_odds_ratio_checked || cont.compute_odds_and_risk_ratios,
            "Baptista-Pike",
            if ep.contingency_or_ci_method == "Baptista-Pike" { "Woolf" } else { "Baptista-Pike" },
        ),
        "rr" => (
            "edit.contingency.ci.koopman",
            ep.contingency_relative_risk_checked || cont.compute_odds_and_risk_ratios,
            "Koopman",
            if ep.contingency_rr_ci_method == "Koopman" { "Katz" } else { "Koopman" },
        ),
        "arr" => (
            "edit.contingency.ci.newcombe_wilson",
            ep.contingency_attributable_risk_checked,
            "Newcombe-Wilson",
            if ep.contingency_arr_ci_method == "Newcombe-Wilson" { "Wald" } else { "Newcombe-Wilson" },
        ),
        "diag" => (
            "edit.contingency.ci.wilson_brown",
            cont.compute_sensitivity_specificity,
            "Wilson-Brown",
            if ep.contingency_diag_ci_method == "Wilson-Brown" { "Clopper-Pearson" } else { "Wilson-Brown" },
        ),
        _ => return Ok(false),
    };
    let gate = r6_contingency_ci_selector(rows, cols, checked, label);
    if !gate.enabled {
        return Err(gate.disabled_reason.unwrap_or_default());
    }
    let mut map = serde_json::Map::new();
    map.insert(wid_key.to_owned(), serde_json::Value::String(next_val.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn toggle_km_flag(app: &mut UiApp, flag: &str) -> Result<bool, String> {
    let Some(AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
        log_rank_mantel_cox,
        gehan_breslow_wilcoxon,
        trend_test,
        asymmetrical_greenwood_ci,
        ..
    })) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind)
    else {
        return Ok(false);
    };
    let (key, next) = match flag {
        "log_rank" => ("log_rank_mantel_cox", !log_rank_mantel_cox),
        "gehan" => ("gehan_breslow_wilcoxon", !gehan_breslow_wilcoxon),
        "trend" => ("trend_test", !trend_test),
        "greenwood" => ("asymmetrical_greenwood_ci", !asymmetrical_greenwood_ci),
        _ => return Ok(false),
    };
    let mut map = serde_json::Map::new();
    map.insert(key.to_owned(), serde_json::Value::Bool(next));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn cycle_km_column(app: &mut UiApp, which: &str) -> Result<bool, String> {
    let cols = source_sheet_col_count(app).max(1);
    let Some(AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column, mv_status_column, mv_group_column, .. })) =
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind)
    else {
        return Ok(false);
    };
    let (key, cur) = match which {
        "time" => ("mv_time_column", *mv_time_column),
        "status" => ("mv_status_column", *mv_status_column),
        "group" => ("mv_group_column", *mv_group_column),
        _ => return Ok(false),
    };
    let next = match cur {
        None => Some(0),
        Some(c) if c + 1 < cols => Some(c + 1),
        Some(_) => None,
    };
    let mut map = serde_json::Map::new();
    map.insert(key.to_owned(), next.map_or(serde_json::Value::Null, |v| serde_json::json!(v)));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn cycle_cox_column(app: &mut UiApp, which: &str) -> Result<bool, String> {
    let cols = source_sheet_col_count(app).max(1);
    let Some(AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, .. })) =
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind)
    else {
        return Ok(false);
    };
    let (key, cur) = match which {
        "time" => ("time_column", *time_column),
        "status" => ("status_column", *status_column),
        _ => return Ok(false),
    };
    let mut map = serde_json::Map::new();
    map.insert(key.to_owned(), serde_json::json!((cur + 1) % cols));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn toggle_cox_predictor(app: &mut UiApp, idx: usize) -> Result<bool, String> {
    let Some(AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { predictor_columns, .. })) =
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind)
    else {
        return Ok(false);
    };
    let mut next = predictor_columns.clone();
    if let Some(pos) = next.iter().position(|&c| c == idx) {
        if next.len() > 1 {
            next.remove(pos);
        }
    } else {
        next.push(idx);
        next.sort_unstable();
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("predictor_columns"), serde_json::json!(next));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_cox_tie_method(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let mut map = serde_json::Map::new();
    map.insert(String::from("tie_method"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_roc_ci_method(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let ci_method = match raw {
        "delong" => RocCiMethod::DeLong,
        "hanley_mcneil" => RocCiMethod::HanleyMcNeil,
        _ => return Ok(false),
    };
    if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft {
        spec.update_kind(AnalysisKind::RocCurve(RocSpec { ci_method }));
        return Ok(true);
    }
    Ok(false)
}

fn set_glm_family(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let mut map = serde_json::Map::new();
    map.insert(String::from("family"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_glm_response(app: &mut UiApp, idx: usize) -> Result<bool, String> {
    let mut map = serde_json::Map::new();
    map.insert(String::from("response_column"), serde_json::json!(idx));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn toggle_glm_predictor(app: &mut UiApp, idx: usize) -> Result<bool, String> {
    let Some(AnalysisKind::GeneralizedLinearModel(glm)) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) else {
        return Ok(false);
    };
    let mut next = glm.predictor_columns.clone();
    if let Some(pos) = next.iter().position(|&c| c == idx) {
        if next.len() > 1 {
            next.remove(pos);
        }
    } else {
        next.push(idx);
        next.sort_unstable();
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("predictor_columns"), serde_json::json!(next));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn handle_toggle_glm_vif(app: &mut UiApp) -> bool {
    let cur = matches!(
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::GeneralizedLinearModel(g)) if g.compute_vif
    );
    let mut map = serde_json::Map::new();
    map.insert(String::from("compute_vif"), serde_json::Value::Bool(!cur));
    app.canvas_dialogs.edit_params.apply_field_patch(&map).is_ok()
}

fn handle_toggle_pca_std(app: &mut UiApp) -> bool {
    let cur = matches!(
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, .. }))
    );
    let mut map = serde_json::Map::new();
    map.insert(String::from("standardize"), serde_json::Value::Bool(!cur));
    app.canvas_dialogs.edit_params.apply_field_patch(&map).is_ok()
}

fn handle_toggle_corr_spearman(app: &mut UiApp) -> bool {
    let cur = matches!(
        app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: true }))
    );
    let mut map = serde_json::Map::new();
    map.insert(String::from("spearman_rank"), serde_json::Value::Bool(!cur));
    app.canvas_dialogs.edit_params.apply_field_patch(&map).is_ok()
}

fn handle_toggle_corr_all_pairs(app: &mut UiApp) -> bool {
    let next = !app.canvas_dialogs.edit_params.correlation_all_pairs;
    let mut map = serde_json::Map::new();
    map.insert(String::from("correlation_all_pairs"), serde_json::Value::Bool(next));
    app.canvas_dialogs.edit_params.apply_field_patch(&map).is_ok()
}

fn handle_toggle_corr_heatmap(app: &mut UiApp) -> Result<bool, String> {
    let gate = r7_correlation_heatmap_toggle(app.canvas_dialogs.edit_params.correlation_all_pairs);
    if !gate.enabled {
        return Err(gate.disabled_reason.unwrap_or_default());
    }
    let next = !app.canvas_dialogs.edit_params.correlation_heatmap;
    let mut map = serde_json::Map::new();
    map.insert(String::from("correlation_heatmap"), serde_json::Value::Bool(next));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_cluster_distance(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let mut map = serde_json::Map::new();
    map.insert(String::from("distance"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_cluster_linkage(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let mut map = serde_json::Map::new();
    map.insert(String::from("linkage"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_pvalue_stack_method(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let mut map = serde_json::Map::new();
    map.insert(String::from("pvalue_stack_method"), serde_json::Value::String(raw.to_owned()));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn cycle_pvalue_stack_col(app: &mut UiApp) -> Result<bool, String> {
    let cols = source_sheet_col_count(app).max(1);
    let Some(AnalysisKind::PValueStack(ps)) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) else {
        return Ok(false);
    };
    let next = match ps.column {
        None => Some(0),
        Some(c) if c + 1 < cols => Some(c + 1),
        Some(_) => None,
    };
    let mut map = serde_json::Map::new();
    map.insert(String::from("pvalue_stack_column"), next.map_or(serde_json::Value::Null, |v| serde_json::json!(v)));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn cycle_ba_column(app: &mut UiApp, which: &str) -> Result<bool, String> {
    let cols = source_sheet_col_count(app).max(2);
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return Ok(false);
    };
    if let AnalysisKind::BlandAltman(mut ba) = spec.kind {
        match which {
            "a" => ba.column_a = (ba.column_a + 1) % cols,
            "b" => ba.column_b = (ba.column_b + 1) % cols,
            _ => return Ok(false),
        }
        spec.update_kind(AnalysisKind::BlandAltman(ba));
        return Ok(true);
    }
    Ok(false)
}

fn handle_toggle_ba_percent(app: &mut UiApp) -> bool {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return false;
    };
    if let AnalysisKind::BlandAltman(mut ba) = spec.kind {
        ba.percent_difference = !ba.percent_difference;
        spec.update_kind(AnalysisKind::BlandAltman(ba));
        return true;
    }
    false
}

fn toggle_diag_flag(app: &mut UiApp, state: &mut StatParamsViewState, flag: &str) -> bool {
    match flag {
        "qq" => {
            state.diag_qq_plot = !state.diag_qq_plot;
            true
        }
        "residuals" => {
            state.diag_residuals_plot = !state.diag_residuals_plot;
            true
        }
        "homoscedasticity" => {
            state.diag_homoscedasticity = !state.diag_homoscedasticity;
            true
        }
        "influence" => {
            state.diag_influence_leverage = !state.diag_influence_leverage;
            if matches!(app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::GeneralizedLinearModel(_))) {
                let _ = handle_toggle_glm_vif(app);
            }
            true
        }
        "checklist" => {
            state.diag_assumption_checklist = !state.diag_assumption_checklist;
            true
        }
        _ => false,
    }
}

fn handle_spinner_step(app: &mut UiApp, rest: &str) -> Result<bool, String> {
    let Some((prefix, delta_str)) = rest.rsplit_once(':') else {
        return Ok(false);
    };
    let delta = delta_str.parse::<f64>().map_err(|_| format!("invalid spinner delta `{delta_str}`"))?;
    match prefix {
        "edit.pvalue.digits" => {
            let Some(spec) = &app.canvas_dialogs.edit_params.draft else {
                return Ok(false);
            };
            let gate = r9_pvalue_precision_spinner(&spec.p_value_format);
            if !gate.enabled {
                return Err(gate.disabled_reason.unwrap_or_default());
            }
            let cur = match spec.p_value_format {
                PValueFormat::Custom { decimals, .. } => i32::from(decimals),
                _ => 4,
            };
            let next = (cur + delta.round() as i32).clamp(1, 9);
            let mut map = serde_json::Map::new();
            map.insert(String::from("p_value_decimals"), serde_json::json!(next));
            app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
            Ok(true)
        }
        "edit.mult_threshold" => step_multiplicity_threshold(app, delta),
        "edit.factorial.max_interaction_order" => {
            let cur = match app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
                Some(AnalysisKind::FactorialAnova(f)) => f.max_interaction_order,
                _ => 2,
            };
            let next = if delta >= 0.0 { (cur + 1).min(3) } else { cur.saturating_sub(1).max(1) };
            let mut map = serde_json::Map::new();
            map.insert(String::from("max_interaction_order"), serde_json::json!(next));
            app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
            Ok(true)
        }
        "edit.contingency.null_proportion" => {
            let cur = match app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
                Some(AnalysisKind::Contingency(c)) => c.null_proportion.unwrap_or(0.5),
                _ => 0.5,
            };
            let next = (cur + delta).clamp(0.05, 0.95);
            let mut map = serde_json::Map::new();
            map.insert(String::from("null_proportion"), serde_json::json!(next));
            app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
            Ok(true)
        }
        "edit.cluster.k_means_k" => {
            let cur = match app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
                Some(AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k, .. })) => k_means_k.unwrap_or(2),
                _ => 2,
            };
            let next = if delta >= 0.0 { (cur + 1).min(12) } else { cur.saturating_sub(1).max(2) };
            let mut map = serde_json::Map::new();
            map.insert(String::from("k_means_k"), serde_json::json!(next));
            app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
            Ok(true)
        }
        "edit.imputation.m" => {
            let cur = match app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
                Some(AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { imputations_m, .. })) => *imputations_m,
                _ => 5,
            };
            let next = if delta >= 0.0 { (cur + 1).min(50) } else { cur.saturating_sub(1).max(1) };
            let mut map = serde_json::Map::new();
            map.insert(String::from("imputations_m"), serde_json::json!(next));
            app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
            Ok(true)
        }
        "edit.two_group.hypothetical" => step_hypothetical_value(app, delta),
        "edit.outlier.param" => step_outlier_param(app, delta),
        _ => Ok(false),
    }
}

fn step_multiplicity_threshold(app: &mut UiApp, delta: f64) -> Result<bool, String> {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return Ok(false);
    };
    let mut updated = spec.kind.clone();
    let mut changed = false;
    match &mut updated {
        AnalysisKind::OneWayAnova(a) => {
            a.multiplicity.threshold = (a.multiplicity.threshold + delta).clamp(0.001, 0.50);
            changed = true;
        }
        AnalysisKind::FactorialAnova(f) => {
            f.multiplicity.threshold = (f.multiplicity.threshold + delta).clamp(0.001, 0.50);
            changed = true;
        }
        AnalysisKind::NestedComparison(n) => {
            n.multiplicity.threshold = (n.multiplicity.threshold + delta).clamp(0.001, 0.50);
            changed = true;
        }
        AnalysisKind::MultipleRowTTests(m) => {
            m.multiplicity.threshold = (m.multiplicity.threshold + delta).clamp(0.001, 0.50);
            changed = true;
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => {
            multiplicity.threshold = (multiplicity.threshold + delta).clamp(0.001, 0.50);
            changed = true;
        }
        AnalysisKind::PValueStack(ps) => {
            ps.threshold = (ps.threshold + delta).clamp(0.001, 0.50);
            changed = true;
        }
        _ => {}
    }
    if changed {
        spec.update_kind(updated);
    }
    Ok(changed)
}

fn step_hypothetical_value(app: &mut UiApp, delta: f64) -> Result<bool, String> {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return Ok(false);
    };
    let mut updated = spec.kind.clone();
    let changed = match &mut updated {
        AnalysisKind::TwoGroupComparison(t) => match &mut t.method {
            TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } => {
                *hypothetical += delta;
                true
            }
            _ => false,
        },
        AnalysisKind::DescriptiveStats(d) => {
            d.hypothetical_value = Some(d.hypothetical_value.unwrap_or(0.0) + delta);
            true
        }
        _ => false,
    };
    if changed {
        spec.update_kind(updated);
    }
    Ok(changed)
}

fn step_outlier_param(app: &mut UiApp, delta: f64) -> Result<bool, String> {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return Ok(false);
    };
    let updated = match spec.kind {
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent }) => {
            AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: (fdr_q_percent + delta).clamp(0.1, 20.0) })
        }
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative }) => {
            AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: (alpha + delta).clamp(0.001, 0.25), iterative })
        }
        _ => return Ok(false),
    };
    spec.update_kind(updated);
    Ok(true)
}

fn parse_mult_method(raw: &str, ctrl: usize) -> Result<MultiplicityMethod, String> {
    match raw {
        "none" => Ok(MultiplicityMethod::None),
        "tukey" => Ok(MultiplicityMethod::TukeyHsd),
        "dunnett" => Ok(MultiplicityMethod::Dunnett { control_column: ctrl }),
        "sidak" => Ok(MultiplicityMethod::Sidak),
        "bonferroni" => Ok(MultiplicityMethod::Bonferroni),
        "holm_sidak" => Ok(MultiplicityMethod::HolmSidak),
        "dunnett_t3" => Ok(MultiplicityMethod::DunnettT3),
        "games_howell" => Ok(MultiplicityMethod::GamesHowell),
        "tamhane_t2" => Ok(MultiplicityMethod::TamhaneT2),
        "dunn" => Ok(MultiplicityMethod::DunnRankSum),
        "linear_trend" => Ok(MultiplicityMethod::LinearTrend),
        "bh" => Ok(MultiplicityMethod::BenjaminiHochbergFdr),
        "by" => Ok(MultiplicityMethod::BenjaminiYekutieliFdr),
        "bky" => Ok(MultiplicityMethod::TwoStageBkyFdr),
        "newman_keuls" => Ok(MultiplicityMethod::NewmanKeuls),
        "fisher_lsd" => Ok(MultiplicityMethod::FisherLsd),
        _ => Err(format!("unknown multiplicity `{raw}`")),
    }
}

// ============================================================================
// Modal Body Builder & Layout Model
// ============================================================================

/// Build the multi-tab `Edit Analysis Parameters` modal body inside `body_rect`.
pub fn build_edit_params_modal_body(builder: &mut LayoutBuilder<'_>, parent: WidgetId, body_rect: LayoutRectPx) {
    if is_fit_analysis_draft(builder.app) {
        let controls = crate::inspect::dialog_widget_descriptors(builder.app).map_or_else(Vec::new, |(_, _, _, _, c, _)| c);
        build_fit_params_extra_section(builder, &parent, body_rect, &controls);
        return;
    }

    let edit = &builder.app.canvas_dialogs.edit_params;
    let Some(spec) = edit.draft.as_ref() else {
        return;
    };
    let locale = builder.tree.locale.as_str();
    let supports_rm = supports_repeated_measures(&spec.kind);
    let tab2_id = if supports_rm { "repeated_measures" } else { "factor_settings" };
    let tab2_lbl = if supports_rm {
        label_i18n(locale, "stat_params.tab.repeated_measures", "Repeated Measures")
    } else {
        label_i18n(locale, "stat_params.tab.factor_settings", "Factor Settings")
    };
    let tabs = [
        ("experimental_design", label_i18n(locale, "stat_params.tab.experimental_design", "Experimental Design")),
        (tab2_id, tab2_lbl),
        ("multiple_comparisons", label_i18n(locale, "stat_params.tab.multiple_comparisons", "Multiple Comparisons")),
        ("options", label_i18n(locale, "stat_params.tab.options", "Options")),
        ("diagnostics", label_i18n(locale, "stat_params.tab.diagnostics", "Diagnostics / Residuals")),
    ];

    let scale = builder.scale();
    let tab_gap = scaled_u32(4.0, scale, 3, 10);
    let tab_h = scaled_u32(24.0, scale, 20, 48);
    let min_tab_w = scaled_u32(44.0, scale, 40, 110);
    let tab_w = (body_rect.width.saturating_sub(tab_gap * 4) / (tabs.len() as u32)).max(min_tab_w);
    for (idx, (tid, lbl)) in tabs.iter().enumerate() {
        let tx = body_rect.x + i32::try_from(idx as u32 * (tab_w + tab_gap)).unwrap_or(0);
        let en = evaluate_edit_params_tab(edit, tid);
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id: &format!("edit.tab.{tid}"),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect: LayoutRectPx::new(tx, body_rect.y, tab_w, tab_h),
                label: lbl,
                keyboard_path: &format!("edit.tab.{idx}"),
                focus_scope: FocusScope::ModalDialog,
                tab_order: Some(20 + idx as u32),
                primary: false,
                selected: edit.active_tab == *tid,
                shortcut: None,
                enablement: en,
                action: WidgetAction::SelectEditParamsTab((*tid).to_owned()),
            },
        );
    }

    let doc = builder.app.document();
    let src_sheet = resolve_source_sheet(doc, spec);
    let (rows, cols, max_sub) = source_table_shape(doc, Some(&spec.source_sheet_id));

    let banner_gap = scaled_u32(4.0, scale, 3, 10);
    let banner_h = scaled_u32(20.0, scale, 18, 42);
    let summary_y = body_rect.y + i32::try_from(tab_h + banner_gap).unwrap_or(28);
    if body_rect.height >= scaled_u32(54.0, scale, 48, 120) {
        let banner_rect = LayoutRectPx::new(body_rect.x, summary_y, body_rect.width, banner_h);
        let banner_wid = builder.push_node(NodeSpec {
            id: WidgetId::from("edit.summary_banner"),
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect_px: banner_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Label, spec.effective_title(), FocusScope::ModalDialog),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        builder.paint_fill(ZLayer::ModalDialog, Some(&banner_wid), banner_rect, builder.palette.panel);
        builder.paint_stroke(ZLayer::ModalDialog, Some(&banner_wid), banner_rect, builder.palette.border, 1);
        let src_name = src_sheet.map_or("Source", |ds| ds.title.as_str());
        let alpha = (1.0 - spec.confidence_level.percent() / 100.0).max(0.001);
        let banner_text = format!("{}  |  {} ({rows}×{cols})  |  α = {alpha:.3}", spec.effective_title(), src_name);
        let size_px = (11.5 * scale).clamp(8.5, 24.0);
        paint_text_fitted(
            builder,
            &banner_wid,
            FittedTextSpec {
                z_layer: ZLayer::ModalDialog,
                rect: banner_rect,
                pad_x: scaled_u32(8.0, scale, 6, 16),
                text: &banner_text,
                style: &UiTextStyle::ui_bold(size_px),
                color: builder.palette.text,
                scroll_container: None,
            },
        );
    }

    let content_off = tab_h + banner_h + scaled_u32(8.0, scale, 6, 20);
    let content_rect = LayoutRectPx::new(
        body_rect.x,
        body_rect.y + i32::try_from(content_off).unwrap_or(52),
        body_rect.width,
        body_rect.height.saturating_sub(content_off),
    );
    if content_rect.height < scaled_u32(28.0, scale, 24, 60) || content_rect.width < scaled_u32(80.0, scale, 70, 160) {
        return;
    }

    let active_tab = normalize_active_tab(edit.active_tab.as_str(), supports_rm);
    let shape = TableShapeInfo { rows, cols, max_sub, sheet: src_sheet };

    if matches!(spec.kind, AnalysisKind::Transform(_) | AnalysisKind::PowerAnalysis(_)) && active_tab == "experimental_design" {
        render_transform_or_power_tab(builder, &parent, content_rect, spec, &shape);
        return;
    }

    render_tab_body(builder, &parent, content_rect, active_tab, spec, &shape);
}

fn supports_repeated_measures(kind: &AnalysisKind) -> bool {
    matches!(
        kind,
        AnalysisKind::OneWayAnova(_)
            | AnalysisKind::FactorialAnova(_)
            | AnalysisKind::TwoGroupComparison(_)
            | AnalysisKind::MultipleRowTTests(_)
            | AnalysisKind::NestedComparison(_)
    )
}

fn normalize_active_tab(raw: &str, supports_rm: bool) -> &'static str {
    match raw {
        "repeated_measures" if supports_rm => "repeated_measures",
        "factor_settings" if !supports_rm => "factor_settings",
        "multiple_comparisons" => "multiple_comparisons",
        "options" => "options",
        "diagnostics" => "diagnostics",
        _ => "experimental_design",
    }
}

fn resolve_source_sheet<'a>(doc: &'a curvebook_doc::ProjectDocument, spec: &AnalysisSpec) -> Option<&'a DataSheet> {
    doc.data_sheet(&spec.source_sheet_id).or_else(|| doc.result_sheet(&spec.source_sheet_id).and_then(|rs| rs.analyzable_data.as_ref()))
}

struct TableShapeInfo<'a> {
    rows: usize,
    cols: usize,
    max_sub: usize,
    sheet: Option<&'a DataSheet>,
}

impl TableShapeInfo<'_> {
    fn column_titles(&self) -> Vec<String> {
        let Some(ds) = self.sheet else {
            return (0..self.cols.max(2)).map(|i| format!("Col {}", i + 1)).collect();
        };
        match &ds.archetype {
            TableArchetype::Column(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
            TableArchetype::Grouped(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
            TableArchetype::Xy(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
            TableArchetype::Contingency(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
            TableArchetype::Survival(t) => t.cohorts.iter().map(|c| c.title.clone()).collect(),
            TableArchetype::PartsOfWhole(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
            TableArchetype::MultipleVariables(t) => t.variables.iter().map(|v| v.title.clone()).collect(),
            TableArchetype::Nested(t) => t.groups.iter().map(|g| g.title.clone()).collect(),
        }
    }

    fn col_title(&self, idx: usize) -> String {
        self.column_titles().get(idx).cloned().unwrap_or_else(|| format!("Col {}", idx + 1))
    }

    fn is_multiple_variables(&self) -> bool {
        matches!(self.sheet.map(|s| &s.archetype), Some(TableArchetype::MultipleVariables(_)))
    }
}

// ============================================================================
// Declarative Tab Item Model & Scrolled Renderer
// ============================================================================

struct SegItem {
    id: String,
    label: String,
    selected: bool,
    enablement: ControlEnablement,
    action: WidgetAction,
}

enum StatItem {
    Section(String, String),
    Info(String, String, bool),
    Toggle { id: String, label: String, checked: bool, is_radio: bool, enablement: ControlEnablement, action: WidgetAction },
    ButtonRow { items: Vec<SegItem> },
    Dropdown { id: String, label: String, selected_value: String, enablement: ControlEnablement },
    AlphaSpinner { value_text: String },
    CustomSpinner { id_prefix: String, label: String, value_text: String, step: f64, enablement: ControlEnablement },
}

impl StatItem {
    fn height(&self, scale: f64) -> u32 {
        match self {
            Self::Section(..) | Self::Info(..) => scaled_u32(20.0, scale, 18, 42),
            Self::Toggle { .. } | Self::ButtonRow { .. } => scaled_u32(24.0, scale, 20, 48),
            Self::Dropdown { .. } | Self::AlphaSpinner { .. } | Self::CustomSpinner { .. } => scaled_u32(26.0, scale, 22, 52),
        }
    }
}

fn render_transform_or_power_tab(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    spec: &AnalysisSpec,
    shape: &TableShapeInfo<'_>,
) {
    let mut items = vec![
        StatItem::Section(String::from("edit.sec.scope"), format!("{} Parameters", spec.effective_title())),
        StatItem::Info(
            String::from("edit.info.scope"),
            format!("Source shape: {} rows × {} columns ({} subcolumns)", shape.rows, shape.cols, shape.max_sub),
            false,
        ),
    ];
    append_dataset_items(builder.app, shape, &mut items);
    render_single_column_tab(builder, parent, rect, items, Some(spec));
}

fn render_tab_body(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    tab: &str,
    spec: &AnalysisSpec,
    shape: &TableShapeInfo<'_>,
) {
    if tab == "experimental_design" {
        let mut left = Vec::new();
        collect_experimental_design_items(builder.app, spec, shape, &mut left);
        render_design_two_column(builder, parent, rect, left, spec, shape);
        return;
    }
    let mut items = Vec::new();
    match tab {
        "repeated_measures" => collect_repeated_measures_items(builder.app, spec, shape, &mut items),
        "factor_settings" => collect_factor_settings_items(builder.app, spec, shape, &mut items),
        "multiple_comparisons" => collect_multiple_comparisons_items(builder.app, spec, shape, &mut items),
        "options" => collect_options_items(builder.app, spec, shape, &mut items),
        "diagnostics" => collect_diagnostics_items(builder.app, spec, shape, &mut items),
        _ => collect_experimental_design_items(builder.app, spec, shape, &mut items),
    }
    render_single_column_tab(builder, parent, rect, items, None);
}

fn render_single_column_tab(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    items: Vec<StatItem>,
    extra_descriptors_for: Option<&AnalysisSpec>,
) {
    let scale = builder.scale();
    let row_gap = scaled_u32(4.0, scale, 3, 10);
    let list_pad = scaled_u32(12.0, scale, 8, 24);
    let items_h: u32 = items.iter().map(|it| it.height(scale) + row_gap).sum::<u32>() + list_pad;
    let scroll_id = String::from("scroll.modal.stat_params");
    let scroll_off = builder.scroll_offset(&scroll_id);

    if extra_descriptors_for.is_some()
        && rect.height > items_h + scaled_u32(44.0, scale, 36, 96)
        && let Some((_, _, _, _, controls, _)) = crate::inspect::dialog_widget_descriptors(builder.app)
    {
        let top_rect = LayoutRectPx::new(rect.x, rect.y, rect.width, items_h.min(rect.height / 2));
        emit_scrolled_item_list(builder, parent, top_rect, &scroll_id, scroll_off, &items, 30);
        let desc_y = top_rect.bottom() + i32::try_from(row_gap).unwrap_or(4);
        let desc_h = (rect.bottom() - desc_y).max(0) as u32;
        if desc_h >= scaled_u32(36.0, scale, 28, 72) {
            emit_descriptor_controls_grid(builder, parent, LayoutRectPx::new(rect.x, desc_y, rect.width, desc_h), &controls, 160);
        }
        return;
    }

    emit_scrolled_item_list(builder, parent, rect, &scroll_id, scroll_off, &items, 30);
}

fn emit_scrolled_item_list(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    scroll_id: &str,
    scroll_off: [f64; 2],
    items: &[StatItem],
    base_tab_order: u32,
) {
    let scale = builder.scale();
    let row_gap = scaled_u32(4.0, scale, 3, 10);
    let list_pad = scaled_u32(12.0, scale, 8, 24);
    let sb_reserve = scaled_u32(18.0, scale, 16, 36);
    let content_h: u32 = items.iter().map(|it| it.height(scale) + row_gap).sum::<u32>() + list_pad;
    let scroll_meta = ScrollMeta::new(scroll_id.to_owned(), rect, [rect.width, content_h], scroll_off, ScrollAxes::Vertical);
    let panel_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("edit.content_panel"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Parameter Controls", FocusScope::ModalDialog, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&panel_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&panel_wid), rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&panel_wid), rect, builder.palette.border, 1);

    let scroll_clip = LayoutRectPx::new(rect.x + 4, rect.y + 4, rect.width.saturating_sub(sb_reserve), rect.height.saturating_sub(8));
    if scroll_clip.width < 32 || scroll_clip.height < 16 {
        return;
    }
    builder.push_clip(scroll_clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let mut cur_y = scroll_clip.y + 2 - y_scroll;
    let mut tab_ord = base_tab_order;
    for item in items {
        let h = item.height(scale);
        let row_rect = LayoutRectPx::new(scroll_clip.x + 2, cur_y, scroll_clip.width.saturating_sub(4), h);
        if row_rect.y >= scroll_clip.y - 2 && row_rect.bottom() <= scroll_clip.bottom() + 2 {
            emit_stat_item(builder, &panel_wid, row_rect, item, &mut tab_ord, Some(scroll_id));
        }
        cur_y += i32::try_from(h + row_gap).unwrap_or(28);
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(panel_wid));
}

fn render_design_two_column(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    left_items: Vec<StatItem>,
    spec: &AnalysisSpec,
    shape: &TableShapeInfo<'_>,
) {
    let scale = builder.scale();
    let row_gap = scaled_u32(4.0, scale, 3, 10);
    let list_pad = scaled_u32(12.0, scale, 8, 24);
    let sb_reserve = scaled_u32(18.0, scale, 16, 36);
    let split = rect.width >= scaled_u32(480.0, scale, 420, 960);
    let left_h: u32 = left_items.iter().map(|it| it.height(scale) + row_gap).sum::<u32>() + list_pad;
    let schematic_block_h = scaled_u32(260.0, scale, 220, 520);
    let content_h = if split { left_h.max(schematic_block_h + 16) } else { left_h + schematic_block_h + 16 };

    let scroll_id = String::from("scroll.modal.stat_params");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), rect, [rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let panel_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("edit.content_panel"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Experimental Design", FocusScope::ModalDialog, "ArrowUp/Down"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&panel_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::ModalDialog, Some(&panel_wid), rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&panel_wid), rect, builder.palette.border, 1);

    let scroll_clip = LayoutRectPx::new(rect.x + 4, rect.y + 4, rect.width.saturating_sub(sb_reserve), rect.height.saturating_sub(8));
    if scroll_clip.width < 40 || scroll_clip.height < 20 {
        return;
    }
    builder.push_clip(scroll_clip);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;

    let min_left_w = scaled_u32(180.0, scale, 150, 360);
    let (left_w, right_x, right_w, right_y_start) = if split {
        let lw = (scroll_clip.width.saturating_mul(55) / 100).max(min_left_w);
        let rx = scroll_clip.x + i32::try_from(lw + 8).unwrap_or(200);
        let rw = scroll_clip.width.saturating_sub(lw + 10);
        (lw, rx, rw, scroll_clip.y + 2 - y_scroll)
    } else {
        (
            scroll_clip.width.saturating_sub(4),
            scroll_clip.x + 2,
            scroll_clip.width.saturating_sub(4),
            scroll_clip.y + 2 + i32::try_from(left_h).unwrap_or(200) - y_scroll,
        )
    };

    let mut cur_y = scroll_clip.y + 2 - y_scroll;
    let mut tab_ord = 30u32;
    for item in &left_items {
        let h = item.height(scale);
        let row_rect = LayoutRectPx::new(scroll_clip.x + 2, cur_y, left_w.saturating_sub(4), h);
        if row_rect.y >= scroll_clip.y - 2 && row_rect.bottom() <= scroll_clip.bottom() + 2 {
            emit_stat_item(builder, &panel_wid, row_rect, item, &mut tab_ord, Some(&scroll_id));
        }
        cur_y += i32::try_from(h + row_gap).unwrap_or(28);
    }

    if right_w >= scaled_u32(110.0, scale, 96, 220) {
        let hdr_h = scaled_u32(20.0, scale, 18, 42);
        let hdr_rect = LayoutRectPx::new(right_x, right_y_start, right_w, hdr_h);
        if hdr_rect.y >= scroll_clip.y - 2 && hdr_rect.bottom() <= scroll_clip.bottom() + 2 {
            let hdr_label = label_i18n(builder.tree.locale.as_str(), "stat_params.section.schematic", "Design Schematic");
            emit_stat_item(
                builder,
                &panel_wid,
                hdr_rect,
                &StatItem::Section(String::from("edit.design.schematic_hdr"), hdr_label.to_owned()),
                &mut tab_ord,
                Some(&scroll_id),
            );
        }

        let schem_off = i32::try_from(scaled_u32(24.0, scale, 20, 48)).unwrap_or(24);
        let schem_h = scaled_u32(196.0, scale, 160, 392)
            .min(scroll_clip.height.saturating_sub(scaled_u32(56.0, scale, 44, 112)))
            .max(scaled_u32(96.0, scale, 80, 192));
        let schem_rect = LayoutRectPx::new(right_x, right_y_start + schem_off, right_w, schem_h);
        let vis_schem = schem_rect.intersect(scroll_clip);
        if vis_schem.width >= 40 && vis_schem.height >= 28 {
            emit_design_schematic_canvas(builder, &panel_wid, schem_rect, spec, shape, tab_ord + 100);
        }

        let cap_gap = i32::try_from(scaled_u32(6.0, scale, 4, 12)).unwrap_or(6);
        let cap_h = scaled_u32(22.0, scale, 18, 44);
        let cap_rect = LayoutRectPx::new(right_x, schem_rect.bottom() + cap_gap, right_w, cap_h);
        if cap_rect.y >= scroll_clip.y - 2 && cap_rect.bottom() <= scroll_clip.bottom() + 2 {
            let cap_text = schematic_caption(builder.app, spec, shape);
            emit_stat_item(
                builder,
                &panel_wid,
                cap_rect,
                &StatItem::Info(String::from("edit.design.schematic_caption"), cap_text, true),
                &mut tab_ord,
                Some(&scroll_id),
            );
        }
    }

    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::ModalDialog, &scroll_meta, Some(panel_wid));
}

fn emit_stat_item(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    item: &StatItem,
    tab_ord: &mut u32,
    scroll_id: Option<&str>,
) {
    let scale = builder.scale();
    let size_px = (11.5 * scale).clamp(8.5, 26.0);
    match item {
        StatItem::Section(id, title) => {
            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(id.as_str()),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect_px: rect,
                a11y: AccessibilityMeta::new(WidgetRole::Label, title.clone(), FocusScope::ModalDialog),
                state: WidgetVisualState::default(),
                action: WidgetAction::NoOp,
            });
            paint_text_fitted(
                builder,
                &wid,
                FittedTextSpec {
                    z_layer: ZLayer::ModalDialog,
                    rect,
                    pad_x: 2,
                    text: title,
                    style: &UiTextStyle::ui_bold(size_px),
                    color: builder.palette.accent,
                    scroll_container: scroll_id,
                },
            );
            builder.paint_line(
                ZLayer::ModalDialog,
                Some(&wid),
                [rect.x + 2, rect.bottom() - 1],
                [rect.right() - 2, rect.bottom() - 1],
                builder.palette.border,
                1,
            );
        }
        StatItem::Info(id, text, accent) => {
            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(id.as_str()),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect_px: rect,
                a11y: AccessibilityMeta::new(WidgetRole::Label, text.clone(), FocusScope::ModalDialog),
                state: WidgetVisualState::default(),
                action: WidgetAction::NoOp,
            });
            let bg = if *accent { builder.palette.panel } else { builder.palette.surface };
            builder.paint_fill(ZLayer::ModalDialog, Some(&wid), rect, bg);
            if *accent {
                builder.paint_stroke(ZLayer::ModalDialog, Some(&wid), rect, builder.palette.border, 1);
            }
            paint_text_fitted(
                builder,
                &wid,
                FittedTextSpec {
                    z_layer: ZLayer::ModalDialog,
                    rect,
                    pad_x: 6,
                    text,
                    style: &UiTextStyle::ui_sans((11.0 * scale).clamp(8.5, 24.0)),
                    color: if *accent { builder.palette.text } else { builder.palette.muted },
                    scroll_container: scroll_id,
                },
            );
        }
        StatItem::Toggle { id, label, checked, is_radio, enablement, action } => {
            *tab_ord += 1;
            emit_toggle(
                builder,
                ToggleWidgetSpec {
                    id,
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect,
                    label,
                    keyboard_path: id,
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(*tab_ord),
                    checked: *checked,
                    is_radio: *is_radio,
                    enablement: enablement.clone(),
                    scroll_container: scroll_id,
                    action: action.clone(),
                },
            );
        }
        StatItem::ButtonRow { items } => {
            if items.is_empty() {
                return;
            }
            let gap = scaled_u32(4.0, scale, 3, 10);
            let min_bw = scaled_u32(36.0, scale, 32, 76);
            let count = items.len() as u32;
            let bw = (rect.width.saturating_sub(gap.saturating_mul(count.saturating_sub(1))) / count).max(min_bw);
            for (idx, seg) in items.iter().enumerate() {
                let bx = rect.x + i32::try_from((idx as u32) * (bw + gap)).unwrap_or(0);
                if bx + i32::try_from(bw).unwrap_or(36) > rect.right() + 2 {
                    break;
                }
                *tab_ord += 1;
                emit_button(
                    builder,
                    ButtonWidgetSpec {
                        id: &seg.id,
                        parent: Some(parent.clone()),
                        z_layer: ZLayer::ModalDialog,
                        rect: LayoutRectPx::new(bx, rect.y, bw, rect.height),
                        label: &seg.label,
                        keyboard_path: &seg.id,
                        focus_scope: FocusScope::ModalDialog,
                        tab_order: Some(*tab_ord),
                        primary: false,
                        selected: seg.selected,
                        shortcut: None,
                        enablement: seg.enablement.clone(),
                        action: seg.action.clone(),
                    },
                );
            }
        }
        StatItem::Dropdown { id, label, selected_value, enablement } => {
            *tab_ord += 1;
            emit_dropdown(
                builder,
                DropdownSpec {
                    id,
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect,
                    label,
                    selected_value,
                    keyboard_path: id,
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(*tab_ord),
                    enablement: enablement.clone(),
                },
            );
        }
        StatItem::AlphaSpinner { value_text } => {
            let lbl_w = (rect.width * 48 / 100).max(scaled_u32(80.0, scale, 70, 180));
            let spin_gap = scaled_u32(6.0, scale, 4, 12);
            let lbl_rect = LayoutRectPx::new(rect.x, rect.y, lbl_w, rect.height);
            let lbl = label_i18n(builder.tree.locale.as_str(), "stat_params.label.alpha", "Alpha (1 - Confidence)");
            let lbl_wid = builder.push_node(NodeSpec {
                id: WidgetId::from("edit.alpha.label"),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect_px: lbl_rect,
                a11y: AccessibilityMeta::new(WidgetRole::Label, lbl, FocusScope::ModalDialog),
                state: WidgetVisualState::default(),
                action: WidgetAction::NoOp,
            });
            paint_text_fitted(
                builder,
                &lbl_wid,
                FittedTextSpec {
                    z_layer: ZLayer::ModalDialog,
                    rect: lbl_rect,
                    pad_x: 4,
                    text: lbl,
                    style: &UiTextStyle::ui_sans(size_px),
                    color: builder.palette.text,
                    scroll_container: scroll_id,
                },
            );
            let spin_rect = LayoutRectPx::new(
                lbl_rect.right() + i32::try_from(spin_gap).unwrap_or(6),
                rect.y,
                rect.width.saturating_sub(lbl_w + spin_gap).max(scaled_u32(90.0, scale, 80, 200)),
                rect.height,
            );
            *tab_ord += 2;
            emit_numeric_spinner(
                builder,
                SpinnerSpec {
                    id_prefix: "edit.alpha",
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::ModalDialog,
                    rect: spin_rect,
                    label: "Alpha",
                    value_text,
                    step: 0.01,
                    focus_scope: FocusScope::ModalDialog,
                    tab_order: Some(*tab_ord),
                    enablement: ControlEnablement::enabled(),
                },
            );
        }
        StatItem::CustomSpinner { id_prefix, label, value_text, step, enablement } => {
            let lbl_w = (rect.width * 50 / 100).max(scaled_u32(80.0, scale, 70, 180));
            let spin_gap = scaled_u32(6.0, scale, 4, 12);
            let lbl_rect = LayoutRectPx::new(rect.x, rect.y, lbl_w, rect.height);
            let lbl_wid = builder.push_node(NodeSpec {
                id: WidgetId::from(format!("{id_prefix}.label")),
                parent: Some(parent.clone()),
                z_layer: ZLayer::ModalDialog,
                rect_px: lbl_rect,
                a11y: AccessibilityMeta::new(WidgetRole::Label, label.clone(), FocusScope::ModalDialog),
                state: WidgetVisualState::default(),
                action: WidgetAction::NoOp,
            });
            paint_text_fitted(
                builder,
                &lbl_wid,
                FittedTextSpec {
                    z_layer: ZLayer::ModalDialog,
                    rect: lbl_rect,
                    pad_x: 4,
                    text: label,
                    style: &UiTextStyle::ui_sans(size_px),
                    color: if enablement.enabled { builder.palette.text } else { builder.palette.muted },
                    scroll_container: scroll_id,
                },
            );
            let spin_rect = LayoutRectPx::new(
                lbl_rect.right() + i32::try_from(spin_gap).unwrap_or(6),
                rect.y,
                rect.width.saturating_sub(lbl_w + spin_gap).max(scaled_u32(90.0, scale, 80, 200)),
                rect.height,
            );
            emit_custom_spinner(builder, parent, spin_rect, id_prefix, label, value_text, *step, enablement, tab_ord, scroll_id);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_custom_spinner(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    id_prefix: &str,
    label: &str,
    value_text: &str,
    step: f64,
    enablement: &ControlEnablement,
    tab_ord: &mut u32,
    scroll_id: Option<&str>,
) {
    let scale = builder.scale();
    let btn_w = rect.height.clamp(scaled_u32(18.0, scale, 16, 36), scaled_u32(28.0, scale, 24, 56));
    let dec_rect = LayoutRectPx::new(rect.x, rect.y, btn_w, rect.height);
    let inc_rect = LayoutRectPx::new(rect.right() - i32::try_from(btn_w).unwrap_or(20), rect.y, btn_w, rect.height);
    let val_w = rect.width.saturating_sub(btn_w.saturating_mul(2)).max(scaled_u32(24.0, scale, 20, 56));
    let val_rect = LayoutRectPx::new(dec_rect.right(), rect.y, val_w, rect.height);

    *tab_ord += 1;
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{id_prefix}.dec"),
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: dec_rect,
            label: "-",
            keyboard_path: &format!("{id_prefix}.dec"),
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(*tab_ord),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: enablement.clone(),
            action: custom_action(format!("spinner:{id_prefix}:{}", -step)),
        },
    );

    let val_id = format!("{id_prefix}.val");
    let st = widget_state(builder, &val_id, enablement.clone());
    let mut a11y = AccessibilityMeta::new(WidgetRole::SpinButton, label, FocusScope::ModalDialog).with_keyboard_path(val_id.clone());
    a11y.value_text = Some(value_text.to_owned());
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(val_id),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: val_rect,
        a11y,
        state: st,
        action: WidgetAction::FocusWidget(format!("{id_prefix}.val")),
    });
    builder.paint_fill(ZLayer::ModalDialog, Some(&wid), val_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(&wid), val_rect, builder.palette.border, 1);
    let size_px = (11.5 * scale).clamp(8.5, 26.0);
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: ZLayer::ModalDialog,
            rect: val_rect,
            pad_x: 4,
            text: value_text,
            style: &UiTextStyle::grid_tabular(size_px),
            color: if enablement.enabled { builder.palette.text } else { builder.palette.muted },
            scroll_container: scroll_id,
        },
    );

    *tab_ord += 1;
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{id_prefix}.inc"),
            parent: Some(parent.clone()),
            z_layer: ZLayer::ModalDialog,
            rect: inc_rect,
            label: "+",
            keyboard_path: &format!("{id_prefix}.inc"),
            focus_scope: FocusScope::ModalDialog,
            tab_order: Some(*tab_ord),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: enablement.clone(),
            action: custom_action(format!("spinner:{id_prefix}:{step}")),
        },
    );
}

// ============================================================================
// Tab 1: Experimental Design
// ============================================================================

fn append_dataset_items(app: &UiApp, shape: &TableShapeInfo<'_>, items: &mut Vec<StatItem>) {
    let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() else {
        return;
    };
    let titles = shape.column_titles();
    if titles.is_empty() {
        return;
    }
    let locale = app.locale.as_str();
    items.push(StatItem::Section(
        String::from("edit.sec.datasets"),
        label_i18n(locale, "stat_params.section.datasets", "Datasets & Columns").to_owned(),
    ));
    let mut row = vec![SegItem {
        id: String::from("edit.dataset.all"),
        label: String::from("All"),
        selected: spec.selected_data_sets.is_empty(),
        enablement: ControlEnablement::enabled(),
        action: custom_action("select_all_datasets"),
    }];
    for (idx, title) in titles.iter().enumerate().take(6) {
        let included = spec.selected_data_sets.is_empty() || spec.selected_data_sets.contains(&idx);
        let short = if title.chars().count() > 11 {
            let prefix: String = title.chars().take(10).collect();
            format!("{prefix}…")
        } else {
            title.clone()
        };
        row.push(SegItem {
            id: format!("edit.dataset.{idx}"),
            label: short,
            selected: included,
            enablement: ControlEnablement::enabled(),
            action: custom_action(format!("toggle_dataset:{idx}")),
        });
        if row.len() == 4 {
            items.push(StatItem::ButtonRow { items: std::mem::take(&mut row) });
        }
    }
    if !row.is_empty() {
        items.push(StatItem::ButtonRow { items: row });
    }
}

fn collect_experimental_design_items(app: &UiApp, spec: &AnalysisSpec, shape: &TableShapeInfo<'_>, items: &mut Vec<StatItem>) {
    let ep = &app.canvas_dialogs.edit_params;
    let doc = app.document();
    let locale = app.locale.as_str();
    append_dataset_items(app, shape, items);

    match &spec.kind {
        AnalysisKind::OneWayAnova(a) => collect_one_way_design_items(app, ep, a, locale, items),
        AnalysisKind::FactorialAnova(f) => collect_factorial_design_items(ep, f, shape, locale, items),
        AnalysisKind::TwoGroupComparison(t) => collect_two_group_design_items(app, ep, t, locale, items),
        AnalysisKind::NestedComparison(n) => {
            items.push(StatItem::Section(String::from("edit.sec.nested"), String::from("Nested Design")));
            items.push(StatItem::Toggle {
                id: String::from("edit.nested.anova"),
                label: String::from("Nested one-way ANOVA (3+ groups)"),
                checked: !n.two_group_t_test,
                is_radio: true,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_nested_mode:anova"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.nested.t_test"),
                label: String::from("Nested t test (2 groups)"),
                checked: n.two_group_t_test,
                is_radio: true,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_nested_mode:t_test"),
            });
            push_alpha_spinner(spec, items);
        }
        AnalysisKind::MultipleRowTTests(m) => {
            items.push(StatItem::Section(String::from("edit.sec.mrow"), String::from("Row-by-Row Comparison Design")));
            items.push(StatItem::Toggle {
                id: String::from("edit.mrow.paired"),
                label: String::from("Paired / Matched replicates"),
                checked: m.paired,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_mrow:paired"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.mrow.nonparametric"),
                label: String::from("Nonparametric rank tests"),
                checked: m.nonparametric,
                is_radio: false,
                enablement: r2_assume_gaussian_allowed(doc, Some(&spec.source_sheet_id)),
                action: custom_action("toggle_mrow:nonparametric"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.mrow.equal_sd"),
                label: String::from("Assume consistent SD across rows"),
                checked: m.assume_equal_sd,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_mrow:equal_sd"),
            });
            push_alpha_spinner(spec, items);
        }
        AnalysisKind::Contingency(c) => collect_contingency_design_items(ep, c, shape, items),
        AnalysisKind::Survival(s) => collect_survival_design_items(s, shape, items),
        AnalysisKind::RocCurve(r) => {
            items.push(StatItem::Section(String::from("edit.sec.roc"), String::from("ROC Curve & AUC Method")));
            items.push(StatItem::Toggle {
                id: String::from("edit.roc.ci.delong"),
                label: String::from("DeLong et al. (1988) nonparametric CI"),
                checked: r.ci_method == RocCiMethod::DeLong,
                is_radio: true,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_roc_ci:delong"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.roc.ci.hanley_mcneil"),
                label: String::from("Hanley-McNeil (1982) exponential SE"),
                checked: r.ci_method == RocCiMethod::HanleyMcNeil,
                is_radio: true,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_roc_ci:hanley_mcneil"),
            });
            push_alpha_spinner(spec, items);
        }
        AnalysisKind::GeneralizedLinearModel(g) => collect_glm_design_items(g, shape, spec, items),
        AnalysisKind::Multivariate(m) => collect_multivariate_design_items(ep, m, spec, items),
        AnalysisKind::DescriptiveStats(d) => {
            items.push(StatItem::Section(String::from("edit.sec.desc"), String::from("Descriptive & Normality")));
            let sub_en = r8_subcolumn_aggregation(shape.max_sub);
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.desc.agg.row_means"),
                        label: String::from("Row Means"),
                        selected: d.aggregation == SubcolumnAggregation::RowMeans,
                        enablement: sub_en.clone(),
                        action: custom_action("set_desc_agg:row_means"),
                    },
                    SegItem {
                        id: String::from("edit.desc.agg.per_sub"),
                        label: String::from("Per Subcol"),
                        selected: d.aggregation == SubcolumnAggregation::PerSubcolumn,
                        enablement: sub_en.clone(),
                        action: custom_action("set_desc_agg:per_subcolumn"),
                    },
                    SegItem {
                        id: String::from("edit.desc.agg.pooled"),
                        label: String::from("Pooled"),
                        selected: d.aggregation == SubcolumnAggregation::PooledGrandSample,
                        enablement: sub_en,
                        action: custom_action("set_desc_agg:pooled"),
                    },
                ],
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.desc.normality_battery"),
                label: String::from("Run normality & lognormality test battery"),
                checked: d.run_normality_battery,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_desc_normality"),
            });
            push_alpha_spinner(spec, items);
        }
        AnalysisKind::OutlierDetection(o) => {
            let is_rout = matches!(o, OutlierSpec::Rout { .. });
            items.push(StatItem::Section(String::from("edit.sec.outlier"), String::from("Outlier Detection Method")));
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.outlier.rout"),
                        label: String::from("ROUT (FDR Q)"),
                        selected: is_rout,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_outlier_mode:rout"),
                    },
                    SegItem {
                        id: String::from("edit.outlier.grubbs"),
                        label: String::from("Grubbs (Alpha)"),
                        selected: !is_rout,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_outlier_mode:grubbs"),
                    },
                ],
            });
            let (lbl, val, step) = match o {
                OutlierSpec::Rout { fdr_q_percent } => ("ROUT Q (%)", format!("{fdr_q_percent:.2}%"), 0.5),
                OutlierSpec::Grubbs { alpha, .. } => ("Grubbs Alpha", format!("{alpha:.3}"), 0.01),
            };
            items.push(StatItem::CustomSpinner {
                id_prefix: String::from("edit.outlier.param"),
                label: lbl.to_owned(),
                value_text: val,
                step,
                enablement: ControlEnablement::enabled(),
            });
            if let OutlierSpec::Grubbs { iterative, .. } = o {
                items.push(StatItem::Toggle {
                    id: String::from("edit.outlier.iterative"),
                    label: String::from("Iterative ESD removal"),
                    checked: *iterative,
                    is_radio: false,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action("toggle_grubbs_iterative"),
                });
            }
        }
        AnalysisKind::BlandAltman(ba) => {
            items.push(StatItem::Section(String::from("edit.sec.ba"), String::from("Bland-Altman Method Comparison")));
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.ba.col_a"),
                        label: format!("Method A: {}", shape.col_title(ba.column_a)),
                        selected: true,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("cycle_ba_col:a"),
                    },
                    SegItem {
                        id: String::from("edit.ba.col_b"),
                        label: format!("Method B: {}", shape.col_title(ba.column_b)),
                        selected: true,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("cycle_ba_col:b"),
                    },
                ],
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.ba.percent_diff"),
                label: String::from("Plot % difference (100 * (B - A) / Mean)"),
                checked: ba.percent_difference,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_ba_percent"),
            });
            push_alpha_spinner(spec, items);
        }
        AnalysisKind::PValueStack(ps) => {
            items.push(StatItem::Section(String::from("edit.sec.pstack"), String::from("P-Value Stack Adjustment")));
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.pstack.bh"),
                        label: String::from("BH (FDR)"),
                        selected: ps.method == PValueStackMethod::BenjaminiHochberg,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_pstack_method:bh"),
                    },
                    SegItem {
                        id: String::from("edit.pstack.bky"),
                        label: String::from("2-Stage BKY"),
                        selected: ps.method == PValueStackMethod::TwoStageBky,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_pstack_method:bky"),
                    },
                    SegItem {
                        id: String::from("edit.pstack.bonf"),
                        label: String::from("Bonferroni"),
                        selected: ps.method == PValueStackMethod::Bonferroni,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_pstack_method:bonferroni"),
                    },
                ],
            });
            items.push(StatItem::CustomSpinner {
                id_prefix: String::from("edit.mult_threshold"),
                label: String::from("Alpha / FDR Q"),
                value_text: format!("{:.3}", ps.threshold),
                step: 0.01,
                enablement: ControlEnablement::enabled(),
            });
            let col_lbl = ps.column.map_or_else(|| String::from("Auto (selected)"), |c| shape.col_title(c));
            items.push(StatItem::ButtonRow {
                items: vec![SegItem {
                    id: String::from("edit.pstack.col"),
                    label: format!("P-value column: {col_lbl}"),
                    selected: false,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action("cycle_pstack_col"),
                }],
            });
        }
        AnalysisKind::LinearRegression(_) | AnalysisKind::NonlinearRegression(_) | AnalysisKind::PassingBablok(_) => {}
        AnalysisKind::Transform(_) | AnalysisKind::PowerAnalysis(_) => push_alpha_spinner(spec, items),
    }
}

fn collect_one_way_design_items(app: &UiApp, ep: &EditAnalysisParametersState, a: &OneWayAnovaSpec, locale: &str, items: &mut Vec<StatItem>) {
    let doc = app.document();
    let assume_en = evaluate_dialog_widget(&app.canvas_dialogs, doc, "edit.assume_gaussian");
    let paired_en = evaluate_dialog_widget(&app.canvas_dialogs, doc, "edit.paired_or_matched");
    items.push(StatItem::Section(String::from("edit.sec.design"), String::from("Design & Distribution")));
    items.push(StatItem::Toggle {
        id: String::from("edit.assume_gaussian"),
        label: label_i18n(locale, "stat_params.label.assume_gaussian", "Assume Gaussian (Parametric)").to_owned(),
        checked: ep.assume_gaussian,
        is_radio: false,
        enablement: assume_en.clone(),
        action: WidgetAction::ToggleDialogCheckbox(String::from("assume_gaussian")),
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.paired_or_matched"),
        label: label_i18n(locale, "stat_params.label.paired_matched", "Paired / Repeated Measures Matching").to_owned(),
        checked: ep.paired_or_matched,
        is_radio: false,
        enablement: paired_en.clone(),
        action: WidgetAction::ToggleDialogCheckbox(String::from("paired_or_matched")),
    });

    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.anova.ordinary"),
                label: String::from("Ordinary"),
                selected: a.method == OneWayMethod::OrdinaryAnova,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_one_way_method:ordinary"),
            },
            SegItem {
                id: String::from("edit.anova.welch"),
                label: String::from("Welch"),
                selected: a.method == OneWayMethod::BrownForsytheAndWelch,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_one_way_method:welch"),
            },
            SegItem {
                id: String::from("edit.anova.rm"),
                label: String::from("RM ANOVA"),
                selected: matches!(a.method, OneWayMethod::RepeatedMeasures { .. }),
                enablement: paired_en.clone(),
                action: custom_action("set_one_way_method:rm"),
            },
        ],
    });
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.anova.kruskal"),
                label: String::from("Kruskal-Wallis"),
                selected: a.method == OneWayMethod::KruskalWallis,
                enablement: assume_en.clone(),
                action: custom_action("set_one_way_method:kruskal"),
            },
            SegItem {
                id: String::from("edit.anova.friedman"),
                label: String::from("Friedman"),
                selected: a.method == OneWayMethod::Friedman,
                enablement: if !assume_en.enabled { assume_en } else { paired_en },
                action: custom_action("set_one_way_method:friedman"),
            },
        ],
    });

    // Quick multiplicity + Tukey/Dunn + Alpha strip (`r10_core_and_spike` parity + direct workflow)
    items.push(StatItem::Section(String::from("edit.sec.quick_posthoc"), String::from("Multiple Comparisons & Alpha")));
    items.push(StatItem::Dropdown {
        id: String::from("edit.multiplicity_method"),
        label: String::from("Multiple Comparisons"),
        selected_value: multiplicity_label(a.multiplicity.method),
        enablement: ControlEnablement::enabled(),
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.post_hoc.tukey"),
        label: String::from("Tukey HSD (all-pairs)"),
        checked: a.multiplicity.method == MultiplicityMethod::TukeyHsd,
        is_radio: true,
        enablement: r2_post_hoc_option_enabled(ep.assume_gaussian, true),
        action: WidgetAction::ClickDialogControl(String::from("edit.post_hoc.tukey")),
    });
    if let Some(spec) = ep.draft.as_ref() {
        push_alpha_spinner(spec, items);
    }
}

fn collect_factorial_design_items(
    ep: &EditAnalysisParametersState,
    f: &FactorialAnovaSpec,
    shape: &TableShapeInfo<'_>,
    locale: &str,
    items: &mut Vec<StatItem>,
) {
    items.push(StatItem::Section(String::from("edit.sec.factorial"), String::from("Factorial & RM / REML Design")));
    items.push(StatItem::Toggle {
        id: String::from("edit.factorial.three_way"),
        label: String::from("Three-way factorial model"),
        checked: f.three_way,
        is_radio: false,
        enablement: ControlEnablement::enabled(),
        action: custom_action("toggle_three_way"),
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.paired_or_matched"),
        label: label_i18n(locale, "stat_params.label.paired_matched", "Matched / Repeated Measures").to_owned(),
        checked: ep.paired_or_matched,
        is_radio: false,
        enablement: ControlEnablement::enabled(),
        action: WidgetAction::ToggleDialogCheckbox(String::from("paired_or_matched")),
    });
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.fac.rm.none"),
                label: String::from("No RM"),
                selected: f.repeated_measures == RepeatedMeasuresDesign::None,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_rm_design:none"),
            },
            SegItem {
                id: String::from("edit.fac.rm.row"),
                label: String::from("Row RM"),
                selected: f.repeated_measures == RepeatedMeasuresDesign::RowFactorMatched,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_rm_design:row"),
            },
            SegItem {
                id: String::from("edit.fac.rm.col"),
                label: String::from("Col RM"),
                selected: f.repeated_measures == RepeatedMeasuresDesign::ColumnFactorMatched,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_rm_design:col"),
            },
            SegItem {
                id: String::from("edit.fac.rm.both"),
                label: String::from("Both"),
                selected: f.repeated_measures == RepeatedMeasuresDesign::BothFactorsMatched,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_rm_design:both"),
            },
        ],
    });
    let reml_en = r1_sphericity_or_reml_control(f.repeated_measures != RepeatedMeasuresDesign::None);
    let cov_hdr = label_i18n(locale, "analyze.factorial.residual_covariance", "Residual covariance structure");
    items.push(StatItem::Section(String::from("edit.sec.reml_cov"), cov_hdr.to_owned()));
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.factorial.cov.cs"),
                label: String::from("Compound Symmetry"),
                selected: f.residual_covariance == ResidualCovariance::CompoundSymmetry,
                enablement: reml_en.clone(),
                action: custom_action("set_residual_cov:compound_symmetry"),
            },
            SegItem {
                id: String::from("edit.factorial.cov.un"),
                label: String::from("Unstructured (REML)"),
                selected: f.residual_covariance == ResidualCovariance::Unstructured,
                enablement: reml_en,
                action: custom_action("set_residual_cov:unstructured"),
            },
        ],
    });
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.factorial.ss.type3"),
                label: String::from("Type III SS"),
                selected: f.ss_type == SumOfSquaresType::TypeIII,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_ss_type:type3"),
            },
            SegItem {
                id: String::from("edit.factorial.ss.type1"),
                label: String::from("Type I SS"),
                selected: f.ss_type == SumOfSquaresType::TypeI,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_ss_type:type1"),
            },
        ],
    });
    if shape.is_multiple_variables() {
        let resp_lbl = f.response_column.map_or_else(|| String::from("Auto"), |c| shape.col_title(c));
        items.push(StatItem::Info(
            String::from("edit.fac.mv_info"),
            format!("MV Response: {resp_lbl} | Factors: {}", f.multifactor_columns.len()),
            true,
        ));
    }
    if let Some(spec) = ep.draft.as_ref() {
        push_alpha_spinner(spec, items);
    }
}

fn collect_two_group_design_items(app: &UiApp, ep: &EditAnalysisParametersState, t: &TwoGroupSpec, locale: &str, items: &mut Vec<StatItem>) {
    let doc = app.document();
    let assume_en = evaluate_dialog_widget(&app.canvas_dialogs, doc, "edit.assume_gaussian");
    let paired_en = evaluate_dialog_widget(&app.canvas_dialogs, doc, "edit.paired_or_matched");
    items.push(StatItem::Section(String::from("edit.sec.two_group"), String::from("Two-Group Design")));
    items.push(StatItem::Toggle {
        id: String::from("edit.assume_gaussian"),
        label: label_i18n(locale, "stat_params.label.assume_gaussian", "Assume Gaussian (Parametric)").to_owned(),
        checked: ep.assume_gaussian,
        is_radio: false,
        enablement: assume_en.clone(),
        action: WidgetAction::ToggleDialogCheckbox(String::from("assume_gaussian")),
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.paired_or_matched"),
        label: label_i18n(locale, "stat_params.label.paired_matched", "Paired / Matched Subjects").to_owned(),
        checked: ep.paired_or_matched,
        is_radio: false,
        enablement: paired_en.clone(),
        action: WidgetAction::ToggleDialogCheckbox(String::from("paired_or_matched")),
    });
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.tg.unpaired"),
                label: String::from("Unpaired t"),
                selected: t.method == TwoGroupMethod::UnpairedT,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_two_group_method:unpaired"),
            },
            SegItem {
                id: String::from("edit.tg.welch"),
                label: String::from("Welch t"),
                selected: t.method == TwoGroupMethod::WelchT,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_two_group_method:welch"),
            },
            SegItem {
                id: String::from("edit.tg.paired"),
                label: String::from("Paired t"),
                selected: t.method == TwoGroupMethod::PairedT,
                enablement: paired_en.clone(),
                action: custom_action("set_two_group_method:paired"),
            },
        ],
    });
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.tg.mw"),
                label: String::from("Mann-Whitney"),
                selected: t.method == TwoGroupMethod::MannWhitneyU,
                enablement: assume_en.clone(),
                action: custom_action("set_two_group_method:mann_whitney"),
            },
            SegItem {
                id: String::from("edit.tg.wilcoxon"),
                label: String::from("Wilcoxon"),
                selected: t.method == TwoGroupMethod::WilcoxonSignedRank,
                enablement: if !assume_en.enabled { assume_en.clone() } else { paired_en.clone() },
                action: custom_action("set_two_group_method:wilcoxon"),
            },
            SegItem {
                id: String::from("edit.tg.ks"),
                label: String::from("K-S Test"),
                selected: t.method == TwoGroupMethod::KolmogorovSmirnov,
                enablement: assume_en,
                action: custom_action("set_two_group_method:ks"),
            },
        ],
    });
    if let TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } = t.method {
        items.push(StatItem::CustomSpinner {
            id_prefix: String::from("edit.two_group.hypothetical"),
            label: String::from("Hypothetical Mean/Median"),
            value_text: format!("{hypothetical:.2}"),
            step: 1.0,
            enablement: ControlEnablement::enabled(),
        });
    }
    if let Some(spec) = ep.draft.as_ref() {
        push_alpha_spinner(spec, items);
    }
}

fn collect_contingency_design_items(ep: &EditAnalysisParametersState, c: &ContingencySpec, shape: &TableShapeInfo<'_>, items: &mut Vec<StatItem>) {
    let gate_2x2 = r5_contingency_2x2_effect_sizes(shape.rows, shape.cols);
    let gate_trend = r5_contingency_cochran_armitage_trend(shape.rows, shape.cols);
    items.push(StatItem::Section(String::from("edit.sec.cont"), format!("Contingency Test ({}×{} Table)", shape.rows, shape.cols)));
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.contingency.chi2"),
                label: String::from("Chi-square"),
                selected: matches!(c.method, ContingencyMethod::PearsonChiSquare { yates_continuity: false }),
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_contingency_method:chisquare"),
            },
            SegItem {
                id: String::from("edit.contingency.yates"),
                label: String::from("Yates"),
                selected: matches!(c.method, ContingencyMethod::PearsonChiSquare { yates_continuity: true }),
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_contingency_method:yates"),
            },
            SegItem {
                id: String::from("edit.contingency.likelihood_g2"),
                label: String::from("G2 Ratio"),
                selected: c.method == ContingencyMethod::LikelihoodRatioG2,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_contingency_method:likelihood_ratio_g2"),
            },
        ],
    });
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.contingency.fisher"),
                label: String::from("Fisher Exact"),
                selected: c.method == ContingencyMethod::FisherExact,
                enablement: gate_2x2.clone(),
                action: custom_action("set_contingency_method:fisher"),
            },
            SegItem {
                id: String::from("edit.contingency.barnard_exact"),
                label: String::from("Barnard"),
                selected: c.method == ContingencyMethod::BarnardExact,
                enablement: gate_2x2.clone(),
                action: custom_action("set_contingency_method:barnard"),
            },
            SegItem {
                id: String::from("edit.contingency.mcnemar"),
                label: String::from("McNemar"),
                selected: c.method == ContingencyMethod::McNemarPaired,
                enablement: gate_2x2,
                action: custom_action("set_contingency_method:mcnemar"),
            },
        ],
    });
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.contingency.cochran_armitage"),
                label: String::from("Trend Test"),
                selected: c.method == ContingencyMethod::CochranArmitageTrend,
                enablement: gate_trend,
                action: custom_action("set_contingency_method:cochran_armitage"),
            },
            SegItem {
                id: String::from("edit.contingency.kappa"),
                label: String::from("Cohen Kappa"),
                selected: matches!(c.method, ContingencyMethod::CohenKappa { .. }),
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_contingency_method:cohen_kappa"),
            },
            SegItem {
                id: String::from("edit.contingency.exact_binomial"),
                label: String::from("Binomial"),
                selected: c.method == ContingencyMethod::ExactBinomial,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_contingency_method:exact_binomial"),
            },
        ],
    });
    if let Some(spec) = ep.draft.as_ref() {
        push_alpha_spinner(spec, items);
    }
}

fn collect_survival_design_items(s: &SurvivalAnalysisSpec, shape: &TableShapeInfo<'_>, items: &mut Vec<StatItem>) {
    match s {
        SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox,
            gehan_breslow_wilcoxon,
            trend_test,
            mv_time_column,
            mv_status_column,
            mv_group_column,
            ..
        } => {
            items.push(StatItem::Section(String::from("edit.sec.km"), String::from("Kaplan-Meier Survival Tests")));
            items.push(StatItem::Toggle {
                id: String::from("edit.km.log_rank_mantel_cox"),
                label: String::from("Log-rank (Mantel-Cox) test"),
                checked: *log_rank_mantel_cox,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_km_flag:log_rank"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.km.gehan_breslow_wilcoxon"),
                label: String::from("Gehan-Breslow-Wilcoxon test"),
                checked: *gehan_breslow_wilcoxon,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_km_flag:gehan"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.km.trend_test"),
                label: String::from("Log-rank test for trend"),
                checked: *trend_test,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_km_flag:trend"),
            });
            if shape.is_multiple_variables() {
                let t_lbl = mv_time_column.map_or_else(|| String::from("Auto"), |c| shape.col_title(c));
                let s_lbl = mv_status_column.map_or_else(|| String::from("Auto"), |c| shape.col_title(c));
                let g_lbl = mv_group_column.map_or_else(|| String::from("Auto"), |c| shape.col_title(c));
                items.push(StatItem::ButtonRow {
                    items: vec![
                        SegItem {
                            id: String::from("edit.km.mv_time"),
                            label: format!("Time: {t_lbl}"),
                            selected: false,
                            enablement: ControlEnablement::enabled(),
                            action: custom_action("cycle_km_col:time"),
                        },
                        SegItem {
                            id: String::from("edit.km.mv_status"),
                            label: format!("Status: {s_lbl}"),
                            selected: false,
                            enablement: ControlEnablement::enabled(),
                            action: custom_action("cycle_km_col:status"),
                        },
                        SegItem {
                            id: String::from("edit.km.mv_group"),
                            label: format!("Group: {g_lbl}"),
                            selected: false,
                            enablement: ControlEnablement::enabled(),
                            action: custom_action("cycle_km_col:group"),
                        },
                    ],
                });
            }
        }
        SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, tie_method, .. } => {
            items.push(StatItem::Section(String::from("edit.sec.cox"), String::from("Cox Proportional Hazards Model")));
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.cox.time_col"),
                        label: format!("Time: {}", shape.col_title(*time_column)),
                        selected: true,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("cycle_cox_col:time"),
                    },
                    SegItem {
                        id: String::from("edit.cox.status_col"),
                        label: format!("Event: {}", shape.col_title(*status_column)),
                        selected: true,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("cycle_cox_col:status"),
                    },
                ],
            });
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.cox.tie.efron"),
                        label: String::from("Efron Ties"),
                        selected: *tie_method == CoxTieMethod::Efron,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_cox_tie:efron"),
                    },
                    SegItem {
                        id: String::from("edit.cox.tie.breslow"),
                        label: String::from("Breslow Ties"),
                        selected: *tie_method == CoxTieMethod::Breslow,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_cox_tie:breslow"),
                    },
                ],
            });
            let mut pred_row = Vec::new();
            for c in 0..shape.cols.min(4) {
                if c == *time_column || c == *status_column {
                    continue;
                }
                pred_row.push(SegItem {
                    id: format!("edit.cox.pred.{c}"),
                    label: shape.col_title(c),
                    selected: predictor_columns.contains(&c),
                    enablement: ControlEnablement::enabled(),
                    action: custom_action(format!("toggle_cox_pred:{c}")),
                });
            }
            if !pred_row.is_empty() {
                items.push(StatItem::ButtonRow { items: pred_row });
            }
        }
    }
}

fn collect_glm_design_items(g: &GlmSpec, shape: &TableShapeInfo<'_>, spec: &AnalysisSpec, items: &mut Vec<StatItem>) {
    items.push(StatItem::Section(String::from("edit.sec.glm"), String::from("Generalized Linear Model (GLM)")));
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.glm.family.linear"),
                label: String::from("Linear (OLS)"),
                selected: g.family == GlmFamily::Linear,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_glm_family:linear"),
            },
            SegItem {
                id: String::from("edit.glm.family.logistic"),
                label: String::from("Logistic"),
                selected: g.family == GlmFamily::Logistic,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_glm_family:logistic"),
            },
            SegItem {
                id: String::from("edit.glm.family.poisson"),
                label: String::from("Poisson"),
                selected: g.family == GlmFamily::Poisson,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_glm_family:poisson"),
            },
        ],
    });
    items.push(StatItem::Info(
        String::from("edit.glm.roles_info"),
        format!("Response: {}  |  Predictors: {}", shape.col_title(g.response_column), g.predictor_columns.len()),
        true,
    ));
    let mut pred_row = Vec::new();
    for c in 0..shape.cols.min(6) {
        if c == g.response_column {
            continue;
        }
        pred_row.push(SegItem {
            id: format!("edit.glm.pred.{c}"),
            label: format!("X: {}", shape.col_title(c)),
            selected: g.predictor_columns.contains(&c),
            enablement: ControlEnablement::enabled(),
            action: custom_action(format!("toggle_glm_pred:{c}")),
        });
        if pred_row.len() == 3 {
            items.push(StatItem::ButtonRow { items: std::mem::take(&mut pred_row) });
        }
    }
    if !pred_row.is_empty() {
        items.push(StatItem::ButtonRow { items: pred_row });
    }
    items.push(StatItem::Toggle {
        id: String::from("edit.glm.compute_vif"),
        label: String::from("Compute predictor collinearity (VIF)"),
        checked: g.compute_vif,
        is_radio: false,
        enablement: ControlEnablement::enabled(),
        action: custom_action("toggle_glm_vif"),
    });
    push_alpha_spinner(spec, items);
}

fn collect_multivariate_design_items(ep: &EditAnalysisParametersState, m: &MultivariateSpec, spec: &AnalysisSpec, items: &mut Vec<StatItem>) {
    match m {
        MultivariateSpec::Pca { standardize, parallel_analysis_seed } => {
            items.push(StatItem::Section(String::from("edit.sec.pca"), String::from("Principal Component Analysis")));
            items.push(StatItem::Toggle {
                id: String::from("edit.pca.standardize"),
                label: String::from("Standardize variables (Z-scores / correlation matrix)"),
                checked: *standardize,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_pca_std"),
            });
            let pa_text =
                parallel_analysis_seed.map_or_else(|| String::from("Parallel analysis: disabled"), |s| format!("Parallel analysis seed: {s}"));
            items.push(StatItem::Info(String::from("edit.pca.pa_info"), pa_text, false));
        }
        MultivariateSpec::CorrelationMatrix { spearman_rank } => {
            items.push(StatItem::Section(String::from("edit.sec.corr"), String::from("Correlation Matrix Design")));
            items.push(StatItem::Toggle {
                id: String::from("edit.correlation.spearman_rank"),
                label: String::from("Spearman nonparametric rank correlation"),
                checked: *spearman_rank,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_corr_spearman"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.correlation_all_pairs"),
                label: String::from("Compute all-pairs correlation matrix"),
                checked: ep.correlation_all_pairs,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_corr_all_pairs"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.correlation.heatmap"),
                label: String::from("Create correlation heatmap graph"),
                checked: ep.correlation_heatmap,
                is_radio: false,
                enablement: r7_correlation_heatmap_toggle(ep.correlation_all_pairs),
                action: custom_action("toggle_corr_heatmap"),
            });
        }
        MultivariateSpec::Clustering { k_means_k, distance, linkage, .. } => {
            items.push(StatItem::Section(String::from("edit.sec.cluster"), String::from("Clustering & Distance Metric")));
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.cluster.dist.euclidean"),
                        label: String::from("Euclidean"),
                        selected: *distance == curvebook_doc::ClusterDistance::Euclidean,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_cluster_dist:euclidean"),
                    },
                    SegItem {
                        id: String::from("edit.cluster.dist.manhattan"),
                        label: String::from("Manhattan"),
                        selected: *distance == curvebook_doc::ClusterDistance::Manhattan,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_cluster_dist:manhattan"),
                    },
                    SegItem {
                        id: String::from("edit.cluster.dist.pearson"),
                        label: String::from("Pearson"),
                        selected: *distance == curvebook_doc::ClusterDistance::Pearson,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_cluster_dist:pearson"),
                    },
                ],
            });
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.cluster.link.ward"),
                        label: String::from("Ward"),
                        selected: *linkage == curvebook_doc::ClusterLinkage::Ward,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_cluster_link:ward"),
                    },
                    SegItem {
                        id: String::from("edit.cluster.link.complete"),
                        label: String::from("Complete"),
                        selected: *linkage == curvebook_doc::ClusterLinkage::Complete,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_cluster_link:complete"),
                    },
                    SegItem {
                        id: String::from("edit.cluster.link.upgma"),
                        label: String::from("Average"),
                        selected: *linkage == curvebook_doc::ClusterLinkage::Upgma,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_cluster_link:upgma"),
                    },
                ],
            });
            items.push(StatItem::CustomSpinner {
                id_prefix: String::from("edit.cluster.k_means_k"),
                label: String::from("K-Means Clusters (k)"),
                value_text: k_means_k.map_or_else(|| String::from("3"), |k| k.to_string()),
                step: 1.0,
                enablement: ControlEnablement::enabled(),
            });
        }
        MultivariateSpec::MultipleImputation { imputations_m, max_iters, .. } => {
            items.push(StatItem::Section(String::from("edit.sec.mi"), String::from("Multiple Imputation (MICE)")));
            items.push(StatItem::CustomSpinner {
                id_prefix: String::from("edit.imputation.m"),
                label: String::from("Imputed Datasets (m)"),
                value_text: imputations_m.to_string(),
                step: 1.0,
                enablement: ControlEnablement::enabled(),
            });
            items.push(StatItem::Info(String::from("edit.mi.iters"), format!("Chained-equations Gibbs iterations: {max_iters}"), true));
        }
    }
    push_alpha_spinner(spec, items);
}

fn push_alpha_spinner(spec: &AnalysisSpec, items: &mut Vec<StatItem>) {
    let alpha = (1.0 - spec.confidence_level.percent() / 100.0).max(0.001);
    items.push(StatItem::AlphaSpinner { value_text: format!("{alpha:.3}") });
}

// ============================================================================
// Tab 2: Repeated Measures / Factor Settings
// ============================================================================

fn collect_repeated_measures_items(app: &UiApp, spec: &AnalysisSpec, shape: &TableShapeInfo<'_>, items: &mut Vec<StatItem>) {
    let ep = &app.canvas_dialogs.edit_params;
    let locale = app.locale.as_str();
    match &spec.kind {
        AnalysisKind::OneWayAnova(a) => {
            items.push(StatItem::Section(String::from("edit.sec.rm_one_way"), String::from("Repeated-Measures Sphericity")));
            let gg_checked = !matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false });
            items.push(StatItem::Toggle {
                id: String::from("edit.sphericity.geisser_greenhouse"),
                label: String::from("Apply Geisser-Greenhouse sphericity correction (epsilon)"),
                checked: gg_checked,
                is_radio: false,
                enablement: r1_sphericity_or_reml_control(ep.paired_or_matched),
                action: WidgetAction::ClickDialogControl(String::from("edit.sphericity.geisser_greenhouse")),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.rm.assume_sphericity"),
                label: String::from("Assume sphericity (unadjusted df)"),
                checked: !gg_checked,
                is_radio: true,
                enablement: r1_sphericity_or_reml_control(ep.paired_or_matched),
                action: custom_action("toggle_geisser_greenhouse"),
            });
            items.push(StatItem::Info(
                String::from("edit.rm.block_info"),
                format!("Subject blocks: {} matched rows across {} timepoints/treatments", shape.rows, shape.cols),
                true,
            ));
        }
        AnalysisKind::FactorialAnova(f) => {
            items.push(StatItem::Section(String::from("edit.sec.rm_fac"), String::from("Repeated-Measures Axes & REML")));
            let reml_en = r1_sphericity_or_reml_control(f.repeated_measures != RepeatedMeasuresDesign::None);
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.rm.fac.row"),
                        label: String::from("Row Factor Matched"),
                        selected: f.repeated_measures == RepeatedMeasuresDesign::RowFactorMatched,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_rm_design:row"),
                    },
                    SegItem {
                        id: String::from("edit.rm.fac.col"),
                        label: String::from("Col Factor Matched"),
                        selected: f.repeated_measures == RepeatedMeasuresDesign::ColumnFactorMatched,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_rm_design:col"),
                    },
                    SegItem {
                        id: String::from("edit.rm.fac.both"),
                        label: String::from("Both Factors"),
                        selected: f.repeated_measures == RepeatedMeasuresDesign::BothFactorsMatched,
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_rm_design:both"),
                    },
                ],
            });
            items.push(StatItem::Section(
                String::from("edit.sec.rm_cov"),
                label_i18n(locale, "analyze.factorial.residual_covariance", "Residual covariance structure").to_owned(),
            ));
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.reml.cov.cs"),
                        label: label_i18n(locale, "reml.covariance.compound_symmetry", "Compound symmetry (REML)").to_owned(),
                        selected: f.residual_covariance == ResidualCovariance::CompoundSymmetry,
                        enablement: reml_en.clone(),
                        action: custom_action("set_residual_cov:compound_symmetry"),
                    },
                    SegItem {
                        id: String::from("edit.reml.cov.un"),
                        label: label_i18n(locale, "reml.covariance.unstructured", "Unstructured covariance (REML)").to_owned(),
                        selected: f.residual_covariance == ResidualCovariance::Unstructured,
                        enablement: reml_en.clone(),
                        action: custom_action("set_residual_cov:unstructured"),
                    },
                ],
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.factorial.fit_reml_on_missing"),
                label: String::from("Fit Mixed-Effects REML model when missing values occur"),
                checked: f.fit_reml_on_missing,
                is_radio: false,
                enablement: reml_en,
                action: custom_action("toggle_fit_reml_on_missing"),
            });
            items.push(StatItem::CustomSpinner {
                id_prefix: String::from("edit.factorial.max_interaction_order"),
                label: String::from("Max Interaction Order"),
                value_text: f.max_interaction_order.to_string(),
                step: 1.0,
                enablement: ControlEnablement::enabled(),
            });
        }
        AnalysisKind::TwoGroupComparison(t) => {
            items.push(StatItem::Section(String::from("edit.sec.rm_tg"), String::from("Matched-Pairs Design")));
            items.push(StatItem::Toggle {
                id: String::from("edit.rm.tg.paired_t"),
                label: String::from("Paired t test (mean of differences)"),
                checked: t.method == TwoGroupMethod::PairedT,
                is_radio: true,
                enablement: r1_paired_or_matched_allowed(app.document(), Some(&spec.source_sheet_id)),
                action: custom_action("set_two_group_method:paired"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.rm.tg.ratio_paired"),
                label: String::from("Ratio paired t test (geometric mean of ratios)"),
                checked: t.method == TwoGroupMethod::RatioPairedT,
                is_radio: true,
                enablement: r1_paired_or_matched_allowed(app.document(), Some(&spec.source_sheet_id)),
                action: custom_action("set_two_group_method:ratio_paired"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.rm.tg.wilcoxon"),
                label: String::from("Wilcoxon matched-pairs signed-rank"),
                checked: t.method == TwoGroupMethod::WilcoxonSignedRank,
                is_radio: true,
                enablement: r2_assume_gaussian_allowed(app.document(), Some(&spec.source_sheet_id)),
                action: custom_action("set_two_group_method:wilcoxon"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.two_group.graph_differences"),
                label: String::from("Graph estimation plot of paired differences"),
                checked: ep.graph_differences,
                is_radio: false,
                enablement: r4_two_group_graph_differences(t.method),
                action: WidgetAction::ClickDialogControl(String::from("edit.two_group.graph_differences")),
            });
        }
        AnalysisKind::MultipleRowTTests(m) => {
            items.push(StatItem::Section(String::from("edit.sec.rm_mrow"), String::from("Matched Row Replicates")));
            items.push(StatItem::Toggle {
                id: String::from("edit.rm.mrow.paired"),
                label: String::from("Treat each row's A vs B replicates as matched pairs"),
                checked: m.paired,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_mrow:paired"),
            });
        }
        _ => {
            items.push(StatItem::Info(
                String::from("edit.rm.na"),
                String::from("Repeated-measures matching is configured on Experimental Design."),
                true,
            ));
        }
    }
}

fn collect_factor_settings_items(_app: &UiApp, spec: &AnalysisSpec, shape: &TableShapeInfo<'_>, items: &mut Vec<StatItem>) {
    match &spec.kind {
        AnalysisKind::GeneralizedLinearModel(g) => {
            items.push(StatItem::Section(String::from("edit.sec.glm_factors"), String::from("GLM Outcome & Predictor Roles")));
            let mut resp_row = Vec::new();
            for c in 0..shape.cols.min(6) {
                resp_row.push(SegItem {
                    id: format!("edit.glm.resp.{c}"),
                    label: format!("Y: {}", shape.col_title(c)),
                    selected: g.response_column == c,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action(format!("set_glm_resp:{c}")),
                });
                if resp_row.len() == 3 {
                    items.push(StatItem::ButtonRow { items: std::mem::take(&mut resp_row) });
                }
            }
            if !resp_row.is_empty() {
                items.push(StatItem::ButtonRow { items: resp_row });
            }
            items.push(StatItem::Section(String::from("edit.sec.glm_preds"), String::from("Predictor Inclusion")));
            for c in 0..shape.cols.min(8) {
                if c == g.response_column {
                    continue;
                }
                items.push(StatItem::Toggle {
                    id: format!("edit.glm.factor_pred.{c}"),
                    label: format!("Include predictor `{}`", shape.col_title(c)),
                    checked: g.predictor_columns.contains(&c),
                    is_radio: false,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action(format!("toggle_glm_pred:{c}")),
                });
            }
            if !g.reference_levels.is_empty() {
                let ref_str: Vec<String> = g.reference_levels.iter().map(|(c, lvl)| format!("{}: {lvl}", shape.col_title(*c))).collect();
                items.push(StatItem::Info(String::from("edit.glm.ref_levels"), format!("Reference levels: {}", ref_str.join(", ")), true));
            }
        }
        AnalysisKind::FactorialAnova(f) => {
            items.push(StatItem::Section(String::from("edit.sec.fac_roles"), String::from("Factorial Factor & Response Roles")));
            for c in 0..shape.cols.min(6) {
                items.push(StatItem::ButtonRow {
                    items: vec![
                        SegItem {
                            id: format!("edit.fac.resp.{c}"),
                            label: format!("Set Y: {}", shape.col_title(c)),
                            selected: f.response_column == Some(c),
                            enablement: ControlEnablement::enabled(),
                            action: custom_action(format!("set_fac_resp:{c}")),
                        },
                        SegItem {
                            id: format!("edit.fac.col.{c}"),
                            label: format!("Factor: {}", shape.col_title(c)),
                            selected: f.multifactor_columns.contains(&c),
                            enablement: ControlEnablement::enabled(),
                            action: custom_action(format!("toggle_fac_col:{c}")),
                        },
                    ],
                });
            }
        }
        AnalysisKind::Contingency(c) => {
            items.push(StatItem::Section(String::from("edit.sec.cont_factors"), String::from("Kappa Weighting & Null Hypothesis")));
            let cur_w = match c.method {
                ContingencyMethod::CohenKappa { weighting } => Some(weighting),
                _ => None,
            };
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.kappa.unweighted"),
                        label: String::from("Unweighted Kappa"),
                        selected: cur_w == Some(KappaWeighting::Unweighted),
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_kappa_weighting:unweighted"),
                    },
                    SegItem {
                        id: String::from("edit.kappa.linear"),
                        label: String::from("Linear Kappa"),
                        selected: cur_w == Some(KappaWeighting::Linear),
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_kappa_weighting:linear"),
                    },
                    SegItem {
                        id: String::from("edit.kappa.quadratic"),
                        label: String::from("Quadratic Kappa"),
                        selected: cur_w == Some(KappaWeighting::Quadratic),
                        enablement: ControlEnablement::enabled(),
                        action: custom_action("set_kappa_weighting:quadratic"),
                    },
                ],
            });
            items.push(StatItem::CustomSpinner {
                id_prefix: String::from("edit.contingency.null_proportion"),
                label: String::from("Binomial Null Proportion"),
                value_text: format!("{:.2}", c.null_proportion.unwrap_or(0.5)),
                step: 0.05,
                enablement: ControlEnablement::enabled(),
            });
        }
        AnalysisKind::Survival(s) => {
            items.push(StatItem::Section(String::from("edit.sec.surv_factors"), String::from("Survival Time, Event & Covariate Factors")));
            match s {
                SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, .. } => {
                    items.push(StatItem::Info(
                        String::from("edit.cox.factor_summary"),
                        format!(
                            "Time: {} | Event: {} | Covariates: {}",
                            shape.col_title(*time_column),
                            shape.col_title(*status_column),
                            predictor_columns.len()
                        ),
                        true,
                    ));
                    for c in 0..shape.cols.min(8) {
                        if c == *time_column || c == *status_column {
                            continue;
                        }
                        items.push(StatItem::Toggle {
                            id: format!("edit.cox.factor_pred.{c}"),
                            label: format!("Covariate: {}", shape.col_title(c)),
                            checked: predictor_columns.contains(&c),
                            is_radio: false,
                            enablement: ControlEnablement::enabled(),
                            action: custom_action(format!("toggle_cox_pred:{c}")),
                        });
                    }
                }
                SurvivalAnalysisSpec::KaplanMeier { mv_time_column, mv_status_column, mv_group_column, .. } => {
                    items.push(StatItem::Info(
                        String::from("edit.km.factor_summary"),
                        format!(
                            "Cohorts: {} | MV Time: {:?} | Event: {:?} | Group: {:?}",
                            shape.cols, mv_time_column, mv_status_column, mv_group_column
                        ),
                        true,
                    ));
                }
            }
        }
        _ => {
            items.push(StatItem::Section(String::from("edit.sec.factors_gen"), String::from("Factor & Column Mapping")));
            for (idx, title) in shape.column_titles().iter().enumerate().take(8) {
                let included = spec.selected_data_sets.is_empty() || spec.selected_data_sets.contains(&idx);
                items.push(StatItem::Toggle {
                    id: format!("edit.factor.include.{idx}"),
                    label: format!("Column {}: {title}", idx + 1),
                    checked: included,
                    is_radio: false,
                    enablement: ControlEnablement::enabled(),
                    action: custom_action(format!("toggle_dataset:{idx}")),
                });
            }
        }
    }
}

// ============================================================================
// Tab 3: Multiple Comparisons
// ============================================================================

fn current_multiplicity(ep: &EditAnalysisParametersState) -> Option<curvebook_doc::MultiplicitySpec> {
    let spec = ep.draft.as_ref()?;
    match &spec.kind {
        AnalysisKind::OneWayAnova(a) => Some(a.multiplicity),
        AnalysisKind::FactorialAnova(f) => Some(f.multiplicity),
        AnalysisKind::NestedComparison(n) => Some(n.multiplicity),
        AnalysisKind::MultipleRowTTests(m) => Some(m.multiplicity),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => Some(*multiplicity),
        _ => None,
    }
}

fn multiplicity_label(method: MultiplicityMethod) -> String {
    match method {
        MultiplicityMethod::None => String::from("None (uncorrected)"),
        MultiplicityMethod::TukeyHsd => String::from("Tukey HSD"),
        MultiplicityMethod::Dunnett { control_column } => format!("Dunnett (Ctrl col {})", control_column + 1),
        MultiplicityMethod::Sidak => String::from("Sidak"),
        MultiplicityMethod::Bonferroni => String::from("Bonferroni"),
        MultiplicityMethod::HolmSidak => String::from("Holm-Sidak"),
        MultiplicityMethod::DunnettT3 => String::from("Dunnett T3"),
        MultiplicityMethod::GamesHowell => String::from("Games-Howell"),
        MultiplicityMethod::TamhaneT2 => String::from("Tamhane T2"),
        MultiplicityMethod::DunnRankSum => String::from("Dunn (Rank Sum)"),
        MultiplicityMethod::LinearTrend => String::from("Linear Trend"),
        MultiplicityMethod::BenjaminiHochbergFdr => String::from("Benjamini-Hochberg (FDR)"),
        MultiplicityMethod::BenjaminiYekutieliFdr => String::from("Benjamini-Yekutieli (FDR)"),
        MultiplicityMethod::TwoStageBkyFdr => String::from("Two-Stage BKY (FDR)"),
        MultiplicityMethod::NewmanKeuls => String::from("Newman-Keuls (SNK)"),
        MultiplicityMethod::FisherLsd => String::from("Fisher's LSD"),
    }
}

fn cld_compatibility_status(method: MultiplicityMethod) -> (bool, &'static str) {
    match method {
        MultiplicityMethod::None => (false, "CLD Incompatible: select an all-pairwise post-hoc test (Tukey, Sidak, Bonferroni, Dunn, etc.)."),
        MultiplicityMethod::Dunnett { .. } => {
            (false, "CLD Incompatible: Dunnett compares vs. control only; Piepho (2004) CLD requires all-pairwise contrasts.")
        }
        MultiplicityMethod::LinearTrend => (false, "CLD Incompatible: Linear Trend is an omnibus contrast without all-pairwise letters."),
        _ => (true, "CLD Compatible (Piepho 2004): complete all-pairwise comparisons ready for Compact Letter Display."),
    }
}

fn collect_multiple_comparisons_items(app: &UiApp, spec: &AnalysisSpec, shape: &TableShapeInfo<'_>, items: &mut Vec<StatItem>) {
    let ep = &app.canvas_dialogs.edit_params;
    let locale = app.locale.as_str();
    if let AnalysisKind::PValueStack(ps) = &spec.kind {
        items.push(StatItem::Section(String::from("edit.sec.pstack_mult"), String::from("P-Value Stack Multiplicity & FDR")));
        for (id_s, lbl, m) in [
            ("bonferroni", "Bonferroni (FWER)", PValueStackMethod::Bonferroni),
            ("holm_bonferroni", "Holm-Bonferroni (FWER)", PValueStackMethod::HolmBonferroni),
            ("sidak", "Sidak (FWER)", PValueStackMethod::Sidak),
            ("holm_sidak", "Holm-Sidak (FWER)", PValueStackMethod::HolmSidak),
            ("bh", "Benjamini-Hochberg (FDR)", PValueStackMethod::BenjaminiHochberg),
            ("bky", "Benjamini-Krieger-Yekutieli 2-stage (FDR)", PValueStackMethod::TwoStageBky),
            ("by", "Benjamini-Yekutieli (FDR)", PValueStackMethod::BenjaminiYekutieli),
        ] {
            items.push(StatItem::Toggle {
                id: format!("edit.pstack.m.{id_s}"),
                label: lbl.to_owned(),
                checked: ps.method == m,
                is_radio: true,
                enablement: ControlEnablement::enabled(),
                action: custom_action(format!("set_pstack_method:{id_s}")),
            });
        }
        items.push(StatItem::CustomSpinner {
            id_prefix: String::from("edit.mult_threshold"),
            label: String::from("Familywise Alpha / FDR Q"),
            value_text: format!("{:.3}", ps.threshold),
            step: 0.01,
            enablement: ControlEnablement::enabled(),
        });
        return;
    }

    let Some(mult) = current_multiplicity(ep) else {
        items.push(StatItem::Section(String::from("edit.sec.mc_na"), String::from("Multiple Comparisons")));
        items.push(StatItem::Info(
            String::from("edit.mc.na_info"),
            String::from("This analysis runs a single omnibus/contrast model without post-hoc pairwise comparisons."),
            true,
        ));
        return;
    };

    let param_en = r2_post_hoc_option_enabled(ep.assume_gaussian, true);
    let nonparam_en = r2_post_hoc_option_enabled(ep.assume_gaussian, false);
    items.push(StatItem::Section(String::from("edit.sec.mc_family"), String::from("Post-Hoc Test Family")));
    items.push(StatItem::Dropdown {
        id: String::from("edit.multiplicity_method"),
        label: String::from("Multiplicity Method"),
        selected_value: multiplicity_label(mult.method),
        enablement: ControlEnablement::enabled(),
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.post_hoc.tukey"),
        label: String::from("Tukey HSD (all-pairwise equal variance)"),
        checked: mult.method == MultiplicityMethod::TukeyHsd,
        is_radio: true,
        enablement: param_en.clone(),
        action: custom_action("set_multiplicity:tukey"),
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.post_hoc.dunnett"),
        label: String::from("Dunnett (every group vs. control column)"),
        checked: matches!(mult.method, MultiplicityMethod::Dunnett { .. }),
        is_radio: true,
        enablement: param_en.clone(),
        action: custom_action("set_multiplicity:dunnett"),
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.post_hoc.dunn"),
        label: String::from("Dunn's rank-sum comparisons (nonparametric)"),
        checked: mult.method == MultiplicityMethod::DunnRankSum,
        is_radio: true,
        enablement: nonparam_en,
        action: custom_action("set_multiplicity:dunn"),
    });
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.post_hoc.bonferroni"),
                label: String::from("Bonferroni"),
                selected: mult.method == MultiplicityMethod::Bonferroni,
                enablement: param_en.clone(),
                action: custom_action("set_multiplicity:bonferroni"),
            },
            SegItem {
                id: String::from("edit.post_hoc.holm_sidak"),
                label: String::from("Holm-Sidak"),
                selected: mult.method == MultiplicityMethod::HolmSidak,
                enablement: param_en.clone(),
                action: custom_action("set_multiplicity:holm_sidak"),
            },
            SegItem {
                id: String::from("edit.post_hoc.games_howell"),
                label: String::from("Games-Howell"),
                selected: mult.method == MultiplicityMethod::GamesHowell,
                enablement: param_en,
                action: custom_action("set_multiplicity:games_howell"),
            },
        ],
    });
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.post_hoc.bh"),
                label: String::from("BH (FDR)"),
                selected: mult.method == MultiplicityMethod::BenjaminiHochbergFdr,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_multiplicity:bh"),
            },
            SegItem {
                id: String::from("edit.post_hoc.bky"),
                label: String::from("BKY (FDR)"),
                selected: mult.method == MultiplicityMethod::TwoStageBkyFdr,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_multiplicity:bky"),
            },
            SegItem {
                id: String::from("edit.post_hoc.none"),
                label: String::from("None"),
                selected: mult.method == MultiplicityMethod::None,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_multiplicity:none"),
            },
        ],
    });

    items.push(StatItem::CustomSpinner {
        id_prefix: String::from("edit.mult_threshold"),
        label: String::from("Familywise Alpha / FDR Q"),
        value_text: format!("{:.3}", mult.threshold),
        step: 0.01,
        enablement: ControlEnablement::enabled(),
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.compare_against_control"),
        label: String::from("Compare all groups against a designated control column"),
        checked: ep.compare_against_control,
        is_radio: false,
        enablement: ControlEnablement::enabled(),
        action: WidgetAction::ToggleDialogCheckbox(String::from("compare_against_control")),
    });
    let ctrl_en = r3_control_column_picker(ep.compare_against_control, mult.method);
    items.push(StatItem::ButtonRow {
        items: vec![SegItem {
            id: String::from("edit.control_column"),
            label: format!("Control Column: {} (col {})", shape.col_title(ep.control_column), ep.control_column + 1),
            selected: ctrl_en.enabled,
            enablement: ctrl_en,
            action: custom_action("cycle_control_column"),
        }],
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.mult_report_ci"),
        label: String::from("Compute confidence intervals for pairwise differences"),
        checked: mult.report_confidence_intervals,
        is_radio: false,
        enablement: ControlEnablement::enabled(),
        action: custom_action("toggle_mult_report_ci"),
    });

    items.push(StatItem::Section(
        String::from("edit.sec.cld"),
        label_i18n(locale, "stat_params.section.cld_compat", "Compact Letter Display (CLD) Compatibility").to_owned(),
    ));
    let (cld_ok, cld_msg) = cld_compatibility_status(mult.method);
    items.push(StatItem::Info(String::from("edit.mc.cld_status"), cld_msg.to_owned(), cld_ok));
}

// ============================================================================
// Tab 4: Options
// ============================================================================

fn collect_options_items(app: &UiApp, spec: &AnalysisSpec, shape: &TableShapeInfo<'_>, items: &mut Vec<StatItem>) {
    let ep = &app.canvas_dialogs.edit_params;
    items.push(StatItem::Section(String::from("edit.sec.ci_alpha"), String::from("Significance Level & Confidence Intervals")));
    push_alpha_spinner(spec, items);
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.conf.90"),
                label: String::from("90% CI (α=0.10)"),
                selected: (spec.confidence_level.percent() - 90.0).abs() < 0.05,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_confidence:90.0"),
            },
            SegItem {
                id: String::from("edit.conf.95"),
                label: String::from("95% CI (α=0.05)"),
                selected: (spec.confidence_level.percent() - 95.0).abs() < 0.05,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_confidence:95.0"),
            },
            SegItem {
                id: String::from("edit.conf.99"),
                label: String::from("99% CI (α=0.01)"),
                selected: (spec.confidence_level.percent() - 99.0).abs() < 0.05,
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_confidence:99.0"),
            },
        ],
    });

    match &spec.kind {
        AnalysisKind::TwoGroupComparison(t) => {
            items.push(StatItem::Section(String::from("edit.sec.tg_opts"), String::from("Two-Group Test & Graph Options")));
            items.push(StatItem::Toggle {
                id: String::from("edit.two_group.two_tailed"),
                label: String::from("Two-tailed P value (uncheck for one-tailed)"),
                checked: t.two_tailed,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_two_tailed"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.two_group.welch"),
                label: String::from("Welch's unequal-variance correction"),
                checked: ep.welch_correction || t.method == TwoGroupMethod::WelchT,
                is_radio: false,
                enablement: r4_two_group_welch_correction(t.method),
                action: custom_action("toggle_two_group_flag:welch"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.two_group.graph_differences"),
                label: String::from("Graph estimation plot of paired differences"),
                checked: ep.graph_differences,
                is_radio: false,
                enablement: r4_two_group_graph_differences(t.method),
                action: WidgetAction::ClickDialogControl(String::from("edit.two_group.graph_differences")),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.two_group.graph_ranks"),
                label: String::from("Graph ranks (nonparametric)"),
                checked: ep.graph_ranks,
                is_radio: false,
                enablement: r4_two_group_nonparametric_options(t.method),
                action: custom_action("toggle_two_group_flag:graph_ranks"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.two_group.hodges_lehmann"),
                label: String::from("Hodges-Lehmann median difference CI"),
                checked: ep.hodges_lehmann_ci,
                is_radio: false,
                enablement: r4_two_group_nonparametric_options(t.method),
                action: custom_action("toggle_two_group_flag:hodges_lehmann"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.compute_effect_sizes"),
                label: String::from("Compute standardized effect sizes (Cohen's d / Hedges' g)"),
                checked: t.compute_effect_sizes,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_effect_sizes"),
            });
        }
        AnalysisKind::OneWayAnova(a) => {
            items.push(StatItem::Section(String::from("edit.sec.anova_opts"), String::from("ANOVA Options")));
            items.push(StatItem::Toggle {
                id: String::from("edit.compute_effect_sizes"),
                label: String::from("Compute effect sizes (eta-squared / omega-squared)"),
                checked: a.compute_effect_sizes,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_effect_sizes"),
            });
            let gg_checked = !matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false });
            items.push(StatItem::Toggle {
                id: String::from("edit.sphericity.geisser_greenhouse"),
                label: String::from("Geisser-Greenhouse sphericity correction"),
                checked: gg_checked,
                is_radio: false,
                enablement: r1_sphericity_or_reml_control(ep.paired_or_matched),
                action: WidgetAction::ClickDialogControl(String::from("edit.sphericity.geisser_greenhouse")),
            });
        }
        AnalysisKind::FactorialAnova(f) => {
            items.push(StatItem::Section(String::from("edit.sec.fac_opts"), String::from("Factorial ANOVA Options")));
            items.push(StatItem::Toggle {
                id: String::from("edit.compute_effect_sizes"),
                label: String::from("Compute partial eta-squared effect sizes"),
                checked: f.compute_effect_sizes,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_effect_sizes"),
            });
        }
        AnalysisKind::Contingency(c) => {
            let gate_2x2 = r5_contingency_2x2_effect_sizes(shape.rows, shape.cols);
            items.push(StatItem::Section(String::from("edit.sec.cont_opts"), String::from("2×2 Effect Sizes & Confidence Intervals")));
            items.push(StatItem::Toggle {
                id: String::from("edit.contingency.odds_ratio"),
                label: String::from("Odds Ratio (OR) & Relative Risk (RR)"),
                checked: ep.contingency_odds_ratio_checked || c.compute_odds_and_risk_ratios,
                is_radio: false,
                enablement: gate_2x2.clone(),
                action: custom_action("toggle_contingency_flag:or"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.contingency.attributable_risk"),
                label: String::from("Attributable Risk (Risk Difference)"),
                checked: ep.contingency_attributable_risk_checked,
                is_radio: false,
                enablement: gate_2x2.clone(),
                action: custom_action("toggle_contingency_flag:arr"),
            });
            items.push(StatItem::Toggle {
                id: String::from("edit.contingency.sensitivity_specificity"),
                label: String::from("Sensitivity, Specificity, PPV & NPV"),
                checked: c.compute_sensitivity_specificity,
                is_radio: false,
                enablement: gate_2x2,
                action: custom_action("toggle_contingency_flag:sens"),
            });
            let or_checked = ep.contingency_odds_ratio_checked || c.compute_odds_and_risk_ratios;
            let rr_checked = ep.contingency_relative_risk_checked || c.compute_odds_and_risk_ratios;
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.contingency.ci.baptista_pike"),
                        label: format!("OR CI: {}", ep.contingency_or_ci_method),
                        selected: false,
                        enablement: r6_contingency_ci_selector(shape.rows, shape.cols, or_checked, "Baptista-Pike"),
                        action: custom_action("cycle_contingency_ci:or"),
                    },
                    SegItem {
                        id: String::from("edit.contingency.ci.koopman"),
                        label: format!("RR CI: {}", ep.contingency_rr_ci_method),
                        selected: false,
                        enablement: r6_contingency_ci_selector(shape.rows, shape.cols, rr_checked, "Koopman"),
                        action: custom_action("cycle_contingency_ci:rr"),
                    },
                ],
            });
            items.push(StatItem::ButtonRow {
                items: vec![
                    SegItem {
                        id: String::from("edit.contingency.ci.newcombe_wilson"),
                        label: format!("AR CI: {}", ep.contingency_arr_ci_method),
                        selected: false,
                        enablement: r6_contingency_ci_selector(shape.rows, shape.cols, ep.contingency_attributable_risk_checked, "Newcombe-Wilson"),
                        action: custom_action("cycle_contingency_ci:arr"),
                    },
                    SegItem {
                        id: String::from("edit.contingency.ci.wilson_brown"),
                        label: format!("Sens CI: {}", ep.contingency_diag_ci_method),
                        selected: false,
                        enablement: r6_contingency_ci_selector(shape.rows, shape.cols, c.compute_sensitivity_specificity, "Wilson-Brown"),
                        action: custom_action("cycle_contingency_ci:diag"),
                    },
                ],
            });
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { asymmetrical_greenwood_ci, .. }) => {
            items.push(StatItem::Section(String::from("edit.sec.km_opts"), String::from("Survival CI Options")));
            items.push(StatItem::Toggle {
                id: String::from("edit.km.asymmetrical_greenwood_ci"),
                label: String::from("Asymmetrical log-log Greenwood survival CIs"),
                checked: *asymmetrical_greenwood_ci,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                action: custom_action("toggle_km_flag:greenwood"),
            });
        }
        _ => {}
    }

    items.push(StatItem::Section(String::from("edit.sec.pval_fmt"), String::from("P-Value Display Formatting")));
    items.push(StatItem::ButtonRow {
        items: vec![
            SegItem {
                id: String::from("edit.pval.four_decimal"),
                label: String::from("0.0001"),
                selected: matches!(spec.p_value_format, PValueFormat::FourDecimal),
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_pvalue_format:four_decimal"),
            },
            SegItem {
                id: String::from("edit.pval.apa"),
                label: String::from("APA"),
                selected: matches!(spec.p_value_format, PValueFormat::Apa),
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_pvalue_format:apa"),
            },
            SegItem {
                id: String::from("edit.pval.nejm"),
                label: String::from("NEJM"),
                selected: matches!(spec.p_value_format, PValueFormat::Nejm),
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_pvalue_format:nejm"),
            },
            SegItem {
                id: String::from("edit.pval.custom"),
                label: String::from("Custom"),
                selected: matches!(spec.p_value_format, PValueFormat::Custom { .. }),
                enablement: ControlEnablement::enabled(),
                action: custom_action("set_pvalue_format:custom"),
            },
        ],
    });
    let digits = match spec.p_value_format {
        PValueFormat::FourDecimal => 4,
        PValueFormat::Apa | PValueFormat::Nejm => 3,
        PValueFormat::Custom { decimals, .. } => decimals,
    };
    items.push(StatItem::CustomSpinner {
        id_prefix: String::from("edit.pvalue.digits"),
        label: String::from("Custom P-Value Decimal Digits"),
        value_text: digits.to_string(),
        step: 1.0,
        enablement: r9_pvalue_precision_spinner(&spec.p_value_format),
    });
}

// ============================================================================
// Tab 5: Diagnostics / Residuals
// ============================================================================

fn collect_diagnostics_items(app: &UiApp, spec: &AnalysisSpec, _shape: &TableShapeInfo<'_>, items: &mut Vec<StatItem>) {
    let st = &app.view_shards.stat_params;
    let locale = app.locale.as_str();
    items.push(StatItem::Section(String::from("edit.sec.diag"), String::from("Residual, Q-Q & Assumption Diagnostics")));
    items.push(StatItem::Toggle {
        id: String::from("edit.diag.qq_plot"),
        label: label_i18n(locale, "stat_params.label.qq_diagnostics", "Normal & Lognormal Q-Q Quantiles").to_owned(),
        checked: st.diag_qq_plot,
        is_radio: false,
        enablement: ControlEnablement::enabled(),
        action: custom_action("toggle_diag:qq"),
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.diag.residuals_table"),
        label: label_i18n(locale, "stat_params.label.residuals_table", "Emit Residuals & Standardized Residuals").to_owned(),
        checked: st.diag_residuals_plot,
        is_radio: false,
        enablement: ControlEnablement::enabled(),
        action: custom_action("toggle_diag:residuals"),
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.diag.homoscedasticity"),
        label: String::from("Test homogeneity of variances (Brown-Forsythe / Bartlett)"),
        checked: st.diag_homoscedasticity,
        is_radio: false,
        enablement: ControlEnablement::enabled(),
        action: custom_action("toggle_diag:homoscedasticity"),
    });
    items.push(StatItem::Toggle {
        id: String::from("edit.diag.assumption_checklist"),
        label: String::from("Generate plain-language assumption diagnostic checklist"),
        checked: st.diag_assumption_checklist,
        is_radio: false,
        enablement: ControlEnablement::enabled(),
        action: custom_action("toggle_diag:checklist"),
    });

    let vif_checked = match &spec.kind {
        AnalysisKind::GeneralizedLinearModel(g) => g.compute_vif,
        _ => st.diag_influence_leverage,
    };
    items.push(StatItem::Toggle {
        id: String::from("edit.diag.influence_leverage"),
        label: String::from("Collinearity (VIF) / Leverage & Influence diagnostics"),
        checked: vif_checked,
        is_radio: false,
        enablement: ControlEnablement::enabled(),
        action: custom_action("toggle_diag:influence"),
    });

    let expected_tabs = match &spec.kind {
        AnalysisKind::OneWayAnova(_) | AnalysisKind::FactorialAnova(_) | AnalysisKind::NestedComparison(_) => {
            "Expected output tabs: Main ANOVA, Multiple Comparisons, Homogeneity of Variance / Sphericity, Diagnostics"
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { .. }) => {
            "Expected output tabs: Cohort Summary, Number at Risk, Pairwise Log-Rank, Log-Minus-Log Diagnostics"
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { .. }) => {
            "Expected output tabs: Hazard Ratios & CI, Covariance Matrix, Baseline Survival, Log-Minus-Log Diagnostics"
        }
        AnalysisKind::GeneralizedLinearModel(g) if g.family == GlmFamily::Logistic => {
            "Expected output tabs: Parameter Estimates, ANOVA/Deviance, Covariance, Residuals, Classification Table"
        }
        AnalysisKind::GeneralizedLinearModel(_) => "Expected output tabs: Parameter Estimates, ANOVA/Deviance, Covariance, Residuals, VIF",
        AnalysisKind::RocCurve(_) => "Expected output tabs: ROC Area & CI Summary, Sensitivity/Specificity Cutoff Coordinates",
        AnalysisKind::Contingency(_) => "Expected output tabs: Contingency Summary, Effect Sizes & CIs, Standardized Residuals",
        _ => "Expected output tabs: Main Summary, Normality / Q-Q & Diagnostic Checklist",
    };
    items.push(StatItem::Section(String::from("edit.sec.diag_preview"), String::from("ResultSheet Diagnostic Output")));
    items.push(StatItem::Info(String::from("edit.diag.tabs_preview"), expected_tabs.to_owned(), true));
}

// ============================================================================
// Original Vector Schematics Renderer (`R10.3`, `R10.14` Cleanroom)
// ============================================================================

fn emit_design_schematic_canvas(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    spec: &AnalysisSpec,
    shape: &TableShapeInfo<'_>,
    tab_order: u32,
) {
    let id = "edit.design.schematic";
    let st = widget_state(builder, id, ControlEnablement::enabled());
    let mut a11y = labeled_meta(WidgetRole::Canvas, "Experimental Design Vector Schematic", FocusScope::ModalDialog, "edit.design.schematic");
    a11y.tab_order = Some(tab_order);
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(id),
        parent: Some(parent.clone()),
        z_layer: ZLayer::ModalDialog,
        rect_px: rect,
        a11y,
        state: st.clone(),
        action: custom_action("schematic:toggle"),
    });

    builder.paint_fill(ZLayer::ModalDialog, Some(&wid), rect, builder.palette.panel);
    let border_col = if st.hovered || st.focused { builder.palette.accent } else { builder.palette.border };
    builder.paint_stroke(ZLayer::ModalDialog, Some(&wid), rect, border_col, if st.focused { 2 } else { 1 });

    let inner = rect.inset(10);
    if inner.width < 48 || inner.height < 48 {
        return;
    }

    match &spec.kind {
        AnalysisKind::OneWayAnova(_)
        | AnalysisKind::TwoGroupComparison(_)
        | AnalysisKind::NestedComparison(_)
        | AnalysisKind::MultipleRowTTests(_) => {
            draw_pairing_schematic(builder, &wid, inner, builder.app.canvas_dialogs.edit_params.paired_or_matched, shape.cols.clamp(2, 4));
        }
        AnalysisKind::FactorialAnova(f) => {
            if f.repeated_measures != RepeatedMeasuresDesign::None {
                draw_reml_covariance_schematic(builder, &wid, inner, f.residual_covariance);
            } else {
                draw_factorial_grid_schematic(builder, &wid, inner, f.three_way);
            }
        }
        AnalysisKind::Survival(_) => draw_survival_schematic(builder, &wid, inner),
        AnalysisKind::Contingency(_) => draw_contingency_schematic(builder, &wid, inner, shape.rows, shape.cols),
        AnalysisKind::RocCurve(_) => draw_roc_schematic(builder, &wid, inner),
        AnalysisKind::GeneralizedLinearModel(g) => draw_glm_schematic(builder, &wid, inner, g),
        _ => draw_pairing_schematic(builder, &wid, inner, false, shape.cols.clamp(2, 3)),
    }
}

fn push_circle(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, center: [i32; 2], radius: u32, fill: Rgba8, stroke: Option<Rgba8>) {
    let order = builder.next_order();
    builder.tree.paint_ops.push(PaintOp {
        z_layer: ZLayer::ModalDialog,
        paint_order: order,
        clip: Some(builder.active_clip_id()),
        owner_widget_id: Some(owner.clone()),
        kind: PaintOpKind::CircleIndicator { center, radius, fill, stroke },
    });
}

fn draw_pairing_schematic(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, rect: LayoutRectPx, paired: bool, groups: usize) {
    let g_count = groups.clamp(2, 4);
    let lane_w = rect.width / (g_count as u32);
    let colors = [builder.palette.link_blue, builder.palette.link_green, builder.palette.accent, builder.palette.link_red];
    let subject_fracs = [[0.22_f64, 0.25, 0.30, 0.20], [0.46, 0.50, 0.54, 0.48], [0.68, 0.72, 0.65, 0.75], [0.84, 0.80, 0.86, 0.82]];
    let unpaired_dx = [-8i32, 7, -4, 9];

    for g in 0..g_count {
        let lx = rect.x + i32::try_from((g as u32) * lane_w).unwrap_or(0) + 4;
        let lane_rect = LayoutRectPx::new(lx, rect.y + 4, lane_w.saturating_sub(8), rect.height.saturating_sub(8));
        builder.paint_fill(ZLayer::ModalDialog, Some(owner), lane_rect, builder.palette.surface);
        builder.paint_stroke(ZLayer::ModalDialog, Some(owner), lane_rect, builder.palette.border, 1);
    }

    let y_top = rect.y + 18;
    let y_span = f64::from(rect.height.saturating_sub(36).max(20));

    for subj in 0..4usize {
        let mut prev_pt: Option<[i32; 2]> = None;
        for g in 0..g_count {
            let cx_base = rect.x + i32::try_from((g as u32) * lane_w + lane_w / 2).unwrap_or(0);
            let cx = if paired { cx_base } else { cx_base + unpaired_dx[(subj + g) % 4] };
            let frac = subject_fracs[subj][g % 4];
            #[allow(clippy::cast_possible_truncation)]
            let cy = y_top + (y_span * frac).round() as i32;
            let pt = [cx, cy];
            if paired && let Some(prev) = prev_pt {
                builder.paint_line(ZLayer::ModalDialog, Some(owner), prev, pt, builder.palette.muted, 2);
            }
            let col = if paired { colors[subj % colors.len()] } else { colors[g % colors.len()] };
            push_circle(builder, owner, pt, 4, col, Some(builder.palette.text));
            prev_pt = Some(pt);
        }
    }

    if !paired {
        for g in 0..g_count {
            let cx = rect.x + i32::try_from((g as u32) * lane_w + lane_w / 2).unwrap_or(0);
            let mean_y = rect.y + i32::try_from(rect.height / 2).unwrap_or(40) + (g as i32 - 1) * 8;
            builder.paint_line(ZLayer::ModalDialog, Some(owner), [cx - 14, mean_y], [cx + 14, mean_y], builder.palette.text, 2);
        }
    }
}

fn draw_reml_covariance_schematic(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, rect: LayoutRectPx, cov: ResidualCovariance) {
    let n = 4u32;
    let cell_sz = (rect.width.min(rect.height).saturating_sub(24) / n).clamp(14, 38);
    let mat_w = cell_sz * n;
    let start_x = rect.x + i32::try_from(rect.width.saturating_sub(mat_w) / 2).unwrap_or(0);
    let start_y = rect.y + i32::try_from(rect.height.saturating_sub(mat_w) / 2).unwrap_or(0);
    let diag_col = builder.palette.accent;
    let cs_off_col = builder.palette.link_blue;
    let un_cols = [builder.palette.link_blue, builder.palette.link_green, builder.palette.link_red, builder.palette.excluded];

    for r in 0..n {
        for c in 0..n {
            let cell = LayoutRectPx::new(
                start_x + i32::try_from(c * cell_sz).unwrap_or(0),
                start_y + i32::try_from(r * cell_sz).unwrap_or(0),
                cell_sz.saturating_sub(2),
                cell_sz.saturating_sub(2),
            );
            let fill = if r == c {
                diag_col
            } else if cov == ResidualCovariance::CompoundSymmetry {
                cs_off_col
            } else {
                un_cols[((r + c * 2) as usize) % un_cols.len()]
            };
            builder.paint_fill(ZLayer::ModalDialog, Some(owner), cell, fill);
            builder.paint_stroke(ZLayer::ModalDialog, Some(owner), cell, builder.palette.border, 1);
        }
    }
}

fn draw_factorial_grid_schematic(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, rect: LayoutRectPx, three_way: bool) {
    let cols = if three_way { 3u32 } else { 2u32 };
    let rows = 2u32;
    let cw = (rect.width.saturating_sub(16) / cols).max(24);
    let ch = (rect.height.saturating_sub(16) / rows).max(24);
    let colors = [builder.palette.link_blue, builder.palette.link_green, builder.palette.accent];
    for r in 0..rows {
        for c in 0..cols {
            let cell = LayoutRectPx::new(
                rect.x + 8 + i32::try_from(c * cw).unwrap_or(0),
                rect.y + 8 + i32::try_from(r * ch).unwrap_or(0),
                cw.saturating_sub(6),
                ch.saturating_sub(6),
            );
            builder.paint_fill(ZLayer::ModalDialog, Some(owner), cell, builder.palette.surface);
            builder.paint_stroke(ZLayer::ModalDialog, Some(owner), cell, colors[(c as usize) % colors.len()], 1);
            push_circle(builder, owner, cell.center(), 4, colors[((r + c) as usize) % colors.len()], None);
        }
    }
}

fn draw_survival_schematic(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, rect: LayoutRectPx) {
    let x0 = rect.x + 12;
    let y0 = rect.y + 12;
    let w = i32::try_from(rect.width.saturating_sub(24)).unwrap_or(80);
    let h = i32::try_from(rect.height.saturating_sub(24)).unwrap_or(80);
    builder.paint_line(ZLayer::ModalDialog, Some(owner), [x0, y0], [x0, y0 + h], builder.palette.text, 2);
    builder.paint_line(ZLayer::ModalDialog, Some(owner), [x0, y0 + h], [x0 + w, y0 + h], builder.palette.text, 2);
    let steps_a = [[0, 0], [w / 4, 0], [w / 4, h / 4], [w / 2, h / 4], [w / 2, h / 2], [w * 4 / 5, h / 2], [w * 4 / 5, h * 3 / 4], [w, h * 3 / 4]];
    let steps_b = [[0, 0], [w / 5, 0], [w / 5, h / 5], [w * 2 / 3, h / 5], [w * 2 / 3, h * 2 / 5], [w, h * 2 / 5]];
    for win in steps_a.windows(2) {
        builder.paint_line(
            ZLayer::ModalDialog,
            Some(owner),
            [x0 + win[0][0], y0 + win[0][1]],
            [x0 + win[1][0], y0 + win[1][1]],
            builder.palette.link_blue,
            2,
        );
    }
    for win in steps_b.windows(2) {
        builder.paint_line(
            ZLayer::ModalDialog,
            Some(owner),
            [x0 + win[0][0], y0 + win[0][1]],
            [x0 + win[1][0], y0 + win[1][1]],
            builder.palette.link_green,
            2,
        );
    }
    // Censoring tick marks
    builder.paint_line(
        ZLayer::ModalDialog,
        Some(owner),
        [x0 + w * 2 / 3, y0 + h * 2 / 5 - 4],
        [x0 + w * 2 / 3, y0 + h * 2 / 5 + 4],
        builder.palette.link_green,
        2,
    );
}

fn draw_contingency_schematic(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, rect: LayoutRectPx, rows: usize, cols: usize) {
    let r_n = (rows.clamp(2, 4)) as u32;
    let c_n = (cols.clamp(2, 4)) as u32;
    let cw = (rect.width.saturating_sub(16) / c_n).max(16);
    let ch = (rect.height.saturating_sub(16) / r_n).max(16);
    for r in 0..r_n {
        for c in 0..c_n {
            let cell = LayoutRectPx::new(
                rect.x + 8 + i32::try_from(c * cw).unwrap_or(0),
                rect.y + 8 + i32::try_from(r * ch).unwrap_or(0),
                cw.saturating_sub(4),
                ch.saturating_sub(4),
            );
            let fill = if r == 0 && c == 0 {
                builder.palette.accent
            } else if r == c {
                builder.palette.link_blue
            } else {
                builder.palette.surface
            };
            builder.paint_fill(ZLayer::ModalDialog, Some(owner), cell, fill);
            builder.paint_stroke(ZLayer::ModalDialog, Some(owner), cell, builder.palette.border, 1);
        }
    }
}

fn draw_roc_schematic(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, rect: LayoutRectPx) {
    let sz = rect.width.min(rect.height).saturating_sub(24).max(32);
    let x0 = rect.x + i32::try_from(rect.width.saturating_sub(sz) / 2).unwrap_or(0);
    let y0 = rect.y + i32::try_from(rect.height.saturating_sub(sz) / 2).unwrap_or(0);
    let box_rect = LayoutRectPx::new(x0, y0, sz, sz);
    builder.paint_fill(ZLayer::ModalDialog, Some(owner), box_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::ModalDialog, Some(owner), box_rect, builder.palette.text, 1);
    let szi = i32::try_from(sz).unwrap_or(60);
    // Diagonal chance line
    builder.paint_line(ZLayer::ModalDialog, Some(owner), [x0, y0 + szi], [x0 + szi, y0], builder.palette.muted, 1);
    // Stepped ROC curve above diagonal
    let pts = [[0, szi], [0, szi / 2], [szi / 5, szi / 2], [szi / 5, szi / 5], [szi / 2, szi / 5], [szi / 2, szi / 12], [szi, 0]];
    for win in pts.windows(2) {
        builder.paint_line(
            ZLayer::ModalDialog,
            Some(owner),
            [x0 + win[0][0], y0 + win[0][1]],
            [x0 + win[1][0], y0 + win[1][1]],
            builder.palette.accent,
            2,
        );
    }
}

fn draw_glm_schematic(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, rect: LayoutRectPx, g: &GlmSpec) {
    let link_x = rect.x + i32::try_from(rect.width / 2).unwrap_or(60);
    let link_y = rect.y + i32::try_from(rect.height / 2).unwrap_or(50);
    let resp_x = rect.right() - 28;
    let pred_n = g.predictor_columns.len().clamp(2, 4);
    let pred_x = rect.x + 28;
    let step_y = rect.height.saturating_sub(24) / (pred_n as u32);
    builder.paint_line(ZLayer::ModalDialog, Some(owner), [link_x, link_y], [resp_x, link_y], builder.palette.accent, 2);
    for i in 0..pred_n {
        let py = rect.y + 18 + i32::try_from((i as u32) * step_y + step_y / 4).unwrap_or(0);
        builder.paint_line(ZLayer::ModalDialog, Some(owner), [pred_x, py], [link_x, link_y], builder.palette.link_blue, 2);
        push_circle(builder, owner, [pred_x, py], 6, builder.palette.link_blue, Some(builder.palette.text));
    }
    push_circle(builder, owner, [link_x, link_y], 8, builder.palette.accent, Some(builder.palette.text));
    push_circle(builder, owner, [resp_x, link_y], 7, builder.palette.link_green, Some(builder.palette.text));
}

fn schematic_caption(app: &UiApp, spec: &AnalysisSpec, shape: &TableShapeInfo<'_>) -> String {
    let ep = &app.canvas_dialogs.edit_params;
    match &spec.kind {
        AnalysisKind::OneWayAnova(a) => {
            if ep.paired_or_matched {
                format!(
                    "Matched/RM blocks ({} rows × {} cols) | {}",
                    shape.rows,
                    shape.cols,
                    match a.method {
                        OneWayMethod::RepeatedMeasures { geisser_greenhouse: true } => "GG epsilon",
                        OneWayMethod::RepeatedMeasures { geisser_greenhouse: false } => "Sphericity",
                        OneWayMethod::Friedman => "Friedman ranks",
                        _ => "RM",
                    }
                )
            } else {
                format!("Independent groups ({} cols) | {}", shape.cols, if ep.assume_gaussian { "Parametric" } else { "Kruskal-Wallis" })
            }
        }
        AnalysisKind::FactorialAnova(f) => {
            if f.repeated_measures != RepeatedMeasuresDesign::None {
                let cov = match f.residual_covariance {
                    ResidualCovariance::CompoundSymmetry => "Compound Symmetry",
                    ResidualCovariance::Unstructured => "Unstructured REML",
                };
                format!("{:?} | {cov}", f.repeated_measures)
            } else {
                format!("Independent {:?} | {} factors", f.ss_type, if f.three_way { 3 } else { 2 })
            }
        }
        AnalysisKind::TwoGroupComparison(_) => {
            if ep.paired_or_matched {
                String::from("Paired subjects: connected within-subject differences")
            } else {
                String::from("Independent samples: two unlinked groups")
            }
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { .. }) => {
            String::from("Step survival curves + censoring ticks (click to toggle detail)")
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, .. }) => format!("Cox PH hazards | Ties: {tie_method:?}"),
        AnalysisKind::Contingency(_) => format!("{}×{} contingency cell matrix", shape.rows, shape.cols),
        AnalysisKind::RocCurve(r) => format!("ROC AUC curve | CI: {:?}", r.ci_method),
        AnalysisKind::GeneralizedLinearModel(g) => format!("{:?} GLM link g(μ) | {} predictors", g.family, g.predictor_columns.len()),
        _ => format!("{} ({} cols)", spec.effective_title(), shape.cols),
    }
}
