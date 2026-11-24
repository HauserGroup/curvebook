# Audit Report: SEC-18 — Egui Shell Modals & Wizards

- **Section:** `SEC-18` (Egui Shell Modals & Wizards — 37 files, `22,307` LOC)
- **Base SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Date:** `2026-11-22`
- **Scope:**
  - `crates/ui-egui/src/egui_shell/` top modals (`app_modals.rs`, `fit_modal.rs`, `import_wizard.rs`, `manage_vars_modal.rs`, `power_studio.rs`, `stat_anova_modal.rs`, `transform_modals.rs`)
  - `crates/ui-egui/src/egui_shell/graph_modals/` (`mod.rs`, `state.rs`, `helpers.rs`, `apply_style.rs`, `bubble_plot.rs`, `change_graph_type.rs`, `insert_equation.rs`, `page_layout.rs`, `pairwise.rs`, `r13_modals.rs`, `selected_points.rs`, `format_axes/{additional_ticks,mod,segments_ticks,single_axis}.rs`, `format_graph/{heatmap,mod,mv,series}.rs`)
  - `crates/ui-egui/src/egui_shell/stats_modals/` (`mod.rs`, `helpers.rs`, `analyze_picker.rs`, `factorial_mv.rs`, `model_formula.rs`, `nested_misc.rs`, `r9_modals.rs`, `result_viewer.rs`, `stat_params.rs`, `tests.rs`, `two_group.rs`)
- **Mode:** Report-only static audit (`100%` file coverage `1..=N`, cross-checked against downstream handlers in `egui_shell/mod.rs`, `gui_actions.rs`, `interaction.rs`, `grid_view.rs`, `dialogs_canvas/`, `views/`, `apps/curvebook/src/`, and `crates/engine/src/cmd/`).
- **Summary Counts:** `S1 = 4`, `S2 = 10`, `S3 = 5`, `S4 = 1` (**20 findings**; `FA-SEC18-01` .. `FA-SEC18-20`).

---

## File Coverage Table

All paths are relative to `crates/ui-egui/src/egui_shell/`.

| File | LOC | Lines Inspected | Invariants Checked | Findings / Status |
|---|---:|---|---|---|
| `app_modals.rs` | 2,454 | `1..=2454` | Modal exclusivity, Cancel vs Save staging, host SaveFile export routing, checkpoint confirm/create state, Escape/backdrop handling, `atr()` i18n, dead fields | `FA-SEC18-01`, `FA-SEC18-05`, `FA-SEC18-11`, `FA-SEC18-14`, `FA-SEC18-17`, `FA-SEC18-18`, `FA-SEC18-20` |
| `fit_modal.rs` | 2,010 | `1..=2010` | Numeric alert completeness (`constant_drafts`, `x_min/max`, `r2_threshold`), DSL compile gating, equation catalog/hook truncation, state sync/reset, `Save as Default`, dead bindings | `FA-SEC18-06`, `FA-SEC18-07`, `FA-SEC18-20` |
| `import_wizard.rs` | 1,329 | `1..=1329` | 5-stage draft propagation on `Import`, `Save Preset`, `Next`/`Back` backend stage sync, `InternalResultSheet` source list & preview, `ImportReshape`/`ConvertArchetype` & Escape | `FA-SEC18-02`, `FA-SEC18-13`, `FA-SEC18-14` |
| `manage_vars_modal.rs` | 807 | `1..=807` | Master-detail selection, `Name`/`Unit`/`Rename/Merge` vs `OK`/`Cancel` commit & rollback semantics, `table.setTitles`/`table.setVariableRole`/`table.recodeLevels` payloads, Escape | `FA-SEC18-08`, `FA-SEC18-14` |
| `power_studio.rs` | 1,048 | `1..=1048` | 4-stage navigation in Create vs Edit mode, Stage 2 `effect_mode` (`Pilot` vs `Direct`) and `SurvivalLogRank` UI parity, stepper clamps, sidebar scrubbers, session reset | `FA-SEC18-09` |
| `stat_anova_modal.rs` | 2,348 | `1..=2348` | 1-way ANOVA tabs, `R1`/`R2`/`R3` predicates, `Assume sphericity` radio toggle, `Save as Default`, Diagnostics flags vs engine params, `StepperSpec` / textbox sync | `FA-SEC18-06`, `FA-SEC18-15` |
| `transform_modals.rs` | 1,959 | `1..=1959` | Standalone vs Edit sync/reset, `Simulate`/`RandomSample` body & params, `extract_binding_*` dead inputs, formula syntax checks, col cap truncation, `ExcludeWithReason`, Escape | `FA-SEC18-04`, `FA-SEC18-12`, `FA-SEC18-14` |
| `graph_modals/mod.rs` | 470 | `1..=470` | Sub-dialog dispatch, Apply/OK commit ordering, FormatAxes tab/target sync, Heatmap local sync & overwrite on commit, Cancel reset, Escape | `FA-SEC18-03`, `FA-SEC18-10`, `FA-SEC18-14`, `FA-SEC18-16` |
| `graph_modals/state.rs` | 362 | `1..=362` | Tab/schema detection across graph kinds, Heatmap/MV/Axes draft field defaults | `FA-SEC18-03`, `FA-SEC18-16` |
| `graph_modals/helpers.rs` | 363 | `1..=363` | Locale-aware heatmap range validation (`validate_heatmap_range`), RGBA conversion, label truncation (`max_width.max(8.0)`), widget recording, axis reset | Clean — checked locale number parsing, `lo >= hi` rejection, zero unsafe indexing, and inspect metadata helpers. |
| `graph_modals/apply_style.rs` | 116 | `1..=116` | Donor single-select patch, multi-target checkbox/Select All (donor exclusion), clone toggles (`apply_style.*`) | `FA-SEC18-14` (modal shell Escape gap; donor/target/clone logic verified Clean). |
| `graph_modals/bubble_plot.rs` | 283 | `1..=283` | Available column bounds (`.min(n_cols-1)`), categorical/continuous/constant color patches, size bounds, convex hulls & confidence ellipses | `FA-SEC18-14` (modal shell Escape gap; column cycling & patch keys verified Clean). |
| `graph_modals/change_graph_type.rs` | 541 | `1..=541` | Category filter, recommended/all options, summary/error-bar/grouped/survival controls, preview rendering (`temp.spec`) | `FA-SEC18-14` (modal shell Escape gap; option filtering & intents verified Clean). |
| `graph_modals/insert_equation.rs` | 96 | `1..=96` | Result picker, Substituted/Symbolic mode patches, font size clamp (`[6, 36]`), bordered callout checkbox | `FA-SEC18-14` (modal shell Escape gap; patch keys & bounds verified Clean). |
| `graph_modals/page_layout.rs` | 165 | `1..=165` | Journal presets, grid summary, orientation toggle, align-origins, badge styles, text callout addition | `FA-SEC18-14` (modal shell Escape gap; callout validation & updates verified Clean). |
| `graph_modals/pairwise.rs` | 187 | `1..=187` | `pairwise.auto_add` gating, display/bracket/label modes, CLD font `[6, 24]` & offset `[0, 48]`, uppercase & mean ordering | `FA-SEC18-14` (modal shell Escape gap; enablement & state updates verified Clean). |
| `graph_modals/r13_modals.rs` | 666 | `1..=666` | `CreateGraph`, `EqualizeScaling` (`[5, 500]%`), `FormatEmbeddedTable` styles, `FormatImage` aspect lock / crop caps / border options | `FA-SEC18-14` (modal shell Escape gap; DragValue ranges & crop non-negative bounds verified Clean). |
| `graph_modals/selected_points.rs` | 125 | `1..=125` | Color/symbol/size (`[2, 24]`)/border (`[0.5, 6]`) patches, hide & clear-overrides checkboxes | `FA-SEC18-14` (modal shell Escape gap; point-override patches verified Clean). |
| `graph_modals/format_axes/mod.rs` | 202 | `1..=202` | Session/generation sync, axis-switch draft reset & silent error clearing, frame/single-axis/titles routing | `FA-SEC18-10` |
| `graph_modals/format_axes/additional_ticks.rs` | 188 | `1..=188` | Locale-aware tick/shaded-end parsing, bounded `remove(idx)` on existing ticks/thresholds, validation error reporting | Clean — verified localized numeric parsing, bounded deletion, and shaded-zone validation. |
| `graph_modals/format_axes/segments_ticks.rs` | 469 | `1..=469` | Multi-segment bounds (`[lo, hi]`), major interval validation (`axis_ticks_error`), minor ticks input parsing/blur fallback | `FA-SEC18-10` |
| `graph_modals/format_axes/single_axis.rs` | 238 | `1..=238` | Title patch, scale/tick-format categorical confirmation guard, break ornament, stroke width `[0.25, 6.0]` | Clean — checked categorical downgrade confirmation guard, break ornament patches, and axis validation. |
| `graph_modals/format_graph/mod.rs` | 57 | `1..=57` | FormatGraph tab dispatch across Appearance, Settings, Heatmap, MV, and Legends | Clean — checked schema-gated tab dispatch to specialized sub-renderers. |
| `graph_modals/format_graph/heatmap.rs` | 236 | `1..=236` | Colormap/reverse/auto-range/min-max/labels/angle/tile-gap controls vs `GraphModalEguiState` vs `FormatGraphState` | `FA-SEC18-03` |
| `graph_modals/format_graph/mv.rs` | 539 | `1..=539` | MV X/Y/Color/Size/Label mappings, convex hull/ellipse toggles, OOR policy (`Clamp` vs `Omit`), dead `let _ = fg;` | `FA-SEC18-16`, `FA-SEC18-20` |
| `graph_modals/format_graph/series.rs` | 843 | `1..=843` | Series selection (`series.len()` bounds), markers, error bars, bar border/X-nudge `ctx.data` spinner keys, overlays, palette/paper-fill | `FA-SEC18-17` |
| `stats_modals/mod.rs` | 66 | `1..=66` | `StatsModalsEguiState` defaults and submodule exports | Clean — verified default fields and clean module re-exports. |
| `stats_modals/helpers.rs` | 580 | `1..=580` | Shared `show_wave4_modal` sizing/ScrollArea/backdrop capture, `Role::Dialog` metadata, dataset checklist, TeX preview | Clean — verified bounded layout math, backdrop scroll/click capture, and checklist index handling. |
| `stats_modals/analyze_picker.rs` | 657 | `1..=657` | Search/category filtering, MV/PCA/PCR/Outlier config, Enter/Escape handling, duplicate prompt, compatible target selection, dead `active_tab` bind | `FA-SEC18-20` (core Analyze/Duplicate/ChangeData logic & Escape verified Clean). |
| `stats_modals/factorial_mv.rs` | 387 | `1..=387` | Two/Three-way ANOVA, Correlation (`all_pairs`/`heatmap`), Clustering (`k_means_k`/`k_sweep_max`), Multiple Imputation (`m`/`iters`) | Clean — checked bounds clamps (`clamp(2, 12)`, `clamp(2, 50)`, `clamp(5, 200)`), predicates, and draft intents. |
| `stats_modals/model_formula.rs` | 412 | `1..=412` | Combinatorial formula generation (`collect_term_combinations`), GLM/Cox/KM/MI variable binding & options | Clean — verified full k-subset formula terms (`R15-GUI-03` fix intact) and survival/GLM/MI draft patches. |
| `stats_modals/nested_misc.rs` | 480 | `1..=480` | Contingency CI predicates (`r6_contingency_ci_selector`), Nested/Bland-Altman/PValueStack/PassingBablok/ROC/Outlier/LinearRegression | Clean — checked `r6` predicates, numeric clamps, and spec patches. |
| `stats_modals/r9_modals.rs` | 455 | `1..=455` | `RandomizeGroups`, `RandomSampleRows`, `SimulateDistributions` steppers, column bounds, `dialogs_graph_misc` actions, Escape, dead `active_tab` bind | `FA-SEC18-14`, `FA-SEC18-20` (domain controls & `dialogs_graph_misc` action parity verified Clean). |
| `stats_modals/result_viewer.rs` | 106 | `1..=106` | Read-only result tab clamping (`tab_idx`), scrollable preview table, Close/OK footer, Escape | `FA-SEC18-14` (tab bounds and scroll table verified Clean). |
| `stats_modals/stat_params.rs` | 860 | `1..=860` | Shared stat param shell, linear range textbox sync (`fit.lr.range.input.*`), `Save as Default`, Diagnostics toggles (`diag_qq`/`diag_shapiro`), Escape | `FA-SEC18-06` |
| `stats_modals/tests.rs` | 41 | `1..=41` | Unit test for `build_live_symbolic_formula` interaction expansion | Clean — unit-test-only module. |
| `stats_modals/two_group.rs` | 162 | `1..=162` | Gaussian/Paired predicates, two-group method radios (`welch_en` misuse), `OneSampleT`/`OneSampleWilcoxon` coverage, MV role grid | `FA-SEC18-19` |

---

## Findings

### `FA-SEC18-01` · `S1` · `ExportFigure` modal silently drops PDF, SVG, PNG, and TIFF exports without opening Save File or writing to disk

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/app_modals.rs:1129-1138` (cross-module seam: `crates/ui-egui/src/gui_actions.rs:2542-2561, 2823-2832`, `crates/ui-egui/src/dialogs_canvas/layout_export.rs:229-253`, `crates/ui-egui/src/menus/mod.rs:23-48`)
- **Category:** `data-loss` / `contract-drift` (**REGRESSION — incomplete fix of `R15-GUI-23`**; spans into `SEC-19`)
- **Defect:** Clicking `Export` (`modal.ok`) in `render_export_modal` for PDF, SVG, PNG, or TIFF dispatches `intents.push_dialog_button("ok")` instead of host-routed file-picker export, executing the engine export command directly in-memory with `path: None` and discarding the rendered bytes while closing the dialog.
- **Failure scenario:** Open `File › Export Figure…` on any Graph or Layout sheet, choose `PDF (Vector)`, `SVG (Vector)`, `PNG (Raster)`, or `TIFF`, optionally adjust DPI, physical dimensions, transparent background, or color space (`Display-P3`, `Grayscale`), and click `Export`. Only `EMF` (`state.export_emf_selected`) queues `intents.push_engine(cmd, ...)` for `gui_window.rs` native SaveFile routing (`R15-GUI-23`). For all four other formats, `commit_active_dialog_action` runs `commit_single_intent` -> `app.execute_command` directly; because `ExportFigureState::commit_intent` omits `"path"`, `crates/engine/src/cmd/export.rs:481,590,723,765` skips `write_atomic_output_file`, throws away the transient payload, and closes the modal with no file saved and no error shown. Furthermore, `export_transparent_bg` (`app_modals.rs:1326-1350`) is never passed to any engine command, `Display-P3`/`Grayscale` collapse to `"srgb"` for PDF and are ignored for SVG/PNG/TIFF (`gui_actions.rs:2545-2550`), and `width_px`/`height_px`/`export.preset.*mm` are ignored for SVG/PDF (`layout_export.rs:231-232`).
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/app_modals.rs:1129-1138
  if state.export_emf_selected {
      let cmd = if is_layout { "layout.exportEmf" } else { "graph.exportEmf" };
      state.export_emf_selected = false;
      intents.push_engine(cmd, json!({ "sheet_id": sheet_id.as_str() }));
      intents.push(UiIntent::CloseAllModals);
  } else {
      intents.push_dialog_button("ok");
  }
  ```
  ```rust
  // crates/ui-egui/src/gui_actions.rs:2542-2556 & 2823-2831
  ActiveDialog::ExportFigure { sheet_id } => {
      let mut intent = app.canvas_dialogs.export_figure.commit_intent(&sheet_id);
      let export_opts = &app.view_shards.dialogs_graph_misc;
      if matches!(app.canvas_dialogs.export_figure.format, ExportFigureFormat::Pdf)
          && let Some(obj) = intent.params.as_object_mut()
      {
          let pdf_cs = if export_opts.export_color_space == "CMYK-Proof" { "cmyk" } else { "srgb" };
          obj.insert(String::from("color_space"), json!(pdf_cs));
      }
      ...
      let mut res = commit_single_intent(app, Some(intent), true)?;
  ```
- **Suggested fix:** Add `"graph.exportSvg"`, `"graph.exportPdf"`, `"graph.exportPng"`, and `"export.tiff"` to `DESKTOP_HOST_ROUTED_COMMANDS` (`menus/mod.rs`), queue host-routed `UiIntent::EngineCommand` from `app_modals.rs` (closing the modal on dispatch or host completion) with the full export parameter payload, and strip/hide or implement unsupported vector dimensions, transparency, and color-space options honestly.
- **Related:** `R15-GUI-23`, `GWF-28`, `SEC-19`.

---

### `FA-SEC18-02` · `S1` · 5-stage `Import & Paste Special` wizard shows transformed preview while silently dropping Stage 1/3/4/5 drafts on commit

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/import_wizard.rs:433-451, 593-605, 681-714, 765-788, 1111-1138, 1152-1237` (commit path `crates/ui-egui/src/egui_shell/mod.rs:2301-2369`, `crates/ui-egui/src/grid_view.rs:1149-1183`)
- **Category:** `wrong-result` / `contract-drift` / `data-loss`
- **Defect:** Footer `Import` (`modal.ok`) in the 5-stage `Import & Paste Special` wizard copies only `state.raw_source_draft` onto `wiz.raw_clipboard_text` and executes `app.grid.import_wizard.build_command_intent(...)`, silently ignoring every local option edited on `ImportWizardEguiState` (`missing_tokens_draft`, `filter_predicate_draft`, `block_reshape`, `plate_values_per_row_draft`, `plate_rows_per_dataset_draft`, `encoding_name`, `extract_info_metadata`, `add_floating_note`) even though the preview (`compute_wizard_transformed_grid`) reflects them.
- **Failure scenario:** User pastes a plate or delimited dataset into Stage 1, customizes `Missing sentinels` (e.g. `NODATA`), adds a `Text filter` in Stage 3, selects `96-Well Plate (8×12)` or `Row-Wrap Block` in Stage 4 (`Placement`), confirms the reshaped/filtered table in the preview grid, and clicks `Import`: `CommitImportWizard` runs `table.paste` / `table.pasteLink` on raw `ImportPasteWizardState` with none of `missing_tokens_draft`, `filter_predicate_draft`, or `block_reshape` applied, corrupting imported geometry and retaining unwanted rows/tokens. Additionally, clicking `Save Preset` (`import_paste.btn.save_default`, `786-787`) has no `.clicked()` handler, and `Next >` / `< Back` (`1129-1138`) do not synchronize `wiz.stage`.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/import_wizard.rs:433-451, 593-605, 681-714, 1111-1122
  row.label("Missing sentinels:");
  let mr = row.add(egui::TextEdit::singleline(&mut state.missing_tokens_draft)...);
  ...
  row.label("Text filter:");
  let fr = row.add(egui::TextEdit::singleline(&mut state.filter_predicate_draft)...);
  ...
  (WizardBlockReshape::Plate96, "plate96", "96-Well Plate (8×12)")
  ...
  if ok_resp.clicked() {
      if is_wizard {
          let mut wiz = app.grid.import_wizard.clone();
          if !state.raw_source_draft.is_empty() {
              wiz.raw_clipboard_text = state.raw_source_draft.clone();
          }
          intents.push_update_import_wizard(wiz);
          intents.push_commit_import_wizard(true);
  ```
- **Suggested fix:** Move `missing_tokens`, `filter_pattern`, `block_reshape` / plate dimensions, `encoding`, and Info/FloatingNote options into `ImportPasteWizardState` + engine command schemas (`table.paste` / `table.importPlate` / `table.pasteLink`), wire `import_paste.btn.save_default`, and sync `wiz.stage` on `Next`/`Back`.
- **Related:** `FA-SEC18-13`, `SEC-17` (`ImportPasteWizardState`).

---

### `FA-SEC18-03` · `S1` · `Format Graph` Heatmap modal never loads existing heatmap settings and unconditionally overwrites `multivariate.color` and `x_axis.tick_angle_deg` on commit

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/graph_modals/mod.rs:337-422`, `crates/ui-egui/src/egui_shell/graph_modals/state.rs:228-236, 288-296`, `crates/ui-egui/src/egui_shell/graph_modals/format_graph/heatmap.rs:26-97, 205`
- **Category:** `wrong-result` / `data-loss` (**related: `GWF-21`**)
- **Defect:** `sync_format_graph_local_drafts` never initializes `heatmap_colormap`, `heatmap_reversed`, `heatmap_auto_range`, `heatmap_min_draft`, `heatmap_max_draft`, or `heatmap_tick_angle_deg` from the open heatmap `GraphSheet` (nor resets them on `Cancel`), while `commit_graph_modal_action` unconditionally pushes `graph.setSpec { multivariate }` and `graph.setAxes { tick_angle_deg }` from those unsynced/stale defaults on `OK`/`Apply`.
- **Failure scenario:** Open `Format Graph` on a Heatmap previously saved with e.g. `Plasma` reversed ramp, explicit clamp `[0, 100]`, and `45°` X-axis tick rotation, change only legend visibility or tile gap (or cancel edits on Heatmap A then open Heatmap B), and click `OK`: `commit_graph_modal_action` overwrites `multivariate.color` with `state.heatmap_colormap` (default `Viridis`, unreversed, `domain_clamp: None` unless previously touched) and overwrites `x_axis.tick_angle_deg` with `state.heatmap_tick_angle_deg` (default `0.0`). Meanwhile, `gctx.intents.push_engine("graph.setSpec", ...)` executes before `push_dialog_button(action)` (`FormatGraphState::commit_intents()`), so if `mv_levels_dirty` is also set, the second `graph.setSpec` carries stale `fg.mv_encoding`.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/graph_modals/mod.rs:337-361, 400-421
  fn sync_format_graph_local_drafts(app: &UiApp, state: &mut GraphModalEguiState, sheet_id: &SheetId) {
      ...
      if let Some(enc) = enc_opt
          && let ColorChannelMapping::Continuous { domain_clamp: Some((lo, hi)), .. } = enc.color
      {
          state.mv_clamp_min_draft = ...; // Only mv_clamp_* synced; heatmap_* never touched
      ...
  }
  ...
  enc.color = ColorChannelMapping::Continuous {
      column: col,
      ramp: ColorRamp::Named { id: state.heatmap_colormap, reversed: state.heatmap_reversed },
      domain_clamp: clamp,
  };
  gctx.intents.push_engine("graph.setSpec", json!({ "sheet_id": gid.as_str(), "multivariate": enc }));
  gctx.intents.push_engine("graph.setAxes", json!({ "sheet_id": gid.as_str(), "axis": "x", "tick_angle_deg": state.heatmap_tick_angle_deg }));
  gctx.intents.push_dialog_button(action);
  ```
- **Suggested fix:** Sync `heatmap_colormap`, `heatmap_reversed`, `heatmap_auto_range`, `heatmap_min_draft`, `heatmap_max_draft`, and `heatmap_tick_angle_deg` from `GraphSheet` inside `sync_format_graph_local_drafts` (plus `Cancel`/sheet-switch resets), and update `fg.mv_encoding` directly via `FormatGraphState` instead of issuing competing `graph.setSpec` commands.
- **Related:** `GWF-21`.

---

### `FA-SEC18-04` · `S1` · Standalone `Simulate XY Curve` / `Random Sample Rows` show `Extract & Rearrange` UI and hardcode ignored parameters; `Extract & Rearrange` column bindings are dead

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/transform_modals.rs:258-260, 1463-1513, 1734-1756, 1768-1771`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `render_transform_modal` routes `TransformModalKind::Simulate` and `TransformModalKind::RandomSample` to `render_extract_and_rearrange_body` (rendering table-format pills and column-binding inputs), `build_standalone_transform_params` discards all UI state to execute hardcoded `seed=42 / row_count=13 / replicates=3` and `seed=42 / sample_size=5`, and `extract_binding_x`/`extract_binding_y`/`extract_binding_grp` are never read on commit in either `ExtractAndRearrange` or edit mode.
- **Failure scenario:** User opens standalone `Simulate XY Curve` or `Random Sample Rows`, configures table target / `Predictor / X` / `Response / Y`, and clicks `OK`: instead of any user-configured simulation equation/X-range/replicates or sample size/stratification, `transform.run` runs with hardcoded constants (`seed: 42, row_count: 13, replicates: 3` or `seed: 42, sample_size: 5`). In `ExtractAndRearrange`, values typed into `edit.transform.extract.binding_x/y/grp` (`1467-1509`) are similarly dropped because `build_standalone_transform_params` (`1734-1756`) never serializes them.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/transform_modals.rs:258-260 & 1768-1771
  TransformModalKind::ExtractAndRearrange | TransformModalKind::Simulate | TransformModalKind::RandomSample => {
      render_extract_and_rearrange_body(body, &mut modal_cx, effective_kind)
  }
  ...
  TransformModalKind::Simulate => {
      Ok(json!({ "source_sheet_id": ds.id.as_str(), "kind": "simulate", "seed": 42, "row_count": 13, "replicates": 3 }))
  }
  TransformModalKind::RandomSample => Ok(json!({ "source_sheet_id": ds.id.as_str(), "kind": "random_sample", "seed": 42, "sample_size": 5 })),
  ```
- **Suggested fix:** Give `Simulate` and `RandomSample` dedicated parameter editors (seed, row count, replicates, X range, equation; sample size, stratify column, proportional toggle) wired in both standalone and `EditAnalysisParameters` paths, and either pass `extract_binding_*` into `SelectAndTransform` / extraction commands or remove the unbound inputs.
- **Related:** `FA-SEC18-12`.

---

### `FA-SEC18-05` · `S2` · `Preferences` Cancel fails to revert immediate Theme, UI Scale, Locale, and Navigator Nesting mutations

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/app_modals.rs:452-482, 500-509, 533-558, 578-595` (intent handler `crates/ui-egui/src/egui_shell/mod.rs:2670-2686`)
- **Category:** `wrong-result` / `robustness` (**partial fix of `R15-GUI-25`**)
- **Defect:** Navigator tree mode (`508`), theme (`541`), UI scale (`557`), and locale (`593`) push live `UiIntent` mutations on every click/spin inside `Preferences`, and clicking `Cancel` (`468`, `ClosePreferences { save: false }`) only discards `staged_shortcuts` without reverting any of those four settings.
- **Failure scenario:** User opens `Preferences`, clicks `Dark` theme, `150%` UI scale, `fr` locale, or `Nested Data + Results` navigator layout to preview how it looks, then clicks `Cancel` expecting changes to be aborted: all four changes persist on `UiApp` (`app.theme.mode`, `app.theme.ui_scale`, `app.locale`, `app.nav.nesting_mode`).
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/app_modals.rs:467-469, 508, 541, 557, 593
  intents.push_close_preferences(false); // Cancel
  ...
  intents.push_set_navigator_nesting(mode);
  intents.push(UiIntent::SetTheme(mode));
  intents.push(UiIntent::SetUiScale(clamped));
  intents.push(UiIntent::SetLocale(code.to_owned()));
  ```
  ```rust
  // crates/ui-egui/src/egui_shell/mod.rs:2673-2686
  UiIntent::ClosePreferences { save } => {
      if save { ... app.theme.ui_scale = scale; if let Some(staged) = ...take() { app.shortcuts = staged; } }
      app.egui_views.app_modals.preferences.discard_shortcut_draft();
      app.egui_views.app_modals.preferences_open = false;
  }
  ```
- **Suggested fix:** Snapshot initial `theme`, `ui_scale`, `locale`, and `nesting_mode` when `OpenPreferences` runs and restore them on `ClosePreferences { save: false }` (or stage them identically to `staged_shortcuts` until `OK`).
- **Related:** `R15-GUI-25`.

---

### `FA-SEC18-06` · `S2` · Diagnostics checkboxes and `"Save as Default"` in ANOVA / Stat Params / Fit modals have no effect on analysis output or persisted defaults

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/stat_anova_modal.rs:1268-1288, 1451-1460, 1511-1523`, `crates/ui-egui/src/egui_shell/stats_modals/stat_params.rs:386-398, 797-819`, `crates/ui-egui/src/egui_shell/fit_modal.rs:272-283` (handler `crates/ui-egui/src/egui_shell/mod.rs:2593-2596`, `crates/ui-egui/src/views/stat_params.rs:1150-1174`)
- **Category:** `wrong-result` / `contract-drift` (**related: `GWF-20`**)
- **Defect:** Toggling the 5 diagnostics checkboxes (`edit.diag.*` -> `toggle_diag:*`) updates only unused `StatParamsViewState` booleans (`diag_qq_plot`, `diag_residuals_plot`, `diag_homoscedasticity`, `diag_assumption_checklist`, `diag_influence_leverage`) that are never read by `EditAnalysisParametersState::commit_intent` or the engine, while `"Save as Default"` (`dialog.button.save_default`) only toggles `method_saved_as_default = true` on transient modal structs (`stat_anova_modal.rs` never even sends `UiIntent::SaveEditParamsAsDefault`).
- **Failure scenario:** User unchecks Q-Q / Residuals / Shapiro-Wilk diagnostics or checks Leverage & Influence before clicking `OK` — committed `analysis.updateParams` carries zero diagnostic flags and produces identical engine output. Similarly, clicking `"Save as Default"` highlights the button as saved, yet closing the modal or opening a second ANOVA/t-test/fit ignores it completely.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/stat_anova_modal.rs:1268-1287 & 1458-1460
  for (wid, payload, lbl, checked) in [
      ("edit.diag.qq_plot", "toggle_diag:qq", ..., st.diag_qq_plot),
      ("edit.diag.residuals_table", "toggle_diag:residuals", ..., st.diag_residuals_plot),
      ...
  ] { if render_modal_checkbox(...) { intents.push_custom_view_action("stat_params", payload); } }
  ...
  { state.method_saved_as_default = true; } // dialog.button.save_default in stat_anova_modal.rs
  ```
  ```rust
  // crates/ui-egui/src/egui_shell/mod.rs:2593-2596
  UiIntent::SaveEditParamsAsDefault => {
      app.egui_views.anova_modal.method_saved_as_default = true;
      app.egui_views.stats_modals.method_saved_as_default = true;
  }
  ```
- **Suggested fix:** Add persistent diagnostic options to the relevant `AnalysisSpec` structs (or remove unbacked checkboxes), and store/apply per-analysis default presets in `UiApp` preferences when `SaveEditParamsAsDefault` fires (including emitting `push_save_edit_params_as_default()` from `stat_anova_modal.rs`).
- **Related:** `GWF-20`.

---

### `FA-SEC18-07` · `S2` · Nonlinear Fit modal misses validation for Constant value drafts and Custom DSL syntax errors, truncates Compare/Hook lists, and leaks `constant_drafts` on equation change

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/fit_modal.rs:420-453, 478-520, 818-834, 906-920, 973, 1065-1093, 1113`
- **Category:** `wrong-result` / `robustness`
- **Defect:** `check_draft_numeric_alerts` checks `x_min_draft`/`x_max_draft`, `initial_override_drafts`, `lower_bound_drafts`, and `upper_bound_drafts`, but omits `state.constant_drafts` and `CompiledDslModel::compile` errors; switching built-in equations (`910-919`) fails to clear `state.constant_drafts`; `render_fit_compare_tab` (`973`) limits selectable comparison equations to `builtin_catalog().iter().take(18)`; and the inline hook picker (`1113`) caps options at `.take(12)`.
- **Failure scenario:** Fix parameter `HillSlope` to `Constant`, type `"not_a_number"` into `fit.con.const.HillSlope`, and click `OK`: `blocking_error` stays `None` and the modal silently commits the previous `Constant(val)` instead of reporting a validation error. Similarly, entering invalid Custom DSL leaves `OK` enabled (`831-833`), switching equations reuses prior parameter `constant_drafts` of the same name (e.g. `Top`), comparison equations past index 17 in the catalog cannot be selected in `Compare`, and projects with >12 constants truncate inline hook candidates.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/fit_modal.rs:500-519, 916-918, 973, 1085-1089, 1113
  // check_draft_numeric_alerts loops over initial_override_drafts, lower_bound_drafts, upper_bound_drafts — omits constant_drafts & custom_dsl
  ...
  state.initial_override_drafts.clear();
  state.lower_bound_drafts.clear();
  state.upper_bound_drafts.clear(); // constant_drafts not cleared
  ...
  for entry in builtin_catalog().iter().take(18) { ... }
  ...
  if ed.changed() {
      state.constant_drafts.insert(p.clone(), text.clone());
      if let Some(parsed) = parse_fit_number(&text, locale_preset) {
          update_param_rule(spec, p, |r| r.scope = ParameterScope::Constant(parsed), intents);
      }
  }
  ...
  for opt in picker.options.iter().take(12) { ... }
  ```
- **Suggested fix:** Validate `constant_drafts` and `CompiledDslModel::compile` failures inside `compute_fit_alerts` so invalid entries block commit, clear `constant_drafts` and `model_search` on equation change/session sync, and replace `.take(18)` / `.take(12)` with searchable scrollable lists.
- **Related:** `R15-GUI-16`, `R15-GUI-17`.

---

### `FA-SEC18-08` · `S2` · `Manage Variables & Levels` OK discards unapplied Name/Unit/Rename drafts while inline actions mutate immediately past Cancel

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/manage_vars_modal.rs:239-260, 403-453, 757-765`
- **Category:** `data-loss` / `wrong-result`
- **Defect:** Footer `OK` (`251-258`) commits only `recode_mappings` and closes, silently discarding un-saved `state.title_draft`, `state.unit_draft`, and `state.rename_draft` text edits, whereas inline `"Save"`, role buttons, `"Auto"`, drag-reorder, and `"Apply Rename/Merge"` mutate the engine document immediately so `Cancel` (`244`) / `"×"` (`156`) cannot undo them.
- **Failure scenario:** User selects variable `Dose`, edits `Name:` to `Concentration` and `Unit:` to `nM` (or types a new category name in `Rename/Merge to:`), clicks `OK` directly without clicking the separate inline `"Save"` / `"Apply Rename/Merge"` button, and loses those edits silently. Conversely, clicking `"Categorical (CAT)"` or `"Apply Rename/Merge"` and then clicking `"Cancel"` leaves the document mutated.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/manage_vars_modal.rs:243-258 & 403-408
  if cancel_resp.clicked() {
      intents.push_close_manage_variables();
  }
  ...
  if ok_resp.clicked() {
      if let Some((ds, _)) = mv_ctx && !app.grid.variables_dialog.recode_mappings.is_empty() {
          intents.push_apply_manage_var_recodes(ds.id.clone());
      }
      intents.push_close_manage_variables();
  }
  ...
  if ar.clicked() {
      let next_title = combine_title_and_unit(&state.title_draft, &state.unit_draft);
      if !next_title.is_empty() {
          intents.push_engine("table.setTitles", json!({ "sheet_id": ds.id.as_str(), "col": col, "title": next_title }));
      }
  }
  ```
- **Suggested fix:** Commit pending `title_draft`/`unit_draft` and `rename_draft` on `OK` (or stage all variable metadata/role/order/recode changes locally in modal draft state and apply atomically on `OK` while discarding on `Cancel`).
- **Related:** `R15-GUI-21`.

---

### `FA-SEC18-09` · `S2` · `Power & Sample Size Studio` Stage 2 ignores `effect_mode == Standardized` for t-tests and shows `p1`/`p2` steppers for `SurvivalLogRank`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/power_studio.rs:78-80, 414-551` (cf. `crates/ui-egui/src/views/power_wizard.rs:172-177, 198-226`)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `render_stage2_effect` branches exclusively on `pw.family` and ignores both `pw.effect_mode` (`Pilot Data Inputs` vs `Direct Standardized Effect`) and `SurvivalLogRank` state; additionally `PowerStudioEguiState` (`results_tab`, `hover_n`, `narrative_copied`) never resets when the dialog closes (`78-80`).
- **Failure scenario:** Pick `Two-Sample Unpaired t-test` in Stage 1, switch to `Direct Standardized Effect` (`power.effect_mode.direct`) in Stage 2: the UI still displays `Expected Mean Difference` and `Common SD (σ)` steppers instead of a standardized `d` stepper, so adjusting `mean_diff` overwrites `d` via `derived_standardized_effect()`. Similarly, picking `Survival (Log-Rank)` shows `Proportion p₁` / `Proportion p₂` steppers with no hazard-ratio/event controls, and `One-Way ANOVA` / `Simple Linear Regression` have no pilot-mode inputs (`group_means` / `r_squared` / `beta1`).
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/power_studio.rs:453-551
  match pw.family {
      PowerTestFamily::TwoSampleUnpairedT | PowerTestFamily::TwoSamplePairedT | PowerTestFamily::OneSampleT => {
          // Always renders Expected Mean Difference + Common SD even when pw.effect_mode == Standardized
      }
      PowerTestFamily::TwoProportions => {
          // Renders Proportion p1 / p2 even when SurvivalLogRank is active
      }
      _ => {
          // OneWayAnova, Regression, Correlation, ChiSquare always render standardized_effect even in Pilot mode
      }
  }
  ```
- **Suggested fix:** Branch Stage 2 inputs on `(pw.family, pw.effect_mode, app.view_shards.power_wizard.survival_log_rank)` and reset `PowerStudioEguiState` transient fields when `!is_power_studio_dialog_active(app)`.
- **Related:** `WF-15`.

---

### `FA-SEC18-10` · `S2` · `Format Axes` minor ticks silently drops invalid input and `X/Y/Y2` tab clicks cause 1-frame draft mismatch and silent error clearing

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/graph_modals/mod.rs:212-225, 423-426`, `crates/ui-egui/src/egui_shell/graph_modals/format_axes/mod.rs:42-80`, `crates/ui-egui/src/egui_shell/graph_modals/format_axes/segments_ticks.rs:424-445`
- **Category:** `wrong-result` / `robustness`
- **Defect:** `axes.seg.0.minor_ticks_per_interval` (`segments_ticks.rs:426-445`) never sets `state.axis_ticks_error` when unparseable text is entered (unlike `major_tick_interval`) and silently snaps back on `lost_focus`; additionally, clicking `X Axis` / `Left Y` / `Right Y2` (`graph_modals/mod.rs:212-225`) mutates `state.format_axes_tab` immediately while deferring `axes.target_axis` via `PatchActiveDialog`, causing 1 frame of old `axis_draft` rendered under the new tab while `sync_format_axes_local_drafts` clears any pending `axis_bounds_error` / `axis_ticks_error` (`format_axes/mod.rs:63,67`).
- **Failure scenario:** Enter `"xyz"` into `Minor Ticks per Interval` and immediately click `OK` (or `Apply`): `axis_ticks_error` remains `None`, so commit proceeds silently with the prior minor-tick count instead of blocking. Entering invalid Min/Max bounds on `X Axis` and clicking `Left Y Axis` clears `axis_bounds_error`, re-enabling `OK` while the previous invalid axis change waits inside `axis_draft`.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/graph_modals/format_axes/segments_ticks.rs:426-445
  if resp.changed() {
      let trimmed = buf.trim();
      if let Ok(parsed) = trimmed.parse::<u16>() { ... push_minor_ticks_update(gctx, fa, clamped); }
      // Invalid input leaves state.axis_ticks_error == None
  } else if resp.lost_focus() {
      ... else if trimmed.parse::<u16>().is_err() { buf = state.axis_minor_ticks_draft.to_string(); }
  }
  ```
- **Suggested fix:** Set `state.axis_ticks_error` when `axes.seg.0.minor_ticks_per_interval` is non-empty non-numeric or `> 10`, drive tab highlight from `fa.target_axis` (or apply the switch synchronously cleanly), and validate bounds/ticks per-axis on commit rather than dropping error state on axis switch.
- **Related:** `R15-GUI-15`, `R15-GUI-24`, `GWF-18`.

---

### `FA-SEC18-11` · `S2` · `Checkpoints` modal restores snapshots (`MAY_DISCARD_DATA`) without confirmation and advances `name_draft` before creation succeeds

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/app_modals.rs:1855-1871, 1903-1919` (engine effect in `crates/engine/src/cmd/project.rs:786-828, 978-990`)
- **Category:** `data-loss` (**related: `R15-GUI-31`**)
- **Defect:** Clicking `Restore Checkpoint` (`checkpoints.restore.{idx}`) in `render_checkpoints_modal` immediately dispatches `project.restoreCheckpoint` (which overwrites the live `ProjectDocument`) with zero confirmation step, while `Create Checkpoint` (`checkpoints.create`) advances `name_draft` and clears `note_draft` optimistically before knowing if `project.createCheckpoint` succeeded.
- **Failure scenario:** User makes post-checkpoint edits, opens `Checkpoints`, and misclicks `Restore` on an older checkpoint: `ctx.session.doc` is immediately replaced (`project.rs:813`) with no confirmation prompt. If `project.createCheckpoint` fails (e.g., checkpoint cap reached), the user's typed `note_draft` is erased prematurely.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/app_modals.rs:1916-1919 & 1863-1870
  if app_button(..., "checkpoints.restore.{idx}", ...).clicked() {
      intents.push_engine("project.restoreCheckpoint", json!({ "id": cp.id }));
  }
  ...
  intents.push_engine("project.createCheckpoint", Value::Object(params));
  let next_default = format!("Checkpoint {}", doc.checkpoints.len() + 2);
  intents.push(UiIntent::PatchActiveDialog { fields: serde_json::Map::from_iter([
      (String::from("name"), Value::String(next_default)),
      (String::from("note"), Value::String(String::new())),
  ]) });
  ```
- **Suggested fix:** Add a two-step inline confirmation prompt before executing `project.restoreCheckpoint` (and optionally `deleteCheckpoint`), and reset `name_draft` / `note_draft` only after `project.createCheckpoint` succeeds (as `gui_actions.rs:2636-2639` already does).
- **Related:** `R15-GUI-31`.

---

### `FA-SEC18-12` · `S2` · `transform_modals.rs` leaks draft state across reopens, adds unvalidated derived formulas, miswires `Hook Constant`, and truncates columns at 16/20

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/transform_modals.rs:341-431, 887, 1039-1056, 1124, 1186`
- **Category:** `wrong-result` / `robustness`
- **Defect:** `sync_standalone_transform_state` fails to clear `standalone_x_preset`, `standalone_y_preset`, `standalone_error_propagation`, `pharmacology_mode`, `baseline_cols`, `baseline_op`, `constant_k_draft`, `selected_k_op`, and `hook_picker_open`; `sync_transform_edit_state` fails to reset `constant_k_draft`, `hook_picker_open`, `baseline_cols`/`baseline_op`, `derived_title_draft`, `formula_draft`, `filter_rows`, and `extract_binding_*`; `MvFilterAndTransform` adds `DerivedColumnFormula` without validating expression syntax (`1039-1056`) while its `Select Rows` `"Hook Constant"` button (`1124`) writes to unused `constant_k_draft`; and column checklists cap at `.min(16)` (`RemoveBaseline:887`) and `.min(20)` (`MvFilterAndTransform:1186`).
- **Failure scenario:** Cancel `Remove Baseline` after picking Col 3 + `Ratio`, reopen on a different sheet, and stale Col 3 + `Ratio` remain active. On a 24-column table, columns `>= 16` (`RemoveBaseline`) or `>= 20` (`MvFilterAndTransform`) are unreachable. Entering an invalid derived expression and clicking `Add Derived` stores the broken formula and clears `formula_draft`.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/transform_modals.rs:887, 1039-1055, 1124, 1186
  let col_count = ds.map_or(2, |s| s.archetype.column_count().min(16)); // RemoveBaseline
  ...
  if add_resp.clicked() { // no formula parse check before pushing DerivedColumnFormula
      ...
      state.derived_title_draft.clear();
      state.formula_draft.clear();
  }
  ...
  if widgets::secondary_button(row, "Hook Constant").clicked() { state.hook_picker_open = !state.hook_picker_open; }
  ...
  for c in 0..s.archetype.column_count().min(20) { ... } // MvFilterAndTransform
  ```
- **Suggested fix:** Reset all modal draft fields inside `sync_standalone_transform_state` and `sync_transform_edit_state` whenever `(kind, sheet_id)` changes or modal closes, validate derived/filter formulas before `Add Derived`, remove or wire `Hook Constant` in `Select Rows`, and remove `.min(16)`/`.min(20)` column caps cleanly.
- **Related:** `R15-GUI-09`, `R15-GUI-10`, `FA-SEC18-04`.

---

### `FA-SEC18-13` · `S2` · `ImportWizard` lists `data_sheets` instead of `result_sheets` for `InternalResultSheet` and renders empty preview

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/import_wizard.rs:335-358, 1190`
- **Category:** `wrong-result` / `robustness`
- **Defect:** When `origin_kind == ImportOriginKind::InternalResultSheet`, the Source Sheet selector loops over `&app.document().data_sheets` instead of `result_sheets`, and `compute_wizard_transformed_grid` unconditionally returns `Vec::new()`.
- **Failure scenario:** User selects `InternalResultSheet` as the import source to link an analyzable Result sheet into a new or existing table: only Data sheets appear as selectable sources and the parsed preview grid always displays `0 rows × 0 cols`.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/import_wizard.rs:335-339 & 1190
  } else if matches!(wiz.origin_kind, ImportOriginKind::InternalDataSheet | ImportOriginKind::InternalResultSheet) {
      ui.horizontal_wrapped(|row| {
          row.label("Source Sheet:");
          for ds in &app.document().data_sheets { ... }
  ...
  ImportOriginKind::InternalResultSheet => Vec::new(),
  ```
- **Suggested fix:** Iterate `&app.document().result_sheets` (filtered by `analyzable_data.is_some()` or active tab) when `origin_kind == ImportOriginKind::InternalResultSheet`, and populate `compute_wizard_transformed_grid` from `rs.analyzable_data`.
- **Related:** `FA-SEC18-02`, `R15-GUI-20`.

---

### `FA-SEC18-14` · `S2` · 22 modals and sub-dialogs fail to handle `Escape` (and host/backdrop never dismisses them)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/app_modals.rs:282-1618`, `import_wizard.rs:156-192`, `manage_vars_modal.rs:121-278`, `transform_modals.rs:233-296, 1831-1951`, `graph_modals/mod.rs:85-160`, `stats_modals/r9_modals.rs:24-455`, `stats_modals/result_viewer.rs:17-106` (supporting seams: `inspect_bridge.rs:205-250`, `apps/curvebook/src/gui_window.rs:1032-1043`, `interaction.rs:1595-1602`)
- **Category:** `robustness`
- **Defect:** `fit_modal`, `power_studio`, `stat_anova_modal`, `stat_params`, `analyze_picker`, `About`, `Checkpoints`, and `Feedback` explicitly handle `Key::Escape` to cancel/close, whereas `Preferences`, `UnsavedChanges`, `FindReplace`, `ExportFigure`, `Print`, `ImportWizard` / `ImportReshape` / `ConvertArchetype`, `ManageVariables`, `TransformModals` / `ExcludeWithReason`, all 13 `graph_modals` dialogs, `r9_modals`, and `ResultViewer` never handle `Key::Escape` — and neither `finalize_egui_modal_response` nor `gui_window.rs::dispatch_key_event` dismisses open egui modals on `Escape` (nor does `ModalResponse::should_close()` trigger close).
- **Failure scenario:** Pressing `Esc` while any of those 22 modal dialogs is open on desktop does nothing; keyboard-only users cannot dismiss `Preferences`, `ExportFigure`, `Print`, `FindReplace`, `Import & Paste Special`, `Manage Variables`, `Transform Values`, `Format Graph`, `Format Axes`, `Change Graph Type`, `Page Layout`, `Result Viewer`, or `Simulate Distributions` via `Esc`.
- **Evidence:**
  ```rust
  // Handled in fit_modal.rs:312, power_studio.rs:252, stat_anova_modal.rs:431,
  // stats_modals/analyze_picker.rs:188,524,653, stat_params.rs:427,
  // app_modals.rs:1656 (About), 1791 (Checkpoints), 1979 (Feedback).
  // Completely absent in Unsaved/Preferences/FindReplace/Export/Print, import_wizard.rs,
  // manage_vars_modal.rs, transform_modals.rs, all graph_modals/*, r9_modals.rs, result_viewer.rs.
  ```
- **Suggested fix:** Handle `Key::Escape` (and/or `modal_resp.should_close()`) centrally in `finalize_egui_modal_response` or each modal's footer/key handler so every modal dismisses cleanly with the appropriate Cancel intent (`dialog.button("cancel")`, `ClosePreferences(false)`, `CloseFindReplace`, `CloseImportWizard`, `CloseTransformModal`, `CloseManageVariables`).
- **Related:** `R15-GUI-33`.

---

### `FA-SEC18-15` · `S3` · `stat_anova_modal.rs`: clicking already-selected `"Assume sphericity"` radio toggles Geisser-Greenhouse ON

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/stat_anova_modal.rs:946-949`
- **Category:** `wrong-result` / `robustness`
- **Defect:** `"Assume sphericity (unadjusted df)"` radio button (`edit.rm.assume_sphericity`) emits `"toggle_geisser_greenhouse"` whenever clicked, even when `!gg_checked` (already selected).
- **Failure scenario:** On the `Repeated Measures` tab when `"Assume sphericity"` is already selected, clicking it toggles `geisser_greenhouse` back to `true`, inverting the radio selection unexpectedly.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/stat_anova_modal.rs:946-949
  if render_modal_radio(ui, rc.tokens, meta, "edit.rm.assume_sphericity", "Assume sphericity (unadjusted df)", !gg_checked && !reml_active, &sph_en)
  {
      intents.push_custom_view_action("stat_params", "toggle_geisser_greenhouse");
  }
  ```
- **Suggested fix:** Guard the click with `if gg_checked` (or add a dedicated idempotent `set_geisser_greenhouse:false` / `true` action and guard both controls).
- **Related:** `none`.

---

### `FA-SEC18-16` · `S3` · `format_graph/mv.rs`: `"Omit Outliers"` out-of-range policy is local-only and resets to `ClampToBounds` on reopen

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/graph_modals/format_graph/mv.rs:452-475`, `crates/ui-egui/src/egui_shell/graph_modals/mod.rs:350-360`
- **Category:** `contract-drift` / `wrong-result`
- **Defect:** Clicking `format_graph.mv.oor_omit` (`"Omit Outliers"`) sets transient `state.mv_out_of_range = MvOutOfRangePolicy::Omit` and clears `domain_clamp = None`, which `sync_format_graph_local_drafts` always resets back to `ClampToBounds` because `MultivariateGraphEncoding` has no out-of-range omission mode.
- **Failure scenario:** User sets a continuous color clamp on a multivariate bubble plot, clicks `"Omit Outliers"`, and commits `OK`: out-of-range points are never omitted (auto-scaling applies with `domain_clamp: None`) and reopening `Format Graph` shows `"Clamp to Bounds"` selected again.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/graph_modals/format_graph/mv.rs:466-474
  state.mv_out_of_range = MvOutOfRangePolicy::Omit;
  if let ColorChannelMapping::Continuous { column, ramp, domain_clamp: Some(_) } = &enc.color {
      enc.color = ColorChannelMapping::Continuous { column: *column, ramp: ramp.clone(), domain_clamp: None };
      ...
  }
  // graph_modals/mod.rs:355,359 unconditionally assigns state.mv_out_of_range = MvOutOfRangePolicy::ClampToBounds;
  ```
- **Suggested fix:** Either persist an `OutOfRangePolicy` field on `ColorChannelMapping::Continuous` (and honor it in `crates/chart/src/multivariate_layout.rs`) or remove the unbacked `"Omit Outliers"` option.
- **Related:** `WF-48`.

---

### `FA-SEC18-17` · `S3` · Unkeyed spinner `ctx.data` caches in `series.rs` and un-cleared `export_emf_selected` in `AppModalsEguiState::close_all`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/graph_modals/format_graph/series.rs:391, 672`, `crates/ui-egui/src/egui_shell/app_modals.rs:193-198`
- **Category:** `robustness`
- **Defect:** `series.rs` stores `ScrubUnitFieldState` in `ctx.data` under static IDs (`"format_graph.bar_border_width_spinner"`, `"format_graph.series_offset_x_spinner"`) unkeyed by graph sheet ID or `selected_series_index`, and `AppModalsEguiState::close_all` (`app_modals.rs:193-198`) fails to clear `export_emf_selected` (likewise `find_replace.match_count` / `about_tab`).
- **Failure scenario:** While editing a bar border width or X-nudge spinner on Series 0, switching to Series 1 or another Graph sheet carries over the active `editing`/`draft_text` spinner buffer across series/graphs. Closing modals via `CloseAllModals` while EMF is chosen leaves `export_emf_selected = true` on reopen.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/graph_modals/format_graph/series.rs:391 & 672
  let spin_id = egui::Id::unique("format_graph.bar_border_width_spinner");
  ...
  let off_spin_id = egui::Id::unique("format_graph.series_offset_x_spinner");
  ```
  ```rust
  // crates/ui-egui/src/egui_shell/app_modals.rs:193-198
  pub fn close_all(&mut self) {
      self.preferences.discard_shortcut_draft();
      self.preferences_open = false;
      self.unsaved_changes_open = false;
      self.find_replace_open = false;
  }
  ```
- **Suggested fix:** Key spinner `Id::unique` values by `(fg.graph_sheet_id.as_ref(), fg.selected_series_index)` (and clear on close/series change), and reset `self.export_emf_selected = false` inside `AppModalsEguiState::close_all`.
- **Related:** `FA-SEC18-01`, `FA-SEC18-03`.

---

### `FA-SEC18-18` · `S3` · Hardcoded English strings across `app_modals::atr` and modal renderers

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/app_modals.rs:501-502, 529-531, 947-1025, 1155-1375, 1414-1565, 1817-1908, 2402-2440` (and inline strings in `fit_modal.rs`, `import_wizard.rs`, `manage_vars_modal.rs`, `power_studio.rs`, `transform_modals.rs`, `graph_modals/`, `stats_modals/`)
- **Category:** `i18n` / `contract-drift`
- **Defect:** `app_modals::atr()` translates `"prefs.title"` for only 8 of 14 Beta locales and `"prefs.cat.*"` for only `de` and `ja`, returning hardcoded English for all other `atr` keys (`"prefs.general.tree_heading"`, `"prefs.appearance.*"`, `"prefs.language.*"`, `"prefs.shortcuts.filter"`, `"export.title.layout"`, `"export.commit"`, `"print.commit"`), while dozens of interactive labels (`Find:`, `Replace:`, `Match Case`, `Create Checkpoint`, `Save as Default`, `Transform Values`, `Fit Curve`, etc.) bypass `tr()` across `SEC-18`.
- **Failure scenario:** Running the GUI in `fr`, `es`, `ko`, `pt-BR`, `zh-CN`, `zh-TW`, `it`, `nl`, `pl`, `ru`, or `tr` displays mixed English labels on Preferences tabs/headings, Find & Replace, Export, Print, Checkpoints, Fit, Import Wizard, Manage Variables, Power Studio, and Transform dialogs.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/app_modals.rs:2414-2439
  ("prefs.cat.general", "de") => "Allgemein",
  ("prefs.cat.general", "ja") => "一般",
  ("prefs.cat.general", _) => "General",
  ...
  ("prefs.general.tree_heading", _) => "Project Navigator Organization",
  ("export.commit", _) => "Export",
  ("print.commit", _) => "Print / Spool PDF",
  ```
- **Suggested fix:** Move modal UI strings into `crates/ui-egui/src/i18n/` with full coverage across all 14 active Beta locales and replace `atr()` / inline literals with `tr(locale, key)`.
- **Related:** `docs/evidence/user-interface.md` (14-locale gate).

---

### `FA-SEC18-19` · `S3` · `two_group.rs`: `"Welch's t"` radio reuses checkbox enablement predicate `welch_en` and `OneSample` methods are absent

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/stats_modals/two_group.rs:41-65` (predicate in `crates/ui-egui/src/modal_predicates.rs:292-299`)
- **Category:** `robustness` / `wrong-result`
- **Defect:** `render_two_group_design` passes `welch_en` (`r4_two_group_welch_correction(t.method)`, which disables Welch's correction checkbox whenever `t.method` is not `UnpairedT`/`WelchT`/`LognormalUnpairedT`) to the `"edit.tg.welch_t"` radio button (`47`), and omits `TwoGroupMethod::OneSampleT` and `TwoGroupMethod::OneSampleWilcoxon` (plus hypothetical value controls) from the method selector (`45-55`).
- **Failure scenario:** When a user selects `Paired Student t`, `Mann-Whitney U`, `Wilcoxon signed-rank`, or `Kolmogorov-Smirnov`, the `"Welch's t"` radio button becomes disabled (`ControlEnablement::disabled`), blocking direct switching back to Welch's t until `Unpaired Student t` is clicked first. Opening `EditAnalysisParameters` on an existing `OneSampleT` / `OneSampleWilcoxon` analysis displays no selected radio option and no hypothetical value control.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/stats_modals/two_group.rs:41-55
  let welch_en = r4_two_group_welch_correction(t.method);
  ...
  ("edit.tg.unpaired_t", "Unpaired Student t", TwoGroupMethod::UnpairedT, ControlEnablement::enabled()),
  ("edit.tg.welch_t", "Welch's t", TwoGroupMethod::WelchT, welch_en.clone()),
  ```
- **Suggested fix:** Use `ControlEnablement::enabled()` for the `"edit.tg.welch_t"` method radio button (keeping `welch_en` solely on `"edit.two_group.welch"` checkbox), and add `OneSampleT`/`OneSampleWilcoxon` radios + hypothetical-value stepper when `TwoGroupMethod::OneSample*` is active.
- **Related:** `WF-07`, `WF-08`.

---

### `FA-SEC18-20` · `S4` · Dead local bindings and unused fields (`about_tab`, `PValueFormat` in `fit_modal.rs`, unused `active_tab` / `fg`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/ui-egui/src/egui_shell/fit_modal.rs:1613`, `crates/ui-egui/src/egui_shell/app_modals.rs:1648`, `crates/ui-egui/src/egui_shell/stats_modals/r9_modals.rs:32`, `crates/ui-egui/src/egui_shell/stats_modals/analyze_picker.rs:444, 542`, `crates/ui-egui/src/egui_shell/graph_modals/format_graph/mv.rs:482`
- **Category:** `cleanup`
- **Defect:** Multiple modal renderers suppress unused arguments/fields with `let _ = ...;` instead of tightening signatures or implementing the missing controls (notably `fit_modal.rs:1613` leaving `&spec.p_value_format` / `PValueFormat::FourDecimal` / `column_titles_for_draft` unused, and `app_modals.rs:1648` leaving `state.about_tab` unused).
- **Failure scenario:** `FitModal`'s Output tab exposes no `PValueFormat` selector (requiring silent suppression at `:1613`), and dead parameter noise obscures unused tab/state fields across sub-modals.
- **Evidence:**
  ```rust
  // crates/ui-egui/src/egui_shell/fit_modal.rs:1613
  let _ = (&spec.p_value_format, PValueFormat::FourDecimal, column_titles_for_draft);
  // app_modals.rs:1648 -> let _ = state.about_tab;
  // stats_modals/r9_modals.rs:32 -> let _ = &state.active_tab;
  // stats_modals/analyze_picker.rs:444,542 -> let _ = &state.active_tab;
  // graph_modals/format_graph/mv.rs:482 -> let _ = fg;
  ```
- **Suggested fix:** Remove dead fields/unused parameters (`about_tab`, `&state.active_tab` in stateless dialogs, `let fg` in `render_fg_legends`), and either expose `PValueFormat` on `FitModalTab::Output` or remove the unused import/binding.
- **Related:** `none`.
