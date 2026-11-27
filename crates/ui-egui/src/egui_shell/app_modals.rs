//! Application-level modals in `egui 0.37` (`R15.4`, `gui-layout-spec.md` §8.14):
//! conditional `Export Figure / Layout`, `Print`, `Unsaved Changes`, `About`,
//! `Find & Replace`, and 4-category `Preferences` (with pure-Rust conflict-checked
//! staged `Keyboard Shortcuts` editor and strict 14-locale Beta language selector).

use curvebook_doc::SheetId;
use curvebook_palette::CanvasPreviewFilter;
use curvebook_pdf::PdfColorSpace;
use curvebook_render::TiffCompression;
use egui::scroll_area::ScrollBarVisibility;
use egui::{Context, Key, Rect, Response, Stroke, Ui};
use serde_json::{Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, CsvExportTargetKind, ExportFigureFormat};
use crate::i18n::{BETA_ACTIVE_LOCALES, is_rtl_locale, locale_picker_label, normalize_locale, tr};
use crate::inspect::ZLayer;
use crate::menus::shortcuts::{ShortcutBinding, ShortcutRegistry, normalize_shortcut_combo};
use crate::theme::UiThemeMode;
use crate::workspace_nav::NavigatorNestingMode;

use super::inspect_bridge::{ModalCaptureReport, ModalCaptureSpec, finalize_egui_modal_response};
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
    pub preview_filter: CanvasPreviewFilter,
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
    /// Capture live `locale`, `theme_mode`, `ui_scale`, `nesting_mode`, and `preview_filter` when opening Preferences.
    pub fn capture_live_snapshot(
        &mut self,
        locale: &str,
        theme_mode: UiThemeMode,
        ui_scale: f32,
        nesting_mode: NavigatorNestingMode,
        preview_filter: CanvasPreviewFilter,
    ) {
        self.initial_snapshot = Some(PreferencesSnapshot { locale: locale.to_owned(), theme_mode, ui_scale, nesting_mode, preview_filter });
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
    pub export_selected_gallery_scope: bool,
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
            export_selected_gallery_scope: false,
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
        self.export_selected_gallery_scope = false;
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
    if !matches!(app.canvas_dialogs.active_dialog, ActiveDialog::ExportFigure { .. }) {
        state.export_emf_selected = false;
        state.export_selected_gallery_scope = false;
    }

    match &app.canvas_dialogs.active_dialog {
        ActiveDialog::ExportFigure { sheet_id } => render_export_modal(ctx, app, state, sheet_id.clone(), tokens, meta, intents),
        ActiveDialog::Print { .. } => render_print_modal(ctx, app, state, tokens, meta, intents),
        ActiveDialog::Checkpoints => render_checkpoints_modal(ctx, app, state, tokens, meta, intents),
        ActiveDialog::About => render_about_modal(ctx, app, state, tokens, meta, intents),
        ActiveDialog::ImportWarnings => render_import_warnings_modal(ctx, app, tokens, meta, intents),
        ActiveDialog::ImportProjectMode { command_id, path, has_unsaved_changes } => {
            render_import_project_mode_modal(ctx, app, command_id, path, *has_unsaved_changes, tokens, meta, intents)
        }
        ActiveDialog::CsvExportOptions(target) => render_csv_export_options_modal(ctx, app, target, tokens, meta, intents),
        ActiveDialog::ExportAllTablesExistingFiles { dir, existing_files } => {
            render_export_all_tables_existing_files_modal(ctx, app, dir, existing_files, tokens, meta, intents)
        }
        ActiveDialog::PzfxExportConfirm { preflight } => render_pzfx_export_confirm_modal(ctx, app, preflight, tokens, meta, intents),
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

    let wants_kb_before = ctx.egui_wants_keyboard_input();
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
            let dont_save_lbl = tr(locale, "w6.view.shell.dont_save");
            let cancel_lbl = tr(locale, "btn.cancel");
            let save_lbl = tr(locale, "menu.file.save");
            render_trailing_modal_footer(
                ui,
                meta,
                body_w,
                &[dont_save_lbl],
                &[cancel_lbl, save_lbl],
                tokens,
                intents,
                |ui, meta, intents| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "dialog.button.dont_save",
                            label: dont_save_lbl,
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
                },
                |ui, meta, intents| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "dialog.button.cancel",
                            label: cancel_lbl,
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
                            widget_id: "dialog.button.save",
                            label: save_lbl,
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
                },
            );
        },
    );

    if !wants_kb_before && !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
        intents.push(UiIntent::ResolveUnsavedChanges { action: UnsavedCloseAction::Save });
    }

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
        state.preferences.capture_live_snapshot(&app.locale, app.theme.mode, app.theme.ui_scale, app.nav.nesting_mode, app.preview_filter);
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
    let wants_kb_before = ctx.egui_wants_keyboard_input();
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
        &mut (&mut *state, &mut *intents),
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
            let cancel_lbl = tr(locale, "btn.cancel");
            let ok_lbl = tr(locale, "btn.ok");
            render_trailing_modal_footer(
                ui,
                meta,
                body_w,
                &[],
                &[cancel_lbl, ok_lbl],
                tokens,
                &mut (&mut **state, &mut **intents),
                |_, _, _| {},
                |ui, meta, (state, intents)| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "modal.cancel",
                            label: cancel_lbl,
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
                        AppBtn { widget_id: "modal.ok", label: ok_lbl, command_id: None, selected: true, enabled: true, disabled_reason: None },
                        tokens,
                    )
                    .clicked()
                    {
                        state.preferences.shortcut_capture_armed = false;
                        intents.push(UiIntent::UpdatePreferencesDraft(Box::new(state.preferences.clone())));
                        intents.push(UiIntent::ClosePreferences { save: true });
                    }
                },
            );
        },
    );

    if !state.preferences.shortcut_capture_armed
        && !wants_kb_before
        && !ctx.egui_wants_keyboard_input()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
    {
        state.preferences.shortcut_capture_armed = false;
        intents.push(UiIntent::UpdatePreferencesDraft(Box::new(state.preferences.clone())));
        intents.push(UiIntent::ClosePreferences { save: true });
    }

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

    ui.add_space(8.0);
    ui.label(egui::RichText::new("Color-Vision Deficiency & Grayscale Preview (CVD)").strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(body_w);
        for (wid, lbl, filter) in [
            ("prefs.appearance.cvd.normal", if locale.starts_with("zh") { "正常视觉" } else { "Normal" }, CanvasPreviewFilter::Normal),
            ("prefs.appearance.cvd.protanopia", "Protanopia", CanvasPreviewFilter::Protanopia),
            ("prefs.appearance.cvd.deuteranopia", "Deuteranopia", CanvasPreviewFilter::Deuteranopia),
            ("prefs.appearance.cvd.tritanopia", "Tritanopia", CanvasPreviewFilter::Tritanopia),
            ("prefs.appearance.cvd.grayscale", "Grayscale", CanvasPreviewFilter::GrayscalePrint),
        ] {
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: wid,
                    label: lbl,
                    command_id: Some("ui.previewFilter.set"),
                    selected: app.preview_filter == filter,
                    enabled: true,
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                intents.push(UiIntent::SetPreviewFilter { filter });
            }
        }
    });

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
                if modifiers.mac_cmd || (modifiers.command && !modifiers.ctrl) {
                    parts.push("Cmd");
                }
                if modifiers.ctrl {
                    parts.push("Ctrl");
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

    let wants_kb_before = ctx.egui_wants_keyboard_input();
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
        &mut (&mut *state, &mut *intents),
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
            let can_search = !state.find_replace.find_query.is_empty();
            let find_lbl = tr(locale, "find.btn.next");
            let cancel_lbl = tr(locale, "btn.cancel");
            let replace_lbl = tr(locale, "find.btn.replace_all");
            render_trailing_modal_footer(
                ui,
                meta,
                body_w,
                &[find_lbl],
                &[cancel_lbl, replace_lbl],
                tokens,
                &mut (&mut **state, &mut **intents),
                |ui, meta, (state, intents)| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "find_replace.find",
                            label: find_lbl,
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
                },
                |ui, meta, (state, intents)| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "modal.cancel",
                            label: cancel_lbl,
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
                            widget_id: "find_replace.replace_all",
                            label: replace_lbl,
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
                },
            );
        },
    );

    if !state.find_replace.find_query.is_empty()
        && !wants_kb_before
        && !ctx.egui_wants_keyboard_input()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
    {
        intents.push(UiIntent::ExecuteFindReplace {
            replace_all: false,
            find_query: state.find_replace.find_query.clone(),
            replace_with: state.find_replace.replace_with.clone(),
            match_case: state.find_replace.match_case,
            match_entire_cell: state.find_replace.match_entire_cell,
        });
    }

    report_from_capture(capture)
}

// ============================================================================
// 4. Conditional Export Figure / Layout Modal (§8.14)
// ============================================================================

pub(crate) fn gallery_selected_renderable_ids(app: &UiApp) -> Vec<SheetId> {
    let doc = app.document();
    app.nav.selected_gallery_ids().into_iter().filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some()).collect()
}

fn emit_export_figure_commits(
    app: &UiApp,
    state: &AppModalsEguiState,
    sheet_id: &SheetId,
    is_layout: bool,
    selected_renderables: &[SheetId],
    intents: &mut PendingUiIntents,
) {
    let doc = app.document();
    let ef = &app.canvas_dialogs.export_figure;
    if state.export_selected_gallery_scope && !selected_renderables.is_empty() {
        if !state.export_emf_selected && matches!(ef.format, ExportFigureFormat::Pdf) && selected_renderables.len() >= 2 {
            let (_, mut params) = export_figure_command_and_params(app, false, &selected_renderables[0], false);
            if let Some(obj) = params.as_object_mut() {
                obj.remove("sheet_id");
                let ids: Vec<&str> = selected_renderables.iter().map(SheetId::as_str).collect();
                obj.insert(String::from("sheet_ids"), json!(ids));
            }
            intents.push_engine("export.pdf", params);
        } else {
            for sid in selected_renderables {
                let sid_is_layout = doc.layout_sheet(sid).is_some();
                let (cmd, params) = export_figure_command_and_params(app, sid_is_layout, sid, state.export_emf_selected);
                intents.push_engine(cmd, params);
            }
        }
    } else {
        let (cmd, params) = export_figure_command_and_params(app, is_layout, sheet_id, state.export_emf_selected);
        intents.push_engine(cmd, params);
    }
    intents.push(UiIntent::CloseAllModals);
}

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
    let selected_renderables = gallery_selected_renderable_ids(app);
    let is_graph = doc.graph_sheet(&sheet_id).is_some();
    let is_layout = doc.layout_sheet(&sheet_id).is_some();
    let scope_batch = state.export_selected_gallery_scope && !selected_renderables.is_empty();
    let target_valid = is_graph || is_layout || scope_batch;
    let title = if is_layout && !scope_batch { tr(locale, "export.title.layout") } else { tr(locale, "w6.view.modal.export_figure") };
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 620.0);

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        state.export_emf_selected = false;
        state.export_selected_gallery_scope = false;
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    }

    let wants_kb_before = ctx.egui_wants_keyboard_input();
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
        &mut (&mut *state, &mut *intents),
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
            let cancel_lbl = tr(locale, "btn.cancel");
            let export_lbl = tr(locale, "btn.export");
            let ef = &app.canvas_dialogs.export_figure;
            let is_vector = state.export_emf_selected || matches!(ef.format, ExportFigureFormat::Pdf | ExportFigureFormat::Svg);
            let raster_dims_valid = is_vector || (ef.width_px > 0 && ef.height_px > 0 && ef.dpi > 0);
            let can_export = target_valid && raster_dims_valid;
            let disabled_reason = if !target_valid {
                Some("Select a Graph or Layout sheet to export")
            } else if !raster_dims_valid {
                Some("Raster export requires positive width, height, and DPI")
            } else {
                None
            };
            render_trailing_modal_footer(
                ui,
                meta,
                body_w,
                &[],
                &[cancel_lbl, export_lbl],
                tokens,
                &mut (&mut **state, &mut **intents),
                |_, _, _| {},
                |ui, meta, (state, intents)| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "modal.cancel",
                            label: cancel_lbl,
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
                        state.export_selected_gallery_scope = false;
                        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
                    }
                    if app_button(
                        ui,
                        meta,
                        AppBtn { widget_id: "modal.ok", label: export_lbl, command_id: None, selected: true, enabled: can_export, disabled_reason },
                        tokens,
                    )
                    .clicked()
                    {
                        // Keep emitting host-routed `EngineCommand` + `CloseAllModals` so desktop
                        // `gui_window.rs` routes exports through the OS Save File dialog (and preserves
                        // `audit_w1_sec18_01_export` / `audit_w1_sec19_s1` / `r15_6_phase1_pipeline`
                        // contracts); `export_emf_selected` is cleared on actual modal close.
                        emit_export_figure_commits(app, state, &sheet_id, is_layout, &selected_renderables, intents);
                    }
                },
            );
        },
    );

    let ef = &app.canvas_dialogs.export_figure;
    let is_vector = state.export_emf_selected || matches!(ef.format, ExportFigureFormat::Pdf | ExportFigureFormat::Svg);
    let can_export = target_valid && (is_vector || (ef.width_px > 0 && ef.height_px > 0 && ef.dpi > 0));
    if can_export && !wants_kb_before && !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
        emit_export_figure_commits(app, state, &sheet_id, is_layout, &selected_renderables, intents);
    }

    report_from_capture(capture)
}

pub(crate) fn export_figure_command_and_params(
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
            let mut params = json!({ "sheet_id": sheet_id.as_str() });
            if ef.transparent_bg
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert(String::from("transparent_bg"), json!(true));
            }
            (cmd, params)
        }
        ExportFigureFormat::Pdf => {
            let cmd = if is_layout { "export.pdf" } else { "graph.exportPdf" };
            let cs = if ef.color_space == "CMYK-Proof" { "cmyk" } else { "srgb" };
            (cmd, json!({ "sheet_id": sheet_id.as_str(), "color_space": cs }))
        }
        ExportFigureFormat::Png => {
            let cmd = if is_layout { "export.png" } else { "graph.exportPng" };
            let mut params = json!({
                "sheet_id": sheet_id.as_str(),
                "width_px": ef.width_px,
                "height_px": ef.height_px,
                "dpi": ef.dpi,
            });
            if ef.transparent_bg
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert(String::from("transparent_bg"), json!(true));
            }
            (cmd, params)
        }
        ExportFigureFormat::Tiff(compression) => {
            let comp = match compression {
                TiffCompression::Deflate => "deflate",
                TiffCompression::Lzw => "lzw",
                TiffCompression::None => "none",
            };
            let mut params = json!({
                "sheet_id": sheet_id.as_str(),
                "width_px": ef.width_px,
                "height_px": ef.height_px,
                "dpi": ef.dpi,
                "compression": comp,
            });
            if ef.transparent_bg
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert(String::from("transparent_bg"), json!(true));
            }
            ("export.tiff", params)
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
    let locale = normalize_locale(&app.locale);
    let ef = &app.canvas_dialogs.export_figure;
    let selected_renderables = gallery_selected_renderable_ids(app);
    if !selected_renderables.is_empty() && (app.nav.gallery.open || state.export_selected_gallery_scope || selected_renderables.len() > 1) {
        ui.label(egui::RichText::new(tr(locale, "export.scope.label")).strong());
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(body_w);
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "export.scope.active",
                    label: tr(locale, "export.scope.active"),
                    command_id: None,
                    selected: !state.export_selected_gallery_scope,
                    enabled: is_graph || is_layout,
                    disabled_reason: (!(is_graph || is_layout)).then_some("No active Graph or Layout sheet"),
                },
                tokens,
            )
            .clicked()
            {
                state.export_selected_gallery_scope = false;
            }
            let sel_lbl = format!("{} ({})", tr(locale, "export.scope.selected"), selected_renderables.len());
            if app_button(
                ui,
                meta,
                AppBtn {
                    widget_id: "export.scope.selected",
                    label: &sel_lbl,
                    command_id: None,
                    selected: state.export_selected_gallery_scope,
                    enabled: !selected_renderables.is_empty(),
                    disabled_reason: None,
                },
                tokens,
            )
            .clicked()
            {
                state.export_selected_gallery_scope = true;
            }
        });
        ui.add_space(4.0);
    }

    let target_kind_lbl = if state.export_selected_gallery_scope && !selected_renderables.is_empty() {
        format!("{}: {}", tr(locale, "export.scope.selected"), selected_renderables.len())
    } else if is_layout {
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
                let mut next = ef.clone();
                next.format = fmt;
                if matches!(fmt, ExportFigureFormat::Pdf) {
                    next.transparent_bg = false;
                }
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
        let mut next = ef.clone();
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
                    selected: ef.unit == slug,
                    enabled: !is_vector,
                    disabled_reason: dim_disabled_reason,
                },
                tokens,
            )
            .clicked()
            {
                let mut next = ef.clone();
                next.unit = String::from(match slug {
                    "mm" => "mm",
                    "in" => "in",
                    _ => "px",
                });
                intents.push(UiIntent::UpdateExportFigureState(Box::new(next)));
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
                let mut next = ef.clone();
                next.width_px = ((mm / 25.4) * f64::from(ef.dpi.max(72))).round().clamp(16.0, 9600.0) as u32;
                intents.push(UiIntent::UpdateExportFigureState(Box::new(next)));
            }
        }
    });

    let dim_disabled_reason = is_vector.then_some("Vector formats (PDF/SVG/EMF) use sheet geometry instead of pixel dimensions");
    let dpi_f = f64::from(ef.dpi.max(72));
    let (w_display, h_display, dim_max) = match ef.unit.as_str() {
        "mm" => ((f64::from(ef.width_px) / dpi_f) * 25.4, (f64::from(ef.height_px) / dpi_f) * 25.4, 1000.0),
        "in" => (f64::from(ef.width_px) / dpi_f, f64::from(ef.height_px) / dpi_f, 40.0),
        _ => (f64::from(ef.width_px), f64::from(ef.height_px), 9600.0),
    };
    let w_cfg = ScrubUnitFieldConfig {
        step_per_pt: 2.0,
        decimals: if ef.unit == "px" { 0 } else { 1 },
        allow_unit_switch: false,
        enabled: !is_vector,
        disabled_reason: dim_disabled_reason,
        ..ScrubUnitFieldConfig::new("export.width", "Width", 1.0, dim_max)
    };
    let w_out = render_scrub_unit_field(ui, w_display, &mut state.export_width_spinner, w_cfg, tokens, meta);
    if !is_vector && let Some(val) = w_out.committed_value {
        let mut next = ef.clone();
        let px = match ef.unit.as_str() {
            "mm" => (val / 25.4) * dpi_f,
            "in" => val * dpi_f,
            _ => val,
        };
        next.width_px = px.round().clamp(16.0, 9600.0) as u32;
        intents.push(UiIntent::UpdateExportFigureState(Box::new(next)));
    }

    let h_cfg = ScrubUnitFieldConfig {
        step_per_pt: 2.0,
        decimals: if ef.unit == "px" { 0 } else { 1 },
        allow_unit_switch: false,
        enabled: !is_vector,
        disabled_reason: dim_disabled_reason,
        ..ScrubUnitFieldConfig::new("export.height", "Height", 1.0, dim_max)
    };
    let h_out = render_scrub_unit_field(ui, h_display, &mut state.export_height_spinner, h_cfg, tokens, meta);
    if !is_vector && let Some(val) = h_out.committed_value {
        let mut next = ef.clone();
        let px = match ef.unit.as_str() {
            "mm" => (val / 25.4) * dpi_f,
            "in" => val * dpi_f,
            _ => val,
        };
        next.height_px = px.round().clamp(16.0, 9600.0) as u32;
        intents.push(UiIntent::UpdateExportFigureState(Box::new(next)));
    }

    ui.add_space(6.0);
    // Transparency (blocked for opaque PDF / EMF).
    let mut transparent = ef.transparent_bg && !is_opaque_vector;
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
        let mut next = ef.clone();
        next.transparent_bg = !next.transparent_bg;
        intents.push(UiIntent::UpdateExportFigureState(Box::new(next)));
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
            let cs_selected = if is_pdf_export && ef.color_space == "CMYK-Proof" {
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
                let mut next = ef.clone();
                next.color_space = String::from(val);
                intents.push(UiIntent::UpdateExportFigureState(Box::new(next)));
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
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    }

    let wants_kb_before = ctx.egui_wants_keyboard_input();
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
        &mut (&mut *state, &mut *intents),
        |_, _, _| {},
        |ui, meta, (state, intents)| {
            if pd.job.sheet_ids.len() > 1 {
                let sheet_summary = format!("{} ({})", tr(locale, "print.selected_sheets"), pd.job.sheet_ids.len());
                ui.label(egui::RichText::new(sheet_summary).color(tokens.text_muted));
                ui.add_space(4.0);
            }
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
            let mut submit_os = pd.submit_to_os;
            let direct_lbl = tr(locale, "print.direct_queue");
            let os_chk = ui.checkbox(&mut submit_os, direct_lbl);
            meta.record_response(
                ui,
                &os_chk,
                FrameInspectEntryInput {
                    widget_id: "print.submit_to_os",
                    role: "checkbox",
                    label: direct_lbl,
                    command_id: None,
                    value: Some(Value::Bool(submit_os)),
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if os_chk.changed() {
                let mut next = pd;
                next.submit_to_os = submit_os;
                intents.push(UiIntent::UpdatePrintDialogState(Box::new(next)));
            }
        },
        |ui, meta, (_, intents)| {
            let cancel_lbl = tr(locale, "btn.cancel");
            let print_lbl = tr(locale, "print.commit");
            render_trailing_modal_footer(
                ui,
                meta,
                body_w,
                &[],
                &[cancel_lbl, print_lbl],
                tokens,
                &mut **intents,
                |_, _, _| {},
                |ui, meta, intents| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "modal.cancel",
                            label: cancel_lbl,
                            command_id: None,
                            selected: false,
                            enabled: true,
                            disabled_reason: None,
                        },
                        tokens,
                    )
                    .clicked()
                    {
                        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
                    }
                    if app_button(
                        ui,
                        meta,
                        AppBtn { widget_id: "modal.ok", label: print_lbl, command_id: None, selected: true, enabled: true, disabled_reason: None },
                        tokens,
                    )
                    .clicked()
                    {
                        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
                    }
                },
            );
        },
    );

    if !wants_kb_before && !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    }

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
    let close_lbl = tr(locale, "btn.close");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 400.0);
    let build_sha = option_env!("CURVEBOOK_BUILD_SHA");
    let version_line = format_about_version_line(env!("CARGO_PKG_VERSION"), build_sha);
    let copied_id = egui::Id::unique(("egui_shell_app_modal", "dialog.about.copied"));

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        ctx.data_mut(|d| d.insert_temp(copied_id, false));
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    }

    let modal_resp = egui::Modal::new(egui::Id::unique(("egui_shell_app_modal", "dialog.about"))).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(max_h);
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

        ui.horizontal(|ui| {
            ui.set_width(inner_w);
            ui.heading(title);
            let close_dim = tokens.metrics.min_hit_target_pt;
            let spacer = (ui.available_width() - close_dim).max(tokens.metrics.space_sm);
            ui.add_space(spacer);
            let (close_rect, close_resp) = ui.allocate_exact_size(egui::Vec2::splat(close_dim), egui::Sense::click());
            if close_resp.hovered() || close_resp.has_focus() {
                let border = if close_resp.has_focus() { Stroke::new(tokens.metrics.stroke_focus_pt, tokens.focus_ring) } else { Stroke::NONE };
                ui.painter().rect(close_rect, egui::CornerRadius::same(tokens.metrics.radius_md), tokens.hover_bg, border, egui::StrokeKind::Inside);
            }
            let fg = if close_resp.hovered() { tokens.text_primary } else { tokens.text_muted };
            ui.painter().text(close_rect.center(), egui::Align2::CENTER_CENTER, "×", egui::FontId::proportional(tokens.metrics.font_ui_title_pt), fg);
            let close_a11y = close_lbl.to_owned();
            close_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, true, close_a11y.clone()));
            app_record_visible(
                ui,
                &close_resp,
                meta,
                FrameInspectEntryInput {
                    widget_id: "dialog.about.close",
                    role: "button",
                    label: close_lbl,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::ModalDialog,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if close_resp.clicked() {
                ctx.data_mut(|d| d.insert_temp(copied_id, false));
                intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
            }
        });
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
                    let mark_size = crate::egui_shell::brand::ABOUT_MARK_SIZE;
                    let (mark_rect, _) = ui.allocate_exact_size(mark_size, egui::Sense::hover());
                    crate::egui_shell::brand::paint_mark(ui, mark_rect);
                    let text_w = (body_w - mark_size.x - ui.spacing().item_spacing.x).max(80.0);
                    ui.vertical(|ui| {
                        ui.set_max_width(text_w);
                        ui.label(egui::RichText::new(&version_line).strong());
                        ui.add(egui::Label::new("Native scientific biostatistics, curve fitting, and publication graphing suite.").wrap());
                    });
                });
                ui.add_space(8.0);
                ui.add(egui::Label::new("Copyright © 2026 CurveBook Contributors").wrap());
                ui.add(egui::Label::new("Licensed under MIT OR Apache-2.0.").wrap());
                ui.add(egui::Label::new("Developed by the Hauser Group · hausergroup.org").wrap());
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("See NOTICE and ATTRIBUTION.md in the distribution for third-party licenses and clean-room notices.")
                            .color(tokens.text_muted),
                    )
                    .wrap(),
                );
            });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);

        let copied = ctx.data(|d| d.get_temp::<bool>(copied_id).unwrap_or(false));
        let copy_lbl = if copied { "Copied" } else { "Copy version info" };
        let feedback_lbl = tr(locale, "menu.help.feedback");
        render_trailing_modal_footer(
            ui,
            meta,
            body_w,
            &[copy_lbl],
            &[feedback_lbl],
            tokens,
            intents,
            |ui, meta, _intents| {
                let copy_resp = app_button(
                    ui,
                    meta,
                    AppBtn {
                        widget_id: "dialog.about.copy_version",
                        label: copy_lbl,
                        command_id: None,
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    tokens,
                );
                if copy_resp.clicked() {
                    let summary = format_about_diagnostic_summary(app, build_sha);
                    let status = app
                        .write_clipboard_text(&summary)
                        .unwrap_or_else(|err| crate::os_integration::OsAdapterStatus::unavailable("os-pasteboard", err));
                    ctx.data_mut(|d| {
                        d.insert_temp(copied_id, true);
                        d.insert_temp(egui::Id::unique("about.copy_version.unavailable"), !status.available);
                    });
                }
            },
            |ui, meta, intents| {
                let feedback_resp = app_button(
                    ui,
                    meta,
                    AppBtn {
                        widget_id: "dialog.about.send_feedback",
                        label: feedback_lbl,
                        command_id: Some("app.feedback"),
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    tokens,
                );
                if feedback_resp.clicked() {
                    ctx.data_mut(|d| d.insert_temp(copied_id, false));
                    intents.push_ui("app.feedback", json!({}));
                }
            },
        );
        if ctx.data(|d| d.get_temp::<bool>(egui::Id::unique("about.copy_version.unavailable")).unwrap_or(false)) {
            ui.label(egui::RichText::new("Host clipboard unavailable in this session.").small().color(tokens.text_muted));
        }
    });

    if modal_resp.backdrop_response.clicked() {
        ctx.data_mut(|d| d.insert_temp(copied_id, false));
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    }

    modal_resp.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Dialog, true, title));
    let capture =
        finalize_egui_modal_response(ctx, &modal_resp, ModalCaptureSpec { widget_id: "dialog.about", title, z_dialog: ZLayer::ModalDialog }, meta);
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
        intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
    }

    let mut name_draft = app.canvas_dialogs.checkpoints.name_draft.clone();
    let mut note_draft = app.canvas_dialogs.checkpoints.note_draft.clone();
    let doc = app.document();

    let wants_kb_before = ctx.egui_wants_keyboard_input();
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
        &mut (&mut *state, &mut *intents),
        |_, _, _| {},
        |ui, meta, (state, intents)| {
            ui.label(egui::RichText::new("Create Checkpoint").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(tr(locale, "modal.label.name"));
                let name_resp = ui.add(egui::TextEdit::singleline(&mut name_draft).desired_width((body_w - 64.0).max(120.0)));
                app_input(ui, &name_resp, meta, "checkpoints.name", "Checkpoint Name", &name_draft, true);
                if name_resp.changed() {
                    intents.push(UiIntent::SetCheckpointsDraft { name: Some(name_draft.clone()), note: None });
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(tr(locale, "modal.label.note"));
                let note_resp = ui.add(egui::TextEdit::singleline(&mut note_draft).desired_width((body_w - 64.0).max(120.0)));
                app_input(ui, &note_resp, meta, "checkpoints.note", "Checkpoint Note", &note_draft, true);
                if note_resp.changed() {
                    intents.push(UiIntent::SetCheckpointsDraft { name: None, note: Some(note_draft.clone()) });
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
                intents.push(UiIntent::SetCheckpointsDraft { name: Some(name_draft.clone()), note: Some(note_draft.clone()) });
                intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
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
                    intents.push(UiIntent::DialogButtonAction { action: String::from("cancel") });
                }
            });
        },
    );

    if !app.canvas_dialogs.checkpoints.name_draft.trim().is_empty()
        && !wants_kb_before
        && !ctx.egui_wants_keyboard_input()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
    {
        state.checkpoint_pending_restore_id = None;
        intents.push(UiIntent::DialogButtonAction { action: String::from("ok") });
    }

    report_from_capture(capture)
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
    let primary = entry.selected && matches!(entry.widget_id, "modal.ok" | "dialog.button.save" | "import_project_mode.merge");
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

fn estimate_egui_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text_w = ui.painter().layout_no_wrap(label.to_owned(), font, ui.visuals().text_color()).size().x;
    (text_w + ui.spacing().button_padding.x * 2.0).max(ui.spacing().interact_size.x)
}

#[allow(clippy::too_many_arguments, clippy::cast_precision_loss)]
fn render_trailing_modal_footer<S>(
    ui: &mut Ui,
    meta: &mut FrameInspectMeta,
    body_w: f32,
    leading_labels: &[&str],
    trailing_labels: &[&str],
    tokens: &Tokens,
    state: &mut S,
    render_leading: impl FnOnce(&mut Ui, &mut FrameInspectMeta, &mut S),
    render_trailing: impl FnOnce(&mut Ui, &mut FrameInspectMeta, &mut S),
) {
    let item_gap = ui.spacing().item_spacing.x;
    let group_width = |labels: &[&str]| -> f32 {
        if labels.is_empty() {
            0.0
        } else {
            let sum: f32 = labels.iter().map(|lbl| estimate_egui_button_width(ui, lbl)).sum();
            sum + item_gap * (labels.len().saturating_sub(1) as f32)
        }
    };
    let leading_w = group_width(leading_labels);
    let trailing_w = group_width(trailing_labels);
    let min_sep = if leading_labels.is_empty() { 0.0 } else { tokens.metrics.space_lg.max(item_gap) };
    let fits_single_row = leading_labels.is_empty() || (leading_w + min_sep + trailing_w <= body_w);
    if fits_single_row {
        ui.allocate_ui(egui::Vec2::new(body_w, tokens.metrics.min_hit_target_pt), |ui| {
            ui.horizontal(|ui| {
                if !leading_labels.is_empty() {
                    render_leading(ui, meta, state);
                }
                let spacer = (ui.available_width() - trailing_w).max(min_sep);
                if spacer > 0.0 {
                    ui.add_space(spacer);
                }
                render_trailing(ui, meta, state);
            });
        });
    } else {
        ui.vertical(|ui| {
            ui.set_max_width(body_w);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(body_w);
                render_leading(ui, meta, state);
            });
            ui.add_space(tokens.metrics.space_sm);
            ui.allocate_ui(egui::Vec2::new(body_w, tokens.metrics.min_hit_target_pt), |ui| {
                ui.horizontal(|ui| {
                    let spacer = (ui.available_width() - trailing_w).max(0.0);
                    if spacer > 0.0 {
                        ui.add_space(spacer);
                    }
                    render_trailing(ui, meta, state);
                });
            });
        });
    }
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

// ============================================================================
// 8. Import Warnings Modal (R10.29)
// ============================================================================

fn render_import_warnings_modal(
    ctx: &Context,
    app: &UiApp,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "import.warnings.title");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 580.0);

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::CloseAllModals);
    }

    let state = &app.import_warnings;
    let count = state.warnings.len();

    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique(("egui_shell_app_modal", "dialog.import_warnings")),
            scroll_id: "app_modal.import_warnings.scroll",
            widget_id: "dialog.import_warnings",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 116.0).max(120.0),
            footer_separator: true,
        },
        intents,
        |_, _, _| {},
        |ui, meta, _| {
            if count == 0 {
                let empty_lbl = tr(locale, "import.warnings.empty");
                let resp = ui.add(egui::Label::new(egui::RichText::new(empty_lbl).color(tokens.text_muted)).wrap());
                app_text(ui, &resp, meta, "import_warnings.empty", empty_lbl);
                return;
            }

            let fallback_source = match state.command_id.as_str() {
                "table.importPzfx" => tr(locale, "w6.view.shell.filter_xml_project"),
                "table.importPrism" => tr(locale, "w6.view.shell.filter_archive"),
                "table.importXlsx" => tr(locale, "cmd.table.importXlsx"),
                _ => "",
            };
            let source_display = if state.source_label.is_empty() { fallback_source } else { state.source_label.as_str() };
            let summary = if source_display.is_empty() {
                format!("{count} warning{}", if count == 1 { "" } else { "s" })
            } else {
                format!("{count} warning{} — {source_display}", if count == 1 { "" } else { "s" })
            };
            let sum_resp = ui.add(egui::Label::new(egui::RichText::new(&summary).strong().color(tokens.text_primary)).wrap());
            app_text(ui, &sum_resp, meta, "import_warnings.summary", &summary);
            ui.add_space(6.0);

            for (idx, item) in state.warnings.iter().enumerate() {
                egui::Frame::group(ui.style()).fill(tokens.surface_bg).stroke(Stroke::new(1.0, tokens.border)).show(ui, |ui| {
                    ui.set_max_width((body_w - 18.0).max(140.0));
                    let header = match (item.kind.trim().is_empty(), item.source.trim().is_empty()) {
                        (false, false) => format!("[{}] {}", item.kind.trim(), item.source.trim()),
                        (false, true) => format!("[{}]", item.kind.trim()),
                        (true, false) => item.source.trim().to_owned(),
                        (true, true) => format!("#{}", idx + 1),
                    };
                    ui.label(egui::RichText::new(header).small().strong().color(tokens.warning));
                    let msg_resp = ui.add(egui::Label::new(egui::RichText::new(&item.message).color(tokens.text_primary)).wrap());
                    let wid = format!("import_warnings.item.{idx}");
                    app_text(ui, &msg_resp, meta, &wid, &item.message);
                });
                ui.add_space(4.0);
            }
        },
        |ui, meta, intents| {
            let copy_lbl = tr(locale, "import.warnings.copy_all");
            let close_lbl = tr(locale, "btn.close");
            render_trailing_modal_footer(
                ui,
                meta,
                body_w,
                &[copy_lbl],
                &[close_lbl],
                tokens,
                intents,
                |ui, meta, intents| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "import_warnings.copy_all",
                            label: copy_lbl,
                            command_id: None,
                            selected: false,
                            enabled: count > 0,
                            disabled_reason: (count == 0).then_some("No warnings to copy"),
                        },
                        tokens,
                    )
                    .clicked()
                    {
                        intents.push(UiIntent::CopyImportWarnings);
                    }
                },
                |ui, meta, intents| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "modal.cancel",
                            label: close_lbl,
                            command_id: None,
                            selected: true,
                            enabled: true,
                            disabled_reason: None,
                        },
                        tokens,
                    )
                    .clicked()
                    {
                        intents.push(UiIntent::CloseAllModals);
                    }
                },
            );
        },
    );

    report_from_capture(capture)
}

// ============================================================================
// 9. Replace-or-Merge Import Project Mode Modal (R10.29)
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn render_import_project_mode_modal(
    ctx: &Context,
    app: &UiApp,
    command_id: &str,
    path: &str,
    has_unsaved_changes: bool,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = match command_id {
        "table.importPzfx" => tr(locale, "cmd.table.importPzfx"),
        "table.importPrism" => tr(locale, "cmd.table.importPrism"),
        "table.importXlsx" => tr(locale, "cmd.table.importXlsx"),
        _ => tr(locale, "import.mode.title"),
    };
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 540.0);
    let cmd_owned = command_id.to_owned();
    let path_owned = path.to_owned();

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::CloseAllModals);
    }

    let file_display = std::path::Path::new(path).file_name().and_then(|s| s.to_str()).unwrap_or(path);

    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique(("egui_shell_app_modal", "dialog.import_project_mode")),
            scroll_id: "app_modal.import_project_mode.scroll",
            widget_id: "dialog.import_project_mode",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 116.0).max(110.0),
            footer_separator: true,
        },
        intents,
        |_, _, _| {},
        |ui, meta, _| {
            let file_resp = ui.add(egui::Label::new(egui::RichText::new(file_display).strong().color(tokens.text_primary)).wrap());
            app_text(ui, &file_resp, meta, "import_project_mode.file", file_display);
            ui.add_space(6.0);

            let body = tr(locale, "import.mode.body");
            let body_resp = ui.add(egui::Label::new(egui::RichText::new(body).color(tokens.text_primary)).wrap());
            app_text(ui, &body_resp, meta, "import_project_mode.body", body);

            if has_unsaved_changes {
                ui.add_space(8.0);
                egui::Frame::group(ui.style()).fill(tokens.surface_bg).stroke(Stroke::new(1.5, tokens.warning)).show(ui, |ui| {
                    ui.set_max_width((body_w - 18.0).max(140.0));
                    let warn_text = tr(locale, "import.mode.unsaved_warning");
                    let warn_resp = ui.add(egui::Label::new(egui::RichText::new(warn_text).strong().color(tokens.warning)).wrap());
                    app_text(ui, &warn_resp, meta, "import_project_mode.unsaved_warning", warn_text);
                });
            }
        },
        |ui, meta, intents| {
            let new_window_lbl = crate::i18n::tr_or_fallback(locale, "import.mode.open_in_new_window", "Open in new window");
            let open_new_lbl = tr(locale, "import.mode.open_as_new");
            let cancel_lbl = tr(locale, "btn.cancel");
            let add_lbl = tr(locale, "import.mode.add_to_project");
            render_trailing_modal_footer(
                ui,
                meta,
                body_w,
                &[new_window_lbl, open_new_lbl],
                &[cancel_lbl, add_lbl],
                tokens,
                intents,
                |ui, meta, intents| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "import_project_mode.new_window",
                            label: new_window_lbl,
                            command_id: None,
                            selected: false,
                            enabled: true,
                            disabled_reason: None,
                        },
                        tokens,
                    )
                    .clicked()
                    {
                        intents.push_ui(cmd_owned.clone(), json!({ "path": path_owned, "new_window": true }));
                        intents.push(UiIntent::CloseAllModals);
                    }
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "import_project_mode.replace",
                            label: open_new_lbl,
                            command_id: None,
                            selected: false,
                            enabled: true,
                            disabled_reason: None,
                        },
                        tokens,
                    )
                    .clicked()
                    {
                        intents.push(UiIntent::DialogButtonAction { action: String::from("__import_project_mode_replace") });
                    }
                },
                |ui, meta, intents| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "modal.cancel",
                            label: cancel_lbl,
                            command_id: None,
                            selected: false,
                            enabled: true,
                            disabled_reason: None,
                        },
                        tokens,
                    )
                    .clicked()
                    {
                        intents.push(UiIntent::CloseAllModals);
                    }
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "import_project_mode.merge",
                            label: add_lbl,
                            command_id: None,
                            selected: true,
                            enabled: true,
                            disabled_reason: None,
                        },
                        tokens,
                    )
                    .clicked()
                    {
                        intents.push(UiIntent::DialogButtonAction { action: String::from("__import_project_mode_merge") });
                    }
                },
            );
        },
    );

    report_from_capture(capture)
}

// ============================================================================
// 9b. Export All Data Tables: Existing Files Modal (R10.29)
// ============================================================================

/// Close the prompt and send the user's decision back to the desktop host, which holds the
/// folder and export options for the paused batch.
fn push_export_existing_files_choice(intents: &mut PendingUiIntents, choice: &str) {
    intents.push(UiIntent::CloseAllModals);
    intents.push(UiIntent::UiCommand { command_id: String::from("ui.export.allTablesCsv"), params: json!({ "on_existing": choice }) });
}

fn render_export_all_tables_existing_files_modal(
    ctx: &Context,
    app: &UiApp,
    dir: &str,
    existing_files: &[String],
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "export.csv.existing_title");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 500.0);

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        push_export_existing_files_choice(intents, "cancel");
    }

    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique(("egui_shell_app_modal", "dialog.export_existing_files")),
            scroll_id: "app_modal.export_existing_files.scroll",
            widget_id: "dialog.export_existing_files",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 116.0).max(110.0),
            footer_separator: true,
        },
        intents,
        |_, _, _| {},
        |ui, meta, _| {
            let dir_resp = ui.add(egui::Label::new(egui::RichText::new(dir).strong().color(tokens.text_primary)).wrap());
            app_text(ui, &dir_resp, meta, "export_existing_files.dir", dir);
            ui.add_space(6.0);

            let body = tr(locale, "export.csv.existing_body");
            let body_resp = ui.add(egui::Label::new(egui::RichText::new(body).color(tokens.text_primary)).wrap());
            app_text(ui, &body_resp, meta, "export_existing_files.body", body);
            ui.add_space(4.0);

            for (idx, name) in existing_files.iter().enumerate() {
                let resp = ui.add(egui::Label::new(egui::RichText::new(name).monospace().color(tokens.text_primary)).wrap());
                app_text(ui, &resp, meta, &format!("export_existing_files.file.{idx}"), name);
            }

            ui.add_space(8.0);
            let hint = tr(locale, "export.csv.existing_keep_hint");
            let hint_resp = ui.add(egui::Label::new(egui::RichText::new(hint).color(tokens.text_muted)).wrap());
            app_text(ui, &hint_resp, meta, "export_existing_files.hint", hint);
        },
        |ui, meta, intents| {
            let replace_lbl = tr(locale, "export.csv.existing_replace");
            let cancel_lbl = tr(locale, "btn.cancel");
            let keep_lbl = tr(locale, "export.csv.existing_keep_both");
            render_trailing_modal_footer(
                ui,
                meta,
                body_w,
                &[replace_lbl],
                &[cancel_lbl, keep_lbl],
                tokens,
                intents,
                |ui, meta, intents| {
                    let btn = AppBtn {
                        widget_id: "export_existing_files.replace",
                        label: replace_lbl,
                        command_id: None,
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    };
                    if app_button(ui, meta, btn, tokens).clicked() {
                        push_export_existing_files_choice(intents, "replace");
                    }
                },
                |ui, meta, intents| {
                    for (widget_id, label, selected, choice) in
                        [("modal.cancel", cancel_lbl, false, "cancel"), ("export_existing_files.keep_both", keep_lbl, true, "keep_both")]
                    {
                        let btn = AppBtn { widget_id, label, command_id: None, selected, enabled: true, disabled_reason: None };
                        if app_button(ui, meta, btn, tokens).clicked() {
                            push_export_existing_files_choice(intents, choice);
                        }
                    }
                },
            );
        },
    );

    report_from_capture(capture)
}

// ============================================================================
// 10. CSV Export Options Modal (R10.29)
// ============================================================================

fn emit_csv_export_intent(target: &CsvExportTargetKind, opts: &crate::dialogs_canvas::CsvExportOptionsState, intents: &mut PendingUiIntents) {
    let delimiter = opts.normalized_delimiter().to_string();
    let decimal_separator = opts.normalized_decimal_separator().to_string();
    match target {
        CsvExportTargetKind::ResultSheet { sheet_id } => {
            let mut params = serde_json::Map::new();
            params.insert(String::from("sheet_id"), json!(sheet_id.as_str()));
            if let Some(tab_id) = &opts.tab_id {
                params.insert(String::from("tab_id"), json!(tab_id));
            }
            params.insert(String::from("delimiter"), json!(delimiter));
            params.insert(String::from("decimal_separator"), json!(decimal_separator));
            intents.push(UiIntent::EngineCommand { command_id: String::from("analysis.exportCsv"), params: Value::Object(params) });
        }
        CsvExportTargetKind::DataTable { sheet_id } => {
            intents.push(UiIntent::EngineCommand {
                command_id: String::from("table.exportCsv"),
                params: json!({
                    "sheet_id": sheet_id.as_str(),
                    "delimiter": delimiter,
                    "decimal_separator": decimal_separator,
                }),
            });
        }
        CsvExportTargetKind::DataTables { sheet_ids } => {
            for sheet_id in sheet_ids {
                intents.push(UiIntent::EngineCommand {
                    command_id: String::from("table.exportCsv"),
                    params: json!({
                        "sheet_id": sheet_id.as_str(),
                        "delimiter": delimiter,
                        "decimal_separator": decimal_separator,
                    }),
                });
            }
        }
        CsvExportTargetKind::AllDataTables => {
            intents.push(UiIntent::UiCommand {
                command_id: String::from("ui.export.allTablesCsv"),
                params: json!({
                    "delimiter": delimiter,
                    "decimal_separator": decimal_separator,
                }),
            });
        }
    }
    intents.push(UiIntent::CloseAllModals);
}

fn render_csv_export_options_modal(
    ctx: &Context,
    app: &UiApp,
    target: &CsvExportTargetKind,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let doc = app.document();
    let (title, target_valid) = match target {
        CsvExportTargetKind::ResultSheet { sheet_id } => (tr(locale, "export.csv.title_results"), doc.result_sheet(sheet_id).is_some()),
        CsvExportTargetKind::DataTable { sheet_id } => (tr(locale, "export.csv.title_table"), doc.data_sheet(sheet_id).is_some()),
        CsvExportTargetKind::DataTables { sheet_ids } => {
            (tr(locale, "export.csv.title_selected_tables"), !sheet_ids.is_empty() && sheet_ids.iter().all(|id| doc.data_sheet(id).is_some()))
        }
        CsvExportTargetKind::AllDataTables => (tr(locale, "export.csv.title_all_tables"), !doc.data_sheets.is_empty()),
    };
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 480.0);

    let opts = app.canvas_dialogs.csv_export.clone();
    let changed = false;
    let wants_kb_before = ctx.egui_wants_keyboard_input();

    let mut enter_export_opts: Option<crate::dialogs_canvas::CsvExportOptionsState> = None;
    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique(("egui_shell_app_modal", "dialog.csv_export_options")),
            scroll_id: "app_modal.csv_export.scroll",
            widget_id: "dialog.csv_export_options",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 116.0).max(120.0),
            footer_separator: true,
        },
        &mut (opts, changed, &mut *intents, &mut enter_export_opts),
        |_, _, _| {},
        |ui, meta, (opts, changed, _, _)| {
            if let CsvExportTargetKind::ResultSheet { sheet_id } = target
                && let Some(res) = doc.result_sheet(sheet_id)
            {
                ui.label(egui::RichText::new(tr(locale, "export.csv.tabs_label")).strong().color(tokens.text_primary));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(body_w);
                    if app_tab(ui, meta, "csv_export.tab.all", tr(locale, "export.csv.all_tabs_tidy"), opts.tab_id.is_none(), tokens).clicked() {
                        opts.tab_id = None;
                        *changed = true;
                    }
                    for (idx, tab) in res.tabs.iter().enumerate() {
                        let selected = opts.tab_id.as_deref() == Some(tab.id.as_str());
                        let wid = format!("csv_export.tab.{idx}");
                        if app_tab(ui, meta, &wid, &tab.title, selected, tokens).clicked() {
                            opts.tab_id = Some(tab.id.clone());
                            *changed = true;
                        }
                    }
                });
                ui.add_space(8.0);
            }

            ui.label(egui::RichText::new(tr(locale, "export.csv.delimiter_label")).strong().color(tokens.text_primary));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(body_w);
                let delims = [
                    (',', "csv_export.delim.comma", tr(locale, "export.csv.delim_comma")),
                    (';', "csv_export.delim.semicolon", tr(locale, "export.csv.delim_semicolon")),
                    ('\t', "csv_export.delim.tab", tr(locale, "export.csv.delim_tab")),
                ];
                for (ch, wid, lbl) in delims {
                    let selected = opts.normalized_delimiter() == ch;
                    if app_tab(ui, meta, wid, lbl, selected, tokens).clicked() && !selected {
                        opts.delimiter = ch;
                        *changed = true;
                    }
                }
            });

            ui.add_space(8.0);
            ui.label(egui::RichText::new(tr(locale, "export.csv.decimal_label")).strong().color(tokens.text_primary));
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(body_w);
                let decs = [
                    ('.', "csv_export.decimal.period", tr(locale, "export.csv.decimal_period")),
                    (',', "csv_export.decimal.comma", tr(locale, "export.csv.decimal_comma")),
                ];
                for (ch, wid, lbl) in decs {
                    let selected = opts.normalized_decimal_separator() == ch;
                    if app_tab(ui, meta, wid, lbl, selected, tokens).clicked() && !selected {
                        opts.decimal_separator = ch;
                        if ch == ',' && opts.normalized_delimiter() == ',' {
                            opts.delimiter = ';';
                        }
                        *changed = true;
                    }
                }
            });

            ui.add_space(8.0);
            let hint = tr(locale, "export.csv.tsv_hint");
            let hint_resp = ui.add(egui::Label::new(egui::RichText::new(hint).small().color(tokens.text_muted)).wrap());
            app_text(ui, &hint_resp, meta, "csv_export.tsv_hint", hint);
        },
        |ui, meta, (opts, changed, intents, enter_export_opts)| {
            if *changed {
                intents.push(UiIntent::UpdateCsvExportOptions(Box::new(opts.clone())));
            }
            **enter_export_opts = Some(opts.clone());
            let cancel_lbl = tr(locale, "btn.cancel");
            let export_lbl = tr(locale, "btn.export");
            render_trailing_modal_footer(
                ui,
                meta,
                body_w,
                &[],
                &[cancel_lbl, export_lbl],
                tokens,
                intents,
                |_, _, _| {},
                |ui, meta, intents| {
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "modal.cancel",
                            label: cancel_lbl,
                            command_id: None,
                            selected: false,
                            enabled: true,
                            disabled_reason: None,
                        },
                        tokens,
                    )
                    .clicked()
                    {
                        intents.push(UiIntent::CloseAllModals);
                    }
                    if app_button(
                        ui,
                        meta,
                        AppBtn {
                            widget_id: "modal.ok",
                            label: export_lbl,
                            command_id: None,
                            selected: true,
                            enabled: target_valid,
                            disabled_reason: (!target_valid).then_some("No eligible sheet available to export"),
                        },
                        tokens,
                    )
                    .clicked()
                    {
                        emit_csv_export_intent(target, opts, intents);
                    }
                },
            );
        },
    );

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::CloseAllModals);
    } else if target_valid
        && !wants_kb_before
        && !ctx.egui_wants_keyboard_input()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
        && let Some(final_opts) = enter_export_opts.as_ref()
    {
        emit_csv_export_intent(target, final_opts, intents);
    }

    report_from_capture(capture)
}

// ============================================================================
// 11. `.pzfx` Pre-Export Confirmation Modal (§4.2, §6.1, §6.3)
// ============================================================================

fn pzfx_dropped_kind_label(locale: &str, kind: curvebook_format::PzfxDroppedKind) -> &'static str {
    use curvebook_format::PzfxDroppedKind;
    match kind {
        PzfxDroppedKind::AnalysesAndResults => tr(locale, "export.pzfx.kind.analyses"),
        PzfxDroppedKind::Graphs => tr(locale, "export.pzfx.kind.graphs"),
        PzfxDroppedKind::Layouts => tr(locale, "export.pzfx.kind.layouts"),
        PzfxDroppedKind::LiveLinks => tr(locale, "export.pzfx.kind.live_links"),
        PzfxDroppedKind::Images => tr(locale, "export.pzfx.kind.images"),
        PzfxDroppedKind::Checkpoints => tr(locale, "export.pzfx.kind.checkpoints"),
        PzfxDroppedKind::FormulasWrittenAsValues => tr(locale, "export.pzfx.kind.formulas"),
        PzfxDroppedKind::UnrepresentableTableFeatures => tr(locale, "export.pzfx.kind.unrepresentable"),
    }
}

fn render_pzfx_export_confirm_modal(
    ctx: &Context,
    app: &UiApp,
    preflight: &curvebook_format::PzfxExportPreflight,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> AppModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let title = tr(locale, "export.pzfx.confirm_title");
    let (inner_w, max_h, body_w) = modal_bounds(ctx, 540.0);

    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        intents.push(UiIntent::CloseAllModals);
    }

    let capture = show_scrolled_modal(
        ctx,
        meta,
        ScrolledModalCfg {
            tokens,
            modal_id: egui::Id::unique(("egui_shell_app_modal", "dialog.pzfx_export_confirm")),
            scroll_id: "app_modal.pzfx_export_confirm.scroll",
            widget_id: "dialog.pzfx_export_confirm",
            title,
            inner_w,
            max_h,
            body_w,
            body_max_h: (max_h - 116.0).max(120.0),
            footer_separator: true,
        },
        intents,
        |_, _, _| {},
        |ui, meta, _| {
            let data_only = tr(locale, "export.pzfx.data_only_notice");
            let data_only_resp = ui.add(egui::Label::new(egui::RichText::new(data_only).color(tokens.text_primary)).wrap());
            app_text(ui, &data_only_resp, meta, "pzfx_export_confirm.data_only_notice", data_only);
            ui.add_space(6.0);

            let structure = tr(locale, "export.pzfx.structure_notice");
            let structure_resp = ui.add(egui::Label::new(egui::RichText::new(structure).color(tokens.text_muted)).wrap());
            app_text(ui, &structure_resp, meta, "pzfx_export_confirm.structure_notice", structure);
            ui.add_space(8.0);

            let dropped_heading = tr(locale, "export.pzfx.dropped_heading");
            let head_text = format!("{dropped_heading} ({})", preflight.total_dropped_count());
            let head_resp = ui.add(egui::Label::new(egui::RichText::new(&head_text).strong().color(tokens.text_strong)).wrap());
            app_text(ui, &head_resp, meta, "pzfx_export_confirm.dropped_heading", &head_text);
            ui.add_space(4.0);

            for (idx, group) in preflight.dropped.iter().enumerate() {
                let kind_lbl = pzfx_dropped_kind_label(locale, group.kind);
                let header = format!("{kind_lbl} ({})", group.count);
                let hdr_resp = ui.add(egui::Label::new(egui::RichText::new(&header).strong().color(tokens.warning)).wrap());
                app_text(ui, &hdr_resp, meta, &format!("pzfx_export_confirm.group.{idx}"), &header);
                if !group.items.is_empty() {
                    let items_text = group.items.join(", ");
                    let items_resp = ui.add(egui::Label::new(egui::RichText::new(&items_text).color(tokens.text_primary)).wrap());
                    app_text(ui, &items_resp, meta, &format!("pzfx_export_confirm.group.{idx}.items"), &items_text);
                }
                ui.add_space(4.0);
            }
        },
        |ui, meta, intents| {
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
                    intents.push(UiIntent::CloseAllModals);
                }
                if app_button(
                    ui,
                    meta,
                    AppBtn {
                        widget_id: "pzfx_export_confirm.export",
                        label: tr(locale, "export.pzfx.confirm_action"),
                        command_id: Some("project.exportPzfx"),
                        selected: true,
                        enabled: true,
                        disabled_reason: None,
                    },
                    tokens,
                )
                .clicked()
                {
                    intents.push(UiIntent::EngineCommand { command_id: String::from("project.exportPzfx"), params: json!({}) });
                    intents.push(UiIntent::CloseAllModals);
                }
            });
        },
    );

    report_from_capture(capture)
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
