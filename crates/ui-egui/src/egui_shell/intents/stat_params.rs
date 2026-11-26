//! Typed executors for `StatParamsIntent` and `AnalyzePickerIntent` (retiring pre-R15 `views/*` custom dispatch).

use curvebook_doc::{
    AnalysisKind, ContingencySpec, MultiplicityMethod, PValueFormat, RepeatedMeasuresDesign, ResidualCovariance, SurvivalAnalysisSpec, TwoGroupMethod,
};
use curvebook_engine::EngineError;

use super::{AnalyzePickerIntent, ContingencyCiTarget, DiagToggleFlag, StatParamsIntent, UiIntentOutcome};
use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, DuplicateChoice, EditAnalysisParametersState};
use crate::modal_predicates::{
    is_reml_mixed_effects_active, r1_paired_or_matched_allowed, r1_sphericity_or_reml_control, r1b_post_hoc_option_enabled,
    r1b_sphericity_correction, r2_assume_gaussian_allowed, r2_parametric_post_hoc_method, r3_control_column_picker, r4_two_group_diagnostics,
    r6_contingency_ci_selector, r7_correlation_heatmap_toggle, r9_pvalue_precision_spinner, source_table_shape,
};

fn conflict(msg: impl Into<String>) -> EngineError {
    EngineError::with_code(curvebook_doc::ErrorCode::Conflict, msg.into())
}

fn patch_edit_params(app: &mut UiApp, map: &serde_json::Map<String, serde_json::Value>) -> Result<(), EngineError> {
    crate::dialogs_canvas::dialog_patch::patch_edit_params(&mut app.canvas_dialogs.edit_params, map, &app.locale_preset).map_err(conflict)
}

fn source_sheet_col_count(app: &UiApp) -> usize {
    let doc = app.document();
    let Some(src_id) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id) else {
        return 0;
    };
    doc.data_sheet(src_id).or_else(|| doc.result_sheet(src_id).and_then(|rs| rs.analyzable_data.as_ref())).map_or(0, |ds| ds.archetype.column_count())
}

pub(super) fn apply_analyze_picker_intent(app: &mut UiApp, intent: AnalyzePickerIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        AnalyzePickerIntent::ClearSearch => {
            app.interaction.text_field_buffers.remove("analyze.search");
            app.canvas_dialogs.analyze.search_query.clear();
            app.interaction.scroll_offsets.offsets.insert(String::from("scroll.analyze.entries"), [0.0, 0.0]);
        }
        AnalyzePickerIntent::ToggleCategory { slug } => {
            if let Some(buf) = app.interaction.text_field_buffers.get("analyze.search") {
                app.canvas_dialogs.analyze.search_query = buf.text.clone();
            }
            let collapsed = &mut app.canvas_dialogs.analyze.collapsed_categories;
            if let Some(pos) = collapsed.iter().position(|s| s == &slug) {
                collapsed.remove(pos);
            } else if !slug.is_empty() {
                collapsed.push(slug);
            }
        }
        AnalyzePickerIntent::TogglePcaPredictor { col } => app.canvas_dialogs.analyze.toggle_pca_predictor(col),
        AnalyzePickerIntent::SetPcrOutcome { col } => {
            app.canvas_dialogs.analyze.select_pcr_outcome(col).map_err(conflict)?;
        }
        AnalyzePickerIntent::SetMvRoleFilter { filter } => {
            app.canvas_dialogs.analyze.role_filter = filter;
            app.interaction.scroll_offsets.offsets.insert(String::from("scroll.analyze.mv_vars"), [0.0, 0.0]);
        }
        AnalyzePickerIntent::ToggleMvVar { col } => app.canvas_dialogs.analyze.toggle_variable(col),
        AnalyzePickerIntent::SetOutlierMethod { method } => app.canvas_dialogs.analyze.outlier_method = method,
        AnalyzePickerIntent::StepOutlierParam { delta } => {
            let an = &mut app.canvas_dialogs.analyze;
            if an.outlier_method == "grubbs" {
                an.outlier_alpha = (an.outlier_alpha + delta).clamp(0.001, 0.25);
            } else {
                an.outlier_fdr_q_percent = (an.outlier_fdr_q_percent + delta).clamp(0.1, 20.0);
            }
        }
        AnalyzePickerIntent::ToggleOutlierIterative => {
            let an = &mut app.canvas_dialogs.analyze;
            if an.outlier_method == "grubbs" {
                an.outlier_iterative = !an.outlier_iterative;
            }
        }
        AnalyzePickerIntent::SetDuplicateChoice { choice } => {
            app.canvas_dialogs.analyze.duplicate_choice = choice;
            if choice == DuplicateChoice::ModifyExisting
                && app.canvas_dialogs.analyze.duplicate_target_id.is_none()
                && let ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, .. } = &app.canvas_dialogs.active_dialog
            {
                app.canvas_dialogs.analyze.duplicate_target_id = existing_result_ids.first().map(|id| id.as_str().to_owned());
            }
        }
        AnalyzePickerIntent::SetDuplicateTarget { target_id } => {
            app.canvas_dialogs.analyze.duplicate_target_id = Some(target_id);
            app.canvas_dialogs.analyze.duplicate_choice = DuplicateChoice::ModifyExisting;
        }
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

pub(super) fn apply_stat_params_intent(app: &mut UiApp, intent: StatParamsIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        StatParamsIntent::SetOneWayMethod { method } => set_one_way_method(app, &method)?,
        StatParamsIntent::SelectAllDatasets => {
            if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft {
                spec.selected_data_sets.clear();
            }
        }
        StatParamsIntent::ToggleDataset { idx } => toggle_dataset_index(app, idx),
        StatParamsIntent::ToggleSchematic => handle_schematic_toggle(app)?,
        StatParamsIntent::ToggleGeisserGreenhouse => handle_toggle_gg(app)?,
        StatParamsIntent::SetResidualCovariance { cov } => set_factorial_residual_cov(app, &cov)?,
        StatParamsIntent::SetMultiplicityMethod { slug } => set_multiplicity_method(app, &slug)?,
        StatParamsIntent::StepMultiplicityThreshold { delta } => step_multiplicity_threshold(app, delta),
        StatParamsIntent::SetCompareAgainstControl { enabled } => {
            let mut map = serde_json::Map::new();
            map.insert(String::from("compare_against_control"), serde_json::Value::Bool(enabled));
            patch_edit_params(app, &map)?;
        }
        StatParamsIntent::CycleControlColumn => handle_cycle_control_col(app)?,
        StatParamsIntent::ToggleMultiplicityReportCi => handle_toggle_mult_ci(app),
        StatParamsIntent::ToggleEffectSizes => handle_toggle_effect_sizes(app),
        StatParamsIntent::SetPValueFormat { slug } => {
            let mut map = serde_json::Map::new();
            map.insert(String::from("p_value_format"), serde_json::Value::String(slug));
            patch_edit_params(app, &map)?;
        }
        StatParamsIntent::StepPValueDigits { delta } => step_pvalue_digits(app, delta)?,
        StatParamsIntent::StepTwoGroupHypothetical { delta } => step_hypothetical_value(app, delta),
        StatParamsIntent::CycleContingencyCi { which } => cycle_contingency_ci(app, which)?,
        StatParamsIntent::ToggleCorrAllPairs => {
            let next = !app.canvas_dialogs.edit_params.correlation_all_pairs;
            let mut map = serde_json::Map::new();
            map.insert(String::from("correlation_all_pairs"), serde_json::Value::Bool(next));
            patch_edit_params(app, &map)?;
        }
        StatParamsIntent::ToggleCorrHeatmap => {
            let gate = r7_correlation_heatmap_toggle(app.canvas_dialogs.edit_params.correlation_all_pairs);
            if !gate.enabled {
                return Err(conflict(gate.disabled_reason.unwrap_or_default()));
            }
            let next = !app.canvas_dialogs.edit_params.correlation_heatmap;
            let mut map = serde_json::Map::new();
            map.insert(String::from("correlation_heatmap"), serde_json::Value::Bool(next));
            patch_edit_params(app, &map)?;
        }
        StatParamsIntent::ToggleDiag { flag } => toggle_diag_flag(app, flag)?,
    }
    Ok(UiIntentOutcome::ShellUpdated)
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

fn handle_schematic_toggle(app: &mut UiApp) -> Result<(), EngineError> {
    let Some(kind) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| d.kind.clone()) else {
        return Ok(());
    };
    match kind {
        AnalysisKind::OneWayAnova(_) | AnalysisKind::TwoGroupComparison(_) => {
            let next = !app.canvas_dialogs.edit_params.paired_or_matched;
            let src = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id);
            if next && !r1_paired_or_matched_allowed(app.document(), src).enabled {
                let mode = &mut app.canvas_dialogs.edit_params.schematic_mode;
                *mode = if mode == "detail" { String::from("auto") } else { String::from("detail") };
                return Ok(());
            }
            let mut map = serde_json::Map::new();
            map.insert(String::from("paired_or_matched"), serde_json::Value::Bool(next));
            patch_edit_params(app, &map)
        }
        AnalysisKind::FactorialAnova(f) => {
            if f.repeated_measures != RepeatedMeasuresDesign::None {
                let next = if f.residual_covariance == ResidualCovariance::CompoundSymmetry { "unstructured" } else { "compound_symmetry" };
                return set_factorial_residual_cov(app, next);
            }
            let src = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.source_sheet_id);
            if !r1_paired_or_matched_allowed(app.document(), src).enabled {
                let mode = &mut app.canvas_dialogs.edit_params.schematic_mode;
                *mode = if mode == "detail" { String::from("auto") } else { String::from("detail") };
                return Ok(());
            }
            let mut map = serde_json::Map::new();
            map.insert(String::from("paired_or_matched"), serde_json::Value::Bool(true));
            patch_edit_params(app, &map)
        }
        _ => {
            let mode = &mut app.canvas_dialogs.edit_params.schematic_mode;
            *mode = if mode == "detail" { String::from("auto") } else { String::from("detail") };
            Ok(())
        }
    }
}

fn set_one_way_method(app: &mut UiApp, method_name: &str) -> Result<(), EngineError> {
    let ep = &app.canvas_dialogs.edit_params;
    let src = ep.draft.as_ref().map(|d| &d.source_sheet_id);
    let wants_nonparam = matches!(method_name, "kruskal" | "friedman");
    let wants_paired = matches!(method_name, "rm" | "friedman");
    if wants_nonparam {
        let g = r2_assume_gaussian_allowed(app.document(), src);
        if !g.enabled {
            return Err(conflict(g.disabled_reason.unwrap_or_else(|| String::from("Nonparametric tests unavailable"))));
        }
    }
    if wants_paired {
        let p = r1_paired_or_matched_allowed(app.document(), src);
        if !p.enabled {
            return Err(conflict(p.disabled_reason.unwrap_or_else(|| String::from("Repeated measures unavailable"))));
        }
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("anova_method"), serde_json::Value::String(method_name.to_owned()));
    patch_edit_params(app, &map)?;
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
    Ok(())
}

fn handle_toggle_gg(app: &mut UiApp) -> Result<(), EngineError> {
    let ep = &app.canvas_dialogs.edit_params;
    let reml_active = is_reml_mixed_effects_active(ep, app.document());
    let gate = r1b_sphericity_correction(ep.paired_or_matched, reml_active);
    if !gate.enabled {
        return Err(conflict(gate.disabled_reason.unwrap_or_default()));
    }
    let cur = ep.draft.as_ref().is_none_or(|d| {
        !matches!(d.kind, AnalysisKind::OneWayAnova(ref a) if matches!(a.method, curvebook_doc::OneWayMethod::RepeatedMeasures { geisser_greenhouse: false }))
    });
    let mut map = serde_json::Map::new();
    map.insert(String::from("geisser_greenhouse"), serde_json::Value::Bool(!cur));
    patch_edit_params(app, &map)
}

fn set_factorial_residual_cov(app: &mut UiApp, raw: &str) -> Result<(), EngineError> {
    if !app.canvas_dialogs.edit_params.paired_or_matched {
        return Err(conflict(r1_sphericity_or_reml_control(false).disabled_reason.unwrap_or_default()));
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("residual_covariance"), serde_json::Value::String(raw.to_owned()));
    patch_edit_params(app, &map)
}

fn set_multiplicity_method(app: &mut UiApp, raw: &str) -> Result<(), EngineError> {
    let ep = &app.canvas_dialogs.edit_params;
    let method = parse_mult_method(raw, ep.control_column)?;
    if matches!(ep.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::MultipleRowTTests(_))) {
        let nonparam = matches!(ep.draft.as_ref().map(|d| &d.kind), Some(AnalysisKind::MultipleRowTTests(m)) if m.nonparametric);
        if !r2_parametric_post_hoc_method(!nonparam, method).enabled {
            return Err(conflict("Parametric multiplicity disabled in nonparametric mode."));
        }
        if let Some(spec) = &mut app.canvas_dialogs.edit_params.draft
            && let AnalysisKind::MultipleRowTTests(mut mr) = spec.kind.clone()
        {
            mr.multiplicity.method = method;
            spec.update_kind(AnalysisKind::MultipleRowTTests(mr));
        }
        return Ok(());
    }
    let reml_active = is_reml_mixed_effects_active(ep, app.document());
    let gate = r1b_post_hoc_option_enabled(ep.assume_gaussian, reml_active, method);
    if !gate.enabled {
        return Err(conflict(gate.disabled_reason.unwrap_or_default()));
    }
    let mut map = serde_json::Map::new();
    map.insert(String::from("multiplicity_method"), serde_json::Value::String(raw.to_owned()));
    patch_edit_params(app, &map)
}

fn step_multiplicity_threshold(app: &mut UiApp, delta: f64) {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return;
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
}

fn handle_cycle_control_col(app: &mut UiApp) -> Result<(), EngineError> {
    let method = current_multiplicity(&app.canvas_dialogs.edit_params).map_or(MultiplicityMethod::None, |m| m.method);
    let gate = r3_control_column_picker(app.canvas_dialogs.edit_params.compare_against_control, method);
    if !gate.enabled {
        return Err(conflict(gate.disabled_reason.unwrap_or_default()));
    }
    let cols = source_sheet_col_count(app).max(2);
    let next = (app.canvas_dialogs.edit_params.control_column + 1) % cols;
    let mut map = serde_json::Map::new();
    map.insert(String::from("control_column"), serde_json::json!(next));
    patch_edit_params(app, &map)
}

fn handle_toggle_mult_ci(app: &mut UiApp) {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return;
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
}

fn handle_toggle_effect_sizes(app: &mut UiApp) {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return;
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
}

fn step_pvalue_digits(app: &mut UiApp, delta: i32) -> Result<(), EngineError> {
    let Some(spec) = &app.canvas_dialogs.edit_params.draft else {
        return Ok(());
    };
    let gate = r9_pvalue_precision_spinner(&spec.p_value_format);
    if !gate.enabled {
        return Err(conflict(gate.disabled_reason.unwrap_or_default()));
    }
    let cur = match spec.p_value_format {
        PValueFormat::Custom { decimals, .. } => i32::from(decimals),
        _ => 4,
    };
    let next = (cur + delta).clamp(1, 9);
    let mut map = serde_json::Map::new();
    map.insert(String::from("p_value_decimals"), serde_json::json!(next));
    patch_edit_params(app, &map)
}

fn step_hypothetical_value(app: &mut UiApp, delta: f64) {
    let Some(spec) = &mut app.canvas_dialogs.edit_params.draft else {
        return;
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
}

fn cycle_contingency_ci(app: &mut UiApp, which: ContingencyCiTarget) -> Result<(), EngineError> {
    let doc = app.document();
    let ep = &app.canvas_dialogs.edit_params;
    let (rows, cols, _) = source_table_shape(doc, ep.draft.as_ref().map(|d| &d.source_sheet_id));
    let cont = match ep.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::Contingency(c)) => c.clone(),
        _ => ContingencySpec::default(),
    };
    let (wid_key, checked, label, next_val) = match which {
        ContingencyCiTarget::OddsRatio => (
            "edit.contingency.ci.baptista_pike",
            ep.contingency_odds_ratio_checked || cont.compute_odds_and_risk_ratios,
            "Baptista-Pike",
            if ep.contingency_or_ci_method == "Baptista-Pike" { "Woolf" } else { "Baptista-Pike" },
        ),
        ContingencyCiTarget::RelativeRisk => (
            "edit.contingency.ci.koopman",
            ep.contingency_relative_risk_checked || cont.compute_odds_and_risk_ratios,
            "Koopman",
            if ep.contingency_rr_ci_method == "Koopman" { "Katz" } else { "Koopman" },
        ),
        ContingencyCiTarget::AttributableRisk => (
            "edit.contingency.ci.newcombe_wilson",
            ep.contingency_attributable_risk_checked,
            "Newcombe-Wilson",
            if ep.contingency_arr_ci_method == "Newcombe-Wilson" { "Wald" } else { "Newcombe-Wilson" },
        ),
        ContingencyCiTarget::Diagnostic => (
            "edit.contingency.ci.wilson_brown",
            cont.compute_sensitivity_specificity,
            "Wilson-Brown",
            if ep.contingency_diag_ci_method == "Wilson-Brown" { "Clopper-Pearson" } else { "Wilson-Brown" },
        ),
    };
    let gate = r6_contingency_ci_selector(rows, cols, checked, label);
    if !gate.enabled {
        return Err(conflict(gate.disabled_reason.unwrap_or_default()));
    }
    let mut map = serde_json::Map::new();
    map.insert(wid_key.to_owned(), serde_json::Value::String(next_val.to_owned()));
    patch_edit_params(app, &map)
}

fn toggle_diag_flag(app: &mut UiApp, flag: DiagToggleFlag) -> Result<(), EngineError> {
    if let Some(AnalysisKind::TwoGroupComparison(t)) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
        let gate = r4_two_group_diagnostics(t.method);
        if !gate.enabled {
            return Err(conflict(gate.disabled_reason.unwrap_or_default()));
        }
    }
    match flag {
        DiagToggleFlag::QqPlot => app.canvas_dialogs.edit_params.diag_qq_plot = !app.canvas_dialogs.edit_params.diag_qq_plot,
        DiagToggleFlag::Residuals => app.canvas_dialogs.edit_params.diag_residuals_plot = !app.canvas_dialogs.edit_params.diag_residuals_plot,
        DiagToggleFlag::Homoscedasticity => {
            app.canvas_dialogs.edit_params.diag_homoscedasticity = !app.canvas_dialogs.edit_params.diag_homoscedasticity;
        }
        DiagToggleFlag::Checklist => {
            app.canvas_dialogs.edit_params.diag_assumption_checklist = !app.canvas_dialogs.edit_params.diag_assumption_checklist;
        }
        DiagToggleFlag::Influence => {
            if let Some(AnalysisKind::GeneralizedLinearModel(g)) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| &d.kind) {
                let next_vif = !g.compute_vif;
                let mut map = serde_json::Map::new();
                map.insert(String::from("compute_vif"), serde_json::Value::Bool(next_vif));
                patch_edit_params(app, &map)?;
            }
            app.canvas_dialogs.edit_params.diag_influence_leverage = !app.canvas_dialogs.edit_params.diag_influence_leverage;
        }
    }
    Ok(())
}

fn parse_mult_method(raw: &str, ctrl: usize) -> Result<MultiplicityMethod, EngineError> {
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
        _ => Err(conflict(format!("unknown multiplicity `{raw}`"))),
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
