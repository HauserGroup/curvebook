//! Factorial ANOVA and Multivariate (PCA / PCR / Clustering / Multiple Imputation) design controls.

use curvebook_doc::{AnalysisKind, ClusterDistance, ClusterLinkage, MultivariateSpec, ProjectDocument, SumOfSquaresType, TableArchetype};
use egui::Ui;

use crate::dialogs_canvas::EditAnalysisParametersState;
use crate::i18n::tr;
use crate::modal_predicates::{ControlEnablement, r1_paired_or_matched_allowed, r7_correlation_heatmap_toggle};

use crate::egui_shell::stat_anova_modal::{
    StepperSpec, render_modal_checkbox, render_modal_radio, render_modal_seg_btn, render_section_title, render_stepper_row,
};
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};

use super::helpers::{render_dataset_checklist, render_mv_role_badge, resolve_source_data_sheet};

pub(super) fn render_factorial_anova_design(
    ui: &mut Ui,
    doc: &ProjectDocument,
    ep: &EditAnalysisParametersState,
    spec: &curvebook_doc::AnalysisSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let AnalysisKind::FactorialAnova(f) = &spec.kind else {
        return;
    };
    let paired_en = r1_paired_or_matched_allowed(doc, spec.source_sheet_id.as_ref());
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.paired_or_matched",
        tr(tokens.locale, "stat_params.label.paired_matched"),
        ep.paired_or_matched,
        &paired_en,
    ) {
        intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("edit.paired_or_matched") });
    }
    ui.add_space(4.0);
    render_section_title(ui, tokens, "Factorial & Sum of Squares Structure");
    ui.horizontal_wrapped(|ui| {
        for (wid, lbl, target) in [
            ("edit.fac.ss_type_iii", "Type III SS (Marginal)", SumOfSquaresType::TypeIII),
            ("edit.fac.ss_type_i", "Type I SS (Sequential)", SumOfSquaresType::TypeI),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, f.ss_type == target, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                    fa.ss_type = target;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        for (ord, lbl) in [(1usize, "Main effects only"), (2, "Up to 2-way interactions"), (3, "Up to 3-way interactions")] {
            if render_modal_seg_btn(
                ui,
                tokens,
                meta,
                &format!("edit.fac.order.{ord}"),
                lbl,
                f.max_interaction_order == ord,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                    fa.max_interaction_order = ord;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    });
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "edit.compute_effect_sizes",
        "Report partial eta-squared & omega-squared",
        f.compute_effect_sizes,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
            fa.compute_effect_sizes = !f.compute_effect_sizes;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if let Some(ds) = spec.source_sheet_id.as_ref().and_then(|src| resolve_source_data_sheet(doc, src))
        && let TableArchetype::MultipleVariables(mv) = &ds.archetype
    {
        ui.add_space(4.0);
        render_section_title(ui, tokens, "Outcome (Y) & Categorical Factors");
        egui::Grid::new("edit_fac_mv_grid").num_columns(3).spacing([8.0, 4.0]).show(ui, |ui| {
            for (idx, var) in mv.variables.iter().enumerate() {
                ui.horizontal(|ui| {
                    render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                    ui.label(&var.title);
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("edit.fac.resp.{idx}"),
                        "Y",
                        f.response_column == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                            fa.response_column = Some(idx);
                            fa.multifactor_columns.retain(|&c| c != idx);
                        }
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                    }
                    let fac_en = if f.response_column == Some(idx) {
                        ControlEnablement::disabled("Outcome column cannot also be a factor.")
                    } else {
                        ControlEnablement::enabled()
                    };
                    if render_modal_seg_btn(ui, tokens, meta, &format!("edit.fac.col.{idx}"), "Fac", f.multifactor_columns.contains(&idx), &fac_en) {
                        let mut next = spec.clone();
                        if let AnalysisKind::FactorialAnova(fa) = &mut next.kind {
                            if let Some(p) = fa.multifactor_columns.iter().position(|&c| c == idx) {
                                fa.multifactor_columns.remove(p);
                            } else {
                                fa.multifactor_columns.push(idx);
                                fa.multifactor_columns.sort_unstable();
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
    ui.add_space(6.0);
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}

pub(super) struct MultivariateDesignCtx<'a> {
    pub doc: &'a ProjectDocument,
    pub ep: &'a EditAnalysisParametersState,
    pub spec: &'a curvebook_doc::AnalysisSpec,
    pub mv: &'a MultivariateSpec,
}

pub(super) fn render_multivariate_design_controls(
    ui: &mut Ui,
    mvx: MultivariateDesignCtx<'_>,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let MultivariateDesignCtx { doc, ep, spec, mv } = mvx;
    match mv {
        MultivariateSpec::Pca { standardize, parallel_analysis_seed, pcr_outcome_column } => {
            render_section_title(ui, tokens, "PCA Matrix & Component Retention");
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "edit.pca.corr",
                "Standardize variables (Correlation matrix)",
                *standardize,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: s, .. }) = &mut next.kind {
                    *s = true;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if render_modal_radio(ui, tokens, meta, "edit.pca.cov", "Center only (Covariance matrix)", !*standardize, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: s, .. }) = &mut next.kind {
                    *s = false;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.pca.parallel",
                "Horn's Monte Carlo parallel analysis",
                parallel_analysis_seed.is_some(),
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::Pca { parallel_analysis_seed: seed, .. }) = &mut next.kind {
                    *seed = if seed.is_some() { None } else { Some(42) };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if let Some(ds) = spec.source_sheet_id.as_ref().and_then(|src| resolve_source_data_sheet(doc, src))
                && let TableArchetype::MultipleVariables(mv_tbl) = &ds.archetype
            {
                let selected_outcome = (*pcr_outcome_column).or(ep.pcr_outcome_column);
                ui.add_space(4.0);
                render_section_title(ui, tokens, "Optional PCR Outcome Variable (Supervised Mode)");
                ui.horizontal_wrapped(|ui| {
                    for (idx, var) in mv_tbl.variables.iter().enumerate() {
                        if matches!(var.role, curvebook_doc::VariableRole::Label | curvebook_doc::VariableRole::Categorical) {
                            continue;
                        }
                        let en = crate::modal_predicates::r10_pcr_outcome_column(idx, &spec.selected_data_sets);
                        if render_modal_seg_btn(ui, tokens, meta, &format!("edit.pcr.outcome.{idx}"), &var.title, selected_outcome == Some(idx), &en)
                        {
                            let mut next = spec.clone();
                            let next_outcome = if selected_outcome == Some(idx) { None } else { Some(idx) };
                            if let AnalysisKind::Multivariate(MultivariateSpec::Pca { pcr_outcome_column: out_col, .. }) = &mut next.kind {
                                *out_col = next_outcome;
                            }
                            if let Some(y_col) = next_outcome {
                                next.selected_data_sets.retain(|&c| c != y_col);
                            }
                            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                        }
                    }
                });
            }
        }
        MultivariateSpec::CorrelationMatrix { spearman_rank } => {
            render_section_title(ui, tokens, "Correlation Coefficient");
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "edit.corr.pearson",
                "Pearson r (linear correlation)",
                !*spearman_rank,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: sr }) = &mut next.kind {
                    *sr = false;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "edit.corr.spearman",
                "Spearman r_s (monotonic rank)",
                *spearman_rank,
                &ControlEnablement::enabled(),
            ) {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { spearman_rank: sr }) = &mut next.kind {
                    *sr = true;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.corr.all_pairs",
                "All pairwise matrix (enables correlation heatmap)",
                ep.correlation_all_pairs,
                &ControlEnablement::enabled(),
            ) {
                intents.push(UiIntent::StatParams(crate::egui_shell::StatParamsIntent::ToggleCorrAllPairs));
            }
            if !ep.correlation_all_pairs {
                ui.label(
                    egui::RichText::new("Engine computes all selected column pairs; unchecking disables heatmap graph creation.")
                        .small()
                        .color(tokens.text_muted),
                );
            }
            let heat_en = r7_correlation_heatmap_toggle(ep.correlation_all_pairs);
            if render_modal_checkbox(ui, tokens, meta, "edit.corr.heatmap", "Generate correlation heatmap graph", ep.correlation_heatmap, &heat_en) {
                intents.push(UiIntent::StatParams(crate::egui_shell::StatParamsIntent::ToggleCorrHeatmap));
            }
        }
        MultivariateSpec::Clustering { k_means_k, distance, linkage, k_sweep_max } => {
            render_section_title(ui, tokens, "Distance Metric & Linkage");
            ui.horizontal_wrapped(|ui| {
                for (wid, lbl, d) in [
                    ("edit.cluster.dist.euclidean", "Euclidean", ClusterDistance::Euclidean),
                    ("edit.cluster.dist.manhattan", "Manhattan", ClusterDistance::Manhattan),
                    ("edit.cluster.dist.pearson", "Pearson (1-r)", ClusterDistance::Pearson),
                    ("edit.cluster.dist.cosine", "Cosine", ClusterDistance::Cosine),
                ] {
                    if render_modal_seg_btn(ui, tokens, meta, wid, lbl, *distance == d, &ControlEnablement::enabled()) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Multivariate(MultivariateSpec::Clustering { distance: dm, .. }) = &mut next.kind {
                            *dm = d;
                        }
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                for (wid, lbl, l) in [
                    ("edit.cluster.link.ward", "Ward", ClusterLinkage::Ward),
                    ("edit.cluster.link.complete", "Complete", ClusterLinkage::Complete),
                    ("edit.cluster.link.single", "Single", ClusterLinkage::Single),
                    ("edit.cluster.link.upgma", "UPGMA", ClusterLinkage::Upgma),
                    ("edit.cluster.link.wpgma", "WPGMA", ClusterLinkage::Wpgma),
                    ("edit.cluster.link.centroid", "Centroid", ClusterLinkage::Centroid),
                    ("edit.cluster.link.median", "Median", ClusterLinkage::Median),
                ] {
                    if render_modal_seg_btn(ui, tokens, meta, wid, lbl, *linkage == l, &ControlEnablement::enabled()) {
                        let mut next = spec.clone();
                        if let AnalysisKind::Multivariate(MultivariateSpec::Clustering { linkage: lm, .. }) = &mut next.kind {
                            *lm = l;
                        }
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                    }
                }
            });
            if render_modal_checkbox(
                ui,
                tokens,
                meta,
                "edit.cluster.k_means_auto",
                "Hierarchical only (no fixed K-means)",
                k_means_k.is_none(),
                &ControlEnablement::enabled(),
            ) {
                intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("edit.cluster.k_means_auto") });
            }
            let k_val = k_means_k.unwrap_or(0);
            let step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.cluster.k_means_k",
                    label: "K-Means Clusters (0 = hierarchical only)",
                    value_text: &k_val.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_means_k: km, .. }) = &mut next.kind {
                    let next_k = if step > 0 {
                        k_val.saturating_add(1).clamp(2, 12)
                    } else if k_val <= 2 {
                        0
                    } else {
                        k_val - 1
                    };
                    *km = if next_k == 0 { None } else { Some(next_k) };
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            let sweep_val = k_sweep_max.unwrap_or(6);
            let sweep_step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.cluster.k_sweep_max",
                    label: "Silhouette/Elbow Sweep Max K",
                    value_text: &sweep_val.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if sweep_step != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::Clustering { k_sweep_max: ks, .. }) = &mut next.kind {
                    let next_sweep = if sweep_step > 0 { sweep_val.saturating_add(1).clamp(2, 12) } else { sweep_val.saturating_sub(1).max(2) };
                    *ks = Some(next_sweep);
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
        MultivariateSpec::MultipleImputation { imputations_m, max_iters, seed, .. } => {
            render_section_title(ui, tokens, "Chained Equations (MICE) Settings");
            let step_m = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.mi.m",
                    label: "Imputed Datasets (m)",
                    value_text: &imputations_m.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_m != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { imputations_m: m, .. }) = &mut next.kind {
                    *m = (*m as isize + step_m as isize).clamp(2, 50) as usize;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            let step_it = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: "edit.mi.iters",
                    label: "Gibbs Sweep Iterations",
                    value_text: &max_iters.to_string(),
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step_it != 0 {
                let mut next = spec.clone();
                if let AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation { max_iters: it, .. }) = &mut next.kind {
                    *it = (*it as isize + step_it as isize * 5).clamp(5, 100) as usize;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
            ui.label(egui::RichText::new(format!("Deterministic PRNG seed: {seed}")).small().color(tokens.text_muted));
        }
    }
    render_dataset_checklist(ui, doc, spec, tokens, meta, intents);
}
