# SEC-X2 Cross-Cutting Audit Report — End-to-End Scientific Data Fidelity

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Scope:** `SEC-X2` — End-to-End Scientific Data Fidelity across `crates/doc` -> `crates/transform` -> `crates/stats` & `crates/fit` -> `crates/engine` (`dag.rs`, `methods_report.rs`, `cmd/{analysis,table,table/import,sheet,graph}.rs`) -> `crates/chart` / `svg` / `pdf` / `render` -> `crates/format` (`.curvebook`, `.curvebook.d`, WAL, CSV, `.pzfx`, `.prism`)
- **Date:** `2026-11-22`
- **Reviewer:** `rev-x2` (report-only cross-crate seam trace; baseline SHA verified on `HEAD`)
- **Summary Counts:** `S1 = 1`, `S2 = 6`, `S3 = 1`, `S4 = 0` (Total cross-cutting seam findings: `8`; `13` end-to-end seams traced across Families 1–4)

> [!NOTE]
> Single-crate defects already recorded in `FA-SEC01`–`FA-SEC23` are cross-referenced in the **Data Fidelity Seam Matrix** and `Related` fields rather than duplicated as new `FA-X2-*` IDs. `FA-X2-01` through `FA-X2-08` capture genuine cross-crate seam gaps and compounding data-fidelity interactions discovered during this trace.

---

## Data Fidelity Seam Matrix (`13` End-to-End Seams Traced)

### Family 1: Cell Exclusions (`CellExclusion`, `CellExclusionNote`, Shift / Delete / Undo Alignment)

| # | Data Condition | Pipeline Stages Traced | Verified Guards | Findings / Status |
|---:|---|---|---|---|
| 1 | Manual cell exclusion & exclusion notes (`table.excludeCells`, `table.excludeCellsWithReason`, `table.setXCells`, `table.setCell`) | `crates/doc/src/links.rs:189-198` -> `crates/engine/src/cmd/table.rs:1195-1220` -> `crates/engine/src/dag.rs:1019-1127` -> `crates/engine/src/methods_report.rs:2060-2084` -> `crates/chart/src/xy.rs:1902-1918` | `CellExclusionNote` tracks `(coord, reason, source)`; `methods_report` counts `count_active_exclusion_notes` / `count_excluded_cells` consistently (`methods_report.rs:2060-2084`); ROUT `eliminated_outliers` suppresses eliminated points and remaps cleanly on sheet clones (`cmd/sheet.rs:1430-1495`, `chart/src/xy.rs:1902-1918`). | **Cross-Ref Only**: `FA-SEC02-02` (`TableArchetype::shift_rows_down` leaves `exclusion_notes` and `linked_regions` unshifted), `FA-SEC02-10` (`set_x_or_time_excluded` on `GeneratedSequence`), `FA-X2-07` (`GeneratedSequence` `x_cells` exclusions dropped in CSV / archive sidecars). |
| 2 | Live-linked `exclude_row_predicate` & `baseline` normalization (`table.pasteLink`) | `crates/engine/src/cmd/table/import.rs:345-391` -> `crates/transform/src/import.rs:341-401, 443-480, 526-547` -> `crates/doc/src/links.rs:157-177` -> `crates/engine/src/dag.rs:847-900` | Initial `table.pasteLink` applies `apply_grid_baseline` and `apply_row_exclusion_predicate` (`transform/src/import.rs:386-392`) inside `ImportPipelineOptions` (`import.rs:345-351`) and validates DAG acyclicity (`dag::topological_order` at `import.rs:376`). | **`FA-X2-01` (`S1`)**: `ImportLinkSpec` omits `baseline` and `exclude_row_predicate`, so `dag::refresh_single_region` hardcodes `baseline: None` and `exclude_row_predicate: None` (`dag.rs:859-865`), silently reverting baseline transforms and un-excluding predicate-excluded rows on every live refresh. |
| 3 | Excluded cells across derived transforms, calculated variables, archetype conversion, & randomization | `crates/transform/src/formula/{eval.rs:36, dag.rs:132-144}` -> `ops/{math.rs:513-744, normalize_baseline.rs:620-704, reshape_select.rs:880-1030}` -> `randomize.rs:260-277` -> `archetype_convert.rs:839, 936, 1837` -> `dag.rs:607-632` | `FormulaValue::from_cell` treats excluded cells as `Missing` (`eval.rs:36`) and `evaluate_calculated_variables` preserves `prev_excluded` across DAG recomputation (`formula/dag.rs:132-144`, `dag.rs:625`); `randomize` & `random_sample_rows` preserve pairing and remap `exclusion_notes` (`randomize.rs:260-277`, `cmd/table.rs:4773-4966`); `ops::math` propagates exclusions/bounds swaps on `MeanLowHigh`/`UpperLowerLimits`/`MedianIqr` (`ops/math.rs:513-744`); `place_grid` on `InsertRows` shifts cells and `exclusion_notes` (`transform/src/import.rs:590-597`). | **Cross-Ref Only**: `FA-SEC03-01` (`execute_select_with_derived` missing axis column shift), `FA-SEC03-02` (`execute_transpose` drops `exclusion_notes`), `FA-SEC03-04` (in-place transform mutation on `Contingency`), `FA-SEC03-09` (`pivot_tidy_long_to_datasheet` drops `x_excluded`). |
| 4 | Excluded cells across CSV export / re-import & `.curvebook` / `.curvebook.d` / WAL / `.pzfx` / `.prism` round-trips | `crates/format/src/{archive.rs:898-920, csv.rs:479-507, pzfx, prism}` -> `crates/engine/src/cmd/{table.rs:2654-2664, project}` | Primary `document.json` inside `.curvebook` and `.curvebook.d` archives serializes `Cell.excluded`, `x_cells`, and `exclusion_notes` losslessly (`archive.rs:898-902`); `export_datasheet_to_csv` marks excluded numeric cells with trailing `*` markers parsed back on CSV import. | **`FA-X2-07` (`S2`)** (`GeneratedSequence` omits X values and `x_cells` exclusions from `table.exportCsv` and `sheets/*.csv` archive sidecars); cross-ref `FA-SEC04-06` (`apply_csv_import_link` corrupts `GeneratedSequence` rectangularity) and `FA-SEC04-09` (excluded `CellValue::Text` `*` token mangled on CSV round-trip). |
| 5 | Per-point visual hiding / styling across row shifts, deletions, pastes, & multi-sheet graphs (`PointStyleOverride`) | `crates/doc/src/graph.rs:832-861, 1258` -> `crates/engine/src/cmd/{graph.rs:1405-1452, 1591-1784, table.rs:1201-2597, helpers.rs:719-922}` -> `crates/chart/src/{lib.rs:88-99, xy.rs:2228, 2433, 2457, column_grouped.rs:520, 1158, 1511, 1815, multivariate_layout.rs:251, 2236, 2501, 3300, 3464}` | `validate_point_override_in_bounds` (`helpers.rs:822-845`), `prune_out_of_bounds_point_overrides` (`helpers.rs:848-862`, called on project open/undo), `snapshot_active_series_lengths` + `prune_shortened_series_overrides` (`helpers.rs:885-922`, called in `table.{setCell,setCells,excludeCells,clearCells}` at `table.rs:1201,1207,1319,1440,2360`); `resolve_overridden_marker_for_cell` applies `hidden` / style overrides uniformly (`chart/src/lib.rs:88-99`). | **`FA-X2-06` (`S2`)**: `PointStyleOverride.series_index` collides across multi-sheet graphs (`doc` contract vs `helpers.rs`/`chart` `data_set_index` lookup), and `table.paste` / `table.importCsv` / `dag::refresh_single_region` omit both `InsertRows` index shifting and shortened-series pruning; cross-ref `FA-SEC12-01` (`table.convertArchetype` corrupts `point_overrides`). |

### Family 2: Missing / Blank, `NaN`, `±Inf`, & Non-Numeric Text Cells, X vs Y Pairing, & `ResultSheet` Round-Trips

| # | Data Condition | Pipeline Stages Traced | Verified Guards | Findings / Status |
|---:|---|---|---|---|
| 6 | Non-finite (`NaN`, `±Inf`), missing, and non-numeric text cells in numeric columns | `crates/doc/src/cell.rs` -> `crates/transform` -> `crates/stats` / `crates/fit` -> `crates/engine/src/dag.rs:1188-1300` -> `crates/format/src/export.rs:40, 74, 98, 386-431` -> `crates/render` / `svg` / `pdf` | `Cell::active_number` filters blank, excluded, text, and non-finite inputs; `dag::validate_result_sheet_finite` (`dag.rs:1188-1300`) blocks `NaN`/`±Inf` in completed `ResultSheet`s before caching; `format::export::validate_result_floats` (`export.rs:386-431`) checks CSV/JSON result exports on `try_export_result_*`; `chart`/`render`/`svg`/`pdf` drop non-finite coordinates. | **Cross-Ref Only**: `FA-SEC04-04` (`FormatRegistry` CSV/TSV & `export.rs:40, 74` failed-result guard bypass), `FA-SEC04-10` (`export_result_sheet_json` skips `sheet.analysis`/`analyzable_data`), `FA-SEC06-07` & `FA-SEC05-11` (Regression/MV/ROC runners skip `sanitize_result_sheet_floats`), `FA-SEC05-07` & `FA-SEC06-11` (missing ignored-text footnotes). |
| 7 | X vs Y pairing & blank vs non-numeric/excluded handling for unknown sample interpolation (`unknowns_y` / `unknowns_x`) | `crates/doc/src/archetypes.rs:334-425` -> `crates/doc/src/cell.rs:315-358` -> `crates/fit/src/types.rs:380-430` -> `crates/engine/src/dag.rs:1019-1127` -> `crates/chart/src/xy.rs:1835-1848` | `x_row_is_blank_unknown` and `y_cells_are_blank_unknown` isolate truly blank cells (`X = Empty` with valid Y -> `unknowns_y`; valid X with `Y = Empty` -> `unknowns_x`) from excluded/text cells (`fit/src/types.rs:380, 400`); `chart::xy::extract_xy_x_row` skips blank-X rows when drawing data series (`xy.rs:1835-1848`). | **`FA-X2-03` (`S2`)**: Under parametric pre-summarized Y modes (`MeanSdN`, `MeanSemN`, `MeanCvN`), unknown-sample rows (`X = Empty`) call `xy.summarized_y_point` (`doc/src/cell.rs:321-357`), aborting the entire curve fit if unknown samples enter only `Mean` without `SD`/`N` or have `%CV` with `mean == 0.0` (`fit/src/types.rs:404-421`); cross-ref `FA-SEC07-12` (drops `unknowns_y`/`unknowns_x` on `MeanLowHigh`/`UpperLowerLimits`/`MedianIqr`). |
| 8 | `ResultSheet` -> `DataSheet` linked & extracted round-trips (`AnalyzableResultSheet`, `ResultTab`, `ConfidenceInterval` cells) | `crates/engine/src/cmd/table/import.rs:47-84, 220-244, 345-391` + `cmd/sheet.rs:1960-1986` -> `crates/transform/src/import.rs:278-287, 341-401, 563-627` -> `crates/engine/src/dag.rs:758-900` | Link write-protection (`ensure_range_outside_linked_regions` at `table.rs:2556`, `import.rs:359, 578`) prevents manual corruption of linked regions; `format_ci_bound_text` formats finite/open CI bounds safely when coordinates align (`dag.rs:795-803`). | **`FA-X2-02` (`S2`)**: Linking `AnalyzableResultSheet` with `tab_id: None` pastes `ResultTab` initially (`import.rs:54-59, 234-242`) while `dag::resolve_refresh_source` swaps to `analyzable_data` truncated to the initial tab's `range` (`dag.rs:777-783`). **`FA-X2-08` (`S3`)**: `result_cell_to_cell` drops `ConfidenceInterval` (`transform/src/import.rs:278-287`) and `dag::backfill_confidence_interval_cells` ignores `ImportFilterSpec` and `skip_contiguous_blanks` (`dag.rs:807-845`), misplacing or omitting CI text (extends `FA-SEC03-15`). |

### Family 3: Replicate Subcolumns (`RawReplicates`) & Summarized Subcolumn Modes (`MeanSdN`, `MeanSemN`, `MeanCvN`, `n=1`, `MeanLowHigh`, `UpperLowerLimits`, `MedianIqr`, Asymmetric Errors)

| # | Data Condition | Pipeline Stages Traced | Verified Guards | Findings / Status |
|---:|---|---|---|---|
| 9 | `XyTable` `X` replicate (`RawReplicates`) & pre-summarized (`MeanSdN`/`Sem`/`Cv`/`Limits`/`MedianIqr`) subcolumns | `crates/doc/src/archetypes.rs:334-425` -> `crates/transform/{curve.rs:145, 166, 218, 798, formula/eval.rs:483}` -> `crates/stats/{regression.rs:1509, 1825, column.rs:1833, two_group.rs:2512}` -> `crates/fit/{types.rs:384-429, linear.rs:1127-1133}` -> `crates/chart/src/xy.rs:1835-1848` | `chart::xy::extract_xy_x_row` averages `RawReplicates` X subcells and preserves X error bars for pre-summarized numeric X columns (`xy.rs:1842-1844`); `doc::XyTable::summarized_x_point` provides parametric X moment reconstruction (`archetypes.rs:411-419`). | **`FA-X2-04` (`S2`)**: Every computation consumer outside `chart` calls `xy.x_value_at(row)` (`archetypes.rs:340`), reading only `x_cells[row][0]` and silently dropping X replicates `1..` plus X spread validation/propagation across `fit::types::populate_xy_column` (`types.rs:384-396`, including discarded `let _ = x_sd;`), `deming_regression` (`linear.rs:1127-1133`), `transform::curve`, `stats::{regression, column, two_group}`, and `transform::formula`. Cross-ref `FA-SEC07-08`. |
| 10 | `ColumnTable` pre-summarized modes (`MeanSdN`, `MeanSemN`, `MeanCvN`) & multi-row pooling / `n=1` rows | `crates/doc/src/cell.rs:315-358` -> `crates/stats/{column.rs:574-583, two_group.rs:470-498, 1687-1696, anova/one_way.rs:180, 252, anova/sheet/one_way.rs:953-999}` -> `crates/chart/src/column_grouped.rs:762-776` | `stats::column` (`column.rs:574-583`) and `stats::two_group` (`two_group.rs:1687-1696, 470-498`) use `SummarizedPoint::reconstruct` (`cell.rs:321-357`) and pool multiple summary rows per column (`n=1` rows accepted when pooled total `N >= 2`, negative spread and `%CV` at `mean == 0.0` rejected). | **`FA-X2-05` (`S2`)**: `stats::anova::sheet::one_way` (`one_way.rs:953-997`) bypasses `summarized_point`, `break`s on the first active row (`:981`), accepts `%CV` at `mean == 0.0` (`:993`) and negative spread, and rejects `n=1` unpooled (`anova/one_way.rs:180, 252`), while `chart::column_grouped::extract_column_presummarized` also returns only the first active row (`column_grouped.rs:762-776`). Cross-ref `FA-SEC06-01`, `FA-SEC09-05`, `FA-SEC09-08`. |
| 11 | `RawReplicates` & pre-summarized `Y` subcolumns across fit weighting, error propagation, `n=1` CI, asymmetric errors, & chart rendering | `crates/doc` -> `crates/transform/src/ops/{math.rs:513-744, normalize_baseline.rs:620-704}` -> `crates/stats` -> `crates/fit` -> `crates/chart/src/xy.rs:241-267, 363-442` | `chart::xy::emit_directional_error_bar` renders asymmetric `UpperLowerLimits`/`MeanLowHigh`/`MedianIqr` bounds accurately with log lower-limit clipping (`xy.rs:363-442`) and omits 95% CI bars cleanly with a warning when `N == 1` (`xy.rs:256-267`); `ops::math` & `normalize_baseline` propagate transforms and swap lower/upper bounds on sign inversion/reciprocal (`ops/math.rs:513-744`). | **Cross-Ref Only**: `FA-SEC07-05` (`uses_replicate_summary_moments` drops `N` and `(N-1)*SD^2` weighting across `1/|X|`, `1/X^2`, `1/Y^2`, `1/|Y|`), `FA-SEC07-12` (non-moment unknown interpolation dropped), `FA-SEC06-02` (MV drops replicates / accepts pre-summarized), `FA-SEC06-07` (Passing–Bablok drops replicates), `FA-SEC05-07` (XY Bland–Altman drops replicates in `Y[A] vs Y[B]`), `FA-SEC09-02` (`CategoricalSummary` 95% CI & truncated error domains). |

### Family 4: Paired / Matched Designs, Ties, Row Ordering, Weights, Dynamic DAG Live-Edit Recomputation, & `MethodsReport`

| # | Data Condition | Pipeline Stages Traced | Verified Guards | Findings / Status |
|---:|---|---|---|---|
| 12 | Paired/matched designs, randomization/sampling, row ordering, & tied coordinates | `crates/transform/src/{randomize.rs:260-277, curve.rs:755-817}` -> `crates/stats` -> `crates/fit` -> `crates/engine/src/cmd/table.rs:4773-4966` | `randomize_subjects_to_groups` and `random_sample_rows` preserve aligned row pairing and remap `exclusion_notes` (`randomize.rs:260-277`, `table.rs:4773-4966`); `transform::curve` pools tied X values deterministically before integration/differentiation (`curve.rs:755-817`). | **Cross-Ref Only**: `FA-SEC03-05` (`select_log_spaced_indices` breaks on descending/non-monotonic X), `FA-SEC03-06` (`lowess_point` unaveraged ties at `>= q`), `FA-SEC05-06` (`WilcoxonSignedRank` Pratt zero-handling mismatch), `FA-SEC06-01` (RM ANOVA accepts pre-summarized tables), `FA-SEC07-08` (Deming `n=3` aborts on duplicate `(x, y)`). |
| 13 | Dynamic DAG live-edit recomputation, circularity checks, & `methods_report` consistency | `crates/engine/src/{dag.rs:607-632, 758-900, 1019-1300, methods_report.rs:2060-2084, cmd/table/import.rs:376-390}` | `recompute_result_sheet` clears stale cached results cleanly on upstream error (`dag.rs:1066, 1122-1125`) and validates finite result values (`dag.rs:1188-1300`); `methods_report` counts exclusions identically to sheet exclusion predicates (`methods_report.rs:2060-2084`); `table.pasteLink` verifies `dag::topological_order` with atomic rollback (`import.rs:376-390`). | **Impacted by `FA-X2-01`, `FA-X2-02`, `FA-X2-03`, `FA-X2-06`, `FA-X2-08`**; cross-ref `FA-SEC02-03` (`upstream_dependencies` strips self-refs), `FA-SEC02-08` (`ResultSheet::lookup_scalar` returns stale `Dirty` scalars), `FA-SEC11-01` (`describe_bland_altman` X-vs-Y[0] mislabeling), `FA-SEC11-05` (checkpoint WAL replay divergence), `FA-SEC12-04` (`setCalculatedVariable` overwrites linked columns). |

---

## Cross-Cutting Findings (`S1` -> `S4`)

### `FA-X2-01` · `S1` · `table.pasteLink` silently drops `baseline` normalization and `exclude_row_predicate` on DAG live-link refresh

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `crates/doc/src/links.rs:157-177` (`ImportLinkSpec`, `LinkedDataRegion`)
  - `crates/engine/src/cmd/table/import.rs:345-351` (`run_paste_link`)
  - `crates/engine/src/dag.rs:858-866` (`refresh_single_region`)
  - `crates/transform/src/import.rs:386-392, 443-480, 526-547` (`apply_stage2_filter`, `apply_grid_baseline`, `apply_row_exclusion_predicate`)
- **Category:** `data-fidelity` / `silent-regression`
- **Defect:** Across the `engine/cmd/table/import` <-> `doc/links` <-> `transform/import` <-> `engine/dag` live-link boundary, `run_paste_link` supports `p.baseline` (e.g., subtract/ratio baseline column adjustment) and `p.exclude_row_predicate` (rule-based row exclusion setting `cell.excluded = true`) via `ImportPipelineOptions` (`crates/engine/src/cmd/table/import.rs:345-351`), applied in Stage 2 of `apply_import_link_pipeline` (`crates/transform/src/import.rs:386-392`). However, `ImportLinkSpec` (`crates/doc/src/links.rs:157-164`) persists only `source`, `filter: ImportFilterSpec`, and `placement: ImportPlacementSpec` inside `LinkedDataRegion` (`crates/doc/src/links.rs:168-177`). On every subsequent upstream edit, file reload, or explicit `project.recalculate`, `dag::refresh_single_region` reconstructs `ImportPipelineOptions` with:
  ```rust
  baseline: None,
  exclude_coords: Vec::new(),
  exclude_row_predicate: None,
  ```
  (`crates/engine/src/dag.rs:859-865`).
- **Failure scenario:** A scientist creates a live paste-link importing an upstream sheet or external CSV with `baseline` normalization (such as subtracting baseline column `0`) and an `exclude_row_predicate` (excluding `QC_FAIL` rows). The initial table values and excluded markers look completely valid. As soon as any upstream cell is edited or the project is recalculated with live links enabled, `dag::refresh_single_region` overwrites the linked region with raw un-normalized numbers and clears all predicate-driven exclusions, silently corrupting every downstream statistical analysis, curve fit, and publication graph with zero warning.
- **Evidence:**
  - `crates/engine/src/cmd/table/import.rs:345-351`: `run_paste_link` populates `baseline: resolve_baseline_adjustment(p.baseline)?` and `exclude_row_predicate: p.exclude_row_predicate`.
  - `crates/doc/src/links.rs:157-164`: `ImportLinkSpec` contains only `source`, `filter`, and `placement` fields — neither `baseline` nor `exclude_row_predicate` is stored on the persisted link specification.
  - `crates/engine/src/dag.rs:859-865`: `refresh_single_region` passes `baseline: None` and `exclude_row_predicate: None` unconditionally to `apply_import_link_pipeline`.
  - `crates/transform/src/import.rs:386-392`: both `apply_grid_baseline` and `apply_row_exclusion_predicate` execute only when those `ImportPipelineOptions` fields are `Some(...)`.
- **Suggested fix:** Persist normalized `baseline` (`Option<(usize, BaselineOp)>` or `BaselineSpec`) and `exclude_row_predicate` on `ImportLinkSpec` (with `#[serde(default)]` backward compatibility in `crates/doc/src/links.rs`), populate them in `build_paste_link_spec`, and forward them from `region.spec` into `ImportPipelineOptions` inside `dag::refresh_single_region`.
- **Related:** Connected to `FA-X2-02` and `FA-X2-08` on the same `table.pasteLink` <-> `dag::refresh_single_region` contract boundary.

---

### `FA-X2-02` · `S2` · Linking an `AnalyzableResultSheet` with `tab_id: None` flips source schema between initial `table.pasteLink` and DAG refresh

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `crates/engine/src/cmd/table/import.rs:47-65, 220-244` (`resolve_initial_link_source_payload`, `default_full_source_range`)
  - `crates/engine/src/dag.rs:758-784` (`resolve_refresh_source`)
- **Category:** `data-fidelity` / `contract-drift`
- **Defect:** Across the initial-import vs DAG-refresh seam for `LinkSource::InternalSheet { sheet_id, kind: InternalLinkKind::AnalyzableResultSheet, tab_id: None, .. }`, initial paste and refresh resolve fundamentally different source tables:
  1. `resolve_initial_link_source_payload` (`crates/engine/src/cmd/table/import.rs:54-59`) matches `InternalLinkKind::AnalyzableResultSheet | InternalLinkKind::TerminalResultSheet` uniformly and supplies `OwnedSourcePayload::Result(src_rs)`, importing the summary `ResultTab` (`tabs[0]`), while `default_full_source_range` (`crates/engine/src/cmd/table/import.rs:234-242`) sizes `spec.source.range` strictly to `tab.rows` dimensions.
  2. `dag::resolve_refresh_source` (`crates/engine/src/dag.rs:777-783`) checks `if *kind == InternalLinkKind::AnalyzableResultSheet && tab_id.is_none() && let Some(analyzable) = src_rs.analyzable_data.as_ref()` and switches the payload to `ResolvedRefreshSource::InternalData { sheet: analyzable, propagate_titles: false }`.
- **Failure scenario:** A user live-links a transformed `ResultSheet` (whose `analyzable_data` contains the full numeric output table and whose `tabs[0]` holds a summary or metadata tab) into a downstream `DataSheet` without passing `tab_id`. Initially the target receives `tabs[0]` (for example 2 summary columns x 5 rows) and stores `range = 0..5 x 0..2`. On the very next DAG refresh, `resolve_refresh_source` switches sources to `src_rs.analyzable_data`, overwriting the target `DataSheet` with a completely different table truncated/cropped to `0..5 x 0..2` of `analyzable_data` (or failing refresh if archetypes/columns mismatch).
- **Evidence:**
  - `crates/engine/src/cmd/table/import.rs:54-59`: `InternalLinkKind::AnalyzableResultSheet` returns `OwnedSourcePayload::Result` unconditionally without inspecting `tab_id.is_none()` or `analyzable_data`.
  - `crates/engine/src/cmd/table/import.rs:234-242`: default `range` is computed from `select_result_tab(res, tab_id)?` (`tab.rows`).
  - `crates/engine/src/dag.rs:777-783`: `resolve_refresh_source` diverts `AnalyzableResultSheet` + `tab_id.is_none()` to `src_rs.analyzable_data`.
- **Suggested fix:** Unify source resolution in a shared helper (or extract `analyzable_data` explicitly via a dedicated `InternalLinkKind` / `source_mode` flag) so both `resolve_initial_link_source_payload` / `default_full_source_range` and `dag::resolve_refresh_source` target the exact same table (`analyzable_data` vs `ResultTab`).
- **Related:** `FA-SEC03-11` (`import.rs` bounds/unstacking), `FA-X2-01`.

---

### `FA-X2-03` · `S2` · Unknown sample interpolation (`X = Empty`) aborts entire regression fit on parametric pre-summarized XY tables when `SD` / `N` are omitted

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `crates/doc/src/cell.rs:315-358` (`SummarizedPoint::reconstruct`)
  - `crates/doc/src/archetypes.rs:421-425` (`XyTable::summarized_y_point`)
  - `crates/fit/src/types.rs:380, 396-423` (`populate_xy_column`)
  - `crates/engine/src/dag.rs:1066, 1122-1125` (`recompute_result_sheet`)
  - `crates/chart/src/xy.rs:1815, 1835-1848` (`extract_xy_x_row`)
- **Category:** `data-fidelity` / `usability-breakage`
- **Defect:** Across the `doc` -> `fit` -> `engine/dag` seam for unknown-sample inverse interpolation (standard curves where calibration points occupy `X` and `Y`, and unknown sample readings occupy rows with blank `X` and measured `Y`), `populate_xy_column` enforces strict `SummarizedPoint::reconstruct` validation on `X = Empty` unknown rows whenever `xy.y_subcolumn_mode.supports_parametric_moments()` (`MeanSdN`, `MeanSemN`, `MeanCvN`):
  - `populate_xy_column` (`crates/fit/src/types.rs:404-421`) calls `xy.summarized_y_point(row, ds_idx)?` before checking whether the row is a calibration point (`Some(x)`) or an unknown (`None` with `x_blank`), and then pushes only `pt.mean` (`(row, pt.mean).into()`) into `series.unknowns_y` (`:420`), discarding `pt.sd` and `pt.n`!
  - Meanwhile, `SummarizedPoint::reconstruct` (`crates/doc/src/cell.rs:331-348`) returns `Err(MissingSpread)` if `SD`/`SEM`/`%CV` is blank, `Err(MissingSampleSize)` if `N` is blank, or `Err(MissingSpread)` if `mean == 0.0` under `MeanCvN`.
- **Failure scenario:** An assay scientist builds a standard curve on an `XyTable` configured as `MeanSdN` (or `MeanSemN` / `MeanCvN`) where reference standards have `(X, Mean, SD, N)` populated and unknown test samples at the bottom have `X` left blank and only `Mean` entered (single assay readout, or `Mean = 0.0` blank control in `MeanCvN`). Even though `unknowns_y` only consumes the `Y` mean (`pt.mean`), `fit::types::populate_xy_column` aborts with `FitError::InvalidParameter { name: "pre_summarized_y", reason: ... }`, and `dag::recompute_result_sheet` wipes the calibration curve and transitions the entire `ResultSheet` to `ResultStatus::Failed` (while `chart::xy` still plots the standards cleanly because `extract_xy_x_row` skips `X = Empty` rows).
- **Evidence:**
  - `crates/fit/src/types.rs:404-421`: `let pt_opt = xy.summarized_y_point(row, ds_idx).map_err(...)?;` runs unconditionally before matching `(None, Some(pt)) if x_blank => series.unknowns_y.push((row, pt.mean).into())`.
  - `crates/doc/src/cell.rs:331-348`: requires active finite spread >= 0, integer `N >= 1`, and non-zero `mean` for `MeanCvN`.
  - `crates/engine/src/dag.rs:1066, 1122-1125`: any `FitError` fails the entire `ResultSheet` via `clear_failed_result_sheet`.
- **Suggested fix:** For unknown-`Y` rows (`x_blank && xy.summarized_x_point`/primary `X` empty), read `subcells.first().and_then(Cell::active_number)` directly for `unknowns_y` (or validate optional spread only when spread-based unknown CI is modeled), matching how calibration rows and `RawReplicates` treat single unknown measurements without requiring `(SD, N)` metadata that `series.unknowns_y` discards anyway.
- **Related:** Complements `FA-SEC07-12` (which covers the opposite failure in `crates/fit/src/types.rs:424-435`: silently dropping `unknowns_y` and `unknowns_x` altogether under non-moment modes `MeanLowHigh`, `UpperLowerLimits`, `MedianIqr`).

---

### `FA-X2-04` · `S2` · Replicate and pre-summarized `X` subcolumns are collapsed to `X[0]` in `doc`, `transform`, `stats`, and `fit` while `chart` averages all `X` subcolumns

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `crates/doc/src/archetypes.rs:334-345, 411-419` (`XyTable::x_value_at`, `XyTable::summarized_x_point`)
  - `crates/chart/src/xy.rs:1835-1848` (`extract_xy_x_row`)
  - `crates/fit/src/types.rs:384-396` (`populate_xy_column`)
  - `crates/fit/src/linear.rs:1127-1133` (`deming_regression`)
  - `crates/transform/src/curve.rs:145, 166, 218, 798` (`xy.x_value_at`)
  - `crates/transform/src/formula/eval.rs:483` (`archetype_x_values`)
  - `crates/stats/src/{regression.rs:1509, 1825, column.rs:1833, two_group.rs:2512}`
- **Category:** `data-fidelity` / `cross-crate-divergence`
- **Defect:** Across the `doc` <-> `transform` / `stats` / `fit` <-> `chart` seam for `XyTable`s with `XColumnMode::Numbers { subcolumn_mode }` set to `RawReplicates(k)` or a pre-summarized mode (`MeanSdN`, `MeanSemN`, `MeanCvN`, `MeanLowHigh`, `UpperLowerLimits`, `MedianIqr`):
  1. `chart::xy::extract_xy_x_row` (`crates/chart/src/xy.rs:1842-1844`) explicitly calls `summarize_subcells(x_submode, cells, SummaryMeasure::Mean, x_err, warnings)`, averaging all active `X` replicates and plotting horizontal error bars from `X` spread subcolumns.
  2. By contrast, `XyTable::x_value_at` (`crates/doc/src/archetypes.rs:340`) reads only `self.x_cells.get(row).and_then(|subs| subs.first()).and_then(Cell::active_number)`, discarding `X` replicates `1..k` and ignoring `X` error/spread/`N` subcolumns (including missing/excluded primary `X[0]` when `X[1..]` are present).
  3. Every downstream computational caller calls `xy.x_value_at(row)`: `fit::types::populate_xy_column` (`types.rs:396`, immediately after computing and discarding `let _ = x_sd;` at `:384-393`), `fit::linear::deming_regression` (`linear.rs:1127-1133`), `transform::curve` AUC/smooth/differentiate/integrate (`curve.rs:145, 166, 218, 798`), `stats::{regression, column, two_group}`, and `transform::formula` (`eval.rs:483`).
- **Failure scenario:** A user enters duplicate/triplicate measured X concentrations (`RawReplicates(3)`) or `MeanSdN` X measurements alongside Y responses. The XY scatter graph correctly plots points at the mean X of each row with horizontal error bars, and any regression overlay derived from `fit` plots the fitted curve against only `X[0]` (or omits rows where `X[0]` is excluded/blank even when `X[1], X[2]` are valid!). Curve AUC, Deming regression, Bland–Altman/Passing–Bablok, and derived formulas all compute against `X[0]` while the graph displays averaged X centers — producing plotted curves visibly offset from the plotted data points with no warning.
- **Evidence:**
  - `crates/chart/src/xy.rs:1842-1844`: `extract_xy_x_row` branches on `(x_submode.is_pre_summarized() || matches!(x_submode, SubcolumnMode::RawReplicates(_)))` and calls `summarize_subcells`.
  - `crates/doc/src/archetypes.rs:340`: `x_value_at` ignores `subcolumn_mode` and indexes `subs.first()`.
  - `crates/fit/src/types.rs:384-396`: `let _ = x_sd;` followed by `let x_opt = xy.x_value_at(row);` discards `summarized_x_point` completely.
  - `rg -n "x_value_at"` confirms `curve.rs:145, 166, 218, 798`, `regression.rs:1509, 1825`, `column.rs:1833`, `two_group.rs:2512`, `formula/eval.rs:483` all rely on `x_value_at`.
- **Suggested fix:** Provide a canonical `XyTable::x_summary_at(row)` (returning mean over active replicates for `RawReplicates`, `summarized_x_point` validation for parametric summary modes, or subcolumn `0` center for explicit limits/medians) and update `transform::curve`, `stats`, `fit::types`, and `formula::eval` to use it (plus reject invalid pre-summarized X spread/`N` consistent with `Y`), rather than diverging from `chart::xy`.
- **Related:** `FA-SEC07-08` (`deming_regression` ignores pre-summarized `(SD, N)` moments on `fit` side).

---

### `FA-X2-05` · `S2` · Pre-summarized `ColumnTable` multi-row pooling, `%CV`/spread validation, and `n = 1` handling diverge across `doc`, `stats`, and `chart`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `crates/doc/src/cell.rs:315-358` (`SummarizedPoint::reconstruct`)
  - `crates/stats/src/column.rs:574-583` (`run_column_stats` / pre-summarized column summary)
  - `crates/stats/src/two_group.rs:470-498, 1687-1696` (`summarize_pre_summarized_points`)
  - `crates/stats/src/anova/sheet/one_way.rs:953-999` (`extract_one_way_inputs`)
  - `crates/stats/src/anova/one_way.rs:180, 252` (`one_way_anova_from_summary`)
  - `crates/chart/src/column_grouped.rs:762-776` (`extract_column_presummarized`)
- **Category:** `data-fidelity` / `cross-crate-divergence`
- **Defect:** Across the `doc` <-> `stats` <-> `chart` seam for `ColumnTable`s in `MeanSdN`, `MeanSemN`, and `MeanCvN` modes, three incompatible data contracts exist for multi-row summary columns, invalid spreads, and `N = 1` groups:
  1. **Descriptive Column Stats & Unpaired Two-Group (`column.rs:574-583`, `two_group.rs:470-498, 1687-1696`):** Iterate all rows via `col_tab.summarized_point(r, idx)?` (`SummarizedPoint::reconstruct`), pooling multiple cohort rows per group via weighted means and exact pooled sum-of-squares, accepting `n = 1` rows whenever pooled `N >= 2`, and rejecting negative spread or `%CV` when `mean == 0.0`.
  2. **One-Way ANOVA (`anova/sheet/one_way.rs:953-997`):** Bypasses `summarized_point` entirely, **`break`s on the first active `(Mean, Err, N)` row** (`:981`), silently ignores `Err` when `Err < 0.0`, computes `(err / 100.0) * mean.abs()` even when `mean == 0.0` (`:993`, yielding silent `SD = 0`), and passes unpooled single-row inputs to `anova/one_way.rs:180, 252` which requires `n >= 2` per group.
  3. **Column/Bar/Error Chart (`chart/src/column_grouped.rs:762-776`):** `extract_column_presummarized` also returns immediately on the first active row (`return ExtractedKind::PreSummarized { ... }` at `:772`), never pooling subsequent rows.
- **Failure scenario:** A user enters two batch summary rows per treatment group (`Row 0: Mean=10, SD=2, N=5`; `Row 1: Mean=14, SD=3, N=5`) in a `ColumnTable` (`MeanSdN`). Column Descriptives and Unpaired *t*-test pool both rows (`N = 10`, pooled mean `= 12.0`), while One-Way ANOVA on the exact same table analyzes only `Row 0` (`N = 5`, mean `= 10.0`) and the Column Bar Chart draws `10.0 ± 2.0` instead of `12.0` — silently contradicting both Column Statistics and Unpaired *t*-test on the exact same sheet! Conversely, if Row 0 has `N = 1` and Row 1 has `N = 5`, *t*-test succeeds with `N = 6` while ANOVA aborts on `n < 2`.
- **Evidence:**
  - `crates/stats/src/anova/sheet/one_way.rs:981`: `break;` exits after the first active `(Mean, Err, N)` row; `:993` accepts `MeanCvN` at `mean == 0.0`; `:963-996` lacks any `err >= 0.0` validation.
  - `crates/stats/src/column.rs:574-583` & `crates/stats/src/two_group.rs:1687-1696`: collect all active `summarized_point` rows and pool via `summarize_pre_summarized_points` (`two_group.rs:470-498`).
  - `crates/chart/src/column_grouped.rs:770-774`: `extract_column_presummarized` returns on the first matching row.
- **Suggested fix:** Route `anova/sheet/one_way.rs` and `chart/src/column_grouped.rs::extract_column_presummarized` through `SummarizedPoint::reconstruct` (`col_tab.summarized_point`) and shared multi-row summary pooling so `ColumnTable` mean, SD/SEM/95%-CI, and validation (`spread >= 0`, undefined `%CV` at `mean == 0`) agree across `stats`, `doc`, and `chart`.
- **Related:** `FA-SEC06-01` (RM ANOVA accepts pre-summarized `ColumnTable`), `FA-SEC09-05`, `FA-SEC09-08`.

---

### `FA-X2-06` · `S2` · `PointStyleOverride` `series_index` collides across multi-sheet graphs and goes unshifted/unpruned on `table.paste`, `table.importCsv`, and live-link refresh

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `crates/doc/src/graph.rs:832-839, 1402-1420` (`PointStyleOverride`)
  - `crates/engine/src/cmd/helpers.rs:762-763, 885-922` (`point_override_bounds_parts`, `prune_shortened_series_overrides`)
  - `crates/engine/src/cmd/graph.rs:1405-1452, 1723-1738` (`run_format_selected_points`, `reconcile_point_overrides_on_series_change`)
  - `crates/engine/src/cmd/table.rs:1201, 1319, 1440, 2360, 2569-2597` (`run_paste`)
  - `crates/engine/src/cmd/table/import.rs:597` (`run_import_csv`)
  - `crates/engine/src/dag.rs:847-900` (`refresh_single_region`)
  - `crates/transform/src/import.rs:590-597` (`place_grid` `InsertRows` shifts cells & `exclusion_notes` only)
  - `crates/chart/src/{xy.rs:2228, 2433, 2457, column_grouped.rs:520, 1158, 1511, 1815, multivariate_layout.rs:251, 2236, 2501, 3300, 3464}`
- **Category:** `data-fidelity` / `cross-crate-contract-drift`
- **Defect:** Two compounding cross-crate seam defects break per-point visual overrides (`hidden: true`, highlight colors/symbols):
  1. **Multi-sheet `series_index` collision (`doc` vs `engine`/`chart`):** `PointStyleOverride::series_index` is documented as `/// Zero-based index into GraphSpec::series` (`crates/doc/src/graph.rs:833-834`). Yet `chart` callers (`xy.rs:2433, 2457`, `column_grouped.rs:520, 1158, 1511, 1815`, `multivariate_layout.rs:251, 2236, 2501, 3300, 3464`) pass the source column index (`data.col_index`, `o.source.col`, `data_col`, `enc.y_column`) rather than the `GraphSpec::series` index, and `engine/src/cmd/helpers.rs` (`point_override_bounds_parts` at `:762-763` and `prune_shortened_series_overrides` at `:912-920`) keys `GraphSpec::series` by `data_set_index`! In a multi-sheet graph plotting column `0` of Sheet A and column `0` of Sheet B, `helpers.rs:762-763` (`find`) checks bounds only against Sheet A, `helpers.rs:912-920` (`collect::<HashMap<usize, &SheetId>>()`) overwrites Sheet A with Sheet B, and `chart` silently applies the override to **both** sheets' points.
  2. **Missing `InsertRows` shift & shortening prune:** While `table.{setCell, setCells, excludeCells, clearCells}` call `snapshot_active_series_lengths` + `prune_shortened_series_overrides` (`table.rs:1201, 1319, 1440, 2360`), `table.paste` (`table.rs:2569`), `table.importCsv` (`import.rs:597`), and `dag::refresh_single_region` (`dag.rs:847`) never call either helper. Worse, `PlacementMode::InsertRows` in `place_grid` (`transform/src/import.rs:590-597`) shifts `DataSheet` cells and `exclusion_notes` down cleanly, leaving `GraphSpec::point_overrides` unshifted.
- **Failure scenario:** A researcher hides an artifact point (`hidden: true`) on row 3 (`point_index: 2`) and inserts two new rows above it via `table.paste { placement: InsertRows }`, refreshes a shorter linked CSV, or plots two data sheets on one graph (`Sheet A col 0` + `Sheet B col 0`). Inserting rows moves the artifact to row 5 while `hidden: true` remains at row 3 — making the artifact reappear on the published graph while hiding an unrelated valid measurement! In multi-sheet graphs, hiding row `k` on Sheet A hides row `k` on Sheet B simultaneously.
- **Evidence:**
  - `crates/doc/src/graph.rs:833-834`: `/// Zero-based index into GraphSpec::series. pub series_index: usize`.
  - `crates/engine/src/cmd/helpers.rs:762-763, 912-920`: indexes by `s.data_set_index == series_index` and overwrites colliding keys in `HashMap<usize, &SheetId>`.
  - `crates/chart/src/xy.rs:2433, 2457`, `column_grouped.rs:520, 1158, 1511, 1815`, `multivariate_layout.rs:251, 2236, 2501, 3300, 3464`: pass `col_index` / `source.col` / `data_col`.
  - `crates/engine/src/cmd/table.rs:2569`, `import.rs:597`, `dag.rs:847`: omit `snapshot_active_series_lengths` / `prune_shortened_series_overrides` and `point_index` shifting on `InsertRows`.
- **Suggested fix:** Resolve `PointStyleOverride.series_index` strictly as index into `GraphSpec::series` (falling back to column index only when `spec.series.is_empty()`), update `chart` renderers and `helpers.rs` to index by `GraphSpec::series` slot (or add an explicit `sheet_id` selector on `PointStyleOverride`), and run `point_index` shifting (on `InsertRows`) + `prune_shortened_series_overrides` across `table.paste`, `table.importCsv`, and `dag::refresh_single_region`.
- **Related:** `FA-SEC12-01` (`table.convertArchetype` corrupts `GraphSpec.point_overrides`).

---

### `FA-X2-07` · `S2` · `XColumnMode::GeneratedSequence` CSV exports and `.curvebook` / `.curvebook.d` `sheets/*.csv` archive sidecars drop the `X` column and `x_cells` exclusions

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `crates/doc/src/archetypes.rs:227-233, 375-391` (`XColumnMode::subcolumn_count`, `XyTable::materialized_x_cell_at`)
  - `crates/format/src/csv.rs:479-507` (`export_xy_rows`)
  - `crates/format/src/archive.rs:908-920` (`build_archive_entries`)
  - `crates/engine/src/cmd/table.rs:2654-2664` (`run_export_csv`)
- **Category:** `data-fidelity` / `data-loss`
- **Defect:** `XColumnMode::GeneratedSequence` stores its step formula (`start`, `step`) in `x_mode` rather than materialized `x_cells` numbers, so `XColumnMode::subcolumn_count()` returns `0` (`crates/doc/src/archetypes.rs:227-233`). In `crates/format/src/csv.rs:485-502`, `export_xy_rows` iterates `for s in 0..t.x_mode.subcolumn_count()` instead of using `XyTable::materialized_x_cell_at(r)` (`crates/doc/src/archetypes.rs:375-391`), writing **zero** X header and **zero** X data fields. Because both `table.exportCsv` (`crates/engine/src/cmd/table.rs:2654`) and `.curvebook` / `.curvebook.d` archive saving (`build_archive_entries` at `crates/format/src/archive.rs:908-920`) call `export_datasheet_to_csv`, both exported CSV files and persisted archive `sheets/<id>.csv` sidecars drop the entire X axis (time/dose sequence) and every X-row exclusion marker (`x_cells[r][0].excluded`).
- **Failure scenario:** A time-course experiment (`XColumnMode::GeneratedSequence { start: 0.0, step: 15.0 }`) has row `2` excluded at `X = 30.0` (`x_cells[2][0].excluded = true`) and is exported via `table.exportCsv` or inspected via unpacked `.curvebook.d/sheets/*.csv` sidecars. The resulting CSV contains only `Row Title` (if any) and `Y` columns — the time axis (`0, 15, 30*, ...`) vanishes completely. Re-importing that CSV into another project or `DataSheet` loses the independent variable X and silently misaligns `Y` columns as X/column series.
- **Evidence:**
  - `crates/doc/src/archetypes.rs:227-233`: `XColumnMode::GeneratedSequence { .. } => 0`.
  - `crates/doc/src/archetypes.rs:375-391`: `materialized_x_cell_at` explicitly reconstructs `Cell { value: CellValue::Number(v), excluded }` for `GeneratedSequence`.
  - `crates/format/src/csv.rs:485-502`: loops `for s in 0..x_subs` where `x_subs == 0`, skipping both the X header (`:486-489`) and the X cells (`:497-502`), despite `:499` even having a fallback `t.x_value_at(r)` that can never execute when `x_subs == 0`!
  - `crates/format/src/archive.rs:908-920`: writes `export_datasheet_to_csv` output into `sheets/*.csv` on every `.curvebook` / `.curvebook.d` archive save.
- **Suggested fix:** When `matches!(t.x_mode, XColumnMode::GeneratedSequence { .. })`, emit one X column in `export_xy_rows` (`csv.rs:485-502`) using `t.x_title` and `t.materialized_x_cell_at(r)` so CSV exports and `.curvebook`/`.curvebook.d` CSV sidecars include the generated X values and `*` exclusion markers.
- **Related:** `FA-SEC02-10` (`set_x_or_time_excluded` on `GeneratedSequence`), `FA-SEC04-06` (`apply_csv_import_link` corrupts `GeneratedSequence` rectangularity).

---

### `FA-X2-08` · `S3` · `backfill_confidence_interval_cells` ignores `ImportFilterSpec` and `skip_contiguous_blanks`, misplacing or omitting `[lo, hi]` cells on linked `ResultTab`s

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:**
  - `crates/transform/src/import.rs:278-287, 341-401, 563-627` (`result_cell_to_cell`, `apply_stage2_filter`, `place_grid`)
  - `crates/engine/src/dag.rs:807-845, 888-895` (`backfill_confidence_interval_cells`, `refresh_single_region`)
  - `crates/engine/src/cmd/table/import.rs:368-372` (`run_paste_link`)
  - `crates/engine/src/cmd/sheet.rs:1985` (`sheet.extractResultTab`)
- **Category:** `data-fidelity` / `layout-mismatch`
- **Defect:** Across the `transform/import` -> `engine/dag` / `engine/cmd/{sheet, table/import}` seam, `result_cell_to_cell` maps `ResultCell::ConfidenceInterval { lower, upper }` to `Cell::empty()` (`crates/transform/src/import.rs:278-287`). To compensate, `run_paste_link`, `dag::refresh_single_region`, and `sheet.extractResultTab` run `dag::backfill_confidence_interval_cells` (`crates/engine/src/dag.rs:807-845`) afterwards. However, `backfill_confidence_interval_cells`:
  - Returns early with **all CI cells left blank** whenever `spec.placement.transpose == true` (`dag.rs:814-816`), and
  - Otherwise iterates raw unfiltered `tab.rows[range.start.row..=end_r]` and `range.start.col..=end_c` with sequential counters `grid_row` and `grid_col` (`dag.rs:822-844`), ignoring every filter/placement transformation performed by `apply_import_link_pipeline`: `spec.filter.{row_window, col_window, decimate_every_k, unstack_group_column}` and `spec.placement.skip_contiguous_blanks`.
- **Failure scenario:** A user live-links or extracts a regression or ANOVA `ResultTab` containing `95% CI` cells using a `row_window` / `col_window` subset, `decimate_every_k`, `transpose: true`, or `skip_contiguous_blanks: true`. Either all CI cells remain blank (under `transpose: true`), or `backfill_confidence_interval_cells` writes `[lo, hi]` strings from skipped/pre-filter rows onto unrelated rows/columns of the target `DataSheet`, corrupting or mislabeling confidence bounds.
- **Evidence:**
  - `crates/transform/src/import.rs:285`: `ResultCell::ConfidenceInterval { .. } => Cell::empty()`.
  - `crates/engine/src/dag.rs:814-816`: `if tab.rows.is_empty() || spec.placement.transpose { return; }`.
  - `crates/engine/src/dag.rs:822-844`: walks raw `range.start.row..=end_r` and `range.start.col..=end_c`, never consulting `spec.filter` or `spec.placement.skip_contiguous_blanks`.
- **Suggested fix:** Format `ResultCell::ConfidenceInterval` directly inside `result_cell_to_cell` (moving `format_ci_bound_text` down to `crates/transform` or `crates/doc`) so CI cells pass natively through `row_window`, `col_window`, `decimate_every_k`, `unstack_group_column`, `transpose`, `skip_contiguous_blanks`, and `place_grid` identically to all other `ResultCell` variants, removing the separate `backfill_confidence_interval_cells` post-pass.
- **Related:** Extends `FA-SEC03-15` (`ResultCell::ConfidenceInterval drops on import` at the `crates/transform` boundary).
