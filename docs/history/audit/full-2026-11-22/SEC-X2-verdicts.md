# SEC-X2 Verification Verdicts

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Date:** `2026-11-22`
- **Verifier:** `ver-frontend-3`
- **Target Report:** [SEC-X2.md](SEC-X2.md)
- **Mode:** Report-only independent verification (no repository source/test/config files modified)

---

## Summary Table

| Metric | Value |
|---|---:|
| **Findings Verified** | `8` (`FA-X2-01` .. `FA-X2-08`) |
| `CONFIRMED` | `8` |
| `PLAUSIBLE` | `0` |
| `REFUTED` | `0` |
| `DUPLICATE` | `0` |
| **Settled Severities** | `S1 = 1`, `S2 = 6`, `S3 = 1`, `S4 = 0` |
| **Seam Matrix Completeness** | `PASS` (`13` seams across Families 1–4 covering 100% of `X2` scope) |

| Finding ID | Verdict | Raw Severity | Settled Severity | Category | Non-Duplication Check |
|---|---|---:|---:|---|---|
| `FA-X2-01` | `CONFIRMED` | `S1` | `S1` | `data-fidelity` / `silent-regression` | Unique (`table.pasteLink` -> `dag::refresh_single_region`) |
| `FA-X2-02` | `CONFIRMED` | `S2` | `S2` | `data-fidelity` / `contract-drift` | Unique (`AnalyzableResultSheet` `tab_id: None` link flip) |
| `FA-X2-03` | `CONFIRMED` | `S2` | `S2` | `data-fidelity` / `usability-breakage` | Distinct from `FA-SEC07-12` (parametric vs non-moment modes) |
| `FA-X2-04` | `CONFIRMED` | `S2` | `S2` | `data-fidelity` / `cross-crate-divergence` | Distinct from `FA-SEC07-08` (Deming SD/N vs `X` collapse) |
| `FA-X2-05` | `CONFIRMED` | `S2` | `S2` | `data-fidelity` / `cross-crate-divergence` | Distinct from `FA-SEC06-01`, `FA-SEC09-05`, `FA-SEC09-08` |
| `FA-X2-06` | `CONFIRMED` | `S2` | `S2` | `data-fidelity` / `cross-crate-contract-drift` | Distinct from `FA-SEC12-01` (`convertArchetype`) |
| `FA-X2-07` | `CONFIRMED` | `S2` | `S2` | `data-fidelity` / `data-loss` | Distinct from `FA-SEC02-10` and `FA-SEC04-06` |
| `FA-X2-08` | `CONFIRMED` | `S3` | `S3` | `data-fidelity` / `layout-mismatch` | Distinct engine seam beyond L2 `FA-SEC03-15` |

---

## Seam Matrix Completeness Check (`PASS`)

Checked against [slices.md](../../../../docs/audit/slices.md#L23-L57) (`X2`: `S05` cells through `S04`/`S02`/`S03` results to `S06` save and reopen; follow excluded cells, missing values, replicate subcolumns, and ties) and all `13` seams in [SEC-X2.md](SEC-X2.md#L14-L48):

| Family | Required `X2` Axes | Seams Traced | Independent Spot-Check Evidence | Status |
|---|---|---:|---|---|
| **Family 1** | Cell exclusions (`CellExclusion`, `CellExclusionNote`, shift / delete / undo alignment, CSV / archive round-trips, `PointStyleOverride`) | Seams `1`–`5` | [links.rs](../../../../crates/doc/src/links.rs#L155-L198), [archetypes.rs](../../../../crates/doc/src/archetypes.rs#L1983-L2045), [eval.rs](../../../../crates/transform/src/formula/eval.rs#L35-L45), [formula/dag.rs](../../../../crates/transform/src/formula/dag.rs#L132-L144), [randomize.rs](../../../../crates/transform/src/randomize.rs#L260-L277), [ops/math.rs](../../../../crates/transform/src/ops/math.rs#L513-L590), [archive.rs](../../../../crates/format/src/archive.rs#L898-L920), [methods_report.rs](../../../../crates/engine/src/methods_report.rs#L2060-L2084) | `PASS` |
| **Family 2** | Missing / blank, `NaN`, `±Inf`, non-numeric text, X–Y pairing (`unknowns_y`/`unknowns_x`), & `ResultSheet` round-trips | Seams `6`–`8` | [dag.rs](../../../../crates/engine/src/dag.rs#L1188-L1285), [export.rs](../../../../crates/format/src/export.rs#L386-L431), [fit/src/types.rs](../../../../crates/fit/src/types.rs#L370-L436), [cmd/table/import.rs](../../../../crates/engine/src/cmd/table/import.rs#L47-L65,L220-L244,L345-L391), [cmd/sheet.rs](../../../../crates/engine/src/cmd/sheet.rs#L1960-L1986) | `PASS` |
| **Family 3** | `RawReplicates` & summarized subcolumn modes (`MeanSdN`, `MeanSemN`, `MeanCvN`, `n = 1`, `MeanLowHigh`, `UpperLowerLimits`, `MedianIqr`, asymmetric errors) | Seams `9`–`11` | [chart/src/xy.rs](../../../../crates/chart/src/xy.rs#L219-L310,L363-L442,L1815-L1848), [cell.rs](../../../../crates/doc/src/cell.rs#L227-L273,L321-L357), [ops/math.rs](../../../../crates/transform/src/ops/math.rs#L548-L590), [anova/sheet/one_way.rs](../../../../crates/stats/src/anova/sheet/one_way.rs#L953-L999), [column_grouped.rs](../../../../crates/chart/src/column_grouped.rs#L762-L776) | `PASS` |
| **Family 4** | Paired / matched designs, ties, row ordering, weights, dynamic DAG live-edit recomputation, & `MethodsReport` | Seams `12`–`13` | [transform/src/curve.rs](../../../../crates/transform/src/curve.rs#L764-L817), [dag.rs](../../../../crates/engine/src/dag.rs#L505-L553,L607-L632,L1019-L1127), [cmd/sheet.rs](../../../../crates/engine/src/cmd/sheet.rs#L1433-L1472) | `PASS` |

Cross-references against all ledger findings in [full-code-audit-ledger-2026-11-22.md](../full-code-audit-ledger-2026-11-22.md) (`FA-SEC01`–`FA-SEC23`) confirmed zero unreferenced overlap and accurate attribution in `Cross-Ref Only` seams.

---

## Per-Finding Verdicts

### `FA-X2-01` — `CONFIRMED`
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (silent data corruption and silent un-exclusion of predicate-excluded rows on every live DAG refresh without errors or warnings).
- **Evidence:**
  - [cmd/table/import.rs](../../../../crates/engine/src/cmd/table/import.rs#L345-L351) (`run_paste_link`) populates `ImportPipelineOptions { baseline: resolve_baseline_adjustment(p.baseline)?, ..., exclude_row_predicate: p.exclude_row_predicate, ... }`.
  - [transform/src/import.rs](../../../../crates/transform/src/import.rs#L386-L392) runs `apply_grid_baseline` (`L443-L478`) and `apply_row_exclusion_predicate` (`L526-L547`) only when `opts.baseline` / `opts.exclude_row_predicate` are `Some(...)`.
  - [links.rs](../../../../crates/doc/src/links.rs#L106-L123,L157-L177) confirms neither `ImportFilterSpec` nor `ImportLinkSpec` stores `baseline` or `exclude_row_predicate` inside `LinkedDataRegion`.
  - [dag.rs](../../../../crates/engine/src/dag.rs#L858-L866) (`refresh_single_region`) reconstructs `ImportPipelineOptions` with hardcoded `baseline: None`, `exclude_coords: Vec::new()`, and `exclude_row_predicate: None` after `clear_cell_range` (`L867`).

### `FA-X2-02` — `CONFIRMED`
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent schema flip and truncation to initial `ResultTab` bounds on live DAG refresh).
- **Evidence:**
  - [cmd/table/import.rs](../../../../crates/engine/src/cmd/table/import.rs#L54-L59) (`resolve_initial_link_source_payload`) handles `InternalLinkKind::AnalyzableResultSheet | InternalLinkKind::TerminalResultSheet` uniformly by returning `OwnedSourcePayload::Result(src_rs)`.
  - [cmd/table/import.rs](../../../../crates/engine/src/cmd/table/import.rs#L234-L242) (`default_full_source_range`) computes `spec.source.range` from `select_result_tab(res, tab_id)?` (`tab.rows` dimensions).
  - [transform/src/import.rs](../../../../crates/transform/src/import.rs#L233-L263) (`extract_stage1`) extracts `res.tabs.first()` when `tab_id` is `None`.
  - [dag.rs](../../../../crates/engine/src/dag.rs#L777-L783) (`resolve_refresh_source`) diverts `*kind == InternalLinkKind::AnalyzableResultSheet && tab_id.is_none()` to `src_rs.analyzable_data` while `make_refresh_spec` (`L665-L670`) preserves the `tab.rows`-derived `spec.source` rectangle.

### `FA-X2-03` — `CONFIRMED`
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (aborts calibration curve fitting and wipes `ResultSheet` whenever unknown rows with `X = Empty` lack `SD`/`N` or enter `Mean = 0.0` under `MeanCvN`; distinct from non-moment omission in `FA-SEC07-12`).
- **Evidence:**
  - [fit/src/types.rs](../../../../crates/fit/src/types.rs#L395-L423) (`populate_xy_column`) calls `xy.summarized_y_point(row, ds_idx)` (`L405-L406`) unconditionally before checking `(None, Some(pt)) if x_blank => series.unknowns_y.push((row, pt.mean).into())` (`L420`), discarding `pt.sd` and `pt.n`.
  - [doc/src/cell.rs](../../../../crates/doc/src/cell.rs#L321-L357) (`SummarizedPoint::reconstruct`) returns `Err(MissingSpread)` if `subcells.get(1)` is missing or `mean == 0.0` in `MeanCvN` (`L331, L343-L345`), and `Err(MissingSampleSize)` if `N` is blank (`L335`).
  - [dag.rs](../../../../crates/engine/src/dag.rs#L1066,L1122-L1125) clears the entire `ResultSheet` via `clear_failed_result_sheet` on any `FitError`.
  - [chart/src/xy.rs](../../../../crates/chart/src/xy.rs#L1815,L1835-L1848) skips `X = Empty` rows via `extract_xy_x_row`, continuing to plot standards without warning.

### `FA-X2-04` — `CONFIRMED`
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (chart averages replicate `X` / summarizes `X` error subcolumns while downstream fits, transforms, and statistics compute against `X[0]` only or drop rows whose `X[0]` is empty/excluded; distinct from `FA-SEC07-08`).
- **Evidence:**
  - [chart/src/xy.rs](../../../../crates/chart/src/xy.rs#L1802-L1805,L1842-L1847) branches on `XColumnMode::Numbers` with `RawReplicates(_)` or `is_pre_summarized()` and calls `summarize_subcells` over all active `X` subcells.
  - [archetypes.rs](../../../../crates/doc/src/archetypes.rs#L334-L350) (`XyTable::x_value_at`) ignores `subcolumn_mode` and indexes only `subs.first()`.
  - [fit/src/types.rs](../../../../crates/fit/src/types.rs#L384-L396) discards `x_sd` via `let _ = x_sd;` (`L393`) and calls `xy.x_value_at(row)` (`L396`).
  - Verified downstream callers of `xy.x_value_at`: [fit/src/linear.rs](../../../../crates/fit/src/linear.rs#L1127-L1133) (`deming_regression`), [transform/src/curve.rs](../../../../crates/transform/src/curve.rs#L145,L166,L218,L798), [formula/eval.rs](../../../../crates/transform/src/formula/eval.rs#L483), [stats/src/regression.rs](../../../../crates/stats/src/regression.rs#L1509,L1825), and [stats/src/column.rs](../../../../crates/stats/src/column.rs#L1833). (*Precision note:* [stats/src/two_group.rs](../../../../crates/stats/src/two_group.rs#L2512) calls `xy.x_value_at(r)` solely to construct row-label text `"Row {}"` / `format!("{x}")` while comparing `Y` data sets.)

### `FA-X2-05` — `CONFIRMED`
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (same pre-summarized `ColumnTable` produces contradictory means/SDs, `%CV == 0` / negative-spread validation, and `n = 1` acceptance across Column Descriptives, Unpaired *t*-test, One-Way ANOVA, and Column Charts; distinct from `FA-SEC06-01`, `FA-SEC09-05`, and `FA-SEC09-08`).
- **Evidence:**
  - [stats/src/column.rs](../../../../crates/stats/src/column.rs#L461-L498,L570-L585) and [stats/src/two_group.rs](../../../../crates/stats/src/two_group.rs#L1681-L1707) collect all rows via `col_tab.summarized_point` (`SummarizedPoint::reconstruct` in [cell.rs](../../../../crates/doc/src/cell.rs#L321-L357)) and pool them via `summarize_pre_summarized_points` (`column.rs#L461-L498`), accepting `n = 1` rows when pooled total `N >= 2` and rejecting negative spread or `%CV` at `mean == 0.0`. (*Citation precision note:* `summarize_pre_summarized_points` is defined in `column.rs:461-498`, whereas `two_group.rs:470-498` holds `welch_t_test_from_summary` and `one_sample_t_test_from_summary`.)
  - [anova/sheet/one_way.rs](../../../../crates/stats/src/anova/sheet/one_way.rs#L953-L999) bypasses `summarized_point`, `break`s on the first active row (`L981`), lacks `err >= 0.0` validation (`L963-L995`), computes `(err / 100.0) * mean.abs()` even when `mean == 0.0` (`L993`), and forwards single-row summaries to [anova/one_way.rs](../../../../crates/stats/src/anova/one_way.rs#L157-L186,L252) (where `unequal_variance_one_way_anova` rejects `g.n < 2.0` per group).
  - [chart/src/column_grouped.rs](../../../../crates/chart/src/column_grouped.rs#L762-L776) (`extract_column_presummarized`) also returns on the first matching row (`L772`) without pooling subsequent rows.

### `FA-X2-06` — `CONFIRMED`
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silently hides valid points / unhides excluded artifact points across multi-sheet graphs, `InsertRows` pastes, CSV imports, and DAG live refreshes; distinct from `FA-SEC12-01`).
- **Evidence:**
  - [doc/src/graph.rs](../../../../crates/doc/src/graph.rs#L832-L839) documents `PointStyleOverride::series_index` as `Zero-based index into GraphSpec::series`.
  - [cmd/helpers.rs](../../../../crates/engine/src/cmd/helpers.rs#L762-L768,L912-L920) (`point_override_bounds_parts`, `prune_shortened_series_overrides`) and [cmd/graph.rs](../../../../crates/engine/src/cmd/graph.rs#L1730) look up `GraphSpec::series` by `s.data_set_index == series_index` and collapse multi-sheet collisions in `HashMap<usize, &SheetId>`.
  - [chart/src/xy.rs](../../../../crates/chart/src/xy.rs#L2228,L2433,L2457), [chart/src/column_grouped.rs](../../../../crates/chart/src/column_grouped.rs#L520,L1158,L1511,L1815), and [chart/src/multivariate_layout.rs](../../../../crates/chart/src/multivariate_layout.rs#L251,L2236,L2501,L3300,L3464) all pass data-table column indices (`col_index` / `source.col` / `data_col` / `enc.y_column`) rather than `GraphSpec::series` indices.
  - [cmd/table.rs](../../../../crates/engine/src/cmd/table.rs#L2569-L2626) (`run_paste`), [cmd/table/import.rs](../../../../crates/engine/src/cmd/table/import.rs#L597-L656) (`run_import_csv`), and [dag.rs](../../../../crates/engine/src/dag.rs#L847-L900) (`refresh_single_region`) omit both `snapshot_active_series_lengths`/`prune_shortened_series_overrides` and `InsertRows` `point_index` shifting, while `place_grid` ([transform/src/import.rs](../../../../crates/transform/src/import.rs#L590-L598)) shifts `archetype` rows and `exclusion_notes`. (*Citation precision note:* `table.rs:1440` is `run_set_x_cells` (`L1435-L1455`), whereas `run_exclude_cells` (`L1893`) does not call `prune_shortened_series_overrides`.)

### `FA-X2-07` — `CONFIRMED`
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (`table.exportCsv` and `.curvebook` / `.curvebook.d` archive `sheets/*.csv` sidecars drop the entire `X` variable and all `X` exclusion `*` markers on `GeneratedSequence` `XyTable`s; distinct from `FA-SEC02-10` and `FA-SEC04-06`).
- **Evidence:**
  - [archetypes.rs](../../../../crates/doc/src/archetypes.rs#L227-L233,L375-L391) returns `0` for `XColumnMode::GeneratedSequence { .. }` in `subcolumn_count()`, while `materialized_x_cell_at(row)` explicitly reconstructs `Cell { value: CellValue::Number(v), excluded }`.
  - [csv.rs](../../../../crates/format/src/csv.rs#L485-L502) (`export_xy_rows`) loops `for s in 0..x_subs` where `x_subs = t.x_mode.subcolumn_count()`, executing zero iterations for both header (`L486-L489`) and data fields (`L497-L502`), rendering the `t.x_value_at(r)` fallback (`L499`) unreachable.
  - [archive.rs](../../../../crates/format/src/archive.rs#L908-L920) (`build_archive_entries`) writes `export_datasheet_to_csv` into `sheets/*.csv` alongside `document.json`, and [cmd/table.rs](../../../../crates/engine/src/cmd/table.rs#L2654-L2678) (`run_export_csv`) exposes `export_datasheet_to_csv` directly.

### `FA-X2-08` — `CONFIRMED`
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (engine-side CI backfill breaks when combined with non-default `ImportFilterSpec` options, `skip_contiguous_blanks`, or `transpose`, misplacing or blanking `[lo, hi]` interval bounds; extends beyond L2 `FA-SEC03-15` to the broken `transform <-> engine` backfill contract).
- **Evidence:**
  - [transform/src/import.rs](../../../../crates/transform/src/import.rs#L278-L287) maps `ResultCell::ConfidenceInterval { .. } => Cell::empty()` before `apply_stage2_filter` (`L341-L401`), `collapse_contiguous_blank_rows` (`L176-L178`), `transpose_cell_grid` (`L179-L188`), and `place_grid` (`L563-L615`).
  - [dag.rs](../../../../crates/engine/src/dag.rs#L807-L845) (`backfill_confidence_interval_cells`, invoked by [cmd/table/import.rs](../../../../crates/engine/src/cmd/table/import.rs#L368-L372), [dag.rs](../../../../crates/engine/src/dag.rs#L888-L889), and [cmd/sheet.rs](../../../../crates/engine/src/cmd/sheet.rs#L1985)) exits early whenever `spec.placement.transpose` is `true` (`L814-L816`) and otherwise maps unfiltered `tab.rows[range.start.row..=end_r]` and `range.start.col..=end_c` linearly by `grid_row`/`grid_col` (`L822-L844`) without applying `row_window`, `col_window`, `decimate_every_k`, `unstack_group_column`, or `skip_contiguous_blanks`.
