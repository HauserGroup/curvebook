//! 7-tab `Nonlinear Curve Fitting` modal (plus MV `[Data]` tab) and 3-branch `Hook Constant`
//! browser in `egui 0.37` (`R15.4`, `gui-layout-spec.md` §8.9).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ConvergenceStrictness, FitFlagRules, FitMethod, HookSource, IdentifiabilityMode, ModelComparisonMethod,
    ModelComparisonQuestion, ModelComparisonSpec, NonlinearFitSpec, OutlierHandling, PValueFormat, ParameterRule, ParameterScope, ProjectDocument,
    ReplicateFitMode, SheetId, TableArchetype, WeightingScheme, WeightingSource,
};
use curvebook_fit::{
    CompiledDslModel, DataPoint, DslEquationKind, EquationCatalogEntry, EquationCategory, ParametricModel, extract_datasets_from_sheet_multi,
    format_model_equation_tex, list_builtin_equations, resolve_model_from_spec,
};
use egui::{Context, Id, Key, Modifiers, Rect, Stroke, TextEdit, Ui, Vec2};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, ConstantHookPickerState, EditAnalysisParametersState};
use crate::grid_view::parse_localized_number_input;
use crate::i18n::{LocalePreset, normalize_locale, tr};
use crate::inspect::ZLayer;
use crate::modal_predicates::{ControlEnablement, r9_pvalue_precision_spinner};

use super::stat_anova_modal::{
    ClippedTextSpec, StepperSpec, record_widget, render_clipped_text, render_modal_checkbox, render_modal_radio, render_modal_seg_btn,
    render_modal_textbox_row, render_section_title, render_stepper_row,
};
use super::stats_modals::{Wave4ModalShell, render_modal_action_button, render_mv_role_badge, render_tex_equation_preview, show_wave4_modal};
use super::ui_kit::{LinkButtonSpec, TooltipMode, TooltipSpec, link_button};
use super::{FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};

/// Core tabs of the `Nonlinear Curve Fitting` parameter modal (`gui-layout-spec.md` §8.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FitModalTab {
    Data,
    #[default]
    Model,
    Compare,
    Constrain,
    InitialValues,
    Range,
    Weight,
    Output,
}

impl FitModalTab {
    /// Core 7 tabs in §8.9 order (`Data` prepended dynamically on MV tables).
    pub const CORE_SEVEN: [Self; 7] = [Self::Model, Self::Compare, Self::Constrain, Self::InitialValues, Self::Range, Self::Weight, Self::Output];

    /// Canonical tab slug (`fit.tab.<slug>`).
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Model => "model",
            Self::Compare => "compare",
            Self::Constrain => "constraints",
            Self::InitialValues => "initial_values",
            Self::Range => "weights_range",
            Self::Weight => "weight",
            Self::Output => "output",
        }
    }

    /// Localized tab label across all 16 locales.
    #[must_use]
    pub fn localized_label(self, locale: &str) -> &'static str {
        match self {
            Self::Data => tr(locale, "fit.tab.data"),
            Self::Model => tr(locale, "fit.tab.model"),
            Self::Compare => tr(locale, "fit_params.tab.compare"),
            Self::Constrain => tr(locale, "fit.tab.constrain"),
            Self::InitialValues => tr(locale, "fit_params.tab.initial_values"),
            Self::Range => tr(locale, "fit.tab.range"),
            Self::Weight => tr(locale, "fit.tab.weight"),
            Self::Output => tr(locale, "fit.tab.output"),
        }
    }

    /// Map legacy `fit_params` or §8.9 tab slugs into [`FitModalTab`].
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug.trim().to_ascii_lowercase().as_str() {
            "data" => Some(Self::Data),
            "model" | "equation" => Some(Self::Model),
            "compare" => Some(Self::Compare),
            "constrain" | "constraints" => Some(Self::Constrain),
            "initial_values" | "initial" => Some(Self::InitialValues),
            "range" | "weights_range" => Some(Self::Range),
            "weight" | "weights" => Some(Self::Weight),
            "output" | "confidence" | "diagnostics" => Some(Self::Output),
            _ => None,
        }
    }
}

/// Persistent transient UI state for `render_fit_modal` and `render_hook_constant_modal`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FitModalEguiState {
    pub active_tab: FitModalTab,
    pub model_search: String,
    pub custom_equation_draft: String,
    pub custom_dsl_enabled: bool,
    pub hook_search_draft: String,
    pub open_session_sheet_id: Option<String>,
    pub r2_threshold_draft: String,
    pub expand_info_branch: bool,
    pub expand_analysis_branch: bool,
    pub expand_document_branch: bool,
    pub x_min_draft: String,
    pub x_max_draft: String,
    pub initial_override_drafts: BTreeMap<String, String>,
    pub lower_bound_drafts: BTreeMap<String, String>,
    pub upper_bound_drafts: BTreeMap<String, String>,
    pub constant_drafts: BTreeMap<String, String>,
    pub method_saved_as_default: bool,
    pub saved_default_spec: Option<AnalysisSpec>,
}

/// Reset transient per-session drafts on `FitModalEguiState` while preserving `saved_default_spec`.
pub(crate) fn reset_session_drafts(state: &mut FitModalEguiState) {
    state.open_session_sheet_id = None;
    state.model_search.clear();
    state.hook_search_draft.clear();
    state.custom_dsl_enabled = false;
    state.custom_equation_draft.clear();
    state.r2_threshold_draft.clear();
    state.x_min_draft.clear();
    state.x_max_draft.clear();
    state.initial_override_drafts.clear();
    state.lower_bound_drafts.clear();
    state.upper_bound_drafts.clear();
    state.constant_drafts.clear();
}

/// Frame geometry and modal capture report emitted by `fit_modal`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FitModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

fn builtin_catalog() -> &'static [EquationCatalogEntry] {
    static CATALOG: OnceLock<Vec<EquationCatalogEntry>> = OnceLock::new();
    CATALOG.get_or_init(list_builtin_equations)
}

/// Return `true` when `ActiveDialog::EditAnalysisParameters` edits a `NonlinearRegression` spec.
#[must_use]
pub fn is_nonlinear_fit_dialog_active(app: &UiApp) -> bool {
    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return false;
    };
    if let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() {
        return matches!(spec.kind, AnalysisKind::NonlinearRegression(_));
    }
    app.document().result_sheet(result_sheet_id).is_some_and(|rs| matches!(rs.analysis.kind, AnalysisKind::NonlinearRegression(_)))
}

/// Render the 7-tab `Nonlinear Curve Fitting` modal when active.
pub fn render_fit_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut FitModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> FitModalFrameReport {
    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        reset_session_drafts(state);
        return FitModalFrameReport::default();
    };
    let doc = app.document();
    let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref().or_else(|| doc.result_sheet(result_sheet_id).map(|rs| &rs.analysis)) else {
        reset_session_drafts(state);
        return FitModalFrameReport::default();
    };
    let AnalysisKind::NonlinearRegression(nl) = &spec.kind else {
        reset_session_drafts(state);
        return FitModalFrameReport::default();
    };

    sync_fit_modal_state(result_sheet_id, doc, &app.canvas_dialogs.edit_params, spec, nl, state, intents);
    let is_mv = resolve_fit_source_sheet(doc, spec).is_some_and(|ds| matches!(ds.archetype, TableArchetype::MultipleVariables(_)));
    let mut tabs = Vec::with_capacity(8);
    if is_mv {
        tabs.push(FitModalTab::Data);
    }
    tabs.extend(FitModalTab::CORE_SEVEN);
    if !tabs.contains(&state.active_tab) {
        state.active_tab = FitModalTab::Model;
    }

    let eq_label = if nl.custom_dsl.is_some() {
        format!("Custom DSL: {}", nl.equation_id)
    } else {
        builtin_catalog().iter().find(|e| e.id == nl.equation_id).map_or_else(|| nl.equation_id.clone(), |e| e.name.clone())
    };
    let banner = format!("{eq_label}  |  {:?}  |  {} CI", nl.method, if nl.profile_likelihood_ci { "Profile-likelihood" } else { "Wald-t" });
    let param_names = resolve_parameter_names(nl);
    let locale_preset = &app.locale_preset;
    let (alerts, blocking_error) = compute_fit_alerts(nl, &param_names, state, locale_preset);
    let log_x_warning = if nl.custom_dsl.is_none()
        && let Some(src) = spec.source_sheet_id.as_ref()
    {
        let xs = collect_sheet_active_x_values(doc, src, nl.x_range_filter, &nl.mv_x_columns);
        curvebook_fit::detect_log_x_scale_mismatch(&nl.equation_id, &xs)
    } else {
        None
    };
    let commit_en = blocking_error.as_deref().map_or_else(ControlEnablement::enabled, ControlEnablement::disabled);
    let locale = normalize_locale(&app.locale);
    let is_create = doc.result_sheet(result_sheet_id).is_none();
    let wants_kb_before = ctx.egui_wants_keyboard_input();

    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        state,
        Wave4ModalShell {
            widget_id: "dialog.nonlinear_fit",
            title: tr(locale, "fit.modal.title"),
            subtitle_or_banner: Some(&banner),
            banner_widget_id: "edit.resolved_test_banner",
            max_width_pt: 820.0,
            max_height_pt: 580.0,
            use_body_scroll: false,
            footer_reserve_pt: 42.0,
        },
        |ui, state, meta, intents| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for tab in &tabs {
                    let selected = state.active_tab == *tab;
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("fit.tab.{}", tab.slug()),
                        tab.localized_label(locale),
                        selected,
                        &ControlEnablement::enabled(),
                    ) {
                        state.active_tab = *tab;
                        intents.push(UiIntent::SetEditParamsActiveTab { tab: (tab.slug()).into() });
                    }
                }
            });
            if let Some(warn) = &log_x_warning {
                ui.add_space(4.0);
                egui::Frame::NONE
                    .fill(tokens.panel_bg)
                    .stroke(Stroke::new(1.0, tokens.warning))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(Vec2::new(8.0, 5.0))
                    .show(ui, |ui| {
                        let msg = if let Some(alt_id) = &warn.suggested_equation_id {
                            let alt_name = builtin_catalog().iter().find(|e| e.id == *alt_id).map_or_else(|| alt_id.clone(), |e| e.name.clone());
                            format!("X-scale mismatch: {} Suggested non-log counterpart: {}.", warn.reason, alt_name)
                        } else {
                            format!("X-scale mismatch: {} Transform X to log10(X) before fitting this log-X model.", warn.reason)
                        };
                        render_clipped_text(
                            ui,
                            meta,
                            ClippedTextSpec {
                                widget_id: "fit.warning.log_x_scale",
                                role: "status",
                                text: &msg,
                                font_size: tokens.metrics.font_ui_small_pt,
                                color: tokens.warning,
                                z_layer: ZLayer::ModalDialog,
                                tooltip: Some(&msg),
                                record_meta: true,
                            },
                        );
                        if let Some(alt_id) = &warn.suggested_equation_id {
                            let alt_name = builtin_catalog().iter().find(|e| e.id == *alt_id).map_or_else(|| alt_id.clone(), |e| e.name.clone());
                            if render_modal_seg_btn(
                                ui,
                                tokens,
                                meta,
                                "fit.warning.switch_non_log",
                                &format!("Switch to {alt_name}"),
                                false,
                                &ControlEnablement::enabled(),
                            ) {
                                let mut next = spec.clone();
                                if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                                    fit.equation_id = alt_id.clone();
                                    fit.custom_dsl = None;
                                    fit.parameter_rules.clear();
                                    sync_summary_parameter_for_fit(fit);
                                }
                                state.custom_dsl_enabled = false;
                                clear_fit_parameter_drafts(state);
                                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                            }
                        }
                    });
            }
            if !alerts.is_empty() {
                ui.add_space(4.0);
                egui::Frame::NONE
                    .fill(tokens.panel_bg)
                    .stroke(Stroke::new(1.0, tokens.link_red))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(Vec2::new(8.0, 4.0))
                    .show(ui, |ui| {
                        for (idx, alert) in alerts.iter().enumerate() {
                            render_clipped_text(
                                ui,
                                meta,
                                ClippedTextSpec {
                                    widget_id: &format!("fit.alert.{idx}"),
                                    role: "status",
                                    text: alert,
                                    font_size: tokens.metrics.font_ui_small_pt,
                                    color: tokens.link_red,
                                    z_layer: ZLayer::ModalDialog,
                                    tooltip: Some(alert),
                                    record_meta: true,
                                },
                            );
                        }
                    });
            }
        },
        |ui, state, avail_w, avail_h, meta, intents| {
            let tab_ctx = FitTabCtx { result_sheet_id, doc, spec, nl, param_names: &param_names, locale_preset };
            let scroll_w = (avail_w - 18.0).max(160.0);
            egui::ScrollArea::vertical()
                .id_salt(("wave4_body_scroll", "dialog.nonlinear_fit", result_sheet_id.as_str(), state.active_tab.slug()))
                .max_height(avail_h)
                .min_scrolled_height(avail_h)
                .auto_shrink([false, false])
                .scroll_source(egui::scroll_area::ScrollSource { mouse_wheel: true, scroll_bar: true, ..egui::scroll_area::ScrollSource::NONE })
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    ui.set_width(scroll_w);
                    ui.set_max_width(scroll_w);
                    match state.active_tab {
                        FitModalTab::Data => render_fit_data_tab(ui, doc, spec, nl, tokens, meta, intents),
                        FitModalTab::Model => {
                            render_fit_model_tab(ctx, ui, FitModelTabCtx { doc, spec, nl, param_names: &param_names }, state, tokens, meta, intents)
                        }
                        FitModalTab::Compare => render_fit_compare_tab(ui, spec, nl, locale, tokens, meta, intents),
                        FitModalTab::Constrain => render_fit_constrain_tab(ctx, ui, &tab_ctx, state, tokens, meta, intents),
                        FitModalTab::InitialValues => render_fit_initial_tab(ui, &tab_ctx, state, tokens, meta, intents),
                        FitModalTab::Range => render_fit_range_tab(ui, &tab_ctx, state, tokens, meta, intents),
                        FitModalTab::Weight => render_fit_weight_tab(ui, &tab_ctx, locale, tokens, meta, intents),
                        FitModalTab::Output => render_fit_output_tab(ui, spec, nl, locale, state, tokens, meta, intents),
                    }
                });
        },
        |ui, footer_state, meta, intents| {
            ui.horizontal(|ui| {
                let save_lbl =
                    if footer_state.method_saved_as_default { tr(locale, "anova.saved_default") } else { tr(locale, "anova.save_default") };
                if render_modal_action_button(
                    ui,
                    tokens,
                    meta,
                    "dialog.button.save_default",
                    save_lbl,
                    false,
                    footer_state.method_saved_as_default,
                    &ControlEnablement::enabled(),
                ) {
                    footer_state.saved_default_spec = Some(spec.clone());
                    footer_state.method_saved_as_default = true;
                    intents.push(UiIntent::SaveEditParamsAsDefault);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ok_lbl = if is_create { tr(locale, "fit.commit.create") } else { tr(locale, "btn.ok") };
                    if render_modal_action_button(ui, tokens, meta, "modal.ok", ok_lbl, true, false, &commit_en) {
                        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
                    }
                    if !is_create
                        && doc.result_sheet(result_sheet_id).is_some()
                        && render_modal_action_button(ui, tokens, meta, "modal.apply", tr(locale, "fit.commit.update"), false, false, &commit_en)
                    {
                        intents.push(UiIntent::DialogButtonAction { action: String::from("apply") });
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
    } else if !wants_kb_before
        && !ctx.egui_wants_keyboard_input()
        && blocking_error.is_none()
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
    {
        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Tab) || i.consume_key(Modifiers::CTRL, Key::PageDown)) {
        step_fit_tab(&tabs, state, 1, intents);
    } else if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) || i.consume_key(Modifiers::CTRL, Key::PageUp)) {
        step_fit_tab(&tabs, state, -1, intents);
    }

    FitModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

struct FitTabCtx<'a> {
    result_sheet_id: &'a SheetId,
    doc: &'a ProjectDocument,
    spec: &'a curvebook_doc::AnalysisSpec,
    nl: &'a NonlinearFitSpec,
    param_names: &'a [String],
    locale_preset: &'a LocalePreset,
}

/// Format a finite floating-point fit parameter or domain bound concisely without
/// truncating small scientific values (for example `1e-7`) to `0.000`.
fn format_fit_number(value: f64) -> String {
    if !value.is_finite() {
        return format!("{value}");
    }
    if value == 0.0 {
        return String::from("0");
    }
    let abs = value.abs();
    if !(1e-4..1e6).contains(&abs) {
        let raw = format!("{value:.6e}");
        if let Some((mantissa, exp)) = raw.split_once('e') {
            let trimmed_m = mantissa.trim_end_matches('0').trim_end_matches('.');
            return format!("{trimmed_m}e{exp}");
        }
        return raw;
    }
    let raw = format!("{value:.6}");
    let trimmed = raw.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" || trimmed.is_empty() { String::from("0") } else { trimmed.to_owned() }
}

fn parse_fit_number(raw: &str, preset: &LocalePreset) -> Option<f64> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    parse_localized_number_input(trimmed, preset).or_else(|| trimmed.parse::<f64>().ok()).filter(|v| v.is_finite())
}

fn resolve_fit_source_sheet<'a>(doc: &'a ProjectDocument, spec: &curvebook_doc::AnalysisSpec) -> Option<&'a curvebook_doc::DataSheet> {
    let src = spec.source_sheet_id.as_ref()?;
    doc.data_sheet(src).or_else(|| doc.result_sheet(src).and_then(|rs| rs.analyzable_data.as_ref()))
}

fn extract_fit_data_points(doc: &ProjectDocument, spec: &curvebook_doc::AnalysisSpec, nl: &NonlinearFitSpec) -> Vec<DataPoint> {
    let Some(sheet) = resolve_fit_source_sheet(doc, spec) else {
        return Vec::new();
    };
    let y_cols = nl.effective_mv_y_columns();
    let Ok(datasets) = extract_datasets_from_sheet_multi(sheet, &spec.selected_data_sets, &nl.mv_group_columns, &nl.mv_x_columns, &y_cols) else {
        return Vec::new();
    };
    datasets.into_iter().flat_map(|d| d.points.into_iter()).filter(|p| p.x0().is_finite() && p.y.is_finite()).collect()
}

fn extract_source_x_range(doc: &ProjectDocument, spec: &curvebook_doc::AnalysisSpec, nl: &NonlinearFitSpec) -> Option<(f64, f64)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for p in extract_fit_data_points(doc, spec, nl) {
        let x = p.x0();
        if x.is_finite() {
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    if lo.is_finite() && hi.is_finite() {
        return Some((lo, hi));
    }
    let sheet = resolve_fit_source_sheet(doc, spec)?;
    if let TableArchetype::Xy(xy) = &sheet.archetype {
        for r in 0..xy.row_count() {
            if let Some(x) = xy.raw_x_value_at(r)
                && x.is_finite()
            {
                lo = lo.min(x);
                hi = hi.max(x);
            }
        }
    }
    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
}

fn seed_x_domain_bounds(doc: &ProjectDocument, spec: &curvebook_doc::AnalysisSpec, nl: &NonlinearFitSpec) -> (f64, f64) {
    match extract_source_x_range(doc, spec, nl) {
        Some((lo, hi)) if lo < hi => (lo, hi),
        Some((lo, _)) => (lo, lo + 1.0),
        None => (0.0, 100.0),
    }
}

fn sync_fit_modal_state(
    result_sheet_id: &SheetId,
    doc: &ProjectDocument,
    ep: &EditAnalysisParametersState,
    spec: &AnalysisSpec,
    nl: &NonlinearFitSpec,
    state: &mut FitModalEguiState,
    intents: &mut PendingUiIntents,
) {
    let is_create_mode = doc.result_sheet(result_sheet_id).is_none();
    if state.open_session_sheet_id.as_deref() != Some(result_sheet_id.as_str()) {
        state.open_session_sheet_id = Some(result_sheet_id.as_str().to_owned());
        state.model_search.clear();
        state.hook_search_draft.clear();
        state.active_tab = FitModalTab::from_slug(&ep.active_tab).unwrap_or(FitModalTab::Model);
        let effective_nl = if is_create_mode
            && let Some(saved) = &state.saved_default_spec
            && let AnalysisKind::NonlinearRegression(saved_nl) = &saved.kind
        {
            let mut applied = saved.clone();
            applied.id = spec.id.clone();
            applied.source_sheet_id = spec.source_sheet_id.clone();
            applied.selected_data_sets.clone_from(&spec.selected_data_sets);
            applied.custom_title.clone_from(&spec.custom_title);
            if applied != *spec {
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(applied) });
            }
            state.method_saved_as_default = true;
            saved_nl
        } else {
            state.method_saved_as_default = state.saved_default_spec.as_ref().is_some_and(|saved| {
                saved.kind == spec.kind && saved.confidence_level == spec.confidence_level && saved.p_value_format == spec.p_value_format
            });
            nl
        };
        state.custom_dsl_enabled = effective_nl.custom_dsl.is_some();
        state.custom_equation_draft =
            effective_nl.custom_dsl.clone().unwrap_or_else(|| String::from("Y = Bottom + (Top - Bottom) / (1 + 10^((LogEC50 - X) * HillSlope))"));
        state.r2_threshold_draft = effective_nl.flag_r_squared_below.map_or_else(String::new, |v| format!("{v}"));
        let (xmin_str, xmax_str) = match effective_nl.x_range_filter {
            Some((lo, hi)) => (lo.map_or_else(String::new, format_fit_number), hi.map_or_else(String::new, format_fit_number)),
            None => (String::new(), String::new()),
        };
        state.x_min_draft = xmin_str;
        state.x_max_draft = xmax_str;
        state.initial_override_drafts.clear();
        state.lower_bound_drafts.clear();
        state.upper_bound_drafts.clear();
        state.constant_drafts.clear();
        for rule in &effective_nl.parameter_rules {
            if let ParameterScope::Constant(val) = rule.scope {
                state.constant_drafts.insert(rule.name.clone(), format_fit_number(val));
            }
            if let Some(init) = rule.initial_override {
                state.initial_override_drafts.insert(rule.name.clone(), format_fit_number(init));
            }
            if let Some(lo) = rule.lower_bound {
                state.lower_bound_drafts.insert(rule.name.clone(), format_fit_number(lo));
            }
            if let Some(hi) = rule.upper_bound {
                state.upper_bound_drafts.insert(rule.name.clone(), format_fit_number(hi));
            }
        }
    } else {
        state.method_saved_as_default = state.saved_default_spec.as_ref().is_some_and(|saved| {
            saved.kind == spec.kind && saved.confidence_level == spec.confidence_level && saved.p_value_format == spec.p_value_format
        });
    }
}

fn step_fit_tab(tabs: &[FitModalTab], state: &mut FitModalEguiState, dir: isize, intents: &mut PendingUiIntents) {
    if tabs.is_empty() {
        return;
    }
    let len = tabs.len() as isize;
    let cur = tabs.iter().position(|&t| t == state.active_tab).unwrap_or(0) as isize;
    let next = tabs[(cur + dir).rem_euclid(len) as usize];
    state.active_tab = next;
    intents.push(UiIntent::SetEditParamsActiveTab { tab: (next.slug()).into() });
}

fn resolve_parameter_names(nl: &NonlinearFitSpec) -> Vec<String> {
    if let Some(dsl) = &nl.custom_dsl
        && let Ok(compiled) = CompiledDslModel::compile("custom", dsl)
    {
        return compiled.param_names().to_vec();
    }
    if let Some(model) = curvebook_fit::get_builtin_model(&nl.equation_id) {
        return model.param_names().to_vec();
    }
    nl.parameter_rules.iter().map(|r| r.name.clone()).collect()
}

fn check_draft_numeric_alerts(
    nl: &NonlinearFitSpec,
    params: &[String],
    state: &FitModalEguiState,
    preset: &LocalePreset,
    alerts: &mut Vec<String>,
    blocking: &mut Option<String>,
) {
    if !state.r2_threshold_draft.trim().is_empty() {
        match parse_fit_number(&state.r2_threshold_draft, preset) {
            Some(v) if (0.0..=1.0).contains(&v) => {}
            _ => alerts.push(format!("Output alert: R² flag threshold (`{}`) must lie in [0, 1].", state.r2_threshold_draft.trim())),
        }
    }
    let mut record_blocking = |msg: String| {
        if blocking.is_none() {
            *blocking = Some(msg.clone());
        }
        alerts.push(msg);
    };
    if nl.x_range_filter.is_some() {
        if !state.x_min_draft.trim().is_empty() && parse_fit_number(&state.x_min_draft, preset).is_none() {
            record_blocking(format!("Range alert: X min (`{}`) must be a finite number.", state.x_min_draft.trim()));
        }
        if !state.x_max_draft.trim().is_empty() && parse_fit_number(&state.x_max_draft, preset).is_none() {
            record_blocking(format!("Range alert: X max (`{}`) must be a finite number.", state.x_max_draft.trim()));
        }
    }
    if state.custom_dsl_enabled
        && let Err(err) = CompiledDslModel::compile("custom", &state.custom_equation_draft)
    {
        record_blocking(format!("DSL equation alert: {err}"));
    }
    for p in params {
        if nl.parameter_rules.iter().any(|r| r.name == *p && matches!(r.scope, ParameterScope::Constant(_)))
            && let Some(raw) = state.constant_drafts.get(p)
            && parse_fit_number(raw, preset).is_none()
        {
            record_blocking(format!("Constraint alert on `{p}`: constant value `{}` must be a finite number.", raw.trim()));
        }
        if let Some(raw) = state.initial_override_drafts.get(p)
            && !raw.trim().is_empty()
            && parse_fit_number(raw, preset).is_none()
        {
            record_blocking(format!("Initial value alert on `{p}`: `{}` is not a finite number.", raw.trim()));
        }
        if let Some(raw) = state.lower_bound_drafts.get(p)
            && !raw.trim().is_empty()
            && parse_fit_number(raw, preset).is_none()
        {
            record_blocking(format!("Constraint alert on `{p}`: lower bound `{}` is not a finite number.", raw.trim()));
        }
        if let Some(raw) = state.upper_bound_drafts.get(p)
            && !raw.trim().is_empty()
            && parse_fit_number(raw, preset).is_none()
        {
            record_blocking(format!("Constraint alert on `{p}`: upper bound `{}` is not a finite number.", raw.trim()));
        }
    }
}

fn compute_fit_alerts(nl: &NonlinearFitSpec, params: &[String], state: &FitModalEguiState, preset: &LocalePreset) -> (Vec<String>, Option<String>) {
    let mut alerts = Vec::new();
    let mut blocking: Option<String> = None;
    check_draft_numeric_alerts(nl, params, state, preset, &mut alerts, &mut blocking);

    if let Some((Some(xmin), Some(xmax))) = nl.x_range_filter
        && xmin >= xmax
    {
        let msg = format!("Range alert: X min ({}) must be strictly less than X max ({}).", format_fit_number(xmin), format_fit_number(xmax));
        if blocking.is_none() {
            blocking = Some(msg.clone());
        }
        alerts.push(msg);
    }
    if let Some((Some(ymin), Some(ymax))) = nl.y_range_filter
        && ymin >= ymax
    {
        alerts.push(format!("Range alert: Y min ({ymin}) must be strictly less than Y max ({ymax})."));
    }
    if let (Some(pmin), Some(pmax)) = (nl.plot_x_min, nl.plot_x_max)
        && pmin >= pmax
    {
        alerts.push(format!("Plot domain alert: Plot X min ({pmin}) must be less than Plot X max ({pmax})."));
    }
    let mut fixed_count = 0usize;
    for rule in &nl.parameter_rules {
        if params.iter().any(|p| p == &rule.name)
            && matches!(rule.scope, ParameterScope::Constant(_) | ParameterScope::HookedConstant(_) | ParameterScope::ColumnTitleConstant)
        {
            fixed_count += 1;
        }
        if let (Some(lo), Some(hi)) = (rule.lower_bound, rule.upper_bound)
            && lo >= hi
        {
            let msg = format!(
                "Constraint conflict on `{}`: lower bound ({}) >= upper bound ({}).",
                rule.name,
                format_fit_number(lo),
                format_fit_number(hi)
            );
            if blocking.is_none() {
                blocking = Some(msg.clone());
            }
            alerts.push(msg);
        }
        if let Some(init) = rule.initial_override {
            if let Some(lo) = rule.lower_bound
                && init < lo
            {
                alerts.push(format!(
                    "Identifiability warning: `{}` initial ({}) < lower bound ({}).",
                    rule.name,
                    format_fit_number(init),
                    format_fit_number(lo)
                ));
            }
            if let Some(hi) = rule.upper_bound
                && init > hi
            {
                alerts.push(format!(
                    "Identifiability warning: `{}` initial ({}) > upper bound ({}).",
                    rule.name,
                    format_fit_number(init),
                    format_fit_number(hi)
                ));
            }
        }
    }
    if !params.is_empty() && fixed_count >= params.len() && !nl.plot_function_only {
        alerts.push(String::from("Identifiability alert: all parameters are fixed; enable Plot Function Only or unfix >= 1 parameter."));
    }
    (alerts, blocking)
}

struct FitMvGridCtx<'a> {
    doc: &'a ProjectDocument,
    spec: &'a curvebook_doc::AnalysisSpec,
    nl: &'a NonlinearFitSpec,
    id_salt: &'a str,
}

fn render_fit_mv_mapping_grid(
    ui: &mut Ui,
    grid_ctx: &FitMvGridCtx<'_>,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> bool {
    let FitMvGridCtx { doc, spec, nl, id_salt } = *grid_ctx;
    let Some(ds) = resolve_fit_source_sheet(doc, spec) else {
        return false;
    };
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return false;
    };
    render_section_title(ui, tokens, "Multiple-Variables Curve Mapping (X predictors, Y response, Grouping)");
    egui::Grid::new(id_salt).num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
        for (idx, var) in mv.variables.iter().enumerate() {
            ui.horizontal(|ui| {
                render_mv_role_badge(ui, tokens, var.role, var.role_inferred);
                ui.label(&var.title);
                let eff_y = nl.effective_mv_y_columns();
                if render_modal_seg_btn(ui, tokens, meta, &format!("fit.mv.y.{idx}"), "Y", nl.mv_y_column == Some(idx), &ControlEnablement::enabled())
                {
                    let mut next = spec.clone();
                    if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                        fit.mv_y_column = Some(idx);
                        fit.mv_y_columns.clear();
                        fit.mv_x_columns.retain(|&c| c != idx);
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
                if render_modal_seg_btn(
                    ui,
                    tokens,
                    meta,
                    &format!("fit.mv.y_multi.{idx}"),
                    "+Y",
                    eff_y.contains(&idx) && nl.mv_y_columns.contains(&idx),
                    &ControlEnablement::enabled(),
                ) {
                    let mut next = spec.clone();
                    if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                        if fit.mv_y_columns.is_empty()
                            && let Some(prev) = fit.mv_y_column
                        {
                            fit.mv_y_columns.push(prev);
                        }
                        if let Some(p) = fit.mv_y_columns.iter().position(|&c| c == idx) {
                            fit.mv_y_columns.remove(p);
                        } else {
                            fit.mv_y_columns.push(idx);
                            fit.mv_y_columns.sort_unstable();
                            fit.mv_x_columns.retain(|&c| c != idx);
                        }
                        fit.mv_y_column = fit.mv_y_columns.first().copied();
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
                if render_modal_seg_btn(
                    ui,
                    tokens,
                    meta,
                    &format!("fit.mv.x.{idx}"),
                    "X",
                    nl.mv_x_columns.contains(&idx),
                    &ControlEnablement::enabled(),
                ) {
                    let mut next = spec.clone();
                    if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                        if let Some(p) = fit.mv_x_columns.iter().position(|&c| c == idx) {
                            fit.mv_x_columns.remove(p);
                        } else {
                            fit.mv_x_columns.push(idx);
                            fit.mv_x_columns.sort_unstable();
                            fit.mv_y_columns.retain(|&c| c != idx);
                            if fit.mv_y_column == Some(idx) {
                                fit.mv_y_column = fit.mv_y_columns.first().copied();
                            }
                        }
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
                if render_modal_seg_btn(
                    ui,
                    tokens,
                    meta,
                    &format!("fit.mv.group.{idx}"),
                    "Grp",
                    nl.mv_group_columns.contains(&idx),
                    &ControlEnablement::enabled(),
                ) {
                    let mut next = spec.clone();
                    if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                        if let Some(p) = fit.mv_group_columns.iter().position(|&c| c == idx) {
                            fit.mv_group_columns.remove(p);
                        } else {
                            fit.mv_group_columns.push(idx);
                            fit.mv_group_columns.sort_unstable();
                        }
                    }
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
            });
            if idx % 2 == 1 {
                ui.end_row();
            }
        }
    });
    true
}

fn render_fit_data_tab(
    ui: &mut Ui,
    doc: &ProjectDocument,
    spec: &curvebook_doc::AnalysisSpec,
    nl: &NonlinearFitSpec,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    if !render_fit_mv_mapping_grid(ui, &FitMvGridCtx { doc, spec, nl, id_salt: "fit_mv_data_grid" }, tokens, meta, intents) {
        ui.label("All columns / datasets in the XY source table are available for fitting.");
    }
}

struct FitModelTabCtx<'a> {
    doc: &'a ProjectDocument,
    spec: &'a curvebook_doc::AnalysisSpec,
    nl: &'a NonlinearFitSpec,
    param_names: &'a [String],
}

fn render_fit_model_tab(
    ctx: &Context,
    ui: &mut Ui,
    model_ctx: FitModelTabCtx<'_>,
    state: &mut FitModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let FitModelTabCtx { doc, spec, nl, param_names } = model_ctx;
    if render_fit_mv_mapping_grid(ui, &FitMvGridCtx { doc, spec, nl, id_salt: "fit_mv_model_grid" }, tokens, meta, intents) {
        ui.add_space(4.0);
    }
    let preview_params: Vec<(String, f64)> = param_names.iter().map(|p| (p.clone(), 1.0)).collect();
    let tex = format_model_equation_tex(&nl.equation_id, nl.custom_dsl.as_deref(), &preview_params, false, 4);
    render_section_title(ui, tokens, tr(tokens.locale, "fit_params.eq.preview_heading"));
    render_tex_equation_preview(ui, tokens, meta, "fit.eq.tex_preview", &tex, 13.5);

    ui.add_space(4.0);
    // Guidance Card
    egui::Frame::NONE
        .fill(tokens.panel_bg)
        .stroke(Stroke::new(1.0, tokens.border))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(Vec2::new(8.0, 5.0))
        .show(ui, |ui| {
            let entry_opt = builtin_catalog().iter().find(|e| e.id == nl.equation_id);
            let cat_str = entry_opt.map_or("Custom DSL Equation", |e| eq_category_label(e.category));
            ui.label(
                egui::RichText::new(format!("Guidance — {cat_str}  |  Parameters ({}): {}", param_names.len(), param_names.join(", ")))
                    .small()
                    .color(tokens.text_primary),
            );
        });

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        if render_modal_seg_btn(
            ui,
            tokens,
            meta,
            "fit.eq.mode.builtin",
            "Built-in Library (117+)",
            !state.custom_dsl_enabled,
            &ControlEnablement::enabled(),
        ) {
            state.custom_dsl_enabled = false;
            if nl.custom_dsl.is_some() {
                let mut next = spec.clone();
                if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                    fit.custom_dsl = None;
                    fit.parameter_rules.clear();
                    sync_summary_parameter_for_fit(fit);
                }
                clear_fit_parameter_drafts(state);
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
        if render_modal_seg_btn(
            ui,
            tokens,
            meta,
            "fit.eq.custom_toggle",
            "Custom Equation DSL (Explicit / Implicit / ODE)",
            state.custom_dsl_enabled,
            &ControlEnablement::enabled(),
        ) {
            state.custom_dsl_enabled = true;
            let mut next = spec.clone();
            if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                fit.custom_dsl = Some(state.custom_equation_draft.clone());
                fit.parameter_rules.clear();
                sync_summary_parameter_for_fit(fit);
            }
            clear_fit_parameter_drafts(state);
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    });

    ui.add_space(6.0);
    if state.custom_dsl_enabled {
        render_section_title(ui, tokens, "Custom Explicit / Implicit / ODE DSL Equation");
        ui.horizontal_wrapped(|ui| {
            for (wid, lbl, preset_src) in [
                ("fit.eq.dsl.explicit", "Explicit Example", "Y = Top * X / (Kd + X)"),
                ("fit.eq.dsl.implicit", "Implicit F(X,Y)=0", "Y + Km * ln(Y / Y0) - Y0 + Vmax * X = 0"),
                ("fit.eq.dsl.ode", "ODE System d[A]/dt", "d[A]/dt = -k_el * [A]\nY = [A]"),
            ] {
                if render_modal_seg_btn(ui, tokens, meta, wid, lbl, false, &ControlEnablement::enabled()) {
                    state.custom_equation_draft = String::from(preset_src);
                    let mut next = spec.clone();
                    if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                        fit.custom_dsl = Some(state.custom_equation_draft.clone());
                        fit.parameter_rules.clear();
                        sync_summary_parameter_for_fit(fit);
                    }
                    clear_fit_parameter_drafts(state);
                    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                }
            }
        });
        let edit_resp = ui.add_sized(
            Vec2::new(ui.available_width().max(180.0), 74.0),
            TextEdit::multiline(&mut state.custom_equation_draft).id(Id::unique("fit.eq.custom_dsl")).code_editor(),
        );
        record_widget(
            ui,
            meta,
            &edit_resp,
            "fit.eq.custom_dsl",
            "textbox",
            "Custom DSL Equation",
            None,
            Some(serde_json::json!(state.custom_equation_draft)),
            true,
            None,
            ZLayer::ModalDialog,
            false,
            None,
        );
        if edit_resp.changed() {
            let mut next = spec.clone();
            if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                fit.custom_dsl = Some(state.custom_equation_draft.clone());
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
        match CompiledDslModel::compile("custom", &state.custom_equation_draft) {
            Ok(compiled) => {
                let kind_str = match compiled.kind() {
                    DslEquationKind::Explicit => "Explicit algebraic",
                    DslEquationKind::Implicit => "Implicit root-solve F(x,y)=0",
                    DslEquationKind::OdeFirstOrder => "First-order RK45 ODE",
                };
                ui.label(
                    egui::RichText::new(format!("Compiled OK ({kind_str}): params [{}]", compiled.param_names().join(", ")))
                        .small()
                        .color(tokens.link_green),
                );
            }
            Err(err) => {
                ui.label(egui::RichText::new(format!("DSL syntax note: {err}")).small().color(tokens.link_red));
            }
        }
    } else {
        let cat_filter_key = Id::unique("fit_modal.category_filter");
        let mut active_cat: Option<EquationCategory> = ctx.data(|d| d.get_temp(cat_filter_key)).flatten();

        ui.horizontal(|ui| {
            let q_resp = ui.add_sized(
                Vec2::new((ui.available_width() - 64.0).max(120.0), 22.0),
                TextEdit::singleline(&mut state.model_search)
                    .id(Id::unique("fit.eq.search"))
                    .hint_text("Search 117+ equations (EC50, Michaelis, exponential, NIST)"),
            );
            record_widget(
                ui,
                meta,
                &q_resp,
                "fit.eq.search",
                "textbox",
                "Search Equations",
                None,
                Some(serde_json::json!(state.model_search)),
                true,
                None,
                ZLayer::ModalDialog,
                false,
                None,
            );
            if render_modal_seg_btn(ui, tokens, meta, "fit.eq.search.clear", "Clear", false, &ControlEnablement::enabled()) {
                state.model_search.clear();
            }
        });

        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            if render_modal_seg_btn(ui, tokens, meta, "fit.eq.cat.all", "All Categories", active_cat.is_none(), &ControlEnablement::enabled()) {
                active_cat = None;
                ctx.data_mut(|d| d.insert_temp(cat_filter_key, active_cat));
            }
            for cat in ALL_EQ_CATEGORIES {
                let is_on = active_cat == Some(cat);
                if render_modal_seg_btn(
                    ui,
                    tokens,
                    meta,
                    &format!("fit.eq.cat.{cat:?}"),
                    eq_category_label(cat),
                    is_on,
                    &ControlEnablement::enabled(),
                ) {
                    active_cat = if is_on { None } else { Some(cat) };
                    ctx.data_mut(|d| d.insert_temp(cat_filter_key, active_cat));
                }
            }
        });

        ui.add_space(4.0);
        let q = state.model_search.trim().to_ascii_lowercase();
        let matches: Vec<&EquationCatalogEntry> = builtin_catalog()
            .iter()
            .filter(|e| {
                active_cat.is_none_or(|c| e.category == c)
                    && (q.is_empty()
                        || e.id.to_ascii_lowercase().contains(&q)
                        || e.name.to_ascii_lowercase().contains(&q)
                        || eq_category_label(e.category).to_ascii_lowercase().contains(&q))
            })
            .collect();
        ui.label(
            egui::RichText::new(format!("Showing {} of {} built-in equations", matches.len(), builtin_catalog().len()))
                .small()
                .color(tokens.text_muted),
        );
        for entry in matches {
            let selected = nl.custom_dsl.is_none() && nl.equation_id == entry.id;
            let lbl = format!("{} [{}]", entry.name, entry.param_names.join(", "));
            if render_modal_radio(ui, tokens, meta, &format!("fit.eq.item.{}", entry.id), &lbl, selected, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                    fit.equation_id = entry.id.clone();
                    fit.custom_dsl = None;
                    fit.parameter_rules.clear();
                    sync_summary_parameter_for_fit(fit);
                }
                clear_fit_parameter_drafts(state);
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    }
}

fn clear_fit_parameter_drafts(state: &mut FitModalEguiState) {
    state.initial_override_drafts.clear();
    state.lower_bound_drafts.clear();
    state.upper_bound_drafts.clear();
    state.constant_drafts.clear();
}

fn sync_summary_parameter_for_fit(fit: &mut NonlinearFitSpec) {
    let params = resolve_parameter_names(fit);
    if let Some(cur) = &fit.summary_parameter
        && !params.iter().any(|p| p == cur)
    {
        fit.summary_parameter = if fit.create_summary_graph { params.first().cloned() } else { None };
    } else if fit.create_summary_graph && fit.summary_parameter.is_none() {
        fit.summary_parameter = params.first().cloned();
    }
}

/// Collect active finite predictor `X` values from a sheet for `EXT-01` log-X scale checks.
pub(crate) fn collect_sheet_active_x_values(
    doc: &ProjectDocument,
    sheet_id: &SheetId,
    x_range_filter: Option<(Option<f64>, Option<f64>)>,
    mv_x_columns: &[usize],
) -> Vec<f64> {
    let Some(ds) = doc.data_sheet(sheet_id).or_else(|| doc.result_sheet(sheet_id).and_then(|rs| rs.analyzable_data.as_ref())) else {
        return Vec::new();
    };
    let in_range = |x: f64| -> bool {
        if !x.is_finite() {
            return false;
        }
        match x_range_filter {
            Some((lo, hi)) => lo.is_none_or(|a| x >= a) && hi.is_none_or(|b| x <= b),
            None => true,
        }
    };
    let mut xs = Vec::new();
    match &ds.archetype {
        TableArchetype::Xy(xy) => {
            for row in 0..xy.row_count() {
                if let Some(x) = xy.x_summary_at(row).ok().flatten()
                    && in_range(x)
                {
                    xs.push(x);
                }
            }
        }
        TableArchetype::MultipleVariables(mv) => {
            let cols: Vec<usize> = if mv_x_columns.is_empty() {
                mv.variables
                    .iter()
                    .enumerate()
                    .find(|(_, v)| !matches!(v.role, curvebook_doc::VariableRole::Label | curvebook_doc::VariableRole::Categorical))
                    .map(|(i, _)| vec![i])
                    .unwrap_or_default()
            } else {
                mv_x_columns.to_vec()
            };
            for col in cols {
                if let Some(var) = mv.variables.get(col) {
                    for cell in &var.cells {
                        if let Some(x) = cell.active_number()
                            && in_range(x)
                        {
                            xs.push(x);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    xs
}

fn render_fit_compare_tab(
    ui: &mut Ui,
    spec: &curvebook_doc::AnalysisSpec,
    nl: &NonlinearFitSpec,
    locale: &str,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Compare Two Models");
    let eff_cmp = nl.effective_comparison();
    let eff_method = eff_cmp.as_ref().map(|(m, _)| *m);
    let alt_id = match eff_cmp.as_ref().map(|(_, q)| q) {
        Some(ModelComparisonQuestion::AlternativeEquation { alternative_equation_id }) => alternative_equation_id.clone(),
        _ => match &nl.comparison {
            Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id })
            | Some(ModelComparisonSpec::Aicc { alternative_equation_id })
            | Some(ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id }) => alternative_equation_id.clone(),
            None => String::from("straight_line"),
        },
    };
    for (wid, lbl, active, comp) in [
        ("fit.cmp.none", "Fit one model only (no comparison)", eff_cmp.is_none(), None),
        (
            "fit.cmp.extra_f",
            "Extra sum-of-squares F test (nested models)",
            eff_method == Some(ModelComparisonMethod::ExtraSumOfSquaresF),
            Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: alt_id.clone() }),
        ),
        (
            "fit.cmp.aicc",
            "Compare AICc evidence ratio (nested or non-nested)",
            eff_method == Some(ModelComparisonMethod::Aicc),
            Some(ModelComparisonSpec::Aicc { alternative_equation_id: alt_id.clone() }),
        ),
        (
            "fit.cmp.lrt",
            "Likelihood Ratio Test (Poisson Deviance)",
            eff_method == Some(ModelComparisonMethod::LikelihoodRatioTest),
            Some(ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: alt_id.clone() }),
        ),
    ] {
        if render_modal_radio(ui, tokens, meta, wid, lbl, active, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                if let Some(next_cmp) = comp.clone() {
                    let next_method = next_cmp.method();
                    fit.comparison = Some(next_cmp);
                    fit.comparison_method = Some(next_method);
                } else {
                    fit.comparison = None;
                    fit.comparison_question = None;
                    fit.comparison_method = None;
                }
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }

    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "fit.cmp.prefer_well_defined",
        tr(locale, "fit.cmp.prefer_well_defined"),
        nl.prefer_well_defined_fit,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.prefer_well_defined_fit = !nl.prefer_well_defined_fit;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }

    if eff_cmp.is_some() {
        ui.add_space(6.0);
        render_section_title(ui, tokens, tr(locale, "fit.cmp.question.heading"));
        let first_param =
            resolve_model_from_spec(nl).ok().and_then(|m| m.param_names().first().cloned()).unwrap_or_else(|| String::from("HillSlope"));
        let active_q = nl.comparison_question.as_ref();
        for (wid, lbl, is_active, next_q) in [
            (
                "fit.cmp.q.alt_eq",
                tr(locale, "fit.cmp.q.alt_eq"),
                matches!(active_q, None | Some(ModelComparisonQuestion::AlternativeEquation { .. })),
                Some(ModelComparisonQuestion::AlternativeEquation { alternative_equation_id: alt_id.clone() }),
            ),
            (
                "fit.cmp.q.param_inv",
                tr(locale, "fit.cmp.q.param_inv"),
                matches!(active_q, Some(ModelComparisonQuestion::ParameterInvarianceAcrossDatasets { .. })),
                Some(ModelComparisonQuestion::ParameterInvarianceAcrossDatasets { shared_params: vec![first_param.clone()] }),
            ),
            (
                "fit.cmp.q.global_vs_sep",
                tr(locale, "fit.cmp.q.global_vs_sep"),
                matches!(active_q, Some(ModelComparisonQuestion::SingleGlobalCurveVsSeparate)),
                Some(ModelComparisonQuestion::SingleGlobalCurveVsSeparate),
            ),
            (
                "fit.cmp.q.theoretical",
                tr(locale, "fit.cmp.q.theoretical"),
                matches!(active_q, Some(ModelComparisonQuestion::ParameterEqualsTheoretical { .. })),
                Some(ModelComparisonQuestion::ParameterEqualsTheoretical { param: first_param.clone(), theoretical_value: 1.0 }),
            ),
        ] {
            if render_modal_radio(ui, tokens, meta, wid, lbl, is_active, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                    fit.comparison_question = next_q.clone();
                    if fit.comparison_method.is_none() {
                        fit.comparison_method =
                            Some(fit.comparison.as_ref().map_or(ModelComparisonMethod::ExtraSumOfSquaresF, ModelComparisonSpec::method));
                    }
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    }

    if let Some((active_method, _)) = eff_cmp
        && matches!(nl.comparison_question, None | Some(ModelComparisonQuestion::AlternativeEquation { .. }))
    {
        ui.add_space(6.0);
        render_section_title(ui, tokens, "Select Null / Alternative Comparison Equation");
        for entry in builtin_catalog() {
            if entry.id == nl.equation_id {
                continue;
            }
            if render_modal_radio(
                ui,
                tokens,
                meta,
                &format!("fit.cmp.alt.{}", entry.id),
                &entry.name,
                alt_id == entry.id,
                &ControlEnablement::enabled(),
            ) {
                let updated = match active_method {
                    ModelComparisonMethod::ExtraSumOfSquaresF => {
                        ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: entry.id.clone() }
                    }
                    ModelComparisonMethod::Aicc => ModelComparisonSpec::Aicc { alternative_equation_id: entry.id.clone() },
                    ModelComparisonMethod::LikelihoodRatioTest => {
                        ModelComparisonSpec::LikelihoodRatioTest { alternative_equation_id: entry.id.clone() }
                    }
                };
                let mut next = spec.clone();
                if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                    fit.comparison = Some(updated);
                    fit.comparison_method = Some(active_method);
                    if matches!(fit.comparison_question, Some(ModelComparisonQuestion::AlternativeEquation { .. })) {
                        fit.comparison_question = Some(ModelComparisonQuestion::AlternativeEquation { alternative_equation_id: entry.id.clone() });
                    }
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    }
}

fn render_fit_constrain_tab(
    ctx: &Context,
    ui: &mut Ui,
    tab_ctx: &FitTabCtx<'_>,
    state: &mut FitModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let FitTabCtx { result_sheet_id, doc, spec, nl, param_names, locale_preset } = *tab_ctx;
    render_section_title(ui, tokens, "Parameter Constraints, Sharing & Constant Hooks");
    let hook_target_key = Id::unique("fit_modal.inline_hook_param");
    let mut hook_target: Option<String> = ctx.data(|d| d.get_temp(hook_target_key)).flatten();

    for p in param_names {
        let existing_idx = nl.parameter_rules.iter().position(|r| r.name == *p);
        let rule = existing_idx.map_or_else(
            || ParameterRule { name: p.clone(), scope: ParameterScope::Local, initial_override: None, lower_bound: None, upper_bound: None },
            |i| nl.parameter_rules[i].clone(),
        );
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(p).strong().color(tokens.text_strong));
                for (slug, lbl, scope_eq) in [
                    ("local", "No constraint", matches!(rule.scope, ParameterScope::Local)),
                    ("shared", "Shared global", matches!(rule.scope, ParameterScope::SharedGlobal)),
                    ("fixed", "Constant", matches!(rule.scope, ParameterScope::Constant(_))),
                    ("col_title", "Col Title", matches!(rule.scope, ParameterScope::ColumnTitleConstant)),
                ] {
                    if render_modal_seg_btn(ui, tokens, meta, &format!("fit.con.rule.{p}.{slug}"), lbl, scope_eq, &ControlEnablement::enabled()) {
                        let new_scope = match slug {
                            "shared" => ParameterScope::SharedGlobal,
                            "fixed" => {
                                let cur = match rule.scope {
                                    ParameterScope::Constant(v) => v,
                                    _ => 1.0,
                                };
                                state.constant_drafts.insert(p.clone(), format_fit_number(cur));
                                ParameterScope::Constant(cur)
                            }
                            "col_title" => ParameterScope::ColumnTitleConstant,
                            _ => ParameterScope::Local,
                        };
                        update_param_rule(spec, p, |r| r.scope = new_scope, intents);
                    }
                }
                if render_modal_seg_btn(
                    ui,
                    tokens,
                    meta,
                    &format!("fit.con.hook.{p}"),
                    "🔗 Hook",
                    matches!(rule.scope, ParameterScope::HookedConstant(_)) || hook_target.as_deref() == Some(p.as_str()),
                    &ControlEnablement::enabled(),
                ) {
                    hook_target = if hook_target.as_deref() == Some(p.as_str()) { None } else { Some(p.clone()) };
                    ctx.data_mut(|d| d.insert_temp(hook_target_key, hook_target.clone()));
                }
            });

            if let ParameterScope::Constant(val) = rule.scope {
                let mut text = state.constant_drafts.get(p).cloned().unwrap_or_else(|| format_fit_number(val));
                ui.horizontal(|ui| {
                    ui.label("Constant value:");
                    let ed = ui.add_sized(Vec2::new(90.0, 20.0), TextEdit::singleline(&mut text).id(Id::unique(("fit_con_const", p.as_str()))));
                    record_widget(
                        ui,
                        meta,
                        &ed,
                        &format!("fit.con.const.{p}"),
                        "textbox",
                        &format!("{p} Constant Value"),
                        None,
                        Some(serde_json::json!(text)),
                        true,
                        None,
                        ZLayer::ModalDialog,
                        false,
                        None,
                    );
                    if ed.changed() {
                        state.constant_drafts.insert(p.clone(), text.clone());
                        if let Some(parsed) = parse_fit_number(&text, locale_preset) {
                            update_param_rule(spec, p, |r| r.scope = ParameterScope::Constant(parsed), intents);
                        }
                    } else if !state.constant_drafts.contains_key(p) {
                        state.constant_drafts.insert(p.clone(), text);
                    }
                });
            } else if let ParameterScope::HookedConstant(ref src) = rule.scope {
                let link_lbl = format!("Linked hook: {}", format_hook_source_short(src));
                let tip_summary = format!("Open constant hook browser for `{p}`");
                let link_wid = format!("fit.con.linked_hook.{p}");
                let link_resp = link_button(
                    ui,
                    tokens,
                    meta,
                    LinkButtonSpec {
                        font_size: tokens.metrics.font_ui_small_pt,
                        value: Some(serde_json::json!({ "parameter": p.as_str() })),
                        tooltip: Some(TooltipSpec::new("Change Linked Constant").with_summary(&tip_summary)),
                        tooltip_mode: TooltipMode::ModalAlwaysOpen,
                        pad_x: 2.0,
                        ..LinkButtonSpec::new(&link_wid, &link_lbl, ZLayer::ModalDialog)
                    },
                );
                if link_resp.clicked() {
                    hook_target = Some(p.clone());
                    ctx.data_mut(|d| d.insert_temp(hook_target_key, hook_target.clone()));
                }
            }

            render_param_bound_inputs(ui, tab_ctx, p, &rule, state, meta, intents);
        });
    }

    if let Some(target_param) = hook_target {
        ui.add_space(6.0);
        egui::Frame::NONE
            .fill(tokens.panel_bg)
            .stroke(Stroke::new(1.2, tokens.accent))
            .corner_radius(egui::CornerRadius::same(tokens.metrics.radius_md))
            .inner_margin(Vec2::new(8.0, 6.0))
            .show(ui, |ui| {
                let mut picker = ConstantHookPickerState::default();
                picker.open_for_document(result_sheet_id.clone(), Some(target_param.clone()), doc);
                render_section_title(ui, tokens, &format!("Link Constant to `{target_param}`"));
                egui::ScrollArea::vertical().id_salt("fit.con.hook_opt.scroll").max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
                    for opt in &picker.options {
                        if render_modal_seg_btn(
                            ui,
                            tokens,
                            meta,
                            &format!("fit.con.hook_opt.{}.{}", target_param, opt.label),
                            &format!("{} ({})", opt.label, opt.preview_value),
                            false,
                            &ControlEnablement::enabled(),
                        ) {
                            let source = opt.source.clone();
                            update_param_rule(spec, &target_param, move |r| r.scope = ParameterScope::HookedConstant(source), intents);
                            ctx.data_mut(|d| d.insert_temp::<Option<String>>(hook_target_key, None));
                        }
                    }
                });
            });
    }
}

fn render_param_bound_inputs(
    ui: &mut Ui,
    tab_ctx: &FitTabCtx<'_>,
    param: &str,
    rule: &ParameterRule,
    state: &mut FitModalEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let FitTabCtx { spec, locale_preset, .. } = *tab_ctx;
    let lo_draft =
        state.lower_bound_drafts.entry(param.to_owned()).or_insert_with(|| rule.lower_bound.map(format_fit_number).unwrap_or_default()).clone();
    let hi_draft =
        state.upper_bound_drafts.entry(param.to_owned()).or_insert_with(|| rule.upper_bound.map(format_fit_number).unwrap_or_default()).clone();
    let mut lo_text = lo_draft;
    let mut hi_text = hi_draft;

    ui.horizontal_wrapped(|ui| {
        ui.label("Lower bound:");
        let lo_resp =
            ui.add_sized(Vec2::new(96.0, 20.0), TextEdit::singleline(&mut lo_text).id(Id::unique(("fit_con_lower", param))).hint_text("None"));
        record_widget(
            ui,
            meta,
            &lo_resp,
            &format!("fit.con.lower.{param}"),
            "textbox",
            &format!("{param} Lower Bound"),
            None,
            Some(serde_json::json!(lo_text)),
            true,
            None,
            ZLayer::ModalDialog,
            false,
            None,
        );
        if lo_resp.changed() {
            if lo_text.trim().is_empty() {
                update_param_rule(spec, param, |r| r.lower_bound = None, intents);
            } else if let Some(val) = parse_fit_number(&lo_text, locale_preset) {
                update_param_rule(spec, param, |r| r.lower_bound = Some(val), intents);
            }
        }

        ui.label("Upper bound:");
        let hi_resp =
            ui.add_sized(Vec2::new(96.0, 20.0), TextEdit::singleline(&mut hi_text).id(Id::unique(("fit_con_upper", param))).hint_text("None"));
        record_widget(
            ui,
            meta,
            &hi_resp,
            &format!("fit.con.upper.{param}"),
            "textbox",
            &format!("{param} Upper Bound"),
            None,
            Some(serde_json::json!(hi_text)),
            true,
            None,
            ZLayer::ModalDialog,
            false,
            None,
        );
        if hi_resp.changed() {
            if hi_text.trim().is_empty() {
                update_param_rule(spec, param, |r| r.upper_bound = None, intents);
            } else if let Some(val) = parse_fit_number(&hi_text, locale_preset) {
                update_param_rule(spec, param, |r| r.upper_bound = Some(val), intents);
            }
        }
    });

    state.lower_bound_drafts.insert(param.to_owned(), lo_text);
    state.upper_bound_drafts.insert(param.to_owned(), hi_text);
}

fn render_fit_initial_tab(
    ui: &mut Ui,
    tab_ctx: &FitTabCtx<'_>,
    state: &mut FitModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let FitTabCtx { doc, spec, nl, param_names, locale_preset, .. } = *tab_ctx;
    render_section_title(ui, tokens, "Initial Parameter Guesses");
    let data_points = extract_fit_data_points(doc, spec, nl);
    let has_data = !data_points.is_empty();
    let heuristic_vals = resolve_model_from_spec(nl).ok().map(|m| m.initial_guess(&data_points)).unwrap_or_default();

    for (idx, p) in param_names.iter().enumerate() {
        let rule = nl.parameter_rules.iter().find(|r| r.name == *p);
        let override_val = rule.and_then(|r| r.initial_override);
        let auto_guess = heuristic_vals.get(idx).copied().unwrap_or(1.0);
        let is_auto = override_val.is_none();
        let auto_str = format_fit_number(auto_guess);
        let auto_btn_lbl = if has_data { format!("Auto ({auto_str})") } else { format!("Model default ({auto_str})") };
        let mut init_text =
            state.initial_override_drafts.entry(p.clone()).or_insert_with(|| override_val.map(format_fit_number).unwrap_or_default()).clone();

        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(p).strong());
            if render_modal_seg_btn(ui, tokens, meta, &format!("fit.init.auto.{p}"), &auto_btn_lbl, is_auto, &ControlEnablement::enabled()) {
                init_text.clear();
                update_param_rule(spec, p, |r| r.initial_override = None, intents);
            }

            let input_resp = ui.add_sized(
                Vec2::new(108.0, 22.0),
                TextEdit::singleline(&mut init_text).id(Id::unique(("fit_init_input", p.as_str()))).hint_text(&auto_str),
            );
            record_widget(
                ui,
                meta,
                &input_resp,
                &format!("fit.init.input.{p}"),
                "textbox",
                &format!("{p} Initial Value"),
                None,
                Some(serde_json::json!(init_text)),
                true,
                None,
                ZLayer::ModalDialog,
                false,
                None,
            );
            if input_resp.changed() {
                if init_text.trim().is_empty() {
                    update_param_rule(spec, p, |r| r.initial_override = None, intents);
                } else if let Some(val) = parse_fit_number(&init_text, locale_preset) {
                    update_param_rule(spec, p, |r| r.initial_override = Some(val), intents);
                }
            }

            let cur = override_val.unwrap_or(auto_guess);
            let cur_text = format_fit_number(cur);
            let step = render_stepper_row(
                ui,
                tokens,
                meta,
                StepperSpec {
                    id_prefix: &format!("fit.init.val.{p}"),
                    label: "Step",
                    value_text: &cur_text,
                    enablement: &ControlEnablement::enabled(),
                },
            );
            if step != 0 {
                let next_v = cur + f64::from(step) * 0.5;
                init_text = format_fit_number(next_v);
                update_param_rule(spec, p, |r| r.initial_override = Some(next_v), intents);
            }
        });
        state.initial_override_drafts.insert(p.clone(), init_text);
    }
}

fn render_fit_range_tab(
    ui: &mut Ui,
    tab_ctx: &FitTabCtx<'_>,
    state: &mut FitModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let FitTabCtx { doc, spec, nl, locale_preset, .. } = *tab_ctx;
    render_section_title(ui, tokens, "Fit X Domain & Curve Plot Window");
    let has_x_filter = nl.x_range_filter.is_some();
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "fit.wr.x_range_enable",
        "Restrict fit to X domain filter",
        has_x_filter,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            if has_x_filter {
                fit.x_range_filter = None;
                state.x_min_draft.clear();
                state.x_max_draft.clear();
            } else {
                let (seed_min, seed_max) = seed_x_domain_bounds(doc, spec, nl);
                state.x_min_draft = format_fit_number(seed_min);
                state.x_max_draft = format_fit_number(seed_max);
                fit.x_range_filter = Some((Some(seed_min), Some(seed_max)));
            }
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if let Some((cur_lo, cur_hi)) = nl.x_range_filter {
        ui.horizontal_wrapped(|ui| {
            ui.label("X min:");
            let min_resp =
                ui.add_sized(Vec2::new(100.0, 22.0), TextEdit::singleline(&mut state.x_min_draft).id(Id::unique("fit.wr.x_min")).hint_text("Min X"));
            record_widget(
                ui,
                meta,
                &min_resp,
                "fit.wr.x_min",
                "textbox",
                "X min",
                None,
                Some(serde_json::json!(state.x_min_draft)),
                true,
                None,
                ZLayer::ModalDialog,
                false,
                None,
            );

            ui.label("X max:");
            let max_resp =
                ui.add_sized(Vec2::new(100.0, 22.0), TextEdit::singleline(&mut state.x_max_draft).id(Id::unique("fit.wr.x_max")).hint_text("Max X"));
            record_widget(
                ui,
                meta,
                &max_resp,
                "fit.wr.x_max",
                "textbox",
                "X max",
                None,
                Some(serde_json::json!(state.x_max_draft)),
                true,
                None,
                ZLayer::ModalDialog,
                false,
                None,
            );

            if min_resp.changed() || max_resp.changed() {
                let next_lo =
                    if state.x_min_draft.trim().is_empty() { Some(None) } else { parse_fit_number(&state.x_min_draft, locale_preset).map(Some) };
                let next_hi =
                    if state.x_max_draft.trim().is_empty() { Some(None) } else { parse_fit_number(&state.x_max_draft, locale_preset).map(Some) };
                let lo_val = next_lo.unwrap_or(cur_lo);
                let hi_val = next_hi.unwrap_or(cur_hi);
                let mut next = spec.clone();
                if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                    fit.x_range_filter = Some((lo_val, hi_val));
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "fit.wr.plot_only",
        "Simulate curve using initial/fixed parameters only (no fit)",
        nl.plot_function_only,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.plot_function_only = !nl.plot_function_only;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    let step = render_stepper_row(
        ui,
        tokens,
        meta,
        StepperSpec {
            id_prefix: "fit.wr.max_iters",
            label: "Max Levenberg-Marquardt Iterations",
            value_text: &nl.max_iters.to_string(),
            enablement: &ControlEnablement::enabled(),
        },
    );
    if step != 0 {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.max_iters = (fit.max_iters as isize + step as isize * 100).clamp(50, NonlinearFitSpec::HARD_MAX_ITERS as isize) as usize;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
}

fn render_fit_weight_tab(
    ui: &mut Ui,
    tab_ctx: &FitTabCtx<'_>,
    locale: &str,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let spec = tab_ctx.spec;
    let nl = tab_ctx.nl;
    render_section_title(ui, tokens, "Fitting Objective / Outlier Method");
    for (wid, lbl, active, method) in [
        ("fit.method.ls", "Ordinary / Weighted Least Squares", matches!(nl.method, FitMethod::LeastSquares), FitMethod::LeastSquares),
        ("fit.method.poisson", "Poisson Count Regression (Deviance)", matches!(nl.method, FitMethod::PoissonDeviance), FitMethod::PoissonDeviance),
        ("fit.method.robust", "Robust Regression (Cauchy M-estimator)", matches!(nl.method, FitMethod::RobustCauchy), FitMethod::RobustCauchy),
        (
            "fit.method.rout",
            "ROUT Outlier Detection + Refit",
            matches!(nl.method, FitMethod::RoutOutlierElimination { .. }),
            FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 },
        ),
    ] {
        if render_modal_radio(ui, tokens, meta, wid, lbl, active, &ControlEnablement::enabled()) {
            let mut next = spec.clone();
            if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                fit.method = method;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }

    ui.add_space(6.0);
    render_section_title(ui, tokens, tr(locale, "fit.outlier.title"));
    ui.horizontal_wrapped(|ui| {
        for (wid, key, mode) in [
            ("fit.outlier.none", "fit.outlier.none", OutlierHandling::None),
            ("fit.outlier.count_only", "fit.outlier.count_only", OutlierHandling::IdentifyAndCountOnly),
            ("fit.outlier.eliminate", "fit.outlier.eliminate", OutlierHandling::Eliminate),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, tr(locale, key), nl.outlier_handling == mode, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                    fit.outlier_handling = mode;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    });
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "fit.outlier.cleaned_tab",
        tr(locale, "fit.outlier.cleaned_tab"),
        nl.emit_cleaned_data_tab,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.emit_cleaned_data_tab = !nl.emit_cleaned_data_tab;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }

    ui.add_space(6.0);
    render_section_title(ui, tokens, "Residual Weighting Scheme");
    let wt_en = if matches!(nl.method, FitMethod::PoissonDeviance) {
        ControlEnablement::disabled("Poisson deviance likelihood fixes intrinsic variance weights.")
    } else {
        ControlEnablement::enabled()
    };
    for (wid, lbl, w) in [
        ("fit.wt.none", "No weighting (equal SD everywhere)", WeightingScheme::None),
        ("fit.wt.rel_sq", "Relative 1/Y^2 (constant %CV)", WeightingScheme::RelativeSquare),
        ("fit.wt.rel", "Poisson-like 1/|Y|", WeightingScheme::Relative),
        ("fit.wt.inv_x2", "1/X^2", WeightingScheme::InverseX2),
        ("fit.wt.obs_sd", "By 1/SD^2 of Y replicates", WeightingScheme::ObservedSd),
    ] {
        if render_modal_radio(ui, tokens, meta, wid, lbl, nl.weighting == w, &wt_en) {
            let mut next = spec.clone();
            if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                fit.weighting = w;
            }
            intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
        }
    }

    let y_wt_en = if matches!(nl.weighting, WeightingScheme::RelativeSquare | WeightingScheme::Relative | WeightingScheme::GeneralPowerY(_))
        && !matches!(nl.method, FitMethod::PoissonDeviance)
    {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Applies to Y-dependent weighting schemes (1/Y^2, 1/|Y|, 1/|Y|^k).")
    };
    ui.add_space(4.0);
    render_section_title(ui, tokens, tr(locale, "fit.wt.src.title"));
    ui.horizontal_wrapped(|ui| {
        for (wid, key, src) in [
            ("fit.wt.src.observed", "fit.wt.src.observed", WeightingSource::ObservedY),
            ("fit.wt.src.predicted", "fit.wt.src.predicted", WeightingSource::PredictedY),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, tr(locale, key), nl.weighting_source == src, &y_wt_en) {
                let mut next = spec.clone();
                if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                    fit.weighting_source = src;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    });

    ui.add_space(6.0);
    render_section_title(ui, tokens, tr(locale, "fit.rep.title"));
    ui.horizontal_wrapped(|ui| {
        for (wid, key, mode) in [
            ("fit.rep.individual", "fit.rep.individual", ReplicateFitMode::IndividualReplicates),
            ("fit.rep.row_means", "fit.rep.row_means", ReplicateFitMode::RowMeansOnly),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, tr(locale, key), nl.replicate_fit_mode == mode, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                    fit.replicate_fit_mode = mode;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    });

    ui.add_space(6.0);
    render_section_title(ui, tokens, tr(locale, "fit.conv.title"));
    ui.horizontal_wrapped(|ui| {
        for (wid, key, strictness) in [
            ("fit.conv.default", "fit.conv.default", ConvergenceStrictness::Default),
            ("fit.conv.quick", "fit.conv.quick", ConvergenceStrictness::Quick),
            ("fit.conv.medium", "fit.conv.medium", ConvergenceStrictness::Medium),
            ("fit.conv.strict", "fit.conv.strict", ConvergenceStrictness::Strict),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, tr(locale, key), nl.convergence_strictness == strictness, &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                    fit.convergence_strictness = strictness;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn render_fit_output_tab(
    ui: &mut Ui,
    spec: &curvebook_doc::AnalysisSpec,
    nl: &NonlinearFitSpec,
    locale: &str,
    state: &mut FitModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, tokens, "Confidence Intervals & Curve Bands");
    if render_modal_radio(
        ui,
        tokens,
        meta,
        "fit.ci.profile",
        "Profile-likelihood asymmetrical CIs (recommended)",
        nl.profile_likelihood_ci,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.profile_likelihood_ci = true;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_radio(ui, tokens, meta, "fit.ci.wald", "Symmetric Wald-t intervals", !nl.profile_likelihood_ci, &ControlEnablement::enabled()) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.profile_likelihood_ci = false;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }

    let conf_pct = spec.confidence_level.percent();
    ui.horizontal_wrapped(|ui| {
        for (wid, pct, lbl) in [("fit.conf.90", 90.0_f64, "90% CI"), ("fit.conf.95", 95.0, "95% CI"), ("fit.conf.99", 99.0, "99% CI")] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, (conf_pct - pct).abs() < 0.05, &ControlEnablement::enabled())
                && let Ok(level) = curvebook_doc::ConfidenceLevel::from_percent(pct)
            {
                intents.push(UiIntent::SetEditParamsConfidence { level });
            }
        }
    });

    ui.add_space(6.0);
    render_section_title(ui, tokens, "Diagnostics, Interpolation & Output Tabs");
    if render_modal_checkbox(ui, tokens, meta, "fit.out.conf_bands", "Plot 95% confidence bands", nl.confidence_bands, &ControlEnablement::enabled())
    {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.confidence_bands = !nl.confidence_bands;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(ui, tokens, meta, "fit.out.pred_bands", "Plot 95% prediction bands", nl.prediction_bands, &ControlEnablement::enabled())
    {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.prediction_bands = !nl.prediction_bands;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "fit.out.interp",
        "Interpolate unknowns from standard curve",
        nl.interpolate_unknowns,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.interpolate_unknowns = !nl.interpolate_unknowns;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    let unstable_checked = nl.effective_identifiability_mode() == Some(IdentifiabilityMode::IdentifyUnstableParameters);
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "fit.out.unstable",
        "Identify unstable / ambiguous parameters",
        unstable_checked,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            let next_val = !unstable_checked;
            fit.identify_unstable_parameters = next_val;
            fit.identifiability_mode = if next_val {
                Some(IdentifiabilityMode::IdentifyUnstableParameters)
            } else if fit.identifiability_mode == Some(IdentifiabilityMode::IdentifyUnstableParameters) {
                None
            } else {
                fit.identifiability_mode
            };
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "fit.out.normality",
        "Test normality of residuals + Runs test",
        nl.normality_of_residuals,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.normality_of_residuals = !nl.normality_of_residuals;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "fit.out.residuals_tab",
        "Emit Residuals diagnostics tab",
        nl.emit_residuals_tab,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.emit_residuals_tab = !nl.emit_residuals_tab;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "fit.out.cov_tab",
        "Emit Parameter Covariance Matrix tab",
        nl.emit_covariance_tab,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.emit_covariance_tab = !nl.emit_covariance_tab;
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    render_fit_identifiability_and_flags_controls(ui, spec, nl, locale, state, tokens, meta, intents);

    ui.add_space(6.0);
    render_section_title(ui, tokens, "P-Value Display Formatting");
    ui.horizontal_wrapped(|ui| {
        for (wid, slug, lbl, active) in [
            ("fit.out.pval.four_decimal", "four_decimal", "0.0001", matches!(spec.p_value_format, PValueFormat::FourDecimal)),
            ("fit.out.pval.apa", "apa", "APA", matches!(spec.p_value_format, PValueFormat::Apa)),
            ("fit.out.pval.nejm", "nejm", "NEJM", matches!(spec.p_value_format, PValueFormat::Nejm)),
            ("fit.out.pval.custom", "custom", "Custom", matches!(spec.p_value_format, PValueFormat::Custom { .. })),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, active, &ControlEnablement::enabled()) {
                intents.push(UiIntent::StatParams(super::StatParamsIntent::SetPValueFormat { slug: slug.to_owned() }));
            }
        }
    });
    let digits = match spec.p_value_format {
        PValueFormat::FourDecimal => 4,
        PValueFormat::Apa | PValueFormat::Nejm => 3,
        PValueFormat::Custom { decimals, .. } => decimals,
    };
    let r9_en = r9_pvalue_precision_spinner(&spec.p_value_format);
    let step = render_stepper_row(
        ui,
        tokens,
        meta,
        StepperSpec {
            id_prefix: "fit.out.pvalue.digits",
            label: "Custom P-Value Decimal Digits",
            value_text: &digits.to_string(),
            enablement: &r9_en,
        },
    );
    if step < 0 {
        intents.push(UiIntent::StatParams(super::StatParamsIntent::StepPValueDigits { delta: -1 }));
    } else if step > 0 {
        intents.push(UiIntent::StatParams(super::StatParamsIntent::StepPValueDigits { delta: 1 }));
    }
}

#[allow(clippy::too_many_arguments)]
fn render_fit_identifiability_and_flags_controls(
    ui: &mut Ui,
    spec: &curvebook_doc::AnalysisSpec,
    nl: &NonlinearFitSpec,
    locale: &str,
    state: &mut FitModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.add_space(4.0);
    render_modal_textbox_row(
        ui,
        tokens,
        meta,
        intents,
        "fit.diagnostics.r2.input",
        "Flag R² Below Threshold (0..1)",
        "e.g. 0.90",
        &mut state.r2_threshold_draft,
        96.0,
    );
    ui.add_space(4.0);
    render_section_title(ui, tokens, tr(locale, "fit.ident.heading"));
    let eff_ident = nl.effective_identifiability_mode();
    ui.horizontal_wrapped(|ui| {
        for (wid, lbl, mode) in [
            ("fit.ident.unstable", tr(locale, "fit.ident.unstable"), IdentifiabilityMode::IdentifyUnstableParameters),
            ("fit.ident.ambiguous", tr(locale, "fit.ident.ambiguous"), IdentifiabilityMode::IdentifyAmbiguousFits),
            ("fit.ident.report_all", tr(locale, "fit.ident.report_all"), IdentifiabilityMode::ReportAll),
        ] {
            if render_modal_seg_btn(ui, tokens, meta, wid, lbl, eff_ident == Some(mode), &ControlEnablement::enabled()) {
                let mut next = spec.clone();
                if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                    fit.identifiability_mode = Some(mode);
                    fit.identify_unstable_parameters = mode == IdentifiabilityMode::IdentifyUnstableParameters;
                }
                intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
            }
        }
    });
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "fit.out.show_se",
        tr(locale, "fit.out.show_se"),
        nl.should_show_standard_errors(),
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.show_standard_errors = Some(!nl.should_show_standard_errors());
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    let flags_enabled = nl.fit_flag_rules.as_ref().is_some_and(FitFlagRules::has_any_rule);
    if render_modal_checkbox(ui, tokens, meta, "fit.flags.enable", tr(locale, "fit.flags.enable"), flags_enabled, &ControlEnablement::enabled()) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.fit_flag_rules = if flags_enabled {
                None
            } else {
                Some(FitFlagRules {
                    min_r_squared: fit.flag_r_squared_below.or(Some(0.8)),
                    min_observations: Some(4),
                    max_dependency: Some(0.999),
                    min_normality_p: Some(0.05),
                    min_runs_or_lof_p: Some(0.05),
                    max_rout_outliers: Some(3),
                    exclude_flagged_from_summary: false,
                })
            };
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if render_modal_checkbox(
        ui,
        tokens,
        meta,
        "fit.out.create_summary_graph",
        tr(locale, "fit.out.create_summary_graph"),
        nl.create_summary_graph,
        &ControlEnablement::enabled(),
    ) {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            fit.create_summary_graph = !nl.create_summary_graph;
            sync_summary_parameter_for_fit(fit);
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
    if nl.create_summary_graph {
        let summary_params = resolve_parameter_names(nl);
        if !summary_params.is_empty() {
            let active_summary_param = nl
                .summary_parameter
                .as_deref()
                .filter(|p| summary_params.iter().any(|cand| cand == p))
                .or_else(|| summary_params.first().map(String::as_str));
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("Summary Parameter:").small().color(tokens.text_muted));
                for param in &summary_params {
                    let selected = active_summary_param == Some(param.as_str());
                    if render_modal_seg_btn(
                        ui,
                        tokens,
                        meta,
                        &format!("fit.out.summary_param.{param}"),
                        param,
                        selected,
                        &ControlEnablement::enabled(),
                    ) {
                        let mut next = spec.clone();
                        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
                            fit.summary_parameter = Some(param.clone());
                        }
                        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
                    }
                }
            });
        }
    }
    let sig_display = nl.output_significant_digits.map_or_else(|| String::from("Auto"), |d| d.to_string());
    let sig_step = render_stepper_row(
        ui,
        tokens,
        meta,
        StepperSpec {
            id_prefix: "fit.out.sig_figs",
            label: tr(locale, "fit.out.sig_figs"),
            value_text: &sig_display,
            enablement: &ControlEnablement::enabled(),
        },
    );
    if sig_step != 0 {
        let mut next = spec.clone();
        if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
            let cur = fit.output_significant_digits.unwrap_or(4) as i16;
            let updated = (cur + sig_step as i16).clamp(1, 15) as u8;
            fit.output_significant_digits = Some(updated);
        }
        intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
    }
}

fn update_param_rule<F: FnOnce(&mut ParameterRule)>(spec: &curvebook_doc::AnalysisSpec, param_name: &str, mutate: F, intents: &mut PendingUiIntents) {
    let mut next = spec.clone();
    if let AnalysisKind::NonlinearRegression(fit) = &mut next.kind {
        if let Some(existing) = fit.parameter_rules.iter_mut().find(|r| r.name == param_name) {
            mutate(existing);
        } else {
            let mut created = ParameterRule {
                name: param_name.to_owned(),
                scope: ParameterScope::Local,
                initial_override: None,
                lower_bound: None,
                upper_bound: None,
            };
            mutate(&mut created);
            fit.parameter_rules.push(created);
        }
    }
    intents.push(UiIntent::SetEditParamsDraft { spec: Box::new(next) });
}

// ============================================================================
// Hook Constant 3-Branch Browser (§8.9)
// ============================================================================

/// Render the 3-branch `Hook Constant` browser modal when `ActiveDialog::ConstantHookPicker` is active.
pub fn render_hook_constant_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut FitModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> FitModalFrameReport {
    let ActiveDialog::ConstantHookPicker { .. } = &app.canvas_dialogs.active_dialog else {
        ctx.data_mut(|d| d.remove::<bool>(Id::unique("hook_modal.init_branches")));
        return FitModalFrameReport::default();
    };
    let hp = &app.canvas_dialogs.hook_picker;
    let init_key = Id::unique("hook_modal.init_branches");
    if !ctx.data(|d| d.get_temp::<bool>(init_key)).unwrap_or(false) {
        ctx.data_mut(|d| d.insert_temp(init_key, true));
        state.expand_info_branch = true;
        state.expand_analysis_branch = true;
        state.expand_document_branch = true;
        state.hook_search_draft = hp.search_query.clone();
    }

    let locale = normalize_locale(&app.locale);
    let target_desc =
        hp.target_parameter_name.as_deref().map_or_else(|| String::from("Link constant to target"), |p| format!("Binding parameter `{p}`"));
    let link_en = hp
        .selected_option()
        .map(|_| ControlEnablement::enabled())
        .unwrap_or_else(|| ControlEnablement::disabled("Select a constant from one of the three branches."));
    let wants_kb_before = ctx.egui_wants_keyboard_input();

    let capture = show_wave4_modal(
        ctx,
        tokens,
        meta,
        intents,
        state,
        Wave4ModalShell {
            widget_id: "dialog.constant_hook_picker",
            title: tr(locale, "w6.view.modal.hook_constant"),
            subtitle_or_banner: Some(&target_desc),
            banner_widget_id: "hook.target_banner",
            max_width_pt: 640.0,
            max_height_pt: 520.0,
            use_body_scroll: true,
            footer_reserve_pt: 40.0,
        },
        |ui, state, meta, intents| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let hook_search_id = Id::unique("hook.search_input");
                if !ui.ctx().memory(|m| m.has_focus(hook_search_id)) && state.hook_search_draft != hp.search_query {
                    state.hook_search_draft = hp.search_query.clone();
                }
                let s_resp = ui.add_sized(
                    Vec2::new((ui.available_width() - 64.0).max(120.0), 22.0),
                    TextEdit::singleline(&mut state.hook_search_draft)
                        .id(hook_search_id)
                        .hint_text("Search Info Constants, Fit Parameters, Result Summaries"),
                );
                record_widget(
                    ui,
                    meta,
                    &s_resp,
                    "hook.search_input",
                    "textbox",
                    "Search Hooks",
                    None,
                    Some(serde_json::json!(state.hook_search_draft)),
                    true,
                    None,
                    ZLayer::ModalDialog,
                    false,
                    None,
                );
                if s_resp.changed() {
                    intents.push(UiIntent::SetHookPickerSearch { query: state.hook_search_draft.clone() });
                }
                if render_modal_seg_btn(ui, tokens, meta, "hook.search_clear", "Clear", false, &ControlEnablement::enabled()) {
                    state.hook_search_draft.clear();
                    intents.push(UiIntent::ClearHookPickerSearch);
                }
            });
        },
        |ui, state, _w, _h, meta, intents| {
            let filtered = hp.filtered_options();
            let mut info_opts = Vec::new();
            let mut analysis_opts = Vec::new();
            let mut doc_opts = Vec::new();
            for opt in filtered {
                if let Some(raw_idx) = hp.options.iter().position(|o| o == opt) {
                    match &opt.source {
                        HookSource::InfoConstant { .. } => info_opts.push((raw_idx, opt)),
                        HookSource::ResultScalar { .. } => analysis_opts.push((raw_idx, opt)),
                        HookSource::ColumnTitleValue { .. } => doc_opts.push((raw_idx, opt)),
                    }
                }
            }

            if render_modal_seg_btn(
                ui,
                tokens,
                meta,
                "hook.branch.info",
                &format!("{} Info Constants ({})", if state.expand_info_branch { "▾" } else { "▸" }, info_opts.len()),
                state.expand_info_branch,
                &ControlEnablement::enabled(),
            ) {
                state.expand_info_branch = !state.expand_info_branch;
            }
            if state.expand_info_branch {
                for (idx, opt) in info_opts {
                    if render_modal_radio(
                        ui,
                        tokens,
                        meta,
                        &format!("hook.item.{idx}"),
                        &format!("{} = {}", opt.label, opt.preview_value),
                        hp.selected_index == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        intents.push(UiIntent::SelectHookConstant { index: Some(idx) });
                    }
                }
            }

            ui.add_space(4.0);
            if render_modal_seg_btn(
                ui,
                tokens,
                meta,
                "hook.branch.analysis",
                &format!("{} Fit Parameters & Analysis Results ({})", if state.expand_analysis_branch { "▾" } else { "▸" }, analysis_opts.len()),
                state.expand_analysis_branch,
                &ControlEnablement::enabled(),
            ) {
                state.expand_analysis_branch = !state.expand_analysis_branch;
            }
            if state.expand_analysis_branch {
                for (idx, opt) in analysis_opts {
                    if render_modal_radio(
                        ui,
                        tokens,
                        meta,
                        &format!("hook.item.{idx}"),
                        &format!("{} = {}", opt.label, opt.preview_value),
                        hp.selected_index == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        intents.push(UiIntent::SelectHookConstant { index: Some(idx) });
                    }
                }
            }

            ui.add_space(4.0);
            if render_modal_seg_btn(
                ui,
                tokens,
                meta,
                "hook.branch.document",
                &format!("{} Result Summaries & Column Titles ({})", if state.expand_document_branch { "▾" } else { "▸" }, doc_opts.len()),
                state.expand_document_branch,
                &ControlEnablement::enabled(),
            ) {
                state.expand_document_branch = !state.expand_document_branch;
            }
            if state.expand_document_branch {
                for (idx, opt) in doc_opts {
                    if render_modal_radio(
                        ui,
                        tokens,
                        meta,
                        &format!("hook.item.{idx}"),
                        &format!("{} = {}", opt.label, opt.preview_value),
                        hp.selected_index == Some(idx),
                        &ControlEnablement::enabled(),
                    ) {
                        intents.push(UiIntent::SelectHookConstant { index: Some(idx) });
                    }
                }
            }

            if let Some(sel) = hp.selected_option() {
                ui.add_space(8.0);
                egui::Frame::NONE
                    .fill(tokens.panel_bg)
                    .stroke(Stroke::new(1.0, tokens.accent))
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(Vec2::new(8.0, 5.0))
                    .show(ui, |ui| {
                        render_clipped_text(
                            ui,
                            meta,
                            ClippedTextSpec {
                                widget_id: "hook.preview",
                                role: "status",
                                text: &format!("Selected: {}  →  {}", sel.label, sel.preview_value),
                                font_size: 11.5,
                                color: tokens.text_muted,
                                z_layer: ZLayer::ModalDialog,
                                tooltip: None,
                                record_meta: true,
                            },
                        );
                    });
            }
        },
        |ui, _state, meta, intents| {
            ui.horizontal(|ui| {
                if render_modal_action_button(ui, tokens, meta, "hook.unlink", "Unlink", false, false, &ControlEnablement::enabled()) {
                    intents.push(UiIntent::UnlinkHookConstant);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if render_modal_action_button(ui, tokens, meta, "modal.ok", "Link Constant", true, false, &link_en) {
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
    } else if !wants_kb_before && !ctx.egui_wants_keyboard_input() && link_en.enabled && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
    {
        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    }

    FitModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

const ALL_EQ_CATEGORIES: [EquationCategory; 14] = [
    EquationCategory::DoseResponseStimulation,
    EquationCategory::DoseResponseInhibition,
    EquationCategory::ReceptorBinding,
    EquationCategory::EnzymeKinetics,
    EquationCategory::Exponential,
    EquationCategory::Polynomial,
    EquationCategory::Gaussian,
    EquationCategory::SineWave,
    EquationCategory::Growth,
    EquationCategory::Peak,
    EquationCategory::StandardCurve,
    EquationCategory::Radiobiology,
    EquationCategory::NistBenchmark,
    EquationCategory::UserDefined,
];

fn eq_category_label(cat: EquationCategory) -> &'static str {
    match cat {
        EquationCategory::DoseResponseStimulation => "Dose-Response (Stim)",
        EquationCategory::DoseResponseInhibition => "Dose-Response (Inhib)",
        EquationCategory::ReceptorBinding => "Binding",
        EquationCategory::EnzymeKinetics => "Enzyme Kinetics",
        EquationCategory::Exponential => "Exponential",
        EquationCategory::Polynomial => "Polynomial",
        EquationCategory::Gaussian => "Gaussian",
        EquationCategory::SineWave => "Sine / Wave",
        EquationCategory::Growth => "Growth",
        EquationCategory::Peak => "Peak",
        EquationCategory::StandardCurve => "Standard Curve",
        EquationCategory::Radiobiology => "Radiobiology",
        EquationCategory::NistBenchmark => "NIST StRD",
        EquationCategory::UserDefined => "User-Defined",
    }
}

fn format_hook_source_short(src: &HookSource) -> String {
    match src {
        HookSource::InfoConstant { info_sheet_id, constant_name } => {
            let sid = info_sheet_id.as_ref().map_or("active", SheetId::as_str);
            format!("Info({sid}:{constant_name})")
        }
        HookSource::ResultScalar { result_sheet_id, parameter_key, .. } => {
            format!("Result({}:{parameter_key})", result_sheet_id.as_str())
        }
        HookSource::ColumnTitleValue { data_sheet_id, data_set_index } => {
            format!("Title({}:col {data_set_index})", data_sheet_id.as_str())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FitModalEguiState, compute_fit_alerts, format_fit_number, parse_fit_number, seed_x_domain_bounds};
    use crate::i18n::LocalePreset;
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, DataSheet, NonlinearFitSpec, ParameterRule, ParameterScope, ProjectDocument, SheetId, SubcolumnMode,
        TableArchetype, XyTable,
    };

    #[test]
    fn format_fit_number_preserves_scientific_values() {
        assert_eq!(format_fit_number(0.0), "0");
        assert_eq!(format_fit_number(-0.0), "0");
        assert_eq!(format_fit_number(1e-7), "1e-7");
        assert_eq!(format_fit_number(2.5e-8), "2.5e-8");
        assert_eq!(format_fit_number(-9.0), "-9");
        assert_eq!(format_fit_number(-4.5), "-4.5");
        assert_eq!(format_fit_number(100.0), "100");
    }

    #[test]
    fn parse_fit_number_supports_scientific_and_comma_decimal_locales() {
        let us = LocalePreset::for_locale("en");
        let de = LocalePreset::for_locale("de");
        assert!((parse_fit_number("1e-7", &us).unwrap() - 1e-7).abs() < 1e-18);
        assert!((parse_fit_number("-9.5", &us).unwrap() - (-9.5)).abs() < 1e-12);
        assert!((parse_fit_number("-4,25", &de).unwrap() - (-4.25)).abs() < 1e-12);
        assert!(parse_fit_number("not-a-number", &us).is_none());
        assert!(parse_fit_number("", &us).is_none());
    }

    #[test]
    fn seed_x_domain_bounds_uses_actual_xy_data_range_and_falls_back_when_empty() {
        let mut doc = ProjectDocument::new("Fit Range Seed").unwrap();
        let xy_id = SheetId::try_new("xy-dose").unwrap();
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 6).unwrap();
        for (r, x) in [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0].into_iter().enumerate() {
            xy.x_cells[r][0] = Cell::number(x);
            *xy.data_sets[0].cell_mut(r, 0).unwrap() = Cell::number(10.0 + r as f64 * 15.0);
        }
        doc.data_sheets.push(DataSheet::new(xy_id.clone(), "Dose", TableArchetype::Xy(xy)).unwrap());

        let nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
        let spec = AnalysisSpec::new("fit-1", xy_id, AnalysisKind::NonlinearRegression(nl.clone())).unwrap();
        assert_eq!(seed_x_domain_bounds(&doc, &spec, &nl), (-9.0, -4.0));

        let empty_id = SheetId::try_new("xy-empty").unwrap();
        let empty_xy = XyTable::new(SubcolumnMode::Single, 1, 3).unwrap();
        doc.data_sheets.push(DataSheet::new(empty_id.clone(), "Empty", TableArchetype::Xy(empty_xy)).unwrap());
        let empty_spec = AnalysisSpec::new("fit-empty", empty_id, AnalysisKind::NonlinearRegression(nl.clone())).unwrap();
        assert_eq!(seed_x_domain_bounds(&doc, &empty_spec, &nl), (0.0, 100.0));
    }

    #[test]
    fn compute_fit_alerts_blocks_inverted_x_domain_and_inverted_bounds() {
        let us = LocalePreset::for_locale("en");
        let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
        nl.x_range_filter = Some((Some(-4.0), Some(-9.0)));
        let state = FitModalEguiState { x_min_draft: String::from("-4"), x_max_draft: String::from("-9"), ..Default::default() };
        let (alerts, blocking) = compute_fit_alerts(&nl, &[String::from("Bottom")], &state, &us);
        assert!(!alerts.is_empty());
        assert!(blocking.is_some());

        nl.x_range_filter = Some((Some(-9.0), Some(-4.0)));
        nl.parameter_rules.push(ParameterRule {
            name: String::from("Bottom"),
            scope: ParameterScope::Local,
            initial_override: Some(1e-7),
            lower_bound: Some(10.0),
            upper_bound: Some(5.0),
        });
        let valid_x_state = FitModalEguiState { x_min_draft: String::from("-9"), x_max_draft: String::from("-4"), ..Default::default() };
        let (alerts2, blocking2) = compute_fit_alerts(&nl, &[String::from("Bottom")], &valid_x_state, &us);
        assert!(alerts2.iter().any(|a| a.contains("Constraint conflict")));
        assert!(blocking2.is_some());
    }
}
