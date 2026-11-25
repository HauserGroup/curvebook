//! Nested, Multiple Row t-Tests, Contingency, Descriptive, Outlier, and Linear Regression design controls.

use curvebook_doc::{
    AnalysisKind, ContingencyMethod, DemingErrorSpec, KappaWeighting, LeveneCenter, OutlierSpec, PValueStackMethod, ProjectDocument,
    SubcolumnAggregation,
};
use egui::{Id, TextEdit, Ui, Vec2};

use crate::dialogs_canvas::EditAnalysisParametersState;
use crate::i18n::tr;
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
    let (rows, cols, _) = source_table_shape(doc, Some(&spec.source_sheet_id));
    let two_by_two_en = r5_contingency_2x2_effect_sizes(rows, cols);
    let trend_en = r5_contingency_cochran_armitage_trend(rows, cols);
    render_section_title(ui, tokens, "Categorical Test & Effect Sizes");
    for (wid, lbl, m, en) in [
        ("edit.cont.fisher", "Fisher's exact test", ContingencyMethod::FisherExact, ControlEnablement::enabled()),
        ("edit.cont.chisq", "Pearson Chi-square test", ContingencyMethod::PearsonChiSquare { yates_continuity: false }, ControlEnablement::enabled()),
        ("edit.cont.yates", "Chi-square + Yates continuity", ContingencyMethod::PearsonChiSquare { yates_continuity: true }, two_by_two_en.clone()),
        ("edit.cont.trend", "Cochran-Armitage linear trend", ContingencyMethod::CochranArmitageTrend, trend_en),
        ("edit.cont.mcnemar", "McNemar's paired test", ContingencyMethod::McNemarPaired, two_by_two_en.clone()),
        (
            "edit.cont.kappa",
            "Cohen's Kappa inter-rater agreement",
            ContingencyMethod::CohenKappa { weighting: KappaWeighting::Unweighted },
            ControlEnablement::enabled(),
        ),
    ] {
        if render_modal_radio(ui, tokens, meta, wid, lbl, c.method == m, &en) {
            let mut next = spec.clone();
            if let AnalysisKind::Contingency(cg) = &mut next.kind {
                cg.method = m;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
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
}

pub(super) fn render_misc_stats_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
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
            render_df_method_picker(ui, spec, n.df_method, tokens, meta, intents);
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
            let (_, _, max_sub) = source_table_shape(doc, Some(&spec.source_sheet_id));
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
                let sid = spec.source_sheet_id.as_str();
                ui.horizontal_wrapped(|ui| {
                    if let Some(new_p) = render_linear_numeric_input(
                        ui,
                        meta,
                        ("edit_desc_custom_pct", sid),
                        "edit.desc.custom_pct",
                        "Upper percentile P (50 < P < 100)",
                        p_hi,
                        true,
                    ) && new_p > 50.0
                        && new_p < 100.0
                    {
                        let mut next = spec.clone();
                        if let AnalysisKind::DescriptiveStats(ds) = &mut next.kind {
                            ds.custom_upper_percentile = Some(new_p);
                        }
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                    }
                    let p_lo = ((100.0 - p_hi) * 1e6).round() / 1e6;
                    ui.label(egui::RichText::new(format!("({p_lo}% & {p_hi}%, Type 7)")).small().color(tokens.text_muted));
                });
            }
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
        AnalysisKind::LinearRegression(lin) => {
            render_linear_regression_design(ui, doc, spec, lin, tokens, meta, intents);
        }
        AnalysisKind::SimpleLogisticRegression(sl) => {
            render_simple_logistic_design(ui, doc, spec, sl, tokens, meta, intents);
        }
        AnalysisKind::FitSplineLowess(sp) => {
            render_spline_lowess_design(ui, spec, sp, tokens, meta, intents);
        }
        AnalysisKind::PassingBablok(pb) => {
            render_section_title(ui, tokens, "Method Comparison Columns (X Reference vs Y Candidate)");
            let titles = column_titles_for_draft(doc, Some(&spec.source_sheet_id));
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
        _ => render_dataset_checklist(ui, doc, spec, tokens, meta, intents),
    }
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
    for m in PValueStackMethod::ALL {
        if render_modal_radio(ui, tokens, meta, &format!("edit.pstack.{m:?}"), m.label(), ps.method == m, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::PValueStack(p) = &mut next.kind {
                p.method = m;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }
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
    let titles = column_titles_for_draft(doc, Some(&spec.source_sheet_id));
    let col_lbl = ps
        .column
        .and_then(|c| titles.get(c))
        .map_or_else(|| ps.column.map_or_else(|| String::from("All"), |c| format!("Col {}", c + 1)), Clone::clone);
    if render_modal_seg_btn(ui, tokens, meta, "edit.pvalue_stack.column", &format!("Source Column: {col_lbl}"), false, &ControlEnablement::enabled())
    {
        let mut next = spec.clone();
        if let AnalysisKind::PValueStack(p) = &mut next.kind {
            p.column = Some(p.column.map_or(0, |c| (c + 1) % titles.len().max(1)));
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
    if render_modal_radio(ui, tokens, meta, "edit.outlier.rout", "ROUT (Robust regression + FDR Q)", is_rout, &ControlEnablement::enabled()) {
        let mut next = spec.clone();
        next.kind = AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent: 1.0 });
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_radio(ui, tokens, meta, "edit.outlier.grubbs", "Grubbs' ESD test (Alpha)", !is_rout, &ControlEnablement::enabled()) {
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

fn render_linear_numeric_input(
    ui: &mut Ui,
    meta: &mut FrameInspectMeta,
    salt: (&str, &str),
    widget_id: &str,
    label: &str,
    current: f64,
    positive_only: bool,
) -> Option<f64> {
    let key = Id::unique(salt);
    let mut draft: String = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_else(|| format!("{current}"));
    let mut updated = None;
    ui.label(label);
    let resp = ui.add_sized(Vec2::new(72.0, 22.0), TextEdit::singleline(&mut draft).id(key));
    record_widget(ui, meta, &resp, widget_id, "textbox", label, None, Some(serde_json::json!(draft)), true, None, ZLayer::ModalDialog, false, None);
    if resp.changed() {
        ui.ctx().data_mut(|d| d.insert_temp(key, draft.clone()));
        if let Ok(parsed) = draft.trim().replace(',', ".").parse::<f64>()
            && parsed.is_finite()
            && (!positive_only || parsed > 0.0)
        {
            updated = Some(parsed);
        }
    } else if !resp.has_focus() {
        let canonical = format!("{current}");
        if draft != canonical {
            ui.ctx().data_mut(|d| d.insert_temp(key, canonical));
        }
    }
    updated
}

fn render_linear_regression_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    lin: &curvebook_doc::LinearRegressionSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let locale = tokens.locale;
    let sid = spec.source_sheet_id.as_str();
    render_section_title(ui, tokens, "Regression Model");
    for (wid, lbl, deming) in
        [("edit.lin.ols", "Ordinary Least Squares (Model I)", false), ("edit.lin.deming", "Deming Model II (Error in both X and Y)", true)]
    {
        if render_modal_radio(ui, tokens, meta, wid, lbl, lin.deming_model_ii == deming, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                l.deming_model_ii = deming;
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
                meta,
                ("edit_lin_fp_x0", sid),
                "edit.lin.fixed_point_x0",
                tr(locale, "linear.fixed_point_x0"),
                x0,
                false,
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.fixed_point = Some((new_x0, y0));
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if let Some(new_y0) = render_linear_numeric_input(
                ui,
                meta,
                ("edit_lin_fp_y0", sid),
                "edit.lin.fixed_point_y0",
                tr(locale, "linear.fixed_point_y0"),
                y0,
                false,
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.fixed_point = Some((x0, new_y0));
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        });
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
        render_deming_error_controls(ui, spec, lin, tokens, meta, intents);
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

fn render_deming_error_controls(
    ui: &mut Ui,
    spec: &curvebook_doc::AnalysisSpec,
    lin: &curvebook_doc::LinearRegressionSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let locale = tokens.locale;
    let sid = spec.source_sheet_id.as_str();
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
            if let Some(new_sd_x) =
                render_linear_numeric_input(ui, meta, ("edit_lin_dem_sdx", sid), "edit.lin.deming.sd_x", tr(locale, "linear.deming_sd_x"), sd_x, true)
            {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.deming_error = DemingErrorSpec::StandardDeviations { sd_x: new_sd_x, sd_y };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if let Some(new_sd_y) =
                render_linear_numeric_input(ui, meta, ("edit_lin_dem_sdy", sid), "edit.lin.deming.sd_y", tr(locale, "linear.deming_sd_y"), sd_y, true)
            {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.deming_error = DemingErrorSpec::StandardDeviations { sd_x, sd_y: new_sd_y };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        });
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
                meta,
                ("edit_lin_dem_lambda", sid),
                "edit.lin.deming.lambda",
                tr(locale, "linear.deming_lambda"),
                lambda,
                true,
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                    l.deming_error = DemingErrorSpec::VarianceRatio { lambda: new_lambda };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        });
    }
}

fn render_simple_logistic_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    sl: &curvebook_doc::SimpleLogisticSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let locale = tokens.locale;
    let sid = spec.source_sheet_id.as_str();
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
            meta,
            ("edit_sl_cutoff", sid),
            "edit.sl.cutoff",
            tr(locale, "simple_logistic.cutoff"),
            sl.classification_cutoff,
            true,
        ) && new_cut > 0.0
            && new_cut < 1.0
        {
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
        if let Some(x_min) = sl.plot_x_min
            && let Some(new_min) = render_linear_numeric_input(ui, meta, ("edit_sl_plot_xmin", sid), "edit.sl.plot_x_min", "", x_min, false)
        {
            let mut next = spec.clone();
            if let AnalysisKind::SimpleLogisticRegression(s) = &mut next.kind {
                s.plot_x_min = Some(new_min);
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
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
        if let Some(x_max) = sl.plot_x_max
            && let Some(new_max) = render_linear_numeric_input(ui, meta, ("edit_sl_plot_xmax", sid), "edit.sl.plot_x_max", "", x_max, false)
        {
            let mut next = spec.clone();
            if let AnalysisKind::SimpleLogisticRegression(s) = &mut next.kind {
                s.plot_x_max = Some(new_max);
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    });
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

fn render_spline_lowess_design(
    ui: &mut Ui,
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
        let knots_val = sp.smoothing_knots.unwrap_or(5);
        let knot_step = render_stepper_row(
            ui,
            tokens,
            meta,
            StepperSpec {
                id_prefix: "edit.spline.knots",
                label: "Smoothing knots / df",
                value_text: &format!("{knots_val}"),
                enablement: &ControlEnablement::enabled(),
            },
        );
        if knot_step != 0 {
            let mut next = spec.clone();
            if let AnalysisKind::FitSplineLowess(s) = &mut next.kind {
                let updated = (knots_val as isize + isize::from(knot_step)).clamp(2, 50) as usize;
                s.smoothing_knots = Some(updated);
                s.smoothing_df = None;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }

    if sp.method == SplineLowessMethod::Lowess {
        ui.add_space(4.0);
        render_section_title(ui, tokens, "LOWESS Window Preset");
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
}
