//! Native `egui` 3-pane Project Launcher, Table Schema Composer & Clone Pipeline Hub
//! (`docs/planning/gui-layout-spec.md` §8.1, `R15.3`).

use curvebook_doc::{
    Cell, CellValue, ColumnDesignHint, ColumnTable, DataSetColumn, DataSheet, GraphSheet, GroupedTable, InfoSheet, ProjectDocument, ResultSheet,
    SequenceProgression, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, VariableSource, XColumnMode, XyTable,
};
use curvebook_format::save_template_bytes;
use egui::{Align, Color32, CornerRadius, FontId, Id, Key, Layout, Modifiers, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Vec2};
use serde_json::{Value, json};

use crate::UiApp;
use crate::egui_shell::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, UiIntent, doc_sheet_title};
use crate::inspect::ZLayer;
use crate::tr;
use crate::workspace_nav::{SAMPLE_FIXTURES, SampleFixtureDescriptor, instantiate_sample_fixture};

/// Ordered data archetypes selectable inside the 3-pane Welcome & Workspace Hub (`§8.1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WelcomeArchetypeChoice {
    Xy,
    #[default]
    Column,
    Grouped,
    Contingency,
    Survival,
    PartsOfWhole,
    MultipleVariables,
    Nested,
}

impl WelcomeArchetypeChoice {
    /// Ordered list of all eight supported CurveBook table archetypes.
    pub const ALL: [Self; 8] =
        [Self::Xy, Self::Column, Self::Grouped, Self::Contingency, Self::Survival, Self::PartsOfWhole, Self::MultipleVariables, Self::Nested];

    /// Engine command kind string accepted by `sheet.createData`.
    #[must_use]
    pub const fn as_command_kind(self) -> &'static str {
        match self {
            Self::Xy => "xy",
            Self::Column => "column",
            Self::Grouped => "grouped",
            Self::Contingency => "contingency",
            Self::Survival => "survival",
            Self::PartsOfWhole => "parts_of_whole",
            Self::MultipleVariables => "multiple_variables",
            Self::Nested => "nested",
        }
    }

    /// Corresponding [`curvebook_doc::TableKind`] variant.
    #[must_use]
    pub const fn table_kind(self) -> TableKind {
        match self {
            Self::Xy => TableKind::Xy,
            Self::Column => TableKind::Column,
            Self::Grouped => TableKind::Grouped,
            Self::Contingency => TableKind::Contingency,
            Self::Survival => TableKind::Survival,
            Self::PartsOfWhole => TableKind::PartsOfWhole,
            Self::MultipleVariables => TableKind::MultipleVariables,
            Self::Nested => TableKind::Nested,
        }
    }

    /// Localized archetype label from the verified i18n catalog.
    #[must_use]
    pub fn label(self, locale: &str) -> &'static str {
        match self {
            Self::Xy => tr(locale, "table.xy"),
            Self::Column => tr(locale, "table.column"),
            Self::Grouped => tr(locale, "table.grouped"),
            Self::Contingency => tr(locale, "table.contingency"),
            Self::Survival => tr(locale, "table.survival"),
            Self::PartsOfWhole => tr(locale, "table.parts_of_whole"),
            Self::MultipleVariables => tr(locale, "table.multiple_variables"),
            Self::Nested => tr(locale, "table.nested"),
        }
    }

    /// Inspector widget identifier for this archetype button.
    #[must_use]
    pub const fn widget_id(self) -> &'static str {
        match self {
            Self::Xy => "welcome.arch.xy",
            Self::Column => "welcome.arch.column",
            Self::Grouped => "welcome.arch.grouped",
            Self::Contingency => "welcome.arch.contingency",
            Self::Survival => "welcome.arch.survival",
            Self::PartsOfWhole => "welcome.arch.parts_of_whole",
            Self::MultipleVariables => "welcome.arch.multiple_variables",
            Self::Nested => "welcome.arch.nested",
        }
    }
}

/// Active left-rail entry inside the 3-pane Welcome & Workspace Hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WelcomeLeftMode {
    NewTableArchetype(WelcomeArchetypeChoice),
    StarterTemplates,
    ClonePipeline,
    OpenFile,
}

impl Default for WelcomeLeftMode {
    fn default() -> Self {
        Self::NewTableArchetype(WelcomeArchetypeChoice::Column)
    }
}

/// Starter `.cbtemplate` preset rendered inside `WelcomeLeftMode::StarterTemplates`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StarterTemplateSpec {
    id: &'static str,
    title: &'static str,
    archetype: WelcomeArchetypeChoice,
    summary: &'static str,
}

const STARTER_TEMPLATES: [StarterTemplateSpec; 4] = [
    StarterTemplateSpec {
        id: "dose_response_ic50",
        title: "Dose-Response IC50",
        archetype: WelcomeArchetypeChoice::Xy,
        summary: "XY table (triplicate Y) + linked XY graph + dose-response info sheet",
    },
    StarterTemplateSpec {
        id: "paired_treatment_column",
        title: "Paired Before / After",
        archetype: WelcomeArchetypeChoice::Column,
        summary: "Paired Column table + linked Column comparison plot + protocol notes",
    },
    StarterTemplateSpec {
        id: "kaplan_meier_cohort",
        title: "Kaplan-Meier Survival",
        archetype: WelcomeArchetypeChoice::Survival,
        summary: "2-cohort Survival table (1=event, 0=censored) + step curve scaffold",
    },
    StarterTemplateSpec {
        id: "two_way_grouped",
        title: "Two-Way Grouped Assay",
        archetype: WelcomeArchetypeChoice::Grouped,
        summary: "Grouped factorial table (triplicate subcolumns) + grouped bar graph",
    },
];

/// Editable table schema & clone draft maintained locally inside [`WelcomeHubEguiState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeSchemaDraft {
    pub archetype: WelcomeArchetypeChoice,
    pub title: String,
    pub cols: usize,
    pub rows: usize,
    pub subcols: usize,
    pub paired_design: bool,
    pub auto_graph: bool,
    pub auto_info: bool,
    pub entry_mode_sample: bool,
    pub selected_sample_index: usize,
    pub x_mode_kind: usize,
    pub x_title: String,
    pub y_subcolumn_kind: usize,
    pub col_design_kind: usize,
    pub row_factor_title: String,
    pub column_factor_title: String,
    pub survival_time_kind: usize,
    pub survival_time_title: String,
    pub survival_event_code: i64,
    pub survival_censored_code: i64,
    pub mv_role_kinds: Vec<usize>,
    pub selected_template_index: usize,
    pub clone_source_sheet_id: Option<SheetId>,
    pub clone_include_downstream: bool,
    pub clone_graphs: bool,
    pub clone_layouts: bool,
    pub clone_clear_values: bool,
    pub clone_keep_x: bool,
    pub clone_clear_column_titles: bool,
    pub clone_clear_row_titles: bool,
    pub clone_rerun_analyses: bool,
}

impl Default for WelcomeSchemaDraft {
    fn default() -> Self {
        Self {
            archetype: WelcomeArchetypeChoice::Column,
            title: String::new(),
            cols: 3,
            rows: 12,
            subcols: 3,
            paired_design: false,
            auto_graph: true,
            auto_info: true,
            entry_mode_sample: false,
            selected_sample_index: 0,
            x_mode_kind: 0,
            x_title: String::from("X"),
            y_subcolumn_kind: 0,
            col_design_kind: 0,
            row_factor_title: String::from("Row Factor"),
            column_factor_title: String::from("Column Factor"),
            survival_time_kind: 0,
            survival_time_title: String::from("Time"),
            survival_event_code: 1,
            survival_censored_code: 0,
            mv_role_kinds: vec![0, 1, 0, 0, 2, 0],
            selected_template_index: 0,
            clone_source_sheet_id: None,
            clone_include_downstream: true,
            clone_graphs: true,
            clone_layouts: true,
            clone_clear_values: false,
            clone_keep_x: true,
            clone_clear_column_titles: false,
            clone_clear_row_titles: false,
            clone_rerun_analyses: true,
        }
    }
}

/// Persistent `egui_shell` state for the 3-pane Welcome & Workspace Hub.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WelcomeHubEguiState {
    pub left_mode: WelcomeLeftMode,
    pub schema: WelcomeSchemaDraft,
    pub clone_options_open: bool,
    pub open_file_path_draft: String,
    pub open_file_request_count: u64,
    pub template_error: Option<String>,
    /// Inline error from the last failed `Load Sample` attempt (cleared on any sample-mode change).
    pub sample_error: Option<String>,
    /// Inline error from the last failed or policy-rejected `Open Existing Project` attempt.
    pub open_file_error: Option<String>,
    /// Snapshot of `app.mutation_seq()` when `project.open` was dispatched; closes launcher only once it succeeds.
    pub pending_open_mutation_seq: Option<u64>,
}

/// Per-frame layout report returned by [`render_welcome_hub`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WelcomeHubFrameReport {
    pub card_rect_pt: Option<Rect>,
    pub left_rail_rect_pt: Option<Rect>,
    pub middle_pane_rect_pt: Option<Rect>,
    pub right_pane_rect_pt: Option<Rect>,
    pub schematic_rect_pt: Option<Rect>,
    pub clone_modal_rect_pt: Option<Rect>,
    pub cancel_visible: bool,
}

#[derive(Debug, Clone, Copy)]
struct LabelSpec<'a> {
    widget_id: &'a str,
    role: &'a str,
    a11y_role: egui::Role,
    label: &'a str,
    command_id: Option<&'a str>,
    selected: bool,
    enabled: bool,
    disabled_reason: Option<&'a str>,
    z_layer: ZLayer,
}

struct WelcomeMiddleCtx<'a> {
    app: &'a UiApp,
    state: &'a mut WelcomeHubEguiState,
    tokens: &'a Tokens,
    enabled: bool,
    z_layer: ZLayer,
    meta: &'a mut FrameInspectMeta,
    content_w: f32,
}

struct WelcomeFooterCtx<'a> {
    app: &'a UiApp,
    state: &'a mut WelcomeHubEguiState,
    tokens: &'a Tokens,
    enabled: bool,
    cancel_visible: bool,
    z_layer: ZLayer,
    meta: &'a mut FrameInspectMeta,
    intents: &'a mut PendingUiIntents,
}

#[derive(Debug, Clone, Copy)]
struct TextEditSpec<'a> {
    widget_id: &'a str,
    label: &'a str,
    hint: &'a str,
    width: f32,
    enabled: bool,
    z_layer: ZLayer,
}

#[derive(Debug, Clone, Copy)]
struct ArchRailButtonSpec<'a> {
    arch: WelcomeArchetypeChoice,
    width: f32,
    selected: bool,
    enabled: bool,
    locale: &'a str,
    z_layer: ZLayer,
}

#[derive(Debug, Clone, Copy)]
struct ActionButtonSpec<'a> {
    max_w: f32,
    primary: bool,
    enabled: bool,
    label: &'a str,
    widget_id: &'a str,
    command_id: Option<&'a str>,
    disabled_reason: Option<&'a str>,
    z_layer: ZLayer,
}

#[derive(Debug, Clone, Copy)]
struct CheckToggleSpec<'a> {
    widget_id: &'a str,
    label: &'a str,
    checked: bool,
    enabled: bool,
    z_layer: ZLayer,
}

#[derive(Debug, Clone)]
struct StepperSpec<'a, T> {
    widget_id: &'a str,
    label: &'a str,
    value: T,
    range: std::ops::RangeInclusive<T>,
    enabled: bool,
    z_layer: ZLayer,
}

/// Render the centered 3-pane Welcome & Workspace Hub inside `CenterStage` (`§8.1`).
pub fn render_welcome_hub(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut WelcomeHubEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> WelcomeHubFrameReport {
    let mut report = WelcomeHubFrameReport::default();
    let doc = app.document();
    let empty_project = doc.total_sheet_count() == 0;
    let cancel_visible = !empty_project;
    report.cancel_visible = cancel_visible;
    ensure_clone_source_default(app, state);

    if let Some(prev_seq) = state.pending_open_mutation_seq {
        if app.mutation_seq() != prev_seq {
            state.pending_open_mutation_seq = None;
            state.open_file_error = None;
            intents.push(UiIntent::SetLauncherOpen { open: false });
        } else if !ui.ctx().will_discard()
            && let Some(banner) = app.canvas_dialogs.status_banner.as_ref()
        {
            state.open_file_error = Some(banner.clone());
            state.pending_open_mutation_seq = None;
        }
    }

    let stage_rect = ui.max_rect();
    if !stage_rect.is_positive() {
        return report;
    }

    if !empty_project {
        ui.painter().rect_filled(stage_rect, CornerRadius::ZERO, tokens.scrim);
        if enabled && !state.clone_options_open && ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            intents.push(UiIntent::SetLauncherOpen { open: false });
        }
    }

    let pad_x = if stage_rect.width() < 640.0 { 6.0 } else { 16.0 };
    let pad_y = if stage_rect.height() < 360.0 { 6.0 } else { 14.0 };
    let card_w = (stage_rect.width() - pad_x * 2.0).clamp(240.0_f32.min(stage_rect.width()), 880.0);
    let card_h = (stage_rect.height() - pad_y * 2.0).clamp(220.0_f32.min(stage_rect.height()), 540.0);
    let card_rect = Rect::from_center_size(stage_rect.center(), Vec2::new(card_w, card_h));
    report.card_rect_pt = Some(card_rect);

    let card_z = if empty_project { ZLayer::FloatingOverlay } else { ZLayer::ModalDialog };
    let card_enabled = enabled && !state.clone_options_open;

    let mut card_ui = ui.new_child(egui::UiBuilder::new().max_rect(card_rect).layout(Layout::top_down(Align::Min)));
    card_ui.set_clip_rect(card_rect);
    if !card_enabled {
        card_ui.disable();
    }
    card_ui.painter().rect_filled(card_rect, CornerRadius::same(tokens.metrics.radius_lg), tokens.surface_bg);
    card_ui.painter().rect_stroke(card_rect, CornerRadius::same(tokens.metrics.radius_lg), Stroke::new(1.0, tokens.border), StrokeKind::Inside);

    let inner_rect = card_rect.shrink2(Vec2::new(12.0, 10.0));
    let header_h = 36.0;
    let footer_h = 30.0;
    let header_rect = Rect::from_min_size(inner_rect.min, Vec2::new(inner_rect.width(), header_h));
    let footer_rect = Rect::from_min_size(Pos2::new(inner_rect.left(), inner_rect.bottom() - footer_h), Vec2::new(inner_rect.width(), footer_h));
    let body_rect =
        Rect::from_min_max(Pos2::new(inner_rect.left(), header_rect.bottom() + 6.0), Pos2::new(inner_rect.right(), footer_rect.top() - 8.0));

    let mut header_ui = card_ui.new_child(egui::UiBuilder::new().max_rect(header_rect).layout(Layout::top_down(Align::Min)));
    header_ui.set_clip_rect(header_rect);
    render_card_header(&mut header_ui, app, tokens, card_z, meta);

    let body_w = body_rect.width().max(200.0);
    let body_h = body_rect.height().max(80.0);
    let left_w = (body_w * 0.27).clamp(124.0_f32.min(body_w * 0.34), 206.0);
    let mid_w = (body_w * 0.38).clamp(148.0_f32.min(body_w * 0.42), 340.0);
    let right_w = (body_w - left_w - mid_w - 16.0).max(90.0);

    let left_rect = Rect::from_min_size(body_rect.min, Vec2::new(left_w, body_h));
    let mid_rect = Rect::from_min_size(Pos2::new(left_rect.right() + 8.0, body_rect.top()), Vec2::new(mid_w, body_h));
    let right_rect = Rect::from_min_size(Pos2::new(mid_rect.right() + 8.0, body_rect.top()), Vec2::new(right_w, body_h));
    report.left_rail_rect_pt = Some(left_rect);
    report.middle_pane_rect_pt = Some(mid_rect);
    report.right_pane_rect_pt = Some(right_rect);

    let mut left_ui = card_ui.new_child(egui::UiBuilder::new().max_rect(left_rect).layout(Layout::top_down(Align::Min)));
    left_ui.set_clip_rect(left_rect);
    left_ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
    render_left_rail(&mut left_ui, app, state, tokens, card_enabled, card_z, meta);

    card_ui.painter().vline(left_rect.right() + 4.0, body_rect.y_range(), Stroke::new(1.0, tokens.divider));

    let mut mid_ui = card_ui.new_child(egui::UiBuilder::new().max_rect(mid_rect).layout(Layout::top_down(Align::Min)));
    mid_ui.set_clip_rect(mid_rect);
    mid_ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
    render_middle_pane(&mut mid_ui, app, state, tokens, card_enabled, card_z, meta);

    card_ui.painter().vline(mid_rect.right() + 4.0, body_rect.y_range(), Stroke::new(1.0, tokens.divider));

    let mut right_ui = card_ui.new_child(egui::UiBuilder::new().max_rect(right_rect).layout(Layout::top_down(Align::Min)));
    right_ui.set_clip_rect(right_rect);
    right_ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
    report.schematic_rect_pt = render_right_pane(&mut right_ui, app, state, tokens, card_enabled, card_z, meta);

    card_ui.painter().hline(inner_rect.x_range(), footer_rect.top() - 4.0, Stroke::new(1.0, tokens.divider));

    let mut footer_ui = card_ui.new_child(egui::UiBuilder::new().max_rect(footer_rect).layout(Layout::top_down(Align::Min)));
    footer_ui.set_clip_rect(footer_rect);
    let mut footer_cx = WelcomeFooterCtx { app, state, tokens, enabled: card_enabled, cancel_visible, z_layer: card_z, meta, intents };
    render_card_footer(&mut footer_ui, &mut footer_cx);

    if state.clone_options_open {
        report.clone_modal_rect_pt = render_clone_options_submodal(ui, app, state, tokens, enabled, meta);
    }

    report
}

fn ensure_clone_source_default(app: &UiApp, state: &mut WelcomeHubEguiState) {
    let doc = app.document();
    let valid_existing = state.schema.clone_source_sheet_id.as_ref().is_some_and(|id| doc.data_sheet(id).is_some());
    if !valid_existing {
        let active_ds = doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id));
        state.schema.clone_source_sheet_id = active_ds.or_else(|| doc.data_sheets.first()).map(|ds| ds.id.clone());
    }
}

fn render_card_header(ui: &mut egui::Ui, app: &UiApp, tokens: &Tokens, z_layer: ZLayer, meta: &mut FrameInspectMeta) {
    let title_txt = tr(&app.locale, "welcome.title");
    let sub_txt = tr(&app.locale, "welcome.subtitle");
    let avail_w = ui.available_width();
    ui.horizontal_centered(|ui| {
        let (mark_rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
        crate::egui_shell::brand::paint_mark(ui, mark_rect);
        let text_w = (avail_w - 32.0 - ui.spacing().item_spacing.x).max(80.0);
        ui.vertical(|ui| {
            ui.set_max_width(text_w);
            render_clipped_text_label(ui, "welcome.header.title", title_txt, FontId::proportional(15.5), tokens.text_strong, z_layer, meta);
            render_clipped_text_label(ui, "welcome.header.subtitle", sub_txt, FontId::proportional(11.5), tokens.text_muted, z_layer, meta);
        });
    });
}

fn render_left_rail(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut WelcomeHubEguiState,
    tokens: &Tokens,
    enabled: bool,
    z_layer: ZLayer,
    meta: &mut FrameInspectMeta,
) {
    let locale = &app.locale;
    let outer = ui.max_rect();
    ui.painter().rect_filled(outer, CornerRadius::same(6), tokens.panel_bg);
    ui.painter().rect_stroke(outer, CornerRadius::same(6), Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
    let inner = outer.shrink(6.0);
    let mut rail_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    rail_ui.set_clip_rect(inner);
    rail_ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

    let max_w = inner.width().max(72.0);
    egui::ScrollArea::vertical().id_salt("welcome_left_rail_scroll").auto_shrink([false, false]).show(&mut rail_ui, |ui| {
        let row_w = (ui.available_width() - 12.0).clamp(64.0, max_w);
        render_clipped_text_label(
            ui,
            "welcome.rail.archetypes",
            tr(locale, "welcome.rail.archetypes"),
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            tokens.text_muted,
            z_layer,
            meta,
        );
        ui.add_space(4.0);

        for arch in WelcomeArchetypeChoice::ALL {
            let selected = matches!(state.left_mode, WelcomeLeftMode::NewTableArchetype(a) if a == arch);
            if render_arch_rail_button(ui, tokens, meta, ArchRailButtonSpec { arch, width: row_w, selected, enabled, locale, z_layer }) && enabled {
                state.left_mode = WelcomeLeftMode::NewTableArchetype(arch);
                state.schema.archetype = arch;
                state.schema.selected_sample_index = 0;
                state.sample_error = None;
                if matches!(arch, WelcomeArchetypeChoice::PartsOfWhole) {
                    state.schema.cols = 1;
                } else if state.schema.cols < 2 {
                    state.schema.cols = 3;
                }
                if matches!(arch, WelcomeArchetypeChoice::Grouped) && state.schema.y_subcolumn_kind == 0 {
                    state.schema.y_subcolumn_kind = 1;
                }
            }
            ui.add_space(2.0);
        }

        ui.add_space(6.0);
        ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(), Stroke::new(1.0, tokens.divider));
        ui.add_space(6.0);

        render_clipped_text_label(
            ui,
            "welcome.rail.actions",
            tr(locale, "welcome.rail.actions"),
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            tokens.text_muted,
            z_layer,
            meta,
        );
        ui.add_space(4.0);

        let tpl_sel = matches!(state.left_mode, WelcomeLeftMode::StarterTemplates);
        if render_pill_button(
            ui,
            row_w,
            tokens,
            meta,
            LabelSpec {
                widget_id: "welcome.mode.template",
                role: "tab",
                a11y_role: egui::Role::Tab,
                label: tr(locale, "welcome.action.templates"),
                command_id: None,
                selected: tpl_sel,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            state.left_mode = WelcomeLeftMode::StarterTemplates;
        }
        ui.add_space(2.0);

        let has_data = !app.document().data_sheets.is_empty();
        let clone_enabled = enabled && has_data;
        let clone_sel = matches!(state.left_mode, WelcomeLeftMode::ClonePipeline);
        if render_pill_button(
            ui,
            row_w,
            tokens,
            meta,
            LabelSpec {
                widget_id: "welcome.mode.clone",
                role: "tab",
                a11y_role: egui::Role::Tab,
                label: tr(locale, "btn.clone_pipeline"),
                command_id: None,
                selected: clone_sel,
                enabled: clone_enabled,
                disabled_reason: (!has_data).then_some("Create or open a Data table before cloning a pipeline"),
                z_layer,
            },
        ) && clone_enabled
        {
            state.left_mode = WelcomeLeftMode::ClonePipeline;
            ensure_clone_source_default(app, state);
        }
        ui.add_space(2.0);

        let open_sel = matches!(state.left_mode, WelcomeLeftMode::OpenFile);
        if render_pill_button(
            ui,
            row_w,
            tokens,
            meta,
            LabelSpec {
                widget_id: "welcome.action.open_existing",
                role: "tab",
                a11y_role: egui::Role::Tab,
                label: tr(locale, "welcome.action.open_existing"),
                command_id: Some("project.open"),
                selected: open_sel,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            state.left_mode = WelcomeLeftMode::OpenFile;
        }
    });
}

fn render_middle_pane(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut WelcomeHubEguiState,
    tokens: &Tokens,
    enabled: bool,
    z_layer: ZLayer,
    meta: &mut FrameInspectMeta,
) {
    egui::ScrollArea::vertical().id_salt("welcome_mid_pane_scroll").auto_shrink([false, false]).show(ui, |ui| {
        let content_w = (ui.available_width() - 12.0).max(120.0);
        ui.set_max_width(content_w);

        let left_mode = state.left_mode;
        let mut cx = WelcomeMiddleCtx { app, state, tokens, enabled, z_layer, meta, content_w };
        match left_mode {
            WelcomeLeftMode::NewTableArchetype(arch) => {
                render_middle_new_table(ui, &mut cx, arch);
            }
            WelcomeLeftMode::StarterTemplates => {
                render_middle_starter_templates(ui, &mut cx);
            }
            WelcomeLeftMode::ClonePipeline => {
                render_middle_clone_pipeline(ui, &mut cx);
            }
            WelcomeLeftMode::OpenFile => {
                render_middle_open_file(ui, &mut cx);
            }
        }
    });
}

fn render_middle_new_table(ui: &mut egui::Ui, cx: &mut WelcomeMiddleCtx<'_>, arch: WelcomeArchetypeChoice) {
    let locale = cx.app.locale.as_str();
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let z_layer = cx.z_layer;
    let content_w = cx.content_w;

    render_clipped_text_label(
        ui,
        "welcome.section.setup",
        tr(locale, "welcome.section.setup"),
        FontId::proportional(tokens.metrics.font_ui_body_pt),
        tokens.text_strong,
        z_layer,
        cx.meta,
    );
    ui.add_space(4.0);

    render_plain_clipped_label(ui, tr(locale, "welcome.field.title"), FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
    render_clipped_text_edit(
        ui,
        &mut cx.state.schema.title,
        cx.meta,
        TextEditSpec {
            widget_id: "welcome.input.title",
            label: tr(locale, "welcome.field.title"),
            hint: arch.label(locale),
            width: (content_w - 8.0).max(80.0),
            enabled,
            z_layer,
        },
    );
    ui.add_space(6.0);

    let seg_w = ((content_w - 6.0) * 0.5).max(52.0);
    let has_fixture = archetype_has_sample(arch);
    let sample_enabled = enabled && has_fixture;
    let sample_unavailable = tr(locale, "welcome.sample.unavailable");
    if rect_visible_in_clip(ui, Vec2::new(content_w, 24.0)) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if render_pill_button(
                ui,
                seg_w,
                tokens,
                cx.meta,
                LabelSpec {
                    widget_id: "welcome.entry_mode.empty",
                    role: "radio",
                    a11y_role: egui::Role::RadioButton,
                    label: tr(locale, "welcome.mode.empty"),
                    command_id: None,
                    selected: !cx.state.schema.entry_mode_sample,
                    enabled,
                    disabled_reason: None,
                    z_layer,
                },
            ) && enabled
            {
                cx.state.schema.entry_mode_sample = false;
                cx.state.sample_error = None;
            }
            if render_pill_button(
                ui,
                seg_w,
                tokens,
                cx.meta,
                LabelSpec {
                    widget_id: "welcome.entry_mode.sample",
                    role: "radio",
                    a11y_role: egui::Role::RadioButton,
                    label: tr(locale, "welcome.mode.sample"),
                    command_id: None,
                    selected: cx.state.schema.entry_mode_sample,
                    enabled: sample_enabled,
                    disabled_reason: (!has_fixture).then_some(sample_unavailable),
                    z_layer,
                },
            ) && sample_enabled
            {
                cx.state.schema.entry_mode_sample = true;
                cx.state.schema.selected_sample_index = 0;
                cx.state.sample_error = None;
            }
        });
    } else {
        ui.allocate_space(Vec2::new(content_w, 22.0));
    }
    ui.add_space(6.0);

    if cx.state.schema.entry_mode_sample {
        let fixtures = matching_samples_for(arch);
        if fixtures.is_empty() {
            render_plain_clipped_label(ui, sample_unavailable, FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
        } else {
            let sel_idx = cx.state.schema.selected_sample_index.min(fixtures.len() - 1);
            for (idx, fx) in fixtures.iter().enumerate() {
                let wid = format!("welcome.sample.{}", fx.id);
                if render_pill_button(
                    ui,
                    content_w,
                    tokens,
                    cx.meta,
                    LabelSpec {
                        widget_id: &wid,
                        role: "radio",
                        a11y_role: egui::Role::RadioButton,
                        label: fx.title,
                        command_id: None,
                        selected: idx == sel_idx,
                        enabled,
                        disabled_reason: None,
                        z_layer,
                    },
                ) && enabled
                {
                    cx.state.schema.selected_sample_index = idx;
                    cx.state.sample_error = None;
                }
                ui.add_space(2.0);
            }
            if let Some(chosen) = fixtures.get(sel_idx) {
                render_plain_clipped_label(ui, chosen.summary, FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
            }
        }
        if let Some(err_msg) = cx.state.sample_error.as_deref() {
            ui.add_space(4.0);
            let err_resp = ui.add(egui::Label::new(egui::RichText::new(err_msg).small().color(tokens.link_red)).wrap());
            cx.meta.record_response(
                ui,
                &err_resp,
                FrameInspectEntryInput {
                    widget_id: "welcome.sample.error",
                    role: "text",
                    label: err_msg,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer,
                    ellipsized: false,
                    full_label: None,
                },
            );
        }
    } else {
        render_archetype_schema_controls(ui, cx, arch);
    }

    ui.add_space(6.0);
    render_clipped_text_label(
        ui,
        "welcome.section.options",
        tr(locale, "welcome.section.options"),
        FontId::proportional(11.5),
        tokens.text_muted,
        z_layer,
        cx.meta,
    );
    if render_check_toggle(
        ui,
        tokens,
        cx.meta,
        CheckToggleSpec {
            widget_id: "welcome.toggle.auto_graph",
            label: tr(locale, "welcome.toggle.auto_graph"),
            checked: cx.state.schema.auto_graph,
            enabled,
            z_layer,
        },
    ) && enabled
    {
        cx.state.schema.auto_graph = !cx.state.schema.auto_graph;
    }
    if render_check_toggle(
        ui,
        tokens,
        cx.meta,
        CheckToggleSpec {
            widget_id: "welcome.toggle.auto_info",
            label: tr(locale, "welcome.toggle.auto_info"),
            checked: cx.state.schema.auto_info,
            enabled,
            z_layer,
        },
    ) && enabled
    {
        cx.state.schema.auto_info = !cx.state.schema.auto_info;
    }
}

fn render_archetype_schema_controls(ui: &mut egui::Ui, cx: &mut WelcomeMiddleCtx<'_>, arch: WelcomeArchetypeChoice) {
    let locale = cx.app.locale.as_str();
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let z_layer = cx.z_layer;
    let content_w = cx.content_w;

    match arch {
        WelcomeArchetypeChoice::Xy => {
            render_clipped_text_label(
                ui,
                "welcome.section.x_column",
                tr(locale, "welcome.section.x_column"),
                FontId::proportional(11.5),
                tokens.text_muted,
                z_layer,
                cx.meta,
            );
            let x_opts: [(&str, &str); 5] = [
                ("welcome.x_mode.numbers", tr(locale, "welcome.x_mode.numbers")),
                ("welcome.x_mode.numbers_error", "Numbers + X Error"),
                ("welcome.x_mode.dates", tr(locale, "welcome.x_mode.dates")),
                ("welcome.x_mode.elapsed", tr(locale, "welcome.x_mode.elapsed")),
                ("welcome.x_mode.sequence", tr(locale, "welcome.x_mode.sequence")),
            ];
            let chip_w = ((content_w - 6.0) * 0.5).max(56.0);
            for chunk in [0usize..2, 2usize..4, 4usize..5] {
                if rect_visible_in_clip(ui, Vec2::new(content_w, 22.0)) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for idx in chunk {
                            let (wid, lbl) = x_opts[idx];
                            if render_pill_button(
                                ui,
                                chip_w,
                                tokens,
                                cx.meta,
                                LabelSpec {
                                    widget_id: wid,
                                    role: "radio",
                                    a11y_role: egui::Role::RadioButton,
                                    label: lbl,
                                    command_id: None,
                                    selected: cx.state.schema.x_mode_kind == idx,
                                    enabled,
                                    disabled_reason: None,
                                    z_layer,
                                },
                            ) && enabled
                            {
                                cx.state.schema.x_mode_kind = idx;
                            }
                        }
                    });
                } else {
                    ui.allocate_space(Vec2::new(content_w, 22.0));
                }
                ui.add_space(2.0);
            }
            ui.add_space(4.0);
            render_clipped_text_label(
                ui,
                "welcome.section.y_format",
                tr(locale, "welcome.section.y_format"),
                FontId::proportional(11.5),
                tokens.text_muted,
                z_layer,
                cx.meta,
            );
            render_subcolumn_selector(ui, cx);
            render_dimension_steppers(ui, cx, tr(locale, "welcome.field.datasets"), tr(locale, "welcome.field.rows"), 1..=64);
        }
        WelcomeArchetypeChoice::Column => {
            render_clipped_text_label(
                ui,
                "welcome.section.col_design",
                tr(locale, "welcome.section.col_design"),
                FontId::proportional(11.5),
                tokens.text_muted,
                z_layer,
                cx.meta,
            );
            let col_opts: [(&str, &str); 3] = [
                ("welcome.design.unpaired", tr(locale, "welcome.design.unpaired")),
                ("welcome.design.paired", tr(locale, "welcome.design.paired")),
                ("welcome.design.precomputed", tr(locale, "welcome.design.precomputed")),
            ];
            for (idx, (wid, lbl)) in col_opts.into_iter().enumerate() {
                if render_pill_button(
                    ui,
                    content_w,
                    tokens,
                    cx.meta,
                    LabelSpec {
                        widget_id: wid,
                        role: "radio",
                        a11y_role: egui::Role::RadioButton,
                        label: lbl,
                        command_id: None,
                        selected: cx.state.schema.col_design_kind == idx,
                        enabled,
                        disabled_reason: None,
                        z_layer,
                    },
                ) && enabled
                {
                    cx.state.schema.col_design_kind = idx;
                    cx.state.schema.paired_design = idx == 1;
                    if idx == 2 {
                        cx.state.schema.y_subcolumn_kind = normalize_summary_subcolumn_kind(cx.state.schema.y_subcolumn_kind);
                    }
                }
                ui.add_space(2.0);
            }
            if cx.state.schema.col_design_kind == 2 {
                ui.add_space(4.0);
                render_summary_subcolumn_selector(ui, cx);
            }
            ui.add_space(4.0);
            render_dimension_steppers(ui, cx, tr(locale, "welcome.field.datasets"), tr(locale, "welcome.field.rows"), 1..=64);
        }
        WelcomeArchetypeChoice::Grouped => {
            if cx.state.schema.y_subcolumn_kind == 0 {
                cx.state.schema.y_subcolumn_kind = 1;
            }
            render_clipped_text_label(
                ui,
                "welcome.section.y_format",
                tr(locale, "welcome.section.y_format"),
                FontId::proportional(11.5),
                tokens.text_muted,
                z_layer,
                cx.meta,
            );
            render_subcolumn_selector(ui, cx);
            ui.add_space(4.0);
            render_dimension_steppers(ui, cx, tr(locale, "welcome.field.datasets"), tr(locale, "welcome.field.rows"), 1..=64);
        }
        WelcomeArchetypeChoice::Contingency => {
            render_plain_clipped_label(
                ui,
                "Enter integer frequency counts per cross-tabulated cell.",
                FontId::proportional(tokens.metrics.font_ui_small_pt),
                tokens.text_muted,
            );
            ui.add_space(4.0);
            render_dimension_steppers(ui, cx, "Columns", "Rows", 2..=32);
        }
        WelcomeArchetypeChoice::Survival => {
            let t_opts = [
                ("welcome.survival.time_elapsed", tr(locale, "welcome.x_mode.elapsed")),
                ("welcome.survival.time_date", tr(locale, "welcome.x_mode.dates")),
            ];
            for (idx, (wid, lbl)) in t_opts.into_iter().enumerate() {
                if render_pill_button(
                    ui,
                    content_w,
                    tokens,
                    cx.meta,
                    LabelSpec {
                        widget_id: wid,
                        role: "radio",
                        a11y_role: egui::Role::RadioButton,
                        label: lbl,
                        command_id: None,
                        selected: cx.state.schema.survival_time_kind == idx,
                        enabled,
                        disabled_reason: None,
                        z_layer,
                    },
                ) && enabled
                {
                    cx.state.schema.survival_time_kind = idx;
                }
                ui.add_space(2.0);
            }
            ui.add_space(4.0);
            cx.state.schema.survival_event_code = render_i64_stepper(
                ui,
                tokens,
                cx.meta,
                StepperSpec {
                    widget_id: "welcome.survival.event_code",
                    label: "Event code",
                    value: cx.state.schema.survival_event_code,
                    range: -99..=99,
                    enabled,
                    z_layer,
                },
            );
            cx.state.schema.survival_censored_code = render_i64_stepper(
                ui,
                tokens,
                cx.meta,
                StepperSpec {
                    widget_id: "welcome.survival.censored_code",
                    label: "Censored code",
                    value: cx.state.schema.survival_censored_code,
                    range: -99..=99,
                    enabled,
                    z_layer,
                },
            );
            if cx.state.schema.survival_event_code == cx.state.schema.survival_censored_code {
                cx.state.schema.survival_censored_code = i64::from(cx.state.schema.survival_event_code == 0);
            }
            render_dimension_steppers(ui, cx, "Cohorts", tr(locale, "welcome.field.rows"), 1..=32);
        }
        WelcomeArchetypeChoice::PartsOfWhole => {
            render_plain_clipped_label(
                ui,
                "One slice or proportion per category row.",
                FontId::proportional(tokens.metrics.font_ui_small_pt),
                tokens.text_muted,
            );
            render_plain_clipped_label(
                ui,
                "Analysis computes proportions against column sums automatically.",
                FontId::proportional(tokens.metrics.font_ui_small_pt),
                tokens.text_muted,
            );
            render_dimension_steppers(ui, cx, "Series", "Categories", 1..=16);
        }
        WelcomeArchetypeChoice::MultipleVariables => {
            render_dimension_steppers(ui, cx, "Variables", "Observations", 1..=32);
            ui.add_space(4.0);
            render_clipped_text_label(
                ui,
                "welcome.mv.roles_heading",
                "Initial variable roles",
                FontId::proportional(11.5),
                tokens.text_muted,
                z_layer,
                cx.meta,
            );
            let shown_vars = cx.state.schema.cols.clamp(1, 32);
            if cx.state.schema.mv_role_kinds.len() < shown_vars {
                cx.state.schema.mv_role_kinds.resize(shown_vars, 0);
            }
            let roles = [("Cont", "Continuous"), ("Cat", "Categorical"), ("Lbl", "Label")];
            let chip_w = ((content_w - 58.0) / 3.0).clamp(32.0, 68.0);
            for v_idx in 0..shown_vars {
                if rect_visible_in_clip(ui, Vec2::new(content_w, 24.0)) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        render_plain_clipped_label_w(
                            ui,
                            &format!("Var {}", v_idx + 1),
                            40.0,
                            FontId::proportional(tokens.metrics.font_ui_small_pt),
                            tokens.text_primary,
                        );
                        for (r_idx, (short, full)) in roles.iter().enumerate() {
                            let wid = format!("welcome.mv.role.{v_idx}.{full}");
                            if render_pill_button(
                                ui,
                                chip_w,
                                tokens,
                                cx.meta,
                                LabelSpec {
                                    widget_id: &wid,
                                    role: "radio",
                                    a11y_role: egui::Role::RadioButton,
                                    label: short,
                                    command_id: None,
                                    selected: cx.state.schema.mv_role_kinds[v_idx] == r_idx,
                                    enabled,
                                    disabled_reason: None,
                                    z_layer,
                                },
                            ) && enabled
                            {
                                cx.state.schema.mv_role_kinds[v_idx] = r_idx;
                            }
                        }
                    });
                } else {
                    ui.allocate_space(Vec2::new(content_w, 22.0));
                }
                ui.add_space(2.0);
            }
        }
        WelcomeArchetypeChoice::Nested => {
            render_plain_clipped_label(
                ui,
                "Groups contain biological-unit subcolumns with technical rows.",
                FontId::proportional(tokens.metrics.font_ui_small_pt),
                tokens.text_muted,
            );
            cx.state.schema.subcols = render_usize_stepper(
                ui,
                tokens,
                cx.meta,
                StepperSpec {
                    widget_id: "welcome.nested.depth",
                    label: "Subcolumn units",
                    value: cx.state.schema.subcols.max(2),
                    range: 2..=64,
                    enabled,
                    z_layer,
                },
            );
            render_dimension_steppers(ui, cx, "Groups", "Technical rows", 2..=32);
        }
    }
}

fn render_subcolumn_selector(ui: &mut egui::Ui, cx: &mut WelcomeMiddleCtx<'_>) {
    let locale = cx.app.locale.as_str();
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let z_layer = cx.z_layer;
    let content_w = cx.content_w;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;

    let sub_opts: [(&str, &str); 8] = [
        ("welcome.subcol.single", tr(locale, "welcome.subcol.single")),
        ("welcome.subcol.raw", tr(locale, "welcome.subcol.raw")),
        ("welcome.subcol.mean_sd_n", tr(locale, "welcome.subcol.mean_sd_n")),
        ("welcome.subcol.mean_sem_n", tr(locale, "welcome.subcol.mean_sem_n")),
        ("welcome.subcol.mean_cv_n", tr(locale, "welcome.subcol.mean_cv_n")),
        ("welcome.subcol.mean_low_high", tr(locale, "welcome.subcol.mean_low_high")),
        ("welcome.subcol.upper_lower", tr(locale, "welcome.subcol.upper_lower")),
        ("welcome.subcol.median_iqr", tr(locale, "welcome.subcol.median_iqr")),
    ];
    let chip_w = ((content_w - 6.0) * 0.5).max(56.0);
    for pair in sub_opts.chunks(2) {
        if rect_visible_in_clip(ui, Vec2::new(content_w, 22.0)) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for &(wid, lbl) in pair {
                    let idx = sub_opts.iter().position(|&(w, _)| w == wid).unwrap_or(0);
                    if render_pill_button(
                        ui,
                        chip_w,
                        tokens,
                        meta,
                        LabelSpec {
                            widget_id: wid,
                            role: "radio",
                            a11y_role: egui::Role::RadioButton,
                            label: lbl,
                            command_id: None,
                            selected: state.schema.y_subcolumn_kind == idx,
                            enabled,
                            disabled_reason: None,
                            z_layer,
                        },
                    ) && enabled
                    {
                        state.schema.y_subcolumn_kind = idx;
                    }
                }
            });
        } else {
            ui.allocate_space(Vec2::new(content_w, 22.0));
        }
        ui.add_space(2.0);
    }
    if state.schema.y_subcolumn_kind == 1 {
        state.schema.subcols = render_usize_stepper(
            ui,
            tokens,
            meta,
            StepperSpec {
                widget_id: "welcome.stepper.replicates",
                label: tr(locale, "welcome.field.replicates"),
                value: state.schema.subcols.max(2),
                range: 1..=256,
                enabled,
                z_layer,
            },
        );
    }
}

const fn normalize_summary_subcolumn_kind(kind: usize) -> usize {
    match kind {
        2 | 3 | 7 => kind,
        _ => 2,
    }
}

fn render_summary_subcolumn_selector(ui: &mut egui::Ui, cx: &mut WelcomeMiddleCtx<'_>) {
    let locale = cx.app.locale.as_str();
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let z_layer = cx.z_layer;
    let content_w = cx.content_w;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    state.schema.y_subcolumn_kind = normalize_summary_subcolumn_kind(state.schema.y_subcolumn_kind);

    let opts = [
        (2usize, "welcome.subcol.mean_sd_n", tr(locale, "welcome.subcol.mean_sd_n")),
        (3usize, "welcome.subcol.mean_sem_n", tr(locale, "welcome.subcol.mean_sem_n")),
        (7usize, "welcome.subcol.median_iqr", tr(locale, "welcome.subcol.median_iqr")),
    ];
    for (idx, wid, lbl) in opts {
        if render_pill_button(
            ui,
            content_w,
            tokens,
            meta,
            LabelSpec {
                widget_id: wid,
                role: "radio",
                a11y_role: egui::Role::RadioButton,
                label: lbl,
                command_id: None,
                selected: state.schema.y_subcolumn_kind == idx,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            state.schema.y_subcolumn_kind = idx;
        }
        ui.add_space(2.0);
    }
}

fn render_dimension_steppers(
    ui: &mut egui::Ui,
    cx: &mut WelcomeMiddleCtx<'_>,
    cols_label: &str,
    rows_label: &str,
    col_range: std::ops::RangeInclusive<usize>,
) {
    let locale = cx.app.locale.as_str();
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let z_layer = cx.z_layer;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;

    render_clipped_text_label(
        ui,
        "welcome.section.dimensions",
        tr(locale, "welcome.section.dimensions"),
        FontId::proportional(11.5),
        tokens.text_muted,
        z_layer,
        meta,
    );
    state.schema.cols = render_usize_stepper(
        ui,
        tokens,
        meta,
        StepperSpec { widget_id: "welcome.stepper.cols", label: cols_label, value: state.schema.cols, range: col_range, enabled, z_layer },
    );
    state.schema.rows = render_usize_stepper(
        ui,
        tokens,
        meta,
        StepperSpec { widget_id: "welcome.stepper.rows", label: rows_label, value: state.schema.rows, range: 1..=10_000, enabled, z_layer },
    );
}

fn render_middle_starter_templates(ui: &mut egui::Ui, cx: &mut WelcomeMiddleCtx<'_>) {
    let locale = cx.app.locale.as_str();
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let z_layer = cx.z_layer;
    let content_w = cx.content_w;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;

    render_clipped_text_label(
        ui,
        "welcome.template.heading",
        tr(locale, "welcome.template.heading"),
        FontId::proportional(tokens.metrics.font_ui_body_pt),
        tokens.text_strong,
        z_layer,
        meta,
    );
    ui.add_space(4.0);
    let sel = state.schema.selected_template_index.min(STARTER_TEMPLATES.len() - 1);
    for (idx, tpl) in STARTER_TEMPLATES.iter().enumerate() {
        let wid = format!("welcome.template.{idx}");
        if render_pill_button(
            ui,
            content_w,
            tokens,
            meta,
            LabelSpec {
                widget_id: &wid,
                role: "radio",
                a11y_role: egui::Role::RadioButton,
                label: tpl.title,
                command_id: None,
                selected: idx == sel,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            state.schema.selected_template_index = idx;
            state.schema.archetype = tpl.archetype;
            state.template_error = None;
        }
        ui.add_space(2.0);
    }
    if let Some(chosen) = STARTER_TEMPLATES.get(sel) {
        ui.add_space(4.0);
        render_plain_clipped_label(ui, chosen.summary, FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
    }
    if let Some(ref err_msg) = state.template_error {
        ui.add_space(4.0);
        let err_resp = ui.add(egui::Label::new(egui::RichText::new(err_msg).small().color(tokens.link_red)).wrap());
        meta.record_response(
            ui,
            &err_resp,
            FrameInspectEntryInput {
                widget_id: "welcome.template.error",
                role: "text",
                label: err_msg,
                command_id: None,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer,
                ellipsized: false,
                full_label: None,
            },
        );
    }
    ui.add_space(6.0);
    render_plain_clipped_label(ui, tr(locale, "welcome.field.title"), FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
    render_clipped_text_edit(
        ui,
        &mut state.schema.title,
        meta,
        TextEditSpec {
            widget_id: "welcome.input.title",
            label: tr(locale, "welcome.field.title"),
            hint: "",
            width: (content_w - 8.0).max(80.0),
            enabled,
            z_layer,
        },
    );
}

fn render_middle_clone_pipeline(ui: &mut egui::Ui, cx: &mut WelcomeMiddleCtx<'_>) {
    let app = cx.app;
    let locale = &app.locale;
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let z_layer = cx.z_layer;
    let content_w = cx.content_w;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;

    render_clipped_text_label(
        ui,
        "welcome.clone.heading",
        tr(locale, "welcome.clone.heading"),
        FontId::proportional(tokens.metrics.font_ui_body_pt),
        tokens.text_strong,
        z_layer,
        meta,
    );
    ui.add_space(4.0);

    render_plain_clipped_label(ui, tr(locale, "welcome.clone.source"), FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
    for ds in &app.document().data_sheets {
        let selected = state.schema.clone_source_sheet_id.as_ref() == Some(&ds.id);
        let wid = format!("welcome.clone.source.{}", ds.id.as_str());
        let title = doc_sheet_title(app.document(), &ds.id).unwrap_or(ds.id.as_str()).to_owned();
        if render_pill_button(
            ui,
            content_w,
            tokens,
            meta,
            LabelSpec {
                widget_id: &wid,
                role: "radio",
                a11y_role: egui::Role::RadioButton,
                label: &title,
                command_id: None,
                selected,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            state.schema.clone_source_sheet_id = Some(ds.id.clone());
        }
        ui.add_space(2.0);
    }

    ui.add_space(6.0);
    render_plain_clipped_label(ui, tr(locale, "welcome.field.title"), FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
    render_clipped_text_edit(
        ui,
        &mut state.schema.title,
        meta,
        TextEditSpec {
            widget_id: "welcome.input.title",
            label: tr(locale, "welcome.field.title"),
            hint: "Cloned Run",
            width: (content_w - 8.0).max(80.0),
            enabled,
            z_layer,
        },
    );

    ui.add_space(6.0);
    let summary = format!(
        "Links: {} • Data: {} • Analyses: {}",
        if state.schema.clone_include_downstream { "Preserve" } else { "Table only" },
        if state.schema.clone_clear_values { "Clear" } else { "Copy" },
        if state.schema.clone_rerun_analyses { "Re-run" } else { "Deferred" },
    );
    render_plain_clipped_label(ui, &summary, FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
    ui.add_space(4.0);

    if render_pill_button(
        ui,
        content_w.min(180.0),
        tokens,
        meta,
        LabelSpec {
            widget_id: "welcome.btn.clone_options",
            role: "button",
            a11y_role: egui::Role::Button,
            label: "Clone Options…",
            command_id: None,
            selected: state.clone_options_open,
            enabled,
            disabled_reason: None,
            z_layer,
        },
    ) && enabled
    {
        state.clone_options_open = true;
    }
}

fn render_middle_open_file(ui: &mut egui::Ui, cx: &mut WelcomeMiddleCtx<'_>) {
    let locale = cx.app.locale.as_str();
    let tokens = cx.tokens;
    let external_rejected = matches!(cx.app.session().path_policy(), curvebook_engine::ExternalPathPolicy::RejectExternal);
    let enabled = cx.enabled && !external_rejected;
    let z_layer = cx.z_layer;
    let content_w = cx.content_w;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;

    render_clipped_text_label(
        ui,
        "welcome.open.heading",
        tr(locale, "menu.file.open"),
        FontId::proportional(tokens.metrics.font_ui_body_pt),
        tokens.text_strong,
        z_layer,
        meta,
    );
    ui.add_space(4.0);
    render_clipped_text_label(
        ui,
        "welcome.open.subtitle",
        tr(locale, "welcome.open.subtitle"),
        FontId::proportional(tokens.metrics.font_ui_small_pt),
        tokens.text_muted,
        z_layer,
        meta,
    );
    ui.add_space(6.0);
    let prev_draft = state.open_file_path_draft.clone();
    render_clipped_text_edit(
        ui,
        &mut state.open_file_path_draft,
        meta,
        TextEditSpec {
            widget_id: "welcome.input.open_path",
            label: tr(locale, "welcome.open.path_label"),
            hint: tr(locale, "welcome.open.path_placeholder"),
            width: (content_w - 8.0).max(80.0),
            enabled,
            z_layer,
        },
    );
    if state.open_file_path_draft != prev_draft {
        state.open_file_error = None;
    }
    if external_rejected {
        ui.add_space(4.0);
        let notice = "External file access is disabled by host security policy.";
        let resp = ui.add(egui::Label::new(egui::RichText::new(notice).small().color(tokens.text_muted)).wrap());
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: "welcome.open.policy_notice",
                role: "text",
                label: notice,
                command_id: None,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer,
                ellipsized: false,
                full_label: None,
            },
        );
    }
    if let Some(err_msg) = state.open_file_error.as_deref() {
        ui.add_space(4.0);
        let err_resp = ui.add(egui::Label::new(egui::RichText::new(err_msg).small().color(tokens.link_red)).wrap());
        meta.record_response(
            ui,
            &err_resp,
            FrameInspectEntryInput {
                widget_id: "welcome.open.error",
                role: "text",
                label: err_msg,
                command_id: None,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer,
                ellipsized: false,
                full_label: None,
            },
        );
    }
}

fn render_right_pane(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut WelcomeHubEguiState,
    tokens: &Tokens,
    _enabled: bool,
    z_layer: ZLayer,
    meta: &mut FrameInspectMeta,
) -> Option<Rect> {
    let locale = &app.locale;
    let mut schematic_rect = None;
    egui::ScrollArea::vertical().id_salt("welcome_right_pane_scroll").auto_shrink([false, false]).show(ui, |ui| {
        let preview_w = (ui.available_width() - 12.0).max(96.0);
        render_clipped_text_label(
            ui,
            "welcome.section.preview",
            tr(locale, "welcome.section.preview"),
            FontId::proportional(tokens.metrics.font_ui_body_pt),
            tokens.text_strong,
            z_layer,
            meta,
        );
        ui.add_space(4.0);

        let arch = active_preview_archetype(app, state);
        let box_h = (ui.available_height() - 54.0).clamp(84.0, 176.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(preview_w, box_h), Sense::hover());
        schematic_rect = Some(rect);

        ui.painter().rect_filled(rect, CornerRadius::same(6), tokens.paper_white);
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, tokens.border), StrokeKind::Inside);

        let split_x = rect.left() + rect.width() * 0.56;
        let table_rect = Rect::from_min_max(rect.min + Vec2::splat(6.0), Pos2::new(split_x - 3.0, rect.bottom() - 6.0));
        let plot_rect = Rect::from_min_max(Pos2::new(split_x + 3.0, rect.top() + 6.0), rect.max - Vec2::splat(6.0));
        draw_vector_table_schematic(ui.painter(), table_rect, arch, &state.schema, tokens);
        draw_vector_plot_preview(ui.painter(), plot_rect, arch, &state.schema, tokens);

        ui.add_space(6.0);
        let pipeline = match state.left_mode {
            WelcomeLeftMode::ClonePipeline => {
                if state.schema.clone_include_downstream {
                    "Data -> Analyses -> Graphs -> Layouts"
                } else {
                    "Data only"
                }
            }
            WelcomeLeftMode::StarterTemplates => "Data -> Info -> Linked Graph",
            WelcomeLeftMode::OpenFile => tr(locale, "welcome.open.pipeline_label"),
            WelcomeLeftMode::NewTableArchetype(_) => match (state.schema.auto_info, state.schema.auto_graph) {
                (true, true) => "Data + Info + Graph",
                (true, false) => "Data + Info",
                (false, true) => "Data + Graph",
                (false, false) => "Data only",
            },
        };
        render_clipped_text_label(
            ui,
            "welcome.preview.pipeline",
            pipeline,
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            tokens.text_muted,
            z_layer,
            meta,
        );
    });
    schematic_rect
}

fn active_preview_archetype(app: &UiApp, state: &WelcomeHubEguiState) -> WelcomeArchetypeChoice {
    match state.left_mode {
        WelcomeLeftMode::NewTableArchetype(a) => a,
        WelcomeLeftMode::StarterTemplates => {
            STARTER_TEMPLATES.get(state.schema.selected_template_index).map_or(WelcomeArchetypeChoice::Xy, |t| t.archetype)
        }
        WelcomeLeftMode::ClonePipeline => {
            state.schema.clone_source_sheet_id.as_ref().and_then(|id| app.document().data_sheet(id)).map_or(state.schema.archetype, |ds| {
                match ds.archetype.kind() {
                    TableKind::Xy => WelcomeArchetypeChoice::Xy,
                    TableKind::Column => WelcomeArchetypeChoice::Column,
                    TableKind::Grouped => WelcomeArchetypeChoice::Grouped,
                    TableKind::Contingency => WelcomeArchetypeChoice::Contingency,
                    TableKind::Survival => WelcomeArchetypeChoice::Survival,
                    TableKind::PartsOfWhole => WelcomeArchetypeChoice::PartsOfWhole,
                    TableKind::MultipleVariables => WelcomeArchetypeChoice::MultipleVariables,
                    TableKind::Nested => WelcomeArchetypeChoice::Nested,
                }
            })
        }
        WelcomeLeftMode::OpenFile => state.schema.archetype,
    }
}

fn render_card_footer(ui: &mut egui::Ui, cx: &mut WelcomeFooterCtx<'_>) {
    let app = cx.app;
    let locale = &app.locale;
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let cancel_visible = cx.cancel_visible;
    let z_layer = cx.z_layer;
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;

    ui.horizontal(|ui| {
        if cancel_visible
            && render_action_button(
                ui,
                tokens,
                meta,
                ActionButtonSpec {
                    max_w: 92.0,
                    primary: false,
                    enabled,
                    label: tr(locale, "btn.cancel"),
                    widget_id: "welcome.action.cancel",
                    command_id: None,
                    disabled_reason: None,
                    z_layer,
                },
            )
            && enabled
        {
            intents.push(UiIntent::SetLauncherOpen { open: false });
        }

        let (cta_id, cta_label, cta_cmd, cta_enabled, disabled_reason) = match state.left_mode {
            WelcomeLeftMode::NewTableArchetype(arch) => {
                if state.schema.entry_mode_sample {
                    let has_fixture = archetype_has_sample(arch);
                    (
                        "welcome.btn.create",
                        tr(locale, "welcome.btn.load_sample"),
                        Some("sheet.createData"),
                        enabled && has_fixture,
                        (!has_fixture).then(|| tr(locale, "welcome.sample.unavailable")),
                    )
                } else {
                    ("welcome.btn.create", tr(locale, "welcome.btn.create_table"), Some("sheet.createData"), enabled, None)
                }
            }
            WelcomeLeftMode::StarterTemplates => {
                ("welcome.btn.create", tr(locale, "welcome.btn.instantiate_template"), Some("project.instantiateTemplate"), enabled, None)
            }
            WelcomeLeftMode::ClonePipeline => {
                let has_src = state.schema.clone_source_sheet_id.is_some();
                (
                    "welcome.btn.create",
                    tr(locale, "welcome.btn.clone_pipeline"),
                    Some("sheet.clonePipeline"),
                    enabled && has_src,
                    (!has_src).then_some("Select a Data table to clone"),
                )
            }
            WelcomeLeftMode::OpenFile => {
                let external_rejected = matches!(app.session().path_policy(), curvebook_engine::ExternalPathPolicy::RejectExternal);
                (
                    "welcome.btn.open_file",
                    tr(locale, "welcome.action.open_existing"),
                    Some("project.open"),
                    enabled && !external_rejected,
                    external_rejected.then_some("External file access is disabled by host security policy"),
                )
            }
        };

        let btn_w = ui.available_width().clamp(72.0_f32.min(ui.available_width().max(48.0)), 196.0);
        if ui.available_width() > btn_w {
            ui.add_space(ui.available_width() - btn_w);
        }
        if render_action_button(
            ui,
            tokens,
            meta,
            ActionButtonSpec {
                max_w: btn_w,
                primary: true,
                enabled: cta_enabled,
                label: cta_label,
                widget_id: cta_id,
                command_id: cta_cmd,
                disabled_reason,
                z_layer,
            },
        ) && cta_enabled
        {
            dispatch_welcome_primary_action(app, state, intents);
        }
    });
}

fn dispatch_welcome_primary_action(app: &UiApp, state: &mut WelcomeHubEguiState, intents: &mut PendingUiIntents) {
    match state.left_mode {
        WelcomeLeftMode::NewTableArchetype(arch) => {
            if state.schema.entry_mode_sample {
                match dispatch_sample_creation(app, arch, &state.schema, intents) {
                    Ok(()) => state.sample_error = None,
                    Err(msg) => {
                        state.sample_error = Some(msg);
                        return;
                    }
                }
            } else {
                let params = build_empty_create_params(arch, &state.schema, &app.locale);
                intents.push_engine("sheet.createData", params);
            }
            intents.push(UiIntent::SetLauncherOpen { open: false });
        }
        WelcomeLeftMode::StarterTemplates => {
            let Some(tpl) = STARTER_TEMPLATES.get(state.schema.selected_template_index) else {
                state.template_error = Some(format!("Starter template index {} is unavailable.", state.schema.selected_template_index));
                return;
            };
            match build_starter_template_bytes(tpl.id) {
                Ok(bytes) => {
                    state.template_error = None;
                    let b64 = curvebook_platform::base64::encode(&bytes);
                    let title_trim = state.schema.title.trim();
                    let mut obj = json!({ "bytes_base64": b64 });
                    if !title_trim.is_empty()
                        && let Some(map) = obj.as_object_mut()
                    {
                        map.insert(String::from("title"), json!(title_trim));
                    }
                    intents.push_engine("project.instantiateTemplate", obj);
                    intents.push(UiIntent::SetLauncherOpen { open: false });
                }
                Err(err) => {
                    state.template_error = Some(format!("Failed to load starter template `{}`: {err}", tpl.title));
                }
            }
        }
        WelcomeLeftMode::ClonePipeline => {
            if let Some(ref src_id) = state.schema.clone_source_sheet_id {
                let mut params = json!({
                    "sheet_id": src_id.as_str(),
                    "keep_x_values": state.schema.clone_keep_x,
                    "clear_y_values": state.schema.clone_clear_values,
                    "clear_column_titles": state.schema.clone_clear_column_titles,
                    "clear_row_titles": state.schema.clone_clear_row_titles,
                    "clone_analyses": state.schema.clone_include_downstream,
                    "clone_graphs": state.schema.clone_include_downstream && state.schema.clone_graphs,
                    "clone_layouts": state.schema.clone_include_downstream && state.schema.clone_layouts,
                    "clear_results": !state.schema.clone_rerun_analyses,
                });
                let title_trim = state.schema.title.trim();
                if !title_trim.is_empty()
                    && let Some(map) = params.as_object_mut()
                {
                    map.insert(String::from("title"), json!(title_trim));
                }
                intents.push_engine("sheet.clonePipeline", params);
                intents.push(UiIntent::SetLauncherOpen { open: false });
            }
        }
        WelcomeLeftMode::OpenFile => {
            if matches!(app.session().path_policy(), curvebook_engine::ExternalPathPolicy::RejectExternal) {
                state.open_file_error = Some(String::from("External file access is disabled by host security policy."));
                return;
            }
            let trimmed = state.open_file_path_draft.trim();
            if trimmed.is_empty() {
                state.open_file_error = None;
                state.open_file_request_count = state.open_file_request_count.saturating_add(1);
                intents.push(UiIntent::OpenProjectFile);
            } else {
                let open_path = match app.session().path_policy() {
                    curvebook_engine::ExternalPathPolicy::Unconstrained => trimmed.to_owned(),
                    curvebook_engine::ExternalPathPolicy::RejectExternal => {
                        state.open_file_error = Some(String::from("External file access is disabled by host security policy."));
                        return;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    curvebook_engine::ExternalPathPolicy::Workdir(guard) => match guard.resolve(trimmed) {
                        Ok(resolved) => resolved.to_string_lossy().into_owned(),
                        Err(err) => {
                            state.open_file_error = Some(err.to_string());
                            return;
                        }
                    },
                };
                state.open_file_error = None;
                state.pending_open_mutation_seq = Some(app.mutation_seq());
                intents.push_engine("project.open", json!({ "path": open_path }));
            }
        }
    }
}

fn render_clone_options_submodal(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut WelcomeHubEguiState,
    tokens: &Tokens,
    enabled: bool,
    meta: &mut FrameInspectMeta,
) -> Option<Rect> {
    let locale = &app.locale;
    let screen = ui.ctx().content_rect();
    let modal_w = (screen.width() - 24.0).clamp(260.0, 420.0);
    let modal_h = (screen.height() - 24.0).clamp(240.0, 350.0);
    let modal_rect = Rect::from_center_size(screen.center(), Vec2::new(modal_w, modal_h));
    let mut modal_rect_out = None;

    egui::Area::new(Id::unique("welcome_clone_options_modal")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ui.ctx(), |ui| {
        let (_, bg_resp) = ui.allocate_exact_size(screen.size(), Sense::click());
        ui.painter().rect_filled(screen, CornerRadius::ZERO, tokens.scrim);
        if bg_resp.clicked() || (enabled && ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))) {
            state.clone_options_open = false;
        }

        let mut modal_ui = ui.new_child(egui::UiBuilder::new().max_rect(modal_rect).layout(Layout::top_down(Align::Min)));
        modal_ui.set_clip_rect(modal_rect);
        let frame_resp = egui::Frame::NONE
            .fill(tokens.surface_bg)
            .stroke(Stroke::new(1.0, tokens.border))
            .corner_radius(CornerRadius::same(tokens.metrics.radius_lg))
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(&mut modal_ui, |ui| {
                let z = ZLayer::ModalDialog;
                render_clipped_text_label(
                    ui,
                    "welcome.clone_modal.heading",
                    "Clone Options",
                    FontId::proportional(14.0),
                    tokens.text_strong,
                    z,
                    meta,
                );
                ui.add_space(6.0);

                if render_check_toggle(
                    ui,
                    tokens,
                    meta,
                    CheckToggleSpec {
                        widget_id: "welcome.clone.preserve_links",
                        label: tr(locale, "welcome.clone.analyses"),
                        checked: state.schema.clone_include_downstream,
                        enabled,
                        z_layer: z,
                    },
                ) && enabled
                {
                    state.schema.clone_include_downstream = !state.schema.clone_include_downstream;
                }

                if render_check_toggle(
                    ui,
                    tokens,
                    meta,
                    CheckToggleSpec {
                        widget_id: "welcome.clone.graphs",
                        label: tr(locale, "welcome.clone.graphs"),
                        checked: state.schema.clone_graphs,
                        enabled: enabled && state.schema.clone_include_downstream,
                        z_layer: z,
                    },
                ) && enabled
                    && state.schema.clone_include_downstream
                {
                    state.schema.clone_graphs = !state.schema.clone_graphs;
                }

                if render_check_toggle(
                    ui,
                    tokens,
                    meta,
                    CheckToggleSpec {
                        widget_id: "welcome.clone.layouts",
                        label: tr(locale, "welcome.clone.layouts"),
                        checked: state.schema.clone_layouts,
                        enabled: enabled && state.schema.clone_include_downstream,
                        z_layer: z,
                    },
                ) && enabled
                    && state.schema.clone_include_downstream
                {
                    state.schema.clone_layouts = !state.schema.clone_layouts;
                }

                ui.add_space(6.0);
                let row_w = (ui.available_width() - 8.0).max(120.0);
                if render_pill_button(
                    ui,
                    row_w,
                    tokens,
                    meta,
                    LabelSpec {
                        widget_id: "welcome.clone.copy_data",
                        role: "radio",
                        a11y_role: egui::Role::RadioButton,
                        label: "Copy existing data",
                        command_id: None,
                        selected: !state.schema.clone_clear_values,
                        enabled,
                        disabled_reason: None,
                        z_layer: z,
                    },
                ) && enabled
                {
                    state.schema.clone_clear_values = false;
                }
                ui.add_space(2.0);
                if render_pill_button(
                    ui,
                    row_w,
                    tokens,
                    meta,
                    LabelSpec {
                        widget_id: "welcome.clone.clear_data",
                        role: "radio",
                        a11y_role: egui::Role::RadioButton,
                        label: tr(locale, "welcome.clone.clear_y"),
                        command_id: None,
                        selected: state.schema.clone_clear_values,
                        enabled,
                        disabled_reason: None,
                        z_layer: z,
                    },
                ) && enabled
                {
                    state.schema.clone_clear_values = true;
                }

                ui.add_space(4.0);
                if render_check_toggle(
                    ui,
                    tokens,
                    meta,
                    CheckToggleSpec {
                        widget_id: "welcome.clone.keep_x",
                        label: tr(locale, "welcome.clone.keep_x"),
                        checked: state.schema.clone_keep_x,
                        enabled,
                        z_layer: z,
                    },
                ) && enabled
                {
                    state.schema.clone_keep_x = !state.schema.clone_keep_x;
                }

                if render_check_toggle(
                    ui,
                    tokens,
                    meta,
                    CheckToggleSpec {
                        widget_id: "welcome.clone.rerun_analyses",
                        label: "Re-run analyses immediately",
                        checked: state.schema.clone_rerun_analyses,
                        enabled: enabled && state.schema.clone_include_downstream,
                        z_layer: z,
                    },
                ) && enabled
                    && state.schema.clone_include_downstream
                {
                    state.schema.clone_rerun_analyses = !state.schema.clone_rerun_analyses;
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if render_action_button(
                        ui,
                        tokens,
                        meta,
                        ActionButtonSpec {
                            max_w: 92.0,
                            primary: false,
                            enabled,
                            label: tr(locale, "btn.close"),
                            widget_id: "welcome.clone_modal.close",
                            command_id: None,
                            disabled_reason: None,
                            z_layer: z,
                        },
                    ) && enabled
                    {
                        state.clone_options_open = false;
                    }
                    if render_action_button(
                        ui,
                        tokens,
                        meta,
                        ActionButtonSpec {
                            max_w: 92.0,
                            primary: true,
                            enabled,
                            label: tr(locale, "btn.apply"),
                            widget_id: "welcome.clone_modal.apply",
                            command_id: None,
                            disabled_reason: None,
                            z_layer: z,
                        },
                    ) && enabled
                    {
                        state.clone_options_open = false;
                    }
                });
            });
        modal_rect_out = Some(frame_resp.response.rect);
    });
    modal_rect_out
}

fn build_empty_create_params(arch: WelcomeArchetypeChoice, schema: &WelcomeSchemaDraft, locale: &str) -> Value {
    let title = if schema.title.trim().is_empty() { arch.label(locale).to_owned() } else { schema.title.trim().to_owned() };
    let mut obj = json!({
        "kind": arch.as_command_kind(),
        "title": title,
        "cols": schema.cols.clamp(1, 64),
        "rows": schema.rows.clamp(1, 10_000),
        "auto_graph": schema.auto_graph,
        "auto_info": schema.auto_info,
    });
    if let Some(map) = obj.as_object_mut() {
        match arch {
            WelcomeArchetypeChoice::Xy => {
                let x_mode = match schema.x_mode_kind {
                    1 => XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN },
                    2 => XColumnMode::DateOrElapsedTime { unit_label: String::from("Date") },
                    3 => XColumnMode::DateOrElapsedTime { unit_label: String::from("min") },
                    4 => XColumnMode::GeneratedSequence { start: 0.0, step: 1.0, progression: SequenceProgression::Arithmetic },
                    _ => XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single },
                };
                map.insert(String::from("x_mode"), json!(x_mode));
                map.insert(String::from("x_title"), json!(schema.x_title));
                map.insert(String::from("subcolumn_mode"), json!(subcolumn_mode_from_index(schema.y_subcolumn_kind, schema.subcols)));
            }
            WelcomeArchetypeChoice::Column => {
                let (design, subcol) = match schema.col_design_kind {
                    1 => (ColumnDesignHint::PairedRepeatedMeasures, SubcolumnMode::Single),
                    2 => (
                        ColumnDesignHint::PrecomputedError,
                        subcolumn_mode_from_index(normalize_summary_subcolumn_kind(schema.y_subcolumn_kind), schema.subcols),
                    ),
                    _ => (ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single),
                };
                map.insert(String::from("design_hint"), json!(design));
                map.insert(String::from("subcolumn_mode"), json!(subcol));
            }
            WelcomeArchetypeChoice::Grouped => {
                map.insert(String::from("subcolumn_mode"), json!(subcolumn_mode_from_index(schema.y_subcolumn_kind.max(1), schema.subcols)));
                map.insert(String::from("replicates"), json!(schema.subcols.clamp(1, 256)));
                map.insert(String::from("row_factor_title"), json!(schema.row_factor_title));
                map.insert(String::from("column_factor_title"), json!(schema.column_factor_title));
            }
            WelcomeArchetypeChoice::Contingency => {
                map.insert(String::from("row_factor_title"), json!(schema.row_factor_title));
                map.insert(String::from("column_factor_title"), json!(schema.column_factor_title));
            }
            WelcomeArchetypeChoice::Survival => {
                let time_title = if schema.survival_time_kind == 1 { "Date" } else { schema.survival_time_title.as_str() };
                map.insert(String::from("time_title"), json!(time_title));
                map.insert(String::from("event_code"), json!(schema.survival_event_code));
                map.insert(String::from("censored_code"), json!(schema.survival_censored_code));
            }
            WelcomeArchetypeChoice::PartsOfWhole => {
                map.insert(String::from("cols"), json!(schema.cols.clamp(1, 16)));
            }
            WelcomeArchetypeChoice::MultipleVariables => {
                let n = schema.cols.clamp(1, 32);
                let mut vars = Vec::with_capacity(n);
                for idx in 0..n {
                    let role = match schema.mv_role_kinds.get(idx).copied().unwrap_or(0) {
                        1 => "categorical",
                        2 => "label",
                        _ => "continuous",
                    };
                    vars.push(json!({ "title": format!("Var {}", idx + 1), "role": role }));
                }
                map.insert(String::from("variables"), Value::Array(vars));
            }
            WelcomeArchetypeChoice::Nested => {
                map.insert(String::from("replicates"), json!(schema.subcols.clamp(2, 64)));
            }
        }
    }
    obj
}

fn subcolumn_mode_from_index(idx: usize, replicates: usize) -> SubcolumnMode {
    match idx {
        1 => SubcolumnMode::RawReplicates(replicates.clamp(1, 256)),
        2 => SubcolumnMode::MeanSdN,
        3 => SubcolumnMode::MeanSemN,
        4 => SubcolumnMode::MeanCvN,
        5 => SubcolumnMode::MeanLowHigh,
        6 => SubcolumnMode::UpperLowerLimits,
        7 => SubcolumnMode::MedianIqr,
        _ => SubcolumnMode::Single,
    }
}

fn matching_samples_in(fixtures: &'static [SampleFixtureDescriptor], arch: WelcomeArchetypeChoice) -> Vec<&'static SampleFixtureDescriptor> {
    fixtures.iter().filter(|f| f.kind == arch.table_kind()).collect()
}

fn matching_samples_for(arch: WelcomeArchetypeChoice) -> Vec<&'static SampleFixtureDescriptor> {
    matching_samples_in(SAMPLE_FIXTURES, arch)
}

fn archetype_has_sample(arch: WelcomeArchetypeChoice) -> bool {
    !matching_samples_for(arch).is_empty()
}

/// Build an atomic [`UiIntent::EngineCommandGroup`] instantiating sample fixture `fixture_id`.
///
/// # Errors
/// Returns an error string if `fixture_id` is unknown or cannot be constructed.
pub fn build_sample_creation_intent(
    app: &UiApp,
    fixture_id: &str,
    title_override: Option<&str>,
    auto_graph: bool,
    auto_info: bool,
) -> Result<UiIntent, String> {
    let doc = app.document();
    let new_sid = next_unique_sheet_id(doc, "data");
    let ds = instantiate_sample_fixture(fixture_id, new_sid.clone()).map_err(|e| e.to_string())?;
    let title = title_override.map(str::trim).filter(|s| !s.is_empty()).map_or_else(|| ds.title.clone(), str::to_owned);
    let mut create_obj = json!({
        "sheet_id": new_sid.as_str(),
        "title": title,
        "kind": ds.archetype.kind(),
        "cols": ds.archetype.column_count().max(1),
        "rows": ds.archetype.row_count().max(1),
        "auto_graph": auto_graph,
        "auto_info": auto_info,
    });
    if let Some(map) = create_obj.as_object_mut() {
        match &ds.archetype {
            TableArchetype::Xy(t) => {
                map.insert(String::from("x_mode"), json!(t.x_mode));
                map.insert(String::from("x_title"), json!(t.x_title));
                map.insert(String::from("subcolumn_mode"), json!(t.y_subcolumn_mode));
            }
            TableArchetype::Column(t) => {
                map.insert(String::from("design_hint"), json!(t.design_hint));
                map.insert(String::from("subcolumn_mode"), json!(t.subcolumn_mode));
            }
            TableArchetype::Grouped(t) => {
                map.insert(String::from("subcolumn_mode"), json!(t.subcolumn_mode));
                map.insert(String::from("row_factor_title"), json!(t.row_factor_title));
                map.insert(String::from("column_factor_title"), json!(t.column_factor_title));
            }
            TableArchetype::Contingency(t) => {
                map.insert(String::from("row_factor_title"), json!(t.row_factor_title));
                map.insert(String::from("column_factor_title"), json!(t.column_factor_title));
            }
            TableArchetype::Survival(t) => {
                map.insert(String::from("time_title"), json!(t.time_title));
                map.insert(String::from("event_code"), json!(t.event_code));
                map.insert(String::from("censored_code"), json!(t.censored_code));
            }
            TableArchetype::MultipleVariables(t) => {
                let vars: Vec<Value> = t
                    .variables
                    .iter()
                    .map(|v| {
                        let role = if matches!(v.source, VariableSource::Calculated { .. }) { json!("continuous") } else { json!(v.role) };
                        json!({ "title": v.title, "role": role })
                    })
                    .collect();
                map.insert(String::from("variables"), Value::Array(vars));
            }
            TableArchetype::Nested(t) => {
                let reps = t.groups.first().map_or(2, |g| g.subcolumn_titles.len().max(1));
                map.insert(String::from("replicates"), json!(reps));
            }
            TableArchetype::PartsOfWhole(_) => {}
        }
    }
    let mut commands = vec![(String::from("sheet.createData"), create_obj)];
    collect_sample_population_commands(new_sid.as_str(), &ds.archetype, &mut commands);
    Ok(UiIntent::EngineCommandGroup { commands })
}

fn dispatch_sample_creation(
    app: &UiApp,
    arch: WelcomeArchetypeChoice,
    schema: &WelcomeSchemaDraft,
    intents: &mut PendingUiIntents,
) -> Result<(), String> {
    dispatch_sample_creation_from(app, SAMPLE_FIXTURES, arch, schema, intents)
}

/// Queue creation of the selected sample fixture drawn from `catalog`.
///
/// Never falls back to an empty table: an archetype with no fixture, or a fixture that fails to
/// build, returns a localised error and queues nothing.
fn dispatch_sample_creation_from(
    app: &UiApp,
    catalog: &'static [SampleFixtureDescriptor],
    arch: WelcomeArchetypeChoice,
    schema: &WelcomeSchemaDraft,
    intents: &mut PendingUiIntents,
) -> Result<(), String> {
    let fixtures = matching_samples_in(catalog, arch);
    let Some(fx) = fixtures.get(schema.selected_sample_index.min(fixtures.len().saturating_sub(1))) else {
        return Err(tr(&app.locale, "welcome.sample.unavailable").to_owned());
    };
    match build_sample_creation_intent(app, fx.id, Some(&schema.title), schema.auto_graph, schema.auto_info) {
        Ok(intent) => {
            intents.push(intent);
            Ok(())
        }
        Err(err) => Err(format!("{} {err}", tr(&app.locale, "welcome.sample.load_failed"))),
    }
}

fn collect_sample_population_commands(sid: &str, arch: &TableArchetype, commands: &mut Vec<(String, Value)>) {
    let mut x_edits = Vec::new();
    match arch {
        TableArchetype::Xy(t) => {
            for (r, subs) in t.x_cells.iter().enumerate() {
                for (s, cell) in subs.iter().enumerate() {
                    push_cell_edit(&mut x_edits, r, None, Some(s), cell);
                }
            }
        }
        TableArchetype::Survival(t) => {
            for (r, cell) in t.times.iter().enumerate() {
                push_cell_edit(&mut x_edits, r, None, Some(0), cell);
            }
        }
        _ => {}
    }
    if !x_edits.is_empty() {
        commands.push((String::from("table.setXCells"), json!({ "sheet_id": sid, "edits": x_edits })));
    }

    let col_titles: Vec<String> = match arch {
        TableArchetype::Xy(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Column(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Grouped(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Contingency(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Survival(t) => t.cohorts.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::PartsOfWhole(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::MultipleVariables(t) => t.variables.iter().map(|v| v.title.clone()).collect(),
        TableArchetype::Nested(t) => t.groups.iter().map(|g| g.title.clone()).collect(),
    };
    if col_titles.iter().any(|t| !t.trim().is_empty()) {
        commands.push((String::from("table.setTitles"), json!({ "sheet_id": sid, "titles": col_titles })));
    }

    let row_titles: &[String] = match arch {
        TableArchetype::Xy(t) => &t.row_titles,
        TableArchetype::Column(t) => &t.row_titles,
        TableArchetype::Grouped(t) => &t.row_titles,
        TableArchetype::Contingency(t) => &t.row_titles,
        TableArchetype::Survival(t) => &t.row_titles,
        TableArchetype::PartsOfWhole(t) => &t.slice_labels,
        TableArchetype::MultipleVariables(t) => &t.row_titles,
        TableArchetype::Nested(t) => &t.row_titles,
    };
    if row_titles.iter().any(|t| !t.trim().is_empty()) {
        let titles: Vec<Option<String>> = row_titles.iter().map(|t| (!t.trim().is_empty()).then(|| t.clone())).collect();
        commands.push((String::from("table.setRowTitles"), json!({ "sheet_id": sid, "titles": titles })));
    }

    if let TableArchetype::Nested(t) = arch {
        for (col, grp) in t.groups.iter().enumerate() {
            if grp.subcolumn_titles.iter().any(|s| !s.trim().is_empty()) {
                commands.push((String::from("table.setSubcolumnTitles"), json!({ "sheet_id": sid, "col": col, "titles": grp.subcolumn_titles })));
            }
        }
    }

    let mut edits = Vec::new();
    match arch {
        TableArchetype::Xy(t) => collect_dataset_edits(&t.data_sets, &mut edits),
        TableArchetype::Column(t) => collect_dataset_edits(&t.data_sets, &mut edits),
        TableArchetype::Grouped(t) => collect_dataset_edits(&t.data_sets, &mut edits),
        TableArchetype::Contingency(t) => collect_dataset_edits(&t.data_sets, &mut edits),
        TableArchetype::PartsOfWhole(t) => collect_dataset_edits(&t.data_sets, &mut edits),
        TableArchetype::Survival(t) => collect_dataset_edits(&t.cohorts, &mut edits),
        TableArchetype::MultipleVariables(t) => {
            for (c, var) in t.variables.iter().enumerate() {
                if matches!(var.source, VariableSource::Calculated { .. }) {
                    continue;
                }
                for (r, cell) in var.cells.iter().enumerate() {
                    push_cell_edit(&mut edits, r, Some(c), Some(0), cell);
                }
            }
        }
        TableArchetype::Nested(t) => {
            for (c, grp) in t.groups.iter().enumerate() {
                for (r, subs) in grp.cells.iter().enumerate() {
                    for (s, cell) in subs.iter().enumerate() {
                        push_cell_edit(&mut edits, r, Some(c), Some(s), cell);
                    }
                }
            }
        }
    }
    if !edits.is_empty() {
        commands.push((String::from("table.setCells"), json!({ "sheet_id": sid, "edits": edits })));
    }

    if let TableArchetype::MultipleVariables(t) = arch {
        for (col, var) in t.variables.iter().enumerate() {
            if let VariableSource::Calculated { formula } = &var.source {
                commands.push((
                    String::from("table.setCalculatedVariable"),
                    json!({ "sheet_id": sid, "col": col, "title": var.title, "formula": formula }),
                ));
            }
        }
    }
}

fn collect_dataset_edits(cols: &[DataSetColumn], out: &mut Vec<Value>) {
    for (c, col) in cols.iter().enumerate() {
        for (r, subs) in col.cells.iter().enumerate() {
            for (s, cell) in subs.iter().enumerate() {
                push_cell_edit(out, r, Some(c), Some(s), cell);
            }
        }
    }
}

fn push_cell_edit(out: &mut Vec<Value>, row: usize, col: Option<usize>, subcol: Option<usize>, cell: &Cell) {
    let val = match &cell.value {
        CellValue::Empty => return,
        CellValue::Number(v) if v.is_finite() => json!(v),
        CellValue::Number(_) => return,
        CellValue::Text(s) => json!(s),
    };
    let mut obj = json!({ "row": row, "value": val, "excluded": cell.excluded });
    if let Some(map) = obj.as_object_mut() {
        if let Some(c) = col {
            map.insert(String::from("col"), json!(c));
        }
        if let Some(s) = subcol {
            map.insert(String::from("subcol"), json!(s));
        }
    }
    out.push(obj);
}

fn next_unique_sheet_id(doc: &ProjectDocument, prefix: &str) -> SheetId {
    for idx in 1..10_000 {
        let candidate = SheetId::from(format!("{prefix}-{idx}"));
        if doc.sheet_section(&candidate).is_none() {
            return candidate;
        }
    }
    SheetId::from(format!("{prefix}-new"))
}

fn build_starter_template_bytes(template_id: &str) -> Result<Vec<u8>, String> {
    let mut doc = ProjectDocument::new("Starter Template").map_err(|e| e.to_string())?;
    let data_id = SheetId::from("data-1");
    let info_id = SheetId::from("info-1");
    let graph_id = SheetId::from("graph-1");

    let (ds_title, arch) = match template_id {
        "dose_response_ic50" => {
            let mut xy =
                XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(3), 2, 8)
                    .map_err(|e| e.to_string())?;
            xy.x_title = String::from("Log[Agonist] (M)");
            ("Dose-Response IC50", TableArchetype::Xy(xy))
        }
        "paired_treatment_column" => {
            let col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 10).map_err(|e| e.to_string())?;
            ("Paired Before / After", TableArchetype::Column(col))
        }
        "kaplan_meier_cohort" => {
            let surv = SurvivalTable::new(2, 16).map_err(|e| e.to_string())?;
            ("Kaplan-Meier Cohort", TableArchetype::Survival(surv))
        }
        _ => {
            let grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 3).map_err(|e| e.to_string())?;
            ("Two-Way Grouped Assay", TableArchetype::Grouped(grp))
        }
    };

    let mut ds = DataSheet::new(data_id.clone(), ds_title, arch).map_err(|e| e.to_string())?;
    ds.info_sheet_id = Some(info_id.clone());
    doc.title = String::from(ds_title);
    let graph = GraphSheet::new_uninitialized_for_data(graph_id, &ds).map_err(|e| e.to_string())?;
    doc.data_sheets.push(ds);
    let mut info = InfoSheet::new_preseeded(info_id, format!("{ds_title} Info")).map_err(|e| e.to_string())?;
    info.linked_data_sheets = vec![data_id.clone()];
    info.notes_markdown = format!("Starter template: {ds_title}");
    doc.info_sheets.push(info);
    doc.graph_sheets.push(graph);
    doc.result_sheets = Vec::<ResultSheet>::new();
    doc.active_sheet_id = Some(data_id);

    save_template_bytes(&doc).map_err(|e| e.to_string())
}

fn draw_vector_table_schematic(painter: &egui::Painter, rect: Rect, arch: WelcomeArchetypeChoice, schema: &WelcomeSchemaDraft, tokens: &Tokens) {
    if !rect.is_positive() {
        return;
    }
    let has_x = matches!(arch, WelcomeArchetypeChoice::Xy | WelcomeArchetypeChoice::Survival);
    let rows = 5usize;
    let data_cols = match arch {
        WelcomeArchetypeChoice::PartsOfWhole => 1,
        _ => schema.cols.clamp(2, 3),
    };
    let total_cols = data_cols + usize::from(has_x) + 1;
    let col_w = rect.width() / (total_cols as f32);
    let row_h = rect.height() / (rows as f32);

    for r in 0..rows {
        for c in 0..total_cols {
            let cell = Rect::from_min_size(Pos2::new(rect.left() + c as f32 * col_w, rect.top() + r as f32 * row_h), Vec2::new(col_w, row_h));
            if r == 0 {
                let fill = if c == 0 {
                    tokens.header_bg
                } else if has_x && c == 1 {
                    tokens.active_bg
                } else {
                    tokens.panel_bg
                };
                painter.rect_filled(cell, CornerRadius::ZERO, fill);
            }
            painter.rect_stroke(cell, CornerRadius::ZERO, Stroke::new(0.8, tokens.divider), StrokeKind::Inside);

            let label = match (r, c, has_x) {
                (0, 0, _) => String::from("#"),
                (0, 1, true) => String::from(if matches!(arch, WelcomeArchetypeChoice::Survival) {
                    "Time"
                } else if schema.x_mode_kind == 1 {
                    "X±Err"
                } else {
                    "X"
                }),
                (0, idx, _) => {
                    let k = idx - 1 - usize::from(has_x);
                    match arch {
                        WelcomeArchetypeChoice::MultipleVariables => match schema.mv_role_kinds.get(k).copied().unwrap_or(0) {
                            1 => format!("V{}(Cat)", k + 1),
                            2 => format!("V{}(Lbl)", k + 1),
                            _ => format!("V{}", k + 1),
                        },
                        WelcomeArchetypeChoice::Column if schema.paired_design || schema.col_design_kind == 1 => format!("Pair {}", k + 1),
                        WelcomeArchetypeChoice::Survival => format!("C{} ({}/{})", k + 1, schema.survival_event_code, schema.survival_censored_code),
                        _ => format!("Y{}", k + 1),
                    }
                }
                _ => continue,
            };
            let fg = if r == 0 { tokens.text_strong } else { tokens.text_muted };
            if col_w >= 18.0 && row_h >= 14.0 {
                let (g, _) = super::ui_kit::elided_galley(painter, &label, FontId::proportional(9.5), fg, (col_w - 4.0).max(8.0), false);
                let text_clip = cell.shrink(1.0).intersect(painter.clip_rect());
                if text_clip.is_positive() {
                    painter.with_clip_rect(text_clip).galley(Pos2::new(cell.left() + 2.0, cell.center().y - g.size().y * 0.5), g, fg);
                }
            }
        }
    }
}

fn draw_vector_plot_preview(painter: &egui::Painter, rect: Rect, arch: WelcomeArchetypeChoice, schema: &WelcomeSchemaDraft, tokens: &Tokens) {
    if !rect.is_positive() {
        return;
    }
    let plot = rect.shrink(6.0);
    if !plot.is_positive() {
        return;
    }
    painter.vline(plot.left(), plot.y_range(), Stroke::new(1.0, tokens.text_muted));
    painter.hline(plot.x_range(), plot.bottom(), Stroke::new(1.0, tokens.text_muted));

    let c1 = tokens.accent;
    let c2 = tokens.link_green;
    match arch {
        WelcomeArchetypeChoice::Xy => {
            let pts = [(0.15, 0.25), (0.35, 0.55), (0.60, 0.72), (0.85, 0.88)];
            let screen_pts: Vec<Pos2> =
                pts.iter().map(|&(tx, ty)| Pos2::new(plot.left() + plot.width() * tx, plot.bottom() - plot.height() * ty)).collect();
            for pair in screen_pts.windows(2) {
                painter.line_segment([pair[0], pair[1]], Stroke::new(1.6, c1));
            }
            for &p in &screen_pts {
                painter.circle_filled(p, 2.8, c1);
                if schema.y_subcolumn_kind > 0 || schema.x_mode_kind == 1 {
                    painter.vline(p.x, (p.y - 6.0)..=(p.y + 6.0), Stroke::new(1.0, c1));
                }
            }
        }
        WelcomeArchetypeChoice::Column | WelcomeArchetypeChoice::Nested => {
            if matches!(arch, WelcomeArchetypeChoice::Column) && (schema.paired_design || schema.col_design_kind == 1) {
                for row in [(0.3, 0.55), (0.45, 0.75), (0.25, 0.45)] {
                    let a = Pos2::new(plot.left() + plot.width() * 0.28, plot.bottom() - plot.height() * row.0);
                    let b = Pos2::new(plot.left() + plot.width() * 0.72, plot.bottom() - plot.height() * row.1);
                    painter.line_segment([a, b], Stroke::new(1.2, c1));
                    painter.circle_filled(a, 2.5, c1);
                    painter.circle_filled(b, 2.5, c2);
                }
            } else {
                for (i, h) in [0.42_f32, 0.78, 0.58].iter().enumerate() {
                    let bw = plot.width() * 0.18;
                    let x = plot.left() + plot.width() * (0.16 + i as f32 * 0.28);
                    let top = plot.bottom() - plot.height() * *h;
                    let bar = Rect::from_min_max(Pos2::new(x, top), Pos2::new(x + bw, plot.bottom()));
                    painter.rect_filled(bar, CornerRadius::same(2), if i == 1 { c2 } else { c1 });
                }
            }
        }
        WelcomeArchetypeChoice::Grouped | WelcomeArchetypeChoice::Contingency => {
            for g in 0..3 {
                let gx = plot.left() + plot.width() * (0.12 + g as f32 * 0.30);
                let bw = plot.width() * 0.10;
                for s in 0..2 {
                    let h = 0.35 + ((g + s * 2) as f32 * 0.12);
                    let top = plot.bottom() - plot.height() * h.min(0.88);
                    let bar =
                        Rect::from_min_max(Pos2::new(gx + s as f32 * (bw + 2.0), top), Pos2::new(gx + s as f32 * (bw + 2.0) + bw, plot.bottom()));
                    painter.rect_filled(bar, CornerRadius::same(1), if s == 0 { c1 } else { c2 });
                }
            }
        }
        WelcomeArchetypeChoice::Survival => {
            let steps = [(0.0, 0.92), (0.3, 0.92), (0.3, 0.70), (0.62, 0.70), (0.62, 0.44), (0.9, 0.44)];
            let pts: Vec<Pos2> =
                steps.iter().map(|&(tx, ty)| Pos2::new(plot.left() + plot.width() * tx, plot.bottom() - plot.height() * ty)).collect();
            for pair in pts.windows(2) {
                painter.line_segment([pair[0], pair[1]], Stroke::new(1.6, c1));
            }
            painter.vline(pts[2].x, (pts[2].y - 4.0)..=(pts[2].y + 4.0), Stroke::new(1.2, c2));
        }
        WelcomeArchetypeChoice::PartsOfWhole => {
            let center = plot.center();
            let rad = plot.width().min(plot.height()) * 0.36;
            painter.circle_stroke(center, rad, Stroke::new(6.0, c1));
            painter.circle_stroke(center, (rad - 6.0).max(4.0), Stroke::new(2.0, c2));
        }
        WelcomeArchetypeChoice::MultipleVariables => {
            for (tx, ty, r, col) in [(0.2, 0.25, 3.0, c1), (0.42, 0.50, 4.2, c2), (0.65, 0.68, 3.4, c1), (0.82, 0.84, 4.6, c2)] {
                painter.circle_filled(Pos2::new(plot.left() + plot.width() * tx, plot.bottom() - plot.height() * ty), r, col);
            }
        }
    }
}

fn rect_visible_in_clip(ui: &egui::Ui, size: Vec2) -> bool {
    let predicted = Rect::from_min_size(ui.cursor().min, size);
    let visible = predicted.intersect(ui.clip_rect());
    visible.height() >= 4.0 && visible.width() >= 4.0
}

fn allocate_visible_slot(ui: &mut egui::Ui, size: Vec2, sense: Sense) -> Option<(Rect, Response)> {
    if !rect_visible_in_clip(ui, size) {
        ui.allocate_space(size);
        return None;
    }
    Some(ui.allocate_exact_size(size, sense))
}

fn render_clipped_text_edit(ui: &mut egui::Ui, text: &mut String, meta: &mut FrameInspectMeta, spec: TextEditSpec<'_>) {
    let TextEditSpec { widget_id, label, hint, width, enabled, z_layer } = spec;
    if !rect_visible_in_clip(ui, Vec2::new(width, 22.0)) {
        ui.allocate_space(Vec2::new(width, 22.0));
        return;
    }
    let resp = ui.add_enabled(enabled, egui::TextEdit::singleline(text).hint_text(hint).desired_width(width));
    let ak_label = if label.is_empty() { hint.to_owned() } else { label.to_owned() };
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::TextInput, enabled, ak_label.clone()));
    if resp.rect.intersect(ui.clip_rect()).height() >= 4.0 {
        meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id,
                role: "textbox",
                label,
                command_id: None,
                value: Some(json!(text)),
                enabled,
                disabled_reason: None,
                z_layer,
                ellipsized: false,
                full_label: None,
            },
        );
    }
}

fn render_plain_clipped_label(ui: &mut egui::Ui, text: &str, font: FontId, color: Color32) {
    let max_w = ui.available_width().max(24.0);
    render_plain_clipped_label_w(ui, text, max_w, font, color);
}

fn render_plain_clipped_label_w(ui: &mut egui::Ui, text: &str, max_w: f32, font: FontId, color: Color32) {
    super::ui_kit::plain_clipped_label(ui, text, max_w, 15.0, font, color, true);
}

fn render_arch_rail_button(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: ArchRailButtonSpec<'_>) -> bool {
    let ArchRailButtonSpec { arch, width, selected, enabled, locale, z_layer } = spec;
    let Some((rect, mut resp)) = allocate_visible_slot(ui, Vec2::new(width, 24.0), if enabled { Sense::click() } else { Sense::hover() }) else {
        return false;
    };
    let label = arch.label(locale);
    let fg = if !enabled {
        tokens.text_disabled
    } else if selected {
        tokens.selected_accent_text()
    } else {
        tokens.text_primary
    };
    let text_max_w = (width - 28.0).max(16.0);
    let (galley, ellipsized) = super::ui_kit::elided_galley(ui.painter(), label, FontId::proportional(11.5), fg, text_max_w, false);

    let bg = if selected {
        tokens.active_bg
    } else if resp.hovered() && enabled {
        tokens.hover_bg
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, CornerRadius::same(4), bg);
    }
    if selected {
        ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
    }

    let glyph_rect = Rect::from_min_size(Pos2::new(rect.left() + 6.0, rect.center().y - 5.0), Vec2::new(12.0, 10.0));
    draw_tiny_arch_glyph(ui.painter(), glyph_rect, arch, fg);

    let text_rect = Rect::from_min_max(Pos2::new(rect.left() + 22.0, rect.top() + 2.0), Pos2::new(rect.right() - 4.0, rect.bottom() - 2.0));
    ui.painter().with_clip_rect(text_rect.intersect(ui.clip_rect())).galley(
        Pos2::new(text_rect.left(), rect.center().y - galley.size().y * 0.5),
        galley,
        fg,
    );

    if ellipsized {
        resp = resp.on_hover_text(label);
    }
    let a11y_label = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::Tab, enabled, selected, a11y_label.clone()));
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: arch.widget_id(),
            role: "tab",
            label,
            command_id: None,
            value: Some(json!({ "selected": selected })),
            enabled,
            disabled_reason: None,
            z_layer,
            ellipsized,
            full_label: ellipsized.then_some(label),
        },
    );
    resp.clicked()
}

fn draw_tiny_arch_glyph(painter: &egui::Painter, rect: Rect, arch: WelcomeArchetypeChoice, color: Color32) {
    painter.rect_stroke(rect, CornerRadius::same(1), Stroke::new(1.0, color), StrokeKind::Inside);
    match arch {
        WelcomeArchetypeChoice::Xy => {
            painter.line_segment(
                [Pos2::new(rect.left() + 2.0, rect.bottom() - 2.0), Pos2::new(rect.right() - 2.0, rect.top() + 2.0)],
                Stroke::new(1.2, color),
            );
        }
        WelcomeArchetypeChoice::Column | WelcomeArchetypeChoice::Grouped => {
            painter.vline(rect.left() + 4.0, (rect.top() + 3.0)..=(rect.bottom() - 1.0), Stroke::new(1.4, color));
            painter.vline(rect.left() + 8.0, (rect.top() + 2.0)..=(rect.bottom() - 1.0), Stroke::new(1.4, color));
        }
        WelcomeArchetypeChoice::Survival => {
            let mid = Pos2::new(rect.center().x, rect.center().y);
            painter.hline(rect.left()..=mid.x, rect.top() + 3.0, Stroke::new(1.1, color));
            painter.vline(mid.x, (rect.top() + 3.0)..=(rect.bottom() - 3.0), Stroke::new(1.1, color));
            painter.hline(mid.x..=rect.right(), rect.bottom() - 3.0, Stroke::new(1.1, color));
        }
        WelcomeArchetypeChoice::PartsOfWhole => {
            painter.circle_stroke(rect.center(), 3.2, Stroke::new(1.1, color));
        }
        _ => {
            painter.hline(rect.x_range(), rect.center().y, Stroke::new(1.0, color));
            painter.vline(rect.center().x, rect.y_range(), Stroke::new(1.0, color));
        }
    }
}

fn render_pill_button(ui: &mut egui::Ui, max_w: f32, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: LabelSpec<'_>) -> bool {
    let (_, clicked) = super::ui_kit::styled_button(
        ui,
        tokens,
        meta,
        super::ui_kit::StyledButtonSpec {
            widget_id: spec.widget_id,
            role: spec.role,
            a11y_role: spec.a11y_role,
            a11y_selectable: true,
            label: spec.label,
            tooltip: None,
            command_id: spec.command_id,
            width: super::ui_kit::ButtonWidth::Fixed(max_w.max(28.0)),
            height: 22.0,
            font_size: 11.5,
            radius: tokens.metrics.radius_md,
            text_align: super::ui_kit::ButtonTextAlign::Left(6.0),
            break_anywhere: false,
            primary: false,
            selected: spec.selected,
            enabled: spec.enabled,
            accent_text_when_selected: true,
            border_override: None,
            value: Some(json!({ "selected": spec.selected })),
            disabled_reason: spec.disabled_reason,
            z_layer: spec.z_layer,
            skip_offscreen: true,
            clip_interact_to_viewport: false,
            tooltip_mode: super::ui_kit::TooltipMode::Standard,
        },
    );
    clicked
}

fn render_action_button(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: ActionButtonSpec<'_>) -> bool {
    let (_, clicked) = super::ui_kit::styled_button(
        ui,
        tokens,
        meta,
        super::ui_kit::StyledButtonSpec {
            widget_id: spec.widget_id,
            role: "button",
            a11y_role: egui::Role::Button,
            a11y_selectable: false,
            label: spec.label,
            tooltip: None,
            command_id: spec.command_id,
            width: super::ui_kit::ButtonWidth::Fixed(spec.max_w.max(22.0)),
            height: 26.0,
            font_size: 12.0,
            radius: tokens.metrics.radius_md,
            text_align: super::ui_kit::ButtonTextAlign::Center,
            break_anywhere: false,
            primary: spec.primary,
            selected: false,
            enabled: spec.enabled,
            accent_text_when_selected: false,
            border_override: None,
            value: None,
            disabled_reason: (!spec.enabled).then(|| spec.disabled_reason.unwrap_or("unavailable")),
            z_layer: spec.z_layer,
            skip_offscreen: true,
            clip_interact_to_viewport: false,
            tooltip_mode: super::ui_kit::TooltipMode::Standard,
        },
    );
    clicked
}

fn render_check_toggle(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: CheckToggleSpec<'_>) -> bool {
    super::ui_kit::choice_row(
        ui,
        tokens,
        meta,
        super::ui_kit::ChoiceRowSpec {
            widget_id: spec.widget_id,
            label: spec.label,
            checked: spec.checked,
            is_radio: false,
            enabled: spec.enabled,
            disabled_reason: None,
            font_size: 11.5,
            row_height: 22.0,
            z_layer: spec.z_layer,
            value: json!({ "checked": spec.checked }),
            tooltip_mode: super::ui_kit::TooltipMode::Standard,
        },
    )
}

fn render_int_stepper(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: StepperSpec<'_, i64>) -> i64 {
    let StepperSpec { widget_id, label, value, range, enabled, z_layer } = spec;
    let mut out = value.clamp(*range.start(), *range.end());
    let slot = Vec2::new(ui.available_width().max(40.0), 26.0);
    if !rect_visible_in_clip(ui, slot) {
        ui.allocate_space(slot);
        return out;
    }
    let dec_id = format!("{widget_id}.dec");
    let inc_id = format!("{widget_id}.inc");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let lbl_w = (ui.available_width() - 96.0).max(40.0);
        super::ui_kit::plain_clipped_label(ui, label, lbl_w, 22.0, FontId::proportional(11.5), tokens.text_primary, false);

        if render_action_button(
            ui,
            tokens,
            meta,
            ActionButtonSpec {
                max_w: 24.0,
                primary: false,
                enabled: enabled && out > *range.start(),
                label: "-",
                widget_id: &dec_id,
                command_id: None,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
            && out > *range.start()
        {
            out -= 1;
        }
        render_plain_clipped_label_w(ui, &out.to_string(), 32.0, FontId::proportional(11.5), tokens.text_strong);
        if render_action_button(
            ui,
            tokens,
            meta,
            ActionButtonSpec {
                max_w: 24.0,
                primary: false,
                enabled: enabled && out < *range.end(),
                label: "+",
                widget_id: &inc_id,
                command_id: None,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
            && out < *range.end()
        {
            out += 1;
        }
    });
    out
}

fn render_usize_stepper(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: StepperSpec<'_, usize>) -> usize {
    let start = i64::try_from(*spec.range.start()).unwrap_or(0);
    let end = i64::try_from(*spec.range.end()).unwrap_or(i64::MAX);
    let val = i64::try_from(spec.value).unwrap_or(start);
    let out = render_int_stepper(
        ui,
        tokens,
        meta,
        StepperSpec { widget_id: spec.widget_id, label: spec.label, value: val, range: start..=end, enabled: spec.enabled, z_layer: spec.z_layer },
    );
    usize::try_from(out).unwrap_or(*spec.range.start())
}

fn render_i64_stepper(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: StepperSpec<'_, i64>) -> i64 {
    render_int_stepper(ui, tokens, meta, spec)
}

fn render_clipped_text_label(
    ui: &mut egui::Ui,
    widget_id: &str,
    label: &str,
    font: FontId,
    color: Color32,
    z_layer: ZLayer,
    meta: &mut FrameInspectMeta,
) {
    let _ = super::ui_kit::clipped_label(
        ui,
        meta,
        super::ui_kit::ClippedLabelSpec {
            widget_id,
            role: "label",
            text: label,
            max_w: ui.available_width().max(24.0),
            fixed_width: false,
            min_h: 16.0,
            font,
            color,
            z_layer,
            tooltip: None,
            record_meta: true,
            record_value: false,
            skip_offscreen: true,
        },
    );
}

#[cfg(test)]
mod sample_error_tests {
    use super::*;

    static NO_XY_FIXTURES: &[SampleFixtureDescriptor] = &[SampleFixtureDescriptor {
        id: "unknown-fixture-id",
        title: "Broken",
        kind: TableKind::Column,
        summary: "",
        row_count: 1,
        column_count: 1,
        replicates: 1,
    }];

    static BROKEN_XY_FIXTURES: &[SampleFixtureDescriptor] = &[SampleFixtureDescriptor {
        id: "no-such-fixture",
        title: "Broken",
        kind: TableKind::Xy,
        summary: "",
        row_count: 1,
        column_count: 1,
        replicates: 1,
    }];

    #[test]
    fn archetype_without_fixture_cannot_dispatch_sample_creation() {
        let app = UiApp::new();
        let schema = WelcomeSchemaDraft { entry_mode_sample: true, ..Default::default() };
        let mut intents = PendingUiIntents::default();
        let res = dispatch_sample_creation_from(&app, NO_XY_FIXTURES, WelcomeArchetypeChoice::Xy, &schema, &mut intents);
        assert_eq!(res, Err(tr(&app.locale, "welcome.sample.unavailable").to_owned()));
        assert!(intents.intents().is_empty(), "no intent (e.g. empty sheet.createData) may be queued");
        assert!(!tr(&app.locale, "welcome.sample.unavailable").is_empty());
        assert_eq!(app.document().total_sheet_count(), 0);
    }

    #[test]
    fn failing_fixture_build_returns_error_and_queues_nothing() {
        let app = UiApp::new();
        let mut state = WelcomeHubEguiState { left_mode: WelcomeLeftMode::NewTableArchetype(WelcomeArchetypeChoice::Xy), ..Default::default() };
        state.schema.entry_mode_sample = true;
        let mut intents = PendingUiIntents::default();
        let res = dispatch_sample_creation_from(&app, BROKEN_XY_FIXTURES, WelcomeArchetypeChoice::Xy, &state.schema, &mut intents);
        let msg = res.expect_err("unknown fixture must fail");
        assert!(msg.starts_with(tr(&app.locale, "welcome.sample.load_failed")), "{msg}");
        assert!(intents.intents().is_empty());
    }

    #[test]
    fn primary_action_in_sample_mode_clears_stale_error_and_queues_group() {
        let app = UiApp::new();
        let mut state = WelcomeHubEguiState {
            left_mode: WelcomeLeftMode::NewTableArchetype(WelcomeArchetypeChoice::Xy),
            sample_error: Some(String::from("stale")),
            ..Default::default()
        };
        state.schema.entry_mode_sample = true;
        let mut intents = PendingUiIntents::default();
        dispatch_welcome_primary_action(&app, &mut state, &mut intents);
        assert_eq!(state.sample_error, None);
        assert!(matches!(intents.intents().first(), Some(UiIntent::EngineCommandGroup { .. })));
    }

    #[test]
    fn normal_sample_still_loads_for_every_archetype() {
        let app = UiApp::new();
        for arch in [
            WelcomeArchetypeChoice::Xy,
            WelcomeArchetypeChoice::Column,
            WelcomeArchetypeChoice::Grouped,
            WelcomeArchetypeChoice::Contingency,
            WelcomeArchetypeChoice::Survival,
            WelcomeArchetypeChoice::PartsOfWhole,
            WelcomeArchetypeChoice::MultipleVariables,
            WelcomeArchetypeChoice::Nested,
        ] {
            let schema = WelcomeSchemaDraft { entry_mode_sample: true, ..Default::default() };
            let mut intents = PendingUiIntents::default();
            assert_eq!(dispatch_sample_creation(&app, arch, &schema, &mut intents), Ok(()));
            assert!(matches!(intents.intents().first(), Some(UiIntent::EngineCommandGroup { .. })));
        }
    }
}
