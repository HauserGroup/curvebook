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
use crate::grid_view::ImportOriginKind;
use crate::i18n::{table_kind_label, tr_or_fallback};
use crate::inspect::ZLayer;
use crate::tr;
use crate::workspace_nav::{SAMPLE_FIXTURES, SampleFixtureDescriptor, instantiate_sample_fixture, sample_fixture_instructional_note};

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

    /// Primary core table types shown first inside **New Data Table** (`R10.37`).
    pub const PRIMARY: [Self; 7] = [Self::Xy, Self::Column, Self::Grouped, Self::Contingency, Self::Survival, Self::MultipleVariables, Self::Nested];

    /// Additional table types offered under the **Additional table types** disclosure (`R10.37`).
    pub const ADDITIONAL: [Self; 1] = [Self::PartsOfWhole];

    /// Whether this archetype belongs under **Additional table types**.
    #[must_use]
    pub const fn is_additional(self) -> bool {
        matches!(self, Self::PartsOfWhole)
    }

    /// Map a [`curvebook_doc::TableKind`] to its corresponding [`WelcomeArchetypeChoice`].
    #[must_use]
    pub const fn from_table_kind(kind: TableKind) -> Self {
        match kind {
            TableKind::Xy => Self::Xy,
            TableKind::Column => Self::Column,
            TableKind::Grouped => Self::Grouped,
            TableKind::Contingency => Self::Contingency,
            TableKind::Survival => Self::Survival,
            TableKind::PartsOfWhole => Self::PartsOfWhole,
            TableKind::MultipleVariables => Self::MultipleVariables,
            TableKind::Nested => Self::Nested,
        }
    }

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

/// Data import route selectable inside `WelcomeLeftMode::ImportData` (`R10.37`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WelcomeImportSource {
    #[default]
    XlsxWorkbook,
    DelimitedWizard,
    Pzfx,
    ArchiveFile,
    PlateOrTidy,
}

impl WelcomeImportSource {
    /// Ordered list of import routes surfaced in the Welcome **Import** pane.
    pub const ALL: [Self; 5] = [Self::XlsxWorkbook, Self::DelimitedWizard, Self::Pzfx, Self::ArchiveFile, Self::PlateOrTidy];

    /// Inspector widget identifier for this import source option.
    #[must_use]
    pub const fn widget_id(self) -> &'static str {
        match self {
            Self::XlsxWorkbook => "welcome.import.xlsx",
            Self::DelimitedWizard => "welcome.import.delimited",
            Self::Pzfx => "welcome.import.pzfx",
            Self::ArchiveFile => "welcome.import.archive",
            Self::PlateOrTidy => "welcome.import.reshape",
        }
    }

    /// Localized display label for this import source.
    #[must_use]
    pub fn label(self, locale: &str) -> &'static str {
        match self {
            Self::XlsxWorkbook => tr(locale, "cmd.table.importXlsx"),
            Self::DelimitedWizard => tr(locale, "welcome.import.delimited"),
            Self::Pzfx => tr(locale, "cmd.table.importPzfx"),
            Self::ArchiveFile => tr(locale, "cmd.table.importPrism"),
            Self::PlateOrTidy => tr(locale, "menu.file.import_reshape"),
        }
    }

    /// Concise localized summary shown below the import source list.
    #[must_use]
    pub fn summary(self, locale: &str) -> &'static str {
        match self {
            Self::XlsxWorkbook => tr(locale, "welcome.import.xlsx_summary"),
            Self::DelimitedWizard => tr(locale, "welcome.import.delimited_summary"),
            Self::Pzfx => tr(locale, "welcome.import.pzfx_summary"),
            Self::ArchiveFile => tr(locale, "welcome.import.prism_summary"),
            Self::PlateOrTidy => tr(locale, "welcome.import.reshape_summary"),
        }
    }

    /// Underlying engine or UI command identifier for inspection metadata.
    #[must_use]
    pub const fn command_id(self) -> Option<&'static str> {
        match self {
            Self::XlsxWorkbook => Some("table.importXlsx"),
            Self::DelimitedWizard => Some("table.importCsv"),
            Self::Pzfx => Some("table.importPzfx"),
            Self::ArchiveFile => Some("table.importPrism"),
            Self::PlateOrTidy => Some("ui.import.reshape.open"),
        }
    }

    /// Whether this route opens an external file directly and is therefore blocked under `RejectExternal`.
    #[must_use]
    pub const fn requires_external_file(self) -> bool {
        matches!(self, Self::XlsxWorkbook | Self::Pzfx | Self::ArchiveFile)
    }
}

/// Active left-rail entry inside the 3-pane Welcome & Workspace Hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WelcomeLeftMode {
    OpenFile,
    ImportData,
    NewTableArchetype(WelcomeArchetypeChoice),
    SampleData,
    StarterTemplates,
    ClonePipeline,
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
    pub include_sample_notes: bool,
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
    pub clone_override_replicates: bool,
    pub clone_override_replicates_count: usize,
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
            include_sample_notes: false,
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
            clone_override_replicates: false,
            clone_override_replicates_count: 3,
        }
    }
}

/// Persistent `egui_shell` state for the 3-pane Welcome & Workspace Hub.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WelcomeHubEguiState {
    pub left_mode: WelcomeLeftMode,
    pub schema: WelcomeSchemaDraft,
    /// Selected import format when [`WelcomeLeftMode::ImportData`] is active (`R10.37`).
    pub import_source: WelcomeImportSource,
    /// Whether the **Additional table types** disclosure (containing **Parts of Whole**) is expanded (`R10.37`).
    pub additional_table_types_open: bool,
    /// Remembers whether **Reuse an Analysis** last showed **Clone Pipeline** (`true`) or **New from Template** (`false`).
    pub reuse_prefers_clone: bool,
    pub clone_options_open: bool,
    pub open_file_path_draft: String,
    pub open_file_request_count: u64,
    pub template_error: Option<String>,
    /// Inline error from the last failed `Load Sample` attempt (cleared on any sample-mode change).
    pub sample_error: Option<String>,
    /// Inline error from the last failed or policy-rejected `Open` / `Import` attempt.
    pub open_file_error: Option<String>,
    /// Snapshot of `app.mutation_seq()` when `project.open` or an import command was dispatched; closes launcher only once it succeeds.
    pub pending_open_mutation_seq: Option<u64>,
    /// `egui` frame counter when `pending_open_mutation_seq` was armed, so a cancelled native picker disarms on the next settled frame.
    pub pending_open_frame_nr: u64,
    /// Snapshot of `app.canvas_dialogs.status_banner` before dispatch so a stale pre-existing banner is never rendered as an error.
    pub pending_open_prior_banner: Option<String>,
    /// Whether the armed open/import dispatch came from `welcome.btn.open_template_file` (routes errors to `template_error`).
    pub pending_open_is_template: bool,
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
    intents: &'a mut PendingUiIntents,
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
    disabled_reason: Option<&'a str>,
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

fn clear_pending_open(state: &mut WelcomeHubEguiState) {
    state.pending_open_mutation_seq = None;
    state.pending_open_frame_nr = 0;
    state.pending_open_prior_banner = None;
    state.pending_open_is_template = false;
}

fn begin_pending_open(app: &UiApp, state: &mut WelcomeHubEguiState, frame_nr: u64, is_template: bool) {
    state.pending_open_mutation_seq = Some(app.mutation_seq());
    state.pending_open_frame_nr = frame_nr;
    state.pending_open_prior_banner = app.canvas_dialogs.status_banner.clone();
    state.pending_open_is_template = is_template;
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
            clear_pending_open(state);
            state.open_file_error = None;
            state.template_error = None;
            intents.push(UiIntent::SetLauncherOpen { open: false });
        } else if !ui.ctx().will_discard() && ui.ctx().cumulative_frame_nr() > state.pending_open_frame_nr {
            let new_banner = app.canvas_dialogs.status_banner.as_ref().filter(|b| state.pending_open_prior_banner.as_ref() != Some(*b)).cloned();
            if let Some(banner) = new_banner {
                if state.pending_open_is_template {
                    state.template_error = Some(banner);
                } else {
                    state.open_file_error = Some(banner);
                }
                clear_pending_open(state);
            } else if matches!(app.canvas_dialogs.active_dialog, crate::dialogs_canvas::ActiveDialog::None) {
                clear_pending_open(state);
            }
        }
    }

    let stage_rect = ui.max_rect();
    if !stage_rect.is_positive() {
        return report;
    }

    if !empty_project {
        ui.painter().rect_filled(stage_rect, CornerRadius::ZERO, tokens.scrim);
        if enabled && !state.clone_options_open && ui.ctx().input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            clear_pending_open(state);
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
    render_middle_pane(&mut mid_ui, app, state, tokens, card_enabled, card_z, meta, intents);

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

fn source_sheet_replicate_count(ds: &DataSheet) -> Option<(usize, std::ops::RangeInclusive<usize>)> {
    match &ds.archetype {
        TableArchetype::Xy(t) => Some((t.y_subcolumn_mode.subcolumn_count().clamp(1, 256), 1..=256)),
        TableArchetype::Grouped(t) => Some((t.subcolumn_mode.subcolumn_count().clamp(1, 256), 1..=256)),
        TableArchetype::Nested(t) => {
            let units = t.groups.first().map_or(3, |g| g.biological_unit_count()).clamp(2, 64);
            Some((units, 2..=64))
        }
        TableArchetype::Column(_)
        | TableArchetype::Contingency(_)
        | TableArchetype::Survival(_)
        | TableArchetype::PartsOfWhole(_)
        | TableArchetype::MultipleVariables(_) => None,
    }
}

fn clone_override_replicates_capability(ds: &DataSheet, clear_y_values: bool) -> Result<std::ops::RangeInclusive<usize>, &'static str> {
    match &ds.archetype {
        TableArchetype::Xy(t) => {
            if t.y_subcolumn_mode.is_pre_summarized() && !clear_y_values {
                Err("Enable Clear Y response cells to convert pre-summarized subcolumns into raw replicates")
            } else {
                Ok(1..=256)
            }
        }
        TableArchetype::Grouped(t) => {
            if t.subcolumn_mode.is_pre_summarized() && !clear_y_values {
                Err("Enable Clear Y response cells to convert pre-summarized subcolumns into raw replicates")
            } else {
                Ok(1..=256)
            }
        }
        TableArchetype::Nested(_) => Ok(2..=64),
        TableArchetype::Column(_)
        | TableArchetype::Contingency(_)
        | TableArchetype::Survival(_)
        | TableArchetype::PartsOfWhole(_)
        | TableArchetype::MultipleVariables(_) => Err("Replicate subcolumn overrides only apply to XY, Grouped, and Nested tables"),
    }
}

fn sync_clone_replicates_for_source(schema: &mut WelcomeSchemaDraft, ds: &DataSheet) {
    if let Some((count, _range)) = source_sheet_replicate_count(ds) {
        schema.clone_override_replicates_count = count;
    }
    if clone_override_replicates_capability(ds, schema.clone_clear_values).is_err() {
        schema.clone_override_replicates = false;
    }
}

fn format_clone_pipeline_summary(doc: &ProjectDocument, schema: &WelcomeSchemaDraft, locale: &str) -> String {
    let override_suffix = schema
        .clone_source_sheet_id
        .as_ref()
        .and_then(|id| doc.data_sheet(id))
        .and_then(|ds| clone_override_replicates_capability(ds, schema.clone_clear_values).ok())
        .filter(|_| schema.clone_override_replicates)
        .map(|range| {
            let reps = schema.clone_override_replicates_count.clamp(*range.start(), *range.end());
            format!(" • {}: {}", tr_or_fallback(locale, "welcome.clone.replicates_count", "Replicates"), reps)
        })
        .unwrap_or_default();
    format!(
        "Links: {} • Data: {} • Analyses: {}{}",
        if schema.clone_include_downstream { "Preserve" } else { "Table only" },
        if schema.clone_clear_values { "Clear" } else { "Copy" },
        if schema.clone_rerun_analyses { "Re-run" } else { "Deferred" },
        override_suffix,
    )
}

fn ensure_clone_source_default(app: &UiApp, state: &mut WelcomeHubEguiState) {
    let doc = app.document();
    let valid_existing = state.schema.clone_source_sheet_id.as_ref().is_some_and(|id| doc.data_sheet(id).is_some());
    if !valid_existing {
        let active_ds = doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id));
        let chosen = active_ds.or_else(|| doc.data_sheets.first());
        state.schema.clone_source_sheet_id = chosen.map(|ds| ds.id.clone());
        if let Some(ds) = chosen {
            sync_clone_replicates_for_source(&mut state.schema, ds);
        } else {
            state.schema.clone_override_replicates = false;
        }
    } else if let Some(ds) = state.schema.clone_source_sheet_id.as_ref().and_then(|id| doc.data_sheet(id)) {
        match clone_override_replicates_capability(ds, state.schema.clone_clear_values) {
            Ok(range) => {
                state.schema.clone_override_replicates_count = state.schema.clone_override_replicates_count.clamp(*range.start(), *range.end());
            }
            Err(_) => {
                state.schema.clone_override_replicates = false;
            }
        }
    }
}

fn render_card_header(ui: &mut egui::Ui, app: &UiApp, tokens: &Tokens, z_layer: ZLayer, meta: &mut FrameInspectMeta) {
    let title_txt = tr(&app.locale, "welcome.title");
    let sub_txt = tr(&app.locale, "welcome.subtitle");
    let avail_w = ui.available_width();
    ui.horizontal_centered(|ui| {
        let mark_size = crate::egui_shell::brand::WELCOME_MARK_SIZE;
        let (mark_rect, _) = ui.allocate_exact_size(mark_size, Sense::hover());
        crate::egui_shell::brand::paint_mark(ui, mark_rect);
        let text_w = (avail_w - mark_size.x - ui.spacing().item_spacing.x).max(80.0);
        ui.vertical(|ui| {
            ui.set_max_width(text_w);
            render_clipped_text_label(ui, "welcome.header.title", title_txt, FontId::proportional(15.5), tokens.text_strong, z_layer, meta);
            render_clipped_text_label(ui, "welcome.header.subtitle", sub_txt, FontId::proportional(11.5), tokens.text_muted, z_layer, meta);
        });
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RailModeGlyph {
    Open,
    Import,
    NewTable,
    SampleData,
    Reuse,
}

fn apply_archetype_selection(state: &mut WelcomeHubEguiState, arch: WelcomeArchetypeChoice) {
    clear_pending_open(state);
    state.left_mode = WelcomeLeftMode::NewTableArchetype(arch);
    state.schema.archetype = arch;
    state.schema.entry_mode_sample = false;
    state.sample_error = None;
    if arch.is_additional() {
        state.additional_table_types_open = true;
    }
    if matches!(arch, WelcomeArchetypeChoice::PartsOfWhole) {
        state.schema.cols = 1;
    } else if state.schema.cols < 2 {
        state.schema.cols = 3;
    }
    if matches!(arch, WelcomeArchetypeChoice::Grouped) && state.schema.y_subcolumn_kind == 0 {
        state.schema.y_subcolumn_kind = 1;
    }
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
    ui.painter().rect_filled(outer, CornerRadius::same(tokens.metrics.radius_lg), tokens.panel_bg);
    ui.painter().rect_stroke(outer, CornerRadius::same(tokens.metrics.radius_lg), Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
    let inner = outer.shrink(6.0);
    let mut rail_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    rail_ui.set_clip_rect(inner);
    rail_ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

    let max_w = inner.width().max(72.0);
    let has_data = !app.document().data_sheets.is_empty();
    egui::ScrollArea::vertical().id_salt("welcome_left_rail_scroll").auto_shrink([false, false]).show(&mut rail_ui, |ui| {
        let row_w = (ui.available_width() - 12.0).clamp(64.0, max_w);
        render_clipped_text_label(
            ui,
            "welcome.rail.start",
            tr(locale, "welcome.rail.start"),
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            tokens.text_muted,
            z_layer,
            meta,
        );
        ui.add_space(tokens.metrics.space_xs);

        // 1. Open
        let open_sel = matches!(state.left_mode, WelcomeLeftMode::OpenFile);
        if render_rail_mode_button(
            ui,
            row_w,
            RailModeGlyph::Open,
            tokens,
            meta,
            LabelSpec {
                widget_id: "welcome.action.open_existing",
                role: "tab",
                a11y_role: egui::Role::Tab,
                label: tr(locale, "menu.file.open"),
                command_id: Some("project.open"),
                selected: open_sel,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            clear_pending_open(state);
            state.left_mode = WelcomeLeftMode::OpenFile;
            state.open_file_error = None;
        }
        ui.add_space(tokens.metrics.space_xxs);

        // 2. Import
        let import_sel = matches!(state.left_mode, WelcomeLeftMode::ImportData);
        if render_rail_mode_button(
            ui,
            row_w,
            RailModeGlyph::Import,
            tokens,
            meta,
            LabelSpec {
                widget_id: "welcome.mode.import",
                role: "tab",
                a11y_role: egui::Role::Tab,
                label: tr(locale, "menu.file.import"),
                command_id: None,
                selected: import_sel,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            clear_pending_open(state);
            state.left_mode = WelcomeLeftMode::ImportData;
            state.open_file_error = None;
        }
        ui.add_space(tokens.metrics.space_xxs);

        // 3. New Data Table
        let new_table_sel = matches!(state.left_mode, WelcomeLeftMode::NewTableArchetype(_));
        if render_rail_mode_button(
            ui,
            row_w,
            RailModeGlyph::NewTable,
            tokens,
            meta,
            LabelSpec {
                widget_id: "welcome.mode.new_table",
                role: "tab",
                a11y_role: egui::Role::Tab,
                label: tr(locale, "welcome.mode.new_table"),
                command_id: None,
                selected: new_table_sel,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            apply_archetype_selection(state, state.schema.archetype);
        }
        ui.add_space(tokens.metrics.space_xxs);

        // 4. Sample Data
        let sample_sel = matches!(state.left_mode, WelcomeLeftMode::SampleData);
        if render_rail_mode_button(
            ui,
            row_w,
            RailModeGlyph::SampleData,
            tokens,
            meta,
            LabelSpec {
                widget_id: "welcome.mode.sample_data",
                role: "tab",
                a11y_role: egui::Role::Tab,
                label: tr(locale, "welcome.mode.sample_data"),
                command_id: None,
                selected: sample_sel,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            clear_pending_open(state);
            state.left_mode = WelcomeLeftMode::SampleData;
            state.sample_error = None;
        }

        ui.add_space(tokens.metrics.space_md);
        ui.painter().hline(ui.available_rect_before_wrap().x_range(), ui.cursor().top(), Stroke::new(1.0, tokens.divider));
        ui.add_space(tokens.metrics.space_md);

        // 5. Reuse an Analysis (groups New from Template + Clone Pipeline)
        let reuse_sel = matches!(state.left_mode, WelcomeLeftMode::StarterTemplates | WelcomeLeftMode::ClonePipeline);
        if render_rail_mode_button(
            ui,
            row_w,
            RailModeGlyph::Reuse,
            tokens,
            meta,
            LabelSpec {
                widget_id: "welcome.mode.reuse",
                role: "tab",
                a11y_role: egui::Role::Tab,
                label: tr(locale, "welcome.mode.reuse"),
                command_id: None,
                selected: reuse_sel,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            clear_pending_open(state);
            if !reuse_sel {
                if state.reuse_prefers_clone && has_data {
                    state.left_mode = WelcomeLeftMode::ClonePipeline;
                    ensure_clone_source_default(app, state);
                } else {
                    state.left_mode = WelcomeLeftMode::StarterTemplates;
                }
            }
            state.template_error = None;
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn render_middle_pane(
    ui: &mut egui::Ui,
    app: &UiApp,
    state: &mut WelcomeHubEguiState,
    tokens: &Tokens,
    enabled: bool,
    z_layer: ZLayer,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    egui::ScrollArea::vertical().id_salt("welcome_mid_pane_scroll").auto_shrink([false, false]).show(ui, |ui| {
        let content_w = (ui.available_width() - 12.0).max(120.0);
        ui.set_max_width(content_w);

        let left_mode = state.left_mode;
        let mut cx = WelcomeMiddleCtx { app, state, tokens, enabled, z_layer, meta, intents, content_w };
        match left_mode {
            WelcomeLeftMode::OpenFile => {
                render_middle_open_file(ui, &mut cx);
            }
            WelcomeLeftMode::ImportData => {
                render_middle_import_data(ui, &mut cx);
            }
            WelcomeLeftMode::NewTableArchetype(arch) => {
                render_middle_new_table(ui, &mut cx, arch);
            }
            WelcomeLeftMode::SampleData => {
                render_middle_sample_data(ui, &mut cx);
            }
            WelcomeLeftMode::StarterTemplates | WelcomeLeftMode::ClonePipeline => {
                render_middle_reuse_analysis(ui, &mut cx);
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
    let row_h = tokens.metrics.min_hit_target_pt;

    render_clipped_text_label(
        ui,
        "welcome.section.table_type",
        tr(locale, "welcome.section.table_type"),
        FontId::proportional(tokens.metrics.font_ui_body_pt),
        tokens.text_strong,
        z_layer,
        cx.meta,
    );
    ui.add_space(tokens.metrics.space_xs);

    let chip_w = ((content_w - 6.0) * 0.5).max(56.0);
    for pair in WelcomeArchetypeChoice::PRIMARY.chunks(2) {
        if rect_visible_in_clip(ui, Vec2::new(content_w, row_h)) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for &choice in pair {
                    let selected = arch == choice;
                    if render_arch_rail_button(
                        ui,
                        tokens,
                        cx.meta,
                        ArchRailButtonSpec { arch: choice, width: chip_w, selected, enabled, locale, z_layer },
                    ) && enabled
                    {
                        apply_archetype_selection(cx.state, choice);
                    }
                }
            });
        } else {
            ui.allocate_space(Vec2::new(content_w, row_h));
        }
        ui.add_space(tokens.metrics.space_xxs);
    }

    if render_disclosure_button(
        ui,
        content_w,
        tokens,
        cx.meta,
        "welcome.arch.additional_toggle",
        tr(locale, "welcome.section.additional_tables"),
        cx.state.additional_table_types_open,
        enabled,
        z_layer,
    ) && enabled
    {
        cx.state.additional_table_types_open = !cx.state.additional_table_types_open;
    }

    if cx.state.additional_table_types_open {
        ui.add_space(tokens.metrics.space_xxs);
        for choice in WelcomeArchetypeChoice::ADDITIONAL {
            let selected = arch == choice;
            if render_arch_rail_button(ui, tokens, cx.meta, ArchRailButtonSpec { arch: choice, width: content_w, selected, enabled, locale, z_layer })
                && enabled
            {
                apply_archetype_selection(cx.state, choice);
            }
            ui.add_space(tokens.metrics.space_xxs);
        }
    }

    ui.add_space(tokens.metrics.space_xs);
    render_plain_clipped_label(ui, tr(locale, "welcome.field.title"), FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
    render_clipped_text_edit(
        ui,
        &mut cx.state.schema.title,
        tokens,
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
    ui.add_space(tokens.metrics.space_xs);

    let active_arch = match cx.state.left_mode {
        WelcomeLeftMode::NewTableArchetype(a) => a,
        _ => arch,
    };
    render_archetype_schema_controls(ui, cx, active_arch);
}

fn render_middle_import_data(ui: &mut egui::Ui, cx: &mut WelcomeMiddleCtx<'_>) {
    let locale = cx.app.locale.as_str();
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let z_layer = cx.z_layer;
    let content_w = cx.content_w;
    let external_rejected = matches!(cx.app.session().path_policy(), curvebook_engine::ExternalPathPolicy::RejectExternal);

    render_clipped_text_label(
        ui,
        "welcome.import.heading",
        tr(locale, "welcome.import.heading"),
        FontId::proportional(tokens.metrics.font_ui_body_pt),
        tokens.text_strong,
        z_layer,
        cx.meta,
    );
    ui.add_space(tokens.metrics.space_xs);
    render_clipped_text_label(
        ui,
        "welcome.import.subtitle",
        tr(locale, "welcome.import.subtitle"),
        FontId::proportional(tokens.metrics.font_ui_small_pt),
        tokens.text_muted,
        z_layer,
        cx.meta,
    );
    ui.add_space(tokens.metrics.space_xs);

    for src in WelcomeImportSource::ALL {
        let selected = cx.state.import_source == src;
        if render_pill_button(
            ui,
            content_w,
            tokens,
            cx.meta,
            LabelSpec {
                widget_id: src.widget_id(),
                role: "radio",
                a11y_role: egui::Role::RadioButton,
                label: src.label(locale),
                command_id: None,
                selected,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            cx.state.import_source = src;
            cx.state.open_file_error = None;
        }
        ui.add_space(tokens.metrics.space_xxs);
    }

    ui.add_space(tokens.metrics.space_xs);
    render_plain_clipped_label(ui, cx.state.import_source.summary(locale), FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);

    if external_rejected && cx.state.import_source.requires_external_file() {
        ui.add_space(tokens.metrics.space_xs);
        let notice = "External file access is disabled by host security policy.";
        let resp = ui.add(egui::Label::new(egui::RichText::new(notice).small().color(tokens.text_muted)).wrap());
        cx.meta.record_response(
            ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: "welcome.import.policy_notice",
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

    if let Some(err_msg) = cx.state.open_file_error.as_deref() {
        ui.add_space(tokens.metrics.space_xs);
        let err_resp = ui.add(egui::Label::new(egui::RichText::new(err_msg).small().color(tokens.link_red)).wrap());
        cx.meta.record_response(
            ui,
            &err_resp,
            FrameInspectEntryInput {
                widget_id: "welcome.import.error",
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

fn render_middle_sample_data(ui: &mut egui::Ui, cx: &mut WelcomeMiddleCtx<'_>) {
    let locale = cx.app.locale.as_str();
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let z_layer = cx.z_layer;
    let content_w = cx.content_w;

    render_clipped_text_label(
        ui,
        "welcome.sample.heading",
        tr(locale, "welcome.mode.sample_data"),
        FontId::proportional(tokens.metrics.font_ui_body_pt),
        tokens.text_strong,
        z_layer,
        cx.meta,
    );
    ui.add_space(tokens.metrics.space_xs);
    render_clipped_text_label(
        ui,
        "welcome.sample.subtitle",
        tr(locale, "welcome.sample.subtitle"),
        FontId::proportional(tokens.metrics.font_ui_small_pt),
        tokens.text_muted,
        z_layer,
        cx.meta,
    );
    ui.add_space(tokens.metrics.space_xs);

    let sel_idx = cx.state.schema.selected_sample_index.min(SAMPLE_FIXTURES.len().saturating_sub(1));
    for (idx, fx) in SAMPLE_FIXTURES.iter().enumerate() {
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
            cx.state.schema.archetype = WelcomeArchetypeChoice::from_table_kind(fx.kind);
            cx.state.sample_error = None;
        }
        ui.add_space(tokens.metrics.space_xxs);
    }

    let chosen_hint = if let Some(chosen) = SAMPLE_FIXTURES.get(sel_idx) {
        ui.add_space(tokens.metrics.space_xs);
        let detail = format!("{}: {}", table_kind_label(locale, chosen.kind), chosen.summary);
        render_plain_clipped_label(ui, &detail, FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
        chosen.title
    } else {
        ""
    };

    if !SAMPLE_FIXTURES.is_empty() {
        ui.add_space(tokens.metrics.space_xs);
        if render_check_toggle(
            ui,
            tokens,
            cx.meta,
            CheckToggleSpec {
                widget_id: "welcome.toggle.sample_notes",
                label: tr_or_fallback(locale, "welcome.toggle.sample_notes", "Include instructional note"),
                checked: cx.state.schema.include_sample_notes,
                enabled,
                disabled_reason: None,
                z_layer,
            },
        ) && enabled
        {
            cx.state.schema.include_sample_notes = !cx.state.schema.include_sample_notes;
        }
    }

    if let Some(err_msg) = cx.state.sample_error.as_deref() {
        ui.add_space(tokens.metrics.space_xs);
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

    ui.add_space(tokens.metrics.space_xs);
    render_plain_clipped_label(ui, tr(locale, "welcome.field.title"), FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
    render_clipped_text_edit(
        ui,
        &mut cx.state.schema.title,
        tokens,
        cx.meta,
        TextEditSpec {
            widget_id: "welcome.input.title",
            label: tr(locale, "welcome.field.title"),
            hint: chosen_hint,
            width: (content_w - 8.0).max(80.0),
            enabled,
            z_layer,
        },
    );
}

fn render_middle_reuse_analysis(ui: &mut egui::Ui, cx: &mut WelcomeMiddleCtx<'_>) {
    let locale = cx.app.locale.as_str();
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let z_layer = cx.z_layer;
    let content_w = cx.content_w;
    let row_h = tokens.metrics.min_hit_target_pt;
    let has_data = !cx.app.document().data_sheets.is_empty();

    render_clipped_text_label(
        ui,
        "welcome.reuse.heading",
        tr(locale, "welcome.mode.reuse"),
        FontId::proportional(tokens.metrics.font_ui_body_pt),
        tokens.text_strong,
        z_layer,
        cx.meta,
    );
    ui.add_space(tokens.metrics.space_xxs);
    render_clipped_text_label(
        ui,
        "welcome.reuse.subtitle",
        tr(locale, "welcome.reuse.subtitle"),
        FontId::proportional(tokens.metrics.font_ui_small_pt),
        tokens.text_muted,
        z_layer,
        cx.meta,
    );
    ui.add_space(tokens.metrics.space_xs);

    let seg_w = ((content_w - 6.0) * 0.5).max(52.0);
    let tpl_sel = matches!(cx.state.left_mode, WelcomeLeftMode::StarterTemplates);
    let clone_sel = matches!(cx.state.left_mode, WelcomeLeftMode::ClonePipeline);
    let clone_enabled = enabled && has_data;
    if rect_visible_in_clip(ui, Vec2::new(content_w, row_h)) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if render_pill_button(
                ui,
                seg_w,
                tokens,
                cx.meta,
                LabelSpec {
                    widget_id: "welcome.mode.template",
                    role: "tab",
                    a11y_role: egui::Role::Tab,
                    label: tr(locale, "welcome.reuse.new_from_template"),
                    command_id: None,
                    selected: tpl_sel,
                    enabled,
                    disabled_reason: None,
                    z_layer,
                },
            ) && enabled
            {
                clear_pending_open(cx.state);
                cx.state.left_mode = WelcomeLeftMode::StarterTemplates;
                cx.state.reuse_prefers_clone = false;
                cx.state.template_error = None;
            }
            if render_pill_button(
                ui,
                seg_w,
                tokens,
                cx.meta,
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
                clear_pending_open(cx.state);
                cx.state.left_mode = WelcomeLeftMode::ClonePipeline;
                cx.state.reuse_prefers_clone = true;
                ensure_clone_source_default(cx.app, cx.state);
            }
        });
    } else {
        ui.allocate_space(Vec2::new(content_w, row_h));
    }
    ui.add_space(tokens.metrics.space_md);

    match cx.state.left_mode {
        WelcomeLeftMode::ClonePipeline => render_middle_clone_pipeline(ui, cx),
        _ => render_middle_starter_templates(ui, cx),
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
            let row_h = tokens.metrics.min_hit_target_pt;
            for chunk in [0usize..2, 2usize..4, 4usize..5] {
                if rect_visible_in_clip(ui, Vec2::new(content_w, row_h)) {
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
                    ui.allocate_space(Vec2::new(content_w, row_h));
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
            let row_h = tokens.metrics.min_hit_target_pt;
            for v_idx in 0..shown_vars {
                if rect_visible_in_clip(ui, Vec2::new(content_w, row_h)) {
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
                    ui.allocate_space(Vec2::new(content_w, row_h));
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
    let row_h = tokens.metrics.min_hit_target_pt;
    for pair in sub_opts.chunks(2) {
        if rect_visible_in_clip(ui, Vec2::new(content_w, row_h)) {
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
            ui.allocate_space(Vec2::new(content_w, row_h));
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
    let app = cx.app;
    let locale = app.locale.as_str();
    let tokens = cx.tokens;
    let enabled = cx.enabled;
    let z_layer = cx.z_layer;
    let content_w = cx.content_w;
    let external_rejected = matches!(app.session().path_policy(), curvebook_engine::ExternalPathPolicy::RejectExternal);
    let state = &mut *cx.state;
    let meta = &mut *cx.meta;
    let intents = &mut *cx.intents;

    render_clipped_text_label(
        ui,
        "welcome.template.heading",
        tr(locale, "welcome.template.heading"),
        FontId::proportional(tokens.metrics.font_ui_small_pt),
        tokens.text_muted,
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
        tokens,
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
    ui.add_space(6.0);
    let open_tpl_enabled = enabled && !external_rejected;
    if render_pill_button(
        ui,
        content_w.min(200.0),
        tokens,
        meta,
        LabelSpec {
            widget_id: "welcome.btn.open_template_file",
            role: "button",
            a11y_role: egui::Role::Button,
            label: tr(locale, "welcome.btn.open_template_file"),
            command_id: None,
            selected: false,
            enabled: open_tpl_enabled,
            disabled_reason: external_rejected.then_some("External file access is disabled by host security policy"),
            z_layer,
        },
    ) && open_tpl_enabled
    {
        state.template_error = None;
        state.open_file_error = None;
        begin_pending_open(app, state, ui.ctx().cumulative_frame_nr(), true);
        intents.push_engine("project.instantiateTemplate", json!({}));
    }
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
        FontId::proportional(tokens.metrics.font_ui_small_pt),
        tokens.text_muted,
        z_layer,
        meta,
    );
    ui.add_space(4.0);

    if app.document().data_sheets.is_empty() {
        render_plain_clipped_label(
            ui,
            "Create or open a Data table before cloning a pipeline.",
            FontId::proportional(tokens.metrics.font_ui_small_pt),
            tokens.text_muted,
        );
        return;
    }

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
            && state.schema.clone_source_sheet_id.as_ref() != Some(&ds.id)
        {
            state.schema.clone_source_sheet_id = Some(ds.id.clone());
            sync_clone_replicates_for_source(&mut state.schema, ds);
        }
        ui.add_space(2.0);
    }

    ui.add_space(6.0);
    render_plain_clipped_label(ui, tr(locale, "welcome.field.title"), FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted);
    render_clipped_text_edit(
        ui,
        &mut state.schema.title,
        tokens,
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
    let summary = format_clone_pipeline_summary(app.document(), &state.schema, locale);
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
            label: "Clone Options",
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
        tokens,
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
    enabled: bool,
    z_layer: ZLayer,
    meta: &mut FrameInspectMeta,
) -> Option<Rect> {
    let locale = &app.locale;
    let mut schematic_rect = None;
    let show_auto_options = matches!(state.left_mode, WelcomeLeftMode::NewTableArchetype(_) | WelcomeLeftMode::SampleData);
    egui::ScrollArea::vertical().id_salt("welcome_right_pane_scroll").auto_shrink([false, false]).show(ui, |ui| {
        let preview_w = (ui.available_width() - 12.0).max(96.0);
        ui.set_max_width(preview_w);
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
        let box_h =
            if show_auto_options { (ui.available_height() - 122.0).clamp(72.0, 148.0) } else { (ui.available_height() - 54.0).clamp(84.0, 176.0) };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(preview_w, box_h), Sense::hover());
        schematic_rect = Some(rect);

        ui.painter().rect_filled(rect, CornerRadius::same(tokens.metrics.radius_lg), tokens.paper_white);
        ui.painter().rect_stroke(rect, CornerRadius::same(tokens.metrics.radius_lg), Stroke::new(1.0, tokens.border), StrokeKind::Inside);

        let inset = tokens.metrics.space_xs + tokens.metrics.space_xxs;
        let split_x = rect.left() + rect.width() * 0.56;
        let table_rect = Rect::from_min_max(rect.min + Vec2::splat(inset), Pos2::new(split_x - 3.0, rect.bottom() - inset));
        let plot_rect = Rect::from_min_max(Pos2::new(split_x + 3.0, rect.top() + inset), rect.max - Vec2::splat(inset));
        draw_vector_table_schematic(ui.painter(), table_rect, arch, &state.schema, tokens);
        draw_vector_plot_preview(ui.painter(), plot_rect, arch, &state.schema, tokens);

        ui.add_space(6.0);
        let pipeline = match state.left_mode {
            WelcomeLeftMode::OpenFile => tr(locale, "welcome.open.pipeline_label"),
            WelcomeLeftMode::ImportData => tr(locale, "welcome.import.pipeline_label"),
            WelcomeLeftMode::ClonePipeline => {
                if state.schema.clone_include_downstream {
                    "Data -> Analyses -> Graphs -> Layouts"
                } else {
                    "Data only"
                }
            }
            WelcomeLeftMode::StarterTemplates => "Data -> Info -> Linked Graph",
            WelcomeLeftMode::NewTableArchetype(_) | WelcomeLeftMode::SampleData => match (state.schema.auto_info, state.schema.auto_graph) {
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

        if show_auto_options {
            ui.add_space(tokens.metrics.space_md);
            render_clipped_text_label(
                ui,
                "welcome.section.options",
                tr(locale, "welcome.section.options"),
                FontId::proportional(tokens.metrics.font_ui_small_pt),
                tokens.text_muted,
                z_layer,
                meta,
            );
            if render_check_toggle(
                ui,
                tokens,
                meta,
                CheckToggleSpec {
                    widget_id: "welcome.toggle.auto_graph",
                    label: tr(locale, "welcome.toggle.auto_graph"),
                    checked: state.schema.auto_graph,
                    enabled,
                    disabled_reason: None,
                    z_layer,
                },
            ) && enabled
            {
                state.schema.auto_graph = !state.schema.auto_graph;
            }
            if render_check_toggle(
                ui,
                tokens,
                meta,
                CheckToggleSpec {
                    widget_id: "welcome.toggle.auto_info",
                    label: tr(locale, "welcome.toggle.auto_info"),
                    checked: state.schema.auto_info,
                    enabled,
                    disabled_reason: None,
                    z_layer,
                },
            ) && enabled
            {
                state.schema.auto_info = !state.schema.auto_info;
            }
        }
    });
    schematic_rect
}

fn active_preview_archetype(app: &UiApp, state: &WelcomeHubEguiState) -> WelcomeArchetypeChoice {
    match state.left_mode {
        WelcomeLeftMode::NewTableArchetype(a) => a,
        WelcomeLeftMode::SampleData => {
            let idx = state.schema.selected_sample_index.min(SAMPLE_FIXTURES.len().saturating_sub(1));
            SAMPLE_FIXTURES.get(idx).map_or(state.schema.archetype, |fx| WelcomeArchetypeChoice::from_table_kind(fx.kind))
        }
        WelcomeLeftMode::StarterTemplates => {
            STARTER_TEMPLATES.get(state.schema.selected_template_index).map_or(WelcomeArchetypeChoice::Xy, |t| t.archetype)
        }
        WelcomeLeftMode::ClonePipeline => state
            .schema
            .clone_source_sheet_id
            .as_ref()
            .and_then(|id| app.document().data_sheet(id))
            .map_or(state.schema.archetype, |ds| WelcomeArchetypeChoice::from_table_kind(ds.archetype.kind())),
        WelcomeLeftMode::OpenFile | WelcomeLeftMode::ImportData => state.schema.archetype,
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
            clear_pending_open(state);
            intents.push(UiIntent::SetLauncherOpen { open: false });
        }

        let external_rejected = matches!(app.session().path_policy(), curvebook_engine::ExternalPathPolicy::RejectExternal);
        let (cta_id, cta_label, cta_cmd, cta_enabled, disabled_reason) = match state.left_mode {
            WelcomeLeftMode::OpenFile => (
                "welcome.btn.open_file",
                tr(locale, "menu.file.open"),
                Some("project.open"),
                enabled && !external_rejected,
                external_rejected.then_some("External file access is disabled by host security policy"),
            ),
            WelcomeLeftMode::ImportData => {
                let blocked = external_rejected && state.import_source.requires_external_file();
                (
                    "welcome.btn.import",
                    tr(locale, "menu.file.import"),
                    state.import_source.command_id(),
                    enabled && !blocked,
                    blocked.then_some("External file access is disabled by host security policy"),
                )
            }
            WelcomeLeftMode::NewTableArchetype(_) => {
                ("welcome.btn.create", tr(locale, "welcome.btn.create_table"), Some("sheet.createData"), enabled, None)
            }
            WelcomeLeftMode::SampleData => {
                let has_fixture = !SAMPLE_FIXTURES.is_empty();
                (
                    "welcome.btn.create",
                    tr(locale, "welcome.btn.load_sample"),
                    Some("sheet.createData"),
                    enabled && has_fixture,
                    (!has_fixture).then(|| tr(locale, "welcome.sample.unavailable")),
                )
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
            dispatch_welcome_primary_action(app, state, ui.ctx().cumulative_frame_nr(), intents);
        }
    });
}

fn dispatch_welcome_primary_action(app: &UiApp, state: &mut WelcomeHubEguiState, frame_nr: u64, intents: &mut PendingUiIntents) {
    match state.left_mode {
        WelcomeLeftMode::OpenFile => {
            if matches!(app.session().path_policy(), curvebook_engine::ExternalPathPolicy::RejectExternal) {
                state.open_file_error = Some(String::from("External file access is disabled by host security policy."));
                return;
            }
            let trimmed = state.open_file_path_draft.trim();
            if trimmed.is_empty() {
                state.open_file_error = None;
                state.open_file_request_count = state.open_file_request_count.saturating_add(1);
                begin_pending_open(app, state, frame_nr, false);
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
                begin_pending_open(app, state, frame_nr, false);
                intents.push_engine("project.open", json!({ "path": open_path }));
            }
        }
        WelcomeLeftMode::ImportData => {
            let external_rejected = matches!(app.session().path_policy(), curvebook_engine::ExternalPathPolicy::RejectExternal);
            if external_rejected && state.import_source.requires_external_file() {
                state.open_file_error = Some(String::from("External file access is disabled by host security policy."));
                return;
            }
            state.open_file_error = None;
            match state.import_source {
                WelcomeImportSource::XlsxWorkbook => {
                    begin_pending_open(app, state, frame_nr, false);
                    intents.push_engine("table.importXlsx", json!({}));
                }
                WelcomeImportSource::DelimitedWizard => {
                    intents.push(UiIntent::SetLauncherOpen { open: false });
                    intents.push(UiIntent::OpenImportWizard {
                        origin: ImportOriginKind::ExternalDelimitedFile,
                        is_live_link: false,
                        target_sheet_id: None,
                    });
                }
                WelcomeImportSource::Pzfx => {
                    begin_pending_open(app, state, frame_nr, false);
                    intents.push_engine("table.importPzfx", json!({}));
                }
                WelcomeImportSource::ArchiveFile => {
                    begin_pending_open(app, state, frame_nr, false);
                    intents.push_engine("table.importPrism", json!({}));
                }
                WelcomeImportSource::PlateOrTidy => {
                    intents.push(UiIntent::SetLauncherOpen { open: false });
                    intents.push(UiIntent::UiCommand { command_id: String::from("ui.import.reshape.open"), params: json!({}) });
                }
            }
        }
        WelcomeLeftMode::NewTableArchetype(arch) => {
            let params = build_empty_create_params(arch, &state.schema, &app.locale);
            intents.push_engine("sheet.createData", params);
            intents.push(UiIntent::SetLauncherOpen { open: false });
        }
        WelcomeLeftMode::SampleData => {
            match dispatch_sample_by_index(app, SAMPLE_FIXTURES, &state.schema, intents) {
                Ok(()) => state.sample_error = None,
                Err(msg) => {
                    state.sample_error = Some(msg);
                    return;
                }
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
                let override_replicates = app
                    .document()
                    .data_sheet(src_id)
                    .and_then(|ds| clone_override_replicates_capability(ds, state.schema.clone_clear_values).ok())
                    .filter(|_| state.schema.clone_override_replicates)
                    .map(|range| state.schema.clone_override_replicates_count.clamp(*range.start(), *range.end()));
                if override_replicates.is_none() {
                    state.schema.clone_override_replicates = false;
                }
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
                if let Some(map) = params.as_object_mut() {
                    if let Some(reps) = override_replicates {
                        map.insert(String::from("override_replicates"), json!(reps));
                    }
                    let title_trim = state.schema.title.trim();
                    if !title_trim.is_empty() {
                        map.insert(String::from("title"), json!(title_trim));
                    }
                }
                intents.push_engine("sheet.clonePipeline", params);
                intents.push(UiIntent::SetLauncherOpen { open: false });
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
    let modal_h = (screen.height() - 24.0).clamp(240.0, 420.0);
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
                        disabled_reason: None,
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
                        disabled_reason: None,
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
                        disabled_reason: None,
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
                        disabled_reason: None,
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
                        disabled_reason: None,
                        z_layer: z,
                    },
                ) && enabled
                    && state.schema.clone_include_downstream
                {
                    state.schema.clone_rerun_analyses = !state.schema.clone_rerun_analyses;
                }

                let override_cap = match state.schema.clone_source_sheet_id.as_ref().and_then(|id| app.document().data_sheet(id)) {
                    Some(ds) => clone_override_replicates_capability(ds, state.schema.clone_clear_values),
                    None => Err("Select a Data table to clone"),
                };
                let (override_allowed, override_range, override_disabled_reason) = match override_cap {
                    Ok(range) => (true, range, (!enabled).then_some("Blocked by modal")),
                    Err(reason) => {
                        state.schema.clone_override_replicates = false;
                        (false, 1..=256, Some(reason))
                    }
                };
                let override_checkbox_enabled = enabled && override_allowed;
                if render_check_toggle(
                    ui,
                    tokens,
                    meta,
                    CheckToggleSpec {
                        widget_id: "welcome.clone.override_replicates",
                        label: tr_or_fallback(locale, "welcome.clone.override_replicates", "Override replicate subcolumns"),
                        checked: state.schema.clone_override_replicates,
                        enabled: override_checkbox_enabled,
                        disabled_reason: override_disabled_reason,
                        z_layer: z,
                    },
                ) && override_checkbox_enabled
                {
                    state.schema.clone_override_replicates = !state.schema.clone_override_replicates;
                }

                let stepper_enabled = override_checkbox_enabled && state.schema.clone_override_replicates;
                state.schema.clone_override_replicates_count = render_usize_stepper(
                    ui,
                    tokens,
                    meta,
                    StepperSpec {
                        widget_id: "welcome.clone.override_replicates_count",
                        label: tr_or_fallback(locale, "welcome.clone.replicates_count", "Replicates"),
                        value: state.schema.clone_override_replicates_count.clamp(*override_range.start(), *override_range.end()),
                        range: override_range,
                        enabled: stepper_enabled,
                        z_layer: z,
                    },
                );

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

#[cfg(test)]
fn matching_samples_in(fixtures: &'static [SampleFixtureDescriptor], arch: WelcomeArchetypeChoice) -> Vec<&'static SampleFixtureDescriptor> {
    fixtures.iter().filter(|f| f.kind == arch.table_kind()).collect()
}

/// Queue creation of the selected sample fixture at `schema.selected_sample_index` from `catalog`.
///
/// Never falls back to an empty table: an empty catalog, or a fixture that fails to build,
/// returns a localized error and queues nothing.
fn dispatch_sample_by_index(
    app: &UiApp,
    catalog: &'static [SampleFixtureDescriptor],
    schema: &WelcomeSchemaDraft,
    intents: &mut PendingUiIntents,
) -> Result<(), String> {
    let Some(fx) = catalog.get(schema.selected_sample_index.min(catalog.len().saturating_sub(1))) else {
        return Err(tr(&app.locale, "welcome.sample.unavailable").to_owned());
    };
    match build_sample_creation_intent_with_options(app, fx.id, Some(&schema.title), schema.auto_graph, schema.auto_info, schema.include_sample_notes)
    {
        Ok(intent) => {
            intents.push(intent);
            Ok(())
        }
        Err(err) => Err(format!("{} {err}", tr(&app.locale, "welcome.sample.load_failed"))),
    }
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
    build_sample_creation_intent_with_options(app, fixture_id, title_override, auto_graph, auto_info, false)
}

/// Build an atomic [`UiIntent::EngineCommandGroup`] instantiating sample fixture `fixture_id`,
/// optionally appending a clean-room instructional [`curvebook_doc::FloatingNote`] via `sheet.addFloatingNote`.
///
/// # Errors
/// Returns an error string if `fixture_id` is unknown or cannot be constructed.
pub fn build_sample_creation_intent_with_options(
    app: &UiApp,
    fixture_id: &str,
    title_override: Option<&str>,
    auto_graph: bool,
    auto_info: bool,
    include_sample_notes: bool,
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
    if include_sample_notes && let Some(note) = sample_fixture_instructional_note(fixture_id) {
        commands.push((
            String::from("sheet.addFloatingNote"),
            json!({
                "sheet_id": new_sid.as_str(),
                "body_markdown": note,
                "bounds_pt": { "x": 24.0, "y": 24.0, "width": 260.0, "height": 116.0 },
            }),
        ));
    }
    Ok(UiIntent::EngineCommandGroup { commands })
}

#[cfg(test)]
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
#[cfg(test)]
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
    match build_sample_creation_intent_with_options(app, fx.id, Some(&schema.title), schema.auto_graph, schema.auto_info, schema.include_sample_notes)
    {
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

fn render_clipped_text_edit(ui: &mut egui::Ui, text: &mut String, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: TextEditSpec<'_>) {
    let TextEditSpec { widget_id, label, hint, width, enabled, z_layer } = spec;
    let min_h = tokens.metrics.min_hit_target_pt;
    if !rect_visible_in_clip(ui, Vec2::new(width, min_h)) {
        ui.allocate_space(Vec2::new(width, min_h));
        return;
    }
    let resp = ui.add_enabled(enabled, egui::TextEdit::singleline(text).hint_text(hint).desired_width(width).min_size(Vec2::new(width, min_h)));
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

fn render_rail_mode_button(
    ui: &mut egui::Ui,
    width: f32,
    glyph: RailModeGlyph,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    spec: LabelSpec<'_>,
) -> bool {
    let row_h = tokens.metrics.min_hit_target_pt;
    let Some((rect, mut resp)) = allocate_visible_slot(ui, Vec2::new(width, row_h), if spec.enabled { Sense::click() } else { Sense::hover() })
    else {
        return false;
    };
    let fg = if !spec.enabled {
        tokens.text_disabled
    } else if spec.selected {
        tokens.selected_accent_text()
    } else {
        tokens.text_primary
    };
    let text_max_w = (width - 28.0).max(16.0);
    let (galley, ellipsized) =
        super::ui_kit::elided_galley(ui.painter(), spec.label, FontId::proportional(tokens.metrics.font_ui_small_pt), fg, text_max_w, false);

    let bg = if spec.selected {
        tokens.active_bg
    } else if resp.hovered() && spec.enabled {
        tokens.hover_bg
    } else {
        Color32::TRANSPARENT
    };
    let radius = CornerRadius::same(tokens.metrics.radius_md);
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, radius, bg);
    }
    if spec.selected {
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
    }

    let glyph_w = 12.0;
    let glyph_h = 10.0;
    let glyph_rect = Rect::from_min_size(Pos2::new(rect.left() + 6.0, rect.center().y - glyph_h * 0.5), Vec2::new(glyph_w, glyph_h));
    draw_rail_mode_glyph(ui.painter(), glyph_rect, glyph, fg);

    let text_rect = Rect::from_min_max(Pos2::new(rect.left() + 22.0, rect.top() + 2.0), Pos2::new(rect.right() - 4.0, rect.bottom() - 2.0));
    ui.painter().with_clip_rect(text_rect.intersect(ui.clip_rect())).galley(
        Pos2::new(text_rect.left(), rect.center().y - galley.size().y * 0.5),
        galley,
        fg,
    );

    if ellipsized {
        resp = resp.on_hover_text(spec.label);
    }
    let a11y_label = spec.label.to_owned();
    let enabled = spec.enabled;
    let selected = spec.selected;
    let a11y_role = spec.a11y_role;
    resp.widget_info(move || egui::WidgetInfo::selected(a11y_role, enabled, selected, a11y_label.clone()));
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: spec.widget_id,
            role: spec.role,
            label: spec.label,
            command_id: spec.command_id,
            value: Some(json!({ "selected": selected })),
            enabled,
            disabled_reason: (!enabled).then_some(spec.disabled_reason).flatten(),
            z_layer: spec.z_layer,
            ellipsized,
            full_label: ellipsized.then_some(spec.label),
        },
    );
    enabled && resp.clicked()
}

fn draw_rail_mode_glyph(painter: &egui::Painter, rect: Rect, glyph: RailModeGlyph, color: Color32) {
    let s = Stroke::new(1.2, color);
    match glyph {
        RailModeGlyph::Open => {
            // Simple vector folder outline
            let tab = Rect::from_min_max(rect.min, Pos2::new(rect.left() + rect.width() * 0.45, rect.top() + 3.0));
            let body = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + 2.5), rect.max);
            painter.rect_stroke(tab, CornerRadius::same(1), s, StrokeKind::Inside);
            painter.rect_stroke(body, CornerRadius::same(1), s, StrokeKind::Inside);
        }
        RailModeGlyph::Import => {
            // Downward arrow into tray
            let cx = rect.center().x;
            painter.vline(cx, rect.top()..=(rect.bottom() - 3.5), s);
            painter.line_segment([Pos2::new(cx - 2.5, rect.bottom() - 5.5), Pos2::new(cx, rect.bottom() - 3.0)], s);
            painter.line_segment([Pos2::new(cx + 2.5, rect.bottom() - 5.5), Pos2::new(cx, rect.bottom() - 3.0)], s);
            painter.hline((rect.left() + 1.0)..=(rect.right() - 1.0), rect.bottom() - 1.0, s);
            painter.vline(rect.left() + 1.0, (rect.bottom() - 3.5)..=(rect.bottom() - 1.0), s);
            painter.vline(rect.right() - 1.0, (rect.bottom() - 3.5)..=(rect.bottom() - 1.0), s);
        }
        RailModeGlyph::NewTable => {
            // 2x2 grid table icon
            painter.rect_stroke(rect, CornerRadius::same(1), s, StrokeKind::Inside);
            painter.hline(rect.x_range(), rect.top() + 3.5, Stroke::new(1.0, color));
            painter.vline(rect.center().x, rect.y_range(), Stroke::new(1.0, color));
        }
        RailModeGlyph::SampleData => {
            // Mini three-bar chart glyph
            painter.vline(rect.left() + 2.5, (rect.bottom() - 4.0)..=rect.bottom(), Stroke::new(1.6, color));
            painter.vline(rect.center().x, (rect.top() + 1.5)..=rect.bottom(), Stroke::new(1.6, color));
            painter.vline(rect.right() - 2.5, (rect.top() + 4.0)..=rect.bottom(), Stroke::new(1.6, color));
        }
        RailModeGlyph::Reuse => {
            // Two overlapping cards (template / clone)
            let back = Rect::from_min_max(rect.min, Pos2::new(rect.right() - 3.0, rect.bottom() - 2.5));
            let front = Rect::from_min_max(Pos2::new(rect.left() + 3.0, rect.top() + 2.5), rect.max);
            painter.rect_stroke(back, CornerRadius::same(1), Stroke::new(1.0, color), StrokeKind::Inside);
            painter.rect_stroke(front, CornerRadius::same(1), s, StrokeKind::Inside);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_disclosure_button(
    ui: &mut egui::Ui,
    width: f32,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    widget_id: &str,
    label: &str,
    open: bool,
    enabled: bool,
    z_layer: ZLayer,
) -> bool {
    let row_h = tokens.metrics.min_hit_target_pt;
    let Some((rect, mut resp)) = allocate_visible_slot(ui, Vec2::new(width, row_h), if enabled { Sense::click() } else { Sense::hover() }) else {
        return false;
    };
    let fg = if !enabled {
        tokens.text_disabled
    } else if resp.hovered() || open {
        tokens.text_strong
    } else {
        tokens.text_muted
    };
    let radius = CornerRadius::same(tokens.metrics.radius_md);
    if resp.hovered() && enabled {
        ui.painter().rect_filled(rect, radius, tokens.hover_bg);
    }

    // Crisp 1.5 pt vector disclosure chevron (pointing right when collapsed, downward when expanded)
    let cx = rect.left() + 10.0;
    let cy = rect.center().y;
    let r = 3.2_f32;
    let chevron_stroke = Stroke::new(1.5, fg);
    if open {
        ui.painter().line_segment([Pos2::new(cx - r, cy - r * 0.4), Pos2::new(cx, cy + r * 0.55)], chevron_stroke);
        ui.painter().line_segment([Pos2::new(cx, cy + r * 0.55), Pos2::new(cx + r, cy - r * 0.4)], chevron_stroke);
    } else {
        ui.painter().line_segment([Pos2::new(cx - r * 0.4, cy - r), Pos2::new(cx + r * 0.55, cy)], chevron_stroke);
        ui.painter().line_segment([Pos2::new(cx + r * 0.55, cy), Pos2::new(cx - r * 0.4, cy + r)], chevron_stroke);
    }

    let text_max_w = (width - 24.0).max(16.0);
    let (galley, ellipsized) =
        super::ui_kit::elided_galley(ui.painter(), label, FontId::proportional(tokens.metrics.font_ui_small_pt), fg, text_max_w, false);
    let text_rect = Rect::from_min_max(Pos2::new(rect.left() + 20.0, rect.top() + 2.0), Pos2::new(rect.right() - 4.0, rect.bottom() - 2.0));
    ui.painter().with_clip_rect(text_rect.intersect(ui.clip_rect())).galley(
        Pos2::new(text_rect.left(), rect.center().y - galley.size().y * 0.5),
        galley,
        fg,
    );

    if ellipsized {
        resp = resp.on_hover_text(label);
    }
    let a11y_label = label.to_owned();
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, enabled, a11y_label.clone()));
    ui.ctx().accesskit_node_builder(resp.id, |node| node.set_expanded(open));
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id,
            role: "button",
            label,
            command_id: None,
            value: Some(json!({ "expanded": open })),
            enabled,
            disabled_reason: None,
            z_layer,
            ellipsized,
            full_label: ellipsized.then_some(label),
        },
    );
    enabled && resp.clicked()
}

fn render_arch_rail_button(ui: &mut egui::Ui, tokens: &Tokens, meta: &mut FrameInspectMeta, spec: ArchRailButtonSpec<'_>) -> bool {
    let ArchRailButtonSpec { arch, width, selected, enabled, locale, z_layer } = spec;
    let row_h = tokens.metrics.min_hit_target_pt;
    let Some((rect, mut resp)) = allocate_visible_slot(ui, Vec2::new(width, row_h), if enabled { Sense::click() } else { Sense::hover() }) else {
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
    let (galley, ellipsized) =
        super::ui_kit::elided_galley(ui.painter(), label, FontId::proportional(tokens.metrics.font_ui_small_pt), fg, text_max_w, false);

    let bg = if selected {
        tokens.active_bg
    } else if resp.hovered() && enabled {
        tokens.hover_bg
    } else {
        tokens.button_bg
    };
    let border = if !enabled {
        tokens.divider
    } else if selected {
        tokens.accent
    } else if resp.hovered() {
        tokens.hover_border
    } else {
        tokens.button_border
    };
    let radius = CornerRadius::same(tokens.metrics.radius_md);
    ui.painter().rect(rect, radius, bg, Stroke::new(1.0, border), StrokeKind::Inside);

    let glyph_w = 12.0;
    let glyph_h = 10.0;
    let glyph_rect = Rect::from_min_size(Pos2::new(rect.left() + 6.0, rect.center().y - glyph_h * 0.5), Vec2::new(glyph_w, glyph_h));
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
    resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, selected, a11y_label.clone()));
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: arch.widget_id(),
            role: "radio",
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
    enabled && resp.clicked()
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
            height: tokens.metrics.min_hit_target_pt,
            font_size: tokens.metrics.font_ui_small_pt,
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
            width: super::ui_kit::ButtonWidth::Fixed(spec.max_w.max(24.0)),
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
            disabled_reason: (!spec.enabled).then_some(spec.disabled_reason).flatten(),
            font_size: tokens.metrics.font_ui_small_pt,
            row_height: tokens.metrics.min_hit_target_pt,
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
        let mut state =
            WelcomeHubEguiState { left_mode: WelcomeLeftMode::SampleData, sample_error: Some(String::from("stale")), ..Default::default() };
        let mut intents = PendingUiIntents::default();
        dispatch_welcome_primary_action(&app, &mut state, 1, &mut intents);
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

    #[test]
    fn sample_notes_toggle_appends_floating_note_command_only_when_enabled() {
        let app = UiApp::new();
        for fx in SAMPLE_FIXTURES {
            let without = build_sample_creation_intent_with_options(&app, fx.id, None, false, false, false).expect("build without notes");
            match without {
                UiIntent::EngineCommandGroup { commands } => {
                    assert!(!commands.iter().any(|(cmd, _)| cmd == "sheet.addFloatingNote"), "{}: unexpected note", fx.id);
                }
                other => panic!("expected EngineCommandGroup, got {other:?}"),
            }

            let with_note = build_sample_creation_intent_with_options(&app, fx.id, None, false, false, true).expect("build with notes");
            match with_note {
                UiIntent::EngineCommandGroup { commands } => {
                    let expected_note = sample_fixture_instructional_note(fx.id).expect("instructional note");
                    let (cmd, params) = commands.last().expect("note command");
                    assert_eq!(cmd, "sheet.addFloatingNote");
                    assert_eq!(params["sheet_id"], json!("data-1"));
                    assert_eq!(params["body_markdown"], json!(expected_note));
                    assert_eq!(params["bounds_pt"], json!({ "x": 24.0, "y": 24.0, "width": 260.0, "height": 116.0 }));
                }
                other => panic!("expected EngineCommandGroup, got {other:?}"),
            }
        }
    }

    #[test]
    fn sample_data_mode_loads_every_fixture_by_index_and_rejects_empty_catalog() {
        let app = UiApp::new();
        for idx in 0..SAMPLE_FIXTURES.len() {
            let schema = WelcomeSchemaDraft { selected_sample_index: idx, ..Default::default() };
            let mut intents = PendingUiIntents::default();
            assert_eq!(dispatch_sample_by_index(&app, SAMPLE_FIXTURES, &schema, &mut intents), Ok(()));
            assert!(matches!(intents.intents().first(), Some(UiIntent::EngineCommandGroup { .. })));
        }
        let mut intents = PendingUiIntents::default();
        let err = dispatch_sample_by_index(&app, &[], &WelcomeSchemaDraft::default(), &mut intents);
        assert_eq!(err, Err(tr(&app.locale, "welcome.sample.unavailable").to_owned()));
        assert!(intents.intents().is_empty());
    }

    #[test]
    fn welcome_r10_37_left_rail_order_reuse_group_additional_tables_and_frame_parity() {
        use crate::egui_shell::driver::{
            RegionBudgets, TextBleedPolicy, build_driver_font_definitions, check_region_control_budgets, verify_egui_frame_parity,
        };
        use crate::theme::{UiThemeMode, UiThemeSpec};

        let render_once = |app: &UiApp, state: &mut WelcomeHubEguiState, w: f32, h: f32, mode: UiThemeMode, locale: &str| {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            ctx.set_fonts(build_driver_font_definitions());
            let spec = UiThemeSpec::for_mode(mode);
            let tokens = Tokens::from_theme_spec(&spec, locale);
            tokens.apply_to_ctx(&ctx);
            let viewport = Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h));
            let mut meta = FrameInspectMeta::default();
            let mut last_out: Option<egui::FullOutput> = None;
            for _ in 0..2 {
                if let Some(mut prev) = last_out.take() {
                    prev.textures_delta.clear();
                }
                meta = FrameInspectMeta { pass_nr: 0, scale_factor: 1.0, viewport_rect: viewport, entries: Vec::new() };
                let mut intents = PendingUiIntents::new();
                let raw = egui::RawInput { screen_rect: Some(viewport), ..Default::default() };
                let out = ctx.run_ui(raw, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let _ = render_welcome_hub(ui, app, state, &tokens, true, &mut meta, &mut intents);
                    });
                });
                last_out = Some(out);
            }
            let mut out = last_out.expect("egui pass output");
            let parity = verify_egui_frame_parity(
                out.platform_output.accesskit_update.as_ref(),
                &meta,
                &out.shapes,
                viewport,
                TextBleedPolicy::AllowVerticalScrollClip,
            );
            out.textures_delta.clear();
            let budgets = check_region_control_budgets(&meta, &RegionBudgets::CANONICAL);
            (meta, parity, budgets)
        };

        let app = UiApp::new();
        // 1. Default Welcome: Open, Import, New Data Table, Sample Data come first in left rail,
        //    followed by grouped Reuse an Analysis; Parts of Whole sits under Additional table types.
        let mut st = WelcomeHubEguiState::default();
        let (meta, parity, budgets) = render_once(&app, &mut st, 1280.0, 800.0, UiThemeMode::Light, "en");
        assert!(parity.violations.is_empty(), "default welcome parity violations: {:?}", parity.violations);
        assert!(budgets.is_empty(), "default welcome budget violations: {budgets:?}");

        let rail_ids =
            ["welcome.action.open_existing", "welcome.mode.import", "welcome.mode.new_table", "welcome.mode.sample_data", "welcome.mode.reuse"];
        let mut prev_top = -1.0_f32;
        for wid in rail_ids {
            let entry = meta.find_by_widget_id(wid).unwrap_or_else(|| panic!("missing rail entry {wid}"));
            assert!(entry.clipped_rect.top() > prev_top, "rail entry {wid} out of vertical order");
            assert!(entry.clipped_rect.height() >= 23.5, "rail entry {wid} below 24pt target size");
            prev_top = entry.clipped_rect.top();
        }
        // Parts of Whole is hidden until Additional table types disclosure is expanded.
        assert!(meta.find_by_widget_id("welcome.arch.additional_toggle").is_some());
        assert!(meta.find_by_widget_id("welcome.arch.parts_of_whole").is_none());

        st.additional_table_types_open = true;
        let (meta_exp, parity_exp, _) = render_once(&app, &mut st, 1280.0, 800.0, UiThemeMode::Light, "en");
        assert!(parity_exp.violations.is_empty(), "expanded additional tables parity: {:?}", parity_exp.violations);
        assert!(meta_exp.find_by_widget_id("welcome.arch.parts_of_whole").is_some());

        // 2. Sweep all modes across narrow/wide viewports, themes, and long-word German locale.
        let modes = [
            WelcomeLeftMode::OpenFile,
            WelcomeLeftMode::ImportData,
            WelcomeLeftMode::NewTableArchetype(WelcomeArchetypeChoice::Column),
            WelcomeLeftMode::SampleData,
            WelcomeLeftMode::StarterTemplates,
            WelcomeLeftMode::ClonePipeline,
        ];
        for left_mode in modes {
            for (w, h, theme, loc) in [
                (640.0, 420.0, UiThemeMode::Light, "en"),
                (640.0, 420.0, UiThemeMode::Dark, "de"),
                (1280.0, 800.0, UiThemeMode::HighContrast, "de"),
                (1920.0, 1080.0, UiThemeMode::Light, "en"),
            ] {
                let mut mode_app = UiApp::new();
                mode_app.locale = loc.to_owned();
                let mut mode_st = WelcomeHubEguiState { left_mode, ..Default::default() };
                let (m, p, b) = render_once(&mode_app, &mut mode_st, w, h, theme, loc);
                assert!(p.violations.is_empty(), "mode={left_mode:?} {w}x{h} {theme:?} {loc} parity violations: {:?}", p.violations);
                assert!(b.is_empty(), "mode={left_mode:?} {w}x{h} {theme:?} {loc} budget violations: {b:?}");
                if matches!(left_mode, WelcomeLeftMode::StarterTemplates | WelcomeLeftMode::ClonePipeline) {
                    assert!(m.find_by_widget_id("welcome.mode.template").is_some());
                    assert!(m.find_by_widget_id("welcome.mode.clone").is_some());
                }
                if matches!(left_mode, WelcomeLeftMode::ImportData) {
                    assert!(m.find_by_widget_id("welcome.import.xlsx").is_some());
                    assert!(m.find_by_widget_id("welcome.btn.import").is_some());
                }
            }
        }
    }

    #[test]
    fn clone_override_replicates_capabilities_sync_summary_and_dispatch() {
        let mut doc = ProjectDocument::new("Clone Test").unwrap();
        let xy = DataSheet::new("xy-4", "XY 4 Reps", TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(4), 2, 5).unwrap())).unwrap();
        let col =
            DataSheet::new("col-1", "Column", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5).unwrap())).unwrap();
        let grp_sum =
            DataSheet::new("grp-sum", "Grouped MeanSDN", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::MeanSdN, 2, 3).unwrap())).unwrap();
        let nested = DataSheet::new("nest-5", "Nested 5 Units", TableArchetype::Nested(curvebook_doc::NestedTable::new(2, 5, 4).unwrap())).unwrap();
        doc.data_sheets.extend([xy, col, grp_sum, nested]);
        doc.active_sheet_id = Some(SheetId::new("xy-4"));
        let mut app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
        app.sync_views();

        let mut state = WelcomeHubEguiState { left_mode: WelcomeLeftMode::ClonePipeline, ..Default::default() };
        ensure_clone_source_default(&app, &mut state);
        assert_eq!(state.schema.clone_source_sheet_id.as_ref().map(SheetId::as_str), Some("xy-4"));
        assert_eq!(state.schema.clone_override_replicates_count, 4);

        state.schema.clone_override_replicates = true;
        state.schema.clone_override_replicates_count = 6;
        let summary = format_clone_pipeline_summary(app.document(), &state.schema, "en");
        assert!(summary.contains("Replicates: 6"), "summary={summary}");

        let mut intents = PendingUiIntents::new();
        dispatch_welcome_primary_action(&app, &mut state, 1, &mut intents);
        let cp = intents
            .intents()
            .iter()
            .find_map(|i| match i {
                UiIntent::EngineCommand { command_id, params } if command_id == "sheet.clonePipeline" => Some(params),
                _ => None,
            })
            .expect("clonePipeline intent");
        assert_eq!(cp["override_replicates"], json!(6));

        // Column table rejects override and forces clone_override_replicates = false.
        let col_ds = app.document().data_sheet(&SheetId::new("col-1")).unwrap();
        state.schema.clone_source_sheet_id = Some(col_ds.id.clone());
        sync_clone_replicates_for_source(&mut state.schema, col_ds);
        assert!(!state.schema.clone_override_replicates);
        assert!(clone_override_replicates_capability(col_ds, true).is_err());

        // Pre-summarized Grouped table requires clear_y_values == true.
        let grp_ds = app.document().data_sheet(&SheetId::new("grp-sum")).unwrap();
        assert!(clone_override_replicates_capability(grp_ds, false).is_err());
        assert_eq!(clone_override_replicates_capability(grp_ds, true), Ok(1..=256));

        // Nested table initializes from biological unit count and clamps to 2..=64.
        let nest_ds = app.document().data_sheet(&SheetId::new("nest-5")).unwrap();
        state.schema.clone_source_sheet_id = Some(nest_ds.id.clone());
        sync_clone_replicates_for_source(&mut state.schema, nest_ds);
        assert_eq!(state.schema.clone_override_replicates_count, 5);
        assert_eq!(clone_override_replicates_capability(nest_ds, false), Ok(2..=64));
    }

    #[test]
    fn welcome_hub_renders_sample_notes_toggle_and_clone_override_submodal_controls() {
        let mut doc = ProjectDocument::new("Submodal Render").unwrap();
        let grp_sum =
            DataSheet::new("grp-sum", "Grouped MeanSDN", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::MeanSdN, 2, 3).unwrap())).unwrap();
        doc.data_sheets.push(grp_sum);
        doc.active_sheet_id = Some(SheetId::new("grp-sum"));
        let mut app = UiApp::from_session(curvebook_engine::Session::from_document(doc));
        app.sync_views();

        let tokens = Tokens::default();
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 640.0));

        // 1. Sample Data mode shows welcome.toggle.sample_notes.
        let mut sample_state = WelcomeHubEguiState { left_mode: WelcomeLeftMode::SampleData, ..Default::default() };
        let mut meta = FrameInspectMeta::default();
        let mut intents = PendingUiIntents::new();
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
            let _ = render_welcome_hub(ui, &app, &mut sample_state, &tokens, true, &mut meta, &mut intents);
        });
        out.textures_delta.clear();
        assert!(meta.find_by_widget_id("welcome.toggle.sample_notes").is_some());

        // 2. Clone Options submodal disables override on pre-summarized Grouped when clear_values == false, enables when true.
        let mut clone_state = WelcomeHubEguiState { left_mode: WelcomeLeftMode::ClonePipeline, clone_options_open: true, ..Default::default() };
        clone_state.schema.clone_source_sheet_id = Some(SheetId::new("grp-sum"));
        clone_state.schema.clone_clear_values = false;
        clone_state.schema.clone_override_replicates = true;
        let mut meta_dis = FrameInspectMeta::default();
        let mut intents_dis = PendingUiIntents::new();
        let mut out_dis = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
            let _ = render_welcome_hub(ui, &app, &mut clone_state, &tokens, true, &mut meta_dis, &mut intents_dis);
        });
        out_dis.textures_delta.clear();
        let ov_dis = meta_dis.find_by_widget_id("welcome.clone.override_replicates").expect("override checkbox");
        assert!(!ov_dis.enabled);
        assert!(ov_dis.disabled_reason.as_deref().is_some_and(|r| !r.is_empty()));
        assert!(!clone_state.schema.clone_override_replicates);
        assert!(meta_dis.find_by_widget_id("welcome.clone.override_replicates_count.inc").is_some());

        clone_state.schema.clone_clear_values = true;
        let mut meta_en = FrameInspectMeta::default();
        let mut intents_en = PendingUiIntents::new();
        let mut out_en = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
            let _ = render_welcome_hub(ui, &app, &mut clone_state, &tokens, true, &mut meta_en, &mut intents_en);
        });
        out_en.textures_delta.clear();
        let ov_en = meta_en.find_by_widget_id("welcome.clone.override_replicates").expect("override checkbox");
        assert!(ov_en.enabled);
    }

    #[test]
    fn import_mode_dispatches_expected_intents_for_every_source() {
        let app = UiApp::new();
        for (src, expected_cmd) in [
            (WelcomeImportSource::XlsxWorkbook, Some("table.importXlsx")),
            (WelcomeImportSource::Pzfx, Some("table.importPzfx")),
            (WelcomeImportSource::ArchiveFile, Some("table.importPrism")),
        ] {
            let mut st = WelcomeHubEguiState { left_mode: WelcomeLeftMode::ImportData, import_source: src, ..Default::default() };
            let mut intents = PendingUiIntents::default();
            dispatch_welcome_primary_action(&app, &mut st, 1, &mut intents);
            assert!(
                intents
                    .intents()
                    .iter()
                    .any(|i| matches!(i, UiIntent::EngineCommand { command_id, .. } if Some(command_id.as_str()) == expected_cmd)),
                "{src:?}: missing {expected_cmd:?} intent in {:?}",
                intents.intents()
            );
            assert!(st.pending_open_mutation_seq.is_some());
            assert_eq!(st.pending_open_frame_nr, 1);
            assert!(!st.pending_open_is_template);
        }

        let mut st_delim =
            WelcomeHubEguiState { left_mode: WelcomeLeftMode::ImportData, import_source: WelcomeImportSource::DelimitedWizard, ..Default::default() };
        let mut intents_delim = PendingUiIntents::default();
        dispatch_welcome_primary_action(&app, &mut st_delim, 1, &mut intents_delim);
        assert!(
            intents_delim.intents().iter().any(|i| matches!(i, UiIntent::OpenImportWizard { origin: ImportOriginKind::ExternalDelimitedFile, .. }))
        );

        let mut st_reshape =
            WelcomeHubEguiState { left_mode: WelcomeLeftMode::ImportData, import_source: WelcomeImportSource::PlateOrTidy, ..Default::default() };
        let mut intents_reshape = PendingUiIntents::default();
        dispatch_welcome_primary_action(&app, &mut st_reshape, 1, &mut intents_reshape);
        assert!(
            intents_reshape.intents().iter().any(|i| matches!(i, UiIntent::UiCommand { command_id, .. } if command_id == "ui.import.reshape.open"))
        );
    }

    #[test]
    fn pending_open_disarms_on_cancel_ignores_stale_banner_and_routes_template_errors() {
        let run_frame = |ctx: &egui::Context, app: &UiApp, st: &mut WelcomeHubEguiState| -> (u64, FrameInspectMeta, PendingUiIntents) {
            let tokens = Tokens::for_mode(crate::UiThemeMode::Light);
            tokens.apply_to_ctx(ctx);
            let mut meta = FrameInspectMeta::default();
            let mut intents = PendingUiIntents::default();
            let mut frame_nr = 0_u64;
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                frame_nr = ui.ctx().cumulative_frame_nr();
                egui::CentralPanel::default().show(ui, |ui| {
                    let _ = render_welcome_hub(ui, app, st, &tokens, true, &mut meta, &mut intents);
                });
            });
            out.textures_delta.clear();
            (frame_nr, meta, intents)
        };

        // 1. Cancelling a file picker with a pre-existing status banner disarms pending state
        //    on the next settled frame without turning the stale banner into a red error.
        let ctx = egui::Context::default();
        let mut app = UiApp::new();
        app.canvas_dialogs.status_banner = Some(String::from("Exported previous table cleanly"));
        let mut st = WelcomeHubEguiState { left_mode: WelcomeLeftMode::ImportData, ..Default::default() };
        let (dispatch_frame, _, _) = run_frame(&ctx, &app, &mut st);
        begin_pending_open(&app, &mut st, dispatch_frame, false);
        let (next_frame, _, intents_after_cancel) = run_frame(&ctx, &app, &mut st);
        assert!(next_frame > dispatch_frame, "subsequent frame must advance cumulative_frame_nr");
        assert!(st.pending_open_mutation_seq.is_none(), "cancelled file picker must disarm pending_open_mutation_seq");
        assert!(st.open_file_error.is_none(), "pre-existing status banner must not surface as red open_file_error");
        assert!(
            !intents_after_cancel.intents().iter().any(|i| matches!(i, UiIntent::SetLauncherOpen { open: false })),
            "cancelled file picker must not auto-close Welcome"
        );

        // 2. A fresh error banner from `welcome.btn.open_template_file` routes to `template_error`
        //    and renders `welcome.template.error` inside the Starter Templates pane.
        let mut st_tpl = WelcomeHubEguiState { left_mode: WelcomeLeftMode::StarterTemplates, ..Default::default() };
        let (tpl_dispatch_frame, _, _) = run_frame(&ctx, &app, &mut st_tpl);
        begin_pending_open(&app, &mut st_tpl, tpl_dispatch_frame, true);
        app.canvas_dialogs.status_banner = Some(String::from("Template file is corrupted"));
        let (_, meta_tpl, _) = run_frame(&ctx, &app, &mut st_tpl);
        assert!(st_tpl.pending_open_mutation_seq.is_none());
        assert_eq!(st_tpl.template_error.as_deref(), Some("Template file is corrupted"));
        assert!(st_tpl.open_file_error.is_none());
        assert!(meta_tpl.find_by_widget_id("welcome.template.error").is_some(), "template error must be rendered visibly");
    }
}
