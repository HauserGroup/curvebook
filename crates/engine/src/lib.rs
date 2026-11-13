//! Core session, undo/redo history, reactive `Data -> Analysis -> Graph -> Layout` recalculation
//! pipeline, command registry and panic recovery boundary for LumenCraft.
//!
//! Layer L5 crate: shared identically by the desktop app, browser WASM build, headless CLI and MCP
//! server. Nothing in or below this crate depends on a UI toolkit.
//! Governing spec: `docs/architecture.md`, `docs/data-tables-and-transforms.md` (§6), and `docs/development.md`.
#![forbid(unsafe_code)]

use std::fmt;

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

/// Metadata describing an engine command exposed to menus, shortcuts, CLI and MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandDescriptor {
    pub id: &'static str,
    pub label: &'static str,
    pub menu_path: &'static str,
    pub shortcut: &'static str,
    pub params_doc: &'static str,
}

#[cfg(test)]
mod tests {
    use super::EngineError;

    #[test]
    fn engine_error_formats_message() {
        let err = EngineError::new("table not found");
        assert_eq!(err.to_string(), "table not found");
    }
}
