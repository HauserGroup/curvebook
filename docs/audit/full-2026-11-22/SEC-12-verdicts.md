# SEC-12 Verification Verdicts — Engine Commands (Part 2)

- **Verifier:** `ver-backend-3`
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Report Verified:** [SEC-12.md](../../../docs/audit/full-2026-11-22/SEC-12.md)
- **Date:** `2026-11-22`

---

## Summary Table

| ID | Reported Severity | Verdict | Settled Severity | Summary |
|---|---|---|---|---|
| `FA-SEC12-01` | `S2` | `CONFIRMED` | `S2` | `table.convertArchetype` omits remapping/updating `point_overrides` and ignores `plan.map_one` errors on `spec.multivariate`; precision note on `prune_out_of_bounds_point_overrides` |
| `FA-SEC12-02` | `S2` | `CONFIRMED` | `S2` | All `7` listed table import/generation/simulation commands (`+ importCsv { replace: true }`) bypass `MAX_PROJECT_CELLS = 50_000_000` |
| `FA-SEC12-03` | `S3` | `CONFIRMED` | `S3` | Inline `bytes` (`table.importPrism`) and inline `csv` (`table.importCsv`, `table.importTidyLong`) bypass archive/CSV file byte ceilings |
| `FA-SEC12-04` | `S3` | `CONFIRMED` | `S3` | `table.setCalculatedVariable` converts and overwrites columns covered by active `linked_regions` without checking overlap |
| `FA-SEC12-05` | `S3` | `CONFIRMED` | `S3` | `graph.setAxes` silently no-ops when `axis` is omitted; axis/segment/series patchers silently drop unknown keys and mistyped values |
| `FA-SEC12-06` | `S3` | `CONFIRMED` | `S3` | `layout.setSpec` and `graph.setSpec` drop top-level shorthand fields when `spec` is passed and deserialize `spec` non-strictly |
| `FA-SEC12-07` | `S3` | `CONFIRMED` | `S3` | All `28` `table.*` schemas advertise `"additionalProperties": true`; precision note: `19` reject unknown keys while `9` `#[serde(flatten)]` commands silently drop them |
| `FA-SEC12-08` | `S3` | `CONFIRMED` | `S3` | `table.pasteLink` ignores malformed `link_spec` objects via `if let Ok(...)` and falls back to `source_sheet_id` / `path` |
| `FA-SEC12-09` | `S3` | `CONFIRMED` | `S3` | `CellPayloadFields::into_cell` (`setCell`, `setCells`, `setXCells`) and `table.paste { rows }` silently coerce `"auto"` and invalid `decimal_separator` values to `'.'` |
| `FA-SEC12-10` | `S3` | `CONFIRMED` | `S3` | `TransformAnalysisSpec::Simulate` pre-filter `&& p_val.is_object()` silently drops non-object `parameters`/`params`; precision note on reachable vs already-rejected callers |
| `FA-SEC12-11` | `S4` | `CONFIRMED` | `S4` | `table.setTitles` / `table.setGroupTitles` accepts zero-field no-op calls as `mutated: true` (`updated_count: 0`) and drops `title` when `titles` is provided |

**Totals:** `CONFIRMED = 11`, `PLAUSIBLE = 0`, `REFUTED = 0`, `DUPLICATE = 0` (`S1 = 0`, `S2 = 2`, `S3 = 8`, `S4 = 1`)

---

## File Coverage Completeness Check (`7/7` Files — `PASS`)

Verified exact scope line counts against `7c85a8e516165ddadb101236a88eec937bf1f095` via `wc -l`:

| # | File | Reported LOC | Actual LOC (`wc -l`) | Status |
|---|---|---:|---:|---|
| 1 | `crates/engine/src/cmd/table.rs` | `5,443` | `5,443` | PASS |
| 2 | `crates/engine/src/cmd/table/import.rs` | `1,454` | `1,454` | PASS |
| 3 | `crates/engine/src/cmd/analysis.rs` | `4,406` | `4,406` | PASS |
| 4 | `crates/engine/src/cmd/graph.rs` | `4,147` | `4,147` | PASS |
| 5 | `crates/engine/src/cmd/layout.rs` | `1,548` | `1,548` | PASS |
| 6 | `crates/engine/src/cmd/export.rs` | `1,042` | `1,042` | PASS |
| 7 | `crates/engine/src/cmd/image.rs` | `390` | `390` | PASS |
| **Total** | **`7` files** | **`18,430`** | **`18,430`** | **100% PASS** |

- Prior clean verdicts on `crates/engine/src/cmd/export.rs` (`additionalProperties: false`, PDF/raster limits checked at `:231-430, 580-720`) and `crates/engine/src/cmd/image.rs` (`decode_base64_capped`, `reject_uri_scheme`, `resolve_image_source`, `default_image_bounds` checked at `:1-390`) hold.
- Baseline engine check on `7c85a8e516165ddadb101236a88eec937bf1f095` prior to `SEC-12` probe runs: `cargo test -p curvebook-engine --quiet` (`269 passed; 0 failed; 0 ignored`: `56` unit + `213` integration across `34` binaries + `0` doc-tests).
- Ephemeral dynamic verification suite (`crates/engine/tests/ver_sec12_probe.rs` in an isolated worktree, cleanly deleted afterward): `7 passed; 0 failed; 0 ignored` in `70.95s`.

---

## Per-Finding Verdicts

### Verdict `FA-SEC12-01` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:**
  - Code trace: [table.rs:3585-3604](../../../crates/engine/src/cmd/table.rs#L3585-L3604) (`run_convert_archetype`), [table.rs:3846-3910](../../../crates/engine/src/cmd/table.rs#L3846-L3910) (`from_xy_to_column`), [table.rs:4444-4496](../../../crates/engine/src/cmd/table.rs#L4444-L4496) (`remap_graph_spec_for_conversion`), [table.rs:4588-4621](../../../crates/engine/src/cmd/table.rs#L4588-L4621) (`apply_conversion_dependents`), [project.rs:63-73](../../../crates/engine/src/cmd/project.rs#L63-L73) (`recalculate_dirty_with_links`), [helpers.rs:762-768, 848-862](../../../crates/engine/src/cmd/helpers.rs#L762-L768) (`point_override_bounds_parts`, `prune_out_of_bounds_point_overrides`), and [archetype_convert.rs:653-690](../../../crates/transform/src/archetype_convert.rs#L653-L690).
  - **Precision correction to `SEC-12.md` failure trace:** `SEC-12.md` stated parenthetically *"(nor does `run_convert_archetype` invoke `prune_out_of_bounds_point_overrides`)"* and described single-Y `XY -> Column` as leaving `{ series_index: 0 }` targeting `X` col `0` and leaving dropped-series overrides orphaned. In fact, `run_convert_archetype` calls `recalculate_dirty_with_links` ([table.rs:3602](../../../crates/engine/src/cmd/table.rs#L3602)), which **does** invoke `prune_out_of_bounds_point_overrides(doc)` ([project.rs:70](../../../crates/engine/src/cmd/project.rs#L70)). Because `helpers.rs:763` checks `spec.series.iter().find(|s| s.data_set_index == series_index)`:
    1. On a **1-Y `XY -> Column`** conversion (`Y0`: `data_set_index` `0 -> 1`), `series` has no `data_set_index == 0` entry (X is not a plotted `SeriesSpec`), so `prune_out_of_bounds_point_overrides` **silently deletes** the user's `Y0` point override (`point_overrides.len() == 0` instead of remapping `series_index` to `1` — silent formatting data loss), and dropped-series overrides are similarly pruned.
    2. On a **2-Y `XY -> Column`** conversion (`Y0`: `0 -> 1`, `Y1`: `1 -> 2`), a `PointStyleOverride` on **`Y1` (`series_index: 1`)** passes `point_override_bounds_parts` because `Y0` now occupies `data_set_index == 1`, so `Y1`'s override **silently re-targets and hides/styles `Y0`'s point** instead of `Y1`, while `Y0`'s override (`series_index: 0`) is silently deleted.
    3. On **`MultipleVariables -> Grouped`** conversion (`mv` 3 cols with col `0` Categorical collapsed to `1` Grouped col), `spec.multivariate` (`table.rs:4468-4495`) ignores `plan.map_one(...)` errors and leaves `mv_spec.y_column == 2` pointing out of bounds.
  - Dynamic probe (`probe_01_convert_archetype_corrupts_point_overrides_and_multivariate`): **ok** — confirmed 1-Y deletion, 2-Y `Y1 -> Y0` silent mis-targeting (`g2.spec.series[1].data_set_index == 2` while `g2.spec.point_overrides[0].series_index == 1`), and stale `mv_spec.y_column == 2`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent data loss and silent mis-application of point-hiding/styling overrides across series on valid archetype conversions).
- **Still present on main:** yes

---

### Verdict `FA-SEC12-02` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:**
  - Code trace: [import.rs:523-535, 633-638, 1143-1158, 1436](../../../crates/engine/src/cmd/table/import.rs#L523-L535) (`import_into_existing_sheet`, `run_import_csv`, `run_import_plate_map`, `run_import_tidy_long`) and [table.rs:3455-3465, 4828-4833, 4910-4943, 5134-5137](../../../crates/engine/src/cmd/table.rs#L3455-L3465) (`run_simulate_xy`, `run_randomize_groups`, `run_random_sample_rows`, `run_simulate_distributions`) directly mutate or `doc.add_data_sheet(...)` without calling `validate_project_cells` (compare `merge_imported_document` at [import.rs:744-746](../../../crates/engine/src/cmd/table/import.rs#L744-L746)).
  - Dynamic probe (`probe_02_seven_table_commands_bypass_max_project_cells`): **ok** — initialized a `49_999_990`-cell document (confirming `sheet.createData` of `20` cells fails with `ErrorCode::ResourceLimit`), then executed all `7` listed commands plus `table.importCsv { replace: true }` sequentially; every invocation succeeded and monotonically increased allocated project cells above `MAX_PROJECT_CELLS = 50_000_000`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2`
- **Still present on main:** yes

---

### Verdict `FA-SEC12-03` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:**
  - Code trace:
    - [import.rs:608-615](../../../crates/engine/src/cmd/table/import.rs#L608-L615) (`run_import_csv`): reads `path` via `policy.read_utf8_file(path, "CSV", MAX_CSV_FILE_BYTES)?`, whereas inline `p.csv` calls `inline.clone()` with no `MAX_CSV_FILE_BYTES` (`64 MiB`) check before parsing.
    - [import.rs:1370-1377](../../../crates/engine/src/cmd/table/import.rs#L1370-L1377) (`extract_tidy_data` for `table.importTidyLong`): identical `inline.clone()` bypass of `MAX_CSV_FILE_BYTES`.
    - [import.rs:800-812](../../../crates/engine/src/cmd/table/import.rs#L800-L812) (`run_import_prism`): enforces `MAX_ARCHIVE_FILE_BYTES` (`128 MiB`) on `p.path` (`:802`) but passes inline `raw_bytes` (`:805`) straight to `import_prism_bytes(&bytes)?` with no `bytes.len() > MAX_ARCHIVE_FILE_BYTES` check; confirmed `ZipArchive::from_bytes_with_limits` ([zip.rs:247-264](../../../crates/format/src/zip.rs#L247-L264)) only caps `max_entries`, `max_total_uncompressed_bytes`, and `max_entry_uncompressed_bytes`, not outer slice length. Cross-ref `FA-SEC11-02` for uncapped `decode_base64` on `bytes_base64`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3`
- **Still present on main:** yes

---

### Verdict `FA-SEC12-04` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:**
  - Code trace: [table.rs:3240-3291](../../../crates/engine/src/cmd/table.rs#L3240-L3291) (`run_set_calculated_variable`) converts `mv.variables[col_idx]` and calls `evaluate_data_sheet_calculated_variables` without calling `ensure_range_outside_linked_regions`.
  - Dynamic probe (`probe_04_set_calculated_variable_overwrites_linked_column`): **ok** — after `table.pasteLink` onto `dst`, `table.setCell` on `(0, 1)` failed with `ErrorCode::Conflict`, whereas `table.setCalculatedVariable { sheet_id: "dst", col: 1, formula: "[Variable A] * 3" }` succeeded, overwrote cell `(0, 1)` to `30.0`, and left `dst.linked_regions.len() == 1`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3`
- **Still present on main:** yes

---

### Verdict `FA-SEC12-05` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:**
  - Code trace: [graph.rs:545-582](../../../crates/engine/src/cmd/graph.rs#L545-L582) (`GraphSetAxesParams`), [graph.rs:1643-1658](../../../crates/engine/src/cmd/graph.rs#L1643-L1658) (`run_graph_set_axes` skips `params.axis_fields` whenever `params.axis` is `None`), and [graph.rs:2563-2651, 2653-2932](../../../crates/engine/src/cmd/graph.rs#L2563-L2651) (`.and_then(Value::as_f64)` / `.and_then(Value::as_u64).and_then(|v| u8::try_from(v).ok())` / `.and_then(Value::as_bool)` / unknown-key ignores in `apply_axis_updates`, `apply_segment_updates`, `apply_single_series_patch`, `apply_line_to_gdo`, `apply_marker_patch`, `patch_error_bars`).
  - Dynamic probe (`probe_05_graph_set_axes_and_patchers_drop_silently`): **ok** — confirmed omitted-`axis` `graph.setAxes` returned `mutated: true` while leaving `y_axis.segments[0]` at `Linear` / `min: None`, and confirmed mistyped/unknown fields (`stroke_width_pt: "2.0"`, `minor_ticks_per_interval: 300`, `bogus_key: 1`, `bind_to_right_y2: "yes"`, `marker: { size_pt: "big", unknown_m: 1 }`) were silently ignored without `BadInput`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3`
- **Still present on main:** yes

---

### Verdict `FA-SEC12-06` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:**
  - Code trace: [layout.rs:220-238, 526-540](../../../crates/engine/src/cmd/layout.rs#L220-L238) (`run_layout_set_spec` skips `apply_layout_spec_fields` whenever `params.spec.is_some()` and calls non-strict `serde_json::from_value::<LayoutSpec>`) and [graph.rs:512-541, 1556-1571](../../../crates/engine/src/cmd/graph.rs#L512-L541) (`run_graph_set_spec` ignores top-level shorthand keys outside the 5 special-cased overrides when `params.spec.is_some()` and calls non-strict `serde_json::from_value::<GraphSpec>`).
  - Dynamic probe (`probe_06_set_spec_drops_shorthand_and_unknown_nested_fields`): **ok** — confirmed `layout.setSpec` with `{ spec: { align_plot_origins: false, bogus_nested: 123 }, width: 555.0 }` kept `width != 555.0` and accepted `bogus_nested`, and `graph.setSpec` with `{ spec: { title: "Updated", bogus_nested: 1 }, legend: { visible: false } }` applied `"Updated"` while leaving `legend.visible == true` and dropping `bogus_nested`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3`
- **Still present on main:** yes

---

### Verdict `FA-SEC12-07` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:**
  - Code trace: [table.rs:808..=5095](../../../crates/engine/src/cmd/table.rs#L808), [import.rs:131..=1271](../../../crates/engine/src/cmd/table/import.rs#L131), [params.rs:527-656](../../../crates/engine/src/cmd/params.rs#L527-L656) (`StrictValueDeserializer`).
  - **Precision correction to `SEC-12.md` runtime claim:** `SEC-12.md` claimed that because all `28` `table.*` schemas declare non-empty `properties` without catch-all maps, `StrictValueDeserializer` rejects undeclared keys across **all `28`** commands. In reality, Serde emits `Deserializer::deserialize_map` (and drops leftover fields via internal `Content` buffering without calling `StrictValueDeserializer::deserialize_ignored_any`) whenever a parameter struct uses `#[serde(flatten)]`:
    - **Only `19` of the `28` `table.*` commands** reject top-level `"bogus": 1` with `ErrorCode::BadInput` (`unknown field \`bogus\``) while advertising `"additionalProperties": true`.
    - **The remaining `9` `table.*` commands** — `table.setCell`, `table.setXCells`, `table.excludeCells`, `table.excludeCellsWithReason`, `table.clearCells`, `table.paste`, `table.pasteLink`, `table.importCsv`, `table.importTidyLong` (plus `CellEditItem` inside `table.setCells.edits`) — contain `#[serde(flatten)]` and **silently ignore top-level `"bogus": 1`** at runtime instead of rejecting it (only matching `additionalProperties: true` by accident of Serde's `flatten` deserializer while hiding misspelled parameters).
  - Dynamic probe (`probe_07_all_28_table_schemas_advertise_additional_properties_true_vs_runtime`): **ok** — iterated all `28` `table.*` commands, verified `"additionalProperties": true` on all `28` schemas, confirmed exact partition (`19` rejected `"bogus": 1` with `ErrorCode::BadInput` `"unknown field \`bogus\`"`, `9` flattened commands + `table.setCells` `edits[0]` silently accepted `"bogus": 1` with `Ok`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (contract drift between Draft 2020-12 schemas and runtime deserialization; fix should set `additional_properties = false` on closed schemas and avoid accidental key drops from `#[serde(flatten)]`).
- **Still present on main:** yes

---

### Verdict `FA-SEC12-08` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:**
  - Code trace: [import.rs:252-266, 295](../../../crates/engine/src/cmd/table/import.rs#L252-L266) (`build_paste_link_spec`).
  - Dynamic probe (`probe_08_09_10_11_paste_link_decimal_sep_simulate_params_and_titles`): **ok** — confirmed `table.pasteLink { target_sheet_id: "dst", link_spec: { bad: true } }` ignores the deserialization error and fails with `"table.pasteLink requires \`link_spec\`, \`source_sheet_id\`, or \`path\`"` (exact wording at `:295`, slightly paraphrased in `SEC-12.md:276`), while supplying `source_sheet_id: "src"` alongside `link_spec: { bad: true }` silently drops `link_spec` and creates the fallback link.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3`
- **Still present on main:** yes

---

### Verdict `FA-SEC12-09` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:**
  - Code trace: [table.rs:531-535, 569-575, 897-899, 1129, 1253, 1372, 2588](../../../crates/engine/src/cmd/table.rs#L531-L535) (`CellPayloadFields::into_cell`, `parse_decimal_opt`, `decimal_sep_schema`, `run_paste` `rows` branch).
  - Dynamic probe (`probe_08_09_10_11_paste_link_decimal_sep_simulate_params_and_titles`): **ok** — confirmed `table.setCell` with `decimal_separator: "auto"` (`"12,5"`) and `"commma"` (`"34,5"`), plus `table.paste` with `{ rows: [["56,75"]], decimal_separator: "auto" }`, all silently coerced to `'.'` and stored text cells (`Cell::text("12,5")`, `Cell::text("34,5")`, `Cell::text("56,75")`) instead of `f64` or `ErrorCode::BadInput`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3`
- **Still present on main:** yes

---

### Verdict `FA-SEC12-10` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:**
  - Code trace: [analysis.rs:1960-1975, 2005-2022, 2328-2335, 2448-2463, 2701-2706, 3058-3063, 3442-3476, 3874-3892](../../../crates/engine/src/cmd/analysis.rs#L3058-L3063).
  - **Precision correction to `SEC-12.md` failure scenario & suggested fix:**
    1. `SEC-12.md` claimed calling `transform.run` or `analysis.updateParams` with `"parameters": [1.0, 2.0]` silently leaves `Simulate.parameters` unchanged. Dynamic testing + code tracing established that:
       - `transform.run` with shorthand `"kind": "simulate"` calls `parse_named_params_vec` directly at [`analysis.rs:2461`](../../../crates/engine/src/cmd/analysis.rs#L2461) and **already rejects** `"parameters": [1.0, 2.0]` with `ErrorCode::BadInput` (`"`params` must be an object of numeric parameter overrides"`).
       - `analysis.updateParams` **also already rejects** both `"params": { "parameters": [1.0, 2.0] }` (via `patch_analysis_kind_fields` at [`analysis.rs:3466-3475`](../../../crates/engine/src/cmd/analysis.rs#L3466-L3475), because `"parameters"` is not in `SHORTHAND_OVERRIDDEN_PATCH_KEYS` and fails `Vec<(String, f64)>` deserialization) and `"params": { "params": [1.0, 2.0] }` (via `validate_map_keys_for_kind` at [`analysis.rs:1969, 2021`](../../../crates/engine/src/cmd/analysis.rs#L2021)).
       - **The reachable silent-ignore path** is **`transform.run` with a typed JSON object `"kind": { "Transform": { "Simulate": { ... } } }`** paired with top-level `"parameters": [1.0, 2.0]` (or `"params": [1.0, 2.0]`), which calls `apply_shorthand_kind_overrides` at [`analysis.rs:2333`](../../../crates/engine/src/cmd/analysis.rs#L2333) without `patch_analysis_kind_fields`, passes `KeyValidationMode::Create`, and silently discards `[1.0, 2.0]` at `:3060`.
    2. Note on fix design: naively removing `&& p_val.is_object()` at `:3060` without checking tuple-array format or adding `"parameters"` to `SHORTHAND_OVERRIDDEN_PATCH_KEYS` would break valid `analysis.updateParams { params: { parameters: [["freq", 2.0]] } }` calls (`patch_analysis_kind_fields` `:2703` accepts `Vec<(String, f64)>`, then `:2704` would reject it).
  - Dynamic probe (`probe_08_09_10_11_paste_link_decimal_sep_simulate_params_and_titles`): **ok** — verified all `3` rejections (`transform.run { kind: "simulate", parameters: [1.0, 2.0] }`, `analysis.updateParams { params: { parameters: [1.0, 2.0] } }`, `analysis.updateParams { params: { params: [1.0, 2.0] } }`) and verified `transform.run` with object `kind` + `"parameters": [1.0, 2.0]` silently succeeds via `:2333 -> :3060`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (narrow robustness/contract defect with workaround and latent hazard per `docs/audit/README.md:107`; only affects typed-object `transform.run`, while shorthand `transform.run` and `analysis.updateParams` already reject invalid payloads).
- **Still present on main:** yes

---

### Verdict `FA-SEC12-11` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:**
  - Code trace: [table.rs:1635-1714](../../../crates/engine/src/cmd/table.rs#L1635-L1714) (`run_set_titles_with_id`).
  - Dynamic probe (`probe_08_09_10_11_paste_link_decimal_sep_simulate_params_and_titles`): **ok** — confirmed `table.setTitles { sheet_id: "src" }` returns `mutated: true` with `updated_count: 0`, and `table.setTitles { sheet_id: "src", titles: ["Kept"], col: 0, title: "Dropped" }` updates column `0` to `"Kept"` and silently ignores `title: "Dropped"`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4`
- **Still present on main:** yes
