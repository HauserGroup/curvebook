//! Thin desktop and browser UI shell for LumenCraft: project navigator, typed data/results
//! spreadsheet grid, statistical parameter dialogs, graph/layout canvas, menus, localization and
//! local JSON-lines control channel.
//!
//! Layer L6 front-end crate: panels inspect engine state and mutate sessions solely by dispatching
//! commands. Governing spec: `docs/ui-parity.md`, `docs/control-protocol.md`, and `docs/development.md`.
#![forbid(unsafe_code)]

pub mod control_io;
pub mod dialogs_canvas;
pub mod grid_view;
pub mod i18n;
pub mod menus;
pub mod theme;
pub mod workspace_nav;

use lumencraft_doc::{ProjectDocument, SheetSection};
use lumencraft_engine::{CommandOutcome, EngineError, Session};
use lumencraft_tools::{Cursor, GridVisibleWindow};

pub use control_io::{
    ClipboardBundle, ControlLoopAction, ControlRequest, ControlResponse, MAX_CONTROL_LINE_BYTES, OsAdapterAvailability, OsAdapterStatus,
    PrintJobSpec, PrintSpoolOutput, dispatch_ui_command, handle_control_line, handle_control_line_with_action, parse_control_port,
    serve_control_stream, serve_control_stream_with_quit,
};
#[cfg(not(target_arch = "wasm32"))]
pub use control_io::{
    bind_control_tcp_listener, read_os_pasteboard, run_control_tcp_server, serve_control_listener, submit_os_print_job, submit_os_print_pdf_bytes,
    validate_loopback_addr, write_os_pasteboard,
};
pub use dialogs_canvas::{ActiveDialog, CanvasDialogState};
pub use grid_view::GridViewState;
pub use i18n::{LOCALES, LocalePreset, locale_display_name, normalize_locale, tr};
pub use menus::{ShortcutBinding, ShortcutExecution, ShortcutMatch, ShortcutRegistry, normalize_shortcut_combo};
pub use theme::{UiThemeMode, UiThemeSpec};
pub use workspace_nav::{NavigatorEntry, WorkspaceNavState};

/// Whether the UI crate was compiled with bundled interface/document fonts enabled.
#[must_use]
pub const fn bundled_fonts_enabled() -> bool {
    cfg!(feature = "bundled-fonts")
}

/// Top-level application shell facade owning [`Session`] and panel/tool view state.
#[derive(Debug, Clone, PartialEq)]
pub struct UiApp {
    session: Session,
    pub theme: UiThemeSpec,
    pub nav: WorkspaceNavState,
    pub grid: GridViewState,
    pub canvas_dialogs: CanvasDialogState,
    pub shortcuts: ShortcutRegistry,
    pub family_open: bool,
    pub locale: String,
    pub locale_preset: i18n::LocalePreset,
}

impl Default for UiApp {
    fn default() -> Self {
        Self::new()
    }
}

impl UiApp {
    /// Construct a new UI application facade with an empty project session.
    #[must_use]
    pub fn new() -> Self {
        Self::from_session(Session::new())
    }

    /// Construct a UI application facade wrapping an existing [`Session`].
    #[must_use]
    pub fn from_session(session: Session) -> Self {
        let mut app = Self {
            session,
            theme: UiThemeSpec::default(),
            nav: WorkspaceNavState::default(),
            grid: GridViewState::default(),
            canvas_dialogs: CanvasDialogState::default(),
            shortcuts: ShortcutRegistry::new(),
            family_open: true,
            locale: String::from("en"),
            locale_preset: i18n::LocalePreset::default(),
        };
        app.sync_views();
        app
    }

    /// Update active UI locale and synchronize its regional numeric/CSV [`LocalePreset`].
    pub fn set_locale(&mut self, locale: &str) -> &str {
        let normalized = i18n::normalize_locale(locale);
        self.locale = normalized.to_owned();
        self.locale_preset = i18n::LocalePreset::for_locale(normalized);
        &self.locale
    }

    /// Update active UI theme mode (`Light`, `Dark`, or `HighContrast`).
    pub fn set_theme_mode(&mut self, mode: UiThemeMode) {
        self.theme = UiThemeSpec::for_mode(mode);
    }

    /// Look up the localized UI string for `key` under the currently selected locale.
    #[must_use]
    pub fn tr(&self, key: &str) -> &'static str {
        i18n::tr(&self.locale, key)
    }

    /// Borrow the underlying headless [`Session`].
    #[must_use]
    pub const fn session(&self) -> &Session {
        &self.session
    }

    /// Mutably borrow the underlying headless [`Session`].
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Borrow the active [`ProjectDocument`] stored in [`Session`].
    #[must_use]
    pub fn document(&self) -> &ProjectDocument {
        self.session.document()
    }

    /// Execute an engine command and refresh UI navigator and selection quick-statistics state.
    pub fn execute_command(&mut self, command_id: &str, params: serde_json::Value) -> Result<CommandOutcome, EngineError> {
        let outcome = self.session.execute(command_id, params)?;
        self.sync_views();
        Ok(outcome)
    }

    /// Process a single JSON-lines control-protocol request line and serialize the JSON reply.
    #[must_use]
    pub fn dispatch_control_line(&mut self, line: &str) -> String {
        handle_control_line(self, line)
    }

    /// Process a single JSON-lines control request and return `(json_reply, loop_action)`.
    #[must_use]
    pub fn dispatch_control_line_with_action(&mut self, line: &str) -> (String, ControlLoopAction) {
        handle_control_line_with_action(self, line)
    }

    /// Dispatch a keyboard shortcut combination (such as `"F6"`, `"Shift+F6"`, or a customized
    /// binding) through [`Self::shortcuts`].
    ///
    /// Returns `Ok(None)` when no command is bound to the normalized key combination.
    ///
    /// # Errors
    /// Returns a descriptive error string if executing the matched UI or engine command fails.
    pub fn handle_key_combo(&mut self, combo: &str) -> Result<Option<ShortcutExecution>, String> {
        let Some(matched) = self.shortcuts.handle_key_combo(combo) else {
            return Ok(None);
        };
        if let Some(ui_result) = control_io::dispatch_ui_command(self, &matched.command_id, &matched.params)? {
            return Ok(Some(ShortcutExecution { command_id: matched.command_id, ui: true, result: ui_result }));
        }
        let outcome = self.execute_command(&matched.command_id, matched.params).map_err(|err| err.to_string())?;
        let result = serde_json::to_value(outcome).map_err(|err| format!("failed to serialize command outcome: {err}"))?;
        Ok(Some(ShortcutExecution { command_id: matched.command_id, ui: false, result }))
    }

    /// Synchronize navigator entries, family pane links, and status-bar statistics from `Session`.
    pub fn sync_views(&mut self) {
        self.nav.sync_from_document(self.session.document());
        self.grid.refresh_selection_stats(self.session.document());
    }

    /// Visible row/column slice when the active sheet is a data table.
    #[must_use]
    pub fn visible_grid_window(&self) -> Option<GridVisibleWindow> {
        self.grid.visible_window(self.session.document())
    }

    /// Active logical cursor determined by the currently selected sheet section and tool mode.
    #[must_use]
    pub fn active_cursor(&self) -> Cursor {
        let doc = self.session.document();
        match doc.active_sheet_id.as_ref().and_then(|id| doc.sheet_section(id)) {
            Some(SheetSection::Data | SheetSection::Analyses) => self.grid.tool.cursor(),
            Some(SheetSection::Graphs | SheetSection::Layouts) => self.canvas_dialogs.canvas.cursor(),
            Some(SheetSection::Info) | None => Cursor::Select,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use lumencraft_doc::{CellCoord, SheetCellRange};
    use serde_json::{Value, json};

    use super::menus::ui_commands::UI_COMMANDS;
    use super::{ClipboardBundle, PrintJobSpec, UiApp, parse_control_port, serve_control_stream};

    #[test]
    fn ui_commands_table_covers_parity_surface() {
        assert_eq!(UI_COMMANDS.len(), 17);
        for required in [
            "ui.navigator.toggle",
            "ui.inspector.toggle",
            "ui.family.toggle",
            "ui.palette.open",
            "ui.analyze.open",
            "ui.changeGraphType.open",
            "ui.clonePipeline.open",
            "ui.zoom.fitSheet",
            "ui.gallery.toggle",
            "ui.rulers.toggle",
            "ui.pane.cycle",
            "ui.shortcut.set",
            "ui.shortcut.reset",
            "ui.locale.set",
            "ui.theme.set",
            "ui.screenshot",
            "app.about",
        ] {
            assert!(UI_COMMANDS.iter().any(|(id, ..)| *id == required), "missing UI command {required}");
        }
        let locale_doc = UI_COMMANDS.iter().find(|(id, ..)| *id == "ui.locale.set").map(|&(_, _, _, doc)| doc).unwrap_or("");
        for tag in ["it", "nl", "pl", "ru", "tr", "ar", "he"] {
            assert!(locale_doc.contains(tag), "ui.locale.set params doc missing `{tag}`: {locale_doc}");
        }
    }

    #[test]
    fn ui_app_facade_and_control_line_smoke() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();
        let reply = app.dispatch_control_line("{\"id\":\"1\",\"method\":\"ui.navigator.toggle\"}");
        let v: Value = serde_json::from_str(&reply)?;
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["id"], json!("1"));
        assert!(!app.nav.navigator_open);

        let create = app.dispatch_control_line(
            "{\"id\":\"2\",\"method\":\"sheet.createData\",\"params\":{\"sheet_id\":\"data-1\",\"title\":\"Assay\",\"kind\":\"xy\",\"auto_graph\":true}}",
        );
        let c: Value = serde_json::from_str(&create)?;
        assert_eq!(c["ok"], json!(true));
        assert!(!app.nav.entries.is_empty());
        assert!(app.visible_grid_window().is_some());
        Ok(())
    }

    #[test]
    fn control_commands_ui_parity_clipboard_and_print_spool() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();

        // Populate data + graph
        let script = [
            json!({"id": 1, "op": "ping"}),
            json!({"id": 2, "op": "command.list"}),
            json!({"id": 3, "op": "command.run", "command": "sheet.createData", "params": {"sheet_id": "xy-1", "graph_sheet_id": "graph-xy-1", "title": "Dose", "kind": "xy", "data_sets": 1, "rows": 3, "auto_graph": true}}),
            json!({"id": 4, "op": "table.setXCells", "params": {"sheet_id": "xy-1", "values": [1.0, 2.0, 3.0]}}),
            json!({"id": 5, "op": "table.setCell", "params": {"sheet_id": "xy-1", "row": 0, "col": 0, "subcol": 0, "value": 10.0}}),
            json!({"id": 6, "op": "table.setCell", "params": {"sheet_id": "xy-1", "row": 1, "col": 0, "subcol": 0, "value": 20.0}}),
            json!({"id": 7, "op": "table.setCell", "params": {"sheet_id": "xy-1", "row": 2, "col": 0, "subcol": 0, "value": 30.0}}),
            json!({"id": 8, "op": "ui.family.toggle"}),
            json!({"id": 9, "op": "ui.palette.open"}),
            json!({"id": 10, "op": "ui.analyze.open", "params": {"sheet_id": "xy-1"}}),
            json!({"id": 11, "op": "ui.changeGraphType.open", "params": {"sheet_id": "graph-xy-1"}}),
            json!({"id": 12, "op": "ui.clonePipeline.open", "params": {"sheet_id": "xy-1"}}),
            json!({"id": 13, "op": "ui.zoom.fitSheet"}),
            json!({"id": 14, "op": "ui.locale.set", "params": {"locale": "de"}}),
            json!({"id": 15, "op": "ui.theme.set", "params": {"mode": "dark"}}),
            json!({"id": 16, "op": "ui.screenshot", "params": {"width_px": 256, "height_px": 160, "dpi": 72}}),
            json!({"id": 17, "op": "render.svg", "sheetId": "graph-xy-1"}),
            json!({"id": 18, "op": "render.pdf", "sheetId": "graph-xy-1"}),
            json!({"id": 19, "op": "doc.inspect"}),
            json!({"id": 20, "op": "ui.state"}),
            json!({"id": 21, "op": "app.about"}),
            json!({"id": 22, "op": "app.quit"}),
        ];

        let mut input = String::new();
        for req in &script {
            input.push_str(&serde_json::to_string(req)?);
            input.push('\n');
        }
        input.push_str("{\"id\": 99, \"op\": \"ping\"}\n");

        let mut reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut output = Vec::new();
        serve_control_stream(&mut app, &mut reader, &mut output)?;

        let out_str = String::from_utf8(output)?;
        let lines: Vec<&str> = out_str.lines().collect();
        assert_eq!(lines.len(), script.len(), "stream must stop immediately after app.quit");
        for line in &lines {
            let parsed: Value = serde_json::from_str(line)?;
            assert_eq!(parsed["ok"], json!(true), "unexpected error response: {parsed}");
        }

        let screenshot: Value = serde_json::from_str(lines[15])?;
        assert_eq!(screenshot["result"]["format"], json!("png"));
        assert_eq!(screenshot["result"]["width_px"], json!(256));
        assert!(screenshot["result"]["data_base64"].as_str().is_some_and(|b64| b64.starts_with("iVBORw0K")));

        let svg_resp: Value = serde_json::from_str(lines[16])?;
        assert!(svg_resp["result"]["svg"].as_str().is_some_and(|s| s.contains("<svg")));

        let ui_state: Value = serde_json::from_str(lines[19])?;
        assert_eq!(ui_state["result"]["locale"], json!("de"));
        assert_eq!(ui_state["result"]["theme_mode"], json!("dark"));
        assert_eq!(ui_state["result"]["family_open"], json!(false));

        // Hardware payloads
        let graph_id = app.document().graph_sheets.first().map(|g| g.id.clone()).ok_or("missing auto graph")?;
        let bundle = ClipboardBundle::from_sheet(app.session(), &graph_id, 144)?;
        assert!(bundle.pdf_bytes.starts_with(b"%PDF-"));
        assert!(bundle.svg.contains("<svg"));
        assert!(bundle.html.contains("<div class=\"lumencraft-figure\""));
        assert!(bundle.png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

        let data_sheet = app.document().data_sheets.first().ok_or("missing data sheet")?;
        let tsv = ClipboardBundle::tsv_from_datasheet(data_sheet, ',')?;
        assert!(tsv.contains('\t'));
        let block = ClipboardBundle::tsv_from_datasheet_range(data_sheet, SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 1, 0)), '.');
        assert!(block.contains("10"));

        let print_out = PrintJobSpec::publication_100_percent().with_crop_and_bleed(9.0).spool_pdf(app.session())?;
        assert!(print_out.pdf_bytes.starts_with(b"%PDF-"));
        assert_eq!(print_out.page_count, 1);
        assert!((print_out.scale_percent - 100.0).abs() < 1e-9);

        assert!(parse_control_port("0").is_err());
        assert_eq!(parse_control_port("9001").ok(), Some(9001));
        Ok(())
    }

    #[test]
    fn atomic_batch_rolls_back_state_on_failure() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();
        let req = json!({
            "id": "b1",
            "method": "command.batch",
            "params": {
                "atomic": true,
                "commands": [
                    {"command": "sheet.createData", "params": {"sheet_id": "temp-1", "title": "Temp"}},
                    {"command": "nonexistent.command", "params": {}}
                ]
            }
        });
        let reply = app.dispatch_control_line(&serde_json::to_string(&req)?);
        let v: Value = serde_json::from_str(&reply)?;
        assert_eq!(v["ok"], json!(false));
        assert_eq!(app.document().total_sheet_count(), 0);
        Ok(())
    }
}
