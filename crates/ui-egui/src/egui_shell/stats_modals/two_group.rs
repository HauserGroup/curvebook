//! Two-group comparison design controls (`TwoGroupComparison`).

use curvebook_doc::{AnalysisKind, TableArchetype, TwoGroupMethod};
use egui::{Id, TextEdit, Ui, Vec2};

use crate::UiApp;
use crate::dialogs_canvas::EditAnalysisParametersState;
use crate::grid_view::parse_localized_number_input;
use crate::i18n::{normalize_locale, tr};
use crate::modal_predicates::{
    ControlEnablement, r1_paired_or_matched_allowed, r2_assume_gaussian_allowed, r4_two_group_graph_differences, r4_two_group_nonparametric_options,
    r4_two_group_welch_correction,
};

use crate::egui_shell::stat_anova_modal::{
    StepperSpec, record_widget, render_modal_checkbox, render_modal_radio, render_modal_seg_btn, render_section_title, render_stepper_row,
};
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};
use crate::inspect::ZLayer;

use super::helpers::{render_dataset_checklist, render_mv_role_badge, resolve_source_data_sheet};

pub(super) fn render_two_group_design(
    ui: &mut Ui,
    app: &UiApp,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let AnalysisKind::TwoGroupComparison(t) = &spec.kind else {
        return;
    };
    let doc = app.document();
    let assume_en = r2_assume_gaussian_allowed(doc, spec.source_sheet_id.as_ref());
    let paired_en = r1_paired_or_matched_allowed(doc, spec.source_sheet_id.as_ref());
    ui.horizontal_wrapped(|ui| {
        if render_modal_checkbox(ui, tokens, meta, "edit.assume_gaussian", "Assume Gaussian distribution", ep.assume_gaussian, &assume_en) {
            intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("edit.assume_gaussian") });
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.paired_or_matched", "Paired / Matched design", ep.paired_or_matched, &paired_en) {
            intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("edit.paired_or_matched") });
        }
    });
    ui.add_space(4.0);
    render_section_title(ui, tokens, "Test Method");
    let welch_en = r4_two_group_welch_correction(t.method);
    let unpaired_nonparam_en = assume_en.clone();
    let paired_nonparam_en = if !assume_en.enabled { assume_en.clone() } else { paired_en.clone() };
    let cur_hyp = match t.method {
        TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } => hypothetical,
        _ => 0.0,
    };
    ui.horizontal_wrapped(|ui| {
        for (wid, lbl, m, en) in [
            ("edit.tg.unpaired_t", "Unpaired Student t", TwoGroupMethod::UnpairedT, ControlEnablement::enabled()),
            ("edit.tg.welch_t", "Welch's t", TwoGroupMethod::WelchT, ControlEnablement::enabled()),
            ("edit.tg.paired_t", "Paired Student t", TwoGroupMethod::PairedT, paired_en.clone()),
            ("edit.tg.ratio_paired", "Ratio paired t", TwoGroupMethod::RatioPairedT, paired_en.clone()),
            ("edit.tg.lognormal", "Lognormal t", TwoGroupMethod::LognormalUnpairedT, ControlEnablement::enabled()),
            ("edit.tg.mann_whitney", "Mann-Whitney U", TwoGroupMethod::MannWhitneyU, unpaired_nonparam_en.clone()),
            ("edit.tg.wilcoxon", "Wilcoxon signed-rank", TwoGroupMethod::WilcoxonSignedRank, paired_nonparam_en.clone()),
            ("edit.tg.ks", "Kolmogorov-Smirnov", TwoGroupMethod::KolmogorovSmirnov, unpaired_nonparam_en.clone()),
            ("edit.tg.sign", "Paired binomial sign", TwoGroupMethod::PairedSignTest, paired_nonparam_en),
            ("edit.tg.one_sample_t", "One-sample t", TwoGroupMethod::OneSampleT { hypothetical: cur_hyp }, ControlEnablement::enabled()),
            ("edit.tg.one_sample_wilcoxon", "One-sample Wilcoxon", TwoGroupMethod::OneSampleWilcoxon { hypothetical: cur_hyp }, unpaired_nonparam_en),
        ] {
            let active = std::mem::discriminant(&t.method) == std::mem::discriminant(&m);
            if render_modal_radio(ui, tokens, meta, wid, lbl, active, &en) {
                let mut next = spec.clone();
                if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                    tg.method = m;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    });
    if let TwoGroupMethod::OneSampleT { hypothetical } | TwoGroupMethod::OneSampleWilcoxon { hypothetical } = t.method {
        ui.add_space(4.0);
        let hyp_key = Id::unique(("edit_tg_hyp_draft", spec.source_sheet_id.as_ref().map_or("", curvebook_doc::SheetId::as_str)));
        let mut hyp_draft: String = ui.ctx().data(|d| d.get_temp(hyp_key)).unwrap_or_else(|| format!("{hypothetical}"));
        let locale = normalize_locale(&app.locale);
        ui.horizontal_wrapped(|ui| {
            ui.label(tr(locale, "two_group.hypothetical"));
            let resp = ui.add_sized(Vec2::new(88.0, 22.0), TextEdit::singleline(&mut hyp_draft).id(hyp_key));
            record_widget(
                ui,
                meta,
                &resp,
                "edit.two_group.hypothetical",
                "textbox",
                "Hypothetical Value",
                None,
                Some(serde_json::json!(hyp_draft)),
                true,
                None,
                ZLayer::ModalDialog,
                false,
                None,
            );
            if resp.changed() {
                ui.ctx().data_mut(|d| d.insert_temp(hyp_key, hyp_draft.clone()));
                if let Some(parsed) = parse_localized_number_input(&hyp_draft, &app.locale_preset).filter(|v| v.is_finite()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                        match &mut tg.method {
                            TwoGroupMethod::OneSampleT { hypothetical: h } | TwoGroupMethod::OneSampleWilcoxon { hypothetical: h } => *h = parsed,
                            _ => {}
                        }
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
            } else if !resp.has_focus() {
                let canonical = format!("{hypothetical}");
                if hyp_draft != canonical {
                    ui.ctx().data_mut(|d| d.insert_temp(hyp_key, canonical));
                }
            }
            let step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.two_group.hypothetical.step",
                    label: "Step",
                    value_text: &format!("{hypothetical}"),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step != 0 {
                ui.ctx().data_mut(|d| d.remove::<String>(hyp_key));
                intents.push(UiIntent::StatParams(crate::egui_shell::StatParamsIntent::StepTwoGroupHypothetical { delta: f64::from(step) }));
            }
        });
    }
    ui.add_space(4.0);
    let diff_en = r4_two_group_graph_differences(t.method);
    let np_en = r4_two_group_nonparametric_options(t.method);
    ui.horizontal_wrapped(|ui| {
        if render_modal_checkbox(ui, tokens, meta, "edit.tg.two_tailed", "Two-tailed P value", t.two_tailed, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                tg.two_tailed = !t.two_tailed;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
        if render_modal_checkbox(
            ui,
            tokens,
            meta,
            "edit.compute_effect_sizes",
            "Effect sizes (Cohen's d)",
            t.compute_effect_sizes,
            &ControlEnablement::enabled(),
        ) {
            let mut next = spec.clone();
            if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                tg.compute_effect_sizes = !t.compute_effect_sizes;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
        if render_modal_checkbox(
            ui,
            tokens,
            meta,
            "edit.two_group.welch",
            "Welch's correction",
            ep.welch_correction || t.method == TwoGroupMethod::WelchT,
            &welch_en,
        ) {
            intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("edit.two_group.welch") });
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.two_group.graph_differences", "Graph differences", ep.graph_differences, &diff_en) {
            intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("edit.two_group.graph_differences") });
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.two_group.graph_ranks", "Graph ranks", ep.graph_ranks, &np_en) {
            intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("edit.two_group.graph_ranks") });
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.two_group.hodges_lehmann", "Hodges-Lehmann CI", ep.hodges_lehmann_ci, &np_en) {
            intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("edit.two_group.hodges_lehmann") });
        }
    });
    if let Some(ds) = spec.source_sheet_id.as_ref().and_then(|src| resolve_source_data_sheet(doc, src))
        && let TableArchetype::MultipleVariables(mv) = &ds.archetype
    {
        ui.add_space(4.0);
        render_section_title(ui, tokens, "Multiple Variables Grouping & Outcome Roles");
        egui::Grid::new("edit_tg_mv_grid").num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.two_group.group.{idx}"),
                        "Grp",
                        t.mv_group_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                            let prev_grp = tg.mv_group_column;
                            tg.mv_group_column = Some(idx);
                            if tg.mv_response_column == Some(idx) {
                                tg.mv_response_column = prev_grp.filter(|&c| c != idx);
                            }
                        }
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.two_group.resp.{idx}"),
                        "Y",
                        t.mv_response_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                            let prev_resp = tg.mv_response_column;
                            tg.mv_response_column = Some(idx);
                            if tg.mv_group_column == Some(idx) {
                                tg.mv_group_column = prev_resp.filter(|&c| c != idx);
                            }
                        }
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                    }
                });
                if idx % 3 == 2 {
                    ui.end_row();
                }
            }
        });
    }
    ui.add_space(4.0);
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}
