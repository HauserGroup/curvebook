//! `Analyze Data...`, `DuplicateAnalysisPrompt`, and `Change Data Analyzed...` modals (§8.7).

use curvebook_doc::{PValueStackMethod, SheetId};
use egui::{Context, Id, Key, Modifiers, TextEdit, Ui, Vec2};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::DuplicateChoice;
use crate::dialogs_canvas::{ActiveDialog, AnalysisCategory, AnalyzeDialogState, VariableRoleFilter};
use crate::egui_shell::AnalyzePickerIntent;
use crate::i18n::{normalize_locale, tr};
use crate::inspect::ZLayer;
use crate::modal_predicates::{ControlEnablement, evaluate_dialog_button, r7_correlation_heatmap_toggle, r10_pcr_outcome_column, source_table_shape};

use crate::egui_shell::stat_anova_modal::{
    ClippedTextSpec, StepperSpec, column_titles_for_draft, record_widget, render_clipped_text, render_modal_checkbox, render_modal_radio,
    render_modal_seg_btn, render_section_title, render_stepper_row,
};
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};

use super::helpers::{Wave4ModalShell, render_modal_action_button, render_mv_role_badge, show_wave4_modal};
use super::{StatsModalFrameReport, StatsModalsEguiState};

// ============================================================================
// 1. Analyze... Entry Picker, DuplicatePrompt, ChangeDataAnalyzed (§8.7)
// ============================================================================

/// Render native `egui` `Analyze Data...` modal (`ActiveDialog::AnalyzeSheet`).
pub fn render_analyze_sheet_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::AnalyzeSheet { .. } = &app.canvas_dialogs.active_dialog else {
        ctx.data_mut(|d| d.remove::<bool>(Id::unique("analyze_modal.init_focus")));
        state.analyze_category_filter = None;
        return StatsModalFrameReport::default();
    };
    let az = &app.canvas_dialogs.analyze;
    let doc = app.document();
    let active_cat_filter = az.category_filter.or(state.analyze_category_filter);
    let entries: Vec<_> = az.compatible_entries().into_iter().filter(|item| active_cat_filter.is_none_or(|c| item.category == c)).collect();
    let src_name = az
        .source_sheet_id
        .as_ref()
        .and_then(|id| doc.data_sheet(id).map(|ds| ds.title.as_str()).or_else(|| doc.result_sheet(id).map(|rs| rs.title())))
        .unwrap_or("Active Sheet");
    let subtitle = format!("{src_name}  |  {} compatible analyses", entries.len());

    let focus_key = Id::unique("analyze_modal.init_focus");
    if !ctx.data(|d| d.get_temp::<bool>(focus_key)).unwrap_or(false) {
        ctx.data_mut(|d| d.insert_temp(focus_key, true));
        state.analyze_focus_requested = true;
        state.analyze_query_draft = az.search_query.clone();
    } else if state.analyze_query_draft != az.search_query && !ctx.egui_wants_keyboard_input() {
        state.analyze_query_draft = az.search_query.clone();
    }

    let ok_en = evaluate_dialog_button(&app.canvas_dialogs, doc, "ok").unwrap_or_else(ControlEnablement::enabled);
    let locale = normalize_locale(&app.locale);

    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        state,
        Wave4ModalShell {
            widget_id: "dialog.analyze_sheet",
            title: tr(locale, "w6.view.modal.analyze_data"),
            subtitle_or_banner: Some(&subtitle),
            banner_widget_id: "analyze.status_banner",
            max_width_pt: 780.0,
            max_height_pt: 560.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |ui, state, meta, intents| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let clear_w = 56.0;
                let edit_w = (ui.available_width() - clear_w - 8.0).max(120.0);
                let search_id = Id::unique("analyze.search.input");
                let search_resp = ui.add_sized(
                    Vec2::new(edit_w, 24.0),
                    TextEdit::singleline(&mut state.analyze_query_draft)
                        .id(search_id)
                        .hint_text("Search analyses (e.g., t-test, ANOVA, nonlinear, GLM, PCA)…"),
                );
                if state.analyze_focus_requested {
                    search_resp.request_focus();
                    if !ctx.will_discard() {
                        state.analyze_focus_requested = false;
                    }
                }
                if search_resp.changed() {
                    intents.push(UiIntent::SetAnalyzeQuery { query: state.analyze_query_draft.clone() });
                }
                record_widget(
                    ui,
                    meta,
                    &search_resp,
                    "analyze.search",
                    "textbox",
                    "Search analyses",
                    None,
                    Some(json!(state.analyze_query_draft)),
                    true,
                    None,
                    ZLayer::ModalDialog,
                    false,
                    None,
                );

                if render_modal_seg_btn(ui, tokens, meta, "analyze.search.clear", "Clear", false, &ControlEnablement::enabled()) {
                    state.analyze_query_draft.clear();
                    state.analyze_focus_requested = true;
                    intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::ClearSearch));
                }
            });

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if render_modal_seg_btn(ui, tokens, meta, "analyze.cat.all", "All", active_cat_filter.is_none(), &ControlEnablement::enabled()) {
                    state.analyze_category_filter = None;
                    intents.push(UiIntent::DialogButtonAction { action: String::from("__analyze_category:all") });
                }
                for (slug, cat) in [
                    ("column", AnalysisCategory::Column),
                    ("grouped", AnalysisCategory::Grouped),
                    ("xy", AnalysisCategory::XyAndRegression),
                    ("contingency", AnalysisCategory::Contingency),
                    ("survival", AnalysisCategory::Survival),
                    ("multivariate", AnalysisCategory::Multivariate),
                    ("transform", AnalysisCategory::Transform),
                ] {
                    let active = active_cat_filter == Some(cat);
                    if render_modal_seg_btn(ui, tokens, meta, &format!("analyze.cat.{slug}"), cat.label(), active, &ControlEnablement::enabled()) {
                        let next_cat = if active { None } else { Some(cat) };
                        state.analyze_category_filter = next_cat;
                        let action = if next_cat.is_some() { format!("__analyze_category:{slug}") } else { String::from("__analyze_category:all") };
                        intents.push(UiIntent::DialogButtonAction { action });
                    }
                }
            });
        },
        |ui, _state, avail_w, _avail_h, meta, intents| {
            let narrow = avail_w < 540.0;
            if narrow {
                render_analyze_entries_pane(ui, app, &entries, tokens, meta, intents);
                ui.add_space(8.0);
                render_analyze_datasets_pane(ui, app, az, tokens, meta, intents);
            } else {
                let left_w = (avail_w * 0.54).max(240.0);
                let right_w = (avail_w - left_w - 10.0).max(180.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(Vec2::new(left_w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(left_w);
                        render_analyze_entries_pane(ui, app, &entries, tokens, meta, intents);
                    });
                    ui.add_space(10.0);
                    ui.allocate_ui_with_layout(Vec2::new(right_w, ui.available_height()), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(right_w);
                        render_analyze_datasets_pane(ui, app, az, tokens, meta, intents);
                    });
                });
            }
        },
        |ui, _state, meta, intents| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ok_en) {
                        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
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
                        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
                    }
                });
            });
        },
    );

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) && ok_en.enabled {
        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    }
    StatsModalFrameReport::from(capture)
}

fn render_analyze_entries_pane(
    ui: &mut Ui,
    app: &UiApp,
    entries: &[&'static crate::dialogs_canvas::AnalysisCatalogItem],
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Compatible Analyses");
    if entries.is_empty() {
        let msg = app.canvas_dialogs.analyze.empty_feedback().unwrap_or_else(|| String::from("No analyses match."));
        render_clipped_text(
            ui,
            meta,
            ClippedTextSpec {
                widget_id: "analyze.empty_feedback",
                role: "status",
                text: &msg,
                font_size: 11.5,
                color: tokens.text_muted,
                z_layer: ZLayer::ModalDialog,
                tooltip: Some(&msg),
                record_meta: true,
            },
        );
        return;
    }
    for item in entries {
        let selected = app.canvas_dialogs.analyze.selected_entry_id.as_deref() == Some(item.id);
        let en = crate::modal_predicates::evaluate_dialog_widget(&app.canvas_dialogs, app.document(), &format!("analyze.entry.{}", item.id));
        if render_modal_radio(ui, tokens, meta, &format!("analyze.entry.{}", item.id), item.label, selected, &en) {
            let mut fields = Map::new();
            fields.insert(String::from("entry_id"), Value::String(item.id.to_owned()));
            intents.push(UiIntent::PatchActiveDialog { fields });
        }
    }

    // Matching existing duplicate notice if present
    let matching = app.canvas_dialogs.analyze.matching_existing_analyses(app.document());
    if !matching.is_empty() {
        ui.add_space(6.0);
        let msg = format!("Note: {} matching analysis sheet(s) already exist for this source.", matching.len());
        render_clipped_text(
            ui,
            meta,
            ClippedTextSpec {
                widget_id: "analyze.duplicate_hint",
                role: "status",
                text: &msg,
                font_size: tokens.metrics.font_ui_small_pt,
                color: tokens.accent,
                z_layer: ZLayer::ModalDialog,
                tooltip: Some(&msg),
                record_meta: true,
            },
        );
    }
}

fn render_analyze_datasets_pane(
    ui: &mut Ui,
    app: &UiApp,
    az: &AnalyzeDialogState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let is_pca_or_pcr = matches!(az.selected_entry_id.as_deref(), Some("stats.pca" | "stats.pcr" | "pca" | "pcr"));
    let mut synced_pca_preds = az.pca_predictors.clone();
    for &c in &az.selected_data_sets {
        if !synced_pca_preds.contains(&c) {
            synced_pca_preds.push(c);
        }
    }
    synced_pca_preds.sort_unstable();

    if is_pca_or_pcr && !az.variable_candidates.is_empty() {
        render_section_title(ui, tokens, "PCA Predictors / PCR Outcome");
        let is_pcr = matches!(az.selected_entry_id.as_deref(), Some("stats.pcr" | "pcr"));
        for var in &az.variable_candidates {
            let pred_on = synced_pca_preds.contains(&var.col_index);
            ui.horizontal_wrapped(|ui| {
                let pred_lbl = if is_pcr { format!("Pred: {}", var.title) } else { format!("Predictor: {}", var.title) };
                if render_modal_checkbox(
                    ui,
                    tokens,
                    meta,
                    &format!("analyze.pca.pred.{}", var.col_index),
                    &pred_lbl,
                    pred_on,
                    &ControlEnablement::enabled(),
                ) {
                    let mut next = synced_pca_preds.clone();
                    if let Some(pos) = next.iter().position(|&c| c == var.col_index) {
                        next.remove(pos);
                    } else {
                        next.push(var.col_index);
                        next.sort_unstable();
                    }
                    let mut fields = Map::new();
                    fields.insert(String::from("pca_predictors"), Value::Array(next.into_iter().map(|i| json!(i)).collect()));
                    intents.push(UiIntent::PatchActiveDialog { fields });
                }
                if is_pcr {
                    let pcr_en = r10_pcr_outcome_column(var.col_index, &synced_pca_preds);
                    if render_modal_radio(
                        ui,
                        tokens,
                        meta,
                        &format!("analyze.pcr.outcome.{}", var.col_index),
                        "Outcome (Y)",
                        az.pcr_outcome_column == Some(var.col_index),
                        &pcr_en,
                    ) {
                        intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::SetPcrOutcome { col: var.col_index }));
                    }
                }
            });
        }
        ui.add_space(6.0);
    }

    if !az.variable_candidates.is_empty() {
        render_section_title(ui, tokens, "Variables & Role Filter");
        ui.horizontal_wrapped(|ui| {
            for (slug, lbl, target) in [
                ("all", "All", VariableRoleFilter::All),
                ("continuous", "Continuous", VariableRoleFilter::ContinuousOnly),
                ("categorical", "Categorical", VariableRoleFilter::CategoricalOnly),
                ("label", "Label", VariableRoleFilter::LabelOnly),
                ("calculated", "Calculated", VariableRoleFilter::CalculatedOnly),
            ] {
                if render_modal_seg_btn(
                    ui,
                    tokens,
                    meta,
                    &format!("analyze.mv.filter.{slug}"),
                    lbl,
                    az.role_filter == target,
                    &ControlEnablement::enabled(),
                ) {
                    intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::SetMvRoleFilter { filter: target }));
                }
            }
        });
        ui.add_space(4.0);
        for var in az.filtered_variables() {
            let checked = if is_pca_or_pcr {
                synced_pca_preds.contains(&var.col_index)
            } else {
                az.selected_data_sets.contains(&var.col_index) || (az.selected_data_sets.is_empty() && var.selected)
            };
            ui.horizontal(|ui| {
                render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                if render_modal_checkbox(
                    ui,
                    tokens,
                    meta,
                    &format!("analyze.mv.var.{}", var.col_index),
                    &var.title,
                    checked,
                    &ControlEnablement::enabled(),
                ) {
                    if is_pca_or_pcr {
                        let mut next = synced_pca_preds.clone();
                        if let Some(pos) = next.iter().position(|&c| c == var.col_index) {
                            next.remove(pos);
                        } else {
                            next.push(var.col_index);
                            next.sort_unstable();
                        }
                        let mut fields = Map::new();
                        fields.insert(String::from("pca_predictors"), Value::Array(next.into_iter().map(|i| json!(i)).collect()));
                        intents.push(UiIntent::PatchActiveDialog { fields });
                    } else {
                        intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::ToggleMvVar { col: var.col_index }));
                    }
                }
            });
        }
    } else if let Some(src_id) = az.source_sheet_id.as_ref() {
        render_section_title(ui, tokens, "Data Sets / Columns");
        let titles = column_titles_for_draft(app.document(), Some(src_id));
        let all_selected = az.selected_data_sets.is_empty();
        if render_modal_checkbox(ui, tokens, meta, "analyze.dataset.all", "All Data Sets", all_selected, &ControlEnablement::enabled()) {
            intents.push(UiIntent::SetAnalyzeSelectedDatasets { cols: Vec::new() });
        }
        for (idx, col_title) in titles.iter().enumerate() {
            let checked = az.selected_data_sets.is_empty() || az.selected_data_sets.contains(&idx);
            if render_modal_checkbox(ui, tokens, meta, &format!("analyze.dataset.{idx}"), col_title, checked, &ControlEnablement::enabled()) {
                let mut next = if az.selected_data_sets.is_empty() { (0..titles.len()).collect() } else { az.selected_data_sets.clone() };
                if let Some(pos) = next.iter().position(|&c| c == idx) {
                    if next.len() > 1 {
                        next.remove(pos);
                    }
                } else {
                    next.push(idx);
                    next.sort_unstable();
                    next.dedup();
                }
                if !titles.is_empty() && next.len() == titles.len() {
                    next.clear();
                }
                intents.push(UiIntent::SetAnalyzeSelectedDatasets { cols: next });
            }
        }
    }

    render_analyze_entry_specific_options(ui, app, az, tokens, meta, intents);
}

fn render_analyze_entry_specific_options(
    ui: &mut Ui,
    app: &UiApp,
    az: &AnalyzeDialogState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    if matches!(az.selected_entry_id.as_deref(), Some("stats.outliers" | "outliers")) {
        ui.add_space(6.0);
        render_section_title(ui, tokens, "Outlier Identification Mode");
        let is_grubbs = az.outlier_method == "grubbs";
        if render_modal_radio(
            ui,
            tokens,
            meta,
            "analyze.outlier.rout",
            "ROUT (Robust regression + FDR Q%)",
            !is_grubbs,
            &ControlEnablement::enabled(),
        ) {
            intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::SetOutlierMethod { method: String::from("rout") }));
        }
        if render_modal_radio(ui, tokens, meta, "analyze.outlier.grubbs", "Grubbs' test (Alpha threshold)", is_grubbs, &ControlEnablement::enabled())
        {
            intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::SetOutlierMethod { method: String::from("grubbs") }));
        }
        let step = render_stepper_row(
            ui,
            tokens,
            meta,
            StepperSpec {
                id_prefix: "analyze.outlier.param",
                label: if is_grubbs { "Significance Alpha" } else { "Max Desired FDR Q (%)" },
                value_text: &if is_grubbs { format!("{:.3}", az.outlier_alpha) } else { format!("{:.1}%", az.outlier_fdr_q_percent) },
                enablement: &ControlEnablement::enabled(),
            },
        );
        if step < 0 {
            intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::StepOutlierParam { delta: if is_grubbs { -0.01 } else { -1.0 } }));
        } else if step > 0 {
            intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::StepOutlierParam { delta: if is_grubbs { 0.01 } else { 1.0 } }));
        }
        if is_grubbs
            && render_modal_checkbox(
                ui,
                tokens,
                meta,
                "analyze.outlier.iterative",
                "Iterative Grubbs (until no outlier)",
                az.outlier_iterative,
                &ControlEnablement::enabled(),
            )
        {
            intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::ToggleOutlierIterative));
        }
    }
    if matches!(az.selected_entry_id.as_deref(), Some("stats.corr_matrix" | "correlation" | "correlation_matrix")) {
        ui.add_space(6.0);
        crate::egui_shell::stat_anova_modal::render_section_title(ui, tokens, "Correlation Options");
        if render_modal_checkbox(
            ui,
            tokens,
            meta,
            "analyze.correlation_all_pairs",
            "All pairwise matrix (enables correlation heatmap)",
            az.correlation_all_pairs,
            &ControlEnablement::enabled(),
        ) {
            intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("analyze.correlation_all_pairs") });
        }
        if !az.correlation_all_pairs {
            ui.label(
                egui::RichText::new("Engine computes all selected column pairs; unchecking disables heatmap graph creation.")
                    .small()
                    .color(tokens.text_muted),
            );
        }
        let heat_en = r7_correlation_heatmap_toggle(az.correlation_all_pairs);
        if render_modal_checkbox(
            ui,
            tokens,
            meta,
            "analyze.correlation.heatmap",
            "Generate correlation heatmap graph",
            az.correlation_heatmap,
            &heat_en,
        ) {
            intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("analyze.correlation.heatmap") });
        }
    }
    if az.selected_entry_id.as_deref() == Some("stats.pvalue_stack") {
        ui.add_space(6.0);
        crate::egui_shell::stat_anova_modal::render_section_title(ui, tokens, "P-Value Stack Options");
        let cur_method_lbl = az.pvalue_stack_method.label();
        let method_resp = crate::egui_shell::widgets::dropdown("analyze.pvalue_stack.method", cur_method_lbl)
            .show_ui(ui, |ui| {
                for m in PValueStackMethod::ALL {
                    if ui.selectable_label(m == az.pvalue_stack_method, m.label()).clicked() {
                        let mut patch = Map::new();
                        patch.insert(String::from("pvalue_stack_method"), json!(m.label()));
                        intents.push(UiIntent::PatchActiveDialog { fields: patch });
                    }
                }
            })
            .response;
        record_widget(
            ui,
            meta,
            &method_resp,
            "analyze.pvalue_stack.method",
            "combobox",
            cur_method_lbl,
            None,
            Some(json!(cur_method_lbl)),
            true,
            None,
            ZLayer::ModalDialog,
            false,
            None,
        );

        let thresh_step = render_stepper_row(
            ui,
            tokens,
            meta,
            StepperSpec {
                id_prefix: "analyze.pvalue_stack.threshold",
                label: if az.pvalue_stack_method.is_fdr() { "Desired FDR Q" } else { "FWER Alpha" },
                value_text: &format!("{:.3}", az.pvalue_stack_threshold),
                enablement: &ControlEnablement::enabled(),
            },
        );
        if thresh_step != 0 {
            let next_t = (az.pvalue_stack_threshold + if thresh_step > 0 { 0.01 } else { -0.01 }).clamp(0.001, 0.50);
            let mut patch = Map::new();
            patch.insert(String::from("pvalue_stack_threshold"), json!(next_t));
            intents.push(UiIntent::PatchActiveDialog { fields: patch });
        }

        let titles = column_titles_for_draft(app.document(), az.source_sheet_id.as_ref());
        let col_count = titles.len();
        let col_lbl = az
            .pvalue_stack_column
            .and_then(|c| titles.get(c).cloned().or_else(|| Some(format!("Col {}", c + 1))))
            .unwrap_or_else(|| String::from("All"));
        if render_modal_seg_btn(
            ui,
            tokens,
            meta,
            "analyze.pvalue_stack.column",
            &format!("Source Column: {col_lbl}"),
            false,
            &ControlEnablement::enabled(),
        ) {
            let next_col = if col_count == 0 {
                None
            } else {
                match az.pvalue_stack_column {
                    None => Some(0usize),
                    Some(c) if c + 1 < col_count => Some(c + 1),
                    Some(_) => None,
                }
            };
            let mut patch = Map::new();
            patch.insert(String::from("pvalue_stack_column"), json!(next_col));
            intents.push(UiIntent::PatchActiveDialog { fields: patch });
        }
    }
    if matches!(az.selected_entry_id.as_deref(), Some("stats.factorial_anova" | "two_way_anova")) {
        ui.add_space(6.0);
        crate::egui_shell::stat_anova_modal::render_section_title(ui, tokens, "REML Residual Covariance Structure");
        let cov_en = crate::modal_predicates::evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "analyze.factorial.residual_covariance");
        let cov_lbl = match az.factorial_residual_covariance {
            curvebook_doc::ResidualCovariance::CompoundSymmetry => "Compound Symmetry",
            curvebook_doc::ResidualCovariance::Unstructured => "Unstructured",
        };
        if render_modal_seg_btn(ui, tokens, meta, "analyze.factorial.residual_covariance", cov_lbl, false, &cov_en) {
            intents.push(UiIntent::DialogWidgetClick { widget_id: String::from("analyze.factorial.residual_covariance") });
        }
    }
}

/// Render `DuplicateAnalysisPrompt` disambiguation modal.
pub fn render_duplicate_analysis_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::DuplicateAnalysisPrompt { existing_result_ids, .. } = &app.canvas_dialogs.active_dialog else {
        return StatsModalFrameReport::default();
    };
    let locale = normalize_locale(&app.locale);
    let az = &app.canvas_dialogs.analyze;
    let doc = app.document();

    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        state,
        Wave4ModalShell {
            widget_id: "dialog.duplicate_analysis",
            title: tr(locale, "w6.view.modal.analysis_already_exists"),
            subtitle_or_banner: Some("An analysis of this type already exists on this source sheet."),
            banner_widget_id: "analyze_picker.dup.banner",
            max_width_pt: 520.0,
            max_height_pt: 380.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |_, _, _, _| {},
        |ui, _state, _w, _h, meta, intents| {
            let is_create = az.duplicate_choice == DuplicateChoice::CreateNew;
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "analyze_picker.dup.create_new",
                "Create a new ResultSheet",
                is_create,
                &ControlEnablement::enabled(),
            ) {
                intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::SetDuplicateChoice { choice: DuplicateChoice::CreateNew }));
            }
            if render_modal_radio(
                ui,
                tokens,
                meta,
                "analyze_picker.dup.modify_existing",
                "Modify existing analysis parameters & results",
                !is_create,
                &ControlEnablement::enabled(),
            ) {
                intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::SetDuplicateChoice { choice: DuplicateChoice::ModifyExisting }));
            }

            if !is_create && !existing_result_ids.is_empty() {
                ui.add_space(6.0);
                render_section_title(ui, tokens, "Select Existing ResultSheet");
                let selected_id = az.duplicate_target_id.as_deref().or_else(|| existing_result_ids.first().map(SheetId::as_str)).unwrap_or("");
                for (idx, rid) in existing_result_ids.iter().enumerate() {
                    let lbl = doc.result_sheet(rid).map_or_else(|| rid.as_str().to_owned(), |rs| format!("{} ({})", rs.title(), rid.as_str()));
                    if render_modal_radio(
                        ui,
                        tokens,
                        meta,
                        &format!("analyze_picker.dup.target.{idx}"),
                        &lbl,
                        selected_id == rid.as_str(),
                        &ControlEnablement::enabled(),
                    ) {
                        intents.push(UiIntent::AnalyzePicker(AnalyzePickerIntent::SetDuplicateTarget { target_id: rid.as_str().to_owned() }));
                    }
                }
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ControlEnablement::enabled()) {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
                }
            });
        },
    );

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    } else if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    }
    StatsModalFrameReport::from(capture)
}

/// Render `Change Data Analyzed...` retargeting modal.
pub fn render_change_data_analyzed_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut StatsModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> StatsModalFrameReport {
    let ActiveDialog::ChangeDataAnalyzed { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return StatsModalFrameReport::default();
    };
    let cd = &app.canvas_dialogs.change_data;
    let doc = app.document();
    let rs_title = doc.result_sheet(result_sheet_id).map_or("Result", |rs| rs.title());
    let subtitle = format!("Retargeting `{rs_title}` ({} candidate tables)", cd.candidates.len());
    let ok_en = cd
        .selected_target_id
        .as_ref()
        .map(|_| ControlEnablement::enabled())
        .unwrap_or_else(|| ControlEnablement::disabled("Select a compatible source DataSheet."));
    let locale = normalize_locale(&app.locale);

    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        state,
        Wave4ModalShell {
            widget_id: "dialog.change_data_analyzed",
            title: tr(locale, "btn.change_data_analyzed"),
            subtitle_or_banner: Some(&subtitle),
            banner_widget_id: "change_data.status_banner",
            max_width_pt: 620.0,
            max_height_pt: 480.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |_, _, _, _| {},
        |ui, _state, _w, _h, meta, intents| {
            render_section_title(ui, tokens, "Compatible Source DataSheets");
            if cd.candidates.is_empty() {
                render_clipped_text(
                    ui,
                    meta,
                    ClippedTextSpec {
                        widget_id: "change_data.empty",
                        role: "status",
                        text: "No DataSheets available in document.",
                        font_size: 11.5,
                        color: tokens.text_muted,
                        z_layer: ZLayer::ModalDialog,
                        tooltip: None,
                        record_meta: true,
                    },
                );
            } else {
                for (idx, cand) in cd.candidates.iter().enumerate() {
                    let selected = cd.selected_target_id.as_ref() == Some(&cand.sheet_id);
                    let en = if cand.compatible {
                        ControlEnablement::enabled()
                    } else {
                        ControlEnablement::disabled(cand.incompatible_reason.as_deref().unwrap_or("Incompatible table archetype"))
                    };
                    let (rows, cols, _) = source_table_shape(doc, Some(&cand.sheet_id));
                    let lbl = format!("{} [{} — {}×{}]", cand.title, cand.table_kind.label(), rows, cols);
                    if render_modal_radio(ui, tokens, meta, &format!("change_data.candidate.{idx}"), &lbl, selected, &en) {
                        intents.push(UiIntent::SelectRetargetSource { target_data_sheet_id: cand.sheet_id.clone() });
                    }
                }
            }

            if let Some(target_id) = &cd.selected_target_id {
                let titles = column_titles_for_draft(doc, Some(target_id));
                if !titles.is_empty() {
                    ui.add_space(8.0);
                    render_section_title(ui, tokens, "Columns Included After Retargeting");
                    if render_modal_checkbox(
                        ui,
                        tokens,
                        meta,
                        "change_data.col.all",
                        "All Columns",
                        cd.selected_data_sets.is_empty(),
                        &ControlEnablement::enabled(),
                    ) {
                        let mut fields = Map::new();
                        fields.insert(String::from("selected_data_sets"), Value::Array(Vec::new()));
                        intents.push(UiIntent::PatchActiveDialog { fields });
                    }
                    for (ci, col_name) in titles.iter().enumerate() {
                        let on = cd.selected_data_sets.is_empty() || cd.selected_data_sets.contains(&ci);
                        if render_modal_checkbox(ui, tokens, meta, &format!("change_data.col.{ci}"), col_name, on, &ControlEnablement::enabled()) {
                            let mut next = if cd.selected_data_sets.is_empty() { (0..titles.len()).collect() } else { cd.selected_data_sets.clone() };
                            if let Some(pos) = next.iter().position(|&c| c == ci) {
                                if next.len() > 1 {
                                    next.remove(pos);
                                }
                            } else {
                                next.push(ci);
                                next.sort_unstable();
                                next.dedup();
                            }
                            if !titles.is_empty() && next.len() == titles.len() {
                                next.clear();
                            }
                            let mut fields = Map::new();
                            fields.insert(String::from("selected_data_sets"), Value::Array(next.into_iter().map(|i| json!(i)).collect()));
                            intents.push(UiIntent::PatchActiveDialog { fields });
                        }
                    }
                }
            }
        },
        |ui, _state, meta, intents| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_modal_action_button(ui, tokens, meta, "modal.ok", tr(locale, "btn.ok"), true, false, &ok_en) {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
                }
                if render_modal_action_button(ui, tokens, meta, "modal.cancel", tr(locale, "btn.cancel"), false, false, &ControlEnablement::enabled())
                {
                    intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
                }
            });
        },
    );

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    } else if !ctx.egui_wants_keyboard_input() && ok_en.enabled && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    }
    StatsModalFrameReport::from(capture)
}
