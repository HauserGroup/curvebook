//! Leading Dock: collapsible Project Navigator (`240 pt -> 36 pt` rail, `Cmd/Ctrl+Shift+N`) +
//! Scoped Family Lineage Tree (`~65% / ~35%` split, `Cmd/Ctrl+Shift+F`, `>= 450 ms` hover preview)
//! (`docs/planning/gui-layout-spec.md` §4, `R15.3`).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use curvebook_doc::{LayoutItem, ProjectDocument, ResultStatus, SheetId, SheetSection};
use egui::{Align, Color32, CornerRadius, FontId, Id, Key, Layout, Modifiers, Order, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::inspect_bridge::{
    ModalCaptureReport, ModalCaptureSpec, consume_matching_shortcut, finalize_egui_modal_response, has_registered_shortcut,
};
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, ResponsiveShellLayout, Tokens, UiIntent, doc_sheet_title};
use crate::inspect::ZLayer;
use crate::tr;
use crate::workspace_nav::{
    ActiveWorkspacePane, FamilyPaneNode, FamilyRelation, NavigatorEntry, NavigatorNestingMode, collect_downstream_dependents,
};

/// Quick kind filter chip in the Navigator filter header (`§4.1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NavigatorKindChip {
    #[default]
    All,
    Data,
    Info,
    Results,
    Graphs,
    Layouts,
}

impl NavigatorKindChip {
    /// Ordered list of filter chips displayed in the Navigator header.
    pub const ALL: [Self; 6] = [Self::All, Self::Data, Self::Info, Self::Results, Self::Graphs, Self::Layouts];

    /// Whether entries in `section` pass this filter chip.
    #[must_use]
    pub const fn matches_section(self, section: SheetSection) -> bool {
        match self {
            Self::All => true,
            Self::Data => matches!(section, SheetSection::Data),
            Self::Info => matches!(section, SheetSection::Info),
            Self::Results => matches!(section, SheetSection::Analyses),
            Self::Graphs => matches!(section, SheetSection::Graphs),
            Self::Layouts => matches!(section, SheetSection::Layouts),
        }
    }

    /// Stable inspector widget suffix for this chip.
    #[must_use]
    pub const fn widget_id(self) -> &'static str {
        match self {
            Self::All => "nav.chip.all",
            Self::Data => "nav.chip.data",
            Self::Info => "nav.chip.info",
            Self::Results => "nav.chip.results",
            Self::Graphs => "nav.chip.graphs",
            Self::Layouts => "nav.chip.layouts",
        }
    }

    /// Concise localized or canonical chip label.
    #[must_use]
    pub fn label(self, locale: &str) -> &'static str {
        match self {
            Self::All => "All",
            Self::Data => tr(locale, "nav.data_tables"),
            Self::Info => tr(locale, "nav.info"),
            Self::Results => tr(locale, "nav.results"),
            Self::Graphs => tr(locale, "nav.graphs"),
            Self::Layouts => tr(locale, "nav.layouts"),
        }
    }
}

/// Active right-click context menu target inside the Navigator tree (`R15-GUI-38`).
#[derive(Debug, Clone, PartialEq)]
pub struct NavigatorContextMenuState {
    pub sheet_id: SheetId,
    pub section: SheetSection,
    pub is_frozen: bool,
    pub anchor: Pos2,
}

/// Active cascade-delete confirmation modal target triggered from Navigator context menu (`R15-GUI-38`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigatorDeleteConfirmState {
    pub sheet_id: SheetId,
    pub title: String,
    pub dependents: Vec<(SheetId, String)>,
}

/// Persistent `egui_shell` UI state for the Leading Dock (`Navigator` + `Family`).
#[derive(Debug, Clone, PartialEq)]
pub struct LeadingDockEguiState {
    pub search_draft: String,
    pub last_synced_query: String,
    pub kind_chip: NavigatorKindChip,
    pub nav_split_fraction: f32,
    pub renaming_sheet_id: Option<SheetId>,
    pub rename_draft: String,
    pub rename_initial_title: String,
    pub rename_just_started: bool,
    pub dragging_sheet_id: Option<SheetId>,
    pub hover_preview_sheet_id: Option<SheetId>,
    /// Monotonic `egui` frame clock (`ctx.input(|i| i.time)`) when hover started (`>= 0.45 s` triggers preview).
    pub hover_started_sec: Option<f64>,
    pub hover_preview_active: bool,
    pub nav_pointer_hovered: bool,
    pub nested_collapsed_parents: BTreeSet<SheetId>,
    pub scroll_to_sheet: Option<SheetId>,
    pub context_menu: Option<NavigatorContextMenuState>,
    pub delete_confirm: Option<NavigatorDeleteConfirmState>,
}

impl Default for LeadingDockEguiState {
    fn default() -> Self {
        Self {
            search_draft: String::new(),
            last_synced_query: String::new(),
            kind_chip: NavigatorKindChip::All,
            nav_split_fraction: 0.65,
            renaming_sheet_id: None,
            rename_draft: String::new(),
            rename_initial_title: String::new(),
            rename_just_started: false,
            dragging_sheet_id: None,
            hover_preview_sheet_id: None,
            hover_started_sec: None,
            hover_preview_active: false,
            nav_pointer_hovered: false,
            nested_collapsed_parents: BTreeSet::new(),
            scroll_to_sheet: None,
            context_menu: None,
            delete_confirm: None,
        }
    }
}

/// Input context bundle for [`render_leading_dock`] (`<= 7` function arguments).
#[derive(Debug, Clone, Copy)]
pub struct LeadingDockContext<'a> {
    pub tokens: &'a Tokens,
    pub layout: ResponsiveShellLayout,
    pub enabled: bool,
    pub focused: bool,
}

/// Per-frame geometry report emitted by [`render_leading_dock`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LeadingDockFrameReport {
    pub dock_rect_pt: Option<Rect>,
    pub navigator_rect_pt: Option<Rect>,
    pub family_rect_pt: Option<Rect>,
    pub splitter_rect_pt: Option<Rect>,
    pub hover_preview_rect_pt: Option<Rect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragDropOutcome {
    None,
    ReorderCandidate { section: SheetSection, target_index: usize },
    RetargetCompatible,
    RetargetIncompatible,
}

#[derive(Debug, Default)]
struct NavFrameSignals {
    any_row_hovered_family: Option<SheetId>,
    drop_commit: Option<DropCommitAction>,
}

#[derive(Debug, Clone)]
enum DropCommitAction {
    Reorder { sheet_id: SheetId, target_index: usize },
    Retarget { result_id: SheetId, target_data_id: SheetId },
}

#[derive(Clone, Copy)]
struct RailIconButtonSpec<'a> {
    widget_id: &'a str,
    glyph: &'a str,
    tooltip: &'a str,
    command_id: Option<&'a str>,
    selected: bool,
    enabled: bool,
}

#[derive(Clone, Copy)]
struct NavigatorTreeCtx<'a> {
    app: &'a UiApp,
    dock: LeadingDockContext<'a>,
    row_w: f32,
    query: &'a str,
}

#[derive(Clone, Copy)]
struct FolderHeaderSpec<'a> {
    section: SheetSection,
    count: usize,
    collapsed: bool,
    row_w: f32,
    tokens: &'a Tokens,
    enabled: bool,
}

#[derive(Clone, Copy)]
struct NavigatorRowSpec<'a> {
    app: &'a UiApp,
    dock: LeadingDockContext<'a>,
    entry: &'a NavigatorEntry,
    section_index: usize,
    indent_depth: usize,
    is_nested_parent: bool,
    nested_collapsed: bool,
    row_w: f32,
}

#[derive(Clone, Copy)]
struct InlineRenameRowSpec<'a> {
    app: &'a UiApp,
    entry: &'a NavigatorEntry,
    rect: Rect,
    enabled: bool,
}

#[derive(Clone, Copy)]
struct SectionNewRowSpec<'a> {
    section: SheetSection,
    row_w: f32,
    tokens: &'a Tokens,
    enabled: bool,
}

struct FamilyHoverPreviewInput<'a> {
    hovered_family: Option<SheetId>,
    tokens: &'a Tokens,
    dock_rect: Rect,
    rtl_mirrored: bool,
}

#[derive(Clone, Copy)]
struct CompactChipSpec<'a> {
    max_w: f32,
    widget_id: &'a str,
    label: &'a str,
    selected: bool,
    enabled: bool,
}

/// Render the Leading Dock (`240 pt -> 36 pt` collapsed rail) containing `Navigator` and `Family`.
pub fn render_leading_dock(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut LeadingDockEguiState,
    ctx_in: LeadingDockContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> LeadingDockFrameReport {
    let mut report = LeadingDockFrameReport::default();
    if ctx_in.enabled {
        handle_leading_dock_shortcuts(ui, app, state, &ctx_in, intents);
    }

    if !ctx_in.layout.leading_visible || ctx_in.layout.leading_dock_w_pt <= 0.0 {
        return report;
    }

    let tokens = ctx_in.tokens;
    let w = ctx_in.layout.leading_dock_w_pt;
    let dock_frame = egui::Frame::NONE
        .fill(tokens.panel_bg)
        .stroke(Stroke::new(1.0, tokens.divider))
        .inner_margin(if ctx_in.layout.leading_collapsed_to_rail { egui::Margin::same(4) } else { egui::Margin::ZERO });

    let mut hovered_family_this_frame = None;
    let panel_builder =
        if ctx_in.layout.rtl_mirrored { egui::Panel::right("egui_shell.leading_dock") } else { egui::Panel::left("egui_shell.leading_dock") };
    let panel_resp = panel_builder.exact_size(w).resizable(false).frame(dock_frame).show(ui, |ui| {
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
        if !ctx_in.enabled {
            ui.disable();
        }

        if ctx_in.layout.leading_collapsed_to_rail {
            render_collapsed_icon_rail(ui, app, state, tokens, ctx_in.layout.rtl_mirrored, ctx_in.enabled, meta, intents);
            return;
        }

        let mut signals = NavFrameSignals::default();
        let full_rect = ui.available_rect_before_wrap();
        let inner_h = full_rect.height().max(120.0);
        state.nav_split_fraction = state.nav_split_fraction.clamp(0.30, 0.85);

        let show_family_body = ctx_in.layout.family_visible && app.family_open && inner_h >= 220.0;
        let family_bar_h = 24.0;
        let splitter_h = if show_family_body { 6.0 } else { 0.0 };
        let family_h =
            if show_family_body { (inner_h * (1.0 - state.nav_split_fraction)).clamp(84.0, (inner_h - 116.0).max(84.0)) } else { family_bar_h };
        let nav_h = (inner_h - family_h - splitter_h).max(60.0);

        let nav_rect = Rect::from_min_size(full_rect.min, Vec2::new(full_rect.width(), nav_h));
        report.navigator_rect_pt = Some(nav_rect);
        state.nav_pointer_hovered = ui.ctx().input(|i| i.pointer.hover_pos().is_some_and(|p| nav_rect.contains(p)));

        let mut nav_ui = ui.new_child(egui::UiBuilder::new().max_rect(nav_rect).layout(Layout::top_down(Align::Min)));
        nav_ui.set_clip_rect(nav_rect);
        render_navigator_pane(&mut nav_ui, app, state, ctx_in, &mut signals, meta, intents);

        if show_family_body {
            let split_rect = Rect::from_min_size(Pos2::new(full_rect.left(), nav_rect.bottom()), Vec2::new(full_rect.width(), splitter_h));
            report.splitter_rect_pt = Some(split_rect);
            let split_hit = split_rect.expand2(Vec2::new(0.0, 2.0));
            let split_sense = if ctx_in.enabled { Sense::drag() } else { Sense::hover() };
            let split_resp = ui.interact(split_hit, ui.scope_id().with("nav_family_splitter"), split_sense);
            let split_enabled = ctx_in.enabled;
            split_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Splitter, split_enabled, "Navigator / Family Split"));
            ui.painter().hline(split_rect.x_range(), split_rect.center().y, Stroke::new(1.0, tokens.divider));
            let handle_rect = Rect::from_center_size(split_rect.center(), Vec2::new(28.0, 2.0));
            ui.painter().rect_filled(
                handle_rect,
                CornerRadius::same(1),
                if split_resp.hovered() || split_resp.dragged() { tokens.accent } else { tokens.border },
            );
            if split_resp.dragged()
                && ctx_in.enabled
                && let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos())
            {
                let frac = ((pos.y - full_rect.top()) / inner_h.max(1.0)).clamp(0.30, 0.85);
                state.nav_split_fraction = frac;
            }
            meta.record_response(
                ui,
                &split_resp,
                FrameInspectEntryInput {
                    widget_id: "nav.splitter",
                    role: "separator",
                    label: "Navigator / Family Split",
                    command_id: None,
                    value: Some(json!({ "fraction": state.nav_split_fraction })),
                    enabled: ctx_in.enabled,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );

            let fam_rect = Rect::from_min_size(Pos2::new(full_rect.left(), split_rect.bottom()), Vec2::new(full_rect.width(), family_h));
            report.family_rect_pt = Some(fam_rect);
            let mut fam_ui = ui.new_child(egui::UiBuilder::new().max_rect(fam_rect).layout(Layout::top_down(Align::Min)));
            fam_ui.set_clip_rect(fam_rect);
            render_family_pane(&mut fam_ui, app, &ctx_in, &mut signals, meta, intents);
        } else {
            let fam_bar_rect = Rect::from_min_size(Pos2::new(full_rect.left(), nav_rect.bottom()), Vec2::new(full_rect.width(), family_bar_h));
            report.family_rect_pt = Some(fam_bar_rect);
            let mut fam_bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(fam_bar_rect).layout(Layout::top_down(Align::Min)));
            fam_bar_ui.set_clip_rect(fam_bar_rect);
            render_family_collapsed_strip(&mut fam_bar_ui, app, tokens, ctx_in.enabled, meta, intents);
        }

        // Advance parent UI cursor cleanly over allocated dock body.
        let _ = ui.allocate_rect(full_rect, Sense::hover());

        finalize_drag_state(ui, app, state, &signals, intents);
        hovered_family_this_frame = signals.any_row_hovered_family;
    });

    report.dock_rect_pt = Some(panel_resp.response.rect);
    if ctx_in.layout.leading_collapsed_to_rail {
        report.navigator_rect_pt = Some(panel_resp.response.rect);
    } else {
        update_family_hover_preview_timer(
            ui,
            app,
            state,
            FamilyHoverPreviewInput {
                hovered_family: hovered_family_this_frame,
                tokens,
                dock_rect: panel_resp.response.rect,
                rtl_mirrored: ctx_in.layout.rtl_mirrored,
            },
            meta,
            &mut report,
        );
        if ctx_in.layout.leading_visible && ctx_in.layout.leading_splitter_w_pt > 0.5 {
            render_leading_dock_splitter(ui, app, ctx_in, meta, intents);
        }
    }
    if !ctx_in.enabled {
        state.context_menu = None;
    } else {
        render_navigator_context_menu(ui, app, state, tokens, meta, intents);
        let _ = render_navigator_delete_confirm_modal(ui.ctx(), &app.locale, state, tokens, meta, intents);
    }
    report
}

fn handle_leading_dock_shortcuts(
    ui: &egui::Ui,
    app: &UiApp,
    state: &mut LeadingDockEguiState,
    ctx_in: &LeadingDockContext<'_>,
    intents: &mut PendingUiIntents,
) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }

    let on_result_sheet = crate::interaction::resolve_result_sheet_for_shortcut(app, true).is_some();
    let can_rename = !ctx_in.layout.leading_collapsed_to_rail && (ctx_in.focused || state.nav_pointer_hovered) && state.renaming_sheet_id.is_none();

    while let Some(m) = consume_matching_shortcut(ui.ctx(), app, on_result_sheet, |cmd| match cmd {
        "ui.navigator.toggle" | "ui.family.toggle" => true,
        "sheet.rename" => can_rename && app.document().active_sheet_id.is_some(),
        _ => false,
    }) {
        match m.command_id.as_str() {
            "ui.navigator.toggle" => intents.push(UiIntent::ToggleLeadingCollapsedRail),
            "ui.family.toggle" => intents.push_ui("ui.family.toggle", json!({})),
            "sheet.rename" => {
                if let Some(active_id) = app.document().active_sheet_id.clone() {
                    begin_inline_rename(app, state, active_id);
                }
            }
            _ => {}
        }
    }

    let f2_fallback_allowed = can_rename
        && app.shortcuts.active_combo("sheet.rename").is_none_or(str::is_empty)
        && !has_registered_shortcut(app, on_result_sheet, Key::F2, Modifiers::NONE);
    if f2_fallback_allowed
        && ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2))
        && let Some(active_id) = app.document().active_sheet_id.clone()
    {
        begin_inline_rename(app, state, active_id);
    }
}

fn begin_inline_rename(app: &UiApp, state: &mut LeadingDockEguiState, sheet_id: SheetId) {
    let title = doc_sheet_title(app.document(), &sheet_id).unwrap_or(sheet_id.as_str()).to_owned();
    state.renaming_sheet_id = Some(sheet_id);
    state.rename_draft = title.clone();
    state.rename_initial_title = title;
    state.rename_just_started = true;
}

#[allow(clippy::too_many_arguments)]
fn render_collapsed_icon_rail(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut LeadingDockEguiState,
    tokens: &Tokens,
    rtl_mirrored: bool,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let expand_resp = render_rail_icon_button(
            ui,
            RailIconButtonSpec {
                widget_id: "nav.rail.expand",
                glyph: if rtl_mirrored { "«" } else { "»" },
                tooltip: tr(&app.locale, "menu.view.navigator"),
                command_id: Some("ui.navigator.toggle"),
                selected: false,
                enabled,
            },
            tokens,
            meta,
        );
        if expand_resp.clicked() && enabled {
            intents.push(UiIntent::ToggleLeadingCollapsedRail);
        }

        for (chip, badge, wid) in [
            (NavigatorKindChip::Data, "D", "nav.rail.data"),
            (NavigatorKindChip::Info, "I", "nav.rail.info"),
            (NavigatorKindChip::Results, "R", "nav.rail.results"),
            (NavigatorKindChip::Graphs, "G", "nav.rail.graphs"),
            (NavigatorKindChip::Layouts, "L", "nav.rail.layouts"),
        ] {
            let selected = state.kind_chip == chip;
            let resp = render_rail_icon_button(
                ui,
                RailIconButtonSpec { widget_id: wid, glyph: badge, tooltip: chip.label(&app.locale), command_id: None, selected, enabled },
                tokens,
                meta,
            );
            if resp.clicked() && enabled {
                state.kind_chip = chip;
                intents.push(UiIntent::ToggleLeadingCollapsedRail);
            }
        }

        let fam_resp = render_rail_icon_button(
            ui,
            RailIconButtonSpec {
                widget_id: "family.rail.toggle",
                glyph: "F",
                tooltip: tr(&app.locale, "menu.view.family_pane"),
                command_id: Some("ui.family.toggle"),
                selected: app.family_open,
                enabled,
            },
            tokens,
            meta,
        );
        if fam_resp.clicked() && enabled {
            intents.push_ui("ui.family.toggle", json!({}));
        }
    });
}

fn render_rail_icon_button(ui: &mut egui::Ui, spec: RailIconButtonSpec<'_>, tokens: &Tokens, meta: &mut FrameInspectMeta) -> Response {
    let RailIconButtonSpec { widget_id, glyph, tooltip, command_id, selected, enabled } = spec;
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::splat(26.0), if enabled { Sense::click() } else { Sense::hover() });
    super::widgets::paint_interactive_chrome(
        ui,
        rect,
        tokens,
        super::widgets::InteractiveChromeSpec {
            radius: CornerRadius::same(tokens.metrics.radius_md),
            base_bg: tokens.surface_bg,
            base_border: Some(tokens.border),
            hovered: resp.hovered(),
            selected,
            pressed: resp.is_pointer_button_down_on(),
            enabled,
        },
    );
    let fg = if !enabled {
        tokens.text_disabled
    } else if selected {
        tokens.selected_accent_text()
    } else {
        tokens.text_strong
    };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, FontId::proportional(11.5), fg);
    resp = resp.on_hover_text(tooltip);
    let a11y_label = tooltip.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, a11y_label.clone()));
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id,
            role: "button",
            label: tooltip,
            command_id,
            value: Some(json!({ "selected": selected })),
            enabled,
            disabled_reason: (!enabled).then_some("unavailable"),
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );
    resp
}

fn render_navigator_pane(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut LeadingDockEguiState,
    ctx_in: LeadingDockContext<'_>,
    signals: &mut NavFrameSignals,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let enabled = ctx_in.enabled;
    let locale = &app.locale;
    let avail_w = ui.available_width().max(120.0);

    // Shared 32 pt Panel Header (aligned flush with Center Split panes and Trailing Studio).
    let header_h = tokens.metrics.panel_header_h;
    let (header_rect, hdr_region) = ui.allocate_exact_size(Vec2::new(avail_w, header_h), Sense::hover());
    hdr_region.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Region, enabled, "Project Navigator Header"));
    ui.painter().rect_filled(header_rect, CornerRadius::ZERO, tokens.panel_bg);
    ui.painter().hline(header_rect.x_range(), header_rect.bottom(), Stroke::new(1.0, tokens.divider));
    meta.record_response(
        ui,
        &hdr_region,
        FrameInspectEntryInput {
            widget_id: "nav.panel_header",
            role: "region",
            label: "Project Navigator Header",
            command_id: None,
            value: None,
            enabled,
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );

    let mut hdr_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(header_rect.shrink2(Vec2::new(tokens.metrics.space_md, tokens.metrics.space_xs)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    hdr_ui.set_clip_rect(header_rect);
    hdr_ui.spacing_mut().item_spacing.x = tokens.metrics.space_xs;
    let hdr_inner_w = hdr_ui.available_width().max(80.0);
    let title_w = (hdr_inner_w - 86.0).max(40.0);
    render_clipped_label(
        &mut hdr_ui,
        "nav.header.title",
        tr(locale, "menu.view.navigator"),
        title_w,
        FontId::proportional(tokens.metrics.font_ui_body_pt),
        tokens.text_strong,
        meta,
    );

    let is_nested = matches!(app.nav.nesting_mode, NavigatorNestingMode::NestedResultsUnderData);
    let view_lbl = if is_nested { "Nested" } else { "Folders" };
    let view_resp = render_compact_chip(
        &mut hdr_ui,
        CompactChipSpec { max_w: 54.0, widget_id: "nav.view_mode.toggle", label: view_lbl, selected: is_nested, enabled },
        tokens,
        meta,
    );
    if view_resp.clicked() && enabled {
        let next = if is_nested { NavigatorNestingMode::SeparateFolders } else { NavigatorNestingMode::NestedResultsUnderData };
        intents.push(UiIntent::SetNavigatorNesting { mode: next });
    }

    let collapse_glyph = if ctx_in.layout.rtl_mirrored { "»" } else { "«" };
    let col_resp = render_compact_chip(
        &mut hdr_ui,
        CompactChipSpec { max_w: 24.0, widget_id: "nav.collapse_rail", label: collapse_glyph, selected: false, enabled },
        tokens,
        meta,
    );
    if col_resp.clicked() && enabled {
        intents.push(UiIntent::ToggleLeadingCollapsedRail);
    }
    ui.add_space(tokens.metrics.space_xs);

    // Search filter box + clear X
    if state.last_synced_query != app.nav.search_query {
        state.search_draft = app.nav.search_query.clone();
        state.last_synced_query = app.nav.search_query.clone();
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let clear_w = if state.search_draft.is_empty() { 0.0 } else { 24.0 };
        let search_w = (avail_w - clear_w - 4.0).max(60.0);
        let search_resp = ui.add_enabled(
            enabled,
            egui::TextEdit::singleline(&mut state.search_draft).hint_text(tr(locale, "nav.search_label")).desired_width(search_w),
        );
        if search_resp.changed() && enabled {
            state.last_synced_query = state.search_draft.clone();
            intents.push(UiIntent::SetNavigatorSearch { query: state.search_draft.clone() });
        }
        meta.record_response(
            ui,
            &search_resp,
            FrameInspectEntryInput {
                widget_id: "nav.search_input",
                role: "textbox",
                label: tr(locale, "nav.search_label"),
                command_id: None,
                value: Some(json!(state.search_draft)),
                enabled,
                disabled_reason: None,
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
        if !state.search_draft.is_empty() {
            let clr_resp = render_compact_chip(
                ui,
                CompactChipSpec { max_w: 20.0, widget_id: "nav.search_clear", label: "×", selected: false, enabled },
                tokens,
                meta,
            );
            if clr_resp.clicked() && enabled {
                state.search_draft.clear();
                state.last_synced_query.clear();
                intents.push(UiIntent::SetNavigatorSearch { query: String::new() });
            }
        }
    });
    ui.add_space(4.0);

    // Kind filter chips (2 crisp rows of 3 chips so 240 pt never overflows horizontally)
    let chip_w = ((avail_w - 8.0) / 3.0).max(40.0);
    for row_chips in NavigatorKindChip::ALL.chunks(3) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for &chip in row_chips {
                let selected = state.kind_chip == chip;
                let resp = render_compact_chip(
                    ui,
                    CompactChipSpec { max_w: chip_w, widget_id: chip.widget_id(), label: chip.label(locale), selected, enabled },
                    tokens,
                    meta,
                );
                if resp.clicked() && enabled {
                    state.kind_chip = chip;
                }
            }
        });
        ui.add_space(2.0);
    }
    ui.add_space(2.0);
    ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(), Stroke::new(1.0, tokens.divider));
    ui.add_space(3.0);

    let visible_ids = collect_visible_sheet_ids(app, state);
    handle_navigator_keyboard(ui, app, state, &ctx_in, &visible_ids, intents);

    egui::ScrollArea::vertical().id_salt("leading_nav_scroll").animated(false).auto_shrink([false, false]).show(ui, |ui| {
        let row_w = (ui.available_width() - 12.0).max(96.0);
        let q = state.search_draft.trim().to_ascii_lowercase();
        let nested_mode = matches!(app.nav.nesting_mode, NavigatorNestingMode::NestedResultsUnderData);
        let tree_ctx = NavigatorTreeCtx { app, dock: ctx_in, row_w, query: &q };

        if nested_mode {
            render_nested_navigator_tree(ui, tree_ctx, state, signals, meta, intents);
        } else {
            render_folder_navigator_tree(ui, tree_ctx, state, signals, meta, intents);
        }
    });
}

fn render_folder_navigator_tree(
    ui: &mut egui::Ui,
    tree: NavigatorTreeCtx<'_>,
    state: &mut LeadingDockEguiState,
    signals: &mut NavFrameSignals,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let NavigatorTreeCtx { app, dock: ctx_in, row_w, query } = tree;
    let tokens = ctx_in.tokens;
    let enabled = ctx_in.enabled;
    let locale = &app.locale;
    let mut rendered_any = false;

    for folder in &app.nav.folders {
        if !state.kind_chip.matches_section(folder.section) {
            continue;
        }
        let filtered: Vec<(usize, &NavigatorEntry)> = folder.entries.iter().enumerate().filter(|(_, e)| entry_matches_query(e, query)).collect();

        rendered_any = true;
        render_folder_header(
            ui,
            app,
            FolderHeaderSpec { section: folder.section, count: folder.entries.len(), collapsed: folder.collapsed, row_w, tokens, enabled },
            meta,
            intents,
        );
        if !folder.collapsed {
            for (section_idx, entry) in filtered {
                render_navigator_row(
                    ui,
                    NavigatorRowSpec {
                        app,
                        dock: ctx_in,
                        entry,
                        section_index: section_idx,
                        indent_depth: 0,
                        is_nested_parent: false,
                        nested_collapsed: false,
                        row_w,
                    },
                    state,
                    signals,
                    meta,
                    intents,
                );
            }
            render_section_new_row(ui, app, SectionNewRowSpec { section: folder.section, row_w, tokens, enabled }, meta, intents);
        }
        ui.add_space(2.0);
    }

    if !rendered_any
        || (!query.is_empty()
            && app
                .nav
                .folders
                .iter()
                .filter(|f| state.kind_chip.matches_section(f.section))
                .all(|f| !f.entries.iter().any(|e| entry_matches_query(e, query))))
    {
        ui.label(egui::RichText::new(tr(locale, "nav.no_matches")).size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted));
    }
}

fn render_nested_navigator_tree(
    ui: &mut egui::Ui,
    tree: NavigatorTreeCtx<'_>,
    state: &mut LeadingDockEguiState,
    signals: &mut NavFrameSignals,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let NavigatorTreeCtx { app, dock: ctx_in, row_w, query } = tree;
    let doc = app.document();
    let entry_map: BTreeMap<SheetId, (usize, &NavigatorEntry)> =
        app.nav.folders.iter().flat_map(|f| f.entries.iter().enumerate()).map(|(i, e)| (e.sheet_id.clone(), (i, e))).collect();

    let mut placed = BTreeSet::new();
    let mut any = false;

    if state.kind_chip.matches_section(SheetSection::Data) || state.kind_chip == NavigatorKindChip::All {
        for (data_idx, ds) in doc.data_sheets.iter().enumerate() {
            let Some((_, data_entry)) = entry_map.get(&ds.id) else {
                continue;
            };
            let children = collect_data_sheet_children(doc, &ds.id, &entry_map);
            let matching_children: Vec<(&NavigatorEntry, usize)> =
                children.into_iter().filter(|(e, _)| state.kind_chip.matches_section(e.section) && entry_matches_query(e, query)).collect();
            let parent_matches = state.kind_chip.matches_section(SheetSection::Data) && entry_matches_query(data_entry, query);
            if !parent_matches && matching_children.is_empty() {
                continue;
            }
            any = true;
            placed.insert(ds.id.clone());
            let has_children = !matching_children.is_empty();
            let collapsed = state.nested_collapsed_parents.contains(&ds.id);
            render_navigator_row(
                ui,
                NavigatorRowSpec {
                    app,
                    dock: ctx_in,
                    entry: data_entry,
                    section_index: data_idx,
                    indent_depth: 0,
                    is_nested_parent: has_children,
                    nested_collapsed: collapsed,
                    row_w,
                },
                state,
                signals,
                meta,
                intents,
            );
            if !collapsed {
                for (child, depth) in matching_children {
                    placed.insert(child.sheet_id.clone());
                    let s_idx = entry_map.get(&child.sheet_id).map_or(0, |(idx, _)| *idx);
                    render_navigator_row(
                        ui,
                        NavigatorRowSpec {
                            app,
                            dock: ctx_in,
                            entry: child,
                            section_index: s_idx,
                            indent_depth: depth,
                            is_nested_parent: false,
                            nested_collapsed: false,
                            row_w,
                        },
                        state,
                        signals,
                        meta,
                        intents,
                    );
                }
            }
        }
    }

    // Show any remaining sheets (unattached or filtered outside Data parents)
    for folder in &app.nav.folders {
        if !state.kind_chip.matches_section(folder.section) {
            continue;
        }
        let leftovers: Vec<(usize, &NavigatorEntry)> =
            folder.entries.iter().enumerate().filter(|(_, e)| !placed.contains(&e.sheet_id) && entry_matches_query(e, query)).collect();
        if !leftovers.is_empty() || folder.section == SheetSection::Data {
            for (idx, entry) in leftovers {
                any = true;
                render_navigator_row(
                    ui,
                    NavigatorRowSpec {
                        app,
                        dock: ctx_in,
                        entry,
                        section_index: idx,
                        indent_depth: 0,
                        is_nested_parent: false,
                        nested_collapsed: false,
                        row_w,
                    },
                    state,
                    signals,
                    meta,
                    intents,
                );
            }
        }
    }

    render_section_new_row(
        ui,
        app,
        SectionNewRowSpec { section: SheetSection::Data, row_w, tokens: ctx_in.tokens, enabled: ctx_in.enabled },
        meta,
        intents,
    );
    if !any && !query.is_empty() {
        ui.label(egui::RichText::new(tr(&app.locale, "nav.no_matches")).size(ctx_in.tokens.metrics.font_ui_small_pt).color(ctx_in.tokens.text_muted));
    }
}

fn collect_data_sheet_children<'a>(
    doc: &ProjectDocument,
    data_id: &SheetId,
    entry_map: &BTreeMap<SheetId, (usize, &'a NavigatorEntry)>,
) -> Vec<(&'a NavigatorEntry, usize)> {
    let mut out = Vec::new();
    let mut owned_results = BTreeSet::new();
    let mut owned_graphs = BTreeSet::new();

    for info in &doc.info_sheets {
        if info.linked_data_sheets.contains(data_id)
            && let Some((_, e)) = entry_map.get(&info.id)
        {
            out.push((*e, 1));
        }
    }
    for res in &doc.result_sheets {
        if &res.analysis.source_sheet_id == data_id || owned_results.contains(&res.analysis.source_sheet_id) {
            owned_results.insert(res.id.clone());
            if let Some((_, e)) = entry_map.get(&res.id) {
                out.push((*e, 1));
            }
        }
    }
    for graph in &doc.graph_sheets {
        let linked = graph.spec.series.iter().any(|s| &s.source_sheet_id == data_id || owned_results.contains(&s.source_sheet_id));
        if linked {
            owned_graphs.insert(graph.id.clone());
            if let Some((_, e)) = entry_map.get(&graph.id) {
                out.push((*e, 1));
            }
        }
    }
    for layout in &doc.layout_sheets {
        let linked = layout.spec.items.iter().any(|item| match item {
            LayoutItem::LiveGraph { graph_sheet_id, .. } => owned_graphs.contains(graph_sheet_id),
            LayoutItem::LiveTable { source_sheet_id, .. } => source_sheet_id == data_id || owned_results.contains(source_sheet_id),
            LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => false,
        });
        if linked && let Some((_, e)) = entry_map.get(&layout.id) {
            out.push((*e, 2));
        }
    }
    out
}

fn allocate_visible_slot(ui: &mut egui::Ui, size: Vec2, sense: Sense) -> Option<(Rect, Response)> {
    let predicted = Rect::from_min_size(ui.cursor().min, size);
    let visible = predicted.intersect(ui.clip_rect());
    if visible.height() < 4.0 || visible.width() < 4.0 {
        ui.allocate_space(size);
        return None;
    }
    Some(ui.allocate_exact_size(size, sense))
}

fn render_folder_header(ui: &mut egui::Ui, app: &UiApp, spec: FolderHeaderSpec<'_>, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let FolderHeaderSpec { section, count, collapsed, row_w, tokens, enabled } = spec;
    let Some((rect, mut resp)) = allocate_visible_slot(ui, Vec2::new(row_w, 24.0), if enabled { Sense::click() } else { Sense::hover() }) else {
        return;
    };
    let label = format!("{} ({count})", tr(&app.locale, section_label_key(section)));
    if enabled {
        super::widgets::paint_hover_row(ui, rect, CornerRadius::same(tokens.metrics.radius_md), resp.hovered(), false, tokens);
    }
    let arrow = if collapsed { "▸" } else { "▾" };
    let text = format!("{arrow} {label}");
    let (galley, elided) =
        super::ui_kit::elided_galley(ui.painter(), &text, FontId::proportional(11.5), tokens.text_muted, (row_w - 8.0).max(16.0), false);
    ui.painter().with_clip_rect(rect.shrink(2.0)).galley(
        Pos2::new(rect.left() + 4.0, rect.center().y - galley.size().y * 0.5),
        galley,
        tokens.text_muted,
    );

    if elided {
        resp = resp.on_hover_text(&label);
    }
    if resp.clicked() && enabled {
        intents.push(UiIntent::ToggleNavigatorSection { section });
    }
    let wid = section_header_widget_id(section);
    let a11y_lbl = label.clone();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::TreeItem, enabled, !collapsed, a11y_lbl.clone()));
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: wid,
            role: "treeitem",
            label: &label,
            command_id: None,
            value: Some(json!({ "collapsed": collapsed, "count": count })),
            enabled,
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            ellipsized: elided,
            full_label: elided.then_some(&label),
        },
    );
}

fn render_navigator_row(
    ui: &mut egui::Ui,
    row: NavigatorRowSpec<'_>,
    state: &mut LeadingDockEguiState,
    signals: &mut NavFrameSignals,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let NavigatorRowSpec { app, dock: ctx_in, entry, section_index, indent_depth, is_nested_parent, nested_collapsed, row_w } = row;
    let tokens = ctx_in.tokens;
    let enabled = ctx_in.enabled;
    let doc = app.document();
    let title = if entry.title.trim().is_empty() { tr(&app.locale, "nav.untitled_sheet") } else { entry.title.as_str() };
    let row_h = 26.0;
    let indent_px = (indent_depth.min(4) as f32) * 14.0;

    if state.scroll_to_sheet.as_ref() == Some(&entry.sheet_id) {
        let predicted = Rect::from_min_size(ui.cursor().min, Vec2::new(row_w, row_h));
        ui.scroll_to_rect(predicted, Some(egui::Align::Center));
        if !ui.ctx().will_discard() {
            state.scroll_to_sheet = None;
            ui.ctx().request_repaint();
        }
    }

    let Some((rect, mut resp)) = allocate_visible_slot(ui, Vec2::new(row_w, row_h), if enabled { Sense::click_and_drag() } else { Sense::hover() })
    else {
        return;
    };
    let is_renaming = state.renaming_sheet_id.as_ref() == Some(&entry.sheet_id);

    if is_renaming {
        render_inline_rename_row(ui, InlineRenameRowSpec { app, entry, rect, enabled }, state, meta, intents);
        return;
    }

    let drop_outcome = evaluate_drag_drop_outcome(doc, state.dragging_sheet_id.as_ref(), entry, section_index);
    let pointer_pos = ui.ctx().input(|i| i.pointer.latest_pos().or_else(|| i.pointer.hover_pos()));
    let pointer_inside = pointer_pos.is_some_and(|p| rect.contains(p));

    if enabled {
        super::widgets::paint_hover_row(ui, rect, CornerRadius::same(tokens.metrics.radius_md), resp.hovered(), entry.is_active, tokens);
    } else if entry.is_active {
        ui.painter().rect_filled(rect, CornerRadius::same(tokens.metrics.radius_md), tokens.active_bg);
    }
    if entry.is_active {
        let bar = Rect::from_min_size(Pos2::new(rect.left() + 1.0, rect.top() + 3.0), Vec2::new(3.0, rect.height() - 6.0));
        ui.painter().rect_filled(bar, CornerRadius::same(1), tokens.accent);
    }

    if pointer_inside && state.dragging_sheet_id.is_some() {
        match drop_outcome {
            DragDropOutcome::ReorderCandidate { target_index, .. } => {
                ui.painter().hline(rect.x_range(), rect.top() + 1.0, Stroke::new(2.0, tokens.accent));
                if ui.ctx().input(|i| i.pointer.any_released())
                    && let Some(drag_id) = state.dragging_sheet_id.clone()
                {
                    signals.drop_commit = Some(DropCommitAction::Reorder { sheet_id: drag_id, target_index });
                }
            }
            DragDropOutcome::RetargetCompatible => {
                ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.5, tokens.link_green), StrokeKind::Inside);
                if ui.ctx().input(|i| i.pointer.any_released())
                    && let Some(drag_id) = state.dragging_sheet_id.clone()
                {
                    signals.drop_commit = Some(DropCommitAction::Retarget { result_id: drag_id, target_data_id: entry.sheet_id.clone() });
                }
            }
            DragDropOutcome::RetargetIncompatible => {
                ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.5, tokens.link_red), StrokeKind::Inside);
            }
            DragDropOutcome::None => {}
        }
    }

    // Disclosure button if nested parent
    let mut x_cur = rect.left() + 5.0 + indent_px;
    if is_nested_parent {
        let tri_rect = Rect::from_min_size(Pos2::new(x_cur, rect.center().y - 8.0), Vec2::splat(16.0));
        let tri_resp =
            ui.interact(tri_rect, ui.scope_id().with(("nested_tri", &entry.sheet_id)), if enabled { Sense::click() } else { Sense::hover() });
        let tri = if nested_collapsed { "▸" } else { "▾" };
        ui.painter().text(
            tri_rect.center(),
            egui::Align2::CENTER_CENTER,
            tri,
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            tokens.text_muted,
        );
        if tri_resp.clicked() && enabled {
            if nested_collapsed {
                state.nested_collapsed_parents.remove(&entry.sheet_id);
            } else {
                state.nested_collapsed_parents.insert(entry.sheet_id.clone());
            }
        }
        x_cur += 16.0;
    }

    // Original vector archetype icon
    let icon_rect = Rect::from_min_size(Pos2::new(x_cur, rect.center().y - 6.0), Vec2::new(14.0, 12.0));
    draw_sheet_vector_icon(
        ui.painter(),
        icon_rect,
        entry.section,
        entry.archetype_label.as_deref(),
        if entry.is_active { tokens.accent } else { tokens.icon },
    );
    x_cur += 18.0;

    // Reserve trailing badges on right
    let badge_w = compute_badge_strip_width(entry);
    let title_max_w = (rect.right() - x_cur - badge_w - 6.0).max(20.0);
    let fg = if !enabled {
        tokens.text_disabled
    } else if entry.is_active || entry.is_bold {
        tokens.text_strong
    } else {
        tokens.text_primary
    };
    let font_size = if entry.is_active || entry.is_bold { 12.0 } else { 11.5 };
    let (galley, ellipsized) = super::ui_kit::elided_galley(ui.painter(), title, FontId::proportional(font_size), fg, title_max_w, false);
    let title_clip = Rect::from_min_max(Pos2::new(x_cur, rect.top() + 2.0), Pos2::new(x_cur + title_max_w, rect.bottom() - 2.0));
    ui.painter().with_clip_rect(title_clip).galley(Pos2::new(x_cur, rect.center().y - galley.size().y * 0.5), galley, fg);

    paint_row_badges(ui.painter(), rect, entry, tokens, &app.locale);

    if ellipsized {
        resp = resp.on_hover_text(title);
    }

    if resp.drag_started() && enabled {
        state.dragging_sheet_id = Some(entry.sheet_id.clone());
    }
    if resp.secondary_clicked() && enabled {
        let anchor = ui.ctx().input(|i| i.pointer.interact_pos()).unwrap_or(rect.center());
        state.context_menu =
            Some(NavigatorContextMenuState { sheet_id: entry.sheet_id.clone(), section: entry.section, is_frozen: entry.is_frozen, anchor });
    }
    if resp.double_clicked() && enabled {
        begin_inline_rename(app, state, entry.sheet_id.clone());
    } else if resp.clicked() && enabled && state.dragging_sheet_id.is_none() {
        intents.push_engine("sheet.select", json!({ "sheet_id": entry.sheet_id.as_str(), "mode": "navigator" }));
        intents.push(UiIntent::SetFocusPane { pane: ActiveWorkspacePane::Navigator });
    }

    let wid = format!("nav.sheet.{}", entry.sheet_id.as_str());
    let a11y_lbl = title.to_owned();
    let is_selected = entry.is_active;
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::TreeItem, enabled, is_selected, a11y_lbl.clone()));
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: &wid,
            role: "treeitem",
            label: title,
            command_id: Some("sheet.select"),
            value: Some(json!({ "sheet_id": entry.sheet_id.as_str(), "selected": is_selected, "bold": entry.is_bold })),
            enabled,
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            ellipsized,
            full_label: ellipsized.then_some(title),
        },
    );
}

fn render_inline_rename_row(
    ui: &mut egui::Ui,
    spec: InlineRenameRowSpec<'_>,
    state: &mut LeadingDockEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let InlineRenameRowSpec { app, entry, rect, enabled } = spec;
    let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(4.0, 2.0))).layout(Layout::left_to_right(Align::Center)));
    let id = Id::unique(("nav_inline_rename", &entry.sheet_id));
    let edit_resp =
        child_ui.add_enabled(enabled, egui::TextEdit::singleline(&mut state.rename_draft).id(id).desired_width((rect.width() - 12.0).max(48.0)));
    if state.rename_just_started {
        edit_resp.request_focus();
        state.rename_just_started = false;
    }

    let enter = edit_resp.lost_focus() && ui.ctx().input(|i| i.key_pressed(Key::Enter));
    let esc = ui.ctx().input(|i| i.key_pressed(Key::Escape));
    let focus_away = edit_resp.lost_focus() && !enter && !esc;

    if esc {
        state.renaming_sheet_id = None;
        state.rename_draft.clear();
        state.rename_initial_title.clear();
    } else if enter || focus_away {
        let trimmed = state.rename_draft.trim().to_owned();
        if !trimmed.is_empty() && trimmed != state.rename_initial_title {
            intents.push_engine("sheet.rename", json!({ "sheet_id": entry.sheet_id.as_str(), "title": trimmed }));
        }
        state.renaming_sheet_id = None;
        state.rename_draft.clear();
        state.rename_initial_title.clear();
    }

    meta.record_response(
        &child_ui,
        &edit_resp,
        FrameInspectEntryInput {
            widget_id: "nav.rename_input",
            role: "textbox",
            label: tr(&app.locale, "cmd.sheet.rename"),
            command_id: Some("sheet.rename"),
            value: Some(json!({ "sheet_id": entry.sheet_id.as_str(), "draft": state.rename_draft })),
            enabled,
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );
}

fn evaluate_drag_drop_outcome(
    doc: &ProjectDocument,
    drag_id: Option<&SheetId>,
    target_entry: &NavigatorEntry,
    target_index: usize,
) -> DragDropOutcome {
    let Some(drag_id) = drag_id else {
        return DragDropOutcome::None;
    };
    if drag_id == &target_entry.sheet_id {
        return DragDropOutcome::None;
    }
    let Some(drag_section) = doc.sheet_section(drag_id) else {
        return DragDropOutcome::None;
    };
    if drag_section == SheetSection::Analyses && target_entry.section == SheetSection::Data {
        if let (Some(res), Some(ds)) = (doc.result_sheet(drag_id), doc.data_sheet(&target_entry.sheet_id)) {
            if res.analysis.kind.is_compatible_with(ds.archetype.kind()) {
                return DragDropOutcome::RetargetCompatible;
            }
            return DragDropOutcome::RetargetIncompatible;
        }
        return DragDropOutcome::RetargetIncompatible;
    }
    if drag_section == target_entry.section {
        return DragDropOutcome::ReorderCandidate { section: target_entry.section, target_index };
    }
    DragDropOutcome::None
}

fn render_section_new_row(ui: &mut egui::Ui, app: &UiApp, spec: SectionNewRowSpec<'_>, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let SectionNewRowSpec { section, row_w, tokens, enabled } = spec;
    let doc = app.document();
    let locale = &app.locale;
    let (wid, label, cmd, row_enabled, disabled_reason) = match section {
        SheetSection::Data => ("nav.new.data", tr(locale, "menu.insert.new_data_table"), Some("sheet.createData"), enabled, None),
        SheetSection::Info => {
            let ok = !doc.info_sheets.is_empty();
            (
                "nav.new.info",
                tr(locale, "menu.insert.new_info_sheet"),
                Some("sheet.duplicate"),
                enabled && ok,
                (!ok).then(|| tr(locale, "nav.disabled_need_info")),
            )
        }
        SheetSection::Analyses => {
            let ok = !doc.data_sheets.is_empty();
            (
                "nav.new.results",
                tr(locale, "nav.new_analysis"),
                Some("ui.analyze.open"),
                enabled && ok,
                (!ok).then(|| tr(locale, "nav.disabled_need_data")),
            )
        }
        SheetSection::Graphs => {
            let ok = !doc.data_sheets.is_empty();
            (
                "nav.new.graphs",
                tr(locale, "menu.insert.new_graph"),
                Some("graph.create"),
                enabled && ok,
                (!ok).then(|| tr(locale, "nav.disabled_need_data")),
            )
        }
        SheetSection::Layouts => {
            let ok = !doc.graph_sheets.is_empty() || !doc.result_sheets.is_empty() || !doc.data_sheets.is_empty();
            (
                "nav.new.layouts",
                tr(locale, "menu.insert.new_layout"),
                Some("layout.create"),
                enabled && ok,
                (!ok).then(|| tr(locale, "nav.disabled_need_data_or_graph")),
            )
        }
    };

    let text = if label.starts_with('+') { label.to_owned() } else { format!("+ {label}") };
    let fg = if row_enabled { tokens.accent } else { tokens.text_disabled };
    let Some((rect, mut resp)) = allocate_visible_slot(ui, Vec2::new(row_w, 26.0), if row_enabled { Sense::click() } else { Sense::hover() }) else {
        return;
    };
    if row_enabled {
        super::widgets::paint_hover_row(ui, rect, CornerRadius::same(tokens.metrics.radius_md), resp.hovered(), false, tokens);
    }
    let (galley, elided) =
        super::ui_kit::elided_galley(ui.painter(), &text, FontId::proportional(tokens.metrics.font_ui_small_pt), fg, (row_w - 12.0).max(20.0), false);
    ui.painter().with_clip_rect(rect.shrink(2.0)).galley(Pos2::new(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5), galley, fg);

    if let Some(reason) = disabled_reason
        && !row_enabled
    {
        resp = resp.on_hover_text(reason);
    } else if elided {
        resp = resp.on_hover_text(&text);
    }

    if resp.clicked() && row_enabled {
        match section {
            SheetSection::Data => intents.push_engine("sheet.createData", json!({})),
            SheetSection::Info => {
                if let Some(first_info) = doc.info_sheets.first() {
                    intents.push_engine("sheet.duplicate", json!({ "sheet_id": first_info.id.as_str() }));
                }
            }
            SheetSection::Analyses => intents.push_ui("ui.analyze.open", json!({})),
            SheetSection::Graphs => {
                let active_ds = doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id));
                if let Some(src) = active_ds.or_else(|| doc.data_sheets.first()) {
                    intents.push_engine("graph.create", json!({ "data_sheet_id": src.id.as_str() }));
                }
            }
            SheetSection::Layouts => intents.push_engine("layout.create", json!({})),
        }
    }

    let a11y_lbl = text.clone();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, row_enabled, a11y_lbl.clone()));
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: wid,
            role: "button",
            label: &text,
            command_id: cmd,
            value: None,
            enabled: row_enabled,
            disabled_reason,
            z_layer: ZLayer::Workspace,
            ellipsized: elided,
            full_label: elided.then_some(&text),
        },
    );
}

fn render_family_collapsed_strip(
    ui: &mut egui::Ui,
    app: &UiApp,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(), Stroke::new(1.0, tokens.divider));
    let w = ui.available_width().max(80.0);
    let label = format!("▸ {}", tr(&app.locale, "nav.family"));
    let resp = render_compact_chip(
        ui,
        CompactChipSpec { max_w: w, widget_id: "family.header.toggle", label: &label, selected: false, enabled },
        tokens,
        meta,
    );
    if resp.clicked() && enabled {
        intents.push_ui("ui.family.toggle", json!({}));
    }
}

fn render_family_pane(
    ui: &mut egui::Ui,
    app: &UiApp,
    ctx_in: &LeadingDockContext<'_>,
    signals: &mut NavFrameSignals,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let enabled = ctx_in.enabled;
    let locale = &app.locale;
    let avail_w = ui.available_width().max(100.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        render_clipped_label(
            ui,
            "family.header.title",
            tr(locale, "nav.family"),
            (avail_w - 30.0).max(40.0),
            FontId::proportional(12.0),
            tokens.text_strong,
            meta,
        );
        let col_resp =
            render_compact_chip(ui, CompactChipSpec { max_w: 24.0, widget_id: "family.toggle", label: "▾", selected: false, enabled }, tokens, meta);
        if col_resp.clicked() && enabled {
            intents.push_ui("ui.family.toggle", json!({}));
        }
    });
    ui.add_space(2.0);

    if app.nav.family_nodes.is_empty() {
        ui.label(egui::RichText::new(tr(locale, "nav.family_empty")).size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted));
        return;
    }

    let ordered = order_family_nodes(&app.nav.family_nodes);
    let active_chain = compute_active_chain(app.document());
    let doc = app.document();

    egui::ScrollArea::vertical().id_salt("leading_family_scroll").auto_shrink([false, false]).show(ui, |ui| {
        let row_w = (ui.available_width() - 12.0).max(80.0);
        for (idx, (depth, node)) in ordered.iter().enumerate() {
            let Some((rect, mut resp)) = allocate_visible_slot(ui, Vec2::new(row_w, 24.0), if enabled { Sense::click() } else { Sense::hover() })
            else {
                continue;
            };
            let in_chain = node.is_active || node.is_anchor || active_chain.contains(&node.sheet_id);

            if enabled {
                super::widgets::paint_hover_row(ui, rect, CornerRadius::same(tokens.metrics.radius_md), resp.hovered(), node.is_active, tokens);
            } else if node.is_active {
                ui.painter().rect_filled(rect, CornerRadius::same(tokens.metrics.radius_md), tokens.active_bg);
            }

            let indent = ((*depth).min(4) as f32) * 12.0;
            let spine_x = rect.left() + 6.0 + indent * 0.5;
            if *depth > 0 {
                ui.painter().vline(spine_x, rect.top()..=rect.center().y, Stroke::new(1.0, if in_chain { tokens.accent } else { tokens.divider }));
                ui.painter().hline(
                    spine_x..=(rect.left() + 10.0 + indent),
                    rect.center().y,
                    Stroke::new(1.0, if in_chain { tokens.accent } else { tokens.divider }),
                );
            }

            let icon_x = rect.left() + 12.0 + indent;
            let icon_rect = Rect::from_min_size(Pos2::new(icon_x, rect.center().y - 5.0), Vec2::new(12.0, 10.0));
            draw_sheet_vector_icon(ui.painter(), icon_rect, node.section, None, if in_chain { tokens.accent } else { tokens.icon });

            let multi_dep = multi_dependency_badge(doc, &node.sheet_id);
            let badge_reserve = if multi_dep.is_some() { 28.0 } else { 4.0 };
            let text_x = icon_rect.right() + 5.0;
            let max_text_w = (rect.right() - text_x - badge_reserve).max(16.0);
            let fg = if in_chain { tokens.text_strong } else { tokens.text_primary };
            let (galley, elided) = super::ui_kit::elided_galley(
                ui.painter(),
                &node.title,
                FontId::proportional(if in_chain { 11.5 } else { 11.0 }),
                fg,
                max_text_w,
                false,
            );
            ui.painter()
                .with_clip_rect(Rect::from_min_max(Pos2::new(text_x, rect.top()), Pos2::new(rect.right() - badge_reserve, rect.bottom())))
                .galley(Pos2::new(text_x, rect.center().y - galley.size().y * 0.5), galley, fg);

            if let Some(dep_lbl) = multi_dep {
                ui.painter().text(
                    Pos2::new(rect.right() - 4.0, rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    dep_lbl,
                    FontId::proportional(tokens.metrics.font_ui_small_pt),
                    tokens.link_blue,
                );
            }

            if resp.hovered() {
                signals.any_row_hovered_family = Some(node.sheet_id.clone());
            }
            if elided {
                resp = resp.on_hover_text(&node.title);
            }
            if resp.clicked() && enabled {
                let mode = if node.section == SheetSection::Data { "navigator" } else { "family" };
                intents.push_engine("sheet.select", json!({ "sheet_id": node.sheet_id.as_str(), "mode": mode }));
            }

            let wid = format!("family.node.{}", node.sheet_id.as_str());
            let a11y_lbl = node.title.clone();
            let active = node.is_active;
            resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::TreeItem, enabled, active, a11y_lbl.clone()));
            meta.record_response(
                ui,
                &resp,
                FrameInspectEntryInput {
                    widget_id: &wid,
                    role: "treeitem",
                    label: &node.title,
                    command_id: Some("sheet.select"),
                    value: Some(json!({ "sheet_id": node.sheet_id.as_str(), "idx": idx, "in_chain": in_chain })),
                    enabled,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: elided,
                    full_label: elided.then_some(node.title.as_str()),
                },
            );
        }
    });
}

fn order_family_nodes(nodes: &[FamilyPaneNode]) -> Vec<(usize, &FamilyPaneNode)> {
    let mut upstream: Vec<&FamilyPaneNode> = nodes.iter().filter(|n| n.relation == FamilyRelation::Upstream).collect();
    let anchors: Vec<&FamilyPaneNode> = nodes.iter().filter(|n| n.relation == FamilyRelation::Anchor).collect();
    let mut downstream: Vec<&FamilyPaneNode> = nodes.iter().filter(|n| n.relation == FamilyRelation::Downstream).collect();
    upstream.sort_by(|a, b| b.hop_distance.cmp(&a.hop_distance).then(a.title.cmp(&b.title)));
    downstream
        .sort_by(|a, b| a.hop_distance.cmp(&b.hop_distance).then(section_rank(a.section).cmp(&section_rank(b.section))).then(a.title.cmp(&b.title)));

    let mut out = Vec::with_capacity(nodes.len());
    for n in upstream {
        out.push((0, n));
    }
    for n in anchors {
        out.push((0, n));
    }
    for n in downstream {
        out.push((n.hop_distance.clamp(1, 3), n));
    }
    out
}

const fn section_rank(s: SheetSection) -> u8 {
    match s {
        SheetSection::Info => 0,
        SheetSection::Data => 1,
        SheetSection::Analyses => 2,
        SheetSection::Graphs => 3,
        SheetSection::Layouts => 4,
    }
}

fn compute_active_chain(doc: &ProjectDocument) -> BTreeSet<SheetId> {
    let mut chain = BTreeSet::new();
    let Some(active) = doc.active_sheet_id.clone() else {
        return chain;
    };
    chain.insert(active.clone());
    if let Some(res) = doc.result_sheet(&active) {
        chain.insert(res.analysis.source_sheet_id.clone());
    }
    if let Some(graph) = doc.graph_sheet(&active) {
        for s in &graph.spec.series {
            chain.insert(s.source_sheet_id.clone());
        }
    }
    if let Some(layout) = doc.layout_sheet(&active) {
        for item in &layout.spec.items {
            match item {
                LayoutItem::LiveGraph { graph_sheet_id, .. } => {
                    chain.insert(graph_sheet_id.clone());
                }
                LayoutItem::LiveTable { source_sheet_id, .. } => {
                    chain.insert(source_sheet_id.clone());
                }
                LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => {}
            }
        }
    }
    chain
}

fn multi_dependency_badge(doc: &ProjectDocument, sheet_id: &SheetId) -> Option<&'static str> {
    if let Some(g) = doc.graph_sheet(sheet_id)
        && g.spec.series.len() > 1
    {
        return Some("2+src");
    }
    if let Some(l) = doc.layout_sheet(sheet_id)
        && l.spec.items.len() > 1
    {
        return Some("2+src");
    }
    None
}

fn update_family_hover_preview_timer(
    ui: &egui::Ui,
    app: &UiApp,
    state: &mut LeadingDockEguiState,
    input: FamilyHoverPreviewInput<'_>,
    meta: &mut FrameInspectMeta,
    report: &mut LeadingDockFrameReport,
) {
    let FamilyHoverPreviewInput { hovered_family, tokens, dock_rect, rtl_mirrored } = input;
    let now = ui.ctx().input(|i| i.time);
    match hovered_family {
        Some(id) => {
            if state.hover_preview_sheet_id.as_ref() == Some(&id) {
                let start = state.hover_started_sec.unwrap_or(now);
                let elapsed = (now - start).max(0.0);
                if elapsed >= 0.45 {
                    state.hover_preview_active = true;
                } else {
                    state.hover_preview_active = false;
                    ui.ctx().request_repaint_after(Duration::from_secs_f64((0.45 - elapsed).max(0.016)));
                }
            } else {
                state.hover_preview_sheet_id = Some(id);
                state.hover_started_sec = Some(now);
                state.hover_preview_active = false;
                ui.ctx().request_repaint_after(Duration::from_millis(450));
            }
        }
        None => {
            state.hover_preview_sheet_id = None;
            state.hover_started_sec = None;
            state.hover_preview_active = false;
        }
    }

    if state.hover_preview_active
        && let Some(ref preview_id) = state.hover_preview_sheet_id
    {
        let screen = ui.ctx().content_rect();
        let x = if rtl_mirrored {
            (dock_rect.left() - 212.0).clamp(screen.left() + 8.0, (screen.right() - 212.0).max(screen.left() + 8.0))
        } else {
            (dock_rect.right() + 8.0).min(screen.right() - 228.0).max(screen.left() + 8.0)
        };
        let y = (dock_rect.bottom() - 156.0).max(screen.top() + 36.0);
        let card_pos = Pos2::new(x, y);
        let area_resp = egui::Area::new(Id::unique("family_hover_preview_card"))
            .order(egui::Order::Tooltip)
            .fixed_pos(card_pos)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(tokens.surface_bg)
                    .stroke(Stroke::new(1.0, tokens.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(204.0);
                        render_hover_preview_contents(ui, app.document(), preview_id, tokens);
                    });
            });
        report.hover_preview_rect_pt = Some(area_resp.response.rect);
        let title = doc_sheet_title(app.document(), preview_id).unwrap_or(preview_id.as_str()).to_owned();
        let hover_resp = area_resp.response;
        let a11y_title = title.clone();
        hover_resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Tooltip, true, a11y_title.clone()));
        meta.record_response(
            ui,
            &hover_resp,
            FrameInspectEntryInput {
                widget_id: "family.hover_preview",
                role: "tooltip",
                label: &title,
                command_id: None,
                value: Some(json!({ "sheet_id": preview_id.as_str() })),
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::Tooltip,
                ellipsized: false,
                full_label: None,
            },
        );
    }
}

fn render_hover_preview_contents(ui: &mut egui::Ui, doc: &ProjectDocument, sheet_id: &SheetId, tokens: &Tokens) {
    let title = doc_sheet_title(doc, sheet_id).unwrap_or(sheet_id.as_str());
    ui.label(egui::RichText::new(title).size(12.0).strong().color(tokens.text_strong));
    if let Some(ds) = doc.data_sheet(sheet_id) {
        ui.label(
            egui::RichText::new(format!("Data ({:?}) • {}×{}", ds.archetype.kind(), ds.archetype.row_count(), ds.archetype.column_count()))
                .size(tokens.metrics.font_ui_small_pt)
                .color(tokens.text_muted),
        );
        if ds.frozen {
            ui.label(egui::RichText::new("Frozen").size(tokens.metrics.font_ui_small_pt).color(tokens.link_blue));
        }
    } else if let Some(res) = doc.result_sheet(sheet_id) {
        let status = match &res.status {
            ResultStatus::UpToDate => "Up-to-date",
            ResultStatus::Dirty => "Dirty (recalc pending)",
            ResultStatus::Failed { .. } => "Failed",
        };
        ui.label(egui::RichText::new(format!("Result • {status}")).size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted));
        for scalar in res.scalars.iter().take(2) {
            ui.label(
                egui::RichText::new(format!("{}: {:.4}", scalar.key, scalar.value)).size(tokens.metrics.font_ui_small_pt).color(tokens.text_primary),
            );
        }
    } else if let Some(graph) = doc.graph_sheet(sheet_id) {
        ui.label(
            egui::RichText::new(format!("Graph ({:?}) • {} series", graph.spec.kind, graph.spec.series.len()))
                .size(tokens.metrics.font_ui_small_pt)
                .color(tokens.text_muted),
        );
        let (plot_r, _) = ui.allocate_exact_size(Vec2::new(184.0, 44.0), Sense::hover());
        ui.painter().rect_filled(plot_r, CornerRadius::same(tokens.metrics.radius_md), tokens.paper_white);
        ui.painter().rect_stroke(plot_r, CornerRadius::same(tokens.metrics.radius_md), Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
        let p0 = Pos2::new(plot_r.left() + 12.0, plot_r.bottom() - 10.0);
        let p1 = Pos2::new(plot_r.center().x, plot_r.center().y - 4.0);
        let p2 = Pos2::new(plot_r.right() - 14.0, plot_r.top() + 10.0);
        ui.painter().line_segment([p0, p1], Stroke::new(1.5, tokens.accent_strong));
        ui.painter().line_segment([p1, p2], Stroke::new(1.5, tokens.accent_strong));
    } else if let Some(info) = doc.info_sheet(sheet_id) {
        ui.label(
            egui::RichText::new(format!("Info • {} constants", info.constants.len())).size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted),
        );
    } else if let Some(layout) = doc.layout_sheet(sheet_id) {
        ui.label(
            egui::RichText::new(format!("Layout • {} items", layout.spec.items.len())).size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted),
        );
    }
}

fn finalize_drag_state(ui: &egui::Ui, app: &UiApp, state: &mut LeadingDockEguiState, signals: &NavFrameSignals, intents: &mut PendingUiIntents) {
    if let Some(ref drag_id) = state.dragging_sheet_id
        && let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos())
    {
        let title = doc_sheet_title(app.document(), drag_id).unwrap_or(drag_id.as_str());
        let drag_label = format!("{} {title}", tr(&app.locale, "nav.moving_prefix"));
        let badge_pos = pos + Vec2::new(12.0, 10.0);
        egui::Area::new(Id::unique("nav_drag_ghost")).order(egui::Order::Tooltip).fixed_pos(badge_pos).interactable(false).show(ui.ctx(), |ui| {
            ui.label(egui::RichText::new(drag_label).size(11.5));
        });
    }

    if let Some(ref commit) = signals.drop_commit {
        match commit {
            DropCommitAction::Reorder { sheet_id, target_index } => {
                intents.push_engine("sheet.reorder", json!({ "sheet_id": sheet_id.as_str(), "index": target_index }));
            }
            DropCommitAction::Retarget { result_id, target_data_id } => {
                intents.push(UiIntent::OpenRetargetDialog { result_sheet_id: result_id.clone(), target_data_sheet_id: Some(target_data_id.clone()) });
            }
        }
    }

    if ui.ctx().input(|i| i.pointer.any_released()) {
        state.dragging_sheet_id = None;
    }
}

fn collect_visible_sheet_ids(app: &UiApp, state: &LeadingDockEguiState) -> Vec<SheetId> {
    let q = state.search_draft.trim().to_ascii_lowercase();
    let mut ids = Vec::new();
    for folder in &app.nav.folders {
        if !state.kind_chip.matches_section(folder.section) || folder.collapsed {
            continue;
        }
        for entry in &folder.entries {
            if entry_matches_query(entry, &q) {
                ids.push(entry.sheet_id.clone());
            }
        }
    }
    ids
}

fn handle_navigator_keyboard(
    ui: &egui::Ui,
    app: &UiApp,
    state: &LeadingDockEguiState,
    ctx_in: &LeadingDockContext<'_>,
    visible_ids: &[SheetId],
    intents: &mut PendingUiIntents,
) {
    if !ctx_in.enabled || ui.ctx().egui_wants_keyboard_input() || !(ctx_in.focused || state.nav_pointer_hovered) || visible_ids.is_empty() {
        return;
    }
    let down = ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown));
    let up = ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp));
    if !down && !up {
        return;
    }
    let cur_idx = app.document().active_sheet_id.as_ref().and_then(|aid| visible_ids.iter().position(|id| id == aid));
    let next_idx = match (cur_idx, down) {
        (Some(i), true) => (i + 1) % visible_ids.len(),
        (Some(i), false) => {
            if i == 0 {
                visible_ids.len() - 1
            } else {
                i - 1
            }
        }
        (None, true) => 0,
        (None, false) => visible_ids.len() - 1,
    };
    if let Some(target_id) = visible_ids.get(next_idx) {
        intents.push_engine("sheet.select", json!({ "sheet_id": target_id.as_str(), "mode": "navigator" }));
    }
}

fn entry_matches_query(entry: &NavigatorEntry, query_lower: &str) -> bool {
    query_lower.is_empty()
        || entry.title.to_ascii_lowercase().contains(query_lower)
        || entry.sheet_id.as_str().to_ascii_lowercase().contains(query_lower)
}

fn compute_badge_strip_width(entry: &NavigatorEntry) -> f32 {
    let mut w = 0.0;
    if entry.color_tag.is_some() {
        w += 8.0;
    }
    if entry.is_frozen {
        w += 14.0;
    }
    if entry.is_dirty {
        w += 14.0;
    }
    if !entry.link_colors.is_empty() {
        w += 10.0;
    }
    w
}

fn paint_row_badges(painter: &egui::Painter, rect: Rect, entry: &NavigatorEntry, tokens: &Tokens, _locale: &str) {
    let mut x_right = rect.right() - 4.0;
    if let Some(tag) = entry.color_tag {
        let r = Rect::from_min_size(Pos2::new(x_right - 4.0, rect.center().y - 5.0), Vec2::new(tokens.metrics.space_xs, 10.0)); // design-token-exempt: compact colour-tag pip
        painter.rect_filled(r, CornerRadius::same(1), crate::egui_shell::rgba8_to_color32(tag));
        x_right -= 7.0;
    }
    if entry.is_dirty {
        painter.circle_filled(Pos2::new(x_right - 4.0, rect.center().y), 3.0, tokens.link_red);
        x_right -= 11.0;
    }
    if entry.is_frozen {
        painter.text(Pos2::new(x_right, rect.center().y), egui::Align2::RIGHT_CENTER, "F", FontId::proportional(10.0), tokens.link_blue);
        x_right -= 12.0;
    }
    if !entry.link_colors.is_empty() {
        painter.circle_stroke(Pos2::new(x_right - 4.0, rect.center().y), 3.0, Stroke::new(1.2, tokens.link_green));
    }
}

fn draw_sheet_vector_icon(painter: &egui::Painter, rect: Rect, section: SheetSection, arch_lbl: Option<&str>, color: Color32) {
    painter.rect_stroke(rect, CornerRadius::same(1), Stroke::new(1.0, color), StrokeKind::Inside);
    match section {
        SheetSection::Data => {
            let short = arch_lbl.unwrap_or("D");
            let glyph = if short.starts_with("X") {
                "XY"
            } else if short.starts_with("G") {
                "Gr"
            } else if short.starts_with("S") {
                "Sv"
            } else if short.starts_with("N") {
                "Ns"
            } else {
                "D"
            };
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, glyph, FontId::proportional(7.5), color);
        }
        SheetSection::Info => {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "i", FontId::proportional(9.0), color);
        }
        SheetSection::Analyses => {
            painter.line_segment(
                [Pos2::new(rect.left() + 2.0, rect.center().y + 1.0), Pos2::new(rect.center().x - 1.0, rect.bottom() - 2.0)],
                Stroke::new(1.2, color),
            );
            painter.line_segment(
                [Pos2::new(rect.center().x - 1.0, rect.bottom() - 2.0), Pos2::new(rect.right() - 2.0, rect.top() + 2.0)],
                Stroke::new(1.2, color),
            );
        }
        SheetSection::Graphs => {
            painter.line_segment(
                [Pos2::new(rect.left() + 2.0, rect.bottom() - 2.0), Pos2::new(rect.right() - 2.0, rect.top() + 2.0)],
                Stroke::new(1.2, color),
            );
        }
        SheetSection::Layouts => {
            painter.rect_filled(Rect::from_min_size(rect.min + Vec2::splat(2.0), Vec2::splat(4.0)), CornerRadius::ZERO, color);
        }
    }
}

fn render_compact_chip(ui: &mut egui::Ui, spec: CompactChipSpec<'_>, tokens: &Tokens, meta: &mut FrameInspectMeta) -> Response {
    super::ui_kit::styled_button(
        ui,
        tokens,
        meta,
        super::ui_kit::StyledButtonSpec {
            widget_id: spec.widget_id,
            role: "button",
            a11y_role: egui::Role::Button,
            a11y_selectable: true,
            label: spec.label,
            tooltip: None,
            command_id: None,
            width: super::ui_kit::ButtonWidth::Fixed(spec.max_w.max(20.0)),
            height: 20.0,
            font_size: tokens.metrics.font_ui_small_pt,
            radius: tokens.metrics.radius_md,
            text_align: super::ui_kit::ButtonTextAlign::Center,
            break_anywhere: false,
            primary: false,
            selected: spec.selected,
            enabled: spec.enabled,
            accent_text_when_selected: true,
            border_override: None,
            value: Some(json!({ "selected": spec.selected })),
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            skip_offscreen: false,
            clip_interact_to_viewport: false,
            tooltip_mode: super::ui_kit::TooltipMode::Standard,
        },
    )
    .0
    .unwrap_or_else(|| ui.allocate_exact_size(Vec2::ZERO, Sense::hover()).1)
}

fn render_clipped_label(ui: &mut egui::Ui, widget_id: &str, label: &str, max_w: f32, font: FontId, color: Color32, meta: &mut FrameInspectMeta) {
    let _ = super::ui_kit::clipped_label(
        ui,
        meta,
        super::ui_kit::ClippedLabelSpec {
            widget_id,
            role: "label",
            text: label,
            max_w: max_w.max(16.0),
            fixed_width: true,
            min_h: 20.0,
            font,
            color,
            z_layer: ZLayer::Workspace,
            tooltip: None,
            record_meta: true,
            record_value: false,
            skip_offscreen: false,
        },
    );
}

const fn section_label_key(section: SheetSection) -> &'static str {
    match section {
        SheetSection::Data => "nav.data_tables",
        SheetSection::Info => "nav.info",
        SheetSection::Analyses => "nav.results",
        SheetSection::Graphs => "nav.graphs",
        SheetSection::Layouts => "nav.layouts",
    }
}

const fn section_header_widget_id(section: SheetSection) -> &'static str {
    match section {
        SheetSection::Data => "nav.section.data",
        SheetSection::Info => "nav.section.info",
        SheetSection::Analyses => "nav.section.results",
        SheetSection::Graphs => "nav.section.graphs",
        SheetSection::Layouts => "nav.section.layouts",
    }
}

fn render_leading_dock_splitter(
    ui: &mut egui::Ui,
    app: &UiApp,
    ctx_in: LeadingDockContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let tokens = ctx_in.tokens;
    let metrics = tokens.metrics;
    let splitter_w = ctx_in.layout.leading_splitter_w_pt;
    let splitter_panel = if ctx_in.layout.rtl_mirrored {
        egui::Panel::right("egui_shell.leading_dock_splitter")
    } else {
        egui::Panel::left("egui_shell.leading_dock_splitter")
    };
    splitter_panel.exact_size(splitter_w).resizable(false).frame(egui::Frame::NONE.fill(tokens.panel_bg)).show(ui, |ui| {
        let split_rect = ui.available_rect_before_wrap();
        let _ = ui.allocate_rect(split_rect, Sense::hover());
        let slop = ((metrics.splitter_hit_slop_w - split_rect.width()) * 0.5).max(0.0);
        let hit_rect = split_rect.expand2(Vec2::new(slop, 0.0)).intersect(meta.viewport_rect);
        ui.set_clip_rect(hit_rect);
        let enabled = ctx_in.enabled;
        let sense = if enabled { Sense::click_and_drag() } else { Sense::hover() };
        let resp = ui.interact(hit_rect, ui.scope_id().with("nav_dock_splitter"), sense);
        resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Splitter, enabled, "Navigator Dock Splitter"));
        if enabled && (resp.hovered() || resp.dragged()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        let line_col = if enabled && (resp.hovered() || resp.dragged()) { tokens.accent } else { tokens.border };
        ui.painter().vline(split_rect.center().x, split_rect.y_range(), Stroke::new(1.0, line_col));

        if enabled {
            if resp.double_clicked() {
                let default_w = app.egui_views.workspace_preset.default_dock_config().leading_width_pt;
                intents.push(UiIntent::SyncLeadingWidth { width_pt: default_w });
            } else if resp.dragged() {
                let dx = if ctx_in.layout.rtl_mirrored { -resp.drag_delta().x } else { resp.drag_delta().x };
                if dx.abs() > 0.01 {
                    let vw = ui.ctx().content_rect().width().max(1.0);
                    let target_center_min = if !ctx_in.layout.leading_collapsed_to_rail
                        && !ctx_in.layout.trailing_collapsed_to_rail
                        && vw >= metrics.inspector_collapse_breakpoint_w
                    {
                        480.0
                    } else {
                        metrics.center_stage_min_w
                    };
                    let max_for_center = (vw
                        - ctx_in.layout.trailing_dock_w_pt
                        - ctx_in.layout.leading_splitter_w_pt
                        - ctx_in.layout.trailing_splitter_w_pt
                        - target_center_min)
                        .max(metrics.navigator_min_w);
                    let new_w = (ctx_in.layout.leading_dock_w_pt + dx).clamp(metrics.navigator_min_w, metrics.navigator_max_w.min(max_for_center));
                    intents.push(UiIntent::SyncLeadingWidth { width_pt: new_w });
                }
            }
        }

        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: "nav.dock_splitter",
                role: "splitter",
                label: "Navigator Dock Splitter",
                command_id: None,
                value: Some(json!({ "width_pt": ctx_in.layout.leading_dock_w_pt })),
                enabled,
                disabled_reason: (!enabled).then_some("modal dialog is active"),
                z_layer: ZLayer::SplitHandle,
                ellipsized: false,
                full_label: None,
            },
        );
    });
}

struct NavCtxItemSpec<'a> {
    widget_id: &'a str,
    label: &'a str,
    command_id: Option<&'a str>,
    enabled: bool,
    disabled_reason: Option<&'a str>,
    tokens: &'a Tokens,
}

fn render_nav_ctx_item(ui: &mut egui::Ui, meta: &mut FrameInspectMeta, spec: NavCtxItemSpec<'_>) -> bool {
    let sense = if spec.enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(208.0, 26.0), sense);
    let fg = if spec.enabled { spec.tokens.text_strong } else { spec.tokens.text_disabled };
    if spec.enabled {
        super::widgets::paint_hover_row(ui, rect, CornerRadius::same(spec.tokens.metrics.radius_md), resp.hovered(), false, spec.tokens);
    }
    ui.painter().with_clip_rect(rect.shrink2(Vec2::new(6.0, 0.0))).text(
        Pos2::new(rect.left() + 8.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        spec.label,
        FontId::proportional(12.0),
        fg,
    );
    let ak_lbl = spec.label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::MenuItem, spec.enabled, ak_lbl.clone()));
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: spec.widget_id,
            role: "menuitem",
            label: spec.label,
            command_id: spec.command_id,
            value: None,
            enabled: spec.enabled,
            disabled_reason: spec.disabled_reason,
            z_layer: ZLayer::ContextMenu,
            ellipsized: false,
            full_label: None,
        },
    );
    spec.enabled && resp.clicked()
}

fn render_nav_section_ctx_items(
    ui: &mut egui::Ui,
    locale: &str,
    menu: &NavigatorContextMenuState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> bool {
    let sid = menu.sheet_id.as_str();
    let frozen_reason = menu.is_frozen.then(|| tr(locale, "nav.frozen_badge"));
    match menu.section {
        SheetSection::Data => {
            if render_nav_ctx_item(
                ui,
                meta,
                NavCtxItemSpec {
                    widget_id: "nav.ctx.analyze",
                    label: tr(locale, "menu.analyze.analyze"),
                    command_id: Some("ui.analyze.open"),
                    enabled: true,
                    disabled_reason: None,
                    tokens,
                },
            ) {
                intents.push_ui("ui.analyze.open", json!({ "sheet_id": sid }));
                return true;
            }
            if render_nav_ctx_item(
                ui,
                meta,
                NavCtxItemSpec {
                    widget_id: "nav.ctx.new_graph",
                    label: tr(locale, "menu.insert.new_graph"),
                    command_id: Some("graph.create"),
                    enabled: true,
                    disabled_reason: None,
                    tokens,
                },
            ) {
                intents.push_engine("graph.create", json!({ "data_sheet_id": sid }));
                return true;
            }
            if render_nav_ctx_item(
                ui,
                meta,
                NavCtxItemSpec {
                    widget_id: "nav.ctx.convert_archetype",
                    label: tr(locale, "cmd.ui.table.convertArchetype.open"),
                    command_id: Some("ui.table.convertArchetype.open"),
                    enabled: !menu.is_frozen,
                    disabled_reason: frozen_reason,
                    tokens,
                },
            ) {
                intents.push_ui("ui.table.convertArchetype.open", json!({ "sheet_id": sid }));
                return true;
            }
        }
        SheetSection::Analyses => {
            if render_nav_ctx_item(
                ui,
                meta,
                NavCtxItemSpec {
                    widget_id: "nav.ctx.edit_params",
                    label: tr(locale, "menu.analyze.change_params"),
                    command_id: Some("ui.editParams.open"),
                    enabled: true,
                    disabled_reason: None,
                    tokens,
                },
            ) {
                intents.push_ui("ui.editParams.open", json!({ "sheet_id": sid }));
                return true;
            }
            if render_nav_ctx_item(
                ui,
                meta,
                NavCtxItemSpec {
                    widget_id: "nav.ctx.rerun",
                    label: tr(locale, "cmd.analysis.run"),
                    command_id: Some("analysis.run"),
                    enabled: true,
                    disabled_reason: None,
                    tokens,
                },
            ) {
                intents.push_engine("analysis.run", json!({ "sheet_id": sid }));
                return true;
            }
            if render_nav_ctx_item(
                ui,
                meta,
                NavCtxItemSpec {
                    widget_id: "nav.ctx.retarget",
                    label: tr(locale, "menu.analyze.change_data"),
                    command_id: Some("analysis.retargetData"),
                    enabled: true,
                    disabled_reason: None,
                    tokens,
                },
            ) {
                intents.push(UiIntent::OpenRetargetDialog { result_sheet_id: menu.sheet_id.clone(), target_data_sheet_id: None });
                return true;
            }
        }
        SheetSection::Graphs => {
            if render_nav_ctx_item(
                ui,
                meta,
                NavCtxItemSpec {
                    widget_id: "nav.ctx.change_graph_type",
                    label: tr(locale, "menu.graph.change_type"),
                    command_id: Some("ui.changeGraphType.open"),
                    enabled: true,
                    disabled_reason: None,
                    tokens,
                },
            ) {
                intents.push_ui("ui.changeGraphType.open", json!({ "sheet_id": sid }));
                return true;
            }
            if render_nav_ctx_item(
                ui,
                meta,
                NavCtxItemSpec {
                    widget_id: "nav.ctx.export_figure",
                    label: tr(locale, "cmd.ui.export.figure.open"),
                    command_id: Some("ui.export.figure.open"),
                    enabled: true,
                    disabled_reason: None,
                    tokens,
                },
            ) {
                intents.push_ui("ui.export.figure.open", json!({ "sheet_id": sid }));
                return true;
            }
            if render_nav_ctx_item(
                ui,
                meta,
                NavCtxItemSpec {
                    widget_id: "nav.ctx.print",
                    label: tr(locale, "cmd.ui.print.open"),
                    command_id: Some("ui.print.open"),
                    enabled: true,
                    disabled_reason: None,
                    tokens,
                },
            ) {
                intents.push_ui("ui.print.open", json!({ "sheet_id": sid }));
                return true;
            }
        }
        SheetSection::Layouts => {
            if render_nav_ctx_item(
                ui,
                meta,
                NavCtxItemSpec {
                    widget_id: "nav.ctx.export_figure",
                    label: tr(locale, "cmd.ui.export.figure.open"),
                    command_id: Some("ui.export.figure.open"),
                    enabled: true,
                    disabled_reason: None,
                    tokens,
                },
            ) {
                intents.push_ui("ui.export.figure.open", json!({ "sheet_id": sid }));
                return true;
            }
            if render_nav_ctx_item(
                ui,
                meta,
                NavCtxItemSpec {
                    widget_id: "nav.ctx.print",
                    label: tr(locale, "cmd.ui.print.open"),
                    command_id: Some("ui.print.open"),
                    enabled: true,
                    disabled_reason: None,
                    tokens,
                },
            ) {
                intents.push_ui("ui.print.open", json!({ "sheet_id": sid }));
                return true;
            }
        }
        SheetSection::Info => {}
    }
    false
}

fn render_navigator_context_menu(
    ui: &egui::Ui,
    app: &UiApp,
    state: &mut LeadingDockEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let Some(menu) = state.context_menu.clone() else {
        return;
    };
    let doc = app.document();
    let Some(title) = doc_sheet_title(doc, &menu.sheet_id).map(str::to_owned) else {
        state.context_menu = None;
        return;
    };

    let locale = &app.locale;
    let screen = ui.ctx().content_rect();
    let pos = Pos2::new(
        menu.anchor.x.clamp(screen.left() + 4.0, (screen.right() - 246.0).max(screen.left() + 4.0)),
        menu.anchor.y.clamp(screen.top() + 4.0, (screen.bottom() - 280.0).max(screen.top() + 4.0)),
    );
    let mut close_menu = false;
    let mut start_rename = false;
    let mut open_delete_confirm = false;
    let frozen_reason = menu.is_frozen.then(|| tr(locale, "nav.frozen_badge"));
    let sid = menu.sheet_id.as_str();

    let area_resp = egui::Area::new(Id::unique(("nav_sheet_ctx_menu", sid))).order(Order::Foreground).fixed_pos(pos).show(ui.ctx(), |menu_ui| {
        egui::Frame::popup(menu_ui.style())
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0, tokens.border))
            .corner_radius(CornerRadius::same(4))
            .show(menu_ui, |ui| {
                ui.set_min_width(232.0);
                if render_nav_ctx_item(
                    ui,
                    meta,
                    NavCtxItemSpec {
                        widget_id: "nav.ctx.rename",
                        label: tr(locale, "cmd.sheet.rename"),
                        command_id: Some("sheet.rename"),
                        enabled: !menu.is_frozen,
                        disabled_reason: frozen_reason,
                        tokens,
                    },
                ) {
                    start_rename = true;
                    close_menu = true;
                }
                if render_nav_ctx_item(
                    ui,
                    meta,
                    NavCtxItemSpec {
                        widget_id: "nav.ctx.duplicate",
                        label: tr(locale, "cmd.sheet.duplicate"),
                        command_id: Some("sheet.duplicate"),
                        enabled: true,
                        disabled_reason: None,
                        tokens,
                    },
                ) {
                    intents.push_engine("sheet.duplicate", json!({ "sheet_id": sid }));
                    close_menu = true;
                }
                if render_nav_ctx_item(
                    ui,
                    meta,
                    NavCtxItemSpec {
                        widget_id: "nav.ctx.duplicate_family",
                        label: tr(locale, "cmd.sheet.duplicateFamily"),
                        command_id: Some("sheet.duplicateFamily"),
                        enabled: true,
                        disabled_reason: None,
                        tokens,
                    },
                ) {
                    intents.push_engine("sheet.duplicateFamily", json!({ "sheet_id": sid }));
                    close_menu = true;
                }
                let freeze_lbl = tr(locale, if menu.is_frozen { "cmd.sheet.unfreeze" } else { "cmd.sheet.freeze" });
                let freeze_cmd = if menu.is_frozen { "sheet.unfreeze" } else { "sheet.freeze" };
                if render_nav_ctx_item(
                    ui,
                    meta,
                    NavCtxItemSpec {
                        widget_id: "nav.ctx.freeze",
                        label: freeze_lbl,
                        command_id: Some(freeze_cmd),
                        enabled: true,
                        disabled_reason: None,
                        tokens,
                    },
                ) {
                    intents.push_engine(freeze_cmd, json!({ "sheet_id": sid }));
                    close_menu = true;
                }

                if !matches!(menu.section, SheetSection::Info) {
                    ui.separator();
                    if render_nav_section_ctx_items(ui, locale, &menu, tokens, meta, intents) {
                        close_menu = true;
                    }
                }

                ui.separator();
                if render_nav_ctx_item(
                    ui,
                    meta,
                    NavCtxItemSpec {
                        widget_id: "nav.ctx.delete",
                        label: tr(locale, "cmd.sheet.delete"),
                        command_id: Some("sheet.delete"),
                        enabled: !menu.is_frozen,
                        disabled_reason: frozen_reason,
                        tokens,
                    },
                ) {
                    open_delete_confirm = true;
                    close_menu = true;
                }
            });
    });

    if start_rename {
        begin_inline_rename(app, state, menu.sheet_id.clone());
    }
    if open_delete_confirm {
        let dependents = collect_downstream_dependents(doc, &menu.sheet_id)
            .into_iter()
            .map(|dep_id| {
                let dep_title = doc_sheet_title(doc, &dep_id).unwrap_or(dep_id.as_str()).to_owned();
                (dep_id, dep_title)
            })
            .collect();
        state.delete_confirm = Some(NavigatorDeleteConfirmState { sheet_id: menu.sheet_id, title, dependents });
    }

    let esc = ui.ctx().input(|i| i.key_pressed(Key::Escape));
    let outside_click =
        ui.ctx().input(|i| i.pointer.primary_clicked() && i.pointer.interact_pos().is_some_and(|p| !area_resp.response.rect.contains(p)));
    if close_menu || esc || outside_click {
        state.context_menu = None;
    }
}

pub(crate) fn render_navigator_delete_confirm_modal(
    ctx: &egui::Context,
    locale: &str,
    state: &mut LeadingDockEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ModalCaptureReport {
    let Some(confirm) = state.delete_confirm.clone() else {
        return ModalCaptureReport::default();
    };

    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        state.delete_confirm = None;
        return ModalCaptureReport::default();
    }

    let mut close_confirm = false;
    let mut execute_delete = false;
    let delete_lbl = tr(locale, "cmd.sheet.delete");
    let cancel_lbl = tr(locale, "btn.cancel");
    let heading = format!("{delete_lbl}: \"{}\"", confirm.title);
    let modal_w = (ctx.content_rect().width() - 48.0).clamp(240.0, 400.0);

    let modal_resp = egui::Modal::new(Id::unique("nav_delete_confirm_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(modal_w);
        ui.set_max_width(modal_w);
        ui.heading(&heading);
        ui.add_space(6.0);
        ui.add(egui::Label::new(format!("Are you sure you want to delete \"{}\"?", confirm.title)).wrap());
        if !confirm.dependents.is_empty() {
            ui.add_space(6.0);
            ui.add(egui::Label::new("The following downstream dependent sheet(s) will also be deleted:").wrap());
            ui.add_space(4.0);
            for (_, dep_title) in &confirm.dependents {
                ui.add(egui::Label::new(format!("• {dep_title}")).wrap());
            }
        } else {
            ui.add_space(4.0);
            ui.add(egui::Label::new(egui::RichText::new("No downstream dependent sheets rely on this sheet.").color(tokens.text_muted)).wrap());
        }
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let cancel_btn = ui.button(cancel_lbl);
            let cancel_vis = cancel_btn.rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
            if cancel_vis.is_positive() {
                meta.record_response(
                    ui,
                    &cancel_btn,
                    FrameInspectEntryInput {
                        widget_id: "nav.delete_confirm.cancel",
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
            }
            if cancel_btn.clicked() {
                close_confirm = true;
            }

            let del_btn = ui.button(egui::RichText::new(delete_lbl).color(tokens.link_red).strong());
            let del_vis = del_btn.rect.intersect(ui.clip_rect()).intersect(meta.viewport_rect);
            if del_vis.is_positive() {
                meta.record_response(
                    ui,
                    &del_btn,
                    FrameInspectEntryInput {
                        widget_id: "nav.delete_confirm.confirm",
                        role: "button",
                        label: delete_lbl,
                        command_id: Some("sheet.delete"),
                        value: Some(json!({ "sheet_id": confirm.sheet_id.as_str(), "cascade": true })),
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::ModalDialog,
                        ellipsized: false,
                        full_label: None,
                    },
                );
            }
            if del_btn.clicked() {
                execute_delete = true;
                close_confirm = true;
            }
        });
    });

    let capture = finalize_egui_modal_response(
        ctx,
        &modal_resp,
        ModalCaptureSpec { widget_id: "dialog.nav_delete_confirm", title: &heading, z_dialog: ZLayer::ModalDialog },
        meta,
    );

    if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        execute_delete = true;
        close_confirm = true;
    }

    if execute_delete {
        intents.push_engine("sheet.delete", json!({ "sheet_id": confirm.sheet_id.as_str(), "cascade": true }));
    }
    if close_confirm {
        state.delete_confirm = None;
    }
    capture
}
