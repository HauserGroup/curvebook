# GUI-Only Agent Workflows & Surface Parity (`R8.6`–`R8.8`, `R9.17`)

> Related specifications: [`ui-parity.md`](ui-parity.md), [`mcp.md`](mcp.md), [`control-protocol.md`](control-protocol.md), [`statistics-implementation.md`](statistics-implementation.md), [`curve-fitting-implementation.md`](curve-fitting-implementation.md), [`graphing-and-layouts.md`](graphing-and-layouts.md), [`qol-beyond-parity.md`](qol-beyond-parity.md) · Roadmap: [`ROADMAP.md`](ROADMAP.md#r8-desktop--webassembly-live-shell-localization--10-hardening) (`R8.6`–`R8.8`, `R9.17`) · Live metrics: [`STATUS.md`](STATUS.md)

This reference documents LumenCraft's **strict `gui_only` execution contract**, the semantic UI inspection tree and 8-tool human-equivalent GUI surface (`R8.6`, `45` `UI_COMMANDS` synced to `crates/mcp/src/ui_catalog.rs`, `95` typed engine commands behind UI actions, `16` locales × `307` keys = `4,912` i18n strings), the 4-stage Power & Sample Size wizard, declarative modal graying predicates, grid-to-graph formatting and `ResultSheet` chaining (`R8.7`), the **28 Core end-to-end GUI workflows** (`R8.8`, `WF-01`–`WF-28` across Shards 1–4), and the **21 extended multivariable, clustering, GLM, survival & statistical-graphing `gui_only` workflows** (`R9.17` / `QOL-17`, `WF-29`–`WF-49` across Shards 5–7 — **49 total strict `gui_only` E2E workflows**).

> **Verification Status (`R8.6`–`R8.8`, `R9.7`, `R9.16`, `R9.17` Implemented):**
> - **Workspace shell & inspection (`R8.6`):** `UiApp` (`crates/ui-egui`), software RGBA/PNG compositing (`render_workspace_rgba` / `render_workspace_frame_png`), `UiTreeSnapshot` (`crates/ui-egui/src/inspect.rs`, `45` `UI_COMMANDS`), the 8 `ui.*` interactive GUI action handlers (`crates/ui-egui/src/gui_actions.rs`), `--control` JSONL loopback (`crates/ui-egui/src/control_io.rs`), `GuiOnlyPolicy` (`crates/engine/src/lib.rs`), `ErrorCode::GuiOnlyViolation` (`crates/doc/src/result.rs`), `GuiSessionHost` + the 8 `gui` MCP tools (`crates/mcp/src/{tools.rs, gui_tools.rs, protocol.rs}`), `InProcessUiGuiHost` (`apps/lumencraft-cli/src/main.rs`), and `lumencraft --gui-only` (`apps/lumencraft/src/main.rs`).
> - **Interactive wizards, predicates & chaining (`R8.7`):** 4-stage **Power & Sample Size** wizard (`crates/ui-egui/src/power_wizard.rs`, `crates/stats/src/power.rs`), 10 declarative modal enablement predicates `R1`–`R10` (`crates/ui-egui/src/modal_predicates.rs`), multi-row grid selection -> `FormatSelectedPoints` (`graph.formatSelectedPoints`, `PointStyleOverride`), and `ResultSheet` `Analyze...` chaining (`sheet.extractResultTab` + `ResultSheet::analyzable_data`).
> - **49-workflow acceptance suite (`R8.8` Shards 1–4 + `R9.17` Shards 5–7):** `crates/ui-egui/tests/gui_wf_{foundations_and_data,group_comparisons_and_power,regression_contingency_multivariate,statistical_graphs,multivariable_and_transforms,clustering_glm_and_pharma,advanced_statistical_graphs}.rs`, `crates/ui-egui/tests/gui_only_workflows.rs`, `crates/ui-egui/tests/gui_w22b_qol_and_r9.rs`, `crates/mcp/tests/gui_only_surface.rs`, `apps/lumencraft-cli/tests/mcp_gui_only_parity.rs`, and `apps/lumencraft/tests/gui_only_wiring.rs`.

---

**QOL-7 diagnostics-only alignment (implemented in Waves 20–22b):** `AssumptionDiagnosticBanner` (`crates/ui-egui/src/dialogs_canvas.rs`, `sanitize_diagnostics_only_text` in `crates/ui-egui/src/dialogs_r9.rs`) renders diagnostic facts and opens existing diagnostic detail tabs via `ui.result.inspectDiagnostics` (`mutated: false`), without test/transform recommendations, remediation buttons, or banner-triggered sibling analyses. Deprecated `ui.result.applyRemediation` returns `ErrorCode::Conflict` (`details.code: "diagnostics_only_policy"`, `mutated: false`). Independently choosing or switching analyses via `Cmd+T` (`EditAnalysisParameters`) or the normal analysis dialog remains supported ([`R9.7`](ROADMAP.md#r97-inline-assumption-diagnostic-banners-diagnostics-only-qol-7)).

## 1. Strict `gui_only` Execution Contract

Passing headless engine tests (`command_run` invoking `sheet.createData`, `table.setCell`, `stats.run`, or `fit.run`) proves `L1`–`L5` engine correctness, but does not prove that a researcher or GUI-bound agent can complete an analysis strictly through visible workspace controls (`L6`–`L7`).

Under strict `--gui-only` mode (`lumencraft-cli mcp --gui-only`, `lumencraft-cli mcp --control-port <N> --gui-only`, and `lumencraft --control <N> --gui-only`):

1. **Permitted Observation Primitives (Read-Only Perception):**
   - In-memory viewport PNG capture (`ui_screenshot` / `ui.screenshot`) and structured accessibility/layout snapshots (`ui_inspect` / `ui.inspect` returning `UiTreeSnapshot`).
   - Reading visible labels, formatted cell values, XY/Survival `grid.xcell.*` cells, `grid_selection` / `quick_stats`, `brushed_points`, widget `enabled` / `disabled_reason` predicates, `status_banner`, `Navigator` tree nodes, and modal tabs/controls/buttons.
   - On desktop `--control --gui-only`, read-only inspection RPCs (`ping`, `doc.inspect`, `ui.state` / `state.snapshot`, `command.list`, `ui.inspect`, `ui.canvas.preflight`) remain permitted; on MCP `--gui-only`, `doc_inspect`, `command_list`, `resources/read`, and non-GUI tools are all blocked with `ErrorCode::GuiOnlyViolation`, and `resources/list` returns `[]`.
2. **Permitted Input Primitives (Human-Equivalent Actions):**
   - Pointer gestures (`click`, `double_click`, `right_click`, `drag`, `hover`, `scroll` / `wheel`) resolved against visible `widget_id` targets (enforcing `node.enabled`) or viewport `(x, y)` coordinates, including `shift`/`meta` multi-row grid selection and `power.curve_hover`.
   - Keyboard and TSV text entry (`ui_type_text` / `ui_press_key` / `ui.type_text` / `ui.press_key`) supporting scalar cell values, TSV multi-cell blocks, `Enter`, `Escape`, `Cmd/Ctrl+T` (`EditAnalysisParameters` from active `ResultSheet`), and `ShortcutRegistry` combos.
   - Visible menu bar (`ui_invoke_menu` / `ui.invoke_menu`), tab (`ui_switch_tab` / `ui.switch_tab`), and modal/wizard control updates and footer buttons (`ui_answer_dialog` / `ui.answer_dialog`), each gated by its live enablement predicate.
3. **Forbidden Shortcuts (Structurally Blocked by `GuiOnlyPolicy::Strict`):**
   - **`ErrorCode::GuiOnlyViolation` (`"gui_only_violation"`, `ErrorCategory::Policy`):** Defined in `crates/doc/src/result.rs`.
   - **Engine caller guard (`crates/engine/src/lib.rs`):** `is_strict_gui_only_allowed(spec, caller, params)` blocks `Caller::Mcp`, `Caller::Control`, and `Caller::Cli` from running any command unless it is read-only without `Effects::READS_FILESYSTEM`, or is `table.convertArchetype` with `preview_only` / `previewOnly` / `preview` / `dry_run` / `dryRun: true`.
   - **MCP `--gui-only` lock (`crates/mcp/src/{tools.rs, protocol.rs}`):** `list_tools_for_state` exposes strictly the 8 `gui` tools (`GUI_TOOL_NAMES`), `initialize` emits `GUI_ONLY_SERVER_INSTRUCTIONS`, `resources/list` returns `[]`, `resources/read` fails with `-32600` (`"gui_only_violation"`), and any call to `command_run`, `command_batch`, `doc_inspect`, `open_file`, `save_file`, `export`, `render_preview`, `list_toolsets`, `enable_toolset`, or direct command tools returns `isError: true` with `structuredContent.code == "gui_only_violation"`.
   - **Desktop `--control --gui-only` lock (`crates/ui-egui/src/control_io.rs`):** Direct `"command.run"`, `"command.batch"`, `"engine.execute"`, mutating domain commands (`Caller::Control`), and direct `"ui.*"` / `"app.about"` / `"file.print"` calls (other than the 8 interactive `ui.*` actions and read-only `"ui.canvas.preflight"`) return `"gui_only_violation"`. Passing a non-empty disk `path` to `"ui.screenshot"` or `"render.pdf"` under `--gui-only` also fails with `ErrorCode::GuiOnlyViolation`.
   - **Internal UI execution (`Caller::Ui`):** Menu, shortcut, and modal handlers inside `UiApp` continue executing through `Session::execute` / `execute_as_ui` with `Caller::Ui`.

---

## 2. Semantic UI Inspection & `gui` Surface (`R8.6`)

### 2.1 `ui.inspect` (`UiTreeSnapshot` in `crates/ui-egui/src/inspect.rs`)

`inspect_ui_tree(app, scope, include_bounds)` computes a deterministic JSON snapshot from the live `UiApp`, attaching `ControlEnablement` (`enabled`, `disabled_reason`) from `crates/ui-egui/src/modal_predicates.rs` and optional `[x, y, width, height]` bounds from `WorkspaceLayoutMap`:

- **Always-present shell & state fields on `UiTreeSnapshot`:**
  - `active_sheet_id`, `active_sheet_kind`, `active_dialog` (`"none"`, `"power_and_sample_size"`, `"format_selected_points"`, `"analyze_sheet"`, `"edit_analysis_parameters"`, `"change_data_analyzed"`, `"import_reshape"`, `"convert_archetype"`, `"change_graph_type"`, `"format_graph"`, `"print"`, etc.), `focus_widget_id`, `zoom`, `gui_only`, `undo_depth`, `redo_depth`, `dirty`, `cursor_readout`, `grid_selection` (`GridSelectionSnapshot`: `active_cell`, `anchor_cell`, `selected_rows`, `selected_coords`, `selected_cells_count`, `quick_stats`), `brushed_points` (`Vec<BrushedPointSnapshot>`), `status_banner`, `context_menu_target`.
- **Scoped optional subtrees** (`inspect_ui_tree` accepts `"all"` / `"workspace"`, `"menus"`, `"toolbar"`, `"navigator"`, `"tabs"`, `"canvas"`, `"grid"`, `"dialog"`; the `ui_inspect` MCP schema constrains `scope` to `["all", "dialog", "grid", "navigator", "canvas", "menus"]`):
  - `menus: Option<Vec<UiMenuSnapshot>>` — 7 localized top-level menus (`menu_key`, `title`, `bounds`, `items: Vec<UiWidgetNode>`).
  - `toolbar: Option<Vec<UiWidgetNode>>` — contextual toolbar buttons (`Analyze`, `Change Graph Type`, `Analysis Parameters`, `Export`, etc.).
  - `navigator_sheets: Option<Vec<UiNavigatorSheetNode>>` — navigator entries (`widget_id`, `sheet_id`, `section`, `title`, `selected`, `role`, `label`, `enabled`, `bounds`).
  - `tabs: Option<Vec<UiWidgetNode>>` — active `ResultSheet` sub-tabs or contextual view tabs.
  - `canvas_elements: Option<Vec<UiWidgetNode>>` — hit-testable graph/layout/inspector targets.
  - `grid_cells: Option<Vec<UiWidgetNode>>` — up to 64 visible `grid.cell.{row}.{col}.{subcol}` nodes plus, on `TableArchetype::Xy` and `TableArchetype::Survival`, up to 16 rows of `grid.xcell.{row}.{subcol}` nodes (marking `XColumnMode::GeneratedSequence` read-only with `enabled: false` and `disabled_reason: "X column is a generated sequence (read-only)."`).
  - `dialog: Option<UiDialogSnapshot>` — `dialog_kind`, `title`, `target_sheet_id`, `stage`, `bounds`, `tabs`, `controls`, `buttons`, where every `UiWidgetNode` carries `widget_id`, `role`, `label`, optional `command_id`, optional `value`, `enabled`, optional `disabled_reason`, and optional `bounds`.

### 2.2 Ergonomic 8-Tool `gui` MCP & JSON-Lines Surface (`crates/mcp/src/gui_tools.rs`, `crates/ui-egui/src/gui_actions.rs`)

Both `InProcessUiGuiHost` (`lumencraft-cli mcp --gui-only`) and loopback `--control` (`lumencraft-cli mcp --control-port <N> [--gui-only]`) route the identical 8-tool surface:

| MCP Tool (`GUI_TOOL_NAMES`) | `--control` Action | Annotations / Effects | Implemented Behavior |
|---|---|---|---|
| `ui_inspect` | `ui.inspect` | `readOnlyHint: true`, `idempotentHint: true` | Returns `UiTreeSnapshot` (`scope`, `include_bounds`, optional `sheet_id`). |
| `ui_screenshot` | `ui.screenshot` | `readOnlyHint: true`, `idempotentHint: true` | Renders in-memory PNG (`width_px`/`height_px` `64..=4096`, `dpi` `72..=600`); MCP returns inline `image/png` block + metadata without `path`; `--control` rejects `path` under `--gui-only`. |
| `ui_pointer_gesture` | `ui.pointer_gesture` | `readOnlyHint: true`, `idempotentHint: false` (`CHANGES_UI_STATE`) | Dispatches `click`, `double_click`, `right_click`, `drag`, `hover`, `scroll` (`wheel` normalized by MCP/action) on `widget_id` (checking `node.enabled`) or `(x, y)` coordinates, incl. `power.curve_hover` and `shift`/`meta` multi-row grid selection. |
| `ui_type_text` | `ui.type_text` | `readOnlyHint: false` (`MUTATES_DOCUMENT \| CHANGES_UI_STATE`) | Enters UTF-8 text or TSV blocks into modal fields, `InfoSheet` notes (`info.setNote`), or active grid cells (`grid.cell.*` -> `table.paste`/`table.setCell`, `grid.xcell.*` -> `table.setXCells`). |
| `ui_press_key` | `ui.press_key` | `CHANGES_UI_STATE` | Dispatches keystrokes & `ShortcutRegistry` combos (`Enter`, `Escape`, `Tab`, `Delete`, `Backspace`, `Cmd/Ctrl+T`, `Cmd/Ctrl+Z`, etc.). |
| `ui_invoke_menu` | `ui.invoke_menu` | `CHANGES_UI_STATE` | Triggers a visible menu item by `command_id` or `path` (string or string array); fails with `ErrorCode::Conflict` if disabled. |
| `ui_switch_tab` | `ui.switch_tab` | `CHANGES_UI_STATE \| IDEMPOTENT` | Selects sheets (`sheet_id`), `EditAnalysisParameters` tabs (validated via `evaluate_edit_params_tab`), `PowerWizardStage` tabs (`can_advance`), or `ResultSheet` sub-tabs. |
| `ui_answer_dialog` | `ui.answer_dialog` | `readOnlyHint: false` (`MUTATES_DOCUMENT \| CHANGES_UI_STATE`) | Patches `fields`/`values`/`patches` and invokes footer buttons (`ok`, `cancel`, `apply`, `back`, `next`, `create`, `preview` via `evaluate_dialog_button`), incl. `sheet.extractResultTab` for `ResultSheet` -> `AnalyzeSheet` chaining. |

---

## 3. Interactive Wizards, Modal Predicates & Chained Surfaces (`R8.7`)

### 3.1 Standalone 4-Stage Power & Sample Size Wizard (`crates/ui-egui/src/power_wizard.rs`, `crates/stats/src/power.rs`)

Opened via `Analyze › Power and Sample Size...` (`ui.powerWizard.open`, `ActiveDialog::PowerAndSampleSize`), backed by exact power solvers in `crates/stats/src/power.rs`:

1. **`PowerWizardStage::ExperimentDesign` (`1. Experiment Design`):** Choose among **8** `PowerTestFamily::ALL` test families (`TwoSampleUnpairedT`, `TwoSamplePairedT`, `OneSampleT`, `OneWayAnova`, `TwoProportions`, `Correlation`, `ChiSquare`, `SimpleLinearRegression`) and **3** `PowerSolveTarget` modes (`SampleSize`, `Power`, `DetectableEffect`).
2. **`PowerWizardStage::EffectSize` (`2. Effect Size`):** Parameterize effect via `PowerEffectInputMode` (`Standardized` [$d$, $d_z$, $f$, $h$, $r$, $w$, $f^2$], `MeansAndSd` including within-pair correlation $\sigma_d = \sigma\sqrt{2(1-r)}$ via `paired_sd_from_correlation` / `computed_paired_sigma_d`, `PairedDiffDirect`, `Proportions` $(p_1, p_2)$ or baseline $p_1$, `RegressionRSquared`, `RegressionSlope` $(\beta_1, \sigma_x, \sigma_\varepsilon)$).
3. **`PowerWizardStage::SignificanceAndSolve` (`3. Significance & Solve`):** Configure `TailMode` (`TwoSided` vs. `OneSided`, disabled automatically when `!family.supports_one_sided()` for `OneWayAnova` and `ChiSquare`), `alpha`, `target_power`, sample size / allocation ratio, `TwoProportionsMethod` (`PooledNullUnpooledAlt`, `Unpooled`, `CohensH`), and `CorrelationPowerMethod` (`FisherZ`, `FisherZWithTCrit`, `ExactTTransform`).
4. **`PowerWizardStage::Results` (`4. Results & Sensitivity`):** Inspect `PowerAnalysisResult` (`continuous_n`, `n_per_group`, `n_group2`, `total_n`, `power_at_continuous_n`, `achieved_power`, `critical_value`, `noncentrality_param`), hover the Stage 4 curve (`power.curve_hover` via `compute_power_curve`), inspect the multi-$\alpha \times$ multi-power `sensitivity_table` (`SensitivityCell` via `compute_sensitivity_table`), and commit `create` (`Caller::Ui`) to emit an XY `DataSheet`, `InfoSheet`, `ResultSheet`, and linked `GraphSheet` (`Power Curve — ...`).

### 3.2 Ten Mandatory Conditional Enablement & Graying Rules

Modal and wizard widgets carry declarative `ControlEnablement { enabled, disabled_reason }` evaluated by `crates/ui-egui/src/modal_predicates.rs` (and emitted into `UiDialogSnapshot` by `crates/ui-egui/src/inspect.rs`):

1. **`R1` — ANOVA pairing -> `Repeated Measures` tab & sphericity/REML (`r1_repeated_measures_tab`, `r1_sphericity_or_reml_control`, `r1_paired_or_matched_allowed`):** Grays out the `Repeated Measures` tab (`edit.tab.repeated_measures`) and sphericity / mixed-effects REML controls when unpaired; on `Column` tables, pairing requires $\ge 2$ columns with equal active replicate counts and $\ge 2$ complete paired rows (`WF-08`, `WF-10`, `WF-13`, `WF-24`, `WF-25`).
2. **`R2` — Gaussian toggle -> Post-hoc family (`r2_parametric_post_hoc_method`, `r2_post_hoc_option_enabled`, `apply_r2_gaussian_toggle`):** Switching `Assume Gaussian?` off disables parametric post-hoc choices (`Tukey`, `Dunnett`, `DunnettT3`, `Šídák`, `Holm–Šídák`, `Bonferroni`) and selects nonparametric `Dunn's` (`WF-10`, `WF-11`, `WF-24`).
3. **`R3` — Control contrast -> Control column picker (`r3_control_column_picker`):** Enables `edit.anova.control_column` only when comparing against a control column (`Dunnett` / `DunnettT3`) (`WF-10`).
4. **`R4` — Two-group test mode -> Graph & Welch options (`r4_two_group_graph_differences`, `r4_two_group_nonparametric_options`, `r4_two_group_welch_correction`):** Enables `Graph differences` only when paired, `Graph ranks` and `Hodges–Lehmann CI` only when nonparametric, and `Welch's correction` only when unpaired parametric (`WF-07`, `WF-08`, `WF-09`, `WF-21`).
5. **`R5` — Contingency dimensions -> $2 \times 2$ clinical stats & Cochran–Armitage trend (`r5_contingency_2x2_effect_sizes`, `r5_contingency_cochran_armitage_trend`):** Enables `Odds ratio`, `Attributable risk`, `Sensitivity & Specificity`, `Fisher's exact test`, and `McNemar's test` strictly on $2 \times 2$ tables; enables `Cochran-Armitage trend test` specifically on ordered $R \times 2$ ($R \ge 3$) or $2 \times C$ ($C \ge 3$) tables (`WF-19`, `WF-20`).
6. **`R6` — Contingency effect checkbox -> CI selectors (`r6_contingency_ci_selector`):** Grays out `Baptista-Pike / Woolf`, `Koopman`, `Newcombe-Wilson`, and `Wilson-Brown` CI method selectors until the corresponding $2 \times 2$ table + effect-size checkbox is active (`WF-19`).
7. **`R7` — Correlation mode -> Heatmap toggle (`r7_correlation_heatmap_toggle`):** Enables correlation heatmap creation iff all-pairs `Correlation matrix` mode is active (`WF-17`, `WF-26`).
8. **`R8` — Replicate count -> Subcolumn aggregation (`r8_subcolumn_aggregation`):** Grays out replicate-subcolumn aggregation controls unless `max_subcolumns > 1` (`WF-16`, `WF-26`).
9. **`R9` — P-value formatting style -> Precision digits (`r9_pvalue_precision_spinner`):** Enables custom P-value precision digits only when `PValueFormat::Custom { .. }` is active (`WF-07`, `WF-16`, `WF-26`); paired with `evaluate_dialog_button` guarding invalid `confidence_percent`.
10. **`R10` — PCA predictor checklist -> Leakage-safe `PCR` outcome picker (`r10_pcr_outcome_column`):** Excludes/grays out any column checked as a PCA predictor from the `Principal Component Regression` outcome selector so $Y$ cannot leak into principal components (`WF-23`, `WF-26`).

> **Dialog Inspection Coverage Note:** `inspect.rs` and `gui_actions.rs` populate specialized structured controls for `PowerAndSampleSize`, `FormatSelectedPoints`, `AnalyzeSheet`, `EditAnalysisParameters`, `ImportReshape`, `ConvertArchetype`, `ChangeGraphType`, `FormatGraph`, `FormatAxes`, `PairwiseComparisons`, `BubblePlotInspector`, `ApplyStyleFrom`, `PageLayoutComposer`, `ExportFigure`, `RandomizeGroups`, `RandomSampleRows`, `SimulateDistributions`, `InsertModelEquation`, and `Print` (remaining helper/utility dialogs such as `ChangeDataAnalyzed`, `ExcludeWithReason`, `CommandPalette`, and `ResultViewer` map through `build_generic_dialog_nodes` alongside direct `ui.*` commands).

### 3.3 Chained Result-Table Pipelines & Grid-to-Graph Point Styling

- **GUI `Analyze...` chaining on `ResultSheet` tabs (`crates/ui-egui/src/gui_actions.rs`, `crates/engine/src/cmd/sheet.rs`):**
  1. **Direct analyzable results:** `ResultSheet::analyzable_data: Option<DataSheet>` enables direct downstream analysis from `Transform`, `Normalize`, `PruneRows`, etc., when active in the Navigator (`WF-02`, `WF-05`, `WF-06`, `WF-23`).
  2. **Automatic tabular extraction (`sheet.extractResultTab`):** Confirming `AnalyzeSheet` while a non-analyzable `ResultSheet` or switched sub-tab (e.g., `pca-loadings` or `pc_scores`) is active dispatches `sheet.extractResultTab` (`archetype: "auto"`, producing `Column` for purely numeric tabs or `MultipleVariables` when text/boolean cells are present) via `Caller::Ui` and launches the downstream analysis on the extracted `DataSheet` (`gui_only_workflows.rs`, `WF-05`).
- **Multi-row grid selection -> `Format Selected Points` (`ui.formatPoints.open`, `ActiveDialog::FormatSelectedPoints`, `graph.formatSelectedPoints`):**
  - `Shift+Click` and `Cmd/Meta+Click` on grid cells populate `grid.selected_rows` and sync `brushed_points`.
  - Invoking `Format › Selected Points...` (`ui.formatPoints.open`) and confirming `ui_answer_dialog` writes `PointStyleOverride` entries (`color`, `symbol`, `size_pt`, `border_width_pt`, `fill_color`, `fill_mode`, `hidden`) on `GraphSpec::point_overrides` (`crates/doc/src/graph.rs`) via `graph.formatSelectedPoints` (`mode`: `merge`, `replace`, `clear`) and optionally attaches row-label `GraphAnnotation::TextBox` callouts (`WF-01`, `WF-03`, `WF-22`).

---

## 4. Canonical 28-Workflow `gui_only` Acceptance Matrix (`R8.8`)

Every workflow test runs under `UiApp` with `app.set_gui_only(true)` (`GuiOnlyPolicy::Strict`) and drives all workspace interactions strictly through `dispatch_gui_action` (`ui.inspect`, `ui.screenshot`, `ui.pointer_gesture`, `ui.type_text`, `ui.press_key`, `ui.invoke_menu`, `ui.switch_tab`, `ui.answer_dialog`), alongside the supporting policy/protocol integration suites:

- `crates/ui-egui/tests/gui_only_workflows.rs` — core `--gui-only` lock, all 8 interactive actions, `PowerAndSampleSize` 4-stage flow, multi-row `FormatSelectedPoints`, and `ResultSheet` `Analyze...` chaining.
- `crates/mcp/tests/gui_only_surface.rs` — MCP `--gui-only` tool-list shrink, `GUI_ONLY_SERVER_INSTRUCTIONS`, `gui_only_violation` enforcement, `GuiSessionHost` / `--control-port` routing, `enable_toolset("gui")`, and parameter validation.
- `apps/lumencraft-cli/tests/mcp_gui_only_parity.rs` — spawned `lumencraft-cli mcp --gui-only` (`InProcessUiGuiHost`) and `lumencraft-cli mcp --control-port <N> --gui-only` end-to-end tests.
- `apps/lumencraft/tests/gui_only_wiring.rs` — `lumencraft --headless-once --gui-only` binary and multi-window `DesktopHostRuntime` enforcement.

### Shard 1 — `crates/ui-egui/tests/gui_wf_foundations_and_data.rs` (`WF-01`–`WF-06`)

| ID | Test Function | Key GUI Surfaces & Verified Pipeline |
|---|---|---|
| `WF-01` | `wf01_xy_dose_response_format_exclude_with_reason_and_screenshot` | Control bypass guard check, XY concentration-response table via menu, `grid.xcell.*` + TSV paste, Shift/Meta multi-row selection -> `Format Selected Points...`, `ExcludeWithReason` provenance, linked graph screenshot. |
| `WF-02` | `wf02_normalize_to_result_sheet_chaining_and_log_x_transform` | XY normalization -> `ResultSheet` (`analyzable_data`) -> direct `Analyze...` `transform.math` (`Log10(X)`) chaining and multi-hop reactive propagation on cell edit. |
| `WF-03` | `wf03_column_brushing_format_and_undo_redo_shortcuts` | Column 3-group entry, multi-row grid & graph canvas brushing (`ui.pointer_gesture`), `Format Selected Points...`, `Cmd+Z`/`Cmd+Shift+Z` shortcuts and Edit menu Undo/Redo. |
| `WF-04` | `wf04_import_reshape_and_convert_archetype` | `ui.import.reshape.open` (`ImportReshape` dialog), 96-well `Plate Map...` & `Tidy Long Table...` menus, preview and apply `Convert Table Structure...` (`ui.table.convertArchetype.open`). |
| `WF-05` | `wf05_transform_pca_and_clustering_pipeline` | `Transform/PruneRows` -> `ResultSheet` `Analyze...` PCA -> `ui.switch_tab` (`pca-loadings`) -> chained Cluster Analysis (`K-means++` / `Ward.D2`). |
| `WF-06` | `wf06_prune_descriptive_survival_chaining_and_reactive_recompute` | `PruneRows` -> chained Descriptive Statistics + Kaplan-Meier Survival, Multiple Imputation, and reactive recompute on source cell edit. |

### Shard 2 — `crates/ui-egui/tests/gui_wf_group_comparisons_and_power.rs` (`WF-07`–`WF-15`)

| ID | Test Function | Key GUI Surfaces & Verified Pipeline |
|---|---|---|
| `WF-07` | `wf07_unpaired_student_and_welch_t_test_with_predicates_and_graph` | Unpaired Student's & Welch's *t*-test with `R4` & `R9` modal predicates verified in `ui.inspect` and linked graph. |
| `WF-08` | `wf08_paired_t_test_equal_n_check_and_before_after_graph` | Paired *t*-test equal-$N$ `R1` check, `R4` difference/Welch controls, and Before-After graph. |
| `WF-09` | `wf09_nonparametric_mann_whitney_and_wilcoxon_with_rank_invariance` | Mann-Whitney $U$ (rank invariance under outlier edit) and Wilcoxon matched-pairs signed-rank test. |
| `WF-10` | `wf10_one_way_anova_welch_remediation_dunnett_tukey_and_brackets` | One-way ANOVA, Brown–Forsythe / Welch via explicit `Cmd+T` parameter switching (diagnostics-only `AssumptionDiagnosticBanner` + `ui.result.inspectDiagnostics`, zero sibling creation), `R2`–`R3` post-hoc controls (Dunnett / Tukey), and significance brackets. |
| `WF-11` | `wf11_kruskal_wallis_and_dunns_multiple_comparisons` | Kruskal-Wallis + Dunn's multiple comparisons (`R2` nonparametric post-hoc swap) -> `ResultSheet` & graph. |
| `WF-12` | `wf12_two_way_anova_interaction_and_grouped_error_bar_guard` | Two-way ANOVA on Grouped replicates, interaction effect, and grouped error-bar guard. |
| `WF-13` | `wf13_rm_anova_sphericity_geisser_greenhouse_and_huynh_feldt` | Repeated-Measures ANOVA (`R1` enabled `Repeated Measures` tab) with Geisser-Greenhouse and Huynh-Feldt sphericity corrections. |
| `WF-14` | `wf14_friedman_and_apply_remediation_flow` | Friedman nonparametric RM-ANOVA, diagnostics-only `AssumptionDiagnosticBanner` (`ui.result.inspectDiagnostics`), deprecated `ui.result.applyRemediation` rejecting with `diagnostics_only_policy` (no sibling `ResultSheet` created), and explicit `Cmd+T` parameter switching. |
| `WF-15` | `wf15_full_four_stage_power_and_sample_size_wizard` | Full 4-stage `PowerAndSampleSize` wizard navigation (`next`/`back`/`create`), Stage 4 curve hover, sensitivity table, and linked `ResultSheet` + `GraphSheet`. |

### Shard 3 — `crates/ui-egui/tests/gui_wf_regression_contingency_multivariate.rs` (`WF-16`–`WF-23`)

| ID | Test Function | Key GUI Surfaces & Verified Pipeline |
|---|---|---|
| `WF-16` | `wf16_linear_deming_passing_bablok_ancova_and_r8_r9_predicates` | Simple linear regression, Deming / Passing-Bablok regression, ANCOVA, and `R8` (subcolumn aggregation) + `R9` (P-value precision) predicates. |
| `WF-17` | `wf17_dose_response_4pl_5pl_interpolation_and_r7_correlation_matrix` | Dose-Response 4PL / 5PL nonlinear regression with unknown interpolation + Pearson/Spearman Correlation Matrix & `R7` heatmap predicate check. |
| `WF-18` | `wf18_enzyme_kinetics_global_shared_vmax_and_five_gate_diagnostics` | Michaelis-Menten enzyme kinetics with global shared `Vmax` fit and five-gate regression diagnostics. |
| `WF-19` | `wf19_contingency_2x2_fisher_r5_r6_and_rxc_chi_square` | $2 \times 2$ Contingency (Fisher's exact, Baptista-Pike OR, Koopman RR, Newcombe RD, `R5` & `R6` graying) and $R \times C$ Chi-square. |
| `WF-20` | `wf20_roc_curve_analysis_graph_and_cochran_armitage_trend` | ROC curve analysis (AUC, Youden's $J$, ROC graph) and $2 \times 3$ ordered Contingency `Cochran-Armitage trend test` (`R5` enabled). |
| `WF-21` | `wf21_kaplan_meier_survival_hr_logistic_and_r4_checks` | Kaplan-Meier 1- vs. 2-group survival (Log-Rank, Hazard Ratio), Multiple Logistic Regression (`MultipleVariables`), and `R4` checks. |
| `WF-22` | `wf22_multivariate_pca_clustering_glm_volcano_and_format_points` | Multivariate PCA, Hierarchical Clustering, Multiple Linear Regression / GLM, Multiple row *t*-tests + Volcano Plot + `Format Selected Points...`. |
| `WF-23` | `wf23_stacked_superimposed_z_order_pcr_r10_and_result_analyze_chaining` | Grouped `Stacked` vs. `Superimposed` (`GroupedArrangement::Superimposed`) z-order, 2-hop `ResultSheet` `Analyze...` chaining, and leakage-safe PCR with `R10` modal guard. |

### Shard 4 — `crates/ui-egui/tests/gui_wf_statistical_graphs.rs` (`WF-24`–`WF-28`)

| ID | Test Function | Key GUI Surfaces & Verified Pipeline |
|---|---|---|
| `WF-24` | `wf24_significance_brackets_auto_stars_and_axes_strict_gui_only` | `Column` One-Way ANOVA + Tukey -> significance bracket annotations with auto-stars (`*`/`**`/`***`/`ns`), callouts, axis ticks/log config, SVG/PNG checks, and `R1`/`R2` modal graying. |
| `WF-25` | `wf25_violin_boxplot_column_styles_error_bars_and_svg_strict_gui_only` | Violin / Box-and-Whisker / Bar-with-Replicates & Aligned Dot plots (`ChangeGraphType` + `Format/Data on Graph...`), scatter overlay & SD/SEM/95%-CI error bars, `R1` check, SVG & PNG export. |
| `WF-26` | `wf26_bubble_volcano_heatmap_multivariate_and_r7_to_r10_inspect_strict_gui_only` | Bubble Plot (Viridis + variable size + convex hulls), Volcano Plot, Correlation Heatmap, and `R7`–`R10` verification in `ui.inspect`. |
| `WF-27` | `wf27_multi_panel_layout_drag_snap_apply_style_and_preflight_strict_gui_only` | Multi-panel `LayoutSheet` creation, live pointer drag & snap (`ui.pointer_gesture` `drag`), `ApplyStyleFrom` style cloning preserving target bindings, and publication preflight check. |
| `WF-28` | `wf28_canvas_preset_cvd_print_export_and_light_dark_screenshots_strict_gui_only` | Journal canvas preset, non-mutating Color-Vision Preview, `ActiveDialog::Print` & in-memory PDF spooling, inline SVG/PDF/PNG export, and Light vs. Dark theme screenshots. |

---

## 5. Extended 21-Workflow Multivariable, Clustering, GLM & Statistical-Graphing `gui_only` Matrix (`R9.17` / `QOL-17`, Implemented)

> **Scope & Guardrail Alignment (`QOL-7`, `QOL-16`, `R9.17`):**
> - Every workflow in `WF-29`–`WF-49` (`Shards 5–7`, implemented in Wave 23) runs under `UiApp` with `GuiOnlyPolicy::Strict` (`app.set_gui_only(true)`), constructing original synthetic datasets via `lumencraft-testkit` and driving 100% of interactions through the 8 `ui.*` / `gui` perception and input primitives (`Section 2.2`), including at least one live upstream cell edit or `Cmd/Ctrl+E` exclusion check verifying immediate reactive DAG recomputation.
> - **Diagnostics-only adherence (`QOL-7` / `R9.7`):** Assumption and residual checks inspect diagnostic values and child plots only (`ui.result.inspectDiagnostics`) — never asserting test recommendations, remediation buttons, or banner-triggered sibling analyses.
> - **Equations-only & CLD exclusion adherence (`QOL-16` / `R9.16`):** Pairwise significance assertions verify bracket annotations (`*`/`**`/`***`/`ns` and exact $p$-values) only; Compact Letter Display (`CLD`), group-letter displays, and extra fit-statistic callouts remain excluded from product scope, and `WF-49` exercises implemented `R9.16` equations-only live $\TeX$ best-fit model annotations (`GraphAnnotation::ModelEquation`, `ui.graph.insertModelEquation`).

### Shard 5 — `crates/ui-egui/tests/gui_wf_multivariable_and_transforms.rs` (`WF-29`–`WF-36`)

| ID | Test Function | Key GUI Surfaces & Verified Pipeline |
|---|---|---|
| `WF-29` | `wf29_select_transform_row_stats_and_baseline_subtraction` | Chained math transforms (`-log10(P)`, `Y / K`), column descriptive summaries, per-row subject statistics, baseline column subtraction, and conjunctive `Variable vs. Value` / `Variable vs. Variable` row filtering in `MultipleVariables` with live DAG propagation. |
| `WF-30` | `wf30_calculated_variables_formula_bar_and_circularity_guards` | `MultipleVariables` formula-bar `Calculated Variables` (`ROUND`, `SUM`, `MEAN`, `IF`, `LOG10`, cross-column ratios), live syntax and circular-reference error guards, read-only computed cell rendering, and reactive upstream cell edits. |
| `WF-31` | `wf31_extract_and_rearrange_cross_schema_chaining` | Reshaping a typed `MultipleVariables` master table (`Continuous`, `Categorical`, `Label` columns) into chained `XY` (simple linear regression), `Column` (Welch *t* & One-Way ANOVA + Tukey), `Grouped` (Two-Way ANOVA), and $2 \times 2$ `Contingency` (`Fisher's exact`, `RR`, `OR`) analyses with two-hop DAG updates. |
| `WF-32` | `wf32_direct_multiple_variables_welch_t_and_effect_sizes` | In-table calculated difference column (`=D-C`) plus direct two-sample Welch's *t*-test binding tidy `Response (Y)` and 2-level `Grouping` columns on `MultipleVariables` without reshaping, verifying Satterthwaite $df$, Cohen's $d$ / Hedges' $g$ / Glass's $\Delta$, variance *F*-test, and `MultipleVariables` `CategoricalSummary` graph (`plot_mode: "categorical_summary"`). |
| `WF-33` | `wf33_direct_multiple_variables_multifactor_anova` | Direct ordinary $N$-way `Multifactor ANOVA` from `MultipleVariables` (including `LOG10` calculated columns), 2-way and 3-way factorial interactions, Type III sums of squares, $\eta^2$ / partial $\eta^2$ / Cohen's $f$, Tukey post-hoc contrasts (`R9.12`), and residual outlier inspection. |
| `WF-34` | `wf34_qpcr_ddct_calculated_variables_and_welch_pipeline` | End-to-end Livak–Schmittgen $2^{-\Delta\Delta C_t}$ qPCR workflow in `MultipleVariables` chaining replicate $\overline{C_t}$, $\Delta C_t$, calibrator `MEAN`, $\Delta\Delta C_t$, and $2^{-\Delta\Delta C_t}$ fold-change formulas into a direct Welch's *t*-test and `MultipleVariables` `CategoricalSummary` graph (`plot_mode: "categorical_summary"`). |
| `WF-35` | `wf35_direct_multiple_variables_kaplan_meier_survival` | Direct `Kaplan-Meier` survival estimation from a long-format `MultipleVariables` table (`Time`, `Status`, `Grouping` column role binding + unit-rescaled calculated column), omnibus Log-rank / Gehan–Breslow–Wilcoxon / trend tests, Holm–Šídák pairwise curve comparisons, and median survival reference line. |
| `WF-36` | `wf36_direct_multiple_variables_4pl_dose_response` | Direct multi-series 4PL dose–response fitting from `MultipleVariables` combining formula-bar `Calculated Variables` (molar unit scaling, `% viability` normalization, `LOG10` concentration), `X`/`Y`/`Grouping` role assignment, `Bottom=0`/`Top=100` asymptote constraints, profile-likelihood $\text{IC}_{50}$ CIs, and residual plots. |

### Shard 6 — `crates/ui-egui/tests/gui_wf_clustering_glm_and_pharma.rs` (`WF-37`–`WF-43`)

| ID | Test Function | Key GUI Surfaces & Verified Pipeline |
|---|---|---|
| `WF-37` | `wf37_hierarchical_clustering_linkages_cophenetic_and_dendrogram` | Bidirectional agglomerative `Hierarchical Clustering` on `MultipleVariables` across `Euclidean`/`Manhattan`/`Pearson`/`Cosine` distances and 7 linkage rules (`Complete`, `Single`, `UPGMA`, `WPGMA`, `Centroid`, `Median`, `Ward`), verifying pairwise/cophenetic matrices, cophenetic correlation, standalone dendrograms, biclustered heatmaps, and Silhouette curves. |
| `WF-38` | `wf38_kmeans_clustering_silhouette_elbow_and_cluster_scatter` | Hartigan–Wong `K-Means` clustering across candidate $k$ sweeps, `Silhouette` and `Elbow` WCSS optimal-$k$ diagnostic curves, row-level categorical cluster assignment columns, and multivariable cluster scatter plots with confidence ellipses and convex hulls. |
| `WF-39` | `wf39_cox_proportional_hazards_efron_ties_and_covariate_curves` | Multivariable `Cox proportional hazards` regression on `MultipleVariables` time-to-event data configuring categorical reference levels, `Efron` tie handling, profile-likelihood hazard ratios, adjusted Wald $Z$ tests, `AIC` diagnostics, parameter correlation heatmap, and covariate survival curves. |
| `WF-40` | `wf40_multiple_linear_regression_type_iii_vif_and_diagnostics` | Least-squares `Multiple linear regression` across continuous and dummy-coded categorical predictors, Type III ANOVA, partial slope *t*-tests, `VIF` multicollinearity and covariance matrices, residual normality diagnostics (`QOL-7` diagnostics-only), and `Actual vs Predicted` + residual plots. |
| `WF-41` | `wf41_multiple_logistic_classification_hosmer_lemeshow_and_roc` | Multivariable binary `Multiple logistic regression` configuring reference levels, profile-likelihood `95%` OR CIs, Wald $\|Z\|$, $\text{AIC}_c$, `ROC` AUC, $2 \times 2$ cutoff classification table, Tjur's $R^2$, Hosmer–Lemeshow goodness-of-fit, cutoff-accuracy curves, and `Row prediction` probabilities. |
| `WF-42` | `wf42_poisson_count_glm_rate_ratios_dispersion_and_vif` | Multivariable log-link `Poisson` count GLM on non-negative integer outcomes $Y \in \mathbb{Z}_{\ge 0}$, categorical reference levels, profile-likelihood `95%` CIs, incidence rate ratios $\exp(\beta)$, deviance/dispersion/pseudo-$R^2$ diagnostics, residual inspection, and `VIF` pruning. |
| `WF-43` | `wf43_xy_serial_dilution_constrained_4pl_ic50_and_auc` | Serial dilution $\log_{10}(X)$ concentration transform on multi-replicate `XY` tables, unconstrained vs. `Bottom=0`/`Top=1` constrained 4PL variable-slope inhibition fitting resolving unbounded profile-likelihood intervals into exact $\text{IC}_{50}$ estimates, plus `AUC` viability integration. |

### Shard 7 — `crates/ui-egui/tests/gui_wf_advanced_statistical_graphs.rs` (`WF-44`–`WF-49`)

| ID | Test Function | Key GUI Surfaces & Verified Pipeline |
|---|---|---|
| `WF-44` | `wf44_confidence_ellipses_and_convex_hulls_multivariable_guards` | Pooled and per-group bivariate `Confidence Ellipses` (`95%` vs. `50%` confidence levels across categorical factors) and minimal enclosing `Convex Hull` polygons on `MultipleVariables` scatter plots with continuous-axis guards and live outlier-exclusion updates. |
| `WF-45` | `wf45_grouped_and_multivariable_cross_tab_summary_heatmaps` | Multi-row $\times$ multi-column `Grouped` heatmaps (`Viridis` vs. diverging colormaps, cell borders, inline values, `45°` column labels) and cross-tabulated `MultipleVariables` summary heatmaps (`Mean`, `Median`, `Count`, `SD`, `Proportion`) across dynamically swapped categorical axes. |
| `WF-46` | `wf46_interactive_multivariable_graph_inspector_variable_swapping` | Non-modal multivariable `BubblePlotInspector` / `FormatGraph` workflow exercising `Label`/`Continuous`/`Categorical` variable-type guards, live `X`/`Y`/`Color`/`Size` column swapping with automatic axis rescaling, sheet duplication, and point-to-grid drill-back. |
| `WF-47` | `wf47_kaplan_meier_synchronized_number_at_risk_and_pairwise_survival` | Three-arm `Survival` Kaplan–Meier plot with X-axis-coupled `Number at Risk (number censored)` table that re-bins automatically when X-axis tick intervals change, `95% CI` error-band shading, `Y = 50%` median line, and Holm–Šídák pairwise curve comparisons. |
| `WF-48` | `wf48_multivariable_bubble_symbol_sizing_and_outlier_clamping` | `MultipleVariables` bubble symbol sizing across uniform sizes, proportional vs. bounded min–max continuous area scaling (with 3-point size legends), binary and ordinal categorical per-level sizing, discrete integer dot-plot layouts, and custom threshold outlier clamping vs. omission. |
| `WF-49` | `wf49_live_tex_best_fit_equation_annotations_equations_only` | One-phase exponential decay and 3PL nonlinear regression fits with `95%` profile-likelihood confidence bands coupled to symbolic and live-substituted $\TeX$-subset best-fit model equation annotations (`R9.16` equations-only scope; Compact Letter Display and extra fit-statistic callouts excluded), verifying live coefficient refresh across RGBA, SVG, and `%PDF-1.7` on upstream `XY` edits. |
