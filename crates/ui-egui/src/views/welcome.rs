//! Welcome & New Data Table Launcher shard state and sample/table creation dispatchers (`R10.4`).

use curvebook_doc::{Cell, CellValue, ProjectDocument, SheetId, TableArchetype, TableKind, VariableRole, VariableSource, XColumnMode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::interaction::UiEventOutcome;
use crate::workspace_nav::{SAMPLE_FIXTURES, WelcomeMode, instantiate_sample_fixture};

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

/// Dispatch custom `WidgetAction::Custom` events targeting the `welcome` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut WelcomeViewState, payload: &str) -> Result<UiEventOutcome, String> {
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

    match payload {
        "action:open_file" => {
            state.last_open_file_request = state.last_open_file_request.saturating_add(1);
            bump_state(state, Some(String::from("Open Existing File requested via host file picker")));
            Ok(handled_outcome(String::from("welcome.open_file_requested")))
        }
        "action:create" => execute_create_table_or_sample(app, state),
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
    let prev_shell = app.snapshot_ui_shell();
    match app.execute_atomic_group(f) {
        Ok(payload) => Ok(UiEventOutcome {
            handled: true,
            action: String::from("welcome.create_sample"),
            command_id: Some(String::from("sheet.createData")),
            command_result: Some(payload),
        }),
        Err(err) => {
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
