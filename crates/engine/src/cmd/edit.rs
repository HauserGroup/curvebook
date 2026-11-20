//! Undo and redo editing commands (`edit.undo`, `edit.redo`).

use serde::Deserialize;
use serde_json::{Value, json};

use crate::cmd::params::{empty_object_schema, parse_params};
use crate::cmd::{CommandGroup, CommandSpec, Ctx, Effects};
use crate::{CommandOutcome, EngineError};

#[derive(Debug, Deserialize)]
struct EmptyEditParams {}

fn always_enabled(_session: &crate::Session) -> bool {
    true
}

fn run_edit_undo(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let _parsed: EmptyEditParams = parse_params(&empty_object_schema(), params)?;
    let applied = ctx.session.undo()?;
    let message = if applied { "undid last edit" } else { "nothing to undo" };
    Ok(CommandOutcome {
        command_id: String::from("edit.undo"),
        mutated: applied,
        message: String::from(message),
        payload: json!({ "applied": applied }),
        warnings: Vec::new(),
    })
}

fn run_edit_redo(ctx: &mut Ctx<'_>, params: Value) -> Result<CommandOutcome, EngineError> {
    let _parsed: EmptyEditParams = parse_params(&empty_object_schema(), params)?;
    let applied = ctx.session.redo()?;
    let message = if applied { "redid edit" } else { "nothing to redo" };
    Ok(CommandOutcome {
        command_id: String::from("edit.redo"),
        mutated: applied,
        message: String::from(message),
        payload: json!({ "applied": applied }),
        warnings: Vec::new(),
    })
}

static SPECS: [CommandSpec; 2] = [
    CommandSpec {
        id: "edit.undo",
        label: "Undo",
        menu_path: "Edit/Undo",
        shortcut: "Cmd+Z",
        summary: "Undo the most recent document-mutating command.",
        beginner: "Restores the project snapshot captured immediately before the last edit; succeeds with mutated=false when the undo stack is empty.",
        group: CommandGroup::Edit,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: empty_object_schema,
        enabled: always_enabled,
        run: run_edit_undo,
    },
    CommandSpec {
        id: "edit.redo",
        label: "Redo",
        menu_path: "Edit/Redo",
        shortcut: "Cmd+Shift+Z",
        summary: "Redo the most recently undone document edit.",
        beginner: "Re-applies the project snapshot reverted by the last undo command; succeeds with mutated=false when the redo stack is empty.",
        group: CommandGroup::Edit,
        effects: Effects::MUTATES_DOCUMENT,
        input_schema: empty_object_schema,
        enabled: always_enabled,
        run: run_edit_redo,
    },
];

/// Registered command specifications for the `edit` group.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    &SPECS
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use curvebook_doc::{ErrorCode, ProjectDocument};
    use serde_json::json;

    use crate::Session;
    use crate::cmd::Caller;

    use super::*;

    fn find_spec(id: &str) -> &'static CommandSpec {
        specs().iter().find(|spec| spec.id == id).unwrap()
    }

    fn new_session(title: &str) -> Session {
        Session::from_document(ProjectDocument::new(title).unwrap())
    }

    fn run_spec(session: &mut Session, spec: &CommandSpec, caller: Caller, params: Value) -> Result<CommandOutcome, EngineError> {
        let cancel = AtomicBool::new(false);
        session.execute_registered_spec(spec, params, caller, &cancel)
    }

    #[test]
    fn undo_and_redo_stay_enabled_and_noop_cleanly_on_empty_stacks() {
        let mut session = new_session("Empty Edit Stack");
        let undo_spec = find_spec("edit.undo");
        let redo_spec = find_spec("edit.redo");

        assert!((undo_spec.enabled)(&session));
        assert!((redo_spec.enabled)(&session));

        let undo_out = run_spec(&mut session, undo_spec, Caller::Mcp, json!({})).unwrap();
        assert!(!undo_out.mutated);
        assert_eq!(undo_out.payload["applied"], json!(false));

        let redo_out = run_spec(&mut session, redo_spec, Caller::Mcp, json!({})).unwrap();
        assert!(!redo_out.mutated);
        assert_eq!(redo_out.payload["applied"], json!(false));
    }

    #[test]
    fn undo_and_redo_round_trip_mutating_command_and_reject_bad_params() {
        let mut session = new_session("Initial Title");
        let new_spec = crate::cmd::project::specs().iter().find(|s| s.id == "project.new").unwrap();
        let undo_spec = find_spec("edit.undo");
        let redo_spec = find_spec("edit.redo");

        run_spec(&mut session, new_spec, Caller::Ui, json!({ "title": "Updated Title" })).unwrap();
        assert_eq!(session.document().title, "Updated Title");

        let undo_out = run_spec(&mut session, undo_spec, Caller::Ui, json!({})).unwrap();
        assert!(undo_out.mutated);
        assert_eq!(undo_out.payload["applied"], json!(true));
        assert_eq!(session.document().title, "Initial Title");

        let redo_out = run_spec(&mut session, redo_spec, Caller::Ui, json!({})).unwrap();
        assert!(redo_out.mutated);
        assert_eq!(redo_out.payload["applied"], json!(true));
        assert_eq!(session.document().title, "Updated Title");

        let bad_err = run_spec(&mut session, undo_spec, Caller::Mcp, json!(null)).unwrap_err();
        assert_eq!(bad_err.code, ErrorCode::BadInput);
        let extra_err = run_spec(&mut session, redo_spec, Caller::Mcp, json!({ "unexpected": 1 })).unwrap_err();
        assert_eq!(extra_err.code, ErrorCode::BadInput);
    }
}
