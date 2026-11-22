//! Nested, Multiple Row t-Tests, Contingency, Descriptive, Outlier, and Linear Regression design controls.

use curvebook_doc::{AnalysisKind, ContingencyMethod, KappaWeighting, OutlierSpec, PValueStackMethod, ProjectDocument, SubcolumnAggregation};
use egui::Ui;

use crate::dialogs_canvas::EditAnalysisParametersState;
use crate::modal_predicates::{
    ControlEnablement, r5_contingency_2x2_effect_sizes, r5_contingency_cochran_armitage_trend, r6_contingency_ci_selector, r8_subcolumn_aggregation,
    source_table_shape,
};

use crate::egui_shell::stat_anova_modal::{
    StepperSpec, column_titles_for_draft, render_modal_checkbox, render_modal_radio, render_modal_seg_btn, render_section_title, render_stepper_row,
};
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens};

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
            intents.push_set_edit_params_draft(next);
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
        intents.push_set_edit_params_draft(next);
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
        intents.push_set_edit_params_draft(next);
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
        intents.push_dialog_widget_click("edit.contingency.attributable_risk");
    }

    ui.add_space(6.0);
    render_section_title(ui, tokens, "2×2 Effect-Size CI Methods");
    let or_ci_en =
        r6_contingency_ci_selector(rows, cols, ep.contingency_odds_ratio_checked || c.compute_odds_and_risk_ratios, "Baptista-Pike / Woolf");
    let rr_ci_en = r6_contingency_ci_selector(rows, cols, ep.contingency_relative_risk_checked || c.compute_odds_and_risk_ratios, "Koopman");
    let arr_ci_en = r6_contingency_ci_selector(rows, cols, ep.contingency_attributable_risk_checked, "Newcombe-Wilson");
    let diag_ci_en = r6_contingency_ci_selector(rows, cols, c.compute_sensitivity_specificity, "Wilson-Brown");
    ui.horizontal_wrapped(|ui| {
        for (wid, lbl, active, en, payload) in [
            (
                "edit.contingency.ci.baptista_pike",
                format!("OR CI: {}", ep.contingency_or_ci_method),
                ep.contingency_or_ci_method == "Baptista-Pike",
                or_ci_en,
                "cycle_contingency_ci:or",
            ),
            (
                "edit.contingency.ci.koopman",
                format!("RR CI: {}", ep.contingency_rr_ci_method),
                ep.contingency_rr_ci_method == "Koopman",
                rr_ci_en,
                "cycle_contingency_ci:rr",
            ),
            (
                "edit.contingency.ci.newcombe_wilson",
                format!("ARR CI: {}", ep.contingency_arr_ci_method),
                ep.contingency_arr_ci_method == "Newcombe-Wilson",
                arr_ci_en,
                "cycle_contingency_ci:arr",
            ),
            (
                "edit.contingency.ci.wilson_brown",
                format!("Diag CI: {}", ep.contingency_diag_ci_method),
                ep.contingency_diag_ci_method == "Wilson-Brown",
                diag_ci_en,
                "cycle_contingency_ci:diag",
            ),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, &lbl, active, &en) {
                intents.push_custom_view_action("stat_params", payload);
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
                    intents.push_set_edit_params_draft(next);
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
                    intents.push_set_edit_params_draft(next);
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
                intents.push_set_edit_params_draft(next);
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
                    intents.push_set_edit_params_draft(next);
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
                intents.push_set_edit_params_draft(next);
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
                    intents.push_set_edit_params_draft(next);
                }
            }
        }
        AnalysisKind::PValueStack(ps) => {
            render_section_title(ui, tokens, "Multiplicity / FDR Correction Procedure");
            for m in PValueStackMethod::ALL {
                if render_modal_radio(ui, tokens, meta, &format!("edit.pstack.{m:?}"), m.label(), ps.method == m, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::PValueStack(p) = &mut next.kind {
                        p.method = m;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            let step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.pstack.threshold",
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
                intents.push_set_edit_params_draft(next);
            }
        }
        AnalysisKind::LinearRegression(lin) => {
            render_linear_regression_design(ui, doc, spec, lin, tokens, meta, intents);
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
                        intents.push_set_edit_params_draft(next);
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
                        intents.push_set_edit_params_draft(next);
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
                    intents.push_set_edit_params_draft(next);
                }
            }
            render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        }
        _ => render_dataset_checklist(ui, doc, spec, tokens, meta, intents),
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
        intents.push_set_edit_params_draft(next);
    }
    if render_modal_radio(ui, tokens, meta, "edit.outlier.grubbs", "Grubbs' ESD test (Alpha)", !is_rout, &ControlEnablement::enabled()) {
        let mut next = spec.clone();
        next.kind = AnalysisKind::OutlierDetection(OutlierSpec::Grubbs { alpha: 0.05, iterative: false });
        intents.push_set_edit_params_draft(next);
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
                intents.push_set_edit_params_draft(next);
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
                intents.push_set_edit_params_draft(next);
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
                intents.push_set_edit_params_draft(next);
            }
        }
    }
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
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
    render_section_title(ui, tokens, "Regression Model");
    for (wid, lbl, deming) in
        [("edit.lin.ols", "Ordinary Least Squares (Model I)", false), ("edit.lin.deming", "Deming Model II (Error in both X and Y)", true)]
    {
        if render_modal_radio(ui, tokens, meta, wid, lbl, lin.deming_model_ii == deming, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::LinearRegression(l) = &mut next.kind {
                l.deming_model_ii = deming;
            }
            intents.push_set_edit_params_draft(next);
        }
    }
    let origin_en = if lin.deming_model_ii {
        ControlEnablement::disabled("Force through origin is unavailable in Deming Model II.")
    } else {
        ControlEnablement::enabled()
    };
    if render_modal_checkbox(ui, tokens, meta, "edit.lin.origin", "Force line through origin (X=0, Y=0)", lin.force_through_origin, &origin_en) {
        let mut next = spec.clone();
        if let AnalysisKind::LinearRegression(l) = &mut next.kind {
            l.force_through_origin = !lin.force_through_origin;
        }
        intents.push_set_edit_params_draft(next);
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
        intents.push_set_edit_params_draft(next);
    }
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}
