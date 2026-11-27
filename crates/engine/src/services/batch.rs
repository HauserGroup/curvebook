//! Atomic multi-step command batch execution with eviction-safe rollback.

use std::sync::atomic::{AtomicBool, Ordering};

use curvebook_doc::ErrorCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cmd::{Effects, registry};
use crate::{Caller, CommandOutcome, EngineError, Session};

/// Single command step inside an atomic batch execution request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchCommandStep {
    /// Registered engine command identifier (e.g. `"table.setCells"`).
    pub command_id: String,
    /// JSON parameter object for the command.
    pub params: Value,
}

impl BatchCommandStep {
    /// Construct a new batch step.
    #[must_use]
    pub fn new(command_id: impl Into<String>, params: Value) -> Self {
        Self { command_id: command_id.into(), params }
    }
}

/// Structured failure report emitted when [`Session::execute_batch_atomic`] rolls back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchStepFailure {
    /// Zero-based index of the failing step in the submitted batch.
    pub failed_index: usize,
    /// Identifier of the failing command.
    pub failed_command_id: String,
    /// Underlying structured engine error.
    pub error: EngineError,
    /// Outcomes of steps that succeeded prior to the failure (before rollback).
    pub completed_before_failure: Vec<CommandOutcome>,
    /// Indices and command IDs of steps that performed host filesystem writes before failure.
    pub persisted_filesystem_steps: Vec<(usize, String)>,
}

fn has_non_empty_dest_path_param(params: &Value) -> bool {
    let Some(obj) = params.as_object() else {
        return false;
    };
    ["path", "output_path", "outputPath", "file_path", "filePath"]
        .iter()
        .any(|key| obj.get(*key).and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty()))
}

fn is_optional_path_export_or_save_cmd(command_id: &str) -> bool {
    matches!(
        command_id,
        "project.save" | "project.saveTemplate" | "table.exportCsv" | "analysis.exportJson" | "analysis.exportCsv" | "result.generateReport"
    ) || command_id.starts_with("export.")
        || command_id.contains(".export")
}

/// Return `true` when invoking `command_id` with `params` will write to the host filesystem.
#[must_use]
pub fn command_step_writes_filesystem(command_id: &str, params: &Value) -> bool {
    if command_id == "ui.screenshot" {
        return has_non_empty_dest_path_param(params);
    }
    if command_id == "fit.cloneBuiltinEquation" {
        return params.get("save").and_then(Value::as_bool).unwrap_or(true);
    }
    let Some(spec) = registry().get(command_id) else {
        return false;
    };
    if !spec.effects.contains(Effects::WRITES_FILESYSTEM) {
        return false;
    }
    if is_optional_path_export_or_save_cmd(command_id) { has_non_empty_dest_path_param(params) } else { true }
}

fn outcome_has_written_path(outcome: &CommandOutcome) -> bool {
    outcome.payload.get("path").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty())
}

impl Session {
    /// Execute a slice of [`BatchCommandStep`]s atomically against `self`.
    ///
    /// Snapshots the entire session state (`doc`, `file_path`, `dirty`, `undo_stack`, `redo_stack`)
    /// before running any step so rollback on error or cancellation is exact even when the batch
    /// executes more mutating commands than [`crate::MAX_UNDO_ENTRIES`] or triggers undo-stack
    /// memory-budget eviction.
    ///
    /// On success, intermediate undo entries collapse into a single undo step via
    /// [`Self::collapse_atomic_group`]; purely read-only batches leave `dirty` and undo/redo stacks
    /// untouched.
    ///
    /// # Errors
    /// Returns [`BatchStepFailure`] describing the failing step, prior outcomes, and any non-revertible
    /// host filesystem writes after restoring `self` to its exact pre-batch state.
    pub fn execute_batch_atomic(
        &mut self,
        steps: &[BatchCommandStep],
        caller: Caller,
        cancelled: &AtomicBool,
    ) -> Result<Vec<CommandOutcome>, Box<BatchStepFailure>> {
        let baseline = self.clone();
        let mut outcomes = Vec::with_capacity(steps.len());
        let mut persisted_filesystem_steps: Vec<(usize, String)> = Vec::new();

        for (idx, step) in steps.iter().enumerate() {
            if cancelled.load(Ordering::Acquire) {
                *self = baseline;
                return Err(Box::new(BatchStepFailure {
                    failed_index: idx,
                    failed_command_id: step.command_id.clone(),
                    error: EngineError::with_code(ErrorCode::Cancelled, format!("command `{}` cancelled", step.command_id))
                        .with_retryable(true)
                        .with_details(json!({ "command_id": step.command_id })),
                    completed_before_failure: outcomes,
                    persisted_filesystem_steps,
                }));
            }
            let writes_fs_hint = command_step_writes_filesystem(&step.command_id, &step.params);
            match self.execute_with_caller(&step.command_id, step.params.clone(), caller, cancelled) {
                Ok(outcome) => {
                    if writes_fs_hint || outcome_has_written_path(&outcome) {
                        persisted_filesystem_steps.push((idx, step.command_id.clone()));
                    }
                    outcomes.push(outcome);
                }
                Err(error) => {
                    *self = baseline;
                    return Err(Box::new(BatchStepFailure {
                        failed_index: idx,
                        failed_command_id: step.command_id.clone(),
                        error,
                        completed_before_failure: outcomes,
                        persisted_filesystem_steps,
                    }));
                }
            }
        }

        let _ = self.collapse_atomic_group(baseline);
        Ok(outcomes)
    }
}
