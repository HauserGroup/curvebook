//! Application-level modals in `egui 0.37` (`R15.4`, `gui-layout-spec.md` §8.14):
//! conditional `Export Figure / Layout`, `Print`, `Unsaved Changes`, `About`,
//! `Find & Replace`, and 4-category `Preferences` (with pure-Rust conflict-checked
//! staged `Keyboard Shortcuts` editor and strict 14-locale Beta language selector).

use curvebook_doc::SheetId;
use curvebook_pdf::PdfColorSpace;
use curvebook_render::TiffCompression;
use egui::scroll_area::ScrollBarVisibility;
use egui::{Context, Key, Rect, Response, Stroke, Ui};
use serde_json::{Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, ExportFigureFormat};
use crate::i18n::{BETA_ACTIVE_LOCALES, is_rtl_locale, locale_picker_label, normalize_locale, tr};
use crate::inspect::ZLayer;
use crate::menus::shortcuts::{ShortcutBinding, ShortcutRegistry, normalize_shortcut_combo};
use crate::theme::UiThemeMode;
use crate::workspace_nav::NavigatorNestingMode;

use super::inspect_bridge::ModalCaptureReport;
use super::ui_kit::{self, ModalButtonSpec, ScrolledModalCfg, show_scrolled_modal};
use super::unit_spinners::{ScrubUnitFieldConfig, ScrubUnitFieldState, ScrubUnitKind, render_scrub_unit_field};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};

/// Four active categories in the `Preferences` sidebar (`gui-layout-spec.md` §8.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PreferencesCategory {
    #[default]
    General,
    Appearance,
    Language,
    KeyboardShortcuts,
}

impl PreferencesCategory {
    pub const ALL: [Self; 4] = [Self::General, Self::Appearance, Self::Language, Self::KeyboardShortcuts];

    /// Stable machine slug for layout and test assertions.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Appearance => "appearance",
            Self::Language => "language",
            Self::KeyboardShortcuts => "keyboard_shortcuts",
        }
    }

    #[must_use]
    const fn label_key(self) -> &'static str {
        match self {
            Self::General => "prefs.cat.general",
            Self::Appearance => "prefs.cat.appearance",
            Self::Language => "prefs.cat.language",
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

/// Snapshot of live preferences captured on modal open so `Cancel` / `Escape` reverts live previews.
#[derive(Debug, Clone, PartialEq)]
pub struct PreferencesSnapshot {
    pub locale: String,
    pub theme_mode: UiThemeMode,
    pub ui_scale: f32,
    pub nesting_mode: NavigatorNestingMode,
}

/// Editable draft settings inside the 4-category `Preferences` modal.
#[derive(Debug, Clone, PartialEq)]
pub struct PreferencesModalState {
    pub active_category: PreferencesCategory,
    pub shortcut_search: String,
    pub selected_shortcut_command: Option<String>,
    pub shortcut_combo_draft: String,
    pub shortcut_conflict_error: Option<String>,
    pub ui_scale_spinner: ScrubUnitFieldState,
    pub shortcuts_ui_only: bool,
    pub shortcut_capture_armed: bool,
    /// Staged shortcut bindings edited inside the modal and committed to `app.shortcuts` only on OK.
    pub staged_shortcuts: Option<ShortcutRegistry>,
    /// Live preference values captured when `Preferences` opens so `Cancel` / `Escape` restores them.
    pub initial_snapshot: Option<PreferencesSnapshot>,
}

impl Default for PreferencesModalState {
    fn default() -> Self {
        Self {
            active_category: PreferencesCategory::General,
            shortcut_search: String::new(),
            selected_shortcut_command: None,
            shortcut_combo_draft: String::new(),
            shortcut_conflict_error: None,
            ui_scale_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Percent),
            shortcuts_ui_only: false,
            shortcut_capture_armed: false,
            staged_shortcuts: None,
            initial_snapshot: None,
        }
    }
}

impl PreferencesModalState {
    /// Capture live `locale`, `theme_mode`, `ui_scale`, and `nesting_mode` when opening Preferences.
    pub fn capture_live_snapshot(&mut self, locale: &str, theme_mode: UiThemeMode, ui_scale: f32, nesting_mode: NavigatorNestingMode) {
        self.initial_snapshot = Some(PreferencesSnapshot { locale: locale.to_owned(), theme_mode, ui_scale, nesting_mode });
    }

    /// Reset transient UI scale spinner state back to `ScrubUnitFieldState::new(Percent)`.
    pub fn reset_scale_spinner(&mut self) {
        self.ui_scale_spinner = ScrubUnitFieldState::new(ScrubUnitKind::Percent);
    }

    /// Snapshot `shortcuts` into the modal draft and clear transient editor fields when opening Preferences.
    pub fn begin_shortcut_session(&mut self, shortcuts: &ShortcutRegistry) {
        self.staged_shortcuts = Some(shortcuts.clone());
        self.selected_shortcut_command = None;
        self.shortcut_combo_draft.clear();
        self.shortcut_conflict_error = None;
        self.shortcut_capture_armed = false;
    }

    /// Discard any uncommitted shortcut edits and reset transient shortcut editor fields.
    pub fn discard_shortcut_draft(&mut self) {
        self.staged_shortcuts = None;
        self.selected_shortcut_command = None;
        self.shortcut_combo_draft.clear();
        self.shortcut_conflict_error = None;
        self.shortcut_capture_armed = false;
    }

    /// Borrow the staged shortcut registry when active, falling back to `fallback`.
    #[must_use]
    pub fn effective_shortcuts<'a>(&'a self, fallback: &'a ShortcutRegistry) -> &'a ShortcutRegistry {
        self.staged_shortcuts.as_ref().unwrap_or(fallback)
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
    pub checkpoint_pending_restore_id: Option<String>,
}

impl Default for AppModalsEguiState {
    fn default() -> Self {
        Self {
            preferences_open: false,
            preferences: PreferencesModalState::default(),
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
            checkpoint_pending_restore_id: None,
        }
    }
}

impl AppModalsEguiState {
    /// Open the `Preferences` modal, snapshotting `shortcuts` into the staged draft.
    pub fn open_preferences(&mut self, shortcuts: &ShortcutRegistry, category: Option<PreferencesCategory>) {
        if let Some(cat) = category {
            self.preferences.active_category = cat;
        }
        self.preferences.begin_shortcut_session(shortcuts);
        self.preferences_open = true;
    }

    /// Return `true` when any shell-managed app modal flag (`Preferences`, `Unsaved`, `Find`) is active.
    #[must_use]
    pub const fn any_open(&self) -> bool {
        self.preferences_open || self.unsaved_changes_open || self.find_replace_open
    }

    /// Dismiss all shell-managed app modal flags cleanly and discard any staged preference drafts.
    pub fn close_all(&mut self) {
        self.preferences.discard_shortcut_draft();
        self.preferences.initial_snapshot = None;
        self.preferences.reset_scale_spinner();
        self.preferences_open = false;
        self.unsaved_changes_open = false;
        self.find_replace_open = false;
        self.find_replace.match_count = 0;
        self.find_replace.status_message = None;
        self.export_emf_selected = false;
        self.checkpoint_pending_restore_id = None;
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

#[derive(Clone, Copy)]
struct PrefsTabCtx<'a> {
    app: &'a UiApp,
    locale: &'a str,
    body_w: f32,
    tokens: &'a Tokens,
}

#[derive(Clone, Copy)]
struct ShortcutEditorSpec<'a> {
    active_shortcuts: &'a ShortcutRegistry,
    cmd_id: &'a str,
    binding_opt: Option<&'a ShortcutBinding>,
    body_w: f32,
    tokens: &'a Tokens,
}

#[derive(Clone, Copy)]
struct ExportBodyCtx<'a> {
    app: &'a UiApp,
    sheet_id: &'a SheetId,
    is_graph: bool,
    is_layout: bool,
    target_valid: bool,
    body_w: f32,
    tokens: &'a Tokens,
}

/// Render whichever application-level modal (`UnsavedChanges`, `Preferences`,
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
    if state.preferences_open {
        return render_preferences_modal(ctx, app, state, tokens, meta, intents);
    }
    if state.find_replace_open {
        return render_find_replace_modal(ctx, app, state, tokens, meta, intents);
    }

    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::Checkpoints) {
        state.checkpoint_pending_restore_id = None;
    }

    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::ExportFigure { sheet_id } => render_export_modal(ctx, app, state, sheet_id.clone(), tokens, meta, intents),
        ActiveDialog::Print { .. } => render_print_modal(ctx, app, state, tokens, meta, intents),
        ActiveDialog::Checkpoints => render_checkpoints_modal(ctx, app, state, tokens, meta, intents),
        ActiveDialog::About => render_about_modal(ctx, app, state, tokens, meta, intents),
        ActiveDialog::Feedback => render_feedback_modal(ctx, app, tokens, meta, intents),
        _ => AppModalFrameReport::default(),
    }
}

// ============================================================================
// 1. Unsaved Changes Modal
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

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::ResolveUnsavedChanges { action: UnsavedCloseAction::Cancel });
    }

    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique("egui_shell_unsaved_changes_modal"),
            scroll_id: "app_modal.unsaved.scroll",
            widget_id: "dialog.unsaved_changes",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 104.0).max(80.0),
            footer_separator: true,
        },
        intents,
        |_, _, _| {},
        |ui, _, _| {
            ui.add(egui::Label::new(tr(locale, "w6.view.shell.unsaved_prompt")).wrap());
        },
        |ui, meta, intents| {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(body_w);
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
                    intents.push(UiIntent::ResolveUnsavedChanges { action: UnsavedCloseAction::Cancel });
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
                    intents.push(UiIntent::ResolveUnsavedChanges { action: UnsavedCloseAction::DontSave });
                }
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
                    intents.push(UiIntent::ResolveUnsavedChanges { action: UnsavedCloseAction::Save });
                }
            });
        },
    );

    report_from_capture(capture)
}

// ============================================================================
// 2. Four-Category Preferences Modal (§8.14)
// ============================================================================

fn render_preferences_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    if state.preferences.staged_shortcuts.is_none() {
        state.preferences.staged_shortcuts = Some(app.shortcuts.clone());
    }
    if state.preferences.initial_snapshot.is_none() {
        state.preferences.capture_live_snapshot(&app.locale, app.theme.mode, app.theme.ui_scale, app.nav.nesting_mode);
    }
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "prefs.title");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 720.0);

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        if state.preferences.shortcut_capture_armed {
            state.preferences.shortcut_capture_armed = false;
        } else {
            state.preferences.shortcut_capture_armed = false;
            intents.push(UiIntent::ClosePreferences { save: false });
        }
    }

    // Optional live key-capture when armed on Keyboard Shortcuts tab.
    if state.preferences.active_category == PreferencesCategory::KeyboardShortcuts
        && state.preferences.shortcut_capture_armed
        && let Some(combo) = capture_keyboard_combo(ctx)
    {
        state.preferences.shortcut_combo_draft = combo;
        state.preferences.shortcut_capture_armed = false;
        state.preferences.shortcut_conflict_error = None;
    }

    let footer_h = if inner_w < 460.0 { 68.0 } else { 42.0 };
    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique("egui_shell_preferences_modal"),
            scroll_id: "app_modal.preferences.scroll",
            widget_id: "dialog.preferences",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 118.0 - footer_h).max(110.0),
            footer_separator: true,
        },
        &mut (state, intents),
        |ui, meta, (state, _)| {
            // 4-Category tab/sidebar bar (wraps safely at 640x420 & 1440x600).
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(body_w);
                for &cat in &PreferencesCategory::ALL {
                    let cat_lbl = tr(locale, cat.label_key());
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
        },
        |ui, meta, (state, intents)| {
            let prefs_ctx = PrefsTabCtx { app, locale, body_w, tokens };
            match state.preferences.active_category {
                PreferencesCategory::General => render_prefs_general(ui, app, locale, body_w, tokens, meta, intents),
                PreferencesCategory::Appearance => render_prefs_appearance(ui, prefs_ctx, state, meta, intents),
                PreferencesCategory::Language => render_prefs_language(ui, locale, body_w, tokens, meta, intents),
                PreferencesCategory::KeyboardShortcuts => render_prefs_shortcuts(ui, prefs_ctx, state, meta, intents),
            }
        },
        |ui, meta, (state, intents)| {
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
                    intents.push(UiIntent::ClosePreferences { save: false });
                }
                if app_button(
                    ui,
                    meta,
                    AppBtn {
                        widget_id: "modal.ok",
                        label: tr(locale, "btn.ok"),
                        command_id: None,
                        selected: true,
                        enabled: true,
                        disabled_reason: None,
                    },
                    tokens,
                )
                .clicked()
                {
                    state.preferences.shortcut_capture_armed = false;
                    intents.push(UiIntent::UpdatePreferencesDraft(Box::new(state.preferences.clone())));
                    intents.push(UiIntent::ClosePreferences { save: true });
                }
            });
        },
    );

    report_from_capture(capture)
}

fn render_prefs_general(
    ui: &mut Ui,
    app: &UiApp,
    locale: &str,
    body_w: f32,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.label(egui::RichText::new(tr(locale, "prefs.general.tree_heading")).strong());
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
                intents.push(UiIntent::SetNavigatorNesting { mode });
            }
        }
    });

    ui.add_space(6.0);
    ui.label(format!("Active regional format preset: {}", app.locale_preset.locale));
}

fn render_prefs_appearance(
    ui: &mut Ui,
    ctx: PrefsTabCtx<'_>,
    state: &mut AppModalsEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let PrefsTabCtx { app, locale, body_w, tokens } = ctx;
    ui.label(egui::RichText::new(tr(locale, "prefs.appearance.theme_heading")).strong());
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
        ..ScrubUnitFieldConfig::new("prefs.appearance.ui_scale", tr(locale, "prefs.appearance.ui_scale"), 75.0, 300.0)
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
    ui.label(egui::RichText::new(tr(locale, "prefs.language.heading")).strong());
    ui.label(egui::RichText::new(tr(locale, "prefs.language.subtitle")).color(tokens.text_muted).small());
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

fn render_prefs_shortcuts(
    ui: &mut Ui,
    ctx: PrefsTabCtx<'_>,
    state: &mut AppModalsEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let PrefsTabCtx { app, locale, body_w, tokens } = ctx;
    let active_shortcuts = state.preferences.effective_shortcuts(&app.shortcuts).clone();

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        ui.label(tr(locale, "prefs.shortcuts.filter"));
        let search_resp = ui.add(egui::TextEdit::singleline(&mut state.preferences.shortcut_search).desired_width(180.0));
        app_input(ui, &search_resp, meta, "prefs.shortcuts.search", "Search shortcuts", &state.preferences.shortcut_search, true);

        let ui_only_lbl = tr(locale, "prefs.shortcuts.ui_only");
        let ui_only_chk = ui.checkbox(&mut state.preferences.shortcuts_ui_only, ui_only_lbl);
        app_checkbox(ui, &ui_only_chk, meta, "prefs.shortcuts.ui_only", ui_only_lbl, state.preferences.shortcuts_ui_only);

        if app_button(
            ui,
            meta,
            AppBtn {
                widget_id: "prefs.shortcuts.reset_all",
                label: tr(locale, "prefs.shortcuts.reset_all"),
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
            if let Some(sel_id) = state.preferences.selected_shortcut_command.as_deref()
                && let Some(binding) = active_shortcuts.get(sel_id)
            {
                state.preferences.shortcut_combo_draft = binding.default_combo.clone();
            }
            intents.push(UiIntent::ResetShortcutBinding { command_id: None });
        }
    });

    ui.add_space(6.0);
    // Selected command editor + pure-Rust conflict check against the staged shortcut snapshot.
    if let Some(sel_id) = state.preferences.selected_shortcut_command.clone() {
        let binding_opt = active_shortcuts.get(&sel_id);
        render_shortcut_editor_bar(
            ui,
            ShortcutEditorSpec { active_shortcuts: &active_shortcuts, cmd_id: &sel_id, binding_opt, body_w, tokens },
            state,
            meta,
            intents,
        );
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
    let matching: Vec<&ShortcutBinding> = active_shortcuts
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
                    .stroke(Stroke::new(1.0, if selected { tokens.accent } else { tokens.border }));
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

fn render_shortcut_editor_bar(
    ui: &mut Ui,
    spec: ShortcutEditorSpec<'_>,
    state: &mut AppModalsEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let ShortcutEditorSpec { active_shortcuts, cmd_id, binding_opt, body_w, tokens } = spec;
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
                label: tr(tokens.locale, "prefs.shortcuts.apply"),
                command_id: None,
                selected: true,
                enabled: true,
                disabled_reason: None,
            },
            tokens,
        )
        .clicked()
        {
            validate_and_queue_shortcut(active_shortcuts, state, cmd_id, binding_opt, intents);
        }

        if app_button(
            ui,
            meta,
            AppBtn {
                widget_id: "prefs.shortcuts.reset_one",
                label: tr(tokens.locale, "btn.reset"),
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
            intents.push(UiIntent::ResetShortcutBinding { command_id: Some(cmd_id.to_owned()) });
        }
    });
}

fn validate_and_queue_shortcut(
    active_shortcuts: &ShortcutRegistry,
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
                Some(sc) => active_shortcuts.find_conflict_with_scope(&normalized, sc, Some(cmd_id)),
                None => active_shortcuts.find_conflict(&normalized, Some(cmd_id)),
            };
            if let Some(other_cmd) = conflict {
                state.preferences.shortcut_conflict_error = Some(format!("Shortcut `{normalized}` conflicts with `{other_cmd}`"));
            } else {
                state.preferences.shortcut_conflict_error = None;
                state.preferences.shortcut_combo_draft = normalized.clone();
                intents.push(UiIntent::SetShortcutBinding { command_id: cmd_id.into(), combo: normalized });
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

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::CloseFindReplace);
    }

    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique("egui_shell_find_replace_modal"),
            scroll_id: "app_modal.find_replace.scroll",
            widget_id: "dialog.find_replace",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 108.0).max(90.0),
            footer_separator: true,
        },
        &mut (state, intents),
        |_, _, _| {},
        |ui, meta, (state, _)| {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(body_w);
                ui.label(tr(locale, "find.label.find"));
                let q_resp = ui.add(egui::TextEdit::singleline(&mut state.find_replace.find_query).desired_width((body_w - 110.0).max(120.0)));
                app_input(ui, &q_resp, meta, "find_replace.query", tr(locale, "find.label.find"), &state.find_replace.find_query, true);
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(body_w);
                ui.label(tr(locale, "find.label.replace"));
                let r_resp = ui.add(egui::TextEdit::singleline(&mut state.find_replace.replace_with).desired_width((body_w - 110.0).max(120.0)));
                app_input(ui, &r_resp, meta, "find_replace.replace", tr(locale, "find.label.replace"), &state.find_replace.replace_with, true);
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(body_w);
                let mc_lbl = tr(locale, "find.match_case");
                let c_chk = ui.checkbox(&mut state.find_replace.match_case, mc_lbl);
                app_checkbox(ui, &c_chk, meta, "find_replace.match_case", mc_lbl, state.find_replace.match_case);
                let ec_lbl = tr(locale, "find.match_entire_cell");
                let e_chk = ui.checkbox(&mut state.find_replace.match_entire_cell, ec_lbl);
                app_checkbox(ui, &e_chk, meta, "find_replace.match_entire_cell", ec_lbl, state.find_replace.match_entire_cell);
            });
            ui.add_space(4.0);
            let status_txt = format!("Matches found: {}", state.find_replace.match_count);
            let s_resp = ui.label(&status_txt);
            app_text(ui, &s_resp, meta, "find_replace.status", &status_txt);
        },
        |ui, meta, (state, intents)| {
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
                    intents.push(UiIntent::CloseFindReplace);
                }
                if app_button(
                    ui,
                    meta,
                    AppBtn {
                        widget_id: "find_replace.find",
                        label: tr(locale, "find.btn.next"),
                        command_id: None,
                        selected: false,
                        enabled: can_search,
                        disabled_reason: (!can_search).then_some("Enter search text first"),
                    },
                    tokens,
                )
                .clicked()
                {
                    intents.push(UiIntent::ExecuteFindReplace {
                        replace_all: false,
                        find_query: state.find_replace.find_query.clone(),
                        replace_with: state.find_replace.replace_with.clone(),
                        match_case: state.find_replace.match_case,
                        match_entire_cell: state.find_replace.match_entire_cell,
                    });
                }
                if app_button(
                    ui,
                    meta,
                    AppBtn {
                        widget_id: "find_replace.replace_all",
                        label: tr(locale, "find.btn.replace_all"),
                        command_id: None,
                        selected: true,
                        enabled: can_search,
                        disabled_reason: (!can_search).then_some("Enter search text first"),
                    },
                    tokens,
                )
                .clicked()
                {
                    intents.push(UiIntent::ExecuteFindReplace {
                        replace_all: true,
                        find_query: state.find_replace.find_query.clone(),
                        replace_with: state.find_replace.replace_with.clone(),
                        match_case: state.find_replace.match_case,
                        match_entire_cell: state.find_replace.match_entire_cell,
                    });
                }
            });
        },
    );

    report_from_capture(capture)
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
    let title = if is_layout { tr(locale, "export.title.layout") } else { tr(locale, "w6.view.modal.export_figure") };
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 620.0);

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        state.export_emf_selected = false;
        intents.push_dialog_button("cancel");
    }

    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique(("egui_shell_app_modal", "dialog.export_figure")),
            scroll_id: "app_modal.export.scroll",
            widget_id: "dialog.export_figure",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 108.0).max(100.0),
            footer_separator: true,
        },
        &mut (state, intents),
        |_, _, _| {},
        |ui, meta, (state, intents)| {
            render_export_body(
                ui,
                ExportBodyCtx { app, sheet_id: &sheet_id, is_graph, is_layout, target_valid, body_w, tokens },
                state,
                meta,
                intents,
            );
        },
        |ui, meta, (state, intents)| {
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
                        label: tr(locale, "btn.export"),
                        command_id: None,
                        selected: true,
                        enabled: target_valid,
                        disabled_reason: (!target_valid).then_some("Select a Graph or Layout sheet to export"),
                    },
                    tokens,
                )
                .clicked()
                {
                    let (cmd, params) = export_figure_command_and_params(app, is_layout, &sheet_id, state.export_emf_selected);
                    state.export_emf_selected = false;
                    intents.push_engine(cmd, params);
                    intents.push(UiIntent::CloseAllModals);
                }
            });
        },
    );

    report_from_capture(capture)
}

fn export_figure_command_and_params(
    app: &UiApp,
    is_layout: bool,
    sheet_id: &SheetId,
    export_emf_selected: bool,
) -> (&'static str, serde_json::Value) {
    if export_emf_selected {
        let cmd = if is_layout { "layout.exportEmf" } else { "graph.exportEmf" };
        return (cmd, json!({ "sheet_id": sheet_id.as_str() }));
    }
    let ef = &app.canvas_dialogs.export_figure;
    match ef.format {
        ExportFigureFormat::Svg => {
            let cmd = if is_layout { "export.svg" } else { "graph.exportSvg" };
            (cmd, json!({ "sheet_id": sheet_id.as_str() }))
        }
        ExportFigureFormat::Pdf => {
            let cmd = if is_layout { "export.pdf" } else { "graph.exportPdf" };
            let cs = if app.view_shards.dialogs_graph_misc.export_color_space == "CMYK-Proof" { "cmyk" } else { "srgb" };
            (cmd, json!({ "sheet_id": sheet_id.as_str(), "color_space": cs }))
        }
        ExportFigureFormat::Png => {
            let cmd = if is_layout { "export.png" } else { "graph.exportPng" };
            (
                cmd,
                json!({
                    "sheet_id": sheet_id.as_str(),
                    "width_px": ef.width_px,
                    "height_px": ef.height_px,
                    "dpi": ef.dpi,
                }),
            )
        }
        ExportFigureFormat::Tiff(compression) => {
            let comp = match compression {
                TiffCompression::Deflate => "deflate",
                TiffCompression::Lzw => "lzw",
                TiffCompression::None => "none",
            };
            (
                "export.tiff",
                json!({
                    "sheet_id": sheet_id.as_str(),
                    "width_px": ef.width_px,
                    "height_px": ef.height_px,
                    "dpi": ef.dpi,
                    "compression": comp,
                }),
            )
        }
    }
}

fn render_export_body(
    ui: &mut Ui,
    ctx: ExportBodyCtx<'_>,
    state: &mut AppModalsEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let ExportBodyCtx { app, sheet_id, is_graph, is_layout, target_valid, body_w, tokens } = ctx;
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
                intents.push(UiIntent::UpdateExportFigureState(Box::new(next)));
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
        intents.push(UiIntent::UpdateExportFigureState(Box::new(next)));
    }

    ui.add_space(4.0);
    // Physical Dimension units + spinners (`px`, `mm`, `in`) + Journal presets (`85/114/174 mm`).
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        let dim_disabled_reason = is_vector.then_some("Vector formats (PDF/SVG/EMF) use sheet geometry instead of pixel dimensions");
        ui.label(tr(normalize_locale(&app.locale), "export.unit"));
        for (slug, lbl) in [("px", "px"), ("mm", "mm"), ("in", "in")] {
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: &format!("export.unit.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: vs.export_unit == slug,
                    enabled: !is_vector,
                    disabled_reason: dim_disabled_reason,
                },
                tokens,
            )
            .clicked()
            {
                intents.push_custom_view_action("dialogs_graph_misc", format!("export:unit:{slug}"));
            }
        }
        ui.label(tr(normalize_locale(&app.locale), "export.journal_width"));
        for mm in [85.0_f64, 114.0_f64, 174.0_f64] {
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: &format!("export.preset.{}mm", mm as u32),
                    label: &format!("{} mm", mm as u32),
                    command_id: None,
                    selected: false,
                    enabled: !is_vector,
                    disabled_reason: dim_disabled_reason,
                },
                tokens,
            )
            .clicked()
            {
                let mut next = ef;
                next.width_px = ((mm / 25.4) * f64::from(ef.dpi.max(72))).round().clamp(16.0, 9600.0) as u32;
                intents.push(UiIntent::UpdateExportFigureState(Box::new(next)));
            }
        }
    });

    let dim_disabled_reason = is_vector.then_some("Vector formats (PDF/SVG/EMF) use sheet geometry instead of pixel dimensions");
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
        enabled: !is_vector,
        disabled_reason: dim_disabled_reason,
        ..ScrubUnitFieldConfig::new("export.width", "Width", 1.0, dim_max)
    };
    let w_out = render_scrub_unit_field(ui, w_display, &mut state.export_width_spinner, w_cfg, tokens, meta);
    if !is_vector && let Some(val) = w_out.committed_value {
        let mut next = ef;
        let px = match vs.export_unit.as_str() {
            "mm" => (val / 25.4) * dpi_f,
            "in" => val * dpi_f,
            _ => val,
        };
        next.width_px = px.round().clamp(16.0, 9600.0) as u32;
        intents.push(UiIntent::UpdateExportFigureState(Box::new(next)));
    }

    let h_cfg = ScrubUnitFieldConfig {
        step_per_pt: 2.0,
        decimals: if vs.export_unit == "px" { 0 } else { 1 },
        allow_unit_switch: false,
        enabled: !is_vector,
        disabled_reason: dim_disabled_reason,
        ..ScrubUnitFieldConfig::new("export.height", "Height", 1.0, dim_max)
    };
    let h_out = render_scrub_unit_field(ui, h_display, &mut state.export_height_spinner, h_cfg, tokens, meta);
    if !is_vector && let Some(val) = h_out.committed_value {
        let mut next = ef;
        let px = match vs.export_unit.as_str() {
            "mm" => (val / 25.4) * dpi_f,
            "in" => val * dpi_f,
            _ => val,
        };
        next.height_px = px.round().clamp(16.0, 9600.0) as u32;
        intents.push(UiIntent::UpdateExportFigureState(Box::new(next)));
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
    if transparent {
        ui.label(egui::RichText::new("Note: exported files preserve the sheet canvas background.").small().weak());
    }

    ui.add_space(4.0);
    let is_pdf_export = !state.export_emf_selected && matches!(ef.format, ExportFigureFormat::Pdf);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        ui.label(tr(normalize_locale(&app.locale), "w5.export.colorspace"));
        for (slug, lbl, val) in
            [("srgb", "sRGB", "sRGB"), ("p3", "Display-P3", "Display-P3"), ("gray", "Grayscale", "Grayscale"), ("cmyk", "CMYK-Proof", "CMYK-Proof")]
        {
            let cs_enabled = match slug {
                "srgb" => true,
                "cmyk" => is_pdf_export,
                _ => false,
            };
            let cs_reason = match slug {
                "cmyk" if !is_pdf_export => Some("CMYK color space is supported for PDF exports only"),
                "p3" | "gray" => Some("Display-P3 and Grayscale color spaces are not yet supported"),
                _ => None,
            };
            let cs_selected = if is_pdf_export && vs.export_color_space == "CMYK-Proof" {
                slug == "cmyk"
            } else {
                let _ = val;
                slug == "srgb"
            };
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: &format!("export.cs.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: cs_selected,
                    enabled: cs_enabled,
                    disabled_reason: cs_reason,
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

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        intents.push_dialog_button("cancel");
    }

    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique(("egui_shell_app_modal", "dialog.print")),
            scroll_id: "app_modal.print.scroll",
            widget_id: "dialog.print",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 108.0).max(100.0),
            footer_separator: true,
        },
        &mut (state, intents),
        |_, _, _| {},
        |ui, meta, (state, intents)| {
            ui.label(egui::RichText::new("Printer Queue").strong());
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(body_w);
                if app_button(
                    ui,
                    meta,
                    AppBtn {
                        widget_id: "print.queue.default",
                        label: tr(locale, "print.os_spool"),
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
                    intents.push(UiIntent::UpdatePrintDialogState(Box::new(next)));
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
                        intents.push(UiIntent::UpdatePrintDialogState(Box::new(next)));
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
                intents.push(UiIntent::UpdatePrintDialogState(Box::new(next)));
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
                intents.push(UiIntent::UpdatePrintDialogState(Box::new(next)));
            }

            ui.add_space(4.0);
            let mut crops = pd.job.crop_marks;
            let crop_lbl = tr(locale, "print.crop_marks");
            let crop_chk = ui.checkbox(&mut crops, crop_lbl);
            app_checkbox(ui, &crop_chk, meta, "print.crop_marks", crop_lbl, crops);
            if crop_chk.changed() {
                let mut next = pd.clone();
                next.job.crop_marks = crops;
                intents.push(UiIntent::UpdatePrintDialogState(Box::new(next)));
            }

            if let Some(val) = super::graph_modals::render_length_spinner_pt(
                ui,
                super::graph_modals::LengthSpinnerSpec {
                    widget_id: "print.bleed_pt",
                    label: tr(locale, "print.bleed"),
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
                intents.push(UiIntent::UpdatePrintDialogState(Box::new(next)));
            }

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(body_w);
                ui.label(tr(normalize_locale(&app.locale), "w5.export.colorspace"));
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
                        intents.push(UiIntent::UpdatePrintDialogState(Box::new(next)));
                    }
                }
            });

            ui.add_space(4.0);
            let gui_only = app.is_gui_only();
            let mut submit_os = pd.submit_to_os && !gui_only;
            let direct_lbl = tr(locale, "print.direct_queue");
            let os_chk = ui.add_enabled(!gui_only, egui::Checkbox::new(&mut submit_os, direct_lbl));
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
                    label: direct_lbl,
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
                intents.push(UiIntent::UpdatePrintDialogState(Box::new(next)));
            }
        },
        |ui, meta, (_, intents)| {
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
                        label: tr(locale, "print.commit"),
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
        },
    );

    report_from_capture(capture)
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

pub(crate) fn render_about_modal(
    ctx: &Context,
    app: &UiApp,
    _state: &AppModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "menu.help.about");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 400.0);
    let build_sha = option_env!("CURVEBOOK_BUILD_SHA");
    let version_line = format_about_version_line(env!("CARGO_PKG_VERSION"), build_sha);
    let copied_id = egui::Id::unique(("egui_shell_app_modal", "dialog.about.copied"));

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        intents.push_dialog_button("cancel");
    }

    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique(("egui_shell_app_modal", "dialog.about")),
            scroll_id: "app_modal.about.scroll",
            widget_id: "dialog.about",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 108.0).max(80.0),
            footer_separator: true,
        },
        intents,
        |_, _, _| {},
        |ui, _, _| {
            ui.horizontal(|ui| {
                let (mark_rect, _) = ui.allocate_exact_size(egui::Vec2::splat(44.0), egui::Sense::hover());
                crate::egui_shell::brand::paint_mark(ui, mark_rect);
                let text_w = (body_w - 44.0 - ui.spacing().item_spacing.x).max(80.0);
                ui.vertical(|ui| {
                    ui.set_max_width(text_w);
                    ui.label(egui::RichText::new(&version_line).strong());
                    ui.add(egui::Label::new("Native scientific biostatistics, curve fitting, and publication graphing suite.").wrap());
                });
            });
            ui.add_space(8.0);
            ui.add(egui::Label::new("Copyright © 2026 CurveBook Contributors").wrap());
            ui.add(egui::Label::new("Licensed under MIT OR Apache-2.0.").wrap());
            ui.add(
                egui::Label::new(
                    egui::RichText::new("See NOTICE and ATTRIBUTION.md in the distribution for third-party licenses and clean-room notices.")
                        .color(tokens.text_muted),
                )
                .wrap(),
            );
        },
        |ui, meta, intents| {
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

                let feedback_lbl = tr(locale, "menu.help.feedback");
                let feedback_resp = ui.button(feedback_lbl);
                app_record_visible(
                    ui,
                    &feedback_resp,
                    meta,
                    FrameInspectEntryInput {
                        widget_id: "dialog.about.send_feedback",
                        role: "button",
                        label: feedback_lbl,
                        command_id: Some("app.feedback"),
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::ModalDialog,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if feedback_resp.clicked() {
                    ctx.data_mut(|d| d.insert_temp(copied_id, false));
                    intents.push_ui("app.feedback", json!({}));
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
                    intents.push_dialog_button("ok");
                }
            });
        },
    );

    report_from_capture(capture)
}

fn render_checkpoints_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut AppModalsEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "cmd.project.listCheckpoints");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 500.0);

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        state.checkpoint_pending_restore_id = None;
        intents.push_dialog_button("cancel");
    }

    let mut name_draft = app.canvas_dialogs.checkpoints.name_draft.clone();
    let mut note_draft = app.canvas_dialogs.checkpoints.note_draft.clone();
    let doc = app.document();

    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique(("egui_shell_app_modal", "dialog.checkpoints")),
            scroll_id: "app_modal.checkpoints.scroll",
            widget_id: "dialog.checkpoints",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 112.0).max(120.0),
            footer_separator: true,
        },
        &mut (state, intents),
        |_, _, _| {},
        |ui, meta, (state, intents)| {
            ui.label(egui::RichText::new("Create Checkpoint").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(tr(locale, "modal.label.name"));
                let name_resp = ui.add(egui::TextEdit::singleline(&mut name_draft).desired_width((body_w - 64.0).max(120.0)));
                app_input(ui, &name_resp, meta, "checkpoints.name", "Checkpoint Name", &name_draft, true);
                if name_resp.changed() {
                    intents.push(UiIntent::PatchActiveDialog {
                        fields: serde_json::Map::from_iter([(String::from("name"), Value::String(name_draft.clone()))]),
                    });
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(tr(locale, "modal.label.note"));
                let note_resp = ui.add(egui::TextEdit::singleline(&mut note_draft).desired_width((body_w - 64.0).max(120.0)));
                app_input(ui, &note_resp, meta, "checkpoints.note", "Checkpoint Note", &note_draft, true);
                if note_resp.changed() {
                    intents.push(UiIntent::PatchActiveDialog {
                        fields: serde_json::Map::from_iter([(String::from("note"), Value::String(note_draft.clone()))]),
                    });
                }
            });
            ui.add_space(6.0);
            let can_create = !name_draft.trim().is_empty();
            let create_resp = app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "checkpoints.create",
                    label: tr(locale, "cmd.project.createCheckpoint"),
                    command_id: Some("project.createCheckpoint"),
                    selected: can_create,
                    enabled: can_create,
                    disabled_reason: (!can_create).then_some("Enter a checkpoint name"),
                },
                tokens,
            );
            if create_resp.clicked() && can_create {
                intents.push(UiIntent::PatchActiveDialog {
                    fields: serde_json::Map::from_iter([
                        (String::from("name"), Value::String(name_draft.clone())),
                        (String::from("note"), Value::String(note_draft.clone())),
                    ]),
                });
                intents.push_dialog_button("ok");
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Saved Checkpoints").strong());
            ui.add_space(4.0);

            if doc.checkpoints.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new("No checkpoints saved yet.").color(tokens.text_muted)).wrap());
            } else {
                for (idx, cp) in doc.checkpoints.iter().enumerate() {
                    egui::Frame::NONE
                        .fill(tokens.surface_bg)
                        .stroke(Stroke::new(1.0, tokens.border))
                        .corner_radius(egui::CornerRadius::same(4))
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.set_width((body_w - 18.0).max(120.0));
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(&cp.name).strong());
                                if !cp.created_utc.is_empty() {
                                    ui.label(egui::RichText::new(&cp.created_utc).color(tokens.text_muted).small());
                                }
                            });
                            if !cp.note.is_empty() {
                                ui.add(egui::Label::new(egui::RichText::new(&cp.note).color(tokens.text_muted)).wrap());
                            }
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| {
                                let restore_wid = format!("checkpoints.restore.{idx}");
                                let confirm_wid = format!("checkpoints.restore.confirm.{idx}");
                                let cancel_restore_wid = format!("checkpoints.restore.cancel.{idx}");
                                let delete_wid = format!("checkpoints.delete.{idx}");
                                let awaiting_confirm = state.checkpoint_pending_restore_id.as_deref() == Some(cp.id.as_str());
                                if awaiting_confirm {
                                    if app_button(
                                        ui,
                                        meta,
                                        AppBtn {
                                            widget_id: &confirm_wid,
                                            label: tr(locale, "checkpoints.confirm_restore"),
                                            command_id: Some("project.restoreCheckpoint"),
                                            selected: true,
                                            enabled: true,
                                            disabled_reason: None,
                                        },
                                        tokens,
                                    )
                                    .clicked()
                                    {
                                        state.checkpoint_pending_restore_id = None;
                                        intents.push_engine("project.restoreCheckpoint", json!({ "id": cp.id }));
                                    }
                                    if app_button(
                                        ui,
                                        meta,
                                        AppBtn {
                                            widget_id: &cancel_restore_wid,
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
                                        state.checkpoint_pending_restore_id = None;
                                    }
                                    ui.label(
                                        egui::RichText::new("Replaces current unsaved edits (undoable via Edit > Undo).")
                                            .small()
                                            .color(tokens.text_muted),
                                    );
                                } else if app_button(
                                    ui,
                                    meta,
                                    AppBtn {
                                        widget_id: &restore_wid,
                                        label: tr(locale, "cmd.project.restoreCheckpoint"),
                                        command_id: Some("project.restoreCheckpoint"),
                                        selected: false,
                                        enabled: true,
                                        disabled_reason: None,
                                    },
                                    tokens,
                                )
                                .clicked()
                                {
                                    state.checkpoint_pending_restore_id = Some(cp.id.clone());
                                }
                                if app_button(
                                    ui,
                                    meta,
                                    AppBtn {
                                        widget_id: &delete_wid,
                                        label: tr(locale, "cmd.project.deleteCheckpoint"),
                                        command_id: Some("project.deleteCheckpoint"),
                                        selected: false,
                                        enabled: true,
                                        disabled_reason: None,
                                    },
                                    tokens,
                                )
                                .clicked()
                                {
                                    if state.checkpoint_pending_restore_id.as_deref() == Some(cp.id.as_str()) {
                                        state.checkpoint_pending_restore_id = None;
                                    }
                                    intents.push_engine("project.deleteCheckpoint", json!({ "id": cp.id }));
                                }
                            });
                        });
                    ui.add_space(4.0);
                }
            }
        },
        |ui, meta, (state, intents)| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let close_lbl = tr(locale, "btn.cancel");
                if app_button(
                    ui,
                    meta,
                    AppBtn { widget_id: "modal.cancel", label: close_lbl, command_id: None, selected: false, enabled: true, disabled_reason: None },
                    tokens,
                )
                .clicked()
                {
                    state.checkpoint_pending_restore_id = None;
                    intents.push_dialog_button("cancel");
                }
            });
        },
    );

    report_from_capture(capture)
}

fn render_feedback_modal(
    ctx: &Context,
    app: &UiApp,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "menu.help.feedback");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 560.0);
    let draft = app.canvas_dialogs.feedback.clone();
    let changed = false;
    let diag = app.feedback_diagnostics();

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        intents.push_dialog_button("cancel");
    }

    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique(("egui_shell_app_modal", "dialog.feedback")),
            scroll_id: "app_modal.feedback.scroll",
            widget_id: "dialog.feedback",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 116.0).max(120.0),
            footer_separator: false,
        },
        &mut (draft, changed, intents),
        |_, _, _| {},
        |ui, meta, (draft, changed, _)| {
            if render_feedback_category_row(ui, draft, tokens, meta) {
                *changed = true;
            }
            ui.add_space(4.0);
            ui.add(egui::Label::new(egui::RichText::new(draft.category.guidance()).color(tokens.text_muted).small()).wrap());
            ui.add_space(8.0);

            if render_feedback_fields(ui, draft, body_w, meta) {
                *changed = true;
            }
            ui.add_space(8.0);
            if render_feedback_diagnostics_section(ui, draft, &diag, body_w, tokens, meta) {
                *changed = true;
            }
        },
        |ui, meta, (draft, changed, intents)| {
            if *changed {
                draft.copied_markdown = false;
                intents.push(UiIntent::UpdateFeedbackDraft(Box::new(draft.clone())));
            }
            let active_diag = draft.include_diagnostics.then_some(&diag);
            let preview = crate::feedback::build_github_issue_url(draft, active_diag);
            render_feedback_footer(ui, &FeedbackFooterInput { locale, draft, active_diag, preview: &preview, body_w, tokens }, meta, intents);
        },
    );

    report_from_capture(capture)
}

fn render_feedback_category_row(ui: &mut Ui, draft: &mut crate::feedback::FeedbackDraft, tokens: &Tokens, meta: &mut FrameInspectMeta) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for cat in crate::feedback::FeedbackCategory::ALL {
            let selected = draft.category == cat;
            let widget_id = format!("feedback.category.{}", cat.slug());
            if app_tab(ui, meta, &widget_id, cat.label(), selected, tokens).clicked() && !selected {
                draft.category = cat;
                draft.status_message = None;
                changed = true;
            }
        }
    });
    changed
}

fn render_feedback_fields(ui: &mut Ui, draft: &mut crate::feedback::FeedbackDraft, body_w: f32, meta: &mut FrameInspectMeta) -> bool {
    let mut changed = false;

    let title_lbl_resp = ui.label("Title");
    let title_resp = ui
        .add(
            egui::TextEdit::singleline(&mut draft.title)
                .id_salt("feedback.title")
                .desired_width(body_w)
                .hint_text("Short summary of the issue or request"),
        )
        .labelled_by(title_lbl_resp.id);
    record_feedback_text_edit(ui, &title_resp, meta, "feedback.title", "Title", &draft.title);
    if title_resp.changed() {
        changed = true;
    }

    ui.add_space(6.0);
    let summary_lbl = draft.category.summary_label();
    let summary_lbl_resp = ui.label(summary_lbl);
    let summary_resp = ui
        .add(egui::TextEdit::multiline(&mut draft.summary).id_salt("feedback.summary").desired_width(body_w).desired_rows(3))
        .labelled_by(summary_lbl_resp.id);
    record_feedback_text_edit(ui, &summary_resp, meta, "feedback.summary", summary_lbl, &draft.summary);
    if summary_resp.changed() {
        changed = true;
    }

    ui.add_space(6.0);
    let details_lbl = draft.category.details_label();
    let details_lbl_resp = ui.label(details_lbl);
    let details_resp = ui
        .add(egui::TextEdit::multiline(&mut draft.details).id_salt("feedback.details").desired_width(body_w).desired_rows(3))
        .labelled_by(details_lbl_resp.id);
    record_feedback_text_edit(ui, &details_resp, meta, "feedback.details", details_lbl, &draft.details);
    if details_resp.changed() {
        changed = true;
    }

    ui.add_space(6.0);
    let extra_lbl = draft.category.extra_label();
    let extra_lbl_resp = ui.label(extra_lbl);
    let extra_resp = ui
        .add(egui::TextEdit::multiline(&mut draft.extra).id_salt("feedback.extra").desired_width(body_w).desired_rows(2))
        .labelled_by(extra_lbl_resp.id);
    record_feedback_text_edit(ui, &extra_resp, meta, "feedback.extra", extra_lbl, &draft.extra);
    if extra_resp.changed() {
        changed = true;
    }

    changed
}

fn render_feedback_diagnostics_section(
    ui: &mut Ui,
    draft: &mut crate::feedback::FeedbackDraft,
    diag: &crate::feedback::FeedbackDiagnostics,
    body_w: f32,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
) -> bool {
    let diag_lbl = "Include sanitized environment diagnostics (no project data or file paths)";
    let diag_resp = ui.checkbox(&mut draft.include_diagnostics, diag_lbl);
    app_checkbox(ui, &diag_resp, meta, "feedback.include_diagnostics", diag_lbl, draft.include_diagnostics);
    if draft.include_diagnostics {
        egui::Frame::group(ui.style()).fill(tokens.surface_bg).stroke(Stroke::new(1.0, tokens.border)).show(ui, |ui| {
            ui.set_max_width((body_w - 16.0).max(120.0));
            ui.add(egui::Label::new(egui::RichText::new(diag.preview_text()).monospace().small()).wrap());
        });
    }
    diag_resp.changed()
}

struct FeedbackFooterInput<'a> {
    locale: &'a str,
    draft: &'a crate::feedback::FeedbackDraft,
    active_diag: Option<&'a crate::feedback::FeedbackDiagnostics>,
    preview: &'a crate::feedback::BuiltFeedbackUrl,
    body_w: f32,
    tokens: &'a Tokens,
}

fn render_feedback_footer(ui: &mut Ui, input: &FeedbackFooterInput<'_>, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let locale = input.locale;
    let draft = input.draft;
    let active_diag = input.active_diag;
    let preview = input.preview;
    let body_w = input.body_w;
    let tokens = input.tokens;
    ui.add_space(6.0);
    let length_note = if preview.truncated {
        format!("GitHub URL capped at {} bytes ({} characters omitted — use Copy Markdown for full text).", preview.url.len(), preview.omitted_chars)
    } else {
        format!(
            "Prefilled GitHub URL: {} / {} bytes · template: {}",
            preview.url.len(),
            crate::feedback::MAX_GITHUB_ISSUE_URL_BYTES,
            draft.category.template_filename()
        )
    };
    ui.add(egui::Label::new(egui::RichText::new(length_note).color(tokens.text_muted).small()).wrap());
    if let Some(status) = draft.status_message.as_deref() {
        ui.add(egui::Label::new(egui::RichText::new(status).color(tokens.accent).small()).wrap());
    }
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
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
            intents.push_dialog_button("cancel");
        }

        if app_button(
            ui,
            meta,
            AppBtn {
                widget_id: "dialog.feedback.reset",
                label: tr(locale, "modal.btn.clear"),
                command_id: None,
                selected: false,
                enabled: true,
                disabled_reason: None,
            },
            tokens,
        )
        .clicked()
        {
            intents.push(UiIntent::ResetFeedbackDraft);
        }

        let copy_lbl = if draft.copied_markdown { "Copied Markdown" } else { "Copy Markdown" };
        if app_button(
            ui,
            meta,
            AppBtn {
                widget_id: "dialog.feedback.copy_markdown",
                label: copy_lbl,
                command_id: None,
                selected: false,
                enabled: true,
                disabled_reason: None,
            },
            tokens,
        )
        .clicked()
        {
            let md = crate::feedback::build_feedback_markdown(draft, active_diag);
            ui.ctx().copy_text(md);
            intents.push(UiIntent::CopyFeedbackMarkdown);
        }

        if app_button(
            ui,
            meta,
            AppBtn {
                widget_id: "dialog.feedback.open_github",
                label: tr(locale, "feedback.open_github"),
                command_id: None,
                selected: true,
                enabled: true,
                disabled_reason: None,
            },
            tokens,
        )
        .clicked()
        {
            intents.push(UiIntent::OpenFeedbackOnGitHub);
        }
    });
}

fn record_feedback_text_edit(ui: &Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, value: &str) {
    app_record_visible(
        ui,
        resp,
        meta,
        FrameInspectEntryInput {
            widget_id,
            role: "textbox",
            label,
            command_id: None,
            value: Some(json!(value)),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
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

const fn report_from_capture(capture: ModalCaptureReport) -> AppModalFrameReport {
    AppModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn app_record_visible(ui: &Ui, resp: &Response, meta: &mut FrameInspectMeta, input: FrameInspectEntryInput<'_>) {
    ui_kit::record_if_visible(ui, resp, meta, input);
}

fn app_button(ui: &mut Ui, meta: &mut FrameInspectMeta, entry: AppBtn<'_>, tokens: &Tokens) -> Response {
    let primary = entry.selected && matches!(entry.widget_id, "modal.ok" | "dialog.button.save" | "dialog.feedback.open_github");
    ui_kit::modal_button(
        ui,
        meta,
        ModalButtonSpec {
            widget_id: entry.widget_id,
            label: entry.label,
            command_id: entry.command_id,
            selected: entry.selected,
            primary,
            enabled: entry.enabled,
            disabled_reason: entry.disabled_reason,
            ranked: false,
        },
        tokens,
    )
}

fn app_tab(ui: &mut Ui, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, selected: bool, tokens: &Tokens) -> Response {
    ui_kit::modal_tab(ui, meta, widget_id, label, selected, tokens)
}

fn app_checkbox(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, checked: bool) {
    ui_kit::record_modal_checkbox(ui, resp, meta, widget_id, label, checked);
}

fn app_input(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, text: &str, enabled: bool) {
    ui_kit::record_modal_input(ui, resp, meta, widget_id, label, text, enabled);
}

fn app_text(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, text: &str) {
    ui_kit::record_modal_text(ui, resp, meta, widget_id, text);
}

#[cfg(test)]
mod tests {
    use super::{AppModalsEguiState, PreferencesCategory};
    use crate::i18n::{BETA_ACTIVE_LOCALES, is_rtl_locale};

    #[test]
    fn beta_locale_list_is_strictly_14_non_rtl() {
        assert_eq!(BETA_ACTIVE_LOCALES.len(), 14);
        assert!(BETA_ACTIVE_LOCALES.iter().all(|loc| !is_rtl_locale(loc)));
        assert_eq!(PreferencesCategory::ALL.len(), 4);
        assert!(!AppModalsEguiState::default().any_open());
    }
}
