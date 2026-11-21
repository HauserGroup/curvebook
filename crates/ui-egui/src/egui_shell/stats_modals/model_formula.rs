//! GLM, Cox, Survival, Multiple-Imputation, and MV model-and-formula controls.

use curvebook_doc::{AnalysisKind, CoxTieMethod, GlmFamily, MultivariateSpec, ProjectDocument, SurvivalAnalysisSpec, TableArchetype};
use egui::Ui;

use crate::modal_predicates::{ControlEnablement, r_km_trend_test, survival_group_count};

use crate::egui_shell::stat_anova_modal::{
    column_titles_for_draft, render_modal_checkbox, render_modal_radio, render_modal_seg_btn, render_section_title,
};
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens};

use super::helpers::{render_dataset_checklist, render_mv_role_badge, resolve_source_data_sheet};

fn collect_term_combinations(n: usize, k: usize, start: usize, cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    if cur.len() == k {
        out.push(cur.clone());
        return;
    }
    for i in start..n {
        cur.push(i);
        collect_term_combinations(n, k, i + 1, cur, out);
        cur.pop();
    }
}

pub(super) fn build_live_symbolic_formula(spec: &curvebook_doc::AnalysisSpec, doc: &ProjectDocument) -> Option<String> {
    let titles = column_titles_for_draft(doc, Some(&spec.source_sheet_id));
    let name = |col: usize| titles.get(col).cloned().unwrap_or_else(|| format!("X{}", col + 1));
    match &spec.kind {
        AnalysisKind::FactorialAnova(f) if !f.multifactor_columns.is_empty() => {
            let y = f.response_column.map_or_else(|| String::from("Y"), name);
            let terms: Vec<String> = f.multifactor_columns.iter().map(|&c| name(c)).collect();
            let order_cap = f.max_interaction_order.clamp(1, terms.len());
            let mut subsets: Vec<Vec<usize>> = Vec::new();
            for ord in 1..=order_cap {
                collect_term_combinations(terms.len(), ord, 0, &mut Vec::new(), &mut subsets);
            }
            let rhs =
                subsets.iter().map(|sub| sub.iter().map(|&idx| terms[idx].as_str()).collect::<Vec<_>>().join(":")).collect::<Vec<_>>().join(" + ");
            Some(format!("{y} ~ {rhs}"))
        }
        AnalysisKind::GeneralizedLinearModel(glm) => {
            let y = name(glm.response_column);
            let lhs = match glm.family {
                GlmFamily::Linear => y,
                GlmFamily::Logistic => format!("logit(P({y}=1))"),
                GlmFamily::Poisson => format!("log(E[{y}])"),
            };
            let preds = if glm.predictor_columns.is_empty() {
                String::from("1")
            } else {
                glm.predictor_columns.iter().enumerate().map(|(i, &c)| format!("β{}·{}", i + 1, name(c))).collect::<Vec<_>>().join(" + ")
            };
            Some(format!("{lhs} ~ β₀ + {preds}"))
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { predictor_columns, .. }) => {
            let preds = if predictor_columns.is_empty() {
                String::from("0")
            } else {
                predictor_columns.iter().enumerate().map(|(i, &c)| format!("β{}·{}", i + 1, name(c))).collect::<Vec<_>>().join(" + ")
            };
            Some(format!("h(t | X) = h₀(t) · exp({preds})"))
        }
        _ => None,
    }
}

pub(super) fn render_stat_mv_data_tab(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let Some(ds) = resolve_source_data_sheet(doc, &spec.source_sheet_id) else {
        return;
    };
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        return;
    };

    render_section_title(ui, tokens, "Multivariate Variable Role Binding");
    match &spec.kind {
        AnalysisKind::GeneralizedLinearModel(glm) => {
            render_section_title(ui, tokens, "Response (Y) Variable");
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    if render_modal_radio(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.glm.resp.{idx}"),
                        &var.title,
                        glm.response_column == idx,
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::GeneralizedLinearModel(g) = &mut next.kind {
                            g.response_column = idx;
                            g.predictor_columns.retain(|&c| c != idx);
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
            }
            ui.add_space(6.0);
            render_section_title(ui, tokens, "Predictor Covariates (X)");
            for (idx, var) in mv.variables.iter().enumerate() {
                let is_resp = glm.response_column == idx;
                let en =
                    if is_resp { ControlEnablement::disabled("Outcome variable cannot also be a predictor.") } else { ControlEnablement::enabled() };
                if render_modal_checkbox(ui, tokens, meta, &format!("edit.glm.pred.{idx}"), &var.title, glm.predictor_columns.contains(&idx), &en) {
                    let mut next = spec.clone();
                    if let AnalysisKind::GeneralizedLinearModel(g) = &mut next.kind {
                        if let Some(p) = g.predictor_columns.iter().position(|&c| c == idx) {
                            g.predictor_columns.remove(p);
                        } else {
                            g.predictor_columns.push(idx);
                            g.predictor_columns.sort_unstable();
                        }
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, .. }) => {
            render_section_title(ui, tokens, "Time-to-Event & Status Indicator");
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.cox.time.{idx}"),
                        "Time",
                        *time_column == idx,
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { time_column: tc, .. }) = &mut next.kind {
                            *tc = idx;
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.cox.status.{idx}"),
                        "Event (0/1)",
                        *status_column == idx,
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { status_column: sc, .. }) = &mut next.kind {
                            *sc = idx;
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    let pred_en = if *time_column == idx || *status_column == idx {
                        ControlEnablement::disabled("Time/status column cannot be a covariate.")
                    } else {
                        ControlEnablement::enabled()
                    };
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.cox.pred.{idx}"),
                        "Covariate",
                        predictor_columns.contains(&idx),
                        &pred_en,
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { predictor_columns: pc, .. }) = &mut next.kind {
                            if let Some(p) = pc.iter().position(|&c| c == idx) {
                                pc.remove(p);
                            } else {
                                pc.push(idx);
                                pc.sort_unstable();
                            }
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
            }
        }
        AnalysisKind::FactorialAnova(f) => {
            render_section_title(ui, tokens, "Outcome (Y) & Categorical Factors");
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.fac.resp.{idx}"),
                        "Response (Y)",
                        f.response_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                            fa.response_column = Some(idx);
                            fa.multifactor_columns.retain(|&c| c != idx);
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    let fac_en = if f.response_column == Some(idx) {
                        ControlEnablement::disabled("Outcome column cannot also be a factor.")
                    } else {
                        ControlEnablement::enabled()
                    };
                    if render_modal_seg_btn(ui, tokens, meta, &format!("edit.fac.col.{idx}"), "Factor", f.multifactor_columns.contains(&idx), &fac_en)
                    {
                        let mut next = spec.clone();
                        if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                            if let Some(p) = fa.multifactor_columns.iter().position(|&c| c == idx) {
                                fa.multifactor_columns.remove(p);
                            } else {
                                fa.multifactor_columns.push(idx);
                                fa.multifactor_columns.sort_unstable();
                            }
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
            }
        }
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column, mv_status_column, mv_group_column, .. }) => {
            render_section_title(ui, tokens, "Kaplan-Meier MV Column Roles");
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.km.time.{idx}"),
                        "Time",
                        *mv_time_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_time_column: c, .. }) = &mut next.kind {
                            *c = Some(idx);
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.km.status.{idx}"),
                        "Status",
                        *mv_status_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_status_column: c, .. }) = &mut next.kind {
                            *c = Some(idx);
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.km.group.{idx}"),
                        "Cohort",
                        *mv_group_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { mv_group_column: c, .. }) = &mut next.kind {
                            *c = if *c == Some(idx) { None } else { Some(idx) };
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
            }
        }
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { response_column, predictor_columns, .. }) => {
            render_section_title(ui, tokens, "Pooled Regression Variables (Optional)");
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.mi.resp.{idx}"),
                        "Outcome",
                        *response_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
                            response_column: rc, predictor_columns: pc, ..
                        }) = &mut next.kind
                        {
                            *rc = if *rc == Some(idx) { None } else { Some(idx) };
                            if *rc == Some(idx) {
                                pc.retain(|&c| c != idx);
                            }
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.mi.pred.{idx}"),
                        "Predictor",
                        predictor_columns.contains(&idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { predictor_columns: pc, .. }) = &mut next.kind {
                            if let Some(p) = pc.iter().position(|&c| c == idx) {
                                pc.remove(p);
                            } else {
                                pc.push(idx);
                                pc.sort_unstable();
                            }
                        }
                        intents.push_set_edit_params_draft(next);
                    }
                });
            }
        }
        _ => render_dataset_checklist(ui, doc, spec, tokens, meta, intents),
    }
}

pub(super) fn render_survival_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    surv: &SurvivalAnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    match surv {
        SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, .. } => {
            render_section_title(ui, tokens, "Tied Failure Likelihood Handling");
            for (wid, lbl, tm) in [
                ("edit.cox.efron", "Efron approximation (recommended default)", CoxTieMethod::Efron),
                ("edit.cox.breslow", "Breslow partial likelihood", CoxTieMethod::Breslow),
                ("edit.cox.exact", "Exact discrete conditional likelihood", CoxTieMethod::Exact),
            ] {
                if render_modal_radio(ui, tokens, meta, wid, lbl, *tie_method == tm, &ControlEnablement::enabled()) {
                    let mut next = spec.clone();
                    if let AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method: tm_mut, .. }) = &mut next.kind {
                        *tm_mut = tm;
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
        }
        SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox,
            gehan_breslow_wilcoxon,
            trend_test,
            asymmetrical_greenwood_ci,
            mv_group_column,
            ..
        } => {
            render_section_title(ui, tokens, "Survival Curve Comparison Tests");
            let grp_count = survival_group_count(doc, Some(&spec.source_sheet_id), *mv_group_column);
            let trend_en = r_km_trend_test(grp_count);
            for (wid, lbl, checked, en, idx) in [
                ("edit.km.mantel_cox", "Log-rank (Mantel-Cox) test", *log_rank_mantel_cox, ControlEnablement::enabled(), 0u8),
                ("edit.km.gehan", "Gehan-Breslow-Wilcoxon test", *gehan_breslow_wilcoxon, ControlEnablement::enabled(), 1),
                ("edit.km.trend", "Log-rank test for trend (>= 3 cohorts)", *trend_test, trend_en, 2),
                ("edit.km.asym_ci", "Asymmetrical ln(-ln S(t)) Greenwood CI bounds", *asymmetrical_greenwood_ci, ControlEnablement::enabled(), 3),
            ] {
                if render_modal_checkbox(ui, tokens, meta, wid, lbl, checked, &en) {
                    let mut next = spec.clone();
                    if let AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
                        log_rank_mantel_cox: mc,
                        gehan_breslow_wilcoxon: gb,
                        trend_test: tt,
                        asymmetrical_greenwood_ci: ac,
                        ..
                    }) = &mut next.kind
                    {
                        match idx {
                            0 => *mc = !*mc,
                            1 => *gb = !*gb,
                            2 => *tt = !*tt,
                            _ => *ac = !*ac,
                        }
                    }
                    intents.push_set_edit_params_draft(next);
                }
            }
            ui.add_space(6.0);
            render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
        }
    }
}
