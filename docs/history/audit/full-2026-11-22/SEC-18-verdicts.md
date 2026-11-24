# Verification Verdicts: SEC-18 — Egui Shell Modals & Wizards

- **Section:** `SEC-18` (Egui Shell Modals & Wizards)
- **Source Report:** [`docs/audit/full-2026-11-22/SEC-18.md`](SEC-18.md)
- **Baseline SHA:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-frontend`
- **Date:** `2026-11-22`

## Summary

| Verdict | Count | Finding IDs |
|---|---:|---|
| `CONFIRMED` | 20 | `FA-SEC18-01`..`FA-SEC18-20` |
| `PLAUSIBLE` | 0 | — |
| `REFUTED` | 0 | — |
| `DUPLICATE` | 0 | — |

| Settled Severity | Count | Finding IDs |
|---|---:|---|
| `S1` | 4 | `FA-SEC18-01`, `FA-SEC18-02`, `FA-SEC18-03`, `FA-SEC18-04` |
| `S2` | 9 | `FA-SEC18-05`, `FA-SEC18-06`, `FA-SEC18-07`, `FA-SEC18-08`, `FA-SEC18-09`, `FA-SEC18-10`, `FA-SEC18-11`, `FA-SEC18-12`, `FA-SEC18-13` |
| `S3` | 6 | `FA-SEC18-14` (adjusted from `S2`), `FA-SEC18-15`, `FA-SEC18-16`, `FA-SEC18-17`, `FA-SEC18-18`, `FA-SEC18-19` |
| `S4` | 1 | `FA-SEC18-20` |

## File Coverage Completeness Check

- **Result:** **PASS (37 / 37 files, 22,307 LOC — 100% covered)**
- Independently verified all files under `crates/ui-egui/src/egui_shell/` (`app_modals.rs`, `fit_modal.rs`, `import_wizard.rs`, `manage_vars_modal.rs`, `power_studio.rs`, `stat_anova_modal.rs`, `transform_modals.rs`, all 19 files in `graph_modals/`, and all 11 files in `stats_modals/`) via directory enumeration and `wc -l`. Every file and exact LOC matches the File Coverage Table in `SEC-18.md`.

---

## Per-Finding Verdicts

### Verdict `FA-SEC18-01` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (data-loss on desktop figure exports)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/app_modals.rs:1130-1138`: only `state.export_emf_selected` queues `intents.push_engine(cmd, json!({ "sheet_id": sheet_id.as_str() }))` (`graph.exportEmf` / `layout.exportEmf`); PDF, SVG, PNG, and TIFF hit the `else` branch and emit `intents.push_dialog_button("ok")`.
  - `crates/ui-egui/src/menus/mod.rs:23-48`: `DESKTOP_HOST_ROUTED_COMMANDS` contains `"graph.exportEmf"` and `"layout.exportEmf"`, plus `"export.svg"`, `"export.pdf"`, `"export.png"`, `"file.export_*"`, but omits `"graph.exportSvg"`, `"graph.exportPdf"`, `"graph.exportPng"`, and `"export.tiff"`.
  - `crates/ui-egui/src/egui_shell/mod.rs:1949-1952` -> `crates/ui-egui/src/gui_actions.rs:2144, 2542-2561, 2823-2832`: `DialogButtonAction("ok")` enters `commit_active_dialog_action` directly (`ActiveDialog::ExportFigure`), bypassing `apps/curvebook/src/gui_window.rs:550-581` host routing. `ExportFigureState::commit_intent` (`crates/ui-egui/src/dialogs_canvas/layout_export.rs:229-253`) omits `"path"`, `"width_px"`/`"height_px"` for SVG/PDF, and `"transparent_bg"`. `gui_actions.rs:2545-2556` collapses non-CMYK PDF color spaces to `"srgb"`, ignores color space for SVG/PNG/TIFF, snapshots `export_options` only into the return JSON, and calls `commit_single_intent` (`app.execute_command`, `close = true`).
  - `crates/engine/src/cmd/export.rs:481-483, 590-592, 723-725, 765-767`: without `params.path`, `write_atomic_output_file` is skipped, the rendered bytes are returned only in `CommandOutcome::payload`, and `BatchCommitOutcome` discards them on desktop while closing the modal. (Note: `apps/curvebook-web/src/host_adapters.rs:2933-2959` separately inspects `ActiveDialog::ExportFigure` before commit to trigger browser download, whereas desktop writes nothing.)

### Verdict `FA-SEC18-02` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (silent data-loss / wrong imported table geometry vs preview)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/import_wizard.rs:433-451, 593-605, 681-714, 765-766`: `encoding_name`, `missing_tokens_draft`, `filter_predicate_draft`, `block_reshape`, `plate_values_per_row_draft`, `plate_rows_per_dataset_draft`, `extract_info_metadata`, and `add_floating_note` live solely on `ImportWizardEguiState`.
  - `import_wizard.rs:1152-1237`: `compute_wizard_transformed_grid` applies `missing_tokens_draft`, `filter_predicate_draft`, and `block_reshape` (`RowWrapBlock`, `Plate96`, `Plate384`) to the preview table.
  - `import_wizard.rs:1111-1118`, `crates/ui-egui/src/grid_view.rs:990-1010, 1149-1183`, and `crates/ui-egui/src/egui_shell/mod.rs:2301-2369`: on clicking `Import` (`modal.ok`), only `state.raw_source_draft` is copied onto `wiz.raw_clipboard_text`, and `ImportPasteWizardState::build_command_intent` sends `table.paste` / `table.pasteLink` with none of those `ImportWizardEguiState` options. Repo-wide grep confirms zero reads of those 8 fields outside `import_wizard.rs`.
  - `import_wizard.rs:786-787`: `import_paste.btn.save_default` (`"Save Preset"`) has no `.clicked()` handler; `1129-1138`: `Next >` and `< Back` mutate `state.active_step` without updating `wiz.stage`.

### Verdict `FA-SEC18-03` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (silent overwrite of existing heatmap colormap and X-axis tick angle)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/graph_modals/mod.rs:337-361`: `sync_format_graph_local_drafts` only initializes `mv_clamp_min_draft`, `mv_clamp_max_draft`, and `mv_out_of_range`; it never reads `heatmap_colormap`, `heatmap_reversed`, `heatmap_auto_range`, `heatmap_min_draft`, `heatmap_max_draft`, or `heatmap_tick_angle_deg` from the active `GraphSheet`. Repo-wide grep confirms these `heatmap_*` fields keep their `GraphModalEguiState::default()` values (`Viridis`, `reversed: false`, `auto_range: true`, `0.0` deg at `state.rs:288-296`) unless edited in the current UI run, and `Cancel` (`mod.rs:288-297`) does not reset them.
  - `graph_modals/mod.rs:368-421`: on `OK`/`Apply` for `FormatGraphSchema::Heatmap`, `commit_graph_modal_action` unconditionally pushes `graph.setSpec { multivariate }` with `ColorRamp::Named { id: state.heatmap_colormap, reversed: state.heatmap_reversed }` and `domain_clamp: clamp`, plus `graph.setAxes { axis: "x", tick_angle_deg: state.heatmap_tick_angle_deg }`, overwriting saved heatmap colormap/range/tick rotation. Note: `mv_levels_dirty` (`dialogs_canvas/format_graph.rs:641`) is not normally toggled in Heatmap tabs (`format_graph/mod.rs:30-32`), though if pre-set by a prior MV edit/patch, `push_dialog_button(action)` executes after the `graph.setSpec` at `405` and would overwrite `multivariate` with `fg.mv_encoding`.

### Verdict `FA-SEC18-04` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (silently computes hardcoded simulation/sample ignoring UI controls)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/transform_modals.rs:258-260`: `TransformModalKind::ExtractAndRearrange | TransformModalKind::Simulate | TransformModalKind::RandomSample` all invoke `render_extract_and_rearrange_body`.
  - `transform_modals.rs:1463-1513`: standalone mode renders `edit.transform.extract.binding_x`, `binding_y`, and `binding_grp` textboxes backed by `state.extract_binding_*`.
  - `transform_modals.rs:1768-1771`: `build_standalone_transform_params` ignores all UI fields for `Simulate` and `RandomSample`, emitting hardcoded `{"kind": "simulate", "seed": 42, "row_count": 13, "replicates": 3}` and `{"kind": "random_sample", "seed": 42, "sample_size": 5}`.
  - Repo-wide grep confirms `extract_binding_x`, `extract_binding_y`, and `extract_binding_grp` are only cleared (`354-356`) and bound to textboxes (`1467-1508`), never read on commit in `build_standalone_transform_params` (`1734-1756`) or edit mode.

### Verdict `FA-SEC18-05` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (partial fix of `R15-GUI-25`; Cancel leaves live mutations persisted)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/app_modals.rs:508, 541, 557, 593` immediately push `UiIntent::SetNavigatorNesting`, `UiIntent::SetTheme`, `UiIntent::SetUiScale`, and `UiIntent::SetLocale`.
  - `crates/ui-egui/src/egui_shell/mod.rs:1909-1923, 2010-2012` mutate `app.theme`, `app.locale`, `app.theme.ui_scale`, and `app.nav.nesting_mode` directly during interaction.
  - `app_modals.rs:466-469` (`Cancel`) pushes `ClosePreferences { save: false }`, and `egui_shell/mod.rs:2673-2686` only discards `staged_shortcuts` and closes `preferences_open` without snapshotting or restoring any of those four settings.

### Verdict `FA-SEC18-06` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (user options and defaults silently dropped)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/stat_anova_modal.rs:1268-1287` and `crates/ui-egui/src/egui_shell/stats_modals/stat_params.rs:797-818` dispatch `toggle_diag:*` custom view actions to `crates/ui-egui/src/views/stat_params.rs:1150-1174`, which flip booleans on `StatParamsViewState` (`diag_qq_plot`, `diag_residuals_plot`, `diag_homoscedasticity`, `diag_influence_leverage`, `diag_assumption_checklist`). Repo-wide grep confirms no commit or engine path reads these five booleans.
  - `stat_anova_modal.rs:1458-1460, 1522-1523` only sets `state.method_saved_as_default = true` locally (not even pushing `SaveEditParamsAsDefault`); `stats_modals/stat_params.rs:396-397` and `fit_modal.rs:282` push `SaveEditParamsAsDefault`, which in `egui_shell/mod.rs:2593-2596` sets `anova_modal.method_saved_as_default = true` and `stats_modals.method_saved_as_default = true` without persisting or applying defaults anywhere.

### Verdict `FA-SEC18-07` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent stale constant commit, missing DSL block, truncated lists)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/fit_modal.rs:478-520` (`check_draft_numeric_alerts`) and `522-592` (`compute_fit_alerts`) validate `x_min_draft`, `x_max_draft`, `initial_override_drafts`, `lower_bound_drafts`, and `upper_bound_drafts`, but never validate `state.constant_drafts` or `CompiledDslModel::compile(&state.custom_equation_draft)` failures, so `blocking_error` (`188-189`) stays `None` and `OK` silently commits the previous `Constant(val)` (`1085-1089`).
  - `fit_modal.rs:916-918`: switching built-in equations clears `initial_override_drafts`, `lower_bound_drafts`, and `upper_bound_drafts`, omitting `state.constant_drafts`.
  - `fit_modal.rs:973`: `render_fit_compare_tab` loops `builtin_catalog().iter().take(18)`; `1113`: inline hook picker loops `picker.options.iter().take(12)`.

### Verdict `FA-SEC18-08` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent loss of pending edits on OK + immediate mutation past Cancel)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/manage_vars_modal.rs:240-260`: `Cancel` pushes `CloseManageVariables`, while `OK` commits only `!app.grid.variables_dialog.recode_mappings.is_empty()` (`push_apply_manage_var_recodes`) and closes, silently discarding unsaved `state.title_draft`, `state.unit_draft`, and `state.rename_draft`.
  - `manage_vars_modal.rs:403-408` (`table.setTitles`), `428-452` (`table.setVariableRole`), `757-764` (`table.recodeLevels`), and `784-797` (`push_apply_manage_var_order`) dispatch live mutations immediately before `OK`, so `Cancel` / `"×"` (`155-157`) cannot revert them.

### Verdict `FA-SEC18-09` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (wrong Stage 2 controls / contract drift across test families)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/power_studio.rs:453-551`: `render_stage2_effect` matches solely on `pw.family`, ignoring `pw.effect_mode` (`Standardized` vs pilot modes handled in `crates/ui-egui/src/views/power_wizard.rs:198-226`) and `app.view_shards.power_wizard.survival_log_rank` (`views/power_wizard.rs:172-177`). T-tests always show `mean_diff`/`sd` even when `Standardized` is selected; `SurvivalLogRank` (`TwoProportions` family) renders `p1`/`p2` steppers; `OneWayAnova`, `SimpleLinearRegression`, `Correlation`, and `ChiSquare` always render `standardized_effect`.
  - `power_studio.rs:78-80`: early return when `!is_power_studio_dialog_active(app)` leaves `PowerStudioEguiState` (`results_tab`, `hover_n`, `narrative_copied`) un-reset across sessions.

### Verdict `FA-SEC18-10` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (invalid minor-tick input ignored + error state silently cleared on axis tab switch)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/graph_modals/format_axes/segments_ticks.rs:424-445`: `axes.seg.0.minor_ticks_per_interval` only acts on valid `u16` parses, never sets `state.axis_ticks_error` on unparseable input, and snaps `buf` back on `lost_focus`.
  - `crates/ui-egui/src/egui_shell/graph_modals/mod.rs:212-225`: clicking `X Axis`/`Left Y`/`Right Y2` updates `state.format_axes_tab` immediately while deferring `axes.target_axis` via `PatchActiveDialog`, causing 1 frame of old `fa.axis_draft` rendered under the new tab (`format_axes/mod.rs:29-33`). On the next frame, `fa.target_axis != state.format_axes_last_target` triggers `sync_format_axes_local_drafts` (`format_axes/mod.rs:57-67`), unconditionally clearing `state.axis_bounds_error = None` and `state.axis_ticks_error = None`, allowing `commit_graph_modal_action` (`graph_modals/mod.rs:424-448`) to proceed.

### Verdict `FA-SEC18-11` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (single-click `MAY_DISCARD_DATA` restore without confirmation + premature draft erasure)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/app_modals.rs:1903-1919`: clicking `checkpoints.restore.{idx}` immediately enqueues `intents.push_engine("project.restoreCheckpoint", json!({ "id": cp.id }))` with no confirmation prompt; `crates/engine/src/cmd/project.rs:813, 978-989` marks `project.restoreCheckpoint` with `Effects::MUTATES_DOCUMENT.with(Effects::MAY_DISCARD_DATA)` and replaces `ctx.session.doc`.
  - `app_modals.rs:1855-1871`: clicking `checkpoints.create` queues `project.createCheckpoint` and immediately dispatches `PatchActiveDialog` resetting `name` and clearing `note` (`""`) before creation succeeds.

### Verdict `FA-SEC18-12` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (cross-session draft leaks, unvalidated formulas, dead hook button, column caps)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/transform_modals.rs:341-368` (`sync_standalone_transform_state`) omits `standalone_x_preset`, `standalone_y_preset`, `standalone_error_propagation`, `pharmacology_mode`, `baseline_cols`, `baseline_op`, `constant_k_draft`, `selected_k_op`, and `hook_picker_open`; `370-431` (`sync_transform_edit_state`) omits `constant_k_draft` (for non-`ConstantKOp` kinds), `hook_picker_open`, `baseline_cols`, `baseline_op`, `derived_title_draft`, `formula_draft`, `filter_rows`, and `extract_binding_*`.
  - `transform_modals.rs:1039-1056`: `edit.transform.add_derived` stores `DerivedColumnFormula` and clears drafts with no syntax check.
  - `transform_modals.rs:1124-1126, 1516-1540`: `"Hook Constant"` inside `MvFilterAndTransform` Select Rows opens `render_inline_hook_picker`, which writes to unused `state.constant_k_draft`.
  - `transform_modals.rs:887` caps `RemoveBaseline` columns at `.min(16)` and `1186` caps `MvFilterAndTransform` columns at `.min(20)`.

### Verdict `FA-SEC18-13` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (internal result-sheet import picker broken)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/import_wizard.rs:335-358`: `ImportOriginKind::InternalDataSheet | ImportOriginKind::InternalResultSheet` iterates solely over `&app.document().data_sheets` and never inspects `result_sheets`.
  - `import_wizard.rs:1190`: `compute_wizard_transformed_grid` returns `Vec::new()` unconditionally for `ImportOriginKind::InternalResultSheet`.

### Verdict `FA-SEC18-14` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity adjusted:** `S2` -> `S3`
- **Rationale for adjustment:** Per `docs/audit/README.md`, `S3` covers robustness problems with an available workaround (`S2` covers DoS or silent/misleading failures). Unlike `R15-GUI-33` (which left About uncloseable), every affected modal renders a working `Cancel`/`×` button reachable by pointer or `Tab` (`app_modals.rs:320, 456, 981, 1101, 1584`; `import_wizard.rs:1098`; `manage_vars_modal.rs:154, 242`; `transform_modals.rs:1559, 1920`; `graph_modals/mod.rs:264, 284`; `stats_modals/helpers.rs:95`; `result_viewer.rs:100`). Missing `Escape`/backdrop dismissal across all 28 dialogs/sub-dialogs listed is nonetheless a clear UX/accessibility robustness defect (`S3`).
- **Evidence:**
  - Explicit `Key::Escape` handling exists only in `fit_modal.rs:312, 1871`, `power_studio.rs:252`, `stat_anova_modal.rs:431`, `stats_modals/analyze_picker.rs:188, 524, 653`, `stats_modals/stat_params.rs:427`, and `app_modals.rs:1656, 1791, 1979`.
  - Repo-wide grep confirms zero `Key::Escape` or `modal_resp.should_close()` handling in `UnsavedChanges`, `Preferences`, `FindReplace`, `ExportFigure`, `Print` (`app_modals.rs`), `ImportWizard`/`ImportReshape`/`ConvertArchetype` (`import_wizard.rs`), `ManageVariables` (`manage_vars_modal.rs`), `TransformModals` & `ExcludeWithReason` (`transform_modals.rs`), all 13 `graph_modals` + `AdditionalTicks`, all 3 `r9_modals` (`RandomizeGroups`, `RandomSampleRows`, `SimulateDistributions`), and `ResultViewer` (`stats_modals/helpers.rs:167-172` and `inspect_bridge.rs:117-179` ignore `should_close()`, and `apps/curvebook/src/gui_window.rs:1036-1088` sets `dialog_inactive = false` whenever a dialog is open).

### Verdict `FA-SEC18-15` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (radio re-click inverts state; workaround: click `"Apply Geisser-Greenhouse"` checkbox)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/stat_anova_modal.rs:946-949`: `"Assume sphericity (unadjusted df)"` radio emits `intents.push_custom_view_action("stat_params", "toggle_geisser_greenhouse")` unconditionally on click even when already selected (`!gg_checked && !reml_active`).
  - `crates/ui-egui/src/views/stat_params.rs:390-404`: `handle_toggle_gg` unconditionally flips `geisser_greenhouse` to `!cur`.

### Verdict `FA-SEC18-16` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (transient unbacked option resets on reopen)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/graph_modals/format_graph/mv.rs:467-474`: `"Omit Outliers"` (`format_graph.mv.oor_omit`) sets transient `state.mv_out_of_range = MvOutOfRangePolicy::Omit` and clears `domain_clamp` to `None`.
  - `crates/doc/src/graph.rs:865-940`: `MultivariateGraphEncoding` / `ColorChannelMapping::Continuous` has no out-of-range omission mode; `graph_modals/mod.rs:355, 359` unconditionally resets `state.mv_out_of_range = MvOutOfRangePolicy::ClampToBounds` when syncing.

### Verdict `FA-SEC18-17` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (stale widget cache / modal flag leaks on series/sheet switch or `close_all`)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/graph_modals/format_graph/series.rs:391, 672`: `bar_border_width_spinner` and `series_offset_x_spinner` read/write `ScrubUnitFieldState` (`unit`, `editing`, `draft_text`) under static `egui::Id::unique(...)` keys without incorporating `graph_sheet_id` or `selected_series_index`.
  - `crates/ui-egui/src/egui_shell/app_modals.rs:193-198`: `AppModalsEguiState::close_all` clears `preferences_open`, `unsaved_changes_open`, and `find_replace_open`, leaving `export_emf_selected`, `find_replace.match_count`, and `about_tab` intact.

### Verdict `FA-SEC18-18` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (incomplete i18n coverage across modal labels)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/app_modals.rs:2402-2440`: `atr()` translates `"prefs.title"` for 8 of 14 `BETA_ACTIVE_LOCALES` (missing `it`, `nl`, `pl`, `ru`, `tr`), `"prefs.cat.*"` only for `de` and `ja`, and falls back to hardcoded English for all other keys (`"prefs.general.tree_heading"`, `"prefs.appearance.*"`, `"prefs.language.*"`, `"prefs.shortcuts.filter"`, `"export.title.layout"`, `"export.commit"`, `"print.commit"`), alongside inline English literals in `app_modals.rs`, `fit_modal.rs`, `import_wizard.rs`, `manage_vars_modal.rs`, `power_studio.rs`, `transform_modals.rs`, `graph_modals/`, and `stats_modals/`.

### Verdict `FA-SEC18-19` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (workaround: click `"Unpaired Student t"` first; `OneSample*` omitted)
- **Evidence:**
  - `crates/ui-egui/src/egui_shell/stats_modals/two_group.rs:41, 47`: passes `welch_en` (`r4_two_group_welch_correction(t.method)`, `crates/ui-egui/src/modal_predicates.rs:292-299`) to the `"edit.tg.welch_t"` radio button, disabling it whenever `t.method` is not `UnpairedT | WelchT | LognormalUnpairedT`.
  - `two_group.rs:45-55` vs `crates/doc/src/analysis.rs:355-384`: omits `TwoGroupMethod::OneSampleT` and `TwoGroupMethod::OneSampleWilcoxon` and any hypothetical-value control.

### Verdict `FA-SEC18-20` · `CONFIRMED`

- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (dead bindings / unused fields cleanup)
- **Evidence:**
  - Verified exact unused-suppression sites: `fit_modal.rs:1613` (`let _ = (&spec.p_value_format, PValueFormat::FourDecimal, column_titles_for_draft);`), `app_modals.rs:1648` (`let _ = state.about_tab;`), `stats_modals/r9_modals.rs:32` (`let _ = &state.active_tab;`), `stats_modals/analyze_picker.rs:444, 542` (`let _ = &state.active_tab;`), and `graph_modals/format_graph/mv.rs:482` (`let _ = fg;`).
