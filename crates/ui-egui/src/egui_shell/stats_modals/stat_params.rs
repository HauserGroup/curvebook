//! Statistical parameter modal dispatcher, dynamic test banners, and shared tabs (Design router, Range, RM, Multiplicity, Options, Diagnostics).

use curvebook_doc::{
    AnalysisKind, GlmFamily, MultiplicityMethod, MultivariateSpec, OneWayMethod, OutlierSpec, PValueFormat, RepeatedMeasuresDesign,
    ResidualCovariance, SumOfSquaresType, SurvivalAnalysisSpec, TableArchetype, TwoGroupMethod,
};
use egui::{Context, CornerRadius, Key, Modifiers, Stroke, Ui, Vec2};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, EditAnalysisParametersState};
use crate::i18n::{normalize_locale, tr};
use crate::modal_predicates::{
    ControlEnablement, is_reml_mixed_effects_active, r1_paired_or_matched_allowed, r1b_post_hoc_option_enabled, r1b_sphericity_correction,
    r4_two_group_diagnostics, r9_pvalue_precision_spinner,
};

use crate::egui_shell::stat_anova_modal::{
    StepperSpec, render_modal_checkbox, render_modal_radio, render_modal_seg_btn, render_modal_textbox_row, render_section_title, render_stepper_row,
};
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};

use super::factorial_mv::{MultivariateDesignCtx, render_factorial_anova_design, render_multivariate_design_controls};
use super::helpers::{
    Wave4ModalShell, contingency_method_label, df_method_label, method_reference_text, multiplicity_short_label, render_cov_structure_picker,
    render_df_method_picker, render_formula_bar, render_modal_action_button, resolve_source_data_sheet, show_wave4_modal,
};
use super::model_formula::{build_live_symbolic_formula, render_stat_mv_data_tab, render_survival_design};
use super::nested_misc::{ContingencyDesignCtx, render_contingency_design, render_misc_stats_design};
use super::two_group::render_two_group_design;
use super::{StatsModalFrameReport, StatsModalsEguiState};

// ============================================================================
// 2. Generalized Statistical / GLM / Cox / PCA / PCR / Clustering / Survival Modals (§8.8)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatTabDef {
    slug: &'static str,
    label: &'static str,
}

fn tabs_for_spec(app: &UiApp, spec: &curvebook_doc::AnalysisSpec, locale: &str) -> Vec<StatTabDef> {
    let is_mv = resolve_source_data_sheet(app.document(), &spec.source_sheet_id)
        .is_some_and(|ds| matches!(ds.archetype, TableArchetype::MultipleVariables(_)));
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(_) => vec![
            StatTabDef { slug: "experimental_design", label: tr(locale, "stat_params.tab.design_test") },
            StatTabDef { slug: "repeated_measures", label: tr(locale, "stat_params.tab.repeated_measures") },
            StatTabDef { slug: "options", label: tr(locale, "stat_params.tab.options") },
            StatTabDef { slug: "diagnostics", label: tr(locale, "stat_params.tab.diagnostics") },
        ],
        AnalysisKind::FactorialAnova(_) => {
            let mut v = Vec::new();
            if is_mv {
                v.push(StatTabDef { slug: "data", label: tr(locale, "stat_params.tab.data") });
            }
            v.push(StatTabDef { slug: "experimental_design", label: tr(locale, "stat_params.tab.design_model") });
            v.push(StatTabDef { slug: "repeated_measures", label: tr(locale, "stat_params.tab.repeated_measures") });
            v.push(StatTabDef { slug: "multiple_comparisons", label: tr(locale, "stat_params.tab.comparisons") });
            v.push(StatTabDef { slug: "options", label: tr(locale, "stat_params.tab.options") });
            v
        }
        AnalysisKind::NestedComparison(_) | AnalysisKind::MultipleRowTTests(_) => vec![
            StatTabDef { slug: "experimental_design", label: tr(locale, "stat_params.tab.design_data") },
            StatTabDef { slug: "multiple_comparisons", label: tr(locale, "stat_params.tab.multiple_comparisons") },
            StatTabDef { slug: "options", label: tr(locale, "stat_params.tab.options") },
        ],
        AnalysisKind::GeneralizedLinearModel(_)
        | AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { .. })
        | AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { .. }) => vec![
            StatTabDef { slug: "data", label: tr(locale, "stat_params.tab.data") },
            StatTabDef { slug: "experimental_design", label: tr(locale, "stat_params.tab.model_formula") },
            StatTabDef { slug: "options", label: tr(locale, "stat_params.tab.options") },
            StatTabDef { slug: "diagnostics", label: tr(locale, "stat_params.tab.diagnostics") },
        ],
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { .. }) => {
            let mut v = Vec::new();
            if is_mv {
                v.push(StatTabDef { slug: "data", label: tr(locale, "stat_params.tab.data") });
            }
            v.push(StatTabDef { slug: "experimental_design", label: tr(locale, "stat_params.tab.survival_tests") });
            v.push(StatTabDef { slug: "multiple_comparisons", label: tr(locale, "stat_params.tab.pairwise_logrank") });
            v.push(StatTabDef { slug: "options", label: tr(locale, "stat_params.tab.conf_options") });
            v
        }
        AnalysisKind::LinearRegression(_) => {
            let mut v = Vec::new();
            if is_mv {
                v.push(StatTabDef { slug: "data", label: tr(locale, "stat_params.tab.data") });
            }
            v.push(StatTabDef { slug: "experimental_design", label: tr(locale, "stat_params.tab.parameters") });
            v.push(StatTabDef { slug: "range", label: tr(locale, "stat_params.tab.range_curve") });
            v.push(StatTabDef { slug: "options", label: tr(locale, "stat_params.tab.options") });
            v.push(StatTabDef { slug: "diagnostics", label: tr(locale, "stat_params.tab.diagnostics") });
            v
        }
        _ => {
            let mut v = Vec::new();
            if is_mv {
                v.push(StatTabDef { slug: "data", label: tr(locale, "stat_params.tab.data") });
            }
            v.push(StatTabDef { slug: "experimental_design", label: tr(locale, "stat_params.tab.parameters") });
            v.push(StatTabDef { slug: "options", label: tr(locale, "stat_params.tab.options") });
            v.push(StatTabDef { slug: "diagnostics", label: tr(locale, "stat_params.tab.diagnostics") });
            v
        }
    }
}

fn resolve_survival_banner(surv: &SurvivalAnalysisSpec) -> (String, String) {
    match surv {
        SurvivalAnalysisSpec::KaplanMeier { log_rank_mantel_cox, gehan_breslow_wilcoxon, trend_test, .. } => {
            let mut tests = Vec::new();
            if *log_rank_mantel_cox {
                tests.push("Mantel-Cox");
            }
            if *gehan_breslow_wilcoxon {
                tests.push("Gehan-Wilcoxon");
            }
            if *trend_test {
                tests.push("Log-Rank Trend");
            }
            let summary = if tests.is_empty() { String::from("KM curves only") } else { tests.join(" + ") };
            (String::from("Parameters: Kaplan-Meier Survival"), format!("Product-limit KM ({summary})"))
        }
        SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, predictor_columns, .. } => {
            (String::from("Parameters: Cox Proportional Hazards"), format!("Cox PH ({} covariates, {:?} ties)", predictor_columns.len(), tie_method))
        }
    }
}

fn resolve_multivariate_banner(mv: &MultivariateSpec, alpha: f64) -> (String, String) {
    match mv {
        MultivariateSpec::Pca { standardize, parallel_analysis_seed, pcr_outcome_column } => {
            let title = if pcr_outcome_column.is_some() {
                String::from("Parameters: Principal Component Regression (PCR)")
            } else {
                String::from("Parameters: Principal Component Analysis (PCA)")
            };
            let matrix = if *standardize { "Correlation matrix (standardized)" } else { "Covariance matrix (centered)" };
            let pa = if parallel_analysis_seed.is_some() { " + Horn parallel analysis" } else { "" };
            let outcome = pcr_outcome_column.map_or(String::new(), |col| format!(" | Outcome: col {}", col + 1));
            (title, format!("{matrix}{pa}{outcome}"))
        }
        MultivariateSpec::CorrelationMatrix { spearman_rank } => (
            String::from("Parameters: Correlation Matrix"),
            format!("{} pairwise matrix (α={alpha:.3})", if *spearman_rank { "Spearman rank r_s" } else { "Pearson product-moment r" }),
        ),
        MultivariateSpec::Clustering { k_means_k, distance, linkage, .. } => (
            String::from("Parameters: Clustering & Dendrogram"),
            format!("{:?} / {:?}{}", distance, linkage, k_means_k.map_or_else(String::new, |k| format!(" + k-means (k={k})"))),
        ),
        MultivariateSpec::MultipleImputation { imputations_m, response_column, .. } => (
            String::from("Parameters: Multiple Imputation (Rubin MICE)"),
            format!("m={imputations_m} imputations{}", if response_column.is_some() { " + pooled GLM" } else { "" }),
        ),
    }
}

/// Resolve the live dynamic header title and test banner for any `AnalysisSpec`.
#[must_use]
pub fn resolve_stat_params_banner(app: &UiApp, spec: &curvebook_doc::AnalysisSpec) -> (String, String) {
    let conf = spec.confidence_level.percent();
    let alpha = (1.0 - conf / 100.0).max(0.0001);
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(t) => {
            let tail = if t.two_tailed { "two-tailed" } else { "one-tailed" };
            let m = match t.method {
                TwoGroupMethod::UnpairedT => "Unpaired Student t-test",
                TwoGroupMethod::WelchT => "Welch's unpaired t-test",
                TwoGroupMethod::PairedT => "Paired Student t-test",
                TwoGroupMethod::RatioPairedT => "Ratio paired t-test",
                TwoGroupMethod::LognormalUnpairedT => "Lognormal unpaired t-test",
                TwoGroupMethod::MannWhitneyU => "Mann-Whitney U rank test",
                TwoGroupMethod::WilcoxonSignedRank => "Wilcoxon signed-rank test",
                TwoGroupMethod::KolmogorovSmirnov => "Kolmogorov-Smirnov test",
                TwoGroupMethod::OneSampleT { .. } => "One-sample t-test",
                TwoGroupMethod::OneSampleWilcoxon { .. } => "One-sample Wilcoxon test",
                TwoGroupMethod::PairedSignTest => "Paired binomial sign test",
            };
            (String::from("Parameters: Two-Group / One-Sample Test"), format!("{m} ({tail}, α={alpha:.3})"))
        }
        AnalysisKind::FactorialAnova(f) => {
            let kind = if f.three_way {
                "Three-way"
            } else if f.multifactor_columns.len() >= 2 {
                "Multifactor N-way"
            } else {
                "Two-way"
            };
            let ss = match f.ss_type {
                SumOfSquaresType::TypeI => "Type I SS",
                SumOfSquaresType::TypeIII => "Type III SS",
            };
            let rm = if f.mixed_effects_reml || is_reml_mixed_effects_active(&app.canvas_dialogs.edit_params, app.document()) {
                "REML mixed-effects"
            } else {
                match f.repeated_measures {
                    RepeatedMeasuresDesign::None => "Between-subjects",
                    RepeatedMeasuresDesign::RowFactorMatched => "Row-matched RM",
                    RepeatedMeasuresDesign::ColumnFactorMatched => "Col-matched RM",
                    RepeatedMeasuresDesign::BothFactorsMatched => "Doubly RM",
                }
            };
            (String::from("Parameters: Factorial / Multifactor ANOVA"), format!("{kind} ANOVA ({rm}, {ss})"))
        }
        AnalysisKind::NestedComparison(n) => (
            String::from("Parameters: Nested Comparison"),
            format!("{} ({})", if n.two_group_t_test { "Nested t-test" } else { "Nested 1-way ANOVA" }, df_method_label(n.df_method)),
        ),
        AnalysisKind::MultipleRowTTests(m) => {
            let test = if m.nonparametric {
                "Nonparametric rank"
            } else if m.paired {
                "Paired t"
            } else if m.assume_equal_sd {
                "Student t"
            } else {
                "Welch t"
            };
            (String::from("Parameters: Multiple Row t-Tests"), format!("{test} row tests + {}", multiplicity_short_label(m.multiplicity.method)))
        }
        AnalysisKind::Contingency(c) => (
            String::from("Parameters: Contingency / Categorical"),
            format!("{}{}", contingency_method_label(c.method), if c.compute_odds_and_risk_ratios { " + OR/RR" } else { "" }),
        ),
        AnalysisKind::RocCurve(r) => {
            let ci = match r.ci_method {
                curvebook_doc::RocCiMethod::DeLong => "DeLong (1988)",
                curvebook_doc::RocCiMethod::HanleyMcNeil => "Hanley-McNeil",
            };
            (String::from("Parameters: ROC Curve Analysis"), format!("AUC + {ci} CI ({conf:.0}%)"))
        }
        AnalysisKind::Survival(surv) => resolve_survival_banner(surv),
        AnalysisKind::GeneralizedLinearModel(glm) => {
            let fam = match glm.family {
                GlmFamily::Linear => "Gaussian Linear GLM (Identity)",
                GlmFamily::Logistic => "Binomial Logistic GLM (Logit)",
                GlmFamily::Poisson => "Poisson Count GLM (Log)",
            };
            (String::from("Parameters: Generalized Linear Model"), format!("{fam}, {} predictors", glm.predictor_columns.len()))
        }
        AnalysisKind::Multivariate(mv) => resolve_multivariate_banner(mv, alpha),
        AnalysisKind::DescriptiveStats(d) => (
            String::from("Parameters: Descriptive Statistics"),
            format!("{:?}{}", d.aggregation, if d.run_normality_battery { " + Normality battery" } else { "" }),
        ),
        AnalysisKind::OutlierDetection(o) => {
            let desc = match o {
                OutlierSpec::Rout { fdr_q_percent } => format!("ROUT (Q = {fdr_q_percent:.2}%)"),
                OutlierSpec::Grubbs { alpha, iterative } => format!("Grubbs (α={alpha:.3}{})", if *iterative { ", iterative" } else { "" }),
            };
            (String::from("Parameters: Identify Outliers"), desc)
        }
        AnalysisKind::BlandAltman(ba) => (
            String::from("Parameters: Bland-Altman Method Comparison"),
            format!("{} ({conf:.0}% LoA)", if ba.percent_difference { "% Difference vs Average" } else { "Difference (A-B) vs Average" }),
        ),
        AnalysisKind::PValueStack(ps) => {
            (String::from("Parameters: Analyze Stack of P Values"), format!("{} (threshold {:.3})", ps.method.label(), ps.threshold))
        }
        AnalysisKind::LinearRegression(lin) => {
            let summary = if lin.deming_model_ii {
                match lin.deming_error {
                    curvebook_doc::DemingErrorSpec::EqualError => String::from("Deming Model II orthogonal regression (λ = 1)"),
                    curvebook_doc::DemingErrorSpec::StandardDeviations { sd_x, sd_y } => {
                        format!("Deming Model II regression (SD_X={sd_x}, SD_Y={sd_y})")
                    }
                    curvebook_doc::DemingErrorSpec::VarianceRatio { lambda } => {
                        format!("Deming Model II regression (λ = {lambda})")
                    }
                }
            } else {
                let constraint = match lin.effective_fixed_point() {
                    Some((0.0, 0.0)) => String::from(" (through origin)"),
                    Some((x0, y0)) => format!(" (fixed ({x0}, {y0}))"),
                    None => String::new(),
                };
                let sa = if lin.standard_addition { ", standard addition" } else { "" };
                format!("Ordinary Least Squares (Model I){constraint}{sa}")
            };
            (String::from("Parameters: Simple Linear / Deming Regression"), summary)
        }
        AnalysisKind::PassingBablok(_) => {
            (String::from("Parameters: Passing-Bablok Regression"), format!("Nonparametric median-slope + Cusum ({conf:.0}% CI)"))
        }
        _ => (String::from("Parameters: Analysis"), format!("{} (α={alpha:.3})", spec.effective_title())),
    }
}

/// Render general Statistical / GLM / Cox / PCA / PCR / Clustering / Survival parameter modals.
pub fn render_stat_params_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        state.open_session_sheet_id = None;
        return StatsModalFrameReport::default();
    };
    let doc = app.document();
    let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref().or_else(|| doc.result_sheet(result_sheet_id).map(|rs| &rs.analysis)) else {
        state.open_session_sheet_id = None;
        return StatsModalFrameReport::default();
    };

    let locale = normalize_locale(&app.locale);
    let (header_title, banner) = resolve_stat_params_banner(app, spec);
    state.last_resolved_banner.clone_from(&banner);
    let tabs = tabs_for_spec(app, spec, locale);
    let ep = &app.canvas_dialogs.edit_params;

    // Sync active tab and transient input drafts cleanly when opening or switching sheets
    if state.open_session_sheet_id.as_deref() != Some(result_sheet_id.as_str()) {
        state.open_session_sheet_id = Some(result_sheet_id.as_str().to_owned());
        state.active_tab = tabs.iter().find(|t| t.slug == ep.active_tab).map_or(tabs[0].slug, |t| t.slug).to_owned();
        state.show_method_reference = false;
        let is_create_session = doc.result_sheet(result_sheet_id).is_none();
        if is_create_session
            && let Some(saved) = state.saved_defaults.iter().find(|d| std::mem::discriminant(&d.kind) == std::mem::discriminant(&spec.kind))
        {
            let mut applied = saved.clone();
            applied.source_sheet_id = spec.source_sheet_id.clone();
            applied.selected_data_sets.clone_from(&spec.selected_data_sets);
            applied.custom_title.clone_from(&spec.custom_title);
            if applied != *spec {
                intents.push_set_edit_params_draft(applied);
            }
            state.method_saved_as_default = true;
        } else {
            state.method_saved_as_default = state.saved_defaults.iter().any(|saved| {
                std::mem::discriminant(&saved.kind) == std::mem::discriminant(&spec.kind)
                    && saved.kind == spec.kind
                    && saved.confidence_level == spec.confidence_level
                    && saved.p_value_format == spec.p_value_format
            });
        }
        if let AnalysisKind::LinearRegression(lin) = &spec.kind {
            let (lo, hi) = lin.x_range_filter.unwrap_or_default();
            state.lin_x_min_draft = lo.map_or_else(String::new, |v| format!("{v}"));
            state.lin_x_max_draft = hi.map_or_else(String::new, |v| format!("{v}"));
            state.lin_plot_min_draft = lin.plot_x_min.map_or_else(String::new, |v| format!("{v}"));
            state.lin_plot_max_draft = lin.plot_x_max.map_or_else(String::new, |v| format!("{v}"));
            state.lin_segments_draft = lin.plot_segments.map_or_else(String::new, |v| format!("{v}"));
        } else {
            state.lin_x_min_draft.clear();
            state.lin_x_max_draft.clear();
            state.lin_plot_min_draft.clear();
            state.lin_plot_max_draft.clear();
            state.lin_segments_draft.clear();
        }
    } else {
        state.method_saved_as_default = state.saved_defaults.iter().any(|saved| {
            std::mem::discriminant(&saved.kind) == std::mem::discriminant(&spec.kind)
                && saved.kind == spec.kind
                && saved.confidence_level == spec.confidence_level
                && saved.p_value_format == spec.p_value_format
        });
        if !tabs.iter().any(|t| t.slug == state.active_tab) {
            state.active_tab = tabs[0].slug.to_owned();
        }
    }

    let is_create_mode = doc.result_sheet(result_sheet_id).is_none();
    let ok_lbl = if is_create_mode { tr(locale, "anova.run_analysis") } else { tr(locale, "btn.ok") };

    let mut report = StatsModalFrameReport::from(show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        state,
        Wave4ModalShell {
            widget_id: "dialog.edit_analysis_parameters",
            title: &header_title,
            subtitle_or_banner: Some(&banner),
            banner_widget_id: "edit.resolved_test_banner",
            max_width_pt: 780.0,
            max_height_pt: 570.0,
            use_body_scroll: true,
            footer_reserve_pt: 42.0,
        },
        |ui, state, meta, intents| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for tab in &tabs {
                    let en = crate::modal_predicates::evaluate_edit_params_tab(ep, tab.slug);
                    let selected = state.active_tab == tab.slug;
                    if render_modal_seg_btn(ui, tokens, meta, &format!("edit.tab.{}", tab.slug), tab.label, selected, &en) {
                        state.active_tab = tab.slug.to_owned();
                        intents.push_set_edit_params_active_tab(tab.slug);
                    }
                }
            });
            if let Some(formula) = build_live_symbolic_formula(spec, doc) {
                ui.add_space(4.0);
                render_formula_bar(ui, tokens, meta, "edit.formula_bar", "Model Formula", &formula);
            }
        },
        |ui, state, _w, _h, meta, intents| {
            if state.show_method_reference {
                egui::Frame::NONE
                    .fill(tokens.panel_bg)
                    .stroke(Stroke::new(1.0, tokens.border))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(Vec2::new(8.0, 6.0))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Method Reference & Citations").strong().color(tokens.accent));
                        ui.label(egui::RichText::new(method_reference_text(spec)).small().color(tokens.text_primary));
                    });
                ui.add_space(6.0);
            }
            let tab_slug = state.active_tab.clone();
            render_stat_params_tab_body(ui, state, StatTabBodyCtx { app, ep, spec, tab_slug: &tab_slug }, tokens, meta, intents);
        },
        |ui, state, meta, intents| {
            ui.horizontal(|ui| {
                if render_modal_action_button(
                    ui,
                    tokens,
                    meta,
                    "dialog.button.method_ref",
                    tr(locale, "anova.method_ref"),
                    false,
                    state.show_method_reference,
                    &ControlEnablement::enabled(),
                ) {
                    state.show_method_reference = !state.show_method_reference;
                }
                let save_lbl = if state.method_saved_as_default { tr(locale, "anova.saved_default") } else { tr(locale, "anova.save_default") };
                if render_modal_action_button(
                    ui,
                    tokens,
                    meta,
                    "dialog.button.save_default",
                    save_lbl,
                    false,
                    state.method_saved_as_default,
                    &ControlEnablement::enabled(),
                ) {
                    let disc = std::mem::discriminant(&spec.kind);
                    state.saved_defaults.retain(|d| std::mem::discriminant(&d.kind) != disc);
                    state.saved_defaults.push(spec.clone());
                    state.method_saved_as_default = true;
                    intents.push_save_edit_params_as_default();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if render_modal_action_button(ui, tokens, meta, "modal.ok", ok_lbl, true, false, &ControlEnablement::enabled()) {
                        intents.push_dialog_button("ok");
                    }
                    if !is_create_mode
                        && doc.result_sheet(result_sheet_id).is_some()
                        && render_modal_action_button(
                            ui,
                            tokens,
                            meta,
                            "modal.apply",
                            tr(locale, "anova.update_analysis"),
                            false,
                            false,
                            &ControlEnablement::enabled(),
                        )
                    {
                        intents.push_dialog_button("apply");
                    }
                    if render_modal_action_button(
                        ui,
                        tokens,
                        meta,
                        "modal.cancel",
                        tr(locale, "btn.cancel"),
                        false,
                        false,
                        &ControlEnablement::enabled(),
                    ) {
                        intents.push_dialog_button("cancel");
                    }
                });
            });
        },
    ));

    // Keyboard navigation: Ctrl+Tab/Ctrl+PageDown/Up between enabled tabs, Escape/Enter
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        if state.show_method_reference {
            state.show_method_reference = false;
        } else {
            intents.push_dialog_button("cancel");
        }
    } else if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        intents.push_dialog_button("ok");
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Tab) || i.consume_key(Modifiers::CTRL, Key::PageDown)) {
        step_stat_tab(ep, &tabs, state, 1, intents);
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) || i.consume_key(Modifiers::CTRL, Key::PageUp)) {
        step_stat_tab(ep, &tabs, state, -1, intents);
    }

    report.resolved_test_banner = banner;
    report
}

fn step_stat_tab(
    ep: &EditAnalysisParametersState,
    tabs: &[StatTabDef],
    state: &mut StatsModalsEguiState,
    dir: isize,
    intents: &mut PendingUiIntents,
) {
    let len = tabs.len() as isize;
    if len == 0 {
        return;
    }
    let cur = tabs.iter().position(|t| t.slug == state.active_tab).unwrap_or(0) as isize;
    for offset in 1..=len {
        let idx = (cur + dir * offset).rem_euclid(len) as usize;
        let cand = tabs[idx].slug;
        if crate::modal_predicates::evaluate_edit_params_tab(ep, cand).enabled {
            state.active_tab = cand.to_owned();
            intents.push_set_edit_params_active_tab(cand);
            return;
        }
    }
}

struct StatTabBodyCtx<'a> {
    app: &'a UiApp,
    ep: &'a EditAnalysisParametersState,
    spec: &'a curvebook_doc::AnalysisSpec,
    tab_slug: &'a str,
}

fn render_stat_params_tab_body(
    ui: &mut Ui,
    state: &mut StatsModalsEguiState,
    tab_ctx: StatTabBodyCtx<'_>,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let StatTabBodyCtx { app, ep, spec, tab_slug } = tab_ctx;
    let doc = app.document();
    match tab_slug {
        "data" => render_stat_mv_data_tab(ui, doc, spec, tokens, meta, intents),
        "range" => render_linear_range_tab(ui, state, spec, tokens, meta, intents),
        "repeated_measures" => render_stat_rm_tab(ui, app, ep, spec, tokens, meta, intents),
        "multiple_comparisons" => render_stat_multiplicity_tab(ui, app, ep, spec, tokens, meta, intents),
        "options" => render_stat_options_tab(ui, ep, spec, tokens, meta, intents),
        "diagnostics" => render_stat_diagnostics_tab(ui, app, spec, tokens, meta, intents),
        _ => render_stat_design_tab(ui, app, ep, spec, tokens, meta, intents),
    }
}

fn render_linear_range_tab(
    ui: &mut Ui,
    state: &mut StatsModalsEguiState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    if !matches!(spec.kind, AnalysisKind::LinearRegression(_)) {
        return;
    }
    render_section_title(ui, tokens, "Fit X Range Filter & Curve Plot Window");
    render_modal_textbox_row(
        ui,
        tokens,
        meta,
        intents,
        "fit.lr.range.input.x_min",
        "Restrict fit X >= (optional)",
        "auto min",
        &mut state.lin_x_min_draft,
        110.0,
    );
    render_modal_textbox_row(
        ui,
        tokens,
        meta,
        intents,
        "fit.lr.range.input.x_max",
        "Restrict fit X <= (optional)",
        "auto max",
        &mut state.lin_x_max_draft,
        110.0,
    );
    ui.add_space(4.0);
    render_modal_textbox_row(
        ui,
        tokens,
        meta,
        intents,
        "fit.lr.range.input.plot_min",
        "Plot curve start X (optional)",
        "data min",
        &mut state.lin_plot_min_draft,
        110.0,
    );
    render_modal_textbox_row(
        ui,
        tokens,
        meta,
        intents,
        "fit.lr.range.input.plot_max",
        "Plot curve end X (optional)",
        "data max",
        &mut state.lin_plot_max_draft,
        110.0,
    );
    render_modal_textbox_row(
        ui,
        tokens,
        meta,
        intents,
        "fit.lr.range.input.segments",
        "Curve segments (blank = auto)",
        "auto",
        &mut state.lin_segments_draft,
        110.0,
    );
}

fn render_stat_design_tab(
    ui: &mut Ui,
    app: &UiApp,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(_) => render_two_group_design(ui, app, ep, spec, tokens, meta, intents),
        AnalysisKind::FactorialAnova(_) => render_factorial_anova_design(ui, doc, ep, spec, tokens, meta, intents),
        AnalysisKind::GeneralizedLinearModel(glm) => {
            render_section_title(ui, tokens, "Exponential Family & Link Function");
            for (wid, lbl, fam) in [
                ("edit.glm.linear", "Gaussian (Identity link: Y = Xβ)", GlmFamily::Linear),
                ("edit.glm.logistic", "Binomial Logistic (Logit link: odds ratios)", GlmFamily::Logistic),
                ("edit.glm.poisson", "Poisson Count (Log link: rate ratios)", GlmFamily::Poisson),
            ] {
                if render_modal_radio(ui, tokens, meta, wid, lbl, glm.family == fam, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::GeneralizedLinearModel(g) = &mut next.kind {
                        g.family = fam;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.glm.compute_vif",
                "Compute Variance Inflation Factors (VIF)",
                glm.compute_vif,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::GeneralizedLinearModel(g) = &mut next.kind {
                    g.compute_vif = !glm.compute_vif;
                }
                intents.push_set_edit_params_draft(next);
            }
        }
        AnalysisKind::Survival(surv) => render_survival_design(ui, doc, spec, surv, tokens, meta, intents),
        AnalysisKind::Contingency(c) => render_contingency_design(ui, ContingencyDesignCtx { doc, ep, spec, c }, tokens, meta, intents),
        AnalysisKind::Multivariate(mv) => {
            render_multivariate_design_controls(ui, MultivariateDesignCtx { doc, ep, spec, mv }, tokens, meta, intents);
        }
        _ => render_misc_stats_design(ui, doc, spec, tokens, meta, intents),
    }
}

fn render_stat_rm_tab(
    ui: &mut Ui,
    app: &UiApp,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let reml_active = is_reml_mixed_effects_active(ep, app.document());
    match &spec.kind {
        AnalysisKind::FactorialAnova(f) => {
            render_section_title(ui, tokens, "Repeated-Measures Matching Axis");
            for (wid, lbl, rm) in [
                ("edit.fac.rm.none", "No matching (independent cells)", RepeatedMeasuresDesign::None),
                ("edit.fac.rm.row", "Matched along Row Factor", RepeatedMeasuresDesign::RowFactorMatched),
                ("edit.fac.rm.col", "Matched along Column Factor", RepeatedMeasuresDesign::ColumnFactorMatched),
                ("edit.fac.rm.both", "Repeated measures in both factors", RepeatedMeasuresDesign::BothFactorsMatched),
            ] {
                if render_modal_radio(ui, tokens, meta, wid, lbl, f.repeated_measures == rm, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                        fa.repeated_measures = rm;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.rm.mixed_effects_reml",
                "Fit Mixed-Effects REML (handles missing cells)",
                f.mixed_effects_reml || reml_active,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                    fa.mixed_effects_reml = !f.mixed_effects_reml;
                }
                intents.push_set_edit_params_draft(next);
            }
            render_df_method_picker(ui, spec, f.df_method, tokens, meta, intents);
            render_cov_structure_picker(ui, spec, f.residual_covariance, tokens, meta, intents);
        }
        AnalysisKind::TwoGroupComparison(_) => {
            let paired_en = r1_paired_or_matched_allowed(app.document(), Some(&spec.source_sheet_id));
            render_section_title(ui, tokens, "Within-Subject Matching");
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.paired_or_matched",
                "Each row is a matched subject pair",
                ep.paired_or_matched,
                &paired_en,
            ) {
                intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("edit.paired_or_matched") });
            }
        }
        _ => {}
    }
}

fn render_stat_multiplicity_tab(
    ui: &mut Ui,
    app: &UiApp,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let reml_active = is_reml_mixed_effects_active(ep, app.document());
    let cur = match &spec.kind {
        AnalysisKind::FactorialAnova(f) => f.multiplicity.method,
        AnalysisKind::NestedComparison(n) => n.multiplicity.method,
        AnalysisKind::MultipleRowTTests(m) => m.multiplicity.method,
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => multiplicity.method,
        _ => MultiplicityMethod::None,
    };
    render_section_title(ui, tokens, "Multiplicity Adjustment / Post-Hoc Method");
    for (slug, lbl, m) in [
        ("tukey", "Tukey HSD (all-pairs)", MultiplicityMethod::TukeyHsd),
        ("bonferroni", "Bonferroni (FWER)", MultiplicityMethod::Bonferroni),
        ("sidak", "Sidak (FWER)", MultiplicityMethod::Sidak),
        ("holm_sidak", "Holm-Sidak (step-down FWER)", MultiplicityMethod::HolmSidak),
        ("dunnett", "Dunnett (compare vs. control)", MultiplicityMethod::Dunnett { control_column: ep.control_column }),
        ("bh", "Benjamini-Hochberg (FDR)", MultiplicityMethod::BenjaminiHochbergFdr),
        ("bky", "Two-Stage BKY (adaptive FDR)", MultiplicityMethod::TwoStageBkyFdr),
        ("none", "None (uncorrected)", MultiplicityMethod::None),
    ] {
        let en = r1b_post_hoc_option_enabled(ep.assume_gaussian, reml_active, m);
        if render_modal_radio(ui, tokens, meta, &format!("edit.post_hoc.{slug}"), lbl, cur == m, &en) {
            let mut next = spec.clone();
            match &mut next.kind {
                AnalysisKind::FactorialAnova(f) => f.multiplicity.method = m,
                AnalysisKind::NestedComparison(n) => n.multiplicity.method = m,
                AnalysisKind::MultipleRowTTests(mr) => mr.multiplicity.method = m,
                AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { multiplicity, .. }) => multiplicity.method = m,
                _ => {}
            }
            intents.push_set_edit_params_draft(next);
        }
    }
}

fn render_stat_options_tab(
    ui: &mut Ui,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Confidence Level & Alpha");
    let conf_pct = spec.confidence_level.percent();
    ui.horizontal_wrapped(|ui| {
        for (wid, pct, lbl) in [("edit.conf.90", 90.0_f64, "90% CI"), ("edit.conf.95", 95.0, "95% CI"), ("edit.conf.99", 99.0, "99% CI")] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, (conf_pct - pct).abs() < 0.05, &ControlEnablement::enabled()) {
                intents.push_custom_view_action("stat_params", format!("set_confidence:{pct:.1}"));
            }
        }
    });

    ui.add_space(6.0);
    render_section_title(ui, tokens, "P-Value Display Precision");
    ui.horizontal_wrapped(|ui| {
        for (wid, slug, lbl, active) in [
            ("edit.pval.four_decimal", "four_decimal", "0.0001", matches!(spec.p_value_format, PValueFormat::FourDecimal)),
            ("edit.pval.apa", "apa", "APA", matches!(spec.p_value_format, PValueFormat::Apa)),
            ("edit.pval.nejm", "nejm", "NEJM", matches!(spec.p_value_format, PValueFormat::Nejm)),
            ("edit.pval.custom", "custom", "Custom", matches!(spec.p_value_format, PValueFormat::Custom { .. })),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, active, &ControlEnablement::enabled()) {
                intents.push_custom_view_action("stat_params", format!("set_pvalue_format:{slug}"));
            }
        }
    });

    let digits = match spec.p_value_format {
        PValueFormat::FourDecimal => 4,
        PValueFormat::Apa | PValueFormat::Nejm => 3,
        PValueFormat::Custom { decimals, .. } => decimals,
    };
    let r9_en = r9_pvalue_precision_spinner(&spec.p_value_format);
    let step = render_stepper_row(
        ui,
        tokens,
        meta,
        StepperSpec { id_prefix: "edit.pvalue.digits", label: "Custom P-Value Decimal Digits", value_text: &digits.to_string(), enablement: &r9_en },
    );
    if step != 0 {
        intents.push_custom_view_action("stat_params", format!("spinner:edit.pvalue.digits:{step}"));
    }

    if let AnalysisKind::OneWayAnova(a) = &spec.kind {
        let reml_active = a.mixed_effects_reml || a.residual_covariance == ResidualCovariance::Unstructured;
        let sph = r1b_sphericity_correction(ep.paired_or_matched, reml_active);
        let gg_checked = !matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false });
        if render_modal_checkbox(ui, tokens, meta, "edit.sphericity.geisser_greenhouse", "Geisser-Greenhouse", gg_checked, &sph) {
            intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("edit.sphericity.geisser_greenhouse") });
        }
    }
}

fn render_stat_diagnostics_tab(
    ui: &mut Ui,
    app: &UiApp,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Diagnostic Plots & Residual Checks");
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(t) => {
            let diag_en = r4_two_group_diagnostics(t.method);
            ui.label(
                egui::RichText::new("Client preview toggles: ResultSheet diagnostics tabs are generated automatically by the engine.")
                    .small()
                    .color(tokens.text_muted),
            );
            ui.add_space(4.0);
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.diag.qq",
                "Quantile-Quantile (QQ) residual plot",
                app.view_shards.stat_params.diag_qq_plot,
                &diag_en,
            ) {
                intents.push_custom_view_action("stat_params", "toggle_diag:qq");
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.diag.shapiro",
                "Shapiro-Wilk normality check",
                app.view_shards.stat_params.diag_assumption_checklist,
                &diag_en,
            ) {
                intents.push_custom_view_action("stat_params", "toggle_diag:checklist");
            }
        }
        AnalysisKind::LinearRegression(lin) => {
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.lin.conf_bands",
                "Plot 95% confidence bands",
                lin.confidence_bands,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.confidence_bands = !lin.confidence_bands;
                }
                intents.push_set_edit_params_draft(next);
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.lin.pred_bands",
                "Plot prediction bands",
                lin.prediction_bands,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.prediction_bands = !lin.prediction_bands;
                }
                intents.push_set_edit_params_draft(next);
            }
        }
        _ => {
            ui.label(
                egui::RichText::new("Residual QQ plots, assumption checklist, and diagnostic exports are enabled automatically.")
                    .small()
                    .color(tokens.text_muted),
            );
        }
    }
}
