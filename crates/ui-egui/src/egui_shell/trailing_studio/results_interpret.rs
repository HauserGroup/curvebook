//! Results Interpret & Diagnostics studio surface (`Assumptions`, `Guidance`, `Methods`).

use curvebook_doc::{ChecklistStatus, ProjectDocument, ResultSheet, ResultStatus};
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use egui::{Color32, Stroke};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_r9::sanitize_diagnostics_only_text;
use crate::egui_shell::{FrameInspectMeta, PendingUiIntents, Tokens, doc_sheet_title, widgets};
use crate::i18n::tr;

use super::helpers::{StudioButtonSpec, StudioChipSpec, studio_chip, studio_full_button};
use super::{ResultsStudioTab, TrailingStudioContext, TrailingStudioEguiState};

fn checklist_badge(status: ChecklistStatus, tokens: &Tokens) -> (&'static str, Color32) {
    match status {
        ChecklistStatus::Pass => ("PASS", tokens.link_green),
        ChecklistStatus::Warn => ("WARN", tokens.accent_strong),
        ChecklistStatus::Violated => ("VIOL", tokens.link_red),
        ChecklistStatus::Info => ("INFO", tokens.link_blue),
    }
}

fn resolve_methods_paragraph(doc: &ProjectDocument, app: &UiApp, rs: &ResultSheet, state: &mut TrailingStudioEguiState) -> String {
    let fmt_str = if state.methods_format.trim().is_empty() { "markdown" } else { state.methods_format.trim() };
    let cache_hit = state.methods_cache_key.as_ref().is_some_and(|(sid, seq, fmt)| sid == &rs.id && *seq == app.mutation_seq() && fmt == fmt_str);
    if cache_hit && !state.methods_cached_text.is_empty() {
        return state.methods_cached_text.clone();
    }

    let report_fmt = match fmt_str {
        "plain" => ReportFormat::PlainText,
        "latex" => ReportFormat::Latex,
        _ => ReportFormat::Markdown,
    };
    let generated = generate_methods_report(doc, &rs.id, report_fmt).unwrap_or_else(|_| {
        app.canvas_dialogs
            .result_view
            .last_methods_report
            .clone()
            .unwrap_or_else(|| format!("Analysis `{}` on source `{}`.", rs.title(), rs.analysis.source_sheet_id.as_str()))
    });
    state.methods_cache_key = Some((rs.id.clone(), app.mutation_seq(), String::from(fmt_str)));
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

    let small = tokens.metrics.font_ui_small_pt;
    let (status_lbl, status_clr) = match &rs.status {
        ResultStatus::UpToDate => ("Up to date", tokens.link_green),
        ResultStatus::Dirty => ("Recalculating", tokens.accent_strong),
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
                    tooltip: lbl,
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
                let safe_text = sanitize_diagnostics_only_text(item);
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
            let src_title = doc_sheet_title(doc, &rs.analysis.source_sheet_id).unwrap_or(rs.analysis.source_sheet_id.as_str());
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
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (slug, lbl) in [("plain", "Txt"), ("markdown", "MD"), ("latex", "TeX")] {
                    let wid = format!("inspector.methods.fmt.{slug}");
                    let sel = state.methods_format == slug;
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
                    ) {
                        state.methods_format = String::from(slug);
                    }
                }
            });
            ui.add_space(4.0);
            let report = resolve_methods_paragraph(doc, app, rs, state);
            if studio_full_button(
                ui,
                tokens,
                meta,
                &StudioButtonSpec {
                    label: tr(locale, "results.copy_methods"),
                    tooltip: "Copy publication methods paragraph to clipboard",
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
                ui.ctx().copy_text(state.last_copied_methods_text.clone());
                intents.push_ui("ui.result.copyMethodsReport", json!({ "result_sheet_id": rs.id.as_str(), "format": state.methods_format }));
            }
            if state.last_copied_methods {
                ui.label(egui::RichText::new("Methods paragraph copied").size(small).color(tokens.link_green));
            }
            ui.add_space(4.0);
            egui::Frame::NONE
                .fill(tokens.panel_bg)
                .stroke(Stroke::new(1.0, tokens.divider))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("inspector.methods.preview_scroll").max_height(180.0).show(ui, |ui| {
                        let mut preview = report;
                        ui.add(egui::TextEdit::multiline(&mut preview).desired_width(ui.available_width()).desired_rows(6).interactive(false));
                    });
                });
        }
    }

    ui.add_space(6.0);
    widgets::divider(ui);
    ui.add_space(4.0);
    if studio_full_button(
        ui,
        tokens,
        meta,
        &StudioButtonSpec {
            label: tr(locale, "results.parameters"),
            tooltip: "Edit upstream analysis parameters (Cmd/Ctrl+T)",
            selected: false,
            enabled: ctx_in.enabled,
            primary: false,
            widget_id: "inspector.results.edit_params",
            command_id: Some("ui.editParams.open"),
            disabled_reason: None,
        },
    ) {
        intents.push_ui("ui.editParams.open", json!({ "result_sheet_id": rs.id.as_str() }));
    }
}
