# SEC-14 Audit Report: UI Grid, Menus & Canvas Dialogs (`crates/ui-egui`)

- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-14` — `crates/ui-egui/src/{grid_view.rs, dialogs_r9.rs, power_wizard.rs, text_bridge.rs, i18n.rs}`, `crates/ui-egui/src/menus/*` (`5` files), and `crates/ui-egui/src/dialogs_canvas/*` (`16` files) (`26` files, `19,322` lines)
- **Date:** `2026-11-22`
- **Summary Counts:** `S1 = 0`, `S2 = 7`, `S3 = 6`, `S4 = 0` (`13` total findings)

---

## File Coverage Table

| File | Lines | Inspected | Concrete Invariants Verified | Findings |
| :--- | ---: | :--- | :--- | :--- |
| `crates/ui-egui/src/grid_view.rs` | 3,305 | `1..=3305` | Virtualized `O(visible_rows * visible_subcols)` frame materialization; uncapped `GridClipboardTarget` vs `MAX_MATERIALIZED_SELECTION_COORDS = 4096`; discontiguous vs rectangular selection state transitions; `navigate` cursor/selection synchronization; UTF-8 safe `clamp_char_boundary` & `active_token_span`; locale-aware `parse_localized_number_input` & `ImportReshapeState` patching; archetype cell validation & exclusion tooltips | [`FA-SEC14-07`](#fa-sec14-07), [`FA-SEC14-09`](#fa-sec14-09), [`FA-SEC14-10`](#fa-sec14-10) |
| `crates/ui-egui/src/dialogs_r9.rs` | 1,504 | `1..=1504` | Strict `parse_fit_method`, `parse_rout_inline_q`, `parse_pvalue_stack_method_val`, `parse_export_format` validation; diagnostics-only `sanitize_diagnostics_only_text`; `RandomizeGroupsDialogState`, `RandomSampleRowsDialogState`, `SimulateDistributionsDialogState`, and `InsertModelEquationDialogState` patch/commit schemas | [`FA-SEC14-13`](#fa-sec14-13) |
| `crates/ui-egui/src/power_wizard.rs` | 1,254 | `1..=1254` | 4-stage `PowerWizardState` progression, `can_advance` (`alpha`, `target_power`, non-zero effect guards), 8-family live solver recalculation, `hover_at_fraction`/`interpolate_curve_power` bounds safety, localized number/integer patching, and `execute_create_with` multi-command sheet creation under `GuiOnlyPolicy::Strict` | [`FA-SEC14-11`](#fa-sec14-11) |
| `crates/ui-egui/src/text_bridge.rs` | 802 | `1..=802` | `DesktopTextMeasurer` glyph caching and vector fallback sizing; UTF-8 safe `ellipsize_text` and RTL scientific span isolation (`isolate_rtl_scientific_spans`, `match_bracketed_interval`, `match_formula_span`, `match_signed_number_span` ASCII `+ 1` byte boundary checks) | Clean — UTF-8 boundary safety, RTL math isolation, and finite dimension clamping verified |
| `crates/ui-egui/src/i18n.rs` | 566 | `1..=566` | 16-locale `LOCALES` vs 14-locale `BETA_ACTIVE_LOCALES` registry alignment; `normalize_locale` / `normalize_beta_gui_locale`; `LocalePreset` decimal/CSV separator rules; `mirror_x_for_rtl` bounded mirroring; `missing_translations` catalog coverage lookup | Clean — locale normalization, regional `LocalePreset` wiring, and RTL mirroring verified |
| `crates/ui-egui/src/menus/menu_bar.rs` | 369 | `1..=369` | Curated 8-menu structure (`File`, `Edit`, `View`, `Insert`, `Data`, `Analyze`, `Graph`, `Help`); `host_routed_tooltip` coverage; every menu action checked against `cmd::registry()` and `UI_COMMANDS` | [`FA-SEC14-06`](#fa-sec14-06) (exposure of non-routed file actions) |
| `crates/ui-egui/src/menus/mod.rs` | 54 | `1..=54` | Re-exports and `DESKTOP_HOST_ROUTED_COMMANDS` coverage vs file-open/save/import/export/clipboard actions exposed in `menu_bar.rs` (`R15-GUI-07`, `R15-GUI-13`, `R15-GUI-23`) | [`FA-SEC14-06`](#fa-sec14-06) |
| `crates/ui-egui/src/menus/shortcuts.rs` | 653 | `1..=653` | `ShortcutScope` conflict checks and `ui.pane.cycle` `Shift+` collision guard (`find_conflict_with_scope`); atomic `import_overrides_json` rollback; `+`-key splitting in `split_shortcut_tokens`; canonical OS modifier normalization & formatting | Clean — scope-aware conflicts, literal `+` handling, and atomic JSON import verified |
| `crates/ui-egui/src/menus/tooltips.rs` | 316 | `1..=316` | `TOOLBAR_BUTTONS` & `toolbar_caption_command_id` resolution; `command_tooltip` metadata; `ui_command_menu_info` path/bucket completeness vs `menu_bar.rs`; `ui_command_enabled` vs all `53` `UI_COMMANDS` | [`FA-SEC14-12`](#fa-sec14-12) |
| `crates/ui-egui/src/menus/ui_commands.rs` | 441 | `1..=441` | Unique IDs, non-empty `label`/`summary`/`beginner`/`params_doc`, and shortcut conventions across all `53` `UI_COMMANDS` | Clean — unique command specs and metadata completeness verified |
| `crates/ui-egui/src/dialogs_canvas/analysis_catalog.rs` | 373 | `1..=373` | All `30` `ANALYSIS_CATALOG` entries mapped in `default_analysis_kind_for_id`; table-archetype compatibility checks | [`FA-SEC14-03`](#fa-sec14-03) |
| `crates/ui-egui/src/dialogs_canvas/analyze_dialog.rs` | 1,533 | `1..=1533` | Variable role filtering/candidates; `compatible_entries` filtering; PCR target-leakage guard & `pcr_outcome_column` propagation; localized parameter patching; `try_build_run_intent` / `resolve_duplicate_choice` / `build_create_new_intent` engine contract alignment | [`FA-SEC14-03`](#fa-sec14-03) |
| `crates/ui-egui/src/dialogs_canvas/command_palette.rs` | 325 | `1..=325` | Fuzzy scoring & ranking (`fuzzy_score`, `score_command`); Engine + UI command aggregation; session `enabled` evaluation; `params_doc` and `activate_selected` payload emission | [`FA-SEC14-08`](#fa-sec14-08) |
| `crates/ui-egui/src/dialogs_canvas/edit_params.rs` | 1,564 | `1..=1564` | `open_for_result` draft initialization; `sync_rule_flags_from_kind` (`R1`/`R2`/`REML`/Contingency/PCA); localized parameter patching across all `AnalysisKind` variants; `commit_intent` / `commit_intents` engine `analysis.updateParams` parity | [`FA-SEC14-03`](#fa-sec14-03), [`FA-SEC14-04`](#fa-sec14-04) |
| `crates/ui-egui/src/dialogs_canvas/format_axes.rs` | 618 | `1..=618` | Discontinuous `1..=3` segment bounds validation; `Y2` enablement & `FramePreset::DualYSecondaryRight` wiring; `switch_target_axis_preserving_thresholds` staging for `X`/`Y`/`Y2`; `commit_intent` `graph.setAxes` payload fidelity | [`FA-SEC14-05`](#fa-sec14-05) |
| `crates/ui-egui/src/dialogs_canvas/format_graph.rs` | 857 | `1..=857` | `sync_from_graph_with_doc` dirty flags; MV categorical `reorder_mv_level`; `graph.setSpec` + `graph.setSeries` + `graph.setRegressionOverlay` intent generation and finite validation | Clean — dirty-tracked spec/series intents and finite validation verified |
| `crates/ui-egui/src/dialogs_canvas/graph_overlays.rs` | 780 | `1..=780` | `PairwiseComparisonsState` (`brackets_commit_intent`, `cld_commit_intent`); `BubblePlotInspectorState` (`sync_from_graph`, `to_encoding`, `commit_intent`); `ApplyStyleFromState` donor/target validation and intent construction | [`FA-SEC14-01`](#fa-sec14-01), [`FA-SEC14-02`](#fa-sec14-02) |
| `crates/ui-egui/src/dialogs_canvas/graph_type.rs` | 485 | `1..=485` | All `21` `GRAPH_TYPE_OPTIONS` mapped to valid `GraphKind` / GDO templates; grouped arrangement persistence; `graph.setKind` + `graph.setSpec` + `graph.setSeries` intent sequence | Clean — option-to-kind mapping and intent sequence verified |
| `crates/ui-egui/src/dialogs_canvas/layout_export.rs` | 277 | `1..=277` | `PageLayoutComposerState` bounds validation & `layout.setSpec` intents; `ExportFigureState` raster pixel/DPI bounds (`>= 16px`, `72..=2400 dpi`) and `export.tiff` / `graph.export*` intent schemas (`compile_scene_for_sheet` accepts graph and layout sheets) | Clean — layout composer bounds and export figure parameter schemas verified |
| `crates/ui-egui/src/dialogs_canvas/mod.rs` | 702 | `1..=702` | `ActiveDialog` / `ActiveInspector` state transitions; `on_sheet_selected` inspector sync; bounded zoom/pan inertia clamping | Clean — dialog/inspector routing and pan-zoom clamping verified |
| `crates/ui-egui/src/dialogs_canvas/parse_helpers.rs` | 252 | `1..=252` | Shared enum/number/formula parsers (`parse_multiplicity_method_name`, `parse_contingency_method_name`, `parse_table_kind_str`, `parse_palette_id_str`, `apply_transform_formula_str`) rejecting unknown inputs | Clean — strict input parsing without panics verified |
| `crates/ui-egui/src/dialogs_canvas/print_dialog.rs` | 157 | `1..=157` | Finite bounded validation on `bleed_pt` (`0..=144`), `copies` (`1..=999` whole integer), `scale_percent` (`10..=400`), and trimmed printer/title strings | Clean — numeric/string validation and bounded clamping verified |
| `crates/ui-egui/src/dialogs_canvas/r13_dialogs.rs` | 583 | `1..=583` | `CreateGraphDialogState` (including analyzable `ResultSheet` resolution via `resolve_graph_create_source_sheet`), `EqualizeScalingDialogState`, `FormatEmbeddedTableDialogState`, and `FormatImageDialogState` patch validation and `graph.*`/`layout.*` command intents | Clean — R13 dialog state validation and command schemas verified |
| `crates/ui-egui/src/dialogs_canvas/results_hooks.rs` | 445 | `1..=445` | `ChangeDataAnalyzedState` (`analysis.retargetData`); `ResultSheetViewState` tab selection & diagnostics extraction; `ConstantHookPickerState` token generation (`analysis.updateParams`, `graph.addAnnotation`, `layout.addPanel`) | Clean — hook token formatting and bounds-safe selection verified |
| `crates/ui-egui/src/dialogs_canvas/tests.rs` | 453 | `1..=453` | Unit tests covering command palette, `FormatAxes` Y2 sync, duplicate analysis prompt, theme contrast, constant hooks, assumption banner, PCR state guards, heatmap intents, and `FormatGraph` | Clean — test assertions and fixtures inspected |
| `crates/ui-egui/src/dialogs_canvas/workspace_dialogs.rs` | 654 | `1..=654` | `ConvertArchetypeDialogState` preview/apply intents; `ExcludeWithReasonDialogState` non-empty `coords`/`reason_tag` guards; `SplitWorkspaceState` `[0.2, 0.8]` clamping; `AssumptionDiagnosticBanner`; `FormatSelectedPointsState` `merge`/`clear` & label stagger bounds | Clean — archetype conversion, exclusion, split ratio, and point-format state verified |

---

## Findings

### FA-SEC14-01

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/dialogs_canvas/graph_overlays.rs:206-235` (and `:65-86`, `:283-305`)
- **Category:** `data-loss` / `wrong-result` / `contract-drift`
- **Defect:** `PairwiseComparisonsState::brackets_commit_intent` only serializes `group_a` and `group_b` when `self.brackets` has length `1` (`[first]`). Whenever `self.brackets.len() >= 2`, both `group_a` and `group_b` are omitted from the `graph.addComparisonBrackets` payload while `clear_existing: true` is sent. On the engine side (`crates/engine/src/cmd/graph.rs:2010-2036`), `(None, None)` adds **every** comparison (or all significant comparisons if `significant_only: true`) from `result_sheet.pairwise_comparisons`, overwriting the user's edited bracket subset. Conversely, when a user deletes all brackets (`self.brackets.is_empty()`), `brackets_commit_intent` unconditionally returns `None` without checking whether brackets previously existed on the graph, so removed brackets remain stuck on the graph.
- **Failure scenario:** A user clicks `Auto-add from analysis` on a 3-group ANOVA graph (adding 3 brackets), removes one unwanted bracket in the `Pairwise Comparisons` inspector so `2` brackets remain, and clicks `Apply`/`OK`. Because `self.brackets.len() == 2`, `brackets_commit_intent` omits `group_a`/`group_b`, causing `graph.addComparisonBrackets` to clear and re-insert all `3` comparisons (resurrecting the deleted bracket). Similarly, removing the last remaining bracket from an existing graph never emits a clear command and leaves that bracket rendered.
- **Evidence:**
```rust
// crates/ui-egui/src/dialogs_canvas/graph_overlays.rs:206-235
    fn brackets_commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        let res_id = self.selected_result_sheet_id.as_ref()?;
        if self.brackets.is_empty() {
            return None;
        }
        ...
        let mut payload = json!({
            "sheet_id": id.as_str(),
            "result_sheet_id": res_id.as_str(),
            "clear_existing": true,
            "style": style_str,
            "label_mode": self.label_mode,
            "hide_if_non_significant": self.hide_non_significant,
            "significant_only": self.hide_non_significant,
            "stroke_width_pt": 1.0,
            "font_size_pt": 9.0,
            "bracket_step_pt": 12.0,
        });
        if let ([first], Some(map)) = (self.brackets.as_slice(), payload.as_object_mut()) {
            map.insert(String::from("group_a"), json!(first.group_a));
            map.insert(String::from("group_b"), json!(first.group_b));
        }
        Some(ToolCommandIntent::new("graph.addComparisonBrackets", payload))
    }
```
  Engine branch in `crates/engine/src/cmd/graph.rs:2010-2036` runs `(None, None) => { for cmp in comparisons { ... } }`, adding every pair from `ResultSheet` instead of the `self.brackets` subset.
- **Suggested fix:** Track `had_brackets_on_graph` (mirroring `had_cld_on_graph`) in `sync_from_graph`, and emit the exact `self.brackets` vector via `graph.setSpec { sheet_id, comparisons: self.brackets }` (which supports 0, 1, or N brackets atomically, including custom per-bracket pairs and clearing all brackets cleanly).
- **Related:** `PairwiseComparisonsState::apply_to_spec` (`graph_overlays.rs:187-188`).

### FA-SEC14-02

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/dialogs_canvas/graph_overlays.rs:448-514`
- **Category:** `data-loss` / `wrong-result`
- **Defect:** `BubblePlotInspectorState` neither stores `MultivariateGraphEncoding::x_level_order_override` during `sync_from_graph` nor preserves it in `to_encoding()`, instead hardcoding `x_level_order_override: None`. When `commit_intent()` sends `graph.setSpec { sheet_id, kind: MultipleVariables, multivariate: self.to_encoding() }`, `parse_multivariate_encoding` (`crates/engine/src/cmd/graph.rs:3011-3014`) deserializes the complete `MultivariateGraphEncoding` object and overwrites the graph's existing `x_level_order_override` with `None`.
- **Failure scenario:** A user configures a custom categorical X level order (via `Format Graph` -> categorical level ordering) on a `MultipleVariables` Categorical Summary or Bubble Scatter plot, then opens `Bubble Plot Channels` to adjust point size, label column, or confidence ellipses. Saving `Bubble Plot Channels` silently wipes `x_level_order_override` back to `None`.
- **Evidence:**
```rust
// crates/ui-egui/src/dialogs_canvas/graph_overlays.rs:448-460, 482-494, 504-514
    pub fn sync_from_graph(&mut self, graph: &GraphSheet, doc: &ProjectDocument) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.available_columns = doc.data_sheet(&graph.spec.primary_source_sheet_id).map_or_else(Vec::new, collect_variable_candidates);
        if let Some(mv) = &graph.spec.multivariate {
            self.plot_mode = mv.plot_mode.clone();
            self.x_column = mv.x_column;
            self.y_column = mv.y_column;
            self.color_channel = mv.color.clone();
            self.size_channel = mv.size;
            self.label_column = mv.label_column;
            self.show_convex_hulls = mv.show_convex_hulls;
            self.show_confidence_ellipses = mv.show_confidence_ellipses;
        }
    }
    ...
    pub fn to_encoding(&self) -> MultivariateGraphEncoding {
        MultivariateGraphEncoding {
            plot_mode: self.plot_mode.clone(),
            x_column: self.x_column,
            y_column: self.y_column,
            color: self.color_channel.clone(),
            size: self.size_channel,
            label_column: self.label_column,
            x_level_order_override: None,
            show_convex_hulls: self.show_convex_hulls,
            show_confidence_ellipses: self.show_confidence_ellipses,
        }
    }
```
- **Suggested fix:** Add `pub x_level_order_override: Option<LevelOrderSpec>` to `BubblePlotInspectorState`, copy `mv.x_level_order_override.clone()` in `sync_from_graph`, and propagate it in `to_encoding()`.
- **Related:** `FormatGraphState::mv_encoding` (`crates/ui-egui/src/dialogs_canvas/format_graph.rs`).

### FA-SEC14-03

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/dialogs_canvas/analyze_dialog.rs:594-603, 733-743, 1135-1140, 1447-1475`, `crates/ui-egui/src/dialogs_canvas/edit_params.rs:189, 526-541, 1526-1528`, `crates/ui-egui/src/dialogs_canvas/analysis_catalog.rs:351`
- **Category:** `wrong-result` / `contract-drift` / `data-loss`
- **Defect:** The PCR (`"stats.pcr"`) UI workflow accepts `pcr_outcome_column` and enforces the R10 predictor-leakage guard in both `AnalyzeDialogState` and `EditAnalysisParametersState`, yet `pcr_outcome_column` is dropped before/inside the engine:
  1. `analysis_catalog.rs:351` maps `"stats.pcr"` to `MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(7) }`, and `AnalyzeDialogState::build_create_new_intent` (`1464-1475`) / `resolve_duplicate_choice` (`1447-1459`) send only `source_sheet_id`, `selected_data_sets`, and `kind` (`MultivariateSpec::Pca`, which has no `pcr_outcome_column` field in `crates/doc/src/analysis.rs:897-902`) without `pcr_outcome_column`.
  2. `EditAnalysisParametersState::commit_intent` (`1526-1528`) appends `"pcr_outcome_column"`, and `crates/engine/src/cmd/analysis.rs:2218` whitelists `"pcr_outcome_column"` for `MultivariateSpec::Pca`, **but `apply_multivariate_overrides` (`crates/engine/src/cmd/analysis.rs:3192-3199`) ignores `"pcr_outcome_column"` completely**, and `open_for_result` (`edit_params.rs:189`) resets `self.pcr_outcome_column = None`.
  3. Additionally, in `AnalyzeDialogState`, `toggle_variable` (`1135-1140`) mutates `selected_data_sets` without updating `pca_predictors`, and `apply_field_patch_localized` (`733-743`) silently ignores `"pcr_outcome_column"` values that are not JSON `u64` (instead of returning an error or handling `null`).
- **Failure scenario:** A user selects **Principal Component Regression (PCR)** (`stats.pcr`) in Analyze or Edit Parameters, assigns predictor columns `[0, 1, 2]` and outcome column `3`, and runs/commits. Neither `analysis.run` nor `analysis.updateParams` records or uses `pcr_outcome_column = 3`; the engine runs ordinary unsupervised PCA on `[0, 1, 2]` and loses the outcome selection upon reopening parameters.
- **Evidence:**
```rust
// crates/ui-egui/src/dialogs_canvas/analysis_catalog.rs:351
        "stats.pca" | "stats.pcr" => Some(AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(7) })),

// crates/ui-egui/src/dialogs_canvas/analyze_dialog.rs:733-743, 1464-1475
                "pcr_outcome_column" | "response_column" | "analyze.pcr_outcome_column" => {
                    if let Some(col) = val.as_u64().map(|n| n as usize) {
                        if self.pca_predictors.contains(&col) || self.selected_data_sets.contains(&col) {
                            return Err(format!(
                                "Column {col} is checked as a PCA input predictor and cannot be selected as PCR outcome (target leakage guard)."
                            ));
                        }
                        self.pcr_outcome_column = Some(col);
                        self.set_response_column(col);
                    }
                }
...
    pub fn build_create_new_intent(&self) -> Option<ToolCommandIntent> {
        let src = self.source_sheet_id.as_ref()?;
        let kind = self.build_analysis_kind()?;
        Some(ToolCommandIntent::new(
            "analysis.run",
            json!({
                "source_sheet_id": src.as_str(),
                "selected_data_sets": self.selected_data_sets,
                "kind": kind,
            }),
        ))
    }
```
```rust
// crates/engine/src/cmd/analysis.rs:3192-3199
fn apply_multivariate_overrides(spec: &mut MultivariateSpec, params: &Value) -> Result<(), EngineError> {
    match spec {
        MultivariateSpec::Pca { standardize, parallel_analysis_seed } => {
            apply_bool_opt_checked(params, &["standardize"], standardize)?;
            if let Some(seed_val) = params.get("parallel_analysis_seed") {
                *parallel_analysis_seed = if seed_val.is_null() { None } else { Some(parse_u64_json_value(seed_val, "parallel_analysis_seed")?) };
            }
        }
```
- **Suggested fix:** Add `pcr_outcome_column: Option<usize>` (with `#[serde(default)]`) to `MultivariateSpec::Pca` (or a dedicated `MultivariateSpec::Pcr` variant) in `crates/doc`, handle `"pcr_outcome_column"` in `apply_multivariate_overrides` and `stats` PCR computation in `crates/engine`, emit `pcr_outcome_column` from `build_create_new_intent` / `resolve_duplicate_choice`, sync `pcr_outcome_column` in `EditAnalysisParametersState::open_for_result`, keep `pca_predictors` synchronized in `toggle_variable`, and validate non-null non-integer `pcr_outcome_column` patches explicitly.
- **Related:** `modal_predicates::r10_pcr_outcome_column`.

### FA-SEC14-04

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/dialogs_canvas/edit_params.rs:285-288, 421-453, 646-673, 1510-1531`
- **Category:** `data-loss` / `contract-drift`
- **Defect:** `EditAnalysisParametersState` defines UI fields and accepts patches/clicks for several Contingency and Two-Group options that are never synced from `AnalysisSpec`, never mapped into `spec.kind`, and never emitted by `commit_intent()`:
  1. `"contingency_attributable_risk_checked" | "attributable_risk" | "compute_attributable_risk" | "edit.contingency.attributable_risk"` (`669-673`) only sets `self.contingency_attributable_risk_checked` without updating `ContingencySpec::compute_odds_and_risk_ratios` (`crates/doc/src/analysis.rs:749-750`, where OR, RR, and difference in proportions / attributable risk are controlled by `compute_odds_and_risk_ratios`), and `sync_rule_flags_from_kind` (`285-288`) omits `contingency_attributable_risk_checked` so it always opens unchecked.
  2. Contingency CI-method selectors (`"edit.contingency.ci.baptista_pike"`, `"edit.contingency.ci.koopman"`, `"edit.contingency.ci.newcombe_wilson"`, `"edit.contingency.ci.wilson_brown"` at `646-661`) mutate `contingency_*_ci_method` UI fields that are never read by `commit_intent()` (`1510-1531`) or present on `ContingencySpec`.
  3. Two-Group options `"graph_differences"`, `"graph_ranks"`, and `"hodges_lehmann_ci"` (`421-453`) mutate `self.graph_differences`, `self.graph_ranks`, and `self.hodges_lehmann_ci` (also mutated in `gui_actions.rs:792-803`), which `commit_intent()` never emits.
- **Failure scenario:** Toggling `Attributable Risk` in `Edit Analysis Parameters` on a `2 x 2` contingency table leaves `ContingencySpec::compute_odds_and_risk_ratios` `false` and computes no attributable risk / difference in proportions; similarly, changing Contingency CI methods or Two-Group estimation/Hodges–Lehmann checkboxes succeeds in the dialog state but silently disappears on `OK`/`Apply`.
- **Evidence:**
```rust
// crates/ui-egui/src/dialogs_canvas/edit_params.rs:285-288, 662-673, 1510-1531
            AnalysisKind::Contingency(spec) => {
                self.contingency_odds_ratio_checked = spec.compute_odds_and_risk_ratios;
                self.contingency_relative_risk_checked = spec.compute_odds_and_risk_ratios;
            }
...
            "edit.contingency.ci.baptista_pike" => { ... self.contingency_or_ci_method = s.to_owned(); }
            "edit.contingency.ci.koopman" => { ... self.contingency_rr_ci_method = s.to_owned(); }
            "edit.contingency.ci.newcombe_wilson" => { ... self.contingency_arr_ci_method = s.to_owned(); }
            "edit.contingency.ci.wilson_brown" => { ... self.contingency_diag_ci_method = s.to_owned(); }
            "compute_odds_and_risk_ratios" | "contingency_odds_ratio_checked" | "edit.contingency.odds_ratio" => {
                if let Some(b) = val.as_bool() {
                    self.contingency_odds_ratio_checked = b;
                    self.contingency_relative_risk_checked = b;
                    self.update_contingency_spec(|c| c.compute_odds_and_risk_ratios = b);
                }
            }
            "contingency_attributable_risk_checked" | "attributable_risk" | "compute_attributable_risk" | "edit.contingency.attributable_risk" => {
                if let Some(b) = val.as_bool() {
                    self.contingency_attributable_risk_checked = b;
                }
            }
```
- **Suggested fix:** Sync `contingency_attributable_risk_checked` from `spec.compute_odds_and_risk_ratios` and wire its toggle to update `compute_odds_and_risk_ratios` (or add distinct `ContingencySpec` / `TwoGroupSpec` fields in `crates/doc` + `crates/engine` if separate CI/graph toggles are supported rather than read-only informational labels).
- **Related:** `FA-SEC13-11` (`SEC-13`), `gui_actions.rs:792-809`.

### FA-SEC14-05

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/dialogs_canvas/format_axes.rs:101-109, 149-156, 170-173, 197-231, 288-327, 331-371`
- **Category:** `data-loss` / `wrong-result`
- **Defect:** `FormatAxesState` preserves `pending_x_*`, `pending_y_*`, and `pending_y2_additional_ticks` across axis-tab switches, but has three gaps:
  1. There is no `pending_y2_thresholds` field: `stage_active_thresholds` (`107`) and `set_active_thresholds` (`154`) both match `AxisTarget::Y2 => {}`, and `commit_intent()` (`366`) passes `None` for thresholds (`build_pending_axis_patch(None, self.pending_y2_additional_ticks.as_deref())`). Switching from `Y2` to `X` or `Y` calls `sync_from_graph` (`211`) and discards any unsaved `Y2` thresholds.
  2. `switch_target_axis_preserving_thresholds` (`197-231`) only stages `thresholds` and `additional_ticks` before resetting `axis_draft` via `sync_from_graph(gs, next)`: any edits made to `min`/`max`, `scale`, `title`, `segments`, `major_tick_interval`, `tick_format`, `tick_direction`, or `break_ornament` on one axis are discarded when switching to another axis tab inside the same dialog session.
  3. `commit_intent()` (`345-352`) only handles disabling `Y2` (`"y2_axis": Value::Null`) or enabling `frame_preset: DualYSecondaryRight` inside the `AxisTarget::Y2` match arm (`364-369` skips `y2_axis` completely if `enable_y2` is `false` or if `enable_y2` was toggled on while on `X`/`Y` without `pending_y2_additional_ticks`).
- **Failure scenario:** A user edits `Left Y` bounds `(0, 100)` or adds a threshold on `Right Y2`, switches to the `X Axis` tab to tweak the X title, and clicks `OK`: both the `Left Y` bounds and the `Y2` threshold are silently reverted to the prior graph state. Toggling `enable_y2` off and switching to `X`/`Y` before `OK` likewise leaves the `Y2` axis enabled.
- **Evidence:**
```rust
// crates/ui-egui/src/dialogs_canvas/format_axes.rs:102-109, 197-231, 345-369
    pub fn stage_active_thresholds(&mut self) {
        let thresholds = self.axis_draft.thresholds.clone();
        match self.target_axis {
            AxisTarget::X => self.pending_x_thresholds = Some(thresholds),
            AxisTarget::Y => self.pending_y_thresholds = Some(thresholds),
            AxisTarget::Y2 => {}
        }
    }
    ...
    fn switch_target_axis_preserving_thresholds(&mut self, next: AxisTarget, doc: Option<&ProjectDocument>) {
        ...
        if let (Some(d), Some(gid)) = (doc, &self.graph_sheet_id)
            && let Some(gs) = d.graph_sheet(gid)
        {
            self.sync_from_graph(gs, next); // overwrites axis_draft bounds/scale/title/ticks/segments from gs
        }
        ...
    }
    ...
            AxisTarget::Y2 => {
                if self.enable_y2 {
                    payload.insert(String::from("frame_preset"), serde_json::to_value(FramePreset::DualYSecondaryRight).ok()?);
                    payload.insert(String::from("y2_axis"), serde_json::to_value(&self.axis_draft).ok()?);
                } else {
                    payload.insert(String::from("y2_axis"), Value::Null);
                }
            }
```
- **Suggested fix:** Store full per-axis drafts (`x_draft`, `y_draft`, `y2_draft: Option<AxisSpec>`) instead of a single overwritten `axis_draft`, commit all modified axes (`x_axis`, `y_axis`, `y2_axis`, and `frame_preset` whenever `enable_y2` changes regardless of `target_axis`), and preserve all three axis drafts across tab switches.

### FA-SEC14-06

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/menus/mod.rs:23-54`, `crates/ui-egui/src/menus/menu_bar.rs:199-227, 244, 283-284`
- **Category:** `contract-drift` / `data-loss` (`REGRESSION` — incomplete `R15-GUI-07` / `R15-GUI-23` fix)
- **Defect:** `DESKTOP_HOST_ROUTED_COMMANDS` (`menus/mod.rs:23-48`) is documented as the canonical allowlist of commands intercepted by the desktop host for native open/save dialogs, EMF/figure export, window lifecycle, and clipboard integration. However, `menu_bar.rs` exposes `"export.tiff"`, `"table.exportCsv"`, `"analysis.exportCsv"`, `"analysis.exportJson"`, `"table.importCsv"`, `"table.importPzfx"`, legacy project-import action (`line 204`), `"project.saveTemplate"`, `"project.instantiateTemplate"`, `"edit.pasteImage"`, `"graph.insertImage"`, and `"layout.insertImage"` directly in the curated File/Edit/Insert menus, while `DESKTOP_HOST_ROUTED_COMMANDS` omits all of them (and `"ui.print.open"` is also a native host/print surface).
- **Failure scenario:** When the user activates `"export.tiff"`, `"table.exportCsv"`, `"analysis.exportCsv"`, `"analysis.exportJson"`, `"project.saveTemplate"`, or an importer from the in-window menu bar or Command Palette (`filter_palette_commands`), `is_desktop_host_routed_command` returns `false`; the shell executes the engine command with `{}` (or without `"path"`), discarding exported bytes or failing with a missing-path error instead of opening an OS file dialog.
- **Evidence:**
```rust
// crates/ui-egui/src/menus/mod.rs:23-48
pub const DESKTOP_HOST_ROUTED_COMMANDS: &[&str] = &[
    "project.open", "project.save", "project.saveAs", "file.open", "file.open_unpacked_dir",
    "file.save", "file.save_as", "export.svg", "file.export_svg", "export.pdf",
    "file.export_pdf", "export.png", "file.export_png", "graph.exportEmf", "layout.exportEmf",
    "window.new", "file.newWindow", "window.close", "file.close", "file.quit",
    "app.quit", "edit.cut", "edit.copy", "edit.paste",
];
```
```rust
// crates/ui-egui/src/menus/menu_bar.rs:199-228, 283-284
    push_action(&mut import_children, app, "table.importCsv", None);
    ...
    push_action(&mut import_children, app, "table.importPzfx", None);
    push_action(&mut import_children, app, /* legacy project import */ ..., None);
    ...
    push_action(&mut export_children, app, "export.tiff", None);
    push_action(&mut export_children, app, "table.exportCsv", None);
    push_action(&mut export_children, app, "analysis.exportCsv", None);
    push_action(&mut export_children, app, "analysis.exportJson", None);
    ...
    push_action(&mut nodes, app, "project.saveTemplate", None);
    push_action(&mut nodes, app, "project.instantiateTemplate", None);
    ...
    push_action(&mut nodes, app, "graph.insertImage", None);
    push_action(&mut nodes, app, "layout.insertImage", None);
```
- **Suggested fix:** Add all native file-picker / clipboard-image commands surfaced in `menu_bar.rs` and `CommandSpec` (`export.tiff`, `graph.exportSvg`, `graph.exportPdf`, `graph.exportPng`, `table.exportCsv`, `analysis.exportCsv`, `analysis.exportJson`, `table.importCsv`, `table.importPzfx`, legacy importers, `project.saveTemplate`, `project.instantiateTemplate`, `result.generateReport`, `graph.insertImage`, `layout.insertImage`, `edit.pasteImage`) to `DESKTOP_HOST_ROUTED_COMMANDS` (or derive the list from a `CommandSpec` host-routing property).
- **Related:** `FA-SEC19-02` (`SEC-19`), `R15-GUI-07`, `R15-GUI-23`.

### FA-SEC14-07

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/grid_view.rs:1383-1405, 1442-1468, 1484-1502, 1885-1913`
- **Category:** `wrong-result` / `data-loss`
- **Defect:** Two selection state invariants in `GridViewState` drift out of sync between `self.selected_coords` and `self.tool.anchor`/`self.tool.active`:
  1. `has_discontiguous_selection` (`1500-1502`) evaluates `self.selected_coords.len() > 1 || (!self.selected_coords.is_empty() && self.tool.anchor == self.tool.active)`. Whenever `toggle_cmd_selection` (`1383-1405`) or `toggle_command_row_selection` (`1442-1468`) deselects down to a single coordinate (`selected_coords.len() == 1`) from an earlier range where `tool.anchor != tool.active` (e.g., `select_single_sync((0,0,0))` -> `extend_shift_range((0,1,0))` -> `toggle_cmd_selection((0,1,0))`), `has_discontiguous_selection()` returns `false`. Subsequent calls to `is_cell_selected`, `clipboard_target`, `visible_frame_for_sheet` (`1826`), and `emit_toggle_exclude_intents` revert to `self.tool.selection_bounds()` (`(0,0,0)..(0,1,0)`), acting on and rendering the explicitly deselected cell/row while `all_selected_coords()` (`1611`) returns only the 1 remaining cell.
  2. `GridViewState::navigate` (`1885-1913`) updates `self.tool.anchor`/`active` without clearing `self.selected_coords`, `self.selected_rows`, or `self.active_x_cell` and without updating quick stats (`refresh_selection_stats_for_sheet`). Any direct caller of `navigate` after a `Cmd/Ctrl+Click` multi-selection leaves stale `selected_coords` taking precedence across rendering, clipboard cut/copy, exclusion, and quick stats. (Note: `views/grid_core.rs:325-331` manually compensates by calling `select_single_sync`/`sync_extended_selection` afterward, masking the API defect only in that handler.)
- **Failure scenario:** A user drag-selects cells `(0,0)` and `(0,1)`, then `Cmd/Ctrl+Click`s `(0,1)` to remove it from the selection (`selected_coords = [(0,0)]`, `anchor = (0,0)`, `active = (0,1)`), and triggers **Cut** (`edit.cut`) or **Copy** (`edit.copy`). Because `has_discontiguous_selection()` returns `false`, `clipboard_target` resolves `Range((0,0)..=(0,1))` and copies/clears the deselected cell `(0,1)` too.
- **Evidence:**
```rust
// crates/ui-egui/src/grid_view.rs:1387-1404, 1500-1502, 1904-1913
        let mut set: BTreeSet<GridCellCoord> = if !self.selected_coords.is_empty() {
            self.selected_coords.iter().copied().collect()
        } else {
            self.selection_coords(sheet).into_iter().collect()
        };
        if set.contains(&coord) {
            if set.len() > 1 {
                set.remove(&coord);
            }
        } else if set.len() < MAX_MATERIALIZED_SELECTION_COORDS {
            set.insert(coord);
        }
        self.tool.active = Some(coord);
        if self.tool.anchor.is_none() {
            self.tool.anchor = Some(coord);
        }
        ...
    fn has_discontiguous_selection(&self) -> bool {
        self.selected_coords.len() > 1 || (!self.selected_coords.is_empty() && self.tool.anchor == self.tool.active)
    }
    ...
        let next = GridCellCoord::new(next_row, col, subcol);
        if extend_selection {
            if self.tool.anchor.is_none() {
                self.tool.anchor = Some(GridCellCoord::new(curr_row, current.col, current.subcol));
            }
            self.tool.active = Some(next);
        } else {
            self.tool.select_single(next);
        }
```
- **Suggested fix:** When `set.len() == 1` at the end of `toggle_cmd_selection` / `toggle_command_row_selection`, collapse `self.tool.anchor` and `self.tool.active` onto that single coordinate (and simplify `has_discontiguous_selection()` / `visible_frame_for_sheet` to check `!self.selected_coords.is_empty()` whenever `selected_coords` holds an explicit selection). In `navigate`, clear `selected_coords`, `selected_rows`, and `active_x_cell` and refresh selection stats cleanly on both single-step and range extension.

### FA-SEC14-08

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/dialogs_canvas/command_palette.rs:199-235, 272-278`
- **Category:** `contract-drift` / `wrong-result`
- **Defect:** `CommandPaletteState::collect_matches` (`199-235`):
  1. Hardcodes `enabled: true` (`line 226`) for every entry in `UI_COMMANDS` regardless of the active `Session`, instead of calling `ui_command_enabled` (implemented in `crates/ui-egui/src/menus/tooltips.rs:162-207`), and sets `menu_path: "UI"` / search category `"View"` instead of each command's actual menu path. Downstream `filter_palette_commands` (`crates/ui-egui/src/egui_shell/command_palette.rs:64-78`) only checks an ad-hoc override for `ui.result.applyRemediation`, leaving modal UI actions like `ui.graph.axes.open` and `ui.layout.composer.open` marked enabled on empty projects.
  2. Copies `spec.summary` into `params_doc` (`line 207`) for all Engine commands rather than schema parameter documentation, and `activate_selected()` (`274-278`) calls `self.matches()` (without `Session`) and emits `json!({})`, causing parameterised commands invoked via `CommandPaletteState::activate_selected` to fail.
- **Failure scenario:** Opening the Command Palette on an empty project shows contextual UI modal commands (`Format Axes`, `Edit Analysis Parameters`, `Page Layout Composer`, `Format Image`) as enabled (`enabled == true`), and activating any non-zero-arg command through `CommandPaletteState::activate_selected()` sends `{}` instead of routing/filling parameters.
- **Evidence:**
```rust
// crates/ui-egui/src/dialogs_canvas/command_palette.rs:201-232, 274-278
        for spec in cmd::registry().specs() {
            let candidate = PaletteCandidate {
                ...
                params_doc: spec.summary,
                summary: spec.summary,
                beginner: spec.beginner,
                enabled: session.is_none_or(|s| (spec.enabled)(s)),
                origin: CommandOrigin::Engine,
            };
            ...
        }
        for spec in UI_COMMANDS {
            let candidate = PaletteCandidate {
                id: spec.id,
                label: spec.label,
                menu_path: "UI",
                default_shortcut: spec.shortcut,
                params_doc: spec.params_doc,
                summary: spec.summary,
                beginner: spec.beginner,
                enabled: true,
                origin: CommandOrigin::Ui,
            };
            ...
        }
...
    pub fn activate_selected(&self) -> Option<ToolCommandIntent> {
        let list = self.matches();
        let entry = list.get(self.selected_index).or_else(|| list.first())?;
        Some(ToolCommandIntent::new(entry.id, json!({})))
    }
```
- **Suggested fix:** Share the `Session`/`ProjectDocument` precondition helper from `menus::tooltips` so `collect_matches` evaluates real `enabled` states and curated menu paths for `UI_COMMANDS`, expose concise schema parameter hints in `params_doc` for Engine commands, and require session-aware routing/intent building in `activate_selected`.
- **Related:** `menus/tooltips.rs:119-207`, `egui_shell/command_palette.rs:60-79`.

### FA-SEC14-09

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/grid_view.rs:805-822, 854-886, 925-945`
- **Category:** `wrong-result`
- **Defect:** `ImportReshapeState::apply_field_patch_localized` hoists `decimal_separator` checks before the main field loop (`lines 815-822`) so `"data"` / `"plate_values"` parse with the newly requested decimal separator regardless of map key order. However, the hoist only checks `"decimal_separator"` and `"import.decimal_separator"`, omitting the third key alias `"decimal_sep"` that is accepted inside the loop (`lines 879-886`).
- **Failure scenario:** If a caller submits `{"decimal_sep": ",", "data": "1,2;3,4"}` when `self.decimal_separator` was `.`, `effective_preset` retains `decimal_separator = '.'`. Line `859` calls `parse_plate_values_text("1,2;3,4", &effective_preset)`, which treats `,` as a cell delimiter and parses `1,2` into two separate wells `[1.0, 2.0, 3.0, 4.0]` before line `881` sets `self.decimal_separator = ','` after the fact.
- **Evidence:**
```rust
// crates/ui-egui/src/grid_view.rs:815-822, 854-886
        let effective_preset =
            if let Some(ds) = fields.get("decimal_separator").or_else(|| fields.get("import.decimal_separator")).and_then(Value::as_str) {
                let mut p = active_preset;
                p.decimal_separator = if ds.trim() == "," { ',' } else { '.' };
                p
            } else {
                active_preset
            };
...
                "decimal_separator" | "decimal_sep" | "import.decimal_separator" => {
                    let s = val.as_str().ok_or_else(|| String::from("`decimal_separator` must be a string"))?;
                    self.decimal_separator = match s.trim() {
                        "," => ',',
                        "." => '.',
                        _ => return Err(format!("unsupported decimal_separator `{s}`")),
                    };
                }
```
- **Suggested fix:** Check `fields.get("decimal_separator").or_else(|| fields.get("decimal_sep")).or_else(|| fields.get("import.decimal_separator"))` and validate `ds` before parsing `"data"` or `"plate_values"`.

### FA-SEC14-10

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/grid_view.rs:1743-1777, 2114-2130`
- **Category:** `wrong-result` / `data-loss`
- **Defect:** `GridViewState::materialized_selection_flat_rect` (`1743-1777`) and `bounded_row_range_vec` (`2114-2130`) use `last_populated_row_in_range(sheet, r0, window_r1)` and `total_rows <= MAX_MATERIALIZED_SELECTION_COORDS` (`4,096`) in ways that drop legitimate small empty-row selections and miss sparse populated rows:
  1. In `bounded_row_range_vec`, on a sheet with `total_rows > 4_096`, `Shift+Click` across empty rows `[r0, r1]` returns `Vec::new()` (when `r0 != r1`) instead of `(r0..=window_r1).collect()`; even on sheets with `<= 4_096` rows, if row `0` is populated and rows `1..=5` are empty, `Shift+Click` selecting rows `0..=5` truncates `selected_rows` to `[0]`.
  2. In `materialized_selection_flat_rect`, when `total_rows > 4_096`, selecting an unpopulated multi-cell region (`2 x 2`, `span_cells = 4 <= 4096`) returns `None` (`selection_coords` is empty, breaking `toggle_cmd_selection` starting from a small empty range), and selecting a range `0..=5000` over `2` columns (`window_r1 = 2047`) where only row `3000` is populated causes `last_populated_row_in_range(sheet, 0, 2047)` to return `None`, dropping row `3000` from `all_selected_coords` / graph brushing.
- **Failure scenario:** On a `5,000`-row table where rows `0..20` hold data, a user `Shift+Click`s row headers `50..=55` (empty rows) or drags a `2x2` range at rows `50..=51`: `selected_rows` collapses to `[]` and `selection_coords` returns `[]` even though the total selection is only 4–6 cells.
- **Evidence:**
```rust
// crates/ui-egui/src/grid_view.rs:1767-1776, 2120-2130
        let max_rows_for_cols = (MAX_MATERIALIZED_SELECTION_COORDS / (hi - lo + 1).max(1)).max(1);
        let window_r1 = raw_r1.min(r0.saturating_add(max_rows_for_cols - 1));
        if let Some(last_pop) = last_populated_row_in_range(sheet, r0, window_r1) {
            return Some((r0, last_pop, lo, hi));
        }
        let span_cells = (raw_r1 - r0 + 1).saturating_mul(hi - lo + 1);
        if !require_populated_on_small && span_cells <= MAX_MATERIALIZED_SELECTION_COORDS && total_rows <= MAX_MATERIALIZED_SELECTION_COORDS {
            return Some((r0, window_r1, lo, hi));
        }
        None
...
    if clamped_r1 - r0 < MAX_MATERIALIZED_SELECTION_COORDS && total_rows <= MAX_MATERIALIZED_SELECTION_COORDS {
        let end = last_populated_row_in_range(sheet, r0, clamped_r1).unwrap_or(clamped_r1);
        return (r0..=end).collect();
    }
    let window_r1 = clamped_r1.min(r0.saturating_add(MAX_MATERIALIZED_SELECTION_COORDS - 1));
    match last_populated_row_in_range(sheet, r0, window_r1) {
        Some(last_pop) => (r0..=last_pop).collect(),
        None if r0 == clamped_r1 => vec![r0],
        None => Vec::new(),
    }
```
- **Suggested fix:** Check `span_cells <= MAX_MATERIALIZED_SELECTION_COORDS` without requiring `total_rows <= MAX_MATERIALIZED_SELECTION_COORDS`, and for `bounded_row_range_vec` when `!require_populated_on_small && clamped_r1 - r0 < MAX_MATERIALIZED_SELECTION_COORDS`, retain `(r0..=clamped_r1)` instead of truncating at `last_populated_row_in_range` or returning empty. For multi-thousand-row sparse ranges, search `last_populated_row_in_range(sheet, r0, raw_r1)` before clamping the emitted count to `MAX_MATERIALIZED_SELECTION_COORDS`.

### FA-SEC14-11

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/power_wizard.rs:915-993` (cross-checked with `crates/ui-egui/src/lib.rs:538-546`)
- **Category:** `data-loss` / `contract-drift`
- **Defect:** `PowerWizardState::execute_create_with` runs 7–15+ sequential engine commands (`sheet.createData` with `auto_info: true, auto_graph: true`, `table.setTitles`, `table.setXCells`, `table.setCells`, multiple `info.setConstant`, `info.setNote`, `graph.setKind`/`graph.setSpec`/`graph.setAxes`, `analysis.run`, `sheet.select`) without an atomic transaction or error rollback. Neither `PowerWizardState::execute_create` (`991-993`) nor `UiApp::execute_power_wizard_create` (`crates/ui-egui/src/lib.rs:538-546`) rolls back the earlier mutations if a later step fails.
- **Failure scenario:** In a project near the sheet-count limit (e.g., `MAX_SHEETS_PER_PROJECT - 3` sheets), clicking `Create` in the Power Wizard succeeds at `sheet.createData` (adding 3 sheets — Data, Info, Graph — to `doc`), populates all three sheets across 12 commands, and then fails with `TooManySheets` on `analysis.run` (`line 964`), leaving orphaned partially configured sheets and multiple undo history steps in the document.
- **Evidence:**
```rust
// crates/ui-egui/src/power_wizard.rs:923-974
        let create_data_outcome = exec("sheet.createData", json!({ ..., "auto_info": true, "auto_graph": true }))?;
        ...
        exec("table.setTitles", ...)?;
        exec("table.setXCells", ...)?;
        exec("table.setCells", ...)?;
        if let Some(ref info_id) = info_id_str { populate_power_info_sheet(exec, info_id, &res)?; }
        if let Some(ref graph_id) = graph_id_str { configure_power_graph_sheet(exec, graph_id, &res)?; }
        ...
        let res_outcome = exec("analysis.run", ...)?;
```
- **Suggested fix:** Execute `PowerWizardState::execute_create` inside a single undoable transaction (or snapshot/restore `Session` on `Err`, and expose an engine batch/power-create command or rollback hook for `execute_create_with`).

### FA-SEC14-12

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/menus/tooltips.rs:119-207` (cross-checked with `crates/ui-egui/src/menus/ui_commands.rs:16-441` and `crates/ui-egui/src/menus/menu_bar.rs:254-345`)
- **Category:** `contract-drift` / `cleanup`
- **Defect:** `menus/tooltips.rs` is out of sync with `UI_COMMANDS` and `curated_menu_bar`:
  1. `ui_command_menu_info` (`119-160`) omits `16` of the `53` `UI_COMMANDS` (`ui.navigator.toggle`, `ui.inspector.toggle`, `ui.family.toggle`, `ui.palette.open`, `ui.zoom.fitSheet`, `ui.gallery.toggle`, `ui.rulers.toggle`, `ui.pane.cycle`, `ui.shortcut.set`, `ui.shortcut.reset`, `ui.locale.set`, `ui.theme.set`, `ui.screenshot`, `ui.event.dispatch`, `ui.split.toggle`, `ui.previewFilter.set`), sending all `16` to `_ => ("menu.view", "UI")` even though `11` of them live under `View/...` in `menu_bar.rs:254-273`.
  2. Lines `146-148` and `150` return bucket `"menu.insert"` for `"Data/..."` menu paths (`ui.table.convertArchetype.open`, `ui.table.randomizeGroups`, `ui.table.randomSampleRows`, `ui.table.openFormulaBar`), whereas `menu_bar.rs:290-305` places them under `menu.data`.
  3. `ui_command_enabled` (`162-207`) omits `"ui.brush.point"` (`line 181` omits it despite it operating on graphs) as well as sheet-dependent view actions (`"ui.zoom.fitSheet"`, `"ui.view.rowTitles.toggle"`), letting them fall through to `_ => true` (`line 205`) even on empty documents.
- **Failure scenario:** Tooltip and palette queries (`command_tooltip`) report `"UI"` instead of `"View/..."` for View menu toggles, misclassify Data dialogs as `menu.insert`, and report `"ui.brush.point"` and sheet view commands as enabled when no graph or sheet exists.
- **Evidence:**
```rust
// crates/ui-egui/src/menus/tooltips.rs:146-160, 174-206
        "ui.table.convertArchetype.open" => ("menu.insert", "Data/Convert Table Archetype..."),
        "ui.table.randomizeGroups" => ("menu.insert", "Data/Randomize Groups Dialog..."),
        "ui.table.randomSampleRows" => ("menu.insert", "Data/Random Sample Rows Dialog..."),
        "ui.table.simulateDistributions" => ("menu.insert", "Insert/Simulate Distributions Dialog..."),
        "ui.table.openFormulaBar" => ("menu.insert", "Data/Open Formula Bar..."),
        ...
        _ => ("menu.view", "UI"),
...
        "ui.formatPoints.open"
        | "ui.changeGraphType.open"
        | "ui.graph.axes.open"
        | "ui.graph.insertModelEquation"
        | "ui.graph.bubblePlot.open"
        | "ui.graph.format.open"
        | "ui.graph.pairwise.open"
        | "ui.applyStyleFrom.open" => !doc.graph_sheets.is_empty(),
        ...
        _ => true,
```
- **Suggested fix:** Add exhaustive `ui_command_menu_info` arms for all `53` `UI_COMMANDS`, fix the `"menu.data"` bucket for lines `146-148` and `150`, and gate `"ui.brush.point"` (`!doc.graph_sheets.is_empty()`) and sheet-bound view actions in `ui_command_enabled`.
- **Related:** [`FA-SEC14-08`](#fa-sec14-08).

### FA-SEC14-13

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/dialogs_r9.rs:1311-1342`
- **Category:** `wrong-result` / `data-loss`
- **Defect:** In `InsertModelEquationDialogState::commit_intent`, when `self.update_existing` is `true` and a graph already contains two or more `GraphAnnotation::ModelEquation` callouts (`[eq_A, eq_B]`), the update condition `&& (result_sheet_id == res_id || !updated)` (`line 1322`) triggers on the **first** equation (`eq_A`) because `!updated` is `true`, overwrites `eq_A` with `self.result_sheet_id = res_B` and `self.bounds_pt`, sets `updated = true`, and then **also** matches `eq_B` (`result_sheet_id == res_id`) later in the loop, overwriting both annotations identically.
- **Failure scenario:** A graph with two curves fitted by separate regression result sheets `res_A` and `res_B` has two `ModelEquation` annotations (`res_A` at index 0, `res_B` at index 1). Editing `res_B` via `InsertModelEquationDialogState` with `update_existing: true` silently replaces `res_A`'s callout with a duplicate of `res_B` at the same coordinates, destroying `res_A`'s equation annotation.
- **Evidence:**
```rust
// crates/ui-egui/src/dialogs_r9.rs:1318-1332
            let mut next_annotations = gs.spec.annotations.clone();
            let mut updated = false;
            for ann in &mut next_annotations {
                if let GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, mode, bounds_pt, font_size_pt, bordered } = ann
                    && (result_sheet_id == res_id || !updated)
                {
                    *result_sheet_id = res_id.clone();
                    *data_set_index = self.data_set_index;
                    *mode = self.mode;
                    *bounds_pt = self.bounds_pt;
                    *font_size_pt = self.font_size_pt;
                    *bordered = self.bordered;
                    updated = true;
                }
            }
```
- **Suggested fix:** First search for an annotation matching `result_sheet_id == res_id` (and matching `data_set_index` if present); only fall back to the first `ModelEquation` annotation (and stop after updating that single entry) when no annotation already references `res_id`.
