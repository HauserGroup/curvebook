//! Typed executors for `StatParamsIntent` and `AnalyzePickerIntent` (retiring pre-R15 `views/*` custom dispatch).

use curvebook_doc::{
    AnalysisKind, ContingencySpec, MultiplicityMethod, OneWayMethod, PValueFormat, RepeatedMeasuresDesign, ResidualCovariance, SurvivalAnalysisSpec,
    TwoGroupMethod,
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

fn source_sheet_col_count(app: &UiApp) -> usize {
    let doc = app.document();
    let Some(src_id) = app.canvas_dialogs.edit_params.draft.as_ref().and_then(|d| d.source_sheet_id.as_ref()) else {
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
        AnalyzePickerIntent::SelectCatalogEntry { entry_id } => {
            app.canvas_dialogs.analyze.select_catalog_entry(&entry_id);
        }
        AnalyzePickerIntent::TogglePcaPredictor { col } => app.canvas_dialogs.analyze.toggle_pca_predictor(col),
        AnalyzePickerIntent::SetPcaPredictors { cols } => app.canvas_dialogs.analyze.set_pca_predictors(cols),
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
        AnalyzePickerIntent::SetPValueStackMethod { method } => {
            app.canvas_dialogs.analyze.pvalue_stack_method = method;
        }
        AnalyzePickerIntent::SetPValueStackThreshold { threshold } => {
            app.canvas_dialogs.analyze.pvalue_stack_threshold = threshold.clamp(0.001, 0.50);
        }
        AnalyzePickerIntent::SetPValueStackColumn { column } => {
            app.canvas_dialogs.analyze.pvalue_stack_column = column;
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
        AnalyzePickerIntent::SetEquationId { equation_id } => {
            app.canvas_dialogs.analyze.equation_id = equation_id;
            app.canvas_dialogs.analyze.custom_dsl = None;
        }
    }
    Ok(UiIntentOutcome::ShellUpdated)
}

pub(super) fn apply_stat_params_intent(app: &mut UiApp, intent: StatParamsIntent) -> Result<UiIntentOutcome, EngineError> {
    match intent {
        StatParamsIntent::SetCustomTitle { title } => {
            app.canvas_dialogs.edit_params.set_custom_title(Some(title)).map_err(conflict)?;
        }
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
            app.canvas_dialogs.edit_params.set_compare_against_control(enabled);
        }
        StatParamsIntent::CycleControlColumn => handle_cycle_control_col(app)?,
        StatParamsIntent::SetCompareSelectedPairs { enabled } => {
            let cols = available_dataset_indices(app);
            app.canvas_dialogs.edit_params.set_compare_selected_pairs(enabled, &cols);
        }
        StatParamsIntent::ToggleSelectedPair { a, b } => {
            app.canvas_dialogs.edit_params.toggle_selected_pair(a, b);
        }
        StatParamsIntent::ToggleMultiplicityReportCi => handle_toggle_mult_ci(app),
        StatParamsIntent::ToggleEffectSizes => handle_toggle_effect_sizes(app),
        StatParamsIntent::SetPValueFormat { slug } => {
            let trimmed = slug.trim();
            let norm: String = trimmed.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
            let next = match norm.as_str() {
                "apa" => PValueFormat::Apa,
                "nejm" => PValueFormat::Nejm,
                "custom" => PValueFormat::new_custom(4, None),
                "fourdecimal" | "fourdecimals" | "default" | "standard" => PValueFormat::FourDecimal,
                _ if trimmed == "0.0001" => PValueFormat::FourDecimal,
                _ => return Err(conflict(format!("unknown P-value format `{slug}`"))),
            };
            app.canvas_dialogs.edit_params.set_p_value_format(next);
        }
        StatParamsIntent::StepPValueDigits { delta } => step_pvalue_digits(app, delta)?,
        StatParamsIntent::StepTwoGroupHypothetical { delta } => step_hypothetical_value(app, delta),
        StatParamsIntent::CycleContingencyCi { which } => cycle_contingency_ci(app, which)?,
        StatParamsIntent::ToggleCorrAllPairs => {
            let next = !app.canvas_dialogs.edit_params.correlation_all_pairs;
            app.canvas_dialogs.edit_params.set_correlation_all_pairs(next);
        }
        StatParamsIntent::ToggleCorrHeatmap => {
            let gate = r7_correlation_heatmap_toggle(app.canvas_dialogs.edit_params.correlation_all_pairs);
            if !gate.enabled {
                return Err(conflict(gate.disabled_reason.unwrap_or_default()));
            }
            let next = !app.canvas_dialogs.edit_params.correlation_heatmap;
            app.canvas_dialogs.edit_params.set_correlation_heatmap(next).map_err(conflict)?;
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
            let src = app.canvas_dialogs.edit_params.draft.as_ref().and_then(|d| d.source_sheet_id.as_ref());
            if next && !r1_paired_or_matched_allowed(app.document(), src).enabled {
                let mode = &mut app.canvas_dialogs.edit_params.schematic_mode;
                *mode = if mode == "detail" { String::from("auto") } else { String::from("detail") };
                return Ok(());
            }
            app.canvas_dialogs.edit_params.set_paired_or_matched(next);
            Ok(())
        }
        AnalysisKind::FactorialAnova(f) => {
            if f.repeated_measures != RepeatedMeasuresDesign::None {
                let next = if f.residual_covariance == ResidualCovariance::CompoundSymmetry { "unstructured" } else { "compound_symmetry" };
                return set_factorial_residual_cov(app, next);
            }
            let src = app.canvas_dialogs.edit_params.draft.as_ref().and_then(|d| d.source_sheet_id.as_ref());
            if !r1_paired_or_matched_allowed(app.document(), src).enabled {
                let mode = &mut app.canvas_dialogs.edit_params.schematic_mode;
                *mode = if mode == "detail" { String::from("auto") } else { String::from("detail") };
                return Ok(());
            }
            app.canvas_dialogs.edit_params.set_paired_or_matched(true);
            Ok(())
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
    let src = ep.draft.as_ref().and_then(|d| d.source_sheet_id.as_ref());
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
    let saved_gg = app.canvas_dialogs.edit_params.saved_rm_geisser_greenhouse;
    let next_method = match method_name.trim().to_ascii_lowercase().as_str() {
        "ordinary" | "ordinary_anova" | "anova" => OneWayMethod::OrdinaryAnova,
        "welch" | "brown_forsythe" | "brown_forsythe_welch" | "brown_forsythe_and_welch" => OneWayMethod::BrownForsytheAndWelch,
        "kruskal" | "kruskal_wallis" => OneWayMethod::KruskalWallis,
        "friedman" => OneWayMethod::Friedman,
        "rm" | "repeated_measures" => OneWayMethod::RepeatedMeasures { geisser_greenhouse: saved_gg },
        other => return Err(conflict(format!("unknown one-way ANOVA method `{other}`"))),
    };
    app.canvas_dialogs.edit_params.set_one_way_method(next_method);
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
    let cur = ep.draft.as_ref().is_none_or(
        |d| !matches!(d.kind, AnalysisKind::OneWayAnova(ref a) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })),
    );
    app.canvas_dialogs.edit_params.set_geisser_greenhouse(!cur).map_err(conflict)
}

fn set_factorial_residual_cov(app: &mut UiApp, raw: &str) -> Result<(), EngineError> {
    if !app.canvas_dialogs.edit_params.paired_or_matched {
        return Err(conflict(r1_sphericity_or_reml_control(false).disabled_reason.unwrap_or_default()));
    }
    let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
    let cov = match norm.as_str() {
        "compoundsymmetry" | "cs" | "randomintercept" => ResidualCovariance::CompoundSymmetry,
        "unstructured" | "un" | "general" => ResidualCovariance::Unstructured,
        _ => return Err(conflict(format!("unknown residual covariance structure `{raw}`"))),
    };
    app.canvas_dialogs.edit_params.set_residual_covariance(cov);
    Ok(())
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
    app.canvas_dialogs.edit_params.set_multiplicity_method(method).map_err(conflict)
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
    app.canvas_dialogs.edit_params.set_control_column(next);
    Ok(())
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
    let next = u8::try_from((cur + delta).clamp(1, 9)).unwrap_or(4);
    app.canvas_dialogs.edit_params.set_p_value_decimals(next).map_err(conflict)
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
    let (rows, cols, _) = source_table_shape(doc, ep.draft.as_ref().and_then(|d| d.source_sheet_id.as_ref()));
    let cont = match ep.draft.as_ref().map(|d| &d.kind) {
        Some(AnalysisKind::Contingency(c)) => c.clone(),
        _ => ContingencySpec::default(),
    };
    let (checked, label) = match which {
        ContingencyCiTarget::OddsRatio => (ep.contingency_odds_ratio_checked || cont.compute_odds_and_risk_ratios, "Baptista-Pike"),
        ContingencyCiTarget::RelativeRisk => (ep.contingency_relative_risk_checked || cont.compute_odds_and_risk_ratios, "Koopman"),
        ContingencyCiTarget::AttributableRisk => (ep.contingency_attributable_risk_checked, "Newcombe-Wilson"),
        ContingencyCiTarget::Diagnostic => (cont.compute_sensitivity_specificity, "Wilson-Brown"),
    };
    let gate = r6_contingency_ci_selector(rows, cols, checked, label);
    if !gate.enabled {
        return Err(conflict(gate.disabled_reason.unwrap_or_default()));
    }
    let ep = &mut app.canvas_dialogs.edit_params;
    match which {
        ContingencyCiTarget::OddsRatio => {
            ep.contingency_or_ci_method = String::from(if ep.contingency_or_ci_method == "Baptista-Pike" { "Woolf" } else { "Baptista-Pike" });
        }
        ContingencyCiTarget::RelativeRisk => {
            ep.contingency_rr_ci_method = String::from(if ep.contingency_rr_ci_method == "Koopman" { "Katz" } else { "Koopman" });
        }
        ContingencyCiTarget::AttributableRisk => {
            ep.contingency_arr_ci_method = String::from(if ep.contingency_arr_ci_method == "Newcombe-Wilson" { "Wald" } else { "Newcombe-Wilson" });
        }
        ContingencyCiTarget::Diagnostic => {
            ep.contingency_diag_ci_method =
                String::from(if ep.contingency_diag_ci_method == "Wilson-Brown" { "Clopper-Pearson" } else { "Wilson-Brown" });
        }
    }
    Ok(())
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
                app.canvas_dialogs.edit_params.set_glm_compute_vif(next_vif);
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

fn available_dataset_indices(app: &UiApp) -> Vec<usize> {
    let total = source_sheet_col_count(app);
    let selected = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| d.selected_data_sets.as_slice()).unwrap_or(&[]);
    if selected.is_empty() { (0..total).collect() } else { selected.iter().copied().filter(|&i| i < total).collect() }
}

fn current_multiplicity(ep: &EditAnalysisParametersState) -> Option<curvebook_doc::MultiplicitySpec> {
    let spec = ep.draft.as_ref()?;
    match &spec.kind {
        AnalysisKind::OneWayAnova(a) => Some(a.multiplicity.clone()),
        AnalysisKind::FactorialAnova(f) => Some(f.multiplicity.clone()),
        AnalysisKind::NestedComparison(n) => Some(n.multiplicity.clone()),
        AnalysisKind::MultipleRowTTests(m) => Some(m.multiplicity.clone()),
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => Some(multiplicity.clone()),
        _ => None,
    }
}
