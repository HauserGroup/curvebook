# SEC-03 Audit Report — Transforms & Calculated Variables

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Scope:** `crates/transform/Cargo.toml` (`1` manifest, `20` lines) + all `16` source files in `crates/transform/src/` (`16,190` LOC) = `17` files (`16,210` lines total)
- **Date:** `2026-11-22`
- **Reviewer:** `rev-core` (report-only; line `1..=N` coverage on all `17` files)
- **Summary Counts:** `S1 = 0`, `S2 = 4`, `S3 = 7`, `S4 = 5` (Total: `16` findings)
- **Prior Remediation Check (`docs/audit/local-code-audit-2026-11-18.md`):**
  - Closed findings affecting `crates/transform`: `#14` (`sumif`/`meanif` canonical `(value, condition)` arg order), `#54` (negative-slope/baseline swap + exclusion note remap), `#55` (blanked spread warning in `ops/math.rs`), `#56` (replicate slot preservation + summary schema-loss flag), `#59` (`(Missing)` row for blank numeric X in tidy-to-grouped), `#60` (X/Time column preservation in `Xy`/`Survival -> Column/Grouped/MV`), `#61` (`execute_prune` exclusion-note remap), `#62` (`curve.rs` central-only summary AUC, tied-X collapse, `total_area = positive_area + negative_area`), `#63` (`curve.rs` relative grid tolerance in `SavitzkyGolay`), `#125` (`FirstAndLastLinear` weighted row centroid), `#126` (randomization apportionment / `PermutedBlock` capacity / BTRS binomial), and Wave-4 `archetype_convert.rs:2684` zero-`Label`-column guard (`MultipleVariables -> Contingency`): **all verified fixed on `main` (`0` regressions)**, though residual gaps remain in adjacent unguarded paths (`FA-SEC03-01` reverse projection, `FA-SEC03-06` LOWESS ties, `FA-SEC03-14` `seed as f64` scalar, `FA-SEC03-12` `Probit` doc sync, `FA-SEC03-10` reps->MV `row_titles`).

## Baseline Verification Summary

- `cargo test --locked --offline -p curvebook-transform` (with `CARGO_TARGET_DIR=/tmp/cb-sec03-rev`): exited `0`. Observed `133` passed, `0` failed, `0` ignored across:
  - Unit tests (`src/lib.rs`): `15` passed, `0` failed.
  - Integration suites (`tests/`): `117` passed across `14` files (`audit_r1_s4a`: `7`, `audit_r1_s4b_transform`: `3`, `audit_r1_s4b_transform_s3`: `4`, `audit_r2_s04`: `15`, `audit_w2_transform_ops`: `10`, `d1_curve_transforms`: `6`, `d1_normalization`: `8`, `d1_randomization`: `9`, `prop_transform`: `5`, `qol_reshape_convert`: `11`, `r63_window_and_conditional`: `8`, `rv_ledger_formula`: `7`, `rv_ledger_ops`: `4`, `unit_transform`: `20`), `0` failed.
  - Doc-tests (`crates/transform/src/lib.rs`): `1` passed, `0` failed.
- `cargo clippy --locked --offline -p curvebook-transform --all-targets`: exited `0` with zero warnings.
- `cargo check --locked --offline -p curvebook-transform --target wasm32-unknown-unknown`: blocked locally (`E0463: can't find crate for std` — `wasm32-unknown-unknown` target sysroot not installed); source inspection confirms pure-Rust dependencies (`curvebook-math`, `curvebook-doc`, `serde`), `#![forbid(unsafe_code)]`, `[lints] workspace = true`, zero `chrono`, zero network/OS/clock APIs, zero production `.unwrap()`/`.expect()`/`panic!`, and checked allocation budgets.

---

## File Coverage Table

| # | File | Lines | Inspected | Concrete Invariants & Dimensions Verified | Findings |
|---|---|---:|---|---|---|
| 1 | `crates/transform/Cargo.toml` | 20 | `1..=20` | `[lints] workspace = true`; runtime deps strictly `curvebook-math`, `curvebook-doc`, `serde`; dev-deps `curvebook-testkit`, `proptest` (`wasm32`-clean, no `chrono`, no network/OS crates). | Clean — manifest & lints verified |
| 2 | `crates/transform/src/lib.rs` | 83 | `1..=83` | `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`; public module & symbol facade (`TransformKind`, formula/op/convert/plate/randomize/simulate re-exports) and doctest. | Clean — crate safety attributes & re-exports verified |
| 3 | `crates/transform/src/error.rs` | 129 | `1..=129` | Typed `TransformError` variants (`Doc`, `Math`, `SummaryPoint`, `CircularDependency`, `ParseError`, `EvalError`, `UnknownVariable`, `UnknownFunction`, `ArityMismatch`, `InvalidSpec`, `IncompatibleTable`, `MissingHook`, `UnknownEquation`, `InvalidInput`), `Display`, `Error`, and `From` impls. | Clean — error taxonomy & formatting verified |
| 4 | `crates/transform/src/formula/mod.rs` | 16 | `1..=16` | Submodule wiring and public re-exports for `dag`, `eval`, and `parser`. | Clean — module facade verified |
| 5 | `crates/transform/src/formula/dag.rs` | 316 | `1..=316` | `extract_dependencies` across `PARTITION BY`/`ORDER BY`/`FILTER`/`ROWS`, iterative `walk_expr_vars`, Kahn + min-heap topological sort (`topo_sort_columns`), DFS cycle reconstruction bounded `<= MAX_DATA_SETS (1024)`, preservation of prior `excluded` cell flags on rewritten cells. | Clean — iterative AST walk & cycle-free DAG verified |
| 6 | `crates/transform/src/formula/parser.rs` | 1,173 | `1..=1,173` | `MAX_PARSE_DEPTH = 128`, `MAX_TOKENS = 16_384`, iterative `Expr::drop` preventing stack overflow on deep AST drop, UTF-8 safe lexer spans, Pratt precedence (`not` looser than comparisons, tighter than `and`/`or`), `OVER (...)` / `FILTER` / `ROWS` validation, span-based `rewrite_formula_column_refs`. | Clean — bounded lexer/parser & iterative drop verified |
| 7 | `crates/transform/src/formula/eval.rs` | 2,134 | `1..=2,134` | Iterative `EvalStep` evaluation, Kleene 3-valued logic, `#14` canonical `(values, cond)` arg order, `OVER`/`PARTITION BY`/`ORDER BY`/`ROWS` frames, `countif(col, "true")` bypass & row-0 coercion in `eval_scalar_const_arg`. | `FA-SEC03-13` |
| 8 | `crates/transform/src/ops/mod.rs` | 288 | `1..=288` | Checked `active_columns` deduplication preserving order, dispatch across all 8 `TransformAnalysisSpec` variants, bound-swap exclusion-note helpers; single-`InfoSheet` `TransformContext`, stale `Dirty` `ResultScalar` acceptance, `seed as f64` precision loss. | `FA-SEC03-03`, `FA-SEC03-14` |
| 9 | `crates/transform/src/ops/math.rs` | 744 | `1..=744` | JCGM GUM (2008) & Zar (2010) `D1` citations, `FirstOrderDeltaMethod` vs `ReplicateEndpoints`, `#54` negative-slope bound swap + note remap, `#55` spread blanking warnings; unvalidated in-place `Contingency` output, `Antilog` `base == 1.0`, `Probit` `Doc` comment drift. | `FA-SEC03-04`, `FA-SEC03-12` |
| 10 | `crates/transform/src/ops/normalize_baseline.rs` | 704 | `1..=704` | `#125` `FirstAndLastLinear` weighted row centroid, `#54` negative-factor/baseline swap + note remap, `apply_baseline_mean_cv_row` warnings; unvalidated in-place `Contingency` mutation, `hundred - zero` overflow on wide finite anchors, silent `MeanCvN` blanking in `normalize_dataset_col`. | `FA-SEC03-04`, `FA-SEC03-07` |
| 11 | `crates/transform/src/ops/reshape_select.rs` | 1,554 | `1..=1,554` | `AverageBins` parametric/non-parametric/text pooling & `Survival` rejection, `SelectRule` bounds/ties, `retain_rows` `row_numeric_spacing`; `Non-XY -> Xy/Survival` `-1` exclusion-note shift omission, `execute_transpose` zero-row failure & dropped notes/X, unvalidated `AverageBins` `Contingency`, `select_log_spaced_indices` non-monotonic bugs. | `FA-SEC03-01`, `FA-SEC03-02`, `FA-SEC03-04`, `FA-SEC03-05` |
| 12 | `crates/transform/src/curve.rs` | 817 | `1..=817` | Yeh & Kwan (1978), Bailer (1988), Cleveland (1979), Gorry (1990) `D1` citations, `#62` central-only summary AUC / tied-X AUC collapse / `total_area`, `#63` relative `is_uniform_grid` tolerance; `lowess_point` unaveraged tied `max_d == 0.0`, unvalidated `AucOptions::min_peak_height`/`min_area_fraction`. | `FA-SEC03-06` |
| 13 | `crates/transform/src/simulate.rs` | 377 | `1..=377` | `MAX_IMPORT_CELLS` overflow-checked budget, `DeterministicRng`, Atkinson PTRS <= `1e15` & asymptotic normal transition > `1e15`, isolated pole recovery, seed `ResultCell::Text` preservation; `x_max - x_min` overflow on wide finite X bounds. | `FA-SEC03-07` |
| 14 | `crates/transform/src/randomize.rs` | 1,557 | `1..=1,557` | Durstenfeld (1964) & Hormann (1993) BTRS `D1` citations, exact `u128` Hamilton apportionment, constrained iterative stratum allocation proof, `#126` `PermutedBlock` overflow/`try_reserve_exact`/trial cap, `remap_exclusion_notes`; `Exponential` missing overflow guard (`+Inf`). | `FA-SEC03-15` |
| 15 | `crates/transform/src/import.rs` | 969 | `1..=969` | Cycle detection (`detect_link_cycles`), RFC-4180 dialect scanning, `#77` `InsertRows` exclusion-note shift, overflow-checked `validate_cell_budget` & `validate_archetype_growth_budget`; `ImportRowEnv` dropping `InfoSheet`, `unstack_grid_by_group` multi-column/`-0.0`, `anchor.subcol` mismatch, `ConfidenceInterval` drop, budget headroom. | `FA-SEC03-03`, `FA-SEC03-11`, `FA-SEC03-15` |
| 16 | `crates/transform/src/plate_and_tidy.rs` | 2,367 | `1..=2,367` | Bounded 96/384 `.cbtemplate` validation (`MAX_CBTEMPLATE_JSON_BYTES`, `MAX_PLATE_WELLS`, `MAX_JSON_DEPTH = 64`), `#59` numeric `(Missing)` row, blank subtraction, `*` / `excluded_col` handling; `O(N^2)` UTF-8 rescan in `json_codec::parse_string`, tidy `x_excluded`/`decimal_sep` gaps, silent replicate slot bump. | `FA-SEC03-08`, `FA-SEC03-09` |
| 17 | `crates/transform/src/archetype_convert.rs` | 2,962 | `1..=2,962` | Lossy preview/guard, `ReplicateSlotMapper` subcolumn slot preservation (`#56`), summary `schema_lossy` flags, Wave-4 zero-`Label`-column guard (`2684`); missing `validate_cell_budget` on wide `XY`/`Column -> MV`, `pivot_mv_summary_to_grouped` empty `data_sets`, lost replicate-MV `row_titles`, minor `-0.0`/dead/count items. | `FA-SEC03-10`, `FA-SEC03-16` |

---

## Findings (`S1` -> `S4`)

*(No `S1` Critical findings identified in `SEC-03`.)*

### `FA-SEC03-01` · `S2` · `execute_select_with_derived` omits target-axis column shift when projecting non-axis archetypes to `Xy`/`Survival`, misassigning or losing `CellExclusionNote`s

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/ops/reshape_select.rs:981-993, 1030-1038, 1346-1433`
- **Category:** `wrong-result` / `data-loss`
- **Defect:** When `execute_select_with_derived` projects `working` into a different `target_archetype`, line `987` computes `col_shift = usize::from(include_axis && !matches!(target_archetype, TableKind::Xy | TableKind::Survival))`, which handles only the `+1` rightward column shift needed when an `Xy`/`Survival` source prepends its external axis column (`X`/`Time`) into a non-axis target (`#60`). When projecting any non-axis source (`Column`, `Grouped`, `Contingency`, `PartsOfWhole`, `MultipleVariables`, `Nested`) with `output_cols.len() >= 2` into `TableKind::Xy` or `TableKind::Survival`, `has_external_axis` is `false` (`include_axis == false`), so `col_shift` is `0`. However, `project_into_xy` (`1365-1386`) and `project_into_survival` (`1396-1416`) consume source column `selected[0]` as `X`/`Time` and move source columns `selected[1..]` leftward to `data_sets[0..]` / `cohorts[0..]`. Because `remap_projected_column_notes` (`1030-1038`) only supports non-negative rightward shifts and receives `col_shift = 0`, all exclusion notes remain at unshifted indices.
- **Failure scenario:** Project a 3-column `ColumnTable` (`[Concentration, Drug A, Drug B]`) into `TableKind::Xy` via `execute_select` when `Drug A` (`col = 1`) and `Drug B` (`col = 2`) have excluded cells carrying `CellExclusionNote`s at `(r, 1, 0)` and `(r, 2, 0)`. `project_into_xy` places `Concentration` in `t.x_cells`, `Drug A` at `col = 0`, and `Drug B` at `col = 1`. `remap_projected_column_notes` maps `(r, 1, 0)` -> `(r, 1, 0)` and `(r, 2, 0)` -> `(r, 2, 0)`. Next, `out.prune_stale_exclusion_notes()` (`993`) deletes `Drug B`'s note (`col = 2` is out of bounds) and either deletes `Drug A`'s note or, if `Drug B` at row `r` is also `excluded = true`, **silently attaches `Drug A`'s exclusion note to `Drug B`**. Any note originally on `col = 0` (`Concentration`) lands on `Drug A` (`col = 0`). The identical off-by-1 shift occurs for `Column`/`Grouped`/`MultipleVariables -> Survival`.
- **Evidence:**
  ```rust
  // crates/transform/src/ops/reshape_select.rs:981-993, 1365-1386
  let has_external_axis = matches!(&working.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_));
  let drops_axis_target = matches!(target_archetype, TableKind::Contingency | TableKind::PartsOfWhole);
  let include_axis = has_external_axis && !drops_axis_target;
  ...
  let col_shift = usize::from(include_axis && !matches!(target_archetype, TableKind::Xy | TableKind::Survival));
  let remapped_notes = remap_projected_column_notes(&working.exclusion_notes, &output_cols, col_shift);
  (project_archetype(&working.archetype, target_archetype, &output_cols, include_axis)?, remapped_notes)
  ...
  // project_into_xy when cols >= 2:
  for (y_idx, ds) in t.data_sets.iter_mut().enumerate() {
      ...
      for (r, row_vals) in grid.iter().enumerate().take(rows) {
          if let Some(cell) = row_vals.get(y_idx + 1) ...
  ```
- **Suggested fix:** Replace `col_shift: usize` with an explicit axis-mapping mode (`None`, `PrependExternalAxis (+1)`, `ConsumeFirstColumnAsAxis (-1)` when `!has_external_axis && output_cols.len() >= 2 && matches!(target_archetype, TableKind::Xy | TableKind::Survival)`). In the `-1` mode, exclude notes targeting `output_cols[0]` (emitting a warning because `Xy.x_cells`/`Survival.times` have no `data_cell` slot for `CellExclusionNote`) and decrement mapped column indices `c >= 1` to `c - 1`.
- **Related:** `crates/transform/tests/audit_w2_transform_ops.rs:230-262` (`audit_60_select_and_transform_carries_xy_x_and_survival_times` covers `Xy -> Column` only), `crates/doc/src/archetypes.rs:1902-1913`, `crates/doc/src/links.rs:385-387`.

---

### `FA-SEC03-02` · `S2` · `execute_transpose` fails validation on zero-row tables, discards `CellExclusionNote`s and `XyTable` X values, and allocates before checking `MAX_DATA_SETS`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/ops/reshape_select.rs:61-75, 84-133, 149-197, 243-344` (cross-reference `crates/doc/src/archetypes.rs:704, 875, 994, 1272`)
- **Category:** `wrong-result` / `data-loss` / `contract-drift`
- **Defect:** `execute_transpose` has four structural defects:
  1. When transposing a valid `0`-row table with `cols >= 1` (`Column`, `Contingency`, `PartsOfWhole`, `Grouped`, or `Xy` including `RawReplicates(n)`), `transpose_subcolumn_datasets` produces `new_rows = cols >= 1`, `new_cols = 0`, `tc.data_sets = []`. Each caller executes `out.data_sets.clear()` (`out.row_count() == 0`) while assigning `out.row_titles = tc.row_titles` (`len == cols >= 1`, or `cols * n` at `330`, or `slice_labels` at `131`), causing `DataSheet::new` (`64`) -> `validate()` -> `validate_title_vec` to fail with `DocError::InvalidCellValue` ("row title count does not match table row count").
  2. `execute_transpose` (`61-75`) builds a fresh `DataSheet::new(...)` without remapping `source.exclusion_notes` across `(row, col, subcol) -> (col, row, subcol)` (or `(col * n + s, row, 0)` for `Xy::RawReplicates(n)`), silently deleting every cell exclusion note while leaving `Cell::excluded = true` on the transposed cells.
  3. In `transpose_xy` (`260-270`), `row_labels` uses `t.x_value_at(r)` instead of `t.materialized_x_cell_at(r)`, losing `DateOrElapsedTime` text labels and falling back to `"A"`, `"B"`, ..., and when `swap_titles == false`, dropping all X values without emitting a warning.
  4. `transpose_subcolumn_datasets` (`158-194`) allocates full `sub_grids` and constructs `new_cols = rows` `DataSetColumn`s (up to `MAX_ROWS`) before callers construct `ColumnTable`/`GroupedTable` and trigger `validate_data_sets(MAX_DATA_SETS)`.
- **Failure scenario:** Calling `execute_transpose` on an empty 3-column `ColumnTable` (`rows == 0`) fails with a `DocError` row-title count mismatch instead of succeeding cleanly; transposing any sheet carrying excluded cells with rationale notes permanently erases all `exclusion_notes`; transposing a 1-column x 500,000-row table allocates 500,000 `DataSetColumn` buffers before failing `TooManyDataSets`.
- **Evidence:**
  ```rust
  // crates/transform/src/ops/reshape_select.rs:84-98, 265-270, 313-341
  let tc = transpose_single_columns(&t.data_sets, &t.row_titles, swap_titles)?;
  let mut out = ColumnTable::new(t.design_hint, tc.data_sets.len().max(1), tc.data_sets.first().map_or(0, DataSetColumn::row_count))?;
  if tc.data_sets.is_empty() {
      out.data_sets.clear();
  } ...
  out.row_titles = tc.row_titles; // len == cols >= 1 while out.row_count() == 0
  ...
  } else if let Some(x) = t.x_value_at(r) {
      format!("{x}")
  } else {
      default_column_label(r)
  }
  ...
  if rows == 0 {
      out.data_sets.clear();
  } ...
  out.row_titles = if swap_titles { ... } else { vec![String::new(); new_rows] };
  ```
- **Suggested fix:** Check `validate_data_sets(rows)` (and `validate_rows(cols)` / `validate_cell_budget`) at the top of `transpose_subcolumn_datasets`; clear `row_titles`/`slice_labels` when `tc.data_sets.is_empty()` (`out.row_count() == 0`) or return an explicit `InvalidInput` error on 0-row tables; remap `source.exclusion_notes` onto the transposed coordinates (with warning if `swap_titles`/`RawReplicates` alters layout); use `t.materialized_x_cell_at(r)` for X labels and warn when `Xy` transposition discards numeric X data.
- **Related:** `crates/doc/src/archetypes.rs:704, 875, 994, 1272`, `docs/audit/local-code-audit-2026-11-18.md:609`.

---

### `FA-SEC03-03` · `S2` · `TransformContext` accepts only one `InfoSheet`, breaking multi-`InfoSheet` explicit hooks and permitting stale `Dirty` `ResultScalar` hooks

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/ops/mod.rs:22-79`; `crates/transform/src/import.rs:517, 535` (cross-reference `crates/engine/src/dag.rs:1008-1050`)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:**
  1. `TransformContext<'a>` (`ops/mod.rs:22-36`) carries only a single `pub info_sheet: Option<&'a InfoSheet>` rather than `info_sheets: &'a [InfoSheet]` + primary ID (contrast `curvebook_fit::HookContext` at `crates/engine/src/dag.rs:1059-1065`). In `ops/mod.rs:48-58`, any `HookSource::InfoConstant { info_sheet_id: Some(expected_id), .. }` whose ID differs from `self.info_sheet` returns `Err(TransformError::MissingHook)`. Moreover, `apply_import_link_pipeline` takes `info_sheet: Option<&mut InfoSheet>`, yet `apply_row_exclusion_predicate` (`import.rs:535`) builds `ScalarEnv::new(..., None)` and `ImportRowEnv::resolve_info_constant` (`import.rs:517`) unconditionally returns `None`.
  2. In `ops/mod.rs:67-79`, `resolve_hook` for `HookSource::ResultScalar` guards against `ResultStatus::Failed` (`73`) but does not reject `ResultStatus::Dirty`, returning stale scalars via `res.lookup_scalar` whenever an upstream `ResultSheet` has not been refreshed (see `FA-SEC02-08`).
- **Failure scenario:** Running `Normalize` where `zero_anchor` hooks `InfoSheet A` (`"blank"`) and `hundred_anchor` hooks `InfoSheet B` (`"saturation"`) passes `InfoSheet A` only from `dag.rs:1008-1017`, failing `resolve_hook` on `hundred_anchor` with `InfoSheet A does not match hooked B`. Calling `apply_import_link_pipeline` with an `exclude_row_predicate` such as `Response < Info("Cutoff")` and a bound `info_sheet` evaluates `Info("Cutoff")` to `Missing` and silently fails to exclude rows.
- **Evidence:**
  ```rust
  // crates/transform/src/ops/mod.rs:48-58, 67-79
  HookSource::InfoConstant { info_sheet_id, constant_name } => {
      let info = self.info_sheet.ok_or_else(...)?;
      if let Some(expected_id) = info_sheet_id && &info.id != expected_id {
          return Err(TransformError::MissingHook {
              description: format!("InfoSheet `{}` does not match hooked `{}`", info.id, expected_id),
          });
      } ...
  }
  HookSource::ResultScalar { result_sheet_id, parameter_key, data_set_index } => {
      ...
      if let ResultStatus::Failed { message, .. } = &res.status { return Err(...); }
      res.lookup_scalar(parameter_key, *data_set_index)...
  }

  // crates/transform/src/import.rs:517-519, 535
  fn resolve_info_constant(&self, _name: &str) -> Option<crate::formula::FormulaValue> { None }
  ...
  let scalars = ScalarEnv::new(vec![("$row", (r + 1) as f64), ("$index", r as f64)], None);
  ```
- **Suggested fix:** Add `info_sheets: &'a [InfoSheet]` (keeping `info_sheet` as primary/default or resolving by `info_sheet_id` across `info_sheets` with fallback to `info_sheet`), reject `ResultStatus::Dirty` in `resolve_hook` for `HookSource::ResultScalar`, and forward `info_sheet.as_deref()` into `ImportRowEnv` / `ScalarEnv` in `apply_row_exclusion_predicate`.
- **Related:** `FA-SEC02-08`, `crates/engine/src/dag.rs:1008-1066`.

---

### `FA-SEC03-04` · `S2` · `Math`, `Normalize`, `RemoveBaseline`, and `PruneRows(AverageBins)` mutate `Contingency` tables in place without calling `validate_counts()`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/ops/math.rs:344, 391-393`, `crates/transform/src/ops/normalize_baseline.rs:106, 159, 279, 516`, `crates/transform/src/ops/reshape_select.rs:590-595`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** Unlike `execute_select_with_derived` (`reshape_select.rs:1076, 1265, 1327`) and `archetype_convert` (`2589, 2732, 2850`), which call `ContingencyTable::validate_counts()`, and `average_bins_archetype` (`reshape_select.rs:543`), which rejects `Survival`, `execute_math_transform`, `execute_normalize`, `execute_remove_baseline_kind`, and `average_bins_archetype` mutate `source.clone()` in place across `TableArchetype::Contingency` (and `Survival`/`PartsOfWhole`) without validating count invariants on output.
- **Failure scenario:** Running `Math(Log10)`, `Normalize`, `RemoveBaseline(Difference)`, or `PruneRows(AverageBins(2))` on a valid `ContingencyTable` produces a `DataSheet` whose `TableArchetype::Contingency` holds negative or fractional numbers (`-5.0`, `1.301`, `15.5`). Because `ContingencyTable::validate()` checks only rectangular bounds (`crates/doc/src/archetypes.rs:988-996`), `execute_transform` succeeds and stores a count-invalid `ContingencyTable` in `ResultSheet::analyzable_data`, causing downstream contingency analyses (`count_at`) to fail with `DocError::InvalidCellValue` ("contingency table cells must be non-negative integer counts").
- **Evidence:**
  ```rust
  // crates/transform/src/ops/math.rs:391-393 (similarly normalize_baseline.rs:279, 516; reshape_select.rs:590-595)
  TableArchetype::Contingency(t) => {
      transform_dataset_vec(&mut t.data_sets, SubcolumnMode::Single, selected, op, err_mode, x_ctx, info, count, warnings)
  }
  ...
  Ok(TransformOutput { data_sheet: out, summary_tab, scalars: Vec::new(), warnings })
  ```
- **Suggested fix:** Either validate `t.validate_counts()?` after mutating a `Contingency` table (consistent with `reshape_select.rs:1076, 1265`), or return `TransformError::IncompatibleTable` on non-integer-preserving transforms (`Normalize`, `AverageBins`, domain math) for `Contingency`/`Survival` tables; call `out.archetype.validate()?` before returning `TransformOutput`.
- **Related:** `crates/transform/src/ops/reshape_select.rs:543, 1076, 1265, 1327`, `crates/doc/src/archetypes.rs:978-985`.

---

### `FA-SEC03-05` · `S3` · `select_log_spaced_indices` breaks on descending or non-monotonic `Xy` coordinates and can duplicate rows on oscillating `X`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/ops/reshape_select.rs:459-478`
- **Category:** `wrong-result`
- **Defect:** For `TableArchetype::Xy`, `select_log_spaced_indices` uses `xs.first()` and `xs.last()` as `[log_min, log_max]` endpoints instead of the sorted min/max of positive X values, checks `if log_max > log_min`, and deduplicates only consecutively equal indices (`out.last().copied() != Some(*best_r)`).
- **Failure scenario:**
  1. Serial dilution data entered in descending X order (e.g., `[1000, 100, 10, 1]`) has `log_max < log_min`, so `log_max > log_min` is `false` and the routine silently falls back to row-index `powf` spacing ignoring X coordinates.
  2. Unsorted X values where `first_x < last_x` (e.g., `[1, 10, 1, 100]`) allow `best_r` to pick row `0`, then row `1`, then row `0` again (`out.last()` is `Some(1)`, not `Some(0)`), returning non-distinct indices `[0, 1, 0, ...]` that duplicate rows in `retain_rows` and overwrite `old_to_new` entries in `remap_exclusion_notes`.
- **Evidence:**
  ```rust
  // crates/transform/src/ops/reshape_select.rs:459-476
  if let TableArchetype::Xy(xy) = arch {
      let xs: Vec<(usize, f64)> = (0..n).filter_map(|r| xy.x_value_at(r).filter(|&x| x > 0.0).map(|x| (r, x))).collect();
      if let (Some(&(_, first_x)), Some(&(_, last_x))) = (xs.first(), xs.last())
          && xs.len() >= 2
      {
          let log_min = first_x.ln();
          let log_max = last_x.ln();
          if log_max > log_min {
              for step in 0..target_points {
                  let target_ln = log_min + ((step as f64) / ((target_points - 1) as f64)) * (log_max - log_min);
                  if let Some((best_r, _)) = xs.iter().min_by(|a, b| (a.1.ln() - target_ln).abs().total_cmp(&(b.1.ln() - target_ln).abs()))
                      && out.last().copied() != Some(*best_r)
  ```
- **Suggested fix:** Sort a working copy of `(r, x)` by `x` (`total_cmp`) before computing `log_min`/`log_max`, track selected indices in a `BTreeSet` (or emit sorted distinct indices preserving table row order or X order consistently), and guard `x.is_finite() && x > 0.0`.
- **Related:** `crates/transform/src/randomize.rs:714-717` (`remap_exclusion_notes` assumes distinct selected row indices).

---

### `FA-SEC03-06` · `S3` · `lowess_point` returns unaveraged raw `ys[i]` when `>= q` points tie at `xs[i]`, and `compute_auc_internal` omits `min_peak_height`/`min_area_fraction` validation

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/curve.rs:248-271, 598-621`
- **Category:** `wrong-result`
- **Defect:**
  1. In `lowess_point` (`602-605`), when the `q`-th nearest neighbor has distance `max_d == 0.0` (meaning `>= q` points share `xs[i]`), the function immediately returns raw `ys[i]` without averaging or applying robustness weights across the tied neighbors, unlike `local_poly_eval` (`531-533`) and `collapse_tied_auc_points` (`277-305`). When `ys` differ across tied `x`, points at identical `x` receive different smoothed outputs (`ys[i]`) and depend on row order if `max_d == 0.0` or during robustness iterations.
  2. `compute_auc_internal` (`249-251`) validates `opts.baseline.is_finite()` but never validates `opts.min_peak_height` or `opts.min_area_fraction` (`265-266`): passing `NaN` silently filters out every peak (`>= f64::NAN` is `false`), and negative/out-of-range thresholds are accepted without error.
- **Failure scenario:** Calling `smooth_xy` with `SmoothMethod::Lowess { span: 0.3, robustness_iters: 0 }` on 10 points containing 3 ties `[(1.0, 0.0), (1.0, 10.0), (1.0, 20.0), ...]` returns unsmoothed `[0.0, 10.0, 20.0, ...]` at `x = 1.0`. Calling `compute_auc` with `min_peak_height: f64::NAN` silently returns `peaks: []` instead of `TransformError::InvalidSpec`.
- **Evidence:**
  ```rust
  // crates/transform/src/curve.rs:248-251, 263-266, 602-605
  if !opts.baseline.is_finite() {
      return Err(TransformError::InvalidSpec { message: String::from("AUC baseline must be finite") });
  }
  ...
  .filter(|p| p.height_above_baseline >= opts.min_peak_height)
  .filter(|p| pos_area <= 0.0 || (p.area / pos_area) >= opts.min_area_fraction)
  ...
  let max_d = dists[q - 1].1;
  if max_d == 0.0 {
      return ys[i];
  }
  ```
- **Suggested fix:** When `max_d == 0.0` in `lowess_point`, return the robustness-weighted mean across all points with `(xs[j] - xi).abs() == 0.0` (falling back to unweighted mean if robustness weights sum to `0.0`); validate `opts.min_peak_height.is_finite() && opts.min_peak_height >= 0.0` and `opts.min_area_fraction.is_finite() && (0.0..=1.0).contains(&opts.min_area_fraction)` in `compute_auc_internal`.
- **Related:** `crates/transform/src/curve.rs:531-533` (`local_poly_eval` tie averaging), `crates/transform/tests/d1_curve_transforms.rs:218-242`.

---

### `FA-SEC03-07` · `S3` · Wide anchor/grid spans overflow `(hundred - zero)` / `(x_max - x_min)` to `+Inf`, and `normalize_dataset_col` silently blanks undefined `%CV`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/ops/normalize_baseline.rs:68-87, 269-274, 321-345`; `crates/transform/src/simulate.rs:125-126` (contrast `ops/math.rs:572-582`, `ops/normalize_baseline.rs:648-650`)
- **Category:** `wrong-result`
- **Defect:**
  1. `execute_normalize` (`70, 81`) checks only `(h - z).abs() == 0.0`. When finite anchors span opposite signs near `f64::MAX` (e.g., `zero = -1e308, hundred = 1e308`), `hundred - zero` (`270`) overflows to `+Inf`, `factor = scale / Inf = 0.0`, and finite values collapse to `0.0` or `Empty` (`+Inf * 0.0 = NaN`). The same unchecked subtraction `x_max - x_min` occurs in `simulate_xy_sheet` (`simulate.rs:125-126`), producing `NaN` (`0.0 * Inf`) at `row == 0`.
  2. In `normalize_dataset_col` (`344`), when `m_out == 0.0` or `100.0 * sd_out / m_out.abs()` overflows to `+Inf`, `row_cells[1].value` is silently blanked via `CellValue::from_finite_or_empty` / `CellValue::Empty` without appending a warning to `warnings`, contradicting both `ops/math.rs:572-582` and `apply_baseline_mean_cv_row` (`normalize_baseline.rs:648-650`). Similarly `normalize_dataset_col` (`324`) silently blanks cells whose `map_val(v)` overflows `f64`.
- **Failure scenario:** Calling `execute_normalize` with `FixedValue(-1e308)` and `FixedValue(1e308)` collapses `-1e308` and modest values to `0.0` and blanks `+1e308` instead of rejecting non-finite span or scaling `(0.5*v - 0.5*z) / (0.5*h - 0.5*z)`; normalizing `[Mean=50, CV%=10, N=3]` with `zero = FixedValue(50.0)` shifts `Mean` to `0.0` and clears `%CV` with `warnings.is_empty()`.
- **Evidence:**
  ```rust
  // crates/transform/src/ops/normalize_baseline.rs:70, 269-273, 344
  if (h - z).abs() == 0.0 { ... }
  ...
  let factor = scale / (hundred - zero);
  let reverses = factor < 0.0;
  let map_val = |v: f64| (v - zero) * factor;
  ...
  row_cells[1].value = if m_out != 0.0 { CellValue::from_finite_or_empty(100.0 * sd_out / m_out.abs()) } else { CellValue::Empty };

  // crates/transform/src/simulate.rs:125-126
  let frac = (row as f64) / ((row_count - 1) as f64);
  x_min + frac * (x_max - x_min)
  ```
- **Suggested fix:** Check `(h - z).is_finite()` or use midpoint-safe scaling `(1.0 - t) * a + t * b` / half-scaled `(0.5 * h - 0.5 * z)` when differences overflow; use `(1.0 - frac) * x_min + frac * x_max` in `simulate_xy_sheet`; pass `warnings: &mut Vec<String>` into `normalize_dataset_col` and warn when `%CV` or transformed values are blanked (for unexcluded cells).
- **Related:** `crates/transform/src/ops/math.rs:572-582`, `crates/transform/src/ops/normalize_baseline.rs:648-650`.

---

### `FA-SEC03-08` · `S3` · `PlateMapTemplate` JSON parser `json_codec::parse_string` executes `O(N^2)` UTF-8 validation on multi-byte characters

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/plate_and_tidy.rs:1971-1975`
- **Category:** `resources`
- **Defect:** In `JsonParser::parse_string`, every non-ASCII byte at `self.pos` invokes `std::str::from_utf8(&self.bytes[self.pos..])`, validating the *entire remaining JSON byte slice to EOF* on every single non-ASCII character. Even though `from_cbtemplate_json` (`226`) bounds input to `4 MiB` and `from_json_str` (`1463`) already takes `&'de str` (guaranteed valid UTF-8), a string containing many multi-byte UTF-8 characters degrades to quadratic `O(N^2)` work.
- **Failure scenario:** Deserializing a crafted 4 MiB `.cbtemplate` JSON file whose string field holds ~2M 2-byte non-ASCII characters (e.g., `'é'` or `'µ'`) executes `~2 * 10^12` UTF-8 continuation-byte checks inside `parse_string`, stalling the desktop or WASM UI thread without hitting any parser byte or depth limit.
- **Evidence:**
  ```rust
  // crates/transform/src/plate_and_tidy.rs:1971-1975
  } else {
      let rest = std::str::from_utf8(&self.bytes[self.pos..]).map_err(|_| JsonCodecError::new("invalid UTF-8 in JSON"))?;
      let ch = rest.chars().next().ok_or_else(|| JsonCodecError::new("unexpected EOF"))?;
      out.push(ch);
      self.pos += ch.len_utf8();
  }
  ```
- **Suggested fix:** Store `input: &'de str` alongside `bytes` (or slice at most `&self.bytes[self.pos..(self.pos + 4).min(self.bytes.len())]`, finding char length via leading-byte UTF-8 width or `utf8_chunks`) so decoding each non-ASCII code point is strict `O(1)`.
- **Related:** `crates/transform/src/plate_and_tidy.rs:23, 1463-1471`.

---

### `FA-SEC03-09` · `S3` · `pivot_tidy_long_to_datasheet` drops `x_excluded` and `decimal_sep == ','` replicates in `Grouped`/`MultipleVariables` pivots, and silently shifts colliding replicates

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/plate_and_tidy.rs:455-458, 865-873, 1039-1041, 1099-1195, 1236-1256`
- **Category:** `wrong-result` / `data-loss`
- **Defect:**
  1. `collect_tidy_records` (`865-873`) populates `rec.x_excluded` when `x_col` has a trailing `*`, and `pivot_tidy_to_xy` (`1051-1056`) propagates it onto cells and notes. However, `pivot_tidy_to_grouped` (`1099-1195`) ignores `rec.x_excluded` entirely (never excluding cells or emitting notes/warnings), and `pivot_tidy_to_mv` (`1236-1256`) sets `excluded = rec.x_excluded` on `mv.variables[xi]` without generating corresponding `CellExclusionNote`s in `notes` (only `val_var` notes are created at `1253-1256`).
  2. In `pivot_tidy_to_mv` (`1247`), `rec.replicate` is parsed using `rec.replicate.parse::<f64>()` rather than respecting `decimal_sep` (`','`), turning decimal-comma numeric replicate identifiers into `Cell::text`.
  3. When two plate wells (`build_treatment_grid:455-458`) or tidy rows (`pivot_tidy_to_xy:1039-1041`, `pivot_tidy_to_grouped:1122-1124`) collide on the same `(row, group, replicate)`, `while ... contains_key(...) { slot += 1; }` silently bumps the duplicate into a neighboring replicate slot with no diagnostic warning.
- **Failure scenario:** Pivoting tidy CSV with `decimal_sep = ','`, `"10,5*"` in `x_col`, and `"1,5"` in `replicate_col` into `MultipleVariables` parses `X = 10.5` with `excluded = true` and **zero** exclusion note for `X`, and stores `Replicate = Text("1,5")` instead of `Number(1.5)`; pivoting the same tidy input to `Grouped` loses `x_excluded` silently.
- **Evidence:**
  ```rust
  // crates/transform/src/plate_and_tidy.rs:1236-1256
  if let Some(xi) = x_var {
      mv.variables[xi].cells[r] = match (rec.x_num, rec.x_token.as_ref()) {
          (Some(n), _) => Cell { value: CellValue::Number(n), excluded: rec.x_excluded },
          ...
      };
  }
  ...
  if let Some(ri) = rep_var && !rec.replicate.is_empty() {
      mv.variables[ri].cells[r] = match rec.replicate.parse::<f64>() { ... };
  }
  ...
  if rec.value.excluded {
      ...
      notes.push(CellExclusionNote { coord: CellCoord::new(r, val_var, 0), ... });
  }
  ```
- **Suggested fix:** Emit a `CellExclusionNote` for `x_var` when `rec.x_excluded` in `pivot_tidy_to_mv`; propagate `rec.x_excluded` (or emit a warning) in `pivot_tidy_to_grouped`; parse numeric `rec.replicate` respecting `decimal_sep`; track replicate-slot collisions and surface a non-fatal warning or error in `PlateMapTemplate::validate`/pivot summaries.
- **Related:** `docs/audit/local-code-audit-2026-11-18.md:609`.

---

### `FA-SEC03-10` · `S3` · `archetype_convert` bypasses `MAX_IMPORT_CELLS` on wide `XY`/`Column -> MV` flattening, emits empty `data_sets` in `pivot_mv_summary_to_grouped`, and drops `row_titles` on replicate unrolling

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/archetype_convert.rs:1012, 1467-1484, 1773, 1855-1870, 2210-2233`
- **Category:** `resources` / `contract-drift` / `data-loss`
- **Defect:**
  1. Unlike `convert_grouped_reps_to_mv` (`1010`), `convert_xy_reps_to_mv` (`1772`), and `unstack_mv_to_column` (`2361`), neither `convert_xy_wide_to_mv` (`1855-1870`) nor `convert_column_to_mv` (`2210-2233`) calls `validate_cell_budget(rows, total_vars, 1, 0, ...)` before `MultipleVariablesTable::new(total_vars, rows)`. A valid 200-column x 3-subcolumn (`MeanSdN`) x 49,000-row table (`29.4M <= MAX_SHEET_CELLS`) bypasses `MAX_IMPORT_CELLS = 10_000_000` and allocates another `29.4M`-cell `MultipleVariablesTable`.
  2. In `pivot_mv_summary_to_grouped` (`1467-1484`), if all rows have missing/excluded factor values (`entries.is_empty()`), line `1474` clears `grouped.data_sets.clear()` (`0` data columns) and skips copying factor titles (`1476-1477`), whereas `pivot_mv_two_way_to_grouped` (`1296`) rejects empty `col_levels` with `TransformError::InvalidInput`.
  3. `convert_xy_reps_to_mv` (`1773`) and `convert_grouped_reps_to_mv` (`1012`) drop source `row_titles` silently without emitting a `RowTitle`/`row_titles` column or warning (contrast `convert_xy_single_to_mv:1671`).
- **Failure scenario:** Converting a `ColumnTable` with `300` `MeanSdN` columns and `30,000` rows (`27M` cells) to `MultipleVariables` bypasses `MAX_IMPORT_CELLS` (`10M`); pivoting a `MultipleVariables` summary table whose categorical factor columns are empty/excluded to `Grouped` silently constructs a 0-column `GroupedTable` instead of returning `InvalidInput`.
- **Evidence:**
  ```rust
  // crates/transform/src/archetype_convert.rs:1472-1475, 1866-1869, 2221-2233
  let mut grouped = GroupedTable::new(mode, col_levels.len().max(1), row_levels.len())?;
  if col_levels.is_empty() {
      grouped.data_sets.clear();
  } ...
  ...
  validate_data_sets(total_vars)?;
  let var_titles = collect_xy_wide_mv_titles(xy, x_subs, y_subs, total_vars)?;
  let mut mv = MultipleVariablesTable::new(total_vars, rows)?; // no validate_cell_budget
  ```
- **Suggested fix:** Enforce `validate_cell_budget` in `convert_xy_wide_to_mv` and `convert_column_to_mv`; return `Err(TransformError::InvalidInput)` when `col_levels.is_empty()` in `pivot_mv_summary_to_grouped`; preserve non-empty `row_titles` (or emit a `"Row_Label"` column / warning) when flattening replicates to `MultipleVariables`.
- **Related:** `docs/audit/local-code-audit-2026-11-18.md:609`, `crates/transform/tests/audit_r2_s04.rs:111`.

---

### `FA-SEC03-11` · `S3` · `import.rs` unstacking silently drops extra columns, `anchor.subcol` misaligns `Single`-table bounds/range, and `InternalData`/delimited stages bypass `MAX_IMPORT_CELLS`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/import.rs:190-202, 232, 270-273, 304-328, 411-424, 626, 638-663`
- **Category:** `wrong-result` / `resources`
- **Defect:**
  1. `unstack_grid_by_group` (`419`) computes `val_cell = row.iter().enumerate().find(|&(idx, _)| idx != group_col)`, silently discarding columns `2..` whenever a 3+ column grid is unstacked without an exact 2-column `col_window`; line `413` formats numeric keys with `format!("{v}")` without normalizing `-0.0` (splitting `"-0"` and `"0"` into separate output columns).
  2. When placing a grid into a `Single`-subcolumn table (`subs_per_col == 1`) with `anchor.subcol > 0`, `validate_grid_col_bounds` (`641-646`) adds `base_sub` into `needed_cols` (allocating `anchor.subcol` extra empty columns in `ensure_sheet_columns`), whereas `resolve_target_coord` (`651-654`) writes cells into `subcol = 0` at `base_col + offset`, and `target_range` (`626`) still records `start.subcol = anchor.subcol` and `end.subcol = anchor.subcol`.
  3. `parse_delimited_text` permits `MAX_IMPORT_CELLS + 65_536` cells (`305`) and `MAX_ROWS + 16` rows (`326`), and `extract_stage1` for `ImportSourcePayload::InternalData` (`232, 272`) materializes `sheet.extract_cell_grid(...)` up to `MAX_SHEET_CELLS = 30M` before checking `MAX_IMPORT_CELLS = 10M` inside `place_grid`.
- **Failure scenario:** Placing a 2-column CSV into a `Single`-subcolumn `ColumnTable` with `anchor = CellCoord::new(0, 0, 5)` allocates 7 columns (`A..G`), writes data only into columns `A..B`, and returns `target_range = (0, 0, 5)..(r, 1, 5)` pointing to nonexistent subcolumn `5`. Unstacking `[ID, Group, Response]` with `group_col = 1` silently outputs `ID` values instead of `Response`.
- **Evidence:**
  ```rust
  // crates/transform/src/import.rs:419, 641-662
  let val_cell = row.iter().enumerate().find(|&(idx, _)| idx != group_col).map_or_else(Cell::empty, |(_, c)| c.clone());
  ...
  let total_flat = base_col.checked_mul(subs_per_col).and_then(|v| v.checked_add(base_sub)).and_then(|v| v.checked_add(span))?;
  ...
  if subs_per_col <= 1 {
      let col = base_col.checked_add(offset)?;
      Ok((col, 0))
  ```
- **Suggested fix:** Require `anchor.subcol < subcolumns_per_data_column(&target.archetype)` (rejecting out-of-range `anchor.subcol` with `TransformError::InvalidSpec` and setting `max_sub = 0` cleanly); normalize `-0.0` in `unstack_grid_by_group` and enforce `row.len() <= 2` (or expose explicit `value_column` selection); enforce `MAX_IMPORT_CELLS` in `extract_stage1` for `InternalData` before `extract_cell_grid`.
- **Related:** `FA-SEC02-02` (`linked_regions` unshifted during `InsertRows`), `crates/doc/src/links.rs:424-455`.

---

### `FA-SEC03-12` · `S4` · `MathTransformOp::Antilog` `base == 1.0` acceptance and `MathTransformPreset::Probit` documentation mismatch

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/ops/math.rs:69-73, 129-136, 213-219` (cross-reference `crates/doc/src/analysis.rs:112-113`, `crates/transform/src/formula/eval.rs:2111-2117`)
- **Category:** `contract-drift`
- **Defect:**
  1. `MathTransformOp::Antilog` (`69`) documents `base > 0, base != 1`, whereas `CompiledOp::compile` (`131`) checks only `*base <= 0.0` and accepts `base == 1.0` (evaluating `1.0^u = 1.0` with zero derivative).
  2. `ops/math.rs:215` and `formula/eval.rs:2113` implement Finney's +5 probit `5.0 + norm_ppf(v)`, while `MathTransformPreset::Probit` in `crates/doc/src/analysis.rs:112` documents `Inverse standard normal CDF Phi^{-1}(u)` without the `+ 5` offset (compare Zar 2010 / `formula/eval.rs:2118` `norm_ppf`).
- **Failure scenario:** Callers passing `MathTransformOp::Antilog { base: 1.0 }` succeed silently instead of receiving `InvalidSpec`; callers reading `MathTransformPreset::Probit` docstrings expect zero-centered z-scores `Phi^{-1}(0.5) == 0.0` instead of Finney probits `5.0`.
- **Evidence:**
  ```rust
  // crates/transform/src/ops/math.rs:69, 130-133, 215
  /// Arbitrary-base exponential `base^u` (`base > 0`, `base != 1`).
  Antilog { ... }
  ...
  if *base <= 0.0 {
      return Err(TransformError::InvalidSpec { message: format!("Antilog base must be > 0, got {base}") });
  }
  ...
  dist::norm_ppf(v).ok().map(|z| 5.0 + z)
  ```
- **Suggested fix:** Reject `(*base - 1.0).abs() == 0.0` (or update `Antilog` rustdoc to document `base > 0`), and update `crates/doc/src/analysis.rs:112` (`MathTransformPreset::Probit`) to specify Finney's probit `5 + Phi^{-1}(u)` consistently with `ops/math.rs:215` and `formula/eval.rs:2113`.
- **Related:** `docs/audit/local-code-audit-2026-11-18.md:609`.

---

### `FA-SEC03-13` · `S4` · `countif(col, "true")` / `"false"` cannot match literal strings and `eval_scalar_const_arg` reads row `0` of non-constant column expressions

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/formula/eval.rs:1342-1360, 1451-1464`
- **Category:** `wrong-result` / `robustness`
- **Defect:**
  1. In `countif` (`1343-1359`), the 2-argument literal-text equality shorthand `Expr::Text(lit)` explicitly excludes `"true"` and `"false"` (`1345-1346`), falling through to `resolve_cond_val_pair` (`1358`) where `"true"` is coerced via `FormulaValue::as_bool()` to `Some(true)` for *every* row and `CountIf` restricts `vals` to `as_finite_f64().is_some()` (`1195`). Thus `countif(StatusCol, "true")` returns `0` on text columns and counts all numeric rows regardless of string equality.
  2. `eval_scalar_const_arg` (`1460-1462`, used by `percentile`, `quantile`, `lag`, `lead`, `diff`, `moving_avg`) evaluates the argument as a column vector and takes `.first()` on non-empty tables, silently accepting column expressions (e.g. `lag(Y, OffsetCol)`) using only row `0`'s value instead of rejecting non-scalar parameters or evaluating against scalar/info context only (`1452-1459`).
- **Failure scenario:** `countif(LabelCol, "true")` returns `0` even when rows contain text `"true"`; `lag(Response, StepCol)` silently uses `StepCol[0]` for every row instead of raising an error.
- **Evidence:**
  ```rust
  // crates/transform/src/formula/eval.rs:1343-1358, 1460-1463
  if let Expr::Text(lit) = &args[1]
      && !is_condition_ast(&args[0])
      && !lit.eq_ignore_ascii_case("true")
      && !lit.eq_ignore_ascii_case("false")
  ...
  let vals = eval_column_expr_inner(expr, ctx)?;
  vals.first().and_then(FormulaValue::as_finite_f64).ok_or_else(...)
  ```
- **Suggested fix:** Remove the `"true"`/`"false"` exclusion in `countif` (`Expr::Text` literals represent string match patterns; boolean conditions are written unquoted as `true`/`false` or `Col == "true"`), and evaluate `eval_scalar_const_arg` using an `Info`-only `ScalarEnv` (or verify zero `ColumnName`/`X`/`ColIndex` dependencies) so row-varying column expressions are cleanly rejected.
- **Related:** `crates/transform/src/formula/eval.rs:1195, 1338-1367`.

---

### `FA-SEC03-14` · `S4` · `RandomSample` still emits `*seed as f64` in `ResultScalarEntry`, losing precision for seeds above `2^53`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/ops/mod.rs:205, 214` (cross-reference `crates/transform/tests/audit_r1_s4b_transform_s3.rs:43-71`)
- **Category:** `wrong-result` / `cleanup`
- **Defect:** `R1-S04-12` fixed `ResultTab` seed display (`ops/mod.rs:205` uses `ResultCell::Text(seed.to_string())`), yet line `214` still emits `ResultScalarEntry { key: "seed", value: *seed as f64, ... }`, silently rounding any `u64` seed `> 2^53`.
- **Failure scenario:** Passing `seed = 9_007_199_254_740_993_u64` (`2^53 + 1`) records `"9007199254740993"` in the summary tab but `9007199254740992.0` in `ResultSheet::scalars`; any downstream hook reading `ResultScalar { parameter_key: "seed", .. }` receives an altered seed.
- **Evidence:**
  ```rust
  // crates/transform/src/ops/mod.rs:205, 214
  ("Seed", ResultCell::Text(seed.to_string())),
  ...
  ResultScalarEntry { key: String::from("seed"), data_set_index: None, value: *seed as f64, units: None },
  ```
- **Suggested fix:** Omit `"seed"` from numeric `ResultScalarEntry` (only exact representable integer counts belong in `f64` scalars, or check exact `<= (1_u64 << 53)` before emitting a numeric scalar).
- **Related:** `docs/audit/local-code-audit-2026-11-18.md:609`, `crates/transform/tests/audit_r1_s4b_transform_s3.rs:50-61`.

---

### `FA-SEC03-15` · `S4` · `SimulatedDistribution::Exponential` lacks finite overflow check and `ResultCell::ConfidenceInterval` drops on import

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/randomize.rs:828`; `crates/transform/src/import.rs:285`
- **Category:** `robustness` / `wrong-result`
- **Defect:**
  1. In `sample_distribution_value` (`randomize.rs:820-828`), `LogNormal` checks `!exp_z.is_finite()` and returns `TransformError::EvalError`, whereas `Exponential` (`-rng.next_open_f64().ln() / rate`) does not check finiteness; for subnormal `rate > 0.0` (e.g., `1e-320`) or extreme `Gaussian` parameters, `+Inf` passes into `Cell::number` and fails downstream inside `DataSheet::new` as `TransformError::Doc` instead of `EvalError`.
  2. In `import::result_cell_to_cell` (`import.rs:285`), `ResultCell::ConfidenceInterval { .. }` maps unconditionally to `Cell::empty()`, silently discarding interval cells when importing a `ResultSheet` tab.
- **Failure scenario:** Simulating `Exponential { rate: 1e-320 }` fails inside `DataSheet::new` (`DocError`) rather than `EvalError`; linking a `ResultSheet` column containing confidence intervals imports blank cells without warning.
- **Evidence:**
  ```rust
  // crates/transform/src/randomize.rs:821, 828
  if !exp_z.is_finite() {
      return Err(TransformError::EvalError { ... });
  }
  ...
  SimulatedDistribution::Exponential { rate } => Ok(-rng.next_open_f64().ln() / rate),

  // crates/transform/src/import.rs:285
  ResultCell::ConfidenceInterval { .. } => Cell::empty(),
  ```
- **Suggested fix:** Validate `.is_finite()` on all continuous distribution draws in `sample_distribution_value` (or return `EvalError` cleanly), and either map `ConfidenceInterval` to formatted text or warn when importing CI cells into a scalar `DataSheet` grid.
- **Related:** `crates/transform/src/randomize.rs:788-799`.

---

### `FA-SEC03-16` · `S4` · Minor normalization and cleanup defects in `archetype_convert.rs` (`-0.0` row labels, dead branch, `rep_col` occupation counting)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/transform/src/archetype_convert.rs:1040, 1288-1290, 2130-2132, 2459, 2620`
- **Category:** `cleanup` / `wrong-result`
- **Defect:**
  1. `convert_xy_to_grouped` (`2459`) and `convert_xy_to_contingency` (`2620`) format numeric `X` coordinates into row labels via `format!("{v}")` without normalizing `-0.0` (unlike lines `840`, `2880`, and `2952`), emitting `"-0"`.
  2. `convert_grouped_reps_to_mv` (`1040`) computes `let schema_lossy = grp.subcolumn_mode.is_pre_summarized();` even though the function is dispatched solely for `SubcolumnMode::RawReplicates(_)`.
  3. `pivot_mv_two_way_to_grouped` (`1288-1290`) and `pivot_mv_grouped_xy` (`2130-2132`) increment `mapped_fac_cells += 1` whenever `rep_col` cell `is_occupied`, even if the cell is excluded or non-numeric text (`rep_order = None`) and therefore discarded rather than mapped.
- **Failure scenario:** Converting an `XyTable` with `x = -0.0` and blank `row_titles` into `Grouped` labels row 0 as `"-0"` instead of `"0"`; converting a `MultipleVariablesTable` with an excluded/text `"Replicate"` cell inflates `ConversionPreview::preserved_cells` while omitting it from `dropped_cells`.
- **Evidence:**
  ```rust
  // crates/transform/src/archetype_convert.rs:1040, 1287-1290, 2459
  let schema_lossy = grp.subcolumn_mode.is_pre_summarized();
  ...
  let rep_order = rep_col.and_then(|rc| mv.variables[rc].cells.get(r).and_then(Cell::active_number));
  if rep_col.is_some_and(|rc| mv.variables[rc].cells.get(r).is_some_and(is_occupied)) {
      mapped_fac_cells += 1;
  }
  ...
  (None, Some(v)) => format!("{v}"),
  ```
- **Suggested fix:** Normalize `-0.0` -> `0.0` in `convert_xy_to_grouped` and `convert_xy_to_contingency`; remove the dead `is_pre_summarized()` check in `convert_grouped_reps_to_mv`; increment `mapped_fac_cells` only when `rep_order.is_some()` and otherwise count `is_occupied` discarded replicate cells in `dropped_cells`.
- **Related:** `crates/transform/tests/audit_r2_s04.rs:106`.
