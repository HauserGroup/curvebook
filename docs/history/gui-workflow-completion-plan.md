# Plan: make WF-01–WF-65 genuinely GUI-completable (Beta gate)

## Context

Beta requires all 65 reference workflows to complete end to end through the drawn desktop GUI (`R10.25`). The 2026-11-21 audit (`docs/audit/gui-workflow-completion-audit-2026-11-21.md`, untracked on `refactor/R15.9-visual-design-tokens`) found that every `human_wf_*` test passes, but none meets the §6 five-stage definition:
- 8 tests seed stages with engine commands or state writes.
- 53 have no live upstream edit.
- Assertions are mostly existence-only, some assert enum defaults, and guarded clicks skip silently.
- WF-64 (graph orientation) and WF-65 (replicate trajectories, hide symbols) have no GUI control at all.

The docs still claim "50 / 50", "15 / 15" and "65 / 65" human-complete. `cargo xtask status` cannot catch this because it validates only test *names*, and it caps the count at 50.

Decisions taken with the user:
- Scope is the full Beta gate: controls, docs, the gate itself, and all 65 tests.
- `R10.25` and `R13.6` are reopened until the hardened tests land.

The intended outcome is honest status now, then a mechanically enforced definition of "complete" that every workflow meets through rendered-widget input.

## Work items (one PR each, per AGENTS.md)

New roadmap items go in `docs/ROADMAP.md`, using the R15.9 template (`Owner · Status · Was · Depends on · Gate: **Beta**`). Each is also added to the milestone table, the Order table and the Beta row of Release Gates. Each item is claimed before branching. Every PR is branched from `main`, not from the R15.9 branch.

### PR 1 — Honest status and audit record (`R10.25`, docs-only)
Branch `docs/R10.25-reopen-gui-completion`. Carry over the untracked audit file.

- **Audit record:**
  - Correct GWF-13. The paste wizard writes into the active sheet by design; the problem is that WF-04's test name and flow disagree.
  - Add three findings:
    - GWF-15: no GUI path adds, positions or resizes a graph panel in a layout. `layout.arrange` only auto-fills, and `resizing_corner` is never set in `canvas_view.rs`.
    - GWF-16: View > Theme submenu options have no widget ids (`top_chrome.rs:679-692`), and there is no menu or toolbar entry for Preferences.
    - GWF-17: WF-54's right-axis assignment and horizontal nudge have no GUI control ("Right Y2" at `format_graph/series.rs:372` is a read-only label).
  - Add a row to `docs/audit/README.md`.
- **`ROADMAP.md`:**
  - `R10.25`: untick `:243–244`, reworded as "GUI smoke paths pass; five-stage completion pending R10.26–R10.28".
  - `R13.6` (`:497–504`): set Status to `in progress` and untick `:502`, which also wrongly claims "0 skipped assertions".
  - Fix the counts at `:50, :53, :55, :58, :218, :222, :769`.
  - Add the new items `R10.26`, `R10.27`, `R10.28` and `R13.10` below.
- **`gui-workflows.md`:**
  - Spell out the five stages and the completion rules (next section) in §6. `R10.2` in history is frozen, so §6 becomes the binding text.
  - Rename the duplicate `### 6.2` heading at `:239` to `6.1a`.
  - Reword the §6.1, §6.2 and §7 text, Total rows and Blockers / Remaining-gaps cells to the vocabulary "GUI smoke `N / N`; five-stage `0 / N`". Keep the test names in each cell, so `check_gui_workflows_human_path` still finds evidence.
- **`STATUS.md`:** update `:3, :15, :30, :80–88, :119`, and fix the stale wording at `:109`.
- **`docs/evidence/user-interface.md`:** set `:72` and `:75` to `[ ]`, reworded as "five-stage completion pending; GUI smoke paths 50/50 egui pass (test names)". Keep the same `50/50 egui` fraction on every line, because the telemetry check fails on conflicting N values. Leave `:73` as it is.
- **Check:** `cargo xtask status`, `brands`, `cleanroom` and `pending`. None of them read `Status:` fields (verified in `xtask/src/status.rs`).

### PR 2 — Five-stage completion contract and gate (`R10.26`)
Branch `test/R10.26-gui-completion-contract`. Note: the raster-worker self-join fix (`AsyncRasterWorkerOwner::drop` and `owner_dropped_on_its_own_worker_thread_does_not_self_join` in `crates/ui-egui/src/egui_shell/canvas_view.rs`) already landed on `main` in `7116d92` (`#57`); this PR verifies that test passes alongside the new contract and gate.

**Harness** (`crates/ui-egui/tests/human_wf_common/`, new `workflow_run.rs`). Add a `WorkflowRun` type that wraps `EguiWfHarness`:
- **Starting state.** It is built only by `WorkflowRun::start("WF-NN")`, from an empty `UiApp`. Optional OS-level inputs are allowed: a pre-seeded `StubPlatformClipboardAdapter` (add `EguiWfHarness::new_with_clipboard`) or fixture files on disk.
- **What it exposes.** GUI input methods (click, type, keys, wheel, drag, plus the existing helpers `create_table_via_welcome`, `enter_grid_cell`, `run_analyze_entry` and `open_cmd_t`) and read-only `app()` / `state()`. It does not expose `app_mut`, `state_mut`, `from_app` or `dispatch_gui_action`, so a bypass fails to compile.
- **Stage recording.** It records stages automatically:
  - Create: a Welcome or Insert table creation.
  - Enter: grid or title input on a data sheet.
  - Configure: an analysis, fit or graph-format commit via `modal.ok` / apply.
  - Verify: an explicit `verify_*` call.
  - Live edit: `live_edit(cell, value, expect)`. It requires a result or graph to exist. It snapshots `scalars` / `tabs` / the graph SVG before and after, and asserts `ResultStatus::UpToDate` plus the expected change (or explicit invariance, as WF-09 and WF-56 need).
- **Intent errors.** The post-frame hook accumulates `last_intent_error` values into a list instead of overwriting.
- **Panics.** It installs a panic hook that records panics on any thread (this is what would have caught GWF-12).
- **`finish()`** asserts all of the following, then runs the existing gates `assert_zero_shortcut_compliance`, `verify_14_locales_section_9_4` and `assert_input_to_paint_budget`:
  - all five stages occurred, in order;
  - no result sheet is `Failed` or `Dirty`;
  - no intent errors and no background panics were recorded;
  - at least one `verify_reference` happened.
- **`verify_reference`** checks that the GUI configured the intended analysis on the intended data, without inventing numbers (D1). It builds the intended spec headlessly on a `curvebook_engine::Session`, using the same data from a `curvebook-testkit` fixture. It then compares named scalars from the GUI result and the reference result within a documented tolerance. The engine side is already oracle-tested.

**Gate** (`xtask/src/status.rs`):
- **Completion count.** Statically count a test function in `crates/ui-egui/tests/human_wf_*.rs` as *complete* for WF-NN when its body contains `WorkflowRun::start("WF-NN")` and `.finish()`.
- **Static lint.** Fail if such a function also contains `execute_command`, `Session::from_document`, `app_mut`, `state_mut`, `dispatch_gui_action`, or `find_by_widget_id(` used as a guard. The lint backs up the compile-time restriction.
- **Raise the cap from 50 to 65.** This covers `parse_backend_wf_index` (`:531`), the `den != 50` filter (`:593`) and the messages (`:129`, `:634`).
- **New grammar.** Accept a "complete `N / M`" form in §6.2 Human-path cells. The gate fails if N differs from the counted complete set for that shard, or if any "complete" claim appears in `user-interface.md` beyond the counted set.
- **Unit tests.** Extend `evidence_tests` (`:859–1160`).

**Docs:** `docs/extending.md` gets a "Writing a workflow test" entry, and `docs/pitfalls.md` gets a new §7 on existence-only assertions and seeded workflow tests.

### PR 3 — Missing graph controls and the control-gap sweep (`R13.10`)
Branch `feature/R13.10-graph-orientation-replicate-lines`.

**Step 1: sweep.** For every WF spec row, list each named feature and either cite its GUI widget id or log it as a gap. The output is a table in the audit record. This sweep is a precondition for PR 5, so gaps are filled rather than seeded.

**Controls.** All three reuse existing engine commands, so `UI_COMMANDS` and `docs/commands.md` are unchanged.
- **Orientation (GWF-01).**
  - In `dialogs_canvas/format_graph.rs` `FormatGraphState`, add `orientation` and `orientation_dirty` (both `#[serde(default)]`). Add them to `Default` and `sync_from_graph`, emit `orientation` in the `graph.setSpec` patch inside `commit_intents` only when dirty, and add the key to `apply_extended_format_graph_field`.
  - In `format_graph/series.rs` `render_fg_series_arrangement`, add a button row `format_graph.orientation.{vertical,horizontal}`, copied from the GroupedArrangement row (`:329-363`, using `record_button`). Gate it to `GraphKind::Column | Grouped`.
  - Optionally add a chip row in `trailing_studio/graph_inspector.rs`, copied from the arrangement chips at `:400-430`.
- **Show symbols and replicate lines (GWF-02).**
  - Add `show_symbols` and `replicate_lines` to the state, synced from the series GDO.
  - In `commit_intents`, send `"marker": null` when symbols are hidden; it currently always sends a marker object (`:497-503`), which would re-create one. Also add `replicate_lines` to `series_patch`.
  - Add a "Show symbols" checkbox `format_graph.show_symbols` next to "Show data points" (`series.rs:203`), using `record_checkbox`.
  - Add a chip row `format_graph.replicate_lines.{summary_center,individual_trajectories}`, gated to `ScatterAndLine` series.
  - Stop `views/inspector.rs:332` `retopologize_series` from resetting `replicate_lines`.
- **Right-axis assignment and nudge (GWF-17),** plus any other gaps the sweep finds, follow the same Format Graph pattern.
- **Labels.** Use `gtr()` headings with literal option labels, which is the modal's existing convention, or full 16-locale `entry!` keys in `i18n/r13_8_ui.rs`. Full keys also mean bumping the catalog count in `i18n/tests.rs:60`, `tests/i18n_theme.rs:192` and `tests/rv_ledger_third_pass_ui.rs:45`.
- **Tests.** Add unit tests in `dialogs_canvas` for sync, dirty and commit. Rewrite WF-64 and WF-65 (both human and `gui_wf`) on `WorkflowRun`, using these controls.

### PR 4 — Remaining GUI path gaps (`R10.27`)
Branch `feature/R10.27-layout-panels-theme-paths`.

- **Layout panels (GWF-15).**
  - An "Add graph panel" picker in the layout composer or canvas context menu, which emits `layout.addPanel` with the graph id and grid cell.
  - Corner resize handles on graph panels: wire `resizing_corner` in `canvas_view.rs:1480-1564`, writing `layout.setSpec` bounds the way the drag path does.
  - z-order controls, if the sweep shows WF-59 lacks one.
- **Theme and Preferences (GWF-16).** Record theme submenu options as `popup.menu.ui.theme.set.opt.{slug}`, mirroring the locale submenu at `top_chrome.rs:723`. Add a Preferences entry to the curated menu bar (`menus/menu_bar.rs`).
- **Factor titles.** Make Grouped row and column factor titles editable, if the sweep requires them (`headers.rs:342`, `:639`, `:715` are labels today).
- **Tests.** Add a human-path unit test per new control.

### PR 5 — Rewrite all 65 workflow tests on the contract (`R10.28`)
Planned split into three PRs on disjoint files, so parallel agents never share a file. Each agent uses its own `CARGO_TARGET_DIR` and worktree under `../.cb-worktrees/`.
- 5a covers WF-01–24 (`human_wf_01_06`, `07_12`, `13_18`, `19_24`).
- 5b covers WF-25–50 (`25_31`, `32_37`, `38_43`, `44_50`).
- 5c covers WF-51–65 (`51_58`, `59_65`), except WF-64 and WF-65, which are done in PR 3.

**Rules for each rewrite:**
- Start from an empty app.
- Use per-workflow data, taken from new `curvebook-testkit` fixtures where the shared MV seed does not fit the analysis (survival, Cox, GLM, 4PL, qPCR).
- Exercise every feature in the spec row, assert non-default settings, and include a live edit and `verify_reference`.
- Never wrap a click in `if find_by_widget_id(..)`.

**When a spec feature has no GUI path,** stop and log a GWF finding, then add the control in R13.10 or R10.27, or explicitly narrow the spec row in `gui-workflows.md` with a reason. Seeding is never the fallback.

**Also in this PR:**
- Retire or rename `r15_4_phase_a_*` (GWF-11) as layout gates.
- Fix WF-04's flow (File > Import menu to the paste wizard, comma delimiter, cell assertions) or rename it.
- Fix WF-62's replicate stepper.
- Update the §6.2 cells to "complete `N / M`" as each shard lands, so the gate enforces the counts.

### Final — Beta sign-off (`R10.25`)
- Run every suite on the candidate revision with the gate at 65/65 complete, and record the commands and revision.
- Set `R13.6` and `R10.25` to `done`.
- Restore "complete" wording across `ROADMAP.md`, `STATUS.md`, `gui-workflows.md` and `user-interface.md`.

## Order and dependencies

1. PR 1 lands first.
2. PR 2 is needed by PR 5.
3. PR 3 and PR 4 can run in parallel with PR 2, because they touch disjoint files (`format_graph*`, `canvas_view.rs` / `top_chrome.rs` versus `tests/` and `xtask`).
4. PR 5 shards start once PR 2 lands. They pick up controls from PR 3 and PR 4 as those land.
5. Sign-off comes last.

Rough agent-time estimates:
- PR 1: about 2 h.
- PR 2: about 1–2 days.
- PR 3: about 1 day, sweep included.
- PR 4: about 1–1.5 days.
- PR 5: about 1–2 h per workflow, plus gap fixes the sweep surfaces. This is the dominant cost.

## Verification

- **Each PR:** `cargo xtask ci quick`, plus `cargo xtask status`, `brands`, `cleanroom`, `pending`, `ui-commands --check` and `commands --check`.
- **PR 2:**
  - Gate self-tests with known-bad fixtures in `evidence_tests`: a seeded body, a guarded click, a missing `finish`, and a count mismatch.
  - Harness self-tests: a deliberately failing `live_edit`, a `Failed` result, an intent error, and a background-thread panic must each fail `finish()`.
  - The raster regression test `owner_dropped_on_its_own_worker_thread_does_not_self_join`.
- **PR 3 and PR 4:** unit tests for each control, plus the rewritten WF-64 and WF-65 on `WorkflowRun`.
- **PR 5 and sign-off:**
  - Run `cargo test -p curvebook-ui-egui` with `--test` for all ten `human_wf_*` suites, plus every `gui_wf_*` suite, `gui_only_workflows`, `gui_w22b_qol_and_r9` and `r15_4_gui_only_wf_egui`.
  - Expect 0 failed, 0 skipped, no background-thread panics in the log, and `cargo xtask status` reporting 65/65 complete.
  - Record the command and revision under `R10.25`.
