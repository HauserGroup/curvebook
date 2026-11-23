# SEC-03 Verification Verdicts — Transforms & Calculated Variables (`crates/transform`)

- **Base Checked:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-backend-2` (independent `100%` line-by-line source inspection + `crates/engine` & `crates/doc` cross-checks + isolated `.lc-worktrees/ver-backend-2` dynamic probe `ver_sec03_probe`, cleaned up after execution)
- **Source Report:** [SEC-03.md](../../../docs/audit/full-2026-11-22/SEC-03.md) (`FA-SEC03-01`..`FA-SEC03-16`)

## Summary Table

| Verdict | Count | Settled Severity Breakdown |
|---|---:|---|
| `CONFIRMED` | 16 | `S1: 0`, `S2: 3`, `S3: 9`, `S4: 4` |
| `PLAUSIBLE` | 0 | `—` |
| `REFUTED` | 0 | `—` |
| `DUPLICATE` | 0 | `—` |
| **Total** | **16** | **`S1=0, S2=3, S3=9, S4=4`** |

Dynamic verification run against an isolated detached worktree at `7c85a8e516165ddadb101236a88eec937bf1f095` (removed cleanly afterward):
- Baseline suite (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-2 cargo test --manifest-path .lc-worktrees/ver-backend-2/Cargo.toml --locked --offline -p curvebook-transform`): `133 passed; 0 failed; 0 ignored` (`15` unit + `117` integration across `14` binaries + `1` doc-test).
- Clippy check (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-2 cargo clippy --manifest-path .lc-worktrees/ver-backend-2/Cargo.toml --locked --offline -p curvebook-transform --all-targets`): exited `0` with zero warnings.
- Targeted reproducer suite (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-2 cargo test --manifest-path .lc-worktrees/ver-backend-2/Cargo.toml --locked --offline -p curvebook-transform --test ver_sec03_probe -- --nocapture`): `running 16 tests` -> `test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.40s`.

Cross-reference and calibration notes:
- `FA-SEC03-03` is adjusted `S2 -> S3` for consistency with settled `FA-SEC02-08` (`SEC-02-verdicts.md`, `S3`) after verifying that `crates/engine/src/dag.rs#L1028-L1036` rejects `ResultStatus::Dirty` upstream sheets before calling `execute_analysis_transform` (limiting silent stale-`Dirty` scalar resolution to direct `curvebook-transform` callers), while multi-`InfoSheet` explicit hooks (`MissingHook`) and `Info(...)` in `exclude_row_predicate` (`UnknownVariable`) fail explicitly rather than producing silent wrong values.
- `FA-SEC03-13` is adjusted `S4 -> S3` because `countif([Col], "true")` produces silently inverted wrong counts on ordinary inputs (`0` on text `"true"`, `N` on finite numeric columns) and `eval_scalar_const_arg` silently evaluates row `0` of arbitrary column expressions passed to `lag`/`lead`/`diff`/`percentile`/`moving_avg`.
- Precision nuances identified during verification (without refuting the underlying defects) are recorded on `FA-SEC03-02` (`0`-row transpose succeeds as a degenerate `0`-column table rather than failing `validate_title_vec`), `FA-SEC03-03` (`UnknownVariable` vs `Missing` on `exclude_row_predicate`), `FA-SEC03-05` (Voronoi convexity avoids `[0, 1, 0]` duplicate rows; real failures are out-of-order row scrambling and fallback), `FA-SEC03-07` (`simulate_xy_sheet` returns `InvalidSpec` in `evaluate_equation`), `FA-SEC03-09` (`build_replicate_slot_map` shares `.parse::<f64>()` comma-decimal sorting defect), and `FA-SEC03-15` (`Exponential` `+Inf` draws silently turn into `CellValue::Empty` via `Cell::number` rather than failing `DocError`, and `crates/engine` backfills non-transposed CI imports).

---

## File Coverage Completeness Check

Verified `100%` coverage against `SEC-03` scope at `7c85a8e516165ddadb101236a88eec937bf1f095` (`17` files, `16,210` total lines: `crates/transform/Cargo.toml` + `16` source files in `crates/transform/src/`, zero files omitted):

- [Cargo.toml](../../../crates/transform/Cargo.toml) (`20`)
- [lib.rs](../../../crates/transform/src/lib.rs) (`83`), [error.rs](../../../crates/transform/src/error.rs) (`129`), [curve.rs](../../../crates/transform/src/curve.rs) (`817`), [simulate.rs](../../../crates/transform/src/simulate.rs) (`377`), [randomize.rs](../../../crates/transform/src/randomize.rs) (`1,557`), [import.rs](../../../crates/transform/src/import.rs) (`969`), [plate_and_tidy.rs](../../../crates/transform/src/plate_and_tidy.rs) (`2,367`), [archetype_convert.rs](../../../crates/transform/src/archetype_convert.rs) (`2,962`)
- `crates/transform/src/formula/`: [mod.rs](../../../crates/transform/src/formula/mod.rs) (`16`), [dag.rs](../../../crates/transform/src/formula/dag.rs) (`316`), [parser.rs](../../../crates/transform/src/formula/parser.rs) (`1,173`), [eval.rs](../../../crates/transform/src/formula/eval.rs) (`2,134`)
- `crates/transform/src/ops/`: [mod.rs](../../../crates/transform/src/ops/mod.rs) (`288`), [math.rs](../../../crates/transform/src/ops/math.rs) (`744`), [normalize_baseline.rs](../../../crates/transform/src/ops/normalize_baseline.rs) (`704`), [reshape_select.rs](../../../crates/transform/src/ops/reshape_select.rs) (`1,554`)

Every file and line count matches `SEC-03.md`'s File Coverage Table with `1..=N` inspection and zero omitted files.

---

## Per-Finding Verdicts

### Verdict `FA-SEC03-01` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [execute_select_with_derived](../../../crates/transform/src/ops/reshape_select.rs#L981-L993) computes `col_shift = usize::from(include_axis && !matches!(target_archetype, TableKind::Xy | TableKind::Survival))` (`#L987`) and passes it to [remap_projected_column_notes](../../../crates/transform/src/ops/reshape_select.rs#L1030-L1038) before calling [project_into_xy](../../../crates/transform/src/ops/reshape_select.rs#L1365-L1386) / [project_into_survival](../../../crates/transform/src/ops/reshape_select.rs#L1396-L1416), which consume `selected[0]` as `X`/`times` and shift `selected[1..]` to `col = 0..`. `out.prune_stale_exclusion_notes()` (`#L993`) then either deletes shifted notes or misattributes col `0`'s note onto col `1`. Dynamic `probe_01_select_column_to_xy_and_survival_note_misalignment`: on a 3-column `ColumnTable` with excluded notes `"col0-x-note"` at `(0, 0, 0)` and `"col1-y1-note"` at `(0, 1, 0)`, both `TableKind::Xy` and `TableKind::Survival` kept `1` note at `(0, 0, 0)` (`"col0-x-note"` attached to `Y1`) and deleted `"col1-y1-note"`; on a 2-column `ColumnTable` with `"y1-only-note"` at `(0, 1, 0)`, both projections dropped all exclusion notes (`exclusion_notes.is_empty()`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent misattribution or deletion of `CellExclusionNote` provenance on `Column`/`Grouped`/`MultipleVariables -> Xy`/`Survival` projections).
- **Still present on main:** yes

### Verdict `FA-SEC03-02` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [execute_transpose](../../../crates/transform/src/ops/reshape_select.rs#L61-L75) constructs `DataSheet::new(id, title, archetype)` without remapping `source.exclusion_notes`; [transpose_xy](../../../crates/transform/src/ops/reshape_select.rs#L259-L339) calls `t.x_value_at(r)` (`#L265`, returning `None` on text `DateOrElapsedTime` cells and falling back to `default_column_label(r)`), discards numeric X silently (`warnings.is_empty()`) when `swap_titles == false`, and [transpose_subcolumn_datasets](../../../crates/transform/src/ops/reshape_select.rs#L149-L197) allocates full `sub_grids` and `new_cols` `DataSetColumn`s before callers invoke `DataSheet::new` (`validate_data_sets`). Dynamic `probe_02_transpose_notes_empty_rows_and_xy_x_loss`: verified `exclusion_notes` wiped (`excluded == true` retained with `exclusion_notes == []`), `DateOrElapsedTime` labels `["00:15", "00:30"]` replaced by `["A", "B"]`, and `swap_titles == false` dropping numeric X with `warnings == []`. *Precision note (Defect #1):* Contrary to the literal failure description that `DataSheet::new` fails `validate_title_vec` with `DocError` on `0`-row tables, `row_count()` on [ColumnTable](../../../crates/doc/src/archetypes.rs#L618-L620), [GroupedTable](../../../crates/doc/src/archetypes.rs#L819-L821), [ContingencyTable](../../../crates/doc/src/archetypes.rs#L952-L954), [PartsOfWholeTable](../../../crates/doc/src/archetypes.rs#L1235-L1237), and [MultipleVariablesTable](../../../crates/doc/src/archetypes.rs#L1640-L1642) falls back to `row_titles.len()` / `slice_labels.len()` when `data_sets`/`variables` is empty; `probe_02` observed `execute_transpose` *succeeding* across all 6 `0`-row archetypes (`Column`, `Contingency`, `PartsOfWhole`, `Grouped`, `MultipleVariables`, `Xy(RawReplicates(2))`), emitting degenerate `0`-column tables (`column_count() == 0`) with phantom non-zero `row_count() >= 2` and stale `row_titles`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent deletion of all `CellExclusionNote`s, silent loss of X/elapsed-time labels, pre-validation memory allocation up to `MAX_ROWS`, and degenerate `0`-column `0`-row transpose output).
- **Still present on main:** yes

### Verdict `FA-SEC03-03` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [TransformContext](../../../crates/transform/src/ops/mod.rs#L22-L79) holds only a single `info_sheet: Option<&'a InfoSheet>` (`#L25`), rejects mismatched `HookSource::InfoConstant { info_sheet_id: Some(expected_id), .. }` with `MissingHook` (`#L52-L58`), and checks only `ResultStatus::Failed` while ignoring `ResultStatus::Dirty` on `HookSource::ResultScalar` (`#L73-L78`); in `crates/engine/src/dag.rs`, [resolve_transform_info_sheet](../../../crates/engine/src/dag.rs#L1008-L1017) likewise returns at most one `InfoSheet` (contrast [Fit HookContext](../../../crates/engine/src/dag.rs#L1059-L1065)). Furthermore, [ImportRowEnv::resolve_info_constant](../../../crates/transform/src/import.rs#L517-L519) returns `None` and [apply_row_exclusion_predicate](../../../crates/transform/src/import.rs#L535) passes `None` to `ScalarEnv::new` despite `apply_import_link_pipeline` receiving `info_sheet: Option<&mut InfoSheet>` (and [refresh_single_region](../../../crates/engine/src/dag.rs#L872-L880) passes `None`). Dynamic `probe_03_transform_context_dirty_and_import_info_predicate`: resolved `Ok(42.0)` from a `ResultStatus::Dirty` sheet, rejected a second explicit `InfoSheet` hook with `TransformError::MissingHook`, and returned `Err(TransformError::UnknownVariable { name: "Info(\"Cutoff\")" })` on `[A] > Info("Cutoff")` with `Some(&mut info)`. *Precision notes:* (1) Inside `crates/engine`, [recompute_result_sheet](../../../crates/engine/src/dag.rs#L1028-L1036) runs `check_upstream_result_status` (`#L936-L942`) first, rejecting `ResultStatus::Dirty` before invoking `execute_analysis_transform` (parallel to `FA-SEC02-08` in `crates/doc`); (2) `apply_row_exclusion_predicate` fails cleanly with `UnknownVariable` (`eval.rs#L601-L602`) rather than silently evaluating `Info("Cutoff")` to `Missing`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity adjusted:** `S2 -> S3` (engine DAG guards `ResultStatus::Dirty` ahead of `TransformContext::resolve_hook`, while multi-`InfoSheet` explicit hooks and `Info(...)` import predicates fail with explicit errors rather than silently corrupting values; direct L2 `ResultScalar` `Dirty` bypass aligns with `FA-SEC02-08` settled at `S3`).
- **Still present on main:** yes

### Verdict `FA-SEC03-04` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [transform_archetype_y](../../../crates/transform/src/ops/math.rs#L391-L393), [apply_linear_norm_column](../../../crates/transform/src/ops/normalize_baseline.rs#L279), [apply_baseline_to_column](../../../crates/transform/src/ops/normalize_baseline.rs#L517), and [average_bins_archetype](../../../crates/transform/src/ops/reshape_select.rs#L590-L595) mutate `TableArchetype::Contingency(t)` in place without calling [ContingencyTable::validate_counts](../../../crates/doc/src/archetypes.rs#L978-L985), and [ContingencyTable::validate](../../../crates/doc/src/archetypes.rs#L988-L996) checks only rectangular grid bounds. Dynamic `probe_04_contingency_post_op_invalid_counts`: `execute_math_op(Log10)`, `execute_normalize`, `execute_remove_baseline(Difference)`, and `execute_prune(AverageBins(2))` all returned `Ok` producing `Contingency` sheets whose `t.validate_counts()` subsequently failed with `DocError::InvalidCellValue`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (valid transform operations produce invalid `ContingencyTable` data sheets containing negative/fractional values that break downstream contingency analyses).
- **Still present on main:** yes

### Verdict `FA-SEC03-05` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [select_log_spaced_indices](../../../crates/transform/src/ops/reshape_select.rs#L451-L478) takes `log_min = first_x.ln()` / `log_max = last_x.ln()` from `xs.first()` and `xs.last()`, requires `log_max > log_min`, and [retain_rows](../../../crates/transform/src/ops/reshape_select.rs#L800-L819) emits rows strictly in `indices` order. Dynamic `probe_05_prune_log_x_spacing_descending_and_oscillating`: descending `X = [1000, 999, 998, 997, 1]` (`target_points = 3`) hit `log_max < log_min` and fell back to row-index pruning (`[1000.0, 999.0, 1.0]`); non-monotonic `X = [1, 100, 50, 2, 1000]` (`target_points = 4`, `n > target_points`) emitted reordered row indices `[0, 3, 1, 4]`. *Precision note:* Because `|ln(x) - target_ln|` over monotonic `target_ln` yields convex Voronoi intervals and `min_by` deterministically picks the first tied index, `[1, 10, 1, 100]` (`target_points = 3`) produces `[0, 1, 3]` rather than `[0, 1, 0]` duplicate row indices; the actual failures on non-monotonic X are row-order scrambling, clipped extrema outside `[first_x, last_x]`, and dropped points, plus silent row-index fallback whenever `first_x >= last_x`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (wrong X-spacing / row-order corruption on valid descending or unsorted `XyTable` series).
- **Still present on main:** yes

### Verdict `FA-SEC03-06` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [lowess_point](../../../crates/transform/src/curve.rs#L598-L621) returns raw `ys[i]` when `max_d == 0.0` (`#L602-L605`) without tie-averaging or robustness weighting (contrast [local_poly_eval](../../../crates/transform/src/curve.rs#L531-L533)), and [compute_auc_internal](../../../crates/transform/src/curve.rs#L248-L274) validates `opts.baseline.is_finite()` (`#L249`) while omitting `opts.min_peak_height` and `opts.min_area_fraction` finiteness/range validation (`#L265-L266`). Dynamic `probe_06_lowess_all_tied_and_auc_nonfinite_peak_filters`: `smooth_xy` LOWESS (`span = 0.5, robustness_iters = 2`) on tied `[(1, 10), (1, 10), (1, 100), (2, 20), (3, 30), (4, 40)]` returned raw `[10.0, 10.0, 100.0]` at `x = 1.0`, and `compute_auc` accepted `min_peak_height = NaN` (returning `peaks = []`), `min_area_fraction = NaN` (`peaks = []`), and `min_area_fraction = -5.0`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (unsmoothed outlier leakage on tied-X LOWESS neighborhoods and silent peak suppression on non-finite/out-of-range AUC options).
- **Still present on main:** yes

### Verdict `FA-SEC03-07` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [execute_normalize](../../../crates/transform/src/ops/normalize_baseline.rs#L70-L83) checks `(h - z).abs() == 0.0` without `.is_finite()`, so [apply_linear_norm_column](../../../crates/transform/src/ops/normalize_baseline.rs#L269-L273) sets `factor = scale / Inf = 0.0` and maps finite entries to `0.0` or `Empty` (`Inf * 0.0 = NaN`); [normalize_dataset_col](../../../crates/transform/src/ops/normalize_baseline.rs#L324-L345) blanks zero-mean/overflowed `%CV` to `CellValue::Empty` without emitting warnings (contrast `#L648-L650`); [simulate_xy_sheet](../../../crates/transform/src/simulate.rs#L125-L128) computes `x_min + frac * (x_max - x_min)` producing `NaN` at `row = 0` when `x_max - x_min` overflows. Dynamic `probe_07_normalize_overflow_span_cv_and_simulate_inf_span`: normalizing `[-1e308, 0.0, 1e308]` yielded `[Some(0.0), Some(0.0), Empty]`; normalizing `MeanCvN` `[10, 20, 3]` / `[20, 10, 3]` blanked `cv` to `Empty` with `warnings == []`; `simulate_xy_sheet` with `[-1e308, 1e308]` returned `Err(TransformError::InvalidSpec { message: "simulation x coordinate must be finite, got NaN" })` from [evaluate_equation](../../../crates/transform/src/simulate.rs#L16-L18) (`#L129-L135` propagates non-`EvalError`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (silent data collapse/blanking on wide finite normalization spans and missing `%CV` blanking warnings).
- **Still present on main:** yes

### Verdict `FA-SEC03-08` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** Inside `JsonParser::parse_string` ([plate_and_tidy.rs](../../../crates/transform/src/plate_and_tidy.rs#L1971-L1976)), every non-ASCII byte runs `std::str::from_utf8(&self.bytes[self.pos..])`, validating the entire remaining buffer to EOF on each non-ASCII code point despite `from_json_str` (`#L1463`) taking `&str` and [from_cbtemplate_json](../../../crates/transform/src/plate_and_tidy.rs#L226) allowing up to `MAX_CBTEMPLATE_JSON_BYTES = 4 MiB` (`#L23`). Dynamic `probe_08_json_parser_non_ascii_quadratic_scan`: parsing valid 96-well templates (`48` wells each holding `1_000` characters, within `MAX_TITLE_BYTES = 4096`) took `180.209µs` for ASCII (`51,528` bytes) vs `1.402373875s` for `'é'` (`99,528` bytes — `~7,782x` slowdown at `~2.4%` of the `4 MiB` cap).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (quadratic parser latency / UI stall on valid multi-byte UTF-8 `.cbtemplate` files).
- **Still present on main:** yes

### Verdict `FA-SEC03-09` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [pivot_tidy_to_grouped](../../../crates/transform/src/plate_and_tidy.rs#L1099-L1195) ignores `rec.x_excluded` (unlike [pivot_tidy_to_xy](../../../crates/transform/src/plate_and_tidy.rs#L1051-L1056)); [pivot_tidy_to_mv](../../../crates/transform/src/plate_and_tidy.rs#L1236-L1256) marks `mv.variables[xi]` excluded without adding a `CellExclusionNote` (`#L1253-L1256` covers `val_var` only) and parses `rec.replicate.parse::<f64>()` (`#L1247`) ignoring `decimal_sep == ','` (same `.parse::<f64>()` defect in [build_replicate_slot_map](../../../crates/transform/src/plate_and_tidy.rs#L1307-L1310)); slot collisions at `#L455-L458`, `#L1039-L1041`, and `#L1122-L1124` increment `slot += 1` without warnings. Dynamic `probe_09_tidy_grouped_drops_x_excluded_and_decimal_comma_reps`: with `decimal_sep = ','`, `"*1"` in `x_col`, and `["10,5", "2,5"]` in `replicate_col`, `Grouped` dropped `x_excluded` (`excluded = false`, `notes = []`) and ordered `"10,5"` into subcolumn `0` ahead of `"2,5"`, while `MultipleVariables` kept `x0.excluded = true` with `exclusion_notes = []` and stored `Replicate = Text("10,5")`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (lost exclusion propagation/notes, comma-decimal replicate misparsing/misordering, and silent collision shifting).
- **Still present on main:** yes

### Verdict `FA-SEC03-10` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [convert_xy_wide_to_mv](../../../crates/transform/src/archetype_convert.rs#L1855-L1870) and [convert_column_to_mv](../../../crates/transform/src/archetype_convert.rs#L2210-L2233) call `MultipleVariablesTable::new` (`MAX_SHEET_CELLS = 30M` in [error.rs](../../../crates/doc/src/error.rs#L32)) without `validate_cell_budget` (`MAX_IMPORT_CELLS = 10M` in [import.rs](../../../crates/transform/src/import.rs#L17)); [pivot_mv_summary_to_grouped](../../../crates/transform/src/archetype_convert.rs#L1467-L1484) clears `grouped.data_sets` on empty `col_levels` instead of returning `InvalidInput`; [convert_xy_reps_to_mv](../../../crates/transform/src/archetype_convert.rs#L1773) and [convert_grouped_reps_to_mv](../../../crates/transform/src/archetype_convert.rs#L1012) drop source `row_titles`. Dynamic `probe_10_archetype_convert_summary_zero_cols_and_row_titles`: pivoting a summary `MultipleVariablesTable` with empty factor cells emitted a `0`-column, `0`-row `GroupedTable`, and `Xy(RawReplicates(2)) -> MultipleVariables` dropped `xy.row_titles = ["SampleA"]`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (`10M` cell-budget bypass on wide flattening, `0`-column summary pivot inconsistency, and dropped `row_titles`).
- **Still present on main:** yes

### Verdict `FA-SEC03-11` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** In [unstack_grid_by_group](../../../crates/transform/src/import.rs#L411-L424), numeric group keys format with raw `format!("{v}")` without normalizing `-0.0` (`#L413`) and `row.iter().enumerate().find(|&(idx, _)| idx != group_col)` (`#L419`) discards columns `2..`; on `subs_per_col <= 1`, [validate_grid_col_bounds](../../../crates/transform/src/import.rs#L638-L647) adds `base_sub` while [resolve_target_coord](../../../crates/transform/src/import.rs#L649-L663) writes `(col, 0)` and [place_grid](../../../crates/transform/src/import.rs#L626) returns `anchor.subcol`; [parse_delimited_text](../../../crates/transform/src/import.rs#L305-L326) uses `MAX_IMPORT_CELLS + 65_536` / `MAX_ROWS + 16`, and [extract_stage1](../../../crates/transform/src/import.rs#L232-L272) extracts `InternalData` before checking `MAX_IMPORT_CELLS`. Dynamic `probe_11_import_unstack_neg_zero_multi_col_and_subcol_anchor`: unstacking `[-0.0, 0.0]` group keys created `2` columns and dropped column `2`, and placing `"42"` into `ColumnTable(Single)` at `anchor = CellCoord::new(0, 0, 2)` expanded the table to `3` columns, wrote to `(0, 0, 0)`, and returned `target_range = (0, 0, 2)..(0, 0, 2)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (`-0.0` split, silent multi-column unstack truncation, bogus `target_range`/column inflation on `anchor.subcol > 0`, and budget headroom/ordering gaps).
- **Still present on main:** yes

### Verdict `FA-SEC03-12` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [CompiledOp::compile](../../../crates/transform/src/ops/math.rs#L129-L134) checks `*base <= 0.0` without rejecting `base == 1.0` despite [MathTransformOp::Antilog](../../../crates/transform/src/ops/math.rs#L69) documenting `base != 1`; [apply_preset](../../../crates/transform/src/ops/math.rs#L213-L219) and [eval.rs](../../../crates/transform/src/formula/eval.rs#L2111-L2117) compute Finney's `5.0 + norm_ppf(v)`, while [MathTransformPreset::Probit](../../../crates/doc/src/analysis.rs#L112) and [MathTransformOp::Probit](../../../crates/transform/src/ops/math.rs#L74) document `Phi^{-1}(u)` without `+ 5` (contrast [norm_ppf](../../../crates/transform/src/formula/eval.rs#L2118-L2120)). Dynamic `probe_12_math_antilog_one_and_probit_offset_vs_doc`: `Antilog { base: 1.0 }` returned `Ok(1.0)`, `MathTransformOp::Probit` and `probit([A])` returned `5.0` at `0.5`, and `norm_ppf([A])` returned `0.0`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (minor validation and documentation contract mismatch).
- **Still present on main:** yes

### Verdict `FA-SEC03-13` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** In [try_eval_aggregate_call](../../../crates/transform/src/formula/eval.rs#L1338-L1360), `countif` excludes `Expr::Text("true" | "false")` (`#L1345-L1346`) from literal string matching and routes it to [resolve_cond_val_pair](../../../crates/transform/src/formula/eval.rs#L1128-L1134), where [FormulaValue::as_bool](../../../crates/transform/src/formula/eval.rs#L68-L82) evaluates `"true"` to `Some(true)` on every row and [AggOp::CountIf](../../../crates/transform/src/formula/eval.rs#L1189-L1200) requires `as_finite_f64().is_some()`. Meanwhile [eval_scalar_const_arg](../../../crates/transform/src/formula/eval.rs#L1451-L1464) evaluates column expressions on non-empty sheets and returns `.first()` (`row 0`). Dynamic `probe_13_formula_countif_true_literal_and_scalar_const_arg`: `countif([FlagText], "true")` on `["true", "false", "true"]` returned `0.0`, `countif([Num], "true")` on `[10, 20, 30]` returned `3.0` (`countif([Num], "yes")` returned `0.0`), and `lag([Num], [Step])` with `Step = [2, 1, 1]` globally used `Step[0] = 2` (`[Missing, Missing, 10.0]`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity adjusted:** `S4 -> S3` (`countif(col, "true")` silently returns completely wrong counts on both text and numeric columns, and `eval_scalar_const_arg` silently treats arbitrary column expressions as row-0 constants in window/quantile functions).
- **Still present on main:** yes

### Verdict `FA-SEC03-14` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** In [execute_transform](../../../crates/transform/src/ops/mod.rs#L205-L215), `TransformAnalysisSpec::RandomSample` records `"Seed"` accurately as `ResultCell::Text(seed.to_string())` (`#L205`) while still pushing `ResultScalarEntry { key: String::from("seed"), data_set_index: None, value: *seed as f64, units: None }` (`#L214`), losing precision above `2^53`. Dynamic `probe_14_random_sample_seed_scalar_f64_rounding`: `seed = 9_007_199_254_740_993_u64` produced scalar `"seed" = 9_007_199_254_740_992.0`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (legacy numeric scalar entry alongside fixed `ResultCell::Text` display rounds 64-bit seeds `> 2^53`).
- **Still present on main:** yes

### Verdict `FA-SEC03-15` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [sample_distribution_value](../../../crates/transform/src/randomize.rs#L818-L829) checks `exp_z.is_finite()` for `LogNormal` (`#L821-L823`) but omits finiteness checks on `Exponential { rate }` (`-rng.next_open_f64().ln() / rate`, `#L828`) after validating `rate > 0.0 && rate.is_finite()` (`#L795-L799`); [result_cell_to_cell](../../../crates/transform/src/import.rs#L285) maps `ResultCell::ConfidenceInterval { .. }` unconditionally to `Cell::empty()`. Dynamic `probe_15_exponential_tiny_rate_and_result_ci_import`: confirmed both defects. *Precision notes:* (1) Because `Cell::number` ([cell.rs](../../../crates/doc/src/cell.rs#L70)) calls `CellValue::from_finite_or_empty`, `Exponential { rate: 1e-320 }` does **not** fail inside `DataSheet::new` as `TransformError::Doc` — instead `simulate_distribution_columns` (`randomize.rs#L333, #L349`) succeeds (`Ok(DataSheet)`) and silently erases all `+Inf` draws (`10/10` in `probe_15`) to `CellValue::Empty`; (2) `crates/engine` callers follow `apply_import_link_pipeline` with [backfill_confidence_interval_cells](../../../crates/engine/src/dag.rs#L807-L845), so the blank CI mapping affects standalone L2 callers or transposed imports (`dag.rs#L814`) and Stage 2 predicates/baselines.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (subnormal-rate `1e-320` edge case silently blanks `+Inf` draws instead of raising `EvalError`; CI import blanking handled downstream in non-transposed `engine` workflows).
- **Still present on main:** yes

### Verdict `FA-SEC03-16` · `CONFIRMED`

- **Verifier:** `ver-backend-2`
- **Evidence:** [convert_xy_to_grouped](../../../crates/transform/src/archetype_convert.rs#L2459) and [convert_xy_to_contingency](../../../crates/transform/src/archetype_convert.rs#L2620) format numeric `X` into `row_titles` via `format!("{v}")` without `-0.0 -> 0.0` normalization; [convert_grouped_reps_to_mv](../../../crates/transform/src/archetype_convert.rs#L1040) evaluates `grp.subcolumn_mode.is_pre_summarized()` inside a branch dispatched exclusively for `SubcolumnMode::RawReplicates`; [pivot_mv_two_way_to_grouped](../../../crates/transform/src/archetype_convert.rs#L1287-L1290) and [pivot_mv_grouped_xy](../../../crates/transform/src/archetype_convert.rs#L2129-L2132) increment `mapped_*_cells += 1` on `rep_col` `is_occupied` even when `rep_order = None` (`excluded = true` or forced-continuous text). Dynamic `probe_16_archetype_convert_neg_zero_title_and_rep_count`: `Xy([-0.0])` produced `row_titles = ["-0"]` in both `Grouped` and `Contingency`, and pivoting a 2-row `MultipleVariablesTable` with `Cell::excluded_number(1.0)` in `Replicate` reported `preserved_cells = 8, dropped_cells = 0`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (`-0` string cosmetic, dead pre-summarized check, minor preview metric inflation on excluded/non-numeric replicates).
- **Still present on main:** yes
