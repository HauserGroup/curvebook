//! Nested, Multiple Row t-Tests, Contingency, Descriptive, Outlier, and Linear Regression design controls.

use curvebook_doc::{
    AnalysisKind, ContingencyMethod, DemingErrorSpec, FrequencyBinningRule, FrequencyDistributionSpec, FrequencyOutputMode, KappaWeighting,
    LeveneCenter, OutlierSpec, PValueStackMethod, ProjectDocument, SheetId, SubcolumnAggregation,
};
use egui::{Id, TextEdit, Ui, Vec2};

use crate::UiApp;
use crate::dialogs_canvas::EditAnalysisParametersState;
use crate::grid_view::parse_localized_number_input;
use crate::i18n::{LocalePreset, tr};
use crate::inspect::ZLayer;
use crate::modal_predicates::{
    ControlEnablement, r5_contingency_2x2_effect_sizes, r5_contingency_cochran_armitage_trend, r6_contingency_ci_selector, r8_subcolumn_aggregation,
    source_table_shape,
};

use crate::egui_shell::stat_anova_modal::{
    StepperSpec, column_titles_for_draft, record_widget, render_modal_checkbox, render_modal_radio, render_modal_seg_btn, render_section_title,
    render_stepper_row,
};
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};

use super::StatsModalsEguiState;
use super::helpers::{render_dataset_checklist, render_df_method_picker};

pub(super) struct ContingencyDesignCtx<'a> {
    pub(super) doc: &'a ProjectDocument,
    pub(super) ep: &'a EditAnalysisParametersState,
    pub(super) spec: &'a curvebook_doc::AnalysisSpec,
    pub(super) c: &'a curvebook_doc::ContingencySpec,
}

pub(super) fn render_contingency_design(
    ui: &mut Ui,
    cx: ContingencyDesignCtx<'_>,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let ContingencyDesignCtx { doc, ep, spec, c } = cx;
    let (rows, cols, _) = source_table_shape(doc, spec.source_sheet_id.as_ref());
    let two_by_two_en = r5_contingency_2x2_effect_sizes(rows, cols);
    let trend_en = r5_contingency_cochran_armitage_trend(rows, cols);
    render_section_title(ui, tokens, "Categorical Test & Effect Sizes");
    let cur_kappa_weighting = match c.method {
        ContingencyMethod::CohenKappa { weighting } => weighting,
        _ => KappaWeighting::Unweighted,
    };
    let cont_methods = [
        ("edit.cont.fisher", "Fisher's exact test", ContingencyMethod::FisherExact, ControlEnablement::enabled()),
        ("edit.cont.chisq", "Pearson Chi-square", ContingencyMethod::PearsonChiSquare { yates_continuity: false }, ControlEnablement::enabled()),
        ("edit.cont.yates", "Chi-square + Yates", ContingencyMethod::PearsonChiSquare { yates_continuity: true }, two_by_two_en.clone()),
        ("edit.cont.g2", "Likelihood Ratio G²", ContingencyMethod::LikelihoodRatioG2, ControlEnablement::enabled()),
        ("edit.cont.barnard", "Barnard's exact test", ContingencyMethod::BarnardExact, two_by_two_en.clone()),
        ("edit.cont.trend", "Cochran-Armitage trend", ContingencyMethod::CochranArmitageTrend, trend_en),
        ("edit.cont.mcnemar", "McNemar's paired test", ContingencyMethod::McNemarPaired, two_by_two_en.clone()),
        ("edit.cont.kappa", "Cohen's Kappa", ContingencyMethod::CohenKappa { weighting: cur_kappa_weighting }, ControlEnablement::enabled()),
        ("edit.cont.fleiss_kappa", "Fleiss' Kappa", ContingencyMethod::FleissKappa, ControlEnablement::enabled()),
        ("edit.cont.exact_binomial", "Exact Binomial test", ContingencyMethod::ExactBinomial, ControlEnablement::enabled()),
    ];
    egui::Grid::new("edit.cont.methods.grid").num_columns(2).spacing([8.0, 2.0]).show(ui, |grid| {
        for (idx, (wid, lbl, m, en)) in cont_methods.into_iter().enumerate() {
            let is_selected = match (c.method, m) {
                (ContingencyMethod::CohenKappa { .. }, ContingencyMethod::CohenKappa { .. }) => true,
                _ => c.method == m,
            };
            if render_modal_radio(grid, tokens, meta, wid, lbl, is_selected, &en) {
                let mut next = spec.clone();
                if let AnalysisKind::Contingency(cg) = &mut next.kind {
                    cg.method = m;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if idx % 2 == 1 {
                grid.end_row();
            }
        }
    });
    if let ContingencyMethod::CohenKappa { weighting } = c.method {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Kappa Weighting:").small().color(tokens.text_muted));
            for (wid, lbl, w) in [
                ("edit.cont.kappa.unweighted", "Unweighted", KappaWeighting::Unweighted),
                ("edit.cont.kappa.linear", "Linear", KappaWeighting::Linear),
                ("edit.cont.kappa.quadratic", "Quadratic", KappaWeighting::Quadratic),
            ] {
                if render_modal_seg_btn(ui, tokens, meta, wid, lbl, weighting == w, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::Contingency(cg) = &mut next.kind {
                        cg.method = ContingencyMethod::CohenKappa { weighting: w };
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
            }
        });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.cont.or_rr",
        "Odds Ratio / Relative Risk + Attributable Risk",
        c.compute_odds_and_risk_ratios,
        &two_by_two_en,
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::Contingency(cg) = &mut next.kind {
            cg.compute_odds_and_risk_ratios = !c.compute_odds_and_risk_ratios;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.cont.sens_spec",
        "Diagnostic Sensitivity / Specificity / PPV / NPV",
        c.compute_sensitivity_specificity,
        &two_by_two_en,
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::Contingency(cg) = &mut next.kind {
            cg.compute_sensitivity_specificity = !c.compute_sensitivity_specificity;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.contingency.attributable_risk",
        "Attributable Risk (ARR & NNT / NNH)",
        ep.contingency_attributable_risk_checked,
        &two_by_two_en,
    ) {
        intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("edit.contingency.attributable_risk") });
    }

    ui.add_space(6.0);
    render_section_title(ui, tokens, "2×2 Effect-Size CI Methods");
    let or_ci_en =
        r6_contingency_ci_selector(rows, cols, ep.contingency_odds_ratio_checked || c.compute_odds_and_risk_ratios, "Baptista-Pike / Woolf");
    let rr_ci_en = r6_contingency_ci_selector(rows, cols, ep.contingency_relative_risk_checked || c.compute_odds_and_risk_ratios, "Koopman");
    let arr_ci_en = r6_contingency_ci_selector(rows, cols, ep.contingency_attributable_risk_checked, "Newcombe-Wilson");
    let diag_ci_en = r6_contingency_ci_selector(rows, cols, c.compute_sensitivity_specificity, "Wilson-Brown");
    ui.horizontal_wrapped(|ui| {
        for (wid, lbl, active, en, target) in [
            (
                "edit.contingency.ci.baptista_pike",
                format!("OR CI: {}", ep.contingency_or_ci_method),
                ep.contingency_or_ci_method == "Baptista-Pike",
                or_ci_en,
                crate::egui_shell::ContingencyCiTarget::OddsRatio,
            ),
            (
                "edit.contingency.ci.koopman",
                format!("RR CI: {}", ep.contingency_rr_ci_method),
                ep.contingency_rr_ci_method == "Koopman",
                rr_ci_en,
                crate::egui_shell::ContingencyCiTarget::RelativeRisk,
            ),
            (
                "edit.contingency.ci.newcombe_wilson",
                format!("ARR CI: {}", ep.contingency_arr_ci_method),
                ep.contingency_arr_ci_method == "Newcombe-Wilson",
                arr_ci_en,
                crate::egui_shell::ContingencyCiTarget::AttributableRisk,
            ),
            (
                "edit.contingency.ci.wilson_brown",
                format!("Diag CI: {}", ep.contingency_diag_ci_method),
                ep.contingency_diag_ci_method == "Wilson-Brown",
                diag_ci_en,
                crate::egui_shell::ContingencyCiTarget::Diagnostic,
            ),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, &lbl, active, &en) {
                intents.push(UiIntent::StatParams(crate::egui_shell::StatParamsIntent::CycleContingencyCi { which: target }));
            }
        }
    });
    if let Some(err) = ep.unsupported_contingency_ci_error() {
        ui.add_space(4.0);
        let resp = ui.label(egui::RichText::new(&err).small().color(tokens.error));
        record_widget(ui, meta, &resp, "edit.contingency.ci.error", "alert", &err, None, None, true, None, ZLayer::ModalDialog, false, None);
    }
}

pub(super) fn render_misc_stats_design(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    match &spec.kind {
        AnalysisKind::NestedComparison(n) => {
            render_section_title(ui, tokens, "Nested Model Structure");
            for (wid, lbl, val) in
                [("edit.nested.t", "Nested t-test (2 groups)", true), ("edit.nested.anova", "Nested One-Way REML ANOVA (k >= 2)", false)]
            {
                if render_modal_radio(ui, tokens, meta, wid, lbl, n.two_group_t_test == val, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::NestedComparison(ns) = &mut next.kind {
                        ns.two_group_t_test = val;
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
            }
            render_df_method_picker(ui, spec, n.df_method, &ControlEnablement::enabled(), tokens, meta, intents);
            render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        }
        AnalysisKind::MultipleRowTTests(m) => {
            render_section_title(ui, tokens, "Row-Wise Comparison Test");
            for (wid, lbl, checked, idx) in [
                ("edit.row_t.paired", "Paired comparisons per row", m.paired, 0u8),
                ("edit.row_t.nonparam", "Nonparametric rank test (Mann-Whitney / Wilcoxon)", m.nonparametric, 1),
            ] {
                if render_modal_checkbox(ui, tokens, meta, wid, lbl, checked, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::MultipleRowTTests(mr) = &mut next.kind {
                        if idx == 0 {
                            mr.paired = !m.paired;
                        } else {
                            mr.nonparametric = !m.nonparametric;
                        }
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
            }
            if !m.nonparametric
                && !m.paired
                && render_modal_checkbox(
                    ui,
                    tokens,
                    meta,
                    "edit.row_t.equal_sd",
                    "Assume equal SD (pooled Student t)",
                    m.assume_equal_sd,
                    &ControlEnablement::enabled(),
                )
            {
                let mut next = spec.clone();
                if let AnalysisKind::MultipleRowTTests(mr) = &mut next.kind {
                    mr.assume_equal_sd = !m.assume_equal_sd;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
        AnalysisKind::DescriptiveStats(d) => {
            let locale = tokens.locale;
            let (_, _, max_sub) = source_table_shape(doc, spec.source_sheet_id.as_ref());
            let sub_en = r8_subcolumn_aggregation(max_sub);
            render_section_title(ui, tokens, "Replicate Aggregation & Distribution Tests");
            for (wid, lbl, agg) in [
                ("edit.desc.row_means", "Collapse subcolumns by row mean", SubcolumnAggregation::RowMeans),
                ("edit.desc.per_subcol", "Separate summary per subcolumn", SubcolumnAggregation::PerSubcolumn),
                ("edit.desc.pooled", "Pool all replicate cells", SubcolumnAggregation::PooledGrandSample),
            ] {
                if render_modal_radio(ui, tokens, meta, wid, lbl, d.aggregation == agg, &sub_en) {
                    let mut next = spec.clone();
                    if let AnalysisKind::DescriptiveStats(ds) = &mut next.kind {
                        ds.aggregation = agg;
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.desc.normality",
                "Run Shapiro-Wilk, D'Agostino-Pearson, AD & KS normality tests",
                d.run_normality_battery,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::DescriptiveStats(ds) = &mut next.kind {
                    ds.run_normality_battery = !d.run_normality_battery;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            let custom_pct_enabled = d.custom_upper_percentile.is_some();
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.desc.custom_pct_toggle",
                "Compute custom symmetric percentiles (100 - P, P)",
                custom_pct_enabled,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::DescriptiveStats(ds) = &mut next.kind {
                    ds.custom_upper_percentile = if custom_pct_enabled { None } else { Some(90.0) };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if let Some(p_hi) = d.custom_upper_percentile {
                let sid = spec.source_sheet_id.as_ref().map_or("", SheetId::as_str);
                ui.horizontal_wrapped(|ui| {
                    if let Some(new_p) = render_linear_numeric_input(
                        ui,
                        state,
                        &app.locale_preset,
                        meta,
                        NumericInputSpec {
                            salt: ("edit_desc_custom_pct", sid),
                            widget_id: "edit.desc.custom_pct",
                            label: "Upper percentile P (50 < P < 100)",
                            current: p_hi,
                            bounds: NumericBounds::OpenRange(50.0, 100.0),
                        },
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::DescriptiveStats(ds) = &mut next.kind {
                            ds.custom_upper_percentile = Some(new_p);
                        }
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                    }
                    let p_lo = ((100.0 - p_hi) * 1e6).round() / 1e6;
                    ui.label(egui::RichText::new(format!("({p_lo}% & {p_hi}%, Type 6)")).small().color(tokens.text_muted));
                });
                render_numeric_field_errors(ui, state, tokens, &["edit.desc.custom_pct"]);
            } else {
                state.numeric_field_errors.remove("edit.desc.custom_pct");
            }

            ui.add_space(6.0);
            render_section_title(ui, tokens, tr(locale, "desc.displayed_stats"));
            let vm = d.visible_metrics;
            type MetricToggle<'a> = (&'static str, &'a str, bool, fn(&mut curvebook_doc::DescriptiveMetricVisibility));
            let metric_toggles: [MetricToggle<'_>; 12] = [
                ("edit.desc.metric.min_max_range", tr(locale, "desc.metric.min_max_range"), vm.min_max_range, |v| v.min_max_range = !v.min_max_range),
                ("edit.desc.metric.quartiles", tr(locale, "desc.metric.quartiles"), vm.quartiles, |v| v.quartiles = !v.quartiles),
                ("edit.desc.metric.mean_sd_sem", tr(locale, "desc.metric.mean_sd_sem"), vm.mean_sd_sem, |v| v.mean_sd_sem = !v.mean_sd_sem),
                ("edit.desc.metric.ci_of_mean", tr(locale, "desc.metric.ci_of_mean"), vm.ci_of_mean, |v| v.ci_of_mean = !v.ci_of_mean),
                ("edit.desc.metric.ci_of_sd", tr(locale, "desc.metric.ci_of_sd"), vm.ci_of_sd, |v| v.ci_of_sd = !v.ci_of_sd),
                ("edit.desc.metric.ci_of_median", tr(locale, "desc.metric.ci_of_median"), vm.ci_of_median, |v| v.ci_of_median = !v.ci_of_median),
                ("edit.desc.metric.cv_percent", tr(locale, "desc.metric.cv_percent"), vm.cv_percent, |v| v.cv_percent = !v.cv_percent),
                ("edit.desc.metric.geometric_mean", tr(locale, "desc.metric.geometric_mean"), vm.geometric_mean, |v| {
                    v.geometric_mean = !v.geometric_mean
                }),
                ("edit.desc.metric.harmonic_mean", tr(locale, "desc.metric.harmonic_mean"), vm.harmonic_mean, |v| v.harmonic_mean = !v.harmonic_mean),
                ("edit.desc.metric.quadratic_mean", tr(locale, "desc.metric.quadratic_mean"), vm.quadratic_mean, |v| {
                    v.quadratic_mean = !v.quadratic_mean
                }),
                ("edit.desc.metric.skewness_kurtosis", tr(locale, "desc.metric.skewness_kurtosis"), vm.skewness_kurtosis, |v| {
                    v.skewness_kurtosis = !v.skewness_kurtosis
                }),
                ("edit.desc.metric.sum", tr(locale, "desc.metric.sum"), vm.sum, |v| v.sum = !v.sum),
            ];
            egui::Grid::new("edit.desc.metrics.grid").num_columns(2).spacing([8.0, 2.0]).show(ui, |grid| {
                for (idx, (wid, lbl, checked, toggle_fn)) in metric_toggles.into_iter().enumerate() {
                    if render_modal_checkbox(grid, tokens, meta, wid, lbl, checked, &ControlEnablement::enabled()) {
                        let mut next = spec.clone();
                        if let AnalysisKind::DescriptiveStats(ds) = &mut next.kind {
                            toggle_fn(&mut ds.visible_metrics);
                        }
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                    }
                    if idx % 2 == 1 {
                        grid.end_row();
                    }
                }
            });
            render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        }
        AnalysisKind::OutlierDetection(out) => {
            render_outlier_design(ui, doc, spec, out, tokens, meta, intents);
        }
        AnalysisKind::BlandAltman(ba) => {
            render_section_title(ui, tokens, "Bland-Altman Comparison");
            for (wid, lbl, pct) in
                [("edit.ba.diff", "Difference (A - B) vs Average", false), ("edit.ba.pct", "Percentage difference 100*(A-B)/Mean", true)]
            {
                if render_modal_radio(ui, tokens, meta, wid, lbl, ba.percent_difference == pct, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::BlandAltman(b) = &mut next.kind {
                        b.percent_difference = pct;
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
            }
        }
        AnalysisKind::PValueStack(ps) => {
            render_pvalue_stack_design(ui, doc, spec, ps, tokens, meta, intents);
        }
        AnalysisKind::LeveneTest(lt) => {
            render_section_title(ui, tokens, "Deviations measured from each group's");
            for (wid, lbl, center) in [
                ("edit.levene.center.median", "Median (Brown-Forsythe, robust; recommended)", LeveneCenter::Median),
                ("edit.levene.center.mean", "Mean (Levene's original test)", LeveneCenter::Mean),
            ] {
                if render_modal_radio(ui, tokens, meta, wid, lbl, lt.center == center, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::LeveneTest(l) = &mut next.kind {
                        l.center = center;
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
            }
            render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        }
        AnalysisKind::FrequencyDistribution(fd) => {
            render_frequency_distribution_design(ui, app, state, spec, fd, tokens, meta, intents);
        }
        AnalysisKind::LinearRegression(lin) => {
            render_linear_regression_design(ui, app, state, spec, lin, tokens, meta, intents);
        }
        AnalysisKind::SimpleLogisticRegression(sl) => {
            render_simple_logistic_design(ui, app, state, spec, sl, tokens, meta, intents);
        }
        AnalysisKind::FitSplineLowess(sp) => {
            render_spline_lowess_design(ui, doc, spec, sp, tokens, meta, intents);
        }
        AnalysisKind::AreaUnderCurve(auc) => {
            render_area_under_curve_design(ui, app, state, spec, auc, tokens, meta, intents);
        }
        AnalysisKind::PassingBablok(pb) => {
            render_section_title(ui, tokens, "Method Comparison Columns (X Reference vs Y Candidate)");
            let titles = column_titles_for_draft(doc, spec.source_sheet_id.as_ref());
            for (idx, t) in titles.iter().enumerate() {
                ui.horizontal(|ui| {
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.pb.x.{idx}"),
                        &format!("Ref X: {t}"),
                        pb.x_column == idx,
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::PassingBablok(p) = &mut next.kind {
                            p.x_column = idx;
                        }
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.pb.y.{idx}"),
                        &format!("Method Y: {t}"),
                        pb.y_column == idx,
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::PassingBablok(p) = &mut next.kind {
                            p.y_column = idx;
                        }
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                    }
                });
            }
        }
        AnalysisKind::RocCurve(r) => {
            render_section_title(ui, tokens, "ROC AUC Confidence Interval Estimator");
            for (wid, lbl, method) in [
                ("edit.roc.delong", "DeLong et al. (1988) structural components", curvebook_doc::RocCiMethod::DeLong),
                ("edit.roc.hanley", "Hanley & McNeil (1982) approximation", curvebook_doc::RocCiMethod::HanleyMcNeil),
            ] {
                if render_modal_radio(ui, tokens, meta, wid, lbl, r.ci_method == method, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::RocCurve(rc) = &mut next.kind {
                        rc.ci_method = method;
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
            }
            render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        }
        AnalysisKind::Ancova(anc) => render_ancova_design(ui, app, spec, anc, tokens, meta, intents),
        AnalysisKind::CurveCalculus(cc) => render_curve_calculus_design(ui, app, state, spec, cc, tokens, meta, intents),
        _ => render_dataset_checklist(ui, doc, spec, tokens, meta, intents),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_ancova_design(
    ui: &mut Ui,
    app: &UiApp,
    spec: &curvebook_doc::AnalysisSpec,
    anc: &curvebook_doc::AncovaSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    let locale = tokens.locale;
    render_section_title(ui, tokens, tr(locale, "ancova.section.model"));
    for (wid, lbl, ss) in [
        ("edit.ancova.ss.type3", tr(locale, "ancova.ss.type3"), curvebook_doc::SumOfSquaresType::TypeIII),
        ("edit.ancova.ss.type1", tr(locale, "ancova.ss.type1"), curvebook_doc::SumOfSquaresType::TypeI),
    ] {
        if render_modal_radio(ui, tokens, meta, wid, lbl, anc.ss_type == ss, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::Ancova(a) = &mut next.kind {
                a.ss_type = ss;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }
    ui.add_space(4.0);
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.ancova.homogeneity_of_slopes",
        tr(locale, "ancova.homogeneity_of_slopes"),
        anc.test_homogeneity_of_slopes,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::Ancova(a) = &mut next.kind {
            a.test_homogeneity_of_slopes = !anc.test_homogeneity_of_slopes;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.ancova.effect_sizes",
        tr(locale, "ancova.effect_sizes"),
        anc.compute_effect_sizes,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::Ancova(a) = &mut next.kind {
            a.compute_effect_sizes = !anc.compute_effect_sizes;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if let Some(ds) = spec.source_sheet_id.as_ref().and_then(|src| super::helpers::resolve_source_data_sheet(doc, src)) {
        match &ds.archetype {
            curvebook_doc::TableArchetype::Xy(_) => {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tr(locale, "ancova.xy_mapping_hint")).small().color(tokens.text_muted));
                render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
            }
            curvebook_doc::TableArchetype::MultipleVariables(_) => {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tr(locale, "ancova.mv_mapping_hint")).small().color(tokens.text_muted));
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_curve_calculus_design(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    spec: &curvebook_doc::AnalysisSpec,
    cc: &curvebook_doc::CurveCalculusSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    use curvebook_doc::CurveCalculusOp;

    let doc = app.document();
    let locale = tokens.locale;
    let sid = spec.source_sheet_id.as_ref().map_or("", SheetId::as_str);

    render_section_title(ui, tokens, tr(locale, "curve_calc.section.operation"));
    let choices: [(&str, &str, CurveCalculusOp, bool); 6] = [
        (
            "edit.curve_calc.op.savitzky_golay",
            tr(locale, "curve_calc.op.savitzky_golay"),
            CurveCalculusOp::SmoothSavitzkyGolay { window: 5, order: 2 },
            matches!(cc.operation, CurveCalculusOp::SmoothSavitzkyGolay { .. }),
        ),
        (
            "edit.curve_calc.op.moving_avg",
            tr(locale, "curve_calc.op.moving_avg"),
            CurveCalculusOp::SmoothMovingAverage { window: 5 },
            matches!(cc.operation, CurveCalculusOp::SmoothMovingAverage { .. }),
        ),
        (
            "edit.curve_calc.op.lowess",
            tr(locale, "curve_calc.op.lowess"),
            CurveCalculusOp::SmoothLowess { span: 0.3, robustness_iters: 2 },
            matches!(cc.operation, CurveCalculusOp::SmoothLowess { .. }),
        ),
        (
            "edit.curve_calc.op.first_deriv",
            tr(locale, "curve_calc.op.first_deriv"),
            CurveCalculusOp::FirstDerivative,
            matches!(cc.operation, CurveCalculusOp::FirstDerivative),
        ),
        (
            "edit.curve_calc.op.second_deriv",
            tr(locale, "curve_calc.op.second_deriv"),
            CurveCalculusOp::SecondDerivative,
            matches!(cc.operation, CurveCalculusOp::SecondDerivative),
        ),
        (
            "edit.curve_calc.op.integral",
            tr(locale, "curve_calc.op.integral"),
            CurveCalculusOp::CumulativeIntegral { baseline: 0.0 },
            matches!(cc.operation, CurveCalculusOp::CumulativeIntegral { .. }),
        ),
    ];
    for (wid, lbl, default_op, is_selected) in choices {
        if render_modal_radio(ui, tokens, meta, wid, lbl, is_selected, &ControlEnablement::enabled()) && !is_selected {
            let mut next = spec.clone();
            if let AnalysisKind::CurveCalculus(c) = &mut next.kind {
                c.operation = default_op;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }

    ui.add_space(6.0);
    match cc.operation {
        CurveCalculusOp::SmoothSavitzkyGolay { window, order } => {
            let w_step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.curve_calc.sg_window",
                    label: tr(locale, "curve_calc.sg_window"),
                    value_text: &window.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if w_step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::CurveCalculus(c) = &mut next.kind {
                    let next_w = if w_step > 0 { window.saturating_add(2).min(51) } else { window.saturating_sub(2).max(3) };
                    let clamped_order = order.min(next_w - 1);
                    c.operation = CurveCalculusOp::SmoothSavitzkyGolay { window: next_w, order: clamped_order };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            let o_step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.curve_calc.sg_order",
                    label: tr(locale, "curve_calc.sg_order"),
                    value_text: &order.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if o_step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::CurveCalculus(c) = &mut next.kind {
                    let max_ord = window.saturating_sub(1).min(6);
                    let next_o = if o_step > 0 { order.saturating_add(1).min(max_ord) } else { order.saturating_sub(1) };
                    c.operation = CurveCalculusOp::SmoothSavitzkyGolay { window, order: next_o };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
        CurveCalculusOp::SmoothMovingAverage { window } => {
            let w_step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.curve_calc.ma_window",
                    label: tr(locale, "curve_calc.ma_window"),
                    value_text: &window.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if w_step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::CurveCalculus(c) = &mut next.kind {
                    let next_w = if w_step > 0 { window.saturating_add(1).min(51) } else { window.saturating_sub(1).max(2) };
                    c.operation = CurveCalculusOp::SmoothMovingAverage { window: next_w };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
        CurveCalculusOp::SmoothLowess { span, robustness_iters } => {
            if let Some(new_span) = render_linear_numeric_input(
                ui,
                state,
                &app.locale_preset,
                meta,
                NumericInputSpec {
                    salt: ("edit_cc_lowess_span", sid),
                    widget_id: "edit.curve_calc.lowess_span",
                    label: tr(locale, "curve_calc.lowess_span"),
                    current: span,
                    bounds: NumericBounds::ClosedRange(0.05, 1.0),
                },
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::CurveCalculus(c) = &mut next.kind {
                    c.operation = CurveCalculusOp::SmoothLowess { span: new_span, robustness_iters };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            let r_step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.curve_calc.lowess_iters",
                    label: tr(locale, "curve_calc.lowess_iters"),
                    value_text: &robustness_iters.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if r_step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::CurveCalculus(c) = &mut next.kind {
                    let next_r = if r_step > 0 { robustness_iters.saturating_add(1).min(10) } else { robustness_iters.saturating_sub(1) };
                    c.operation = CurveCalculusOp::SmoothLowess { span, robustness_iters: next_r };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
        CurveCalculusOp::CumulativeIntegral { baseline } => {
            if let Some(new_base) = render_linear_numeric_input(
                ui,
                state,
                &app.locale_preset,
                meta,
                NumericInputSpec {
                    salt: ("edit_cc_baseline", sid),
                    widget_id: "edit.curve_calc.baseline",
                    label: tr(locale, "auc.baseline.value"),
                    current: baseline,
                    bounds: NumericBounds::AnyFinite,
                },
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::CurveCalculus(c) = &mut next.kind {
                    c.operation = CurveCalculusOp::CumulativeIntegral { baseline: new_base };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
        CurveCalculusOp::FirstDerivative | CurveCalculusOp::SecondDerivative => {}
    }

    ui.add_space(4.0);
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.curve_calc.create_graph",
        tr(locale, "curve_calc.create_graph"),
        app.canvas_dialogs.edit_params.curve_calculus_create_graph,
        &ControlEnablement::enabled(),
    ) {
        intents.push(UiIntent::StatParams(crate::egui_shell::StatParamsIntent::ToggleCurveCalculusGraph));
    }

    ui.add_space(4.0);
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

fn render_pvalue_stack_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    ps: &curvebook_doc::PValueStackSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Multiplicity / FDR Correction Procedure");
    let method_lbl = ps.method.label();
    let method_resp = crate::egui_shell::widgets::dropdown("edit.pvalue_stack.method", method_lbl)
        .show_ui(ui, |ui| {
            for m in PValueStackMethod::ALL {
                if ui.selectable_label(ps.method == m, m.label()).clicked() {
                    let mut next = spec.clone();
                    if let AnalysisKind::PValueStack(p) = &mut next.kind {
                        p.method = m;
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
            }
        })
        .response;
    record_widget(
        ui,
        meta,
        &method_resp,
        "edit.pvalue_stack.method",
        "combobox",
        method_lbl,
        None,
        Some(serde_json::json!(method_lbl)),
        true,
        None,
        crate::inspect::ZLayer::ModalDialog,
        false,
        None,
    );
    let step = render_stepper_row(
        ui,
        tokens,
        meta,
        StepperSpec {
            id_prefix: "edit.pvalue_stack.threshold",
            label: if ps.method.is_fdr() { "Desired FDR Q" } else { "FWER Alpha" },
            value_text: &format!("{:.3}", ps.threshold),
            enablement: &ControlEnablement::enabled(),
        },
    );
    if step != 0 {
        let mut next = spec.clone();
        if let AnalysisKind::PValueStack(p) = &mut next.kind {
            p.threshold = (p.threshold + if step > 0 { 0.01 } else { -0.01 }).clamp(0.001, 0.50);
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    let titles = column_titles_for_draft(doc, spec.source_sheet_id.as_ref());
    let col_lbl = ps
        .column
        .and_then(|c| titles.get(c))
        .map_or_else(|| ps.column.map_or_else(|| String::from("All"), |c| format!("Col {}", c + 1)), Clone::clone);
    if render_modal_seg_btn(ui, tokens, meta, "edit.pvalue_stack.column", &format!("Source Column: {col_lbl}"), false, &ControlEnablement::enabled())
    {
        let mut next = spec.clone();
        if let AnalysisKind::PValueStack(p) = &mut next.kind {
            p.column = if titles.is_empty() {
                None
            } else {
                match p.column {
                    None => Some(0),
                    Some(c) if c + 1 < titles.len() => Some(c + 1),
                    Some(_) => None,
                }
            };
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
}

fn render_outlier_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    out: &OutlierSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Outlier Algorithm");
    let is_rout = matches!(out, OutlierSpec::Rout { .. });
    if render_modal_radio(ui, tokens, meta, "edit.outlier.rout", "ROUT (Robust regression + FDR Q)", is_rout, &ControlEnablement::enabled())
        && !is_rout
    {
        let mut next = spec.clone();
        next.kind = AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 });
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_radio(ui, tokens, meta, "edit.outlier.grubbs", "Grubbs' ESD test (Alpha)", !is_rout, &ControlEnablement::enabled()) && is_rout {
        let mut next = spec.clone();
        next.kind = AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: false });
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    match out {
        OutlierSpec::Rout { fdr_q_percent } => {
            let step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.outlier.q",
                    label: "Max Desired FDR Q (%)",
                    value_text: &format!("{fdr_q_percent:.2}%"),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: q }) = &mut next.kind {
                    *q = (*q + if step > 0 { 0.5 } else { -0.5 }).clamp(0.1, 10.0);
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
        OutlierSpec::Grubbs { alpha, iterative } => {
            let step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.outlier.alpha",
                    label: "Significance Alpha",
                    value_text: &format!("{alpha:.3}"),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: a, .. }) = &mut next.kind {
                    *a = (*a + if step > 0 { 0.01 } else { -0.01 }).clamp(0.001, 0.25);
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.outlier.iterative",
                "Iterative outlier removal",
                *iterative,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { iterative: it, .. }) = &mut next.kind {
                    *it = !*it;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    }
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

#[derive(Clone, Copy)]
enum NumericBounds {
    AnyFinite,
    NonNegative,
    Positive,
    OpenRange(f64, f64),
    ClosedRange(f64, f64),
}

struct NumericInputSpec<'a> {
    salt: (&'a str, &'a str),
    widget_id: &'a str,
    label: &'a str,
    current: f64,
    bounds: NumericBounds,
}

fn format_locale_f64(value: f64, locale_preset: &LocalePreset) -> String {
    let norm = if value == 0.0 { 0.0 } else { value };
    let text = format!("{norm}");
    if locale_preset.decimal_separator != '.' { text.replace('.', &locale_preset.decimal_separator.to_string()) } else { text }
}

fn validate_numeric_draft(raw: &str, locale_preset: &LocalePreset, bounds: NumericBounds, label: &str) -> Result<f64, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{label}: enter a finite number."));
    }
    let Some(parsed) = parse_localized_number_input(trimmed, locale_preset).filter(|v| v.is_finite()) else {
        return Err(format!("{label}: invalid number `{trimmed}`."));
    };
    match bounds {
        NumericBounds::AnyFinite => Ok(parsed),
        NumericBounds::NonNegative => {
            if parsed >= 0.0 {
                Ok(parsed)
            } else {
                Err(format!("{label}: value must be >= 0."))
            }
        }
        NumericBounds::Positive => {
            if parsed > 0.0 {
                Ok(parsed)
            } else {
                Err(format!("{label}: value must be > 0."))
            }
        }
        NumericBounds::OpenRange(lo, hi) => {
            if parsed > lo && parsed < hi {
                Ok(parsed)
            } else {
                Err(format!("{label}: value must be strictly between {lo} and {hi}."))
            }
        }
        NumericBounds::ClosedRange(lo, hi) => {
            if parsed >= lo && parsed <= hi {
                Ok(parsed)
            } else {
                Err(format!("{label}: value must be between {lo} and {hi}."))
            }
        }
    }
}

fn render_linear_numeric_input(
    ui: &mut Ui,
    state: &mut StatsModalsEguiState,
    locale_preset: &LocalePreset,
    meta: &mut FrameInspectMeta,
    input: NumericInputSpec<'_>,
) -> Option<f64> {
    let session_tag = state.open_session_sheet_id.as_deref().unwrap_or("");
    let key = Id::unique((input.salt.0, session_tag, state.open_generation, input.salt.1));
    let canonical = format_locale_f64(input.current, locale_preset);
    let mut draft: String = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_else(|| canonical.clone());
    let mut updated = None;
    let err_label = if input.label.is_empty() { input.widget_id } else { input.label };
    if !input.label.is_empty() {
        ui.label(input.label);
    }
    let resp = ui.add_sized(Vec2::new(72.0, 24.0), TextEdit::singleline(&mut draft).id(key));
    record_widget(
        ui,
        meta,
        &resp,
        input.widget_id,
        "textbox",
        input.label,
        None,
        Some(serde_json::json!(draft)),
        true,
        None,
        ZLayer::ModalDialog,
        false,
        None,
    );
    if resp.changed() {
        ui.ctx().data_mut(|d| d.insert_temp(key, draft.clone()));
        match validate_numeric_draft(&draft, locale_preset, input.bounds, err_label) {
            Ok(parsed) => {
                state.numeric_field_errors.remove(input.widget_id);
                updated = Some(parsed);
            }
            Err(err) => {
                state.numeric_field_errors.insert(input.widget_id.to_owned(), err);
            }
        }
    } else if resp.has_focus() || resp.lost_focus() {
        match validate_numeric_draft(&draft, locale_preset, input.bounds, err_label) {
            Ok(parsed) => {
                state.numeric_field_errors.remove(input.widget_id);
                if (parsed - input.current).abs() > 1e-12 {
                    updated = Some(parsed);
                }
            }
            Err(err) => {
                state.numeric_field_errors.insert(input.widget_id.to_owned(), err);
            }
        }
    } else if let Err(err) = validate_numeric_draft(&draft, locale_preset, input.bounds, err_label) {
        state.numeric_field_errors.insert(input.widget_id.to_owned(), err);
    } else {
        state.numeric_field_errors.remove(input.widget_id);
        if draft != canonical {
            ui.ctx().data_mut(|d| d.insert_temp(key, canonical));
        }
    }
    updated
}

fn render_numeric_field_errors(ui: &mut Ui, state: &StatsModalsEguiState, tokens: &Tokens, widget_ids: &[&str]) {
    for &wid in widget_ids {
        if let Some(err) = state.numeric_field_errors.get(wid) {
            ui.label(egui::RichText::new(err).small().color(tokens.link_red));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_linear_regression_design(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    spec: &curvebook_doc::AnalysisSpec,
    lin: &curvebook_doc::LinearRegressionSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    let locale = tokens.locale;
    let sid = spec.source_sheet_id.as_ref().map_or("", SheetId::as_str);
    render_section_title(ui, tokens, "Regression Model");
    for (wid, lbl, deming) in
        [("edit.lin.ols", "Ordinary Least Squares (Model I)", false), ("edit.lin.deming", "Deming Model II (Error in both X and Y)", true)]
    {
        if render_modal_radio(ui, tokens, meta, wid, lbl, lin.deming_model_ii == deming, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                l.deming_model_ii = deming;
                if deming {
                    l.force_through_origin = false;
                    l.fixed_point = None;
                    l.standard_addition = false;
                }
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }
    let ols_only_en = if lin.deming_model_ii { ControlEnablement::disabled("Unavailable in Deming Model II.") } else { ControlEnablement::enabled() };
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.lin.origin",
        "Force line through origin (X=0, Y=0)",
        lin.force_through_origin && lin.fixed_point.is_none(),
        &ols_only_en,
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::LinearRegression(l) = &mut next.kind {
            let next_val = !(lin.force_through_origin && lin.fixed_point.is_none());
            l.force_through_origin = next_val;
            if next_val {
                l.fixed_point = None;
            }
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(ui, tokens, meta, "edit.lin.fixed_point", tr(locale, "linear.fixed_point"), lin.fixed_point.is_some(), &ols_only_en) {
        let mut next = spec.clone();
        if let AnalysisKind::LinearRegression(l) = &mut next.kind {
            if lin.fixed_point.is_some() {
                l.fixed_point = None;
            } else {
                l.fixed_point = Some((0.0, 0.0));
                l.force_through_origin = false;
            }
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if !lin.deming_model_ii
        && let Some((x0, y0)) = lin.fixed_point
    {
        ui.horizontal_wrapped(|ui| {
            if let Some(new_x0) = render_linear_numeric_input(
                ui,
                state,
                &app.locale_preset,
                meta,
                NumericInputSpec {
                    salt: ("edit_lin_fp_x0", sid),
                    widget_id: "edit.lin.fixed_point_x0",
                    label: tr(locale, "linear.fixed_point_x0"),
                    current: x0,
                    bounds: NumericBounds::AnyFinite,
                },
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.fixed_point = Some((new_x0, y0));
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if let Some(new_y0) = render_linear_numeric_input(
                ui,
                state,
                &app.locale_preset,
                meta,
                NumericInputSpec {
                    salt: ("edit_lin_fp_y0", sid),
                    widget_id: "edit.lin.fixed_point_y0",
                    label: tr(locale, "linear.fixed_point_y0"),
                    current: y0,
                    bounds: NumericBounds::AnyFinite,
                },
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.fixed_point = Some((x0, new_y0));
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        });
        render_numeric_field_errors(ui, state, tokens, &["edit.lin.fixed_point_x0", "edit.lin.fixed_point_y0"]);
    } else {
        state.numeric_field_errors.remove("edit.lin.fixed_point_x0");
        state.numeric_field_errors.remove("edit.lin.fixed_point_y0");
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.lin.standard_addition",
        tr(locale, "linear.standard_addition"),
        lin.standard_addition,
        &ols_only_en,
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::LinearRegression(l) = &mut next.kind {
            l.standard_addition = !lin.standard_addition;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.lin.extend_x_int",
        tr(locale, "linear.extend_x_int"),
        lin.extend_line_to_x_intercept,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::LinearRegression(l) = &mut next.kind {
            l.extend_line_to_x_intercept = !lin.extend_line_to_x_intercept;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if lin.deming_model_ii {
        render_deming_error_controls(ui, app, state, spec, lin, tokens, meta, intents);
    } else {
        state.numeric_field_errors.remove("edit.lin.deming.sd_x");
        state.numeric_field_errors.remove("edit.lin.deming.sd_y");
        state.numeric_field_errors.remove("edit.lin.deming.lambda");
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.lin.interp",
        "Interpolate unknowns from standard curve",
        lin.interpolate_unknowns,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::LinearRegression(l) = &mut next.kind {
            l.interpolate_unknowns = !lin.interpolate_unknowns;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

#[allow(clippy::too_many_arguments)]
fn render_deming_error_controls(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    spec: &curvebook_doc::AnalysisSpec,
    lin: &curvebook_doc::LinearRegressionSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let locale = tokens.locale;
    let sid = spec.source_sheet_id.as_ref().map_or("", SheetId::as_str);
    render_section_title(ui, tokens, tr(locale, "linear.deming_error_section"));
    let is_equal = matches!(lin.deming_error, DemingErrorSpec::EqualError);
    let is_sds = matches!(lin.deming_error, DemingErrorSpec::StandardDeviations { .. });
    let is_ratio = matches!(lin.deming_error, DemingErrorSpec::VarianceRatio { .. });

    if render_modal_radio(ui, tokens, meta, "edit.lin.deming.equal", tr(locale, "linear.deming_equal_error"), is_equal, &ControlEnablement::enabled())
    {
        let mut next = spec.clone();
        if let AnalysisKind::LinearRegression(l) = &mut next.kind {
            l.deming_error = DemingErrorSpec::EqualError;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_radio(ui, tokens, meta, "edit.lin.deming.sds", tr(locale, "linear.deming_sds"), is_sds, &ControlEnablement::enabled()) {
        let mut next = spec.clone();
        if let AnalysisKind::LinearRegression(l) = &mut next.kind
            && !matches!(l.deming_error, DemingErrorSpec::StandardDeviations { .. })
        {
            l.deming_error = DemingErrorSpec::StandardDeviations { sd_x: 1.0, sd_y: 1.0 };
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if let DemingErrorSpec::StandardDeviations { sd_x, sd_y } = lin.deming_error {
        ui.horizontal_wrapped(|ui| {
            if let Some(new_sd_x) = render_linear_numeric_input(
                ui,
                state,
                &app.locale_preset,
                meta,
                NumericInputSpec {
                    salt: ("edit_lin_dem_sdx", sid),
                    widget_id: "edit.lin.deming.sd_x",
                    label: tr(locale, "linear.deming_sd_x"),
                    current: sd_x,
                    bounds: NumericBounds::Positive,
                },
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.deming_error = DemingErrorSpec::StandardDeviations { sd_x: new_sd_x, sd_y };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if let Some(new_sd_y) = render_linear_numeric_input(
                ui,
                state,
                &app.locale_preset,
                meta,
                NumericInputSpec {
                    salt: ("edit_lin_dem_sdy", sid),
                    widget_id: "edit.lin.deming.sd_y",
                    label: tr(locale, "linear.deming_sd_y"),
                    current: sd_y,
                    bounds: NumericBounds::Positive,
                },
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.deming_error = DemingErrorSpec::StandardDeviations { sd_x, sd_y: new_sd_y };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        });
        render_numeric_field_errors(ui, state, tokens, &["edit.lin.deming.sd_x", "edit.lin.deming.sd_y"]);
    } else {
        state.numeric_field_errors.remove("edit.lin.deming.sd_x");
        state.numeric_field_errors.remove("edit.lin.deming.sd_y");
    }
    if render_modal_radio(
        ui,
        tokens,
        meta,
        "edit.lin.deming.ratio",
        tr(locale, "linear.deming_variance_ratio"),
        is_ratio,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::LinearRegression(l) = &mut next.kind
            && !matches!(l.deming_error, DemingErrorSpec::VarianceRatio { .. })
        {
            l.deming_error = DemingErrorSpec::VarianceRatio { lambda: 1.0 };
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if let DemingErrorSpec::VarianceRatio { lambda } = lin.deming_error {
        ui.horizontal_wrapped(|ui| {
            if let Some(new_lambda) = render_linear_numeric_input(
                ui,
                state,
                &app.locale_preset,
                meta,
                NumericInputSpec {
                    salt: ("edit_lin_dem_lambda", sid),
                    widget_id: "edit.lin.deming.lambda",
                    label: tr(locale, "linear.deming_lambda"),
                    current: lambda,
                    bounds: NumericBounds::Positive,
                },
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.deming_error = DemingErrorSpec::VarianceRatio { lambda: new_lambda };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        });
        render_numeric_field_errors(ui, state, tokens, &["edit.lin.deming.lambda"]);
    } else {
        state.numeric_field_errors.remove("edit.lin.deming.lambda");
    }
}

#[allow(clippy::too_many_arguments)]
fn render_simple_logistic_design(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    spec: &curvebook_doc::AnalysisSpec,
    sl: &curvebook_doc::SimpleLogisticSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    let locale = tokens.locale;
    let sid = spec.source_sheet_id.as_ref().map_or("", SheetId::as_str);
    render_section_title(ui, tokens, "Simple Logistic Regression (Binary Y in {0, 1})");
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.sl.row_class",
        tr(locale, "simple_logistic.row_classification"),
        sl.emit_row_classification,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::SimpleLogisticRegression(s) = &mut next.kind {
            s.emit_row_classification = !sl.emit_row_classification;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.sl.conf_bands",
        tr(locale, "simple_logistic.confidence_bands"),
        sl.confidence_bands,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::SimpleLogisticRegression(s) = &mut next.kind {
            s.confidence_bands = !sl.confidence_bands;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    ui.horizontal_wrapped(|ui| {
        if let Some(new_cut) = render_linear_numeric_input(
            ui,
            state,
            &app.locale_preset,
            meta,
            NumericInputSpec {
                salt: ("edit_sl_cutoff", sid),
                widget_id: "edit.sl.cutoff",
                label: tr(locale, "simple_logistic.cutoff"),
                current: sl.classification_cutoff,
                bounds: NumericBounds::OpenRange(0.0, 1.0),
            },
        ) {
            let mut next = spec.clone();
            if let AnalysisKind::SimpleLogisticRegression(s) = &mut next.kind {
                s.classification_cutoff = new_cut;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    });
    ui.horizontal_wrapped(|ui| {
        let min_enabled = sl.plot_x_min.is_some();
        if render_modal_checkbox(
            ui,
            tokens,
            meta,
            "edit.sl.plot_x_min_toggle",
            tr(locale, "simple_logistic.plot_x_min"),
            min_enabled,
            &ControlEnablement::enabled(),
        ) {
            let mut next = spec.clone();
            if let AnalysisKind::SimpleLogisticRegression(s) = &mut next.kind {
                s.plot_x_min = if min_enabled { None } else { Some(0.0) };
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
        if let Some(x_min) = sl.plot_x_min {
            if let Some(new_min) = render_linear_numeric_input(
                ui,
                state,
                &app.locale_preset,
                meta,
                NumericInputSpec {
                    salt: ("edit_sl_plot_xmin", sid),
                    widget_id: "edit.sl.plot_x_min",
                    label: "",
                    current: x_min,
                    bounds: NumericBounds::AnyFinite,
                },
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::SimpleLogisticRegression(s) = &mut next.kind {
                    s.plot_x_min = Some(new_min);
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        } else {
            state.numeric_field_errors.remove("edit.sl.plot_x_min");
        }
        let max_enabled = sl.plot_x_max.is_some();
        if render_modal_checkbox(
            ui,
            tokens,
            meta,
            "edit.sl.plot_x_max_toggle",
            tr(locale, "simple_logistic.plot_x_max"),
            max_enabled,
            &ControlEnablement::enabled(),
        ) {
            let mut next = spec.clone();
            if let AnalysisKind::SimpleLogisticRegression(s) = &mut next.kind {
                s.plot_x_max = if max_enabled { None } else { Some(10.0) };
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
        if let Some(x_max) = sl.plot_x_max {
            if let Some(new_max) = render_linear_numeric_input(
                ui,
                state,
                &app.locale_preset,
                meta,
                NumericInputSpec {
                    salt: ("edit_sl_plot_xmax", sid),
                    widget_id: "edit.sl.plot_x_max",
                    label: "",
                    current: x_max,
                    bounds: NumericBounds::AnyFinite,
                },
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::SimpleLogisticRegression(s) = &mut next.kind {
                    s.plot_x_max = Some(new_max);
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        } else {
            state.numeric_field_errors.remove("edit.sl.plot_x_max");
        }
    });
    render_numeric_field_errors(ui, state, tokens, &["edit.sl.cutoff", "edit.sl.plot_x_min", "edit.sl.plot_x_max"]);
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

fn render_spline_lowess_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    sp: &curvebook_doc::SplineLowessSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    use curvebook_doc::{LowessPreset, SplineLowessMethod, SplineLowessSpec};

    render_section_title(ui, tokens, "Model-Free Curve Method");
    for m in SplineLowessMethod::ALL {
        if render_modal_radio(ui, tokens, meta, &format!("edit.spline.method.{m:?}"), m.label(), sp.method == m, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::FitSplineLowess(s) = &mut next.kind {
                s.method = m;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }

    ui.add_space(6.0);
    let seg_step = render_stepper_row(
        ui,
        tokens,
        meta,
        StepperSpec {
            id_prefix: "edit.spline.segments",
            label: "Polyline vertices",
            value_text: &format!("{}", sp.segment_count),
            enablement: &ControlEnablement::enabled(),
        },
    );
    if seg_step != 0 {
        let mut next = spec.clone();
        if let AnalysisKind::FitSplineLowess(s) = &mut next.kind {
            let delta = if seg_step > 0 { 50_isize } else { -50_isize };
            let updated = (s.segment_count as isize + delta).clamp(SplineLowessSpec::MIN_SEGMENTS as isize, SplineLowessSpec::MAX_SEGMENTS as isize);
            s.segment_count = updated as usize;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }

    if sp.method == SplineLowessMethod::SmoothingSpline {
        ui.add_space(4.0);
        let using_df = sp.smoothing_df.is_some();
        let knots_val = sp.smoothing_df.map(|df| df.round().clamp(2.0, 50.0) as usize).or(sp.smoothing_knots).unwrap_or(5);
        let value_text = if let Some(df) = sp.smoothing_df { format!("{df:.1}") } else { format!("{knots_val}") };
        let knot_step = render_stepper_row(
            ui,
            tokens,
            meta,
            StepperSpec {
                id_prefix: "edit.spline.knots",
                label: if using_df { "Smoothing df" } else { "Smoothing knots" },
                value_text: &value_text,
                enablement: &ControlEnablement::enabled(),
            },
        );
        if knot_step != 0 {
            let mut next = spec.clone();
            if let AnalysisKind::FitSplineLowess(s) = &mut next.kind {
                if using_df {
                    let cur_df = s.smoothing_df.unwrap_or(5.0);
                    s.smoothing_df = Some((cur_df + f64::from(knot_step)).clamp(2.0, 50.0));
                } else {
                    let updated = (knots_val as isize + isize::from(knot_step)).clamp(2, 50) as usize;
                    s.smoothing_knots = Some(updated);
                }
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }

    if sp.method == SplineLowessMethod::Lowess {
        ui.add_space(4.0);
        render_section_title(ui, tokens, "LOWESS Window Preset");
        if let Some(frac) = sp.lowess_fraction {
            ui.label(
                egui::RichText::new(format!("Custom LOWESS window fraction: {frac:.2} (choose a preset below to reset)"))
                    .small()
                    .color(tokens.text_muted),
            );
        }
        for preset in LowessPreset::ALL {
            if render_modal_radio(
                ui,
                tokens,
                meta,
                &format!("edit.spline.lowess_preset.{preset:?}"),
                preset.label(),
                sp.lowess_fraction.is_none() && sp.lowess_preset == preset,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::FitSplineLowess(s) = &mut next.kind {
                    s.lowess_preset = preset;
                    s.lowess_fraction = None;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
        let iter_step = render_stepper_row(
            ui,
            tokens,
            meta,
            StepperSpec {
                id_prefix: "edit.spline.lowess_iters",
                label: "Bisquare robustness passes",
                value_text: &format!("{}", sp.lowess_robustness_iters),
                enablement: &ControlEnablement::enabled(),
            },
        );
        if iter_step != 0 {
            let mut next = spec.clone();
            if let AnalysisKind::FitSplineLowess(s) = &mut next.kind {
                let updated =
                    (s.lowess_robustness_iters as isize + isize::from(iter_step)).clamp(0, SplineLowessSpec::MAX_LOWESS_ROBUSTNESS_ITERS as isize);
                s.lowess_robustness_iters = updated as usize;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }

    ui.add_space(6.0);
    render_section_title(ui, tokens, "Output & Standard-Curve Options");
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.spline.emit_coords",
        "Emit curve coordinates (X, Y) tab",
        sp.emit_curve_coordinates_tab,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::FitSplineLowess(s) = &mut next.kind {
            s.emit_curve_coordinates_tab = !sp.emit_curve_coordinates_tab;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.spline.interpolate_unknowns",
        "Interpolate unknowns (Y from X / X from Y)",
        sp.interpolate_unknowns,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::FitSplineLowess(s) = &mut next.kind {
            s.interpolate_unknowns = !sp.interpolate_unknowns;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    ui.add_space(4.0);
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

fn render_optional_numeric_input(
    ui: &mut Ui,
    meta: &mut FrameInspectMeta,
    salt: (&str, &str),
    widget_id: &str,
    label: &str,
    hint: &str,
    current: Option<f64>,
) -> Option<Option<f64>> {
    let key = Id::unique(salt);
    let mut draft: String = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_else(|| current.map_or_else(String::new, |v| format!("{v}")));
    let mut updated = None;
    if !label.is_empty() {
        ui.label(label);
    }
    let resp = ui.add(TextEdit::singleline(&mut draft).id(key).hint_text(hint).desired_width(82.0));
    record_widget(ui, meta, &resp, widget_id, "textbox", label, None, Some(serde_json::json!(draft)), true, None, ZLayer::ModalDialog, false, None);
    if resp.changed() {
        ui.ctx().data_mut(|d| d.insert_temp(key, draft.clone()));
        let trimmed = draft.trim();
        if trimmed.is_empty() {
            updated = Some(None);
        } else if let Ok(parsed) = trimmed.replace(',', ".").parse::<f64>()
            && parsed.is_finite()
        {
            updated = Some(Some(parsed));
        }
    } else if !resp.has_focus() {
        let canonical = current.map_or_else(String::new, |v| format!("{v}"));
        if draft != canonical {
            ui.ctx().data_mut(|d| d.insert_temp(key, canonical));
        }
    }
    updated
}

#[allow(clippy::too_many_arguments)]
fn render_area_under_curve_design(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    spec: &curvebook_doc::AnalysisSpec,
    auc: &curvebook_doc::AreaUnderCurveSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    use curvebook_doc::AucBaseline;

    let doc = app.document();
    let locale = tokens.locale;
    let sid = spec.source_sheet_id.as_ref().map_or("", SheetId::as_str);

    render_section_title(ui, tokens, tr(locale, "auc.section.baseline"));
    let is_fixed = matches!(auc.baseline, AucBaseline::Fixed { .. });
    if render_modal_radio(ui, tokens, meta, "edit.auc.baseline.fixed", tr(locale, "auc.baseline.fixed"), is_fixed, &ControlEnablement::enabled()) {
        let mut next = spec.clone();
        if let AnalysisKind::AreaUnderCurve(a) = &mut next.kind
            && !matches!(a.baseline, AucBaseline::Fixed { .. })
        {
            a.baseline = AucBaseline::Fixed { value: 0.0 };
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_radio(
        ui,
        tokens,
        meta,
        "edit.auc.baseline.first_point",
        tr(locale, "auc.baseline.first_point"),
        !is_fixed,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::AreaUnderCurve(a) = &mut next.kind {
            a.baseline = AucBaseline::FirstPoint;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if let AucBaseline::Fixed { value } = auc.baseline {
        ui.horizontal_wrapped(|ui| {
            if let Some(new_val) = render_linear_numeric_input(
                ui,
                state,
                &app.locale_preset,
                meta,
                NumericInputSpec {
                    salt: ("edit_auc_baseline_val", sid),
                    widget_id: "edit.auc.baseline.value",
                    label: tr(locale, "auc.baseline.value"),
                    current: value,
                    bounds: NumericBounds::AnyFinite,
                },
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::AreaUnderCurve(a) = &mut next.kind {
                    a.baseline = AucBaseline::Fixed { value: new_val };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        });
    }

    ui.add_space(6.0);
    render_section_title(ui, tokens, tr(locale, "auc.section.bounds"));
    ui.horizontal_wrapped(|ui| {
        if let Some(new_min) =
            render_optional_numeric_input(ui, meta, ("edit_auc_x_min", sid), "edit.auc.x_min", tr(locale, "auc.x_min"), "auto", auc.x_min)
        {
            let mut next = spec.clone();
            if let AnalysisKind::AreaUnderCurve(a) = &mut next.kind {
                a.x_min = new_min;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
        if let Some(new_max) =
            render_optional_numeric_input(ui, meta, ("edit_auc_x_max", sid), "edit.auc.x_max", tr(locale, "auc.x_max"), "auto", auc.x_max)
        {
            let mut next = spec.clone();
            if let AnalysisKind::AreaUnderCurve(a) = &mut next.kind {
                a.x_max = new_max;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    });

    ui.add_space(6.0);
    render_section_title(ui, tokens, tr(locale, "auc.section.se_peaks"));
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.auc.independent_time_points",
        tr(locale, "auc.independent_time_points"),
        auc.independent_time_points,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::AreaUnderCurve(a) = &mut next.kind {
            a.independent_time_points = !auc.independent_time_points;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    ui.label(egui::RichText::new(tr(locale, "auc.independent_time_points.hint")).small().color(tokens.text_muted));

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if let Some(new_h) = render_linear_numeric_input(
            ui,
            state,
            &app.locale_preset,
            meta,
            NumericInputSpec {
                salt: ("edit_auc_min_peak_height", sid),
                widget_id: "edit.auc.min_peak_height",
                label: tr(locale, "auc.min_peak_height"),
                current: auc.min_peak_height,
                bounds: NumericBounds::NonNegative,
            },
        ) {
            let mut next = spec.clone();
            if let AnalysisKind::AreaUnderCurve(a) = &mut next.kind {
                a.min_peak_height = new_h;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
        if let Some(new_f) = render_linear_numeric_input(
            ui,
            state,
            &app.locale_preset,
            meta,
            NumericInputSpec {
                salt: ("edit_auc_min_area_frac", sid),
                widget_id: "edit.auc.min_area_fraction",
                label: tr(locale, "auc.min_area_fraction"),
                current: auc.min_area_fraction,
                bounds: NumericBounds::ClosedRange(0.0, 1.0),
            },
        ) {
            let mut next = spec.clone();
            if let AnalysisKind::AreaUnderCurve(a) = &mut next.kind {
                a.min_area_fraction = new_f;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    });

    ui.add_space(4.0);
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

#[allow(clippy::too_many_arguments)]
fn render_frequency_distribution_design(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    spec: &curvebook_doc::AnalysisSpec,
    fd: &FrequencyDistributionSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    let locale = tokens.locale;
    let sid = spec.source_sheet_id.as_ref().map_or("", SheetId::as_str);

    render_section_title(ui, tokens, tr(locale, "freq.section.binning"));
    let is_fd = matches!(fd.binning_rule, FrequencyBinningRule::AutoFreedmanDiaconis);
    let is_scott = matches!(fd.binning_rule, FrequencyBinningRule::AutoScott);
    let is_sturges = matches!(fd.binning_rule, FrequencyBinningRule::AutoSturges);
    let is_explicit = matches!(fd.binning_rule, FrequencyBinningRule::ExplicitWidth { .. });
    for (wid, key, active, rule) in [
        ("edit.freq.binning.fd", "freq.binning.fd", is_fd, FrequencyBinningRule::AutoFreedmanDiaconis),
        ("edit.freq.binning.scott", "freq.binning.scott", is_scott, FrequencyBinningRule::AutoScott),
        ("edit.freq.binning.sturges", "freq.binning.sturges", is_sturges, FrequencyBinningRule::AutoSturges),
        ("edit.freq.binning.explicit", "freq.binning.explicit", is_explicit, FrequencyBinningRule::ExplicitWidth { width: 1.0 }),
    ] {
        if render_modal_radio(ui, tokens, meta, wid, tr(locale, key), active, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::FrequencyDistribution(f) = &mut next.kind
                && !matches!((f.binning_rule, rule), (FrequencyBinningRule::ExplicitWidth { .. }, FrequencyBinningRule::ExplicitWidth { .. }))
            {
                f.binning_rule = rule;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }
    if let FrequencyBinningRule::ExplicitWidth { width } = fd.binning_rule {
        ui.horizontal_wrapped(|ui| {
            if let Some(new_w) = render_linear_numeric_input(
                ui,
                state,
                &app.locale_preset,
                meta,
                NumericInputSpec {
                    salt: ("edit_freq_bin_width", sid),
                    widget_id: "edit.freq.bin_width",
                    label: tr(locale, "freq.bin_width"),
                    current: width,
                    bounds: NumericBounds::Positive,
                },
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::FrequencyDistribution(f) = &mut next.kind {
                    f.binning_rule = FrequencyBinningRule::ExplicitWidth { width: new_w };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        if let Some(new_first) = render_optional_numeric_input(
            ui,
            meta,
            ("edit_freq_first_center", sid),
            "edit.freq.first_bin_center",
            tr(locale, "freq.first_bin_center"),
            "auto",
            fd.first_bin_center,
        ) {
            let mut next = spec.clone();
            if let AnalysisKind::FrequencyDistribution(f) = &mut next.kind {
                f.first_bin_center = new_first;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
        if let Some(new_last) = render_optional_numeric_input(
            ui,
            meta,
            ("edit_freq_last_center", sid),
            "edit.freq.last_bin_center",
            tr(locale, "freq.last_bin_center"),
            "auto",
            fd.last_bin_center,
        ) {
            let mut next = spec.clone();
            if let AnalysisKind::FrequencyDistribution(f) = &mut next.kind {
                f.last_bin_center = new_last;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    });

    ui.add_space(6.0);
    render_section_title(ui, tokens, tr(locale, "freq.section.output"));
    ui.horizontal_wrapped(|ui| {
        for (wid, key, mode) in [
            ("edit.freq.output.counts", "freq.output.counts", FrequencyOutputMode::Counts),
            ("edit.freq.output.rel_freq", "freq.output.rel_freq", FrequencyOutputMode::RelativeFrequency),
            ("edit.freq.output.pct", "freq.output.pct", FrequencyOutputMode::Percentages),
            ("edit.freq.output.density", "freq.output.density", FrequencyOutputMode::Density),
            ("edit.freq.output.cum_counts", "freq.output.cum_counts", FrequencyOutputMode::CumulativeCounts),
            ("edit.freq.output.cum_freq", "freq.output.cum_freq", FrequencyOutputMode::CumulativeFrequency),
            ("edit.freq.output.cum_pct", "freq.output.cum_pct", FrequencyOutputMode::CumulativePercentages),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, tr(locale, key), fd.output_mode == mode, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::FrequencyDistribution(f) = &mut next.kind {
                    f.output_mode = mode;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    });

    ui.add_space(6.0);
    render_section_title(ui, tokens, tr(locale, "freq.section.options"));
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.freq.overlay_gaussian",
        tr(locale, "freq.overlay_gaussian"),
        fd.overlay_gaussian,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::FrequencyDistribution(f) = &mut next.kind {
            f.overlay_gaussian = !fd.overlay_gaussian;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.freq.create_histogram_graph",
        tr(locale, "freq.create_histogram_graph"),
        fd.create_histogram_graph,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::FrequencyDistribution(f) = &mut next.kind {
            f.create_histogram_graph = !fd.create_histogram_graph;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }

    let (_, _, subcols) = source_table_shape(doc, spec.source_sheet_id.as_ref());
    let agg_en = r8_subcolumn_aggregation(subcols);
    ui.add_space(4.0);
    render_section_title(ui, tokens, tr(locale, "freq.section.aggregation"));
    for (wid, lbl, agg) in [
        ("edit.freq.agg.pooled", "Pool all replicate subcolumns into one sample", SubcolumnAggregation::PooledGrandSample),
        ("edit.freq.agg.row_means", "Average replicate subcolumns per row first", SubcolumnAggregation::RowMeans),
        ("edit.freq.agg.per_sub", "Bin each replicate subcolumn separately", SubcolumnAggregation::PerSubcolumn),
    ] {
        if render_modal_radio(ui, tokens, meta, wid, lbl, fd.aggregation == agg, &agg_en) {
            let mut next = spec.clone();
            if let AnalysisKind::FrequencyDistribution(f) = &mut next.kind {
                f.aggregation = agg;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }

    ui.add_space(4.0);
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}
