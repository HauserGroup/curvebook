# SEC-13 Audit Report: UI Core Control, Actions & Input (`crates/ui-egui`)

- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-13` — `crates/ui-egui/Cargo.toml` and `crates/ui-egui/src/{control_io.rs, feedback.rs, gui_actions.rs, inspect.rs, interaction.rs, lib.rs, modal_predicates.rs, os_integration.rs, theme.rs, workspace_nav.rs}` (`11` files, `20,368` lines)
- **Date:** `2026-11-22`
- **Summary Counts:** `S1 = 1`, `S2 = 8`, `S3 = 5`, `S4 = 0` (`14` total findings)

---

## File Coverage Table

| File | Lines | Inspected | Concrete Invariants Verified | Findings |
| :--- | ---: | :--- | :--- | :--- |
| `crates/ui-egui/Cargo.toml` | 47 | `1..=47` | `[lints] workspace = true` inherited; `default = ["bundled-fonts"]`; non-`wasm32` target gating for `egui`, `egui_extras`, `fontdb`, `png`, and `egui_kittest`; `curvebook-platform` used for host OS/FS primitives | Clean — workspace lints, `wasm32` target split, and feature gates verified |
| `crates/ui-egui/src/control_io.rs` | 4,799 | `1..=4799` | Loopback-only TCP listener (`MAX_HEADLESS_CONTROL_CLIENTS = 32`); HTTP method rejection; `100 ms` read timeout + `2 s` total line deadline; `64 KiB` pre-auth / `16 MiB` post-auth line cap; per-request `"token"` override check; `catch_unwind` isolation in `handle_authenticated_control_line`; all `53` `UI_COMMANDS` mapped in `dispatch_ui_command_detailed`; `--gui-only` method blocks (`doc.inspect`, `doc.export`, `render.*`, `app.quit`); `command.batch` atomic rollback (`rollback_or_commit_partial_batch`); `render.pdf` host file write (`write_host_file`); `ui.result.copyMethodsReport` clipboard handling | [`FA-SEC13-01`](#fa-sec13-01), [`FA-SEC13-02`](#fa-sec13-02), [`FA-SEC13-04`](#fa-sec13-04) |
| `crates/ui-egui/src/feedback.rs` | 632 | `1..=632` | `FeedbackCategory` slug/template mapping; RFC 3986 query percent-encoding; strict `https://github.com/<owner>/<repo>/issues/new` URL validation (`validate_github_issue_url`); UTF-8 char-boundary URL truncation (`build_github_issue_url_capped`); deterministic Markdown diagnostic builder without network/telemetry calls | Clean — URL allowlisting, UTF-8 safe capping, and offline diagnostics formatting verified |
| `crates/ui-egui/src/gui_actions.rs` | 3,197 | `1..=3197` | All 8 interactive `ui.*` actions (`ui.inspect`, `ui.screenshot`, `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`, `ui.invoke_menu`, `ui.switch_tab`, `ui.answer_dialog`); `ui.screenshot` `--gui-only` / `ExternalPathPolicy` enforcement and `write_host_file`; `handle_ui_inspect` backend parity; `--gui-only` curated `ui.invoke_menu` policy (`is_gui_only_blocked_menu_command`, `find_non_empty_path_key`); dialog patch/commit dispatch; X-cell TSV multi-column paste splitting | [`FA-SEC13-01`](#fa-sec13-01), [`FA-SEC13-03`](#fa-sec13-03), [`FA-SEC13-12`](#fa-sec13-12), [`FA-SEC13-14`](#fa-sec13-14) |
| `crates/ui-egui/src/inspect.rs` | 2,506 | `1..=2506` | Deterministic `UiTreeSnapshot` / `UiWidgetNode` generation; `ZLayer` assignment; `include_bounds` geometry population; `textbox` widget IDs and enablement predicates across `Analyze`, `EditAnalysisParameters`, `PowerWizard`, `FormatSelectedPoints`, `ExportFigure`, `Print`, and `DuplicateAnalysisPrompt` (`duplicate.choice`) | [`FA-SEC13-09`](#fa-sec13-09), [`FA-SEC13-11`](#fa-sec13-11), [`FA-SEC13-12`](#fa-sec13-12), [`FA-SEC13-13`](#fa-sec13-13) |
| `crates/ui-egui/src/interaction.rs` | 1,830 | `1..=1830` | HiDPI `ViewportMetrics` clamping; UTF-8 safe `TextFieldEditState` / `InlineEditorState` char-boundary edits; pinch/wheel zoom bounds and pan inertia; inline edit commit intents (`table.setXCells`, `table.setCell`, `graph.setSpec`, `layout.setSpec`); modal key interception (`Enter` in multiline textboxes, `Escape` popups/dialogs, `Tab` modal containment); textbox copy/paste and `is_text_widget_id`/`sync_focused_text_field` coverage; workspace `Cmd/Ctrl+V` X-cell routing; `formula_bar` `Tab` scoping | [`FA-SEC13-06`](#fa-sec13-06), [`FA-SEC13-07`](#fa-sec13-07), [`FA-SEC13-08`](#fa-sec13-08), [`FA-SEC13-09`](#fa-sec13-09), [`FA-SEC13-10`](#fa-sec13-10) |
| `crates/ui-egui/src/lib.rs` | 1,840 | `1..=1840` | `#![forbid(unsafe_code)]`; `ui_scale` preference validation/clamping (`0.75..=3.0`); `BatchUndoTracker` atomic rollback accounting; `UiApp` session-boundary `path_policy` / `gui_only` preservation; `write_clipboard_text`/`write_clipboard_bundle` `human_path_only` + custom-adapter gating; `dispatch_control_line_with_action` and `try_dispatch_egui_backend` fast path vs `catch_unwind` and duplicate `dispatch_egui_screenshot_action`; offscreen `egui_shell` frame settlement (`<= 8` passes) | [`FA-SEC13-01`](#fa-sec13-01), [`FA-SEC13-02`](#fa-sec13-02), [`FA-SEC13-03`](#fa-sec13-03) |
| `crates/ui-egui/src/modal_predicates.rs` | 1,131 | `1..=1131` | Scientific rule gates `R1`–`R10`, `r1b_*`, `is_reml_mixed_effects_active`, `r_km_trend_test`; `evaluate_dialog_button` gating vs `gui_actions.rs` commits; `evaluate_edit_params_widget` Contingency CI selectors (`Newcombe-Wilson` vs `Baptista-Pike`/`Koopman`); `evaluate_analyze_widget` parity with `inspect.rs` | [`FA-SEC13-11`](#fa-sec13-11), [`FA-SEC13-12`](#fa-sec13-12) |
| `crates/ui-egui/src/os_integration.rs` | 2,130 | `1..=2130` | `ClipboardBundle::from_scene` finite positive dims + DPI bounds (`72..=1200`) + aspect clamp; `build_cf_html_payload` ASCII-lowercase byte offset safety; `StageDirGuard` RAII staging cleanup and stale preview reaper; `ProcessCommand` bounded output (`16 MiB`) and timeouts; `--` end-of-options and leading-dash / control-char rejection for printer names; full `wasm32` stubs | Clean — bounded subprocesses, RAII temp staging cleanup, UTF-8 safe CF_HTML, and `wasm32` stubs verified |
| `crates/ui-egui/src/theme.rs` | 664 | `1..=664` | `Light`, `Dark`, and `HighContrast` palette WCAG AA/AAA contrast ratios (`contrast_ratio`, `meets_wcag_aa`); `bundled-fonts` embedding; `CURVEBOOK_FONTS_DIR` directory scan gated behind `#[cfg(not(target_arch = "wasm32"))]` with `MAX_HOST_FONT_BYTES = 64 MiB` cap via `curvebook_platform::fs::read_regular_file_bytes` | Clean — contrast invariants, font byte caps, and `wasm32` gating verified |
| `crates/ui-egui/src/workspace_nav.rs` | 1,592 | `1..=1592` | `NavigatorNestingMode` folder grouping; `SAMPLE_FIXTURES` finite scientific sample generation; `WorkspaceNavState` click/drag/reorder (`commit_sheet_drop_command_for_doc` live index resolution, `collect_downstream_dependents` DFS); direct `&mut ProjectDocument` mutation methods vs headless `Session::execute` discipline | [`FA-SEC13-05`](#fa-sec13-05) |

---

## Findings

### FA-SEC13-01

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/control_io.rs:3681-3684`, `crates/ui-egui/src/control_io.rs:2091-2101`, `crates/ui-egui/src/gui_actions.rs:118-135`, `crates/ui-egui/src/lib.rs:1434-1452`
- **Category:** `security` (`contract-drift`)
- **Defect:** `control_io::write_host_file` writes directly to `std::path::Path::new(path)` via `curvebook_platform::fs::write_atomic` without validating or confining `path` against `app.session().path_policy()` (`ExternalPathPolicy::Workdir` or `ExternalPathPolicy::RejectExternal`). Meanwhile `gui_actions::handle_ui_screenshot`, `lib::dispatch_egui_screenshot_action`, and `control_io::handle_render_pdf` only check `app.is_gui_only()` and never enforce `app.session().path_policy()`.
- **Failure scenario:** When a host (such as `apps/curvebook-cli` or an embedded test/automation host) confines `UiApp` via `app.session_mut().set_path_policy(ExternalPathPolicy::workdir(&dir)?)` or `ExternalPathPolicy::RejectExternal` with `gui_only = false`, invoking `ui.screenshot` with `{"path": "/outside/workdir/escape.png"}` (or `"../escape.png"`) or `render.pdf` with `{"path": "/outside/workdir/escape.pdf"}` bypasses `WorkdirGuard` / `RejectExternal` and writes arbitrary files on the host filesystem.
- **Evidence:**
```rust
// crates/ui-egui/src/control_io.rs:3681-3684
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn write_host_file(path: &str, bytes: &[u8]) -> Result<(), String> {
    curvebook_platform::fs::write_atomic(std::path::Path::new(path), bytes).map_err(|err| format!("failed to write file `{path}`: {err}"))
}
```
```rust
// crates/ui-egui/src/gui_actions.rs:118-134
    if app.is_gui_only() && params.get("path").is_some_and(|v| !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty())) {
        return Err(EngineError::with_code(
            ErrorCode::GuiOnlyViolation,
            "direct filesystem writes via `ui.screenshot` `path` are blocked under --gui-only",
        )
        .with_remediation("Omit `path` and inspect the returned in-memory `data_base64` PNG instead."));
    }
    let opt_path = opt_str_param(params, &["path"]).map_err(bad_input)?.map(str::trim).filter(|s| !s.is_empty());

    let (png_bytes, out_w, out_h) = capture_screenshot_png(app, region, width_px, height_px, dpi)?;

    let saved_path = match opt_path {
        Some(path) => {
            write_host_file(path, &png_bytes).map_err(|err| EngineError::with_code(ErrorCode::IoError, err))?;
            Some(path.to_owned())
        }
```
  The exact same pattern recurs in `crates/ui-egui/src/lib.rs:1434-1452` (`dispatch_egui_screenshot_action`) and `crates/ui-egui/src/control_io.rs:2091-2101` (`handle_render_pdf`). By contrast, engine command execution (`crates/engine/src/lib.rs:1298-1300`) always enforces `self.path_policy.enforce_command_path_params(&mut params)` / `ExternalPathPolicy::write_atomic_bytes`.
- **Suggested fix:** Remove raw `write_host_file` and expose a checked helper on `Session` (wrapping `ExternalPathPolicy::write_atomic_bytes` from `crates/engine/src/path_policy.rs:181-197`) so every host file write in `ui.screenshot` and `render.pdf` rejects `RejectExternal`, canonicalizes/confines inside `WorkdirGuard`, and rejects `..`/symlink escapes before writing.
- **Related:** `crates/engine/src/path_policy.rs:181-197`; `crates/engine/src/lib.rs:1298-1300`.

---

### FA-SEC13-02

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/lib.rs:301-317`, `crates/ui-egui/src/lib.rs:502-529`, `crates/ui-egui/src/control_io.rs:1661-1686`
- **Category:** `data-loss` (`wrong-result`)
- **Defect:** Atomic `command.batch` rollback (`BatchUndoTracker` + `rollback_or_commit_partial_batch`) only clones a pre-batch `fallback_session` when `force_fallback` is true (`project.save` while dirty, or a forward mutation after net negative undo depth). For normal forward batches, rollback loops `0..tracker.net_batch_depth` calling `let _ = app.session.undo();` — even though `command.batch` permits up to `MAX_CONTROL_BATCH_STEPS = 256` steps while `Session` caps undo history (`MAX_UNDO_ENTRIES = 64`, `MAX_UNDO_CELL_BUDGET = 25_000_000`, `MAX_UNDO_CHECKPOINT_BYTES_BUDGET = 16 MiB`) and evicts older undo snapshots via `pop_front`.
- **Failure scenario:** An `atomic: true` `command.batch` runs 65 mutating steps (or ≥2 large-table imports/mutations exceeding `MAX_UNDO_CELL_BUDGET` or `MAX_UNDO_CHECKPOINT_BYTES_BUDGET`), causing `Session::push_undo_snapshot` to evict the pre-batch snapshot, and then fails on the final step (or exceeds the response payload cap). `rollback_or_commit_partial_batch` pops at most the surviving `undo_stack` entries, silently ignores `Ok(false)` from `app.session.undo()`, leaves earlier batch mutations committed on `app.session`, and returns `"rolled_back=true"` to the caller.
- **Evidence:**
```rust
// crates/ui-egui/src/lib.rs:509-528
        if let Some(t) = tracker.as_deref_mut() {
            let is_undo_or_redo = matches!(command_id, "edit.undo" | "edit.redo");
            let force_fallback = (t.baseline_dirty && command_id == "project.save") || (t.net_batch_depth < 0 && !is_undo_or_redo);
            t.capture_fallback_baseline_before(session, force_fallback);
        }
        let outcome = op(session)?;
```
```rust
// crates/ui-egui/src/control_io.rs:1671-1686
        if let Some(tracker) = child_tracker {
            if let Some(fallback) = tracker.fallback_session {
                app.session = fallback;
            } else {
                if tracker.net_batch_depth > 0 {
                    for _ in 0..tracker.net_batch_depth {
                        let _ = app.session.undo();
                    }
                } else if tracker.net_batch_depth < 0 {
                    for _ in 0..(-tracker.net_batch_depth) {
                        let _ = app.session.redo();
                    }
                }
                app.session.set_file_path(tracker.baseline_file_path, !tracker.baseline_dirty);
            }
        }
```
  And in `crates/engine/src/lib.rs:1201-1208`, `push_undo_snapshot` evicts `undo_stack.pop_front()` whenever `undo_stack.len() > 64` or budgets are exceeded.
- **Suggested fix:** Capture `fallback_session` whenever `net_batch_depth` / `session.undo_depth()` approaches `MAX_UNDO_ENTRIES`, or whenever `session` undo memory/cell budgets are close to eviction, or before executing the first mutating step in an atomic batch (and verify every `undo()`/`redo()` call returns `Ok(true)` during rollback rather than discarding the return value).
- **Related:** Partial regression / incomplete fix for `local-code-audit` `#2` and `#118`.

---

### FA-SEC13-03

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/lib.rs:922-934`, `crates/ui-egui/src/lib.rs:1052-1064`, `crates/ui-egui/src/lib.rs:1388-1470`, `crates/ui-egui/src/gui_actions.rs:90-153`
- **Category:** `contract-drift` (`no-panic` / `cleanup`)
- **Defect:** Three coupled issues around `try_dispatch_egui_backend`:
  1. `UiApp::dispatch_control_line_with_action` (`lib.rs:922-934`) short-circuits top-level `ui.inspect` and `ui.screenshot` via `try_dispatch_egui_backend` *before* `control_io::handle_control_line_with_action`, bypassing `control_io::handle_authenticated_control_line`'s `std::panic::catch_unwind` isolation (`control_io.rs:955-968`). Likewise `apps/curvebook/src/desktop_host.rs:897-928` calls `try_dispatch_egui_backend` on authenticated connections before `serve_control_stream_with_quit_and_hook`, skipping both `catch_unwind` and per-request `"token"` override verification.
  2. When `"ui.inspect"` is invoked via `ui.action` or `command.batch` (not top-level method `"ui.inspect"`), it bypasses `try_dispatch_egui_backend` and hits `gui_actions::handle_ui_inspect` (`gui_actions.rs:90-97`), which unconditionally calls semantic `inspect_ui_tree_for_metrics` instead of `app.inspect_egui_shell_snapshot`, unlike `handle_ui_screenshot` (`gui_actions.rs:127, 163`) which routes through `app.render_egui_shell_screenshot_png`.
  3. `lib.rs:1399-1470` (`dispatch_egui_screenshot_action`) duplicates `gui_actions::handle_ui_screenshot` (`gui_actions.rs:99-153`).
- **Failure scenario:** Any panic inside offscreen `egui_shell` layout or rasterization during top-level `ui.inspect` or `ui.screenshot` unwinds out of the control dispatcher instead of returning `ErrorCode::InternalPanic`; sending `"ui.inspect"` with an explicit stale `"token"` over a live desktop connection succeeds instead of being rejected; and `"ui.inspect"` inside `command.batch` or `ui.action` returns semantic fallback bounds (`None` for dialog controls) instead of settled `egui_shell` widget bounds.
- **Evidence:**
```rust
// crates/ui-egui/src/lib.rs:922-934
    pub fn dispatch_control_line_with_action(&mut self, line: &str) -> (String, ControlLoopAction) {
        #[cfg(not(target_arch = "wasm32"))]
        if line.len() <= MAX_CONTROL_LINE_BYTES
            && let Ok(req) = serde_json::from_str::<ControlRequest>(line.trim())
            && let Some(res) = self.try_dispatch_egui_backend(&req.method, &req.params)
        {
            let reply = encode_egui_control_response(req.id, res);
            if let Ok(json_line) = serde_json::to_string(&reply) {
                return (json_line, ControlLoopAction::Continue);
            }
        }
        handle_control_line_with_action(self, line)
    }
```
```rust
// crates/ui-egui/src/gui_actions.rs:90-97
fn handle_ui_inspect(app: &mut UiApp, params: &Value) -> Result<Value, EngineError> {
    app.sync_views();
    let metrics = parse_metrics(params)?;
    let scope = opt_str_param(params, &["scope"]).map_err(bad_input)?;
    let include_bounds = opt_bool_param(params, &["include_bounds"]).map_err(bad_input)?.unwrap_or(true);
    let snap = inspect_ui_tree_for_metrics(app, metrics, scope, include_bounds);
    serde_json::to_value(snap).map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("failed to serialize ui tree: {err}")))
}
```
- **Suggested fix:** Remove the pre-`control_io` `try_dispatch_egui_backend` fast path from `dispatch_control_line_with_action` (and `desktop_host.rs`), route `ui.inspect` in `gui_actions::handle_ui_inspect` through `app.inspect_egui_shell_snapshot` directly, consolidate `ui.screenshot` into `gui_actions::handle_ui_screenshot` alone, and invoke `egui_dispatch_hook` only inside `control_io` after auth/token checks and inside `catch_unwind`.

---

### FA-SEC13-04

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/control_io.rs:2611-2650`
- **Category:** `contract-drift` (`data-loss`)
- **Defect:** `dispatch_ui_result_copy_methods_report` bypasses `UiApp::write_clipboard_text` (`lib.rs:776-781`) and calls `os_integration::write_os_pasteboard(&report)` directly while gating only on `app.is_gui_only()`. It ignores `app.is_human_path_only()` and any custom `PlatformClipboardAdapter` (`StubPlatformClipboardAdapter`), and silently swallows `Err` returns (`if !skip_os_clipboard && let Ok(status) = ...`).
- **Failure scenario:** Under `human_path_only` mode (without `--gui-only`) or in headless/test environments that attach a `StubPlatformClipboardAdapter` via `UiApp::set_clipboard_adapter`, invoking `ui.result.copyMethodsReport` writes to the real host OS pasteboard instead of the injected adapter (or fails silently with `"clipboard_status": null` without reporting the OS clipboard error).
- **Evidence:**
```rust
// crates/ui-egui/src/control_io.rs:2626-2637
    let skip_os_clipboard = app.is_gui_only() || opt_bool_param(params, &["skip_os_clipboard"])? == Some(true);
    let intent = app.canvas_dialogs.result_view.generate_report_intent(&rs_id);
    let outcome = app.execute_command(&intent.command_id, intent.params)?;
    let report = outcome.payload.get("report").and_then(Value::as_str).unwrap_or_default().to_owned();
    app.canvas_dialogs.result_view.last_methods_report = Some(report.clone());

    #[cfg(not(target_arch = "wasm32"))]
    let clipboard_status = if !skip_os_clipboard && let Ok(status) = write_os_pasteboard(&report) {
        serde_json::to_value(status).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
```
  Compare with `crates/ui-egui/src/lib.rs:776-781`, which gates live OS pasteboard access on `!self.clipboard_adapter.is_custom && (self.is_gui_only() || self.is_human_path_only())` and otherwise dispatches through `self.clipboard_adapter.adapter.write_text(text)`.
- **Suggested fix:** Replace direct `write_os_pasteboard(&report)` with `app.write_clipboard_text(&report)` when `!skip_os_clipboard`, and propagate or record failures in `clipboard_status` cleanly (also removing the `#[cfg(target_arch = "wasm32")]` divergence since `UiApp::write_clipboard_text` handles `wasm32`).

---

### FA-SEC13-05

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/workspace_nav.rs:293-300`, `crates/ui-egui/src/workspace_nav.rs:477-489`, `crates/ui-egui/src/workspace_nav.rs:953-984`
- **Category:** `architecture` (`wrong-result`)
- **Defect:** `WorkspaceNavState::click_navigator_entry` and `click_family_entry` mutate `ProjectDocument` directly (`doc.select_in_navigator(sheet_id)?`, `doc.select_in_family_pane(sheet_id)?`) *and* push a `sheet.select` `ToolCommandIntent` into `self.pending_intents`; similarly `TableCreationDraft::create_into_document` and `ClonePipelineDraft::execute_in_document` mutate `&mut ProjectDocument` directly outside `Session::execute`.
- **Failure scenario:** Any caller using these public `WorkspaceNavState` / draft APIs mutates the document outside `Session::execute`, bypassing undo snapshots, `dirty` tracking, crash-recovery WAL journaling, and recalculation hooks; if a caller calls `click_navigator_entry(&mut doc, id)` and then executes `pending_intents`, the document was already mutated before `Session::execute` snapshots pre-command state.
- **Evidence:**
```rust
// crates/ui-egui/src/workspace_nav.rs:953-984
    pub fn click_navigator_entry(&mut self, doc: &mut ProjectDocument, sheet_id: &SheetId) -> Result<(), DocError> {
        doc.select_in_navigator(sheet_id)?;
        self.pending_intents.push(ToolCommandIntent {
            command_id: String::from("sheet.select"),
            params: json!({ "sheet_id": sheet_id.as_str(), "mode": "navigator" }),
        });
        self.sync_from_document(doc);
        Ok(())
    }

    pub fn click_family_entry(&mut self, doc: &mut ProjectDocument, sheet_id: &SheetId, double_click: bool) -> Result<(), DocError> {
        let is_data = doc.sheet_section(sheet_id) == Some(SheetSection::Data);
        if double_click || is_data {
            doc.select_in_navigator(sheet_id)?;
            ...
        } else {
            doc.select_in_family_pane(sheet_id)?;
            ...
        }
        self.sync_from_document(doc);
        Ok(())
    }
```
  Likewise `TableCreationDraft::create_into_document` (`293-300`) and `ClonePipelineDraft::execute_in_document` (`477-489`) call `doc.add_data_sheet(...)` / `doc.clone_pipeline(...)` directly (currently exercised by `tests/grid_perf_s1.rs`).
- **Suggested fix:** Change `click_navigator_entry` and `click_family_entry` to take `&ProjectDocument` and enqueue `pending_intents` strictly without mutating `doc` (syncing navigator state after the host executes the command), and remove or gate `create_into_document` / `execute_in_document` behind `#[cfg(test)]` / test helpers.

---

### FA-SEC13-06

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/interaction.rs:1624-1637`, `crates/ui-egui/src/interaction.rs:1660-1742`
- **Category:** `wrong-result` (`data-loss`)
- **Defect:** When a modal dialog is open and focus is inside a multiline textbox (such as `import_paste.input.csv`, `import_paste.input.notes`, `import_paste.input.plate_values`, `page_layout.callout_input`, or `exclude.note_input`), `handle_modal_key_down` dispatches to `handle_textbox_key`, which only handles `"Enter" | "Return"` for `"formula_bar.input" | "mv_formula.name.input"` and returns `Ok(None)` for all other textboxes. Execution then falls straight through to `submit_active_dialog_ok(app)`.
- **Failure scenario:** Pressing `Enter` to start a new line while typing CSV rows, notes, or layout Markdown inside a modal dialog prematurely submits and closes the dialog instead of inserting `\n`.
- **Evidence:**
```rust
// crates/ui-egui/src/interaction.rs:1624-1637
    if let Some(fid) = app.focus_widget_id.clone()
        && is_text_widget_id(&fid)
        && let Some(out) = handle_textbox_key(app, metrics, &fid, raw_key, modifiers)?
    {
        return Ok(out);
    }

    if !modifiers.ctrl && !modifiers.meta && !modifiers.alt && matches!(trimmed, "Enter" | "Return") {
        let res = crate::gui_actions::submit_active_dialog_ok(app).map_err(|e| e.to_string())?;
        if app.canvas_dialogs.active_dialog == ActiveDialog::None {
            clear_context_menu_state(app);
        }
        return Ok(UiEventOutcome { handled: true, action: String::from("dialog.confirmed"), command_result: Some(res), ..Default::default() });
    }
```
  And in `handle_textbox_key` (`1667-1741`), only `"formula_bar.input" | "mv_formula.name.input"` checks `matches!(key, "Enter" | "Return")`; all other `fid` values reach `key.chars().count() == 1` (`"Enter"` has length 5) and return `Ok(None)`.
- **Suggested fix:** Classify multiline textboxes explicitly (e.g. `is_multiline_text_widget_id(fid)`), and handle plain `Enter`/`Return` in `handle_textbox_key` for those IDs by calling `insert_text("\n")` and `sync_focused_text_field(app, fid)`, reserving `Cmd/Ctrl+Enter` or unfocused `Enter` for modal submission.

---

### FA-SEC13-07

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/interaction.rs:1694-1708` (`handle_textbox_key`), `crates/ui-egui/src/interaction.rs:1317-1326` (`read_clipboard_text_for_paste`)
- **Category:** `wrong-result`
- **Defect:** `Cmd/Ctrl+C` inside `handle_textbox_key` writes the selected text only to `app.interaction.last_clipboard_text` without calling `app.write_clipboard_text(&sel)`. Meanwhile `Cmd/Ctrl+V` calls `read_clipboard_text_for_paste(app)`, which reads `app.read_clipboard_text()` (OS clipboard or custom `PlatformClipboardAdapter`) first whenever non-empty and only falls back to `last_clipboard_text` if unavailable/empty.
- **Failure scenario:** If the host clipboard or custom `PlatformClipboardAdapter` already contains prior text, selecting text in any textbox (such as `formula_bar.input`, `palette.query`, or `info.notes`), pressing `Cmd/Ctrl+C`, and then pressing `Cmd/Ctrl+V` (or pasting externally) pastes the stale OS/adapter clipboard string instead of what was just copied. (Additionally, `Cmd/Ctrl+X` cut is unhandled in `handle_textbox_key`.)
- **Evidence:**
```rust
// crates/ui-egui/src/interaction.rs:1694-1708
    if (modifiers.ctrl || modifiers.meta) && key.eq_ignore_ascii_case("c") {
        if let Some(sel) = app.interaction.text_field_buffers.get(fid).and_then(TextFieldEditState::copy_selection) {
            app.interaction.last_clipboard_text = Some(sel);
        }
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("textbox.copy"), ..Default::default() }));
    }
    if (modifiers.ctrl || modifiers.meta) && key.eq_ignore_ascii_case("v") {
        if let Some(clip) = read_clipboard_text_for_paste(app) {
```
  Where `read_clipboard_text_for_paste` (`1317-1326`) checks `app.read_clipboard_text()` before `app.interaction.last_clipboard_text`.
- **Suggested fix:** Call `let _ = app.write_clipboard_text(&sel);` when copying in `handle_textbox_key`, and support `Cmd/Ctrl+X` cut symmetrically.

---

### FA-SEC13-08

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/interaction.rs:1328-1358` (`try_handle_workspace_paste_shortcut`), `crates/ui-egui/src/grid_view.rs:1365-1371`
- **Category:** `wrong-result`
- **Defect:** `try_handle_workspace_paste_shortcut` ignores `app.grid.active_x_cell` and always reads `let anchor = app.grid.tool.active.unwrap_or(GridCellCoord::new(0, 0, 0));` before dispatching `table.paste` into Y columns. However, selecting an X/Time cell via `GridViewState::select_x_cell_sync` sets `active_x_cell = Some((row, subcol))` and clears `tool.active = None`.
- **Failure scenario:** On an XY or Survival sheet, selecting an X/Time cell (which leaves `active_x_cell = Some((row, subcol))` and `tool.active = None`) and pressing `Cmd/Ctrl+V` via `UiInputEvent` silently pastes into Y column `(0, 0, 0)` instead of the selected X/Time cell — unlike `toggle_exclude_active_grid_cells` (`interaction.rs:1389-1400`) and `apply_type_text_to_grid` (`gui_actions.rs:1430-1433`), which both handle `active_x_cell`.
- **Evidence:**
```rust
// crates/ui-egui/src/interaction.rs:1335-1349
    let Some(clip) = read_clipboard_text_for_paste(app) else {
        return Ok(None);
    };
    let anchor = app.grid.tool.active.unwrap_or(GridCellCoord::new(0, 0, 0));
    let res = execute_engine_value(
        app,
        "table.paste",
        json!({
            "sheet_id": ds_id.as_str(),
            "text": clip,
            "start_row": anchor.row,
            "start_col": anchor.col,
            "start_subcol": anchor.subcol,
```
- **Suggested fix:** Check `app.grid.active_x_cell` first in `try_handle_workspace_paste_shortcut` (matching `toggle_exclude_active_grid_cells` and `gui_actions::apply_type_text_to_x_cell`) to route pasted text into `table.setXCells` (with Y-spill handling if multi-column TSV) before falling back to `app.grid.tool.active`.
- **Related:** `R15-GUI-12` (previously fixed for `egui_shell/data_grid/shortcuts.rs`, missed in `interaction.rs`).

---

### FA-SEC13-09

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/interaction.rs:1095-1141`, `crates/ui-egui/src/interaction.rs:1182-1198`, `crates/ui-egui/src/inspect.rs:839-2437`
- **Category:** `wrong-result` (`contract-drift`)
- **Defect:** `is_text_widget_id` and `sync_focused_text_field` fail to cover active `textbox` widget IDs emitted by `inspect.rs`:
  1. `is_text_widget_id` (`1182-1198`) only matches 11 exact IDs plus `*search*`/`*input*`, omitting modal/inspector textboxes such as `"checkpoints.name"`, `"checkpoints.note"`, `"exclude.note"`, `"edit.custom_title"`, `"edit.fit.model_id"`, `"edit.fit.custom_dsl"`, `"edit.contingency.expected_proportions"`, `"edit.glm.predictor_columns"`, `"edit.cox.predictor_columns"`, `"change_data.selected_data_sets"`, `"randomize.groups"`, `"randomize.block_sizes"`, `"randomize.title"`, `"sample.title"`, `"simulate.title"`, `"simulate.column_name"`, `"format_points.color"`, `"format_points.size_pt"`, `"create_graph.title"`, `"create_graph.y_axis_title"`, `"import.title"`, `"import.x_title"`, `"import.tidy_csv"`, `"axes.thresholds"`, `"power.alpha"`, `"power.target_power"`, `"power.standardized_effect"`.
  2. Even for IDs that *are* matched by `is_text_widget_id` (`"info.notes"`, `"info.secondary.notes"`), `sync_focused_text_field` (`1105-1140`) falls into `_ => {}` without updating any dialog or document state.
- **Failure scenario:** Focusing any unlisted `textbox` widget ID and sending `TextInput`, `Backspace`, `Delete`, `ArrowLeft`/`Right`, or `Cmd/Ctrl+C`/`V` via `UiInputEvent` treats the focus as non-text: characters bypass `text_field_buffers`, `Cmd/Ctrl+E/C/V` fire workspace shortcuts instead of editing text, and `info.notes` edits remain orphaned in `text_field_buffers` without syncing to the Info sheet.
- **Evidence:**
```rust
// crates/ui-egui/src/interaction.rs:1182-1198
fn is_text_widget_id(wid: &str) -> bool {
    matches!(
        wid,
        "palette.query"
            | "formula_bar.input"
            | "info.notes"
            | "info.secondary.notes"
            | "edit.transform.formula"
            | "edit.transform.filter_formula"
            | "edit.transform.selected_data_sets"
            | "edit.transform.baseline_columns"
            | "axes.y_thresholds"
            | "axes.x_thresholds"
            | "axes.title"
    ) || wid.contains("search")
        || wid.contains("input")
}
```
  And `sync_focused_text_field` (`1105-1140`) contains no arms for `"info.notes"`, `"info.secondary.notes"`, `"checkpoints.*"`, `"create_graph.*"`, `"edit.fit.*"`, etc., hitting `_ => {}`.
- **Suggested fix:** Derive `is_text_widget_id` from a shared registry/metadata table (or inspect node role metadata) alongside `inspect.rs` and wire all synced `textbox` widget IDs in `sync_focused_text_field` (and `info.notes` commit/sync).

---

### FA-SEC13-10

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/interaction.rs:1441-1479` (`handle_key_down`)
- **Category:** `wrong-result`
- **Defect:** Lines `1441-1457` already handle `Tab` completion when `app.grid.formula_bar.active && app.focus_widget_id.as_deref() == Some("formula_bar.input")`. Lines `1467-1479` then add a second `Tab` check guarded only by `app.grid.formula_bar.active`, without checking `focus_widget_id`.
- **Failure scenario:** While the formula bar is open (`formula_bar.active == true`) and focus has moved to the grid, navigator, or another control, pressing `Tab` is intercepted by lines `1467-1479` as `formula_bar.tab_complete` instead of propagating `Tab` navigation.
- **Evidence:**
```rust
// crates/ui-egui/src/interaction.rs:1467-1479
    if trimmed.eq_ignore_ascii_case("tab") && !modifiers.ctrl && !modifiers.meta && !modifiers.alt {
        if app.grid.formula_bar.active {
            let doc = app.document().clone();
            let accepted = app.grid.formula_bar.accept_completion(&doc);
            return Ok(UiEventOutcome {
                handled: true,
                action: String::from("formula_bar.tab_complete"),
                command_result: Some(json!({ "completion_accepted": accepted, "input": app.grid.formula_bar.input })),
                ..Default::default()
            });
        }
        let _ = modifiers;
    }
```
- **Suggested fix:** Remove the redundant unscoped `formula_bar.active` `Tab` block at `1467-1479` since lines `1441-1457` already handle `Tab` cleanly when `"formula_bar.input"` has focus.

---

### FA-SEC13-11

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/modal_predicates.rs:885-901` (`evaluate_edit_params_widget`), `crates/ui-egui/src/inspect.rs:1376-1399`
- **Category:** `contract-drift` (`wrong-result`)
- **Defect:** For `2x2` Contingency analyses, `ContingencySpec::compute_odds_and_risk_ratios` controls Odds Ratio, Relative Risk, and Attributable Risk / Difference in Proportions (`Newcombe-Wilson` CI). `EditAnalysisParametersState::sync_rule_flags_from_kind` (`dialogs_canvas/edit_params.rs:285-288`) initializes `contingency_odds_ratio_checked` and `contingency_relative_risk_checked` from `compute_odds_and_risk_ratios`, leaving `contingency_attributable_risk_checked = false`. While Baptista-Pike (`889`) and Koopman (`894`) check `edit.contingency_*_checked || cont.compute_odds_and_risk_ratios`, Newcombe-Wilson (`897` in `modal_predicates.rs` and `1397` in `inspect.rs`) checks `edit.contingency_attributable_risk_checked` alone.
- **Failure scenario:** Reopening `Change Analysis Parameters` on an existing 2x2 Contingency analysis with `compute_odds_and_risk_ratios: true` falsely disables `edit.contingency.ci.newcombe_wilson` in both `inspect.rs` (`ui.inspect`) and `modal_predicates.rs` (`R6` enablement gate) until the user manually clicks `edit.contingency.attributable_risk`.
- **Evidence:**
```rust
// crates/ui-egui/src/modal_predicates.rs:885-898
        if norm.contains("baptistapike") || norm.contains("woolf") {
            return r6_contingency_ci_selector(
                rows,
                cols,
                edit.contingency_odds_ratio_checked || cont.compute_odds_and_risk_ratios,
                "Baptista-Pike / Woolf",
            );
        }
        if norm.contains("koopman") {
            return r6_contingency_ci_selector(rows, cols, edit.contingency_relative_risk_checked || cont.compute_odds_and_risk_ratios, "Koopman");
        }
        if norm.contains("newcombewilson") {
            return r6_contingency_ci_selector(rows, cols, edit.contingency_attributable_risk_checked, "Newcombe-Wilson");
        }
```
```rust
// crates/ui-egui/src/inspect.rs:1393-1399
        node(
            "edit.contingency.ci.newcombe_wilson",
            "combobox",
            "AR CI (Newcombe-Wilson)",
            json!(ep.contingency_arr_ci_method),
            r6_contingency_ci_selector(rows, cols, ep.contingency_attributable_risk_checked, "Newcombe-Wilson"),
            pos(19),
        ),
```
- **Suggested fix:** Check `edit.contingency_attributable_risk_checked || cont.compute_odds_and_risk_ratios` for Newcombe-Wilson in both `modal_predicates.rs:897` and `inspect.rs:1397` (and sync `contingency_attributable_risk_checked` in `sync_rule_flags_from_kind` in `edit_params.rs`).

---

### FA-SEC13-12

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/modal_predicates.rs:736-750` vs `crates/ui-egui/src/gui_actions.rs:2678-2685`; `crates/ui-egui/src/modal_predicates.rs:932-961` vs `crates/ui-egui/src/inspect.rs:1117-1123`
- **Category:** `contract-drift`
- **Defect:** Two enablement predicate mismatches in `modal_predicates.rs`:
  1. `evaluate_format_points_button` (`modal_predicates.rs:743-745`) disables `ok`/`apply` whenever `dialogs.format_points.selected_points.is_empty()`. Yet `commit_format_points_action` (`gui_actions.rs:2679-2685`) intentionally has a fallback to populate `format_points.selected_points` from `app.grid.selected_graph_point_targets(...)` when `selected_points` is empty. Because `evaluate_dialog_button` gates `execute_dialog_button_action` / `handle_answer_dialog`, that fallback can never run when `selected_points` starts empty.
  2. `evaluate_analyze_widget` (`modal_predicates.rs:932-961`) has no rule for `"analyze.factorial.residual_covariance"` (`norm == "analyzefactorialresidualcovariance"`), returning `ControlEnablement::enabled()`, whereas `inspect.rs:1117-1123` explicitly disables it via `r1_sphericity_or_reml_control(st.factorial_repeated_measures != RepeatedMeasuresDesign::None)`.
- **Failure scenario:** (1) Opening `FormatSelectedPoints` when points are selected/brushed via grid/brush but `format_points.selected_points` was cleared/empty disables `OK`/`Apply` instead of letting `commit_format_points_action` resolve brushed targets; (2) Querying `evaluate_dialog_widget` on `"analyze.factorial.residual_covariance"` disagrees with `ui.inspect` (`enabled: true` vs `false`).
- **Evidence:**
```rust
// crates/ui-egui/src/modal_predicates.rs:743-745
    if dialogs.format_points.selected_points.is_empty() {
        return ControlEnablement::disabled("Select at least one data point before applying point formatting.");
    }
```
```rust
// crates/ui-egui/src/gui_actions.rs:2678-2685
fn commit_format_points_action(app: &mut UiApp) -> Result<Value, EngineError> {
    if app.canvas_dialogs.format_points.selected_points.is_empty()
        && let Some(active_ds_id) = app.brushed_points.first().map(|p| p.sheet_id.clone())
        && let Some(ds) = app.document().data_sheet(&active_ds_id)
    {
        let targets = app.grid.selected_graph_point_targets(ds, usize::MAX);
        app.canvas_dialogs.format_points.set_selected_targets(targets);
    }
```
  And `inspect.rs:1117-1123` uses `r1_sphericity_or_reml_control(st.factorial_repeated_measures != RepeatedMeasuresDesign::None)` on `"analyze.factorial.residual_covariance"`, absent from `evaluate_analyze_widget` (`modal_predicates.rs:932-961`).
- **Suggested fix:** Populate brushed targets when opening `FormatSelectedPoints` (or let `evaluate_format_points_button` check brushed/selected fallback consistency), and add `analyze.factorial.residual_covariance` gating (`r1_sphericity_or_reml_control(analyze.factorial_repeated_measures != RepeatedMeasuresDesign::None)`) to `evaluate_analyze_widget`.

---

### FA-SEC13-13

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/inspect.rs:2343-2346`
- **Category:** `wrong-result`
- **Defect:** In `build_generic_dialog_nodes`, `ActiveDialog::DuplicateAnalysisPrompt` emits `"duplicate.choice"` with a hardcoded `json!("create_new")` value instead of reading `app.view_shards.analyze_picker.duplicate_choice`.
- **Failure scenario:** If a user or automation toggles `"duplicate.choice"` to `"modify_existing"` while `DuplicateAnalysisPrompt` is open, subsequent `ui.inspect` snapshots continue to report `"value": "create_new"`.
- **Evidence:**
```rust
// crates/ui-egui/src/inspect.rs:2342-2346
    let (title, target) = match dialog {
        ActiveDialog::DuplicateAnalysisPrompt { source_sheet_id, .. } => {
            controls.push(node("duplicate.choice", "combobox", "Duplicate handling", json!("create_new"), ControlEnablement::enabled(), pos(0)));
            (insp_tr(app, "w6.view.modal.analysis_already_exists", "Analysis Already Exists"), Some(source_sheet_id))
        }
```
  By contrast, `gui_actions.rs:2753-2778` reads and resets `app.view_shards.analyze_picker.duplicate_choice`.
- **Suggested fix:** Emit `json!("modify_existing")` when `matches!(app.view_shards.analyze_picker.duplicate_choice, crate::views::analyze_picker::DuplicateChoice::ModifyExisting)` and `json!("create_new")` otherwise.

---

### FA-SEC13-14

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/gui_actions.rs:1488-1563` (`apply_type_text_to_x_cell`)
- **Category:** `data-loss` (`contract-drift`)
- **Defect:** When multi-column tab/newline text typed or pasted onto an X cell spills past `avail_x_cols` into Y columns (`has_y_cols == true`), `apply_type_text_to_x_cell` commits Y columns first via `app.execute_command("table.paste", ...)` and then executes `app.execute_command("table.setXCells", ...)` as a second independent command.
- **Failure scenario:** Pressing `Cmd/Ctrl+Z` once after a multi-column X+Y paste reverts only X cells while leaving Y cells pasted; worse, if `table.setXCells` fails (for instance on out-of-range row/cell validation or conflict), `table.paste` has already mutated the document, leaving half the paste applied.
- **Evidence:**
```rust
// crates/ui-egui/src/gui_actions.rs:1517-1551
        if has_y_cols {
            let dec = if preset.decimal_separator == ',' { "," } else { "." };
            let _ = app.execute_command(
                "table.paste",
                json!({
                    "sheet_id": sheet_id.as_str(),
                    "text": y_lines.join("\n"),
                    "start_row": x_row,
                    "start_col": 0,
                    "start_subcol": 0,
                    "delimiter": "\t",
                    "decimal_separator": dec,
                }),
            )?;
        }

        let out = if x_edits.is_empty() {
            ...
        } else {
            app.execute_command(
                "table.setXCells",
                json!({
                    "sheet_id": sheet_id.as_str(),
                    "edits": x_edits,
                }),
            )?
        };
```
- **Suggested fix:** Execute multi-column X+Y paste through a single engine command (or support X+Y target offset in `table.paste`) so X and Y writes share one undo entry and one atomic rollback boundary.
