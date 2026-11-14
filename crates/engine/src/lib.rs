//! Core session, undo/redo history, reactive `Data -> Analysis -> Graph -> Layout` recalculation
//! pipeline, command registry and panic recovery boundary for LumenCraft.
//!
//! Layer L5 crate: shared identically by the desktop app, browser WASM build, headless CLI and MCP
//! server. Nothing in or below this crate depends on a UI toolkit.
//! Governing spec: `docs/architecture.md`, `docs/data-tables-and-transforms.md` (§6), and `docs/development.md`.
#![forbid(unsafe_code)]

pub mod catalog;
pub mod cmd_data;
pub mod cmd_sci;
pub mod dag;
pub mod guard;

use std::fmt;
use std::path::{Path, PathBuf};

use lumencraft_chart::{ChartError, ChartScene};
use lumencraft_doc::{DocError, ProjectDocument, SheetId};
use lumencraft_fit::FitError;
#[cfg(not(target_arch = "wasm32"))]
use lumencraft_format::DirectoryJournalBackend;
use lumencraft_format::FormatError;
use lumencraft_format::recovery::{CrashRecoveryJournal, JournalBackend, RecoveryReport};
use lumencraft_pdf::PdfError;
use lumencraft_render::RenderError;
use lumencraft_stats::StatsError;
use lumencraft_svg::SvgError;
use lumencraft_transform::TransformError;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Structured error returned by engine commands and file I/O operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineError {
    message: String,
}

impl EngineError {
    /// Construct an engine error with a user-facing diagnostic message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }

    /// Diagnostic message text.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EngineError {}

impl From<DocError> for EngineError {
    fn from(err: DocError) -> Self {
        Self::new(err.to_string())
    }
}

impl From<FormatError> for EngineError {
    fn from(err: FormatError) -> Self {
        Self::new(err.to_string())
    }
}

impl From<TransformError> for EngineError {
    fn from(err: TransformError) -> Self {
        Self::new(err.to_string())
    }
}

impl From<StatsError> for EngineError {
    fn from(err: StatsError) -> Self {
        Self::new(err.to_string())
    }
}

impl From<FitError> for EngineError {
    fn from(err: FitError) -> Self {
        Self::new(err.to_string())
    }
}

impl From<ChartError> for EngineError {
    fn from(err: ChartError) -> Self {
        Self::new(err.to_string())
    }
}

impl From<SvgError> for EngineError {
    fn from(err: SvgError) -> Self {
        Self::new(err.to_string())
    }
}

impl From<PdfError> for EngineError {
    fn from(err: PdfError) -> Self {
        Self::new(err.to_string())
    }
}

impl From<RenderError> for EngineError {
    fn from(err: RenderError) -> Self {
        Self::new(err.to_string())
    }
}

/// Metadata describing an engine command exposed to menus, shortcuts, CLI and MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub menu_path: &'static str,
    pub shortcut: &'static str,
    pub params_doc: &'static str,
}

/// Structured result emitted after executing a command on [`Session`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandOutcome {
    pub command_id: String,
    pub mutated: bool,
    pub message: String,
    pub payload: serde_json::Value,
}

impl CommandOutcome {
    /// Construct a mutating command outcome.
    #[must_use]
    pub fn mutated(command_id: impl Into<String>, message: impl Into<String>, payload: serde_json::Value) -> Self {
        Self { command_id: command_id.into(), mutated: true, message: message.into(), payload }
    }

    /// Construct a read-only command outcome (leaves undo stack and dirty state untouched).
    #[must_use]
    pub fn read_only(command_id: impl Into<String>, message: impl Into<String>, payload: serde_json::Value) -> Self {
        Self { command_id: command_id.into(), mutated: false, message: message.into(), payload }
    }
}

/// Stateful project session owning undo/redo history, dirty tracking, and guarded command execution.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    doc: ProjectDocument,
    file_path: Option<PathBuf>,
    dirty: bool,
    undo_stack: Vec<ProjectDocument>,
    redo_stack: Vec<ProjectDocument>,
}

/// Optional crash-recovery WAL journal hooks backed by `lumencraft_format::recovery`.
impl Session {
    /// Open or crash-recover a [`Session`] from `backend`, sanitizing any circular hooks and
    /// replaying committed journal commands on top of the latest snapshot.
    pub fn open_with_recovery<B: JournalBackend>(
        backend: B,
        owner_id: impl Into<String>,
        ttl_ms: u64,
        now_epoch_ms: u64,
    ) -> Result<(Self, CrashRecoveryJournal<B>, RecoveryReport), EngineError> {
        let (journal, report) = CrashRecoveryJournal::open_or_recover(backend, owner_id, ttl_ms, now_epoch_ms)?;
        let doc = match &report.snapshot {
            Some(bytes) => lumencraft_format::open_project_bytes(bytes)?,
            None => ProjectDocument::default(),
        };
        let mut session = Self::from_document(doc);
        for cmd in &report.recovered_commands {
            let command_id = cmd
                .get("command_id")
                .or_else(|| cmd.get("id"))
                .or_else(|| cmd.get("command"))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| EngineError::new("recovered journal entry missing `command_id` string"))?;
            let params = cmd.get("params").cloned().unwrap_or_else(|| json!({}));
            let _ = session.execute(command_id, params)?;
        }
        Ok((session, journal, report))
    }

    /// Native-only helper opening a [`Session`] backed by [`DirectoryJournalBackend`] in `dir`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open_dir_with_recovery(
        dir: impl AsRef<Path>,
        owner_id: impl Into<String>,
        ttl_ms: u64,
        now_epoch_ms: u64,
    ) -> Result<(Self, CrashRecoveryJournal<DirectoryJournalBackend>, RecoveryReport), EngineError> {
        let backend = DirectoryJournalBackend::open_dir(dir)?;
        Self::open_with_recovery(backend, owner_id, ttl_ms, now_epoch_ms)
    }

    /// Record a command invocation `{ command_id, params }` into the WAL `journal`, returning its sequence number.
    pub fn record_to_journal<B: JournalBackend>(
        &self,
        journal: &mut CrashRecoveryJournal<B>,
        command_id: &str,
        params: &serde_json::Value,
    ) -> Result<u64, EngineError> {
        let entry = json!({
            "command_id": command_id,
            "params": params,
        });
        Ok(journal.record_command(entry)?)
    }

    /// Execute `command_id` transactionally and append to `journal` only when the command succeeds and mutates state.
    pub fn execute_and_record<B: JournalBackend>(
        &mut self,
        journal: &mut CrashRecoveryJournal<B>,
        command_id: &str,
        params: serde_json::Value,
    ) -> Result<CommandOutcome, EngineError> {
        let outcome = self.execute(command_id, params.clone())?;
        if outcome.mutated {
            let _ = self.record_to_journal(journal, command_id, &params)?;
        }
        Ok(outcome)
    }

    /// Persist a full `.lcproj` snapshot of the current document into `journal` and prune replayed WAL entries.
    pub fn checkpoint_journal<B: JournalBackend>(&self, journal: &mut CrashRecoveryJournal<B>) -> Result<(), EngineError> {
        let snapshot_bytes = lumencraft_format::save_project_bytes(&self.doc)?;
        journal.checkpoint(&snapshot_bytes)?;
        Ok(())
    }

    /// Flush a clean final snapshot, remove the WAL log and lease lockfile, and return the underlying backend.
    pub fn close_journal_cleanly<B: JournalBackend>(&mut self, journal: CrashRecoveryJournal<B>) -> Result<B, EngineError> {
        let final_snapshot_bytes = lumencraft_format::save_project_bytes(&self.doc)?;
        let backend = journal.close_cleanly(&final_snapshot_bytes)?;
        self.dirty = false;
        Ok(backend)
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Create a fresh empty project session.
    #[must_use]
    pub fn new() -> Self {
        Self::from_document(ProjectDocument::default())
    }

    /// Wrap an existing [`ProjectDocument`] in a clean session.
    #[must_use]
    pub fn from_document(mut document: ProjectDocument) -> Self {
        let _ = lumencraft_format::sanitize_circular_hooks(&mut document);
        Self { doc: document, file_path: None, dirty: false, undo_stack: Vec::new(), redo_stack: Vec::new() }
    }

    /// Borrow the active [`ProjectDocument`].
    #[must_use]
    pub fn document(&self) -> &ProjectDocument {
        &self.doc
    }

    /// Borrow the associated archive file path, if saved or opened from disk.
    #[must_use]
    pub fn file_path(&self) -> Option<&Path> {
        self.file_path.as_deref()
    }

    /// Associate or clear the archive file path and reset dirty tracking.
    pub fn set_file_path(&mut self, path: Option<PathBuf>, mark_clean: bool) {
        self.file_path = path;
        if mark_clean {
            self.dirty = false;
        }
    }

    /// Whether the project has unsaved edits since creation or the last clean save/load.
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Execute an engine command with panic recovery and transactional undo recording.
    ///
    /// # Errors
    /// Returns [`EngineError`] if the command identifier is unknown, parameters are invalid, or a
    /// guarded panic occurs; document state rolls back atomically on failure.
    pub fn execute(&mut self, command_id: &str, params: serde_json::Value) -> Result<CommandOutcome, EngineError> {
        match command_id {
            "edit.undo" => {
                let applied = self.undo()?;
                return Ok(CommandOutcome {
                    command_id: String::from(command_id),
                    mutated: applied,
                    message: if applied { String::from("undid last edit") } else { String::from("nothing to undo") },
                    payload: json!({ "applied": applied }),
                });
            }
            "edit.redo" => {
                let applied = self.redo()?;
                return Ok(CommandOutcome {
                    command_id: String::from(command_id),
                    mutated: applied,
                    message: if applied { String::from("redid edit") } else { String::from("nothing to redo") },
                    payload: json!({ "applied": applied }),
                });
            }
            _ => {}
        }

        let before = self.doc.clone();
        let outcome = guard::run_guarded(command_id, || {
            if let Some(res) = cmd_data::dispatch_data_command(&mut self.doc, command_id, &params)? {
                return Ok(res);
            }
            if let Some(res) = cmd_sci::dispatch_sci_command(&mut self.doc, command_id, &params)? {
                return Ok(res);
            }
            Err(EngineError::new(format!("unknown engine command `{command_id}`")))
        });

        match outcome {
            Ok(res) => {
                if res.mutated {
                    self.undo_stack.push(before);
                    self.redo_stack.clear();
                    self.dirty = true;
                }
                Ok(res)
            }
            Err(err) => {
                self.doc = before;
                Err(err)
            }
        }
    }

    /// Restore the previous document snapshot onto the active document.
    pub fn undo(&mut self) -> Result<bool, EngineError> {
        let Some(prev) = self.undo_stack.pop() else {
            return Ok(false);
        };
        let current = std::mem::replace(&mut self.doc, prev);
        self.redo_stack.push(current);
        self.dirty = true;
        Ok(true)
    }

    /// Re-apply the most recently undone document snapshot.
    pub fn redo(&mut self) -> Result<bool, EngineError> {
        let Some(next) = self.redo_stack.pop() else {
            return Ok(false);
        };
        let current = std::mem::replace(&mut self.doc, next);
        self.undo_stack.push(current);
        self.dirty = true;
        Ok(true)
    }

    /// Re-evaluate all upstream-to-downstream dependencies in topological order.
    pub fn recalculate_all(&mut self) -> Result<(), EngineError> {
        guard::run_guarded("project.recalculate", || {
            let _ = dag::recalculate_project_sanitized(&mut self.doc)?;
            Ok(())
        })
    }

    /// Compile a graph or layout sheet into a resolution-independent [`ChartScene`].
    pub fn render_sheet_scene(&self, sheet_id: &SheetId) -> Result<ChartScene, EngineError> {
        cmd_sci::compile_sheet_scene(&self.doc, sheet_id)
    }
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::SheetId;
    use serde_json::json;

    use super::{EngineError, Session, catalog};

    #[test]
    fn engine_error_formats_message() {
        let err = EngineError::new("table not found");
        assert_eq!(err.to_string(), "table not found");
    }

    #[test]
    fn session_execute_undo_redo_and_render_smoke() {
        assert!(catalog::find_command("sheet.createData").is_some());

        let mut session = Session::new();
        assert!(!session.is_dirty());

        let outcome =
            session.execute("sheet.createData", json!({ "sheet_id": "data-1", "title": "Dose", "graph_sheet_id": "graph-1" })).expect("create data");
        assert!(outcome.mutated);
        assert!(session.is_dirty());
        assert!(session.document().data_sheet(&SheetId::new("data-1")).is_some());

        session.execute("table.setCell", json!({ "sheet_id": "data-1", "row": 0, "col": 0, "value": 42.5 })).expect("set cell");
        let scene = session.render_sheet_scene(&SheetId::new("graph-1")).expect("render scene");
        assert!(scene.width_pt > 0.0);

        assert!(session.undo().expect("undo cell"));
        assert!(session.redo().expect("redo cell"));
        session.recalculate_all().expect("recalculate");
    }
}
