# GUI workflow completion audit (2026-11-21)

Checks whether every documented workflow (`WF-01`–`WF-65`, [`gui-workflows.md`](../gui-workflows.md)) can be completed through the drawn GUI, using the automated `EguiHumanPathDriver` suites only. No human or computer-use walkthrough was done, so this does not touch `R10.13` / `R11.10`.

## Run metadata

| Field | Value |
|---|---|
| Checked commit | `aa8c6f9` (`refactor/R15.9-visual-design-tokens`), clean tree |
| Suites run | `cargo test -p curvebook-ui-egui` with `--test` for `human_wf_{01_06,07_12,13_18,19_24,25_31,32_37,38_43,44_50,51_58,59_65}`, all nine `gui_wf_*`, `gui_only_workflows`, `gui_w22b_qol_and_r9`, `r15_4_gui_only_wf_egui` (macOS, debug profile) |
| Outcome | 23 suites, 0 failed, 0 skipped. Every `WF-01`–`WF-65` test passes on both the human-path driver and the agent harness |
| Status probe | A temporary `Drop` hook on `EguiWfHarness` logged every result sheet's `ResultStatus` at the end of each human-path test. All result sheets in all 65 workflow tests ended `UpToDate`; none `Failed` or `Dirty`. The probe was removed afterwards |
| Method | Static read of each human-path test against its spec row, plus the run and probe above. Each stage is classified as done through rendered-widget input, bypassed (engine command, `Session::from_document`, or direct app/view state write before or during the test), or missing |
| Agent | Claude Code; Claude Opus 5.5 (`claude-opus-5-5`), with three read-only sub-agents for the per-workflow comparison; their key claims were re-checked by hand |

## Verdict

Passing tests do not show that every workflow can be completed through the GUI as specified. Under the five-stage definition in [`gui-workflows.md` §6](../gui-workflows.md#6-human-path-status--blockers-r10) (create table → enter data → configure analysis → verify results and graph → live upstream edit), **no workflow test is complete**:

| Verdict | Count | Meaning |
|---|---|---|
| Complete | 0 | All five stages via GUI input, spec features exercised, results asserted beyond existence |
| Partial | 57 | All steps via GUI input, but spec features missing, stage 5 missing, and/or assertions only check that a sheet exists |
| Seeded | 8 | One or more stages bypass the GUI: `WF-04`, `WF-28`, `WF-55`, `WF-57`, `WF-59`, `WF-61`, `WF-64`, `WF-65` |

Two workflows cannot be completed through the GUI at all, because their defining control does not exist (GWF-01, GWF-02).

## Findings

| ID | Severity | Location | Summary | Status |
|---|---|---|---|---|
| GWF-01 | High | `crates/ui-egui/src` (absent); engine setter `crates/engine/src/cmd/graph.rs` `graph.setSpec` | No GUI control sets graph `CartesianOrientation::Horizontal`. The only "Orientation" control is page orientation. `WF-64` (population pyramid) seeds orientation with `graph.setSpec` and then asserts it | fixed (`a509690` `R13.10` `format_graph.orientation.*` + `inspector.graph.orientation.*`; verified `1a22cca`) |
| GWF-02 | High | `crates/ui-egui/src/dialogs_canvas/graph_type.rs`, `views/inspector.rs` (only `ReplicateLineMode::default()`) | No GUI control selects `ReplicateLineMode::IndividualReplicateTrajectories` or hides symbols. `WF-65` (spaghetti plot) seeds both with `graph.setSeries` and then asserts them. The per-subject single-Y half of the workflow is not attempted | fixed (`a509690` `R13.10` `format_graph.show_symbols` + `format_graph.replicate_lines.*`; verified `1a22cca`) |
| GWF-03 | High | `human_wf_01_06.rs`, `human_wf_25_31.rs`, `human_wf_51_58.rs`, `human_wf_59_65.rs` | Eight workflow tests seed stages outside the GUI: WF-04 pushes the import intent and writes the CSV draft into view state; WF-28 sets dark mode with `set_theme_mode`; WF-55, 61, 64 build Grouped tables, titles and data with `execute_command`; WF-57, 59 build documents, graphs, layouts and panels in code; WF-65 builds the replicate table and graph in code | fixed (`5d0add6`, `0d6125a`, `a509690`, `329a4cf` + Wave B un-seeded `0ceb5a5`, `952a32c`, `1a22cca`) |
| GWF-04 | High | all `human_wf_*.rs` | Stage 5 (live upstream edit, then recompute check) is missing in 53 of 65 tests: all of WF-01–50 except WF-06, plus WF-54, 55, 56, 57. WF-54 also has no configured analysis (the selective decay fit is never run; the test ends with no result sheet) | fixed (`d10f631` `R10.26` `WorkflowRun::live_edit` contract + Wave B `0ceb5a5`, `dd231ab`, `952a32c`, `163035b`, `1a22cca`) |
| GWF-05 | Medium | `human_wf_32_37.rs`, `human_wf_38_43.rs`, `human_wf_19_24.rs` | Assertions that cannot fail: WF-37 (`Ward`), WF-39 (`Efron`) and WF-40 (`Linear`) assert the enum `#[default]` (`crates/doc/src/analysis.rs`), so they pass even if the `Cmd+T` change never applied. WF-19 accepts `FisherExact` or `PearsonChiSquare`. No WF test asserts a numeric result | fixed (`dd231ab`, `952a32c`, `163035b` non-default options + `WorkflowRun::verify_reference` `D1` oracle checks) |
| GWF-06 | Medium | `human_wf_common/mod.rs` post-frame hook | Commit errors are stored in `last_intent_error`, which only `run_analyze_entry` reads. A failed `Cmd+T` → OK re-run rolls back to the previous result and the test still passes | fixed (`d10f631` `WorkflowRun` accumulates `intent_errors` and fails `finish()` on any error) |
| GWF-07 | Medium | `human_wf_51_58.rs`, `human_wf_59_65.rs` | Steps wrapped in `if find_by_widget_id(..).is_some()` / `if let Some` are skipped silently when the widget is absent | fixed (`d10f631` `xtask status` static AST lint + `1a22cca` removed all guarded clicks) |
| GWF-08 | Medium | see per-workflow table | Several tests do a different workflow from their spec row: WF-31 fits a 4PL instead of extract/rearrange; WF-34 has no qPCR calculated variables; WF-44 draws no ellipses or hulls; WF-48 has nothing bubble-related; WF-22 runs PCA only. Most MV workflows reuse one 4-column seed that does not match the analysis (survival, Cox, GLM, 4PL) | fixed (`d10f631` `wf_datasets` + `4399cdb`, `5d0add6` + Wave B rewrites `0ceb5a5`..`1a22cca`) |
| GWF-09 | Medium | `human_wf_*` uses of `graph_sheets.last()`; `welcome_hub.rs` `auto_graph` default | "Linked graph" checks use `graph_sheets.last()`, which is the table's Welcome auto-graph rather than a graph produced by the analysis (WF-08, 10, 12, 22, 24, 58), and may be in WF-25, 27, 28, 44, 48–50 | fixed (`0ceb5a5`, `dd231ab`, `952a32c`, `163035b`, `1a22cca` diff/verify analysis-produced graphs explicitly) |
| GWF-10 | Low | `nav.new.layouts`, `menu.insert.new_layout`, layout composer | Layout creation is exercised only through the command palette (WF-27), never through `nav.new.layouts` or the Insert menu, and no human-path test adds a graph panel through the GUI; WF-57 and WF-59 seed both | fixed (`0d6125a` `R10.27` + `952a32c` `WF-27` / `1a22cca` `WF-57`, `WF-59` via Insert/studio GUI) |
| GWF-11 | Low | `human_wf_01_06.rs`, `human_wf_25_31.rs` | `r15_4_phase_a_wf01_24_*` / `wf25_50_*` are named after workflow ranges but build their documents with engine commands; they are layout gates, not workflow evidence | fixed (`0ceb5a5` & `952a32c` renamed to `r15_4_phase_a_{column_anova,mv}_layout_and_14_locale_gate` and un-seeded) |
| GWF-12 | Medium | `crates/ui-egui/src/egui_shell/canvas_view.rs` `AsyncRasterWorkerOwner::drop` | The `cb-canvas-raster` worker panicked 30 times across the suites ("failed to join thread: Resource deadlock avoided"). An in-flight job holds an `egui::Context` clone and the context owns the worker handle, so the last drop could run on the worker thread and self-join. Hidden from test results because only the background thread panicked | fixed (`7116d92` + `211aa91`; `WorkflowRun::finish()` fails on any background panic; `0` panics at `816a3c7`) |
| GWF-13 | Low | `crates/ui-egui/tests/human_wf_01_06.rs` WF-04 | The paste wizard writes into the active sheet by design; the defect is that `wf04_human_import_reshape_and_convert_archetype` injects an import intent/view state instead of invoking File > Import and its name and flow disagree | fixed (`5d0add6` `ImportReshape` target/delimiter/paste/empty-doc controls + `0ceb5a5` `WF-04` menu flow) |
| GWF-14 | Low | `gui-workflows.md` §6.1, §6.2, §7; `ROADMAP.md` `R10.25`, `R13.6`; `STATUS.md` | "50 / 50" and "15 / 15" human-complete claims cite passing test names, which satisfies `cargo xtask status`, but the tests do not meet the §6 five-stage definition | fixed (`d10f631` `xtask status` enforces `WorkflowRun::start`+`.finish()` & `complete N / M`; Wave C sign-off at `65 / 65`) |
| GWF-15 | High | `crates/ui-egui/src/egui_shell/{canvas_view.rs,trailing_studio/page_layout.rs}` | No GUI path adds, positions, or resizes a graph panel in a layout. `layout.arrange` only auto-fills, and `resizing_corner` is never set in `canvas_view.rs` | fixed (`0d6125a` `R10.27` `studio.layout.add_panel.*`, `resizing_corner`, z-order + `r10_27_layout_theme_paths` & `1a22cca`) |
| GWF-16 | Medium | `crates/ui-egui/src/egui_shell/top_chrome.rs`, `crates/ui-egui/src/menus/menu_bar.rs` | `View > Theme` submenu options have no widget IDs, and there is no menu or toolbar entry for Preferences (`app.preferences`) | fixed (`0d6125a` `R10.27` `popup.menu.ui.theme.set.opt.*` + `app.preferences` menu item; verified `952a32c`) |
| GWF-17 | High | `crates/ui-egui/src/egui_shell/graph_modals/format_graph/series.rs` | `WF-54`'s right-axis assignment and horizontal nudge have no GUI control (`"Right Y2"` at `format_graph/series.rs` is a read-only label) | fixed (`a509690` `R13.10` `format_graph.series_select.*`, `series_axis.*`, `series_offset_x`; verified `1a22cca`) |
| GWF-18 | High | `crates/ui-egui/src/egui_shell/graph_modals/format_axes/segments_ticks.rs`, `trailing_studio/graph_inspector.rs` | No drawn controls for `AxisSegment.major_tick_interval` or `minor_ticks_per_interval` | fixed (`329a4cf` `axes.seg.0.{major_tick_interval,minor_ticks_per_interval}` + `r10_27_gap_sweep_controls`) |
| GWF-19 | High | `crates/ui-egui/src/egui_shell/transform_modals.rs` | `PruneRows`, `Transpose`, and standalone `ExtractAndRearrange` lacked GUI parameters / swaps | fixed (`329a4cf` `edit.transform.prune.*`, `transpose.swap_titles`, standalone extract params) |
| GWF-20 | Medium | `crates/ui-egui/src/egui_shell/stats_modals/{factorial_mv.rs,stat_params.rs}` | `edit.corr.heatmap`, `edit.diag.qq`, and `edit.diag.shapiro` checkboxes discarded clicks | fixed (`329a4cf` wired mutable toggle state + `r10_27_gap_sweep_controls`) |
| GWF-21 | Medium | `crates/ui-egui/src/egui_shell/{data_grid/headers.rs,graph_modals/mod.rs}` | Missing per-column Y duplicate UI action and heatmap `annotate_cells` / `cell_gap` modal persistence | fixed (`329a4cf` `grid.ctx.duplicate_column` + heatmap persistence in `graph_modals/mod.rs`) |

## Per-workflow classification

Stages: create / enter / configure / verify / live edit. `G` = GUI input, `B` = bypassed, `M` = missing, `W` = GUI input but the check is existence-only or circular, `–` = not applicable.

| WF | Human-path test | Stages | Main gaps against the spec row | Verdict |
|---|---|---|---|---|
| 01 | `human_wf_01_06::wf01_*` | G G G W M | No multi-row selection, Format Selected Points, TSV paste; exclusion happens before the fit | Partial |
| 02 | `wf02_*` | G G G W M | Log10(X) never configured; no multi-hop propagation | Partial |
| 03 | `wf03_*` | G G – W M | No graph-canvas brushing, Format Selected Points, Edit-menu Undo/Redo | Partial |
| 04 | `wf04_*` | G B M W M | Import opened by pushing an intent; CSV written to view state; no Plate Map / Tidy Long menus | Seeded |
| 05 | `wf05_*` | G G G W M | No PruneRows; PCA and clustering not chained; K-means++/Ward.D2 not chosen | Partial |
| 06 | `wf06_*` | G G G W W | Not chained; no Kaplan–Meier or imputation; post-edit check is existence only | Partial |
| 07 | `human_wf_07_12::wf07_*` | G G G W M | Student result and graph unchecked | Partial |
| 08 | `wf08_*` | G G G W M | Equal-N predicate and Before-After graph spec unchecked | Partial |
| 09 | `wf09_*` | G G G W M | Rank invariance needs the missing outlier edit | Partial |
| 10 | `wf10_*` | G G G W M | No Welch/Brown–Forsythe, Dunnett, banner; brackets unchecked | Partial |
| 11 | `wf11_*` | G G G W M | Dunn's never selected | Partial |
| 12 | `wf12_*` | G G G G M | Interaction and Type III unchecked (graph error-bar check is real) | Partial |
| 13 | `human_wf_13_18::wf13_*` | G G G W M | Huynh–Feldt never selected; correction unchecked | Partial |
| 14 | `wf14_*` | G G G W M | Banner, `inspectDiagnostics`, remediation rejection unchecked | Partial |
| 15 | `wf15_*` | – – G W M | Stage tabs instead of Next/Back; hover and sensitivity table unchecked | Partial |
| 16 | `wf16_*` | G G G W M | No Passing–Bablok, ANCOVA, R8; Deming unchecked | Partial |
| 17 | `wf17_*` | G G G W M | No 5PL, interpolation, correlation heatmap | Partial |
| 18 | `wf18_*` | G G G W M | Single data set, so no global shared Vmax | Partial |
| 19 | `human_wf_19_24::wf19_*` | G G G W M | Accepts either method; no R×C, R5/R6 | Partial |
| 20 | `wf20_*` | G G G W M | No AUC/Youden, ROC graph, Cochran–Armitage | Partial |
| 21 | `wf21_*` | G G G W M | No log-rank/HR values, logistic regression, R4 | Partial |
| 22 | `wf22_*` | G G G W M | PCA only: no clustering, GLM, volcano, Format Selected Points | Partial |
| 23 | `wf23_*` | G G G W M | No z-order or R10 checks; one hop of chaining | Partial |
| 24 | `wf24_*` | G G G W M | Tukey, brackets, stars, ticks, log axis, export unchecked | Partial |
| 25 | `human_wf_25_31::wf25_*` | G G G W M | Graph kind unchecked; other plot types and error bars absent | Partial |
| 26 | `wf26_*` | G G G W M | Correlation matrix only; no bubble/volcano/heatmap graphs | Partial |
| 27 | `wf27_*` | G M G W M | No data, panels, drag/snap, Apply Style, preflight | Partial |
| 28 | `wf28_*` | G M B W M | Dark mode set in code; export cancelled; no preset/CVD/print | Seeded |
| 29 | `wf29_*` | G G G W M | Default Select & Transform; no transforms, row stats, baseline | Partial |
| 30 | `wf30_*` | G G G W M | One formula; no function, syntax or cycle guards; values unchecked | Partial |
| 31 | `wf31_*` | G G G W M | Runs a 4PL fit; extract/rearrange absent | Partial |
| 32 | `human_wf_32_37::wf32_*` | G G G W M | No calculated column; Welch never confirmed | Partial |
| 33 | `wf33_*` | G G G W M | No 3-way design, Type III, Tukey; recast column leaves numeric levels | Partial |
| 34 | `wf34_*` | G G G W M | No qPCR calculated variables at all | Partial |
| 35 | `wf35_*` | G G G W M | Columns picked automatically; extra tests absent | Partial |
| 36 | `wf36_*` | G G G W M | Data is not dose–response; no constraints or IC50 CI | Partial |
| 37 | `wf37_*` | G G G W M | Asserts the default linkage; no dendrogram | Partial |
| 38 | `human_wf_38_43::wf38_*` | G G G W M | No k sweep, silhouette, elbow, scatter | Partial |
| 39 | `wf39_*` | G G G W M | Asserts the default tie method; no HR or curves | Partial |
| 40 | `wf40_*` | G G G W M | Asserts the default family; no predictors, Type III, VIF | Partial |
| 41 | `wf41_*` | G G G W M | No OR CIs, ROC, Hosmer–Lemeshow | Partial |
| 42 | `wf42_*` | G G G W M | No count outcome, dispersion, VIF | Partial |
| 43 | `wf43_*` | G G G W M | Constraint tabs opened, nothing set; no AUC | Partial |
| 44 | `human_wf_44_50::wf44_*` | G G G M M | No ellipses or hulls; no assertion | Partial |
| 45 | `wf45_*` | G G G W M | No heatmaps or cross-tabs | Partial |
| 46 | `wf46_*` | G G – W M | Opens Manage Variables only | Partial |
| 47 | `wf47_*` | G G G W M | Two arms; no number-at-risk or pairwise | Partial |
| 48 | `wf48_*` | G G G W M | Nothing bubble-related | Partial |
| 49 | `wf49_*` | G G G W M | Default 4PL; equation never asserted | Partial |
| 50 | `wf50_*` | G G G W M | CLD mode set, letters unchecked; no live edit | Partial |
| 51 | `human_wf_51_58::wf51_*` | G G G W G | ROUT badge, F-test, relative IC50 unchecked; guarded click | Partial |
| 52 | `wf52_*` | G G G W G | Ambiguous status, banner, QQ drill-back absent; guarded clicks | Partial |
| 53 | `wf53_*` | G G G G G | One direction per toggle; chi²/HR values unchecked | Partial |
| 54 | `wf54_*` | G G M W M | No dual axis, nudged bars, decay fit, live edit | Partial |
| 55 | `wf55_*` | B B G W M | Table and data seeded; Shift+Click made redundant by select-all | Seeded |
| 56 | `wf56_*` | G G G W M | No antilog axis or edit-invariance check | Partial |
| 57 | `wf57_*` | B B G W M | Document, graph, layout, panel seeded; crop/resize unchecked | Seeded |
| 58 | `wf58_*` | G G G W W | Slice contents unchecked; live-edit diff uses the auto-graph | Partial |
| 59 | `human_wf_59_65::wf59_*` | B B B G G | Graphs, kinds, layout, panels seeded | Seeded |
| 60 | `wf60_*` | G G G W G | Hover tooltip, red override, fitted-curve flag unchecked | Partial |
| 61 | `wf61_*` | B B G W G | Grouped table seeded; raw heatmap absent | Seeded |
| 62 | `wf62_*` | G G G W G | Nested df and variance test unchecked; guarded RM steps | Partial |
| 63 | `wf63_*` | G G G W G | Tick value and gridline style unchecked | Partial |
| 64 | `wf64_*` | B B B W G | Orientation seeded then asserted (GWF-01) | Seeded |
| 65 | `wf65_*` | B B B W G | Trajectory mode seeded then asserted (GWF-02) | Seeded |

## Post-remediation verification (Wave B / `R10.28`, `main = 816a3c7`)

All `21` findings (`GWF-01`–`GWF-21`) and Wave B modal/graph discovery gaps (`4399cdb`, `5d0add6`: `WF-04` import/paste, `WF-06` chained `analyzable_data` source, `WF-10` Dunnett control sync, `WF-15` `PowerWizard` edit-mode sync/slugs, `WF-19` `2×2` AR & CIs, `WF-32`/`34` MV `plot_mode`, `WF-36`/`43`/`52` fit `Constant` textboxes, `WF-45` heatmap color persistence, `WF-48` MV bubble color/size/clamping, `WF-51`/`54`/`60` `format_graph.overlay.{attach,clear}`, `WF-54` `series_gdo.*`, `WF-57` ruler clip bleed, and `WF-61` normalize percent/create_graph) were resolved and integrated onto `main` at `816a3c7`:

| Post-remediation metric | Observed result (`main = 816a3c7`) |
|---|---|
| Complete / Partial / Seeded (`WF-01`–`WF-65`) | **`65` Complete / `0` Partial / `0` Seeded** (`WorkflowRun::start` -> `.finish()` across `human_wf_01_06`..`human_wf_59_65`) |
| `human_wf_*` + `gui_wf_*` + Wave A/B suites (`30` binaries) | **`235 passed, 0 failed, 0 ignored`** (`0` background `panicked at` events; includes `23` GUI workflow suites = `200 passed` and `7` Wave A/B suites = `35 passed`) |
| `cargo xtask status` | Exit `0` (`65/65 egui; backed human tests: 65/65 egui; five-stage complete: 65/65`, zero test lint violations) |

## Suggested follow-up (completed in Wave A–C)

1. Added missing GUI controls for `GWF-01`, `GWF-02`, `GWF-15`–`GWF-21`, and Wave B gaps (`a509690`, `0d6125a`, `329a4cf`, `4399cdb`, `5d0add6`).
2. Replaced all 8 seeded tests with Welcome-hub, grid, Import/Insert, Preferences/Theme, and layout-composer GUI flows (`0ceb5a5`, `952a32c`, `1a22cca`).
3. Enforced 5-stage `WorkflowRun` execution (`ResultStatus::UpToDate`, `intent_errors == 0`, `panicked at == 0`, `verify_reference` `D1` oracles, and `live_edit`) across all `65 / 65` tests (`d10f631`, `0ceb5a5`, `dd231ab`, `952a32c`, `163035b`, `1a22cca`).
4. Removed all silent guarded `find_by_widget_id(` skips and circular default-enum assertions (`xtask status` static AST lint + Wave B rewrites).
5. Signed off `R10.25`, `R10.28`, and `R13.6` at `65 / 65` complete in `gui-workflows.md`, `ROADMAP.md`, `STATUS.md`, and `evidence/user-interface.md` (`R10.13` / `R10.24` manual walkthroughs remain `1.0`).

---

## 65-Workflow GUI Control-Gap Sweep (`R13.10`, 2026-11-21)

Audit of every workflow `WF-01`–`WF-65` in `docs/gui-workflows.md` against drawn widget IDs in `crates/ui-egui/src/egui_shell/`, `crates/ui-egui/src/menus/menu_bar.rs`, and the verified Wave B `WorkflowRun` suites (`human_wf_01_06.rs`–`human_wf_59_65.rs`).

### Additional Control Gaps Discovered in Sweep (all resolved)

| ID | Affected WFs | Component / File | Finding | Disposition |
|---|---|---|---|---|
| `GWF-18` | `WF-24`, `WF-56`, `WF-63` | `crates/ui-egui/src/egui_shell/graph_modals/format_axes/segments_ticks.rs`, `trailing_studio/graph_inspector.rs` | No drawn controls for `AxisSegment.major_tick_interval` or `minor_ticks_per_interval` (`WF-56` "9 log minor ticks", `WF-24`/`WF-63` tick spacing). | Fixed in `329a4cf` (`r10_27_gap_sweep_controls`, `4 passed`); verified in `WF-24`, `WF-56`, `WF-63`. |
| `GWF-19` | `WF-06`, `WF-31`, `WF-55` | `crates/ui-egui/src/egui_shell/transform_modals.rs` | `PruneRows` hardcoded `every_k_rows: 2, average_window: false`; `Transpose` lacked `swap_row_and_column_titles`; standalone `ExtractAndRearrange` dropped target/col params. | Fixed in `329a4cf`; verified in `WF-06`, `WF-31`, `WF-55`. |
| `GWF-20` | `WF-17`, `WF-26` | `crates/ui-egui/src/egui_shell/stats_modals/factorial_mv.rs`, `stat_params.rs` | `edit.corr.heatmap` hardcoded `true`; `edit.diag.qq` & `edit.diag.shapiro` discarded clicks. | Fixed in `329a4cf`; verified in `WF-17`, `WF-26`. |
| `GWF-21` | `WF-45`, `WF-60` | `crates/ui-egui/src/egui_shell/data_grid/headers.rs`, `graph_modals/mod.rs` | Missing per-column Y duplicate action and heatmap `annotate_cells` / `cell_gap` modal persistence. | Fixed in `329a4cf` (`grid.ctx.duplicate_column` + `graph_modals/mod.rs` persistence) & `4399cdb`; verified in `WF-45`, `WF-60`. |
| *(R13.10 Format Graph)* | `WF-63`, `WF-64` | `dialogs_canvas/format_graph.rs`, `graph_modals/format_graph/series.rs` | Missing Format Graph bar `border_width_pt` (`WF-64` `0.5 pt` + `Apply Globally`) and diagnostic forest `forest_show_pooled_diamond` toggle (`WF-63`). | Fixed in `a509690` (`R13.10`); verified in `1a22cca` (`WF-63`, `WF-64`). |

### Per-Workflow Feature-to-Widget Sweep (`WF-01`–`WF-65`, aligned to `docs/gui-workflows.md`)

| WF | Spec Features (from `docs/gui-workflows.md`) | Verified Drawn Widget IDs (Post-Rework `816a3c7`) |
|---|---|---|
| `WF-01` | XY dose-response; `grid.xcell.*` + TSV paste; Shift/Meta multi-row selection -> `Format Selected Points...`; `ExcludeWithReason` | `welcome.arch.xy`, `grid.xcell.*`, `grid.cell.*`, `inspector.table.style_subset`, `fmt_pts.col.Red`, `fmt_pts.sym.Diamond`, `exclude.preset.*`, `exclude.note` |
| `WF-02` | XY Normalize -> `ResultSheet` (`analyzable_data`) -> direct `Analyze...` `Log10(X)` transform chaining + Descriptive/Linear | `welcome.arch.xy`, `analyze.entry.transform.normalize`, `analyze.entry.transform.math`, `edit.transform.x_transform.log10`, `analyze.entry.{stats.descriptive,fit.linear}` |
| `WF-03` | Column 3-group entry; graph canvas brushing; `Format Selected Points...`; `Cmd+Z`/`Cmd+Shift+Z` & Edit Undo/Redo | `welcome.arch.column`, `menu.ui.changeGraphType.open`, `graph_type.card.col.bar_replicates`, `canvas.viewport`, `menu.ui.formatPoints.open`, `menu.edit.{undo,redo}` |
| `WF-04` | `File > Import`, `ImportReshape` (`ui.import.reshape.open`), 96-well `Plate Map...` & `Tidy Long Table...`, `Convert Table Structure...` | `menu.file.import`, `menu.table.{importPlateMap,importTidyLong}`, `import_paste.{target.*,delim.*,btn.paste_clipboard,input.*}`, `convert.*` |
| `WF-05` | `Transform/PruneRows` -> `ResultSheet` PCA -> `pca-loadings` tab -> chained Cluster Analysis (`K-means++` / `Ward.D2`) | `welcome.arch.multiple_variables`, `analyze.entry.transform.prune`, `edit.transform.prune.average_window`, `analyze.entry.stats.pca`, `analyze.entry.stats.clustering` |
| `WF-06` | `PruneRows` (`every_k`, block avg) -> chained Descriptive + Kaplan–Meier (`analyzable_data`) + Multiple Imputation + reactive edit | `analyze.entry.transform.prune`, `edit.transform.prune.every_k.{plus,minus}`, `analyze.entry.stats.{descriptive,survival_km}`, `edit.km.{time,status,group}.*` |
| `WF-07` | Unpaired Student's & Welch's *t*-test with `R4` & `R9` modal predicates + linked graph | `welcome.arch.column`, `analyze.entry.stats.two_group`, `edit.two_group.{graph_differences,welch}`, `edit.conf.99`, `edit.pval.custom`, `edit.pvalue.digits.dec` |
| `WF-08` | Paired *t*-test equal-$N$ `R1` check, `R4` difference/Welch controls, Before-After graph | `welcome.arch.column`, `analyze.entry.stats.two_group`, `edit.paired_or_matched`, `edit.two_group.{graph_differences,welch}`, `menu.ui.graph.create.open` |
| `WF-09` | Mann–Whitney $U$ (rank invariance under outlier edit) & Wilcoxon matched-pairs signed-rank | `welcome.arch.column`, `analyze.entry.stats.two_group`, `edit.assume_gaussian`, `edit.two_group.{hodges_lehmann,graph_ranks,welch}`, `edit.paired_or_matched` |
| `WF-10` | One-way ANOVA; `AssumptionDiagnosticBanner` (`ui.result.inspectDiagnostics`); `Cmd+T` Brown–Forsythe/Welch; `R2`–`R3` Dunnett/Tukey + brackets | `analyze.entry.stats.one_way_anova`, `result.diagnostic_banner.inspect`, `edit.{tab.multiple_comparisons,compare_against_control,control_column}`, `pairwise.*` |
| `WF-11` | Kruskal–Wallis + Dunn's multiple comparisons (`R2` nonparametric post-hoc swap) -> box plot & brackets | `welcome.arch.column`, `analyze.entry.stats.one_way_anova`, `edit.assume_gaussian`, `edit.post_hoc.dunn`, `graph_type.card.col.box_whisker`, `pairwise.auto_add` |
| `WF-12` | Two-way ANOVA on Grouped replicates, interaction effect (Type III vs. Type I SS), & grouped error-bar guard | `welcome.arch.grouped`, `analyze.entry.stats.factorial_anova`, `edit.fac.{ss_type_i,ss_type_iii}`, `edit.post_hoc.sidak`, `graph_type.card.grouped.*` |
| `WF-13` | Repeated-Measures ANOVA (`R1` enabled `Repeated Measures` tab) with Geisser–Greenhouse & Huynh–Feldt sphericity | `welcome.arch.column`, `welcome.design.paired`, `analyze.entry.stats.one_way_anova`, `edit.paired_or_matched`, `edit.sphericity.geisser_greenhouse`, `edit.rm.assume_sphericity` |
| `WF-14` | Friedman nonparametric RM-ANOVA, diagnostics-only banner, deprecated `applyRemediation` rejection, explicit `Cmd+T` | `analyze.entry.stats.one_way_anova`, `result.diagnostic_banner.{inspect,remediate}`, `menu.ui.result.{inspectDiagnostics,applyRemediation}`, `edit.anova.friedman` |
| `WF-15` | Full 4-stage `PowerAndSampleSize` wizard (`next`/`back`/`create`), Stage 4 curve hover, sensitivity table, `Cmd+T` re-entry | `power.family.{one_way_anova,two_sample_t}`, `power.one_sided`, `power.effect_mode.direct`, `power.stage3.power.*`, `power.tab.{curve,sensitivity}`, `power.curve_hover` |
| `WF-16` | Simple linear, Deming, Passing–Bablok regression, ANCOVA, and `R8`/`R9` predicates | `welcome.arch.xy`, `analyze.entry.fit.linear`, `edit.tab.{options,diagnostics}`, `edit.pval.*`, `edit.pvalue.digits.*`, `edit.lin.conf_bands` |
| `WF-17` | Dose–Response 4PL/5PL nonlinear fit + unknowns interpolation + Pearson/Spearman Correlation Matrix (`R7` heatmap) | `analyze.entry.fit.nonlinear`, `fit.eq.item.log_agonist_vs_response_5pl`, `fit.out.{interp,conf_bands}`, `edit.corr.heatmap` (`GWF-20` fixed) |
| `WF-18` | Michaelis–Menten enzyme kinetics with global shared `Vmax` fit & five-gate regression diagnostics | `welcome.arch.xy`, `analyze.entry.fit.nonlinear`, `fit.eq.item.michaelis_menten`, `fit.con.rule.Vmax.shared`, `fit.out.{normality,cov_tab}` |
| `WF-19` | $2 \times 2$ Contingency (Fisher's exact, Baptista–Pike OR, Koopman RR, Newcombe RD, `R5`/`R6`) & $R \times C$ Chi-square | `welcome.arch.contingency`, `analyze.entry.stats.contingency`, `edit.cont.{fisher,or_rr,sens_spec}`, `edit.contingency.{attributable_risk,ci.*}` |
| `WF-20` | ROC curve analysis (AUC, Youden's $J$, ROC graph) & $2 \times 3$ ordered Contingency `Cochran–Armitage` trend test (`R5`) | `analyze.entry.stats.roc`, `edit.roc.hanley`, `graph_type.card.diag.roc`, `analyze.entry.stats.contingency` |
| `WF-21` | Kaplan–Meier 1- vs. 2-group survival (Log-Rank, HR, Gehan–Breslow, Number-at-Risk) & Multiple Logistic Regression (`R4`) | `welcome.arch.survival`, `analyze.entry.stats.survival_km`, `edit.km.gehan`, `canvas.km_number_at_risk`, `analyze.entry.stats.glm` |
| `WF-22` | Multivariate PCA, Hierarchical Clustering, Multiple Linear / GLM, Multiple row *t*-tests + Volcano Plot + `Format Selected Points...` | `analyze.entry.stats.{pca,clustering,glm}`, `edit.pca.cov`, `edit.cluster.link.complete`, `edit.glm.{resp,pred}.*` |
| `WF-23` | Grouped `Stacked` vs. `Superimposed` z-order, 2-hop `ResultSheet` `Analyze...` chaining, leakage-safe PCR + `R10` guard | `welcome.arch.grouped`, `graph_type.card.grouped.{stacked,superimposed}`, `analyze.entry.stats.pcr`, `graph.setSpec` |
| `WF-24` | Column ANOVA + Tukey -> significance bracket auto-stars, callouts, axis ticks/log config, SVG/PNG, `R1`/`R2` graying | `edit.post_hoc.tukey`, `inspector.graph.more_pairwise`, `pairwise.auto_add`, `axes.scale.log10`, `axes.seg.0.{major_tick_interval,minor_ticks_per_interval}` (`GWF-18` fixed) |
| `WF-25` | Violin / Box-and-Whisker / Bar-with-Replicates & Aligned Dot (`ChangeGraphType` + `FormatGraph`), scatter overlay & SD/SEM/95%-CI error bars | `graph_type.card.col.{box_whisker,violin,bar_replicates}`, `graph_type.eb.sem`, `format_graph.{apply_globally,half_violin_with_points}` |
| `WF-26` | Bubble Plot / Volcano Plot / Correlation Heatmap + PCR (`R7`–`R10` inspect) | `analyze.entry.stats.corr_matrix`, `edit.corr.{all_pairs,heatmap}` (`GWF-20`), `format_graph.heatmap.{cmap.Plasma,annotate_cells}` (`GWF-21`), `analyze.entry.stats.pcr` |
| `WF-27` | Multi-panel `LayoutSheet` creation (`Insert` / `Add graph panel`), live pointer drag & corner resize (`R10.27`), `ApplyStyleFrom`, preflight | `studio.layout.add_panel.*`, `canvas.layout_panel.*`, `menu.ui.applyStyleFrom.open`, `apply_style.donor.*` (`GWF-10`, `GWF-15` fixed) |
| `WF-28` | Journal canvas preset, non-mutating CVD preview, `ActiveDialog::Print` & SVG/PDF/PNG export, Light/Dark theme menu (`R10.27`) | `menu.ui.canvas.applyPreset`, `menu.ui.previewFilter.set`, `menu.ui.print.open`, `popup.menu.ui.theme.set.opt.*`, `menu.file` / `app.preferences` (`GWF-16` fixed) |
| `WF-29` | Chained math transforms, `SelectAndTransform` derived formulas, per-row stats, baseline subtraction, `Select Rows` filter | `analyze.entry.transform.{select,math,baseline}`, `edit.transform.{derived_title,formula,add_derived,filter_formula,baseline_col.*}` |
| `WF-30` | `MultipleVariables` formula-bar `Calculated Variables`, syntax & circularity error guards, read-only cells, reactive edits | `inspector.mv.{col.*,formula_input,apply_formula}`, `analyze.entry.stats.descriptive` |
| `WF-31` | `SelectAndTransform` (`ExtractAndRearrange` `GWF-19` fixed) reshaping MV master table -> chained XY / Column / Grouped / Contingency | `analyze.entry.transform.select`, `edit.tab.{select_rows,select_cols,target_format}`, `edit.transform.{filter_formula,col.*,target_archetype.*}` |
| `WF-32` | MV calculated difference (`=D-C`) + direct Welch's *t*-test + `MultipleVariables` `CategoricalSummary` graph (`plot_mode`) | `analyze.entry.stats.two_group`, `edit.tg.welch_t`, `edit.compute_effect_sizes`, `format_graph.mv.plot_mode.categorical_summary` (`4399cdb`) |
| `WF-33` | Direct 2-way/3-way `Multifactor ANOVA` from MV (w/ `LOG10` calc col), Type III SS, $\eta^2$/$f$, Tukey (`R9.12`), outlier inspection | `analyze.entry.stats.factorial_anova`, `edit.fac.{ss_type_iii,order.3,resp.*,col.*}`, `edit.compute_effect_sizes`, `edit.post_hoc.tukey` |
| `WF-34` | Livak–Schmittgen $2^{-\Delta\Delta C_t}$ qPCR MV formulas -> direct Welch *t*-test + `BubblePlot` `CategoricalSummary` (`4399cdb`) | `inspector.mv.*`, `analyze.entry.stats.two_group`, `edit.tg.welch_t`, `menu.ui.graph.bubblePlot.open`, `bubble.plot_mode.categorical_summary` |
| `WF-35` | Direct `Kaplan–Meier` from long-format MV (`Time`, `Status`, `Grouping` + calc col), Log-rank/Gehan/Trend & Holm–Šídák pairwise | `analyze.entry.stats.survival_km`, `edit.km.{time,status,group}.*`, `edit.km.{mantel_cox,gehan,trend,asym_ci}`, `edit.post_hoc.holm_sidak` |
| `WF-36` | Direct MV 4PL dose–response w/ calc cols, `X`/`Y`/`Grouping` roles, `Bottom=0`/`Top=100` (`fit.con.const.*` `5d0add6`), profile IC50 CIs | `analyze.entry.fit.nonlinear`, `fit.eq.item.log_inhibitor_vs_response_4pl`, `fit.mv.*`, `fit.con.rule.{Bottom,Top}.fixed`, `fit.con.const.{Bottom,Top}` |
| `WF-37` | Agglomerative `Hierarchical Clustering` (`Euclidean`/`Manhattan`/`Pearson`/`Cosine` × 7 linkages), dendrogram & heatmap | `analyze.entry.stats.clustering`, `edit.cluster.{dist.manhattan,link.complete}`, `graph_type.family.mv_diag`, `graph_type.card.diag.heatmap` |
| `WF-38` | `K-Means` clustering across $k$ sweeps, Silhouette & Elbow curves, append cluster column, scatter + ellipses/hulls | `analyze.entry.stats.clustering`, `edit.cluster.{k_means_auto,k_sweep_max.inc}`, `toolbar.append_cluster`, `format_graph.mv.{x_next,y_next}` |
| `WF-39` | Multivariable `Cox proportional hazards`, categorical ref levels, `Efron` vs. `Breslow` ties (`GWF-05`), HR, AIC, survival curves | `analyze.entry.stats.survival_cox`, `edit.cox.{pred.*,breslow}`, `table.setCategoricalOrder`, `graph_type.card.survival.km` |
| `WF-40` | `Multiple linear regression` (`GLM linear`), Type III ANOVA, partial *t*, `VIF`, residual diagnostics (`QOL-7`), Actual vs. Predicted | `analyze.entry.stats.glm`, `edit.glm.{pred.*,linear}`, `table.setCategoricalOrder`, `graph_type.card.diag.residual` |
| `WF-41` | `Multiple logistic regression` (`GLM logistic`), ref levels, profile `95%` OR CIs, `ROC` AUC, Hosmer–Lemeshow, row predictions | `analyze.entry.stats.glm`, `edit.glm.logistic`, `table.setCategoricalOrder`, `analysis.run`, `graph.create` |
| `WF-42` | Log-link `Poisson` count GLM, ref levels, IRRs $\exp(\beta)$, deviance/dispersion/pseudo-$R^2$, residual & `VIF` pruning | `analyze.entry.stats.glm`, `edit.glm.poisson`, `table.setCategoricalOrder`, `analysis.run`, `graph.create` |
| `WF-43` | Serial dilution $\log_{10}(X)$ XY -> constrained vs. unconstrained 4PL (`Bottom=0`/`Top=1` `fit.con.const.*` `5d0add6`), $\text{IC}_{50}$ CIs, AUC | `analyze.entry.transform.math`, `edit.transform.x_transform.log10`, `fit.con.rule.{Bottom,Top}.fixed`, `fit.con.const.{Bottom,Top}` |
| `WF-44` | Pooled & per-group `Confidence Ellipses` (`95%` vs. `50%`) & `Convex Hull` polygons on MV scatter + continuous guards & live exclusion | `toolbar.append_cluster`, `format_graph.mv.{color_categorical,confidence_ellipses,convex_hulls}`, `exclude.*` |
| `WF-45` | Grouped multi-row heatmap (`Viridis`/`diverging`, borders, annotations, `45°` labels `GWF-21` + `4399cdb`) & MV correlation heatmap | `welcome.arch.grouped`, `graph_type.card.diag.heatmap`, `format_graph.heatmap.{cmap.*,reversed,min,max,annotate_cells,cell_gap_*,angle.*}` |
| `WF-46` | Non-modal MV `BubblePlotInspector` / `FormatGraph` (`Label`/`Continuous`/`Categorical` guards), live `X`/`Y`/`Color`/`Size` swap, duplicate & drill-back | `format_graph.mv.{x_next,y_next,color_categorical,color_continuous,size_variable}`, `nav.ctx.duplicate`, `inspector.drill_back.*` |
| `WF-47` | 3-arm `Survival` Kaplan–Meier w/ X-coupled `Number at Risk` (re-bins on major tick change `GWF-18`), `95% CI` band, `Y=50%`, Holm–Šídák | `analyze.entry.stats.survival_km`, `edit.km.trend`, `edit.post_hoc.holm_sidak`, `axes.seg.0.major_tick_interval` |
| `WF-48` | MV bubble sizing (`uniform`, proportional/bounded `min_radius`–`max_radius`, 3-pt legend, color/size col steppers, OOR clamp/omit `4399cdb`) | `inspector.graph.more_bubble`, `bubble.{color_col_next,size_variable,size_col_next,min_radius,max_radius}`, `format_graph.mv.{color_col_next,size_col_next,color_clamp_*,oor_*}` |
| `WF-49` | One-phase exponential decay & 3PL w/ `95%` bands + live-substituted $\TeX$ model equation annotations (`R9.16`) across RGBA/SVG/PDF | `analyze.entry.fit.nonlinear`, `fit.eq.item.one_phase_decay`, `fit.out.pred_bands`, `inspector.graph.add_equation`, `eq.mode.substituted`, `eq.bordered` |
| `WF-50` | 3-group Column (`Tukey` + Kruskal–Wallis `Dunn's`) -> `PairwiseComparisons` Piepho (2004) CLD (`[b, ab, a]` -> `[c, a, b]`), `Cmd+Z`/`Y`, RGBA/SVG/PDF | `edit.post_hoc.tukey`, `inspector.graph.more_pairwise`, `pairwise.mode.CompactLetterDisplayOnly`, `pairwise.{cld_uppercase,cld_ordering,auto_add}` |
| `WF-51` | Triplicate XY `4PL` + `ROUT (Q=1%)` + `format_graph.overlay.attach.0` (`4399cdb`), dual interp tabs, `Cmd+T` Extra-SS *F* (`3PL` vs. `4PL`), `Flag R²`, chained `Normalize` rel. IC50 | `fit.eq.item.log_inhibitor_vs_response_4pl`, `fit.method.rout`, `fit.cmp.{extra_f,alt.log_inhibitor_vs_response_3pl}`, `format_graph.overlay.attach.*` |
| `WF-52` | Sparse-dose `4PL` `Identify unstable parameters` (`Ambiguous`), `Constant(0)` + `SharedGlobal` recovery (`fit.con.const.Bottom` `5d0add6`), `QQ` drill-back | `fit.out.unstable`, `fit.con.rule.Bottom.fixed`, `fit.con.const.Bottom`, `fit.con.rule.{Top,HillSlope}.shared`, `inspector.drill_back.jump` |
| `WF-53` | 3-arm numeric `Survival` (`100%`-censored arm + spacers), `Staircase`/`PointToPoint`, `% Survival`/`% Event`, Greenwood SE/`95% CI`, whole-column `Cmd+E` -> HR | `analyze.entry.stats.survival_km`, `graph_type.survival.{metric.pct_*,point_to_point,staircase,err.*}`, `grid.cell.*`, `Cmd+E` |
| `WF-54` | Dual-Y `XY` (`MeanSdN`) right-axis `BarWithReplicates` (`series_gdo.bar_replicates` `4399cdb`) + horizontal nudge (`GWF-17` `series_offset_x`) + `50%` alpha + single-series decay | `analyze.mv.var.1`, `fit.eq.item.one_phase_decay`, `axes.enable_y2`, `format_graph.{alpha.50,overlay.attach.0,series_select.*,series_axis.*,series_offset_x,series_gdo.*}` |
| `WF-55` | Multi-site `Grouped` `Transpose` (`swap_titles` `GWF-19`) -> `Create one graph per dataset` (`12`) -> Gallery `Shift+Click` + `Apply Style From` + 2-hop edit | `analyze.entry.transform.transpose`, `edit.transform.transpose.swap_titles`, `create_graph.{one_per_data_set,y_axis_title}`, `gallery.*`, `apply_style.*` |
| `WF-56` | `Transform` `log10(Y)` -> `Antilog` Y-axis & `9` log minor ticks (`GWF-18`) -> `FitParams > Range` (`X < 10.0` ignored) + invariance vs. live recompute | `edit.transform.y_transform.log10`, `axes.tick_format.antilog`, `axes.seg.0.minor_ticks_per_interval`, `fit.tab.range` |
| `WF-57` | `2 × 2` `LayoutSheet` importing/pasting image (`R13.7`), corner resize (`Shift` free aspect), rotate, `Format Image` 4-edge crop, Rulers (`4399cdb` clip fix) + Canvas Grid non-export | `menu.layout.create`, `studio.layout.add_panel.*`, `canvas.image.handle.{se,rotate}`, `canvas.layout_ctx.format_image`, `format_image.crop.*`, `ui.{rulers,gridOverlay}.toggle` |
| `WF-58` | Drag-copy rectangular Result slice -> `Paste Table` on `GraphSheet` -> `Format Embedded Table` (`0 pt` borders, `Transpose`) + reactive edit | `result.cell.*`, `result.copy_selection`, `canvas.graph_ctx.paste_table`, `embedded_table.{transpose,outer_border.0,gridline.0}` |
| `WF-59` | `BeforeAfterLines` over axis-free transparent `BoxAndWhisker` via `Equalize Scaling` (`GWF-15` fixed) + origin snap + foreground inset (`Bring to Front`) | `graph_type.card.col.{before_after,box_whisker}`, `format_graph.paper_fill.transparent`, `axes.tick_dir.none`, `ui.layout.equalizeScaling.open`, `studio.layout.*` |
| `WF-60` | Canvas hover tooltip (`Row`) -> `grid.ctx.duplicate_column` (`GWF-21`) + `Cmd+E` exclusions -> selective linear fit (`overlay.attach.0`) + red `Format Selected Points` | `canvas.viewport`, `grid.ctx.duplicate_column`, `analyze.mv.var.0`, `format_graph.{show_raw_data,show_fitted_curve,overlay.attach.0}`, `fmt_pts.col.Red` |
| `WF-61` | Grouped raw `Viridis` heatmap vs. `Analyze > Normalize` (`0%`–`100%` per dataset `5d0add6`, `create_graph`) -> unclipped `Vertical (90°)` column titles | `welcome.arch.grouped`, `graph_type.card.diag.heatmap`, `edit.transform.{norm_percent,norm_per_dataset,normalize.create_graph}`, `format_graph.heatmap.*` |
| `WF-62` | Five-design clinical comparison (`Nested` table + `Nested t-test`, matched `Paired t-test`, `BothFactorsMatched` vs. `RowFactorMatched` 2-Way RM ANOVA) | `welcome.arch.nested`, `analyze.entry.stats.nested`, `edit.nested.t`, `welcome.arch.column`, `edit.tg.paired_t`, `welcome.arch.grouped`, `edit.fac.*` |
| `WF-63` | Direct `Forest Plot` from `Grouped` `UpperLowerLimits` (`pooled_diamond` `R13.10` toggle) + `Format Axes > Additional Ticks` `X = 1.0` dotted ref line | `welcome.subcol.upper_lower`, `graph_type.card.diag.forest`, `format_graph.forest_show_pooled_diamond`, `axes.additional_ticks.open`, `axes.add_tick.*` |
| `WF-64` | Two-cohort `Horizontal Stacked Bars` (Population Pyramid, `GWF-01` `orientation.horizontal`), `"All Data Sets"` `0.5 pt` border (`R13.10`), unsigned X ticks | `grid.row_titles.toggle`, `graph_type.card.grouped.stacked`, `format_graph.orientation.horizontal`, `format_graph.{apply_globally,bar_border_width_pt}`, `axes.tick_format.abs_decimal` |
| `WF-65` | Longitudinal `XY` **Spaghetti Plots** (`GWF-02` `replicate_lines.individual_trajectories` & `!show_symbols` `R13.10`) vs. single-Y subject traces w/ editable headers | `welcome.subcol.raw`, `format_graph.{apply_globally,show_symbols,replicate_lines.individual_trajectories}`, `canvas.viewport`, editable column headers |
