//! `serde` JSON patch normalizers, `draft.kind` flat-key routing, and invariant hooks for
//! `AnalyzeDialogState` and `EditAnalysisParametersState`.

use curvebook_doc::{
    AnalysisKind, AreaUnderCurveSpec, AucBaseline, BlandAltmanSpec, ContingencyMethod, ContingencySpec, DemingErrorSpec, FactorialAnovaSpec,
    FitFlagRules, FitMethod, GlmSpec, IdentifiabilityMode, KappaWeighting, LinearRegressionSpec, ModelComparisonMethod, ModelComparisonQuestion,
    ModelComparisonSpec, MultipleRowTestsSpec, MultiplicityMethod, MultivariateSpec, NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod, OutlierSpec,
    PValueFormat, PassingBablokSpec, RepeatedMeasuresDesign, ResidualCovariance, RocCiMethod, RocSpec, SimpleLogisticSpec, SurvivalAnalysisSpec,
    TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec,
};
use serde_json::{Map, Value, json};

use super::engine::{FieldKind, FieldRule, merge_struct_json, try_apply_rule};
use super::r9_misc::{parse_power_family_for_spec, parse_power_solve_target_for_spec, power_tail_mode_from_bool};
use super::value_parsers::{
    apply_transform_formula_str, norm_key, parse_baseline_operation_str, parse_bool_field, parse_ci_method_choice, parse_cluster_distance_str,
    parse_cluster_linkage_str, parse_contingency_method_name, parse_cox_tie_method_str, parse_df_method_str, parse_error_prop_value,
    parse_f64_list_with_locale, parse_finite_f64_with_locale, parse_fit_method, parse_glm_family_str, parse_kappa_weighting, parse_math_preset_opt,
    parse_multiplicity_method_name, parse_nonlinear_rout_q_percent, parse_one_way_method_str, parse_open_unit_f64, parse_opt_finite_f64_with_locale,
    parse_opt_nonempty_string, parse_opt_range_pair, parse_opt_upper_percentile, parse_opt_usize_min, parse_outlier_alpha,
    parse_outlier_fdr_q_percent, parse_outlier_method_str, parse_parameter_rules, parse_pos_f64, parse_pvalue_stack_method_val,
    parse_reference_levels, parse_residual_covariance_str, parse_rm_design_str, parse_ss_type_str, parse_str_field, parse_subcolumn_aggregation_val,
    parse_table_kind_str, parse_two_group_method_str, parse_u64_val, parse_usize_list, parse_usize_min, parse_weighting_scheme,
    post_hoc_method_for_widget_key,
};
use crate::dialogs_canvas::edit_params::{check_pcr_target_leakage, draft_forces_reml_mode, two_group_is_gaussian, two_group_is_paired};
use crate::dialogs_canvas::{AnalyzeDialogState, EditAnalysisParametersState, default_analysis_kind_for_id};
use crate::i18n::LocalePreset;

pub fn patch_analyze(state: &mut AnalyzeDialogState, fields: &Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
    let mut working = state.clone();
    let saved_explicit_resp = working.explicit_response_column;
    let saved_force_create_new_once = working.force_create_new_once;
    apply_analyze_priority(&mut working, fields, preset)?;

    let mut patch = Map::new();
    let mut eff_all_pairs = working.correlation_all_pairs;
    let mut wants_heatmap: Option<bool> = None;
    let mut formula_x = working.transform_x;
    let mut formula_y = working.transform_y;
    let mut saw_formula_transforms = false;
    let mut explicit_x_transform: Option<Value> = None;
    let mut explicit_y_transform: Option<Value> = None;
    let mut post_actions: Vec<AnalyzePostAction> = Vec::new();

    for (key, val) in fields {
        if is_analyze_priority_key(key) {
            continue;
        }
        match key.as_str() {
            "search" | "search_query" | "analyze.search" => {
                patch.insert(String::from("search_query"), json!(parse_str_field(val, key)?));
            }
            "equation_id" | "fit_equation_id" | "analyze.equation_id" | "analyze.fit.equation_id" | "analyze.fit.model_id" => {
                patch.insert(String::from("equation_id"), json!(parse_str_field(val, key)?));
            }
            "correlation_all_pairs" | "analyze.correlation_all_pairs" => {
                let b = parse_bool_field(val, key)?;
                eff_all_pairs = b;
                patch.insert(String::from("correlation_all_pairs"), json!(b));
                if !b {
                    patch.insert(String::from("correlation_heatmap"), json!(false));
                }
            }
            "correlation_heatmap" | "analyze.correlation_heatmap" | "analyze.correlation.heatmap" => {
                wants_heatmap = Some(parse_bool_field(val, key)?);
            }
            "pcr_outcome_column" | "analyze.pcr_outcome_column" => {
                if let Some(col) = parse_opt_usize_min(val, key, 0)? {
                    check_pcr_target_leakage(col, working.is_pca_predictor(col))?;
                    patch.insert(String::from("pcr_outcome_column"), json!(Some(col)));
                    post_actions.push(AnalyzePostAction::SetResponseColumn(col));
                } else {
                    patch.insert(String::from("pcr_outcome_column"), Value::Null);
                }
            }
            "response_column" => {
                let is_pcr = matches!(working.selected_entry_id.as_deref(), Some("stats.pcr" | "pcr"));
                if let Some(col) = parse_opt_usize_min(val, key, 0)? {
                    if is_pcr {
                        check_pcr_target_leakage(col, working.is_pca_predictor(col))?;
                        patch.insert(String::from("pcr_outcome_column"), json!(Some(col)));
                    }
                    post_actions.push(AnalyzePostAction::SetResponseColumn(col));
                } else if is_pcr {
                    patch.insert(String::from("pcr_outcome_column"), Value::Null);
                }
            }
            "formula" | "transform_formula" | "analyze.transform.formula" | "filter_formula" | "analyze.transform.filter_formula" => {
                if val.is_null() {
                    patch.insert(String::from("transform_formula"), Value::Null);
                } else {
                    let raw = val.as_str().ok_or_else(|| String::from("`formula` must be a string or null"))?.trim();
                    if raw.is_empty() {
                        patch.insert(String::from("transform_formula"), Value::Null);
                    } else {
                        patch.insert(String::from("transform_formula"), json!(raw));
                        if working.selected_entry_id.as_deref() != Some("transform.select") {
                            apply_transform_formula_str(raw, &mut formula_x, &mut formula_y)?;
                            saw_formula_transforms = true;
                        }
                    }
                }
            }
            "x_transform" | "analyze.transform.x_transform" | "transform_x" => {
                explicit_x_transform = Some(serde_json::to_value(parse_math_preset_opt(val)?).map_err(|e| e.to_string())?);
            }
            "y_transform" | "analyze.transform.y_transform" | "transform_y" => {
                explicit_y_transform = Some(serde_json::to_value(parse_math_preset_opt(val)?).map_err(|e| e.to_string())?);
            }
            _ => normalize_analyze_extended(&mut working, key, val, preset, &mut patch, &mut post_actions)?,
        }
    }

    if let Some(b) = wants_heatmap {
        if b && !eff_all_pairs {
            return Err(String::from("Create correlation heatmap is disabled in X vs every Y mode."));
        }
        patch.insert(String::from("correlation_heatmap"), json!(b));
    }
    if saw_formula_transforms {
        patch.insert(String::from("transform_x"), serde_json::to_value(formula_x).map_err(|e| e.to_string())?);
        patch.insert(String::from("transform_y"), serde_json::to_value(formula_y).map_err(|e| e.to_string())?);
    }
    if let Some(x_val) = explicit_x_transform {
        patch.insert(String::from("transform_x"), x_val);
    }
    if let Some(y_val) = explicit_y_transform {
        patch.insert(String::from("transform_y"), y_val);
    }

    let patched_hypothetical = patch.contains_key("two_group_hypothetical");
    merge_struct_json(&mut working, &patch, &[])?;
    working.explicit_response_column = saved_explicit_resp;
    working.force_create_new_once = saved_force_create_new_once;
    if patched_hypothetical
        && let Some(TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical }) = &mut working.two_group_method
    {
        *hypothetical = working.two_group_hypothetical;
    }
    post_actions.sort_by_key(|act| match act {
        AnalyzePostAction::SetResponseColumn(_) | AnalyzePostAction::SetPcrOutcome(_, _) => 0u8,
        AnalyzePostAction::ToggleGaussian(_) => 1,
        AnalyzePostAction::TogglePaired(_) => 2,
        AnalyzePostAction::SetTwoGroupMethod(_) => 3,
    });
    for act in post_actions {
        match act {
            AnalyzePostAction::SetResponseColumn(col) => working.set_response_column(col),
            AnalyzePostAction::SetPcrOutcome(col, want_on) => {
                if want_on {
                    check_pcr_target_leakage(col, working.is_pca_predictor(col))?;
                    working.pcr_outcome_column = Some(col);
                    working.set_response_column(col);
                } else if working.pcr_outcome_column == Some(col) {
                    working.pcr_outcome_column = None;
                }
            }
            AnalyzePostAction::ToggleGaussian(g) => {
                let paired = working.two_group_method.is_some_and(two_group_is_paired);
                working.toggle_two_group_design(g, paired);
            }
            AnalyzePostAction::TogglePaired(paired) => {
                let gaussian = working.two_group_method.is_none_or(two_group_is_gaussian);
                working.toggle_two_group_design(gaussian, paired);
            }
            AnalyzePostAction::SetTwoGroupMethod(raw) => {
                let parsed = parse_two_group_method_str(&raw, working.two_group_hypothetical)?;
                working.two_group_memory.remember(parsed);
                working.two_group_method = Some(parsed);
            }
        }
    }
    *state = working;
    Ok(())
}

enum AnalyzePostAction {
    SetResponseColumn(usize),
    SetPcrOutcome(usize, bool),
    ToggleGaussian(bool),
    TogglePaired(bool),
    SetTwoGroupMethod(String),
}

fn is_analyze_priority_key(key: &str) -> bool {
    matches!(
        key,
        "entry_id"
            | "selected_entry_id"
            | "analysis"
            | "analyze.entry"
            | "analyze.entries_list"
            | "selected_data_sets"
            | "analyze.selected_data_sets"
            | "analyze.transform.selected_data_sets"
            | "pca_predictors"
            | "analyze.pca_predictors"
            | "outlier_method"
            | "analyze.outlier.method"
            | "fit_method"
            | "analyze.fit.method"
    ) || key.strip_prefix("analyze.pca.pred.").is_some_and(|rest| rest.parse::<usize>().is_ok())
}

fn set_analyze_pca_predictor(working: &mut AnalyzeDialogState, col: usize, want_on: bool) {
    if want_on {
        if !working.pca_predictors.contains(&col) {
            working.pca_predictors.push(col);
            working.pca_predictors.sort_unstable();
        }
        if !working.selected_data_sets.contains(&col) {
            working.selected_data_sets.push(col);
            working.selected_data_sets.sort_unstable();
        }
    } else {
        working.pca_predictors.retain(|&c| c != col);
        working.selected_data_sets.retain(|&c| c != col);
    }
    for cand in &mut working.variable_candidates {
        cand.selected = working.selected_data_sets.contains(&cand.col_index);
    }
}

fn apply_analyze_pca_pred_keys(working: &mut AnalyzeDialogState, fields: &Map<String, Value>, has_explicit_pcr_outcome: bool) -> Result<(), String> {
    for (key, val) in fields {
        if let Some(col_raw) = key.strip_prefix("analyze.pca.pred.")
            && let Ok(col) = col_raw.parse::<usize>()
        {
            let want_on = if val.is_null() { !working.is_pca_predictor(col) } else { parse_bool_field(val, key)? };
            set_analyze_pca_predictor(working, col, want_on);
            if !has_explicit_pcr_outcome {
                working.clear_overlapping_pcr_outcome();
            }
        }
    }
    Ok(())
}

fn apply_analyze_priority(working: &mut AnalyzeDialogState, fields: &Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
    let has_explicit_predictors =
        ["selected_data_sets", "analyze.selected_data_sets", "pca_predictors", "analyze.pca_predictors"].iter().any(|k| fields.contains_key(*k));
    for key in ["entry_id", "selected_entry_id", "analysis", "analyze.entry", "analyze.entries_list"] {
        if let Some(val) = fields.get(key) {
            let s = parse_str_field(val, key)?.trim();
            if default_analysis_kind_for_id(s).is_none() {
                return Err(format!("unknown analysis catalog entry `{s}`"));
            }
            if s == "stats.pcr" && !has_explicit_predictors {
                working.select_catalog_entry(s);
            } else {
                working.selected_entry_id = Some(s.to_owned());
            }
        }
    }
    let has_explicit_pcr_outcome = fields.contains_key("pcr_outcome_column")
        || fields.contains_key("analyze.pcr_outcome_column")
        || (fields.contains_key("response_column") && matches!(working.selected_entry_id.as_deref(), Some("stats.pcr" | "pcr")))
        || fields.iter().any(|(k, v)| {
            k.strip_prefix("analyze.pcr.outcome.").is_some_and(|rest| rest.parse::<usize>().is_ok()) && parse_bool_field(v, k).ok() != Some(false)
        });
    for key in
        ["selected_data_sets", "analyze.selected_data_sets", "analyze.transform.selected_data_sets", "pca_predictors", "analyze.pca_predictors"]
    {
        if let Some(val) = fields.get(key) {
            let cols = parse_usize_list(val, key)?;
            working.selected_data_sets = cols.clone();
            if matches!(key, "pca_predictors" | "analyze.pca_predictors") || working.is_pca_or_pcr_entry() || has_explicit_pcr_outcome {
                working.pca_predictors = cols;
            }
            if !has_explicit_pcr_outcome {
                working.clear_overlapping_pcr_outcome();
            }
            for cand in &mut working.variable_candidates {
                cand.selected = working.selected_data_sets.contains(&cand.col_index);
            }
        }
    }
    apply_analyze_pca_pred_keys(working, fields, has_explicit_pcr_outcome)?;
    for key in ["outlier_method", "analyze.outlier.method", "fit_method", "analyze.fit.method", "method"] {
        if let Some(val) = fields.get(key) {
            if matches!(key, "outlier_method" | "analyze.outlier.method")
                || (key == "method" && working.selected_entry_id.as_deref() == Some("stats.outliers"))
            {
                let (m, iter_opt) = parse_outlier_method_str(parse_str_field(val, "outlier_method")?)?;
                working.outlier_method = m.to_owned();
                if let Some(it) = iter_opt {
                    working.outlier_iterative = it;
                }
            } else if matches!(key, "fit_method" | "analyze.fit.method")
                || (key == "method" && working.selected_entry_id.as_deref() == Some("fit.nonlinear"))
            {
                working.fit_method = parse_fit_method(val, preset)?;
            }
        }
    }
    Ok(())
}

fn normalize_analyze_extended(
    working: &mut AnalyzeDialogState,
    key: &str,
    val: &Value,
    preset: &LocalePreset,
    patch: &mut Map<String, Value>,
    post_actions: &mut Vec<AnalyzePostAction>,
) -> Result<(), String> {
    if let Some(col_raw) = key.strip_prefix("analyze.pcr.outcome.")
        && let Ok(col) = col_raw.parse::<usize>()
    {
        let want_on = parse_bool_field(val, key)?;
        post_actions.push(AnalyzePostAction::SetPcrOutcome(col, want_on));
        return Ok(());
    }
    if normalize_analyze_two_group_contingency(working, key, val, preset, patch, post_actions)?
        || normalize_analyze_mv_glm_cox_km(key, val, patch)?
        || normalize_analyze_outlier_pstack(working, key, val, preset, patch)?
    {
        return Ok(());
    }
    match key {
        "target_archetype" | "analyze.transform.target_archetype" => {
            let tk = parse_table_kind_str(parse_str_field(val, "target_archetype")?)?;
            patch.insert(String::from("transform_target_archetype"), serde_json::to_value(tk).map_err(|e| e.to_string())?);
        }
        "baseline_columns" | "analyze.transform.baseline_columns" => {
            patch.insert(String::from("baseline_columns"), json!(parse_usize_list(val, key)?));
        }
        "baseline_operation" | "operation" | "analyze.transform.baseline_operation" => {
            let op = parse_baseline_operation_str(parse_str_field(val, "baseline_operation")?)?;
            patch.insert(String::from("baseline_operation"), serde_json::to_value(op).map_err(|e| e.to_string())?);
        }
        "error_propagation" | "analyze.transform.error_propagation" => {
            let ep = parse_error_prop_value(val)?;
            patch.insert(String::from("transform_error_propagation"), serde_json::to_value(ep).map_err(|e| e.to_string())?);
        }
        "descriptive_subcolumn_agg" | "subcolumn_agg" | "analyze.descriptive.subcolumn_agg" => {
            let agg = parse_subcolumn_aggregation_val(val)?;
            patch.insert(String::from("descriptive_aggregation"), serde_json::to_value(agg).map_err(|e| e.to_string())?);
        }
        "custom_upper_percentile" | "analyze.descriptive.custom_upper_percentile" => {
            let pct = parse_opt_upper_percentile(val, key, preset)?;
            patch.insert(String::from("descriptive_custom_upper_percentile"), json!(pct));
        }
        other => return Err(format!("unknown Analyze field `{other}`")),
    }
    Ok(())
}

fn normalize_analyze_two_group_contingency(
    working: &AnalyzeDialogState,
    key: &str,
    val: &Value,
    preset: &LocalePreset,
    patch: &mut Map<String, Value>,
    post_actions: &mut Vec<AnalyzePostAction>,
) -> Result<bool, String> {
    match key {
        "assume_gaussian" | "analyze.assume_gaussian" => {
            post_actions.push(AnalyzePostAction::ToggleGaussian(parse_bool_field(val, "assume_gaussian")?));
        }
        "paired_or_matched" | "analyze.paired_or_matched" => {
            post_actions.push(AnalyzePostAction::TogglePaired(parse_bool_field(val, "paired_or_matched")?));
        }
        "two_group_method" | "analyze.two_group.method" | "analyze.two_group_method" => {
            post_actions.push(AnalyzePostAction::SetTwoGroupMethod(parse_str_field(val, "two_group_method")?.to_owned()));
        }
        "compute_effect_sizes" | "analyze.compute_effect_sizes" | "analyze.two_group.compute_effect_sizes" => {
            patch.insert(String::from("compute_effect_sizes"), json!(Some(parse_bool_field(val, "compute_effect_sizes")?)));
        }
        "hypothetical" | "two_group_hypothetical" | "analyze.two_group.hypothetical" => {
            patch.insert(String::from("two_group_hypothetical"), json!(parse_finite_f64_with_locale(val, key, preset)?));
        }
        "contingency_method" | "analyze.contingency.method" | "analyze.contingency_method" => {
            let cm = parse_contingency_method_name(parse_str_field(val, "contingency_method")?)?;
            patch.insert(String::from("contingency_method"), serde_json::to_value(Some(cm)).map_err(|e| e.to_string())?);
        }
        "kappa_weighting" | "analyze.contingency.kappa_weighting" => {
            let cm = ContingencyMethod::CohenKappa { weighting: parse_kappa_weighting(val)? };
            patch.insert(String::from("contingency_method"), serde_json::to_value(Some(cm)).map_err(|e| e.to_string())?);
        }
        "weighting" => {
            if working.selected_entry_id.as_deref() == Some("stats.contingency") {
                let cm = ContingencyMethod::CohenKappa { weighting: parse_kappa_weighting(val)? };
                patch.insert(String::from("contingency_method"), serde_json::to_value(Some(cm)).map_err(|e| e.to_string())?);
            } else {
                let fw = parse_weighting_scheme(val)?;
                patch.insert(String::from("fit_weighting"), serde_json::to_value(fw).map_err(|e| e.to_string())?);
            }
        }
        "fit_weighting" | "analyze.fit.weighting" => {
            let fw = parse_weighting_scheme(val)?;
            patch.insert(String::from("fit_weighting"), serde_json::to_value(fw).map_err(|e| e.to_string())?);
        }
        "null_proportion" | "analyze.contingency.null_proportion" | "analyze.null_proportion" => {
            patch.insert(String::from("contingency_null_proportion"), json!(parse_opt_finite_f64_with_locale(val, key, preset)?));
        }
        "expected_proportions" | "analyze.contingency.expected_proportions" | "analyze.expected_proportions" => {
            let ep = if val.is_null() { None } else { Some(parse_f64_list_with_locale(val, key, preset)?) };
            patch.insert(String::from("contingency_expected_proportions"), json!(ep));
        }
        "three_way" | "analyze.factorial.three_way" => {
            patch.insert(String::from("factorial_three_way"), json!(parse_bool_field(val, "three_way")?));
        }
        "repeated_measures" | "analyze.factorial.repeated_measures" => {
            let rm = parse_rm_design_str(parse_str_field(val, "repeated_measures")?)?;
            patch.insert(String::from("factorial_repeated_measures"), serde_json::to_value(rm).map_err(|e| e.to_string())?);
        }
        "residual_covariance" | "factorial_residual_covariance" | "analyze.factorial.residual_covariance" => {
            let rc = parse_residual_covariance_str(parse_str_field(val, "residual_covariance")?)?;
            patch.insert(String::from("factorial_residual_covariance"), serde_json::to_value(rc).map_err(|e| e.to_string())?);
        }
        "ss_type" | "sum_of_squares" | "analyze.factorial.ss_type" => {
            let ss = parse_ss_type_str(parse_str_field(val, "ss_type")?)?;
            patch.insert(String::from("factorial_ss_type"), serde_json::to_value(ss).map_err(|e| e.to_string())?);
        }
        "multiplicity_method" | "analyze.multiplicity_method" | "analyze.factorial.multiplicity" => {
            if working.selected_entry_id.as_deref() == Some("stats.pvalue_stack") {
                let psm = parse_pvalue_stack_method_val(val)?;
                patch.insert(String::from("pvalue_stack_method"), serde_json::to_value(psm).map_err(|e| e.to_string())?);
            } else {
                let parsed = parse_multiplicity_method_name(parse_str_field(val, "multiplicity_method")?, 0)?;
                let pv = serde_json::to_value(parsed).map_err(|e| e.to_string())?;
                patch.insert(String::from("factorial_multiplicity"), pv.clone());
                patch.insert(String::from("km_multiplicity_method"), pv);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn normalize_analyze_mv_glm_cox_km(key: &str, val: &Value, patch: &mut Map<String, Value>) -> Result<bool, String> {
    const RULES: &[FieldRule] = &[
        FieldRule::new(&["k_means_k", "k", "analyze.cluster.k_means_k"], "cluster_k_means_k", FieldKind::OptUsizeMin(1)),
        FieldRule::new(&["k_sweep_max", "analyze.cluster.k_sweep_max"], "cluster_k_sweep_max", FieldKind::OptUsizeMin(2)),
        FieldRule::new(&["standardize", "pca_standardize", "analyze.pca.standardize"], "pca_standardize", FieldKind::Bool),
        FieldRule::new(&["spearman_rank", "analyze.corr.spearman_rank", "analyze.correlation.spearman_rank"], "corr_spearman_rank", FieldKind::Bool),
        FieldRule::new(&["imputations_m", "m", "analyze.mi.imputations_m", "analyze.imputation.m"], "imputation_m", FieldKind::UsizeMin(1)),
        FieldRule::new(&["compute_vif", "analyze.glm.compute_vif"], "glm_compute_vif", FieldKind::Bool),
        FieldRule::new(
            &["trend_test", "km_trend_test", "analyze.km.trend_test", "log_rank_for_trend", "analyze.km.log_rank_for_trend"],
            "km_trend_test",
            FieldKind::Bool,
        ),
        FieldRule::new(&["mv_time_column", "km_mv_time_column", "analyze.km.mv_time_column"], "km_mv_time_column", FieldKind::OptUsizeMin(0)),
        FieldRule::new(&["mv_status_column", "km_mv_status_column", "analyze.km.mv_status_column"], "km_mv_status_column", FieldKind::OptUsizeMin(0)),
        FieldRule::new(
            &["log_rank_mantel_cox", "km_log_rank_mantel_cox", "analyze.km.log_rank_mantel_cox"],
            "km_log_rank_mantel_cox",
            FieldKind::Bool,
        ),
        FieldRule::new(
            &["gehan_breslow_wilcoxon", "km_gehan_breslow_wilcoxon", "analyze.km.gehan_breslow_wilcoxon"],
            "km_gehan_breslow_wilcoxon",
            FieldKind::Bool,
        ),
        FieldRule::new(&["mv_y_column", "analyze.fit.mv_y_column"], "mv_y_column", FieldKind::OptUsizeMin(0)),
        FieldRule::new(&["mv_group_column", "analyze.mv_group_column", "analyze.km.mv_group_column"], "mv_group_column", FieldKind::OptUsizeMin(0)),
        FieldRule::new(&["mv_response_column", "analyze.mv_response_column"], "mv_response_column", FieldKind::OptUsizeMin(0)),
        FieldRule::new(&["sample_size", "analyze.sample.sample_size"], "sample_size", FieldKind::UsizeMin(1)),
        FieldRule::new(&["seed", "sample_seed", "analyze.sample.seed"], "sample_seed", FieldKind::U64),
        FieldRule::new(&["stratify_column", "analyze.sample.stratify_column"], "sample_stratify_column", FieldKind::OptUsizeMin(0)),
        FieldRule::new(&["proportional_allocation", "analyze.sample.proportional_allocation"], "sample_proportional", FieldKind::Bool),
    ];
    if try_apply_rule(key, val, RULES, &LocalePreset::default(), patch)? {
        return Ok(true);
    }
    match key {
        "distance" | "cluster_distance" | "analyze.cluster.distance" => {
            let cd = parse_cluster_distance_str(parse_str_field(val, "distance")?)?;
            patch.insert(String::from("cluster_distance"), serde_json::to_value(cd).map_err(|e| e.to_string())?);
        }
        "linkage" | "cluster_linkage" | "analyze.cluster.linkage" => {
            let cl = parse_cluster_linkage_str(parse_str_field(val, "linkage")?)?;
            patch.insert(String::from("cluster_linkage"), serde_json::to_value(cl).map_err(|e| e.to_string())?);
        }
        "family" | "glm_family" | "analyze.glm.family" => {
            let gf = parse_glm_family_str(parse_str_field(val, "glm_family")?)?;
            patch.insert(String::from("glm_family"), serde_json::to_value(gf).map_err(|e| e.to_string())?);
        }
        "reference_levels" | "analyze.reference_levels" => {
            let rv = serde_json::to_value(parse_reference_levels(val)?).map_err(|e| e.to_string())?;
            patch.insert(String::from("glm_reference_levels"), rv.clone());
            patch.insert(String::from("cox_reference_levels"), rv);
        }
        "time_column" | "cox_time_column" | "analyze.cox.time_column" => {
            patch.insert(String::from("cox_time_column"), json!(Some(parse_usize_min(val, key, 0)?)));
        }
        "status_column" | "cox_status_column" | "analyze.cox.status_column" => {
            patch.insert(String::from("cox_status_column"), json!(Some(parse_usize_min(val, key, 0)?)));
        }
        "predictor_columns" | "cox_predictor_columns" | "analyze.cox.predictor_columns" => {
            patch.insert(String::from("cox_predictor_columns"), json!(parse_usize_list(val, key)?));
        }
        "tie_method" | "cox_tie_method" | "analyze.cox.tie_method" => {
            let tm = parse_cox_tie_method_str(parse_str_field(val, "tie_method")?)?;
            patch.insert(String::from("cox_tie_method"), serde_json::to_value(tm).map_err(|e| e.to_string())?);
        }
        "km_multiplicity_method" | "analyze.km.multiplicity_method" => {
            let mm = parse_multiplicity_method_name(parse_str_field(val, "multiplicity_method")?, 0)?;
            patch.insert(String::from("km_multiplicity_method"), serde_json::to_value(mm).map_err(|e| e.to_string())?);
        }
        "custom_dsl" | "analyze.fit.custom_dsl" => {
            let dsl = parse_opt_nonempty_string(val, key)?;
            patch.insert(String::from("custom_dsl"), json!(dsl));
        }
        "parameter_rules" | "analyze.fit.parameter_rules" => {
            patch.insert(String::from("parameter_rules"), serde_json::to_value(parse_parameter_rules(val)?).map_err(|e| e.to_string())?);
        }
        "mv_x_columns" | "analyze.fit.mv_x_columns" => {
            patch.insert(String::from("mv_x_columns"), json!(parse_usize_list(val, key)?));
        }
        "mv_group_columns" | "analyze.fit.mv_group_columns" => {
            patch.insert(String::from("mv_group_columns"), json!(parse_usize_list(val, key)?));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn normalize_analyze_outlier_pstack(
    working: &mut AnalyzeDialogState,
    key: &str,
    val: &Value,
    preset: &LocalePreset,
    patch: &mut Map<String, Value>,
) -> Result<bool, String> {
    let is_pstack = working.selected_entry_id.as_deref() == Some("stats.pvalue_stack");
    let is_outliers = working.selected_entry_id.as_deref() == Some("stats.outliers");
    let is_nonlinear = working.selected_entry_id.as_deref() == Some("fit.nonlinear");
    match key {
        "outlier_fdr_q_percent" | "analyze.outlier.fdr_q_percent" => {
            let q = parse_outlier_fdr_q_percent(val, preset)?;
            patch.insert(String::from("outlier_fdr_q_percent"), json!(q));
            if let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut working.fit_method
                && q <= 50.0
            {
                *fdr_q_percent = q;
            }
        }
        "analyze.fit.rout_q" => {
            let q = parse_nonlinear_rout_q_percent(val, preset)?;
            patch.insert(String::from("outlier_fdr_q_percent"), json!(q));
            if let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut working.fit_method {
                *fdr_q_percent = q;
            }
        }
        "outlier_alpha" | "analyze.outlier.alpha" => {
            patch.insert(String::from("outlier_alpha"), json!(parse_outlier_alpha(val, preset)?));
        }
        "outlier_iterative" | "analyze.outlier.iterative" => {
            patch.insert(String::from("outlier_iterative"), json!(parse_bool_field(val, "outlier_iterative")?));
        }
        "fdr_q_percent" | "q_percent" | "rout_q" if is_outliers || is_nonlinear => {
            if is_nonlinear {
                let q = parse_nonlinear_rout_q_percent(val, preset)?;
                let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut working.fit_method else {
                    return Err(String::from("`fdr_q_percent` requires ROUT outlier elimination fit method"));
                };
                *fdr_q_percent = q;
            } else if is_outliers {
                let q = parse_outlier_fdr_q_percent(val, preset)?;
                patch.insert(String::from("outlier_fdr_q_percent"), json!(q));
            }
        }
        "alpha" if is_outliers => {
            patch.insert(String::from("outlier_alpha"), json!(parse_outlier_alpha(val, preset)?));
        }
        "iterative" if is_outliers => {
            patch.insert(String::from("outlier_iterative"), json!(parse_bool_field(val, "iterative")?));
        }
        "method" if is_outliers || is_nonlinear => {}
        "pvalue_stack_method" | "analyze.pvalue_stack.method" | "analyze.pvalue_stack.multiplicity_method" => {
            let m = parse_pvalue_stack_method_val(val)?;
            patch.insert(String::from("pvalue_stack_method"), serde_json::to_value(m).map_err(|e| e.to_string())?);
        }
        "pvalue_stack_threshold" | "analyze.pvalue_stack.threshold" => {
            patch.insert(String::from("pvalue_stack_threshold"), json!(parse_open_unit_f64(val, key, preset)?));
        }
        "pvalue_stack_column" | "analyze.pvalue_stack.column" => {
            patch.insert(String::from("pvalue_stack_column"), json!(parse_opt_usize_min(val, key, 0)?));
        }
        "method" if is_pstack => {
            let m = parse_pvalue_stack_method_val(val)?;
            patch.insert(String::from("pvalue_stack_method"), serde_json::to_value(m).map_err(|e| e.to_string())?);
        }
        "threshold" | "alpha" | "q" | "fdr_q" if is_pstack => {
            patch.insert(String::from("pvalue_stack_threshold"), json!(parse_open_unit_f64(val, key, preset)?));
        }
        "column" if is_pstack => {
            patch.insert(String::from("pvalue_stack_column"), json!(parse_opt_usize_min(val, key, 0)?));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

pub fn patch_edit_params(state: &mut EditAnalysisParametersState, fields: &Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
    const PRIORITY_KEYS: [&str; 34] = [
        "outlier_method",
        "edit.outlier.method",
        "fit_method",
        "edit.fit.method",
        "method",
        "assume_gaussian",
        "edit.assume_gaussian",
        "paired_or_matched",
        "matched",
        "edit.paired_or_matched",
        "p_value_format",
        "edit.p_value_format",
        "two_group_method",
        "edit.two_group.method",
        "paired_sign_test",
        "edit.two_group.paired_sign_test",
        "anova_method",
        "one_way_method",
        "edit.anova_method",
        "edit.one_way_method",
        "mixed_effects_reml",
        "edit.rm.mixed_effects_reml",
        "residual_covariance",
        "factorial_residual_covariance",
        "edit.rm.residual_covariance",
        "edit.reml.residual_covariance",
        "edit.factorial.residual_covariance",
        "correlation_all_pairs",
        "edit.correlation_all_pairs",
        "selected_data_sets",
        "edit.selected_data_sets",
        "edit.transform.selected_data_sets",
        "pca_predictors",
        "edit.pca.predictors",
    ];
    let mut working = state.clone();
    let has_explicit_pcr_outcome = fields.contains_key("pcr_outcome_column") || fields.contains_key("edit.pcr.outcome_column");
    let has_explicit_all_pairs = fields.contains_key("correlation_all_pairs") || fields.contains_key("edit.correlation_all_pairs");
    let was_corr_matrix =
        working.draft.as_ref().is_some_and(|d| matches!(d.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. })));
    for key in PRIORITY_KEYS {
        if let Some(val) = fields.get(key) {
            apply_edit_param_entry(&mut working, key, val, preset, has_explicit_pcr_outcome)?;
        }
    }
    let pinned_all_pairs = (has_explicit_all_pairs || was_corr_matrix).then_some(working.correlation_all_pairs);
    for (key, val) in fields {
        if !PRIORITY_KEYS.contains(&key.as_str()) {
            apply_edit_param_entry(&mut working, key, val, preset, has_explicit_pcr_outcome)?;
            if let Some(b) = pinned_all_pairs {
                working.correlation_all_pairs = b;
                if !b {
                    working.correlation_heatmap = false;
                }
            }
        }
    }
    if let Some(spec) = &working.draft {
        validate_edit_draft_bounds(&spec.kind)?;
    }
    *state = working;
    Ok(())
}

fn validate_edit_draft_bounds(kind: &AnalysisKind) -> Result<(), String> {
    let plot_bounds = match kind {
        AnalysisKind::LinearRegression(lr) => (lr.plot_x_min, lr.plot_x_max),
        AnalysisKind::SimpleLogisticRegression(sl) => (sl.plot_x_min, sl.plot_x_max),
        AnalysisKind::NonlinearRegression(nl) => (nl.plot_x_min, nl.plot_x_max),
        AnalysisKind::FitSplineLowess(sp) => (sp.plot_x_min, sp.plot_x_max),
        _ => (None, None),
    };
    if let (Some(lo), Some(hi)) = plot_bounds
        && lo >= hi
    {
        return Err(format!("invalid plot X range: `plot_x_min` ({lo}) must be strictly less than `plot_x_max` ({hi})"));
    }
    Ok(())
}

fn apply_edit_param_entry(
    working: &mut EditAnalysisParametersState,
    key: &str,
    val: &Value,
    preset: &LocalePreset,
    has_explicit_pcr_outcome: bool,
) -> Result<(), String> {
    if apply_edit_top_level_field(working, key, val, preset)?
        || apply_edit_two_group_and_pca_field(working, key, val, has_explicit_pcr_outcome)?
        || apply_edit_contingency_field(working, key, val, preset)?
        || apply_edit_draft_kind_field(working, key, val, preset)?
    {
        return Ok(());
    }
    Err(format!("unknown Edit Analysis Parameters field `{key}`"))
}

fn apply_edit_top_level_field(working: &mut EditAnalysisParametersState, key: &str, val: &Value, preset: &LocalePreset) -> Result<bool, String> {
    let mut top_patch = Map::new();
    match key {
        "active_tab" | "tab" => {
            let t = parse_str_field(val, key)?;
            if t.eq_ignore_ascii_case("repeated_measures") && !working.paired_or_matched {
                return Err(String::from("Repeated Measures tab is disabled when No matching is selected."));
            }
            top_patch.insert(String::from("active_tab"), json!(t));
        }
        "assume_gaussian" | "edit.assume_gaussian" => {
            apply_edit_gaussian_toggle(working, parse_bool_field(val, "assume_gaussian")?);
            return Ok(true);
        }
        "paired_or_matched" | "matched" | "edit.paired_or_matched" => {
            let matched = parse_bool_field(val, "paired_or_matched")?;
            top_patch.insert(String::from("paired_or_matched"), json!(matched));
            merge_struct_json(working, &top_patch, &[])?;
            apply_edit_matched_toggle(working, matched);
            return Ok(true);
        }
        "compare_against_control" | "edit.compare_against_control" => {
            let cmp = parse_bool_field(val, "compare_against_control")?;
            top_patch.insert(String::from("compare_against_control"), json!(cmp));
            merge_struct_json(working, &top_patch, &[])?;
            if cmp {
                set_edit_anova_multiplicity(working, MultiplicityMethod::Dunnett { control_column: working.control_column });
            }
            return Ok(true);
        }
        "control_column" | "edit.control_column" => {
            let col = usize::try_from(val.as_u64().ok_or_else(|| String::from("`control_column` must be a non-negative integer"))?)
                .map_err(|_| String::from("`control_column` exceeds usize"))?;
            top_patch.insert(String::from("control_column"), json!(col));
            merge_struct_json(working, &top_patch, &[])?;
            if working.compare_against_control {
                set_edit_anova_multiplicity(working, MultiplicityMethod::Dunnett { control_column: col });
            }
            return Ok(true);
        }
        "correlation_all_pairs" | "edit.correlation_all_pairs" => {
            let b = parse_bool_field(val, key)?;
            top_patch.insert(String::from("correlation_all_pairs"), json!(b));
            if !b {
                top_patch.insert(String::from("correlation_heatmap"), json!(false));
            }
        }
        "correlation_heatmap" | "edit.correlation_heatmap" | "edit.correlation.heatmap" => {
            let b = parse_bool_field(val, key)?;
            if b && !working.correlation_all_pairs {
                return Err(String::from("Create correlation heatmap is disabled in X vs every Y mode."));
            }
            top_patch.insert(String::from("correlation_heatmap"), json!(b));
        }
        "p_value_format" | "edit.p_value_format" => {
            let s = parse_str_field(val, key)?;
            let trimmed = s.trim();
            let next = match norm_key(trimmed).as_str() {
                "apa" => PValueFormat::Apa,
                "nejm" => PValueFormat::Nejm,
                "custom" => PValueFormat::new_custom(4, None),
                "fourdecimal" | "fourdecimals" | "default" | "standard" => PValueFormat::FourDecimal,
                _ if trimmed == "0.0001" => PValueFormat::FourDecimal,
                _ => return Err(format!("unknown P-value format `{s}`")),
            };
            working.set_p_value_format(next);
            return Ok(true);
        }
        "p_value_decimals" | "edit.p_value_digits" | "edit.pvalue.digits" | "edit.pvalue.digits.val" => {
            let d = u8::try_from(parse_usize_min(val, key, 0)?).map_err(|_| String::from("`p_value_decimals` out of range"))?;
            if let Some(spec) = &mut working.draft {
                let PValueFormat::Custom { scientific_below, .. } = spec.p_value_format else {
                    return Err(String::from("Custom decimal precision spinner is disabled unless Custom P-value format is selected."));
                };
                spec.p_value_format = PValueFormat::new_custom(d, scientific_below);
            }
            return Ok(true);
        }
        "confidence_percent" | "edit.confidence_percent" => {
            if !val.is_null() {
                let c = parse_finite_f64_with_locale(val, key, preset)?;
                working.set_confidence_percent(c)?;
            }
            return Ok(true);
        }
        "custom_title" | "edit.custom_title" => {
            let parsed = match val {
                Value::Null => None,
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => return Err(String::from("`custom_title` must be a string or null")),
            };
            working.set_custom_title(parsed)?;
            return Ok(true);
        }
        _ => return Ok(false),
    }
    merge_struct_json(working, &top_patch, &[])?;
    Ok(true)
}

fn apply_edit_two_group_and_pca_field(
    working: &mut EditAnalysisParametersState,
    key: &str,
    val: &Value,
    has_explicit_pcr_outcome: bool,
) -> Result<bool, String> {
    let two_method = working.draft.as_ref().and_then(|s| match &s.kind {
        AnalysisKind::TwoGroupComparison(two) => Some(two.method),
        _ => None,
    });
    let mut top_patch = Map::new();
    match key {
        "welch_correction" | "edit.welch_correction" | "edit.two_group.welch" => {
            let w = parse_bool_field(val, "welch_correction")?;
            if let Some(method) = two_method {
                if !crate::modal_predicates::r4_two_group_welch_correction(method).enabled {
                    if w {
                        return Err(String::from("Welch's correction is disabled for paired or nonparametric tests."));
                    }
                    working.welch_correction = false;
                    return Ok(true);
                }
                top_patch.insert(String::from("welch_correction"), json!(w));
                merge_struct_json(working, &top_patch, &[])?;
                working.update_draft_kind(|k| match k {
                    AnalysisKind::TwoGroupComparison(two) => {
                        if w {
                            two.method = TwoGroupMethod::WelchT;
                        } else if matches!(two.method, TwoGroupMethod::WelchT | TwoGroupMethod::UnpairedT) {
                            two.method = TwoGroupMethod::UnpairedT;
                        }
                        true
                    }
                    _ => false,
                });
                working.sync_rule_flags_from_kind();
            }
            return Ok(true);
        }
        "graph_differences" | "edit.two_group.graph_differences" => {
            let b = parse_bool_field(val, "graph_differences")?;
            if b && two_method.is_some_and(|m| !crate::modal_predicates::r4_two_group_graph_differences(m).enabled) {
                return Err(String::from("Estimation plot of differences is disabled in nonparametric/rank mode."));
            }
            top_patch.insert(String::from("graph_differences"), json!(b));
        }
        "graph_ranks" | "edit.two_group.graph_ranks" => {
            let b = parse_bool_field(val, "graph_ranks")?;
            if b && two_method.is_some_and(|m| !crate::modal_predicates::r4_two_group_nonparametric_options(m).enabled) {
                return Err(String::from("Rank-based option is disabled in Gaussian parametric mode."));
            }
            top_patch.insert(String::from("graph_ranks"), json!(b));
        }
        "hodges_lehmann_ci" | "hodges_lehmann" | "edit.two_group.hodges_lehmann" => {
            let b = parse_bool_field(val, "hodges_lehmann_ci")?;
            if b && two_method.is_some_and(|m| !crate::modal_predicates::r4_two_group_nonparametric_options(m).enabled) {
                return Err(String::from("Hodges-Lehmann CI is disabled in Gaussian parametric mode."));
            }
            top_patch.insert(String::from("hodges_lehmann_ci"), json!(b));
        }
        "selected_data_sets" | "edit.selected_data_sets" | "edit.transform.selected_data_sets" | "pca_predictors" | "edit.pca.predictors" => {
            let cols = parse_usize_list(val, key)?;
            let is_pca_key = matches!(key, "pca_predictors" | "edit.pca.predictors");
            let is_pca_draft = working.draft.as_ref().is_some_and(|s| matches!(s.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { .. })));
            if is_pca_key || is_pca_draft || (working.draft.is_none() && has_explicit_pcr_outcome) {
                working.pca_predictors.clone_from(&cols);
                if !has_explicit_pcr_outcome && working.pcr_outcome_column.is_some_and(|y| working.pca_predictors.contains(&y)) {
                    working.set_draft_pcr_outcome(None);
                }
            }
            if let Some(spec) = &mut working.draft {
                spec.selected_data_sets = cols;
            }
            return Ok(true);
        }
        "pcr_outcome_column" | "edit.pcr.outcome_column" => {
            if let Some(col) = parse_opt_usize_min(val, key, 0)? {
                check_pcr_target_leakage(col, working.pca_predictors.contains(&col))?;
                working.set_draft_pcr_outcome(Some(col));
            } else {
                working.set_draft_pcr_outcome(None);
            }
            return Ok(true);
        }
        _ => return Ok(false),
    }
    merge_struct_json(working, &top_patch, &[])?;
    Ok(true)
}

fn apply_edit_contingency_field(working: &mut EditAnalysisParametersState, key: &str, val: &Value, preset: &LocalePreset) -> Result<bool, String> {
    if !working.draft.as_ref().is_some_and(|s| matches!(s.kind, AnalysisKind::Contingency(_))) {
        return Ok(false);
    }
    let method_flag = match key {
        "fisher" | "edit.contingency.fisher" => Some(ContingencyMethod::FisherExact),
        "barnard" | "edit.contingency.barnard" | "edit.contingency.barnard_exact" => Some(ContingencyMethod::BarnardExact),
        "likelihood_ratio_g2" | "g2" | "edit.contingency.g2" | "edit.contingency.likelihood_g2" => Some(ContingencyMethod::LikelihoodRatioG2),
        "mcnemar" | "edit.contingency.mcnemar" => Some(ContingencyMethod::McNemarPaired),
        "cochran_armitage" | "edit.contingency.cochran_armitage" => Some(ContingencyMethod::CochranArmitageTrend),
        "cohen_kappa" | "edit.contingency.cohen_kappa" => Some(ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted }),
        "fleiss_kappa" | "edit.contingency.fleiss_kappa" => Some(ContingencyMethod::FleissKappa),
        "exact_binomial" | "edit.contingency.exact_binomial" => Some(ContingencyMethod::ExactBinomial),
        _ => None,
    };
    if let Some(target_method) = method_flag {
        let b = parse_bool_field(val, key)?;
        let next = if b { target_method } else { ContingencyMethod::PearsonChiSquare { yates_continuity: false } };
        update_contingency_spec(working, |c| c.method = next);
        return Ok(true);
    }
    let mut top_patch = Map::new();
    match key {
        "kappa_weighting" | "edit.contingency.kappa_weighting" => {
            let w = parse_kappa_weighting(val)?;
            update_contingency_spec(working, |c| c.method = ContingencyMethod::CohenKappa { weighting: w });
            return Ok(true);
        }
        "weighting" if working.draft.as_ref().is_some_and(|s| matches!(s.kind, AnalysisKind::Contingency(_))) => {
            let w = parse_kappa_weighting(val)?;
            update_contingency_spec(working, |c| c.method = ContingencyMethod::CohenKappa { weighting: w });
            return Ok(true);
        }
        "null_proportion" | "edit.contingency.null_proportion" => {
            let p = parse_opt_finite_f64_with_locale(val, key, preset)?;
            if let Some(v) = p
                && !(0.0 < v && v < 1.0)
            {
                return Err(format!("`null_proportion` ({v}) must be strictly between 0 and 1"));
            }
            update_contingency_spec(working, |c| c.null_proportion = p);
            return Ok(true);
        }
        "expected_proportions" | "edit.contingency.expected_proportions" => {
            let props = if val.is_null() { None } else { Some(parse_f64_list_with_locale(val, key, preset)?) };
            update_contingency_spec(working, |c| c.expected_proportions = props);
            return Ok(true);
        }
        "edit.contingency.ci.baptista_pike" => {
            top_patch.insert(String::from("contingency_or_ci_method"), json!(parse_ci_method_choice(val, key, "OR", &["Baptista-Pike", "Woolf"])?));
        }
        "edit.contingency.ci.koopman" => {
            top_patch.insert(String::from("contingency_rr_ci_method"), json!(parse_ci_method_choice(val, key, "RR", &["Koopman", "Katz"])?));
        }
        "edit.contingency.ci.newcombe_wilson" => {
            top_patch
                .insert(String::from("contingency_arr_ci_method"), json!(parse_ci_method_choice(val, key, "ARR", &["Newcombe-Wilson", "Wald"])?));
        }
        "edit.contingency.ci.wilson_brown" => {
            top_patch.insert(
                String::from("contingency_diag_ci_method"),
                json!(parse_ci_method_choice(val, key, "diagnostic", &["Wilson-Brown", "Clopper-Pearson"])?),
            );
        }
        "compute_odds_and_risk_ratios" | "contingency_odds_ratio_checked" | "edit.contingency.odds_ratio" => {
            let b = parse_bool_field(val, key)?;
            top_patch.insert(String::from("contingency_odds_ratio_checked"), json!(b));
            top_patch.insert(String::from("contingency_relative_risk_checked"), json!(b));
            if !b {
                top_patch.insert(String::from("contingency_attributable_risk_checked"), json!(false));
            }
            merge_struct_json(working, &top_patch, &[])?;
            update_contingency_spec(working, |c| c.compute_odds_and_risk_ratios = b);
            return Ok(true);
        }
        "contingency_attributable_risk_checked" | "attributable_risk" | "compute_attributable_risk" | "edit.contingency.attributable_risk" => {
            let b = parse_bool_field(val, key)?;
            top_patch.insert(String::from("contingency_attributable_risk_checked"), json!(b));
            merge_struct_json(working, &top_patch, &[])?;
            if b {
                update_contingency_spec(working, |c| c.compute_odds_and_risk_ratios = true);
            } else if !working.contingency_odds_ratio_checked && !working.contingency_relative_risk_checked {
                update_contingency_spec(working, |c| c.compute_odds_and_risk_ratios = false);
            }
            return Ok(true);
        }
        "compute_sensitivity_specificity" | "sensitivity_specificity" | "edit.contingency.sensitivity_specificity" => {
            let b = parse_bool_field(val, key)?;
            update_contingency_spec(working, |c| c.compute_sensitivity_specificity = b);
            return Ok(true);
        }
        "contingency_method" => {
            let raw = parse_str_field(val, key)?;
            let method = parse_contingency_method_name(raw)?;
            update_contingency_spec(working, |c| c.method = method);
            return Ok(true);
        }
        _ => return Ok(false),
    }
    merge_struct_json(working, &top_patch, &[])?;
    Ok(true)
}

fn update_contingency_spec(working: &mut EditAnalysisParametersState, mutate: impl FnOnce(&mut ContingencySpec)) {
    working.update_draft_kind(|k| match k {
        AnalysisKind::Contingency(c) => {
            mutate(c);
            true
        }
        _ => false,
    });
}

fn apply_edit_draft_kind_field(working: &mut EditAnalysisParametersState, key: &str, val: &Value, preset: &LocalePreset) -> Result<bool, String> {
    if apply_edit_anova_and_posthoc_field(working, key, val)? || apply_edit_transform_field(working, key, val)? {
        return Ok(true);
    }
    let Some(mut spec) = working.draft.take() else {
        return Ok(false);
    };
    let mut updated = spec.kind.clone();
    let ctrl = working.control_column;
    let res = match &mut updated {
        AnalysisKind::DescriptiveStats(desc) => match key {
            "descriptive_subcolumn_agg" | "subcolumn_agg" | "edit.descriptive.subcolumn_agg" => {
                desc.aggregation = parse_subcolumn_aggregation_val(val)?;
                Ok(true)
            }
            "custom_upper_percentile" | "edit.desc.custom_pct" | "edit.descriptive.custom_upper_percentile" => {
                desc.custom_upper_percentile = parse_opt_upper_percentile(val, key, preset)?;
                Ok(true)
            }
            _ => Ok(false),
        },
        AnalysisKind::TwoGroupComparison(t) => patch_two_group_spec(key, val, preset, t),
        AnalysisKind::OneWayAnova(a) => patch_one_way_spec(key, val, a),
        AnalysisKind::FactorialAnova(f) => patch_factorial_spec(key, val, ctrl, f),
        AnalysisKind::NestedComparison(n) => match key {
            "df_method" | "edit.rm.df_method" => {
                n.df_method = parse_df_method_str(parse_str_field(val, "df_method")?)?;
                Ok(true)
            }
            _ => Ok(false),
        },
        AnalysisKind::LinearRegression(lr) => patch_linear_regression_spec(key, val, preset, lr),
        AnalysisKind::SimpleLogisticRegression(sl) => patch_simple_logistic_spec(key, val, preset, sl),
        AnalysisKind::RocCurve(roc) => patch_roc_spec(key, val, roc),
        AnalysisKind::MultipleRowTTests(mrt) => patch_multiple_row_spec(key, val, preset, ctrl, mrt),
        AnalysisKind::BlandAltman(ba) => patch_bland_altman_spec(key, val, ba),
        AnalysisKind::PassingBablok(pb) => patch_passing_bablok_spec(key, val, pb),
        AnalysisKind::PValueStack(ps) => match key {
            "method"
            | "pvalue_stack_method"
            | "multiplicity_method"
            | "multiplicity"
            | "edit.pvalue_stack.method"
            | "edit.pvalue_stack.multiplicity_method" => {
                ps.method = parse_pvalue_stack_method_val(val)?;
                Ok(true)
            }
            "threshold" | "pvalue_stack_threshold" | "alpha" | "q" | "fdr_q" | "edit.pvalue_stack.threshold" | "edit.pvalue_stack.threshold.val" => {
                ps.threshold = parse_open_unit_f64(val, key, preset)?;
                Ok(true)
            }
            "column" | "pvalue_stack_column" | "edit.pvalue_stack.column" => {
                ps.column = parse_opt_usize_min(val, key, 0)?;
                Ok(true)
            }
            _ => Ok(false),
        },
        AnalysisKind::PowerAnalysis(pa) => patch_power_spec(key, val, preset, pa),
        AnalysisKind::LeveneTest(lt) => match key {
            "center" | "levene_center" | "edit.levene.center" => {
                lt.center = serde_json::from_value(val.clone()).map_err(|e| format!("invalid Levene `center` (expected `median` or `mean`): {e}"))?;
                Ok(true)
            }
            _ => Ok(false),
        },
        AnalysisKind::FitSplineLowess(sp) => patch_spline_lowess_spec(key, val, preset, sp),
        AnalysisKind::AreaUnderCurve(auc) => patch_area_under_curve_spec(key, val, preset, auc),
        AnalysisKind::Multivariate(mv) => patch_multivariate_spec(key, val, mv),
        AnalysisKind::GeneralizedLinearModel(glm) => patch_glm_spec(key, val, glm),
        AnalysisKind::Survival(surv) => patch_survival_spec(key, val, ctrl, surv),
        AnalysisKind::NonlinearRegression(fit) => patch_nonlinear_spec(key, val, preset, fit),
        AnalysisKind::OutlierDetection(out) => patch_outlier_spec(working, key, val, preset, out),
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { sample_size, seed, stratify_column, proportional_allocation }) => match key {
            "sample_size" | "edit.sample.sample_size" => {
                *sample_size = parse_usize_min(val, key, 1)?;
                Ok(true)
            }
            "seed" | "edit.sample.seed" => {
                *seed = parse_u64_val(val, key)?;
                Ok(true)
            }
            "stratify_column" | "edit.sample.stratify_column" => {
                *stratify_column = parse_opt_usize_min(val, key, 0)?;
                Ok(true)
            }
            "proportional_allocation" | "edit.sample.proportional_allocation" => {
                *proportional_allocation = parse_bool_field(val, "proportional_allocation")?;
                Ok(true)
            }
            _ => Ok(false),
        },
        _ => Ok(false),
    };
    let handled = match res {
        Ok(h) => h,
        Err(e) => {
            working.draft = Some(spec);
            return Err(e);
        }
    };
    if handled {
        spec.update_kind(updated);
    }
    working.draft = Some(spec);
    if handled {
        working.sync_rule_flags_from_kind();
    }
    Ok(handled)
}

fn apply_edit_anova_and_posthoc_field(working: &mut EditAnalysisParametersState, key: &str, val: &Value) -> Result<bool, String> {
    let is_one_way = working.draft.as_ref().is_some_and(|s| matches!(s.kind, AnalysisKind::OneWayAnova(_)));
    let has_posthoc = working.draft.as_ref().is_some_and(|s| {
        matches!(
            s.kind,
            AnalysisKind::OneWayAnova(_)
                | AnalysisKind::FactorialAnova(_)
                | AnalysisKind::NestedComparison(_)
                | AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { .. })
        )
    });
    match key {
        "geisser_greenhouse" | "edit.geisser_greenhouse" | "edit.sphericity.geisser_greenhouse" if is_one_way => {
            let gg = parse_bool_field(val, key)?;
            if !working.paired_or_matched {
                return Err(String::from("Geisser-Greenhouse sphericity control is disabled when No matching is selected."));
            }
            working.try_update_draft_kind(|k| match k {
                AnalysisKind::OneWayAnova(a) => {
                    if a.mixed_effects_reml || a.residual_covariance == ResidualCovariance::Unstructured {
                        return Err(String::from("Sphericity corrections are disabled when Mixed-Effects REML is active."));
                    }
                    a.method = OneWayMethod::RepeatedMeasures { geisser_greenhouse: gg };
                    Ok(true)
                }
                _ => Ok(false),
            })?;
        }
        "multiplicity_method"
            if working.draft.as_ref().is_some_and(|s| {
                matches!(s.kind, AnalysisKind::OneWayAnova(_) | AnalysisKind::FactorialAnova(_) | AnalysisKind::NestedComparison(_))
            }) =>
        {
            let raw = parse_str_field(val, key)?;
            let method = parse_multiplicity_method_name(raw, working.control_column)?;
            apply_edit_checked_post_hoc(working, method, "Post-hoc method disabled.")?;
        }
        k if has_posthoc && (k.starts_with("edit.post_hoc.") || k.starts_with("edit.posthoc.")) => {
            let method = post_hoc_method_for_widget_key(k, working.control_column)
                .ok_or_else(|| format!("unknown parameter field `{k}` in EditAnalysisParameters"))?;
            if parse_bool_field(val, k)? {
                apply_edit_checked_post_hoc(working, method, "Post-hoc method is disabled.")?;
            }
        }
        "anova_method" | "one_way_method" | "edit.anova_method" | "edit.one_way_method" if is_one_way => {
            let next_method = parse_one_way_method_str(parse_str_field(val, "anova_method")?, working.saved_rm_geisser_greenhouse)?;
            if let Some(mut spec) = working.draft.take() {
                working.remember_submethods_from_kind(&spec.kind);
                if let AnalysisKind::OneWayAnova(mut a) = spec.kind.clone() {
                    a.method = next_method;
                    spec.update_kind(AnalysisKind::OneWayAnova(a));
                }
                working.draft = Some(spec);
                working.sync_rule_flags_from_kind();
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn apply_edit_transform_field(working: &mut EditAnalysisParametersState, key: &str, val: &Value) -> Result<bool, String> {
    if !working.draft.as_ref().is_some_and(|s| matches!(s.kind, AnalysisKind::Transform(_))) {
        return Ok(false);
    }
    working.try_update_draft_kind(|k| match (key, k) {
        ("x_transform" | "edit.transform.x_transform", AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, .. })) => {
            *x_transform = parse_math_preset_opt(val)?;
            Ok(true)
        }
        ("y_transform" | "edit.transform.y_transform", AnalysisKind::Transform(TransformAnalysisSpec::Math { y_transform, .. })) => {
            *y_transform = parse_math_preset_opt(val)?;
            Ok(true)
        }
        (
            "error_propagation" | "edit.transform.error_propagation",
            AnalysisKind::Transform(TransformAnalysisSpec::Math { error_propagation, .. }),
        ) => {
            *error_propagation = parse_error_prop_value(val)?;
            Ok(true)
        }
        (
            "formula" | "transform_formula" | "edit.transform.formula",
            AnalysisKind::Transform(TransformAnalysisSpec::Math { x_transform, y_transform, .. }),
        ) => {
            let raw = parse_str_field(val, key)?.trim();
            if raw.is_empty() {
                return Err(format!("`{key}` cannot be empty"));
            }
            apply_transform_formula_str(raw, x_transform, y_transform)?;
            Ok(true)
        }
        (
            "formula" | "transform_formula" | "edit.transform.formula" | "filter_formula" | "edit.transform.filter_formula",
            AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { filter_formula, .. }),
        ) => {
            *filter_formula = parse_opt_nonempty_string(val, key)?;
            Ok(true)
        }
        (
            "target_archetype" | "edit.transform.target_archetype",
            AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform { target_archetype, .. }),
        ) => {
            *target_archetype = parse_table_kind_str(parse_str_field(val, "target_archetype")?)?;
            Ok(true)
        }
        (
            "baseline_columns" | "edit.transform.baseline_columns",
            AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { baseline_columns, .. }),
        ) => {
            *baseline_columns = parse_usize_list(val, key)?;
            Ok(true)
        }
        (
            "baseline_operation" | "operation" | "edit.transform.baseline_operation",
            AnalysisKind::Transform(TransformAnalysisSpec::RemoveBaseline { operation, .. }),
        ) => {
            *operation = parse_baseline_operation_str(parse_str_field(val, "baseline_operation")?)?;
            Ok(true)
        }
        _ => Ok(false),
    })
}

fn apply_edit_gaussian_toggle(working: &mut EditAnalysisParametersState, assume_gaussian: bool) {
    working.assume_gaussian = assume_gaussian;
    let matched = working.paired_or_matched;
    let ctrl = working.control_column;
    let Some(mut spec) = working.draft.take() else {
        return;
    };
    working.remember_submethods_from_kind(&spec.kind);
    let mut k = spec.kind.clone();
    crate::modal_predicates::apply_r2_gaussian_toggle(&mut k, assume_gaussian, matched, ctrl);
    match &mut k {
        AnalysisKind::OneWayAnova(a) => {
            if assume_gaussian {
                a.method = if matched {
                    OneWayMethod::RepeatedMeasures { geisser_greenhouse: working.saved_rm_geisser_greenhouse }
                } else {
                    working.saved_one_way_param_unpaired
                };
            }
        }
        AnalysisKind::TwoGroupComparison(t) => {
            t.method = working.two_group_memory.resolve(assume_gaussian, matched, Some(t.method));
        }
        _ => {}
    }
    spec.update_kind(k);
    working.draft = Some(spec);
    working.sync_rule_flags_from_kind();
}

fn apply_edit_matched_toggle(working: &mut EditAnalysisParametersState, matched: bool) {
    let Some(mut spec) = working.draft.take() else {
        return;
    };
    working.remember_submethods_from_kind(&spec.kind);
    let mut updated = spec.kind.clone();
    match &mut updated {
        AnalysisKind::OneWayAnova(a) => {
            a.method = match (working.assume_gaussian, matched) {
                (true, true) => OneWayMethod::RepeatedMeasures { geisser_greenhouse: working.saved_rm_geisser_greenhouse },
                (true, false) => working.saved_one_way_param_unpaired,
                (false, true) => OneWayMethod::Friedman,
                (false, false) => OneWayMethod::KruskalWallis,
            };
        }
        AnalysisKind::TwoGroupComparison(t) => {
            t.method = working.two_group_memory.resolve(working.assume_gaussian, matched, Some(t.method));
        }
        AnalysisKind::FactorialAnova(f) => {
            f.repeated_measures = if matched { RepeatedMeasuresDesign::RowFactorMatched } else { RepeatedMeasuresDesign::None };
        }
        _ => {}
    }
    spec.update_kind(updated);
    working.draft = Some(spec);
    working.sync_rule_flags_from_kind();
}

fn apply_edit_checked_post_hoc(working: &mut EditAnalysisParametersState, method: MultiplicityMethod, fallback_msg: &str) -> Result<(), String> {
    let draft_reml = working.draft.as_ref().is_some_and(draft_forces_reml_mode);
    let gate = crate::modal_predicates::r1b_post_hoc_option_enabled(working.assume_gaussian, draft_reml, method);
    if !gate.enabled {
        return Err(gate.disabled_reason.unwrap_or_else(|| String::from(fallback_msg)));
    }
    set_edit_anova_multiplicity(working, method);
    Ok(())
}

fn set_edit_anova_multiplicity(working: &mut EditAnalysisParametersState, method: MultiplicityMethod) {
    working.update_draft_kind(|updated| {
        match updated {
            AnalysisKind::OneWayAnova(a) => a.multiplicity.method = method,
            AnalysisKind::FactorialAnova(f) => f.multiplicity.method = method,
            AnalysisKind::NestedComparison(n) => n.multiplicity.method = method,
            AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => multiplicity.method = method,
            _ => {}
        }
        true
    });
    if working.draft.is_some() {
        working.sync_rule_flags_from_kind();
    }
}

fn patch_outlier_spec(
    working: &mut EditAnalysisParametersState,
    key: &str,
    val: &Value,
    preset: &LocalePreset,
    out: &mut OutlierSpec,
) -> Result<bool, String> {
    match key {
        "outlier_method" | "edit.outlier.method" | "method" => {
            match *out {
                OutlierSpec::Rout { fdr_q_percent } => working.saved_outlier_rout_q = fdr_q_percent,
                OutlierSpec::Grubbs { alpha, iterative } => {
                    working.saved_outlier_grubbs_alpha = alpha;
                    working.saved_outlier_grubbs_iterative = iterative;
                }
            }
            let (mode, iter_opt) = parse_outlier_method_str(parse_str_field(val, "outlier_method")?)?;
            *out = if mode == "grubbs" {
                OutlierSpec::Grubbs {
                    alpha: working.saved_outlier_grubbs_alpha,
                    iterative: iter_opt.unwrap_or(working.saved_outlier_grubbs_iterative),
                }
            } else {
                OutlierSpec::Rout { fdr_q_percent: working.saved_outlier_rout_q }
            };
        }
        "fdr_q_percent" | "q_percent" | "rout_q" | "outlier_fdr_q_percent" | "edit.outlier.fdr_q_percent" | "edit.fit.rout_q" => {
            let q = parse_outlier_fdr_q_percent(val, preset)?;
            working.saved_outlier_rout_q = q;
            let OutlierSpec::Rout { fdr_q_percent } = out else {
                return Err(String::from("`fdr_q_percent` requires ROUT outlier detection mode"));
            };
            *fdr_q_percent = q;
        }
        "outlier_alpha" | "edit.outlier.alpha" | "alpha" => {
            let a = parse_outlier_alpha(val, preset)?;
            working.saved_outlier_grubbs_alpha = a;
            let OutlierSpec::Grubbs { alpha, .. } = out else {
                return Err(String::from("`alpha` for Grubbs requires Grubbs outlier detection mode"));
            };
            *alpha = a;
        }
        "iterative" | "outlier_iterative" | "edit.outlier.iterative" => {
            let it = parse_bool_field(val, "iterative")?;
            working.saved_outlier_grubbs_iterative = it;
            let OutlierSpec::Grubbs { iterative, .. } = out else {
                return Err(String::from("`iterative` requires Grubbs outlier detection mode"));
            };
            *iterative = it;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_two_group_spec(key: &str, val: &Value, preset: &LocalePreset, t: &mut TwoGroupSpec) -> Result<bool, String> {
    match key {
        "two_group_method" | "edit.two_group.method" => {
            let hyp = match t.method {
                TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } => hypothetical,
                _ => 0.0,
            };
            t.method = parse_two_group_method_str(parse_str_field(val, "two_group_method")?, hyp)?;
        }
        "hypothetical" | "two_group_hypothetical" | "edit.two_group.hypothetical" => {
            let next_hyp = parse_finite_f64_with_locale(val, key, preset)?;
            match &mut t.method {
                TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } => {
                    *hypothetical = next_hyp;
                }
                _ => return Err(String::from("`hypothetical` requires a one-sample t test or one-sample Wilcoxon method")),
            }
        }
        "paired_sign_test" | "edit.two_group.paired_sign_test" => {
            if parse_bool_field(val, "paired_sign_test")? {
                t.method = TwoGroupMethod::PairedSignTest;
            } else if t.method == TwoGroupMethod::PairedSignTest {
                t.method = TwoGroupMethod::WilcoxonSignedRank;
            }
        }
        "two_tailed" | "edit.two_group.two_tailed" => t.two_tailed = parse_bool_field(val, "two_tailed")?,
        "compute_effect_sizes" | "edit.compute_effect_sizes" => t.compute_effect_sizes = parse_bool_field(val, "compute_effect_sizes")?,
        "mv_group_column" | "edit.two_group.mv_group_column" => t.mv_group_column = parse_opt_usize_min(val, key, 0)?,
        "mv_response_column" | "edit.two_group.mv_response_column" => t.mv_response_column = parse_opt_usize_min(val, key, 0)?,
        _ => {
            if let Some(col) = key.strip_prefix("edit.two_group.group.").and_then(|s| s.parse::<usize>().ok()) {
                if parse_bool_field(val, key)? {
                    t.mv_group_column = Some(col);
                } else if t.mv_group_column == Some(col) {
                    t.mv_group_column = None;
                }
            } else if let Some(col) = key.strip_prefix("edit.two_group.resp.").and_then(|s| s.parse::<usize>().ok()) {
                if parse_bool_field(val, key)? {
                    t.mv_response_column = Some(col);
                } else if t.mv_response_column == Some(col) {
                    t.mv_response_column = None;
                }
            } else {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn patch_one_way_spec(key: &str, val: &Value, a: &mut OneWayAnovaSpec) -> Result<bool, String> {
    match key {
        "mixed_effects_reml" | "edit.rm.mixed_effects_reml" => {
            let b = parse_bool_field(val, "mixed_effects_reml")?;
            a.mixed_effects_reml = b;
            if b && !crate::modal_predicates::r1b_is_reml_allowed_post_hoc(a.multiplicity.method) {
                a.multiplicity.method = MultiplicityMethod::Bonferroni;
            }
        }
        "df_method" | "edit.rm.df_method" => a.df_method = parse_df_method_str(parse_str_field(val, "df_method")?)?,
        "residual_covariance" | "edit.rm.residual_covariance" | "edit.reml.residual_covariance" => {
            a.residual_covariance = parse_residual_covariance_str(parse_str_field(val, "residual_covariance")?)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_factorial_spec(key: &str, val: &Value, control_col: usize, f: &mut FactorialAnovaSpec) -> Result<bool, String> {
    match key {
        "compute_effect_sizes" | "edit.compute_effect_sizes" => f.compute_effect_sizes = parse_bool_field(val, "compute_effect_sizes")?,
        "three_way" | "edit.factorial.three_way" => {
            f.three_way = parse_bool_field(val, "three_way")?;
            f.max_interaction_order = if f.three_way { f.max_interaction_order.max(3) } else { f.max_interaction_order.min(2) };
        }
        "repeated_measures" | "edit.factorial.repeated_measures" => {
            f.repeated_measures = parse_rm_design_str(parse_str_field(val, "repeated_measures")?)?;
        }
        "mixed_effects_reml" | "edit.rm.mixed_effects_reml" => {
            let b = parse_bool_field(val, "mixed_effects_reml")?;
            f.mixed_effects_reml = b;
            if b && !crate::modal_predicates::r1b_is_reml_allowed_post_hoc(f.multiplicity.method) {
                f.multiplicity.method = MultiplicityMethod::Bonferroni;
            }
        }
        "df_method" | "edit.rm.df_method" => f.df_method = parse_df_method_str(parse_str_field(val, "df_method")?)?,
        "residual_covariance" | "factorial_residual_covariance" | "edit.factorial.residual_covariance" | "edit.reml.residual_covariance" => {
            f.residual_covariance = parse_residual_covariance_str(parse_str_field(val, "residual_covariance")?)?;
        }
        "ss_type" | "sum_of_squares" | "edit.factorial.ss_type" => f.ss_type = parse_ss_type_str(parse_str_field(val, "ss_type")?)?,
        "max_interaction_order" | "edit.factorial.max_interaction_order" => f.max_interaction_order = parse_usize_min(val, key, 1)?,
        "fit_reml_on_missing" | "edit.factorial.fit_reml_on_missing" => f.fit_reml_on_missing = parse_bool_field(val, "fit_reml_on_missing")?,
        "multifactor_columns" | "edit.factorial.multifactor_columns" => f.multifactor_columns = parse_usize_list(val, key)?,
        "factorial_multiplicity" | "edit.factorial.multiplicity" => {
            f.multiplicity.method = parse_multiplicity_method_name(parse_str_field(val, "factorial.multiplicity")?, control_col)?;
        }
        "response_column" | "factorial_response_column" | "edit.factorial.response_column" => f.response_column = parse_opt_usize_min(val, key, 0)?,
        _ => {
            if let Some(col) = key.strip_prefix("edit.fac.resp.").and_then(|s| s.parse::<usize>().ok()) {
                if parse_bool_field(val, key)? {
                    f.response_column = Some(col);
                } else if f.response_column == Some(col) {
                    f.response_column = None;
                }
            } else if let Some(col) = key.strip_prefix("edit.fac.col.").and_then(|s| s.parse::<usize>().ok()) {
                let want_on = parse_bool_field(val, key)?;
                if want_on {
                    if !f.multifactor_columns.contains(&col) {
                        f.multifactor_columns.push(col);
                        f.multifactor_columns.sort_unstable();
                    }
                } else {
                    f.multifactor_columns.retain(|&c| c != col);
                }
            } else {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn patch_power_spec(key: &str, val: &Value, preset: &LocalePreset, pa: &mut curvebook_doc::PowerAnalysisSpec) -> Result<bool, String> {
    match key {
        "edit.power.family" | "power.family" => pa.family = parse_power_family_for_spec(parse_str_field(val, key)?)?,
        "edit.power.solve_target" | "power.solve_target" => pa.solve_target = parse_power_solve_target_for_spec(parse_str_field(val, key)?)?,
        "edit.power.alpha" | "power.alpha" | "power.sidebar.alpha" => pa.alpha = parse_open_unit_f64(val, key, preset)?,
        "edit.power.target_power" => {
            pa.target_power = if val.is_null() { None } else { Some(parse_open_unit_f64(val, key, preset)?) };
        }
        "edit.power.sample_size" => {
            pa.sample_size = if val.is_null() { None } else { Some(parse_finite_f64_with_locale(val, key, preset)?) };
        }
        "edit.power.sample_size_group2" => {
            pa.sample_size_group2 = if val.is_null() { None } else { Some(parse_finite_f64_with_locale(val, key, preset)?) };
        }
        "edit.power.allocation_ratio" => pa.allocation_ratio = parse_pos_f64(val, key, preset)?,
        "edit.power.two_tailed" => pa.tail_mode = power_tail_mode_from_bool(parse_bool_field(val, key)?),
        "edit.power.continuity_correction" => pa.continuity_correction = parse_bool_field(val, key)?,
        "edit.power.chi_square_df" => pa.chi_square_df = parse_pos_f64(val, key, preset)?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_multivariate_spec(key: &str, val: &Value, mv: &mut MultivariateSpec) -> Result<bool, String> {
    match (key, mv) {
        ("distance" | "cluster_distance" | "edit.cluster.distance", MultivariateSpec::Clustering { distance, .. }) => {
            *distance = parse_cluster_distance_str(parse_str_field(val, "distance")?)?;
        }
        ("linkage" | "cluster_linkage" | "edit.cluster.linkage", MultivariateSpec::Clustering { linkage, .. }) => {
            *linkage = parse_cluster_linkage_str(parse_str_field(val, "linkage")?)?;
        }
        ("k_means_k" | "k" | "edit.cluster.k_means_k", MultivariateSpec::Clustering { k_means_k, .. }) => {
            *k_means_k = parse_opt_usize_min(val, key, 1)?;
        }
        ("k_means_auto" | "edit.cluster.k_means_auto", MultivariateSpec::Clustering { k_means_k, .. }) => {
            *k_means_k = if parse_bool_field(val, "k_means_auto")? { None } else { Some(k_means_k.unwrap_or(3)) };
        }
        ("k_sweep_max" | "edit.cluster.k_sweep_max", MultivariateSpec::Clustering { k_sweep_max, .. }) => {
            *k_sweep_max = parse_opt_usize_min(val, key, 2)?;
        }
        ("standardize" | "edit.pca.standardize", MultivariateSpec::Pca { standardize, .. }) => {
            *standardize = parse_bool_field(val, "standardize")?;
        }
        ("parallel_analysis_seed" | "edit.pca.parallel_analysis_seed", MultivariateSpec::Pca { parallel_analysis_seed, .. }) => {
            *parallel_analysis_seed = if val.is_null() { None } else { Some(parse_u64_val(val, key)?) };
        }
        ("spearman_rank" | "edit.corr.spearman_rank" | "edit.correlation.spearman_rank", MultivariateSpec::CorrelationMatrix { spearman_rank }) => {
            *spearman_rank = parse_bool_field(val, "spearman_rank")?;
        }
        ("imputations_m" | "m" | "edit.mi.imputations_m" | "edit.imputation.m", MultivariateSpec::MultipleImputation { imputations_m, .. }) => {
            *imputations_m = parse_usize_min(val, key, 1)?;
        }
        ("max_iters" | "edit.mi.max_iters", MultivariateSpec::MultipleImputation { max_iters, .. }) => {
            *max_iters = parse_usize_min(val, key, 1)?;
        }
        ("seed" | "edit.mi.seed", MultivariateSpec::MultipleImputation { seed, .. }) => *seed = parse_u64_val(val, key)?,
        ("response_column" | "edit.mi.response_column", MultivariateSpec::MultipleImputation { response_column, .. }) => {
            *response_column = parse_opt_usize_min(val, key, 0)?;
        }
        ("predictor_columns" | "edit.mi.predictor_columns", MultivariateSpec::MultipleImputation { predictor_columns, .. }) => {
            *predictor_columns = parse_usize_list(val, key)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_glm_spec(key: &str, val: &Value, glm: &mut GlmSpec) -> Result<bool, String> {
    match key {
        "family" | "glm_family" | "edit.glm.family" => glm.family = parse_glm_family_str(parse_str_field(val, "family")?)?,
        "response_column" | "edit.glm.response_column" => glm.response_column = parse_usize_min(val, key, 0)?,
        "predictor_columns" | "edit.glm.predictor_columns" => glm.predictor_columns = parse_usize_list(val, key)?,
        "reference_levels" | "edit.glm.reference_levels" => glm.reference_levels = parse_reference_levels(val)?,
        "compute_vif" | "edit.glm.compute_vif" => glm.compute_vif = parse_bool_field(val, "compute_vif")?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_survival_spec(key: &str, val: &Value, control_col: usize, surv: &mut SurvivalAnalysisSpec) -> Result<bool, String> {
    match (key, surv) {
        ("time_column" | "edit.cox.time_column", SurvivalAnalysisSpec::CoxProportionalHazards { time_column, .. }) => {
            *time_column = parse_usize_min(val, key, 0)?;
        }
        ("status_column" | "edit.cox.status_column", SurvivalAnalysisSpec::CoxProportionalHazards { status_column, .. }) => {
            *status_column = parse_usize_min(val, key, 0)?;
        }
        ("predictor_columns" | "edit.cox.predictor_columns", SurvivalAnalysisSpec::CoxProportionalHazards { predictor_columns, .. }) => {
            *predictor_columns = parse_usize_list(val, key)?;
        }
        ("reference_levels" | "edit.cox.reference_levels", SurvivalAnalysisSpec::CoxProportionalHazards { reference_levels, .. }) => {
            *reference_levels = parse_reference_levels(val)?;
        }
        ("tie_method" | "edit.cox.tie_method", SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, .. }) => {
            *tie_method = parse_cox_tie_method_str(parse_str_field(val, "tie_method")?)?;
        }
        ("log_rank_mantel_cox" | "edit.km.log_rank_mantel_cox", SurvivalAnalysisSpec::KaplanMeier { log_rank_mantel_cox, .. }) => {
            *log_rank_mantel_cox = parse_bool_field(val, "log_rank_mantel_cox")?;
        }
        ("gehan_breslow_wilcoxon" | "edit.km.gehan_breslow_wilcoxon", SurvivalAnalysisSpec::KaplanMeier { gehan_breslow_wilcoxon, .. }) => {
            *gehan_breslow_wilcoxon = parse_bool_field(val, "gehan_breslow_wilcoxon")?;
        }
        (
            "trend_test" | "edit.km.trend_test" | "log_rank_for_trend" | "edit.km.log_rank_for_trend",
            SurvivalAnalysisSpec::KaplanMeier { trend_test, .. },
        ) => {
            *trend_test = parse_bool_field(val, "trend_test")?;
        }
        ("multiplicity_method" | "edit.km.multiplicity_method", SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => {
            multiplicity.method = parse_multiplicity_method_name(parse_str_field(val, "multiplicity_method")?, control_col)?;
        }
        ("asymmetrical_greenwood_ci" | "edit.km.asymmetrical_greenwood_ci", SurvivalAnalysisSpec::KaplanMeier { asymmetrical_greenwood_ci, .. }) => {
            *asymmetrical_greenwood_ci = parse_bool_field(val, "asymmetrical_greenwood_ci")?;
        }
        ("mv_time_column" | "edit.km.mv_time_column", SurvivalAnalysisSpec::KaplanMeier { mv_time_column, .. }) => {
            *mv_time_column = parse_opt_usize_min(val, key, 0)?;
        }
        ("mv_status_column" | "edit.km.mv_status_column", SurvivalAnalysisSpec::KaplanMeier { mv_status_column, .. }) => {
            *mv_status_column = parse_opt_usize_min(val, key, 0)?;
        }
        ("mv_group_column" | "edit.km.mv_group_column", SurvivalAnalysisSpec::KaplanMeier { mv_group_column, .. }) => {
            *mv_group_column = parse_opt_usize_min(val, key, 0)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_nonlinear_spec(key: &str, val: &Value, preset: &LocalePreset, fit: &mut NonlinearFitSpec) -> Result<bool, String> {
    match key {
        "equation_id" | "edit.fit.equation_id" | "edit.fit.model_id" => {
            let raw = parse_str_field(val, "equation_id")?.trim();
            if !raw.is_empty() {
                fit.equation_id = raw.to_owned();
            }
        }
        "custom_dsl" | "edit.fit.custom_dsl" => fit.custom_dsl = parse_opt_nonempty_string(val, key)?,
        "weighting" | "fit_weighting" | "edit.fit.weighting" => fit.weighting = parse_weighting_scheme(val)?,
        "fit_method" | "edit.fit.method" | "method" => fit.method = parse_fit_method(val, preset)?,
        "rout_q" | "fdr_q_percent" | "q_percent" | "edit.fit.rout_q" => {
            let q = parse_nonlinear_rout_q_percent(val, preset)?;
            let FitMethod::RoutOutlierElimination { fdr_q_percent } = &mut fit.method else {
                return Err(String::from("`rout_q` requires ROUT outlier elimination fit method"));
            };
            *fdr_q_percent = q;
        }
        "parameter_rules" | "edit.fit.parameter_rules" => fit.parameter_rules = parse_parameter_rules(val)?,
        "profile_likelihood_ci" | "edit.fit.profile_likelihood_ci" => fit.profile_likelihood_ci = parse_bool_field(val, "profile_likelihood_ci")?,
        "identify_unstable_parameters" | "edit.fit.identify_unstable_parameters" => {
            fit.identify_unstable_parameters = parse_bool_field(val, key)?;
        }
        "identifiability_mode" | "edit.fit.identifiability_mode" => {
            fit.identifiability_mode = if val.is_null() {
                None
            } else {
                Some(serde_json::from_value::<IdentifiabilityMode>(val.clone()).map_err(|e| format!("invalid `identifiability_mode`: {e}"))?)
            };
        }
        "show_standard_errors" | "edit.fit.show_standard_errors" => {
            fit.show_standard_errors = if val.is_null() { None } else { Some(parse_bool_field(val, key)?) };
        }
        "flag_r_squared_below" | "edit.fit.flag_r_squared_below" => {
            let thr = parse_opt_finite_f64_with_locale(val, key, preset)?;
            if let Some(v) = thr
                && !(0.0..=1.0).contains(&v)
            {
                return Err(format!("`{key}` ({v}) must lie in [0, 1]"));
            }
            fit.flag_r_squared_below = thr;
        }
        "fit_flag_rules" | "edit.fit.fit_flag_rules" => {
            fit.fit_flag_rules = if val.is_null() {
                None
            } else {
                let rules: FitFlagRules = serde_json::from_value(val.clone()).map_err(|e| format!("invalid `fit_flag_rules`: {e}"))?;
                if let Some(r2) = rules.min_r_squared
                    && (!r2.is_finite() || !(0.0..=1.0).contains(&r2))
                {
                    return Err(format!("`fit_flag_rules.min_r_squared` ({r2}) must lie in [0, 1]"));
                }
                if let Some(dep) = rules.max_dependency
                    && (!dep.is_finite() || !(0.0..=1.0).contains(&dep))
                {
                    return Err(format!("`fit_flag_rules.max_dependency` ({dep}) must lie in [0, 1]"));
                }
                if let Some(p) = rules.min_normality_p
                    && (!p.is_finite() || !(0.0 < p && p < 1.0))
                {
                    return Err(format!("`fit_flag_rules.min_normality_p` ({p}) must lie strictly in (0, 1)"));
                }
                if let Some(p) = rules.min_runs_or_lof_p
                    && (!p.is_finite() || !(0.0 < p && p < 1.0))
                {
                    return Err(format!("`fit_flag_rules.min_runs_or_lof_p` ({p}) must lie strictly in (0, 1)"));
                }
                Some(rules)
            };
        }
        "comparison" | "edit.fit.comparison" => {
            fit.comparison = if val.is_null() {
                None
            } else {
                Some(serde_json::from_value::<ModelComparisonSpec>(val.clone()).map_err(|e| format!("invalid `comparison`: {e}"))?)
            };
        }
        "comparison_question" | "edit.fit.comparison_question" => {
            fit.comparison_question = if val.is_null() {
                None
            } else {
                Some(serde_json::from_value::<ModelComparisonQuestion>(val.clone()).map_err(|e| format!("invalid `comparison_question`: {e}"))?)
            };
        }
        "comparison_method" | "edit.fit.comparison_method" => {
            fit.comparison_method = if val.is_null() {
                None
            } else {
                Some(serde_json::from_value::<ModelComparisonMethod>(val.clone()).map_err(|e| format!("invalid `comparison_method`: {e}"))?)
            };
        }
        "prefer_well_defined_fit" | "edit.fit.prefer_well_defined_fit" => {
            fit.prefer_well_defined_fit = parse_bool_field(val, key)?;
        }
        "summary_parameter" | "edit.fit.summary_parameter" => {
            fit.summary_parameter = parse_opt_nonempty_string(val, key)?;
        }
        "create_summary_graph" | "edit.fit.create_summary_graph" => {
            fit.create_summary_graph = parse_bool_field(val, key)?;
        }
        "output_significant_digits" | "edit.fit.output_significant_digits" => {
            fit.output_significant_digits = if val.is_null() {
                None
            } else {
                let d = parse_usize_min(val, key, 1)?;
                if d > 15 {
                    return Err(format!("`{key}` ({d}) must lie in 1..=15"));
                }
                #[allow(clippy::cast_possible_truncation)]
                Some(d as u8)
            };
        }
        "confidence_bands" | "edit.fit.confidence_bands" => fit.confidence_bands = parse_bool_field(val, "confidence_bands")?,
        "prediction_bands" | "edit.fit.prediction_bands" => fit.prediction_bands = parse_bool_field(val, "prediction_bands")?,
        "interpolate_unknowns" | "edit.fit.interpolate_unknowns" => fit.interpolate_unknowns = parse_bool_field(val, key)?,
        "replicates_lack_of_fit_test" | "edit.fit.replicates_lack_of_fit_test" => fit.replicates_lack_of_fit_test = parse_bool_field(val, key)?,
        "normality_of_residuals" | "edit.fit.normality_of_residuals" => fit.normality_of_residuals = parse_bool_field(val, key)?,
        "max_iters" | "edit.fit.max_iters" => fit.max_iters = parse_usize_min(val, key, 1)?,
        "x_range_filter" | "edit.fit.x_range_filter" => fit.x_range_filter = parse_opt_range_pair(val, key, preset)?,
        "y_range_filter" | "edit.fit.y_range_filter" => fit.y_range_filter = parse_opt_range_pair(val, key, preset)?,
        "plot_function_only" | "edit.fit.plot_function_only" => fit.plot_function_only = parse_bool_field(val, key)?,
        "plot_x_min" | "edit.fit.plot_x_min" => fit.plot_x_min = parse_opt_finite_f64_with_locale(val, key, preset)?,
        "plot_x_max" | "edit.fit.plot_x_max" => fit.plot_x_max = parse_opt_finite_f64_with_locale(val, key, preset)?,
        "homoscedasticity_test" | "edit.fit.homoscedasticity_test" => fit.homoscedasticity_test = parse_bool_field(val, "homoscedasticity_test")?,
        "emit_residuals_tab" | "edit.fit.emit_residuals_tab" => fit.emit_residuals_tab = parse_bool_field(val, "emit_residuals_tab")?,
        "emit_covariance_tab" | "edit.fit.emit_covariance_tab" => fit.emit_covariance_tab = parse_bool_field(val, "emit_covariance_tab")?,
        "mv_x_columns" | "edit.fit.mv_x_columns" => fit.mv_x_columns = parse_usize_list(val, key)?,
        "mv_y_column" | "edit.fit.mv_y_column" => fit.mv_y_column = parse_opt_usize_min(val, key, 0)?,
        "mv_y_columns" | "edit.fit.mv_y_columns" => fit.mv_y_columns = parse_usize_list(val, key)?,
        "mv_group_columns" | "edit.fit.mv_group_columns" => fit.mv_group_columns = parse_usize_list(val, key)?,
        "weighting_source" | "edit.fit.weighting_source" => {
            fit.weighting_source = serde_json::from_value(val.clone()).map_err(|e| format!("invalid `weighting_source`: {e}"))?;
        }
        "outlier_handling" | "edit.fit.outlier_handling" => {
            fit.outlier_handling = serde_json::from_value(val.clone()).map_err(|e| format!("invalid `outlier_handling`: {e}"))?;
        }
        "emit_cleaned_data_tab" | "edit.fit.emit_cleaned_data_tab" => {
            fit.emit_cleaned_data_tab = parse_bool_field(val, "emit_cleaned_data_tab")?;
        }
        "replicate_fit_mode" | "edit.fit.replicate_fit_mode" => {
            fit.replicate_fit_mode = serde_json::from_value(val.clone()).map_err(|e| format!("invalid `replicate_fit_mode`: {e}"))?;
        }
        "convergence_strictness" | "edit.fit.convergence_strictness" => {
            fit.convergence_strictness = serde_json::from_value(val.clone()).map_err(|e| format!("invalid `convergence_strictness`: {e}"))?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_linear_regression_spec(key: &str, val: &Value, preset: &LocalePreset, lr: &mut LinearRegressionSpec) -> Result<bool, String> {
    match key {
        "deming_model_ii" | "deming" | "edit.lr.deming_model_ii" | "edit.lr.deming" => {
            lr.deming_model_ii = parse_bool_field(val, key)?;
        }
        "force_through_origin" | "edit.lr.force_through_origin" => {
            lr.force_through_origin = parse_bool_field(val, key)?;
        }
        "fixed_point" | "edit.lr.fixed_point" => {
            lr.fixed_point = if val.is_null() {
                None
            } else if let Some(arr) = val.as_array() {
                if arr.len() != 2 {
                    return Err(String::from("`fixed_point` must be a 2-element `[x0, y0]` array or null"));
                }
                Some((parse_finite_f64_with_locale(&arr[0], key, preset)?, parse_finite_f64_with_locale(&arr[1], key, preset)?))
            } else {
                return Err(String::from("`fixed_point` must be a `[x0, y0]` array or null"));
            };
        }
        "standard_addition" | "edit.lr.standard_addition" => {
            lr.standard_addition = parse_bool_field(val, key)?;
        }
        "extend_line_to_x_intercept" | "edit.lr.extend_line_to_x_intercept" => {
            lr.extend_line_to_x_intercept = parse_bool_field(val, key)?;
        }
        "deming_error" | "edit.lr.deming_error" => {
            let parsed: DemingErrorSpec = serde_json::from_value(val.clone()).map_err(|e| format!("invalid `deming_error`: {e}"))?;
            if parsed.resolved_lambda().is_none() {
                return Err(String::from("invalid `deming_error`: standard deviations and variance ratio must be finite and > 0"));
            }
            lr.deming_error = parsed;
        }
        "confidence_bands" | "edit.lr.confidence_bands" => lr.confidence_bands = parse_bool_field(val, key)?,
        "prediction_bands" | "edit.lr.prediction_bands" => lr.prediction_bands = parse_bool_field(val, key)?,
        "interpolate_unknowns" | "edit.lr.interpolate_unknowns" => lr.interpolate_unknowns = parse_bool_field(val, key)?,
        "x_range_filter" | "edit.lr.x_range_filter" => lr.x_range_filter = parse_opt_range_pair(val, key, preset)?,
        "plot_x_min" | "edit.lr.plot_x_min" => lr.plot_x_min = parse_opt_finite_f64_with_locale(val, key, preset)?,
        "plot_x_max" | "edit.lr.plot_x_max" => lr.plot_x_max = parse_opt_finite_f64_with_locale(val, key, preset)?,
        "plot_segments" | "segments" | "edit.lr.plot_segments" => {
            lr.plot_segments = if val.is_null() {
                None
            } else {
                let segs = parse_usize_min(val, key, 1)?;
                if segs > LinearRegressionSpec::MAX_PLOT_SEGMENTS {
                    return Err(format!("`{key}` ({segs}) must lie in 1..={}", LinearRegressionSpec::MAX_PLOT_SEGMENTS));
                }
                Some(segs)
            };
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_simple_logistic_spec(key: &str, val: &Value, preset: &LocalePreset, sl: &mut SimpleLogisticSpec) -> Result<bool, String> {
    match key {
        "emit_row_classification" | "edit.simple_logistic.emit_row_classification" => {
            sl.emit_row_classification = parse_bool_field(val, key)?;
        }
        "confidence_bands" | "edit.simple_logistic.confidence_bands" => {
            sl.confidence_bands = parse_bool_field(val, key)?;
        }
        "classification_cutoff" | "cutoff" | "edit.simple_logistic.classification_cutoff" => {
            sl.classification_cutoff = parse_open_unit_f64(val, key, preset)?;
        }
        "plot_x_min" | "edit.simple_logistic.plot_x_min" => {
            sl.plot_x_min = parse_opt_finite_f64_with_locale(val, key, preset)?;
        }
        "plot_x_max" | "edit.simple_logistic.plot_x_max" => {
            sl.plot_x_max = parse_opt_finite_f64_with_locale(val, key, preset)?;
        }
        "plot_segments" | "segments" | "edit.simple_logistic.plot_segments" => {
            sl.plot_segments = if val.is_null() {
                None
            } else {
                let segs = parse_usize_min(val, key, 1)?;
                if segs > SimpleLogisticSpec::MAX_PLOT_SEGMENTS {
                    return Err(format!("`{key}` ({segs}) must lie in 1..={}", SimpleLogisticSpec::MAX_PLOT_SEGMENTS));
                }
                Some(segs)
            };
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_roc_spec(key: &str, val: &Value, roc: &mut RocSpec) -> Result<bool, String> {
    match key {
        "ci_method" | "roc_ci_method" | "edit.roc.ci_method" => {
            roc.ci_method = if let Ok(direct) = serde_json::from_value::<RocCiMethod>(val.clone()) {
                direct
            } else {
                let raw = parse_str_field(val, key)?;
                match norm_key(raw).as_str() {
                    "delong" => RocCiMethod::DeLong,
                    "hanleymcneil" | "hanley" | "wilson" => RocCiMethod::HanleyMcNeil,
                    _ => return Err(format!("unknown ROC `ci_method` `{raw}`")),
                }
            };
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn patch_multiple_row_spec(
    key: &str,
    val: &Value,
    preset: &LocalePreset,
    control_col: usize,
    mrt: &mut MultipleRowTestsSpec,
) -> Result<bool, String> {
    match key {
        "paired" | "edit.multiple_row.paired" => mrt.paired = parse_bool_field(val, key)?,
        "nonparametric" | "edit.multiple_row.nonparametric" => mrt.nonparametric = parse_bool_field(val, key)?,
        "assume_equal_sd" | "edit.multiple_row.assume_equal_sd" => mrt.assume_equal_sd = parse_bool_field(val, key)?,
        "multiplicity_method" | "multiplicity" | "edit.multiple_row.multiplicity" => {
            mrt.multiplicity.method = parse_multiplicity_method_name(parse_str_field(val, key)?, control_col)?;
        }
        "fdr_q" | "q" | "threshold" | "alpha" | "edit.multiple_row.fdr_q" | "edit.multiple_row.threshold" => {
            mrt.multiplicity.threshold = parse_open_unit_f64(val, key, preset)?;
        }
        "report_confidence_intervals" | "edit.multiple_row.report_confidence_intervals" => {
            mrt.multiplicity.report_confidence_intervals = parse_bool_field(val, key)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_bland_altman_spec(key: &str, val: &Value, ba: &mut BlandAltmanSpec) -> Result<bool, String> {
    match key {
        "column_a" | "edit.bland_altman.column_a" => ba.column_a = parse_usize_min(val, key, 0)?,
        "column_b" | "edit.bland_altman.column_b" => ba.column_b = parse_usize_min(val, key, 0)?,
        "percent_difference" | "edit.bland_altman.percent_difference" => ba.percent_difference = parse_bool_field(val, key)?,
        "xy_compare_x_vs_y" | "edit.bland_altman.xy_compare_x_vs_y" => {
            ba.xy_compare_x_vs_y = if val.is_null() { None } else { Some(parse_bool_field(val, key)?) };
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_passing_bablok_spec(key: &str, val: &Value, pb: &mut PassingBablokSpec) -> Result<bool, String> {
    match key {
        "x_column" | "edit.passing_bablok.x_column" => pb.x_column = parse_usize_min(val, key, 0)?,
        "y_column" | "edit.passing_bablok.y_column" => pb.y_column = parse_usize_min(val, key, 0)?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_spline_lowess_spec(key: &str, val: &Value, preset: &LocalePreset, sp: &mut curvebook_doc::SplineLowessSpec) -> Result<bool, String> {
    match key {
        "method" | "spline_method" | "edit.spline.method" => {
            sp.method = serde_json::from_value(val.clone()).map_err(|e| format!("invalid spline/LOWESS method: {e}"))?;
        }
        "segment_count" | "segments" | "edit.spline.segments" => {
            let n = parse_usize_min(val, key, curvebook_doc::SplineLowessSpec::MIN_SEGMENTS)?;
            if n > curvebook_doc::SplineLowessSpec::MAX_SEGMENTS {
                return Err(format!(
                    "`{key}` ({n}) must lie in {}..={}",
                    curvebook_doc::SplineLowessSpec::MIN_SEGMENTS,
                    curvebook_doc::SplineLowessSpec::MAX_SEGMENTS
                ));
            }
            sp.segment_count = n;
        }
        "smoothing_knots" | "knots" | "edit.spline.knots" | "edit.spline.smoothing_knots" => {
            sp.smoothing_knots = parse_opt_usize_min(val, key, 2)?;
        }
        "smoothing_df" | "df" => {
            let v_opt = parse_opt_finite_f64_with_locale(val, key, preset)?;
            if let Some(v) = v_opt
                && v < 2.0
            {
                return Err(format!("`{key}` ({v}) must be finite and >= 2"));
            }
            sp.smoothing_df = v_opt;
        }
        "lowess_preset" | "edit.spline.lowess_preset" => {
            sp.lowess_preset = serde_json::from_value(val.clone()).map_err(|e| format!("invalid LOWESS preset: {e}"))?;
        }
        "lowess_fraction" | "span" => {
            let v_opt = parse_opt_finite_f64_with_locale(val, key, preset)?;
            if let Some(v) = v_opt
                && (v <= 0.0 || v > 1.0)
            {
                return Err(format!("`{key}` ({v}) must lie in (0, 1]"));
            }
            sp.lowess_fraction = v_opt;
        }
        "lowess_robustness_iters" | "edit.spline.lowess_iters" => {
            let n = parse_usize_min(val, key, 0)?;
            if n > curvebook_doc::SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS {
                return Err(format!("`{key}` ({n}) must lie in 0..={}", curvebook_doc::SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS));
            }
            sp.lowess_robustness_iters = n;
        }
        "plot_x_min" | "edit.spline.plot_x_min" => {
            sp.plot_x_min = parse_opt_finite_f64_with_locale(val, key, preset)?;
        }
        "plot_x_max" | "edit.spline.plot_x_max" => {
            sp.plot_x_max = parse_opt_finite_f64_with_locale(val, key, preset)?;
        }
        "emit_curve_coordinates_tab" | "edit.spline.emit_coords" => {
            sp.emit_curve_coordinates_tab = parse_bool_field(val, key)?;
        }
        "interpolate_unknowns" | "edit.spline.interpolate_unknowns" => {
            sp.interpolate_unknowns = parse_bool_field(val, key)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn patch_area_under_curve_spec(key: &str, val: &Value, preset: &LocalePreset, auc: &mut AreaUnderCurveSpec) -> Result<bool, String> {
    match key {
        "baseline" | "edit.auc.baseline" => {
            if let Some(s) = val.as_str() {
                if matches!(norm_key(s).as_str(), "firstpoint" | "first") {
                    auc.baseline = AucBaseline::FirstPoint;
                } else {
                    let value = parse_finite_f64_with_locale(val, key, preset)?;
                    auc.baseline = AucBaseline::Fixed { value };
                }
            } else if val.is_object() {
                let parsed: AucBaseline = serde_json::from_value(val.clone()).map_err(|e| format!("invalid AUC `baseline`: {e}"))?;
                if let AucBaseline::Fixed { value } = parsed
                    && !value.is_finite()
                {
                    return Err(String::from("AUC `baseline` value must be finite"));
                }
                auc.baseline = parsed;
            } else {
                let value = parse_finite_f64_with_locale(val, key, preset)?;
                auc.baseline = AucBaseline::Fixed { value };
            }
        }
        "baseline_mode" | "edit.auc.baseline_mode" => {
            let raw = parse_str_field(val, key)?;
            match norm_key(raw).as_str() {
                "firstpoint" | "first" => auc.baseline = AucBaseline::FirstPoint,
                "fixed" | "constant" => {
                    if !matches!(auc.baseline, AucBaseline::Fixed { .. }) {
                        auc.baseline = AucBaseline::Fixed { value: 0.0 };
                    }
                }
                _ => return Err(format!("unknown AUC baseline_mode `{raw}`")),
            }
        }
        "baseline_value" | "edit.auc.baseline_value" | "edit.auc.baseline.value" => {
            let value = parse_finite_f64_with_locale(val, key, preset)?;
            auc.baseline = AucBaseline::Fixed { value };
        }
        "x_min" | "edit.auc.x_min" => {
            auc.x_min = parse_opt_finite_f64_with_locale(val, key, preset)?;
        }
        "x_max" | "edit.auc.x_max" => {
            auc.x_max = parse_opt_finite_f64_with_locale(val, key, preset)?;
        }
        "independent_time_points" | "edit.auc.independent_time_points" => {
            auc.independent_time_points = parse_bool_field(val, "independent_time_points")?;
        }
        "min_peak_height" | "edit.auc.min_peak_height" => {
            let v = parse_finite_f64_with_locale(val, key, preset)?;
            if v < 0.0 {
                return Err(format!("`min_peak_height` ({v}) must be >= 0"));
            }
            auc.min_peak_height = v;
        }
        "min_area_fraction" | "edit.auc.min_area_fraction" => {
            let v = parse_finite_f64_with_locale(val, key, preset)?;
            if !(0.0..=1.0).contains(&v) {
                return Err(format!("`min_area_fraction` ({v}) must be within 0.0..=1.0"));
            }
            auc.min_area_fraction = v;
        }
        _ => return Ok(false),
    }
    Ok(true)
}
