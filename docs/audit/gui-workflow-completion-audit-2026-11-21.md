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
| GWF-01 | High | `crates/ui-egui/src` (absent); engine setter `crates/engine/src/cmd/graph.rs` `graph.setSpec` | No GUI control sets graph `CartesianOrientation::Horizontal`. The only "Orientation" control is page orientation. `WF-64` (population pyramid) seeds orientation with `graph.setSpec` and then asserts it | open |
| GWF-02 | High | `crates/ui-egui/src/dialogs_canvas/graph_type.rs`, `views/inspector.rs` (only `ReplicateLineMode::default()`) | No GUI control selects `ReplicateLineMode::IndividualReplicateTrajectories` or hides symbols. `WF-65` (spaghetti plot) seeds both with `graph.setSeries` and then asserts them. The per-subject single-Y half of the workflow is not attempted | open |
| GWF-03 | High | `human_wf_01_06.rs:142–151`, `human_wf_25_31.rs:126`, `human_wf_51_58.rs:266–283`, `:381–403`, `human_wf_59_65.rs:64–114`, `:223–245`, `:449–474`, `:522–545` | Eight workflow tests seed stages outside the GUI: WF-04 pushes the import intent and writes the CSV draft into view state; WF-28 sets dark mode with `set_theme_mode`; WF-55, 61, 64 build Grouped tables, titles and data with `execute_command`; WF-57, 59 build documents, graphs, layouts and panels in code; WF-65 builds the replicate table and graph in code | open |
| GWF-04 | High | all `human_wf_*.rs` | Stage 5 (live upstream edit, then recompute check) is missing in 53 of 65 tests: all of WF-01–50 except WF-06, plus WF-54, 55, 56, 57. WF-54 also has no configured analysis (the selective decay fit is never run; the test ends with no result sheet) | open |
| GWF-05 | Medium | `human_wf_32_37.rs:76–93`, `human_wf_38_43.rs:24–58`, `human_wf_19_24.rs:27–30` | Assertions that cannot fail: WF-37 (`Ward`), WF-39 (`Efron`) and WF-40 (`Linear`) assert the enum `#[default]` (`crates/doc/src/analysis.rs`), so they pass even if the `Cmd+T` change never applied. WF-19 accepts `FisherExact` or `PearsonChiSquare`. No WF test asserts a numeric result | open |
| GWF-06 | Medium | `human_wf_common/mod.rs` post-frame hook | Commit errors are stored in `last_intent_error`, which only `run_analyze_entry` reads. A failed `Cmd+T` → OK re-run rolls back to the previous result and the test still passes | open |
| GWF-07 | Medium | `human_wf_51_58.rs:80`, `:134–139`, `:292`, `:329`; `human_wf_59_65.rs:254`, `:372`, `:381`, `:494` | Steps wrapped in `if find_by_widget_id(..).is_some()` / `if let Some` are skipped silently when the widget is absent | open |
| GWF-08 | Medium | see per-workflow table | Several tests do a different workflow from their spec row: WF-31 fits a 4PL instead of extract/rearrange; WF-34 has no qPCR calculated variables; WF-44 draws no ellipses or hulls; WF-48 has nothing bubble-related; WF-22 runs PCA only. Most MV workflows reuse one 4-column seed that does not match the analysis (survival, Cox, GLM, 4PL) | open |
| GWF-09 | Medium | `human_wf_*` uses of `graph_sheets.last()`; `welcome_hub.rs` `auto_graph` default | "Linked graph" checks use `graph_sheets.last()`, which is the table's Welcome auto-graph rather than a graph produced by the analysis (WF-08, 10, 12, 22, 24, 58), and may be in WF-25, 27, 28, 44, 48–50 | open |
| GWF-10 | Low | `nav.new.layouts`, `menu.insert.new_layout`, layout composer | Layout creation is exercised only through the command palette (WF-27), never through `nav.new.layouts` or the Insert menu, and no human-path test adds a graph panel through the GUI; WF-57 and WF-59 seed both | open |
| GWF-11 | Low | `human_wf_01_06.rs:33–41`, `human_wf_25_31.rs:22–36` | `r15_4_phase_a_wf01_24_*` / `wf25_50_*` are named after workflow ranges but build their documents with engine commands; they are layout gates, not workflow evidence | open |
| GWF-12 | Medium | `crates/ui-egui/src/egui_shell/canvas_view.rs` `AsyncRasterWorkerOwner::drop` | The `cb-canvas-raster` worker panicked 30 times across the suites ("failed to join thread: Resource deadlock avoided"). An in-flight job holds an `egui::Context` clone and the context owns the worker handle, so the last drop could run on the worker thread and self-join. Hidden from test results because only the background thread panicked | fixed in `7116d92` (detach when dropping on the worker thread; regression test `owner_dropped_on_its_own_worker_thread_does_not_self_join`) |
| GWF-13 | Low | `crates/ui-egui/tests/human_wf_01_06.rs` WF-04 | The paste wizard writes into the active sheet by design; the defect is that `wf04_human_import_reshape_and_convert_archetype` injects an import intent/view state instead of invoking File > Import and its name and flow disagree | open |
| GWF-14 | Low | `gui-workflows.md` §6.1, §6.2, §7; `ROADMAP.md` `R10.25`, `R13.6`; `STATUS.md` | "50 / 50" and "15 / 15" human-complete claims cite passing test names, which satisfies `cargo xtask status`, but the tests do not meet the §6 five-stage definition | open |
| GWF-15 | High | `crates/ui-egui/src/egui_shell/{canvas_view.rs,trailing_studio/page_layout.rs}` | No GUI path adds, positions, or resizes a graph panel in a layout. `layout.arrange` only auto-fills, and `resizing_corner` is never set in `canvas_view.rs` | open |
| GWF-16 | Medium | `crates/ui-egui/src/egui_shell/top_chrome.rs:681-694`, `crates/ui-egui/src/menus/menu_bar.rs` | `View > Theme` submenu options have no widget IDs, and there is no menu or toolbar entry for Preferences (`app.preferences`) | open |
| GWF-17 | High | `crates/ui-egui/src/egui_shell/graph_modals/format_graph/series.rs:372` | `WF-54`'s right-axis assignment and horizontal nudge have no GUI control (`"Right Y2"` at `format_graph/series.rs:372` is a read-only label) | open |

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

## Suggested follow-up

1. Add the missing GUI controls for GWF-01 and GWF-02 (graph orientation; replicate line mode and symbol visibility).
2. Replace the seeded setup in the eight Seeded tests with Welcome-hub, grid, Insert and layout-composer clicks, and prove a GUI path for layout creation and panel placement (GWF-10).
3. Give each human-path test a live upstream edit with a recompute check, `ResultStatus::UpToDate` assertions, a `last_intent_error` check after each modal OK, and at least one numeric value cross-checked against the existing oracle tests.
4. Make guarded clicks fail when their widget is missing, and assert non-default settings rather than defaults.
5. Until then, describe `WF-01`–`WF-65` human-path status as "GUI smoke paths pass" rather than "human-complete" in `gui-workflows.md`, `ROADMAP.md` (`R10.25`, `R13.6`) and `STATUS.md`.
