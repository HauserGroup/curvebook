//! Leading Dock: collapsible Project Navigator (`240 pt -> 36 pt` rail, `Cmd/Ctrl+Shift+N`) +
//! Scoped Family Lineage Tree (`~65% / ~35%` split, `Cmd/Ctrl+Shift+F`, `>= 450 ms` hover preview)
//! (`docs/planning/gui-layout-spec.md` §4, `R15.3`).

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use curvebook_doc::{LayoutItem, ProjectDocument, ResultStatus, SheetId, SheetSection};
use egui::{Align, Color32, CornerRadius, FontId, Id, Key, Layout, Modifiers, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Vec2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::inspect_bridge::{
    ModalCaptureReport, ModalCaptureSpec, consume_matching_shortcut, finalize_egui_modal_response, has_registered_shortcut,
};
use crate::egui_shell::ui_kit::{
    ActionMenuEntry, ActionMenuItem, ActionMenuSpec, TooltipSpec, action_menu_popup, show_tooltip, show_tooltip_for_control,
};
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, ResponsiveShellLayout, Tokens, UiIntent, doc_sheet_title};
use crate::inspect::ZLayer;
use crate::menus::{command_tooltip, format_shortcut_for_platform};
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
            Self::All => tr(locale, "nav.filter_all"),
            Self::Data => tr(locale, "nav.data_tables"),
            Self::Info => tr(locale, "nav.info"),
            Self::Results => tr(locale, "nav.results"),
            Self::Graphs => tr(locale, "nav.graphs"),
            Self::Layouts => tr(locale, "nav.layouts"),
        }
    }
}

/// Active right-click context menu target inside the Navigator tree (`R15-GUI-38`, `R20.12`).
#[derive(Debug, Clone, PartialEq)]
pub struct NavigatorContextMenuState {
    pub sheet_id: SheetId,
    pub selected_ids: Vec<SheetId>,
    pub section: SheetSection,
    pub is_frozen: bool,
    pub origin_source_label: Option<String>,
    pub anchor: Pos2,
}

/// Active cascade-delete confirmation modal target triggered from Navigator context menu (`R15-GUI-38`, `R20.12`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigatorDeleteConfirmState {
    pub sheet_id: SheetId,
    pub sheet_ids: Vec<SheetId>,
    pub title: String,
    pub titles: Vec<String>,
    pub dependents: Vec<(SheetId, String)>,
}

/// Persistent `egui_shell` UI state for the Leading Dock (`Navigator` + `Family`).
#[derive(Debug, Clone, PartialEq)]
pub struct LeadingDockEguiState {
    pub search_draft: String,
    pub last_synced_query: String,
    pub kind_chip: NavigatorKindChip,
    pub nav_split_fraction: f32,
    pub selected_sheet_ids: BTreeSet<SheetId>,
    pub selection_anchor_sheet_id: Option<SheetId>,
    pub last_synced_active_sheet_id: Option<SheetId>,
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
            selected_sheet_ids: BTreeSet::new(),
            selection_anchor_sheet_id: None,
            last_synced_active_sheet_id: None,
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
    pub workspace_preset: super::WorkspacePreset,
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
enum RailIconKind {
    Expand { rtl_mirrored: bool },
    Section(SheetSection),
    FamilyTree,
}

#[derive(Clone, Copy)]
struct RailIconButtonSpec<'a> {
    widget_id: &'a str,
    icon: RailIconKind,
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
    visible_ids: &'a [SheetId],
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
    visible_ids: &'a [SheetId],
}

fn sync_navigator_selection(doc: &ProjectDocument, state: &mut LeadingDockEguiState) {
    state.selected_sheet_ids.retain(|id| doc_sheet_title(doc, id).is_some());
    if let Some(anchor) = state.selection_anchor_sheet_id.as_ref()
        && doc_sheet_title(doc, anchor).is_none()
    {
        state.selection_anchor_sheet_id = doc.active_sheet_id.clone();
    }
    if doc.active_sheet_id != state.last_synced_active_sheet_id {
        state.last_synced_active_sheet_id = doc.active_sheet_id.clone();
        state.selected_sheet_ids.clear();
        if let Some(active) = &doc.active_sheet_id {
            state.selected_sheet_ids.insert(active.clone());
        }
        state.selection_anchor_sheet_id = doc.active_sheet_id.clone();
    }
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
    sync_navigator_selection(app.document(), state);
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
        let family_bar_h = tokens.metrics.min_hit_target_pt + tokens.metrics.space_xxs;
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
        "sheet.rename" => can_rename && app.document().active_sheet_id.as_ref().is_some_and(|id| !is_sheet_frozen(app.document(), id)),
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
        && app.document().active_sheet_id.as_ref().is_some_and(|id| !is_sheet_frozen(app.document(), id))
        && app.shortcuts.active_combo("sheet.rename").is_none_or(str::is_empty)
        && !has_registered_shortcut(app, on_result_sheet, Key::F2, Modifiers::NONE);
    if f2_fallback_allowed
        && ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2))
        && let Some(active_id) = app.document().active_sheet_id.clone()
    {
        begin_inline_rename(app, state, active_id);
    }
}

fn is_sheet_frozen(doc: &ProjectDocument, sheet_id: &SheetId) -> bool {
    doc.data_sheet(sheet_id).is_some_and(|ds| ds.frozen)
}

fn begin_inline_rename(app: &UiApp, state: &mut LeadingDockEguiState, sheet_id: SheetId) {
    if is_sheet_frozen(app.document(), &sheet_id) {
        return;
    }
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
        ui.spacing_mut().item_spacing.y = tokens.metrics.space_xs;
        let expand_resp = render_rail_icon_button(
            ui,
            RailIconButtonSpec {
                widget_id: "nav.rail.expand",
                icon: RailIconKind::Expand { rtl_mirrored },
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

        for (chip, section, wid) in [
            (NavigatorKindChip::Data, SheetSection::Data, "nav.rail.data"),
            (NavigatorKindChip::Info, SheetSection::Info, "nav.rail.info"),
            (NavigatorKindChip::Results, SheetSection::Analyses, "nav.rail.results"),
            (NavigatorKindChip::Graphs, SheetSection::Graphs, "nav.rail.graphs"),
            (NavigatorKindChip::Layouts, SheetSection::Layouts, "nav.rail.layouts"),
        ] {
            let selected = state.kind_chip == chip;
            let resp = render_rail_icon_button(
                ui,
                RailIconButtonSpec {
                    widget_id: wid,
                    icon: RailIconKind::Section(section),
                    tooltip: chip.label(&app.locale),
                    command_id: None,
                    selected,
                    enabled,
                },
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
                icon: RailIconKind::FamilyTree,
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

fn rail_command_tooltip(command_id: Option<&str>) -> (Option<String>, Option<&'static str>) {
    let Some(cid) = command_id else {
        return (None, None);
    };
    let sc = match cid {
        "ui.navigator.toggle" => Some(format_shortcut_for_platform("Cmd+Shift+N")),
        "ui.family.toggle" => Some(format_shortcut_for_platform("Cmd+Shift+F")),
        _ => None,
    };
    let summary = match cid {
        "ui.navigator.toggle" => Some("Expand or collapse the Project Navigator pane"),
        "ui.family.toggle" => Some("Toggle the active sheet's Family lineage pane"),
        _ => None,
    };
    (sc, summary)
}

fn render_rail_icon_button(ui: &mut egui::Ui, spec: RailIconButtonSpec<'_>, tokens: &Tokens, meta: &mut FrameInspectMeta) -> Response {
    let RailIconButtonSpec { widget_id, icon, tooltip, command_id, selected, enabled } = spec;
    let rail_btn_sz = tokens.metrics.min_hit_target_pt + tokens.metrics.space_xxs;
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::splat(rail_btn_sz), if enabled { Sense::click() } else { Sense::hover() });
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
        tokens.icon
    };
    let icon_sz = tokens.metrics.space_xl;
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icon_sz));
    match icon {
        RailIconKind::Expand { rtl_mirrored } => draw_rail_expand_vector_icon(ui.painter(), icon_rect, rtl_mirrored, fg),
        RailIconKind::Section(section) => draw_sheet_vector_icon(ui.painter(), icon_rect, section, None, fg),
        RailIconKind::FamilyTree => draw_family_tree_vector_icon(ui.painter(), icon_rect, fg),
    }
    let disabled_reason = (!enabled).then(|| tr(tokens.locale, "tooltip.disabled.close_dialog"));
    let (rail_sc, rail_summary) = rail_command_tooltip(command_id);
    let section_summary = (rail_summary.is_none() && command_id.is_none()).then_some("Open Navigator filtered to this section");
    resp = show_tooltip(
        &resp,
        tokens,
        TooltipSpec { title: tooltip, shortcut: rail_sc.as_deref(), summary: rail_summary.or(section_summary), disabled_reason },
    );
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
            disabled_reason,
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

    render_kind_filter_dropdown(ui, app, state, tokens, avail_w, enabled, meta);
    ui.add_space(tokens.metrics.space_xs);
    ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(), Stroke::new(1.0, tokens.divider));
    ui.add_space(tokens.metrics.space_xs);

    let visible_ids = collect_visible_sheet_ids(app, state);
    handle_navigator_keyboard(ui, app, state, &ctx_in, &visible_ids, intents);

    egui::ScrollArea::vertical().id_salt("leading_nav_scroll").animated(false).auto_shrink([false, false]).show(ui, |ui| {
        let row_w = (ui.available_width() - tokens.metrics.space_lg).max(96.0);
        let q = state.search_draft.trim().to_lowercase();
        let nested_mode = matches!(app.nav.nesting_mode, NavigatorNestingMode::NestedResultsUnderData);
        let tree_ctx = NavigatorTreeCtx { app, dock: ctx_in, row_w, query: &q, visible_ids: &visible_ids };

        if nested_mode {
            render_nested_navigator_tree(ui, tree_ctx, state, signals, meta, intents);
        } else {
            render_folder_navigator_tree(ui, tree_ctx, state, signals, meta, intents);
        }
    });
}

const fn chip_section(chip: NavigatorKindChip) -> Option<SheetSection> {
    match chip {
        NavigatorKindChip::All => None,
        NavigatorKindChip::Data => Some(SheetSection::Data),
        NavigatorKindChip::Info => Some(SheetSection::Info),
        NavigatorKindChip::Results => Some(SheetSection::Analyses),
        NavigatorKindChip::Graphs => Some(SheetSection::Graphs),
        NavigatorKindChip::Layouts => Some(SheetSection::Layouts),
    }
}

fn section_count_for_chip(app: &UiApp, chip: NavigatorKindChip) -> usize {
    match chip {
        NavigatorKindChip::All => app.nav.all_entries.len(),
        _ => app.nav.all_entries.iter().filter(|e| chip.matches_section(e.section)).count(),
    }
}

fn render_kind_filter_dropdown(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut LeadingDockEguiState,
    tokens: &Tokens,
    avail_w: f32,
    enabled: bool,
    meta: &mut FrameInspectMeta,
) {
    let locale = app.locale.as_str();
    let cur_count = section_count_for_chip(app, state.kind_chip);
    let active_label = format!("{} ({cur_count})", state.kind_chip.label(locale));
    let mut chosen_chip = None;

    let combo = super::widgets::dropdown("nav.kind_filter", egui::RichText::new(&active_label).size(tokens.metrics.font_ui_small_pt))
        .width(avail_w)
        .height(tokens.metrics.min_hit_target_pt)
        .enabled(enabled)
        .accessible_name(active_label.clone())
        .show_ui(ui, |popup_ui| {
            let row_font = FontId::proportional(tokens.metrics.font_ui_small_pt);
            let icon_w = tokens.metrics.space_lg + tokens.metrics.space_xxs;
            let gap = tokens.metrics.space_sm;
            let pad_x = tokens.metrics.space_md;
            let row_h = tokens.metrics.min_hit_target_pt;
            let mut max_w = popup_ui.available_width();
            let mut rows = Vec::with_capacity(NavigatorKindChip::ALL.len());
            for chip in NavigatorKindChip::ALL {
                let lbl = chip.label(locale).to_owned();
                let count_str = format!("({})", section_count_for_chip(app, chip));
                let lbl_w = popup_ui.painter().layout_no_wrap(lbl.clone(), row_font.clone(), tokens.text_primary).size().x;
                let cnt_w = popup_ui.painter().layout_no_wrap(count_str.clone(), row_font.clone(), tokens.text_muted).size().x;
                max_w = max_w.max(pad_x * 2.0 + icon_w + gap * 2.0 + lbl_w + cnt_w + tokens.metrics.space_md);
                rows.push((chip, lbl, count_str));
            }
            let row_w = max_w.min(popup_ui.max_rect().width().max(avail_w));
            for (chip, lbl, count_str) in rows {
                let is_selected = state.kind_chip == chip;
                let (row_rect, row_resp) =
                    popup_ui.allocate_exact_size(Vec2::new(row_w, row_h), if enabled { Sense::click() } else { Sense::hover() });
                super::widgets::paint_hover_row(
                    popup_ui,
                    row_rect,
                    CornerRadius::same(tokens.metrics.radius_md),
                    row_resp.hovered(),
                    is_selected,
                    tokens,
                );
                let fg = if !enabled {
                    tokens.text_disabled
                } else if is_selected {
                    tokens.selected_accent_text()
                } else {
                    tokens.text_primary
                };
                let icon_fg = if !enabled {
                    tokens.text_disabled
                } else if is_selected {
                    tokens.selected_accent_text()
                } else {
                    tokens.icon
                };
                let icon_h = tokens.metrics.space_lg;
                let icon_rect =
                    Rect::from_min_size(Pos2::new(row_rect.left() + pad_x, row_rect.center().y - icon_h * 0.5), Vec2::new(icon_w, icon_h));
                match chip_section(chip) {
                    Some(section) => draw_sheet_vector_icon(popup_ui.painter(), icon_rect, section, None, icon_fg),
                    None => draw_all_sheets_vector_icon(popup_ui.painter(), icon_rect, icon_fg),
                }
                let text_x = icon_rect.right() + gap;
                let cnt_galley =
                    popup_ui.painter().layout_no_wrap(count_str.clone(), row_font.clone(), if is_selected { fg } else { tokens.text_muted });
                let lbl_max_w = (row_rect.right() - pad_x - cnt_galley.size().x - gap - text_x).max(20.0);
                let (lbl_galley, elided) = super::ui_kit::elided_galley(popup_ui.painter(), &lbl, row_font.clone(), fg, lbl_max_w, false);
                popup_ui.painter().with_clip_rect(row_rect.shrink(tokens.metrics.space_xxs)).galley(
                    Pos2::new(text_x, row_rect.center().y - lbl_galley.size().y * 0.5),
                    lbl_galley,
                    fg,
                );
                popup_ui.painter().galley(
                    Pos2::new(row_rect.right() - pad_x - cnt_galley.size().x, row_rect.center().y - cnt_galley.size().y * 0.5),
                    cnt_galley,
                    if is_selected { fg } else { tokens.text_muted },
                );

                let full_item_lbl = format!("{lbl} {count_str}");
                let a11y = full_item_lbl.clone();
                row_resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::MenuItem, enabled, is_selected, a11y.clone()));
                meta.record_response(
                    popup_ui,
                    &row_resp,
                    FrameInspectEntryInput {
                        widget_id: chip.widget_id(),
                        role: "menuitem",
                        label: &full_item_lbl,
                        command_id: None,
                        value: Some(json!({ "selected": is_selected, "kind": chip.widget_id() })),
                        enabled,
                        disabled_reason: (!enabled).then(|| tr(&app.locale, "tooltip.disabled.close_dialog")),
                        z_layer: ZLayer::DropdownMenu,
                        ellipsized: elided,
                        full_label: elided.then_some(full_item_lbl.as_str()),
                    },
                );
                if row_resp.clicked() && enabled {
                    chosen_chip = Some(chip);
                }
            }
        });

    if let Some(chip) = chosen_chip {
        state.kind_chip = chip;
    }
    meta.record_response(
        ui,
        &combo.response,
        FrameInspectEntryInput {
            widget_id: "nav.kind_filter",
            role: "combobox",
            label: &active_label,
            command_id: None,
            value: Some(json!({ "selected_kind": state.kind_chip.widget_id(), "count": cur_count })),
            enabled,
            disabled_reason: (!enabled).then(|| tr(&app.locale, "tooltip.disabled.close_dialog")),
            z_layer: ZLayer::Workspace,
            ellipsized: combo.ellipsized,
            full_label: combo.ellipsized.then_some(active_label.as_str()),
        },
    );
}

fn render_folder_navigator_tree(
    ui: &mut egui::Ui,
    tree: NavigatorTreeCtx<'_>,
    state: &mut LeadingDockEguiState,
    signals: &mut NavFrameSignals,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let NavigatorTreeCtx { app, dock: ctx_in, row_w, query, visible_ids } = tree;
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
                        visible_ids,
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
    let NavigatorTreeCtx { app, dock: ctx_in, row_w, query, visible_ids } = tree;
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
                    visible_ids,
                },
                state,
                signals,
                meta,
                intents,
            );
            for (child, _) in &matching_children {
                placed.insert(child.sheet_id.clone());
            }
            if !collapsed {
                for (child, depth) in matching_children {
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
                            visible_ids,
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
                        visible_ids,
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
        if let Some(src) = res.analysis.source_sheet_id.as_ref()
            && (src == data_id || owned_results.contains(src))
        {
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

fn draw_disclosure_triangle(painter: &egui::Painter, center: Pos2, collapsed: bool, color: Color32) {
    let r = 3.2_f32;
    let stroke = Stroke::new(1.5, color);
    if collapsed {
        let top = Pos2::new(center.x - r * 0.45, center.y - r);
        let tip = Pos2::new(center.x + r * 0.65, center.y);
        let bot = Pos2::new(center.x - r * 0.45, center.y + r);
        painter.line_segment([top, tip], stroke);
        painter.line_segment([tip, bot], stroke);
    } else {
        let left = Pos2::new(center.x - r, center.y - r * 0.45);
        let tip = Pos2::new(center.x, center.y + r * 0.65);
        let right = Pos2::new(center.x + r, center.y - r * 0.45);
        painter.line_segment([left, tip], stroke);
        painter.line_segment([tip, right], stroke);
    }
}

fn render_folder_header(ui: &mut egui::Ui, app: &UiApp, spec: FolderHeaderSpec<'_>, meta: &mut FrameInspectMeta, intents: &mut PendingUiIntents) {
    let FolderHeaderSpec { section, count, collapsed, row_w, tokens, enabled } = spec;
    let folder_h = tokens.metrics.min_hit_target_pt;
    let Some((rect, _)) = allocate_visible_slot(ui, Vec2::new(row_w, folder_h), Sense::hover()) else {
        return;
    };

    let mut resp = ui.interact(
        rect,
        ui.make_persistent_id(("nav_section_header", section_header_widget_id(section))),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let folder_name = tr(&app.locale, section_label_key(section));
    let label = format!("{folder_name} ({count})");
    if enabled {
        super::widgets::paint_hover_row(ui, rect, CornerRadius::same(tokens.metrics.radius_md), resp.hovered(), false, tokens);
    }
    let icon_fg = if enabled { tokens.icon } else { tokens.text_disabled };
    draw_disclosure_triangle(ui.painter(), Pos2::new(rect.left() + 9.0, rect.center().y), collapsed, icon_fg);
    let icon_h = tokens.metrics.space_lg;
    let icon_w = icon_h + 1.0;
    let icon_rect = Rect::from_min_size(Pos2::new(rect.left() + 18.0, rect.center().y - icon_h * 0.5), Vec2::new(icon_w, icon_h));
    draw_sheet_vector_icon(ui.painter(), icon_rect, section, None, icon_fg);
    let text_x = icon_rect.right() + tokens.metrics.space_xs;
    let text_max_w = (rect.right() - text_x - tokens.metrics.space_xs).max(16.0);
    let (galley, elided) = super::ui_kit::elided_galley(
        ui.painter(),
        &label,
        FontId::proportional(tokens.metrics.font_ui_small_pt),
        tokens.text_muted,
        text_max_w,
        false,
    );
    ui.painter().with_clip_rect(rect.shrink(tokens.metrics.space_xxs)).galley(
        Pos2::new(text_x, rect.center().y - galley.size().y * 0.5),
        galley,
        tokens.text_muted,
    );

    if elided {
        resp = show_tooltip(&resp, tokens, TooltipSpec::new(&label));
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

fn ordered_selected_subset(visible_ids: &[SheetId], selected_set: &BTreeSet<SheetId>) -> Vec<SheetId> {
    let mut out: Vec<SheetId> = visible_ids.iter().filter(|id| selected_set.contains(*id)).cloned().collect();
    for id in selected_set {
        if !out.contains(id) {
            out.push(id.clone());
        }
    }
    out
}

fn read_click_modifiers(ui: &egui::Ui) -> (bool, bool) {
    ui.ctx().input(|i| {
        let mut m = i.modifiers;
        for ev in &i.events {
            if let egui::Event::PointerButton { modifiers, .. } = ev {
                m.shift |= modifiers.shift;
                m.command |= modifiers.command;
                m.ctrl |= modifiers.ctrl;
                m.mac_cmd |= modifiers.mac_cmd;
            }
        }
        (m.shift, m.command || m.ctrl || m.mac_cmd)
    })
}

struct NavRowDragPaintCtx<'a> {
    doc: &'a ProjectDocument,
    entry: &'a NavigatorEntry,
    section_index: usize,
    tokens: &'a Tokens,
}

fn paint_navigator_row_drag_drop(
    ui: &egui::Ui,
    rect: Rect,
    ctx: NavRowDragPaintCtx<'_>,
    state: &LeadingDockEguiState,
    signals: &mut NavFrameSignals,
) {
    let NavRowDragPaintCtx { doc, entry, section_index, tokens } = ctx;
    let pointer_pos = ui.ctx().input(|i| i.pointer.latest_pos().or_else(|| i.pointer.hover_pos()));
    if !(pointer_pos.is_some_and(|p| rect.contains(p)) && state.dragging_sheet_id.is_some()) {
        return;
    }
    match evaluate_drag_drop_outcome(doc, state.dragging_sheet_id.as_ref(), entry, section_index) {
        DragDropOutcome::ReorderCandidate { target_index, .. } => {
            ui.painter().hline(rect.x_range(), rect.top() + 1.0, Stroke::new(2.0, tokens.accent));
            if ui.ctx().input(|i| i.pointer.any_released())
                && let Some(drag_id) = state.dragging_sheet_id.clone()
            {
                signals.drop_commit = Some(DropCommitAction::Reorder { sheet_id: drag_id, target_index });
            }
        }
        DragDropOutcome::RetargetCompatible => {
            ui.painter().rect_stroke(rect, CornerRadius::same(tokens.metrics.radius_md), Stroke::new(1.5, tokens.link_green), StrokeKind::Inside);
            if ui.ctx().input(|i| i.pointer.any_released())
                && let Some(drag_id) = state.dragging_sheet_id.clone()
            {
                signals.drop_commit = Some(DropCommitAction::Retarget { result_id: drag_id, target_data_id: entry.sheet_id.clone() });
            }
        }
        DragDropOutcome::RetargetIncompatible => {
            ui.painter().rect_stroke(rect, CornerRadius::same(tokens.metrics.radius_md), Stroke::new(1.5, tokens.link_red), StrokeKind::Inside);
        }
        DragDropOutcome::None => {}
    }
}

struct NavRowInteractionCtx<'a> {
    app: &'a UiApp,
    entry: &'a NavigatorEntry,
    visible_ids: &'a [SheetId],
    enabled: bool,
}

fn handle_navigator_row_interaction(
    ui: &egui::Ui,
    resp: &Response,
    rect: Rect,
    ctx: NavRowInteractionCtx<'_>,
    state: &mut LeadingDockEguiState,
    intents: &mut PendingUiIntents,
) {
    let NavRowInteractionCtx { app, entry, visible_ids, enabled } = ctx;
    if !enabled {
        return;
    }
    if resp.drag_started() {
        state.dragging_sheet_id = Some(entry.sheet_id.clone());
    }
    if resp.secondary_clicked() {
        let anchor = ui.ctx().input(|i| i.pointer.interact_pos()).unwrap_or(rect.center());
        let selected_ids = if state.selected_sheet_ids.contains(&entry.sheet_id) && state.selected_sheet_ids.len() > 1 {
            ordered_selected_subset(visible_ids, &state.selected_sheet_ids)
        } else {
            state.selected_sheet_ids.clear();
            state.selected_sheet_ids.insert(entry.sheet_id.clone());
            state.selection_anchor_sheet_id = Some(entry.sheet_id.clone());
            vec![entry.sheet_id.clone()]
        };
        state.context_menu = Some(NavigatorContextMenuState {
            sheet_id: entry.sheet_id.clone(),
            selected_ids,
            section: entry.section,
            is_frozen: entry.is_frozen,
            origin_source_label: entry.origin.as_ref().map(|o| o.source_label.clone()),
            anchor,
        });
    }
    if resp.double_clicked() && !entry.is_frozen {
        begin_inline_rename(app, state, entry.sheet_id.clone());
        return;
    }
    if !resp.clicked() || state.dragging_sheet_id.is_some() {
        return;
    }
    let (shift_held, cmd_held) = read_click_modifiers(ui);
    if shift_held {
        let anchor = state.selection_anchor_sheet_id.clone().or_else(|| app.document().active_sheet_id.clone());
        if let Some(anchor_id) = anchor
            && let (Some(a), Some(b)) = (visible_ids.iter().position(|id| id == &anchor_id), visible_ids.iter().position(|id| id == &entry.sheet_id))
        {
            state.selected_sheet_ids.clear();
            for id in &visible_ids[a.min(b)..=a.max(b)] {
                state.selected_sheet_ids.insert(id.clone());
            }
            state.selection_anchor_sheet_id = Some(anchor_id);
        } else {
            state.selected_sheet_ids.insert(entry.sheet_id.clone());
            state.selection_anchor_sheet_id = Some(entry.sheet_id.clone());
        }
    } else if cmd_held {
        if state.selected_sheet_ids.is_empty()
            && let Some(active) = app.document().active_sheet_id.as_ref()
        {
            state.selected_sheet_ids.insert(active.clone());
        }
        if !state.selected_sheet_ids.remove(&entry.sheet_id) {
            state.selected_sheet_ids.insert(entry.sheet_id.clone());
        }
        state.selection_anchor_sheet_id = Some(entry.sheet_id.clone());
    } else {
        state.selected_sheet_ids.clear();
        state.selected_sheet_ids.insert(entry.sheet_id.clone());
        state.selection_anchor_sheet_id = Some(entry.sheet_id.clone());
        state.last_synced_active_sheet_id = Some(entry.sheet_id.clone());
        intents.push_engine("sheet.select", json!({ "sheet_id": entry.sheet_id.as_str(), "mode": "navigator" }));
        intents.push(UiIntent::SetFocusPane { pane: ActiveWorkspacePane::Navigator });
    }
}

fn render_navigator_row(
    ui: &mut egui::Ui,
    row: NavigatorRowSpec<'_>,
    state: &mut LeadingDockEguiState,
    signals: &mut NavFrameSignals,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let NavigatorRowSpec { app, dock: ctx_in, entry, section_index, indent_depth, is_nested_parent, nested_collapsed, row_w, visible_ids } = row;
    let tokens = ctx_in.tokens;
    let enabled = ctx_in.enabled;
    let doc = app.document();
    let title = if entry.title.trim().is_empty() { tr(&app.locale, "nav.untitled_sheet") } else { entry.title.as_str() };
    let show_two_line =
        entry.subtitle.is_some() && (matches!(entry.section, SheetSection::Graphs | SheetSection::Analyses) || entry.has_duplicate_title);
    let row_h = if show_two_line { 38.0_f32 } else { 26.0_f32 };
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

    let in_multi_select = state.selected_sheet_ids.contains(&entry.sheet_id);
    let is_multi_secondary = !entry.is_active && in_multi_select;
    let radius = CornerRadius::same(tokens.metrics.radius_md);

    if entry.is_active {
        if enabled {
            super::widgets::paint_hover_row(ui, rect, radius, resp.hovered(), true, tokens);
        } else {
            ui.painter().rect_filled(rect, radius, tokens.active_bg);
        }
        let bar_w = tokens.metrics.space_xxs + 1.0;
        let bar = Rect::from_min_size(Pos2::new(rect.left() + 1.0, rect.top() + 3.0), Vec2::new(bar_w, rect.height() - 6.0));
        ui.painter().rect_filled(bar, CornerRadius::same(1), tokens.accent);
    } else if is_multi_secondary {
        ui.painter().rect_filled(rect, radius, tokens.selection_unemphasized_bg);
        if enabled && resp.hovered() {
            ui.painter().rect_filled(rect, radius, tokens.hover_bg.linear_multiply(0.5));
        }
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
    } else if enabled {
        super::widgets::paint_hover_row(ui, rect, radius, resp.hovered(), false, tokens);
    }

    paint_navigator_row_drag_drop(ui, rect, NavRowDragPaintCtx { doc, entry, section_index, tokens }, state, signals);

    let line1_y = if show_two_line { rect.top() + 13.0 } else { rect.center().y };
    // Disclosure button if nested parent
    let mut x_cur = rect.left() + 5.0 + indent_px;
    if is_nested_parent {
        let tri_sz = tokens.metrics.space_xl;
        let tri_rect = Rect::from_min_size(Pos2::new(x_cur, line1_y - tri_sz * 0.5), Vec2::splat(tri_sz));
        let tri_resp =
            ui.interact(tri_rect, ui.scope_id().with(("nested_tri", &entry.sheet_id)), if enabled { Sense::click() } else { Sense::hover() });
        draw_disclosure_triangle(ui.painter(), tri_rect.center(), nested_collapsed, tokens.text_muted);
        if tri_resp.clicked() && enabled {
            if nested_collapsed {
                state.nested_collapsed_parents.remove(&entry.sheet_id);
            } else {
                state.nested_collapsed_parents.insert(entry.sheet_id.clone());
            }
        }
        x_cur += tri_sz;
    }

    // Crisp pixel-snapped vector icon
    let icon_h = tokens.metrics.space_lg;
    let icon_w = icon_h + tokens.metrics.space_xxs;
    let icon_rect = Rect::from_min_size(Pos2::new(x_cur, line1_y - icon_h * 0.5), Vec2::new(icon_w, icon_h));
    let icon_fg = if !enabled {
        tokens.text_disabled
    } else if entry.is_active {
        tokens.selected_accent_text()
    } else {
        tokens.icon
    };
    draw_sheet_vector_icon(ui.painter(), icon_rect, entry.section, entry.archetype_label.as_deref(), icon_fg);
    x_cur += icon_w + tokens.metrics.space_xs;

    // Reserve trailing badges on right of Line 1
    let badge_w = compute_badge_strip_width(entry);
    let title_max_w = (rect.right() - x_cur - badge_w - tokens.metrics.space_sm).max(20.0);
    let fg = if !enabled {
        tokens.text_disabled
    } else if entry.is_active {
        tokens.selected_accent_text()
    } else if entry.is_bold || in_multi_select {
        tokens.text_strong
    } else {
        tokens.text_primary
    };
    let font_size = if entry.is_active || entry.is_bold { 12.0 } else { 11.5 };
    let (galley, ellipsized) = super::ui_kit::elided_galley(ui.painter(), title, FontId::proportional(font_size), fg, title_max_w, false);
    let line1_top = rect.top() + 2.0;
    let line1_bot = if show_two_line { rect.top() + 22.0 } else { rect.bottom() - 2.0 };
    let title_clip = Rect::from_min_max(Pos2::new(x_cur, line1_top), Pos2::new(x_cur + title_max_w, line1_bot));
    ui.painter().with_clip_rect(title_clip).galley(Pos2::new(x_cur, line1_y - galley.size().y * 0.5), galley, fg);

    paint_row_badges(ui.painter(), rect, line1_y, entry, tokens, &app.locale);

    if show_two_line && let Some(sub) = entry.subtitle.as_deref() {
        let sub_max_w = (rect.right() - x_cur - tokens.metrics.space_sm).max(20.0);
        let sub_fg = if !enabled {
            tokens.text_disabled
        } else if entry.is_active {
            tokens.selected_accent_text().linear_multiply(0.88)
        } else {
            tokens.text_muted
        };
        let (sub_galley, _) =
            super::ui_kit::elided_galley(ui.painter(), sub, FontId::proportional(tokens.metrics.font_ui_small_pt), sub_fg, sub_max_w, false);
        let sub_clip =
            Rect::from_min_max(Pos2::new(x_cur, rect.top() + 20.0), Pos2::new(rect.right() - tokens.metrics.space_sm, rect.bottom() - 2.0));
        ui.painter().with_clip_rect(sub_clip).galley(Pos2::new(x_cur, rect.top() + 21.0), sub_galley, sub_fg);
    }
    let mut sheet_summary = build_sheet_hover_summary(doc, &entry.sheet_id, entry.subtitle.as_deref());
    if let Some(orig) = entry.origin.as_ref() {
        let origin_tip = format!("{} {} ({})", tr(&app.locale, "nav.origin_badge_prefix"), orig.source_label, orig.source_format);
        if sheet_summary.is_empty() {
            sheet_summary = origin_tip;
        } else {
            sheet_summary.push('\n');
            sheet_summary.push_str(&origin_tip);
        }
    }
    let row_disabled_reason = (!enabled).then(|| tr(&app.locale, "tooltip.disabled.close_dialog"));
    resp = show_tooltip_for_control(
        &resp,
        tokens,
        title,
        ellipsized,
        TooltipSpec {
            title,
            shortcut: None,
            summary: (!sheet_summary.is_empty()).then_some(sheet_summary.as_str()),
            disabled_reason: row_disabled_reason,
        },
    );

    handle_navigator_row_interaction(ui, &resp, rect, NavRowInteractionCtx { app, entry, visible_ids, enabled }, state, intents);

    let wid = format!("nav.sheet.{}", entry.sheet_id.as_str());
    let a11y_lbl = title.to_owned();
    let is_selected = entry.is_active || state.selected_sheet_ids.contains(&entry.sheet_id);
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::TreeItem, enabled, is_selected, a11y_lbl.clone()));
    let mut row_value = json!({
        "sheet_id": entry.sheet_id.as_str(),
        "selected": is_selected,
        "active": entry.is_active,
        "multi_selected": state.selected_sheet_ids.contains(&entry.sheet_id),
        "selected_count": state.selected_sheet_ids.len(),
        "bold": entry.is_bold,
        "row_h": row_h,
    });
    if let Some(ref orig) = entry.origin
        && let Some(obj) = row_value.as_object_mut()
    {
        obj.insert(String::from("origin_source"), json!(orig.source_label));
        obj.insert(String::from("origin_format"), json!(orig.source_format));
    }
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: &wid,
            role: "treeitem",
            label: title,
            command_id: Some("sheet.select"),
            value: Some(row_value),
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
    let Some((rect, mut resp)) = allocate_visible_slot(ui, Vec2::new(row_w, 26.0), if row_enabled { Sense::click() } else { Sense::hover() }) else {
        return;
    };
    let fg = if !row_enabled {
        tokens.text_disabled
    } else if resp.hovered() {
        tokens.text_strong
    } else {
        tokens.text_muted
    };
    if row_enabled {
        super::widgets::paint_hover_row(ui, rect, CornerRadius::same(tokens.metrics.radius_md), resp.hovered(), false, tokens);
    }
    let (galley, elided) =
        super::ui_kit::elided_galley(ui.painter(), &text, FontId::proportional(tokens.metrics.font_ui_small_pt), fg, (row_w - 12.0).max(20.0), false);
    ui.painter().with_clip_rect(rect.shrink(2.0)).galley(Pos2::new(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5), galley, fg);

    let disabled_reason = (!enabled).then(|| tr(locale, "tooltip.disabled.close_dialog")).or(disabled_reason);
    let new_summary = cmd.and_then(|cid| command_tooltip(app, cid).map(|t| t.summary)).filter(|s| !s.trim().is_empty());
    let new_sc = cmd.and_then(|cid| app.shortcuts.active_combo(cid)).filter(|s| !s.trim().is_empty()).map(format_shortcut_for_platform);
    resp = show_tooltip_for_control(
        &resp,
        tokens,
        &text,
        elided,
        TooltipSpec {
            title: &text,
            shortcut: new_sc.as_deref(),
            summary: new_summary,
            disabled_reason: (!row_enabled).then_some(disabled_reason).flatten(),
        },
    );

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
    ui.add_space(1.0);
    let w = ui.available_width().max(80.0);
    let label = format!("▸ {}", tr(&app.locale, "nav.family"));
    let resp = render_compact_chip_height(
        ui,
        CompactChipSpec { max_w: w, widget_id: "family.header.toggle", label: &label, selected: false, enabled },
        tokens.metrics.min_hit_target_pt,
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
        let row_w = (ui.available_width() - tokens.metrics.space_lg).max(80.0);
        for (idx, (depth, node)) in ordered.iter().enumerate() {
            let show_sub = node.has_duplicate_title && node.subtitle.is_some();
            let row_h = if show_sub { 36.0_f32 } else { tokens.metrics.min_hit_target_pt };
            let Some((rect, mut resp)) = allocate_visible_slot(ui, Vec2::new(row_w, row_h), if enabled { Sense::click() } else { Sense::hover() })
            else {
                continue;
            };
            let in_chain = node.is_active || node.is_anchor || active_chain.contains(&node.sheet_id);

            if enabled {
                super::widgets::paint_hover_row(ui, rect, CornerRadius::same(tokens.metrics.radius_md), resp.hovered(), node.is_active, tokens);
            } else if node.is_active {
                ui.painter().rect_filled(rect, CornerRadius::same(tokens.metrics.radius_md), tokens.active_bg);
            }

            let line1_y = if show_sub { rect.top() + 12.0 } else { rect.center().y };
            let indent = ((*depth).min(4) as f32) * 12.0;
            let spine_x = rect.left() + 6.0 + indent * 0.5;
            if *depth > 0 {
                ui.painter().vline(spine_x, rect.top()..=line1_y, Stroke::new(1.0, if in_chain { tokens.accent } else { tokens.divider }));
                ui.painter().hline(
                    spine_x..=(rect.left() + 10.0 + indent),
                    line1_y,
                    Stroke::new(1.0, if in_chain { tokens.accent } else { tokens.divider }),
                );
            }

            let icon_x = rect.left() + 12.0 + indent;
            let icon_w = tokens.metrics.space_lg;
            let icon_h = tokens.metrics.space_lg - tokens.metrics.space_xxs;
            let icon_rect = Rect::from_min_size(Pos2::new(icon_x, line1_y - icon_h * 0.5), Vec2::new(icon_w, icon_h));
            let icon_fg = if node.is_active {
                tokens.selected_accent_text()
            } else if in_chain {
                tokens.accent
            } else {
                tokens.icon
            };
            draw_sheet_vector_icon(ui.painter(), icon_rect, node.section, None, icon_fg);

            let multi_dep = multi_dependency_badge(doc, &node.sheet_id);
            let badge_reserve = if multi_dep.is_some() { 56.0 } else { 4.0 };
            let text_x = icon_rect.right() + 5.0;
            let max_text_w = (rect.right() - text_x - badge_reserve).max(16.0);
            let fg = if node.is_active {
                tokens.selected_accent_text()
            } else if in_chain {
                tokens.text_strong
            } else {
                tokens.text_primary
            };
            let (galley, elided) = super::ui_kit::elided_galley(
                ui.painter(),
                &node.title,
                FontId::proportional(if in_chain { 11.5 } else { 11.0 }),
                fg,
                max_text_w,
                false,
            );
            let line1_bot = if show_sub { rect.top() + 20.0 } else { rect.bottom() };
            ui.painter()
                .with_clip_rect(Rect::from_min_max(Pos2::new(text_x, rect.top()), Pos2::new(rect.right() - badge_reserve, line1_bot)))
                .galley(Pos2::new(text_x, line1_y - galley.size().y * 0.5), galley, fg);

            if let Some(dep_lbl) = multi_dep {
                ui.painter().text(
                    Pos2::new(rect.right() - 4.0, line1_y),
                    egui::Align2::RIGHT_CENTER,
                    dep_lbl,
                    FontId::proportional(tokens.metrics.font_ui_small_pt),
                    tokens.text_muted,
                );
            }

            if show_sub && let Some(sub) = node.subtitle.as_deref() {
                let sub_max_w = (rect.right() - text_x - 4.0).max(16.0);
                let sub_fg = if node.is_active { tokens.selected_accent_text().linear_multiply(0.88) } else { tokens.text_muted };
                let (sub_galley, _) =
                    super::ui_kit::elided_galley(ui.painter(), sub, FontId::proportional(tokens.metrics.font_ui_small_pt), sub_fg, sub_max_w, false);
                ui.painter()
                    .with_clip_rect(Rect::from_min_max(Pos2::new(text_x, rect.top() + 19.0), Pos2::new(rect.right() - 4.0, rect.bottom() - 2.0)))
                    .galley(Pos2::new(text_x, rect.top() + 20.0), sub_galley, sub_fg);
            }
            let fam_summary = build_sheet_hover_summary(doc, &node.sheet_id, node.subtitle.as_deref());
            let fam_disabled_reason = (!enabled).then(|| tr(&app.locale, "tooltip.disabled.close_dialog"));
            resp = show_tooltip_for_control(
                &resp,
                tokens,
                &node.title,
                elided,
                TooltipSpec {
                    title: &node.title,
                    shortcut: None,
                    summary: (!fam_summary.is_empty()).then_some(fam_summary.as_str()),
                    disabled_reason: fam_disabled_reason,
                },
            );

            if resp.hovered() {
                signals.any_row_hovered_family = Some(node.sheet_id.clone());
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
                    value: Some(json!({ "sheet_id": node.sheet_id.as_str(), "idx": idx, "in_chain": in_chain, "row_h": row_h })),
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
    if let Some(res) = doc.result_sheet(&active)
        && let Some(src) = &res.analysis.source_sheet_id
    {
        chain.insert(src.clone());
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

fn push_unique_source_title(doc: &ProjectDocument, sheet_id: &SheetId, seen: &mut BTreeSet<SheetId>, out: &mut Vec<String>) {
    if seen.insert(sheet_id.clone()) {
        out.push(doc_sheet_title(doc, sheet_id).unwrap_or(sheet_id.as_str()).to_owned());
    }
}

fn build_sheet_hover_summary(doc: &ProjectDocument, sheet_id: &SheetId, fallback_sub: Option<&str>) -> String {
    if let Some(ds) = doc.data_sheet(sheet_id) {
        let arch = ds.kind().label();
        let dims = format!("{}×{}", ds.archetype.row_count(), ds.archetype.column_count());
        let mut seen = BTreeSet::new();
        let mut linked_srcs = Vec::new();
        for reg in &ds.linked_regions {
            if let Some(up_id) = reg.spec.source.upstream_sheet_id() {
                push_unique_source_title(doc, up_id, &mut seen, &mut linked_srcs);
            }
        }
        return if linked_srcs.is_empty() {
            format!("{arch} • {dims}")
        } else {
            format!("{arch} • {dims} • Sources ({}): {}", linked_srcs.len(), linked_srcs.join(", "))
        };
    }
    if let Some(info) = doc.info_sheet(sheet_id) {
        return format!("Info • {} constants", info.constants.len());
    }
    if let Some(rs) = doc.result_sheet(sheet_id) {
        if let Some(src_id) = rs.analysis.source_sheet_id.as_ref() {
            let src_title = doc_sheet_title(doc, src_id).unwrap_or(src_id.as_str());
            return format!("Result • Source: {src_title}");
        }
        return String::from("Result • Imported");
    }
    if let Some(gs) = doc.graph_sheet(sheet_id) {
        let kind_lbl = format!("{:?}", gs.spec.kind);
        let mut seen = BTreeSet::new();
        let mut srcs = Vec::new();
        push_unique_source_title(doc, &gs.spec.primary_source_sheet_id, &mut seen, &mut srcs);
        for s in &gs.spec.series {
            push_unique_source_title(doc, &s.source_sheet_id, &mut seen, &mut srcs);
        }
        return if srcs.is_empty() {
            format!("Graph ({kind_lbl}) • {} series", gs.spec.series.len())
        } else {
            format!("Graph ({kind_lbl}) • Sources ({}): {}", srcs.len(), srcs.join(", "))
        };
    }
    if let Some(ls) = doc.layout_sheet(sheet_id) {
        let mut seen = BTreeSet::new();
        let mut srcs = Vec::new();
        for item in &ls.spec.items {
            match item {
                LayoutItem::LiveGraph { graph_sheet_id, .. } => push_unique_source_title(doc, graph_sheet_id, &mut seen, &mut srcs),
                LayoutItem::LiveTable { source_sheet_id, .. } => push_unique_source_title(doc, source_sheet_id, &mut seen, &mut srcs),
                LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => {}
            }
        }
        return if srcs.is_empty() {
            format!("Page Layout • {} panels", ls.spec.items.len())
        } else {
            format!("Page Layout ({} panels) • Sources ({}): {}", ls.spec.items.len(), srcs.len(), srcs.join(", "))
        };
    }
    fallback_sub.map(str::trim).unwrap_or("").to_owned()
}

fn multi_dependency_badge(doc: &ProjectDocument, sheet_id: &SheetId) -> Option<String> {
    if let Some(g) = doc.graph_sheet(sheet_id)
        && g.spec.series.len() > 1
    {
        return Some(format!("{} sources", g.spec.series.len()));
    }
    if let Some(l) = doc.layout_sheet(sheet_id)
        && l.spec.items.len() > 1
    {
        return Some(format!("{} sources", l.spec.items.len()));
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
    let dismiss = ui.ctx().input(|i| {
        i.pointer.any_down() || i.smooth_scroll_delta.length_sq() > 0.25 || i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. }))
    });
    let now = ui.ctx().input(|i| i.time);
    match if dismiss { None } else { hovered_family } {
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
            ui.label(egui::RichText::new("Frozen").size(tokens.metrics.font_ui_small_pt).color(tokens.text_muted));
        }
    } else if let Some(res) = doc.result_sheet(sheet_id) {
        let (status, status_color) = match &res.status {
            ResultStatus::UpToDate => ("Up-to-date", tokens.text_muted),
            ResultStatus::Dirty => ("Dirty (recalc pending)", tokens.warning),
            ResultStatus::Imported => ("Imported snapshot", tokens.link_blue),
            ResultStatus::ImportedStale => ("Imported (stale)", tokens.stale),
            ResultStatus::Failed { .. } => ("Failed", tokens.error),
        };
        ui.label(egui::RichText::new(format!("Result • {status}")).size(tokens.metrics.font_ui_small_pt).color(status_color));
        if let curvebook_doc::ResultProvenance::Imported { source_format, original_kind_label, .. } = &res.provenance {
            ui.label(
                egui::RichText::new(format!("{source_format} • {original_kind_label}"))
                    .size(tokens.metrics.font_ui_small_pt)
                    .color(tokens.text_muted),
            );
            let note = if matches!(res.status, ResultStatus::ImportedStale) {
                crate::i18n::tr_or_fallback(tokens.locale, "results.imported.stale_suffix", "stale: upstream data changed")
            } else {
                crate::i18n::tr_or_fallback(tokens.locale, "results.imported.not_computed", "not computed by CurveBook")
            };
            ui.label(egui::RichText::new(note).size(tokens.metrics.font_ui_small_pt).color(status_color));
        }
        if res.scalars.is_empty() && !res.tabs.is_empty() {
            ui.label(egui::RichText::new(format!("{} tab(s)", res.tabs.len())).size(tokens.metrics.font_ui_small_pt).color(tokens.text_primary));
        } else {
            for scalar in res.scalars.iter().take(2) {
                ui.label(
                    egui::RichText::new(format!("{}: {:.4}", scalar.key, scalar.value))
                        .size(tokens.metrics.font_ui_small_pt)
                        .color(tokens.text_primary),
                );
            }
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

/// Collect sheet IDs in the exact visible top-to-bottom navigator order (respecting
/// `SeparateFolders` vs `NestedResultsUnderData`, collapsed folders/parents, kind chip, and search).
#[must_use]
pub fn collect_visible_sheet_ids(app: &UiApp, state: &LeadingDockEguiState) -> Vec<SheetId> {
    let q = state.search_draft.trim().to_lowercase();
    let mut ids = Vec::new();
    if app.nav.nesting_mode == NavigatorNestingMode::NestedResultsUnderData {
        let doc = app.document();
        let entry_map: BTreeMap<SheetId, (usize, &NavigatorEntry)> =
            app.nav.folders.iter().flat_map(|f| f.entries.iter().enumerate()).map(|(i, e)| (e.sheet_id.clone(), (i, e))).collect();
        let mut placed = BTreeSet::new();
        if state.kind_chip.matches_section(SheetSection::Data) || state.kind_chip == NavigatorKindChip::All {
            for ds in &doc.data_sheets {
                let Some((_, data_entry)) = entry_map.get(&ds.id) else {
                    continue;
                };
                let children = collect_data_sheet_children(doc, &ds.id, &entry_map);
                let matching_children: Vec<(&NavigatorEntry, usize)> =
                    children.into_iter().filter(|(e, _)| state.kind_chip.matches_section(e.section) && entry_matches_query(e, &q)).collect();
                let parent_matches = state.kind_chip.matches_section(SheetSection::Data) && entry_matches_query(data_entry, &q);
                if !parent_matches && matching_children.is_empty() {
                    continue;
                }
                placed.insert(ds.id.clone());
                ids.push(ds.id.clone());
                for (child, _) in &matching_children {
                    placed.insert(child.sheet_id.clone());
                }
                if !state.nested_collapsed_parents.contains(&ds.id) {
                    for (child, _) in matching_children {
                        ids.push(child.sheet_id.clone());
                    }
                }
            }
        }
        for folder in &app.nav.folders {
            if !state.kind_chip.matches_section(folder.section) {
                continue;
            }
            for entry in &folder.entries {
                if !placed.contains(&entry.sheet_id) && entry_matches_query(entry, &q) {
                    ids.push(entry.sheet_id.clone());
                }
            }
        }
        return ids;
    }
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
        || entry.title.to_lowercase().contains(query_lower)
        || entry.sheet_id.as_str().to_lowercase().contains(query_lower)
        || entry.archetype_label.as_deref().is_some_and(|lbl| lbl.to_lowercase().contains(query_lower))
        || entry.subtitle.as_deref().is_some_and(|sub| sub.to_lowercase().contains(query_lower))
}

fn compute_badge_strip_width(entry: &NavigatorEntry) -> f32 {
    let mut w = 0.0;
    if entry.color_tag.is_some() {
        w += 8.0;
    }
    if entry.origin.is_some() {
        w += 18.0;
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

fn paint_row_badges(painter: &egui::Painter, rect: Rect, center_y: f32, entry: &NavigatorEntry, tokens: &Tokens, _locale: &str) {
    let mut x_right = rect.right() - 4.0;
    if let Some(tag) = entry.color_tag {
        let tag_h = tokens.metrics.space_md + tokens.metrics.space_xxs;
        let r = Rect::from_min_size(Pos2::new(x_right - 4.0, center_y - tag_h * 0.5), Vec2::new(tokens.metrics.space_xs, tag_h));
        painter.rect_filled(r, CornerRadius::same(1), crate::egui_shell::rgba8_to_color32(tag));
        x_right -= 7.0;
    }
    if entry.origin.is_some() {
        let pill = Rect::from_min_size(Pos2::new(x_right - 14.0, rect.center().y - 6.0), Vec2::new(14.0, 12.0)); // design-token-exempt: compact navigator origin badge
        painter.rect_filled(pill, CornerRadius::same(3), tokens.surface_bg);
        painter.rect_stroke(pill, CornerRadius::same(3), Stroke::new(1.0, tokens.border), StrokeKind::Inside);
        let arrow = Stroke::new(1.25, tokens.link_blue);
        let cy = pill.center().y;
        let x0 = pill.left() + 3.0;
        let x1 = pill.right() - 4.5;
        painter.line_segment([Pos2::new(x0, cy), Pos2::new(x1, cy)], arrow);
        painter.line_segment([Pos2::new(x1 - 2.5, cy - 2.5), Pos2::new(x1, cy)], arrow);
        painter.line_segment([Pos2::new(x1 - 2.5, cy + 2.5), Pos2::new(x1, cy)], arrow);
        painter.vline(pill.right() - 2.5, (cy - 3.0)..=(cy + 3.0), Stroke::new(1.1, tokens.link_blue));
        x_right -= 17.0;
    }
    if entry.is_dirty {
        painter.circle_filled(Pos2::new(x_right - 4.0, center_y), 3.0, tokens.link_red);
        x_right -= 11.0;
    }
    if entry.is_frozen {
        painter.text(Pos2::new(x_right, center_y), egui::Align2::RIGHT_CENTER, "F", FontId::proportional(10.0), tokens.text_muted);
        x_right -= 12.0;
    }
    if !entry.link_colors.is_empty() {
        painter.circle_stroke(Pos2::new(x_right - 4.0, center_y), 3.0, Stroke::new(1.2, tokens.link_green));
    }
}

#[inline]
fn snap_px(v: f32, pixels_per_point: f32) -> f32 {
    let ppp = pixels_per_point.max(1.0);
    (v * ppp).round() / ppp
}

fn draw_rail_expand_vector_icon(painter: &egui::Painter, rect: Rect, rtl_mirrored: bool, color: Color32) {
    let ppp = painter.ctx().pixels_per_point();
    let r = rect.shrink(tokens_pad(rect));
    let cy = snap_px(r.center().y, ppp);
    let dy = r.height() * 0.28;
    let tip_off = r.width() * 0.18;
    let stroke = Stroke::new(1.4, color);
    let dir = if rtl_mirrored { -1.0 } else { 1.0 };
    for base_x in [r.center().x - dir * r.width() * 0.22, r.center().x + dir * r.width() * 0.14] {
        let bx = snap_px(base_x, ppp);
        let tip_x = snap_px(base_x + dir * tip_off, ppp);
        painter.add(egui::Shape::line(vec![Pos2::new(bx, cy - dy), Pos2::new(tip_x, cy), Pos2::new(bx, cy + dy)], stroke));
    }
}

fn tokens_pad(rect: Rect) -> f32 {
    (rect.width().min(rect.height()) * 0.12).clamp(1.0, 2.5)
}

fn draw_all_sheets_vector_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let ppp = painter.ctx().pixels_per_point();
    let r = rect.shrink(1.0);
    let gap = snap_px(2.0, ppp);
    let cell_w = ((r.width() - gap) * 0.5).max(2.0);
    let cell_h = ((r.height() - gap) * 0.5).max(2.0);
    let stroke = Stroke::new(1.25, color);
    for row in 0..2 {
        for col in 0..2 {
            let x = snap_px(r.left() + (col as f32) * (cell_w + gap), ppp);
            let y = snap_px(r.top() + (row as f32) * (cell_h + gap), ppp);
            let cell = Rect::from_min_size(Pos2::new(x, y), Vec2::new(cell_w, cell_h));
            painter.rect_stroke(cell, CornerRadius::same(1), stroke, StrokeKind::Inside);
        }
    }
}

fn draw_family_tree_vector_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let ppp = painter.ctx().pixels_per_point();
    let r = rect.shrink(1.5);
    let cy = snap_px(r.center().y, ppp);
    let left_x = snap_px(r.left() + 2.0, ppp);
    let mid_x = snap_px(r.left() + r.width() * 0.52, ppp);
    let right_x = snap_px(r.right() - 2.0, ppp);
    let top_y = snap_px(r.top() + r.height() * 0.22, ppp);
    let bot_y = snap_px(r.bottom() - r.height() * 0.22, ppp);
    let line_stroke = Stroke::new(1.3, color);
    painter.hline(left_x..=mid_x, cy, line_stroke);
    painter.vline(mid_x, top_y..=bot_y, line_stroke);
    painter.hline(mid_x..=right_x, top_y, line_stroke);
    painter.hline(mid_x..=right_x, bot_y, line_stroke);
    let node_r = 1.9_f32;
    painter.circle_filled(Pos2::new(left_x, cy), node_r, color);
    painter.circle_filled(Pos2::new(right_x, top_y), node_r, color);
    painter.circle_filled(Pos2::new(right_x, bot_y), node_r, color);
}

fn draw_sheet_vector_icon(painter: &egui::Painter, rect: Rect, section: SheetSection, _arch_lbl: Option<&str>, color: Color32) {
    let ppp = painter.ctx().pixels_per_point();
    let r = rect.shrink(1.0);
    let stroke = Stroke::new(1.3, color);
    match section {
        SheetSection::Data => {
            // Pixel-snapped 2x2 table grid icon (bold header rule + internal cell dividers).
            let left = snap_px(r.left(), ppp);
            let right = snap_px(r.right(), ppp);
            let top = snap_px(r.top(), ppp);
            let bot = snap_px(r.bottom(), ppp);
            let y_hdr = snap_px(top + (bot - top) * 0.34, ppp);
            let y_mid = snap_px(top + (bot - top) * 0.68, ppp);
            let x_mid = snap_px(left + (right - left) * 0.45, ppp);
            painter.hline(left..=right, top, Stroke::new(1.6, color));
            painter.hline(left..=right, y_hdr, stroke);
            painter.hline(left..=right, y_mid, Stroke::new(1.1, color.linear_multiply(0.8)));
            painter.vline(x_mid, top..=bot, Stroke::new(1.1, color.linear_multiply(0.85)));
        }
        SheetSection::Info => {
            // Crisp info circle + vertical stem + dot.
            let c = Pos2::new(snap_px(r.center().x, ppp), snap_px(r.center().y, ppp));
            let rad = (r.width().min(r.height()) * 0.5).max(3.5);
            painter.circle_stroke(c, rad, stroke);
            painter.circle_filled(Pos2::new(c.x, snap_px(c.y - rad * 0.42, ppp)), 1.05, color);
            painter.vline(c.x, snap_px(c.y - rad * 0.08, ppp)..=snap_px(c.y + rad * 0.48, ppp), stroke);
        }
        SheetSection::Analyses => {
            // Crisp pixel-snapped vector Sigma (Σ) glyph for statistical/regression Results (`R10.36`).
            let left = snap_px(r.left() + r.width() * 0.14, ppp);
            let right = snap_px(r.right() - r.width() * 0.08, ppp);
            let top = snap_px(r.top() + 0.5, ppp);
            let bot = snap_px(r.bottom() - 0.5, ppp);
            let mid_y = snap_px((top + bot) * 0.5, ppp);
            let mid_x = snap_px(left + (right - left) * 0.46, ppp);
            let serif = (r.height() * 0.16).clamp(1.2, 2.2);
            let sigma_stroke = Stroke::new(1.4, color);
            let pts = vec![
                Pos2::new(right, snap_px(top + serif, ppp)),
                Pos2::new(right, top),
                Pos2::new(left, top),
                Pos2::new(mid_x, mid_y),
                Pos2::new(left, bot),
                Pos2::new(right, bot),
                Pos2::new(right, snap_px(bot - serif, ppp)),
            ];
            painter.add(egui::Shape::line(pts, sigma_stroke));
        }
        SheetSection::Graphs => {
            // Pixel-snapped L-axes with a rising sigmoidal curve.
            let ax_x = snap_px(r.left() + 1.0, ppp);
            let ax_bot = snap_px(r.bottom(), ppp);
            let top = snap_px(r.top(), ppp);
            let right = snap_px(r.right(), ppp);
            painter.vline(ax_x, top..=ax_bot, stroke);
            painter.hline(ax_x..=right, ax_bot, stroke);
            let pts = [
                Pos2::new(snap_px(ax_x + 1.5, ppp), snap_px(ax_bot - 1.5, ppp)),
                Pos2::new(snap_px(r.left() + r.width() * 0.42, ppp), snap_px(ax_bot - r.height() * 0.34, ppp)),
                Pos2::new(snap_px(r.left() + r.width() * 0.68, ppp), snap_px(top + r.height() * 0.28, ppp)),
                Pos2::new(snap_px(right - 0.5, ppp), snap_px(top + 1.0, ppp)),
            ];
            painter.add(egui::Shape::line(pts.to_vec(), Stroke::new(1.35, color)));
        }
        SheetSection::Layouts => {
            // Pixel-snapped 2-over-1 publication page mosaic.
            let gap = snap_px(2.0, ppp);
            let left = snap_px(r.left(), ppp);
            let top = snap_px(r.top(), ppp);
            let right = snap_px(r.right(), ppp);
            let bot_edge = snap_px(r.bottom(), ppp);
            let w = (right - left).max(4.0);
            let h = (bot_edge - top).max(4.0);
            let top_h = snap_px((h - gap) * 0.52, ppp);
            let half_w = snap_px((w - gap) * 0.5, ppp);
            let tl = Rect::from_min_size(Pos2::new(left, top), Vec2::new(half_w, top_h));
            let tr =
                Rect::from_min_size(Pos2::new(snap_px(left + half_w + gap, ppp), top), Vec2::new((right - (left + half_w + gap)).max(2.0), top_h));
            let y_bot = snap_px(top + top_h + gap, ppp);
            let bot = Rect::from_min_size(Pos2::new(left, y_bot), Vec2::new(w, (bot_edge - y_bot).max(2.0)));
            painter.rect_stroke(tl, CornerRadius::same(1), stroke, StrokeKind::Inside);
            painter.rect_stroke(tr, CornerRadius::same(1), stroke, StrokeKind::Inside);
            painter.rect_stroke(bot, CornerRadius::same(1), stroke, StrokeKind::Inside);
        }
    }
}

fn render_compact_chip(ui: &mut egui::Ui, spec: CompactChipSpec<'_>, tokens: &Tokens, meta: &mut FrameInspectMeta) -> Response {
    render_compact_chip_height(ui, spec, 20.0, tokens, meta)
}

fn render_compact_chip_height(ui: &mut egui::Ui, spec: CompactChipSpec<'_>, height: f32, tokens: &Tokens, meta: &mut FrameInspectMeta) -> Response {
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
            height,
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
    _app: &UiApp,
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
                let default_w = ctx_in.workspace_preset.default_dock_config().leading_width_pt;
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

#[derive(Clone, Copy)]
enum NavCtxAction {
    CompareSplit,
    Rename,
    Duplicate,
    DuplicateFamily,
    ToggleFreeze,
    ClearOrigin,
    ClearOriginSource,
    Analyze,
    NewGraph,
    ConvertArchetype,
    EditParams,
    Rerun,
    Retarget,
    ChangeGraphType,
    ExportFigure,
    ExportBatchPdf,
    Print,
    Delete,
}

fn push_nav_section_ctx_entries<'a>(
    locale: &'a str,
    menu: &NavigatorContextMenuState,
    frozen_reason: Option<&'a str>,
    entries: &mut Vec<ActionMenuEntry<'a>>,
    actions: &mut Vec<Option<NavCtxAction>>,
) {
    let mut push = |item: ActionMenuItem<'a>, act: NavCtxAction| {
        entries.push(ActionMenuEntry::Item(item));
        actions.push(Some(act));
    };
    match menu.section {
        SheetSection::Data => {
            push(
                ActionMenuItem::menu_item("nav.ctx.analyze", tr(locale, "menu.analyze.analyze")).command_id(Some("ui.analyze.open")),
                NavCtxAction::Analyze,
            );
            push(
                ActionMenuItem::menu_item("nav.ctx.new_graph", tr(locale, "menu.insert.new_graph")).command_id(Some("graph.create")),
                NavCtxAction::NewGraph,
            );
            push(
                ActionMenuItem::menu_item("nav.ctx.convert_archetype", tr(locale, "cmd.ui.table.convertArchetype.open"))
                    .command_id(Some("ui.table.convertArchetype.open"))
                    .enabled(!menu.is_frozen)
                    .disabled_reason(frozen_reason),
                NavCtxAction::ConvertArchetype,
            );
        }
        SheetSection::Analyses => {
            push(
                ActionMenuItem::menu_item("nav.ctx.edit_params", tr(locale, "menu.analyze.change_params")).command_id(Some("ui.editParams.open")),
                NavCtxAction::EditParams,
            );
            push(ActionMenuItem::menu_item("nav.ctx.rerun", tr(locale, "cmd.analysis.run")).command_id(Some("analysis.run")), NavCtxAction::Rerun);
            push(
                ActionMenuItem::menu_item("nav.ctx.retarget", tr(locale, "menu.analyze.change_data")).command_id(Some("analysis.retargetData")),
                NavCtxAction::Retarget,
            );
        }
        SheetSection::Graphs => {
            push(
                ActionMenuItem::menu_item("nav.ctx.change_graph_type", tr(locale, "menu.graph.change_type"))
                    .command_id(Some("ui.changeGraphType.open")),
                NavCtxAction::ChangeGraphType,
            );
            push(
                ActionMenuItem::menu_item("nav.ctx.export_figure", tr(locale, "cmd.ui.export.figure.open")).command_id(Some("ui.export.figure.open")),
                NavCtxAction::ExportFigure,
            );
            push(ActionMenuItem::menu_item("nav.ctx.print", tr(locale, "cmd.ui.print.open")).command_id(Some("ui.print.open")), NavCtxAction::Print);
        }
        SheetSection::Layouts => {
            push(
                ActionMenuItem::menu_item("nav.ctx.export_figure", tr(locale, "cmd.ui.export.figure.open")).command_id(Some("ui.export.figure.open")),
                NavCtxAction::ExportFigure,
            );
            push(ActionMenuItem::menu_item("nav.ctx.print", tr(locale, "cmd.ui.print.open")).command_id(Some("ui.print.open")), NavCtxAction::Print);
        }
        SheetSection::Info => {}
    }
}

pub(super) fn collect_multi_dependents(doc: &ProjectDocument, root_ids: &[SheetId]) -> Vec<(SheetId, String)> {
    let selected_set: BTreeSet<SheetId> = root_ids.iter().cloned().collect();
    let mut deps = Vec::new();
    let mut seen = BTreeSet::new();
    for id in root_ids {
        for dep_id in collect_downstream_dependents(doc, id) {
            if !selected_set.contains(&dep_id) && seen.insert(dep_id.clone()) {
                let dep_title = doc_sheet_title(doc, &dep_id).unwrap_or(dep_id.as_str()).to_owned();
                deps.push((dep_id, dep_title));
            }
        }
    }
    deps
}

pub(super) fn deduplicate_delete_roots(doc: &ProjectDocument, targets: &[SheetId]) -> Vec<SheetId> {
    let mut covered_by_other = BTreeSet::new();
    for id in targets {
        for dep in collect_downstream_dependents(doc, id) {
            covered_by_other.insert(dep);
        }
    }
    let roots: Vec<SheetId> = targets.iter().filter(|id| !covered_by_other.contains(*id)).cloned().collect();
    if roots.is_empty() { targets.to_vec() } else { roots }
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
    let locale = app.locale.as_str();
    let targets: Vec<SheetId> = if menu.selected_ids.len() > 1 {
        menu.selected_ids.iter().filter(|id| doc_sheet_title(doc, id).is_some()).cloned().collect()
    } else if doc_sheet_title(doc, &menu.sheet_id).is_some() {
        vec![menu.sheet_id.clone()]
    } else {
        Vec::new()
    };
    if targets.is_empty() {
        state.context_menu = None;
        return;
    }

    if targets.len() > 1 {
        let count = targets.len();
        let dup_lbl = format!("{} ({count})", tr(locale, "cmd.sheet.duplicate"));
        let data_targets: Vec<SheetId> = targets.iter().filter(|id| doc.data_sheet(id).is_some()).cloned().collect();
        let all_data_frozen = !data_targets.is_empty() && data_targets.iter().all(|id| is_sheet_frozen(doc, id));
        let freeze_cmd = if all_data_frozen { "sheet.unfreeze" } else { "sheet.freeze" };
        let freeze_lbl = tr(locale, if all_data_frozen { "cmd.sheet.unfreeze" } else { "cmd.sheet.freeze" });
        let need_data_reason = data_targets.is_empty().then(|| tr(locale, "nav.disabled_need_data"));
        let renderable_ids: Vec<SheetId> =
            targets.iter().filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some()).cloned().collect();
        let need_export_reason = renderable_ids.is_empty().then(|| tr(locale, "nav.disabled_need_graph_or_layout"));
        let any_frozen = targets.iter().any(|id| is_sheet_frozen(doc, id));
        let frozen_reason = any_frozen.then(|| tr(locale, "nav.frozen_badge"));

        let mut entries: Vec<ActionMenuEntry<'_>> = Vec::new();
        let mut actions: Vec<Option<NavCtxAction>> = Vec::new();

        if targets.len() == 2 {
            entries.push(ActionMenuEntry::Item(
                ActionMenuItem::menu_item("nav.ctx.compare_split", tr(locale, "nav.ctx.compare_split")).command_id(Some("ui.split.toggle")),
            ));
            actions.push(Some(NavCtxAction::CompareSplit));
            entries.push(ActionMenuEntry::Separator);
            actions.push(None);
        }

        entries.push(ActionMenuEntry::Item(ActionMenuItem::menu_item("nav.ctx.duplicate", &dup_lbl).command_id(Some("sheet.duplicate"))));
        actions.push(Some(NavCtxAction::Duplicate));

        entries.push(ActionMenuEntry::Item(
            ActionMenuItem::menu_item("nav.ctx.freeze", freeze_lbl)
                .command_id(Some(freeze_cmd))
                .enabled(!data_targets.is_empty())
                .disabled_reason(need_data_reason),
        ));
        actions.push(Some(NavCtxAction::ToggleFreeze));

        entries.push(ActionMenuEntry::Item(
            ActionMenuItem::menu_item("nav.ctx.export_batch", tr(locale, "btn.export"))
                .command_id(Some(if renderable_ids.len() >= 2 { "export.pdf" } else { "ui.export.figure.open" }))
                .enabled(!renderable_ids.is_empty())
                .disabled_reason(need_export_reason),
        ));
        actions.push(Some(if renderable_ids.len() >= 2 { NavCtxAction::ExportBatchPdf } else { NavCtxAction::ExportFigure }));

        entries.push(ActionMenuEntry::Separator);
        actions.push(None);
        entries.push(ActionMenuEntry::Item(
            ActionMenuItem::menu_item("nav.ctx.delete", tr(locale, "cmd.sheet.delete"))
                .command_id(Some("sheet.delete"))
                .enabled(!any_frozen)
                .disabled_reason(frozen_reason),
        ));
        actions.push(Some(NavCtxAction::Delete));

        let out = action_menu_popup(
            ui.ctx(),
            tokens,
            Some(meta),
            ActionMenuSpec::context(Id::unique(("nav_sheet_ctx_menu", targets[0].as_str())), "popup.nav.ctx", "Sheet Actions", menu.anchor),
            &entries,
        );
        if let Some(hit) = out.clicked_entry
            && let Some(Some(action)) = actions.get(hit)
        {
            match action {
                NavCtxAction::CompareSplit => {
                    intents.push_ui(
                        "ui.split.toggle",
                        json!({ "enabled": true, "primary_sheet_id": targets[0].as_str(), "secondary_sheet_id": targets[1].as_str() }),
                    );
                }
                NavCtxAction::Duplicate => {
                    let cmds: Vec<(String, serde_json::Value)> =
                        targets.iter().map(|id| (String::from("sheet.duplicate"), json!({ "sheet_id": id.as_str() }))).collect();
                    intents.push(UiIntent::EngineCommandGroup { commands: cmds });
                }
                NavCtxAction::ToggleFreeze => {
                    let cmds: Vec<(String, serde_json::Value)> =
                        data_targets.iter().map(|id| (String::from(freeze_cmd), json!({ "sheet_id": id.as_str() }))).collect();
                    if !cmds.is_empty() {
                        intents.push(UiIntent::EngineCommandGroup { commands: cmds });
                    }
                }
                NavCtxAction::ExportFigure => {
                    if let Some(first_id) = renderable_ids.first() {
                        intents.push_ui("ui.export.figure.open", json!({ "sheet_id": first_id.as_str() }));
                    }
                }
                NavCtxAction::ExportBatchPdf => {
                    let sids: Vec<&str> = renderable_ids.iter().map(SheetId::as_str).collect();
                    intents.push_engine("export.pdf", json!({ "sheet_ids": sids }));
                }
                NavCtxAction::Delete => {
                    let roots = deduplicate_delete_roots(doc, &targets);
                    let titles: Vec<String> = targets.iter().map(|id| doc_sheet_title(doc, id).unwrap_or(id.as_str()).to_owned()).collect();
                    let dependents = collect_multi_dependents(doc, &roots);
                    let summary = format!("{} {}", targets.len(), tr(locale, "nav.selected_suffix"));
                    state.delete_confirm =
                        Some(NavigatorDeleteConfirmState { sheet_id: roots[0].clone(), sheet_ids: roots, title: summary, titles, dependents });
                    ui.ctx().request_repaint();
                }
                _ => {}
            }
            state.context_menu = None;
        } else if out.dismissed {
            state.context_menu = None;
        }
        return;
    }

    let Some(title) = doc_sheet_title(doc, &menu.sheet_id).map(str::to_owned) else {
        state.context_menu = None;
        return;
    };

    let frozen_reason = menu.is_frozen.then(|| tr(locale, "nav.frozen_badge"));
    let sid = menu.sheet_id.as_str();
    let freeze_lbl = tr(locale, if menu.is_frozen { "cmd.sheet.unfreeze" } else { "cmd.sheet.freeze" });
    let freeze_cmd = if menu.is_frozen { "sheet.unfreeze" } else { "sheet.freeze" };

    let mut entries = vec![
        ActionMenuEntry::Item(
            ActionMenuItem::menu_item("nav.ctx.rename", tr(locale, "cmd.sheet.rename"))
                .command_id(Some("sheet.rename"))
                .enabled(!menu.is_frozen)
                .disabled_reason(frozen_reason),
        ),
        ActionMenuEntry::Item(ActionMenuItem::menu_item("nav.ctx.duplicate", tr(locale, "cmd.sheet.duplicate")).command_id(Some("sheet.duplicate"))),
        ActionMenuEntry::Item(
            ActionMenuItem::menu_item("nav.ctx.duplicate_family", tr(locale, "cmd.sheet.duplicateFamily")).command_id(Some("sheet.duplicateFamily")),
        ),
        ActionMenuEntry::Item(ActionMenuItem::menu_item("nav.ctx.freeze", freeze_lbl).command_id(Some(freeze_cmd))),
    ];
    let mut actions =
        vec![Some(NavCtxAction::Rename), Some(NavCtxAction::Duplicate), Some(NavCtxAction::DuplicateFamily), Some(NavCtxAction::ToggleFreeze)];

    if menu.origin_source_label.is_some() {
        entries.push(ActionMenuEntry::Separator);
        actions.push(None);
        entries.push(ActionMenuEntry::Item(
            ActionMenuItem::menu_item("nav.ctx.clear_origin", tr(locale, "cmd.sheet.clearOrigin")).command_id(Some("sheet.clearOrigin")),
        ));
        actions.push(Some(NavCtxAction::ClearOrigin));
        entries.push(ActionMenuEntry::Item(
            ActionMenuItem::menu_item("nav.ctx.clear_origin_source", tr(locale, "nav.clear_origin_source")).command_id(Some("sheet.clearOrigin")),
        ));
        actions.push(Some(NavCtxAction::ClearOriginSource));
    }

    if !matches!(menu.section, SheetSection::Info) {
        entries.push(ActionMenuEntry::Separator);
        actions.push(None);
        push_nav_section_ctx_entries(locale, &menu, frozen_reason, &mut entries, &mut actions);
    }

    entries.push(ActionMenuEntry::Separator);
    actions.push(None);
    entries.push(ActionMenuEntry::Item(
        ActionMenuItem::menu_item("nav.ctx.delete", tr(locale, "cmd.sheet.delete"))
            .command_id(Some("sheet.delete"))
            .enabled(!menu.is_frozen)
            .disabled_reason(frozen_reason),
    ));
    actions.push(Some(NavCtxAction::Delete));

    let out = action_menu_popup(
        ui.ctx(),
        tokens,
        Some(meta),
        ActionMenuSpec::context(Id::unique(("nav_sheet_ctx_menu", sid)), "popup.nav.ctx", "Sheet Actions", menu.anchor),
        &entries,
    );

    if let Some(hit) = out.clicked_entry
        && let Some(Some(action)) = actions.get(hit)
    {
        match action {
            NavCtxAction::Rename => begin_inline_rename(app, state, menu.sheet_id.clone()),
            NavCtxAction::Duplicate => intents.push_engine("sheet.duplicate", json!({ "sheet_id": sid })),
            NavCtxAction::DuplicateFamily => intents.push_engine("sheet.duplicateFamily", json!({ "sheet_id": sid })),
            NavCtxAction::ToggleFreeze => intents.push_engine(freeze_cmd, json!({ "sheet_id": sid })),
            NavCtxAction::ClearOrigin => intents.push_engine("sheet.clearOrigin", json!({ "sheet_id": sid })),
            NavCtxAction::ClearOriginSource => {
                intents.push_engine("sheet.clearOrigin", json!({ "sheet_id": sid, "all_from_same_source": true }));
            }
            NavCtxAction::Analyze => intents.push_ui("ui.analyze.open", json!({ "sheet_id": sid })),
            NavCtxAction::NewGraph => intents.push_engine("graph.create", json!({ "data_sheet_id": sid })),
            NavCtxAction::ConvertArchetype => intents.push_ui("ui.table.convertArchetype.open", json!({ "sheet_id": sid })),
            NavCtxAction::EditParams => intents.push_ui("ui.editParams.open", json!({ "sheet_id": sid })),
            NavCtxAction::Rerun => intents.push_engine("analysis.run", json!({ "sheet_id": sid })),
            NavCtxAction::Retarget => {
                intents.push(UiIntent::OpenRetargetDialog { result_sheet_id: menu.sheet_id.clone(), target_data_sheet_id: None });
            }
            NavCtxAction::ChangeGraphType => intents.push_ui("ui.changeGraphType.open", json!({ "sheet_id": sid })),
            NavCtxAction::ExportFigure => intents.push_ui("ui.export.figure.open", json!({ "sheet_id": sid })),
            NavCtxAction::Print => intents.push_ui("ui.print.open", json!({ "sheet_id": sid })),
            NavCtxAction::Delete => {
                let dependents = collect_downstream_dependents(doc, &menu.sheet_id)
                    .into_iter()
                    .map(|dep_id| {
                        let dep_title = doc_sheet_title(doc, &dep_id).unwrap_or(dep_id.as_str()).to_owned();
                        (dep_id, dep_title)
                    })
                    .collect();
                state.delete_confirm = Some(NavigatorDeleteConfirmState {
                    sheet_id: menu.sheet_id.clone(),
                    sheet_ids: vec![menu.sheet_id],
                    title: title.clone(),
                    titles: vec![title],
                    dependents,
                });
                ui.ctx().request_repaint();
            }
            NavCtxAction::CompareSplit | NavCtxAction::ExportBatchPdf => {}
        }
        state.context_menu = None;
    } else if out.dismissed {
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
    let is_multi = confirm.titles.len() > 1;
    let heading = if is_multi { format!("{delete_lbl}: {}", confirm.title) } else { format!("{delete_lbl}: \"{}\"", confirm.title) };
    let modal_w = (ctx.content_rect().width() - 48.0).clamp(240.0, 400.0);

    let modal_resp = egui::Modal::new(Id::unique("nav_delete_confirm_modal")).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(modal_w);
        ui.set_max_width(modal_w);
        ui.heading(&heading);
        ui.add_space(tokens.metrics.space_sm);
        if is_multi {
            ui.add(egui::Label::new(format!("Are you sure you want to delete these {} sheets?", confirm.titles.len())).wrap());
            for item_title in confirm.titles.iter().take(6) {
                ui.add(egui::Label::new(format!("• {item_title}")).wrap());
            }
        } else {
            ui.add(egui::Label::new(format!("Are you sure you want to delete \"{}\"?", confirm.title)).wrap());
        }
        if !confirm.dependents.is_empty() {
            ui.add_space(tokens.metrics.space_sm);
            ui.add(egui::Label::new("The following downstream dependent sheet(s) will also be deleted:").wrap());
            ui.add_space(tokens.metrics.space_xs);
            for (_, dep_title) in &confirm.dependents {
                ui.add(egui::Label::new(format!("• {dep_title}")).wrap());
            }
        } else {
            ui.add_space(tokens.metrics.space_xs);
            ui.add(egui::Label::new(egui::RichText::new("No downstream dependent sheets rely on this selection.").color(tokens.text_muted)).wrap());
        }
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(tokens.metrics.space_sm);
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
                        value: Some(json!({ "sheet_id": confirm.sheet_id.as_str(), "count": confirm.sheet_ids.len(), "cascade": true })),
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
        for id in &confirm.sheet_ids {
            intents.push_engine("sheet.delete", json!({ "sheet_id": id.as_str(), "cascade": true }));
        }
    }
    if close_confirm {
        state.delete_confirm = None;
    }
    capture
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egui_shell::tokens::{Tokens, color32_to_rgba8};
    use crate::theme::UiThemeMode;

    #[test]
    fn r10_36_vector_icons_emit_distinct_snapped_shapes_and_sc1411_contrast() {
        for mode in [UiThemeMode::Light, UiThemeMode::Dark, UiThemeMode::HighContrast] {
            let tokens = Tokens::for_mode(mode);
            let min_icon_cr = color32_to_rgba8(tokens.icon).contrast_ratio(color32_to_rgba8(tokens.surface_bg));
            assert!(min_icon_cr >= 3.0, "{mode:?} icon contrast {min_icon_cr:.2} < 3.0");
            let sel_icon_cr = color32_to_rgba8(tokens.selected_accent_text()).contrast_ratio(color32_to_rgba8(tokens.active_bg));
            assert!(sel_icon_cr >= 3.0, "{mode:?} selected icon contrast {sel_icon_cr:.2} < 3.0");
        }

        let sections = [SheetSection::Data, SheetSection::Info, SheetSection::Analyses, SheetSection::Graphs, SheetSection::Layouts];
        for ppp in [1.0_f32, 1.5_f32] {
            let mut sigs = BTreeSet::new();
            for section in sections {
                let single_ctx = egui::Context::default();
                single_ctx.set_pixels_per_point(ppp);
                let mut out = single_ctx.run_ui(egui::RawInput::default(), |ui| {
                    let rect = Rect::from_min_size(Pos2::new(4.3, 4.3), Vec2::splat(16.0));
                    draw_sheet_vector_icon(ui.painter(), rect, section, None, Tokens::for_mode(UiThemeMode::Dark).selected_accent_text());
                });
                out.textures_delta.clear();
                let desc = format!("{:?}", out.shapes.iter().map(|s| format!("{:?}", s.shape)).collect::<Vec<_>>());
                assert!(sigs.insert(desc.clone()), "section {section:?} at {ppp}x collided with another section's geometry");
                if section == SheetSection::Analyses {
                    // Sigma draws a continuous 7-point polyline path (`Shape::Path`), never 3 separate bars.
                    assert!(desc.contains("Path("), "Analyses sigma should emit a connected Path shape, got: {desc}");
                }
            }
            assert_eq!(sigs.len(), 5);

            // Family tree icon and All-sheets grid icon also render distinct vector primitives.
            for (kind, idx) in [("family_tree", 0usize), ("all_sheets", 1usize)] {
                let single_ctx = egui::Context::default();
                single_ctx.set_pixels_per_point(ppp);
                let mut out = single_ctx.run_ui(egui::RawInput::default(), |ui| {
                    let rect = Rect::from_min_size(Pos2::new(4.3, 4.3), Vec2::splat(16.0));
                    if idx == 0 {
                        draw_family_tree_vector_icon(ui.painter(), rect, Tokens::for_mode(UiThemeMode::Light).icon);
                    } else {
                        draw_all_sheets_vector_icon(ui.painter(), rect, Tokens::for_mode(UiThemeMode::Light).icon);
                    }
                });
                out.textures_delta.clear();
                assert!(!out.shapes.is_empty(), "{kind} emitted no shapes at {ppp}x");
            }
        }
    }

    #[test]
    fn r20_21_navigator_family_multi_source_tooltips_and_actionable_disabled_reasons() {
        use curvebook_doc::{DataSheet, GraphSheet, SubcolumnMode, TableArchetype, XyTable};

        let mut doc = ProjectDocument::new("MultiSrc").expect("doc");
        let ds1 =
            DataSheet::new(SheetId::new("ds-alpha"), "Dose Curve A", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 4).expect("xy1")))
                .expect("ds1");
        let ds2 =
            DataSheet::new(SheetId::new("ds-beta"), "Dose Curve B", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 4).expect("xy2")))
                .expect("ds2");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-multi"), &ds1).expect("gs");
        gs.title = String::from("Combined Dose Graph");
        let mut extra_series = gs.spec.series[0].clone();
        extra_series.source_sheet_id = ds2.id.clone();
        extra_series.legend_label = Some(String::from("Series B"));
        gs.spec.series.push(extra_series);
        doc.data_sheets.push(ds1);
        doc.data_sheets.push(ds2);
        doc.graph_sheets.push(gs.clone());
        doc.active_sheet_id = Some(gs.id.clone());

        let mut app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
        app.execute_command("sheet.select", json!({ "sheet_id": "g-multi" })).expect("select");
        app.family_open = true;
        app.sync_views();
        let tokens = Tokens::default();
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|s| tokens.configure_style(s));
        crate::egui_shell::fonts::install_default_egui_fonts_cached(&ctx);

        let mut state = LeadingDockEguiState::default();

        fn collect_shape_texts(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
            for cs in shapes {
                collect_rec(&cs.shape, out);
            }
        }
        fn collect_rec(s: &egui::epaint::Shape, out: &mut Vec<String>) {
            match s {
                egui::epaint::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                egui::epaint::Shape::Vec(v) => {
                    for c in v {
                        collect_rec(c, out);
                    }
                }
                _ => {}
            }
        }

        let run_pass = |st: &mut LeadingDockEguiState,
                        events: Vec<egui::Event>,
                        time: f64,
                        enabled: bool,
                        rail: bool|
         -> (Vec<String>, FrameInspectMeta, Rect, Rect) {
            let mut n_rect = Rect::NOTHING;
            let mut f_rect = Rect::NOTHING;
            let mut meta = FrameInspectMeta::new();
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 720.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    meta.begin_pass(ui.ctx());
                    let mut intents = PendingUiIntents::new();
                    let layout = tokens.metrics.solve_shell_layout(crate::egui_shell::layout::ShellLayoutInput {
                        viewport_w_pt: 1280.0,
                        viewport_h_pt: 720.0,
                        empty_workspace: false,
                        navigator_open: true,
                        inspector_open: true,
                        family_open: true,
                        leading_user_collapsed: rail,
                        trailing_user_collapsed: false,
                        leading_width_override_pt: None,
                        trailing_width_override_pt: None,
                        host_draws_menu_bar: false,
                        is_rtl: false,
                    });
                    let _ = render_leading_dock(
                        ui,
                        &app,
                        st,
                        LeadingDockContext {
                            tokens: &tokens,
                            layout,
                            workspace_preset: crate::egui_shell::WorkspacePreset::Default,
                            enabled,
                            focused: true,
                        },
                        &mut meta,
                        &mut intents,
                    );
                    if let Some(n) = meta.find_by_widget_id("nav.sheet.g-multi") {
                        n_rect = n.clipped_rect;
                    }
                    if let Some(f) = meta.find_by_widget_id("family.node.g-multi") {
                        f_rect = f.clipped_rect;
                    }
                },
            );
            out.textures_delta.clear();
            let mut texts = Vec::new();
            collect_shape_texts(&out.shapes, &mut texts);
            (texts, meta, n_rect, f_rect)
        };

        // Collapsed rail with modal active (`enabled=false`) must expose actionable disabled reasons, never "unavailable" / "modal active".
        let (_, rail_meta, _, _) = run_pass(&mut state, Vec::new(), 0.0, false, true);
        for entry in &rail_meta.entries {
            if let Some(reason) = entry.disabled_reason.as_deref() {
                assert!(reason != "unavailable" && reason != "modal active", "generic disabled reason `{reason}` found on {}", entry.widget_id);
            }
        }

        // Expanded dock: hover Navigator multi-source graph row & Family node (right side `2 sources` badge area).
        let (_, _, nav_row_rect, fam_row_rect) = run_pass(&mut state, Vec::new(), 0.5, true, false);
        assert!(nav_row_rect.is_positive(), "nav.sheet.g-multi rect missing: {nav_row_rect:?}");
        assert!(fam_row_rect.is_positive(), "family.node.g-multi rect missing: {fam_row_rect:?}");
        let _ = run_pass(&mut state, vec![egui::Event::PointerMoved(nav_row_rect.center())], 1.0, true, false);
        let _ = run_pass(&mut state, Vec::new(), 1.50, true, false);
        let (nav_tips, _, _, _) = run_pass(&mut state, Vec::new(), 1.55, true, false);
        assert!(
            nav_tips.iter().any(|t| t.contains("Dose Curve A") && t.contains("Dose Curve B")),
            "Navigator multi-source tooltip missing upstream tables: {nav_tips:?}"
        );

        let badge_pt = Pos2::new(fam_row_rect.right() - 12.0, fam_row_rect.center().y);
        let _ = run_pass(&mut state, vec![egui::Event::PointerMoved(badge_pt)], 2.0, true, false);
        let _ = run_pass(&mut state, Vec::new(), 2.50, true, false);
        let (fam_tips, _, _, _) = run_pass(&mut state, Vec::new(), 2.55, true, false);
        assert!(
            fam_tips.iter().any(|t| t.contains("Dose Curve A") && t.contains("Dose Curve B")),
            "Family multi-source badge tooltip missing upstream tables: {fam_tips:?}"
        );
    }
}
