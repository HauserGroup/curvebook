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
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::control_io::encode_base64;
use crate::interaction::UiEventOutcome;
use crate::workspace_nav::{SAMPLE_FIXTURES, TableCreationDraft, WelcomeMode, instantiate_sample_fixture};

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
        subcolumn_mode: SubcolumnMode::RawReplicates(3),
        replicates: 3,
        column_design: ColumnDesignHint::UnpairedReplicates,
        graph_kind: GraphKind::Xy,
    },
    TemplatePreset {
        virtual_path: "template://column-anova.cbtemplate",
        title: "Three-Arm Column Comparison Template",
        kind: TableKind::Column,
        subcolumn_mode: SubcolumnMode::Single,
        replicates: 1,
        column_design: ColumnDesignHint::UnpairedReplicates,
        graph_kind: GraphKind::Column,
    },
    TemplatePreset {
        virtual_path: "template://grouped-kinetics.cbtemplate",
        title: "Two-Factor Time-Course Template",
        kind: TableKind::Grouped,
        subcolumn_mode: SubcolumnMode::RawReplicates(3),
        replicates: 3,
        column_design: ColumnDesignHint::UnpairedReplicates,
        graph_kind: GraphKind::Grouped,
    },
    TemplatePreset {
        virtual_path: "template://survival-cohort.cbtemplate",
        title: "Kaplan-Meier Survival Cohort Template",
        kind: TableKind::Survival,
        subcolumn_mode: SubcolumnMode::Single,
        replicates: 1,
        column_design: ColumnDesignHint::UnpairedReplicates,
        graph_kind: GraphKind::Survival,
    },
];

// ============================================================================
// Localization & Geometry Helpers
// ============================================================================

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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

fn run_sample_creation_tx<F>(app: &mut UiApp, f: F) -> Result<UiEventOutcome, String>
where
    F: FnOnce(&mut UiApp) -> Result<Value, String>,
{
    let prev_session = app.session.clone();
    let prev_batch_undo = app.batch_undo_log.clone();
    let prev_shell = app.snapshot_ui_shell();

    match f(app) {
        Ok(payload) => Ok(UiEventOutcome {
            handled: true,
            action: String::from("welcome.create_sample"),
            command_id: Some(String::from("sheet.createData")),
            command_result: Some(payload),
        }),
        Err(err) => {
            app.session = prev_session;
            app.batch_undo_log = prev_batch_undo;
            app.restore_ui_shell(prev_shell);
            app.sync_views();
            Err(err)
        }
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

    run_sample_creation_tx(app, |app| {
        let create_out = app.execute_command("sheet.createData", create_params).map_err(|e| e.to_string())?;
        populate_sample_x_values(app, sid, &fixture.archetype)?;
        populate_sample_titles(app, sid, &fixture.archetype)?;
        populate_sample_cells(app, sid, &fixture.archetype)?;
        populate_sample_mv_roles_and_formulas(app, sid, &fixture.archetype)?;
        Ok(create_out.payload)
    })
}

fn build_sample_create_params(arch: &TableArchetype, sid: &str, title: &str, auto_graph: bool, auto_info: bool) -> Value {
    let mut base = json!({
        "sheet_id": sid,
        "title": title,
        "kind": arch.kind(),
        "cols": arch.column_count(),
        "rows": arch.row_count(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fa_sec15_11_sample_tx_rolls_back_session_and_shell_on_midway_failure() {
        let mut app = UiApp::new();
        let initial_data_len = app.document().data_sheets.len();
        let initial_graph_len = app.document().graph_sheets.len();
        let initial_undo = app.session.undo_depth();

        let err = run_sample_creation_tx(&mut app, |app| {
            app.execute_command(
                "sheet.createData",
                json!({ "sheet_id": "partial-sample", "kind": "column", "cols": 2, "rows": 4, "auto_graph": true, "auto_info": true }),
            )
            .map_err(|e| e.to_string())?;
            assert!(app.document().data_sheet(&SheetId::from("partial-sample")).is_some());
            Err(String::from("simulated populate_sample_cells failure"))
        })
        .expect_err("mid-creation failure must return Err");

        assert!(err.contains("simulated populate_sample_cells failure"));
        assert!(app.document().data_sheet(&SheetId::from("partial-sample")).is_none(), "partial DataSheet must be rolled back");
        assert_eq!(app.document().data_sheets.len(), initial_data_len);
        assert_eq!(app.document().graph_sheets.len(), initial_graph_len);
        assert_eq!(app.session.undo_depth(), initial_undo);
    }
}
