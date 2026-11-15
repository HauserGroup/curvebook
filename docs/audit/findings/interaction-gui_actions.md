# `crates/ui-egui/src/interaction.rs` × `gui_actions.rs`: verified findings

- **Base:** fda1cc0106092834d97b53289ec2a99c1bf0439c (`crates/ui-egui/src` matches HEAD; both files were last changed in 4dc7b4b)
- **Input:** an external audit report titled "interaction.rs × gui_actions.rs — Duplicated Handling, Bugs & Unintended Behaviour", with items 1–12. Each item was checked against the code at the base above. The original numbers are kept.
- **Out-of-file context read:** `crates/ui-egui/src/lib.rs:435`–`445` (`UiApp::handle_key_combo`), `crates/ui-egui/src/menus/shortcuts.rs:225`–`245` (`Shortcuts::handle_key_combo`), `crates/ui-egui/src/grid_view.rs:2003` (`parse_localized_number_input`), `crates/engine/src/cmd/table.rs:780`–`824`, `:1375` (`table.setCell` cell parsing), `crates/doc/src/cell.rs:114`–`140` (`Cell::parse_import_token`), and `apps/lumencraft/src/desktop_host.rs:1250`–`1295` (native key path).
- **Not run:** no tests and no reproductions. Every verdict comes from reading the code.

Two entry points matter throughout:

- **MCP/control path:** `gui_actions::dispatch_gui_action`, for example `ui.press_key` → `handle_press_key`.
- **Native desktop path:** `desktop_host.rs` → `interaction::dispatch_ui_event` directly. It never goes through `gui_actions`.

Several of the real problems below are differences between these two paths, not duplication inside one of them.

## Verdict summary

| # | Original severity | Verdict | Corrected severity | Note |
|---|---|---|---|---|
| 1 | 🔴 Bug | **Incorrect as stated, with a different real bug** | Med | Shortcuts never run twice. The real bug: falling through to `interaction` defeats the "no shortcuts while a dialog is open" guard |
| 2 | 🔴 Bug | **Invalid** | Info | `KeyDown` handling never reads `metrics`, and `sync_views()` takes none |
| 3 | 🟠 Semantic dup | **Partly confirmed** | Low–Med | The divergence that matters is MCP vs native (Enter in a dialog, formula-bar keys) |
| 4 | 🟠 Semantic dup | **Incorrect as stated** | Low | The fallback applies only when no target was set. The real issues are a stale target and an MCP-only label |
| 5 | 🟠 Semantic dup | **Partly confirmed** | Med | Locale bug confirmed, and it also hits paste and X cells. The empty-string and asterisk rows are wrong |
| 6 | 🟡 Smell | Confirmed, no current defect | Info | |
| 7 | 🟡 Smell | Confirmed reading, feature gap | Info | Additive Ctrl+drag isn't implemented anywhere |
| 8 | 🟡 Smell | **Scenario unrealistic, nearby defect real** | Low | See NEW-1 |
| 9 | 🟡 Smell | **Invalid** | — | Or-patterns don't weaken exhaustiveness checking, and the match has no wildcard arm |
| 10 | 🟡 Smell | Confirmed | Info | The node found at L194 already carries `command_id` |
| 11 | 🔵 Minor | Confirmed | Info | |
| 12 | 🔵 Minor | **Invalid** | — | Two owned strings are needed either way |
| NEW-1 | — | New | Med | An active inline edit is never committed or cancelled when the user clicks elsewhere or starts another edit |

---

## Confirmed findings

### 1 · Med · Via `ui.press_key`, shortcuts still run while a dialog is open, because the fall-through to `interaction` skips the dialog guard

- **Location:** `crates/ui-egui/src/gui_actions.rs:1037`–`1045`, `crates/ui-egui/src/interaction.rs:1580`–`1595`
- **Category:** contract-drift
- **Defect:** `handle_press_key` deliberately skips `app.handle_key_combo` while a dialog is open (`active_dialog == ActiveDialog::None` at `:1037`). It then forwards the key to `dispatch_ui_event(KeyDown)`. There, `handle_key_down` checks only for Escape when a dialog is open (`:1580`) and then tries every shortcut candidate with no dialog check (`:1585`–`1594`).
- **Failure scenario:** With the Analyze or Power dialog open, call `ui.press_key { key: "s", modifiers: { meta: true } }`. The `gui_actions` guard skips the shortcut, but the fall-through runs the `Cmd+S` binding anyway. The same applies to any binding, including ones that change the document. On the native path (`desktop_host.rs:1281`) `interaction` gets the key directly, so there is no guard at all.
- **Corrections to the original:**
  - **"`handle_key_combo` fires twice" is wrong.** `Shortcuts::handle_key_combo` is a pure `&self` lookup (`shortcuts.rs:225`–`245`). `UiApp::handle_key_combo` executes something only when a binding matches, and `gui_actions` returns as soon as that happens (`:1039`–`1040`). A keystroke that misses the first lookup can match on the second, because `build_shortcut_candidates` also tries `Cmd` in place of `Ctrl`, but it still executes only once.
  - **"`sync_views()` is called with dummy 1024×640 metrics" is wrong.** `UiApp::sync_views()` takes no metrics (`lib.rs:448`). See item 2.
- **Suggested fix:** Decide whether shortcuts may run while a modal dialog is open. Then enforce that rule in one place, `interaction::handle_key_down`, so both the MCP and native paths follow it, and drop the separate guard in `gui_actions`. Add a test that presses a bound combo while a dialog is open, on both paths.

### 3 · Low–Med · Enter, Escape and Tab behave differently on the MCP and native paths

- **Location:** `crates/ui-egui/src/gui_actions.rs:991`–`1035`, `crates/ui-egui/src/interaction.rs:1562`–`1583`
- **Category:** contract-drift
- **Confirmed differences:**

  | Key / state | MCP (`ui.press_key`) | Native (`desktop_host` → `interaction`) |
  |---|---|---|
  | Enter, dialog open | `handle_answer_dialog("ok"/"next"/"create")` | No dialog handling. Tries shortcuts |
  | Enter / Escape / Tab, formula bar active | `commit_formula_bar` / `cancel_formula_bar` / accept completion | `interaction.rs` has no formula-bar handling |
  | Enter, inline editor **and** dialog both active | Answers the dialog. The inline edit stays pending | Commits the inline edit. The dialog stays open |

- **Is the last row reachable?** In `interaction.rs`, yes: no pointer handler checks `active_dialog` (the file's only `active_dialog` check is at `:1580`), so a double-click can start an inline edit while a dialog is open. Whether the native egui host blocks such clicks behind the modal wasn't checked.
- **Not checked:** whether `gui_window.rs` handles formula-bar keys through its own egui widget. If it does, the formula-bar row is only an MCP-vs-`interaction` difference.
- **Suggested fix:** Put one priority chain in `interaction::handle_key_down`: inline editor → formula bar → dialog → shortcuts. Have `handle_press_key` delegate to it, so `--gui-only` MCP sessions behave like the desktop.

### 4 · Low · A right-click on empty space keeps the previous context-menu target, and the MCP path writes a menu label into a field that holds region names

- **Location:** `crates/ui-egui/src/gui_actions.rs:599`–`606`, `crates/ui-egui/src/interaction.rs:1081`–`1119`
- **Category:** contract-drift
- **Correction to the original:** The fallback doesn't overwrite a hit. It runs only when `context_menu_target.is_none()` (`gui_actions.rs:603`). A right-click on the canvas sets `"canvas"` (`interaction.rs:1110`), so the fallback is skipped. The original's claim that a canvas right-click with brushed points "always shows Format Selected Points…" is wrong.
- **What is actually wrong:**
  1. When the hit test misses, `handle_secondary_pointer_down` returns `secondary_click_ignored` without clearing `context_menu_target` (`:1118`). Only a primary pointer-down clears it (`:966`). A right-click on empty space therefore keeps the previous `"grid"` or `"canvas"` target, and the `gui_actions` fallback then can't apply either.
  2. The fallback stores `"Format Selected Points..."`, a menu label, in a field that otherwise holds region names (`"grid"`, `"canvas"`), and only on the MCP path. A native right-click never gets it.
- **Suggested fix:** Set `context_menu_target = None` on a miss in `handle_secondary_pointer_down`. Move the "selection exists, no region hit" case into that function so both paths share it, and give it a region-style value instead of a label.

### 5 · Med · `ui.type_text` ignores the locale's decimal separator for cells, pasted blocks and X cells

- **Location:** `crates/ui-egui/src/gui_actions.rs:778`–`788` (`table.setCell`), `:754`–`764` (`table.paste`), `:829`–`835` and `:864` (X cells), `:899`–`909` (`parse_grid_scalar_value`)
- **Category:** wrong-result
- **Defect:** `parse_grid_scalar_value` uses `str::parse::<f64>`, so `"3,14"` isn't recognised as a number and is sent as the string `"3,14"`. The engine then parses strings with `Cell::parse_import_token` using the request's `decimal_separator`, which defaults to `'.'` (`table.rs:791`). `gui_actions` never passes one, so the value is stored as text. The `table.paste` call for multi-cell input and the X-cell path also leave out `decimal_separator`. The inline editor is locale-aware (`parse_inline_cell_input` → `parse_localized_number_input`, `interaction.rs:301`–`311`), and so is the native paste in `desktop_host.rs:1568`–`1573`.
- **Failure scenario:** With the locale set to `de-DE`, `ui.type_text { text: "3,14" }` on a grid cell stores the text `"3,14"`. Typing the same value through the inline editor stores the number 3.14. The same happens with `"1,5\t2,5"` pasted through `ui.type_text`.
- **Corrections to the original:**
  - **Empty input:** both paths end up with an empty cell. `Value::String("")` reaches `Cell::parse_import_token`, which returns `Cell::empty()` for blank input (`cell.rs:116`–`117`). There's no "empty vs. cleared" difference.
  - **`*` prefix:** both paths send a string, and the engine strips `*` and marks the cell excluded (`cell.rs:120`–`127`, `parse_asterisk_exclusion` defaults to `true`). The `starts_with('*')` branch in `parse_grid_scalar_value` is redundant, because `"*5".parse::<f64>()` fails anyway.
- **Suggested fix:** Pass `decimal_separator` from `app.locale_preset` in all three engine calls (`table.setCell`, `table.paste`, `table.setXCells`) and send the raw string. The engine already parses it, so `parse_grid_scalar_value` can then be removed. Add a `de-DE` test for each of the three calls.

---

## New finding (missed by the original report)

### NEW-1 · Med · An active inline edit is neither committed nor cancelled when the user clicks elsewhere or starts another edit

- **Location:** `crates/ui-egui/src/interaction.rs:1009`–`1030` (single click on a grid cell), `:1059`–`1067` and `:1436`–`1474` (every `inline_editor.begin` site), `:187`–`194` (`InlineEditorState::begin`)
- **Category:** data-loss
- **Defect:** No pointer handler checks `inline_editor.active`. A single click on another grid cell changes the selection but leaves the editor bound to the original cell. Every `begin(...)` call unconditionally overwrites `target`, `buffer` and `original_text` without committing or cancelling the edit in progress. Nothing in `apps/lumencraft/src` or the rest of `crates/ui-egui/src` commits on a click either: the only `CommitInlineEdit`/`commit_inline_edit` callers are in `interaction.rs`.
- **Failure scenarios** (from reading the code, not reproduced):
  - Double-click cell A, type `42`, then single-click cell B. B is now selected, but Enter commits `42` into **A**. `grid.tool.editing_buffer` still holds `42`, so the grid may also draw it at B. That last part wasn't checked.
  - Double-click cell A, type `42`, then double-click cell B, or single-click an Info constant or the Info notes. The `42` is discarded with no commit and no prompt.
- **Suggested fix:** At the start of primary pointer-down and double-click handling, and inside every `begin` site, commit any active inline edit, the way most spreadsheet grids do, or cancel it explicitly. Add regression tests for both scenarios.
- **Related:** item 8. Starting an Info-constant edit on a single click makes the second scenario a one-click action.

---

## Items to drop or downgrade, with reasons

- **2. Invalid.** `dispatch_ui_event` passes `metrics` only to the pointer, zoom and pan handlers. `KeyDown` goes to `handle_key_down(app, &key, modifiers)`, which has no metrics parameter (`interaction.rs:943`, `:1562`), and `sync_views()` takes none. Using `ViewportMetrics::default()` there has no effect. Passing `parse_metrics(params)?` would be tidier but changes nothing.
- **6. Info.** As the original says, `pointer_down` is set and cleared only for the primary button (`:967`, `:1320`), so no state leaks.
- **7. Info.** `PointerMove`'s `modifiers` are discarded (`:935`), and drag-select always calls `extend_shift_range` (`:1236`). Ctrl+drag adding to the selection is a feature that isn't implemented, not a bug. Not checked: whether a Ctrl+click followed by a drag loses the Ctrl-toggled cells when `extend_shift_range` rebuilds the range.
- **8. Low, scenario unrealistic.** A single click on an Info constant does start an edit (`:1059`–`1062`), and a double-click starts it again (`:1448`–`1450`). But both use the committed `value_text`, and the user would have to type between the two clicks of a double-click to lose anything. What remains is a UX inconsistency, since grid cells need a double-click and Info constants and notes need only one. The real data-loss problem is NEW-1.
- **9. Invalid.** `CanvasTitle { sheet_id } | AxisTitle { sheet_id, .. }` is an ordinary or-pattern. `handle_pointer_down`'s match lists every variant and has no `_` arm (`:975`–`1078`), so adding a new `WorkspaceHitTarget` variant still fails to compile until someone handles it. Ignoring `axis` is fine, because the single-click handler doesn't use it.
- **10. Info.** The second `inspect_ui_tree(app, Some("toolbar"), false)` at `:302` rebuilds part of the tree only to look up a widget that `handle_pointer_gesture` already found at `:194`–`197` (`target_node`). The simplest fix is to pass that node, or its `command_id`, into `try_activate_semantic_widget`, rather than calling `tree.find_widget(wid)` again.
- **11. Info.** `_grid_hit_coord_unused` (`:1989`–`1992`) is dead code behind `#[allow(dead_code)]`. Remove it.
- **12. Invalid.** `disabled_err` needs one owned `String` for the error message and one inside the `json!` details. Removing `.clone()` only moves that allocation somewhere else. Not worth changing.

## Suggested fix order

1. NEW-1 (silent loss of an inline edit)
2. 5 (locale-dependent wrong cell types through the GUI automation path)
3. 1 and 3 together, as one shared key-routing chain in `interaction::handle_key_down` used by both paths
4. 4
5. Info items as cleanup
