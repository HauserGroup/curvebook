# SEC-12 Audit Report — Engine Commands (Part 2)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Scope:** `7` files in `crates/engine/src/cmd/`: `table.rs`, `table/import.rs`, `analysis.rs`, `graph.rs`, `layout.rs`, `export.rs`, `image.rs` (`18,430` lines total)
- **Date:** `2026-11-22`
- **Reviewer:** `rev-engine` (report-only; line `1..=N` coverage on all `7` files)
- **Summary Counts:** `S1 = 0`, `S2 = 2`, `S3 = 8`, `S4 = 1` (Total: `11` findings)
- **Prior Remediation Check (`docs/audit/local-code-audit-2026-11-18.md`):**
  - Closed findings `#1`–`#126` affecting `SEC-12` (`#4`, `#7`, `#79`, `#93`, `#113`, `#114`, `#120`): **0 regressions** verified across `cmd/analysis.rs` and `crates/engine/tests/audit_engine_analysis_fixes.rs`.
  - Open finding `#127` (`docs/audit/local-code-audit-2026-11-18.md:1356-1360`, recorded 2026-11-20) covers the default log-X nonlinear fit model (`log_agonist_vs_response_4pl`) and is not duplicated here.
  - Shared helpers owned by `SEC-11` (`decode_base64` `FA-SEC11-02`, `command_requires_wal_checkpoint` `FA-SEC11-05`, `helpers::write_output_file` `FA-SEC11-09`) are cross-referenced where called rather than duplicated.

## Baseline Verification Summary

- `cargo clippy --locked --offline -p curvebook-engine --all-targets -- -D warnings` (with `CARGO_TARGET_DIR=/tmp/cb-sec12-rev`): exited `0` with zero Clippy lint warnings; Cargo emitted `warning: unused dependency 'curvebook-tools'` on `crates/engine/Cargo.toml:30:1` (tracked in `FA-SEC11-10`).
- `cargo test --locked --offline -p curvebook-engine --quiet` (with `CARGO_TARGET_DIR=/tmp/cb-sec12-rev`): exited `0`. Observed `269` passed, `0` failed, `0` ignored across `35` test runs:
  - Unit tests (`src/lib.rs`): `56` passed, `0` failed, `0` ignored.
  - Integration suites (`crates/engine/tests/`): `213` passed across `34` test binaries (`11 + 16 + 9 + 3 + 5 + 4 + 2 + 6 + 19 + 4 + 11 + 7 + 4 + 9 + 6 + 11 + 4 + 10 + 3 + 3 + 6 + 4 + 4 + 1 + 3 + 4 + 4 + 3 + 10 + 2 + 5 + 10 + 6 + 4`), `0` failed, `0` ignored.
  - Doc-tests (`curvebook_engine`): `0` tests.
- `rustup target list --installed`: `aarch64-apple-darwin` (checked `wasm32` compatibility and `#![forbid(unsafe_code)]` via source inspection across all `7` files).

---

## File Coverage Table

| # | File | Lines | Inspected | Concrete Invariants & Dimensions Verified | Findings |
|---|---|---:|---|---|---|
| 1 | `crates/engine/src/cmd/table.rs` | 5,443 | `1..=5,443` | `28` `CommandSpec`s & `enabled` predicates; `ensure_sheet_not_frozen`, `ensure_mv_columns_editable`, linked-region overlap guards (missing in `table.setCalculatedVariable`); MV formula cycle + rename propagation; `MAX_PROJECT_CELLS` missing on simulation/generation commands; `remap_graph_spec_for_conversion` vs `point_overrides` & `multivariate`; `decimal_separator` coercion; schema `additionalProperties: true` vs strict deserialization. Cross-ref `FA-SEC11-05` for `table.excludeCellsWithReason` WAL coverage. | `FA-SEC12-01`, `FA-SEC12-02`, `FA-SEC12-04`, `FA-SEC12-07`, `FA-SEC12-09`, `FA-SEC12-11` |
| 2 | `crates/engine/src/cmd/table/import.rs` | 1,454 | `1..=1,454` | `pasteLink`, `importCsv`, `importPrism`, `importPzfx`, `importPlateMap`, `importTidyLong`; sheet ID collision remapping (`plan_imported_id_remap`, `merge_imported_document`); `ExternalPathPolicy::read_utf8_file` / `read_binary_file` / `normalize_stored_link_source`; `MAX_PROJECT_CELLS` enforced only in `merge_imported_document`; uncapped inline `bytes` / `csv`; malformed `link_spec` fallthrough; schema `additionalProperties: true`. Cross-ref `FA-SEC11-02` for `decode_base64` at `:804, 865`. | `FA-SEC12-02`, `FA-SEC12-03`, `FA-SEC12-07`, `FA-SEC12-08` |
| 3 | `crates/engine/src/cmd/analysis.rs` | 4,406 | `1..=4,406` | `12` `CommandSpec`s (`analyze.run`, `stats.run`, `stats.pvalueStack`, `fit.run`, `transform.run`, `survival.run`, `analysis.*`, `result.generateReport`); `#4`, `#7`, `#79`, `#93`, `#113`, `#114`, `#120` verified intact; `validate_map_keys_for_kind` & `persist_default_mv_columns`; `validate_multiple_row_tests_spec` matches `crates/stats/src/two_group.rs:2564-2575`; `Simulate` `parameters` non-object silent ignore. | `FA-SEC12-10` |
| 4 | `crates/engine/src/cmd/graph.rs` | 4,147 | `1..=4,147` | `17` `CommandSpec`s; `point_overrides` bounds & `reconcile_point_overrides_on_series_change` alignment with `crates/chart` (`data_set_index`); `graph.setAxes` silent no-op when `axis` omitted; permissive `.and_then(...)` coercion & unknown-key drops in axis/segment/series patchers; `graph.setSpec` shorthand drop & non-strict `GraphSpec` deserialization. | `FA-SEC12-05`, `FA-SEC12-06` |
| 5 | `crates/engine/src/cmd/layout.rs` | 1,548 | `1..=1,548` | `9` `CommandSpec`s; `MAX_LAYOUT_ITEMS`, finite/positive `bounds_pt` & `font_size_pt` (`LayoutSpec::validate`), DAG cycle & inline hook validation; proportional canvas resize scaling; closed schemas on `8` commands; `layout.setSpec` shorthand drop when `spec` is passed and non-strict `LayoutSpec` deserialization. | `FA-SEC12-06` |
| 6 | `crates/engine/src/cmd/export.rs` | 1,042 | `1..=1,042` | `9` `CommandSpec`s (`WRITES_FILESYSTEM \| IDEMPOTENT`, closed `additionalProperties: false` schemas); `MAX_PDF_PAGES` (`64`), `MAX_PDF_TOTAL_PRIMITIVES` (`500_000`) & `MAX_PDF_OUTPUT_BYTES` enforced on both multi-page and single-page PDF (`render_pdf` delegates to `render_pdf_page_refs` at `crates/pdf/src/writer.rs:95-134`); raster pixel/dim limits; native `enforce_command_path_params` containment. Cross-ref `FA-SEC11-09` for `wasm32` `write_output_file` fallback at `:191-194`. | Clean — all limits, schemas, and vector/raster export invariants verified (cross-ref `FA-SEC11-09` at `:191-194`) |
| 7 | `crates/engine/src/cmd/image.rs` | 390 | `1..=390` | Shared image ingestion/geometry helpers; `reject_uri_scheme` rejects `http`/`https`/`file`/`ftp`/`data` URIs; `decode_base64_capped` enforces `MAX_IMAGE_ASSET_BYTES` & rejects empty payloads; `path_policy.read_binary_file` regular-file & size limits; `#[serde(deny_unknown_fields)]` + `additionalProperties: false`; finite crop/border/rotation/aspect-locked bounds checks. | Clean — input caps, URL rejection, schema strictness, and geometry bounds verified |

---

## Findings (`S1` -> `S4`)

*(No `S1` Critical findings identified in `SEC-12`.)*

### `FA-SEC12-01` · `S2` · `table.convertArchetype` corrupts `GraphSpec.point_overrides` and retains unmappable `GraphSpec.multivariate` columns

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/table.rs:4444-4496`, `crates/engine/src/cmd/table.rs:3600-3604`
- **Category:** `data-loss` / `correctness`
- **Defect:** Across all chart renderers (`crates/chart/src/xy.rs:2228`, `crates/chart/src/column_grouped.rs:520`) and engine helpers (`crates/engine/src/cmd/helpers.rs:763`, `crates/engine/src/cmd/graph.rs:1730`), `PointStyleOverride.series_index` identifies a dataset by its `data_set_index` on the backing `DataSheet`. During `table.convertArchetype`, `remap_graph_spec_for_conversion` updates `SeriesSpec::data_set_index` and removes unmappable series from `spec.series`, **but never remaps, expands, or prunes `spec.point_overrides`** (nor does `run_convert_archetype` invoke `prune_out_of_bounds_point_overrides`). In addition, on `spec.multivariate` (`4468-4495`), failed `plan.map_one(...)` calls are silently ignored via `if let Ok(...)`, leaving stale or out-of-bounds `x_column`, `y_column`, `label_column`, `color`, and `size` indices in place.
- **Failure scenario:** A user creates an XY graph with 1 Y column (`data_set_index: 0`), marks or hides outlier row 2 via `graph.formatSelectedPoints` (`PointStyleOverride { series_index: 0, point_index: 2, hidden: true }`), and then converts the sheet via `table.convertArchetype` (`target_kind: "Column"`). Because `convert_xy_to_column` (`crates/transform/src/archetype_convert.rs:692-699`) inserts `X` as Column `0` and shifts old `Y0` to Column `1` (`ConversionColPlan::from_xy_to_column` at `table.rs:3903`), `s.data_set_index` updates to `1`, while `point_overrides` stays `{ series_index: 0, point_index: 2 }`, silently hiding/restyling `X` point 2 instead of `Y` point 2. When series are dropped (e.g. `Grouped -> MultipleVariables`), orphaned `point_overrides` remain attached to `GraphSpec` pointing to non-existent series and fail later `validate_point_override_in_bounds` checks. Similarly, converting an MV table while retaining `spec.multivariate` leaves dropped/unmappable channel column indices pointing to wrong or out-of-range columns.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/table.rs:4451-4495
  let mut kept_series = Vec::with_capacity(spec.series.len());
  for mut s in std::mem::take(&mut spec.series) {
      if s.source_sheet_id == *target_sheet_id {
          match plan.map_one(s.data_set_index, "graph series") {
              Ok(new_col) => {
                  s.data_set_index = new_col;
                  kept_series.push(s);
              }
              Err(reason) => {
                  warnings.push(format!("Graph `{graph_id}` dropped series binding: {reason}"));
              }
          }
      } else {
          kept_series.push(s);
      }
  }
  spec.series = kept_series;
  if spec.primary_source_sheet_id == *target_sheet_id
      && let Some(mv) = &mut spec.multivariate
  {
      if let Ok(xc) = plan.map_one(mv.x_column, "graph multivariate x_column") {
          mv.x_column = xc;
      }
      ...
  }
  ```
- **Suggested fix:** Capture which `data_set_index` values targeted `target_sheet_id` before modifying `spec.series`, then remap each `PointStyleOverride` targeting `target_sheet_id`: expand `ColMapTarget::Expanded(cols)` or update `Exact(new_c)`, drop overrides mapped to `MovedToX` or `Unmappable` (or whose series were dropped), prune out-of-bounds overrides (`point_index >= new_rows`), and clear/reset or warn on unmappable `spec.multivariate` channel bindings when `plan.map_one` returns `Err`.
- **Related:** `crates/engine/src/cmd/graph.rs:1723-1739` (`reconcile_point_overrides_on_series_change`).

---

### `FA-SEC12-02` · `S2` · Table import, generation, and simulation commands bypass project-wide `MAX_PROJECT_CELLS` (`50_000_000`) ceiling

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/table/import.rs:523-535`, `crates/engine/src/cmd/table/import.rs:633-638`, `crates/engine/src/cmd/table/import.rs:1143-1158`, `crates/engine/src/cmd/table.rs:3455-3465`, `crates/engine/src/cmd/table.rs:4828-4833`, `crates/engine/src/cmd/table.rs:4910-4943`, `crates/engine/src/cmd/table.rs:5134-5137`
- **Category:** `resources`
- **Defect:** `curvebook_doc::validate_project_cells` (`crates/doc/src/error.rs:35, 97-103`) caps cumulative allocated cell slots across all `DataSheet`s in a project at `MAX_PROJECT_CELLS = 50_000_000` (vs `MAX_SHEET_CELLS = 30_000_000` per single sheet). `cmd/sheet.rs` (`287, 985, 1299`) and `merge_imported_document` (`cmd/table/import.rs:746`) enforce this ceiling, whereas `ProjectDocument::add_data_sheet` (`crates/doc/src/project.rs:154-162`) only checks `MAX_SHEETS_PER_PROJECT` (`500`) and `curvebook_format::validate_project_document` (`crates/format/src/archive.rs:2115-2180`) only validates per-sheet cell counts. All seven single-sheet import/simulation entry points (`table.importCsv` new sheet & `replace: true`, `table.importPlateMap`, `table.importTidyLong`, `table.simulateXy`, `table.randomizeGroups`, `table.randomSampleRows`, and `table.simulateDistributions`) construct sheets bounded only by `MAX_SHEET_CELLS` and insert or overwrite them without calling `validate_project_cells`.
- **Failure scenario:** Repeated calls to `table.simulateDistributions`, `table.simulateXy`, or `table.importCsv` (each allocating up to `30_000_000` cells) allocate `60M+` cells (potentially up to `MAX_SHEETS_PER_PROJECT * MAX_SHEET_CELLS`) inside a single `ProjectDocument`, bypassing `MAX_PROJECT_CELLS`, bloating checkpoints/undo snapshots, and causing excessive memory consumption or OOM. (Note: `table.setCell`/`setCells`/`paste`/`setCalculatedVariable` also grow existing sheets up to `MAX_SHEET_CELLS` without checking `MAX_PROJECT_CELLS`.)
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/table/import.rs:633-638
  } else {
      let title = p.title.as_deref().unwrap_or("Imported CSV");
      let kind = parse_table_kind(p.archetype.as_deref().unwrap_or("xy"))?;
      let imported = import_csv_to_datasheet(id.clone(), title, &text, kind, subcolumn_mode, &csv_opts)?;
      doc.add_data_sheet(imported)?;
  }
  ```
  ```rust
  // crates/engine/src/cmd/table.rs:5134-5136
  let sheet = simulate_distribution_columns(&cfg)?;
  let kind = sheet.archetype.kind();
  doc.add_data_sheet(sheet)?;
  ```
  Compare `crates/engine/src/cmd/table/import.rs:744-746`:
  ```rust
  let existing_cells: usize = doc.data_sheets.iter().map(|s| s.archetype.allocated_cell_slots()).sum();
  let added_cells: usize = imported.data_sheets.iter().map(|s| s.archetype.allocated_cell_slots()).sum();
  validate_project_cells(existing_cells, added_cells)?;
  ```
- **Suggested fix:** Introduce a shared engine helper `validate_project_cell_budget_after_change(doc, replaced_sheet_id, added_cells)` (computing existing allocated cell slots excluding any replaced sheet, then calling `validate_project_cells`) and enforce it before inserting or replacing sheets in `table/import.rs` and `table.rs`, or enforce `MAX_PROJECT_CELLS` directly inside `validate_project_document` / `ProjectDocument::add_data_sheet`.
- **Related:** `FA-SEC11-03` (`sheet.extractResultTab` missing `validate_project_cells`), `FA-SEC11-08` sibling schema checks.

---

### `FA-SEC12-03` · `S3` · `table.importPrism` (`bytes`) and `table.importCsv` / `table.importTidyLong` (`csv`) bypass file-read byte limits on inline payloads

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/table/import.rs:608-615`, `crates/engine/src/cmd/table/import.rs:800-812`, `crates/engine/src/cmd/table/import.rs:1370-1377`
- **Category:** `security` / `resources`
- **Defect:** File-path imports enforce explicit format byte ceilings (`MAX_ARCHIVE_FILE_BYTES = 128 MiB` via `read_binary_file` at `:802`; `MAX_CSV_FILE_BYTES = 64 MiB` via `read_utf8_file` at `:611, 1373`). By contrast, inline payload inputs bypass those bounds:
  1. `run_import_prism` (`:805-806`) passes `raw_bytes: Vec<u8>` directly to `import_prism_bytes(&bytes)?` with no `bytes.len() > MAX_ARCHIVE_FILE_BYTES` check (`ZipArchive::from_bytes_with_limits` at `crates/format/src/zip.rs:247-264` caps entry counts and uncompressed entry bytes, not total outer slice length).
  2. `run_import_csv` (`:608-609`) and `extract_tidy_data` (`:1370-1371`) clone and parse `p.csv` with no `MAX_CSV_FILE_BYTES` check (unlike `run_import_pzfx` at `:859-860`, where `parse_xml_with_limits` enforces `MAX_XML_BYTES = 64 MiB` on both `xml` and `path`).
- **Failure scenario:** Automated/MCP callers sending oversized `bytes` or `csv` payloads avoid the explicit `ResourceLimit` / `BadInput` pre-checks enforced on `path` inputs, triggering unbounded `clone()` / parsing work (and note `:808` error text `"table.importPrism requires \`path\` or \`bytes_base64\`"` also omits `bytes` from the primary error string). (Uncapped `decode_base64` on `bytes_base64` at `:804, 865` is tracked separately in `FA-SEC11-02`.)
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/table/import.rs:801-812
  let bytes = if let Some(path) = p.path.as_deref() {
      ctx.session.path_policy.read_binary_file(path, "archive", MAX_ARCHIVE_FILE_BYTES)?
  } else if let Some(b64) = p.bytes_base64.as_deref() {
      decode_base64(b64)?
  } else if let Some(raw_bytes) = p.bytes {
      raw_bytes
  } else { ... };
  let report = import_prism_bytes(&bytes)?;
  ```
  ```rust
  // crates/engine/src/cmd/table/import.rs:608-611 (same pattern at :1370-1373)
  let text = if let Some(inline) = &p.csv {
      inline.clone()
  } else if let Some(path) = &p.path {
      policy.read_utf8_file(path, "CSV", MAX_CSV_FILE_BYTES)?
  }
  ```
- **Suggested fix:** Check `raw_bytes.len() > MAX_ARCHIVE_FILE_BYTES` in `run_import_prism` before calling `import_prism_bytes`, check `inline.len() > MAX_CSV_FILE_BYTES` in `run_import_csv` and `extract_tidy_data`, and use a capped `decode_base64` helper (per `FA-SEC11-02`) for `bytes_base64`.
- **Related:** `FA-SEC11-02` (`decode_base64` lacking payload size limits).

---

### `FA-SEC12-04` · `S3` · `table.setCalculatedVariable` converts and overwrites live linked columns without checking `linked_regions`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/table.rs:3240-3291`
- **Category:** `data-loss` / `contract-drift`
- **Defect:** Every other cell-mutating command on `DataSheet` (`table.setCell` at `:1204`, `table.setCells` at `:1322`, `table.excludeCells` at `:1911`, `table.excludeCellsWithReason` at `:2122`, `table.clearCells` at `:2367, 2377`, `table.paste` at `:2556`, `table.recodeLevels` at `:2961`, and `table.importCsv` at `table/import.rs:578`) checks `ds.linked_regions` and rejects overlapping modifications with `ErrorCode::Conflict`. `run_set_calculated_variable` modifies `mv.variables[col_idx]` into `VariableSource::Calculated` and overwrites all cells in that column via `evaluate_data_sheet_calculated_variables` without checking whether `col_idx` lies inside an active `LinkedDataRegion`.
- **Failure scenario:** On a `MultipleVariables` sheet containing a live `pasteLink` or `importCsv { link: true }` region covering column `1`, calling `table.setCalculatedVariable { sheet_id, col: 1, formula: "[Col0] * 2" }` converts the linked column into a calculated variable and overwrites its cells in place. Subsequent upstream edits or link refreshes then either clobber the formula column's cells or fail recalculation unexpectedly, leaving live-link provenance inconsistent with column ownership.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/table.rs:3256-3290
  let (col_idx, created_new) = if let Some(ref col_ref) = p.col {
      (resolve_mv_variable_index(mv, Some(col_ref), None)?, false)
  } else ...
  if col_idx >= mv.variables.len() {
      return Err(EngineError::with_code(ErrorCode::BadInput, format!("variable column {col_idx} out of range")));
  }
  ...
  mv.variables[col_idx].set_calculated_formula(&p.formula);

  evaluate_data_sheet_calculated_variables(ds, info_snapshot.as_ref())?;
  ```
- **Suggested fix:** When `!created_new` and `ds.archetype.row_count() > 0`, check `ensure_range_outside_linked_regions` across rows `0..ds.archetype.row_count()` at column `col_idx` (mirroring `run_recode_levels` at `table.rs:2958-2962`) before mutating `mv.variables[col_idx]`, returning `ErrorCode::Conflict` with remediation to detach the link first.
- **Related:** `table.recodeLevels` (`table.rs:2958-2962`), `table.setCell` (`table.rs:1203-1205`).

---

### `FA-SEC12-05` · `S3` · `graph.setAxes` silently no-ops when `axis` is omitted, and `graph.rs` patch helpers silently discard unknown keys and mistyped values

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/graph.rs:545-582`, `crates/engine/src/cmd/graph.rs:1643-1658`, `crates/engine/src/cmd/graph.rs:2563-2651`, `crates/engine/src/cmd/graph.rs:2653-2932`
- **Category:** `contract-drift` / `correctness`
- **Defect:**
  1. `GraphSetAxesParams` (`545-558`) uses `#[serde(flatten)] axis_fields: Map<String, Value>` paired with `additionalProperties: true` (`580`). Inside `run_graph_set_axes` (`1643-1658`), `params.axis_fields` is only inspected inside `if let Some(ref which) = params.axis`. When `axis` (`which`) is omitted, all top-level axis properties (e.g. `scale`, `min`, `max`, `title`, or typos) are silently discarded and the command returns `CommandOutcome::mutated("graph.setAxes", ...)` with a spurious undo entry.
  2. Inside `apply_axis_updates` (`2563-2617`), `apply_segment_updates` (`2619-2651`), and `apply_single_series_patch` / `apply_line_to_gdo` / `apply_marker_patch` / `patch_error_bars` (`2653-2932`), unknown patch keys are silently ignored, and numerous fields use lossy `.and_then(...)` option chaining that silently drops mistyped or out-of-range inputs instead of returning `ErrorCode::BadInput`:
     - `gap_width_pt` (`2567`), `stroke_width_pt` (`2573`), `tick_angle_deg` (`2576`), `length_share` (`2637`), `line.stroke_width_pt` (`2868, 2879`), `marker.size_pt` (`2893`), `marker.stroke_width_pt` (`2902`), `error_bars.stroke_width_pt` (`2922`), `error_bars.cap_width_pt` (`2925`) use `.and_then(Value::as_f64)`.
     - `minor_ticks_per_interval` (`2647`) uses `.and_then(Value::as_u64).and_then(|v| u8::try_from(v).ok())`.
     - `bind_to_right_y2` (`2675`) uses `.and_then(Value::as_bool)`.
     - `segments` (`2589`) and `thresholds` (`2605`) use `.and_then(Value::as_array)`.
- **Failure scenario:** Calling `graph.setAxes { "sheet_id": "g1", "scale": "log10", "min": 1, "max": 100 }` (missing `axis`) or passing `{ "sheet_id": "g1", "axis": "y", "stroke_width_pt": "2.0", "minor_ticks_per_interval": 300 }` succeeds without modifying the axis, causing downstream scripts and agents to assume axis scaling/styling changed when the graph remained unchanged.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/graph.rs:1643-1658
  if let Some(ref which) = params.axis {
      let top_val = Value::Object(params.axis_fields);
      match which.trim().to_ascii_lowercase().as_str() { ... }
  }
  ```
  ```rust
  // crates/engine/src/cmd/graph.rs:2567-2578, 2647-2649
  if let Some(gap) = val.get("gap_width_pt").and_then(Value::as_f64) { axis.gap_width_pt = gap; }
  if let Some(sw) = val.get("stroke_width_pt").and_then(Value::as_f64) { axis.stroke_width_pt = sw; }
  if let Some(tick_angle) = val.get("tick_angle_deg").and_then(Value::as_f64) { axis.tick_angle_deg = tick_angle; }
  ...
  if let Some(minor) = val.get("minor_ticks_per_interval").and_then(Value::as_u64).and_then(|v| u8::try_from(v).ok()) {
      seg.minor_ticks_per_interval = minor;
  }
  ```
- **Suggested fix:** Reject non-empty `axis_fields` with `ErrorCode::BadInput` when `params.axis` is `None` (prompting callers to supply `axis` or nest under `x_axis`/`y_axis`/`y2_axis`), validate every recognized axis/segment/series key's JSON type via `parse_finite_f64` / `as_bool` / `as_array` / `u8::try_from`, and reject unknown keys inside axis, segment, and series patch objects.
- **Related:** `FA-SEC12-06` (`graph.setSpec` / `layout.setSpec` patch field handling).

---

### `FA-SEC12-06` · `S3` · `layout.setSpec` and `graph.setSpec` silently discard top-level shorthand fields when `spec` is supplied and deserialize specs non-strictly

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/layout.rs:220-238`, `crates/engine/src/cmd/layout.rs:526-540`, `crates/engine/src/cmd/graph.rs:512-541`, `crates/engine/src/cmd/graph.rs:1556-1571`
- **Category:** `contract-drift`
- **Defect:**
  1. In `run_layout_set_spec` (`layout.rs:526-540`), whenever `params.spec` is `Some(_)`, the `else { apply_layout_spec_fields(&mut spec, &params.fields)?; }` branch is skipped, silently dropping any declared top-level shorthand fields (`preset`, `width`, `height`, `unit`, `orientation`, `grid_template`, `align_plot_origins`, `panel_badges`, `background_fill`) sent in the same call. Furthermore, `serde_json::from_value::<LayoutSpec>(merged)` (`533`) is non-strict, silently dropping unknown nested keys inside `spec` instead of using `deserialize_strict`.
  2. In `run_graph_set_spec` (`graph.rs:1562-1567`), `params.spec.clone().unwrap_or_else(...)` discards every top-level key in `patch_fields` other than the 5 special-cased keys (`multivariate`, `paper_fill`, `orientation`, `forest_show_pooled_diamond`, `heatmap_labels`), silently ignoring top-level `palette`, `x_axis`, `y_axis`, `legend`, etc. whenever `spec` is also passed, and `serde_json::from_value::<GraphSpec>(merged)` (`1567`) silently drops unknown nested keys inside `spec`.
- **Failure scenario:** Sending `layout.setSpec` with `{ "sheet_id": "l1", "spec": { "panel_badges": "UpperCase" }, "align_plot_origins": true }` or `graph.setSpec` with `{ "sheet_id": "g1", "spec": { "palette": "OkabeIto" }, "legend": { "visible": false } }` succeeds while silently dropping the top-level shorthand changes (or mistyped keys inside `spec`).
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/layout.rs:526-540
  if let Some(ref spec_val) = params.spec {
      ...
      let merged = merge_json_struct(&spec, spec_val)?;
      spec = serde_json::from_value(merged).map_err(|err| ...)?;
  } else {
      apply_layout_spec_fields(&mut spec, &params.fields)?;
  }
  ```
  ```rust
  // crates/engine/src/cmd/graph.rs:1562-1567
  let patch =
      params.spec.clone().unwrap_or_else(|| strip_keys(&Value::Object(params.patch_fields.clone()), &["sheet_id", "id", "title", "initialized"]));
  let patch_clean = strip_keys(&patch, &["multivariate", "paper_fill", "orientation", "forest_show_pooled_diamond", "heatmap_labels"]);
  let merged = merge_json_struct(&current_spec, &patch_clean)?;
  let mut new_spec: GraphSpec = serde_json::from_value(merged).map_err(|err| ...)?;
  ```
- **Suggested fix:** Either reject combining `spec` with top-level shorthand fields (`ErrorCode::BadInput`) or apply shorthand updates on top of `spec` explicitly, and validate unknown fields in `spec` / `patch_fields` (using `deserialize_strict` where no custom enum/alias shorthands apply).
- **Related:** `FA-SEC12-05`.

---

### `FA-SEC12-07` · `S3` · `table.rs` and `table/import.rs` schemas advertise `"additionalProperties": true` across all 28 commands while runtime rejects unknown keys

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/table.rs:808, 823, 840, 857, 882, 1120, 1245, 1266, 1366, 1383, 1473, 1544, 1731, 1856, 2017, 2152, 2283, 2483, 2641, 2732, 2840, 2853, 3054, 3222, 3356, 3509, 4729, 4741, 4873, 5095`, `crates/engine/src/cmd/table/import.rs:131, 441, 671, 844, 1006, 1253, 1271`
- **Category:** `contract-drift`
- **Defect:** Every top-level and sub-object `obj_schema_with_aliases` call across `cmd/table.rs` and `cmd/table/import.rs` passes `additional_properties = true` (`28` registered `table.*` commands). At runtime, `parse_params_with_warnings` (`crates/engine/src/cmd/params.rs:285, 306-313`) only enables `allow_ignored_fields` when `is_open_catchall_schema` sees an **empty** `properties` object. Because all `28` `table.*` schemas declare non-empty `properties` and deserialize into typed Rust structs without catch-all maps, `StrictValueDeserializer` rejects any undeclared key with `ErrorCode::BadInput` (`unknown field`), contradicting the published Draft 2020-12 JSON Schemas.
- **Failure scenario:** MCP/automation clients inspecting `CommandSpec::input_schema` treat extra metadata or unmapped fields as permitted because `"additionalProperties": true` is advertised, then fail at runtime with `BadInput` (`unknown field`). By contrast, `cmd/export.rs`, `cmd/image.rs`, `cmd/layout.rs` (`8` typed commands), and `cmd/graph.rs` (`13` typed commands) pass `false`.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/table.rs:1120-1142 (pattern repeated across all 28 table.* schemas)
  fn schema_set_cell() -> Value {
      obj_schema_with_aliases(
          vec![ ... ],
          &["sheet_id"],
          vec![ ... ],
          true,
      )
  }
  ```
- **Suggested fix:** Change `additional_properties` from `true` to `false` in the closed `table.rs` and `table/import.rs` schema builders (preserving `true` only on truly open catch-all schemas), matching `cmd/export.rs`, `cmd/image.rs`, and `cmd/project.rs`.
- **Related:** Sibling of `FA-SEC11-08` (`cmd/sheet.rs`).

---

### `FA-SEC12-08` · `S3` · `table.pasteLink` silently ignores malformed `link_spec` objects instead of returning a validation error

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/table/import.rs:252-266`
- **Category:** `contract-drift`
- **Defect:** In `build_paste_link_spec`, if `p.link_spec` is provided, deserialization is guarded with `if let Some(raw_spec) = &p.link_spec && let Ok(mut parsed) = serde_json::from_value::<ImportLinkSpec>(raw_spec.clone())`. When `serde_json::from_value` fails (e.g. invalid field type or missing required field inside `link_spec`), the `&& let Ok(...)` condition evaluates false and execution silently falls through to `p.source_sheet_id` / `p.source_path`, returning an unrelated error or using fallback fields instead of reporting the malformed `link_spec`.
- **Failure scenario:** Calling `table.pasteLink` with `{ "target_sheet_id": "t1", "link_spec": { "source": { "bad": true } } }` produces `"table.pasteLink requires source_sheet_id or source_path"` rather than explaining why `link_spec` failed to deserialize; if `source_sheet_id` or `source_path` is also present, the malformed `link_spec` is silently ignored and a link is created from the fallback source instead.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/table/import.rs:252-266
  if let Some(raw_spec) = &p.link_spec
      && let Ok(mut parsed) = serde_json::from_value::<ImportLinkSpec>(raw_spec.clone())
  {
      policy.normalize_stored_link_source(&mut parsed.source)?;
      ...
      return Ok(parsed);
  }
  ```
- **Suggested fix:** Map `serde_json::from_value::<ImportLinkSpec>` errors directly with `.map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid `link_spec`: {err}")).with_details(json!({ "pointer": "/link_spec" })))?` whenever `p.link_spec` is `Some(_)`.
- **Related:** None.

---

### `FA-SEC12-09` · `S3` · `CellPayloadFields::into_cell` and `table.paste` (`rows`) silently coerce unsupported or `"auto"` `decimal_separator` values to `'.'`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/table.rs:531-535`, `crates/engine/src/cmd/table.rs:569-575`, `crates/engine/src/cmd/table.rs:1129`, `crates/engine/src/cmd/table.rs:1253`, `crates/engine/src/cmd/table.rs:1372`, `crates/engine/src/cmd/table.rs:2588`
- **Category:** `contract-drift` / `correctness`
- **Defect:** `schema_set_cell`, `schema_set_cells`, `schema_set_x_cells`, and `schema_paste` all document `decimal_separator` using `decimal_sep_schema()` (`table.rs:897-899`), which advertises `"Decimal separator (`.`, `,`, `dot`, `comma`, `period`, or `auto`)"`. While `table.paste { text: ... }` (`:2584`) and `table.importCsv` use `resolve_decimal_char` (`:603-619`, rejecting unknown strings with `BadInput` and handling `"auto"`), `CellPayloadFields::into_cell` (`:533`) and `table.paste { rows: ... }` (`:2588`) call `parse_decimal_opt(raw).unwrap_or('.')`, silently coercing `"auto"` and arbitrary invalid strings (e.g. `";"`, `"foo"`) into `'.'`.
- **Failure scenario:** Calling `table.setCell { sheet_id, row: 0, col: 0, value: "12,5", decimal_separator: "auto" }` or passing a typo `"decimal_separator": "commma"` silently parses `"12,5"` with `'.'` (storing text `"12,5"` instead of number `12.5` or returning `BadInput`).
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/table.rs:531-535, 569-575
  impl CellPayloadFields {
      fn into_cell(self) -> Result<Cell, EngineError> {
          let dec = parse_decimal_opt(self.decimal_separator.as_deref()).unwrap_or('.');
          ...
      }
  }
  fn parse_decimal_opt(raw: Option<&str>) -> Option<char> {
      match raw?.to_ascii_lowercase().as_str() {
          "." | "dot" | "period" => Some('.'),
          "," | "comma" => Some(','),
          _ => None,
      }
  }
  ```
- **Suggested fix:** Return `Result<Option<char>, EngineError>` from a checked `decimal_separator` parser (matching `parse_explicit_tidy_decimal` / `resolve_decimal_char`) and document accurately whether `"auto"` is supported on cell-level commands vs delimited-text commands.
- **Related:** `crates/engine/src/cmd/table/import.rs:1334-1344` (`parse_explicit_tidy_decimal` rejects unknown decimal strings cleanly).

---

### `FA-SEC12-10` · `S3` · `TransformAnalysisSpec::Simulate` silently ignores non-object, non-null `parameters`/`params` payloads

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/analysis.rs:3058-3063`, `crates/engine/src/cmd/analysis.rs:3874-3892`
- **Category:** `contract-drift`
- **Defect:** In `apply_transform_overrides` for `TransformAnalysisSpec::Simulate`, updating `*parameters` checks `&& !p_val.is_null() && p_val.is_object()` before calling `parse_named_params_vec(Some(p_val))?`. Even though `parse_named_params_vec` explicitly handles `Value::Null` and rejects non-object JSON values with `ErrorCode::BadInput` (`"`params` must be an object of numeric parameter overrides"` at `:3883`), the pre-filter `&& p_val.is_object()` prevents `parse_named_params_vec` from ever rejecting array or primitive payloads.
- **Failure scenario:** Calling `transform.run` or `analysis.updateParams` on a `Simulate` analysis with `"parameters": [1.0, 2.0]` (a positional array format accepted by `table.simulateXy`, for instance) or `"parameters": "invalid"` silently leaves `Simulate.parameters` unchanged and reports success instead of returning `BadInput`.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/analysis.rs:3058-3063
  if let Some(p_val) = first_present(params, &["parameters", "params"])
      && !p_val.is_null()
      && p_val.is_object()
  {
      *parameters = parse_named_params_vec(Some(p_val))?;
  }
  ```
  Compare `parse_named_params_vec` at `crates/engine/src/cmd/analysis.rs:3882-3883`:
  ```rust
  let obj =
      val.as_object().ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`params` must be an object of numeric parameter overrides"))?;
  ```
- **Suggested fix:** Remove `&& p_val.is_object()` at `crates/engine/src/cmd/analysis.rs:3060` so any non-null non-object `parameters`/`params` value hits `parse_named_params_vec`'s `BadInput` error path.
- **Related:** None.

---

### `FA-SEC12-11` · `S4` · `table.setTitles` / `table.setGroupTitles` accepts zero-field no-op invocations as `mutated` and silently drops `title` when `titles` is present

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/table.rs:1635-1714`
- **Category:** `cleanup` / `contract-drift`
- **Defect:** `run_set_titles_with_id` (`table.setTitles` and `table.setGroupTitles`) never verifies that at least one title field (`x_title`, `row_factor_title`, `column_factor_title`, `titles`, `title`, or `row_titles`) was provided, returning `CommandOutcome::mutated(..., { "updated_count": 0 })` (pushing an unnecessary undo snapshot and running `recalculate_dirty_with_links`) on an empty `{ "sheet_id": "s1" }` call. It also silently ignores single `title`+`col` when `titles` is provided (`else if` at `:1683`), and redundantly calls `validate_title` twice on `p.title` (`:1639`, `:1661`) and `validate_row_title_batch` twice on `p.row_titles` (`:1664`, `:1692`).
- **Failure scenario:** Calling `table.setTitles { "sheet_id": "s1" }` with missing/misspelled fields mutates nothing while inflating the undo stack, and supplying both `titles` and `{ col, title }` silently ignores the single-column update.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/table.rs:1676-1713
  if let Some(titles) = p.titles {
      ...
  } else if let Some(title) = p.title.as_deref() {
      ...
  }
  ...
  let recalc = recalculate_dirty_with_links(doc, std::slice::from_ref(&id), false)?;
  Ok(CommandOutcome::mutated(command_id, "updated group titles", json!({ ..., "updated_count": updated, ... })))
  ```
- **Suggested fix:** Require `updated > 0` (or return `BadInput` when no title parameter is supplied), reject mutually conflicting `titles` + `title` combinations explicitly, and remove the duplicate `validate_title` / `validate_row_title_batch` calls.
- **Related:** None.
