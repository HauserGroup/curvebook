//! Application-level modals in `egui 0.37` (`R15.4`, `gui-layout-spec.md` §8.14):
//! conditional `Export Figure / Layout`, `Print`, `Unsaved Changes`, `Recovery`, `About`,
//! `Find & Replace`, and 5-category `Preferences` (with pure-Rust conflict-checked
//! `Keyboard Shortcuts` editor and strict 14-locale Beta language selector).

use curvebook_doc::{ErrorBarMetric, SheetId};
use curvebook_palette::PaletteId;
use curvebook_pdf::PdfColorSpace;
use curvebook_render::TiffCompression;
use egui::scroll_area::ScrollBarVisibility;
use egui::{Context, Key, Rect, Response, Stroke, Ui};
use serde_json::{Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, ExportFigureFormat};
use crate::i18n::{BETA_ACTIVE_LOCALES, is_rtl_locale, locale_picker_label, normalize_locale, tr};
use crate::inspect::ZLayer;
use crate::menus::shortcuts::{ShortcutBinding, normalize_shortcut_combo};
use crate::theme::UiThemeMode;
use crate::workspace_nav::NavigatorNestingMode;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::unit_spinners::{ScrubUnitFieldConfig, ScrubUnitFieldState, ScrubUnitKind, render_scrub_unit_field};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};

/// Five categories in the `Preferences` sidebar (`gui-layout-spec.md` §8.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PreferencesCategory {
    #[default]
    General,
    Appearance,
    Language,
    GraphDefaults,
    KeyboardShortcuts,
}

impl PreferencesCategory {
    pub const ALL: [Self; 5] = [Self::General, Self::Appearance, Self::Language, Self::GraphDefaults, Self::KeyboardShortcuts];

    /// Stable machine slug for layout and test assertions.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Language => "language",
            Self::GraphDefaults => "graph_defaults",
            Self::KeyboardShortcuts => "keyboard_shortcuts",
        }
    }

    #[must_use]
    const fn label_key(self) -> &'static str {
        match self {
            Self::General => "prefs.cat.general",
            Self::Appearance => "prefs.cat.appearance",
            Self::Language => "prefs.cat.language",
            Self::GraphDefaults => "prefs.cat.graph_defaults",
            Self::KeyboardShortcuts => "prefs.cat.shortcuts",
        }
    }
}

/// Resolution action chosen in the `Unsaved Changes` close confirmation modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnsavedCloseAction {
    Save,
    DontSave,
    Cancel,
}

/// Resolution decision chosen in the `Recovery` crash/session restore prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryDecision {
    RestoreSession,
    DiscardRecovery,
    Dismiss,
}

/// Metadata surfaced in the crash/session recovery modal (`RecoveryDialogSummary`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecoveryDialogSummary {
    pub source_path: Option<String>,
    pub warning_lines: Vec<String>,
    pub replayed_commands: usize,
    pub dirty_timestamp_utc: Option<String>,
}

/// Editable draft settings inside the 5-category `Preferences` modal.
#[derive(Debug, Clone, PartialEq)]
pub struct PreferencesModalState {
    pub active_category: PreferencesCategory,
    pub default_unit: ScrubUnitKind,
    pub default_show_en_dash: bool,
    pub shortcut_search: String,
    pub selected_shortcut_command: Option<String>,
    pub shortcut_combo_draft: String,
    pub shortcut_conflict_error: Option<String>,
    pub ui_scale_spinner: ScrubUnitFieldState,
    pub shortcuts_ui_only: bool,
    pub shortcut_capture_armed: bool,
    pub default_palette: PaletteId,
    pub default_error_bar: ErrorBarMetric,
}

impl Default for PreferencesModalState {
    fn default() -> Self {
        Self {
            active_category: PreferencesCategory::General,
            default_unit: ScrubUnitKind::Millimeters,
            default_show_en_dash: true,
            shortcut_search: String::new(),
            selected_shortcut_command: None,
            shortcut_combo_draft: String::new(),
            shortcut_conflict_error: None,
            ui_scale_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Percent),
            shortcuts_ui_only: false,
            shortcut_capture_armed: false,
            default_palette: PaletteId::OkabeIto,
            default_error_bar: ErrorBarMetric::Sd,
        }
    }
}

/// Persistent draft for `Find & Replace`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FindReplaceModalState {
    pub find_query: String,
    pub replace_with: String,
    pub match_case: bool,
    pub match_entire_cell: bool,
    pub match_count: usize,
    pub status_message: Option<String>,
}

/// Persistent transient UI state for `app_modals`.
#[derive(Debug, Clone, PartialEq)]
pub struct AppModalsEguiState {
    pub preferences_open: bool,
    pub preferences: PreferencesModalState,
    pub recovery_open: bool,
    pub recovery_summary: RecoveryDialogSummary,
    pub last_recovery_decision: Option<RecoveryDecision>,
    pub unsaved_changes_open: bool,
    pub pending_unsaved_action: Option<UnsavedCloseAction>,
    pub find_replace_open: bool,
    pub find_replace: FindReplaceModalState,
    pub export_width_spinner: ScrubUnitFieldState,
    pub export_height_spinner: ScrubUnitFieldState,
    pub export_dpi_spinner: ScrubUnitFieldState,
    pub export_emf_selected: bool,
    pub print_copies_spinner: ScrubUnitFieldState,
    pub print_scale_spinner: ScrubUnitFieldState,
    pub print_bleed_spinner: ScrubUnitFieldState,
    pub about_tab: u8,
}

impl Default for AppModalsEguiState {
    fn default() -> Self {
        Self {
            preferences_open: false,
            preferences: PreferencesModalState::default(),
            recovery_open: false,
            recovery_summary: RecoveryDialogSummary::default(),
            last_recovery_decision: None,
            unsaved_changes_open: false,
            pending_unsaved_action: None,
            find_replace_open: false,
            find_replace: FindReplaceModalState::default(),
            export_width_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Generic),
            export_height_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Generic),
            export_dpi_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Generic),
            export_emf_selected: false,
            print_copies_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Generic),
            print_scale_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Percent),
            print_bleed_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            about_tab: 0,
        }
    }
}

impl AppModalsEguiState {
    /// Return `true` when any shell-managed app modal flag (`Preferences`, `Recovery`, `Unsaved`, `Find`) is active.
    #[must_use]
    pub const fn any_open(&self) -> bool {
        self.preferences_open || self.recovery_open || self.unsaved_changes_open || self.find_replace_open
    }

    /// Dismiss all shell-managed app modal flags cleanly.
    pub fn close_all(&mut self) {
        self.preferences_open = false;
        self.recovery_open = false;
        self.unsaved_changes_open = false;
        self.find_replace_open = false;
    }
}

/// Frame geometry and modal capture report emitted by `app_modals`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

struct AppBtn<'a> {
    widget_id: &'a str,
    label: &'a str,
    command_id: Option<&'a str>,
    selected: bool,
    enabled: bool,
    disabled_reason: Option<&'a str>,
}

/// Render whichever application-level modal (`UnsavedChanges`, `RecoveryDialog`, `Preferences`,
/// `FindAndReplace`, `ExportFigure`, `Print`, or `About`) is currently active.
pub fn render_app_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    host_unsaved_close: bool,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    if host_unsaved_close || state.unsaved_changes_open {
        return render_unsaved_modal(ctx, app, tokens, meta, intents);
    }
    if state.recovery_open {
        return render_recovery_modal(ctx, app, state, tokens, meta, intents);
    }
    if state.preferences_open {
        return render_preferences_modal(ctx, app, state, tokens, meta, intents);
    }
    if state.find_replace_open {
        return render_find_replace_modal(ctx, app, state, tokens, meta, intents);
    }

    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::ExportFigure { sheet_id } => render_export_modal(ctx, app, state, sheet_id.clone(), tokens, meta, intents),
        ActiveDialog::Print { .. } => render_print_modal(ctx, app, state, tokens, meta, intents),
        ActiveDialog::About => render_about_modal(ctx, app, state, tokens, meta, intents),
        _ => AppModalFrameReport::default(),
    }
}

// ============================================================================
// 1. Unsaved Changes & Recovery Modals
// ============================================================================

fn render_unsaved_modal(
    ctx: &Context,
    app: &UiApp,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "w6.view.shell.unsaved_title");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 480.0);

    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_unsaved_changes_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(max_h);
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

        ui.heading(title);
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("app_modal.unsaved.scroll")
            .max_height((max_h - 104.0).max(80.0))
            .auto_shrink([false, true])
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.set_width(body_w);
                ui.set_max_width(body_w);
                ui.add(egui::Label::new(tr(locale, "w6.view.shell.unsaved_prompt")).wrap());
            });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(body_w);
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "dialog.button.save",
                    label: tr(locale, "menu.file.save"),
                    command_id: Some("project.save"),
                    selected: true,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_resolve_unsaved_changes(UnsavedCloseAction::Save);
            }
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "dialog.button.dont_save",
                    label: tr(locale, "w6.view.shell.dont_save"),
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_resolve_unsaved_changes(UnsavedCloseAction::DontSave);
            }
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "dialog.button.cancel",
                    label: tr(locale, "btn.cancel"),
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_resolve_unsaved_changes(UnsavedCloseAction::Cancel);
            }
        });
    });

    build_report(ctx, &modal_resp, "dialog.unsaved_changes", title, meta)
}

fn render_recovery_modal(
    ctx: &Context,
    app: &UiApp,
    state: &AppModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = atr(locale, "recovery.title");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 560.0);

    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_recovery_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(max_h);
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

        ui.heading(title);
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("app_modal.recovery.scroll")
            .max_height((max_h - 108.0).max(90.0))
            .auto_shrink([false, true])
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.set_width(body_w);
                ui.set_max_width(body_w);
                if let Some(src) = &state.recovery_summary.source_path {
                    ui.add(egui::Label::new(format!("{}: {src}", atr(locale, "recovery.source"))).wrap());
                }
                if let Some(ts) = &state.recovery_summary.dirty_timestamp_utc {
                    ui.add(egui::Label::new(format!("{}: {ts}", atr(locale, "recovery.timestamp"))).wrap());
                }
                ui.label(format!(
                    "Replayed commands: {}, warnings: {}",
                    state.recovery_summary.replayed_commands,
                    state.recovery_summary.warning_lines.len()
                ));
                if !state.recovery_summary.warning_lines.is_empty() {
                    ui.add_space(4.0);
                    for (idx, line) in state.recovery_summary.warning_lines.iter().enumerate() {
                        ui.add(egui::Label::new(egui::RichText::new(format!("{}. {line}", idx + 1)).color(tokens.link_red).small()).wrap());
                    }
                }
            });
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(body_w);
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "modal.restore",
                    label: atr(locale, "recovery.restore"),
                    command_id: None,
                    selected: true,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_resolve_recovery(RecoveryDecision::RestoreSession);
            }
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "modal.discard",
                    label: atr(locale, "recovery.discard"),
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_resolve_recovery(RecoveryDecision::DiscardRecovery);
            }
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "modal.cancel",
                    label: atr(locale, "recovery.dismiss"),
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_resolve_recovery(RecoveryDecision::Dismiss);
            }
        });
    });

    build_report(ctx, &modal_resp, "dialog.recovery", title, meta)
}

// ============================================================================
// 2. Five-Category Preferences Modal (§8.14)
// ============================================================================

fn render_preferences_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = atr(locale, "prefs.title");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 720.0);

    // Optional live key-capture when armed on Keyboard Shortcuts tab.
    if state.preferences.active_category == PreferencesCategory::KeyboardShortcuts
        && state.preferences.shortcut_capture_armed
        && let Some(combo) = capture_keyboard_combo(ctx)
    {
        state.preferences.shortcut_combo_draft = combo;
        state.preferences.shortcut_capture_armed = false;
        state.preferences.shortcut_conflict_error = None;
    }

    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_preferences_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(max_h);
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

        ui.heading(title);
        ui.add_space(4.0);

        // 5-Category tab/sidebar bar (wraps safely at 640x420 & 1440x600).
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(body_w);
            for &cat in &PreferencesCategory::ALL {
                let cat_lbl = atr(locale, cat.label_key());
                let wid = format!("edit.tab.prefs.{}", cat.slug());
                let selected = state.preferences.active_category == cat;
                if app_tab(ui, meta, &wid, cat_lbl, selected, tokens).clicked() {
                    state.preferences.active_category = cat;
                    state.preferences.shortcut_capture_armed = false;
                }
            }
        });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);

        let footer_h = if inner_w < 460.0 { 68.0 } else { 42.0 };
        let body_h = (max_h - 118.0 - footer_h).max(110.0);
        egui::ScrollArea::vertical()
            .id_salt("app_modal.preferences.scroll")
            .max_height(body_h)
            .auto_shrink([false, true])
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.set_width(body_w);
                ui.set_max_width(body_w);
                match state.preferences.active_category {
                    PreferencesCategory::General => render_prefs_general(ui, app, state, locale, body_w, tokens, meta, intents),
                    PreferencesCategory::Appearance => render_prefs_appearance(ui, app, state, locale, body_w, tokens, meta, intents),
                    PreferencesCategory::Language => render_prefs_language(ui, locale, body_w, tokens, meta, intents),
                    PreferencesCategory::GraphDefaults => render_prefs_graph_defaults(ui, state, locale, body_w, tokens, meta),
                    PreferencesCategory::KeyboardShortcuts => render_prefs_shortcuts(ui, app, state, locale, body_w, tokens, meta, intents),
                }
            });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(body_w);
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "modal.cancel",
                    label: tr(locale, "btn.cancel"),
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                state.preferences.shortcut_capture_armed = false;
                intents.push_close_preferences(false);
            }
            if app_button(
                ui,
                meta,
                AppBtn { widget_id: "modal.ok", label: tr(locale, "btn.ok"), command_id: None, selected: true, enabled: true, disabled_reason: None },
                tokens,
            )
            .clicked()
            {
                state.preferences.shortcut_capture_armed = false;
                intents.push_update_preferences_draft(state.preferences.clone());
                intents.push_close_preferences(true);
            }
        });
    });

    build_report(ctx, &modal_resp, "dialog.preferences", title, meta)
}

#[allow(clippy::too_many_arguments)]
fn render_prefs_general(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    locale: &str,
    body_w: f32,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.label(egui::RichText::new(atr(locale, "prefs.general.tree_heading")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        for (wid, lbl, mode) in [
            ("prefs.general.nav_separate", "5-Folder Project Tree", NavigatorNestingMode::SeparateFolders),
            ("prefs.general.nav_nested", "Nested Data + Results", NavigatorNestingMode::NestedResultsUnderData),
        ] {
            let selected = app.nav.nesting_mode == mode;
            if app_button(ui, meta, AppBtn { widget_id: wid, label: lbl, command_id: None, selected, enabled: true, disabled_reason: None }, tokens)
                .clicked()
            {
                intents.push_set_navigator_nesting(mode);
            }
        }
    });

    ui.add_space(8.0);
    ui.label(egui::RichText::new(atr(locale, "prefs.general.units_heading")).strong());
    render_default_unit_selector(ui, &mut state.preferences.default_unit, body_w, tokens, meta);

    ui.add_space(6.0);
    ui.label(format!("Active regional format preset: {}", app.locale_preset.locale));
}

#[allow(clippy::too_many_arguments)]
fn render_prefs_appearance(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    locale: &str,
    body_w: f32,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.label(egui::RichText::new(atr(locale, "prefs.appearance.theme_heading")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        for (wid, lbl, mode) in [
            ("prefs.appearance.theme.light", "Light", UiThemeMode::Light),
            ("prefs.appearance.theme.dark", "Dark", UiThemeMode::Dark),
            ("prefs.appearance.theme.high_contrast", "High Contrast (AAA)", UiThemeMode::HighContrast),
        ] {
            if app_button(
                ui,
                meta,
                AppBtn { widget_id: wid, label: lbl, command_id: None, selected: app.theme.mode == mode, enabled: true, disabled_reason: None },
                tokens,
            )
            .clicked()
            {
                intents.push(UiIntent::SetTheme(mode));
            }
        }
    });

    ui.add_space(8.0);
    let scale_pct = (f64::from(app.theme.ui_scale) * 100.0).clamp(75.0, 300.0);
    let cfg = ScrubUnitFieldConfig {
        step_per_pt: 2.0,
        decimals: 0,
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("prefs.appearance.ui_scale", atr(locale, "prefs.appearance.ui_scale"), 75.0, 300.0)
    };
    let out = render_scrub_unit_field(ui, scale_pct, &mut state.preferences.ui_scale_spinner, cfg, tokens, meta);
    if let Some(pct) = out.committed_value {
        let clamped = (pct / 100.0).clamp(0.75, 3.0) as f32;
        intents.push(UiIntent::SetUiScale(clamped));
    }

    ui.add_space(6.0);
    ui.label(format!("Active font book: proportional + tabular monospace metrics ({:.0}%)", scale_pct));
}

fn render_prefs_language(ui: &mut Ui, locale: &str, body_w: f32, tokens: &Tokens, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    ui.label(egui::RichText::new(atr(locale, "prefs.language.heading")).strong());
    ui.label(egui::RichText::new(atr(locale, "prefs.language.subtitle")).color(tokens.text_muted).small());
    ui.add_space(6.0);

    // Strictly display the 14 non-RTL Beta active locales (`BETA_ACTIVE_LOCALES`).
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        for &code in BETA_ACTIVE_LOCALES {
            if is_rtl_locale(code) {
                continue;
            }
            let display = locale_picker_label(code);
            let selected = locale == code;
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: &format!("prefs.locale.{code}"),
                    label: &display,
                    command_id: None,
                    selected,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push(UiIntent::SetLocale(code.to_owned()));
            }
        }
    });
}

fn render_prefs_graph_defaults(ui: &mut Ui, state: &mut AppModalsEguiState, locale: &str, body_w: f32, tokens: &Tokens, meta: &mut FrameInspectMeta) {
    ui.label(egui::RichText::new(atr(locale, "prefs.graph_defaults.units")).strong());
    render_default_unit_selector(ui, &mut state.preferences.default_unit, body_w, tokens, meta);

    ui.add_space(6.0);
    let en_dash_lbl = atr(locale, "prefs.graph_defaults.en_dash");
    let chk = ui.checkbox(&mut state.preferences.default_show_en_dash, en_dash_lbl);
    app_checkbox(ui, &chk, meta, "prefs.graph_defaults.en_dash", en_dash_lbl, state.preferences.default_show_en_dash);

    ui.add_space(6.0);
    ui.label(egui::RichText::new(atr(locale, "prefs.graph_defaults.palette")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        for &pal in &PaletteId::ALL {
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: &format!("prefs.graph_defaults.palette.{pal:?}"),
                    label: pal.label(),
                    command_id: None,
                    selected: state.preferences.default_palette == pal,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                state.preferences.default_palette = pal;
            }
        }
    });

    ui.add_space(6.0);
    ui.label(egui::RichText::new(atr(locale, "prefs.graph_defaults.error_bars")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        for (lbl, metric) in [
            ("SD", ErrorBarMetric::Sd),
            ("SEM", ErrorBarMetric::Sem),
            ("95% CI", ErrorBarMetric::ConfidenceInterval95),
            ("IQR", ErrorBarMetric::InterquartileRange),
            ("Range", ErrorBarMetric::Range),
        ] {
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: &format!("prefs.graph_defaults.eb.{metric:?}"),
                    label: lbl,
                    command_id: None,
                    selected: state.preferences.default_error_bar == metric,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                state.preferences.default_error_bar = metric;
            }
        }
    });
}

fn render_default_unit_selector(ui: &mut Ui, unit: &mut ScrubUnitKind, body_w: f32, tokens: &Tokens, meta: &mut FrameInspectMeta) {
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        for (slug, lbl, kind) in [
            ("pt", "Points (pt)", ScrubUnitKind::Points),
            ("mm", "Millimeters (mm)", ScrubUnitKind::Millimeters),
            ("in", "Inches (in)", ScrubUnitKind::Inches),
        ] {
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: &format!("prefs.unit.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: *unit == kind,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                *unit = kind;
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn render_prefs_shortcuts(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    locale: &str,
    body_w: f32,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        ui.label(atr(locale, "prefs.shortcuts.filter"));
        let search_resp = ui.add(egui::TextEdit::singleline(&mut state.preferences.shortcut_search).desired_width(180.0));
        app_input(ui, &search_resp, meta, "prefs.shortcuts.search", "Search shortcuts", &state.preferences.shortcut_search, true);

        let ui_only_chk = ui.checkbox(&mut state.preferences.shortcuts_ui_only, "UI Commands Only");
        app_checkbox(ui, &ui_only_chk, meta, "prefs.shortcuts.ui_only", "UI Commands Only", state.preferences.shortcuts_ui_only);

        if app_button(
            ui,
            meta,
            AppBtn {
                widget_id: "prefs.shortcuts.reset_all",
                label: "Reset All",
                command_id: None,
                selected: false,
                enabled: true,
                disabled_reason: None,
            },
            tokens,
        )
        .clicked()
        {
            state.preferences.shortcut_conflict_error = None;
            intents.push_reset_shortcut_binding(None);
        }
    });

    ui.add_space(6.0);
    // Selected command editor + pure-Rust conflict check.
    if let Some(sel_id) = state.preferences.selected_shortcut_command.clone() {
        let binding_opt = app.shortcuts.bindings().find(|b| b.command_id == sel_id);
        render_shortcut_editor_bar(ui, app, state, &sel_id, binding_opt, body_w, tokens, meta, intents);
        ui.add_space(6.0);
    } else {
        ui.label(egui::RichText::new("Select any command below to rebind its keyboard shortcut.").color(tokens.text_muted));
    }

    if let Some(err) = &state.preferences.shortcut_conflict_error {
        let err_resp = ui.add(egui::Label::new(egui::RichText::new(err).color(tokens.link_red).strong()).wrap());
        app_text(ui, &err_resp, meta, "prefs.shortcuts.conflict", err);
        ui.add_space(4.0);
    }

    ui.separator();
    let needle = state.preferences.shortcut_search.trim().to_ascii_lowercase();
    let matching: Vec<&ShortcutBinding> = app
        .shortcuts
        .bindings()
        .filter(|binding| {
            if state.preferences.shortcuts_ui_only && !binding.is_ui {
                return false;
            }
            if needle.is_empty() {
                return true;
            }
            binding.command_id.to_ascii_lowercase().contains(&needle)
                || binding.label.to_ascii_lowercase().contains(&needle)
                || binding.active_combo.to_ascii_lowercase().contains(&needle)
        })
        .collect();

    let row_h = 24.0_f32;
    let list_w = (body_w - 22.0).max(160.0);
    egui::ScrollArea::vertical()
        .id_salt("prefs.shortcuts.list")
        .max_height(150.0)
        .auto_shrink([false, true])
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
        .show_rows(ui, row_h, matching.len(), |ui, row_range| {
            ui.set_width(list_w);
            ui.set_max_width(list_w);
            for idx in row_range {
                let Some(&binding) = matching.get(idx) else {
                    continue;
                };
                let selected = state.preferences.selected_shortcut_command.as_deref() == Some(binding.command_id.as_str());
                let combo_txt = if binding.active_combo.is_empty() { "(Unbound)" } else { binding.active_combo.as_str() };
                let custom_tag = if binding.is_customized() { " *" } else { "" };
                let row_lbl = format!("{} [{}] — {combo_txt}{custom_tag}", binding.label, binding.command_id);
                let btn = egui::Button::new(egui::RichText::new(&row_lbl).small())
                    .fill(if selected { tokens.active_bg } else { tokens.surface_bg })
                    .stroke(Stroke::new(1.0, if selected { tokens.accent_strong } else { tokens.border }));
                let resp = ui.add_sized([list_w, 22.0], btn);
                app_record_visible(
                    ui,
                    &resp,
                    meta,
                    FrameInspectEntryInput {
                        widget_id: &format!("prefs.shortcuts.row.{}", binding.command_id),
                        role: "option",
                        label: &row_lbl,
                        command_id: Some(binding.command_id.as_str()),
                        value: Some(json!({ "combo": binding.active_combo, "is_ui": binding.is_ui, "selected": selected })),
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::ModalDialog,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if resp.clicked() {
                    state.preferences.selected_shortcut_command = Some(binding.command_id.clone());
                    state.preferences.shortcut_combo_draft = binding.active_combo.clone();
                    state.preferences.shortcut_conflict_error = None;
                    state.preferences.shortcut_capture_armed = false;
                }
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn render_shortcut_editor_bar(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    cmd_id: &str,
    binding_opt: Option<&ShortcutBinding>,
    body_w: f32,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        ui.label(egui::RichText::new(format!("Editing: {cmd_id}")).strong());
        let combo_resp = ui.add(egui::TextEdit::singleline(&mut state.preferences.shortcut_combo_draft).desired_width(140.0));
        app_input(ui, &combo_resp, meta, "prefs.shortcuts.combo_input", "Key combo", &state.preferences.shortcut_combo_draft, true);

        let cap_lbl = if state.preferences.shortcut_capture_armed { "Listening…" } else { "Capture Keys" };
        if app_button(
            ui,
            meta,
            AppBtn {
                widget_id: "prefs.shortcuts.capture",
                label: cap_lbl,
                command_id: None,
                selected: state.preferences.shortcut_capture_armed,
                enabled: true,
                disabled_reason: None,
            },
            tokens,
        )
        .clicked()
        {
            state.preferences.shortcut_capture_armed = !state.preferences.shortcut_capture_armed;
        }

        if app_button(
            ui,
            meta,
            AppBtn {
                widget_id: "prefs.shortcuts.apply",
                label: "Apply Shortcut",
                command_id: None,
                selected: true,
                enabled: true,
                disabled_reason: None,
            },
            tokens,
        )
        .clicked()
        {
            validate_and_queue_shortcut(app, state, cmd_id, binding_opt, intents);
        }

        if app_button(
            ui,
            meta,
            AppBtn {
                widget_id: "prefs.shortcuts.reset_one",
                label: "Reset",
                command_id: None,
                selected: false,
                enabled: true,
                disabled_reason: None,
            },
            tokens,
        )
        .clicked()
        {
            state.preferences.shortcut_conflict_error = None;
            if let Some(b) = binding_opt {
                state.preferences.shortcut_combo_draft = b.default_combo.clone();
            }
            intents.push_reset_shortcut_binding(Some(cmd_id.to_owned()));
        }
    });
}

fn validate_and_queue_shortcut(
    app: &UiApp,
    state: &mut AppModalsEguiState,
    cmd_id: &str,
    binding_opt: Option<&ShortcutBinding>,
    intents: &mut PendingUiIntents,
) {
    match normalize_shortcut_combo(&state.preferences.shortcut_combo_draft) {
        Err(syntax_err) => {
            state.preferences.shortcut_conflict_error = Some(syntax_err);
        }
        Ok(normalized) => {
            let scope = binding_opt.map(|b| b.scope);
            let conflict = match scope {
                Some(sc) => app.shortcuts.find_conflict_with_scope(&normalized, sc, Some(cmd_id)),
                None => app.shortcuts.find_conflict(&normalized, Some(cmd_id)),
            };
            if let Some(other_cmd) = conflict {
                state.preferences.shortcut_conflict_error = Some(format!("Shortcut `{normalized}` conflicts with `{other_cmd}`"));
            } else {
                state.preferences.shortcut_conflict_error = None;
                state.preferences.shortcut_combo_draft = normalized.clone();
                intents.push_set_shortcut_binding(cmd_id, normalized);
            }
        }
    }
}

fn capture_keyboard_combo(ctx: &Context) -> Option<String> {
    ctx.input(|input| {
        for ev in &input.events {
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev
                && let Some(key_name) = map_egui_key_name(*key)
            {
                let mut parts = Vec::new();
                if modifiers.ctrl || modifiers.mac_cmd || modifiers.command {
                    parts.push("CmdOrCtrl");
                }
                if modifiers.alt {
                    parts.push("Alt");
                }
                if modifiers.shift {
                    parts.push("Shift");
                }
                parts.push(key_name);
                return Some(parts.join("+"));
            }
        }
        None
    })
}

const fn map_egui_key_name(key: Key) -> Option<&'static str> {
    match key {
        Key::A => Some("A"),
        Key::B => Some("B"),
        Key::C => Some("C"),
        Key::D => Some("D"),
        Key::E => Some("E"),
        Key::F => Some("F"),
        Key::G => Some("G"),
        Key::H => Some("H"),
        Key::I => Some("I"),
        Key::J => Some("J"),
        Key::K => Some("K"),
        Key::L => Some("L"),
        Key::M => Some("M"),
        Key::N => Some("N"),
        Key::O => Some("O"),
        Key::P => Some("P"),
        Key::Q => Some("Q"),
        Key::R => Some("R"),
        Key::S => Some("S"),
        Key::T => Some("T"),
        Key::U => Some("U"),
        Key::V => Some("V"),
        Key::W => Some("W"),
        Key::X => Some("X"),
        Key::Y => Some("Y"),
        Key::Z => Some("Z"),
        Key::Num0 => Some("0"),
        Key::Num1 => Some("1"),
        Key::Num2 => Some("2"),
        Key::Num3 => Some("3"),
        Key::Num4 => Some("4"),
        Key::Num5 => Some("5"),
        Key::Num6 => Some("6"),
        Key::Num7 => Some("7"),
        Key::Num8 => Some("8"),
        Key::Num9 => Some("9"),
        Key::F1 => Some("F1"),
        Key::F2 => Some("F2"),
        Key::F3 => Some("F3"),
        Key::F4 => Some("F4"),
        Key::F5 => Some("F5"),
        Key::F6 => Some("F6"),
        Key::F7 => Some("F7"),
        Key::F8 => Some("F8"),
        Key::F9 => Some("F9"),
        Key::F10 => Some("F10"),
        Key::F11 => Some("F11"),
        Key::F12 => Some("F12"),
        _ => None,
    }
}

// ============================================================================
// 3. Find & Replace Modal
// ============================================================================

fn render_find_replace_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "menu.edit.find");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 460.0);

    let modal_resp = egui::Modal::new(egui::Id::unique("egui_shell_find_replace_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(max_h);
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

        ui.heading(title);
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("app_modal.find_replace.scroll")
            .max_height((max_h - 108.0).max(90.0))
            .auto_shrink([false, true])
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.set_width(body_w);
                ui.set_max_width(body_w);

                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(body_w);
                    ui.label("Find:");
                    let q_resp = ui.add(egui::TextEdit::singleline(&mut state.find_replace.find_query).desired_width((body_w - 110.0).max(120.0)));
                    app_input(ui, &q_resp, meta, "find_replace.query", "Find", &state.find_replace.find_query, true);
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(body_w);
                    ui.label("Replace:");
                    let r_resp = ui.add(egui::TextEdit::singleline(&mut state.find_replace.replace_with).desired_width((body_w - 110.0).max(120.0)));
                    app_input(ui, &r_resp, meta, "find_replace.replace", "Replace", &state.find_replace.replace_with, true);
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(body_w);
                    let c_chk = ui.checkbox(&mut state.find_replace.match_case, "Match Case");
                    app_checkbox(ui, &c_chk, meta, "find_replace.match_case", "Match Case", state.find_replace.match_case);
                    let e_chk = ui.checkbox(&mut state.find_replace.match_entire_cell, "Match Entire Cell");
                    app_checkbox(ui, &e_chk, meta, "find_replace.match_entire_cell", "Match Entire Cell", state.find_replace.match_entire_cell);
                });
                ui.add_space(4.0);
                let status_txt = format!("Matches found: {}", state.find_replace.match_count);
                let s_resp = ui.label(&status_txt);
                app_text(ui, &s_resp, meta, "find_replace.status", &status_txt);
            });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(body_w);
            let can_search = !state.find_replace.find_query.is_empty();
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "modal.cancel",
                    label: tr(locale, "btn.cancel"),
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_close_find_replace();
            }
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "find_replace.find",
                    label: "Find Next",
                    command_id: None,
                    selected: false,
                    enabled: can_search,
                    disabled_reason: (!can_search).then_some("Enter search text first"),
                },
                tokens,
            )
            .clicked()
            {
                intents.push_execute_find_replace(
                    false,
                    state.find_replace.find_query.clone(),
                    state.find_replace.replace_with.clone(),
                    state.find_replace.match_case,
                    state.find_replace.match_entire_cell,
                );
            }
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "find_replace.replace_all",
                    label: "Replace All",
                    command_id: None,
                    selected: true,
                    enabled: can_search,
                    disabled_reason: (!can_search).then_some("Enter search text first"),
                },
                tokens,
            )
            .clicked()
            {
                intents.push_execute_find_replace(
                    true,
                    state.find_replace.find_query.clone(),
                    state.find_replace.replace_with.clone(),
                    state.find_replace.match_case,
                    state.find_replace.match_entire_cell,
                );
            }
        });
    });

    build_report(ctx, &modal_resp, "dialog.find_replace", title, meta)
}

// ============================================================================
// 4. Conditional Export Figure / Layout Modal (§8.14)
// ============================================================================

fn render_export_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    sheet_id: SheetId,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let doc = app.document();
    let is_graph = doc.graph_sheet(&sheet_id).is_some();
    let is_layout = doc.layout_sheet(&sheet_id).is_some();
    let target_valid = is_graph || is_layout;
    let title = if is_layout { atr(locale, "export.title.layout") } else { tr(locale, "w6.view.modal.export_figure") };
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 620.0);

    let modal_resp =
        egui::Modal::new(egui::Id::unique(("egui_shell_app_modal", "dialog.export_figure"))).backdrop_color(tokens.scrim).show(ctx, |ui| {
            ui.set_min_width(inner_w);
            ui.set_max_width(inner_w);
            ui.set_max_height(max_h);
            ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

            ui.heading(title);
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt("app_modal.export.scroll")
                .max_height((max_h - 108.0).max(100.0))
                .auto_shrink([false, true])
                .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
                .show(ui, |ui| {
                    ui.set_width(body_w);
                    ui.set_max_width(body_w);
                    render_export_body(ui, app, state, &sheet_id, is_graph, is_layout, target_valid, body_w, tokens, meta, intents);
                });
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(body_w);
                if app_button(
                    ui,
                    meta,
                    AppBtn {
                        widget_id: "modal.cancel",
                        label: tr(locale, "btn.cancel"),
                        command_id: None,
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    tokens,
                )
                .clicked()
                {
                    state.export_emf_selected = false;
                    intents.push_dialog_button("cancel");
                }
                if app_button(
                    ui,
                    meta,
                    AppBtn {
                        widget_id: "modal.ok",
                        label: atr(locale, "export.commit"),
                        command_id: None,
                        selected: true,
                        enabled: target_valid,
                        disabled_reason: (!target_valid).then_some("Select a Graph or Layout sheet to export"),
                    },
                    tokens,
                )
                .clicked()
                {
                    if state.export_emf_selected {
                        let cmd = if is_layout { "layout.exportEmf" } else { "graph.exportEmf" };
                        state.export_emf_selected = false;
                        intents.push_engine(cmd, json!({ "sheet_id": sheet_id.as_str() }));
                        intents.push(UiIntent::CloseAllModals);
                    } else {
                        intents.push_dialog_button("ok");
                    }
                }
            });
        });

    build_report(ctx, &modal_resp, "dialog.export_figure", title, meta)
}

#[allow(clippy::too_many_arguments)]
fn render_export_body(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    sheet_id: &SheetId,
    is_graph: bool,
    is_layout: bool,
    target_valid: bool,
    body_w: f32,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let ef = app.canvas_dialogs.export_figure;
    let vs = &app.view_shards.dialogs_graph_misc;
    let target_kind_lbl = if is_layout {
        format!("Target Layout Sheet: {}", sheet_id.as_str())
    } else if is_graph {
        format!("Target Graph Sheet: {}", sheet_id.as_str())
    } else {
        format!("Target Sheet ({}) is not exportable as figure/layout", sheet_id.as_str())
    };
    ui.label(egui::RichText::new(target_kind_lbl).color(tokens.text_muted));
    ui.add_space(4.0);

    // Format selector (`PDF`, `SVG`, conditional `EMF`, `PNG`, `TIFF`).
    ui.label(egui::RichText::new("Format").strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        for (wid, lbl, fmt) in [
            ("export.fmt.pdf", "PDF (Vector)", ExportFigureFormat::Pdf),
            ("export.fmt.svg", "SVG (Vector)", ExportFigureFormat::Svg),
            ("export.fmt.png", "PNG (Raster)", ExportFigureFormat::Png),
            ("export.fmt.tiff_deflate", "TIFF (Deflate)", ExportFigureFormat::Tiff(TiffCompression::Deflate)),
            ("export.fmt.tiff_lzw", "TIFF (LZW)", ExportFigureFormat::Tiff(TiffCompression::Lzw)),
            ("export.fmt.tiff_none", "TIFF (None)", ExportFigureFormat::Tiff(TiffCompression::None)),
        ] {
            let selected = !state.export_emf_selected && ef.format == fmt;
            if app_button(
                ui,
                meta,
                AppBtn { widget_id: wid, label: lbl, command_id: None, selected, enabled: target_valid, disabled_reason: None },
                tokens,
            )
            .clicked()
            {
                state.export_emf_selected = false;
                let mut next = ef;
                next.format = fmt;
                intents.push_update_export_figure_state(next);
            }
        }

        let emf_lbl = if is_layout { "EMF (Layout Vector)" } else { "EMF (Graph Vector)" };
        let emf_disabled = if !target_valid { Some("EMF requires an active Graph or Layout sheet") } else { None };
        if app_button(
            ui,
            meta,
            AppBtn {
                widget_id: "export.fmt.emf",
                label: emf_lbl,
                command_id: Some(if is_layout { "layout.exportEmf" } else { "graph.exportEmf" }),
                selected: state.export_emf_selected,
                enabled: target_valid,
                disabled_reason: emf_disabled,
            },
            tokens,
        )
        .clicked()
        {
            state.export_emf_selected = true;
        }
    });

    let is_vector = state.export_emf_selected || matches!(ef.format, ExportFigureFormat::Pdf | ExportFigureFormat::Svg);
    let is_opaque_vector = state.export_emf_selected || matches!(ef.format, ExportFigureFormat::Pdf);

    ui.add_space(6.0);
    // DPI (only active for Raster PNG/TIFF; conditionally disabled for pure Vector).
    let dpi_cfg = ScrubUnitFieldConfig {
        step_per_pt: 2.0,
        decimals: 0,
        enabled: !is_vector,
        disabled_reason: is_vector.then_some("Vector formats (PDF/SVG/EMF) are resolution-independent"),
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("export.dpi", "Resolution (DPI)", 72.0, 2400.0)
    };
    let dpi_out = render_scrub_unit_field(ui, f64::from(ef.dpi), &mut state.export_dpi_spinner, dpi_cfg, tokens, meta);
    if let Some(dpi) = dpi_out.committed_value {
        let mut next = ef;
        next.dpi = dpi.clamp(72.0, 2400.0).round() as u16;
        intents.push_update_export_figure_state(next);
    }

    ui.add_space(4.0);
    // Physical Dimension units + spinners (`px`, `mm`, `in`) + Journal presets (`85/114/174 mm`).
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        ui.label("Unit:");
        for (slug, lbl) in [("px", "px"), ("mm", "mm"), ("in", "in")] {
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: &format!("export.unit.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: vs.export_unit == slug,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_custom_view_action("dialogs_graph_misc", format!("export:unit:{slug}"));
            }
        }
        ui.label("Journal Width:");
        for mm in [85.0_f64, 114.0_f64, 174.0_f64] {
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: &format!("export.preset.{}mm", mm as u32),
                    label: &format!("{} mm", mm as u32),
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                let mut next = ef;
                next.width_px = ((mm / 25.4) * f64::from(ef.dpi.max(72))).round().clamp(16.0, 9600.0) as u32;
                intents.push_update_export_figure_state(next);
            }
        }
    });

    let dpi_f = f64::from(ef.dpi.max(72));
    let (w_display, h_display, dim_max) = match vs.export_unit.as_str() {
        "mm" => ((f64::from(ef.width_px) / dpi_f) * 25.4, (f64::from(ef.height_px) / dpi_f) * 25.4, 1000.0),
        "in" => (f64::from(ef.width_px) / dpi_f, f64::from(ef.height_px) / dpi_f, 40.0),
        _ => (f64::from(ef.width_px), f64::from(ef.height_px), 9600.0),
    };
    let w_cfg = ScrubUnitFieldConfig {
        step_per_pt: 2.0,
        decimals: if vs.export_unit == "px" { 0 } else { 1 },
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("export.width", "Width", 1.0, dim_max)
    };
    let w_out = render_scrub_unit_field(ui, w_display, &mut state.export_width_spinner, w_cfg, tokens, meta);
    if let Some(val) = w_out.committed_value {
        let mut next = ef;
        let px = match vs.export_unit.as_str() {
            "mm" => (val / 25.4) * dpi_f,
            "in" => val * dpi_f,
            _ => val,
        };
        next.width_px = px.round().clamp(16.0, 9600.0) as u32;
        intents.push_update_export_figure_state(next);
    }

    let h_cfg = ScrubUnitFieldConfig {
        step_per_pt: 2.0,
        decimals: if vs.export_unit == "px" { 0 } else { 1 },
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("export.height", "Height", 1.0, dim_max)
    };
    let h_out = render_scrub_unit_field(ui, h_display, &mut state.export_height_spinner, h_cfg, tokens, meta);
    if let Some(val) = h_out.committed_value {
        let mut next = ef;
        let px = match vs.export_unit.as_str() {
            "mm" => (val / 25.4) * dpi_f,
            "in" => val * dpi_f,
            _ => val,
        };
        next.height_px = px.round().clamp(16.0, 9600.0) as u32;
        intents.push_update_export_figure_state(next);
    }

    ui.add_space(6.0);
    // Transparency (blocked for opaque PDF / EMF).
    let mut transparent = vs.export_transparent_bg && !is_opaque_vector;
    let trans_chk = ui.add_enabled(!is_opaque_vector, egui::Checkbox::new(&mut transparent, "Transparent Canvas Background"));
    let trans_tip = if is_opaque_vector { Some("PDF and EMF exports require opaque page backgrounds") } else { None };
    if let Some(reason) = trans_tip {
        trans_chk.clone().on_disabled_hover_text(reason);
    }
    meta.record_response(
        ui,
        &trans_chk,
        FrameInspectEntryInput {
            widget_id: "export.transparent_bg",
            role: "checkbox",
            label: "Transparent Canvas Background",
            command_id: None,
            value: Some(Value::Bool(transparent)),
            enabled: !is_opaque_vector,
            disabled_reason: trans_tip,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
    if trans_chk.changed() && !is_opaque_vector {
        intents.push_custom_view_action("dialogs_graph_misc", "export:transparent_bg");
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        ui.label("Color Space:");
        for (slug, lbl, val) in
            [("srgb", "sRGB", "sRGB"), ("p3", "Display-P3", "Display-P3"), ("gray", "Grayscale", "Grayscale"), ("cmyk", "CMYK-Proof", "CMYK-Proof")]
        {
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: &format!("export.cs.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: vs.export_color_space == val,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_custom_view_action("dialogs_graph_misc", format!("export:cs:{slug}"));
            }
        }
    });
}

// ============================================================================
// 5. Print & About Modals (§8.14)
// ============================================================================

fn render_print_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "toolbar.print");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 560.0);
    let pd = app.canvas_dialogs.print.clone();

    let modal_resp = egui::Modal::new(egui::Id::unique(("egui_shell_app_modal", "dialog.print"))).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(max_h);
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

        ui.heading(title);
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("app_modal.print.scroll")
            .max_height((max_h - 108.0).max(100.0))
            .auto_shrink([false, true])
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.set_width(body_w);
                ui.set_max_width(body_w);

                ui.label(egui::RichText::new("Printer Queue").strong());
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(body_w);
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "print.queue.default",
                            label: "OS Default / PDF Spool",
                            command_id: None,
                            selected: pd.selected_printer.is_none(),
                            enabled: true,
                            disabled_reason: None,
                        },
                        tokens,
                    )
                    .clicked()
                    {
                        let mut next = pd.clone();
                        next.selected_printer = None;
                        intents.push_update_print_dialog_state(next);
                    }
                    for (idx, q) in pd.discovered_printers.iter().enumerate() {
                        let selected = pd.selected_printer.as_deref() == Some(q.as_str());
                        if app_button(
                            ui,
                            meta,
                            AppBtn {
                                widget_id: &format!("print.queue.{idx}"),
                                label: q,
                                command_id: None,
                                selected,
                                enabled: true,
                                disabled_reason: None,
                            },
                            tokens,
                        )
                        .clicked()
                        {
                            let mut next = pd.clone();
                            next.selected_printer = Some(q.clone());
                            intents.push_update_print_dialog_state(next);
                        }
                    }
                });

                ui.add_space(6.0);
                let copies_cfg = ScrubUnitFieldConfig {
                    step_per_pt: 0.25,
                    decimals: 0,
                    allow_unit_switch: false,
                    ..ScrubUnitFieldConfig::new("print.copies", "Copies", 1.0, 999.0)
                };
                let c_out = render_scrub_unit_field(ui, f64::from(pd.copies), &mut state.print_copies_spinner, copies_cfg, tokens, meta);
                if let Some(val) = c_out.committed_value {
                    let mut next = pd.clone();
                    next.update_copies(val.round().clamp(1.0, 999.0) as u16);
                    intents.push_update_print_dialog_state(next);
                }

                let scale_cfg = ScrubUnitFieldConfig {
                    step_per_pt: 1.0,
                    decimals: 0,
                    allow_unit_switch: false,
                    ..ScrubUnitFieldConfig::new("print.scale_percent", "Physical Scale", 10.0, 400.0)
                };
                let s_out = render_scrub_unit_field(ui, pd.job.scale_percent, &mut state.print_scale_spinner, scale_cfg, tokens, meta);
                if let Some(val) = s_out.committed_value {
                    let mut next = pd.clone();
                    next.job.scale_percent = val.clamp(10.0, 400.0);
                    intents.push_update_print_dialog_state(next);
                }

                ui.add_space(4.0);
                let mut crops = pd.job.crop_marks;
                let crop_chk = ui.checkbox(&mut crops, "Draw Publication Crop Marks");
                app_checkbox(ui, &crop_chk, meta, "print.crop_marks", "Draw Publication Crop Marks", crops);
                if crop_chk.changed() {
                    let mut next = pd.clone();
                    next.job.crop_marks = crops;
                    intents.push_update_print_dialog_state(next);
                }

                if let Some(val) = super::graph_modals::render_length_spinner_pt(
                    ui,
                    super::graph_modals::LengthSpinnerSpec {
                        widget_id: "print.bleed_pt",
                        label: "Bleed",
                        value_pt: pd.job.bleed_pt,
                        min_pt: 0.0,
                        max_pt: 36.0,
                        step_pt: 0.5,
                        enabled: crops,
                        disabled_reason: (!crops).then_some("Enable Crop Marks first to configure bleed"),
                    },
                    &mut state.print_bleed_spinner,
                    tokens,
                    meta,
                ) {
                    let mut next = pd.clone();
                    next.job.bleed_pt = val;
                    intents.push_update_print_dialog_state(next);
                }

                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(body_w);
                    ui.label("Color Space:");
                    for (lbl, cs) in [("sRGB", PdfColorSpace::Srgb), ("CMYK", PdfColorSpace::Cmyk), ("Grayscale", PdfColorSpace::Grayscale)] {
                        if app_button(
                            ui,
                            meta,
                            AppBtn {
                                widget_id: &format!("print.cs.{cs:?}"),
                                label: lbl,
                                command_id: None,
                                selected: pd.job.color_space == cs,
                                enabled: true,
                                disabled_reason: None,
                            },
                            tokens,
                        )
                        .clicked()
                        {
                            let mut next = pd.clone();
                            next.job.color_space = cs;
                            intents.push_update_print_dialog_state(next);
                        }
                    }
                });

                ui.add_space(4.0);
                let gui_only = app.is_gui_only();
                let mut submit_os = pd.submit_to_os && !gui_only;
                let os_chk = ui.add_enabled(!gui_only, egui::Checkbox::new(&mut submit_os, "Submit directly to OS print queue"));
                let os_reason = gui_only.then_some("Blocked under --gui-only: spool PDF cleanly in-memory");
                if let Some(reason) = os_reason {
                    os_chk.clone().on_disabled_hover_text(reason);
                }
                meta.record_response(
                    ui,
                    &os_chk,
                    FrameInspectEntryInput {
                        widget_id: "print.submit_to_os",
                        role: "checkbox",
                        label: "Submit directly to OS print queue",
                        command_id: None,
                        value: Some(Value::Bool(submit_os)),
                        enabled: !gui_only,
                        disabled_reason: os_reason,
                        z_layer: ZLayer::ModalDialog,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if os_chk.changed() && !gui_only {
                    let mut next = pd;
                    next.submit_to_os = submit_os;
                    intents.push_update_print_dialog_state(next);
                }
            });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(body_w);
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "modal.cancel",
                    label: tr(locale, "btn.cancel"),
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_dialog_button("cancel");
            }
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "modal.ok",
                    label: atr(locale, "print.commit"),
                    command_id: None,
                    selected: true,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_dialog_button("ok");
            }
        });
    });

    build_report(ctx, &modal_resp, "dialog.print", title, meta)
}

fn format_about_version_line(version: &str, build_sha: Option<&str>) -> String {
    let short_sha = build_sha.map(str::trim).filter(|s| !s.is_empty()).map(|s| s.chars().take(12).collect::<String>());
    match short_sha {
        Some(sha) => format!("CurveBook {version} ({sha})"),
        None => format!("CurveBook {version}"),
    }
}

fn format_about_diagnostic_summary(app: &UiApp, build_sha: Option<&str>) -> String {
    let version_line = format_about_version_line(env!("CARGO_PKG_VERSION"), build_sha);
    format!(
        "{version_line}\nTheme: {:?} ({:.0}%)\nLocale: {}\nOS: {} ({})",
        app.theme.mode,
        app.theme.ui_scale * 100.0,
        normalize_locale(&app.locale),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

fn render_about_modal(
    ctx: &Context,
    app: &UiApp,
    state: &AppModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let _ = state.about_tab;
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "menu.help.about");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 340.0);
    let build_sha = option_env!("CURVEBOOK_BUILD_SHA");
    let version_line = format_about_version_line(env!("CARGO_PKG_VERSION"), build_sha);
    let copied_id = egui::Id::unique(("egui_shell_app_modal", "dialog.about.copied"));

    let modal_resp = egui::Modal::new(egui::Id::unique(("egui_shell_app_modal", "dialog.about"))).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(max_h);
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

        ui.heading(title);
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .id_salt("app_modal.about.scroll")
            .max_height((max_h - 108.0).max(80.0))
            .auto_shrink([false, true])
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.set_width(body_w);
                ui.set_max_width(body_w);
                ui.horizontal(|ui| {
                    let (mark_rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
                    crate::egui_shell::brand::paint_mark(ui, mark_rect);
                    let text_w = (body_w - 44.0 - ui.spacing().item_spacing.x).max(80.0);
                    ui.vertical(|ui| {
                        ui.set_max_width(text_w);
                        ui.label(egui::RichText::new(&version_line).strong());
                        ui.add(egui::Label::new("Native scientific biostatistics, curve fitting, and publication graphing suite.").wrap());
                    });
                });
                ui.add_space(6.0);
                ui.label(format!("Active theme: {:?}, UI scale: {:.0}%", app.theme.mode, app.theme.ui_scale * 100.0));
                ui.label(format!("Active Beta locales supported: {} (RTL locales excluded at Beta)", BETA_ACTIVE_LOCALES.len()));
                ui.label(format!("Registered keyboard shortcuts: {}", app.shortcuts.bindings().count()));
            });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(body_w);
            let copied = ctx.data(|d| d.get_temp::<bool>(copied_id).unwrap_or(false));
            let copy_lbl = if copied { "Copied" } else { "Copy version info" };
            let copy_resp = ui.button(copy_lbl);
            app_record_visible(
                ui,
                &copy_resp,
                meta,
                FrameInspectEntryInput {
                    widget_id: "dialog.about.copy_version",
                    role: "button",
                    label: copy_lbl,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if copy_resp.clicked() {
                ui.ctx().copy_text(format_about_diagnostic_summary(app, build_sha));
                ctx.data_mut(|d| d.insert_temp(copied_id, true));
            }

            let ok_lbl = tr(locale, "btn.ok");
            let ok_resp = ui.button(ok_lbl);
            app_record_visible(
                ui,
                &ok_resp,
                meta,
                FrameInspectEntryInput {
                    widget_id: "dialog.button.ok",
                    role: "button",
                    label: ok_lbl,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if ok_resp.clicked() {
                ctx.data_mut(|d| d.insert_temp(copied_id, false));
                intents.push_ui("dialog.ok", json!({}));
            }

            let cancel_lbl = tr(locale, "btn.cancel");
            let cancel_resp = ui.button(cancel_lbl);
            app_record_visible(
                ui,
                &cancel_resp,
                meta,
                FrameInspectEntryInput {
                    widget_id: "dialog.button.cancel",
                    role: "button",
                    label: cancel_lbl,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if cancel_resp.clicked() {
                intents.push_ui("dialog.cancel", json!({}));
            }
        });
    });

    build_report(ctx, &modal_resp, "dialog.about", title, meta)
}

// ============================================================================
// Shared Layout Bounds, Recording & Localization Helpers
// ============================================================================

fn modal_bounds(ctx: &Context, preferred_w: f32) -> (f32, f32, f32) {
    let screen = ctx.content_rect();
    let max_w = (screen.width() - 28.0).clamp(260.0, 920.0);
    let max_h = (screen.height() - 28.0).clamp(220.0, 640.0);
    let inner_w = max_w.min(preferred_w).max(260.0);
    let body_w = (inner_w - 28.0).max(180.0);
    (inner_w, max_h, body_w)
}

fn build_report<T>(
    ctx: &Context,
    modal_resp: &egui::ModalResponse<T>,
    widget_id: &str,
    title: &str,
    meta: &mut FrameInspectMeta,
) -> AppModalFrameReport {
    modal_resp.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Dialog, true, title));
    let capture = finalize_egui_modal_response(ctx, modal_resp, ModalCaptureSpec { widget_id, title, z_dialog: ZLayer::ModalDialog }, meta);
    AppModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn app_record_visible(ui: &Ui, resp: &Response, meta: &mut FrameInspectMeta, input: FrameInspectEntryInput<'_>) {
    let clipped = resp.interact_rect.intersect(ui.clip_rect());
    if clipped.width() > 0.0 && clipped.height() > 0.0 {
        meta.record_response(ui, resp, input);
    }
}

fn app_button(ui: &mut Ui, meta: &mut FrameInspectMeta, entry: AppBtn<'_>, tokens: &Tokens) -> Response {
    let btn = egui::Button::new(entry.label)
        .fill(if entry.selected { tokens.active_bg } else { tokens.panel_bg })
        .stroke(Stroke::new(1.0, if entry.selected { tokens.accent_strong } else { tokens.border }));
    let mut resp = ui.add_enabled(entry.enabled, btn);
    if let Some(reason) = entry.disabled_reason {
        resp = resp.on_disabled_hover_text(reason);
    }
    app_record_visible(
        ui,
        &resp,
        meta,
        FrameInspectEntryInput {
            widget_id: entry.widget_id,
            role: "button",
            label: entry.label,
            command_id: entry.command_id,
            value: Some(json!({ "selected": entry.selected })),
            enabled: entry.enabled,
            disabled_reason: entry.disabled_reason,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
    resp
}

fn app_tab(ui: &mut Ui, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, selected: bool, tokens: &Tokens) -> Response {
    let btn = egui::Button::new(label)
        .fill(if selected { tokens.active_bg } else { tokens.surface_bg })
        .stroke(Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { tokens.accent_strong } else { tokens.border }));
    let resp = ui.add(btn);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
    app_record_visible(
        ui,
        &resp,
        meta,
        FrameInspectEntryInput {
            widget_id,
            role: "tab",
            label,
            command_id: None,
            value: Some(json!({ "selected": selected })),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
    resp
}

fn app_checkbox(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, checked: bool) {
    app_record_visible(
        ui,
        resp,
        meta,
        FrameInspectEntryInput {
            widget_id,
            role: "checkbox",
            label,
            command_id: None,
            value: Some(Value::Bool(checked)),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}

fn app_input(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, text: &str, enabled: bool) {
    app_record_visible(
        ui,
        resp,
        meta,
        FrameInspectEntryInput {
            widget_id,
            role: "textbox",
            label,
            command_id: None,
            value: Some(Value::String(text.to_owned())),
            enabled,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}

fn app_text(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, text: &str) {
    meta.record_response(
        ui,
        resp,
        FrameInspectEntryInput {
            widget_id,
            role: "text",
            label: text,
            command_id: None,
            value: None,
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}

fn atr(locale: &str, key: &str) -> &'static str {
    match (key, locale) {
        ("prefs.title", "de") => "Einstellungen",
        ("prefs.title", "es") => "Preferencias",
        ("prefs.title", "fr") => "Préférences",
        ("prefs.title", "ja") => "環境設定",
        ("prefs.title", "ko") => "환경설정",
        ("prefs.title", "pt-BR") => "Preferências",
        ("prefs.title", "zh-CN") => "首选项",
        ("prefs.title", "zh-TW") => "偏好設定",
        ("prefs.title", _) => "Preferences",

        ("prefs.cat.general", "de") => "Allgemein",
        ("prefs.cat.general", "ja") => "一般",
        ("prefs.cat.general", _) => "General",
        ("prefs.cat.appearance", "de") => "Darstellung",
        ("prefs.cat.appearance", "ja") => "外観",
        ("prefs.cat.appearance", _) => "Appearance",
        ("prefs.cat.language", "de") => "Sprache",
        ("prefs.cat.language", "ja") => "言語",
        ("prefs.cat.language", _) => "Language",
        ("prefs.cat.graph_defaults", "de") => "Diagramm-Standards",
        ("prefs.cat.graph_defaults", "ja") => "グラフ既定値",
        ("prefs.cat.graph_defaults", _) => "Graph Defaults",
        ("prefs.cat.shortcuts", "de") => "Tastenkürzel",
        ("prefs.cat.shortcuts", "ja") => "ショートカット",
        ("prefs.cat.shortcuts", _) => "Keyboard Shortcuts",

        ("prefs.general.tree_heading", _) => "Project Navigator Organization",
        ("prefs.general.units_heading", _) => "Default Physical Unit",
        ("prefs.appearance.theme_heading", _) => "Theme & Contrast",
        ("prefs.appearance.ui_scale", _) => "UI Scale",
        ("prefs.language.heading", _) => "Application Language (14 Active Beta Locales)",
        ("prefs.language.subtitle", _) => "Right-to-left (RTL) locales remain hidden during the Beta release.",
        ("prefs.graph_defaults.units", _) => "Graph Default Dimension Unit",
        ("prefs.graph_defaults.en_dash", _) => "Use Typographic En-Dash Minus Signs (−)",
        ("prefs.graph_defaults.palette", _) => "Default Color Palette",
        ("prefs.graph_defaults.error_bars", _) => "Default Error Bar Metric",
        ("prefs.shortcuts.filter", _) => "Filter:",

        ("recovery.title", _) => "Crash & Session Recovery",
        ("recovery.source", _) => "Project File",
        ("recovery.timestamp", _) => "Dirty Snapshot (UTC)",
        ("recovery.restore", _) => "Restore Session",
        ("recovery.discard", _) => "Discard Recovery",
        ("recovery.dismiss", _) => "Dismiss",

        ("export.title.layout", _) => "Export Page Layout",
        ("export.commit", _) => "Export",
        ("print.commit", _) => "Print / Spool PDF",

        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::{AppModalsEguiState, PreferencesCategory};
    use crate::i18n::{BETA_ACTIVE_LOCALES, is_rtl_locale};

    #[test]
    fn beta_locale_list_is_strictly_14_non_rtl() {
        assert_eq!(BETA_ACTIVE_LOCALES.len(), 14);
        assert!(BETA_ACTIVE_LOCALES.iter().all(|loc| !is_rtl_locale(loc)));
        assert_eq!(PreferencesCategory::ALL.len(), 5);
        assert!(!AppModalsEguiState::default().any_open());
    }
}
