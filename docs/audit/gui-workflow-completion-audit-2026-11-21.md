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

---

## 65-Workflow GUI Control-Gap Sweep (`R13.10`, 2026-11-21)

Audit of every workflow `WF-01`–`WF-65` in `docs/gui-workflows.md` against drawn widget IDs in `crates/ui-egui/src/egui_shell/` and `crates/ui-egui/src/menus/menu_bar.rs`.

### Additional Control Gaps Discovered in Sweep

| ID | Affected WFs | Component / File | Finding | Disposition |
|---|---|---|---|---|
| `GWF-18` | `WF-24`, `WF-56`, `WF-63` | `crates/ui-egui/src/egui_shell/graph_modals/format_axes/segments_ticks.rs`, `trailing_studio/graph_inspector.rs` | No drawn controls for `AxisSegment.major_tick_interval` or `minor_ticks_per_interval` (needed e.g. for `WF-56` "9 log minor ticks" & `WF-24`/`WF-63` tick spacing); engine & `FormatAxesState` support both. | Posted `GAP_SWEEP` to `@r10-27` / `@coordinator`. |
| `GWF-19` | `WF-06`, `WF-31`, `WF-55` | `crates/ui-egui/src/egui_shell/transform_modals.rs` | `PruneRows` hardcodes `every_k_rows: 2, average_window: false` without K/average widgets (`:1257-1259`); `Transpose` lacks `swap_row_and_column_titles` checkbox (`:1256`); standalone `ExtractAndRearrange` drops `extract_target_kind` / derived cols / X-Y-Group bindings (`:1251-1255`). | Posted `GAP_SWEEP` to `@r10-27` / `@coordinator`. |
| `GWF-20` | `WF-17`, `WF-26` | `crates/ui-egui/src/egui_shell/stats_modals/factorial_mv.rs`, `stat_params.rs` | `edit.corr.heatmap` (`factorial_mv.rs:229`) hardcodes `true` and discards click (`let _ = render_modal_checkbox(...)`); `edit.diag.qq` & `edit.diag.shapiro` (`stat_params.rs:788-789`) discard clicks. | Posted `GAP_SWEEP` to `@r10-27` / `@coordinator`. |
| `GWF-21` | `WF-45`, `WF-60` | `crates/ui-egui/src/egui_shell/data_grid/headers.rs`, `graph_modals/mod.rs` | Menus expose `sheet.duplicate` / `sheet.duplicateFamily`, with no per-column Y duplicate action for `WF-60`; `format_graph/heatmap.rs` edits `heatmap_annotate_cells` / `heatmap_cell_gap_pt`, which `commit_graph_modal_action` (`graph_modals/mod.rs:325-370`) never commits. | Posted `GAP_SWEEP` to `@r10-27` / `@coordinator`. |
| *(R13.10 Format Graph)* | `WF-63`, `WF-64` | `dialogs_canvas/format_graph.rs`, `graph_modals/format_graph/series.rs` | Missing Format Graph bar `border_width_pt` (`WF-64` 0.5 pt + `Apply Globally`) and diagnostic forest `forest_show_pooled_diamond` toggle (`WF-63`). | Fixed directly inside `R13.10` alongside `GWF-01`, `GWF-02`, `GWF-17`. |

### Per-Workflow Feature-to-Widget Sweep (`WF-01`–`WF-65`)

| WF | Spec Features (Menus / Dialogs / Inspector / Graphs) | Drawn Widget IDs / Gaps |
|---|---|---|
| `WF-01` | Column table; Bar+dots graph; Welch t/Mann-Whitney; Vertical/Horizontal; Pairwise bracket | `table.*`, `analyze.item.stats.two_group`, `edit.two_group.*`, `change_graph_type.opt.col.bar_replicates`, `pairwise.*`, **Gap `GWF-01`** (`format_graph.orientation.*`, `inspector.graph.orientation.*` -> `R13.10`) |
| `WF-02` | XY table; Log10 transform; 4PL vs 3PL AICc; Replicates vs individual lines; Hide symbols | `edit.transform.x_transform.*`, `fit.*` (`Model`/`Compare`), `format_graph.show_raw_data`, `format_graph.show_fitted_curve`, **Gap `GWF-02`** (`format_graph.show_symbols`, `format_graph.replicate_lines.*` -> `R13.10`) |
| `WF-03` | Grouped table + factor titles; Two-Way/Three-Way ANOVA + Tukey; Superimposed/Interleaved + Error bar stroke | `analyze.item.stats.factorial_anova`, `edit.fac.*`, `edit.post_hoc.*`, `format_graph.arrangement.*`, `format_graph.error_bar_*`, **Gap factor titles inline edit** (assigned `R10.27`) |
| `WF-04` | Plate/Tidy import; One-Way RM ANOVA + Geisser-Greenhouse + Dunnett/Sidak; Box & Violin + half-violin; Tukey outliers | `table.importPlateMap` / `table.importTidyLong` / `import.*`, `edit.rm.*`, `edit.post_hoc.*`, `change_graph_type.opt.col.box_whisker`, `format_graph.half_violin_with_points` |
| `WF-05` | Pharmacology XY; Remove Baseline, Normalize 0-100%, Hook K from Info constant; EC50 ratio & Michaelis-Menten | `edit.transform.baseline_*`, `edit.transform.norm_zero.*`, `edit.transform.hook_item.*`, `edit.transform.normalize.create_graph`, `fit.*` |
| `WF-06` | High-density signal XY; Prune rows every K + block avg; Weighted spline / Akima | `analyze.item.transform.prune`, `fit.*`, **Gap `GWF-19`** (missing K & average-window UI in `transform_modals.rs`) |
| `WF-07` | Survival table; Kaplan-Meier + Log-rank / Gehan-Breslow; Hazard ratios; Survival graph; Thresholds | `analyze.item.stats.survival_km`, `edit.km.*`, `change_graph_type.opt.survival.*`, `axes.additional_ticks.open` |
| `WF-08` | ROC & Contingency tables; DeLong CI, Youden index; Fisher's exact / Chi-square / McNemar / Kappa | `analyze.item.stats.roc`, `analyze.item.stats.contingency`, `edit.cont.*`, `change_graph_type.opt.diag.roc` |
| `WF-09` | Broken axis 2-3 segments + gap width + style; Right Y2 assignment & horizontal series nudge | `axes.segment.{add,remove}`, `axes.break_ornament.*`, `axes.gap_width_pt`, `edit.tab.axes.y2`, **Gap `GWF-17`** (`format_graph.series_select.*`, `series_axis.*`, `series_offset_x` -> `R13.10`) |
| `WF-10` | Multiple Variables table; Correlation matrix / Heatmap / OLS / Logistic; Categorical & continuous color + MV bubble size | `analyze.item.stats.glm`, `format_graph.mv.*`, `format_graph.heatmap.*`, `bubble_plot.*` |
| `WF-11` | Nested table; Nested t-test / Nested One-Way ANOVA / Mixed-Effects REML | `analyze.item.stats.nested`, `edit.reml.*`, `format_graph.*` |
| `WF-12` | Parts of Whole (Pie, Donut, Waffle, Stacked); Palette + CVD simulation preview | `change_graph_type.opt.parts.*`, `format_graph.palette.*`, `format_graph.cvd.*` |
| `WF-13` | Clustered Grouped & Column; Interleaved/Stacked/Superimposed; Heatmap labels & angle; Factor titles | `format_graph.arrangement.*`, `format_graph.heatmap.*`, **Gap factor titles inline edit** (assigned `R10.27`) |
| `WF-14` | Multi-group Column ANOVA + Kruskal-Wallis / Friedman + Dunn; Log scale / Violin / Aligned dot | `edit.one_way.*`, `edit.post_hoc.dunn`, `axes.scale.*`, `change_graph_type.opt.col.*` |
| `WF-15` | Dose-response + Normalize + Custom constraint + Compare models Extra SS F-test + AICc + Profile CI | `edit.transform.norm_*`, `fit.*` (`Compare`, `Constrain`, `InitialValues`, `Range`, `Weight`, `Output`) |
| `WF-16` | Simple Linear / Deming Model II / Passing-Bablok Regression + ANCOVA slopes/intercepts; Bands & Equation overlay | `analyze.item.fit.linear`, `edit.lin.*`, `insert_equation.*`, `format_graph.equation.*` |
| `WF-17` | Multivariate PCA (parallel analysis, scree, biplot) + PCR + Correlation matrix + Clustering | `analyze.item.stats.{pca,pcr,corr_matrix,clustering}`, `edit.pca.*`, `edit.cluster.*`, `edit.corr.{pearson,spearman}`, **Gap `GWF-20`** (`edit.corr.heatmap` no-op) |
| `WF-18` | Cox PH regression + Multiple Imputation (MICE) + Row-wise FDR t-tests | `analyze.item.stats.{survival_cox,multiple_row_t_tests}`, `edit.model_formula.*`, `edit.mi.*` |
| `WF-19` | Publication styling: Paper background (Transparent/White/Cream), Alpha transparency chips, Frame presets, Export PDF/SVG/TIFF/PNG | `format_graph.paper_fill.*`, `format_graph.alpha.*`, `axes.frame.*`, `ui.exportFigure`, `app.export.*` |
| `WF-20` | Interactive data point drill-back, exclude datum, format selected point, Jump to Datasheet cell, Exclude with Reason (`Cmd+Shift+E`) | `inspector.drill_back.{back,toggle_exclude,format_point,jump}`, `selected_points.*`, `exclude.*` |
| `WF-21` | Column Statistics (Descriptive, IQR, Quantiles, Normality suite), Outlier detection (ROUT / Grubbs), Bland-Altman | `analyze.item.stats.{descriptive,outliers,bland_altman}`, `edit.outlier.*` |
| `WF-22` | P-value stack (`FDR` BH/BY/BKY), Multiple-group Pairwise bracket overrides & CLD letters | `analyze.item.stats.p_value_stack`, `pairwise.*`, `inspector.cld.*` |
| `WF-23` | Paired Repeated Measures (Before-After / RM ANOVA / Mixed-effects) & Violin raincloud split | `change_graph_type.opt.col.before_after`, `format_graph.half_violin_with_points`, `edit.rm.*` |
| `WF-24` | Tick formatting (Decimal / 10^x / SI / |x|), Tick direction, Major tick interval & Reference thresholds / Shaded zones | `axes.tick_format.*`, `axes.tick_dir.*`, `axes.additional_ticks.open`, **Gap `GWF-18`** (`major_tick_interval`) |
| `WF-25` | XY Scatter & Line, Marker Fill Mode (`Open`/`Filled`), Size spinbox, Connecting Line Dash, Right Y2 axis | `format_graph.marker_size_pt`, `format_graph.fill.*`, `format_graph.line_dash.*`, **Gap `GWF-02`** (`show_symbols`, `replicate_lines`), **Gap `GWF-17`** (`series_axis.*`, `series_select.*`) |
| `WF-26` | Nonparametric / Welch / Kolmogorov-Smirnov two-group diagnostics + QQ & Shapiro checks | `edit.two_group.*`, `result.diagnostic_banner.inspect`, **Gap `GWF-20`** (`edit.diag.qq` & `edit.diag.shapiro` no-op checkboxes) |
| `WF-27` | Publication Multi-Panel Layout composer: Add Graph panel, Drag/Resize corner handle, Snap/Grid/Align, Journal width presets, Export PDF | `studio.layout.*`, `ui.rulers.toggle`, `ui.gridOverlay.toggle`, `ui.canvas.applyPreset`, **Gap `GWF-15`** (add/corner resize/Z-order -> assigned `R10.27`) |
| `WF-28` | Cross-Platform Theme (Light / Dark / High Contrast), Preferences (14 Beta locales, scale, shortcuts), Apply Style From | `apply_style.*`, `app.preferences`, **Gap `GWF-16`** (`View > Theme` submenu popup IDs & Preferences wiring -> assigned `R10.27`) |
| `WF-29` | Change Data Analyzed, Duplicate Analysis Prompt (`CreateNew` vs `ModifyExisting`), Duplicate Family | `ui.analyze.changeDataAnalyzed`, `dialog.duplicate_analysis`, `sheet.duplicateFamily` |
| `WF-30` | MV Formula bar / Calculated variable (`=log10(...)`, unit awareness) + MV Inspector roles | `inspector.mv_formula.*`, `format_graph.mv.*` |
| `WF-31` | Extract & Rearrange (`SelectAndTransform`) into target table archetype + filter formula | `edit.transform.formula`, `edit.transform.filter_formula`, `edit.transform.col.*`, `edit.transform.target_archetype.*`, **Gap `GWF-19`** (standalone params dropped) |
| `WF-32` | Nonparametric Kruskal-Wallis / Friedman + Dunn / Conover post-hoc + Box/Violin plot | `edit.one_way.*`, `edit.post_hoc.dunn`, `change_graph_type.opt.col.*` |
| `WF-33` | Duplicate / Retarget Nonlinear Regression + Compare 4PL top/bottom shared constraints + EC50 table | `dialog.duplicate_analysis`, `fit.*` (`Compare`, `Constrain`, `Range`, `Weight`, `Output`), `result.tab.*` |
| `WF-34` | Grouped Two-Way Mixed-Effects REML ANOVA with Sidak/Holm-Sidak + Error bar direction (`Upper`/`Both`) | `edit.fac.*`, `edit.paired_or_matched`, `edit.post_hoc.*`, `format_graph.error_bar_dir.*` |
| `WF-35` | Clinical Kaplan-Meier 2+ arms with Greenwood 95% CI, Hazard Ratio (Log-rank/Mantel-Haenszel), Percent survival Y | `edit.km.*`, `change_graph_type.opt.survival.*`, `axes.seg.0.{min,max}` |
| `WF-36` | Weighted Nonlinear Regression (`1/Y^2`, Poisson, Iterative) + Quantile / Normality residual inspection | `fit.*` (`Weight` tab, `Output` residuals), `result.tab.*`, `result.new_graph_from_tab` |
| `WF-37` | Asymmetric Sigmoidal / 5PL / Biphasic dose-response curve fit + Unknowns interpolation | `fit.*` (`Model`, `Output` unknowns interpolation), `result.tab.*` |
| `WF-38` | Bubble plot MV encoding (X/Y continuous, Color ramp, Variable bubble size, Out-of-range Clamp vs Omit) | `format_graph.mv.{x_next,y_next,color_*,size_variable,min_radius,max_radius,oor_*}`, `bubble_plot.*` |
| `WF-39` | Nested Hierarchical Biological vs Technical replicates + Subcolumn aggregation + Nested ANOVA | `analyze.item.stats.nested`, `inspector.graph.*`, `result.tab.*` |
| `WF-40` | Receiver Operating Characteristic (ROC) Youden cutoff, Sensitivity vs 1-Specificity, Diagonal reference | `analyze.item.stats.roc`, `change_graph_type.opt.diag.roc`, `axes.additional_ticks.open` |
| `WF-41` | Clinical Contingency Diagnostic Accuracy (Sensitivity/Specificity/PPV/NPV, Odds Ratio, Fisher/Barnard Exact) | `analyze.item.stats.contingency`, `edit.cont.*` |
| `WF-42` | Multivariate Logistic / Poisson GLM with categorical reference level ordering (`Top`/`Up`/`Down`/`End`) + Odds/Rate Ratios | `analyze.item.stats.glm`, `edit.model_formula.*`, `format_graph.mv.level_*` |
| `WF-43` | Standardized PCA + Horn Parallel Analysis + Confidence Ellipses & Convex Hulls on Biplot | `edit.pca.{corr,cov,parallel}`, `format_graph.mv.{confidence_ellipses,convex_hulls}` |
| `WF-44` | Principal Component Regression (PCR) strict X/Y split + variance explained & predicted vs observed graph | `analyze.item.stats.pcr`, `edit.model_formula.*`, `result.new_graph_from_tab` |
| `WF-45` | Aggregated / Correlation Matrix Heatmap with RdBu/Viridis colormap, custom bounds, rotated labels | `format_graph.heatmap.{cmap.*,reversed,auto_range,min,max,row_labels.*,col_labels.*,angle.*}`, **Gap `GWF-21`** (`annotate_cells` / `cell_gap` not persisted) |
| `WF-46` | Hierarchical Clustering & K-Means Sweep (Euclidean/Pearson/Cosine × Ward/Complete/UPGMA) + Sheet duplicate | `edit.cluster.*`, `sheet.duplicate`, `result.new_graph_from_tab` |
| `WF-47` | Stacked Row-Wise t-Tests (Welch / Mann-Whitney / Wilcoxon + BH FDR) & Volcano diagnostic graph | `analyze.item.stats.multiple_row_t_tests`, `change_graph_type.opt.diag.*`, `result.tab.*` |
| `WF-48` | Method Comparison Trio: Bland-Altman, Passing-Bablok CUSUM linearity, Deming Model II + Reference Line (`y=x`) | `analyze.item.stats.{bland_altman,passing_bablok}`, `edit.lin.deming`, `axes.additional_ticks.open` |
| `WF-49` | Cross-Table Import Reshape / Stack-to-Wide / Wide-to-Long & Archetype Conversion | `import.*` (`ImportReshape`, `ConvertArchetype`) |
| `WF-50` | Locale-Aware Numeric Input (`de`/`fr` comma decimals, localized UI strings, zero bleed/truncation across 14 Beta locales) | `preferences.*` (14 `BETA_ACTIVE_LOCALES`), `unit_spinners`, `axes.seg.0.{min,max}`, **Gap `GWF-16`** (Preferences modal parity -> `R10.27`) |
| `WF-51` | Unsaved Changes guard, Recovery prompt, Dirty-state save & Close, About modal, Find & Replace | `dialog.unsaved_changes`, `dialog.recovery`, `dialog.about`, `dialog.find_replace`, `app.find_replace` |
| `WF-52` | Residual QQ & Normality Diagnostics on ANOVA/Regression + Diagnostic Banner Inspect Details + Point Drill-Back | `result.diagnostic_banner.{inspect,why,dismiss}`, `inspector.drill_back.*`, `canvas.clipping_banner.fit_range` |
| `WF-53` | Page Layout Composition: Journal Single/1.5/2-col width presets, Portrait/Landscape, Margins, Panel Badges A/B/C, Align Origins, PDF Export | `ui.pageLayout.compose`, `page_layout.*`, `studio.layout.*`, `ui.exportFigure`, **Gap `GWF-15`** (`R10.27`) |
| `WF-54` | Horizontal Grouped Bars / Dumbbell / Violin with Floating Axis Frame, Inverted/Log Y, Categorical Labels | `axes.frame.offset_tufte`, `format_graph.half_violin_with_points`, **Gap `GWF-01`** (`format_graph.orientation.horizontal`, `inspector.graph.orientation.*` -> `R13.10`) |
| `WF-55` | Linked Pipeline Chain (Data -> Transpose -> Remove Baseline -> Grouped ANOVA -> Graph) + Reactive downstream updates | `analyze.item.transform.{transpose,baseline}`, `result.tab.*`, **Gap `GWF-19`** (`Transpose` swap-titles checkbox) |
| `WF-56` | Logarithmic XY Calibration Curve: Linear Fit on Log-X / Log-Y scale, Power-of-10 tick labels, **9 Log Minor Ticks**, Interpolate Unknowns | `axes.scale.*`, `axes.tick_format.power10`, `edit.lin.interp`, **Gap `GWF-18`** (`minor_ticks_per_interval` control missing) |
| `WF-57` | Split-View / Dual Pane Canvas + Inspector Canvas Tab + CVD Safe Print (`MonochromePrint` / Deuteranopia) + Preflight | `view.toggleSplit`, `format_graph.palette.*`, `format_graph.cvd.*`, `inspector.graph.*`, `app.export.*` |
| `WF-58` | Results Grid Rectangular Drag-Selection Copy Slice (`Copy Selection` / `Copy Table`), Multi-Tab Inspection & New Graph from Result | `result.tab.*`, `result.copy_selection`, `result.ctx.{copy_selection,copy_table}`, `result.new_graph_from_tab` |
| `WF-59` | Publication Composite Showcase: Multi-panel Bar/Box-Whisker/Violin/Spaghetti + Tukey Whiskers + Pairwise Significance + Legend Boxed | `change_graph_type.opt.col.*`, `pairwise.*`, `format_graph.legend_bordered`, `format_graph.error_bar_*`, **Gap `GWF-02`** (`replicate_lines` -> `R13.10`) |
| `WF-60` | Outlier Robustness & Audit Trail: Grubbs/ROUT Flagging, Duplicate Y Column Comparison, `Cmd+E` / `Cmd+Shift+E` Exclude with Reason | `analyze.item.stats.outliers`, `exclude.*`, `sheet.duplicate`, **Gap `GWF-21`** (missing per-column duplicate UI) |
| `WF-61` | MV Diagnostic Showcase: Annotated Heatmap + Scree/Biplot + Correlation Clustering + Mixed-Factor Ordering | `format_graph.heatmap.*`, `format_graph.mv.*`, `edit.pca.*`, `edit.cluster.*`, **Gap `GWF-21`** (`heatmap` annotate/gap commit) |
| `WF-62` | Clinical Kaplan-Meier + Cox PH Forest Hazard Ratios + Risk Table + Threshold Reference Lines | `analyze.item.stats.{survival_km,survival_cox}`, `change_graph_type.opt.diag.forest`, `axes.additional_ticks.open` |
| `WF-63` | Forest Plot Diagnostic Customization: Custom Major Tick Interval, Zero/Unity Threshold, Optional Pooled Summary Diamond Toggle | `change_graph_type.opt.diag.forest`, `axes.additional_ticks.open`, **Gap `GWF-18`** (`major_tick_interval`), **Gap (`R13.10`)** (`format_graph.forest_show_pooled_diamond`) |
| `WF-64` | Journal-Ready Bar Borders & Global Format Application: Change **All Data Sets** bar border width to **0.5 pt** (`Apply Globally`), Error bar SD caps, Tight Open-L frame | `format_graph.apply_globally`, `format_graph.error_bar_*`, `axes.frame.open_l`, **Gap (`R13.10`)** (`format_graph.bar_border_width_pt`) |
| `WF-65` | Dual-Axis Overlaid Time-Course & Dose-Response: Bind secondary series to **Right Y2**, enable Y2 axis, set Left vs Right tick ranges & series horizontal nudge (`offset.x_data`) | `edit.tab.axes.y2`, `axes.y2_enable`, **Gap `GWF-17`** (`format_graph.series_select.*`, `series_axis.*`, `series_offset_x` -> `R13.10`) |
