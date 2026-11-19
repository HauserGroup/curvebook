//! Welcome & New Data Table Launcher + Portfolio Gallery view builders (`R10.4`).
//!
//! Renders the full interactive two-pane Welcome & New Data Table Launcher whenever
//! `builder.app.nav.welcome.open` is true or the project workspace has zero sheets, supporting:
//! - Left rail: all 8 scientific [`TableKind`] archetypes, `Open Existing File...`,
//!   `Clone a Graph / Pipeline`, and built-in `.cbtemplate` starter templates.
//! - Right pane: original vector table + plot structural schematic, `Empty Data Table` vs.
//!   clean-room `Tutorial Sample Data` (`SAMPLE_FIXTURES`), `XColumnMode` selector, Y replicate
//!   stepper & pre-computed error formats (`SubcolumnMode`), `ColumnDesignHint` (Unpaired vs.
//!   Paired vs. Pre-computed), dimensions, `Auto Graph` / `Auto Info` toggles, Clone options,
//!   and `.cbtemplate` instantiation via canonical engine commands.

use curvebook_doc::{
    Cell, CellValue, ColumnDesignHint, DataSheet, GraphKind, GraphSheet, InfoSheet, MAX_DATA_SETS, MAX_ROWS, MAX_SUBCOLUMNS, ProjectDocument,
    SequenceProgression, SheetFolderKind, SheetId, SubcolumnMode, TableArchetype, TableKind, VariableRole, VariableSource, XColumnMode,
};
use curvebook_format::save_template_bytes;
use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::control_io::encode_base64;
use crate::i18n::{is_rtl_locale, mirror_x_for_rtl, table_kind_label, tr, try_tr};
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::scroll::{ScrollAxes, ScrollMeta, scrollbar_thickness_px};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldSpec, ToggleWidgetSpec, emit_button, emit_scrollbars_for_meta, emit_tab, emit_text_field, emit_toggle,
    paint_text_fitted,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::UiTextStyle;
use crate::workspace_nav::{SAMPLE_FIXTURES, TableCreationDraft, WelcomeMode, instantiate_sample_fixture};

use super::a11y::labeled_meta;
use super::nav_notes::section_for_folder_kind;

const TITLE_INPUT_ID: &str = "welcome.title.input";
const CLONE_TITLE_INPUT_ID: &str = "welcome.clone.title.input";

/// Transient view state owned by the welcome / portfolio gallery shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WelcomeViewState {
    pub search_query: String,
    pub active_template_index: Option<usize>,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub status_message: Option<String>,
    #[serde(default)]
    pub last_open_file_request: u64,
}

#[derive(Debug, Clone, Copy)]
struct TemplatePreset {
    virtual_path: &'static str,
    title: &'static str,
    kind: TableKind,
    summary: &'static str,
    subcolumn_mode: SubcolumnMode,
    replicates: usize,
    column_design: ColumnDesignHint,
    graph_kind: GraphKind,
}

const TEMPLATE_PRESETS: &[TemplatePreset] = &[
    TemplatePreset {
        virtual_path: "template://dose-response-4pl.cbtemplate",
        title: "Dose-Response 4PL Assay Template",
        kind: TableKind::Xy,
        summary: "XY table (3 replicates), X log concentration setup, Info sheet, and XY scatter graph.",
        subcolumn_mode: SubcolumnMode::RawReplicates(3),
        replicates: 3,
        column_design: ColumnDesignHint::UnpairedReplicates,
        graph_kind: GraphKind::Xy,
    },
    TemplatePreset {
        virtual_path: "template://column-anova.cbtemplate",
        title: "Three-Arm Column Comparison Template",
        kind: TableKind::Column,
        summary: "Unpaired Column table (Vehicle, Low, High), Info sheet, and column distribution plot.",
        subcolumn_mode: SubcolumnMode::Single,
        replicates: 1,
        column_design: ColumnDesignHint::UnpairedReplicates,
        graph_kind: GraphKind::Column,
    },
    TemplatePreset {
        virtual_path: "template://grouped-kinetics.cbtemplate",
        title: "Two-Factor Time-Course Template",
        kind: TableKind::Grouped,
        summary: "Grouped table (2 treatments x 4 time points x 3 replicates), Info sheet, and grouped plot.",
        subcolumn_mode: SubcolumnMode::RawReplicates(3),
        replicates: 3,
        column_design: ColumnDesignHint::UnpairedReplicates,
        graph_kind: GraphKind::Grouped,
    },
    TemplatePreset {
        virtual_path: "template://survival-cohort.cbtemplate",
        title: "Kaplan-Meier Survival Cohort Template",
        kind: TableKind::Survival,
        summary: "Survival time-to-event table for 2 cohorts, Info metadata, and step survival curve.",
        subcolumn_mode: SubcolumnMode::Single,
        replicates: 1,
        column_design: ColumnDesignHint::UnpairedReplicates,
        graph_kind: GraphKind::Survival,
    },
];

// ============================================================================
// Localization & Geometry Helpers
// ============================================================================

fn welcome_tr(locale: &str, key: &'static str, fallback: &'static str) -> &'static str {
    try_tr(locale, key).unwrap_or(fallback)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn s_u32(base: u32, scale: f64, min_px: u32) -> u32 {
    ((f64::from(base) * scale).round() as u32).max(min_px)
}

#[allow(clippy::cast_possible_truncation)]
fn s_i32(base: i32, scale: f64) -> i32 {
    (f64::from(base) * scale).round() as i32
}

fn place_rtl(parent: LayoutRectPx, rel_x: i32, y: i32, w: u32, h: u32, is_rtl: bool) -> LayoutRectPx {
    let clamped_rel = u32::try_from(rel_x.max(0)).unwrap_or(0);
    let max_w = parent.width.saturating_sub(clamped_rel);
    let actual_w = w.min(max_w);
    let local_x = mirror_x_for_rtl(clamped_rel, actual_w, parent.width, is_rtl);
    LayoutRectPx::new(parent.x + i32::try_from(local_x).unwrap_or(0), y, actual_w, h)
}

fn rect_fits_inside(inner: LayoutRectPx, clip: LayoutRectPx) -> bool {
    inner.width > 0 && inner.height > 0 && inner.y >= clip.y && inner.bottom() <= clip.bottom() && inner.x >= clip.x && inner.right() <= clip.right()
}

fn next_unique_sheet_id(doc: &ProjectDocument, prefix: &str) -> SheetId {
    for n in 1usize..=10_000 {
        let candidate = SheetId::from(format!("{prefix}-{n}"));
        if doc.sheet_section(&candidate).is_none() {
            return candidate;
        }
    }
    SheetId::from(format!("{prefix}-new"))
}

fn first_fixture_for_kind(kind: TableKind) -> &'static str {
    SAMPLE_FIXTURES.iter().find(|f| f.kind == kind).map_or(SAMPLE_FIXTURES[0].id, |f| f.id)
}

// ============================================================================
// Archetype / Mode Payload Action Dispatcher
// ============================================================================

/// Dispatch custom `WidgetAction::Custom` events targeting the `welcome` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut WelcomeViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if let Some(kind_str) = payload.strip_prefix("arch:")
        && let Some(kind) = parse_archetype_slug(kind_str)
    {
        select_archetype_in_wizard(app, state, kind);
        return Ok(handled_outcome(format!("welcome.arch:{kind_str}")));
    }
    if let Some(mode_str) = payload.strip_prefix("mode:")
        && handle_mode_payload(app, state, mode_str)?
    {
        return Ok(handled_outcome(format!("welcome.mode:{mode_str}")));
    }
    if let Some(fid) = payload.strip_prefix("fixture:")
        && let Some(meta) = SAMPLE_FIXTURES.iter().find(|f| f.id == fid)
    {
        let keep_graph = app.nav.welcome.table_draft.create_default_graph;
        let keep_info = app.nav.welcome.table_draft.create_paired_info;
        app.nav.welcome.mode = WelcomeMode::TutorialSample;
        app.nav.welcome.selected_fixture_id = Some(meta.id.to_owned());
        app.nav.welcome.select_archetype(meta.kind);
        app.nav.welcome.table_draft.title = meta.title.to_owned();
        app.nav.welcome.table_draft.create_default_graph = keep_graph;
        app.nav.welcome.table_draft.create_paired_info = keep_info;
        app.interaction.text_field_buffers.remove(TITLE_INPUT_ID);
        bump_state(state, None);
        return Ok(handled_outcome(format!("welcome.fixture:{fid}")));
    }
    if let Some(idx_str) = payload.strip_prefix("template:")
        && let Ok(idx) = idx_str.parse::<usize>()
        && let Some(preset) = TEMPLATE_PRESETS.get(idx)
    {
        app.nav.welcome.mode = WelcomeMode::FromTemplate;
        state.active_template_index = Some(idx);
        app.nav.welcome.template_path = Some(preset.virtual_path.to_owned());
        app.interaction.text_field_buffers.remove(TITLE_INPUT_ID);
        bump_state(state, None);
        return Ok(handled_outcome(format!("welcome.template:{idx}")));
    }
    if let Some(x_str) = payload.strip_prefix("x_mode:")
        && handle_x_mode_payload(app, state, x_str)
    {
        return Ok(handled_outcome(format!("welcome.x_mode:{x_str}")));
    }
    if let Some(sub_str) = payload.strip_prefix("subcol:")
        && handle_subcol_payload(app, state, sub_str)
    {
        return Ok(handled_outcome(format!("welcome.subcol:{sub_str}")));
    }
    if let Some(des_str) = payload.strip_prefix("col_design:")
        && handle_col_design_payload(app, state, des_str)
    {
        return Ok(handled_outcome(format!("welcome.col_design:{des_str}")));
    }
    if handle_stepper_or_toggle_payload(app, state, payload) {
        return Ok(handled_outcome(format!("welcome.{payload}")));
    }
    if let Some(src_str) = payload.strip_prefix("clone_source:")
        && let Some(ds) = app.document().data_sheet(&SheetId::from(src_str)).cloned()
    {
        app.nav.welcome.clone_draft.source_sheet_id = Some(ds.id.clone());
        app.nav.welcome.clone_draft.options.title = format!("{} Clone", ds.title);
        app.interaction.text_field_buffers.remove(CLONE_TITLE_INPUT_ID);
        bump_state(state, None);
        return Ok(handled_outcome(format!("welcome.clone_source:{src_str}")));
    }
    if handle_clone_toggle_payload(app, state, payload) {
        return Ok(handled_outcome(format!("welcome.{payload}")));
    }
    if payload == "gallery:close" {
        app.nav.gallery.open = false;
        return Ok(handled_outcome(String::from("welcome.gallery:close")));
    }
    if let Some(slug) = payload.strip_prefix("gallery:filter:") {
        let folder = match slug {
            "all" => None,
            "data" => Some(SheetFolderKind::Data),
            "info" => Some(SheetFolderKind::Info),
            "results" => Some(SheetFolderKind::Results),
            "graphs" => Some(SheetFolderKind::Graphs),
            "layouts" => Some(SheetFolderKind::Layouts),
            _ => return Err(format!("unknown gallery folder filter `{slug}`")),
        };
        let doc = app.session.document().clone();
        app.nav.set_gallery_filter(folder, &doc);
        app.interaction.scroll_offsets.offsets.insert(String::from("scroll.gallery.cards"), [0.0, 0.0]);
        return Ok(handled_outcome(format!("welcome.gallery:filter:{slug}")));
    }

    match payload {
        "action:open_file" => {
            state.last_open_file_request = state.last_open_file_request.saturating_add(1);
            bump_state(state, Some(String::from("Open Existing File requested via host file picker")));
            Ok(handled_outcome(String::from("welcome.open_file_requested")))
        }
        "action:cancel" => {
            close_welcome_overlay(app, state);
            Ok(handled_outcome(String::from("welcome.cancel")))
        }
        "action:create" => execute_create_table_or_sample(app, state),
        "action:instantiate_template" => execute_instantiate_template(app, state),
        "action:clone" => execute_clone_pipeline(app, state),
        _ => Ok(UiEventOutcome::default()),
    }
}

fn handled_outcome(action: String) -> UiEventOutcome {
    UiEventOutcome { handled: true, action, command_id: None, command_result: None }
}

fn bump_state(state: &mut WelcomeViewState, status: Option<String>) {
    state.revision = state.revision.wrapping_add(1);
    state.status_message = status;
}

fn close_welcome_overlay(app: &mut UiApp, state: &mut WelcomeViewState) {
    app.nav.welcome.open = false;
    app.nav.gallery.open = false;
    if app.focus_widget_id.as_deref().is_some_and(|id| id.starts_with("welcome.")) {
        app.focus_widget_id = None;
    }
    app.interaction.text_field_buffers.remove(TITLE_INPUT_ID);
    app.interaction.text_field_buffers.remove(CLONE_TITLE_INPUT_ID);
    bump_state(state, None);
}

fn parse_archetype_slug(slug: &str) -> Option<TableKind> {
    match slug {
        "xy" => Some(TableKind::Xy),
        "column" => Some(TableKind::Column),
        "grouped" => Some(TableKind::Grouped),
        "contingency" => Some(TableKind::Contingency),
        "survival" => Some(TableKind::Survival),
        "parts_of_whole" => Some(TableKind::PartsOfWhole),
        "multiple_variables" => Some(TableKind::MultipleVariables),
        "nested" => Some(TableKind::Nested),
        _ => None,
    }
}

const fn archetype_slug(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Xy => "xy",
        TableKind::Column => "column",
        TableKind::Grouped => "grouped",
        TableKind::Contingency => "contingency",
        TableKind::Survival => "survival",
        TableKind::PartsOfWhole => "parts_of_whole",
        TableKind::MultipleVariables => "multiple_variables",
        TableKind::Nested => "nested",
    }
}

fn select_archetype_in_wizard(app: &mut UiApp, state: &mut WelcomeViewState, kind: TableKind) {
    let keep_graph = app.nav.welcome.table_draft.create_default_graph;
    let keep_info = app.nav.welcome.table_draft.create_paired_info;
    let preserve_sample = app.nav.welcome.mode == WelcomeMode::TutorialSample;
    app.nav.welcome.select_archetype(kind);
    app.nav.welcome.table_draft.create_default_graph = keep_graph;
    app.nav.welcome.table_draft.create_paired_info = keep_info;
    app.nav.welcome.selected_fixture_id = Some(String::from(first_fixture_for_kind(kind)));
    if preserve_sample {
        if let Some(fid) = app.nav.welcome.selected_fixture_id.as_deref()
            && let Some(meta) = SAMPLE_FIXTURES.iter().find(|f| f.id == fid)
        {
            app.nav.welcome.table_draft.title = meta.title.to_owned();
        }
    } else {
        app.nav.welcome.mode = WelcomeMode::EmptyTypedTable;
    }
    app.interaction.text_field_buffers.remove(TITLE_INPUT_ID);
    bump_state(state, None);
}

fn handle_mode_payload(app: &mut UiApp, state: &mut WelcomeViewState, mode_str: &str) -> Result<bool, String> {
    match mode_str {
        "empty" => {
            app.nav.welcome.mode = WelcomeMode::EmptyTypedTable;
            let kind = app.nav.welcome.table_draft.kind;
            app.nav.welcome.table_draft.title = format!("{} Data", kind.label());
            app.interaction.text_field_buffers.remove(TITLE_INPUT_ID);
            bump_state(state, None);
            Ok(true)
        }
        "sample" => {
            app.nav.welcome.mode = WelcomeMode::TutorialSample;
            let kind = app.nav.welcome.table_draft.kind;
            let current_matches =
                app.nav.welcome.selected_fixture_id.as_deref().and_then(|id| SAMPLE_FIXTURES.iter().find(|f| f.id == id && f.kind == kind));
            let chosen = current_matches.or_else(|| SAMPLE_FIXTURES.iter().find(|f| f.kind == kind)).unwrap_or(&SAMPLE_FIXTURES[0]);
            app.nav.welcome.selected_fixture_id = Some(chosen.id.to_owned());
            app.nav.welcome.table_draft.title = chosen.title.to_owned();
            app.interaction.text_field_buffers.remove(TITLE_INPUT_ID);
            bump_state(state, None);
            Ok(true)
        }
        "template" => {
            app.nav.welcome.mode = WelcomeMode::FromTemplate;
            let idx = state.active_template_index.filter(|&i| i < TEMPLATE_PRESETS.len()).unwrap_or(0);
            state.active_template_index = Some(idx);
            app.nav.welcome.template_path = Some(TEMPLATE_PRESETS[idx].virtual_path.to_owned());
            app.interaction.text_field_buffers.remove(TITLE_INPUT_ID);
            bump_state(state, None);
            Ok(true)
        }
        "clone" => {
            let doc = app.document();
            let chosen_src = app
                .nav
                .welcome
                .clone_draft
                .source_sheet_id
                .as_ref()
                .and_then(|sid| doc.data_sheet(sid))
                .or_else(|| doc.active_sheet_id.as_ref().and_then(|sid| doc.data_sheet(sid)))
                .or_else(|| doc.data_sheets.first())
                .cloned();
            let Some(src) = chosen_src else {
                bump_state(state, Some(String::from("Clone requires at least one existing data table")));
                return Ok(true);
            };
            app.nav.welcome.mode = WelcomeMode::ClonePipeline;
            app.nav.welcome.clone_draft.source_sheet_id = Some(src.id.clone());
            app.nav.welcome.clone_draft.options.title = format!("{} Clone", src.title);
            app.interaction.text_field_buffers.remove(CLONE_TITLE_INPUT_ID);
            bump_state(state, None);
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn handle_x_mode_payload(app: &mut UiApp, state: &mut WelcomeViewState, x_str: &str) -> bool {
    let kind = app.nav.welcome.table_draft.kind;
    match (kind, x_str) {
        (TableKind::Xy, "numbers") => {
            app.nav.welcome.table_draft.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single };
        }
        (TableKind::Xy, "dates") | (TableKind::Survival, "dates") => {
            app.nav.welcome.table_draft.x_mode = XColumnMode::DateOrElapsedTime { unit_label: String::from("Date") };
        }
        (TableKind::Xy, "elapsed") | (TableKind::Survival, "elapsed") => {
            app.nav.welcome.table_draft.x_mode = XColumnMode::DateOrElapsedTime { unit_label: String::from("Elapsed Time") };
        }
        (TableKind::Xy, "sequence") => {
            app.nav.welcome.table_draft.x_mode =
                XColumnMode::GeneratedSequence { start: 0.0, step: 1.0, progression: SequenceProgression::Arithmetic };
        }
        (k, "none") if !matches!(k, TableKind::Xy | TableKind::Survival) => {}
        _ => return false,
    }
    bump_state(state, None);
    true
}

fn handle_subcol_payload(app: &mut UiApp, state: &mut WelcomeViewState, sub_str: &str) -> bool {
    let draft = &mut app.nav.welcome.table_draft;
    let rep_n = draft.replicates.clamp(2, MAX_SUBCOLUMNS);
    let candidate = match sub_str {
        "single" => SubcolumnMode::Single,
        "raw" => SubcolumnMode::RawReplicates(rep_n),
        "mean_sd_n" => SubcolumnMode::MeanSdN,
        "mean_sem_n" => SubcolumnMode::MeanSemN,
        "mean_cv_n" => SubcolumnMode::MeanCvN,
        "mean_low_high" => SubcolumnMode::MeanLowHigh,
        "upper_lower" => SubcolumnMode::UpperLowerLimits,
        "median_iqr" => SubcolumnMode::MedianIqr,
        _ => return false,
    };
    if !draft.kind.allows_subcolumn_mode(candidate) {
        return false;
    }
    draft.subcolumn_mode = candidate;
    match candidate {
        SubcolumnMode::Single => {
            draft.replicates = 1;
            if draft.kind == TableKind::Column && draft.column_design == ColumnDesignHint::PrecomputedError {
                draft.column_design = ColumnDesignHint::UnpairedReplicates;
            }
        }
        SubcolumnMode::RawReplicates(n) => {
            draft.replicates = n;
        }
        _ => {
            draft.replicates = candidate.subcolumn_count();
            if draft.kind == TableKind::Column {
                draft.column_design = ColumnDesignHint::PrecomputedError;
            }
        }
    }
    bump_state(state, None);
    true
}

fn handle_col_design_payload(app: &mut UiApp, state: &mut WelcomeViewState, des_str: &str) -> bool {
    if app.nav.welcome.table_draft.kind != TableKind::Column {
        return false;
    }
    let design = match des_str {
        "unpaired" => ColumnDesignHint::UnpairedReplicates,
        "paired" => ColumnDesignHint::PairedRepeatedMeasures,
        "precomputed" => ColumnDesignHint::PrecomputedError,
        _ => return false,
    };
    app.nav.welcome.table_draft = app.nav.welcome.table_draft.clone().with_column_design(design);
    if matches!(design, ColumnDesignHint::PrecomputedError) {
        app.nav.welcome.table_draft.replicates = 3;
    } else {
        app.nav.welcome.table_draft.replicates = 1;
    }
    bump_state(state, None);
    true
}

fn handle_stepper_or_toggle_payload(app: &mut UiApp, state: &mut WelcomeViewState, payload: &str) -> bool {
    let kind = app.nav.welcome.table_draft.kind;
    match payload {
        "replicates:dec" | "replicates:inc" => {
            if !matches!(kind, TableKind::Xy | TableKind::Grouped | TableKind::Nested) {
                return false;
            }
            let min_k = if kind == TableKind::Xy { 1 } else { 2 };
            let cur = app.nav.welcome.table_draft.replicates.max(min_k);
            let next = if payload == "replicates:inc" { (cur + 1).min(MAX_SUBCOLUMNS.min(52)) } else { cur.saturating_sub(1).max(min_k) };
            app.nav.welcome.set_replicates(next);
            bump_state(state, None);
            true
        }
        "datasets:dec" | "datasets:inc" => {
            let cur = app.nav.welcome.table_draft.data_set_count.max(1);
            let next = if payload == "datasets:inc" { (cur + 1).min(MAX_DATA_SETS.min(64)) } else { cur.saturating_sub(1).max(1) };
            app.nav.welcome.table_draft.data_set_count = next;
            bump_state(state, None);
            true
        }
        "rows:dec" | "rows:inc" => {
            let min_r = if kind == TableKind::Contingency { 2 } else { 1 };
            let cur = app.nav.welcome.table_draft.row_count.max(min_r);
            let next = if payload == "rows:inc" { (cur + 1).min(MAX_ROWS.min(500)) } else { cur.saturating_sub(1).max(min_r) };
            app.nav.welcome.table_draft.row_count = next;
            bump_state(state, None);
            true
        }
        "toggle:auto_graph" => {
            app.nav.welcome.table_draft.create_default_graph = !app.nav.welcome.table_draft.create_default_graph;
            bump_state(state, None);
            true
        }
        "toggle:auto_info" => {
            app.nav.welcome.table_draft.create_paired_info = !app.nav.welcome.table_draft.create_paired_info;
            bump_state(state, None);
            true
        }
        _ => false,
    }
}

fn handle_clone_toggle_payload(app: &mut UiApp, state: &mut WelcomeViewState, payload: &str) -> bool {
    let clone = &mut app.nav.welcome.clone_draft;
    match payload {
        "clone_toggle:keep_x" => clone.options.keep_x_values = !clone.options.keep_x_values,
        "clone_toggle:clear_y" => clone.options.clear_y_values = !clone.options.clear_y_values,
        "clone_toggle:clear_col_titles" => clone.options.clear_column_titles = !clone.options.clear_column_titles,
        "clone_toggle:clear_row_titles" => clone.options.clear_row_titles = !clone.options.clear_row_titles,
        "clone_toggle:analyses" => clone.clone_downstream_analyses = !clone.clone_downstream_analyses,
        "clone_toggle:graphs" => clone.clone_downstream_graphs = !clone.clone_downstream_graphs,
        "clone_toggle:layouts" => clone.clone_downstream_layouts = !clone.clone_downstream_layouts,
        _ => return false,
    }
    bump_state(state, None);
    true
}

// ============================================================================
// Canonical Engine Execution (`sheet.createData`, Samples, Templates, Clone)
// ============================================================================

fn read_text_override(app: &UiApp, widget_id: &str) -> Option<String> {
    app.interaction.text_field_buffers.get(widget_id).map(|b| b.text.trim().to_owned()).filter(|s| !s.is_empty())
}

fn execute_create_table_or_sample(app: &mut UiApp, state: &mut WelcomeViewState) -> Result<UiEventOutcome, String> {
    let new_sheet_id = next_unique_sheet_id(app.document(), "data");
    let custom_title = read_text_override(app, TITLE_INPUT_ID);
    let outcome = match app.nav.welcome.mode {
        WelcomeMode::TutorialSample => {
            let fid =
                app.nav.welcome.selected_fixture_id.clone().unwrap_or_else(|| String::from(first_fixture_for_kind(app.nav.welcome.table_draft.kind)));
            let auto_graph = app.nav.welcome.table_draft.create_default_graph;
            let auto_info = app.nav.welcome.table_draft.create_paired_info;
            instantiate_sample_via_commands(app, &fid, &new_sheet_id, custom_title.as_deref(), auto_graph, auto_info)?
        }
        _ => {
            let mut draft = app.nav.welcome.table_draft.clone();
            draft.sheet_id = new_sheet_id.as_str().to_owned();
            if let Some(title) = custom_title {
                draft.title = title;
            }
            let mut intent = draft.to_command_intent();
            if let XColumnMode::DateOrElapsedTime { ref unit_label } = draft.x_mode
                && let Some(obj) = intent.params.as_object_mut()
            {
                if draft.kind == TableKind::Xy {
                    obj.insert(String::from("x_title"), Value::String(unit_label.clone()));
                } else if draft.kind == TableKind::Survival {
                    obj.insert(String::from("time_title"), Value::String(unit_label.clone()));
                }
            }
            let cmd_res = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            UiEventOutcome {
                handled: true,
                action: String::from("welcome.create"),
                command_id: Some(intent.command_id),
                command_result: Some(cmd_res.payload),
            }
        }
    };
    app.nav.sync_from_document(app.session.document());
    close_welcome_overlay(app, state);
    Ok(outcome)
}

fn cell_to_json_value(cell: &Cell) -> Option<Value> {
    match &cell.value {
        CellValue::Empty => None,
        CellValue::Number(v) => Some(json!({ "value": v, "excluded": cell.excluded })),
        CellValue::Text(s) => Some(json!({ "value": s, "excluded": cell.excluded })),
    }
}

fn instantiate_sample_via_commands(
    app: &mut UiApp,
    fixture_id: &str,
    new_sheet_id: &SheetId,
    title_override: Option<&str>,
    auto_graph: bool,
    auto_info: bool,
) -> Result<UiEventOutcome, String> {
    let fixture = instantiate_sample_fixture(fixture_id, new_sheet_id.clone()).map_err(|e| e.to_string())?;
    let title = title_override.unwrap_or(&fixture.title).to_owned();
    let sid = new_sheet_id.as_str();
    let create_params = build_sample_create_params(&fixture.archetype, sid, &title, auto_graph, auto_info);
    let create_out = app.execute_command("sheet.createData", create_params).map_err(|e| e.to_string())?;

    populate_sample_x_values(app, sid, &fixture.archetype)?;
    populate_sample_titles(app, sid, &fixture.archetype)?;
    populate_sample_cells(app, sid, &fixture.archetype)?;
    populate_sample_mv_roles_and_formulas(app, sid, &fixture.archetype)?;

    Ok(UiEventOutcome {
        handled: true,
        action: String::from("welcome.create_sample"),
        command_id: Some(String::from("sheet.createData")),
        command_result: Some(create_out.payload),
    })
}

fn build_sample_create_params(arch: &TableArchetype, sid: &str, title: &str, auto_graph: bool, auto_info: bool) -> Value {
    let mut base = json!({
        "sheet_id": sid,
        "title": title,
        "archetype": arch.kind(),
        "cols": arch.column_count(),
        "row_count": arch.row_count(),
        "auto_graph": auto_graph,
        "auto_info": auto_info,
    });
    if let Some(obj) = base.as_object_mut() {
        match arch {
            TableArchetype::Xy(t) => {
                obj.insert(String::from("subcolumn_mode"), json!(t.y_subcolumn_mode));
                obj.insert(String::from("x_mode"), json!(t.x_mode));
                obj.insert(String::from("x_title"), json!(t.x_title));
            }
            TableArchetype::Column(t) => {
                obj.insert(String::from("subcolumn_mode"), json!(t.subcolumn_mode));
                obj.insert(String::from("design_hint"), json!(t.design_hint));
            }
            TableArchetype::Grouped(t) => {
                obj.insert(String::from("subcolumn_mode"), json!(t.subcolumn_mode));
                obj.insert(String::from("row_factor_title"), json!(t.row_factor_title));
                obj.insert(String::from("column_factor_title"), json!(t.column_factor_title));
            }
            TableArchetype::Contingency(t) => {
                obj.insert(String::from("row_factor_title"), json!(t.row_factor_title));
                obj.insert(String::from("column_factor_title"), json!(t.column_factor_title));
            }
            TableArchetype::Survival(t) => {
                obj.insert(String::from("time_title"), json!(t.time_title));
            }
            TableArchetype::MultipleVariables(t) => {
                let vars: Vec<Value> = t.variables.iter().map(|v| json!({ "title": v.title })).collect();
                obj.insert(String::from("variables"), Value::Array(vars));
            }
            TableArchetype::Nested(t) => {
                let reps = t.groups.first().map_or(3, |g| g.subcolumn_titles.len().max(1));
                obj.insert(String::from("replicates"), json!(reps));
            }
            TableArchetype::PartsOfWhole(_) => {}
        }
    }
    base
}

fn populate_sample_x_values(app: &mut UiApp, sid: &str, arch: &TableArchetype) -> Result<(), String> {
    let mut x_edits = Vec::new();
    match arch {
        TableArchetype::Xy(t) => {
            for (r, subs) in t.x_cells.iter().enumerate() {
                for (s, cell) in subs.iter().enumerate() {
                    if let Some(mut item) = cell_to_json_value(cell)
                        && let Some(obj) = item.as_object_mut()
                    {
                        obj.insert(String::from("row"), json!(r));
                        obj.insert(String::from("subcol"), json!(s));
                        x_edits.push(item);
                    }
                }
            }
        }
        TableArchetype::Survival(t) => {
            for (r, cell) in t.times.iter().enumerate() {
                if let Some(mut item) = cell_to_json_value(cell)
                    && let Some(obj) = item.as_object_mut()
                {
                    obj.insert(String::from("row"), json!(r));
                    obj.insert(String::from("subcol"), json!(0));
                    x_edits.push(item);
                }
            }
        }
        _ => {}
    }
    if !x_edits.is_empty() {
        app.execute_command("table.setXCells", json!({ "sheet_id": sid, "edits": x_edits })).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn populate_sample_titles(app: &mut UiApp, sid: &str, arch: &TableArchetype) -> Result<(), String> {
    let col_titles: Vec<String> = match arch {
        TableArchetype::Xy(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Column(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Grouped(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Contingency(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Survival(t) => t.cohorts.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::PartsOfWhole(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Nested(t) => t.groups.iter().map(|g| g.title.clone()).collect(),
        TableArchetype::MultipleVariables(_) => Vec::new(),
    };
    if !col_titles.is_empty() {
        app.execute_command("table.setTitles", json!({ "sheet_id": sid, "titles": col_titles })).map_err(|e| e.to_string())?;
    }

    let row_titles: Option<&[String]> = match arch {
        TableArchetype::Xy(t) => Some(&t.row_titles),
        TableArchetype::Column(t) => Some(&t.row_titles),
        TableArchetype::Grouped(t) => Some(&t.row_titles),
        TableArchetype::Contingency(t) => Some(&t.row_titles),
        TableArchetype::PartsOfWhole(t) => Some(&t.slice_labels),
        TableArchetype::Survival(_) | TableArchetype::MultipleVariables(_) | TableArchetype::Nested(_) => None,
    };
    if let Some(titles) = row_titles
        && titles.iter().any(|t| !t.trim().is_empty())
    {
        let payload: Vec<Value> = titles.iter().map(|s| if s.trim().is_empty() { Value::Null } else { json!(s) }).collect();
        app.execute_command("table.setRowTitles", json!({ "sheet_id": sid, "titles": payload })).map_err(|e| e.to_string())?;
    }

    if let TableArchetype::Nested(t) = arch {
        for (col_idx, group) in t.groups.iter().enumerate() {
            if !group.subcolumn_titles.is_empty() {
                app.execute_command("table.setSubcolumnTitles", json!({ "sheet_id": sid, "col": col_idx, "titles": group.subcolumn_titles }))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn populate_sample_cells(app: &mut UiApp, sid: &str, arch: &TableArchetype) -> Result<(), String> {
    let mut edits = Vec::new();
    match arch {
        TableArchetype::Xy(t) => collect_dataset_edits(&t.data_sets, &mut edits),
        TableArchetype::Column(t) => collect_dataset_edits(&t.data_sets, &mut edits),
        TableArchetype::Grouped(t) => collect_dataset_edits(&t.data_sets, &mut edits),
        TableArchetype::Contingency(t) => collect_dataset_edits(&t.data_sets, &mut edits),
        TableArchetype::Survival(t) => collect_dataset_edits(&t.cohorts, &mut edits),
        TableArchetype::PartsOfWhole(t) => collect_dataset_edits(&t.data_sets, &mut edits),
        TableArchetype::Nested(t) => {
            for (c, grp) in t.groups.iter().enumerate() {
                for (r, subs) in grp.cells.iter().enumerate() {
                    for (s, cell) in subs.iter().enumerate() {
                        if let Some(mut item) = cell_to_json_value(cell)
                            && let Some(obj) = item.as_object_mut()
                        {
                            obj.insert(String::from("row"), json!(r));
                            obj.insert(String::from("col"), json!(c));
                            obj.insert(String::from("subcol"), json!(s));
                            edits.push(item);
                        }
                    }
                }
            }
        }
        TableArchetype::MultipleVariables(t) => {
            for (c, var) in t.variables.iter().enumerate() {
                if matches!(var.source, VariableSource::Calculated { .. }) {
                    continue;
                }
                for (r, cell) in var.cells.iter().enumerate() {
                    if let Some(mut item) = cell_to_json_value(cell)
                        && let Some(obj) = item.as_object_mut()
                    {
                        obj.insert(String::from("row"), json!(r));
                        obj.insert(String::from("col"), json!(c));
                        obj.insert(String::from("subcol"), json!(0));
                        edits.push(item);
                    }
                }
            }
        }
    }
    if !edits.is_empty() {
        app.execute_command("table.setCells", json!({ "sheet_id": sid, "edits": edits })).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn collect_dataset_edits(cols: &[curvebook_doc::DataSetColumn], out: &mut Vec<Value>) {
    for (c, col) in cols.iter().enumerate() {
        for (r, subs) in col.cells.iter().enumerate() {
            for (s, cell) in subs.iter().enumerate() {
                if let Some(mut item) = cell_to_json_value(cell)
                    && let Some(obj) = item.as_object_mut()
                {
                    obj.insert(String::from("row"), json!(r));
                    obj.insert(String::from("col"), json!(c));
                    obj.insert(String::from("subcol"), json!(s));
                    out.push(item);
                }
            }
        }
    }
}

fn populate_sample_mv_roles_and_formulas(app: &mut UiApp, sid: &str, arch: &TableArchetype) -> Result<(), String> {
    let TableArchetype::MultipleVariables(mv) = arch else {
        return Ok(());
    };
    for (col_idx, var) in mv.variables.iter().enumerate() {
        if let VariableSource::Calculated { ref formula } = var.source {
            app.execute_command("table.setCalculatedVariable", json!({ "sheet_id": sid, "col": col_idx, "title": var.title, "formula": formula }))
                .map_err(|e| e.to_string())?;
        } else {
            let role_str = match var.role {
                VariableRole::Continuous => "continuous",
                VariableRole::Categorical => "categorical",
                VariableRole::Label => "label",
                VariableRole::Calculated => continue,
            };
            app.execute_command("table.setVariableRole", json!({ "sheet_id": sid, "col": col_idx, "role": role_str })).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn execute_instantiate_template(app: &mut UiApp, state: &mut WelcomeViewState) -> Result<UiEventOutcome, String> {
    let idx = state.active_template_index.filter(|&i| i < TEMPLATE_PRESETS.len()).unwrap_or(0);
    let preset = &TEMPLATE_PRESETS[idx];
    let title_override = read_text_override(app, TITLE_INPUT_ID).unwrap_or_else(|| preset.title.to_owned());

    let tpl_doc = build_template_project_doc(preset, &title_override)?;
    let bytes = save_template_bytes(&tpl_doc).map_err(|e| e.to_string())?;
    let b64 = encode_base64(&bytes);
    let res =
        app.execute_command("project.instantiateTemplate", json!({ "bytes_base64": b64, "title": title_override })).map_err(|e| e.to_string())?;

    app.nav.sync_from_document(app.session.document());
    close_welcome_overlay(app, state);
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("welcome.instantiate_template"),
        command_id: Some(String::from("project.instantiateTemplate")),
        command_result: Some(res.payload),
    })
}

fn build_template_project_doc(preset: &TemplatePreset, project_title: &str) -> Result<ProjectDocument, String> {
    let mut doc = ProjectDocument::new(project_title).map_err(|e| e.to_string())?;
    let draft = TableCreationDraft {
        kind: preset.kind,
        sheet_id: String::from("data-1"),
        title: String::from("Primary Table"),
        subcolumn_mode: preset.subcolumn_mode,
        replicates: preset.replicates,
        column_design: preset.column_design,
        ..TableCreationDraft::for_kind(preset.kind)
    };
    let mut arch = draft.build_archetype().map_err(|e| e.to_string())?;
    match &mut arch {
        TableArchetype::Xy(xy) => {
            xy.x_title = String::from("Log [Compound] (M)");
            if xy.data_sets.len() >= 2 {
                xy.data_sets[0].title = String::from("Reference");
                xy.data_sets[1].title = String::from("Test Compound");
            }
        }
        TableArchetype::Column(col) => {
            let names = ["Control", "Low Dose", "High Dose"];
            for (ds, name) in col.data_sets.iter_mut().zip(names) {
                ds.title = String::from(name);
            }
        }
        TableArchetype::Grouped(grp) => {
            grp.row_factor_title = String::from("Time Point");
            grp.column_factor_title = String::from("Treatment");
        }
        TableArchetype::Survival(surv) => {
            surv.time_title = String::from("Elapsed Days");
        }
        _ => {}
    }

    let info_id = SheetId::from("info-data-1");
    let info = InfoSheet::new_preseeded(info_id.clone(), String::from("Study Info")).map_err(|e| e.to_string())?;
    doc.add_info_sheet(info).map_err(|e| e.to_string())?;

    let mut data_sheet = DataSheet::new("data-1", "Primary Table", arch).map_err(|e| e.to_string())?;
    data_sheet.info_sheet_id = Some(info_id);
    let mut graph = GraphSheet::new_uninitialized_for_data("graph-data-1", &data_sheet).map_err(|e| e.to_string())?;
    graph.spec.kind = preset.graph_kind;
    graph.initialized = true;

    doc.add_data_sheet(data_sheet).map_err(|e| e.to_string())?;
    doc.add_graph_sheet(graph).map_err(|e| e.to_string())?;
    doc.select_in_navigator(&SheetId::from("data-1")).map_err(|e| e.to_string())?;
    Ok(doc)
}

fn execute_clone_pipeline(app: &mut UiApp, state: &mut WelcomeViewState) -> Result<UiEventOutcome, String> {
    let mut clone = app.nav.welcome.clone_draft.clone();
    if clone.source_sheet_id.is_none() {
        clone.source_sheet_id = app.document().data_sheets.first().map(|s| s.id.clone());
    }
    if let Some(custom_title) = read_text_override(app, CLONE_TITLE_INPUT_ID) {
        clone.options.title = custom_title;
    }
    clone.new_sheet_id = next_unique_sheet_id(app.document(), "data-clone").as_str().to_owned();
    let Some(intent) = clone.to_command_intent() else {
        bump_state(state, Some(String::from("Select an existing data sheet to clone")));
        return Ok(handled_outcome(String::from("welcome.clone.no_source")));
    };
    let res = app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
    app.nav.sync_from_document(app.session.document());
    close_welcome_overlay(app, state);
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("welcome.clone"),
        command_id: Some(intent.command_id),
        command_result: Some(res.payload),
    })
}

// ============================================================================
// Public View Builders (`build_welcome_launcher` & `build_portfolio_gallery`)
// ============================================================================

/// Render the portfolio gallery grid inside `rect`, delegating to [`build_welcome_launcher`]
/// when the Welcome wizard is open or when the workspace has zero sheets.
pub fn build_portfolio_gallery(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if builder.app.nav.welcome.open || (builder.app.nav.gallery.cards.is_empty() && builder.app.document().total_sheet_count() == 0) {
        build_welcome_launcher(builder, rect);
        return;
    }
    build_populated_portfolio_cards(builder, rect);
}

fn build_populated_portfolio_cards(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width < 120 || rect.height < 80 {
        return;
    }
    let scale = builder.scale();
    let is_rtl = is_rtl_locale(&builder.tree.locale);
    let cards = builder.app.nav.gallery.cards.clone();
    let active_filter = builder.app.nav.gallery.filter_folder;
    let total_sheets = builder.app.document().total_sheet_count();
    let pad = s_u32(10, scale, 6);

    let root_id = builder.push_node(NodeSpec {
        id: WidgetId::from("gallery.root"),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Dialog, "Project Gallery", FocusScope::Workspace, "gallery.root"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&root_id), rect, builder.palette.surface);

    let header_h = s_u32(34, scale, 26).min(rect.height / 3);
    let header_rect = LayoutRectPx::new(
        rect.x + i32::try_from(pad).unwrap_or(6),
        rect.y + i32::try_from(pad / 2).unwrap_or(3),
        rect.width.saturating_sub(pad * 2),
        header_h,
    );
    let close_w = s_u32(64, scale, 52).min(header_rect.width / 4);
    let title_w = s_u32(148, scale, 100).min(header_rect.width / 3);
    let title_local_x = mirror_x_for_rtl(0, title_w, header_rect.width, is_rtl);
    let close_local_x = mirror_x_for_rtl(header_rect.width.saturating_sub(close_w), close_w, header_rect.width, is_rtl);

    let title_rect = LayoutRectPx::new(header_rect.x + i32::try_from(title_local_x).unwrap_or(0), header_rect.y, title_w, header_h);
    let title_text = format!("Gallery ({total_sheets})");
    let title_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("gallery.title"),
        parent: Some(root_id.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: title_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, title_text.clone(), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_text_fitted(
        builder,
        &title_wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: title_rect,
            pad_x: 4,
            text: &title_text,
            style: &UiTextStyle::ui_bold((13.0 * scale).clamp(10.0, 24.0)),
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    let close_rect =
        LayoutRectPx::new(header_rect.x + i32::try_from(close_local_x).unwrap_or(0), header_rect.y + 2, close_w, header_h.saturating_sub(4).max(18));
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "gallery.close",
            parent: Some(root_id.clone()),
            z_layer: ZLayer::Workspace,
            rect: close_rect,
            label: "Close",
            keyboard_path: "gallery.close",
            focus_scope: FocusScope::Workspace,
            tab_order: Some(15),
            primary: false,
            selected: false,
            shortcut: Some("Escape"),
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::Custom { domain: String::from("welcome"), payload: String::from("gallery:close") },
        },
    );

    let filter_area_start = title_w + s_u32(8, scale, 4);
    let filter_area_w = header_rect.width.saturating_sub(filter_area_start + close_w + s_u32(8, scale, 4));
    let filters = [
        ("gallery.filter.all", "All", None, "all"),
        ("gallery.filter.data", "Data", Some(SheetFolderKind::Data), "data"),
        ("gallery.filter.info", "Info", Some(SheetFolderKind::Info), "info"),
        ("gallery.filter.results", "Results", Some(SheetFolderKind::Results), "results"),
        ("gallery.filter.graphs", "Graphs", Some(SheetFolderKind::Graphs), "graphs"),
        ("gallery.filter.layouts", "Layouts", Some(SheetFolderKind::Layouts), "layouts"),
    ];
    let f_gap = s_u32(4, scale, 2);
    let chip_w = (filter_area_w.saturating_sub(f_gap * 5) / 6).max(s_u32(36, scale, 28));
    for (idx, (id, lbl, target, slug)) in filters.iter().enumerate() {
        let offset = (idx as u32) * (chip_w + f_gap);
        if offset + chip_w > filter_area_w + 2 {
            break;
        }
        let local_x = mirror_x_for_rtl(filter_area_start + offset, chip_w, header_rect.width, is_rtl);
        emit_tab(
            builder,
            ButtonWidgetSpec {
                id,
                parent: Some(root_id.clone()),
                z_layer: ZLayer::Workspace,
                rect: LayoutRectPx::new(
                    header_rect.x + i32::try_from(local_x).unwrap_or(0),
                    header_rect.y + 2,
                    chip_w,
                    header_h.saturating_sub(4).max(18),
                ),
                label: lbl,
                keyboard_path: id,
                focus_scope: FocusScope::Workspace,
                tab_order: Some(20 + idx as u32),
                primary: false,
                selected: active_filter == *target,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("welcome"), payload: format!("gallery:filter:{slug}") },
            },
        );
    }

    let grid_y = header_rect.bottom() + i32::try_from(s_u32(6, scale, 4)).unwrap_or(6);
    let grid_h = u32::try_from((rect.bottom() - grid_y - i32::try_from(pad / 2).unwrap_or(3)).max(0)).unwrap_or(0);
    if grid_h < 32 {
        return;
    }
    let grid_rect = LayoutRectPx::new(header_rect.x, grid_y, header_rect.width, grid_h);
    let scroll_id = String::from("scroll.gallery.cards");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let sb_guess = scrollbar_thickness_px(scale).saturating_add(4);
    let avail_w = grid_rect.width.saturating_sub(sb_guess);
    let gap = s_u32(10, scale, 6);
    let cols = (avail_w / s_u32(196, scale, 136)).max(1) as usize;
    let cw = (avail_w.saturating_sub(gap.saturating_mul((cols as u32).saturating_add(1))) / (cols as u32).max(1)).max(s_u32(110, scale, 90));
    let ch = s_u32(112, scale, 78);
    let rows = cards.len().div_ceil(cols).max(1);
    let content_h = (rows as u32).saturating_mul(ch + gap).saturating_add(gap);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), grid_rect, [grid_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let grid_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("gallery.viewport"),
        parent: Some(root_id),
        z_layer: ZLayer::Workspace,
        rect_px: grid_rect,
        a11y: labeled_meta(WidgetRole::Label, "Portfolio Cards", FocusScope::Workspace, "gallery.viewport"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&grid_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::Workspace, Some(&grid_wid), grid_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, Some(&grid_wid), grid_rect, builder.palette.border, 1);

    let sb_reserve = if scroll_meta.has_vertical_overflow() { scrollbar_thickness_px(scale).saturating_add(2) } else { 0 };
    let grid_clip = LayoutRectPx::new(
        grid_rect.x + 2,
        grid_rect.y + 2,
        grid_rect.width.saturating_sub(sb_reserve.saturating_add(4)),
        grid_rect.height.saturating_sub(4),
    );
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let bold_px = (12.0 * scale).clamp(9.0, 22.0);
    let sub_px = (10.5 * scale).clamp(8.5, 19.0);

    builder.push_clip(grid_clip);
    if cards.is_empty() {
        let empty_rect = LayoutRectPx::new(grid_clip.x + 8, grid_clip.y + 12, grid_clip.width.saturating_sub(16), s_u32(28, scale, 20));
        let ew = builder.push_node(NodeSpec {
            id: WidgetId::from("gallery.empty"),
            parent: Some(grid_wid.clone()),
            z_layer: ZLayer::Workspace,
            rect_px: empty_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Label, "No sheets in this folder.", FocusScope::Workspace),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        paint_text_fitted(
            builder,
            &ew,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: empty_rect,
                pad_x: 6,
                text: "No sheets match the selected folder filter.",
                style: &UiTextStyle::ui_sans(sub_px),
                color: builder.palette.muted,
                scroll_container: Some(&scroll_id),
            },
        );
    } else {
        for (idx, card) in cards.iter().enumerate() {
            let col = idx % cols;
            let row = idx / cols;
            let cy = grid_clip.y + i32::try_from(gap + (row as u32) * (ch + gap)).unwrap_or(0) - y_scroll;
            if cy + i32::try_from(ch).unwrap_or(0) < grid_clip.y || cy > grid_clip.bottom() {
                continue;
            }
            let local_x = gap + (col as u32) * (cw + gap);
            let mirrored_x = mirror_x_for_rtl(local_x, cw, grid_clip.width, is_rtl);
            let cx = grid_clip.x + i32::try_from(mirrored_x).unwrap_or(0);
            let card_rect = LayoutRectPx::new(cx, cy, cw, ch);
            let cid = format!("gallery.card.{idx}");
            let a11y =
                super::a11y::selectable_meta(WidgetRole::Button, card.title.clone(), FocusScope::Workspace, format!("gallery.{idx}"), card.selected);
            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(cid),
                parent: Some(grid_wid.clone()),
                z_layer: ZLayer::Workspace,
                rect_px: card_rect,
                a11y,
                state: crate::layout::widgets::widget_state(builder, &format!("gallery.card.{idx}"), ControlEnablement::enabled()),
                action: WidgetAction::SwitchSheet { sheet_id: card.sheet_id.clone(), section: section_for_folder_kind(card.folder) },
            });
            let fill_col = if card.selected { builder.palette.header } else { builder.palette.panel };
            let stroke_col = if card.selected { builder.palette.accent } else { builder.palette.border };
            builder.paint_fill(ZLayer::Workspace, Some(&wid), card_rect, fill_col);
            builder.paint_stroke(ZLayer::Workspace, Some(&wid), card_rect, stroke_col, if card.selected { 2 } else { 1 });

            let tag_col = card.color_tag.unwrap_or(builder.palette.accent);
            let stripe_h = s_u32(4, scale, 3);
            let stripe_rect = LayoutRectPx::new(card_rect.x + 1, card_rect.y + 1, card_rect.width.saturating_sub(2), stripe_h);
            builder.paint_fill(ZLayer::Workspace, Some(&wid), stripe_rect, tag_col);

            let badge_h = s_u32(18, scale, 15);
            let badge_w = s_u32(54, scale, 42).min(card_rect.width.saturating_sub(16));
            let badge_local = mirror_x_for_rtl(8, badge_w, card_rect.width, is_rtl);
            let badge_rect = LayoutRectPx::new(
                card_rect.x + i32::try_from(badge_local).unwrap_or(8),
                card_rect.y + i32::try_from(stripe_h + 6).unwrap_or(10),
                badge_w,
                badge_h,
            );
            builder.paint_fill(ZLayer::Workspace, Some(&wid), badge_rect, builder.palette.surface);
            builder.paint_stroke(ZLayer::Workspace, Some(&wid), badge_rect, tag_col, 1);
            paint_text_fitted(
                builder,
                &wid,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: badge_rect,
                    pad_x: 3,
                    text: &card.badge_label,
                    style: &UiTextStyle::ui_bold((9.5 * scale).clamp(8.0, 17.0)),
                    color: builder.palette.text,
                    scroll_container: Some(&scroll_id),
                },
            );
            if card.active && card_rect.width >= s_u32(120, scale, 90) {
                let act_w = s_u32(36, scale, 28);
                let act_local = mirror_x_for_rtl(card_rect.width.saturating_sub(act_w + 8), act_w, card_rect.width, is_rtl);
                let act_rect = LayoutRectPx::new(card_rect.x + i32::try_from(act_local).unwrap_or(8), badge_rect.y, act_w, badge_h);
                paint_text_fitted(
                    builder,
                    &wid,
                    FittedTextSpec {
                        z_layer: ZLayer::Workspace,
                        rect: act_rect,
                        pad_x: 2,
                        text: "Active",
                        style: &UiTextStyle::ui_sans((9.0 * scale).clamp(8.0, 15.0)),
                        color: builder.palette.muted,
                        scroll_container: Some(&scroll_id),
                    },
                );
            }

            let title_h = s_u32(24, scale, 18);
            let card_title_rect = LayoutRectPx::new(card_rect.x + 6, badge_rect.bottom() + 4, card_rect.width.saturating_sub(12), title_h);
            paint_text_fitted(
                builder,
                &wid,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: card_title_rect,
                    pad_x: 4,
                    text: &card.title,
                    style: &UiTextStyle::ui_bold(bold_px),
                    color: builder.palette.text,
                    scroll_container: Some(&scroll_id),
                },
            );

            let sub_h = u32::try_from((card_rect.bottom() - card_title_rect.bottom() - 4).max(0)).unwrap_or(0);
            if sub_h >= 12 && !card.subtitle.is_empty() {
                let sub_rect = LayoutRectPx::new(
                    card_rect.x + 6,
                    card_title_rect.bottom() + 2,
                    card_rect.width.saturating_sub(12),
                    sub_h.min(s_u32(22, scale, 16)),
                );
                paint_text_fitted(
                    builder,
                    &wid,
                    FittedTextSpec {
                        z_layer: ZLayer::Workspace,
                        rect: sub_rect,
                        pad_x: 4,
                        text: &card.subtitle,
                        style: &UiTextStyle::ui_sans(sub_px),
                        color: builder.palette.muted,
                        scroll_container: Some(&scroll_id),
                    },
                );
            }
        }
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(grid_wid));
}

/// Render the full interactive two-pane Welcome & New Data Table Launcher inside `rect` (`R10.4`).
pub fn build_welcome_launcher(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width < 120 || rect.height < 80 {
        return;
    }
    let scale = builder.scale();
    let is_rtl = is_rtl_locale(&builder.tree.locale);
    let pad = s_u32(10, scale, 6);

    builder.paint_fill(ZLayer::Workspace, None, rect, builder.palette.surface);
    builder.push_clip(rect);

    let root_id = builder.push_node(NodeSpec {
        id: WidgetId::from("welcome.root"),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: labeled_meta(
            WidgetRole::Dialog,
            welcome_tr(&builder.tree.locale, "welcome.title", "Welcome & New Data Table"),
            FocusScope::Workspace,
            "F1",
        ),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });

    let inner = rect.inset(pad);
    let header_h = s_u32(44, scale, 32).min(inner.height / 4);
    let header_rect = LayoutRectPx::new(inner.x, inner.y, inner.width, header_h);
    build_header(builder, &root_id, header_rect, is_rtl, scale);

    let body_top = header_rect.bottom() + s_i32(8, scale);
    let body_h = u32::try_from((inner.bottom() - body_top).max(0)).unwrap_or(0);
    if body_h < 60 {
        builder.pop_clip();
        return;
    }
    let body_rect = LayoutRectPx::new(inner.x, body_top, inner.width, body_h);
    let gap = s_u32(10, scale, 6);
    let left_w = (body_rect.width * 30 / 100).clamp(s_u32(180, scale, 130), s_u32(270, scale, 180)).min(body_rect.width / 2);
    let right_w = body_rect.width.saturating_sub(left_w + gap);

    let left_rect = place_rtl(body_rect, 0, body_rect.y, left_w, body_rect.height, is_rtl);
    let right_rect = place_rtl(body_rect, i32::try_from(left_w + gap).unwrap_or(0), body_rect.y, right_w, body_rect.height, is_rtl);

    build_left_rail(builder, &root_id, left_rect, is_rtl, scale);
    build_right_pane(builder, &root_id, right_rect, is_rtl, scale);

    builder.pop_clip();
}

fn build_header(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_rtl: bool, scale: f64) {
    builder.paint_fill(ZLayer::Workspace, Some(parent), rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, Some(parent), rect, builder.palette.border, 1);

    let locale = builder.tree.locale.clone();
    let title = welcome_tr(&locale, "welcome.title", "Welcome & New Data Table");
    let subtitle = welcome_tr(&locale, "welcome.subtitle", "Select a scientific data archetype, tutorial sample, template, or pipeline clone");
    let status = builder.app.view_shards.welcome.status_message.as_deref();

    let status_w = if status.is_some() { rect.width * 32 / 100 } else { 0 };
    let text_w = rect.width.saturating_sub(status_w + s_u32(16, scale, 8));
    let title_h = (rect.height * 54 / 100).max(s_u32(18, scale, 14));
    let sub_h = rect.height.saturating_sub(title_h + 2);

    let title_rect = place_rtl(rect, s_i32(8, scale), rect.y + 2, text_w, title_h, is_rtl);
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: title_rect,
            pad_x: 4,
            text: title,
            style: &UiTextStyle::ui_bold((14.0 * scale).clamp(11.0, 26.0)),
            color: builder.palette.text,
            scroll_container: None,
        },
    );
    if sub_h >= 12 {
        let sub_rect = place_rtl(rect, s_i32(8, scale), title_rect.bottom(), text_w, sub_h, is_rtl);
        paint_text_fitted(
            builder,
            parent,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: sub_rect,
                pad_x: 4,
                text: subtitle,
                style: &UiTextStyle::ui_sans((11.0 * scale).clamp(9.0, 20.0)),
                color: builder.palette.muted,
                scroll_container: None,
            },
        );
    }

    if let Some(msg) = status
        && status_w >= 80
    {
        let pill = place_rtl(
            rect,
            i32::try_from(text_w + s_u32(10, scale, 4)).unwrap_or(0),
            rect.y + s_i32(8, scale),
            status_w.saturating_sub(4),
            rect.height.saturating_sub(s_u32(16, scale, 6)),
            is_rtl,
        );
        builder.paint_fill(ZLayer::Workspace, Some(parent), pill, builder.palette.surface);
        builder.paint_stroke(ZLayer::Workspace, Some(parent), pill, builder.palette.accent, 1);
        paint_text_fitted(
            builder,
            parent,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: pill,
                pad_x: 6,
                text: msg,
                style: &UiTextStyle::ui_sans((10.5 * scale).clamp(8.5, 19.0)),
                color: builder.palette.accent,
                scroll_container: None,
            },
        );
    }
}

// ============================================================================
// Left Rail (All 8 Archetypes + Open File + Clone + .cbtemplate Templates)
// ============================================================================

fn build_left_rail(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_rtl: bool, scale: f64) {
    let scroll_id = String::from("scroll.welcome.left_rail");
    let hdr_h = s_u32(18, scale, 14);
    let btn_h = s_u32(26, scale, 22);
    let row_step = btn_h + s_u32(4, scale, 3);
    let sec_gap = s_u32(10, scale, 6);
    let total_content_h = hdr_h * 2 + row_step * 11 + sec_gap + s_u32(14, scale, 8);
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), rect, [rect.width, total_content_h], scroll_off, ScrollAxes::Vertical);

    let rail_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("welcome.left_rail"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Label, "Table Archetypes & Project Starters", FocusScope::Workspace, "Tab"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&rail_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::Workspace, Some(&rail_wid), rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, Some(&rail_wid), rect, builder.palette.border, 1);

    let sb_thick = if scroll_meta.has_vertical_overflow() { scrollbar_thickness_px(scale) + 2 } else { 0 };
    let clip_x = if is_rtl { rect.x + i32::try_from(sb_thick).unwrap_or(0) + 4 } else { rect.x + 4 };
    let clip = LayoutRectPx::new(clip_x, rect.y + 4, rect.width.saturating_sub(sb_thick + 8), rect.height.saturating_sub(8));
    builder.push_clip(clip);

    let locale = builder.tree.locale.clone();
    let mode = builder.app.nav.welcome.mode;
    let active_kind = builder.app.nav.welcome.table_draft.kind;
    let is_table_mode = matches!(mode, WelcomeMode::EmptyTypedTable | WelcomeMode::TutorialSample);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let mut cy = clip.y + 2 - y_scroll;

    let arch_hdr_rect = LayoutRectPx::new(clip.x, cy, clip.width, hdr_h);
    if rect_fits_inside(arch_hdr_rect, clip) {
        paint_text_fitted(
            builder,
            &rail_wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: arch_hdr_rect,
                pad_x: 4,
                text: welcome_tr(&locale, "welcome.rail.archetypes", "Data Table Archetypes"),
                style: &UiTextStyle::ui_bold((11.0 * scale).clamp(9.0, 20.0)),
                color: builder.palette.muted,
                scroll_container: None,
            },
        );
    }
    cy += i32::try_from(hdr_h + 2).unwrap_or(18);

    for (idx, kind) in TableKind::ALL.into_iter().enumerate() {
        let row_rect = LayoutRectPx::new(clip.x + 2, cy, clip.width.saturating_sub(4), btn_h);
        if rect_fits_inside(row_rect, clip) {
            let slug = archetype_slug(kind);
            let lbl = table_kind_label(&locale, kind);
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("welcome.arch.{slug}"),
                    parent: Some(rail_wid.clone()),
                    z_layer: ZLayer::Workspace,
                    rect: row_rect,
                    label: lbl,
                    keyboard_path: &format!("welcome.arch.{slug}"),
                    focus_scope: FocusScope::Workspace,
                    tab_order: Some(10 + idx as u32),
                    primary: false,
                    selected: is_table_mode && active_kind == kind,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("welcome"), payload: format!("arch:{slug}") },
                },
            );
        }
        cy += i32::try_from(row_step).unwrap_or(28);
    }

    cy += i32::try_from(sec_gap).unwrap_or(8);
    let starter_hdr = LayoutRectPx::new(clip.x, cy, clip.width, hdr_h);
    if rect_fits_inside(starter_hdr, clip) {
        paint_text_fitted(
            builder,
            &rail_wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: starter_hdr,
                pad_x: 4,
                text: welcome_tr(&locale, "welcome.rail.actions", "Project Starters"),
                style: &UiTextStyle::ui_bold((11.0 * scale).clamp(9.0, 20.0)),
                color: builder.palette.muted,
                scroll_container: None,
            },
        );
    }
    cy += i32::try_from(hdr_h + 2).unwrap_or(18);

    let has_sheets = !builder.app.document().data_sheets.is_empty();
    let clone_enable = if has_sheets {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Clone a Graph / Pipeline requires at least one existing Data Sheet in the project.")
    };

    let starters: [(&str, &str, bool, ControlEnablement, &str, u32); 3] = [
        (
            "welcome.action.open_existing",
            welcome_tr(&locale, "welcome.action.open_existing", "Open Existing File..."),
            false,
            ControlEnablement::enabled(),
            "action:open_file",
            20,
        ),
        ("welcome.mode.clone", tr(&locale, "btn.clone_pipeline"), mode == WelcomeMode::ClonePipeline, clone_enable, "mode:clone", 21),
        (
            "welcome.mode.template",
            welcome_tr(&locale, "welcome.action.templates", ".cbtemplate Templates"),
            mode == WelcomeMode::FromTemplate,
            ControlEnablement::enabled(),
            "mode:template",
            22,
        ),
    ];

    for (wid, label, selected, enablement, payload, tab_ord) in starters {
        let row_rect = LayoutRectPx::new(clip.x + 2, cy, clip.width.saturating_sub(4), btn_h);
        if rect_fits_inside(row_rect, clip) {
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: wid,
                    parent: Some(rail_wid.clone()),
                    z_layer: ZLayer::Workspace,
                    rect: row_rect,
                    label,
                    keyboard_path: wid,
                    focus_scope: FocusScope::Workspace,
                    tab_order: Some(tab_ord),
                    primary: false,
                    selected,
                    shortcut: None,
                    enablement,
                    action: WidgetAction::Custom { domain: String::from("welcome"), payload: payload.to_owned() },
                },
            );
        }
        cy += i32::try_from(row_step).unwrap_or(28);
    }

    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(rail_wid));
}

// ============================================================================
// Right Pane (Mode Strip + Scrollable Setup/Schematic + Pinned Footer CTA)
// ============================================================================

fn build_right_pane(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_rtl: bool, scale: f64) {
    let right_id = builder.push_node(NodeSpec {
        id: WidgetId::from("welcome.right_pane"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Label, "Table Configuration Pane", FocusScope::Workspace, "Tab"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&right_id), rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, Some(&right_id), rect, builder.palette.border, 1);

    let pad = s_u32(8, scale, 6);
    let inner = rect.inset(pad);
    if inner.width < 80 || inner.height < 80 {
        return;
    }

    let top_bar_h = s_u32(28, scale, 22);
    let footer_h = s_u32(34, scale, 26);
    let gap = s_u32(6, scale, 4);

    let top_rect = LayoutRectPx::new(inner.x, inner.y, inner.width, top_bar_h);
    build_right_top_bar(builder, &right_id, top_rect, is_rtl, scale);

    let footer_rect = LayoutRectPx::new(inner.x, inner.bottom() - i32::try_from(footer_h).unwrap_or(28), inner.width, footer_h);
    let body_top = top_rect.bottom() + i32::try_from(gap).unwrap_or(4);
    let body_h = u32::try_from((footer_rect.y - i32::try_from(gap).unwrap_or(4) - body_top).max(0)).unwrap_or(0);
    if body_h >= 40 {
        let body_rect = LayoutRectPx::new(inner.x, body_top, inner.width, body_h);
        match builder.app.nav.welcome.mode {
            WelcomeMode::EmptyTypedTable | WelcomeMode::TutorialSample => {
                build_table_or_sample_body(builder, &right_id, body_rect, is_rtl, scale);
            }
            WelcomeMode::FromTemplate => {
                build_template_body(builder, &right_id, body_rect, is_rtl, scale);
            }
            WelcomeMode::ClonePipeline => {
                build_clone_body(builder, &right_id, body_rect, is_rtl, scale);
            }
        }
    }

    build_footer_bar(builder, &right_id, footer_rect, is_rtl, scale);
}

fn build_right_top_bar(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_rtl: bool, scale: f64) {
    let locale = builder.tree.locale.clone();
    let mode = builder.app.nav.welcome.mode;
    match mode {
        WelcomeMode::EmptyTypedTable | WelcomeMode::TutorialSample => {
            let gap = s_u32(8, scale, 4);
            let tab_w = (rect.width.saturating_sub(gap)) / 2;
            let t0 = place_rtl(rect, 0, rect.y, tab_w, rect.height, is_rtl);
            let t1 = place_rtl(rect, i32::try_from(tab_w + gap).unwrap_or(0), rect.y, tab_w, rect.height, is_rtl);
            emit_tab(
                builder,
                ButtonWidgetSpec {
                    id: "welcome.tab.empty",
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::Workspace,
                    rect: t0,
                    label: welcome_tr(&locale, "welcome.mode.empty", "Empty Data Table"),
                    keyboard_path: "welcome.tab.empty",
                    focus_scope: FocusScope::Workspace,
                    tab_order: Some(30),
                    primary: false,
                    selected: mode == WelcomeMode::EmptyTypedTable,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("welcome"), payload: String::from("mode:empty") },
                },
            );
            emit_tab(
                builder,
                ButtonWidgetSpec {
                    id: "welcome.tab.sample",
                    parent: Some(parent.clone()),
                    z_layer: ZLayer::Workspace,
                    rect: t1,
                    label: welcome_tr(&locale, "welcome.mode.sample", "Tutorial Sample Data"),
                    keyboard_path: "welcome.tab.sample",
                    focus_scope: FocusScope::Workspace,
                    tab_order: Some(31),
                    primary: false,
                    selected: mode == WelcomeMode::TutorialSample,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("welcome"), payload: String::from("mode:sample") },
                },
            );
        }
        WelcomeMode::FromTemplate => {
            paint_text_fitted(
                builder,
                parent,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect,
                    pad_x: 4,
                    text: welcome_tr(&locale, "welcome.template.heading", "Curated .cbtemplate Project Starters"),
                    style: &UiTextStyle::ui_bold((13.0 * scale).clamp(10.0, 24.0)),
                    color: builder.palette.text,
                    scroll_container: None,
                },
            );
        }
        WelcomeMode::ClonePipeline => {
            paint_text_fitted(
                builder,
                parent,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect,
                    pad_x: 4,
                    text: welcome_tr(&locale, "welcome.clone.heading", "Clone a Graph / Analysis Pipeline"),
                    style: &UiTextStyle::ui_bold((13.0 * scale).clamp(10.0, 24.0)),
                    color: builder.palette.text,
                    scroll_container: None,
                },
            );
        }
    }
}

fn build_footer_bar(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_rtl: bool, scale: f64) {
    builder.paint_line(ZLayer::Workspace, Some(parent), [rect.x, rect.y], [rect.right(), rect.y], builder.palette.border, 1);

    let locale = builder.tree.locale.clone();
    let mode = builder.app.nav.welcome.mode;
    let btn_y = rect.y + s_i32(6, scale);
    let btn_h = rect.height.saturating_sub(s_u32(6, scale, 4)).max(20);
    let cta_w = s_u32(190, scale, 130).min(rect.width * 55 / 100);
    let cancel_w = s_u32(105, scale, 75).min(rect.width * 32 / 100);
    let gap = s_u32(8, scale, 6);

    let cta_rel_x = i32::try_from(rect.width.saturating_sub(cta_w)).unwrap_or(0);
    let cancel_rel_x = i32::try_from(rect.width.saturating_sub(cta_w + gap + cancel_w)).unwrap_or(0);
    let cta_rect = place_rtl(rect, cta_rel_x, btn_y, cta_w, btn_h, is_rtl);
    let cancel_rect = place_rtl(rect, cancel_rel_x, btn_y, cancel_w, btn_h, is_rtl);

    let (cta_label, cta_payload, cta_enable) = match mode {
        WelcomeMode::EmptyTypedTable => {
            (welcome_tr(&locale, "welcome.btn.create_table", "Create Table"), "action:create", ControlEnablement::enabled())
        }
        WelcomeMode::TutorialSample => {
            (welcome_tr(&locale, "welcome.btn.load_sample", "Open Sample Dataset"), "action:create", ControlEnablement::enabled())
        }
        WelcomeMode::FromTemplate => (
            welcome_tr(&locale, "welcome.btn.instantiate_template", "Start from Template"),
            "action:instantiate_template",
            ControlEnablement::enabled(),
        ),
        WelcomeMode::ClonePipeline => {
            let ok = builder.app.nav.welcome.clone_draft.source_sheet_id.is_some() && !builder.app.document().data_sheets.is_empty();
            let enable = if ok { ControlEnablement::enabled() } else { ControlEnablement::disabled("No source Data Sheet selected to clone.") };
            (welcome_tr(&locale, "welcome.btn.clone_pipeline", "Clone Selected Pipeline"), "action:clone", enable)
        }
    };

    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "welcome.cancel",
            parent: Some(parent.clone()),
            z_layer: ZLayer::Workspace,
            rect: cancel_rect,
            label: tr(&locale, "btn.cancel"),
            keyboard_path: "welcome.cancel",
            focus_scope: FocusScope::Workspace,
            tab_order: Some(98),
            primary: false,
            selected: false,
            shortcut: Some("Escape"),
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::Custom { domain: String::from("welcome"), payload: String::from("action:cancel") },
        },
    );

    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "welcome.create_cta",
            parent: Some(parent.clone()),
            z_layer: ZLayer::Workspace,
            rect: cta_rect,
            label: cta_label,
            keyboard_path: "welcome.create_cta",
            focus_scope: FocusScope::Workspace,
            tab_order: Some(99),
            primary: true,
            selected: false,
            shortcut: Some("Enter"),
            enablement: cta_enable,
            action: WidgetAction::Custom { domain: String::from("welcome"), payload: cta_payload.to_owned() },
        },
    );
}

// ============================================================================
// Right Body: Empty Table & Tutorial Sample Setup
// ============================================================================

fn build_table_or_sample_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_rtl: bool, scale: f64) {
    let wizard = &builder.app.nav.welcome;
    let draft = &wizard.table_draft;
    let is_sample = wizard.mode == WelcomeMode::TutorialSample;
    let locale = builder.tree.locale.clone();

    let schematic_h = s_u32(104, scale, 82);
    let sample_picker_h = if is_sample { s_u32(106, scale, 84) } else { 0 };
    let title_row_h = s_u32(42, scale, 34);
    let x_sec_h = s_u32(44, scale, 36);
    let y_sec_h = s_u32(72, scale, 60);
    let col_sec_h = s_u32(44, scale, 36);
    let toggle_sec_h = s_u32(28, scale, 22);
    let gap = s_u32(6, scale, 4);

    let content_h =
        schematic_h + (if is_sample { sample_picker_h + gap } else { 0 }) + title_row_h + x_sec_h + y_sec_h + col_sec_h + toggle_sec_h + gap * 6 + 6;

    let scroll_id = String::from("scroll.welcome.right_body");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), rect, [rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let body_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("welcome.right_body"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Label, "Table Configuration Form", FocusScope::Workspace, "Tab"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&body_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::Workspace, Some(&body_wid), rect, builder.palette.surface);

    let sb_thick = if scroll_meta.has_vertical_overflow() { scrollbar_thickness_px(scale) + 2 } else { 0 };
    let clip_x = if is_rtl { rect.x + i32::try_from(sb_thick).unwrap_or(0) + 2 } else { rect.x + 2 };
    let clip = LayoutRectPx::new(clip_x, rect.y + 2, rect.width.saturating_sub(sb_thick + 4), rect.height.saturating_sub(4));
    builder.push_clip(clip);

    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let mut cy = clip.y + 2 - y_scroll;

    // 1. Original structural vector schematic card
    let schem_rect = LayoutRectPx::new(clip.x + 2, cy, clip.width.saturating_sub(4), schematic_h);
    if rect_fits_inside(schem_rect, clip) {
        build_structural_schematic(
            builder,
            &body_wid,
            schem_rect,
            draft.kind,
            &draft.x_mode,
            draft.subcolumn_mode,
            draft.column_design,
            draft.data_set_count,
            is_rtl,
            scale,
        );
    }
    cy += i32::try_from(schematic_h + gap).unwrap_or(110);

    // 2. Clean-room tutorial sample dataset selector (when in TutorialSample mode)
    if is_sample {
        let samp_rect = LayoutRectPx::new(clip.x + 2, cy, clip.width.saturating_sub(4), sample_picker_h);
        if rect_fits_inside(samp_rect, clip) {
            build_sample_picker_section(builder, &body_wid, samp_rect, is_rtl, scale);
        }
        cy += i32::try_from(sample_picker_h + gap).unwrap_or(110);
    }

    // 3. Sheet title + dimensions (Datasets, Rows)
    let title_sec_rect = LayoutRectPx::new(clip.x + 2, cy, clip.width.saturating_sub(4), title_row_h);
    if rect_fits_inside(title_sec_rect, clip) {
        build_title_and_dimensions_row(builder, &body_wid, title_sec_rect, is_sample, is_rtl, scale);
    }
    cy += i32::try_from(title_row_h + gap).unwrap_or(48);

    // 4. X Column Format options
    let x_rect = LayoutRectPx::new(clip.x + 2, cy, clip.width.saturating_sub(4), x_sec_h);
    if rect_fits_inside(x_rect, clip) {
        build_x_column_section(builder, &body_wid, x_rect, is_sample, is_rtl, scale);
    }
    cy += i32::try_from(x_sec_h + gap).unwrap_or(50);

    // 5. Y Replicates + Subcolumn Error Format options
    let y_rect = LayoutRectPx::new(clip.x + 2, cy, clip.width.saturating_sub(4), y_sec_h);
    if rect_fits_inside(y_rect, clip) {
        build_y_subcolumn_section(builder, &body_wid, y_rect, is_sample, is_rtl, scale);
    }
    cy += i32::try_from(y_sec_h + gap).unwrap_or(78);

    // 6. Column Experimental Design (Unpaired / Paired / Pre-computed)
    let col_rect = LayoutRectPx::new(clip.x + 2, cy, clip.width.saturating_sub(4), col_sec_h);
    if rect_fits_inside(col_rect, clip) {
        build_column_design_section(builder, &body_wid, col_rect, is_sample, is_rtl, scale);
    }
    cy += i32::try_from(col_sec_h + gap).unwrap_or(50);

    // 7. Auto-created sheets toggles (`create_default_graph` & `create_paired_info`)
    let tog_rect = LayoutRectPx::new(clip.x + 2, cy, clip.width.saturating_sub(4), toggle_sec_h);
    if rect_fits_inside(tog_rect, clip) {
        let half_w = (tog_rect.width.saturating_sub(8)) / 2;
        let g_rect = place_rtl(tog_rect, 0, tog_rect.y + 2, half_w, tog_rect.height.saturating_sub(4), is_rtl);
        let i_rect = place_rtl(tog_rect, i32::try_from(half_w + 8).unwrap_or(0), tog_rect.y + 2, half_w, tog_rect.height.saturating_sub(4), is_rtl);
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "welcome.toggle.auto_graph",
                parent: Some(body_wid.clone()),
                z_layer: ZLayer::Workspace,
                rect: g_rect,
                label: welcome_tr(&locale, "welcome.toggle.auto_graph", "Create linked graph sheet"),
                keyboard_path: "welcome.toggle.auto_graph",
                focus_scope: FocusScope::Workspace,
                tab_order: Some(85),
                checked: draft.create_default_graph,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: None,
                action: WidgetAction::Custom { domain: String::from("welcome"), payload: String::from("toggle:auto_graph") },
            },
        );
        emit_toggle(
            builder,
            ToggleWidgetSpec {
                id: "welcome.toggle.auto_info",
                parent: Some(body_wid.clone()),
                z_layer: ZLayer::Workspace,
                rect: i_rect,
                label: welcome_tr(&locale, "welcome.toggle.auto_info", "Create paired Info sheet"),
                keyboard_path: "welcome.toggle.auto_info",
                focus_scope: FocusScope::Workspace,
                tab_order: Some(86),
                checked: draft.create_paired_info,
                is_radio: false,
                enablement: ControlEnablement::enabled(),
                scroll_container: None,
                action: WidgetAction::Custom { domain: String::from("welcome"), payload: String::from("toggle:auto_info") },
            },
        );
    }

    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(body_wid));
}

fn build_sample_picker_section(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_rtl: bool, scale: f64) {
    let selected_id = builder.app.nav.welcome.selected_fixture_id.as_deref();
    let cols = 3usize;
    let gap_x = s_u32(4, scale, 3);
    let gap_y = s_u32(3, scale, 2);
    let rows_count = 3usize;
    let summary_h = s_u32(18, scale, 14);
    let grid_h = rect.height.saturating_sub(summary_h + gap_y);
    let btn_h = (grid_h.saturating_sub(gap_y * (rows_count as u32 - 1))) / rows_count as u32;
    let col_w = (rect.width.saturating_sub(gap_x * (cols as u32 - 1))) / cols as u32;

    for (idx, fixture) in SAMPLE_FIXTURES.iter().take(9).enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let rx = i32::try_from(col as u32 * (col_w + gap_x)).unwrap_or(0);
        let ry = rect.y + i32::try_from(row as u32 * (btn_h + gap_y)).unwrap_or(0);
        let btn_rect = place_rtl(rect, rx, ry, col_w, btn_h, is_rtl);
        let selected = selected_id == Some(fixture.id);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("welcome.fixture.{}", fixture.id),
                parent: Some(parent.clone()),
                z_layer: ZLayer::Workspace,
                rect: btn_rect,
                label: fixture.title,
                keyboard_path: &format!("welcome.fixture.{}", fixture.id),
                focus_scope: FocusScope::Workspace,
                tab_order: Some(32 + idx as u32),
                primary: false,
                selected,
                shortcut: None,
                enablement: ControlEnablement::enabled(),
                action: WidgetAction::Custom { domain: String::from("welcome"), payload: format!("fixture:{}", fixture.id) },
            },
        );
    }

    if let Some(active) = selected_id.and_then(|id| SAMPLE_FIXTURES.iter().find(|f| f.id == id)).or_else(|| SAMPLE_FIXTURES.first()) {
        let sum_rect = LayoutRectPx::new(rect.x, rect.bottom() - i32::try_from(summary_h).unwrap_or(16), rect.width, summary_h);
        let summary_text = format!("{} rows x {} cols ({} reps) — {}", active.row_count, active.column_count, active.replicates, active.summary);
        paint_text_fitted(
            builder,
            parent,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: sum_rect,
                pad_x: 4,
                text: &summary_text,
                style: &UiTextStyle::ui_sans((10.5 * scale).clamp(8.5, 19.0)),
                color: builder.palette.muted,
                scroll_container: None,
            },
        );
    }
}

fn build_title_and_dimensions_row(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_sample: bool, is_rtl: bool, scale: f64) {
    let locale = builder.tree.locale.clone();
    let draft = &builder.app.nav.welcome.table_draft;
    let lbl_h = s_u32(16, scale, 13);
    let ctrl_h = rect.height.saturating_sub(lbl_h + 2).max(s_u32(20, scale, 18));
    let gap = s_u32(8, scale, 4);

    let title_w = (rect.width * 46 / 100).max(s_u32(140, scale, 90));
    let rem_w = rect.width.saturating_sub(title_w + gap * 2);
    let ds_w = rem_w / 2;
    let row_w = rem_w.saturating_sub(ds_w);

    let title_lbl_r = place_rtl(rect, 0, rect.y, title_w, lbl_h, is_rtl);
    let title_inp_r = place_rtl(rect, 0, title_lbl_r.bottom() + 2, title_w, ctrl_h, is_rtl);
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: title_lbl_r,
            pad_x: 2,
            text: welcome_tr(&locale, "welcome.field.title", "Sheet Title"),
            style: &UiTextStyle::ui_bold((10.5 * scale).clamp(8.5, 19.0)),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
    emit_text_field(
        builder,
        TextFieldSpec {
            id: TITLE_INPUT_ID,
            parent: Some(parent.clone()),
            z_layer: ZLayer::Workspace,
            rect: title_inp_r,
            label: welcome_tr(&locale, "welcome.field.title", "Sheet Title"),
            fallback_text: &draft.title,
            keyboard_path: TITLE_INPUT_ID,
            focus_scope: FocusScope::Workspace,
            tab_order: Some(45),
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );

    let ds_rel_x = i32::try_from(title_w + gap).unwrap_or(0);
    let ds_lbl_r = place_rtl(rect, ds_rel_x, rect.y, ds_w, lbl_h, is_rtl);
    let ds_ctrl_r = place_rtl(rect, ds_rel_x, ds_lbl_r.bottom() + 2, ds_w, ctrl_h, is_rtl);
    let sample_lock = ControlEnablement::disabled("Dimensions are preset by the tutorial sample dataset.");
    let dim_enable = if is_sample { sample_lock.clone() } else { ControlEnablement::enabled() };
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: ds_lbl_r,
            pad_x: 2,
            text: welcome_tr(&locale, "welcome.field.datasets", "Data Sets"),
            style: &UiTextStyle::ui_bold((10.5 * scale).clamp(8.5, 19.0)),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
    emit_welcome_stepper(
        builder,
        WelcomeStepperSpec {
            parent,
            rect: ds_ctrl_r,
            id_prefix: "welcome.datasets",
            payload_prefix: "datasets",
            value_text: &draft.data_set_count.to_string(),
            tab_order: 46,
            enablement: dim_enable.clone(),
            scale,
        },
    );

    let rows_rel_x = i32::try_from(title_w + gap * 2 + ds_w).unwrap_or(0);
    let rows_lbl_r = place_rtl(rect, rows_rel_x, rect.y, row_w, lbl_h, is_rtl);
    let rows_ctrl_r = place_rtl(rect, rows_rel_x, rows_lbl_r.bottom() + 2, row_w, ctrl_h, is_rtl);
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: rows_lbl_r,
            pad_x: 2,
            text: welcome_tr(&locale, "welcome.field.rows", "Rows"),
            style: &UiTextStyle::ui_bold((10.5 * scale).clamp(8.5, 19.0)),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
    emit_welcome_stepper(
        builder,
        WelcomeStepperSpec {
            parent,
            rect: rows_ctrl_r,
            id_prefix: "welcome.rows",
            payload_prefix: "rows",
            value_text: &draft.row_count.to_string(),
            tab_order: 48,
            enablement: dim_enable,
            scale,
        },
    );
}

fn build_x_column_section(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_sample: bool, is_rtl: bool, scale: f64) {
    let locale = builder.tree.locale.clone();
    let draft = &builder.app.nav.welcome.table_draft;
    let lbl_h = s_u32(16, scale, 13);
    let btn_h = rect.height.saturating_sub(lbl_h + 2).max(s_u32(20, scale, 18));

    let hdr_r = LayoutRectPx::new(rect.x, rect.y, rect.width, lbl_h);
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: hdr_r,
            pad_x: 2,
            text: welcome_tr(&locale, "welcome.section.x_column", "X Column Format"),
            style: &UiTextStyle::ui_bold((10.5 * scale).clamp(8.5, 19.0)),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );

    let active_slug = match draft.kind {
        TableKind::Xy => match &draft.x_mode {
            XColumnMode::Numbers { .. } => "numbers",
            XColumnMode::DateOrElapsedTime { unit_label } if unit_label.eq_ignore_ascii_case("Date") => "dates",
            XColumnMode::DateOrElapsedTime { .. } => "elapsed",
            XColumnMode::GeneratedSequence { .. } => "sequence",
        },
        TableKind::Survival => match &draft.x_mode {
            XColumnMode::DateOrElapsedTime { unit_label } if unit_label.eq_ignore_ascii_case("Date") => "dates",
            _ => "elapsed",
        },
        _ => "none",
    };

    let options: [(&str, &str, &str); 5] = [
        ("none", "welcome.x_mode.none", "None"),
        ("numbers", "welcome.x_mode.numbers", "Numbers"),
        ("dates", "welcome.x_mode.dates", "Dates"),
        ("elapsed", "welcome.x_mode.elapsed", "Elapsed"),
        ("sequence", "welcome.x_mode.sequence", "Sequence"),
    ];

    let gap = s_u32(4, scale, 3);
    let btn_w = (rect.width.saturating_sub(gap * 4)) / 5;
    let btn_y = hdr_r.bottom() + 2;

    for (idx, (slug, i18n_key, fallback)) in options.into_iter().enumerate() {
        let rel_x = i32::try_from(idx as u32 * (btn_w + gap)).unwrap_or(0);
        let btn_r = place_rtl(rect, rel_x, btn_y, btn_w, btn_h, is_rtl);
        let enablement = x_option_enablement(draft.kind, slug, is_sample);
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("welcome.x_mode.{slug}"),
                parent: Some(parent.clone()),
                z_layer: ZLayer::Workspace,
                rect: btn_r,
                label: welcome_tr(&locale, i18n_key, fallback),
                keyboard_path: &format!("welcome.x_mode.{slug}"),
                focus_scope: FocusScope::Workspace,
                tab_order: Some(50 + idx as u32),
                primary: false,
                selected: active_slug == slug,
                shortcut: None,
                enablement,
                action: WidgetAction::Custom { domain: String::from("welcome"), payload: format!("x_mode:{slug}") },
            },
        );
    }
}

fn x_option_enablement(kind: TableKind, slug: &str, is_sample: bool) -> ControlEnablement {
    if is_sample {
        return ControlEnablement::disabled("X-column structure is defined by the tutorial sample dataset.");
    }
    match kind {
        TableKind::Xy => {
            if slug == "none" {
                ControlEnablement::disabled("XY tables require an independent X coordinate column (Numbers, Dates, Elapsed, or Sequence).")
            } else {
                ControlEnablement::enabled()
            }
        }
        TableKind::Survival => match slug {
            "dates" | "elapsed" => ControlEnablement::enabled(),
            _ => ControlEnablement::disabled("Survival tables track Time-to-Event using Date or Elapsed Time."),
        },
        _ => {
            if slug == "none" {
                ControlEnablement::enabled()
            } else {
                ControlEnablement::disabled("Only XY and Survival tables use an independent X/Time coordinate column.")
            }
        }
    }
}

fn build_y_subcolumn_section(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_sample: bool, is_rtl: bool, scale: f64) {
    let locale = builder.tree.locale.clone();
    let draft = &builder.app.nav.welcome.table_draft;
    let top_h = s_u32(20, scale, 16);
    let gap_y = s_u32(3, scale, 2);

    let top_r = LayoutRectPx::new(rect.x, rect.y, rect.width, top_h);
    let lbl_w = rect.width * 55 / 100;
    let lbl_r = place_rtl(top_r, 0, top_r.y, lbl_w, top_h, is_rtl);
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: lbl_r,
            pad_x: 2,
            text: welcome_tr(&locale, "welcome.section.y_format", "Y Values & Error Format"),
            style: &UiTextStyle::ui_bold((10.5 * scale).clamp(8.5, 19.0)),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );

    let step_w = s_u32(190, scale, 130).min(rect.width * 43 / 100);
    let step_rel_x = i32::try_from(rect.width.saturating_sub(step_w)).unwrap_or(0);
    let step_wrap = place_rtl(top_r, step_rel_x, top_r.y, step_w, top_h, is_rtl);
    let rep_lbl_w = step_wrap.width * 48 / 100;
    let rep_ctrl_w = step_wrap.width.saturating_sub(rep_lbl_w + 4);
    let rep_lbl_r = place_rtl(step_wrap, 0, step_wrap.y, rep_lbl_w, top_h, is_rtl);
    let rep_ctrl_r = place_rtl(step_wrap, i32::try_from(rep_lbl_w + 4).unwrap_or(0), step_wrap.y, rep_ctrl_w, top_h, is_rtl);
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: rep_lbl_r,
            pad_x: 2,
            text: welcome_tr(&locale, "welcome.field.replicates", "Replicates (k)"),
            style: &UiTextStyle::ui_sans((10.0 * scale).clamp(8.0, 18.0)),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
    let supports_raw_reps = matches!(draft.kind, TableKind::Xy | TableKind::Grouped | TableKind::Nested) && !draft.subcolumn_mode.is_pre_summarized();
    let rep_enable = if is_sample {
        ControlEnablement::disabled("Replicate count is fixed by the selected sample fixture.")
    } else if supports_raw_reps {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Raw replicate subcolumn count (k) applies to XY, Grouped, and Nested tables in Raw Replicates mode.")
    };
    emit_welcome_stepper(
        builder,
        WelcomeStepperSpec {
            parent,
            rect: rep_ctrl_r,
            id_prefix: "welcome.replicates",
            payload_prefix: "replicates",
            value_text: &draft.replicates.to_string(),
            tab_order: 58,
            enablement: rep_enable,
            scale,
        },
    );

    let sub_modes: [(&str, SubcolumnMode, &str, &str); 8] = [
        ("single", SubcolumnMode::Single, "welcome.subcol.single", "Single Y"),
        ("raw", SubcolumnMode::RawReplicates(draft.replicates.max(2)), "welcome.subcol.raw", "Replicates"),
        ("mean_sd_n", SubcolumnMode::MeanSdN, "welcome.subcol.mean_sd_n", "Mean / SD / N"),
        ("mean_sem_n", SubcolumnMode::MeanSemN, "welcome.subcol.mean_sem_n", "Mean / SEM / N"),
        ("mean_cv_n", SubcolumnMode::MeanCvN, "welcome.subcol.mean_cv_n", "Mean / %CV / N"),
        ("mean_low_high", SubcolumnMode::MeanLowHigh, "welcome.subcol.mean_low_high", "Mean / -Err / +Err"),
        ("upper_lower", SubcolumnMode::UpperLowerLimits, "welcome.subcol.upper_lower", "Mean / Lower / Upper"),
        ("median_iqr", SubcolumnMode::MedianIqr, "welcome.subcol.median_iqr", "Median / Q1 / Q3"),
    ];

    let grid_y = top_r.bottom() + i32::try_from(gap_y).unwrap_or(2);
    let grid_h = rect.height.saturating_sub(top_h + gap_y);
    let row_h = (grid_h.saturating_sub(gap_y)) / 2;
    let gap_x = s_u32(4, scale, 3);
    let col_w = (rect.width.saturating_sub(gap_x * 3)) / 4;

    for (idx, (slug, mode_val, key, fallback)) in sub_modes.into_iter().enumerate() {
        let col = idx % 4;
        let row = idx / 4;
        let rx = i32::try_from(col as u32 * (col_w + gap_x)).unwrap_or(0);
        let ry = grid_y + i32::try_from(row as u32 * (row_h + gap_y)).unwrap_or(0);
        let btn_r = place_rtl(rect, rx, ry, col_w, row_h, is_rtl);
        let selected = matches!(
            (draft.subcolumn_mode, slug),
            (SubcolumnMode::Single, "single")
                | (SubcolumnMode::RawReplicates(_), "raw")
                | (SubcolumnMode::MeanSdN, "mean_sd_n")
                | (SubcolumnMode::MeanSemN, "mean_sem_n")
                | (SubcolumnMode::MeanCvN, "mean_cv_n")
                | (SubcolumnMode::MeanLowHigh, "mean_low_high")
                | (SubcolumnMode::UpperLowerLimits, "upper_lower")
                | (SubcolumnMode::MedianIqr, "median_iqr")
        );
        let enablement = if is_sample {
            ControlEnablement::disabled("Subcolumn format is defined by the tutorial sample fixture.")
        } else if draft.kind.allows_subcolumn_mode(mode_val) {
            ControlEnablement::enabled()
        } else {
            ControlEnablement::disabled(format!("{} tables do not support `{}` subcolumns.", draft.kind.label(), fallback))
        };

        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("welcome.subcol.{slug}"),
                parent: Some(parent.clone()),
                z_layer: ZLayer::Workspace,
                rect: btn_r,
                label: welcome_tr(&locale, key, fallback),
                keyboard_path: &format!("welcome.subcol.{slug}"),
                focus_scope: FocusScope::Workspace,
                tab_order: Some(60 + idx as u32),
                primary: false,
                selected,
                shortcut: None,
                enablement,
                action: WidgetAction::Custom { domain: String::from("welcome"), payload: format!("subcol:{slug}") },
            },
        );
    }
}

fn build_column_design_section(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_sample: bool, is_rtl: bool, scale: f64) {
    let locale = builder.tree.locale.clone();
    let draft = &builder.app.nav.welcome.table_draft;
    let lbl_h = s_u32(16, scale, 13);
    let btn_h = rect.height.saturating_sub(lbl_h + 2).max(s_u32(20, scale, 18));

    let hdr_r = LayoutRectPx::new(rect.x, rect.y, rect.width, lbl_h);
    paint_text_fitted(
        builder,
        parent,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: hdr_r,
            pad_x: 2,
            text: welcome_tr(&locale, "welcome.section.col_design", "Column Experimental Design"),
            style: &UiTextStyle::ui_bold((10.5 * scale).clamp(8.5, 19.0)),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );

    let designs: [(&str, ColumnDesignHint, &str, &str); 3] = [
        ("unpaired", ColumnDesignHint::UnpairedReplicates, "welcome.design.unpaired", "Unpaired Groups"),
        ("paired", ColumnDesignHint::PairedRepeatedMeasures, "welcome.design.paired", "Paired / Repeated"),
        ("precomputed", ColumnDesignHint::PrecomputedError, "welcome.design.precomputed", "Pre-computed Summary"),
    ];
    let gap = s_u32(6, scale, 4);
    let btn_w = (rect.width.saturating_sub(gap * 2)) / 3;
    let btn_y = hdr_r.bottom() + 2;

    for (idx, (slug, hint, key, fallback)) in designs.into_iter().enumerate() {
        let rel_x = i32::try_from(idx as u32 * (btn_w + gap)).unwrap_or(0);
        let btn_r = place_rtl(rect, rel_x, btn_y, btn_w, btn_h, is_rtl);
        let enablement = if is_sample {
            ControlEnablement::disabled("Experimental design hint is defined by the selected sample.")
        } else if draft.kind == TableKind::Column {
            ControlEnablement::enabled()
        } else {
            ControlEnablement::disabled("Unpaired vs. Paired Column design hints apply only to Column tables.")
        };
        emit_button(
            builder,
            ButtonWidgetSpec {
                id: &format!("welcome.design.{slug}"),
                parent: Some(parent.clone()),
                z_layer: ZLayer::Workspace,
                rect: btn_r,
                label: welcome_tr(&locale, key, fallback),
                keyboard_path: &format!("welcome.design.{slug}"),
                focus_scope: FocusScope::Workspace,
                tab_order: Some(75 + idx as u32),
                primary: false,
                selected: draft.kind == TableKind::Column && draft.column_design == hint,
                shortcut: None,
                enablement,
                action: WidgetAction::Custom { domain: String::from("welcome"), payload: format!("col_design:{slug}") },
            },
        );
    }
}

struct WelcomeStepperSpec<'b> {
    parent: &'b WidgetId,
    rect: LayoutRectPx,
    id_prefix: &'b str,
    payload_prefix: &'b str,
    value_text: &'b str,
    tab_order: u32,
    enablement: ControlEnablement,
    scale: f64,
}

fn emit_welcome_stepper(builder: &mut LayoutBuilder<'_>, spec: WelcomeStepperSpec<'_>) {
    let rect = spec.rect;
    let btn_w = rect.height.clamp(s_u32(18, spec.scale, 16), s_u32(28, spec.scale, 22));
    let dec_rect = LayoutRectPx::new(rect.x, rect.y, btn_w, rect.height);
    let inc_rect = LayoutRectPx::new(rect.right() - i32::try_from(btn_w).unwrap_or(20), rect.y, btn_w, rect.height);
    let val_w = rect.width.saturating_sub(btn_w.saturating_mul(2)).max(20);
    let val_rect = LayoutRectPx::new(dec_rect.right(), rect.y, val_w, rect.height);

    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{}.dec", spec.id_prefix),
            parent: Some(spec.parent.clone()),
            z_layer: ZLayer::Workspace,
            rect: dec_rect,
            label: "-",
            keyboard_path: &format!("{}.dec", spec.id_prefix),
            focus_scope: FocusScope::Workspace,
            tab_order: Some(spec.tab_order),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: spec.enablement.clone(),
            action: WidgetAction::Custom { domain: String::from("welcome"), payload: format!("{}:dec", spec.payload_prefix) },
        },
    );

    let val_id = WidgetId::from(format!("{}.val", spec.id_prefix));
    let pushed_val = builder.push_node(NodeSpec {
        id: val_id,
        parent: Some(spec.parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: val_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, spec.value_text, FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&pushed_val), val_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, Some(&pushed_val), val_rect, builder.palette.border, 1);
    paint_text_fitted(
        builder,
        &pushed_val,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: val_rect,
            pad_x: 4,
            text: spec.value_text,
            style: &UiTextStyle::grid_tabular((11.5 * spec.scale).clamp(9.0, 22.0)),
            color: if spec.enablement.enabled { builder.palette.text } else { builder.palette.muted },
            scroll_container: None,
        },
    );

    emit_button(
        builder,
        ButtonWidgetSpec {
            id: &format!("{}.inc", spec.id_prefix),
            parent: Some(spec.parent.clone()),
            z_layer: ZLayer::Workspace,
            rect: inc_rect,
            label: "+",
            keyboard_path: &format!("{}.inc", spec.id_prefix),
            focus_scope: FocusScope::Workspace,
            tab_order: Some(spec.tab_order + 1),
            primary: false,
            selected: false,
            shortcut: None,
            enablement: spec.enablement,
            action: WidgetAction::Custom { domain: String::from("welcome"), payload: format!("{}:inc", spec.payload_prefix) },
        },
    );
}

// ============================================================================
// Right Body: .cbtemplate Starters & Clone Pipeline Options
// ============================================================================

fn build_template_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_rtl: bool, scale: f64) {
    let locale = builder.tree.locale.clone();
    let active_idx = builder.app.view_shards.welcome.active_template_index.filter(|&i| i < TEMPLATE_PRESETS.len()).unwrap_or(0);
    let active = &TEMPLATE_PRESETS[active_idx];

    let body_id = builder.push_node(NodeSpec {
        id: WidgetId::from("welcome.template_body"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Label, ".cbtemplate Templates", FocusScope::Workspace, "Tab"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&body_id), rect, builder.palette.surface);

    let clip = rect.inset(4);
    builder.push_clip(clip);

    let schem_h = s_u32(106, scale, 80);
    let schem_r = LayoutRectPx::new(clip.x, clip.y, clip.width, schem_h);
    build_structural_schematic(
        builder,
        &body_id,
        schem_r,
        active.kind,
        &XColumnMode::default(),
        active.subcolumn_mode,
        active.column_design,
        2,
        is_rtl,
        scale,
    );

    let title_lbl_r = LayoutRectPx::new(clip.x, schem_r.bottom() + 8, clip.width, s_u32(16, scale, 13));
    let title_inp_r = LayoutRectPx::new(clip.x, title_lbl_r.bottom() + 2, clip.width, s_u32(24, scale, 20));
    paint_text_fitted(
        builder,
        &body_id,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: title_lbl_r,
            pad_x: 2,
            text: welcome_tr(&locale, "welcome.field.title", "Sheet Title"),
            style: &UiTextStyle::ui_bold((10.5 * scale).clamp(8.5, 19.0)),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );
    emit_text_field(
        builder,
        TextFieldSpec {
            id: TITLE_INPUT_ID,
            parent: Some(body_id.clone()),
            z_layer: ZLayer::Workspace,
            rect: title_inp_r,
            label: welcome_tr(&locale, "welcome.field.title", "Sheet Title"),
            fallback_text: active.title,
            keyboard_path: TITLE_INPUT_ID,
            focus_scope: FocusScope::Workspace,
            tab_order: Some(40),
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );

    let mut cy = title_inp_r.bottom() + s_i32(10, scale);
    let card_h = s_u32(28, scale, 24);
    let desc_h = s_u32(16, scale, 13);
    for (idx, preset) in TEMPLATE_PRESETS.iter().enumerate() {
        let btn_r = LayoutRectPx::new(clip.x, cy, clip.width, card_h);
        if rect_fits_inside(btn_r, clip) {
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("welcome.template.{idx}"),
                    parent: Some(body_id.clone()),
                    z_layer: ZLayer::Workspace,
                    rect: btn_r,
                    label: preset.title,
                    keyboard_path: &format!("welcome.template.{idx}"),
                    focus_scope: FocusScope::Workspace,
                    tab_order: Some(45 + idx as u32),
                    primary: false,
                    selected: idx == active_idx,
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("welcome"), payload: format!("template:{idx}") },
                },
            );
        }
        let d_rect = LayoutRectPx::new(clip.x, btn_r.bottom() + 2, clip.width, desc_h);
        if rect_fits_inside(d_rect, clip) {
            paint_text_fitted(
                builder,
                &body_id,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: d_rect,
                    pad_x: 6,
                    text: preset.summary,
                    style: &UiTextStyle::ui_sans((10.0 * scale).clamp(8.0, 18.0)),
                    color: builder.palette.muted,
                    scroll_container: None,
                },
            );
        }
        cy = d_rect.bottom() + s_i32(6, scale);
    }

    builder.pop_clip();
}

fn build_clone_body(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, rect: LayoutRectPx, is_rtl: bool, scale: f64) {
    let locale = builder.tree.locale.clone();
    let clone = &builder.app.nav.welcome.clone_draft;
    let sheets = &builder.app.document().data_sheets;

    let body_id = builder.push_node(NodeSpec {
        id: WidgetId::from("welcome.clone_body"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Label, "Clone Pipeline Options", FocusScope::Workspace, "Tab"),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&body_id), rect, builder.palette.surface);

    let clip = rect.inset(4);
    builder.push_clip(clip);

    let lbl_h = s_u32(16, scale, 13);
    let src_hdr = LayoutRectPx::new(clip.x, clip.y, clip.width, lbl_h);
    paint_text_fitted(
        builder,
        &body_id,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: src_hdr,
            pad_x: 2,
            text: welcome_tr(&locale, "welcome.clone.source", "Source Data Sheet"),
            style: &UiTextStyle::ui_bold((10.5 * scale).clamp(8.5, 19.0)),
            color: builder.palette.muted,
            scroll_container: None,
        },
    );

    let mut cy = src_hdr.bottom() + 4;
    let src_btn_h = s_u32(24, scale, 20);
    let src_cols = 2usize;
    let src_gap = s_u32(6, scale, 4);
    let src_w = (clip.width.saturating_sub(src_gap)) / 2;
    let active_src = clone.source_sheet_id.as_ref().or_else(|| sheets.first().map(|s| &s.id));

    for (idx, ds) in sheets.iter().take(4).enumerate() {
        let col = idx % src_cols;
        let row = idx / src_cols;
        let rel_x = i32::try_from(col as u32 * (src_w + src_gap)).unwrap_or(0);
        let ry = cy + i32::try_from(row as u32 * (src_btn_h + 4)).unwrap_or(0);
        let btn_r = place_rtl(clip, rel_x, ry, src_w, src_btn_h, is_rtl);
        if rect_fits_inside(btn_r, clip) {
            emit_button(
                builder,
                ButtonWidgetSpec {
                    id: &format!("welcome.clone.source.{}", ds.id.as_str()),
                    parent: Some(body_id.clone()),
                    z_layer: ZLayer::Workspace,
                    rect: btn_r,
                    label: &format!("{} ({})", ds.title, ds.kind().label()),
                    keyboard_path: &format!("welcome.clone.source.{}", ds.id.as_str()),
                    focus_scope: FocusScope::Workspace,
                    tab_order: Some(40 + idx as u32),
                    primary: false,
                    selected: active_src == Some(&ds.id),
                    shortcut: None,
                    enablement: ControlEnablement::enabled(),
                    action: WidgetAction::Custom { domain: String::from("welcome"), payload: format!("clone_source:{}", ds.id.as_str()) },
                },
            );
        }
    }
    let src_rows = sheets.len().clamp(1, 4).div_ceil(2);
    cy += i32::try_from(src_rows as u32 * (src_btn_h + 4) + 6).unwrap_or(36);

    let title_lbl = LayoutRectPx::new(clip.x, cy, clip.width, lbl_h);
    let title_inp = LayoutRectPx::new(clip.x, title_lbl.bottom() + 2, clip.width, s_u32(24, scale, 20));
    if rect_fits_inside(title_lbl, clip) && rect_fits_inside(title_inp, clip) {
        paint_text_fitted(
            builder,
            &body_id,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: title_lbl,
                pad_x: 2,
                text: welcome_tr(&locale, "welcome.field.title", "Sheet Title"),
                style: &UiTextStyle::ui_bold((10.5 * scale).clamp(8.5, 19.0)),
                color: builder.palette.muted,
                scroll_container: None,
            },
        );
        emit_text_field(
            builder,
            TextFieldSpec {
                id: CLONE_TITLE_INPUT_ID,
                parent: Some(body_id.clone()),
                z_layer: ZLayer::Workspace,
                rect: title_inp,
                label: welcome_tr(&locale, "welcome.field.title", "Sheet Title"),
                fallback_text: &clone.options.title,
                keyboard_path: CLONE_TITLE_INPUT_ID,
                focus_scope: FocusScope::Workspace,
                tab_order: Some(49),
                multiline: false,
                enablement: ControlEnablement::enabled(),
            },
        );
    }
    cy = title_inp.bottom() + s_i32(10, scale);

    let tog_h = s_u32(24, scale, 20);
    let clone_toggles: [(&str, &str, bool, &str, u32); 7] = [
        (
            "welcome.clone.keep_x",
            welcome_tr(&locale, "welcome.clone.keep_x", "Keep X / time values"),
            clone.options.keep_x_values,
            "clone_toggle:keep_x",
            50,
        ),
        (
            "welcome.clone.clear_y",
            welcome_tr(&locale, "welcome.clone.clear_y", "Clear Y response cells"),
            clone.options.clear_y_values,
            "clone_toggle:clear_y",
            51,
        ),
        (
            "welcome.clone.clear_col_titles",
            welcome_tr(&locale, "welcome.clone.clear_col_titles", "Clear column titles"),
            clone.options.clear_column_titles,
            "clone_toggle:clear_col_titles",
            52,
        ),
        (
            "welcome.clone.clear_row_titles",
            welcome_tr(&locale, "welcome.clone.clear_row_titles", "Clear row titles"),
            clone.options.clear_row_titles,
            "clone_toggle:clear_row_titles",
            53,
        ),
        (
            "welcome.clone.analyses",
            welcome_tr(&locale, "welcome.clone.analyses", "Clone downstream analyses"),
            clone.clone_downstream_analyses,
            "clone_toggle:analyses",
            54,
        ),
        (
            "welcome.clone.graphs",
            welcome_tr(&locale, "welcome.clone.graphs", "Clone downstream graphs"),
            clone.clone_downstream_graphs,
            "clone_toggle:graphs",
            55,
        ),
        (
            "welcome.clone.layouts",
            welcome_tr(&locale, "welcome.clone.layouts", "Clone page layouts"),
            clone.clone_downstream_layouts,
            "clone_toggle:layouts",
            56,
        ),
    ];

    for (wid, label, checked, payload, tab_ord) in clone_toggles {
        let tr_rect = LayoutRectPx::new(clip.x, cy, clip.width, tog_h);
        if rect_fits_inside(tr_rect, clip) {
            emit_toggle(
                builder,
                ToggleWidgetSpec {
                    id: wid,
                    parent: Some(body_id.clone()),
                    z_layer: ZLayer::Workspace,
                    rect: tr_rect,
                    label,
                    keyboard_path: wid,
                    focus_scope: FocusScope::Workspace,
                    tab_order: Some(tab_ord),
                    checked,
                    is_radio: false,
                    enablement: ControlEnablement::enabled(),
                    scroll_container: None,
                    action: WidgetAction::Custom { domain: String::from("welcome"), payload: payload.to_owned() },
                },
            );
        }
        cy += i32::try_from(tog_h + 4).unwrap_or(28);
    }

    builder.pop_clip();
}

// ============================================================================
// Original Vector Table + Plot Structural Schematic (`R10.4`)
// ============================================================================

#[allow(clippy::too_many_arguments)]
fn build_structural_schematic(
    builder: &mut LayoutBuilder<'_>,
    parent: &WidgetId,
    rect: LayoutRectPx,
    kind: TableKind,
    x_mode: &XColumnMode,
    subcol: SubcolumnMode,
    col_design: ColumnDesignHint,
    data_sets: usize,
    is_rtl: bool,
    scale: f64,
) {
    let locale = builder.tree.locale.clone();
    let schem_id = builder.push_node(NodeSpec {
        id: WidgetId::from("welcome.schematic"),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, table_kind_label(&locale, kind), FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::Workspace, Some(&schem_id), rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, Some(&schem_id), rect, builder.palette.border, 1);

    let pad = s_u32(6, scale, 4);
    let inner = rect.inset(pad);
    let hdr_h = s_u32(16, scale, 13);
    let hdr_r = LayoutRectPx::new(inner.x, inner.y, inner.width, hdr_h);
    let hdr_text = format!(
        "{} — {} ({})",
        welcome_tr(&locale, "welcome.section.preview", "Table & Plot Structure Schematic"),
        table_kind_label(&locale, kind),
        subcolumn_short_tag(subcol),
    );
    paint_text_fitted(
        builder,
        &schem_id,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: hdr_r,
            pad_x: 2,
            text: &hdr_text,
            style: &UiTextStyle::ui_bold((10.5 * scale).clamp(8.5, 18.0)),
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    let body_top = hdr_r.bottom() + s_i32(4, scale);
    let body_h = u32::try_from((inner.bottom() - body_top).max(0)).unwrap_or(0);
    if body_h < 32 {
        return;
    }
    let body_r = LayoutRectPx::new(inner.x, body_top, inner.width, body_h);
    let gap = s_u32(10, scale, 6);
    let table_w = body_r.width * 52 / 100;
    let plot_w = body_r.width.saturating_sub(table_w + gap);

    let table_rect = place_rtl(body_r, 0, body_r.y, table_w, body_r.height, is_rtl);
    let plot_rect = place_rtl(body_r, i32::try_from(table_w + gap).unwrap_or(0), body_r.y, plot_w, body_r.height, is_rtl);

    draw_table_vector_schematic(builder, &schem_id, table_rect, kind, x_mode, subcol, col_design, data_sets, scale);
    draw_plot_vector_schematic(builder, &schem_id, plot_rect, kind, subcol, col_design);
}

const fn subcolumn_short_tag(mode: SubcolumnMode) -> &'static str {
    match mode {
        SubcolumnMode::Single => "Single Y",
        SubcolumnMode::RawReplicates(_) => "Replicates",
        SubcolumnMode::MeanSdN => "Mean/SD/N",
        SubcolumnMode::MeanSemN => "Mean/SEM/N",
        SubcolumnMode::MeanCvN => "Mean/%CV/N",
        SubcolumnMode::MeanLowHigh => "Mean/-Err/+Err",
        SubcolumnMode::UpperLowerLimits => "Mean/Low/Up",
        SubcolumnMode::MedianIqr => "Median/Q1/Q3",
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_table_vector_schematic(
    builder: &mut LayoutBuilder<'_>,
    owner: &WidgetId,
    rect: LayoutRectPx,
    kind: TableKind,
    x_mode: &XColumnMode,
    subcol: SubcolumnMode,
    col_design: ColumnDesignHint,
    data_sets: usize,
    scale: f64,
) {
    builder.paint_fill(ZLayer::Workspace, Some(owner), rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, Some(owner), rect, builder.palette.border, 1);

    let headers: Vec<&str> = match kind {
        TableKind::Xy => {
            let x_lbl = match x_mode {
                XColumnMode::Numbers { .. } => "X",
                XColumnMode::DateOrElapsedTime { unit_label } if unit_label.eq_ignore_ascii_case("Date") => "Date",
                XColumnMode::DateOrElapsedTime { .. } => "Time",
                XColumnMode::GeneratedSequence { .. } => "Seq X",
            };
            let mut h = vec![x_lbl];
            match subcol {
                SubcolumnMode::Single => {
                    h.push("Y A");
                    if data_sets > 1 {
                        h.push("Y B");
                    }
                }
                SubcolumnMode::RawReplicates(k) => {
                    h.push("Y1");
                    if k >= 2 {
                        h.push("Y2");
                    }
                    if k >= 3 {
                        h.push("Y3");
                    }
                }
                SubcolumnMode::MeanSdN => h.extend(["Mean", "SD", "N"]),
                SubcolumnMode::MeanSemN => h.extend(["Mean", "SEM", "N"]),
                SubcolumnMode::MeanCvN => h.extend(["Mean", "%CV", "N"]),
                SubcolumnMode::MeanLowHigh => h.extend(["Mean", "-Err", "+Err"]),
                SubcolumnMode::UpperLowerLimits => h.extend(["Mean", "Low", "Up"]),
                SubcolumnMode::MedianIqr => h.extend(["Med", "Q1", "Q3"]),
            }
            h
        }
        TableKind::Column => match subcol {
            SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN | SubcolumnMode::MeanCvN => vec!["Grp A", "M", "Err", "N"],
            _ if matches!(col_design, ColumnDesignHint::PairedRepeatedMeasures) => vec!["Subject", "Pre", "Post"],
            _ => vec!["A", "B", "C"],
        },
        TableKind::Grouped => match subcol {
            SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN | SubcolumnMode::MeanCvN => vec!["Factor", "M", "Err", "N"],
            _ => vec!["Factor", "A:Y1", "A:Y2", "B:Y1"],
        },
        TableKind::Contingency => vec!["Row", "Col 1", "Col 2"],
        TableKind::Survival => vec!["Time", "Ctrl 0/1", "Trt 0/1"],
        TableKind::PartsOfWhole => vec!["Category", "Share %"],
        TableKind::MultipleVariables => vec!["Group", "X1", "X2", "Calc"],
        TableKind::Nested => vec!["Grp A:1", "Grp A:2", "Grp B:1"],
    };

    let cols = headers.len().max(1);
    let col_w = (rect.width.saturating_sub(2)) / cols as u32;
    let hdr_h = s_u32(18, scale, 14).min(rect.height / 2);
    let hdr_bg = builder.palette.header;
    builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(rect.x + 1, rect.y + 1, rect.width.saturating_sub(2), hdr_h), hdr_bg);
    builder.paint_line(
        ZLayer::Workspace,
        Some(owner),
        [rect.x, rect.y + i32::try_from(hdr_h).unwrap_or(14)],
        [rect.right(), rect.y + i32::try_from(hdr_h).unwrap_or(14)],
        builder.palette.border,
        1,
    );

    let style = UiTextStyle::grid_tabular((9.5 * scale).clamp(8.0, 17.0));
    for (c, lbl) in headers.iter().enumerate() {
        let cx = rect.x + 1 + i32::try_from(c as u32 * col_w).unwrap_or(0);
        if c > 0 {
            builder.paint_line(ZLayer::Workspace, Some(owner), [cx, rect.y], [cx, rect.bottom()], builder.palette.border, 1);
        }
        let cell_hdr = LayoutRectPx::new(cx, rect.y + 1, col_w, hdr_h);
        if cell_hdr.width >= 20 {
            paint_text_fitted(
                builder,
                owner,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: cell_hdr,
                    pad_x: 3,
                    text: lbl,
                    style: &style,
                    color: builder.palette.text,
                    scroll_container: None,
                },
            );
        }
    }

    let body_top = rect.y + i32::try_from(hdr_h + 1).unwrap_or(16);
    let rem_h = u32::try_from((rect.bottom() - body_top - 1).max(0)).unwrap_or(0);
    let row_count = 3u32;
    let row_h = (rem_h / row_count).max(6);
    for r in 1..row_count {
        let ry = body_top + i32::try_from(r * row_h).unwrap_or(0);
        builder.paint_line(ZLayer::Workspace, Some(owner), [rect.x + 1, ry], [rect.right() - 1, ry], builder.palette.border, 1);
    }

    // Accent indicator bar inside column 0 / Y data cells to visualize structure
    let accent_h = (row_h / 2).max(4);
    for r in 0..row_count {
        let ry = body_top + i32::try_from(r * row_h + (row_h - accent_h) / 2).unwrap_or(2);
        let fill_w = col_w.saturating_sub(10);
        if fill_w >= 6 {
            builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(rect.x + 5, ry, fill_w / 2, accent_h), builder.palette.accent);
            if cols >= 2 {
                let c1x = rect.x + i32::try_from(col_w).unwrap_or(0) + 5;
                builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(c1x, ry, fill_w * 3 / 4, accent_h), builder.palette.muted);
            }
        }
    }
}

fn draw_plot_vector_schematic(
    builder: &mut LayoutBuilder<'_>,
    owner: &WidgetId,
    rect: LayoutRectPx,
    kind: TableKind,
    subcol: SubcolumnMode,
    col_design: ColumnDesignHint,
) {
    builder.paint_fill(ZLayer::Workspace, Some(owner), rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, Some(owner), rect, builder.palette.border, 1);
    let plot = rect.inset(8);
    if plot.width < 24 || plot.height < 24 {
        return;
    }

    let c1 = builder.palette.accent;
    let c2 = Rgba8::rgb(217, 119, 6);
    let axis = builder.palette.text;

    if !matches!(kind, TableKind::PartsOfWhole) {
        builder.paint_line(ZLayer::Workspace, Some(owner), [plot.x, plot.y], [plot.x, plot.bottom()], axis, 1);
        builder.paint_line(ZLayer::Workspace, Some(owner), [plot.x, plot.bottom()], [plot.right(), plot.bottom()], axis, 1);
    }

    match kind {
        TableKind::Xy => {
            let pts_x = [12, 34, 58, 82];
            let pts_y = [22, 36, 65, 82];
            let coords: Vec<[i32; 2]> = pts_x
                .iter()
                .zip(pts_y)
                .map(|(&px, py)| [plot.x + (px * plot.width as i32) / 100, plot.y + (py * plot.height as i32) / 100])
                .collect();
            for pair in coords.windows(2) {
                builder.paint_line(ZLayer::Workspace, Some(owner), pair[0], pair[1], c1, 2);
            }
            let show_err = !matches!(subcol, SubcolumnMode::Single);
            for &[cx, cy] in &coords {
                if show_err {
                    builder.paint_line(ZLayer::Workspace, Some(owner), [cx, cy - 7], [cx, cy + 7], axis, 1);
                }
                builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(cx - 3, cy - 3, 6, 6), c1);
            }
        }
        TableKind::Column => {
            if col_design == ColumnDesignHint::PairedRepeatedMeasures {
                for &(y0, y1) in &[(72, 42), (62, 30), (50, 22)] {
                    let p0 = [plot.x + plot.width as i32 * 25 / 100, plot.y + plot.height as i32 * y0 / 100];
                    let p1 = [plot.x + plot.width as i32 * 75 / 100, plot.y + plot.height as i32 * y1 / 100];
                    builder.paint_line(ZLayer::Workspace, Some(owner), p0, p1, c1, 1);
                    builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(p0[0] - 2, p0[1] - 2, 5, 5), axis);
                    builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(p1[0] - 2, p1[1] - 2, 5, 5), c2);
                }
            } else {
                for (i, pct) in [45, 72, 60].into_iter().enumerate() {
                    let bw = (plot.width / 5).max(8);
                    let bx = plot.x + 8 + i32::try_from(i as u32 * (bw + 8)).unwrap_or(0);
                    let bh = ((plot.height - 6) * pct / 100).max(6);
                    let by = plot.bottom() - i32::try_from(bh).unwrap_or(6);
                    let col = if i % 2 == 0 { c1 } else { c2 };
                    builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(bx, by, bw, bh), col);
                    let mx = bx + i32::try_from(bw / 2).unwrap_or(4);
                    builder.paint_line(ZLayer::Workspace, Some(owner), [mx, by - 5], [mx, by + 5], axis, 1);
                }
            }
        }
        TableKind::Grouped => {
            for cluster in 0..2 {
                for series in 0..2 {
                    let bw = (plot.width / 7).max(6);
                    let bx = plot.x + 8 + i32::try_from(cluster * (bw * 2 + 12) + series * (bw + 3)).unwrap_or(0);
                    let pct = if series == 0 { 42 + cluster * 20 } else { 65 - cluster * 12 };
                    let bh = ((plot.height - 6) * pct / 100).max(6);
                    let col = if series == 0 { c1 } else { c2 };
                    builder.paint_fill(
                        ZLayer::Workspace,
                        Some(owner),
                        LayoutRectPx::new(bx, plot.bottom() - i32::try_from(bh).unwrap_or(6), bw, bh),
                        col,
                    );
                }
            }
        }
        TableKind::Contingency => {
            for (i, (p1, _p2)) in [(68, 32), (38, 62)].into_iter().enumerate() {
                let bw = (plot.width / 4).max(10);
                let bx = plot.x + 12 + i32::try_from(i as u32 * (bw + 12)).unwrap_or(0);
                let tot = plot.height.saturating_sub(6);
                let h1 = (tot * p1 / 100).max(4);
                let h2 = tot.saturating_sub(h1).max(4);
                builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(bx, plot.bottom() - i32::try_from(h1).unwrap_or(4), bw, h1), c1);
                builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(bx, plot.y + 4, bw, h2), c2);
            }
        }
        TableKind::Survival => {
            let mut cur_x = plot.x + 2;
            let mut cur_y = plot.y + 4;
            for (dx, dy) in [(24, 14), (24, 18), (24, 14)] {
                let nx = (cur_x + (dx * plot.width as i32) / 100).min(plot.right() - 2);
                let ny = (cur_y + (dy * plot.height as i32) / 100).min(plot.bottom() - 4);
                builder.paint_line(ZLayer::Workspace, Some(owner), [cur_x, cur_y], [nx, cur_y], c1, 2);
                builder.paint_line(ZLayer::Workspace, Some(owner), [nx, cur_y], [nx, ny], c1, 2);
                builder.paint_line(ZLayer::Workspace, Some(owner), [(cur_x + nx) / 2, cur_y - 3], [(cur_x + nx) / 2, cur_y + 3], c2, 1);
                cur_x = nx;
                cur_y = ny;
            }
        }
        TableKind::PartsOfWhole => {
            let bar_h = (plot.height / 2).clamp(14, 28);
            let bar_y = plot.y + i32::try_from((plot.height.saturating_sub(bar_h)) / 2).unwrap_or(0);
            let w1 = plot.width * 52 / 100;
            let w2 = plot.width * 28 / 100;
            let w3 = plot.width.saturating_sub(w1 + w2);
            let c3 = builder.palette.muted;
            builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(plot.x, bar_y, w1, bar_h), c1);
            builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(plot.x + i32::try_from(w1).unwrap_or(0), bar_y, w2, bar_h), c2);
            builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(plot.x + i32::try_from(w1 + w2).unwrap_or(0), bar_y, w3, bar_h), c3);
            builder.paint_stroke(ZLayer::Workspace, Some(owner), LayoutRectPx::new(plot.x, bar_y, plot.width, bar_h), axis, 1);
        }
        TableKind::MultipleVariables => {
            for (px, py, sz, col) in [(18, 70, 5, c1), (36, 48, 8, c2), (56, 36, 6, c1), (78, 22, 9, c2)] {
                let cx = plot.x + (px * plot.width as i32) / 100;
                let cy = plot.y + (py * plot.height as i32) / 100;
                builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(cx - sz / 2, cy - sz / 2, sz as u32, sz as u32), col);
            }
        }
        TableKind::Nested => {
            for (g, bx_pct) in [(0, 22), (1, 66)] {
                let gx = plot.x + (bx_pct * plot.width as i32) / 100;
                let col = if g == 0 { c1 } else { c2 };
                for sub in 0..3 {
                    let sx = gx + (sub - 1) * 8;
                    let sy = plot.bottom() - i32::try_from(plot.height * (35 + g * 25 + sub as u32 * 8) / 100).unwrap_or(10);
                    builder.paint_line(ZLayer::Workspace, Some(owner), [sx, plot.bottom() - 2], [sx, sy], col, 2);
                    builder.paint_fill(ZLayer::Workspace, Some(owner), LayoutRectPx::new(sx - 2, sy - 2, 4, 4), axis);
                }
            }
        }
    }
}
