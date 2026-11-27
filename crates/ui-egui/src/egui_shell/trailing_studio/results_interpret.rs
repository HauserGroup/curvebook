//! Results Interpret & Diagnostics studio surface (`Assumptions`, `Guidance`, `Methods`).

use curvebook_doc::{ChecklistStatus, ProjectDocument, ResultSheet, ResultStatus};
use curvebook_engine::methods_report::{ReportFormat, ReportScope, generate_methods_report_with_scope};
use egui::{Color32, Stroke};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_r9::sanitize_diagnostics_only_text_for_shortcut;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, UiIntent, doc_sheet_title, widgets};
use crate::i18n::tr;
use crate::inspect::ZLayer;

use super::helpers::{StudioButtonSpec, StudioChipSpec, studio_chip, studio_full_button_with_shortcut, studio_sized_button};
use super::{ResultsStudioTab, TrailingStudioContext, TrailingStudioEguiState};

fn checklist_badge(status: ChecklistStatus, tokens: &Tokens) -> (&'static str, Color32) {
    match status {
        ChecklistStatus::Pass => ("PASS", tokens.link_green),
        ChecklistStatus::Warn => ("WARN", tokens.warning),
        ChecklistStatus::Violated => ("VIOL", tokens.link_red),
        ChecklistStatus::Info => ("INFO", tokens.text_muted),
    }
}

fn canonical_format_slug(raw: &str) -> &'static str {
    ReportFormat::from_name(raw).unwrap_or(ReportFormat::Markdown).as_str()
}

fn canonical_scope_slug(raw: &str) -> &'static str {
    ReportScope::from_name(raw).unwrap_or(ReportScope::Methods).as_str()
}

fn sync_methods_options_from_app(app: &UiApp, state: &mut TrailingStudioEguiState) {
    let app_fmt = canonical_format_slug(app.canvas_dialogs.result_view.effective_report_format());
    let app_scope = canonical_scope_slug(app.canvas_dialogs.result_view.effective_report_scope());

    match state.methods_synced_app_format.as_deref() {
        None => {
            if canonical_format_slug(&state.methods_format) == "markdown" && app_fmt != "markdown" {
                state.methods_format = String::from(app_fmt);
            }
            state.methods_synced_app_format = Some(String::from(app_fmt));
        }
        Some(prev) if prev != app_fmt => {
            state.methods_format = String::from(app_fmt);
            state.methods_synced_app_format = Some(String::from(app_fmt));
            state.last_copied_methods = false;
        }
        Some(_) => {}
    }

    match state.methods_synced_app_scope.as_deref() {
        None => {
            if canonical_scope_slug(&state.methods_scope) == "methods" && app_scope != "methods" {
                state.methods_scope = String::from(app_scope);
            }
            state.methods_synced_app_scope = Some(String::from(app_scope));
        }
        Some(prev) if prev != app_scope => {
            state.methods_scope = String::from(app_scope);
            state.methods_synced_app_scope = Some(String::from(app_scope));
            state.last_copied_methods = false;
        }
        Some(_) => {}
    }
}

fn resolve_methods_paragraph(doc: &ProjectDocument, app: &UiApp, rs: &ResultSheet, state: &mut TrailingStudioEguiState) -> String {
    let fmt_str = canonical_format_slug(&state.methods_format);
    let scope_str = canonical_scope_slug(&state.methods_scope);
    let cache_hit = state
        .methods_cache_key
        .as_ref()
        .is_some_and(|(sid, seq, fmt, scope)| sid == &rs.id && *seq == app.mutation_seq() && fmt == fmt_str && scope == scope_str);
    if cache_hit && !state.methods_cached_text.is_empty() {
        return state.methods_cached_text.clone();
    }

    let report_fmt = ReportFormat::from_name(fmt_str).unwrap_or(ReportFormat::Markdown);
    let report_scope = ReportScope::from_name(scope_str).unwrap_or(ReportScope::Methods);
    let generated = generate_methods_report_with_scope(doc, &rs.id, report_fmt, report_scope)
        .unwrap_or_else(|_| format!("Unable to generate methods report for `{}`.", rs.title()));
    state.methods_cache_key = Some((rs.id.clone(), app.mutation_seq(), String::from(fmt_str), String::from(scope_str)));
    state.methods_cached_text = generated.clone();
    generated
}

pub(super) fn render_results_interpret_studio(
    ui: &mut egui::Ui,
    app: &UiApp,
    rs: &ResultSheet,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let locale = &tokens.locale;
    let doc = app.document();

    // Clear stale copy confirmation whenever switching to a different ResultSheet (`AUD-09-46`).
    if state.methods_cache_key.as_ref().is_some_and(|(sid, _, _, _)| sid != &rs.id) {
        state.last_copied_methods = false;
    }
    sync_methods_options_from_app(app, state);

    let small = tokens.metrics.font_ui_small_pt;
    let (status_lbl, status_clr) = match &rs.status {
        ResultStatus::UpToDate => ("Up to date", tokens.link_green),
        ResultStatus::Dirty => ("Recalculating", tokens.stale),
        ResultStatus::Imported => ("Imported (not computed by CurveBook)", tokens.link_blue),
        ResultStatus::ImportedStale => ("Imported (stale — upstream data changed)", tokens.stale),
        ResultStatus::Failed { message, .. } => {
            ui.add(egui::Label::new(egui::RichText::new(format!("Failed: {message}")).size(small).color(tokens.link_red)).wrap());
            ("Failed", tokens.link_red)
        }
    };
    let violated_n = rs.checklist.iter().filter(|c| c.status == ChecklistStatus::Violated).count();
    let warn_n = rs.checklist.iter().filter(|c| c.status == ChecklistStatus::Warn).count();
    let pass_n = rs.checklist.iter().filter(|c| c.status == ChecklistStatus::Pass).count();

    ui.horizontal_wrapped(|ui| {
        ui.add(egui::Label::new(egui::RichText::new(status_lbl).size(small).strong().color(status_clr)).wrap());
        ui.add(
            egui::Label::new(
                egui::RichText::new(format!("• {} pass / {} warn / {} viol", pass_n, warn_n, violated_n)).size(small).color(tokens.text_muted),
            )
            .wrap(),
        );
    });

    ui.add_space(4.0);
    let cur_tab_idx = match state.results_tab {
        ResultsStudioTab::Assumptions => 0,
        ResultsStudioTab::Guidance => 1,
        ResultsStudioTab::Methods => 2,
    };
    let tabs = [
        ("Assumptions", "inspector.results.tab.assumptions", tr(locale, "results.assumptions"), ResultsStudioTab::Assumptions),
        ("Guidance", "inspector.results.tab.guidance", tr(locale, "results.narrative"), ResultsStudioTab::Guidance),
        ("Methods", "inspector.results.tab.methods", tr(locale, "results.methods"), ResultsStudioTab::Methods),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let per_w = ((ui.available_width() - 8.0) / 3.0).max(52.0);
        for (idx, (_, wid, lbl, tab_val)) in tabs.into_iter().enumerate() {
            if studio_chip(
                ui,
                tokens,
                meta,
                &StudioChipSpec {
                    label: lbl,
                    tooltip: "",
                    max_w: per_w,
                    selected: cur_tab_idx == idx,
                    enabled: ctx_in.enabled,
                    border_override: None,
                    widget_id: wid,
                    role: "tab",
                    command_id: None,
                    disabled_reason: None,
                },
            ) {
                state.results_tab = tab_val;
            }
        }
    });

    ui.add_space(4.0);
    match state.results_tab {
        ResultsStudioTab::Assumptions => {
            ui.add(egui::Label::new(egui::RichText::new(tr(locale, "results.diagnostics_only")).size(small).color(tokens.text_muted)).wrap());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if studio_chip(
                    ui,
                    tokens,
                    meta,
                    &StudioChipSpec {
                        label: tr(locale, "results.filter_all"),
                        tooltip: "Show all assumption checks",
                        max_w: 90.0,
                        selected: !state.results_severity_flagged_only,
                        enabled: ctx_in.enabled,
                        border_override: None,
                        widget_id: "inspector.results.filter_all",
                        role: "button",
                        command_id: None,
                        disabled_reason: None,
                    },
                ) {
                    state.results_severity_flagged_only = false;
                }
                if studio_chip(
                    ui,
                    tokens,
                    meta,
                    &StudioChipSpec {
                        label: tr(locale, "results.filter_flagged"),
                        tooltip: "Show warnings and violations only",
                        max_w: 90.0,
                        selected: state.results_severity_flagged_only,
                        enabled: ctx_in.enabled,
                        border_override: None,
                        widget_id: "inspector.results.filter_flagged",
                        role: "button",
                        command_id: None,
                        disabled_reason: None,
                    },
                ) {
                    state.results_severity_flagged_only = true;
                }
            });
            ui.add_space(4.0);
            let mut shown = 0usize;
            for item in &rs.checklist {
                if state.results_severity_flagged_only && !matches!(item.status, ChecklistStatus::Warn | ChecklistStatus::Violated) {
                    continue;
                }
                shown += 1;
                let (tag, clr) = checklist_badge(item.status, tokens);
                let safe_text =
                    sanitize_diagnostics_only_text_for_shortcut(item, app.shortcuts.active_combo("ui.editParams.open").unwrap_or_default());
                egui::Frame::NONE
                    .fill(tokens.panel_bg)
                    .stroke(Stroke::new(1.0, tokens.divider))
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        ui.horizontal_wrapped(|ui| {
                            ui.add(egui::Label::new(egui::RichText::new(tag).size(small).strong().color(clr)));
                            ui.add(egui::Label::new(egui::RichText::new(&item.assumption).size(small).strong().color(tokens.text_strong)).wrap());
                        });
                        if let Some(metric) = &item.diagnostic_metric {
                            ui.add(egui::Label::new(egui::RichText::new(metric).size(small).color(tokens.text_primary)).wrap());
                        }
                        ui.add(egui::Label::new(egui::RichText::new(safe_text).size(small).color(tokens.text_muted)).wrap());
                    });
                ui.add_space(3.0);
            }
            if shown == 0 {
                widgets::dim_label(ui, tr(locale, "results.no_checklist"));
            }
        }
        ResultsStudioTab::Guidance => {
            ui.add(egui::Label::new(egui::RichText::new(tr(locale, "results.diagnostics_only")).size(small).color(tokens.text_muted)).wrap());
            ui.add_space(4.0);
            let src_title = rs.analysis.source_sheet_id.as_ref().map_or("None (unlinked)", |sid| doc_sheet_title(doc, sid).unwrap_or(sid.as_str()));
            ui.add(egui::Label::new(egui::RichText::new(format!("Analysis: {}", rs.title())).size(small).strong().color(tokens.text_strong)).wrap());
            widgets::dim_label(ui, &format!("Source table: {src_title} • Pairwise comparisons: {}", rs.pairwise_comparisons.len()));
            if !rs.scalars.is_empty() {
                ui.add_space(4.0);
                widgets::subheader(ui, "Key Diagnostic Scalars");
                for scalar in rs.scalars.iter().take(6) {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(&scalar.key).size(small).color(tokens.text_muted)).wrap());
                        ui.add(
                            egui::Label::new(egui::RichText::new(format!("{:.4}", scalar.value)).size(small).strong().color(tokens.text_primary))
                                .wrap(),
                        );
                    });
                }
            }
        }
        ResultsStudioTab::Methods => {
            render_methods_tab(ui, app, rs, state, ctx_in, meta, intents);
        }
    }

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);
    let edit_params_sc =
        app.shortcuts.active_combo("ui.editParams.open").filter(|s| !s.trim().is_empty()).map(crate::menus::format_shortcut_for_platform);
    if studio_full_button_with_shortcut(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: tr(locale, "results.parameters"),
            tooltip: "Edit upstream analysis parameters and recalculate",
            selected: false,
            enabled: ctx_in.enabled,
            primary: false,
            widget_id: "inspector.results.edit_params",
            command_id: Some("ui.editParams.open"),
            disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
        },
        edit_params_sc.as_deref(),
    ) {
        intents.push_ui("ui.editParams.open", json!({ "result_sheet_id": rs.id.as_str() }));
    }
}

fn render_methods_tab(
    ui: &mut egui::Ui,
    app: &UiApp,
    rs: &ResultSheet,
    state: &mut TrailingStudioEguiState,
    ctx_in: TrailingStudioContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let locale = &tokens.locale;
    let doc = app.document();
    let small = tokens.metrics.font_ui_small_pt;
    let cur_scope = canonical_scope_slug(&state.methods_scope);
    let cur_fmt = canonical_format_slug(&state.methods_format);

    // Scope toggle (`Methods` manuscript prose vs. `Full Report` ARRIVE/ICMJE audit report).
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let half_w = ((ui.available_width() - 4.0) / 2.0).max(56.0);
        let scopes = [
            (
                "methods",
                "inspector.methods.scope.methods",
                tr(locale, "results.methods_scope_methods"),
                "Manuscript-ready statistical methods paragraph",
            ),
            (
                "full",
                "inspector.methods.scope.full",
                tr(locale, "results.methods_scope_full"),
                "Complete ARRIVE/ICMJE report with provenance, tables, and diagnostics",
            ),
        ];
        for (slug, wid, lbl, tip) in scopes {
            if studio_chip(
                ui,
                tokens,
                meta,
                &StudioChipSpec {
                    label: lbl,
                    tooltip: tip,
                    max_w: half_w,
                    selected: cur_scope == slug,
                    enabled: ctx_in.enabled,
                    border_override: None,
                    widget_id: wid,
                    role: "button",
                    command_id: None,
                    disabled_reason: None,
                },
            ) && cur_scope != slug
            {
                state.methods_scope = String::from(slug);
                state.methods_synced_app_scope = Some(String::from(slug));
                state.last_copied_methods = false;
                intents.push(UiIntent::SetResultReportOptions { format: String::from(cur_fmt), scope: String::from(slug) });
            }
        }
    });

    ui.add_space(4.0);
    // Format selector (`Txt`, `MD`, `TeX`) synchronized with `result_view.report_format`.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (slug, lbl) in [("plain", "Txt"), ("markdown", "MD"), ("latex", "TeX")] {
            let wid = format!("inspector.methods.fmt.{slug}");
            let sel = cur_fmt == slug;
            if studio_chip(
                ui,
                tokens,
                meta,
                &StudioChipSpec {
                    label: lbl,
                    tooltip: "Methods export format",
                    max_w: 48.0,
                    selected: sel,
                    enabled: ctx_in.enabled,
                    border_override: None,
                    widget_id: &wid,
                    role: "button",
                    command_id: None,
                    disabled_reason: None,
                },
            ) && cur_fmt != slug
            {
                state.methods_format = String::from(slug);
                state.methods_synced_app_format = Some(String::from(slug));
                state.last_copied_methods = false;
                intents.push(UiIntent::SetResultReportOptions { format: String::from(slug), scope: state.methods_scope.clone() });
            }
        }
    });

    ui.add_space(4.0);
    let report = resolve_methods_paragraph(doc, app, rs, state);
    if state.last_copied_methods && state.last_copied_methods_text != report {
        state.last_copied_methods = false;
    }

    // Side-by-side dot-free `Copy` (primary) and `Save` (secondary) actions (`R20.7`, `R20.15`).
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let btn_w = ((ui.available_width() - 4.0) / 2.0).max(52.0);
        if studio_sized_button(
            ui,
            tokens,
            meta,
            btn_w,
            &StudioButtonSpec {
                label: tr(locale, "results.methods_copy"),
                tooltip: "Copy report text to clipboard",
                selected: false,
                enabled: ctx_in.enabled,
                primary: true,
                widget_id: "inspector.copy_methods",
                command_id: Some("ui.result.copyMethodsReport"),
                disabled_reason: None,
            },
        ) {
            state.last_copied_methods = true;
            state.last_copied_methods_text = report.clone();
            if !app.is_human_path_only() {
                ui.ctx().copy_text(state.last_copied_methods_text.clone());
            }
            intents.push_ui(
                "ui.result.copyMethodsReport",
                json!({
                    "result_sheet_id": rs.id.as_str(),
                    "format": canonical_format_slug(&state.methods_format),
                    "scope": canonical_scope_slug(&state.methods_scope),
                }),
            );
        }
        if studio_sized_button(
            ui,
            tokens,
            meta,
            btn_w,
            &StudioButtonSpec {
                label: tr(locale, "results.methods_save"),
                tooltip: "Save report to file",
                selected: false,
                enabled: ctx_in.enabled,
                primary: false,
                widget_id: "inspector.save_methods",
                command_id: Some("result.generateReport"),
                disabled_reason: None,
            },
        ) {
            intents.push_engine(
                "result.generateReport",
                json!({
                    "result_sheet_id": rs.id.as_str(),
                    "format": canonical_format_slug(&state.methods_format),
                    "scope": canonical_scope_slug(&state.methods_scope),
                }),
            );
        }
    });

    if state.last_copied_methods {
        ui.label(egui::RichText::new(tr(locale, "results.methods_copied")).size(small).color(tokens.link_green));
    }

    ui.add_space(4.0);
    // Selectable, read-only, full-height preview inside the outer studio ScrollArea (`R20.7`).
    let min_preview_h = (ui.available_height() - 52.0).max(160.0);
    egui::Frame::NONE.fill(tokens.surface_bg).stroke(Stroke::new(1.0, tokens.divider)).corner_radius(4.0).inner_margin(egui::Margin::same(6)).show(
        ui,
        |ui| {
            let preview_w = ui.available_width().max(80.0);
            let mut preview_str = report.as_str();
            let preview_resp = ui.add(
                egui::TextEdit::multiline(&mut preview_str)
                    .id_salt(("inspector.methods.preview", rs.id.as_str(), state.methods_format.as_str(), state.methods_scope.as_str()))
                    .font(egui::FontId::proportional(small))
                    .text_color(tokens.text_primary)
                    .background_color(tokens.surface_bg)
                    .frame(egui::Frame::NONE)
                    .desired_width(preview_w)
                    .min_size(egui::Vec2::new(preview_w, min_preview_h)),
            );
            if preview_resp.rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect).is_positive() {
                meta.record_response(
                    ui,
                    &preview_resp,
                    FrameInspectEntryInput {
                        widget_id: "inspector.methods.preview",
                        role: "textbox",
                        label: tr(locale, "results.methods_preview"),
                        command_id: None,
                        value: Some(json!(report)),
                        enabled: ctx_in.enabled,
                        disabled_reason: (!ctx_in.enabled).then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{AnalysisKind, AnalysisSpec, DescriptiveSpec, ResultSheet};

    use super::*;

    #[test]
    fn resolve_methods_paragraph_error_does_not_leak_last_methods_report() {
        let mut app = UiApp::new();
        app.canvas_dialogs.result_view.last_methods_report = Some(String::from("CROSS_SHEET_LEAK_SENTINEL"));
        let Ok(spec) = AnalysisSpec::new("res-missing", "ds-missing", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())) else {
            return;
        };
        let Ok(detached) = ResultSheet::new(spec) else {
            return;
        };
        let mut state = TrailingStudioEguiState::default();
        let text = resolve_methods_paragraph(app.document(), &app, &detached, &mut state);
        assert!(!text.contains("CROSS_SHEET_LEAK_SENTINEL"), "Error path must never fall back to cross-sheet last_methods_report: {text}");
        assert!(text.contains(detached.title()), "Error fallback should mention the target result sheet title: {text}");
    }
}
