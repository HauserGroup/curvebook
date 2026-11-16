//! Info-sheet note and shared constant commands (`info.setNote`, `info.setConstant`, `info.removeConstant`).

use lumencraft_doc::{ConstantValue, ErrorCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cmd::helpers::{require_info_sheet_mut, resolve_target_info_sheet_id};
use crate::cmd::params::{any_json_schema, bool_schema, non_empty_str_schema, nullable_schema, obj_schema_with_aliases, parse_params, str_schema};
use crate::cmd::project::recalculate_dirty_with_links;
use crate::cmd::{CommandGroup, CommandSpec, Ctx, Effects};
use crate::{CommandOutcome, EngineError, Session};

fn always_enabled(_session: &Session) -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct SetNoteParams {
    #[serde(alias = "sheetId", alias = "id")]
    sheet_id: String,
    #[serde(alias = "note", alias = "notes", alias = "notes_markdown", alias = "notesMarkdown", alias = "markdown")]
    text: String,
    append: Option<bool>,
}

fn set_note_schema() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target InfoSheet ID or paired DataSheet ID")),
            ("text", str_schema("Markdown note content to set or append")),
            ("append", nullable_schema(bool_schema("When true, append to existing notes instead of overwriting"))),
        ],
        &["sheet_id", "text"],
        vec![("sheet_id", &["sheetId", "id"]), ("text", &["note", "notes", "notes_markdown", "notesMarkdown", "markdown"])],
        true,
    )
}

fn run_info_set_note(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: SetNoteParams = parse_params(&set_note_schema(), params)?;
    let info_id = resolve_target_info_sheet_id(&ctx.session.doc, &parsed.sheet_id)?;
    let info = require_info_sheet_mut(&mut ctx.session.doc, &info_id)?;
    if parsed.append == Some(true) {
        if !info.notes_markdown.is_empty() && !parsed.text.is_empty() {
            info.notes_markdown.push('\n');
        }
        info.notes_markdown.push_str(&parsed.text);
    } else {
        info.notes_markdown = parsed.text;
    }

    Ok(CommandOutcome::mutated(
        "info.setNote",
        "updated info note",
        json!({
            "sheet_id": info_id.as_str(),
            "notes_len": info.notes_markdown.len(),
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct SetConstantParams {
    #[serde(alias = "sheetId", alias = "id")]
    sheet_id: String,
    #[serde(alias = "key")]
    name: String,
    value: Value,
    units: Option<String>,
    kind: Option<String>,
}

fn set_constant_schema() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target InfoSheet ID or paired DataSheet ID")),
            ("name", non_empty_str_schema("Hookable constant name")),
            ("value", any_json_schema("Constant value (finite number, string, or null to clear)")),
            ("units", nullable_schema(str_schema("Optional physical unit label"))),
            ("kind", nullable_schema(str_schema("Optional value kind hint (`number`, `text`, or `date`)"))),
        ],
        &["sheet_id", "name", "value"],
        vec![("sheet_id", &["sheetId", "id"]), ("name", &["key"])],
        true,
    )
}

fn parse_constant_value(value: Value, kind: Option<&str>) -> Result<ConstantValue, EngineError> {
    let normalized_kind = kind.map(|k| k.trim().to_ascii_lowercase());
    if let Some(ref k) = normalized_kind
        && !matches!(k.as_str(), "number" | "text" | "date" | "empty")
    {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported constant kind `{k}`"))
            .with_remediation("Use `number`, `text`, `date`, or omit `kind`."));
    }

    match value {
        Value::Null => Ok(ConstantValue::Empty),
        Value::Number(n) => {
            let f = n
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "info constant numeric value must be finite"))?;
            Ok(ConstantValue::Number(f))
        }
        Value::String(s) => match normalized_kind.as_deref() {
            Some("date") => Ok(ConstantValue::Date(s)),
            Some("number") => {
                let parsed: f64 = s
                    .trim()
                    .parse()
                    .ok()
                    .filter(|v: &f64| v.is_finite())
                    .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("constant string `{s}` is not a finite number")))?;
                Ok(ConstantValue::Number(parsed))
            }
            _ => Ok(ConstantValue::Text(s)),
        },
        _ => Err(EngineError::with_code(ErrorCode::BadInput, "info constant `value` must be a finite number, string, or null")
            .with_remediation("Pass a number, string, or null.")),
    }
}

fn run_info_set_constant(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: SetConstantParams = parse_params(&set_constant_schema(), params)?;
    let info_id = resolve_target_info_sheet_id(&ctx.session.doc, &parsed.sheet_id)?;
    let name = parsed.name.trim().to_owned();
    if name.is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "constant `name` must not be empty"));
    }
    let constant_value = parse_constant_value(parsed.value, parsed.kind.as_deref())?;
    let units = parsed.units.map(|u| u.trim().to_owned()).filter(|u| !u.is_empty());

    {
        let info = require_info_sheet_mut(&mut ctx.session.doc, &info_id)?;
        info.set_constant(name.clone(), constant_value, units)?;
    }

    let recalc = recalculate_dirty_with_links(&mut ctx.session.doc, std::slice::from_ref(&info_id), false)?;
    Ok(CommandOutcome::mutated(
        "info.setConstant",
        "updated info constant",
        json!({
            "sheet_id": info_id.as_str(),
            "name": name,
            "recalculated_sheets": recalc.recalculated_sheets,
            "refreshed_links": recalc.refreshed_links,
        }),
    ))
}

#[derive(Debug, Deserialize)]
struct RemoveConstantParams {
    #[serde(alias = "sheetId", alias = "id")]
    sheet_id: String,
    #[serde(alias = "key", alias = "constant")]
    name: String,
    #[serde(default, alias = "ifExists")]
    if_exists: Option<bool>,
}

fn remove_constant_schema() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target InfoSheet ID or paired DataSheet ID")),
            ("name", non_empty_str_schema("Constant name to remove")),
            ("if_exists", nullable_schema(bool_schema("Allow removing a non-existent constant without failing (default: false)"))),
        ],
        &["sheet_id", "name"],
        vec![("sheet_id", &["sheetId", "id"]), ("name", &["key", "constant"]), ("if_exists", &["ifExists"])],
        true,
    )
}

fn run_info_remove_constant(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let parsed: RemoveConstantParams = parse_params(&remove_constant_schema(), params)?;
    let info_id = resolve_target_info_sheet_id(&ctx.session.doc, &parsed.sheet_id)?;
    let name = parsed.name.trim().to_owned();
    if name.is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "constant `name` must not be empty"));
    }
    let if_exists = parsed.if_exists.unwrap_or(false);

    let removed = {
        let info = require_info_sheet_mut(&mut ctx.session.doc, &info_id)?;
        info.remove_constant(&name)
    };
    if !removed {
        if !if_exists {
            return Err(EngineError::with_code(ErrorCode::NotFound, format!("constant `{name}` not found in info sheet `{}`", info_id.as_str()))
                .with_remediation("Verify the constant name or pass `if_exists: true` to ignore missing constants."));
        }
        return Ok(CommandOutcome {
            command_id: String::from("info.removeConstant"),
            mutated: false,
            message: String::from("info constant was not present"),
            payload: json!({
                "sheet_id": info_id.as_str(),
                "name": name,
                "removed": false,
                "recalculated_sheets": 0,
            }),
        });
    }
    let recalc = recalculate_dirty_with_links(&mut ctx.session.doc, std::slice::from_ref(&info_id), false)?;

    Ok(CommandOutcome {
        command_id: String::from("info.removeConstant"),
        mutated: true,
        message: String::from("removed info constant"),
        payload: json!({
            "sheet_id": info_id.as_str(),
            "name": name,
            "removed": true,
            "recalculated_sheets": recalc.recalculated_sheets,
        }),
    })
}

static SPECS: [CommandSpec; 3] = [
    CommandSpec {
        id: "info.setNote",
        label: "Set Info Note",
        menu_path: "Info/Edit Note",
        shortcut: "",
        summary: "Set or append markdown notes on an InfoSheet.",
        beginner: "Updates experiment notes on an InfoSheet (or the InfoSheet linked to a given DataSheet ID).",
        group: CommandGroup::Info,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: set_note_schema,
        enabled: always_enabled,
        run: run_info_set_note,
    },
    CommandSpec {
        id: "info.setConstant",
        label: "Set Info Constant",
        menu_path: "Info/Set Constant",
        shortcut: "",
        summary: "Create or update a named InfoSheet constant and recalculate dependents.",
        beginner: "Sets a hookable constant (number, date, or text with optional units) used by formulas, fits, and graph titles.",
        group: CommandGroup::Info,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: set_constant_schema,
        enabled: always_enabled,
        run: run_info_set_constant,
    },
    CommandSpec {
        id: "info.removeConstant",
        label: "Remove Info Constant",
        menu_path: "Info/Remove Constant",
        shortcut: "",
        summary: "Remove a named constant from an InfoSheet and recalculate dependents.",
        beginner: "Deletes a constant from an InfoSheet and re-evaluates any sheets that depended on it.",
        group: CommandGroup::Info,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: remove_constant_schema,
        enabled: always_enabled,
        run: run_info_remove_constant,
    },
];

/// Registered command specifications for the `info` group.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    &SPECS
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use lumencraft_doc::{DataSheet, InfoSheet, ProjectDocument, SheetId, TableArchetype, XyTable};
    use serde_json::json;

    use crate::cmd::Caller;

    use super::*;

    fn find_spec(id: &str) -> &'static CommandSpec {
        specs().iter().find(|s| s.id == id).unwrap()
    }

    fn new_session(title: &str) -> Session {
        Session::from_document(ProjectDocument::new(title).unwrap())
    }

    fn exec_cmd(session: &mut Session, id: &str, caller: Caller, params: Value) -> Result<CommandOutcome, EngineError> {
        let cancel = AtomicBool::new(false);
        session.execute_registered_spec(find_spec(id), params, caller, &cancel)
    }

    #[test]
    fn info_note_and_constant_resolve_via_data_sheet_or_info_sheet() {
        let mut session = new_session("Info Test");
        let info = InfoSheet::new_preseeded("info-1", "Assay Info").unwrap();
        session.doc.add_info_sheet(info).unwrap();
        let xy = XyTable::new(lumencraft_doc::SubcolumnMode::Single, 1, 2).unwrap();
        let mut ds = DataSheet::new("data-1", "Dose", TableArchetype::Xy(xy)).unwrap();
        ds.info_sheet_id = Some(SheetId::new("info-1"));
        session.doc.add_data_sheet(ds).unwrap();

        let note_out = exec_cmd(&mut session, "info.setNote", Caller::Ui, json!({ "sheet_id": "data-1", "note": "Line 1" })).unwrap();
        assert!(note_out.mutated);
        assert_eq!(note_out.payload["sheet_id"], json!("info-1"));

        exec_cmd(&mut session, "info.setNote", Caller::Ui, json!({ "sheet_id": "info-1", "text": "Line 2", "append": true })).unwrap();
        assert_eq!(session.document().info_sheet(&SheetId::new("info-1")).unwrap().notes_markdown, "Line 1\nLine 2");

        let const_out =
            exec_cmd(&mut session, "info.setConstant", Caller::Mcp, json!({ "sheet_id": "data-1", "name": "Kd", "value": 12.5, "units": "nM" }))
                .unwrap();
        assert!(const_out.mutated);
        assert_eq!(const_out.payload["sheet_id"], json!("info-1"));
        assert_eq!(session.document().info_sheet(&SheetId::new("info-1")).unwrap().get_constant("Kd").unwrap().units.as_deref(), Some("nM"));

        let date_out = exec_cmd(
            &mut session,
            "info.setConstant",
            Caller::Ui,
            json!({ "sheet_id": "info-1", "name": "RunDate", "value": "2026-11-14", "kind": "date" }),
        )
        .unwrap();
        assert!(date_out.mutated);
        assert_eq!(
            session.document().info_sheet(&SheetId::new("info-1")).unwrap().get_constant("RunDate").unwrap().value,
            ConstantValue::Date(String::from("2026-11-14"))
        );

        let rm_out = exec_cmd(&mut session, "info.removeConstant", Caller::Mcp, json!({ "sheet_id": "data-1", "name": "Kd" })).unwrap();
        assert!(rm_out.mutated);
        assert_eq!(rm_out.payload["removed"], json!(true));

        let missing_err = exec_cmd(&mut session, "info.removeConstant", Caller::Mcp, json!({ "sheet_id": "data-1", "name": "Kd" })).unwrap_err();
        assert_eq!(missing_err.code, ErrorCode::NotFound);

        let rm_missing =
            exec_cmd(&mut session, "info.removeConstant", Caller::Mcp, json!({ "sheet_id": "data-1", "name": "Kd", "if_exists": true })).unwrap();
        assert!(!rm_missing.mutated);
        assert_eq!(rm_missing.payload["removed"], json!(false));

        let bad_val =
            exec_cmd(&mut session, "info.setConstant", Caller::Mcp, json!({ "sheet_id": "info-1", "name": "Bad", "value": true })).unwrap_err();
        assert_eq!(bad_val.code, ErrorCode::BadInput);
    }
}
