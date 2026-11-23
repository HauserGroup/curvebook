//! Standardized `Parameters: One-Way ANOVA` (`Cmd/Ctrl+T` / `EditAnalysisParameters`) modal
//! view in `egui 0.37` (`R15.2`, `gui-layout-spec.md` §8.8).
//!
//! Displays the live Dynamic Resolved-Test Banner, 5-tab keyboard-navigable parameter tabs with
//! hover disabled-reasons on blocked tabs, clean-room vector experimental-design schematics,
//! declarative `R1`–`R4`/`R9` predicates keeping disabled controls visible with unlock tooltips,
//! legacy `edit.*`/`modal.*` widget ID parity, all 14 `BETA_ACTIVE_LOCALES`, and `100%`
//! `ZLayer::ModalBackdrop` outside click/wheel capture.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, MixedModelDfMethod, MultiplicityMethod, OneWayAnovaSpec, OneWayMethod, PValueFormat, ProjectDocument,
    ResidualCovariance, SheetId, TableArchetype,
};
use egui::{Align2, Color32, Context, CornerRadius, FontId, Id, Key, Modifiers, Order, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, EditAnalysisParametersState};
use crate::i18n::{normalize_locale, tr};
use crate::inspect::ZLayer;
use crate::modal_predicates::{
    ControlEnablement, is_reml_mixed_effects_active, r1_paired_or_matched_allowed, r1_repeated_measures_tab, r1_sphericity_or_reml_control,
    r1b_post_hoc_option_enabled, r1b_sphericity_correction, r2_assume_gaussian_allowed, r3_control_column_picker, r9_pvalue_precision_spinner,
    source_table_shape,
};

use super::widgets::{dropdown, paint_icon_button_named};
use super::{FrameInspectEntryInput, FrameInspectMeta, FrameInspectNodeMeta, PendingUiIntents, Tokens};

/// Five parameter tabs defined for the standardized One-Way ANOVA modal (`gui-layout-spec.md` §8.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AnovaModalTab {
    #[default]
    ExperimentalDesign,
    AssumptionsAndTest,
    MultipleComparisons,
    Options,
    ResidualsAndPlots,
}

impl AnovaModalTab {
    /// Ordered tabs exposed in the keyboard-navigable tab strip.
    pub const ALL: [Self; 5] =
        [Self::ExperimentalDesign, Self::AssumptionsAndTest, Self::MultipleComparisons, Self::Options, Self::ResidualsAndPlots];

    /// Canonical lowercase tab slug matching `EditAnalysisParametersState::active_tab` / `stat_params`.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::ExperimentalDesign => "design",
            Self::AssumptionsAndTest => "assumptions",
            Self::MultipleComparisons => "post_hoc",
            Self::Options => "options",
            Self::ResidualsAndPlots => "residuals",
        }
    }

    /// Canonical `edit.tab.<id>` suffix shared with `inspect.rs` and `modal_predicates.rs`.
    #[must_use]
    pub const fn legacy_id(self) -> &'static str {
        match self {
            Self::ExperimentalDesign => "experimental_design",
            Self::AssumptionsAndTest => "repeated_measures",
            Self::MultipleComparisons => "multiple_comparisons",
            Self::Options => "options",
            Self::ResidualsAndPlots => "diagnostics",
        }
    }

    /// Parse a tab slug into [`AnovaModalTab`], defaulting unknown values to `None`.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug.trim().to_ascii_lowercase().as_str() {
            "design" | "experimental_design" => Some(Self::ExperimentalDesign),
            "assumptions" | "test" | "assumptions_and_test" | "repeated_measures" | "factor_settings" => Some(Self::AssumptionsAndTest),
            "post_hoc" | "multiple_comparisons" | "comparisons" => Some(Self::MultipleComparisons),
            "options" => Some(Self::Options),
            "residuals" | "plots" | "residuals_and_plots" | "diagnostics" | "diagnostics_residuals" => Some(Self::ResidualsAndPlots),
            _ => None,
        }
    }
}

/// Persistent transient UI state for [`render_one_way_anova_modal`].
#[derive(Debug, Clone, PartialEq)]
pub struct AnovaModalEguiState {
    pub active_tab: AnovaModalTab,
    pub custom_title_draft: String,
    pub confidence_percent_draft: f64,
    pub show_method_reference: bool,
    pub last_resolved_banner: String,
    pub method_saved_as_default: bool,
    pub saved_default_spec: Option<AnalysisSpec>,
    pub open_session_sheet_id: Option<String>,
}

impl Default for AnovaModalEguiState {
    fn default() -> Self {
        Self {
            active_tab: AnovaModalTab::ExperimentalDesign,
            custom_title_draft: String::new(),
            confidence_percent_draft: 95.0,
            show_method_reference: false,
            last_resolved_banner: String::new(),
            method_saved_as_default: false,
            saved_default_spec: None,
            open_session_sheet_id: None,
        }
    }
}

/// Frame geometry and modal capture report emitted by [`render_one_way_anova_modal`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AnovaModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
    pub resolved_test_banner: String,
}

/// Returns `true` when `ActiveDialog::EditAnalysisParameters` is open for a One-Way ANOVA analysis.
#[must_use]
pub fn is_one_way_anova_dialog_active(app: &UiApp) -> bool {
    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        return false;
    };
    if let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() {
        return matches!(spec.kind, AnalysisKind::OneWayAnova(_));
    }
    app.document().result_sheet(result_sheet_id).is_some_and(|rs| matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(_)))
}

fn post_hoc_banner_name(method: MultiplicityMethod) -> &'static str {
    match method {
        MultiplicityMethod::None => "No post-hoc correction",
        MultiplicityMethod::TukeyHsd => "Tukey",
        MultiplicityMethod::Dunnett { .. } => "Dunnett",
        MultiplicityMethod::Sidak => "Sidak",
        MultiplicityMethod::Bonferroni => "Bonferroni",
        MultiplicityMethod::HolmSidak => "Holm-Sidak",
        MultiplicityMethod::DunnettT3 => "Dunnett T3",
        MultiplicityMethod::GamesHowell => "Games-Howell",
        MultiplicityMethod::TamhaneT2 => "Tamhane T2",
        MultiplicityMethod::DunnRankSum => "Dunn",
        MultiplicityMethod::LinearTrend => "Linear Trend",
        MultiplicityMethod::BenjaminiHochbergFdr => "BH (FDR)",
        MultiplicityMethod::BenjaminiYekutieliFdr => "BY (FDR)",
        MultiplicityMethod::TwoStageBkyFdr => "BKY (FDR)",
        MultiplicityMethod::NewmanKeuls => "Newman-Keuls",
        MultiplicityMethod::FisherLsd => "Fisher's LSD",
    }
}

/// Compute the live Dynamic Resolved-Test Banner text for the active One-Way ANOVA parameters.
#[must_use]
pub fn resolve_one_way_anova_banner(app: &UiApp) -> String {
    let ep = &app.canvas_dialogs.edit_params;
    let reml_active = is_reml_mixed_effects_active(ep, app.document());
    if let Some(AnalysisKind::OneWayAnova(a)) = ep.draft.as_ref().map(|d| &d.kind) {
        let post_hoc = post_hoc_banner_name(a.multiplicity.method);
        let omnibus = match a.method {
            OneWayMethod::OrdinaryAnova => "Ordinary one-way ANOVA",
            OneWayMethod::BrownForsytheAndWelch => "Brown-Forsythe and Welch ANOVA",
            OneWayMethod::RepeatedMeasures { .. } if reml_active => match a.df_method {
                MixedModelDfMethod::KenwardRoger => "Mixed-effects REML (Kenward-Roger)",
                MixedModelDfMethod::Satterthwaite => "Mixed-effects REML (Satterthwaite)",
            },
            OneWayMethod::RepeatedMeasures { geisser_greenhouse: true } => "Repeated-measures one-way ANOVA + Geisser-Greenhouse",
            OneWayMethod::RepeatedMeasures { geisser_greenhouse: false } => "Repeated-measures one-way ANOVA (Sphericity assumed)",
            OneWayMethod::KruskalWallis => "Kruskal-Wallis test",
            OneWayMethod::Friedman => "Friedman test",
        };
        if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: true }) && !reml_active && post_hoc == "Tukey" {
            return String::from(omnibus);
        }
        return format!("{omnibus} + {post_hoc}");
    }
    match (ep.assume_gaussian, ep.paired_or_matched) {
        (true, false) if ep.welch_correction => String::from("Brown-Forsythe and Welch ANOVA + Games-Howell"),
        (true, false) => String::from("Ordinary one-way ANOVA + Tukey"),
        (true, true) => String::from("Repeated-measures one-way ANOVA + Geisser-Greenhouse"),
        (false, false) => String::from("Kruskal-Wallis test + Dunn"),
        (false, true) => String::from("Friedman test + Dunn"),
    }
}

/// Render the `One-Way ANOVA` / `EditAnalysisParameters` modal when open, buffering all changes into `intents`.
pub fn render_one_way_anova_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut AnovaModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AnovaModalFrameReport {
    let banner = resolve_one_way_anova_banner(app);
    state.last_resolved_banner.clone_from(&banner);

    let ActiveDialog::EditAnalysisParameters { result_sheet_id } = &app.canvas_dialogs.active_dialog else {
        state.open_session_sheet_id = None;
        return AnovaModalFrameReport { resolved_test_banner: banner, ..AnovaModalFrameReport::default() };
    };
    let Some(spec) = app.canvas_dialogs.edit_params.draft.as_ref() else {
        state.open_session_sheet_id = None;
        return AnovaModalFrameReport { resolved_test_banner: banner, ..AnovaModalFrameReport::default() };
    };
    let AnalysisKind::OneWayAnova(anova) = &spec.kind else {
        state.open_session_sheet_id = None;
        return AnovaModalFrameReport { resolved_test_banner: banner, ..AnovaModalFrameReport::default() };
    };

    sync_modal_state_from_app(ctx, app, state, result_sheet_id, spec, intents);

    let locale = normalize_locale(&app.locale);
    let screen_rect = ctx.content_rect();
    let backdrop_scrim = if tokens.dark { Color32::from_black_alpha(152) } else { Color32::from_rgba_unmultiplied(15, 23, 42, 118) };

    // 1. ZLayer::ModalBackdrop (Order::Middle): 100% outside click & wheel absorption.
    let backdrop_area =
        egui::Area::new(Id::unique("anova_modal.backdrop")).fixed_pos(screen_rect.min).order(Order::Middle).interactable(true).show(ctx, |ui| {
            let (rect, resp) = ui.allocate_exact_size(screen_rect.size(), Sense::click_and_drag());
            ui.painter().rect_filled(rect, CornerRadius::ZERO, backdrop_scrim);
            resp
        });

    let ep = &app.canvas_dialogs.edit_params;
    let doc = app.document();
    let is_create_mode = doc.result_sheet(result_sheet_id).is_none();
    let (modal_w, modal_h) = compute_modal_dimensions(screen_rect);
    let dialog_rect = Rect::from_center_size(screen_rect.center(), Vec2::new(modal_w, modal_h)).intersect(screen_rect);
    let inner_w = (dialog_rect.width() - 24.0).max(240.0);
    let inner_h = (dialog_rect.height() - 24.0).max(180.0);
    let compact_footer = inner_w < 640.0;
    let footer_reserve = if compact_footer { 78.0 } else { 42.0 };
    let title_full = localized_modal_title(locale);
    let title_for_ak = title_full.clone();

    let mut report = AnovaModalFrameReport { modal_open: true, resolved_test_banner: banner.clone(), ..AnovaModalFrameReport::default() };

    let frame = egui::Frame::window(&ctx.global_style())
        .fill(tokens.surface_bg)
        .stroke(Stroke::new(1.5, tokens.accent_strong))
        .corner_radius(CornerRadius::same(tokens.metrics.radius_lg))
        .inner_margin(egui::Margin::same(12));

    let dialog_area = egui::Area::new(Id::unique("anova_modal.dialog"))
        .order(Order::Foreground)
        .fixed_pos(dialog_rect.min)
        .default_size(dialog_rect.size())
        .show(ctx, |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(dialog_rect), |ui| {
                ui.set_clip_rect(dialog_rect);
                ui.set_min_size(dialog_rect.size());
                ui.set_max_size(dialog_rect.size());

                // Register the dialog AccessKit root BEFORE inner widgets so child buttons/inputs win all hover/click hit-tests.
                let dlg_a11y = ui.interact(dialog_rect, Id::unique("anova_modal.a11y_dialog"), Sense::hover());
                dlg_a11y.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Dialog, true, title_for_ak.clone()));

                let frame_resp = frame
                    .show(ui, |ui| {
                        ui.set_width(inner_w);
                        ui.set_max_width(inner_w);
                        ui.set_min_height(inner_h);
                        ui.set_max_height(inner_h);
                        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

                        render_modal_header_and_banner(ui, locale, tokens, &banner, meta, intents);
                        ui.add_space(6.0);
                        render_modal_tab_strip(ui, locale, tokens, ep, state, meta, intents);
                        ui.add_space(6.0);

                        let body_h = (ui.available_height() - footer_reserve).max(76.0);

                        egui::ScrollArea::vertical()
                            .id_salt("anova_modal.body_scroll")
                            .max_height(body_h)
                            .min_scrolled_height(body_h)
                            .auto_shrink([false, false])
                            .scroll_source(egui::scroll_area::ScrollSource {
                                mouse_wheel: true,
                                scroll_bar: true,
                                ..egui::scroll_area::ScrollSource::NONE
                            })
                            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                            .show(ui, |ui| {
                                let scroll_w = (inner_w - 18.0).max(160.0);
                                ui.set_width(scroll_w);
                                ui.set_max_width(scroll_w);
                                if state.show_method_reference {
                                    render_method_reference_card(ui, locale, tokens, state, anova, meta);
                                    ui.add_space(8.0);
                                }
                                render_tab_contents(
                                    ui,
                                    TabRenderCtx {
                                        app,
                                        ep,
                                        anova,
                                        doc,
                                        result_sheet_id,
                                        locale,
                                        tokens,
                                        spec_p_fmt: spec.p_value_format,
                                        conf_pct: spec.confidence_level.percent(),
                                    },
                                    state,
                                    meta,
                                    intents,
                                );
                            });

                        ui.add_space(6.0);
                        render_modal_footer(
                            ui,
                            ModalFooterCtx { locale, tokens, spec, apply_enabled: !is_create_mode && doc.result_sheet(result_sheet_id).is_some() },
                            state,
                            meta,
                            intents,
                        );
                    })
                    .response;
                (dlg_a11y, frame_resp)
            })
            .inner
        });

    let (dlg_a11y, frame_resp) = dialog_area.inner;
    let actual_dialog_rect = frame_resp.rect.intersect(dialog_rect);
    let actual_dialog_rect = if actual_dialog_rect.is_positive() { actual_dialog_rect } else { dialog_rect };
    report.dialog_rect_pt = Some(actual_dialog_rect);

    meta.record(FrameInspectNodeMeta {
        egui_id: backdrop_area.inner.id,
        accesskit_id: backdrop_area.inner.id.accesskit_id(),
        widget_id: String::from("modal.backdrop"),
        role: String::from("backdrop"),
        label: String::from("Modal Backdrop"),
        command_id: None,
        value: None,
        enabled: true,
        disabled_reason: None,
        clipped_rect: screen_rect,
        z_layer: ZLayer::ModalBackdrop,
        ellipsized: false,
        full_label: None,
    });

    meta.record(FrameInspectNodeMeta {
        egui_id: dlg_a11y.id,
        accesskit_id: dlg_a11y.id.accesskit_id(),
        widget_id: String::from("modal.dialog"),
        role: String::from("dialog"),
        label: title_full,
        command_id: None,
        value: None,
        enabled: true,
        disabled_reason: None,
        clipped_rect: screen_rect.intersect(actual_dialog_rect),
        z_layer: ZLayer::ModalDialog,
        ellipsized: false,
        full_label: None,
    });

    handle_modal_capture_and_keyboard(ctx, actual_dialog_rect, ep, state, intents, &mut report);
    report
}

pub(super) fn compute_modal_dimensions(screen_rect: Rect) -> (f32, f32) {
    let max_w = (screen_rect.width() - 24.0).max(280.0);
    let max_h = (screen_rect.height() - 24.0).max(240.0);
    let target_w = max_w.min(760.0);
    let target_h = max_h.min(560.0);
    (target_w, target_h)
}

fn sync_modal_state_from_app(
    ctx: &Context,
    app: &UiApp,
    state: &mut AnovaModalEguiState,
    result_sheet_id: &SheetId,
    spec: &AnalysisSpec,
    intents: &mut PendingUiIntents,
) {
    let ep = &app.canvas_dialogs.edit_params;
    let conf_pct = spec.confidence_level.percent();
    let key_last_ep_tab = Id::unique("anova_modal.last_ep_tab");
    let is_create_mode = app.document().result_sheet(result_sheet_id).is_none();

    if state.open_session_sheet_id.as_deref() != Some(result_sheet_id.as_str()) {
        state.open_session_sheet_id = Some(result_sheet_id.as_str().to_owned());
        state.confidence_percent_draft = conf_pct;
        state.show_method_reference = false;
        if is_create_mode && let Some(saved) = &state.saved_default_spec {
            let mut applied = saved.clone();
            applied.source_sheet_id = spec.source_sheet_id.clone();
            applied.selected_data_sets.clone_from(&spec.selected_data_sets);
            applied.custom_title.clone_from(&spec.custom_title);
            if applied != *spec {
                intents.push_set_edit_params_draft(applied);
            }
            state.method_saved_as_default = true;
        } else {
            state.method_saved_as_default = state.saved_default_spec.as_ref().is_some_and(|saved| {
                saved.kind == spec.kind && saved.confidence_level == spec.confidence_level && saved.p_value_format == spec.p_value_format
            });
        }
        if let Some(tab) = AnovaModalTab::from_slug(&ep.active_tab) {
            state.active_tab =
                if !ep.paired_or_matched && tab == AnovaModalTab::AssumptionsAndTest { AnovaModalTab::ExperimentalDesign } else { tab };
        }
    } else {
        state.method_saved_as_default = state.saved_default_spec.as_ref().is_some_and(|saved| {
            saved.kind == spec.kind && saved.confidence_level == spec.confidence_level && saved.p_value_format == spec.p_value_format
        });
    }

    let prev_ep_tab: Option<String> = ctx.data(|d| d.get_temp(key_last_ep_tab));
    if prev_ep_tab.as_deref() != Some(ep.active_tab.as_str()) {
        ctx.data_mut(|d| d.insert_temp(key_last_ep_tab, ep.active_tab.clone()));
        if let Some(tab) = AnovaModalTab::from_slug(&ep.active_tab)
            && (ep.paired_or_matched || tab != AnovaModalTab::AssumptionsAndTest)
        {
            state.active_tab = tab;
        }
    }
    if !ep.paired_or_matched && state.active_tab == AnovaModalTab::AssumptionsAndTest {
        state.active_tab = AnovaModalTab::ExperimentalDesign;
    }
}

fn handle_modal_capture_and_keyboard(
    ctx: &Context,
    dialog_rect: Rect,
    ep: &EditAnalysisParametersState,
    state: &mut AnovaModalEguiState,
    intents: &mut PendingUiIntents,
    report: &mut AnovaModalFrameReport,
) {
    let pointer_pos = ctx.input(|i| i.pointer.latest_pos().or_else(|| i.pointer.interact_pos()));
    let outside_pointer = pointer_pos.is_some_and(|p| !dialog_rect.contains(p));
    let any_click_outside = outside_pointer
        && ctx.input(|i| {
            i.pointer.primary_clicked()
                || i.pointer.secondary_clicked()
                || i.events.iter().any(|ev| matches!(ev, egui::Event::PointerButton { pos, pressed: false, .. } if !dialog_rect.contains(*pos)))
        });
    if any_click_outside {
        report.outside_click_captured = true;
    }

    let has_wheel = ctx.input(|i| i.smooth_scroll_delta.length_sq() > 0.0 || i.events.iter().any(|ev| matches!(ev, egui::Event::MouseWheel { .. })));
    if has_wheel && (outside_pointer || pointer_pos.is_none()) {
        ctx.input_mut(|i| {
            i.smooth_scroll_delta = Vec2::ZERO;
            i.events.retain(|ev| !matches!(ev, egui::Event::MouseWheel { .. }));
        });
        report.outside_wheel_captured = true;
    }

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        if state.show_method_reference {
            state.show_method_reference = false;
        } else {
            intents.push_dialog_button("cancel");
        }
        return;
    }

    let wants_text = ctx.egui_wants_keyboard_input();
    if !wants_text && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        intents.push_dialog_button("ok");
        return;
    }

    let next_shortcut = ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Tab) || i.consume_key(Modifiers::CTRL, Key::PageDown));
    let prev_shortcut = ctx.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) || i.consume_key(Modifiers::CTRL, Key::PageUp));
    if next_shortcut {
        step_enabled_tab(ep, state, 1, intents);
    } else if prev_shortcut {
        step_enabled_tab(ep, state, -1, intents);
    }
}

fn step_enabled_tab(ep: &EditAnalysisParametersState, state: &mut AnovaModalEguiState, dir: isize, intents: &mut PendingUiIntents) {
    let cur = AnovaModalTab::ALL.iter().position(|&t| t == state.active_tab).unwrap_or(0) as isize;
    let len = AnovaModalTab::ALL.len() as isize;
    for offset in 1..=len {
        let idx = ((cur + dir * offset).rem_euclid(len)) as usize;
        let cand = AnovaModalTab::ALL[idx];
        if tab_enablement(ep, cand).enabled {
            state.active_tab = cand;
            intents.push_custom_view_action("stat_params", format!("tab:{}", cand.legacy_id()));
            break;
        }
    }
}

fn tab_enablement(ep: &EditAnalysisParametersState, tab: AnovaModalTab) -> ControlEnablement {
    match tab {
        AnovaModalTab::AssumptionsAndTest => r1_repeated_measures_tab(ep.paired_or_matched),
        _ => ControlEnablement::enabled(),
    }
}

fn render_modal_header_and_banner(
    ui: &mut Ui,
    locale: &str,
    tokens: &Tokens,
    banner: &str,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let title = localized_modal_title(locale);
    let header_w = ui.available_width().max(120.0);
    ui.horizontal(|ui| {
        let close_reserve = 28.0;
        let title_w = (header_w - close_reserve).max(80.0);
        ui.allocate_ui(Vec2::new(title_w, 22.0), |ui| {
            render_clipped_text(
                ui,
                meta,
                ClippedTextSpec {
                    widget_id: "modal.title",
                    role: "label",
                    text: &title,
                    font_size: 14.5,
                    color: tokens.text_strong,
                    z_layer: ZLayer::ModalDialog,
                    tooltip: None,
                    record_meta: false,
                },
            );
        });
        let (_rect, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
        let close_lbl = tr(locale, "btn.cancel");
        let resp = paint_icon_button_named(ui, resp, "close", close_lbl, close_lbl, false);
        record_widget(ui, meta, &resp, "modal.close", "button", close_lbl, None, None, true, None, ZLayer::ModalDialog, false, None);
        if resp.clicked() {
            intents.push_dialog_button("cancel");
        }
    });

    ui.add_space(4.0);
    let prefix = tr(locale, "anova.resolved_prefix");
    let full_banner = format!("{prefix}: {banner}");
    let banner_frame = egui::Frame::NONE
        .fill(tokens.active_bg)
        .stroke(Stroke::new(1.0, tokens.accent))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 5));
    let banner_inner_w = (header_w - 16.0).max(120.0);
    banner_frame.show(ui, |ui| {
        ui.set_width(banner_inner_w);
        ui.set_max_width(banner_inner_w);
        let resp = render_clipped_text(
            ui,
            meta,
            ClippedTextSpec {
                widget_id: "edit.resolved_test_banner",
                role: "status",
                text: &full_banner,
                font_size: 12.0,
                color: tokens.text_strong,
                z_layer: ZLayer::ModalDialog,
                tooltip: None,
                record_meta: true,
            },
        );
        let _ = resp;
    });
}

fn render_modal_tab_strip(
    ui: &mut Ui,
    locale: &str,
    tokens: &Tokens,
    ep: &EditAnalysisParametersState,
    state: &mut AnovaModalEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let count = AnovaModalTab::ALL.len() as f32;
    let gap = 4.0;
    let total_w = ui.available_width().clamp(220.0, 736.0);
    let each_w = ((total_w - gap * (count - 1.0)) / count).max(40.0);
    let tab_h = 26.0;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (idx, &tab) in AnovaModalTab::ALL.iter().enumerate() {
            let enablement = tab_enablement(ep, tab);
            let full_label = tab_label(locale, tab).to_owned();
            let selected = state.active_tab == tab;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(each_w, tab_h), if enablement.enabled { Sense::click() } else { Sense::hover() });

            let bg = if !enablement.enabled {
                tokens.panel_bg
            } else if selected {
                tokens.active_bg
            } else if resp.hovered() {
                tokens.hover_bg
            } else {
                tokens.button_bg
            };
            let border = if !enablement.enabled {
                tokens.divider
            } else if selected {
                tokens.accent_strong
            } else {
                tokens.button_border
            };
            let fg = if !enablement.enabled {
                tokens.text_disabled
            } else if selected {
                tokens.text_strong
            } else {
                tokens.text_primary
            };

            ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
            ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, border), StrokeKind::Inside);

            let display_prefix = if enablement.enabled { "" } else { "🔒 " };
            let text_max_w = (rect.width() - 8.0).max(16.0);
            let mut job = egui::text::LayoutJob::single_section(
                format!("{display_prefix}{full_label}"),
                egui::TextFormat::simple(FontId::proportional(11.5), fg),
            );
            job.wrap = egui::text::TextWrapping { max_width: text_max_w, max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
            let galley = ui.painter().layout_job(job);
            let ellipsized = galley.elided;
            let text_pos = Pos2::new(rect.center().x - (galley.size().x * 0.5).min(text_max_w * 0.5), rect.center().y - galley.size().y * 0.5);
            ui.painter().with_clip_rect(rect.shrink(3.0)).galley(text_pos, galley, fg);

            let a11y_name = full_label.clone();
            let tab_enabled = enablement.enabled;
            resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::Tab, tab_enabled, selected, a11y_name.clone()));

            let tip = if let Some(reason) = enablement.disabled_reason.as_ref() {
                format!("{full_label}: {reason}")
            } else if ellipsized {
                full_label.clone()
            } else {
                String::new()
            };
            let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };

            if enablement.enabled && resp.has_focus() {
                if ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight)) {
                    step_enabled_tab(ep, state, 1, intents);
                } else if ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft)) {
                    step_enabled_tab(ep, state, -1, intents);
                } else {
                    let _ = idx;
                }
            }

            if enablement.enabled && resp.clicked() {
                state.active_tab = tab;
                intents.push_custom_view_action("stat_params", format!("tab:{}", tab.legacy_id()));
            }

            let wid = format!("edit.tab.{}", tab.legacy_id());
            record_widget(
                ui,
                meta,
                &resp,
                &wid,
                "tab",
                &full_label,
                None,
                Some(json!(selected)),
                enablement.enabled,
                enablement.disabled_reason.as_deref(),
                ZLayer::ModalDialog,
                ellipsized,
                ellipsized.then_some(full_label.as_str()),
            );
        }
    });
}

struct TabRenderCtx<'a> {
    app: &'a UiApp,
    ep: &'a EditAnalysisParametersState,
    anova: &'a OneWayAnovaSpec,
    doc: &'a ProjectDocument,
    result_sheet_id: &'a SheetId,
    locale: &'a str,
    tokens: &'a Tokens,
    spec_p_fmt: PValueFormat,
    conf_pct: f64,
}

fn render_tab_contents(
    ui: &mut Ui,
    rc: TabRenderCtx<'_>,
    state: &mut AnovaModalEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    match state.active_tab {
        AnovaModalTab::ExperimentalDesign => render_tab_experimental_design(ui, rc, meta, intents),
        AnovaModalTab::AssumptionsAndTest => render_tab_repeated_measures(ui, rc, meta, intents),
        AnovaModalTab::MultipleComparisons => render_tab_multiple_comparisons(ui, rc, meta, intents),
        AnovaModalTab::Options => render_tab_options(ui, rc, state, meta, intents),
        AnovaModalTab::ResidualsAndPlots => render_tab_residuals(ui, rc, meta, intents),
    }
}

fn render_tab_experimental_design(ui: &mut Ui, rc: TabRenderCtx<'_>, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let reml_active = is_reml_mixed_effects_active(rc.ep, rc.doc);
    let src_id = rc.ep.draft.as_ref().map(|d| &d.source_sheet_id);
    let assume_en = r2_assume_gaussian_allowed(rc.doc, src_id);
    let paired_en = r1_paired_or_matched_allowed(rc.doc, src_id);

    let wide = ui.available_width() >= 500.0;
    if wide {
        let total_w = ui.available_width();
        let left_w = (total_w * 0.56).max(200.0);
        let right_w = (total_w - left_w - 8.0).max(150.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(Vec2::new(left_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(left_w);
                ui.set_max_width(left_w);
                render_design_controls_left(ui, &rc, &assume_en, &paired_en, reml_active, meta, intents);
            });
            ui.add_space(6.0);
            ui.allocate_ui_with_layout(Vec2::new(right_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_width(right_w);
                ui.set_max_width(right_w);
                render_design_right_pane(ui, &rc, reml_active, meta, intents);
            });
        });
    } else {
        render_design_controls_left(ui, &rc, &assume_en, &paired_en, reml_active, meta, intents);
        ui.add_space(8.0);
        render_design_right_pane(ui, &rc, reml_active, meta, intents);
    }
}

fn render_design_controls_left(
    ui: &mut Ui,
    rc: &TabRenderCtx<'_>,
    assume_en: &ControlEnablement,
    paired_en: &ControlEnablement,
    reml_active: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, rc.tokens, tr(rc.locale, "stat_params.section.datasets"));
    let gauss_lbl = tr(rc.locale, "stat_params.label.assume_gaussian");
    if render_modal_checkbox(ui, rc.tokens, meta, "edit.assume_gaussian", gauss_lbl, rc.ep.assume_gaussian, assume_en) {
        intents.push_dialog_widget_click("edit.assume_gaussian");
    }

    let paired_lbl = tr(rc.locale, "stat_params.label.paired_matched");
    if render_modal_checkbox(ui, rc.tokens, meta, "edit.paired_or_matched", paired_lbl, rc.ep.paired_or_matched, paired_en) {
        intents.push_dialog_widget_click("edit.paired_or_matched");
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if render_modal_seg_btn(
            ui,
            rc.tokens,
            meta,
            "edit.anova.ordinary",
            "Ordinary",
            rc.anova.method == OneWayMethod::OrdinaryAnova,
            &ControlEnablement::enabled(),
        ) {
            intents.push_custom_view_action("stat_params", "set_one_way_method:ordinary");
        }
        if render_modal_seg_btn(
            ui,
            rc.tokens,
            meta,
            "edit.anova.welch",
            "Welch",
            rc.anova.method == OneWayMethod::BrownForsytheAndWelch,
            &ControlEnablement::enabled(),
        ) {
            intents.push_custom_view_action("stat_params", "set_one_way_method:welch");
        }
        if render_modal_seg_btn(
            ui,
            rc.tokens,
            meta,
            "edit.anova.rm",
            "RM ANOVA",
            matches!(rc.anova.method, OneWayMethod::RepeatedMeasures { .. }),
            paired_en,
        ) {
            intents.push_custom_view_action("stat_params", "set_one_way_method:rm");
        }
        if render_modal_seg_btn(
            ui,
            rc.tokens,
            meta,
            "edit.anova.kruskal",
            "Kruskal-Wallis",
            rc.anova.method == OneWayMethod::KruskalWallis,
            assume_en,
        ) {
            intents.push_custom_view_action("stat_params", "set_one_way_method:kruskal");
        }
        let friedman_en = if !assume_en.enabled { assume_en } else { paired_en };
        if render_modal_seg_btn(ui, rc.tokens, meta, "edit.anova.friedman", "Friedman", rc.anova.method == OneWayMethod::Friedman, friedman_en) {
            intents.push_custom_view_action("stat_params", "set_one_way_method:friedman");
        }
    });

    ui.add_space(8.0);
    render_section_title(ui, rc.tokens, tr(rc.locale, "stat_params.tab.multiple_comparisons"));
    render_multiplicity_dropdown(ui, rc, reml_active, "anova_tab1_mult", meta, intents);

    let tukey_en = r1b_post_hoc_option_enabled(rc.ep.assume_gaussian, reml_active, MultiplicityMethod::TukeyHsd);
    if render_modal_radio(
        ui,
        rc.tokens,
        meta,
        "edit.post_hoc.tukey",
        "Tukey HSD (all-pairs)",
        rc.anova.multiplicity.method == MultiplicityMethod::TukeyHsd,
        &tukey_en,
    ) {
        intents.push_dialog_widget_click("edit.post_hoc.tukey");
    }

    let dunn_en = r1b_post_hoc_option_enabled(rc.ep.assume_gaussian, reml_active, MultiplicityMethod::DunnRankSum);
    if render_modal_radio(
        ui,
        rc.tokens,
        meta,
        "edit.post_hoc.dunn",
        "Dunn's rank-sum comparisons",
        rc.anova.multiplicity.method == MultiplicityMethod::DunnRankSum,
        &dunn_en,
    ) {
        intents.push_dialog_widget_click("edit.post_hoc.dunn");
    }

    render_alpha_stepper(ui, rc, meta, intents);
}

fn render_design_right_pane(ui: &mut Ui, rc: &TabRenderCtx<'_>, reml_active: bool, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    render_section_title(ui, rc.tokens, tr(rc.locale, "stat_params.section.schematic"));
    draw_experimental_design_schematic(ui, rc, reml_active, meta, intents);

    ui.add_space(8.0);
    let titles = column_titles_for_draft(rc.doc, rc.ep.draft.as_ref().map(|d| &d.source_sheet_id));
    let selected_sets = rc.ep.draft.as_ref().map_or(&[][..], |d| d.selected_data_sets.as_slice());
    let all_checked = selected_sets.is_empty();
    if render_modal_checkbox(ui, rc.tokens, meta, "edit.dataset.all", "All Data Sets / Columns", all_checked, &ControlEnablement::enabled()) {
        intents.push_custom_view_action("stat_params", "select_all_datasets");
    }
    for (idx, col_name) in titles.iter().enumerate() {
        let checked = selected_sets.is_empty() || selected_sets.contains(&idx);
        let wid = format!("edit.dataset.{idx}");
        if render_modal_checkbox(ui, rc.tokens, meta, &wid, col_name, checked, &ControlEnablement::enabled()) {
            intents.push_custom_view_action("stat_params", format!("toggle_dataset:{idx}"));
        }
    }
}

fn draw_experimental_design_schematic(
    ui: &mut Ui,
    rc: &TabRenderCtx<'_>,
    reml_active: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let w = (ui.available_width() - 6.0).clamp(140.0, 224.0);
    let h = 110.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let bg = rc.tokens.panel_bg;
    ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, rc.tokens.border), StrokeKind::Inside);

    let caption = if rc.ep.paired_or_matched && reml_active {
        tr(rc.locale, "anova.schematic.reml")
    } else if rc.ep.paired_or_matched {
        tr(rc.locale, "anova.schematic.paired")
    } else {
        tr(rc.locale, "anova.schematic.independent")
    };
    let mut job = egui::text::LayoutJob::single_section(
        caption.to_owned(),
        egui::TextFormat::simple(FontId::proportional(rc.tokens.metrics.font_ui_small_pt), rc.tokens.text_muted),
    );
    job.wrap =
        egui::text::TextWrapping { max_width: (rect.width() - 16.0).max(40.0), max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let cap_galley = ui.painter().layout_job(job);
    ui.painter().with_clip_rect(rect.shrink(4.0)).galley(Pos2::new(rect.left() + 8.0, rect.top() + 5.0), cap_galley, rc.tokens.text_muted);

    let plot = Rect::from_min_max(Pos2::new(rect.left() + 14.0, rect.top() + 26.0), Pos2::new(rect.right() - 14.0, rect.bottom() - 10.0));
    let cols_x = [plot.left() + plot.width() * 0.18, plot.center().x, plot.right() - plot.width() * 0.18];
    let palette = [rc.tokens.accent_strong, Color32::from_rgb(22, 163, 74), Color32::from_rgb(234, 88, 12)];
    let subject_offsets = [[0.22_f32, 0.35, 0.25], [0.52, 0.48, 0.62], [0.80, 0.72, 0.85]];

    if rc.ep.paired_or_matched {
        for (s_idx, offsets) in subject_offsets.iter().enumerate() {
            let pts: [Pos2; 3] = [
                Pos2::new(cols_x[0], plot.top() + plot.height() * offsets[0]),
                Pos2::new(cols_x[1], plot.top() + plot.height() * offsets[1]),
                Pos2::new(cols_x[2], plot.top() + plot.height() * offsets[2]),
            ];
            let line_col = if reml_active && s_idx == 1 { rc.tokens.link_red } else { rc.tokens.accent };
            ui.painter().line_segment([pts[0], pts[1]], Stroke::new(1.5, line_col));
            if !(reml_active && s_idx == 1) {
                ui.painter().line_segment([pts[1], pts[2]], Stroke::new(1.5, line_col));
            }
            for (c_idx, pt) in pts.iter().enumerate() {
                if reml_active && s_idx == 1 && c_idx == 2 {
                    ui.painter().circle_stroke(*pt, 4.0, Stroke::new(1.4, rc.tokens.link_red));
                } else {
                    ui.painter().circle_filled(*pt, 4.0, palette[c_idx]);
                }
            }
        }
    } else {
        for (c_idx, cx) in cols_x.iter().enumerate() {
            let col = palette[c_idx];
            let mean_y = plot.top() + plot.height() * (0.45 + 0.08 * (c_idx as f32 - 1.0));
            ui.painter().line_segment([Pos2::new(cx - 14.0, mean_y), Pos2::new(cx + 14.0, mean_y)], Stroke::new(2.0, col));
            for (d_idx, dy) in [-14.0_f32, -4.0, 6.0, 14.0].iter().enumerate() {
                let dx = ((d_idx as f32) - 1.5) * 4.5;
                ui.painter().circle_filled(Pos2::new(cx + dx, mean_y + *dy), 3.3, col);
            }
        }
    }

    let name = caption.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, true, name.clone()));
    let resp = resp.on_hover_text(caption);
    record_widget(
        ui,
        meta,
        &resp,
        "edit.design.schematic",
        "button",
        caption,
        None,
        Some(json!(caption)),
        true,
        None,
        ZLayer::ModalDialog,
        false,
        None,
    );
    if resp.clicked() {
        intents.push_custom_view_action("stat_params", "schematic:toggle");
    }
}

fn render_tab_repeated_measures(ui: &mut Ui, rc: TabRenderCtx<'_>, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let reml_active = is_reml_mixed_effects_active(rc.ep, rc.doc);
    let sph_en = r1b_sphericity_correction(rc.ep.paired_or_matched, reml_active);
    let reml_en = r1_sphericity_or_reml_control(rc.ep.paired_or_matched);
    let gg_checked = !matches!(rc.anova.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false });

    render_section_title(ui, rc.tokens, "Repeated-Measures Sphericity & Mixed-Effects REML");
    if render_modal_checkbox(
        ui,
        rc.tokens,
        meta,
        "edit.sphericity.geisser_greenhouse",
        "Apply Geisser-Greenhouse sphericity correction (epsilon)",
        gg_checked,
        &sph_en,
    ) {
        intents.push_dialog_widget_click("edit.sphericity.geisser_greenhouse");
    }
    if render_modal_radio(ui, rc.tokens, meta, "edit.rm.assume_sphericity", "Assume sphericity (unadjusted df)", !gg_checked && !reml_active, &sph_en)
        && gg_checked
    {
        intents.push_custom_view_action("stat_params", "toggle_geisser_greenhouse");
    }
    if render_modal_checkbox(
        ui,
        rc.tokens,
        meta,
        "edit.rm.mixed_effects_reml",
        tr(rc.locale, "stat_params.rm.mixed_effects_reml"),
        rc.anova.mixed_effects_reml,
        &reml_en,
    ) {
        intents.push_dialog_widget_click("edit.rm.mixed_effects_reml");
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if render_modal_seg_btn(
            ui,
            rc.tokens,
            meta,
            "edit.rm.df_kenward_roger",
            tr(rc.locale, "stat_params.rm.df_kenward_roger"),
            rc.anova.df_method == MixedModelDfMethod::KenwardRoger,
            &reml_en,
        ) {
            intents.push_dialog_widget_click("edit.rm.df_kenward_roger");
        }
        if render_modal_seg_btn(
            ui,
            rc.tokens,
            meta,
            "edit.rm.df_satterthwaite",
            tr(rc.locale, "stat_params.rm.df_satterthwaite"),
            rc.anova.df_method == MixedModelDfMethod::Satterthwaite,
            &reml_en,
        ) {
            intents.push_dialog_widget_click("edit.rm.df_satterthwaite");
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if render_modal_seg_btn(
            ui,
            rc.tokens,
            meta,
            "edit.reml.cov.cs",
            tr(rc.locale, "reml.covariance.compound_symmetry"),
            rc.anova.residual_covariance == ResidualCovariance::CompoundSymmetry,
            &reml_en,
        ) {
            intents.push_custom_view_action("stat_params", "set_residual_cov:compound_symmetry");
        }
        if render_modal_seg_btn(
            ui,
            rc.tokens,
            meta,
            "edit.reml.cov.un",
            tr(rc.locale, "reml.covariance.unstructured"),
            rc.anova.residual_covariance == ResidualCovariance::Unstructured,
            &reml_en,
        ) {
            intents.push_custom_view_action("stat_params", "set_residual_cov:unstructured");
        }
    });

    ui.add_space(6.0);
    let (rows, cols, _) = source_table_shape(rc.doc, Some(rc.result_sheet_id));
    let info = if reml_active {
        format!(
            "Mixed-Effects REML active ({:?}): {rows} rows x {cols} groups (classical sphericity & Tukey/Dunnett/Games-Howell disabled)",
            rc.anova.df_method
        )
    } else {
        format!("Subject blocks: {rows} matched rows across {cols} conditions")
    };
    render_clipped_text(
        ui,
        meta,
        ClippedTextSpec {
            widget_id: "edit.rm.block_info",
            role: "label",
            text: &info,
            font_size: 11.5,
            color: rc.tokens.text_muted,
            z_layer: ZLayer::ModalDialog,
            tooltip: None,
            record_meta: true,
        },
    );
}

fn render_tab_multiple_comparisons(ui: &mut Ui, rc: TabRenderCtx<'_>, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let reml_active = is_reml_mixed_effects_active(rc.ep, rc.doc);
    let mult = &rc.anova.multiplicity;
    let gate = |m: MultiplicityMethod| r1b_post_hoc_option_enabled(rc.ep.assume_gaussian, reml_active, m);

    render_section_title(ui, rc.tokens, tr(rc.locale, "stat_params.tab.multiple_comparisons"));
    render_multiplicity_dropdown(ui, &rc, reml_active, "anova_tab3_mult", meta, intents);

    if render_modal_radio(
        ui,
        rc.tokens,
        meta,
        "edit.post_hoc.tukey",
        "Tukey HSD (all-pairwise equal variance)",
        mult.method == MultiplicityMethod::TukeyHsd,
        &gate(MultiplicityMethod::TukeyHsd),
    ) {
        intents.push_dialog_widget_click("edit.post_hoc.tukey");
    }
    if render_modal_radio(
        ui,
        rc.tokens,
        meta,
        "edit.post_hoc.dunnett",
        "Dunnett (every group vs. control column)",
        matches!(mult.method, MultiplicityMethod::Dunnett { .. }),
        &gate(MultiplicityMethod::Dunnett { control_column: rc.ep.control_column }),
    ) {
        intents.push_dialog_widget_click("edit.post_hoc.dunnett");
    }
    if render_modal_radio(
        ui,
        rc.tokens,
        meta,
        "edit.post_hoc.dunn",
        "Dunn's rank-sum comparisons (nonparametric)",
        mult.method == MultiplicityMethod::DunnRankSum,
        &gate(MultiplicityMethod::DunnRankSum),
    ) {
        intents.push_dialog_widget_click("edit.post_hoc.dunn");
    }

    ui.add_space(4.0);
    let seg_methods: [(&str, &str, &str, MultiplicityMethod); 9] = [
        ("edit.post_hoc.bonferroni", "bonferroni", "Bonferroni", MultiplicityMethod::Bonferroni),
        ("edit.post_hoc.sidak", "sidak", tr(rc.locale, "stat_params.post_hoc.sidak"), MultiplicityMethod::Sidak),
        ("edit.post_hoc.holm_sidak", "holm_sidak", "Holm-Sidak", MultiplicityMethod::HolmSidak),
        ("edit.post_hoc.fisher_lsd", "fisher_lsd", tr(rc.locale, "stat_params.post_hoc.fisher_lsd"), MultiplicityMethod::FisherLsd),
        ("edit.post_hoc.games_howell", "games_howell", "Games-Howell", MultiplicityMethod::GamesHowell),
        ("edit.post_hoc.none", "none", "None", MultiplicityMethod::None),
        ("edit.post_hoc.bh", "bh", "BH (FDR)", MultiplicityMethod::BenjaminiHochbergFdr),
        ("edit.post_hoc.bky", "bky", "BKY (FDR)", MultiplicityMethod::TwoStageBkyFdr),
        ("edit.post_hoc.by", "by", "BY (FDR)", MultiplicityMethod::BenjaminiYekutieliFdr),
    ];
    ui.horizontal_wrapped(|ui| {
        for (wid, slug, lbl, m) in seg_methods {
            let en = gate(m);
            if render_modal_seg_btn(ui, rc.tokens, meta, wid, lbl, mult.method == m, &en) {
                intents.push_custom_view_action("stat_params", format!("set_multiplicity:{slug}"));
            }
        }
    });

    ui.add_space(6.0);
    let step = render_stepper_row(
        ui,
        rc.tokens,
        meta,
        StepperSpec {
            id_prefix: "edit.mult_threshold",
            label: "Familywise Alpha / FDR Q",
            value_text: &format!("{:.3}", mult.threshold),
            enablement: &ControlEnablement::enabled(),
        },
    );
    if step < 0 {
        intents.push_custom_view_action("stat_params", "spinner:edit.mult_threshold:-0.01");
    } else if step > 0 {
        intents.push_custom_view_action("stat_params", "spinner:edit.mult_threshold:0.01");
    }

    if render_modal_checkbox(
        ui,
        rc.tokens,
        meta,
        "edit.compare_against_control",
        "Compare all groups against a designated control column",
        rc.ep.compare_against_control,
        &ControlEnablement::enabled(),
    ) {
        let next_cmp = !rc.ep.compare_against_control;
        if next_cmp && rc.ep.assume_gaussian {
            intents.push_custom_view_action("stat_params", "set_multiplicity:dunnett");
        } else if !next_cmp {
            let fallback = if rc.ep.assume_gaussian { "set_multiplicity:tukey" } else { "set_multiplicity:dunn" };
            intents.push_custom_view_action("stat_params", fallback);
        } else {
            intents.push_custom_view_action("stat_params", "set_compare_against_control:true");
        }
    }

    let ctrl_en = r3_control_column_picker(rc.ep.compare_against_control, mult.method);
    let titles = column_titles_for_draft(rc.doc, rc.ep.draft.as_ref().map(|d| &d.source_sheet_id));
    let ctrl_name = titles.get(rc.ep.control_column).cloned().unwrap_or_else(|| format!("Col {}", rc.ep.control_column + 1));
    let ctrl_lbl = format!("Control Column: {ctrl_name} (col {})", rc.ep.control_column + 1);
    if render_modal_seg_btn(ui, rc.tokens, meta, "edit.control_column", &ctrl_lbl, ctrl_en.enabled, &ctrl_en) {
        intents.push_custom_view_action("stat_params", "cycle_control_column");
    }

    if render_modal_checkbox(
        ui,
        rc.tokens,
        meta,
        "edit.mult_report_ci",
        "Compute confidence intervals for pairwise differences",
        mult.report_confidence_intervals,
        &ControlEnablement::enabled(),
    ) {
        intents.push_custom_view_action("stat_params", "toggle_mult_report_ci");
    }

    ui.add_space(4.0);
    render_section_title(ui, rc.tokens, tr(rc.locale, "stat_params.section.cld_compat"));
    let cld_msg = cld_status_message(mult.method);
    render_clipped_text(
        ui,
        meta,
        ClippedTextSpec {
            widget_id: "edit.mc.cld_status",
            role: "label",
            text: cld_msg,
            font_size: 11.5,
            color: rc.tokens.text_muted,
            z_layer: ZLayer::ModalDialog,
            tooltip: None,
            record_meta: true,
        },
    );
}

fn render_tab_options(
    ui: &mut Ui,
    rc: TabRenderCtx<'_>,
    state: &mut AnovaModalEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    render_section_title(ui, rc.tokens, "Significance Level & Confidence Intervals");
    render_alpha_stepper(ui, &rc, meta, intents);

    ui.horizontal_wrapped(|ui| {
        for (wid, pct, lbl) in
            [("edit.conf.90", 90.0_f64, "90% CI (α=0.10)"), ("edit.conf.95", 95.0, "95% CI (α=0.05)"), ("edit.conf.99", 99.0, "99% CI (α=0.01)")]
        {
            if render_modal_seg_btn(ui, rc.tokens, meta, wid, lbl, (rc.conf_pct - pct).abs() < 0.05, &ControlEnablement::enabled()) {
                state.confidence_percent_draft = pct;
                intents.push_custom_view_action("stat_params", format!("set_confidence:{pct:.1}"));
            }
        }
    });

    ui.add_space(6.0);
    render_section_title(ui, rc.tokens, "ANOVA Options");
    if render_modal_checkbox(
        ui,
        rc.tokens,
        meta,
        "edit.compute_effect_sizes",
        "Compute effect sizes (eta-squared / omega-squared)",
        rc.anova.compute_effect_sizes,
        &ControlEnablement::enabled(),
    ) {
        intents.push_custom_view_action("stat_params", "toggle_effect_sizes");
    }

    let gg_checked = !matches!(rc.anova.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false });
    let reml_active = is_reml_mixed_effects_active(rc.ep, rc.doc);
    let sph_en = r1b_sphericity_correction(rc.ep.paired_or_matched, reml_active);
    if render_modal_checkbox(
        ui,
        rc.tokens,
        meta,
        "edit.sphericity.geisser_greenhouse",
        "Geisser-Greenhouse sphericity correction",
        gg_checked,
        &sph_en,
    ) {
        intents.push_dialog_widget_click("edit.sphericity.geisser_greenhouse");
    }

    ui.add_space(6.0);
    render_section_title(ui, rc.tokens, "P-Value Display Formatting");
    ui.horizontal_wrapped(|ui| {
        for (wid, slug, lbl, active) in [
            ("edit.pval.four_decimal", "four_decimal", "0.0001", matches!(rc.spec_p_fmt, PValueFormat::FourDecimal)),
            ("edit.pval.apa", "apa", "APA", matches!(rc.spec_p_fmt, PValueFormat::Apa)),
            ("edit.pval.nejm", "nejm", "NEJM", matches!(rc.spec_p_fmt, PValueFormat::Nejm)),
            ("edit.pval.custom", "custom", "Custom", matches!(rc.spec_p_fmt, PValueFormat::Custom { .. })),
        ] {
            if render_modal_seg_btn(ui, rc.tokens, meta, wid, lbl, active, &ControlEnablement::enabled()) {
                intents.push_custom_view_action("stat_params", format!("set_pvalue_format:{slug}"));
            }
        }
    });

    let digits = match rc.spec_p_fmt {
        PValueFormat::FourDecimal => 4,
        PValueFormat::Apa | PValueFormat::Nejm => 3,
        PValueFormat::Custom { decimals, .. } => decimals,
    };
    let r9_en = r9_pvalue_precision_spinner(&rc.spec_p_fmt);
    let step = render_stepper_row(
        ui,
        rc.tokens,
        meta,
        StepperSpec { id_prefix: "edit.pvalue.digits", label: "Custom P-Value Decimal Digits", value_text: &digits.to_string(), enablement: &r9_en },
    );
    if step < 0 {
        intents.push_custom_view_action("stat_params", "spinner:edit.pvalue.digits:-1");
    } else if step > 0 {
        intents.push_custom_view_action("stat_params", "spinner:edit.pvalue.digits:1");
    }
}

fn render_tab_residuals(ui: &mut Ui, rc: TabRenderCtx<'_>, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let st = &rc.app.view_shards.stat_params;
    render_section_title(ui, rc.tokens, "Residual, Q-Q & Assumption Diagnostics");
    ui.label(
        egui::RichText::new("Client preview toggles: ResultSheet diagnostics tabs are generated automatically by the engine.")
            .small()
            .color(rc.tokens.text_muted),
    );
    ui.add_space(4.0);

    for (wid, payload, lbl, checked) in [
        ("edit.diag.qq_plot", "toggle_diag:qq", tr(rc.locale, "stat_params.label.qq_diagnostics"), st.diag_qq_plot),
        ("edit.diag.residuals_table", "toggle_diag:residuals", tr(rc.locale, "stat_params.label.residuals_table"), st.diag_residuals_plot),
        (
            "edit.diag.homoscedasticity",
            "toggle_diag:homoscedasticity",
            "Test homogeneity of variances (Brown-Forsythe / Bartlett)",
            st.diag_homoscedasticity,
        ),
        (
            "edit.diag.assumption_checklist",
            "toggle_diag:checklist",
            "Generate plain-language assumption diagnostic checklist",
            st.diag_assumption_checklist,
        ),
        ("edit.diag.influence_leverage", "toggle_diag:influence", "Leverage & influence diagnostics", st.diag_influence_leverage),
    ] {
        if render_modal_checkbox(ui, rc.tokens, meta, wid, lbl, checked, &ControlEnablement::enabled()) {
            intents.push_custom_view_action("stat_params", payload);
        }
    }

    ui.add_space(6.0);
    draw_residual_qq_schematic(ui, &rc, meta);

    ui.add_space(6.0);
    render_clipped_text(
        ui,
        meta,
        ClippedTextSpec {
            widget_id: "edit.diag.tabs_preview",
            role: "label",
            text: "Expected output tabs: Main ANOVA, Multiple Comparisons, Homogeneity / Sphericity, Diagnostics",
            font_size: 11.5,
            color: rc.tokens.text_muted,
            z_layer: ZLayer::ModalDialog,
            tooltip: None,
            record_meta: true,
        },
    );
}

fn draw_residual_qq_schematic(ui: &mut Ui, rc: &TabRenderCtx<'_>, meta: &mut FrameInspectMeta) {
    let w = ui.available_width().clamp(140.0, 340.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 72.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(6), rc.tokens.panel_bg);
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, rc.tokens.border), StrokeKind::Inside);

    let title = if rc.ep.assume_gaussian { "Normal Q-Q & Homoscedasticity Preview" } else { "Rank / Kruskal-Friedman Distribution Preview" };
    ui.painter().with_clip_rect(rect.shrink(4.0)).text(
        Pos2::new(rect.left() + 8.0, rect.top() + 5.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(rc.tokens.metrics.font_ui_small_pt),
        rc.tokens.text_muted,
    );

    let plot = Rect::from_min_max(Pos2::new(rect.left() + 14.0, rect.top() + 22.0), Pos2::new(rect.right() - 14.0, rect.bottom() - 8.0));
    ui.painter().line_segment([plot.left_bottom(), plot.right_top()], Stroke::new(1.2, rc.tokens.accent));
    for (idx, frac) in [0.15_f32, 0.30, 0.45, 0.60, 0.75, 0.90].iter().enumerate() {
        let px = plot.left() + plot.width() * *frac;
        let jitter = if idx % 2 == 0 { -2.5 } else { 2.5 };
        let py = plot.bottom() - plot.height() * *frac + jitter;
        ui.painter().circle_filled(Pos2::new(px, py.clamp(plot.top() + 3.0, plot.bottom() - 3.0)), 2.8, rc.tokens.accent_strong);
    }
    let ak_title = title.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Image, true, ak_title.clone()));
    record_widget(ui, meta, &resp, "edit.diag.schematic", "image", title, None, None, true, None, ZLayer::ModalDialog, false, None);
}

fn render_method_reference_card(
    ui: &mut Ui,
    locale: &str,
    tokens: &Tokens,
    state: &mut AnovaModalEguiState,
    anova: &OneWayAnovaSpec,
    meta: &mut FrameInspectMeta,
) {
    let card_frame = egui::Frame::NONE
        .fill(tokens.panel_bg)
        .stroke(Stroke::new(1.0, tokens.border))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8));
    card_frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            let ref_hdr = tr(locale, "anova.method_ref");
            render_clipped_text(
                ui,
                meta,
                ClippedTextSpec {
                    widget_id: "edit.method_ref.title",
                    role: "label",
                    text: ref_hdr,
                    font_size: 12.0,
                    color: tokens.text_strong,
                    z_layer: ZLayer::ModalDialog,
                    tooltip: None,
                    record_meta: false,
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("×").clicked() {
                    state.show_method_reference = false;
                }
            });
        });
        let summary = match anova.method {
            OneWayMethod::OrdinaryAnova => "Ordinary ANOVA partitions total SS = SS_between + SS_within under Gaussian equal-variance assumptions.",
            OneWayMethod::BrownForsytheAndWelch => "Welch / Brown-Forsythe ANOVA adjusts denominator df for unequal group variances without pooling SD.",
            OneWayMethod::RepeatedMeasures { .. } => {
                "Repeated-Measures ANOVA removes between-subject variability; Geisser-Greenhouse corrects for sphericity violations, or REML fits incomplete blocks."
            }
            OneWayMethod::KruskalWallis => "Kruskal-Wallis compares rank sums across independent columns; pair with Dunn's test + Bonferroni/Sidak multiplicity.",
            OneWayMethod::Friedman => "Friedman's test ranks observations within matched subject rows across >= 2 treatments.",
        };
        render_clipped_text(
            ui,
            meta,
            ClippedTextSpec {
                widget_id: "edit.method_ref.body",
                role: "label",
                text: summary,
                font_size: tokens.metrics.font_ui_small_pt,
                color: tokens.text_primary,
                z_layer: ZLayer::ModalDialog,
                tooltip: None,
                record_meta: false,
            },
        );
    });
}

struct ModalFooterCtx<'a> {
    locale: &'a str,
    tokens: &'a Tokens,
    spec: &'a AnalysisSpec,
    apply_enabled: bool,
}

pub(super) struct FooterBtnSpec<'a> {
    pub(super) widget_id: &'a str,
    pub(super) label: &'a str,
    pub(super) primary: bool,
    pub(super) selected: bool,
    pub(super) enabled: bool,
}

fn render_modal_footer(
    ui: &mut Ui,
    fc: ModalFooterCtx<'_>,
    state: &mut AnovaModalEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.separator();
    ui.add_space(4.0);
    let is_create_mode = !fc.apply_enabled;
    let primary_label = if is_create_mode { tr(fc.locale, "anova.run_analysis") } else { tr(fc.locale, "anova.update_analysis") };
    let method_ref_label = tr(fc.locale, "anova.method_ref");
    let save_default_label = if state.method_saved_as_default { tr(fc.locale, "anova.saved_default") } else { tr(fc.locale, "anova.save_default") };
    let cancel_label = tr(fc.locale, "btn.cancel");

    let compact = ui.available_width() < 640.0;
    if compact {
        ui.horizontal_wrapped(|ui| {
            if render_footer_button(
                ui,
                fc.tokens,
                meta,
                FooterBtnSpec {
                    widget_id: "dialog.button.method_ref",
                    label: method_ref_label,
                    primary: false,
                    selected: state.show_method_reference,
                    enabled: true,
                },
            ) {
                state.show_method_reference = !state.show_method_reference;
            }
            if render_footer_button(
                ui,
                fc.tokens,
                meta,
                FooterBtnSpec {
                    widget_id: "dialog.button.save_default",
                    label: save_default_label,
                    primary: false,
                    selected: state.method_saved_as_default,
                    enabled: true,
                },
            ) {
                state.saved_default_spec = Some(fc.spec.clone());
                state.method_saved_as_default = true;
                intents.push_save_edit_params_as_default();
            }
            if render_footer_button(
                ui,
                fc.tokens,
                meta,
                FooterBtnSpec { widget_id: "modal.cancel", label: cancel_label, primary: false, selected: false, enabled: true },
            ) {
                intents.push_dialog_button("cancel");
            }
            if fc.apply_enabled
                && render_footer_button(
                    ui,
                    fc.tokens,
                    meta,
                    FooterBtnSpec { widget_id: "modal.apply", label: tr(fc.locale, "btn.apply"), primary: false, selected: false, enabled: true },
                )
            {
                intents.push_dialog_button("apply");
            }
            if render_footer_button(
                ui,
                fc.tokens,
                meta,
                FooterBtnSpec { widget_id: "modal.ok", label: primary_label, primary: true, selected: false, enabled: true },
            ) {
                intents.push_dialog_button("ok");
            }
        });
    } else {
        ui.horizontal(|ui| {
            if render_footer_button(
                ui,
                fc.tokens,
                meta,
                FooterBtnSpec {
                    widget_id: "dialog.button.method_ref",
                    label: method_ref_label,
                    primary: false,
                    selected: state.show_method_reference,
                    enabled: true,
                },
            ) {
                state.show_method_reference = !state.show_method_reference;
            }
            if render_footer_button(
                ui,
                fc.tokens,
                meta,
                FooterBtnSpec {
                    widget_id: "dialog.button.save_default",
                    label: save_default_label,
                    primary: false,
                    selected: state.method_saved_as_default,
                    enabled: true,
                },
            ) {
                state.saved_default_spec = Some(fc.spec.clone());
                state.method_saved_as_default = true;
                intents.push_save_edit_params_as_default();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if render_footer_button(
                    ui,
                    fc.tokens,
                    meta,
                    FooterBtnSpec { widget_id: "modal.ok", label: primary_label, primary: true, selected: false, enabled: true },
                ) {
                    intents.push_dialog_button("ok");
                }
                if fc.apply_enabled
                    && render_footer_button(
                        ui,
                        fc.tokens,
                        meta,
                        FooterBtnSpec { widget_id: "modal.apply", label: tr(fc.locale, "btn.apply"), primary: false, selected: false, enabled: true },
                    )
                {
                    intents.push_dialog_button("apply");
                }
                if render_footer_button(
                    ui,
                    fc.tokens,
                    meta,
                    FooterBtnSpec { widget_id: "modal.cancel", label: cancel_label, primary: false, selected: false, enabled: true },
                ) {
                    intents.push_dialog_button("cancel");
                }
            });
        });
    }
}

pub(super) fn render_footer_button(ui: &mut Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: FooterBtnSpec<'_>) -> bool {
    let FooterBtnSpec { widget_id, label, primary, selected, enabled } = spec;
    let min_w: f32 = if primary { 96.0 } else { 68.0 };
    let max_w = (ui.available_width() - 4.0).max(48.0);
    let fg_text = if !enabled {
        tokens.text_disabled
    } else if primary {
        tokens.on_accent
    } else {
        tokens.text_strong
    };
    let mut job = egui::text::LayoutJob::single_section(label.to_owned(), egui::TextFormat::simple(FontId::proportional(12.0), fg_text));
    job.wrap = egui::text::TextWrapping { max_width: (max_w - 16.0).max(24.0), max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let galley = ui.painter().layout_job(job);
    let ellipsized = galley.elided;
    let w = (galley.size().x + 18.0).clamp(min_w.min(max_w), max_w);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), if enabled { Sense::click() } else { Sense::hover() });

    let (bg, fg, border) = if !enabled {
        (tokens.panel_bg, tokens.text_disabled, tokens.divider)
    } else if primary {
        (
            if resp.hovered() { tokens.accent_hover } else { tokens.accent_strong },
            tokens.on_accent,
            if resp.hovered() { tokens.hover_border } else { tokens.accent_strong },
        )
    } else if selected {
        (tokens.active_bg, tokens.text_strong, tokens.accent)
    } else if resp.hovered() {
        (tokens.hover_bg, tokens.text_strong, tokens.hover_border)
    } else {
        (tokens.button_bg, tokens.text_strong, tokens.button_border)
    };
    let r = CornerRadius::same(tokens.metrics.radius_md);
    ui.painter().rect_filled(rect, r, bg);
    ui.painter().rect_stroke(rect, r, Stroke::new(1.0, border), StrokeKind::Inside);

    let text_pos = Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5);
    ui.painter().with_clip_rect(rect.shrink(3.0)).galley(text_pos, galley, fg);

    let a11y_label = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, a11y_label.clone()));
    let resp = if ellipsized { resp.on_hover_text(label) } else { resp };

    record_widget(
        ui,
        meta,
        &resp,
        widget_id,
        "button",
        label,
        None,
        None,
        enabled,
        None,
        ZLayer::ModalDialog,
        ellipsized,
        ellipsized.then_some(label),
    );
    enabled && resp.clicked()
}

fn render_multiplicity_dropdown(
    ui: &mut Ui,
    rc: &TabRenderCtx<'_>,
    reml_active: bool,
    salt: &str,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let selected_text = post_hoc_banner_name(rc.anova.multiplicity.method);
    let options: [(&str, &str, MultiplicityMethod); 12] = [
        ("tukey", "Tukey HSD", MultiplicityMethod::TukeyHsd),
        ("dunnett", "Dunnett", MultiplicityMethod::Dunnett { control_column: rc.ep.control_column }),
        ("dunn", "Dunn", MultiplicityMethod::DunnRankSum),
        ("bonferroni", "Bonferroni", MultiplicityMethod::Bonferroni),
        ("sidak", "Sidak", MultiplicityMethod::Sidak),
        ("holm_sidak", "Holm-Sidak", MultiplicityMethod::HolmSidak),
        ("fisher_lsd", "Fisher's LSD", MultiplicityMethod::FisherLsd),
        ("games_howell", "Games-Howell", MultiplicityMethod::GamesHowell),
        ("bh", "BH (FDR)", MultiplicityMethod::BenjaminiHochbergFdr),
        ("bky", "BKY (FDR)", MultiplicityMethod::TwoStageBkyFdr),
        ("by", "BY (FDR)", MultiplicityMethod::BenjaminiYekutieliFdr),
        ("none", "None", MultiplicityMethod::None),
    ];

    let cb = dropdown(salt, selected_text).width((ui.available_width() - 8.0).clamp(120.0, 260.0)).show_ui(ui, |ui| {
        for (slug, lbl, method) in options {
            let en = r1b_post_hoc_option_enabled(rc.ep.assume_gaussian, reml_active, method);
            let resp = ui.add_enabled(en.enabled, egui::Button::selectable(rc.anova.multiplicity.method == method, lbl));
            let resp = if let Some(reason) = en.disabled_reason.as_deref() { resp.on_disabled_hover_text(reason) } else { resp };
            if en.enabled && resp.clicked() {
                intents.push_custom_view_action("stat_params", format!("set_multiplicity:{slug}"));
            }
        }
    });
    record_widget(
        ui,
        meta,
        &cb.response,
        "edit.multiplicity_method",
        "combobox",
        selected_text,
        None,
        Some(json!(selected_text)),
        true,
        None,
        ZLayer::ModalDialog,
        cb.ellipsized,
        cb.ellipsized.then_some(selected_text),
    );
}

fn render_alpha_stepper(ui: &mut Ui, rc: &TabRenderCtx<'_>, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let alpha = (1.0 - rc.conf_pct / 100.0).clamp(0.001, 0.50);
    let alpha_label = tr(rc.locale, "stat_params.label.alpha");
    let step = render_stepper_row(
        ui,
        rc.tokens,
        meta,
        StepperSpec { id_prefix: "edit.alpha", label: alpha_label, value_text: &format!("{alpha:.3}"), enablement: &ControlEnablement::enabled() },
    );
    if step < 0 {
        let next_alpha = (alpha - 0.01).clamp(0.005, 0.50);
        let next_conf = ((1.0 - next_alpha) * 100.0).clamp(50.0, 99.5);
        intents.push_custom_view_action("stat_params", format!("set_confidence:{next_conf:.1}"));
    } else if step > 0 {
        let next_alpha = (alpha + 0.01).clamp(0.005, 0.50);
        let next_conf = ((1.0 - next_alpha) * 100.0).clamp(50.0, 99.5);
        intents.push_custom_view_action("stat_params", format!("set_confidence:{next_conf:.1}"));
    }
}

pub(super) struct StepperSpec<'a> {
    pub(super) id_prefix: &'a str,
    pub(super) label: &'a str,
    pub(super) value_text: &'a str,
    pub(super) enablement: &'a ControlEnablement,
}

pub(super) fn render_stepper_row(ui: &mut Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: StepperSpec<'_>) -> i8 {
    let enabled = spec.enablement.enabled;
    let mut delta: i8 = 0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let text_col = if enabled { tokens.text_strong } else { tokens.text_disabled };
        let val_font = FontId::monospace(11.5);
        let natural_galley = ui.painter().layout_no_wrap(spec.value_text.to_owned(), val_font.clone(), text_col);
        let val_w = (natural_galley.size().x + 12.0).clamp(42.0, 96.0);
        let controls_w = val_w + 72.0;
        let lbl_w = (ui.available_width() - controls_w).max(60.0);
        ui.allocate_ui(Vec2::new(lbl_w, 22.0), |ui| {
            render_clipped_text(
                ui,
                meta,
                ClippedTextSpec {
                    widget_id: &format!("{}.label", spec.id_prefix),
                    role: "label",
                    text: spec.label,
                    font_size: 12.0,
                    color: if enabled { tokens.text_primary } else { tokens.text_disabled },
                    z_layer: ZLayer::ModalDialog,
                    tooltip: spec.enablement.disabled_reason.as_deref(),
                    record_meta: false,
                },
            );
        });
        if render_modal_seg_btn(ui, tokens, meta, &format!("{}.dec", spec.id_prefix), "-", false, spec.enablement) {
            delta = -1;
        }
        let (val_rect, val_resp) = ui.allocate_exact_size(Vec2::new(val_w, 22.0), Sense::hover());
        ui.painter().rect_filled(val_rect, CornerRadius::same(tokens.metrics.radius_sm), tokens.input_bg);
        ui.painter().rect_stroke(val_rect, CornerRadius::same(tokens.metrics.radius_sm), Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
        let mut job = egui::text::LayoutJob::single_section(spec.value_text.to_owned(), egui::TextFormat::simple(val_font, text_col));
        job.wrap =
            egui::text::TextWrapping { max_width: (val_w - 6.0).max(16.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        let galley = ui.painter().layout_job(job);
        let ellipsized = galley.elided;
        let text_pos = Pos2::new(val_rect.center().x - galley.size().x * 0.5, val_rect.center().y - galley.size().y * 0.5);
        ui.painter().with_clip_rect(val_rect.shrink(2.0)).galley(text_pos, galley, text_col);
        let val_name = format!("{}: {}", spec.label, spec.value_text);
        val_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Label, enabled, val_name.clone()));
        let val_resp = if let Some(reason) = spec.enablement.disabled_reason.as_deref() {
            val_resp.on_hover_text(reason)
        } else if ellipsized {
            val_resp.on_hover_text(spec.value_text)
        } else {
            val_resp
        };
        record_widget(
            ui,
            meta,
            &val_resp,
            &format!("{}.val", spec.id_prefix),
            "textbox",
            spec.label,
            None,
            Some(json!(spec.value_text)),
            enabled,
            spec.enablement.disabled_reason.as_deref(),
            ZLayer::ModalDialog,
            ellipsized,
            ellipsized.then_some(spec.value_text),
        );
        if render_modal_seg_btn(ui, tokens, meta, &format!("{}.inc", spec.id_prefix), "+", false, spec.enablement) {
            delta = 1;
        }
    });
    delta
}

pub(super) fn render_section_title(ui: &mut Ui, tokens: &Tokens, title: &str) {
    let mut job = egui::text::LayoutJob::single_section(title.to_owned(), egui::TextFormat::simple(FontId::proportional(11.5), tokens.text_muted));
    job.wrap =
        egui::text::TextWrapping { max_width: ui.available_width().max(40.0), max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(40.0), 18.0), Sense::hover());
    ui.painter().with_clip_rect(rect).galley(Pos2::new(rect.left(), rect.center().y - galley.size().y * 0.5), galley, tokens.text_muted);
}

pub(super) struct ClippedTextSpec<'a> {
    pub(super) widget_id: &'a str,
    pub(super) role: &'a str,
    pub(super) text: &'a str,
    pub(super) font_size: f32,
    pub(super) color: Color32,
    pub(super) z_layer: ZLayer,
    pub(super) tooltip: Option<&'a str>,
    pub(super) record_meta: bool,
}

pub(super) fn render_clipped_text(ui: &mut Ui, meta: &mut FrameInspectMeta, spec: ClippedTextSpec<'_>) -> Response {
    let avail_w = ui.available_width().max(24.0);
    let font = FontId::proportional(spec.font_size);
    let mut job = egui::text::LayoutJob::single_section(spec.text.to_owned(), egui::TextFormat::simple(font, spec.color));
    job.wrap = egui::text::TextWrapping { max_width: avail_w, max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let galley = ui.painter().layout_job(job);
    let ellipsized = galley.elided;
    let h = galley.size().y.max(18.0);
    let w = galley.size().x.min(avail_w);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    ui.painter().with_clip_rect(rect).galley(Pos2::new(rect.left(), rect.center().y - galley.size().y * 0.5), galley, spec.color);
    let full = spec.text.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Label, true, full.clone()));
    let resp = if let Some(tip) = spec.tooltip {
        resp.on_hover_text(tip)
    } else if ellipsized {
        resp.on_hover_text(spec.text)
    } else {
        resp
    };
    if spec.record_meta {
        record_widget(
            ui,
            meta,
            &resp,
            spec.widget_id,
            spec.role,
            spec.text,
            None,
            Some(json!(spec.text)),
            true,
            None,
            spec.z_layer,
            ellipsized,
            ellipsized.then_some(spec.text),
        );
    }
    resp
}

pub(super) fn render_modal_checkbox(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    checked: bool,
    enablement: &ControlEnablement,
) -> bool {
    render_modal_choice_row(ui, tokens, meta, ModalChoiceSpec { widget_id, label, checked, is_radio: false, enablement })
}

pub(super) fn render_modal_radio(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    selected: bool,
    enablement: &ControlEnablement,
) -> bool {
    render_modal_choice_row(ui, tokens, meta, ModalChoiceSpec { widget_id, label, checked: selected, is_radio: true, enablement })
}

pub(super) struct ModalChoiceSpec<'a> {
    pub(super) widget_id: &'a str,
    pub(super) label: &'a str,
    pub(super) checked: bool,
    pub(super) is_radio: bool,
    pub(super) enablement: &'a ControlEnablement,
}

pub(super) fn render_modal_choice_row(ui: &mut Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: ModalChoiceSpec<'_>) -> bool {
    let ModalChoiceSpec { widget_id, label, checked, is_radio, enablement } = spec;
    let enabled = enablement.enabled;
    let avail_w = ui.available_width().max(48.0);
    let fg = if enabled { tokens.text_primary } else { tokens.text_disabled };
    let text_max_w = (avail_w - 22.0).max(20.0);
    let mut job = egui::text::LayoutJob::single_section(label.to_owned(), egui::TextFormat::simple(FontId::proportional(12.0), fg));
    job.wrap = egui::text::TextWrapping { max_width: text_max_w, max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let galley = ui.painter().layout_job(job);
    let ellipsized = galley.elided;
    let row_w = (22.0 + galley.size().x).min(avail_w);
    let vis_clip = if meta.viewport_rect.is_positive() { ui.clip_rect().intersect(meta.viewport_rect) } else { ui.clip_rect() };
    let row_top = ui.cursor().top();
    let row_bot = row_top + 22.0;
    if row_top > vis_clip.bottom() || row_bot < vis_clip.top() {
        ui.allocate_space(Vec2::new(row_w, 22.0));
        return false;
    }
    // Keep `Sense::click()` so `ScrollArea`'s outer hover sensor never swallows disabled tooltip hover; clicks stay gated by `enabled`.
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(row_w, 22.0), Sense::click());
    let resp = if enabled { resp } else { resp.on_hover_cursor(egui::CursorIcon::Default) };

    let bx = Rect::from_min_size(Pos2::new(rect.left() + 1.0, rect.center().y - 6.5), Vec2::splat(13.0));
    let border = if !enabled {
        tokens.divider
    } else if resp.hovered() {
        tokens.text_primary
    } else {
        tokens.button_border
    };

    if is_radio {
        let c = bx.center();
        ui.painter().circle(c, 6.0, if checked && enabled { tokens.accent_strong } else { tokens.input_bg }, Stroke::new(1.0, border));
        if checked {
            ui.painter().circle_filled(c, 2.5, if enabled { Color32::WHITE } else { tokens.text_disabled });
        }
    } else {
        ui.painter().rect_filled(bx, CornerRadius::same(2), if checked && enabled { tokens.accent_strong } else { tokens.input_bg });
        ui.painter().rect_stroke(bx, CornerRadius::same(2), Stroke::new(1.0, border), StrokeKind::Inside);
        if checked {
            let ck = if enabled { Color32::WHITE } else { tokens.text_disabled };
            ui.painter().line_segment([bx.left_center() + Vec2::new(3.0, 0.0), bx.center_bottom() + Vec2::new(-1.0, -3.5)], Stroke::new(1.6, ck));
            ui.painter().line_segment([bx.center_bottom() + Vec2::new(-1.0, -3.5), bx.right_top() + Vec2::new(-3.0, 3.0)], Stroke::new(1.6, ck));
        }
    }

    let text_rect = Rect::from_min_max(Pos2::new(bx.right() + 5.0, rect.top()), rect.max);
    ui.painter().with_clip_rect(text_rect).galley(Pos2::new(bx.right() + 5.0, rect.center().y - galley.size().y * 0.5), galley, fg);

    let ak_role = if is_radio { egui::Role::RadioButton } else { egui::Role::CheckBox };
    let ak_name = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::selected(ak_role, enabled, checked, ak_name.clone()));

    let tip = if let Some(reason) = enablement.disabled_reason.as_deref() {
        reason.to_owned()
    } else if ellipsized {
        label.to_owned()
    } else {
        String::new()
    };
    let resp = attach_modal_hover_tooltip(ui, resp, &tip);

    record_widget(
        ui,
        meta,
        &resp,
        widget_id,
        if is_radio { "radio" } else { "checkbox" },
        label,
        None,
        Some(json!(checked)),
        enabled,
        enablement.disabled_reason.as_deref(),
        ZLayer::ModalDialog,
        ellipsized,
        ellipsized.then_some(label),
    );
    enabled && resp.clicked()
}

pub(super) fn render_modal_seg_btn(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    selected: bool,
    enablement: &ControlEnablement,
) -> bool {
    let enabled = enablement.enabled;
    let max_w = (ui.available_width() - 2.0).max(32.0);
    let fg = if !enabled {
        tokens.text_disabled
    } else if selected {
        tokens.text_strong
    } else {
        tokens.text_primary
    };
    let mut job = egui::text::LayoutJob::single_section(label.to_owned(), egui::TextFormat::simple(FontId::proportional(11.5), fg));
    job.wrap = egui::text::TextWrapping { max_width: (max_w - 12.0).max(16.0), max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let galley = ui.painter().layout_job(job);
    let ellipsized = galley.elided;
    let w = (galley.size().x + 14.0).clamp(24.0, max_w);
    let vis_clip = if meta.viewport_rect.is_positive() { ui.clip_rect().intersect(meta.viewport_rect) } else { ui.clip_rect() };
    let row_top = ui.cursor().top();
    let row_bot = row_top + 22.0;
    let can_skip_top = !ui.layout().main_wrap() || ui.available_width() >= w;
    if row_top > vis_clip.bottom() || (can_skip_top && row_bot < vis_clip.top()) {
        ui.allocate_space(Vec2::new(w, 22.0));
        return false;
    }
    // Keep `Sense::click()` so `ScrollArea`'s outer hover sensor never swallows disabled tooltip hover; clicks stay gated by `enabled`.
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::click());
    let resp = if enabled { resp } else { resp.on_hover_cursor(egui::CursorIcon::Default) };

    let bg = if !enabled {
        tokens.panel_bg
    } else if selected {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.button_bg
    };
    let border = if !enabled {
        tokens.divider
    } else if selected {
        tokens.accent_strong
    } else {
        tokens.button_border
    };

    ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().with_clip_rect(rect.shrink(3.0)).galley(
        Pos2::new(rect.center().x - galley.size().x * 0.5, rect.center().y - galley.size().y * 0.5),
        galley,
        fg,
    );

    let ak_name = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, ak_name.clone()));

    let tip = if let Some(reason) = enablement.disabled_reason.as_deref() {
        reason.to_owned()
    } else if ellipsized {
        label.to_owned()
    } else {
        String::new()
    };
    let resp = attach_modal_hover_tooltip(ui, resp, &tip);

    record_widget(
        ui,
        meta,
        &resp,
        widget_id,
        "button",
        label,
        None,
        Some(json!(selected)),
        enabled,
        enablement.disabled_reason.as_deref(),
        ZLayer::ModalDialog,
        ellipsized,
        ellipsized.then_some(label),
    );
    enabled && resp.clicked()
}

pub(super) fn attach_modal_hover_tooltip(ui: &Ui, resp: egui::Response, tip: &str) -> egui::Response {
    if tip.is_empty() {
        return resp;
    }
    let delay = ui.style().interaction.tooltip_delay;
    let pointer_ready = ui.input(|i| !i.pointer.any_down() && i.pointer.is_still() && i.pointer.time_since_last_movement() >= delay);
    if resp.hovered() && pointer_ready {
        egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), resp.id.with("hover_tooltip"), egui::PopupAnchor::Pointer).show(|ui| {
            ui.label(tip);
        });
    }
    resp
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_modal_textbox_row(
    ui: &mut Ui,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    widget_id: &str,
    label: &str,
    hint: &str,
    draft: &mut String,
    width: f32,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).color(tokens.text_primary));
        let te_id = Id::unique(widget_id);
        let resp = ui.add_sized(Vec2::new(width, 22.0), egui::TextEdit::singleline(draft).id(te_id).hint_text(hint));
        if resp.changed() {
            intents.push_set_text_field_buffer(widget_id, draft.clone());
        }
        record_widget(ui, meta, &resp, widget_id, "textbox", label, None, Some(json!(draft.as_str())), true, None, ZLayer::ModalDialog, false, None);
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_widget(
    ui: &Ui,
    meta: &mut FrameInspectMeta,
    resp: &Response,
    widget_id: &str,
    role: &str,
    label: &str,
    command_id: Option<&str>,
    value: Option<serde_json::Value>,
    enabled: bool,
    disabled_reason: Option<&str>,
    z_layer: ZLayer,
    ellipsized: bool,
    full_label: Option<&str>,
) {
    let clipped = resp.interact_rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
    if !clipped.is_positive() {
        return;
    }
    meta.record_response(
        ui,
        resp,
        FrameInspectEntryInput { widget_id, role, label, command_id, value, enabled, disabled_reason, z_layer, ellipsized, full_label },
    );
}

pub(super) fn column_titles_for_draft(doc: &ProjectDocument, src_id: Option<&SheetId>) -> Vec<String> {
    let Some(id) = src_id else {
        return Vec::new();
    };
    let Some(ds) = doc.data_sheet(id).or_else(|| doc.result_sheet(id).and_then(|rs| rs.analyzable_data.as_ref())) else {
        return Vec::new();
    };
    match &ds.archetype {
        TableArchetype::Column(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Grouped(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Xy(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Contingency(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Survival(t) => t.cohorts.iter().map(|c| c.title.clone()).collect(),
        TableArchetype::PartsOfWhole(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::MultipleVariables(t) => t.variables.iter().map(|v| v.title.clone()).collect(),
        TableArchetype::Nested(t) => t.groups.iter().map(|g| g.title.clone()).collect(),
    }
}

fn cld_status_message(method: MultiplicityMethod) -> &'static str {
    match method {
        MultiplicityMethod::TukeyHsd
        | MultiplicityMethod::Bonferroni
        | MultiplicityMethod::Sidak
        | MultiplicityMethod::HolmSidak
        | MultiplicityMethod::GamesHowell
        | MultiplicityMethod::DunnRankSum
        | MultiplicityMethod::FisherLsd
        | MultiplicityMethod::NewmanKeuls
        | MultiplicityMethod::DunnettT3
        | MultiplicityMethod::TamhaneT2 => "CLD supported: compact grouping letters (a, b, c) connect non-significantly different groups.",
        MultiplicityMethod::Dunnett { .. } | MultiplicityMethod::LinearTrend => {
            "CLD not applicable: Dunnett/Trend compare groups against a reference or linear contrast."
        }
        MultiplicityMethod::BenjaminiHochbergFdr | MultiplicityMethod::TwoStageBkyFdr | MultiplicityMethod::BenjaminiYekutieliFdr => {
            "CLD advisory: FDR-adjusted discovery thresholds do not define transitive equivalence letters."
        }
        MultiplicityMethod::None => "CLD inactive: select an all-pairwise FWER post-hoc test to generate letter groupings.",
    }
}

fn tab_label(locale: &str, tab: AnovaModalTab) -> &str {
    match tab {
        AnovaModalTab::ExperimentalDesign => tr(locale, "stat_params.tab.experimental_design"),
        AnovaModalTab::AssumptionsAndTest => tr(locale, "stat_params.tab.repeated_measures"),
        AnovaModalTab::MultipleComparisons => tr(locale, "stat_params.tab.multiple_comparisons"),
        AnovaModalTab::Options => tr(locale, "stat_params.tab.options"),
        AnovaModalTab::ResidualsAndPlots => tr(locale, "stat_params.tab.diagnostics"),
    }
}

fn localized_modal_title(locale: &str) -> String {
    let params = tr(locale, "anova.parameters_prefix");
    let family = tr(locale, "analysis.one_way_anova");
    format!("{params}: {family}")
}

#[cfg(test)]
mod tests {
    use super::{AnovaModalEguiState, AnovaModalTab, resolve_one_way_anova_banner};
    use crate::UiApp;

    #[test]
    fn anova_modal_tabs_and_default_banner_resolve_cleanly() {
        assert_eq!(AnovaModalTab::ALL.len(), 5);
        assert_eq!(AnovaModalTab::from_slug("assumptions"), Some(AnovaModalTab::AssumptionsAndTest));
        assert_eq!(AnovaModalTab::from_slug("repeated_measures"), Some(AnovaModalTab::AssumptionsAndTest));
        let state = AnovaModalEguiState::default();
        assert_eq!(state.confidence_percent_draft, 95.0);
        let app = UiApp::new();
        assert!(resolve_one_way_anova_banner(&app).contains("ANOVA"));
    }
}
