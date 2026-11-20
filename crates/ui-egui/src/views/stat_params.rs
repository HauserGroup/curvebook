//! Statistical, ANOVA/RM/REML, survival, contingency, ROC, and GLM multi-tab parameter modal (`R10.3`).

use curvebook_doc::{
    AnalysisKind, ContingencySpec, MultiplicityMethod, MultivariateSpec, OneWayMethod, OutlierSpec, PValueFormat, RepeatedMeasuresDesign,
    ResidualCovariance, RocCiMethod, RocSpec, SurvivalAnalysisSpec, TwoGroupMethod,
};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::dialogs_canvas::{EditAnalysisParametersState, VariableRoleFilter};
use crate::interaction::UiEventOutcome;
use crate::modal_predicates::{
    is_reml_mixed_effects_active, r1_paired_or_matched_allowed, r1_sphericity_or_reml_control, r1b_post_hoc_option_enabled,
    r1b_sphericity_correction, r2_assume_gaussian_allowed, r2_parametric_post_hoc_method, r3_control_column_picker, r4_two_group_diagnostics,
    r5_contingency_2x2_effect_sizes, r5_contingency_cochran_armitage_trend, r6_contingency_ci_selector, r7_correlation_heatmap_toggle,
    r8_subcolumn_aggregation, r9_pvalue_precision_spinner, source_table_shape,
};

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
        "toggle_mixed_reml" => handle_toggle_mixed_reml(app)?,
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
    if let Some(rest) = payload.strip_prefix("set_mv_role_filter:") {
        let filter = VariableRoleFilter::from_slug(rest).ok_or_else(|| format!("unknown variable role filter `{rest}`"))?;
        app.canvas_dialogs.edit_params.role_filter = filter;
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
    if let Some(rest) = payload.strip_prefix("set_df_method:") {
        return set_mixed_df_method(app, rest);
    }
    if let Some(rest) = payload.strip_prefix("set_interaction_order:") {
        let order = rest.parse::<usize>().map_err(|_| format!("invalid interaction order `{rest}`"))?;
        let mut map = serde_json::Map::new();
        map.insert(String::from("max_interaction_order"), serde_json::json!(order));
        app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
        return Ok(true);
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
    if payload == "toggle_kmeans_auto" {
        let cur_auto = matches!(
            app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind),
            Some(AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k: None, .. }))
        );
        let mut map = serde_json::Map::new();
        map.insert(String::from("k_means_auto"), serde_json::json!(!cur_auto));
        app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
        return Ok(true);
    }
    if let Some(rest) = payload.strip_prefix("toggle_pca_pred:") {
        let col = rest.parse::<usize>().map_err(|_| format!("invalid PCA predictor index `{rest}`"))?;
        app.canvas_dialogs.edit_params.toggle_pca_predictor(col);
        return Ok(true);
    }
    if let Some(rest) = payload.strip_prefix("set_pcr_outcome:") {
        let col = rest.parse::<usize>().map_err(|_| format!("invalid PCR outcome index `{rest}`"))?;
        app.canvas_dialogs.edit_params.select_pcr_outcome(col)?;
        return Ok(true);
    }
    if let Some(rest) = payload.strip_prefix("set_tg_group:") {
        let col = rest.parse::<usize>().map_err(|_| format!("invalid two-group group column `{rest}`"))?;
        let mut map = serde_json::Map::new();
        map.insert(String::from("mv_group_column"), serde_json::json!(col));
        app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
        return Ok(true);
    }
    if let Some(rest) = payload.strip_prefix("set_tg_resp:") {
        let col = rest.parse::<usize>().map_err(|_| format!("invalid two-group response column `{rest}`"))?;
        let mut map = serde_json::Map::new();
        map.insert(String::from("mv_response_column"), serde_json::json!(col));
        app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
        return Ok(true);
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
        return toggle_diag_flag(app, state, rest);
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
    let reml_active = is_reml_mixed_effects_active(ep, app.document());
    let gate = r1b_sphericity_correction(ep.paired_or_matched, reml_active);
    if !gate.enabled {
        return Err(gate.disabled_reason.unwrap_or_default());
    }
    let cur = ep.draft.as_ref().is_none_or(
        |d| !matches!(d.kind, AnalysisKind::OneWayAnova(ref a) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })),
    );
    let mut map = serde_json::Map::new();
    map.insert(String::from("geisser_greenhouse"), serde_json::Value::Bool(!cur));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn handle_toggle_mixed_reml(app: &mut UiApp) -> Result<bool, String> {
    let ep = &app.canvas_dialogs.edit_params;
    let paired_ok = match ep.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::FactorialAnova(f)) => f.repeated_measures != RepeatedMeasuresDesign::None,
        _ => ep.paired_or_matched,
    };
    let gate = r1_sphericity_or_reml_control(paired_ok);
    if !gate.enabled {
        return Err(gate.disabled_reason.unwrap_or_default());
    }
    let cur = matches!(
        ep.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::OneWayAnova(a)) if a.mixed_effects_reml
    ) || matches!(
        ep.draft.as_ref().map(|d| &d.kind),
        Some(AnalysisKind::FactorialAnova(f)) if f.mixed_effects_reml
    );
    let mut map = serde_json::Map::new();
    map.insert(String::from("mixed_effects_reml"), serde_json::Value::Bool(!cur));
    app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
    Ok(true)
}

fn set_mixed_df_method(app: &mut UiApp, raw: &str) -> Result<bool, String> {
    let ep = &app.canvas_dialogs.edit_params;
    let gated_ok = match ep.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::NestedComparison(_)) => true,
        Some(AnalysisKind::FactorialAnova(f)) => f.repeated_measures != RepeatedMeasuresDesign::None,
        _ => ep.paired_or_matched,
    };
    if !gated_ok {
        return Err(r1_sphericity_or_reml_control(false).disabled_reason.unwrap_or_default());
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("df_method"), serde_json::Value::String(raw.to_owned()));
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
    let method = parse_mult_method(raw, ep.control_column)?;
    let reml_active = is_reml_mixed_effects_active(ep, app.document());
    let gate = r1b_post_hoc_option_enabled(ep.assume_gaussian, reml_active, method);
    if !gate.enabled {
        return Err(gate.disabled_reason.unwrap_or_default());
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
    let ep = &mut app.canvas_dialogs.edit_params;
    let Some(spec) = &mut ep.draft else {
        return Ok(false);
    };
    match spec.kind {
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent }) => ep.saved_outlier_rout_q = fdr_q_percent,
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative }) => {
            ep.saved_outlier_grubbs_alpha = alpha;
            ep.saved_outlier_grubbs_iterative = iterative;
        }
        _ => {}
    }
    let next = match mode {
        "rout" => OutlierSpec::Rout { fdr_q_percent: ep.saved_outlier_rout_q },
        "grubbs" => OutlierSpec::Grubbs { alpha: ep.saved_outlier_grubbs_alpha, iterative: ep.saved_outlier_grubbs_iterative },
        _ => return Ok(false),
    };
    spec.update_kind(AnalysisKind::OutlierDetection(next));
    ep.sync_rule_flags_from_kind();
    Ok(true)
}

fn handle_toggle_grubbs_iterative(app: &mut UiApp) -> bool {
    let ep = &mut app.canvas_dialogs.edit_params;
    let Some(spec) = &mut ep.draft else {
        return false;
    };
    if let AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative }) = spec.kind {
        let next_iter = !iterative;
        ep.saved_outlier_grubbs_iterative = next_iter;
        spec.update_kind(AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative: next_iter }));
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

fn toggle_diag_flag(app: &mut UiApp, state: &mut StatParamsViewState, flag: &str) -> Result<bool, String> {
    if let Some(AnalysisKind::TwoGroupComparison(t)) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
        let gate = r4_two_group_diagnostics(t.method);
        if !gate.enabled {
            return Err(gate.disabled_reason.unwrap_or_default());
        }
    }
    let changed = match flag {
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
    };
    Ok(changed)
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
            let (cur, max_cap) = match app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
                Some(AnalysisKind::FactorialAnova(f)) => (f.max_interaction_order, f.multifactor_columns.len().max(3)),
                _ => (2, 3),
            };
            let next = if delta >= 0.0 { (cur + 1).min(max_cap) } else { cur.saturating_sub(1).max(1) };
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
        "edit.cluster.k_sweep_max" => {
            let cur_opt = match app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
                Some(AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_sweep_max, .. })) => *k_sweep_max,
                _ => None,
            };
            let next: Option<usize> = if delta >= 0.0 {
                Some(cur_opt.map_or(4, |c| (c + 1).min(12)))
            } else {
                cur_opt.and_then(|c| if c <= 2 { None } else { Some(c - 1) })
            };
            let mut map = serde_json::Map::new();
            map.insert(String::from("k_sweep_max"), next.map_or(serde_json::Value::Null, |k| serde_json::json!(k)));
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
    let ep = &mut app.canvas_dialogs.edit_params;
    let Some(spec) = &mut ep.draft else {
        return Ok(false);
    };
    let updated = match spec.kind {
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent }) => {
            let next_q = (fdr_q_percent + delta).clamp(0.1, 20.0);
            ep.saved_outlier_rout_q = next_q;
            AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: next_q })
        }
        AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha, iterative }) => {
            let next_a = (alpha + delta).clamp(0.001, 0.25);
            ep.saved_outlier_grubbs_alpha = next_a;
            AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: next_a, iterative })
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
