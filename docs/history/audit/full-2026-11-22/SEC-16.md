# Audit Report — SEC-16: Egui Shell Chrome, Canvas & Docks

- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `crates/ui-egui/src/egui_shell/` (`mod.rs`, `brand.rs`, `canvas_view.rs`, `center_stage.rs`, `command_palette.rs`, `driver.rs`, `fonts.rs`, `inspect_bridge.rs`, `leading_dock.rs`, `rasterizer.rs`, `status_footer.rs`, `top_bar_fit.rs`, `top_chrome.rs`, `widgets.rs`) — 14 files, 21,426 LOC
- **Date:** `2026-11-22`
- **Summary counts:** `S1=0`, `S2=4`, `S3=6`, `S4=2` (Total: `12` findings)

---

## Verified Baseline & Resolved-Ticket Status

- **Architecture & Safety Invariants:** All 14 files preserve headless-first discipline (zero direct mutations to `ProjectDocument`; mutations flow through `PendingUiIntents` -> `Session::execute_as_ui` / `dispatch_ui_command_as_ui`), zero `unsafe`, zero `chrono`, zero telemetry, zero bundled `.ttf`/`.otf`/`.woff` binaries, and zero `.unwrap()`/`.expect()`/`panic!()` in non-test code.
- **Resolved prior tickets verified intact:** `R15-GUI-02`/`05` (`merge_pass` + `commit_all_if_settled` in `mod.rs`), `R15-GUI-06` (`status_footer.rs` MV empty-cell guards), `R15-GUI-12` (X-cell clipboard via `WorkspaceCopy` handoff + `data_grid/shortcuts.rs`), `R15-GUI-19`/`20` (`mod.rs` import target/source split + `prepare_clipboard_selection`), `R15-GUI-31` (checkpoints palette route), `R15-GUI-33` (`inspect_bridge.rs:1335-1348` delegates About modal), `R15-GUI-34` (curated menu bar), `R15-GUI-35` (Presentation Mode removed), `R15-GUI-36` (`leading_dock.rs:1949-2025` draggable dock splitter), `R15-GUI-37` (`mod.rs:1025-1064` `Tokens::meets_wcag_aa()` + `widgets.rs:235-261` `paint_interactive_chrome`), `R15-GUI-38` (`leading_dock.rs:2065-2494` navigator context menu + cascade delete confirm, noting live coexistence/inspect gap in `FA-SEC16-04`), `R15-GUI-39` (`top_chrome.rs:875-897` + `w5_toolbar_menus.rs:426` localized `"Commands"` + dynamic shortcut), `R15-GUI-40` (`canvas_view.rs:1291-1347` hit-box-only TeX overlays), `GWF-12` (`canvas_view.rs:188-205` worker self-join guard), `GWF-15` (`canvas_view.rs:1539-1603, 2350-2558` add/drag/resize/z-order panels).
- **Live regressions flagged below:** `FA-SEC16-01` (partial regression of `R15-GUI-18`, `R15-GUI-30`, `R15-GUI-13`, `R15-GUI-23`), `FA-SEC16-02` (regression of `R15-GUI-27`).

---

## File Coverage Table

| File | LOC | Lines Inspected | Concrete Invariants Checked | Findings |
|---|---:|---|---|---|
| [`crates/ui-egui/src/egui_shell/mod.rs`](../../../../crates/ui-egui/src/egui_shell/mod.rs) | 3264 | `1..=3264` | `Tokens::meets_wcag_aa()` contrast checks, `UiMetrics::solve_shell_layout` RTL mirroring & clamp math, `PendingUiIntents` discard-pass retention & batch commit (`R15-GUI-02/05`), headless-first routing, UTF-8 byte offsets in ASCII-lowercased find/replace, `GeometryStamp` scale quantization | `FA-SEC16-05`, `FA-SEC16-08` |
| [`crates/ui-egui/src/egui_shell/brand.rs`](../../../../crates/ui-egui/src/egui_shell/brand.rs) | 90 | `1..=90` | Clean-room embedded 128 px PNG brand mark decode, checked slice access (`buf.get(..info.buffer_size())`), once-per-`Context` mipmapped texture caching | Clean — clean-room PNG decode with checked slice bounds and single texture upload per context |
| [`crates/ui-egui/src/egui_shell/canvas_view.rs`](../../../../crates/ui-egui/src/egui_shell/canvas_view.rs) | 2842 | `1..=2842` | `AsyncRasterWorkerOwner::drop` thread-id self-join guard (`GWF-12`), async scene keying/filters, `render_live_tex_equations` non-duplication (`R15-GUI-40`), layout panel drag/SE-resize/z-order (`GWF-15`), context menu parameter payloads, `ui` pan/selection state scoping | `FA-SEC16-10`, `FA-SEC16-12` |
| [`crates/ui-egui/src/egui_shell/center_stage.rs`](../../../../crates/ui-egui/src/egui_shell/center_stage.rs) | 2190 | `1..=2190` | Split pane geometry & divider ratio clamping vs `app.split.set_ratio`, split grid preview allocation & `FrameInspectMeta` recording, floating notes drag bounds & pin toggle wiring, gallery multi-select style intent (`gallery.apply_style_from`) | `FA-SEC16-11` |
| [`crates/ui-egui/src/egui_shell/command_palette.rs`](../../../../crates/ui-egui/src/egui_shell/command_palette.rs) | 860 | `1..=860` | Backdrop capture, `consume_palette_keys` order vs `TextEdit`, command route coverage vs `UI_COMMANDS`, `DESKTOP_HOST_ROUTED_COMMANDS`, and engine schemas (`R15-GUI-18`, `R15-GUI-31`) | `FA-SEC16-01`, `FA-SEC16-07` |
| [`crates/ui-egui/src/egui_shell/driver.rs`](../../../../crates/ui-egui/src/egui_shell/driver.rs) | 1281 | `1..=1281` | Headless `egui::Context` stepping & settle loop, AccessKit pixel-to-hit-box & `Role::ScrollBar` non-overlap checks, finite coordinate/key validation, float-to-f32 conversions | Clean — finite input guards, settled-frame geometry stamps, AccessKit/scrollbar parity checks, zero production panics |
| [`crates/ui-egui/src/egui_shell/fonts.rs`](../../../../crates/ui-egui/src/egui_shell/fonts.rs) | 898 | `1..=898` | Zero bundled fonts, bounded host OS + `CURVEBOOK_FONTS_DIR` SFNT loading, CJK/symbol/RTL fallback family discovery, `font_data` key uniqueness, deterministic SFNT symbol fallback | `FA-SEC16-06` |
| [`crates/ui-egui/src/egui_shell/inspect_bridge.rs`](../../../../crates/ui-egui/src/egui_shell/inspect_bridge.rs) | 1424 | `1..=1424` | `HybridCoexistenceArbiter` modal arbitration vs shell modals, `build_dialog_snapshot_from_meta` kind/bounds mapping, global shortcut dispatch vs `ShortcutRegistry`, `render_about_modal` delegation (`R15-GUI-33`), copy/paste intent ordering | `FA-SEC16-03`, `FA-SEC16-04`, `FA-SEC16-12` |
| [`crates/ui-egui/src/egui_shell/leading_dock.rs`](../../../../crates/ui-egui/src/egui_shell/leading_dock.rs) | 2495 | `1..=2495` | Dock splitter drag/double-click reset (`R15-GUI-36`), navigator context menu & `sheet_id` routing (`R15-GUI-38`), delete-confirm modal arbitration & inspection, frozen-sheet guards, dock shortcuts vs `ShortcutRegistry`, i18n coverage | `FA-SEC16-03`, `FA-SEC16-04`, `FA-SEC16-12` |
| [`crates/ui-egui/src/egui_shell/rasterizer.rs`](../../../../crates/ui-egui/src/egui_shell/rasterizer.rs) | 641 | `1..=641` | Software triangle rasterizer bounds, `ScissorBoxPx` finite/non-negative clipping, checked `.get(...)` on vertex/index/blit/crop buffers, `checked_mul` framebuffer sizing, zero `unsafe` | Clean — checked slice indexing and framebuffer bounds guards throughout raster/blit/crop paths |
| [`crates/ui-egui/src/egui_shell/status_footer.rs`](../../../../crates/ui-egui/src/egui_shell/status_footer.rs) | 1745 | `1..=1745` | Quick-stats empty-table guards (`R15-GUI-06`), cursor readout coordinate transform, footer popover lifecycle across `enabled`/Escape/outside-click states, RTL chevron orientation | `FA-SEC16-05`, `FA-SEC16-09` |
| [`crates/ui-egui/src/egui_shell/top_bar_fit.rs`](../../../../crates/ui-egui/src/egui_shell/top_bar_fit.rs) | 140 | `1..=140` | Deterministic monotonic width degradation (`fit_right_group`), Presentation Mode absence (`R15-GUI-35`), MIT/Apache-2.0 attribution header | Clean — monotonic Tier-1 right-group width solver verified against overflow/jitter |
| [`crates/ui-egui/src/egui_shell/top_chrome.rs`](../../../../crates/ui-egui/src/egui_shell/top_chrome.rs) | 2195 | `1..=2195` | Tier-1 inline menu bar & `push_menu_command_intent` host/parameter routing (`R15-GUI-30`, `R15-GUI-34`, `R15-GUI-39`), Tier-2 contextual toolbar parameter payloads (`R15-GUI-27`), narrow overflow menu handling | `FA-SEC16-01`, `FA-SEC16-02` |
| [`crates/ui-egui/src/egui_shell/widgets.rs`](../../../../crates/ui-egui/src/egui_shell/widgets.rs) | 1361 | `1..=1361` | Canonical §10.6 `paint_interactive_chrome` hover/active/selected/disabled contrast states (`R15-GUI-37`), AccessKit labels/expanded state, bounded dropdown popups, `panel_footer`/`bottom_bar` click-blocking | Clean — consistent contrast chrome, AccessKit roles, and popup clamping across shared widgets (two private dropdown helpers carry `#[allow(clippy::too_many_arguments)]`) |

---

## Findings

### FA-SEC16-01 — In-window menu bar and Command Palette bypass OS file dialogs and discard bytes on export/template/import commands (REGRESSION of `R15-GUI-18`, `R15-GUI-30`, partial `R15-GUI-13`/`R15-GUI-23`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S2` (High)
- **Location:** [`crates/ui-egui/src/egui_shell/top_chrome.rs:844-860`](../../../../crates/ui-egui/src/egui_shell/top_chrome.rs#L844-L860), [`crates/ui-egui/src/egui_shell/command_palette.rs:673-678,717-737,787`](../../../../crates/ui-egui/src/egui_shell/command_palette.rs#L717-L737)
- **Category:** `wrong-result` / `contract-drift` / `data-loss`
- **Defect:**
  `push_menu_command_intent` (`top_chrome.rs:844-860`) only checks `is_desktop_host_routed_command` (`crates/ui-egui/src/menus/mod.rs:26-50`) and falls back to `command_palette::resolve_shell_command_route` for all other curated menu items (`crates/ui-egui/src/menus/menu_bar.rs:199-227`). However, `DESKTOP_HOST_ROUTED_COMMANDS` omits multiple file-backed export, template, and import commands that the desktop host implements OS file pickers for (`apps/curvebook/src/desktop_host.rs:1550-1567` `menu_action_triggers_native_file_dialog`), including `export.tiff`, `table.exportCsv`, `analysis.exportCsv`, `analysis.exportJson`, `project.saveTemplate`, `project.instantiateTemplate`, `table.importCsv`, `graph.exportSvg`/`Pdf`/`Png`, `result.generateReport`, and the legacy foreign-format XML/binary importers (`menu_bar.rs:203-204`).
  - `command_palette.rs:717-737, 787` maps those export and template-save commands directly to `PaletteCommandRoute::EngineCommand` with `{ "sheet_id": ... }` / `{ "return_bytes": true }` and no `"path"`.
  - `command_palette.rs:673-678` maps `table.importCsv` and the legacy binary/XML importers to `OpenImportWizard` (which only accepts delimited text/CSV/TSV) and `project.instantiateTemplate` to `SetLauncherOpen { open: true }`.
  - In `apps/curvebook/src/gui_window.rs:551-564`, only `graph.insertImage`/`layout.insertImage` have a pathless `EngineCommand` host-routing fallback; all other commands above fall into `remaining_intents` and execute in memory without opening an OS Save/Open dialog.
- **Failure scenario:** On Linux, Windows, or macOS with `--in-window-menus` (or via the Command Palette on any platform), clicking **File › Export › TIFF**, **Export Table CSV**, **Export Analysis CSV/JSON**, or **File › Save as Template** renders/serializes the output into memory and silently drops the bytes without prompting for a destination or writing any file to disk; clicking **File › Import** XML/binary archive entries opens a delimited-text wizard instead of the OS archive file picker.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/top_chrome.rs:844-860
      if crate::menus::is_desktop_host_routed_command(command_id) {
          intents.push_ui(command_id, json!({}));
          return;
      }
      let is_ui = command_id.starts_with("ui.") || command_id.starts_with("app.");
      match super::command_palette::resolve_shell_command_route(app, command_id, is_ui) {
          Some(super::command_palette::PaletteCommandRoute::UiCommand { command_id, params }) => {
              intents.push_ui(command_id, params);
          }
          Some(super::command_palette::PaletteCommandRoute::EngineCommand { command_id, params }) => {
              intents.push_engine(command_id, params);
          }
  ```
  ```rust
  // crates/ui-egui/src/egui_shell/command_palette.rs:717-737, 787
          "export.svg" | "export.pdf" | "export.png" | "export.tiff" => eng(sheet_id_param(resolve_renderable_sheet_id(doc))),
          "graph.exportSvg" | "graph.exportPdf" | "graph.exportPng" | "graph.exportEmf" => eng(sheet_id_param(resolve_graph_sheet_id(doc))),
          "layout.exportEmf" => eng(sheet_id_param(resolve_layout_sheet_id(doc))),
          "table.exportCsv" => { ... }
          "analysis.exportJson" | "analysis.exportCsv" => eng(sheet_id_param(resolve_result_sheet_id(doc))),
          "result.generateReport" => { ... }
  ...
              "project.saveTemplate" => return eng(json!({ "return_bytes": true })),
  ```
- **Suggested fix:** Add all file-dialog export, template, and import commands (`export.tiff`, `graph.exportSvg`/`Pdf`/`Png`, `table.exportCsv`, `analysis.exportJson`/`Csv`, `result.generateReport`, `project.saveTemplate`, `project.instantiateTemplate`, `table.importCsv`, `table.importPzfx`, and sibling foreign-format archive importers) to `DESKTOP_HOST_ROUTED_COMMANDS` (or match `menu_action_triggers_native_file_dialog` consistently in `push_menu_command_intent`, `command_palette.rs`, and `gui_window.rs:552`) so native Save/Open pickers always run before execution.
- **Related:** `R15-GUI-18`, `R15-GUI-30`, `R15-GUI-13`, `R15-GUI-23`; sibling routing in `apps/curvebook/src/gui_window.rs:551-582` and `crates/ui-egui/src/menus/mod.rs:23-54` (`SEC-14`/`SEC-19`).

---

### FA-SEC16-02 — Tier-2 `+ Constant` button overwrites existing `"Constant"` entry with `1.0` (REGRESSION of `R15-GUI-27`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S2` (High)
- **Location:** [`crates/ui-egui/src/egui_shell/top_chrome.rs:1749-1763`](../../../../crates/ui-egui/src/egui_shell/top_chrome.rs#L1749-L1763)
- **Category:** `data-loss` / `wrong-result`
- **Defect:**
  While `R15-GUI-27` was fixed inside [`info_view.rs:368-448`](../../../../crates/ui-egui/src/egui_shell/info_view.rs#L368-L448) via `next_unused_constant_name(info)`, the contextual Tier-2 toolbar (`Tier2SheetContext::InfoSheet`) still emits `info.setConstant` with a hardcoded `"name": "Constant", "value": 1.0`. Additionally, if `doc.active_sheet_id` were `None`, `info_id` falls back to `""` while `enabled` remains `modal_enabled`.
- **Failure scenario:** A user creates or edits a constant named `"Constant"` (case-insensitive) to a meaningful experimental parameter (e.g. `42.5`), then clicks `+ Constant` (`toolbar.info_add_constant`) in the top toolbar to add another constant. `run_info_set_constant` (`crates/engine/src/cmd/info.rs:137`) calls `info.upsert_constant("Constant", ConstantValue::Number(1.0), ...)`, silently overwriting the existing constant's value to `1.0` and recalculating dependent analyses/hooks.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/top_chrome.rs:1749-1763
          Tier2SheetContext::InfoSheet => {
              let info_id = doc.active_sheet_id.as_ref().map(|id| id.as_str()).unwrap_or_default();
              actions.push(make_engine_action(
                  app,
                  Tier2ActionInput {
                      widget_id: "toolbar.info_add_constant",
                      label: "+ Constant",
                      command_id: "info.setConstant",
                      params: json!({ "sheet_id": info_id, "name": "Constant", "value": 1.0 }),
                      enabled: modal_enabled,
                      disabled_reason: None,
                      primary: true,
                  },
                  true,
              ));
  ```
- **Suggested fix:** Pick the first unused name against `doc.info_sheet` (`Constant`, `Constant 1`, `Constant 2`, … case-insensitively, sharing a helper with `info_view::next_unused_constant_name`), default initial value to `null` (or avoid clobbering existing names), and guard `enabled: modal_enabled && doc.active_sheet_id.is_some()`.
- **Related:** `R15-GUI-27` (`crates/ui-egui/src/egui_shell/info_view.rs:368-448`).

---

### FA-SEC16-03 — Global and dock keyboard shortcuts hardcode raw keys and ignore customized `app.shortcuts` bindings

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S2` (High)
- **Location:** [`crates/ui-egui/src/egui_shell/inspect_bridge.rs:1205-1305`](../../../../crates/ui-egui/src/egui_shell/inspect_bridge.rs#L1205-L1305), [`crates/ui-egui/src/egui_shell/leading_dock.rs:391-428`](../../../../crates/ui-egui/src/egui_shell/leading_dock.rs#L391-L428)
- **Category:** `contract-drift` / `wrong-result`
- **Defect:**
  `handle_shell_global_shortcuts` (`inspect_bridge.rs:1205-1305`) and `handle_leading_dock_shortcuts` (`leading_dock.rs:391-428`) consume fixed raw `egui::Key` chords (`Cmd/Ctrl+T`, `Cmd/Ctrl+K`, `Cmd/Ctrl+Shift+Z`/`Cmd/Ctrl+Y`, `Cmd/Ctrl+Z`, `Cmd/Ctrl+C`, `Cmd/Ctrl+V`, `Cmd/Ctrl+\`, `F6`/`Shift+F6`, `Cmd/Ctrl+Shift+N`, `Cmd/Ctrl+Shift+F`, `F2`) without consulting `app.shortcuts` (`ShortcutRegistry`). By contrast, `command_palette.rs:63, 162`, `top_chrome.rs:876`, and `menus/menu_bar.rs` render hints and tooltips from `app.shortcuts.active_combo(...)`, and `apps/curvebook/src/gui_window.rs:1071-1086` (`match_routed_desktop_shortcut`) explicitly skips non-dialog UI commands like `ui.palette.open`, `ui.pane.cycle`, `ui.split.toggle`, `ui.family.toggle`, `edit.undo`, and `edit.redo`.
- **Failure scenario:** Rebinding `ui.palette.open` from `Cmd+K` to `F6` and `ui.pane.cycle` from `F6` to `Cmd+K` (or customizing `ui.split.toggle`, `ui.family.toggle`, `edit.undo`, `edit.redo`, or `ui.editParams.open`) in Preferences › Keyboard Shortcuts or via `ui.shortcut.set` updates menu/palette labels, but pressing `F6` still cycles panes and `Cmd+K` still opens the Command Palette. Additionally, hardcoding `Key::Z`/`Key::Y`/`Key::K`/`Key::C` can intercept user-rebound bindings mapped to those chords.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/inspect_bridge.rs:1213-1247, 1293-1305
      if HybridCoexistenceArbiter::allows_cmd_t_edit_params(app, decision, egui_wants_kb)
          && ctx.input_mut(|i| {
              i.consume_key(Modifiers::COMMAND, Key::T) || i.consume_key(Modifiers::CTRL, Key::T) || i.consume_key(Modifiers::MAC_CMD, Key::T)
          })
      {
          intents.push_ui("ui.editParams.open", json!({}));
      }
      if decision.modal_backdrop_active || egui_wants_kb {
          return;
      }
      if ctx.input_mut(|i| {
          i.consume_key(Modifiers::COMMAND, Key::K) || i.consume_key(Modifiers::CTRL, Key::K) || i.consume_key(Modifiers::MAC_CMD, Key::K)
      }) {
          intents.push_set_palette_open(true);
      }
  ...
      if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F6)) {
          intents.push_ui("ui.pane.cycle", json!({ "reverse": true }));
      } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F6)) {
          intents.push_ui("ui.pane.cycle", json!({ "reverse": false }));
      }
  ```
  ```rust
  // crates/ui-egui/src/egui_shell/leading_dock.rs:402-418
      let toggle_nav = ui.ctx().input_mut(|i| {
          i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::N)
              || i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::N)
              || i.consume_key(Modifiers::MAC_CMD | Modifiers::SHIFT, Key::N)
      });
      if toggle_nav {
          intents.push_toggle_leading_collapsed_rail();
      }
  ```
- **Suggested fix:** Dispatch global and dock shortcuts through `app.shortcuts.handle_key_combo_for_context(...)` (or `active_combo(command_id)` mapped to `Modifiers` + `Key`), falling back to defaults only when unbound, and keep host-window key dispatch synchronized to avoid duplicate or missed triggers.
- **Related:** `apps/curvebook/src/gui_window.rs:1071-1087`, `apps/curvebook/src/desktop_host.rs:4141-4179`.

---

### FA-SEC16-04 — Navigator delete-confirmation modal bypasses `HybridCoexistenceArbiter` modal blocking and breaks `ui.inspect` dialog reporting

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S2` (High)
- **Location:** [`crates/ui-egui/src/egui_shell/leading_dock.rs:2398-2494`](../../../../crates/ui-egui/src/egui_shell/leading_dock.rs#L2398-L2494), [`crates/ui-egui/src/egui_shell/inspect_bridge.rs:215-231,458-476`](../../../../crates/ui-egui/src/egui_shell/inspect_bridge.rs#L215-L231)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:**
  1. `render_navigator_delete_confirm_modal` (`leading_dock.rs:2398-2494`) opens a destructive `egui::Modal` whenever `views.leading_dock.delete_confirm.is_some()`, yet `HybridCoexistenceArbiter::evaluate_with_views` (`inspect_bridge.rs:215-231`) does not check `views.leading_dock.delete_confirm.is_some()`. As a result, `decision.modal_backdrop_active` remains `false`, so workspace panels stay `enabled = true` and `handle_shell_global_shortcuts` / `handle_leading_dock_shortcuts` continue firing behind the modal.
  2. `render_navigator_delete_confirm_modal` never records a `"modal.dialog"`/`"dialog.*"` entry in `FrameInspectMeta`, and `build_dialog_snapshot_from_meta` (`inspect_bridge.rs:458-476`) omits `views.leading_dock.delete_confirm` (`dialog: None` in `ui.inspect`) as well as `"palette.dialog"` (`command_palette.rs:231`, causing `dialog_entry` lookup at `:458-461` to return `None` and drop live palette bounds/items). Moreover, when `app_modals` dialogs using `"modal.dialog"` (`Preferences`, `Find & Replace`, `UnsavedChanges`) or `TransformModal` are open, `build_dialog_snapshot_from_meta:467-475` fails to prioritize `HybridModalSurface` over stale `app.canvas_dialogs.active_dialog` and falls back to `e.widget_id.trim_start_matches("dialog.")` on `"modal.dialog"` (yielding literal `"modal.dialog"`).
- **Failure scenario:** Right-clicking a sheet in Navigator › **Delete…** shows the cascade-delete confirmation modal while workspace controls and `Cmd+Z` / `Cmd+K` / `F2` / `Cmd+C` shortcuts remain active underneath it, and automated inspection/testing receives `dialog: None` instead of the active confirmation modal.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/inspect_bridge.rs:215-231
          let modal_surface = if host_close_dialog_open || views.app_modals.unsaved_changes_open {
              HybridModalSurface::HostUnsavedClose
          } else if views.app_modals.preferences_open {
              HybridModalSurface::EguiPreferences
          } else if views.app_modals.find_replace_open {
              HybridModalSurface::EguiFindReplace
          } else if app.grid.variables_dialog.open {
              HybridModalSurface::EguiManageVariables
          } else if app.grid.import_wizard.open {
              HybridModalSurface::EguiImportWizard
          } else if views.transform_modals.open_kind.is_some() {
              HybridModalSurface::EguiTransformModal
          } else {
              classify_active_dialog_surface(app)
          };

          let modal_backdrop_active = modal_surface != HybridModalSurface::None;
  ```
  ```rust
  // crates/ui-egui/src/egui_shell/inspect_bridge.rs:458-475
      let dialog_entry = meta
          .find_by_widget_id("modal.dialog")
          .or_else(|| meta.find_by_widget_id("dialog.about"))
          .or_else(|| meta.entries.iter().find(|e| (e.widget_id.starts_with("dialog.") || e.widget_id.starts_with("modal.")) && e.role == "dialog"));
      let has_shell_modal = dialog_entry.is_some() || app.grid.variables_dialog.open || app.grid.import_wizard.open;
  ...
      } else {
          dialog_entry.map(|e| e.widget_id.trim_start_matches("dialog.").to_owned()).unwrap_or_else(|| String::from("shell_modal"))
      };
  ```
- **Suggested fix:** Add a `HybridModalSurface` variant (or `modal_backdrop_active` check) for `views.leading_dock.delete_confirm.is_some()`, record `modal.backdrop` and `dialog.nav_delete_confirm` (`role: "dialog"`) in `render_navigator_delete_confirm_modal`, include `"palette.dialog"` in `dialog_entry` lookup, and drive `dialog_kind` in `build_dialog_snapshot_from_meta` with the exact same priority order as `HybridCoexistenceArbiter::evaluate_with_views`.
- **Related:** `R15-GUI-38` (`leading_dock.rs:2398-2494`), `command_palette.rs:231`.

---

### FA-SEC16-05 — Incomplete RTL workspace mirroring: `UiMetrics::solve_shell_layout` hardcodes `rtl_mirrored: false`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/mod.rs:495,521,599`](../../../../crates/ui-egui/src/egui_shell/mod.rs#L599), [`crates/ui-egui/src/egui_shell/status_footer.rs:704`](../../../../crates/ui-egui/src/egui_shell/status_footer.rs#L704)
- **Category:** `contract-drift` / `wrong-result`
- **Defect:**
  `ResponsiveShellLayout` defines `pub rtl_mirrored: bool` (`mod.rs:662`) and downstream layout consumers such as `status_footer.rs:704` branch on `layout.rtl_mirrored`, yet `ShellLayoutInput` carries no RTL/locale boolean and `UiMetrics::solve_shell_layout` (`mod.rs:456-601`) hardcodes `rtl_mirrored: false` in all three return statements (`:495`, `:521`, `:599`). Furthermore, `leading_dock.rs` (`egui::Panel::left`) and `trailing_studio` (`egui::Panel::right`) never mirror dock sides when an RTL locale (`ar` / `he`) is active.
- **Failure scenario:** Switching the shell locale to `ar` or `he` via **View › Language** or `ui.locale.set` translates UI labels and reverses history arrows (`status_footer.rs:1007`), yet keeps Navigator on the left, Format Inspector on the right, and `layout.rtl_mirrored == false`.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/mod.rs:581-600
          ResponsiveShellLayout {
              ...
              center_stage_w_pt: center_w.max(0.0),
              center_stage_h_pt,
              split_orientation,
              rtl_mirrored: false,
          }
  ```
  ```rust
  // crates/ui-egui/src/egui_shell/status_footer.rs:704
      let stage_x = if layout.rtl_mirrored { layout.trailing_dock_w_pt } else { layout.leading_dock_w_pt };
  ```
- **Suggested fix:** Thread `is_rtl: is_rtl_locale(&app.locale)` into `ShellLayoutInput`, propagate `rtl_mirrored` out of `solve_shell_layout`, and honor `layout.rtl_mirrored` when placing the leading/trailing dock panels (`Panel::left`/`Panel::right` and splitters).
- **Related:** `FA-SEC16-06`; `crates/ui-egui/tests/cjk_rtl_hidpi_desktop_qa.rs:179-181`.

---

### FA-SEC16-06 — `fonts.rs` omits Arabic/Hebrew OS fallback queries, saturates a 28-file directory cap, and can collide on `font_data` keys

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/fonts.rs:41,243-282,367-379,534-575,614-668`](../../../../crates/ui-egui/src/egui_shell/fonts.rs#L243-L282)
- **Category:** `wrong-result` / `resources`
- **Defect:**
  1. `assemble_font_definitions` (`fonts.rs:243-282`) explicitly queries `OS_SYMBOL_FAMILIES` and `OS_CJK_FAMILIES`, and `curated_os_font_candidates` (`614-668`) enumerates Latin, monospace, symbol, and CJK files across macOS/Windows/Linux—with zero curated Arabic or Hebrew (`ar`/`he`) family queries or OS candidate paths. Meanwhile `discover_host_font_blobs` (`534-575`) sorts walked OS directory entries lexicographically and stops at `MAX_OS_FONT_FILES = 28` (`:41, 539, 562`). Because curated files + sorted `/System/Library/Fonts/Supplemental` / `/usr/share/fonts` entries easily reach 28 files alphabetically before `GeezaPro` / `NotoSansArabic` / `NotoSansHebrew` / `Raanana`, Arabic and Hebrew faces are never loaded into `egui`.
  2. `register_face_for_family` (`367-372`) constructs keys as `format!("{prefix}_{}_{}", id_to_slug(db, id), face_index)` where `id_to_slug` (`375-379`) hashes only `families.first()`. Under `"os_extra"` (`274-281`), distinct faces that share a first family name and `face_index = 0` (e.g. different widths/files) collide in `defs.font_data.entry(key).or_insert_with(...)`, silently keeping only the first blob while appending duplicate keys or dropping the extra face.
- **Failure scenario:** Selecting `ar` or `he` in the `egui` shell on Linux or on macOS systems where `Arial Unicode.ttf` is absent or 28 earlier fonts fill the budget displays missing-glyph boxes (`□`) for Arabic/Hebrew menu items and labels even though OS Arabic/Hebrew fonts exist on disk.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/fonts.rs:367-379
      let key = format!("{prefix}_{}_{}", id_to_slug(db, id), face_index);
      defs.font_data.entry(key.clone()).or_insert_with(|| Arc::new(egui::FontData::from_blob(blob, face_index)));
      if !chain.iter().any(|existing| existing == &key) {
          chain.push(key);
      }
      true
  }

  fn id_to_slug(db: &fontdb::Database, id: fontdb::ID) -> String {
      let raw = db.face(id).and_then(|f| f.families.first().map(|(name, _)| name.as_str()).or(Some(f.post_script_name.as_str()))).unwrap_or("face");
      let slug: String = raw.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '_' }).collect();
      if slug.is_empty() { String::from("face") } else { slug }
  }
  ```
  ```rust
  // crates/ui-egui/src/egui_shell/fonts.rs:538-572
      for candidate in curated_os_font_candidates() {
          if blobs.len() >= MAX_OS_FONT_FILES {
              break;
          }
  ...
      for path in dir_candidates {
          if blobs.len() >= MAX_OS_FONT_FILES {
              break;
          }
  ```
- **Suggested fix:** Add `OS_RTL_FAMILIES` (`Geeza Pro`, `Arial Hebrew`, `SF Arabic`, `Noto Sans Arabic`, `Noto Sans Hebrew`, `Tahoma`, `Times New Roman`) and platform RTL paths to `curated_os_font_candidates` before filling `MAX_OS_FONT_FILES` (or load curated RTL faces separately from the directory cap), and disambiguate `register_face_for_family` keys using `post_script_name` or a monotonic face index alongside `id_to_slug`.
- **Related:** `FA-SEC16-05`.

---

### FA-SEC16-07 — `command_palette.rs` steals `Home`/`End` from the query `TextEdit` and mismatches `"ui.export.open"`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/command_palette.rs:139,164,281-287,593-597`](../../../../crates/ui-egui/src/egui_shell/command_palette.rs#L274-L295)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:**
  1. `render_command_palette` invokes `consume_palette_keys(ctx, ...)` at line `139` before rendering the search `egui::TextEdit` at line `164`. `consume_palette_keys` (`281-287`) unconditionally consumes `Modifiers::NONE` `Key::Home` and `Key::End` to jump list selection to the first/last command row, preventing `Home`/`End` from moving the text caret inside the focused search box.
  2. `resolve_palette_ui_route` (`593`) matches `"ui.export.open"`, whereas `UI_COMMANDS` (`crates/ui-egui/src/menus/ui_commands.rs:370`) registers `"ui.export.figure.open"`. Consequently `"ui.export.figure.open"` falls into the wildcard arm `_ => UiCommand { params: json!({}) }` (`596`), losing explicit `sheet_id` parameter resolution (`resolve_renderable_sheet_id`).
- **Failure scenario:** While editing a search string in the Command Palette, pressing `Home` or `End` jumps the highlighted command row instead of moving the text cursor to the start/end of the query; `"ui.export.figure.open"` misses its explicit palette route arm.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/command_palette.rs:139, 164, 281-287, 593-597
      let key_nav = consume_palette_keys(ctx, app, state, &rows, intents);
  ...
              let query_resp = ui.add(egui::TextEdit::singleline(&mut state.query).hint_text(hint).desired_width(f32::INFINITY));
  ...
          } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Home)) {
              state.selected_index = 0;
              moved = true;
          } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::End)) {
              state.selected_index = rows.len() - 1;
              moved = true;
          }
  ...
          "ui.canvas.preflight" | "ui.export.open" | "ui.embeddedTable.format.open" | "ui.image.format.open" => {
              Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: sheet_id_param(resolve_renderable_sheet_id(doc)) })
          }
          _ => Some(PaletteCommandRoute::UiCommand { command_id: command_id.to_owned(), params: json!({}) }),
  ```
- **Suggested fix:** Only handle `Home`/`End` list navigation when the query input is empty or `Modifier::ALT`/`CTRL` is held (or run list key handling after `TextEdit` consumes cursor keys while `query_resp.has_focus()`), and replace `"ui.export.open"` with `"ui.export.figure.open"` at `command_palette.rs:593`.
- **Related:** `FA-SEC16-01`.

---

### FA-SEC16-08 — Find & Replace `"Find Next"` stays pinned on the first match forever and skips X/Time columns

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/mod.rs:2758-2843`](../../../../crates/ui-egui/src/egui_shell/mod.rs#L2758-L2843)
- **Category:** `wrong-result`
- **Defect:**
  `apply_find_replace_on_active_sheet` always scans `0..rows`, `0..cols`, `0..subcols` from `(0, 0, 0)` and records only `first_match` (`2801-2803`). When `replace_all == false` (triggered by `"Find Next"` in `app_modals.rs:1009`), it ignores `app.grid.tool.active`, selects `first_match` on every invocation (`2834-2837`), never advances to subsequent matches or wraps around, and inspects only `ds.archetype.data_cell(...)` (`2787`), omitting `x_or_time_cell` values on XY and Survival tables.
- **Failure scenario:** In a table with two or more cells matching the search term, clicking **Find Next** repeatedly selects the first match every time and never visits the second match; searching for a numeric/text value in an X or Time column returns `0` matches.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/mod.rs:2783-2804, 2833-2837
      for r in 0..rows {
          for c in 0..cols {
              let subcols = crate::grid_view::data_set_subcolumn_count(&ds.archetype, c).max(1);
              for s in 0..subcols {
                  let Some(cell) = ds.archetype.data_cell(CellCoord::new(r, c, s)) else {
                      continue;
                  };
  ...
                  match_count += 1;
                  if first_match.is_none() {
                      first_match = Some(curvebook_tools::GridCellCoord::new(r, c, s));
                  }
  ...
      app.egui_views.app_modals.find_replace.match_count = match_count;
      if let Some(coord) = first_match {
          app.grid.select_single_sync(coord);
          app.focus_widget_id = Some(format!("grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol));
      }
  ```
- **Suggested fix:** When `!replace_all`, search forward from the cell strictly after `app.grid.tool.active` with wrap-around to `first_match`, and also scan `x_or_time_cell` (or surface X-cell selection when matched).
- **Related:** `crates/ui-egui/src/egui_shell/app_modals.rs:994-1016`.

---

### FA-SEC16-09 — Status footer popovers ignore modal disabling, Escape, and outside clicks; cursor coordinate readout ignores paper offset and fit scale

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/status_footer.rs:702-717,893-914,1425-1603,1689-1701`](../../../../crates/ui-egui/src/egui_shell/status_footer.rs#L1425-L1603)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:**
  1. `render_status_footer` calls `render_footer_popovers(...)` unconditionally at `:1689` without checking `ctx_in.enabled`. Inside `show_footer_popup` (`:893-914`) and `render_footer_popovers` (`:1425-1603`), `egui::Area::new(...).order(egui::Order::Foreground)` never closes `filter_popup_open`, `sheet_switcher_open`, `link_popover_open`, or `stats_popover_open` when `!ctx_in.enabled` (active modal), on `Key::Escape`, or on pointer clicks outside the popup area—and `stats_popover_open` (`:1570-1602`) has no inner close trigger at all.
  2. `center_stage_cursor_coords` (`702-717`) computes `(pos - stage_rect.min - pan) / zoom` relative to the entire `center_stage` top-left instead of `paper_rect.min` (`canvas_view.rs:462-463` centers `paper_rect` inside `mat_rect` at `fit_scale * zoom`), producing readout coordinates shifted by the canvas mat margin and unadjusted for `fit_scale`.
- **Failure scenario:** Opening the footer Quick Stats popover (or CVD/Sheet/Link popup), clicking elsewhere on the workspace, pressing `Escape`, or opening a modal dialog leaves the foreground popover floating above the workspace/modal; moving the cursor over a graph/layout canvas displays footer `X/Y` point coordinates inconsistent with the sheet's ruler origin.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/status_footer.rs:712-716, 898-902, 1689-1701
      let zoom = app.canvas_dialogs.canvas.zoom.clamp(0.1, 8.0);
      let pan = views.canvas_view.pan_pt;
      let rel_x = f64::from(pos.x - stage_rect.min.x - pan[0]) / zoom;
      let rel_y = f64::from(pos.y - stage_rect.min.y - pan[1]) / zoom;
  ...
      egui::Area::new(egui::Id::unique(spec.popup_id))
          .order(egui::Order::Foreground)
          .fixed_pos(Pos2::new(est_x, (spec.anchor.min.y - 4.0).max(spec.viewport.min.y + 4.0)))
          .pivot(egui::Align2::LEFT_BOTTOM)
          .show(ctx, |ui| { ... });
  ...
                  render_footer_popovers(
                      ui,
                      app,
                      views,
                      FooterPopoverData { ... },
                      meta,
                      intents,
                  );
  ```
- **Suggested fix:** Close all footer popovers immediately when `!ctx_in.enabled`, on `Key::Escape`, or on any pointer click outside the rendered popup `Area` rect; compute cursor coordinates from `views.canvas_view.hover_readout` / cached `paper_rect` and `eff_scale` instead of `stage_rect.min` and raw `zoom`.
- **Related:** `crates/ui-egui/src/egui_shell/canvas_view.rs:455-464`.

---

### FA-SEC16-10 — Canvas `Insert Image…` context menu omits `sheet_id`, and no-op panel drags/resizes push spurious undo entries

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S3` (Medium)
- **Location:** [`crates/ui-egui/src/egui_shell/canvas_view.rs:91-112,1571-1580,2182-2192,2294-2297,2547-2558`](../../../../crates/ui-egui/src/egui_shell/canvas_view.rs#L2182-L2192)
- **Category:** `wrong-result` / `data-loss`
- **Defect:**
  1. Right-click context menu items `canvas.graph_ctx.insert_image` (`2189`) and `canvas.layout_ctx.insert_image` (`2295`) emit `intents.push_engine("graph.insertImage", json!({}))` and `intents.push_engine("layout.insertImage", json!({}))` without `"sheet_id": gs.id.as_str()` / `ls.id.as_str()`, unlike neighboring items (`:2217`, `:2273`). On desktop (`gui_window.rs:558` -> `desktop_host.rs:1878`), target sheet selection ignores missing params and falls back to `doc.active_sheet_id`, misrouting `Insert Image…` from a secondary split pane onto the active sheet (and failing engine schema validation outside `gui_window`).
  2. On `drag_stopped()` for both layout panel move (`1571-1580`) and SE resize handle (`2547-2558`), `layout.setSpec` is emitted unconditionally even when `drag.current_bounds_pt == drag.initial_bounds_pt` (no movement or snapped back to origin). Because `run_layout_set_spec` (`crates/engine/src/cmd/layout.rs:554`) always returns `CommandOutcome::mutated`, a zero-displacement drag marks the document dirty and pushes a no-op undo entry.
  3. `CanvasViewEguiState` (`91-112`) keeps single unkeyed `pan_pt`, `selected_panel_index`, and `selected_graph_annotation` across sheet switches.
- **Failure scenario:** Right-clicking a Graph or Layout displayed in the secondary split pane and choosing **Insert Image…** either targets the primary active sheet instead of the clicked sheet (`desktop_host.rs:1878`) or fails with missing `sheet_id`; clicking and releasing a layout panel or SE handle without moving dirties the project and adds an empty undo step.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/canvas_view.rs:1571-1580, 2189-2191, 2294-2297
              if p_resp.drag_stopped()
                  && state.layout_drag.as_ref().is_some_and(|d| d.item_index == idx && !d.resizing_corner)
                  && let Some(drag) = state.layout_drag.take()
              {
                  let mut updated_items = ls.spec.items.clone();
                  if let Some(target_item) = updated_items.get_mut(idx) {
                      set_layout_item_bounds_doc(target_item, drag.current_bounds_pt);
                      intents.push_engine("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "spec": { "items": updated_items } }));
                  }
                  state.active_snap_guides.clear();
              }
  ...
                      ) {
                          intents.push_engine("graph.insertImage", json!({}));
                          close_menu = true;
                      }
  ...
      if render_context_menu_item(ui, meta, "canvas.layout_ctx.insert_image", tr(locale, "r13.canvas.insert_image"), Some("layout.insertImage"), true) {
          intents.push_engine("layout.insertImage", json!({}));
          clicked = true;
      }
  ```
- **Suggested fix:** Pass `"sheet_id": gs.id.as_str()` / `"sheet_id": ls.id.as_str()` (and forward `params.get("sheet_id")` in `desktop_host.rs:1878`), guard `layout.setSpec` emission on `drag.current_bounds_pt != drag.initial_bounds_pt`, and key or reset `pan_pt` / selection indices per `SheetId`.
- **Related:** `apps/curvebook/src/desktop_host.rs:1874-1897`.

---

### FA-SEC16-11 — Center-stage split preview allocates unvirtualized grids and exposes a dead `note_pinned` toggle

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S4` (Low)
- **Location:** [`crates/ui-egui/src/egui_shell/center_stage.rs:125,958-960,975-1024,1837,1945-1947`](../../../../crates/ui-egui/src/egui_shell/center_stage.rs#L975-L1024)
- **Category:** `resources` / `wrong-result` / `cleanup`
- **Defect:**
  1. `render_split_column_sheet_preview` (`975-1024`) constructs up to `500 × 64` (`32,000`) `egui::Button` widgets inside `ScrollArea::both()` unconditionally every frame instead of using `show_rows` row virtualization and visible-column slicing, and omits `meta.record_response` on `"Focus in Grid"` (`:958`) as well as `split.col_preview.*` cells past `12 × 4`.
  2. `CenterStageEguiState::note_pinned` (`125`, toggled at `1945-1947` and painted as `"P"`/`"P*"` at `1933-1944`) is never read by any z-ordering, persistence, or cross-sheet visibility logic.
- **Failure scenario:** Viewing a large Column table (`>= 500` rows, multi-column) in Split View allocates tens of thousands of widgets per frame, degrading frame time; clicking the `"P"` pin button on a floating note changes its `"P*"` badge visually without pinning the note across sheet changes or restarts.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/center_stage.rs:958, 975-1010, 1945-1947
              if row.add_enabled(enabled, egui::Button::new(RichText::new("Focus in Grid").small())).clicked() {
                  intents.push_engine("sheet.select", json!({ "sheet_id": ds.id.as_str() }));
              }
  ...
              let max_cols = col_table.data_sets.len().min(64);
              let max_rows = col_table.row_count().min(500);
  ...
              for r_idx in 0..max_rows {
                  grid_ui.horizontal(|row_ui| {
                      ...
                      for c_idx in 0..max_cols {
                          ...
                          let resp = row_ui.add_enabled(enabled, btn);
  ...
                  if pin_resp.clicked() && enabled {
                      views.center_stage.note_pinned.insert(key.clone(), !pinned);
                  }
  ```
- **Suggested fix:** Virtualize `render_split_column_sheet_preview` via `show_rows`, record `"Focus in Grid"` in `FrameInspectMeta`, and either wire `note_pinned` to persistent floating-note visibility/z-order or remove the dead control.

---

### FA-SEC16-12 — Leading dock hardcoded English/ID strings, frozen convert-menu enablement, and minor `egui_shell` hygiene

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity:** `S4` (Low)
- **Location:** [`crates/ui-egui/src/egui_shell/leading_dock.rs:1231-1255,1688,2106-2119`](../../../../crates/ui-egui/src/egui_shell/leading_dock.rs#L1231-L1255), [`crates/ui-egui/src/egui_shell/canvas_view.rs:246-269,313-318,358`](../../../../crates/ui-egui/src/egui_shell/canvas_view.rs#L358), [`crates/ui-egui/src/egui_shell/inspect_bridge.rs:977`](../../../../crates/ui-egui/src/egui_shell/inspect_bridge.rs#L977)
- **Category:** `contract-drift` / `cleanup`
- **Defect:**
  1. `render_section_new_row` (`leading_dock.rs:1231-1255`) hardcodes English captions (`"+ New Data…"`, `"+ New Info…"`, `"+ New Graph…"`, `"+ New Layout…"`) and disabled tooltips instead of `tr(...)` (as do Navigator context-menu items at `:2079-2245, 2291-2367`), `finalize_drag_state` (`:1688`) displays internal sheet IDs (`format!("Moving {}", drag_id.as_str())`) instead of localized sheet titles, and `nav.ctx.convert_archetype` (`:2106-2119`) keeps `enabled: true` even when `menu.is_frozen` (`table.convertArchetype` rejects frozen sheets via `ensure_sheet_not_frozen`).
  2. `RasterWorkerResults` (`canvas_view.rs:246-269, 313-318`) retains `completed` and `last_submitted` vectors keyed by `(slot_tag, SheetId)` without pruning deleted sheets; `render_canvas_view_for_sheet` (`canvas_view.rs:358`, 7 args) and `render_egui_shell_frame_with_host` (`inspect_bridge.rs:977`, 6 args) carry stale `#[allow(clippy::too_many_arguments)]` attributes.
- **Failure scenario:** Non-English locales still see English `+ New …` buttons and context-menu labels in Navigator, dragging a sheet shows its internal `SheetId` instead of its title, right-clicking a frozen Data sheet offers an enabled `Convert Table Archetype…` item that fails on commit, and long sessions deleting many sheets retain old `RasterWorkerResults` metadata entries.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/leading_dock.rs:1231-1255, 1688, 2106-2119
      let (wid, label, cmd, row_enabled, disabled_reason) = match section {
          SheetSection::Data => ("nav.new.data", "+ New Data…", Some("sheet.createData"), enabled, None),
          SheetSection::Info => { ... ("nav.new.info", "+ New Info…", Some("sheet.duplicate"), ...) }
          ...
          SheetSection::Graphs => { ... ("nav.new.graphs", "+ New Graph…", Some("graph.create"), ...) }
          SheetSection::Layouts => { ... ("nav.new.layouts", "+ New Layout…", Some("layout.create"), ...) }
      };
  ...
          ui.label(egui::RichText::new(format!("Moving {}", drag_id.as_str())).size(11.5));
  ...
                  NavCtxItemSpec {
                      widget_id: "nav.ctx.convert_archetype",
                      label: "Convert Table Archetype…",
                      command_id: Some("ui.table.convertArchetype.open"),
                      enabled: true,
                      disabled_reason: None,
                      tokens,
                  },
  ```
- **Suggested fix:** Route Navigator labels/tooltips through `tr(...)`, look up `doc_sheet_title` in `finalize_drag_state`, set `enabled: !menu.is_frozen` on `nav.ctx.convert_archetype`, prune `RasterWorkerResults` entries for removed `SheetId`s (or cap `Vec` length), and remove redundant `#[allow(clippy::too_many_arguments)]` attributes on 6/7-arg functions.
- **Related:** `R15-GUI-38`.
