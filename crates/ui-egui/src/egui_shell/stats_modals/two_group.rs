//! Two-group comparison design controls (`TwoGroupComparison`).

use curvebook_doc::{AnalysisKind, ProjectDocument, TableArchetype, TwoGroupMethod};
use egui::Ui;

use crate::dialogs_canvas::EditAnalysisParametersState;
use crate::modal_predicates::{
    ControlEnablement, r1_paired_or_matched_allowed, r2_assume_gaussian_allowed, r4_two_group_graph_differences, r4_two_group_nonparametric_options,
    r4_two_group_welch_correction,
};

use crate::egui_shell::stat_anova_modal::{render_modal_checkbox, render_modal_radio, render_modal_seg_btn, render_section_title};
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens};

use super::helpers::{render_dataset_checklist, render_mv_role_badge, resolve_source_data_sheet};

pub(super) fn render_two_group_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let AnalysisKind::TwoGroupComparison(t) = &spec.kind else {
        return;
    };
    let assume_en = r2_assume_gaussian_allowed(doc, Some(&spec.source_sheet_id));
    let paired_en = r1_paired_or_matched_allowed(doc, Some(&spec.source_sheet_id));
    ui.horizontal_wrapped(|ui| {
        if render_modal_checkbox(ui, tokens, meta, "edit.assume_gaussian", "Assume Gaussian distribution", ep.assume_gaussian, &assume_en) {
            intents.push_dialog_widget_click("edit.assume_gaussian");
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.paired_or_matched", "Paired / Matched design", ep.paired_or_matched, &paired_en) {
            intents.push_dialog_widget_click("edit.paired_or_matched");
        }
    });
    ui.add_space(4.0);
    render_section_title(ui, tokens, "Test Method");
    let welch_en = r4_two_group_welch_correction(t.method);
    let unpaired_nonparam_en = assume_en.clone();
    let paired_nonparam_en = if !assume_en.enabled { assume_en.clone() } else { paired_en.clone() };
    ui.horizontal_wrapped(|ui| {
        for (wid, lbl, m, en) in [
            ("edit.tg.unpaired_t", "Unpaired Student t", TwoGroupMethod::UnpairedT, ControlEnablement::enabled()),
            ("edit.tg.welch_t", "Welch's t", TwoGroupMethod::WelchT, welch_en.clone()),
            ("edit.tg.paired_t", "Paired Student t", TwoGroupMethod::PairedT, paired_en.clone()),
            ("edit.tg.ratio_paired", "Ratio paired t", TwoGroupMethod::RatioPairedT, paired_en.clone()),
            ("edit.tg.lognormal", "Lognormal t", TwoGroupMethod::LognormalUnpairedT, ControlEnablement::enabled()),
            ("edit.tg.mann_whitney", "Mann-Whitney U", TwoGroupMethod::MannWhitneyU, unpaired_nonparam_en.clone()),
            ("edit.tg.wilcoxon", "Wilcoxon signed-rank", TwoGroupMethod::WilcoxonSignedRank, paired_nonparam_en.clone()),
            ("edit.tg.ks", "Kolmogorov-Smirnov", TwoGroupMethod::KolmogorovSmirnov, unpaired_nonparam_en),
            ("edit.tg.sign", "Paired binomial sign", TwoGroupMethod::PairedSignTest, paired_nonparam_en),
        ] {
            let active = std::mem::discriminant(&t.method) == std::mem::discriminant(&m);
            if render_modal_radio(ui, tokens, meta, wid, lbl, active, &en) {
                let mut next = spec.clone();
                if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                    tg.method = m;
                }
                intents.push_set_edit_params_draft(next);
            }
        }
    });
    ui.add_space(4.0);
    let diff_en = r4_two_group_graph_differences(t.method);
    let np_en = r4_two_group_nonparametric_options(t.method);
    ui.horizontal_wrapped(|ui| {
        if render_modal_checkbox(ui, tokens, meta, "edit.tg.two_tailed", "Two-tailed P value", t.two_tailed, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::TwoGroupComparison(tg) = &mut next.kind {
                tg.two_tailed = !t.two_tailed;
            }
            intents.push_set_edit_params_draft(next);
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
            intents.push_set_edit_params_draft(next);
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
            intents.push_dialog_widget_click("edit.two_group.welch");
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.two_group.graph_differences", "Graph differences", ep.graph_differences, &diff_en) {
            intents.push_dialog_widget_click("edit.two_group.graph_differences");
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.two_group.graph_ranks", "Graph ranks", ep.graph_ranks, &np_en) {
            intents.push_dialog_widget_click("edit.two_group.graph_ranks");
        }
        if render_modal_checkbox(ui, tokens, meta, "edit.two_group.hodges_lehmann", "Hodges-Lehmann CI", ep.hodges_lehmann_ci, &np_en) {
            intents.push_dialog_widget_click("edit.two_group.hodges_lehmann");
        }
    });
    if let Some(ds) = resolve_source_data_sheet(doc, &spec.source_sheet_id)
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
                            tg.mv_group_column = Some(idx);
                        }
                        intents.push_set_edit_params_draft(next);
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
                            tg.mv_response_column = Some(idx);
                        }
                        intents.push_set_edit_params_draft(next);
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
