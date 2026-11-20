# R15 egui desktop GUI audit — addendum (2026-11-20)

Follow-on findings for the [R15 egui GUI audit](r15-gui-audit-2026-11-20.md). They came from a GUI reachability review that asked whether every action can be done with the mouse or only from the keyboard. Finding IDs continue the R15-GUI series.

This is a separate file because `feature/R15.6-r15-r16-impl` rewrites the main ledger's findings table and session log. Adding rows there would conflict when that branch merges. These findings can be folded into the main ledger after it lands.

## Run metadata

| Field | Value |
|---|---|
| Checked commits | `1b54e3748d5ff3de9b08485dd91bd46e01dd5983` (`audit/R15-gui-audit-ledger`). Every finding was re-checked at `91fcd0b` (`feature/R15.6-r15-r16-impl`), the newest R15/R16 implementation tip, and still applies there. |
| Scope | Menu model (`crates/ui-egui/src/menus/tooltips.rs`), in-window menu bar (`egui_shell/top_chrome.rs`), desktop command routing (`apps/curvebook/src/desktop_host.rs`), command palette, data-grid context menu and key handlers, shell key handlers (`inspect_bridge.rs`, `leading_dock.rs`, modal key handling). |
| Method | Static trace. One engine probe through MCP stdio (`curvebook-cli` 0.1.0-alpha.2). Nothing was exercised in a live window, so per [`pitfalls.md`](../pitfalls.md) §6 none of this is human-path evidence. |
| Mode | Report only. No implementation code changed. |
| Agent | Claude Code; Claude Opus 5.5 (`claude-opus-5-5`) |
| Slash command | none |

Line numbers below refer to `91fcd0b`. Where the file was split or moved, the `1b54e37` location is given too.

## Findings

| ID | Severity | Location (`91fcd0b`) | Summary | Status |
|---|---|---|---|---|
| R15-GUI-29 | Medium | `crates/ui-egui/src/egui_shell/data_grid/multi_archetype.rs:921`, `:928` | Grid row context menu items "Show Row Titles on Graph" and "Reset to Dataset Style" send `graph.format`, which is not a registered engine command | open |
| R15-GUI-30 | Medium | `crates/ui-egui/src/egui_shell/top_chrome.rs:865–883` | In-window menu bar runs every engine command it does not hand to the host with `{}`, so parameterised menu items fail | open |
| R15-GUI-31 | Medium | `apps/curvebook/src/desktop_host.rs:3535`, `crates/ui-egui/src/egui_shell/command_palette.rs:733–740` | Restore Checkpoint and Delete Checkpoint have no working GUI path | open |
| R15-GUI-32 | Low | `crates/ui-egui/src/egui_shell/data_grid/shortcuts.rs:72`, `:339`; `crates/ui-egui/src/egui_shell/inspect_bridge.rs:1268` | Grid Cut and copying a graph or result sheet to the clipboard are keyboard-only | open |
| R15-GUI-33 | Medium | `crates/ui-egui/src/egui_shell/app_modals.rs:1736`, `:1759`, `:1681–1683`; `crates/ui-egui/src/egui_shell/inspect_bridge.rs:1357`, `:1379` | About CurveBook cannot be closed: OK and Cancel queue unhandled `dialog.ok` / `dialog.cancel`, and Escape and backdrop clicks are ignored. Its body shows internal details and no licence information | open |
| R15-GUI-34 | Medium | `crates/ui-egui/src/menus/tooltips.rs` (`menu_bar_tooltips`), `apps/curvebook/src/mac_menu.rs:408–433`, `crates/ui-egui/src/egui_shell/top_chrome.rs:744` | Menus list every registered command flat: File has 32 items and Analyze 44, with duplicate and overlapping entries (13 exports, 6 imports, 2 × New Project) and internal or low-level commands | open |
| R15-GUI-35 | Low | `crates/ui-egui/src/menus/ui_commands.rs:226` (`ui.presentation.toggle`), `crates/ui-egui/src/egui_shell/top_chrome.rs:341`, `top_bar_fit.rs:81–83` | Maintainer decision: remove Presentation Mode. It is not needed and adds a toolbar button, a menu item, two shortcuts and a parallel chrome state | open |
| R15-GUI-36 | Medium | `crates/ui-egui/src/egui_shell/leading_dock.rs:252`, `crates/ui-egui/src/egui_shell/trailing_studio/mod.rs:315–317` | Project Navigator and Format Inspector cannot be resized by dragging: both panels use `exact_size(w).resizable(false)`, although the layout spec calls for draggable dividers | open |
| R15-GUI-37 | Medium | `crates/ui-egui/src/egui_shell/mod.rs:810–891` (`hover_bg`, `button_bg`), `crates/ui-egui/src/egui_shell/widgets.rs:395` | Hover highlight is too faint: hover fill contrast against the resting fill is 1.02:1 to 1.13:1 for buttons and 1.12:1 to 1.39:1 for menu titles and navigator rows, far below the 3:1 non-text contrast target | open |
| R15-GUI-38 | Medium | `crates/ui-egui/src/egui_shell/leading_dock.rs:946–1100` (`render_navigator_row`) | Project Navigator has no right-click menu, although the layout spec lists navigator items among the context-menu surfaces | open |
| R15-GUI-39 | Low | `crates/ui-egui/src/egui_shell/top_chrome.rs:898–899`, `crates/ui-egui/src/i18n/w5_toolbar_menus.rs:426–427` | The command-palette button in the top-right bar is labelled with the literal shortcut "Cmd+K": in English the full-width form reads "Cmd+K (Cmd+K)", and it ignores Ctrl on Windows/Linux and rebinding | open |
| R15-GUI-40 | Medium | `crates/ui-egui/src/egui_shell/canvas_view.rs:811`, `:1173–1230`; `crates/chart/src/xy.rs:1044` | A model equation added from Annot › "+ Add Model Equation…" appears twice on the graph canvas: the rendered graph image already contains it, and the canvas paints a second, offset copy on top | open |

### Finding details

#### R15-GUI-29 — Grid context menu sends an unregistered command (Medium)

The data-grid row context menu lists "Show Row Titles on Graph" and "Reset to Dataset Style" (`multi_archetype.rs:884–885`). Their handlers queue `push_engine("graph.format", …)` (`:921`, `:928`; at `1b54e37`, `data_grid.rs:2772–2780`). No `graph.format` command exists in `cmd::registry()` (`git grep 'graph\.format"' 91fcd0b -- crates/engine docs/commands.md` finds nothing).

- **Probe:** MCP `command_run` with `graph.format` and `{"sheet_id": …, "reset_row_override": 0}` returned `not_found`: "unknown engine command `graph.format`".
- **Scenario:** right-click a row in a data table that has a graph, then choose either item. The intent fails, so nothing changes on the graph. Under the intent-pipeline behaviour described in R15-GUI-02, the error also affects the rest of that frame's batch.
- **Fix direction:** route these to `graph.setSpec` / `graph.formatSelectedPoints` with the parameters they need, or drop the items. Add a test asserting that every `push_engine` literal in `egui_shell/` names a registered command.

#### R15-GUI-30 — In-window menu bar sends empty parameters (Medium)

`menu_bar_tooltips` builds menu items for every engine and UI command (`menus/tooltips.rs`). When an in-window menu item is clicked, `push_menu_command_intent` sends `push_engine(command_id, {})` for any engine command that is not host-routed (`top_chrome.rs:878–882`; at `1b54e37`, `:806–810`). The native macOS menu does not have this problem: it goes through `resolve_desktop_command_route` (`desktop_host.rs`), which builds parameters from the active sheet and selection. So the two menu backends behave differently for the same item.

- **Scenario:** with the in-window menu bar shown (any non-macOS build, or macOS with `menu_bar_visible`), choose an item such as Set Column Titles or Export Table CSV / TSV. The command runs with `{}` and fails on its missing `sheet_id` or other required parameter.
- **Probe:** MCP `command_run` with `{}` returned `bad_input` "missing required field `sheet_id`" for both `table.setTitles` and `table.exportCsv`.
- **Relation:** this is the same failure class as R15-GUI-18. The R15-GUI-18 fix (`22b3b38`) gave the command palette `resolve_palette_route`; the in-window menu got no equivalent.
- **Fix direction:** route in-window menu clicks through one shared resolver (the desktop route or the palette route), so all three entry points build the same parameters.

#### R15-GUI-31 — Checkpoint restore and delete are unreachable (Medium)

The desktop router sends `project.restoreCheckpoint` and `project.deleteCheckpoint` to the command palette, opened with the command ID as its query (`desktop_host.rs:3535–3539`). The palette lists both in `is_palette_hidden_engine_command` (`command_palette.rs:733–740`), so the query matches nothing. No other widget in `egui_shell/` queues either command, and there is no checkpoint list view. Create Checkpoint works (`command_palette.rs:705` and the desktop route).

- **Scenario:** File › Checkpoints › Restore Checkpoint… opens an empty palette, so a saved checkpoint cannot be restored or deleted through the GUI.
- **Before the R15-GUI-18 fix** (`1b54e37`), the palette ran these commands with `{}` and failed on the missing checkpoint name. They were unreachable then too.
- **Fix direction:** a small checkpoint picker (list, Restore, Delete with confirmation) that queues the command with the selected checkpoint name.

#### R15-GUI-32 — Keyboard-only clipboard actions (Low)

- **Grid Cut:** `Cmd/Ctrl+X` is handled in `data_grid/shortcuts.rs:72` and `:339`. The grid context menu offers Copy and Paste but no Cut (`multi_archetype.rs:883–889`). Neither the menu model nor the native macOS menu has an Edit › Cut/Copy/Paste item.
- **Copy graph or result to clipboard:** the image copy (`push_workspace_copy`) is queued only from the `Cmd/Ctrl+C` key handler in `inspect_bridge.rs:1264–1268`. No button or menu item queues it.
- Other shortcuts handled directly in code have mouse equivalents: `F2` rename has double-click in the navigator (`leading_dock.rs`) and the grid sheet title; `Ctrl+Tab` / `Ctrl+PgUp` in modals and results have clickable tabs; `F6` pane cycling has pane clicks; presentation `Esc` has the exit button.
- **Fix direction:** add Cut to the grid context menu, an Edit › Cut/Copy/Paste group routed to the existing host clipboard actions, and a "Copy as Image" item for graph and result sheets.

#### R15-GUI-33 — About CurveBook cannot be closed and shows the wrong information (Medium)

Reported by the maintainer from the desktop app; root cause traced at `1b54e37` and `91fcd0b`.

- **Cannot be closed.** The About modal's OK and Cancel buttons call `intents.push_ui("dialog.ok", {})` and `push_ui("dialog.cancel", {})` (`app_modals.rs:1736`, `:1759`; at `1b54e37`, `:1911`, `:1934`). Neither ID is a UI command. `dispatch_ui_command_detailed` ends with `_ => Ok(None)` (`control_io.rs`, after the `app.about` arm at `:1304`), so both clicks are silently ignored and `active_dialog` stays `About`. The other app modals close through `push_dialog_button("ok" / "cancel")` (for example Print and Export), which reaches `execute_dialog_button_action`. The Escape handler in `interaction.rs` dismisses popups, Welcome and the Gallery but not an active dialog, and the About modal never reads `ModalResponse::should_close()`, so Escape and backdrop clicks do nothing either. The modal backdrop blocks the workspace, so the user cannot continue working until they quit CurveBook.
- **Second copy.** `inspect_bridge.rs:1321` has its own `render_about_modal` with the same `dialog.ok` / `dialog.cancel` intents (`:1357`, `:1379`).
- **No test covers closing it.** The About tests found open the dialog (`r15_1_host_inspect_raster.rs`) and never assert that it closes.
- **Content.** The body shows the active theme and UI scale, "Active Beta locales supported: N (RTL locales excluded at Beta)" and "Registered keyboard shortcuts: N" (`app_modals.rs:1681–1683`). These are internal or settings details, not product information, and the counts are live values that read as release-planning notes. The dialog shows no licence, copyright or contributor credit, and gives no way to view `NOTICE` or the third-party licences, which is the information an About box normally carries.
- **Fix direction:** close through `push_dialog_button` (or clear `active_dialog` directly) from OK, from Escape via `should_close()`, and from a backdrop click. Delete the duplicate in `inspect_bridge.rs` or route it through the same helper. Show the version, build SHA, a one-line description, licence and copyright, and links to the notices and third-party licences; keep the theme, locale and OS details only in the "Copy version info" text. Add a human-path test that opens About and closes it with OK, Escape and a backdrop click.

#### R15-GUI-34 — Menus are too long, flat and full of duplicates (Medium)

Reported by the maintainer from the desktop app; counts and entries taken at `3aed050` (`feature/R15.6-r15-r16-impl`), and the structure is the same at `1b54e37`.

- **Cause.** `menu_bar_tooltips` adds every command in `cmd::registry()` and every `UI_COMMANDS` entry to a top-level menu, bucketed only by the first word of its `menu_path`. UI commands without a mapping in `ui_command_menu_info` fall into View. Both the native macOS menu (`mac_menu.rs:408–433`) and the in-window menu (`top_chrome.rs:744`) append the items flat: the second level of the path (`File/Export/…`, `File/Import/…`, `File/Checkpoints/…`, `Analyze/Column Analyses/…`) is ignored, and there are no separators or groups. The engine `menu_path` was written as documentation metadata and is being used as the menu definition.
- **Length.** File 32 items, Edit 9, Insert 12, Analyze 44, Graph 27, View 18, Help 1.
- **Export and Import.** File has 13 export entries: Export PDF, PNG, SVG and TIFF (`export.*`); Export Graph PDF, PNG and SVG (`graph.export*`), which do the same for one graph; Export Graph EMF and Export Layout EMF; Export Table CSV / TSV; Export Result Sheet CSV and JSON; and Export Figure… (the dialog that already covers figure formats). It also has 6 import entries: CSV / TSV, Plate Map, External Archive, XML Project, Tidy Long Table, and "Open Plate / Tidy Long Import Reshape". The four checkpoint commands are listed flat beside them.
- **Duplicate pairs** (engine command plus the UI command that opens its dialog, or two commands for one action):
  - File: New Project (`file.new`) / New Project (`project.new`).
  - Insert: Simulate Distributions / Simulate Distributions Dialog….
  - Insert and Analyze: Randomize Subjects to Groups Dialog… / Randomize Subjects to Groups; Random Sample Rows Dialog… / Random Sample Rows; Convert Table Archetype… / Convert Table Archetype.
  - Analyze: Change Analysis Parameters / Change Analysis Parameters…; Set Column Titles / Set Group Column Titles. Analysis can be started from eight overlapping entries: Open Analyze Dialog, New / Run Analysis, Add Analysis, Run Statistical Test, Run Curve Fit / Regression, Run Transformation, Run Analyses and Recalculate All.
  - Graph: Format Axes / Format Axes…; Format Selected Points / Format Selected Points…; Apply Graph Style From… / Apply Style From…; Add Pairwise Significance Brackets / Pairwise Comparison Brackets…; Update Graph Specification / Format Graph…; and Apply Graph Canvas Preset, Apply Journal / Presentation Canvas Preset and Apply Layout Canvas Preset.
  - Preflight appears in two menus: Preflight Check Figure (Analyze) and Run Publication Preflight Check (Graph).
- **Entries that are not user menu actions:** grid-level edits (Set Cell Value, Set Multiple Cells, Set X / Time Column, Set Row Titles, Set Subcolumn Titles), Select Sheet, Inspect Project, Initialize Graph, Brush Data Cell / Plotted Point, Capture Workspace Screenshot PNG, Reorder Sheet, and the always-disabled "Diagnostic Remediation Disabled (QOL-7 Diagnostics-Only)", whose label exposes an internal roadmap ID. Several labels also carry implementation words ("… Dialog…", "Archetype", "Open … Wizard"). Many of the raw engine entries also fail from the in-window menu (R15-GUI-30).
- **Fix direction:**
  - Define the menus in a curated, declarative menu table instead of deriving them from the registries: one entry per user action, with explicit submenus and separator groups.
  - Use one Import… entry, or one File › Import submenu, that picks the reader from the file type.
  - Use one Export… entry, or one File › Export submenu, built on the existing Export Figure dialog, with format and scope (graph, layout, table, result, methods report) chosen in the dialog.
  - Put the checkpoint actions under one File › Checkpoints submenu or a checkpoint manager (see R15-GUI-31).
  - Where a dialog exists, list only the dialog-opening command. Keep low-level and agent-only commands in the command palette and MCP, out of the menus.
  - Add a test that fails when a top-level menu exceeds an agreed length, when two items share a label or action, or when a label contains a roadmap or QOL ID.

#### R15-GUI-35 — Remove Presentation Mode (Low, product decision)

Maintainer decision, 2026-11-20: CurveBook does not need a "Present" mode, and it should be removed. This is not a defect report; it records the decision and the removal scope traced at `3aed050`. The mode came from the deferred `QOL-10` read-only viewer idea (`docs/planning/qol-enhancements.md:137`), whose further work `docs/STATUS.md:50` and `docs/ROADMAP.md:170` already list as deferred.

- **What it adds:** the `ui.presentation.toggle` UI command ("Toggle Presentation Mode", `Cmd+Shift+P`, `ui_commands.rs:226`), which also appears in the View menu (R15-GUI-34); a toolbar toggle that the top-bar width solver reserves room for (`top_bar_fit.rs:51`, `:81–83`); a floating exit chip (`top_chrome.rs:341`, `toolbar.presentation.exit`); `F11` / `Cmd+Shift+P` / `Escape` handling in the shell pass loop (`inspect_bridge.rs:1278–1285`); and `UiApp::presentation_mode` checks across `lib.rs`, `egui_shell/mod.rs`, `center_stage.rs`, `canvas_view.rs`, `control_io.rs` and `gui_window.rs`.
- **Web host:** `apps/curvebook-web/src/chrome_bridge.rs:189–195` (toolbar entry and `Alt+Shift+P` browser mapping at `:481`) and `host_adapters.rs:1696` (`SetPresentationMode`).
- **Generated and gated outputs:** `crates/mcp/src/ui_catalog.rs` (regenerate with `cargo xtask ui-commands`), the live UI command count in `docs/STATUS.md` (checked by `cargo xtask status`), and the i18n label in `crates/ui-egui/src/i18n/core_cmd_ui.rs`.
- **Tests that reference it:** `crates/ui-egui/tests/{audit_r2_s10,interaction_hidpi_qol,r15_3_stage_canvas,r15_3_top_chrome}.rs`.
- **Keep:** the "Presentation Slide 16:9" canvas preset (`top_chrome.rs:39`, a figure size) and the pairwise-significance "presentation mode" in `crates/doc/src/graph.rs:863`. Both are unrelated and share only the word.
- **Compatibility:** MCP or control clients that call `ui.presentation.toggle` will get an unknown-command error after removal; record that in the changelog. Saved shortcut overrides naming the command load safely, because `import_overrides_json` keeps unknown IDs (`menus/shortcuts.rs`). Freeing `F11` and `Cmd+Shift+P` needs no migration.
- **Removal direction:** delete the command, state, toolbar toggle, exit chip and key handling in one change (desktop and web), drop or rewrite the tests that exercise it, regenerate `ui_catalog.rs`, and update `STATUS.md` and the `QOL-10` notes.

#### R15-GUI-36 — Side panes cannot be resized by dragging (Medium)

Reported by the maintainer from the desktop app; traced at `3aed050`.

- **Spec:** `docs/planning/gui-layout-spec.md:57` shows the Navigator (240 pt) and Inspector (280 pt) as "(resizable)", and `:99` lists `SplitHandle` dividers for `Navigator/Center` and `Center/Inspector` alongside the center split.
- **Code:** the left dock is `egui::Panel::left("egui_shell.leading_dock").exact_size(w).resizable(false)` (`leading_dock.rs:252`), and the right studio is `egui::Panel::right("egui_shell.trailing_studio").exact_size(w).resizable(false)` (`trailing_studio/mod.rs:315–317`). Neither has a divider to drag.
- **Half-built support:** `UiIntent::SyncLeadingWidth` / `SyncTrailingWidth` already store a width clamped to `navigator_min_w`–`navigator_max_w` (180–380 pt) and `inspector_min_w`–`inspector_max_w` (220–420 pt) (`egui_shell/mod.rs:1897–1904`, limits at `:364–369`), and the layout reads that width (`:526`, `:531`). But `push_sync_leading_width` / `push_sync_trailing_width` (`:1349`, `:1354`) have no callers, although their doc comments read "Persist dragged Leading Dock width" and "Persist dragged Trailing Studio width". Today only a workspace preset changes the widths (`:1894–1895`).
- **Working pattern to copy:** the center split divider (`center_stage.rs:552–580`) uses a hit rect wider than the visible line, `Role::Splitter` for accessibility, the `ResizeHorizontal` cursor, a clamped drag and double-click to reset.
- **Evidence docs:** no `[x]` claim in `docs/evidence/user-interface.md` covers side-pane resizing, so no evidence row needs to be withdrawn.
- **Fix direction:** add a splitter on the inner edge of each dock, built the same way as the center divider. Have it push `SyncLeadingWidth` / `SyncTrailingWidth` on drag, reset to the preset width on double-click, hide it while a dock is collapsed to its rail, and keep the center stage at or above its protected minimum width. Persist the widths per workspace preset, or with the window layout, if they should survive a restart (not checked here). Add a human-path test that drags each divider and asserts the clamped width.

#### R15-GUI-37 — Hover highlight is too faint (Medium, accessibility)

Reported by the maintainer from the desktop app; colours traced at `bd0d904` (`feature/R15.6-r15-r16-impl`).

- **Tokens** (`egui_shell/mod.rs`, Light / Dark / High Contrast): resting button fill `button_bg` (232,236,242) / (43,49,60) / (24,28,36); hover fill `hover_bg` (228,234,244) / (45,52,66) / (32,38,52) (`:810`, `:815`, `:848`, `:853`, `:886`, `:891`). Panel backgrounds come from `theme.rs`: (245,246,248) / (24,27,33) / (0,0,0).
- **Measured contrast** (WCAG 2.x relative luminance):

  | Change on hover | Light | Dark | High Contrast |
  |---|---:|---|---:|
  | Button `button_bg` → `hover_bg` | 1.02:1 | 1.05:1 | 1.13:1 |
  | Menu title or navigator row, panel → `hover_bg` | 1.12:1 | 1.38:1 | 1.39:1 |

- **Where only the fill changes:** the secondary dialog button paints `hover_bg` instead of `button_bg` and nothing else (`widgets.rs:395`). Menu-bar titles (`top_chrome.rs:605`) and a top-bar chrome button (`:366`) go from transparent or panel colour to `hover_bg`. Navigator rows use the same fill (`leading_dock.rs`). Standard egui widgets also get a 1 px `focus_ring` border on hover (`to_visuals`, `mod.rs:943–945`), which is the only clearly visible cue, and the custom-painted buttons do not use it.
- **Target:** WCAG 2.1 SC 1.4.11 (Non-text Contrast) asks for 3:1 for visual information that identifies a component's state. Hover on its own is not strictly required to meet it, but a hover cue at about 1.0–1.4:1 is effectively invisible on typical displays, which matches the report.
- **Fix direction:** make hover a clearly distinct state in every theme: a darker or tinted fill (for example an accent tint at about 12–16 % alpha) plus a border or outline change, applied in one shared helper used by every custom button, menu title and list row. Add a token test that asserts a minimum rest-to-hover contrast for each theme, so later palette edits cannot regress it.

#### R15-GUI-38 — Project Navigator has no right-click menu (Medium)

Reported by the maintainer from the desktop app; traced at `bd0d904`.

- **Spec:** `docs/planning/gui-layout-spec.md:104` lists "navigator items" among the right-click context-menu surfaces (`ZLayer` `70 ContextMenu`), together with grid cells, column headers, graph marks and layout panels.
- **Code:** `render_navigator_row` (`leading_dock.rs:946`) handles drag to reorder (`:1070`), double-click to rename (`:1073`) and click to select. It never checks `secondary_clicked()` and opens no menu. Every `secondary_clicked()` in `egui_shell/` is in the data grid, the ANOVA modal or modal backdrop handling.
- **Effect:** sheet actions are reachable only through the top menus, which act on the *active* sheet and are long and flat (R15-GUI-34). Right-clicking a sheet that is not active does nothing, so there is no direct way to act on it.
- **Expected actions,** all backed by existing commands: Rename (`sheet.rename`, inline editor already exists), Duplicate and Duplicate Family (`sheet.duplicate`, `sheet.duplicateFamily`), Delete (`sheet.delete`, with its cascade confirmation), Freeze / Unfreeze (`sheet.freeze`, `sheet.unfreeze`); for data sheets, Analyze… and New Graph (`ui.analyze.open`, `graph.create`); for results, Change Analysis Parameters… and Change Data Analyzed (`ui.editParams.open`, `analysis.retargetData`); for graphs and layouts, Export… (R15-GUI-34); plus Copy Methods for results.
- **Fix direction:** a navigator context menu that targets the right-clicked row's `sheet_id` (not the active sheet), shows only the actions valid for that sheet type, disables (rather than hides) actions whose `enabled` predicate fails, and reuses the grid context menu's popup code. Add a human-path test that right-clicks a non-active sheet and renames, duplicates and deletes it.

#### R15-GUI-39 — Command-palette button is labelled "Cmd+K" (Low)

Reported by the maintainer from the desktop app; traced at `bd0d904` (same code at `1b54e37`, `top_chrome.rs:853–854`).

- **Code:** `render_tier1_right_group` labels the `toolbar.command_palette` button `format!("{} (Cmd+K)", tr(locale, "toolbar.palette"))` when the bar has room (`SearchVisibility::Field`) and the hardcoded `String::from("Cmd+K")` when it is narrow (`Icon`) (`top_chrome.rs:898–899`).
- **English string:** the `toolbar.palette` entry is `en: "Cmd+K"`, while every other locale has a word ("Befehle", "Commandes", "Comandos", "コマンド", …) (`i18n/w5_toolbar_menus.rs:426–427`). So the English full-width button reads "Cmd+K (Cmd+K)", and the narrow button reads "Cmd+K" in every locale.
- **Contradicts its own spec:** `SearchVisibility` documents the full form as "Search commands, analyses & sheets (Cmd+K)" and the narrow form as a "magnifier icon button alone" (`top_bar_fit.rs:14–22`).
- **Wrong on other platforms and after rebinding:** the text is always "Cmd", although the shortcut is Ctrl+K on Windows and Linux, and `ui.palette.open` can be rebound (`ui.shortcut.set`), so the label can name a key that no longer opens the palette. The current binding is available from `app.shortcuts.active_combo("ui.palette.open")`.
- **Fix direction:** give the button a real name ("Search commands" or "Commands"), set `en` accordingly, and show a magnifier icon in the narrow form. Show the shortcut, if at all, as a muted hint or tooltip built from the active binding and the platform's modifier name. Add a test that the button label contains no literal modifier names in any locale.

#### R15-GUI-40 — Model equation is drawn twice on the canvas (Medium)

Reported by the maintainer from the desktop app (Graph inspector › Annot › "+ Add Model Equation…"); root cause traced at `bd0d904`, with an engine and export probe through MCP.

- **The document holds one annotation.** The button opens the Insert Model Equation dialog (`trailing_studio/graph_inspector.rs:657–668`). OK sends one `dialog.button ok` (`graph_modals/mod.rs`, default arm of `commit_graph_modal_action`), which runs `commit_intent` once (`gui_actions.rs:2490–2493`) and emits a single `graph.addAnnotation` (`kind: "equation"`). Through MCP, `graph.addAnnotation` on a fitted XY graph returned `total_annotations: 1`.
- **The rendered image contains it once.** The chart compiler typesets every `ModelEquation` annotation into the `ChartScene` (`crates/chart/src/xy.rs:1044`; also `scales.rs:3487` and `survival_cat.rs:904`, through `equation_box::render_model_equation_annotation`). `export.svg` of the same graph contained the bordered equation box exactly once.
- **The canvas adds a second copy.** After compositing the scene raster, `render_graph_hud_overlays` (`canvas_view.rs:719`) calls `render_live_tex_equations` (`canvas_view.rs:811`). That function loops over the same `ModelEquation` annotations and paints each one again with `paint_equation_segments`, offset 4 pt from the annotation bounds and scaled differently, plus its own border when `bordered` is set (`:1173–1230`). So every model equation shows twice on screen, while SVG, PDF and PNG exports show it once. The second copy also differs from the exported figure in position and typesetting.
- **Not covered by tests:** the R9.16 / WF-49 checks look at the TeX string and the exported formats, not at whether the canvas draws the scene equation and the overlay at the same time.
- **Fix direction:** draw the equation in one place. Either keep the scene rendering and change the overlay to an invisible hit and accessibility target (no painting), or skip `ModelEquation` annotations when compiling the on-screen scene and keep the overlay. The first keeps screen and export identical. Add a canvas test that, with one `ModelEquation` annotation, the equation text is painted once.

## Summary

| Severity | Count | IDs |
|---|---:|---|
| Medium | 9 | R15-GUI-29 to 31, 33, 34, 36 to 38, 40 |
| Low | 3 | R15-GUI-32, 35, 39 |

## Session log

- 2026-11-20 — Reachability review: menu model, in-window and native menu dispatch, desktop routing, palette routing, grid context menu, shell key handlers. Findings R15-GUI-29 to R15-GUI-32, re-checked at `91fcd0b`.
- 2026-11-20 — About dialog: maintainer report traced to unhandled close intents and reviewed for content. Finding R15-GUI-33, checked at `1b54e37` and `91fcd0b`.
- 2026-11-20 — Menu structure: maintainer report of overlong, cluttered menus; per-menu items and duplicates enumerated at `3aed050`. Finding R15-GUI-34.
- 2026-11-20 — Maintainer decision to remove Presentation Mode; removal scope traced at `3aed050`. Finding R15-GUI-35.
- 2026-11-20 — Side-pane resizing: maintainer report traced to fixed-width, non-resizable dock panels and unused width-sync intents at `3aed050`. Finding R15-GUI-36.
- 2026-11-20 — Hover contrast measured from theme tokens, and navigator right-click handling traced, at `bd0d904`. Findings R15-GUI-37 and R15-GUI-38.
- 2026-11-20 — Command-palette button label traced at `bd0d904` and `1b54e37`. Finding R15-GUI-39.
- 2026-11-20 — Duplicate model equation: maintainer report traced to scene rendering plus the canvas TeX overlay; engine annotation count and SVG export checked through MCP. Finding R15-GUI-40.
