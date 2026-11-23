# SEC-02 Audit Report — Document Model

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Scope:** `crates/doc/Cargo.toml` (`1` manifest, `21` lines) + all `18` source files in `crates/doc/src/` (`15,348` LOC) = `19` files (`15,369` lines total)
- **Date:** `2026-11-22`
- **Reviewer:** `rev-core` (report-only; line `1..=N` coverage on all `19` files)
- **Summary Counts:** `S1 = 0`, `S2 = 4`, `S3 = 8`, `S4 = 2` (Total: `14` findings)
- **Prior Remediation Check (`docs/audit/local-code-audit-2026-11-18.md`):**
  - Closed findings `#1`–`#126` affecting `crates/doc` (`#4`, `#94`, `#110` docs sync): **0 regressions** (`crates/doc/tests/audit_foundation_doc.rs` passes; `#110` documentation parity verified).
  - Open finding `#127` (`2026-11-20`) concerns engine/UI nonlinear default model choice rather than `crates/doc` schema behavior.

## Baseline Verification Summary

- `cargo test --locked --offline -p curvebook-doc` (with `CARGO_TARGET_DIR=/tmp/cb-sec02-rev`): exited `0`. Observed `0` failures across:
  - Unit tests (`src/lib.rs`): `29` passed, `0` failed, `0` ignored.
  - Integration suites (`tests/`): `48` passed across `9` files (`analysis_descriptors`: `4`, `audit_foundation_doc`: `2`, `audit_r1_s4a`: `2`, `audit_r1_s4b_doc`: `1`, `audit_r1_s4b_doc_fmt`: `3`, `audit_r2_s05_s07`: `8`, `error_and_schema`: `8`, `prop_serde`: `15`, `qol_doc_types`: `5`), `0` failed.
  - Doc-tests (`crates/doc/src/lib.rs`): `1` passed, `0` failed.
- `cargo clippy --locked --offline -p curvebook-doc --all-targets`: exited `0` with zero warnings.
- `cargo check --locked --offline -p curvebook-doc --target wasm32-unknown-unknown`: blocked locally (`E0463: can't find crate for std` — `wasm32-unknown-unknown` target sysroot not installed); source inspection confirms pure-Rust dependencies (`curvebook-math`, `curvebook-palette`, `serde`), zero `unsafe`, zero `chrono`, zero `std::fs`/`std::net`/`std::process`/`SystemTime`/`Instant`, and `usize` budget caps well below `u32::MAX`.

---

## File Coverage Table

| # | File | Lines | Inspected | Concrete Invariants & Dimensions Verified | Findings |
|---|---|---:|---|---|---|
| 1 | `crates/doc/Cargo.toml` | 21 | `1..=21` | `[lints] workspace = true`; runtime deps strictly `curvebook-math`, `curvebook-palette`, `serde` (`wasm32`-clean, no `chrono`, no network/OS deps). | Clean — manifest & lints verified |
| 2 | `crates/doc/src/lib.rs` | 765 | `1..=765` | `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`; public facade exports, `TableKind`/`SheetSection` predicates, and doc unit tests. | Clean — crate safety attributes & re-exports verified |
| 3 | `crates/doc/src/error.rs` | 341 | `1..=341` | Checked caps (`MAX_ROWS = 1_000_000`, `MAX_DATA_SETS = 1_024`, `MAX_SUBCOLUMNS = 512`, `MAX_TITLE_BYTES = 4_096`, `MAX_SHEET_ID_BYTES = 256`, `MAX_SHEETS_PER_PROJECT = 500`, `MAX_SHEET_CELLS = 30_000_000`, `MAX_PROJECT_CELLS = 50_000_000`, `CURRENT_SCHEMA_VERSION = 7`); overflow-checked `validate_cell_grid`/`validate_project_cells`; `TooManyCells` Display wording mixes sheet vs project limits. | `FA-SEC02-13`, `FA-SEC02-14` |
| 4 | `crates/doc/src/ids.rs` | 257 | `1..=257` | `SheetId::try_new` enforces non-empty, `<= 256` bytes, no whitespace/control or `:`, `{{`, `}}`, while transparent `Deserialize` & `SheetId::new` allow invalid IDs; UTF-8 safe `extract_template_hooks`; `SizePt`/`RectPt` derive unvalidated `Deserialize`. | `FA-SEC02-07`, `FA-SEC02-11` |
| 5 | `crates/doc/src/cell.rs` | 358 | `1..=358` | `CellValue::from_finite_or_empty` non-finite sanitization; `parse_import_token` `*` exclusion & locale decimal handling; `SubcolumnMode` bounds; `SummarizedPoint::reconstruct` overflow/variance checks + dead `!mean.is_finite()` & `SampleSizeBelowOne` on fractional `N`. | `FA-SEC02-13` |
| 6 | `crates/doc/src/delimited.rs` | 319 | `1..=319` | UTF-8 BOM stripping, `\r\n`/`\r`/`\n` EOL handling, RFC-4180 quotes/escapes, error line accounting, `format_linked_paste_tsv_cell` tab/newline sanitization & `*` suffixing. | Clean — RFC-4180 scanning & TSV formatting verified |
| 7 | `crates/doc/src/archetypes.rs` | 2,182 | `1..=2,182` | Custom `Deserialize -> validate()` on `DataSetColumn` and all 8 archetypes (`validate_cell_grid` / `validate_sheet_cells`, rectangularity, subcolumn mode checks); `shift_rows_down` omits `exclusion_notes`; `VariableColumn::recode_levels` / `Explicit` omit trimming; `GeneratedSequence` / `row_numeric_spacing` omit finite checks. | `FA-SEC02-02`, `FA-SEC02-05`, `FA-SEC02-10` |
| 8 | `crates/doc/src/info.rs` | 151 | `1..=151` | Case-insensitive `get_constant`/`upsert_constant`/`remove_constant`, `ConstantValue::as_finite_f64`; `InfoSheet` derives raw `Deserialize`, `new_preseeded` skips `SheetId::try_new`, `InfoConstant::new` allows non-finite `ConstantValue::Number`. | `FA-SEC02-07`, `FA-SEC02-11` |
| 9 | `crates/doc/src/image.rs` | 491 | `1..=491` | Header sniffing rejecting SVG/XML, `CropInsetsPt` non-negative finite & sum `< 1.0`, `ImageBorderSpec` check, NIST FIPS 180-4 SHA-256 `content_addressed_asset_id`; `ProjectImageAsset` derives raw `Deserialize` and `serde_base64_bytes` allocates unbounded payload before `MAX_IMAGE_ASSET_BYTES`. | `FA-SEC02-04` |
| 10 | `crates/doc/src/links.rs` | 695 | `1..=695` | `DataSheet::Deserialize -> validate()`, freeze guards, `extract_cell_grid` clamping & bounded `transpose_cell_grid`; `DataSheet::validate` skips cells/`exclusion_notes`/`floating_notes`/`SheetId::try_new`; `set_x_or_time_excluded` fails on 0-wide `GeneratedSequence`; `clone_structure` resets `Nested` `subcolumn_titles`. | `FA-SEC02-02`, `FA-SEC02-07`, `FA-SEC02-09`, `FA-SEC02-10`, `FA-SEC02-11` |
| 11 | `crates/doc/src/result.rs` | 560 | `1..=560` | Legacy-compatible `ErrorCode::from_code_str`, `ResultStatus` serde fallback, structured tabs/comparisons/curves/checklist; `ResultSheet` derives raw `Deserialize` and skips `SheetId::try_new`; `lookup_scalar` returns stale scalars on `ResultStatus::Dirty`. | `FA-SEC02-07`, `FA-SEC02-08`, `FA-SEC02-11` |
| 12 | `crates/doc/src/analysis.rs` | 2,228 | `1..=2,228` | `ConfidenceLevel` serde enforcement, compatibility rules, strict `parse_column_title_numeric_hook`; `PowerAnalysisSpec` & `AnalysisSpec` derive raw `Deserialize` and never run `PowerAnalysisSpec::validate()` on construction or `update_kind`. | `FA-SEC02-06`, `FA-SEC02-07` |
| 13 | `crates/doc/src/analysis/descriptor.rs` | 1,295 | `1..=1,295` | Compile-time `const` bijection and uniqueness checks across all 19 `AnalysisKindId` descriptors (`ALL_ANALYSIS_DESCRIPTORS`); clean lookup helpers. | Clean — descriptor bijection & uniqueness invariants verified |
| 14 | `crates/doc/src/analysis/finite.rs` | 186 | `1..=186` | `AnalysisSpec::first_non_finite_float` traverses all `f64` fields across `ConfidenceLevel` and every `AnalysisKind` payload variant (`MathTransformPreset`, `NormalizeAnchor`, `Simulate`, `NonlinearFitSpec`, `PowerAnalysisSpec`, etc.). | Clean — full `f64` finiteness traversal verified |
| 15 | `crates/doc/src/graph.rs` | 2,102 | `1..=2,102` | `GraphSpec::default_for_data_sheet`, selective style cloning, `point_override_with_subcol`, finite scan, inline hook extraction; raw `Deserialize` skips `validate()` and `validate()` omits documented stroke/margin/radius/tick/axis-segment range bounds. | `FA-SEC02-07`, `FA-SEC02-11` |
| 16 | `crates/doc/src/layout.rs` | 667 | `1..=667` | Physical unit conversions, presets, `MAX_LAYOUT_ITEMS = 64` cap in `add_item` and `LayoutSpec::Deserialize`; `LayoutSpec::Deserialize` skips `LayoutSpec::validate()` geometry/finiteness checks and `LayoutSheet` derives raw `Deserialize`. | `FA-SEC02-07`, `FA-SEC02-11` |
| 17 | `crates/doc/src/project.rs` | 444 | `1..=444` | `MAX_SHEETS_PER_PROJECT = 500` & uniqueness in `add_*_sheet`, cycle-safe `paired_info_sheet_id`, `reorder_sheet` bounds; raw `Deserialize` & missing `MAX_PROJECT_CELLS` check; `upstream_dependencies` strips `d == id` self-edges. | `FA-SEC02-03`, `FA-SEC02-04`, `FA-SEC02-07` |
| 18 | `crates/doc/src/schema.rs` | 854 | `1..=854` | Deterministic `BTreeMap` Draft 2020-12 AST (`project_document_schema()` at `CURRENT_SCHEMA_VERSION = 7`); `SheetId`/`NamedCheckpoint::id` patterns weaker than `try_new`/byte limits, `image_assets` omits `maxItems = 256`, nested sheet specs collapse to `{}`. | `FA-SEC02-12` |
| 19 | `crates/doc/src/param_spellings.rs` | 1,453 | `1..=1,453` | 70 canonical-first spelling tables + command/path alias resolver; `analysis_shorthand_alias` omits accepted GLM, Multiple Imputation, and P-value Stack kind shorthands, inverting `response_column` / `threshold` key polarity. | `FA-SEC02-01` |

---

## Findings (`S1` -> `S4`)

*(No `S1` Critical findings identified in `SEC-02`.)*

### `FA-SEC02-01` · `S2` · `param_spellings::analysis_shorthand_alias` omits valid GLM, Multiple Imputation, and P-value Stack shorthands, inverting canonical parameter polarity

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/param_spellings.rs:460-489, 528-534` (cross-reference `crates/doc/src/analysis/descriptor.rs:837, 866-867, 1003`)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `analysis_shorthand_alias` hardcodes only a subset of the analysis `kind` strings accepted by `AnalysisKindDescriptor` (`descriptor.rs`) and `curvebook-engine` (`crates/engine/src/cmd/analysis.rs:2535-2603`). When any unrecognized alias for GLM, Multiple Imputation, or P-value Stack is passed, the polarity-reversal guards miss and rewrite canonical parameter names to mismatched keys from unrelated analysis families.
- **Failure scenario:** During persisted archive/batch parameter normalization (`crates/format/src/versioned.rs:273-277`, calling `canonical_command_param_alias`):
  1. For `stats.run` / `analysis.add` with `"kind": "linear_glm"`, `"multiple_linear"`, `"logistic_glm"`, `"multiple_logistic"`, `"poisson_glm"`, `"multiple_poisson"`, `"stats.multiple_imputation"`, or `"mice"`, `is_glm_or_mv_model` (`param_spellings.rs:465-479`) returns `false`. Line `528-534` then resolves `"response_column"` via `MV_RESPONSE_COLUMN` (`&["mv_response_column", "response_column"]`) to **`"mv_response_column"`**. Both `apply_glm_overrides` (`crates/engine/src/cmd/analysis.rs:3181`) and `apply_multivariate_overrides` (`crates/engine/src/cmd/analysis.rs:3242`) ignore `"mv_response_column"` and read only `"response_column"` / `MULTIVARIATE_RESPONSE_COLUMN`, silently analyzing the default response column (`0`) instead of the caller's specified column.
  2. For `"kind": "stats.pvalue_stack"`, `"p_value_stack"`, `"pvalues"`, `"pvaluestack"`, or `"stats.pvaluestack"`, `is_pvalue_stack` (`param_spellings.rs:462`) returns `false`, falling into line `485-488` and rewriting canonical `"threshold"` -> `"alpha"` via `MULTIPLICITY_ALPHA` while rewriting `"multiplicity_method"` -> `"multiplicity"` instead of `"method"`.
- **Evidence:**
  ```rust
  // crates/doc/src/param_spellings.rs:460-489, 528-534
  fn analysis_shorthand_alias(kind: Option<&str>, key: &str) -> Option<&'static str> {
      let norm_kind = kind.map(|k| k.trim().to_ascii_lowercase());
      let is_pvalue_stack = norm_kind.as_deref() == Some("pvalue_stack");
      let is_cox = matches!(norm_kind.as_deref(), Some("cox" | "cox_ph" | "cox_proportional_hazards"));
      let is_simulate = matches!(norm_kind.as_deref(), Some("simulate" | "simulate_xy" | "transform.simulate"));
      let is_glm_or_mv_model = matches!(
          norm_kind.as_deref(),
          Some(
              "glm" | "multiple_regression" | "multiple_linear_regression" | "logistic_regression"
                  | "poisson_regression" | "pls" | "pls_regression" | "pca"
                  | "principal_component_analysis" | "multiple_imputation"
          )
      );
      if is_pvalue_stack {
          if let Some(c) = lookup_tables(&[PVALUE_STACK_METHOD, PVALUE_STACK_THRESHOLD, PVALUE_STACK_COLUMN], key) {
              return Some(c);
          }
      } else if norm_kind.is_some() {
          if let Some(c) = lookup_tables(&[MULTIPLICITY, MULTIPLICITY_ALPHA], key) {
              return Some(c);
          }
      }
      ...
      if norm_kind.is_some()
          && !is_glm_or_mv_model
          && !is_cox
          && let Some(c) = lookup_tables(&[MV_GROUP_COLUMN, MV_RESPONSE_COLUMN], key)
      {
          return Some(c);
      }
  ```
  ```rust
  // crates/doc/src/analysis/descriptor.rs:837, 866-867, 1003
  shorthands: &["glm", "linear_glm", "multiple_linear", "logistic_glm", "multiple_logistic", "poisson_glm", "multiple_poisson"],
  ...
  shorthands: &["pca", ..., "multiple_imputation", "stats.multiple_imputation"],
  ...
  shorthands: &["pvalue_stack", "stats.pvalue_stack", "p_value_stack", "pvalues", "pvaluestack", "stats.pvaluestack"],

  // crates/engine/src/cmd/analysis.rs:3181
  apply_usize_field(params, &["response_column"], &mut spec.response_column)?;
  ```
- **Suggested fix:** Derive `is_pvalue_stack`, `is_glm_or_mv_model`, `is_cox`, `is_km_survival`, and `is_simulate` directly from [`crate::analysis::descriptor::find_analysis_descriptor`] (or a single shared helper over `ALL_ANALYSIS_DESCRIPTORS`) plus the UI/engine alias sets (`"mice"`, `"principal_components"`, etc.), and add cross-kind golden tests in `param_spellings::tests` covering all shorthands in `ALL_ANALYSIS_DESCRIPTORS`.
- **Related:** `crates/format/src/versioned.rs:273`, `crates/engine/src/cmd/analysis.rs:2535-2603, 3181, 3242`.

---

### `FA-SEC02-02` · `S2` · `TableArchetype::shift_rows_down` leaves `DataSheet::exclusion_notes` and `linked_regions` unshifted, causing audit metadata loss and save validation errors

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/archetypes.rs:1983-2045`; `crates/doc/src/links.rs:262-280, 322-337`
- **Category:** `data-loss` / `wrong-result`
- **Defect:** `DataSheet` stores row-indexed `exclusion_notes: Vec<CellExclusionNote>` and `linked_regions: Vec<LinkedDataRegion>` outside `TableArchetype`, yet `shift_rows_down` is exposed directly on `TableArchetype` with no `DataSheet`-level wrapper that keeps `exclusion_notes` (and `linked_regions`) synchronized when rows shift downward. Meanwhile `DataSheet::set_cell` deletes any `exclusion_notes` matching newly written unexcluded coordinates (`links.rs:328-330`).
- **Failure scenario:** Verified live in `crates/format/src/csv.rs:1350-1366` (`place_grid_in_datasheet` for `PlacementMode::InsertRows`): when inserting `grid_rows` at `start_row` above an excluded cell that carries a `CellExclusionNote`, `target.archetype.shift_rows_down(start_row, grid_rows, existing_rows)` moves the excluded cell to `row + grid_rows` while `target.exclusion_notes` remains at the old `row`. As `csv.rs:1363` writes new unexcluded cells into `start_row..start_row + grid_rows` via `target.set_cell(...)`, `set_cell` permanently deletes the `exclusion_notes` entry (`links.rs:329`), silently losing the exclusion rationale; if fewer columns were overwritten, the orphaned `CellExclusionNote` points to an unexcluded cell and causes `validate_project_document` (`crates/format/src/archive.rs:2360`) to fail on save/open. Even callers that manually patch `exclusion_notes` (`crates/transform/src/import.rs:591-598`) leave any `DataSheet::linked_regions` below `start_row` unshifted.
- **Evidence:**
  ```rust
  // crates/doc/src/archetypes.rs:1983-1997
  pub fn shift_rows_down(&mut self, start_row: usize, insert_rows: usize, existing_rows: usize) {
      if insert_rows == 0 || start_row >= existing_rows {
          return;
      }
      let Some(end) = existing_rows.checked_add(insert_rows) else {
          return;
      };
      match self { ... } // shifts x_cells/row_titles/cells only
  }

  // crates/doc/src/links.rs:322-330
  pub fn set_cell(&mut self, coord: CellCoord, cell: Cell) -> Result<(), DocError> {
      self.ensure_mutable()?;
      let Some(slot) = self.archetype.data_cell_mut(coord) else { ... };
      *slot = cell;
      if !slot.excluded {
          self.exclusion_notes.retain(|entry| entry.coord != coord);
      }
  ```
  ```rust
  // crates/format/src/csv.rs:1350-1366 (live caller missing exclusion_notes shift)
  if placement.mode == PlacementMode::InsertRows && start_row < existing_rows {
      target.archetype.shift_rows_down(start_row, grid_rows, existing_rows);
  }
  ...
  for (r_off, row) in grid.iter().enumerate() {
      ...
      target.set_cell(coord, cell.clone())?;
  }
  ```
- **Suggested fix:** Add `DataSheet::shift_rows_down(&mut self, start_row: usize, insert_rows: usize, existing_rows: usize) -> Result<(), DocError>` (enforcing `ensure_mutable()`, shifting `self.archetype`, checked-shifting `exclusion_notes` at `row >= start_row`, and adjusting or rejecting intersecting `linked_regions`), and migrate `crates/format/src/csv.rs` and `crates/transform/src/import.rs` to call `DataSheet::shift_rows_down`.
- **Related:** `crates/format/src/csv.rs:1350-1366`, `crates/transform/src/import.rs:591-598`, closed `#77` (`crates/transform/tests/audit_w2_transform_ops.rs:313`).

---

### `FA-SEC02-03` · `S2` · `ProjectDocument::upstream_dependencies` strips self-references (`deps.retain(|d| d != id)`), breaking cycle detection and reference queries

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/project.rs:340-410` (specifically line `406`)
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `ProjectDocument::upstream_dependencies(&self, id: &SheetId)` explicitly deletes self-edges via `deps.retain(|d| d != id);` before returning.
- **Failure scenario:** Verified in `crates/engine/src/dag.rs:277-286`: `topological_order` explicitly iterates `doc.upstream_dependencies(id)` and checks `if &dep == id { return Err(EngineError::new(format!("self-referential dependency detected on sheet `{id}`"))); }`. Because `project.rs:406` filters `d == id` out, that check never fires, `unique_existing` omits `id`, and `dag::topological_order` relies solely on a redundant full-graph re-walk via `curvebook_format::detect_dependency_cycles(doc)` (`dag.rs:317`). All other callers (`crates/engine/src/cmd/project.rs:380` `project.inspect`, `crates/engine/src/cmd/sheet.rs:744-749, 788, 839`) likewise miss self-referential `GraphSheet`, `LayoutSheet`, `ResultSheet`, and `LinkedDataRegion` references.
- **Evidence:**
  ```rust
  // crates/doc/src/project.rs:353-358, 406-409
  } else if let Some(rs) = self.result_sheet(id) {
      deps.push(rs.analysis.source_sheet_id.clone());
      ...
  deps.retain(|d| d != id);
  deps.sort();
  deps.dedup();
  deps
  ```
  ```rust
  // crates/engine/src/dag.rs:277-286
  let deps = doc.upstream_dependencies(id);
  let mut unique_existing: BTreeSet<SheetId> = BTreeSet::new();
  for dep in deps {
      if &dep == id {
          return Err(EngineError::new(format!("self-referential dependency detected on sheet `{id}`")));
      }
  ```
- **Suggested fix:** Remove `deps.retain(|d| d != id);` from `ProjectDocument::upstream_dependencies` (or clearly split `direct_dependencies` preserving self-edges from any self-excluded display helper) so DAG builders and delete/inspect guards observe true self-references.
- **Related:** `crates/engine/src/dag.rs:277-318`, `crates/engine/src/cmd/sheet.rs:744-749, 788, 839`, `crates/format/src/archive.rs:2507-2549`.

---

### `FA-SEC02-04` · `S2` · Missing `MAX_PROJECT_CELLS` (`50_000_000`) enforcement in `ProjectDocument` and unbounded pre-decode allocation in `ProjectImageAsset::Deserialize`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/project.rs:58-87, 139-162`; `crates/doc/src/image.rs:353-356, 369-379, 413-425`
- **Category:** `resources`
- **Defect:** Two cumulative/payload resource ceilings defined by `crates/doc` are unenforced in the model and its deserializers:
  1. `ProjectDocument::ensure_can_insert` (`project.rs:139-151`) and `ProjectDocument::add_data_sheet` (`project.rs:154-162`) enforce `MAX_SHEETS_PER_PROJECT` (`500`), ID non-emptiness, and ID uniqueness, never calling `validate_project_cells` (`MAX_PROJECT_CELLS = 50_000_000`). Furthermore, `ProjectDocument` derives raw `serde::Deserialize` (`project.rs:58`) and defines no `ProjectDocument::validate` method; downstream `curvebook_format::validate_project_document_inner` (`crates/format/src/archive.rs:2115-2174`) also omits `MAX_PROJECT_CELLS`.
  2. `ProjectImageAsset::bytes` (`image.rs:423`) uses `serde_base64_bytes::deserialize` (`image.rs:353-356`), which calls `String::deserialize(deserializer)?` and pre-allocates `Vec::with_capacity((raw.len() / 4) * 3)` (`image.rs:378`) without checking `MAX_IMAGE_ASSET_BYTES` (`32 MiB`), `MAX_IMAGE_DIM_PX` (`16,384 px`), `MAX_IMAGE_PIXELS` (`67,108,864`), or `asset_id` syntax (`is_valid_asset_id`) because `ProjectImageAsset` derives raw `Deserialize` (`image.rs:413`).
- **Failure scenario:** A crafted `ProjectDocument` containing `500` `DataSheet`s of `30_000_000` cells each (`15,000,000,000` `Cell` slots, `300x` `MAX_PROJECT_CELLS`) passes `ProjectDocument::add_data_sheet`, `serde` deserialization, and `format::validate_project_document` (only `engine`'s interactive `sheet.createData`/`duplicate`/`clonePipeline` commands check `validate_project_cells`). Similarly, deserializing an untrusted `ProjectImageAsset` JSON with an oversized inline `"bytes"` base64 string forces full string allocation plus 75%-sized decoded byte allocation before `hydrate_archive_image_assets` or `validate_image_asset` ever inspects it.
- **Evidence:**
  ```rust
  // crates/doc/src/project.rs:58, 139-151
  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct ProjectDocument { ... }
  ...
  fn ensure_can_insert(&self, id: &SheetId) -> Result<(), DocError> {
      if id.as_str().trim().is_empty() { return Err(DocError::EmptyIdentifier); }
      if self.sheet_section(id).is_some() { return Err(DocError::DuplicateSheetId { sheet_id: id.clone() }); }
      let next = self.total_sheet_count() + 1;
      if next > MAX_SHEETS_PER_PROJECT { return Err(DocError::TooManySheets { count: next, max: MAX_SHEETS_PER_PROJECT }); }
      Ok(())
  }

  // crates/doc/src/image.rs:353-356, 378, 413-425
  pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
      let encoded = String::deserialize(deserializer)?;
      decode_base64(&encoded).map_err(D::Error::custom)
  }
  ...
  let mut out = Vec::with_capacity((raw.len() / 4) * 3);
  ...
  #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ProjectImageAsset { ... }
  ```
- **Suggested fix:** Check `validate_project_cells` in `ProjectDocument::add_data_sheet` and in a new `ProjectDocument::validate` (and call it in `ProjectDocument::Deserialize` + `format::validate_project_document_inner`); in `serde_base64_bytes::deserialize`, use a custom `serde::de::Visitor` that bounds encoded input length (`<= MAX_IMAGE_ASSET_BYTES.div_ceil(3) * 4 + padding`) before decoding/allocating, and validate `ProjectImageAsset` fields on `Deserialize`.
- **Related:** `crates/format/src/archive.rs:2115-2174`, `crates/engine/src/lib.rs:739-741`.

---

### `FA-SEC02-05` · `S3` · `VariableColumn::recode_levels` and `ordered_levels_with(Explicit)` omit whitespace trimming, diverging from `LevelOrderSpec` and GLM reference recoding

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/archetypes.rs:1447-1467, 1514-1529`
- **Category:** `wrong-result`
- **Defect:** `cell_categorical_token` (`archetypes.rs:1540-1550`), `LevelOrderSpec::recode_levels` (`archetypes.rs:1348-1355`), and `crates/engine/src/cmd/table.rs:2996-2998` (`recode_reference_pairs`) all normalize categorical tokens via `.trim()`, whereas `VariableColumn::recode_levels` compares `current == *from` without trimming `from` and stores untrimmed `to.clone()` in `CellValue::Text`, and `VariableColumn::ordered_levels_with` inserts untrimmed `level.as_str()` keys into `rank_by_level` for `LevelOrderKind::Explicit`.
- **Failure scenario:** Calling `VariableColumn::recode_levels(&[(" Control ".into(), " Vehicle ".into())])`:
  1. `cell_categorical_token` yields `"Control"` (trimmed), so `current == *from` is `false` — zero data cells are rewritten while `LevelOrderSpec::recode_levels`, `GraphSpec::recode_variable_levels`, and GLM `reference_levels` rewrite `"Control"` -> `"Vehicle"`.
  2. Calling `("A", " B ")` writes untrimmed `" B "` into cells while `LevelOrderSpec` stores `"B"`; and `"A"` -> `" A "` triggers `from != to` so every `"A"` cell is marked changed even though canonical trimmed level is unchanged.
  3. Supplying `LevelOrderKind::Explicit(vec![" B ".into()])` fails to match trimmed `a.level == "B"`, silently sorting `"B"` to the unordered tail.
- **Evidence:**
  ```rust
  // crates/doc/src/archetypes.rs:1348-1355 vs 1453-1464 vs 1515-1518
  // LevelOrderSpec::recode_levels (1348-1349):
  if (target == *from || target.trim() == from.trim()) && from != to {
      target = to.trim().to_owned();
  // VariableColumn::recode_levels (1457-1463):
  for (from, to) in mappings {
      if current == *from && from != to {
          cell.value = CellValue::Text(to.clone());
          changed += 1;
          break;
      }
  }
  // VariableColumn::ordered_levels_with (1516-1518):
  for (pos, level) in order_list.iter().enumerate() {
      rank_by_level.entry(level.as_str()).or_insert(pos);
  }
  ```
- **Suggested fix:** Trim `from` and `to` identically across `VariableColumn::recode_levels`, `LevelOrderSpec::recode_levels`, and `ordered_levels_with` (`Explicit` trimming each token to match `cell_categorical_token`).
- **Related:** `crates/engine/src/cmd/table.rs:2970-3004`.

---

### `FA-SEC02-06` · `S3` · `PowerAnalysisSpec::validate()` is bypassed on `Deserialize`, `AnalysisSpec::{new, update_kind}`, and `format::validate_project_document`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/analysis.rs:1397-1434, 1517-1603, 1996-2035, 2063-2066`
- **Category:** `contract-drift` / `robustness`
- **Defect:** `PowerAnalysisSpec::validate()` (`analysis.rs:1517-1603`) enforces `alpha in (0, 1)`, `target_power in (0, 1)`, tail compatibility (`OneWayAnova`/`ChiSquare` cannot be one-sided), `sample_size > 0`, `allocation_ratio > 0`, `chi_square_df > 0`, `num_groups >= 2`, and effect-size domain bounds (`p1`/`p2 in [0, 1]`, `RSquared in [0, 1)`, `sd > 0`). However, `PowerAnalysisSpec` and `AnalysisSpec` derive raw `Deserialize`, `AnalysisSpec::new` and `AnalysisSpec::update_kind` never invoke `validate()`, and `PowerAnalysisSpec::validate` has zero callers outside `crates/doc/tests/` (neither `crates/format/src/archive.rs:2139-2142` nor `crates/engine/src/cmd/analysis.rs:2871-2924` calls it).
- **Failure scenario:** Invalid finite power specifications (e.g., `alpha: 1.5`, one-sided `OneWayAnova`, `num_groups: Some(1)`, `RSquared(1.0)`, `TwoProportions { p1: -0.1, p2: 0.5 }`) are accepted by `PowerAnalysisSpec` serde deserialization, `AnalysisSpec::new`, `analysis.run`/`updateParams`, and `format::validate_project_document`, failing only later when `curvebook_stats::power::solve_power` runs.
- **Evidence:**
  ```rust
  // crates/doc/src/analysis.rs:1397-1398, 1996-1997, 2063-2066
  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct PowerAnalysisSpec { ... }
  ...
  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct AnalysisSpec { ... }
  ...
  pub fn update_kind(&mut self, new_kind: AnalysisKind) {
      self.auto_title = new_kind.derive_auto_title();
      self.kind = new_kind;
  }
  ```
- **Suggested fix:** Add `AnalysisSpec::validate(&self) -> Result<(), DocError>` calling `PowerAnalysisSpec::validate()` (and title/`SheetId::try_new` checks), enforce it in `PowerAnalysisSpec::Deserialize` / `AnalysisSpec::{new, update_kind}` (returning `Result` or keeping an explicit `validate` entry point called by `format::validate_project_document` and `engine::apply_power_overrides`).
- **Related:** `crates/engine/src/cmd/analysis.rs:2871-2924`, `crates/format/src/archive.rs:2139-2142`.

---

### `FA-SEC02-07` · `S3` · `SheetId::try_new` invariants (`<= 256` bytes, no whitespace/control or `:`, `{{`, `}}`) are bypassed across `Deserialize` and sheet constructors

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/ids.rs:12-39`; `crates/doc/src/links.rs:283-289`; `crates/doc/src/info.rs:99-105`; `crates/doc/src/result.rs:521-525`; `crates/doc/src/analysis.rs:2018-2023, 2078-2082`; `crates/doc/src/graph.rs:1995-2015`; `crates/doc/src/layout.rs:599-625`; `crates/doc/src/project.rs:139-145`
- **Category:** `wrong-result` / `robustness`
- **Defect:** `SheetId::try_new` (`ids.rs:24-39`) enforces non-empty, `<= MAX_SHEET_ID_BYTES` (`256`), no whitespace or control characters, and no hook delimiter tokens (`:`, `{{`, `}}`). Yet `SheetId` derives `#[serde(transparent)] #[derive(Deserialize)]` over `String` (`ids.rs:12-14`), and every sheet constructor/validator (`DataSheet::validate`, `InfoSheet::new_preseeded`, `ResultSheet::new`, `AnalysisSpec::{new, retarget_source}`, `GraphSheet::{new_uninitialized_for_data, validate}`, `LayoutSheet::{new, validate}`, `ProjectDocument::ensure_can_insert`) only checks `!id.as_str().trim().is_empty()`. Downstream `format::register_unique_sheet_id` (`crates/format/src/archive.rs:2241-2249`) shares the same minimal check.
- **Failure scenario:** Deserializing or constructing a sheet with `SheetId::new("exp:1")` or `"foo {{bar}}"` or a 10 KiB whitespace-padded string succeeds through `DataSheet::new`, `serde`, and `validate_project_document`. Any template hook targeting it (e.g., `{{info:exp:1:Kd}}` or `{{result:foo:1:EC50}}`) then splits on `:` (`ids.rs:229`, `crates/chart/src/xy.rs:127`) into 4+ segments and silently fails to resolve `HookSource` (returning `None` and dropping DAG edges).
- **Evidence:**
  ```rust
  // crates/doc/src/ids.rs:12-14, 24-38
  #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
  #[serde(transparent)]
  pub struct SheetId(String);
  ...
  pub fn try_new(id: impl Into<String>) -> Result<Self, DocError> {
      ...
      if s.len() > MAX_SHEET_ID_BYTES { return Err(DocError::InvalidIdentifier { ... }); }
      if s.chars().any(|ch| ch.is_whitespace() || ch.is_control()) { return Err(...); }
      if s.contains(':') || s.contains("{{") || s.contains("}}") { return Err(...); }
      Ok(Self(s))
  }

  // crates/doc/src/links.rs:283-286 (mirrored in info.rs:101, result.rs:522, graph.rs:1996, layout.rs:601,624, project.rs:140)
  pub fn validate(&self) -> Result<(), DocError> {
      if self.id.as_str().trim().is_empty() {
          return Err(DocError::EmptyIdentifier);
      }
  ```
- **Suggested fix:** Add `SheetId::validate(&self) -> Result<(), DocError>` calling the same rules as `try_new`, invoke it across `SheetId::Deserialize` (ensuring third-party project importers sanitize legacy IDs in `serde_json::Value` form prior to typed deserialization) or at minimum across all `validate()` / constructor / `archive::register_unique_sheet_id` boundaries.
- **Related:** `crates/format/src/archive.rs:2241-2249`.

---

### `FA-SEC02-08` · `S3` · `ResultSheet::lookup_scalar` returns stale cached scalars while `ResultStatus::Dirty`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/result.rs:549-559`
- **Category:** `wrong-result`
- **Defect:** `ResultSheet::lookup_scalar` rejects only `ResultStatus::Failed { .. }`, returning stale pre-edit scalar entries from `self.scalars` whenever `self.status` is `ResultStatus::Dirty`.
- **Failure scenario:** When an upstream table is edited or a result is retargeted and marked `ResultStatus::Dirty` (e.g., `crates/engine/src/cmd/sheet.rs:1460`) before recalculation completes or on lazy/partial evaluation, inline hooks (`{{result:...}}` in `crates/chart/src/xy.rs:142-148`, `crates/chart/src/scales.rs:2435-2439`, `crates/ui-egui/src/egui_shell/center_stage.rs:2079`) and `resolve_effective_power_spec` (`crates/engine/src/methods_report.rs:2593-2637`) read outdated pre-edit scalars instead of recognizing the result as pending (unlike `crates/chart/src/equation_box.rs:186` which explicitly checks `ResultStatus::Dirty`).
- **Evidence:**
  ```rust
  // crates/doc/src/result.rs:549-559
  pub fn lookup_scalar(&self, key: &str, data_set_index: Option<usize>) -> Option<f64> {
      if matches!(self.status, ResultStatus::Failed { .. }) {
          return None;
      }
      let needle = key.trim();
      self.scalars
          .iter()
          .find(|s| s.key.trim().eq_ignore_ascii_case(needle) && (data_set_index.is_none() || s.data_set_index == data_set_index))
          .map(|s| s.value)
          .filter(|v| v.is_finite())
  }
  ```
- **Suggested fix:** Guard `matches!(self.status, ResultStatus::Dirty | ResultStatus::Failed { .. })` in `ResultSheet::lookup_scalar` (or document and provide `lookup_scalar_even_if_dirty` only if an explicit stale preview caller genuinely wants cached values).
- **Related:** `crates/chart/src/equation_box.rs:182-188`, `crates/chart/src/xy.rs:140-150`, `crates/engine/src/methods_report.rs:2593-2637`.

---

### `FA-SEC02-09` · `S3` · `DataSheet::clone_structure` unconditionally overwrites `NestedTable` `subcolumn_titles`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/links.rs:646-655`
- **Category:** `data-loss` / `wrong-result`
- **Defect:** In `apply_clone_options` (`DataSheet::clone_structure` / `sheet.clonePipeline`), the `TableArchetype::Nested` branch unconditionally regenerates `g.subcolumn_titles = (1..=units).map(|i| format!("{}:{i}", g.title)).collect();` regardless of `opts.clear_column_titles` and `opts.override_replicates`.
- **Failure scenario:** Cloning a `Nested` pipeline with `clear_column_titles: false` and `override_replicates: None` preserves `g.title` (`Control`, `Treated`) while silently destroying user-entered biological-unit subcolumn headers (`Mouse 1`, `Mouse 2`, `Mouse 3`), replacing them with `"Control:1"`, `"Control:2"`, `"Control:3"`. All other archetypes preserve column headers when `clear_column_titles == false`.
- **Evidence:**
  ```rust
  // crates/doc/src/links.rs:646-656
  for (idx, g) in t.groups.iter_mut().enumerate() {
      if opts.clear_column_titles {
          g.title = default_column_label(idx);
      }
      let units = opts.override_replicates.unwrap_or_else(|| g.biological_unit_count());
      g.subcolumn_titles = (1..=units).map(|i| format!("{}:{i}", g.title)).collect();
      if opts.clear_y_values || opts.override_replicates.is_some() {
          g.cells = vec![vec![Cell::empty(); units]; g.cells.len()];
      }
  }
  ```
- **Suggested fix:** Regenerate `g.subcolumn_titles` only when `opts.clear_column_titles || opts.override_replicates.is_some()` (preserving existing custom labels otherwise, or truncating/extending cleanly if `override_replicates` changes unit count).
- **Related:** `crates/engine/src/cmd/sheet.rs` (`sheet.clonePipeline`).

---

### `FA-SEC02-10` · `S3` · `set_x_or_time_excluded` always fails on constructed `GeneratedSequence` `XyTable`s, and `GeneratedSequence` / `row_numeric_spacing` omit finiteness checks in `doc` validation

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/archetypes.rs:236-242, 307-317, 492-511, 876-885`; `crates/doc/src/links.rs:408-415`
- **Category:** `wrong-result` / `robustness`
- **Defect:** Two inconsistencies affect `XColumnMode::GeneratedSequence` and `GroupedTable`:
  1. `XyTable::new_with_x_mode` allocates `x_cells` with `x_subs = x_mode.subcolumn_count()`, which is `0` for `GeneratedSequence` (`archetypes.rs:231, 307, 317`), even though `XyTable::validate()` (`492-511`) and `materialized_x_cell_at()` (`383-389`) support `0..=1` subcolumns so generated-X rows can carry an exclusion flag (`x_cells[r][0].excluded`). However, `DataSheet::set_x_or_time_excluded` (`links.rs:408-415`) writes through `x_or_time_cell_mut(row, subcol)` without lazily expanding `0`-wide `x_cells` rows to width `1`, unconditionally returning `Err(DocError::CellOutOfBounds)`.
  2. `XColumnMode::validate()` (`archetypes.rs:240`) returns `Ok(())` for `GeneratedSequence { start, step, .. }` without checking `start.is_finite() && step.is_finite()`, and `GroupedTable::validate()` (`archetypes.rs:876-885`) checks `row_numeric_spacing.len()` without verifying that present spacing values are finite (`check_opt_finite` is currently deferred only to `crates/format/src/archive.rs:2231-2236`).
- **Failure scenario:** Calling `table.excludeCells` with X target (`crates/engine/src/cmd/table.rs:1920, 1936, 1949`) on any `XyTable` created with `GeneratedSequence` via `sheet.createData` or `XyTable::new_with_x_mode` always errors with `CellOutOfBounds`. Separately, in-memory `XyTable` / `GroupedTable` constructors/validators accept `NaN` or `±Inf` in `GeneratedSequence` or `row_numeric_spacing` without failing `DataSheet::validate()`.
- **Evidence:**
  ```rust
  // crates/doc/src/archetypes.rs:227-242, 307, 317
  Self::GeneratedSequence { .. } => 0,
  ...
  pub fn validate(&self) -> Result<(), DocError> {
      match self {
          Self::Numbers { subcolumn_mode } => subcolumn_mode.validate(),
          Self::DateOrElapsedTime { unit_label } => validate_title(unit_label),
          Self::GeneratedSequence { .. } => Ok(()),
      }
  }
  ...
  x_cells: vec![vec![Cell::empty(); x_subs]; row_count],

  // crates/doc/src/links.rs:408-415
  pub fn set_x_or_time_excluded(&mut self, row: usize, subcol: usize, excluded: bool) -> Result<(), DocError> {
      self.ensure_mutable()?;
      let Some(slot) = self.archetype.x_or_time_cell_mut(row, subcol) else {
          return Err(DocError::CellOutOfBounds { row, col: 0, subcol });
      };
      slot.excluded = excluded;
      Ok(())
  }
  ```
- **Suggested fix:** Check `start.is_finite() && step.is_finite()` in `XColumnMode::validate()` and element finiteness in `GroupedTable::validate()` (`row_numeric_spacing`), and in `DataSheet::set_x_or_time_excluded` (when `XyTable` uses `GeneratedSequence` and `subcol == 0`, `row < row_count()`) materialize `0`-wide `x_cells` rows to `vec![Cell::empty(); 1]` (or reject X exclusion explicitly if `GeneratedSequence` X cells are strictly non-excludable, aligning docs/tests across `doc`, `engine`, and `transform`).
- **Related:** `crates/engine/src/cmd/table.rs:1920, 1936, 1949`, `crates/format/src/archive.rs:2231-2236, 2256-2259`.

---

### `FA-SEC02-11` · `S3` · L1 model deserialization skips domain validation on `LayoutSpec`, `LayoutSheet`, `GraphSpec`, `GraphSheet`, `InfoSheet`, and `ResultSheet`, and `GraphSpec::validate()` omits documented bounds

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/layout.rs:550-569, 579-591`; `crates/doc/src/graph.rs:490-504, 1216-1235, 1569-1718, 1964-1979`; `crates/doc/src/links.rs:283-289`; `crates/doc/src/info.rs:64-74, 79-95`; `crates/doc/src/result.rs:489-517`
- **Category:** `contract-drift` / `robustness`
- **Defect:** Unlike `DataSetColumn` and all eight `TableArchetype` types (which run `validate()` inside custom `Deserialize`), other validated L1 structs weaken or omit validation on deserialization/construction:
  1. `LayoutSpec::Deserialize` (`layout.rs:550-569`) enforces only `validate_item_count()` (`<= 64`), skipping `LayoutSpec::validate()` (`width`/`height > 0`, `grid_template >= 1`, positive `bounds_pt`, `scale_percent > 0`, `font_size_pt > 0`, crop/border validation, float finiteness); `LayoutSheet`, `GraphSpec`, `GraphSheet`, `InfoSheet`, and `ResultSheet` derive raw `Deserialize` skipping `validate()`.
  2. `InfoConstant::new` (`info.rs:64-74`) accepts non-finite `ConstantValue::Number(f64::NAN / ±Inf)`.
  3. `DataSheet::validate` (`links.rs:283-289`) does not validate cell `CellValue::Number` finiteness, `floating_notes.bounds_pt` finiteness, or `exclusion_notes` consistency (each checked only downstream in `curvebook-format`).
  4. `AxisSpec::validate()` (`graph.rs:490-504`) and `GraphSpec::validate()` (`graph.rs:1569-1718`) do not check multiple explicitly documented bounds, including `AxisSegment::length_share > 0` (`380`), `AxisSpec::stroke_width_pt >= 0` (`409`), `ConnectingLineStyle::stroke_width_pt >= 0` (`568`), `BarWithReplicates::border_width_pt >= 0` (`620`), `SizeChannelMapping::Constant::radius_pt > 0` (`893`), `HeatmapLabelSpec::cell_gap_pt in 0.0..=12.0` (`193-195`), and `PairwiseBracketSpec::{stroke_width_pt >= 0, font_size_pt > 0}` (`988-989`).
- **Failure scenario:** Direct L1 deserializers (`serde_json::from_str::<LayoutSpec>`) or Rust constructors admit zero/negative page sizes, invalid crop/border parameters, negative stroke widths, out-of-range `cell_gap_pt`, or orphaned `exclusion_notes` without error until a downstream engine/format layer inspects them.
- **Evidence:**
  ```rust
  // crates/doc/src/layout.rs:550-569, 579-580
  impl<'de> serde::Deserialize<'de> for LayoutSpec {
      fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> ... {
          ...
          spec.validate_item_count().map_err(serde::de::Error::custom)?; // omits spec.validate()
          Ok(spec)
      }
  }
  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct LayoutSheet { ... }

  // crates/doc/src/links.rs:283-289
  pub fn validate(&self) -> Result<(), DocError> {
      if self.id.as_str().trim().is_empty() { return Err(DocError::EmptyIdentifier); }
      validate_title(&self.title)?;
      self.archetype.validate()
  }
  ```
- **Suggested fix:** Enforce `LayoutSpec::validate()` inside `LayoutSpec::Deserialize`, wire `validate()` inside `GraphSpec`/`GraphSheet`/`LayoutSheet` deserialization (matching `TableArchetype` policy), check `ConstantValue::Number` finiteness in `InfoConstant::new`, add the documented non-negative/positive/interval checks to `AxisSpec::validate` and `GraphSpec::validate`, and add optional `DataSheet::validate_strict` or move `validate_exclusion_notes`/`validate_floating_notes` into `crates/doc` so format/engine share a single validator.
- **Related:** `crates/format/src/archive.rs:2149-2156, 2336-2368, 2413-2427`, `crates/engine/src/cmd/layout.rs:456, 716`, `crates/engine/src/cmd/graph.rs:1442, 1500`.

---

### `FA-SEC02-12` · `S3` · `project_document_schema()` leaves nested sheet/spec structures untyped (`{}`) and encodes weaker `SheetId` / `NamedCheckpoint` / asset constraints than runtime

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/schema.rs:513-519, 630-647, 659-786, 805-832`
- **Category:** `contract-drift`
- **Defect:** `project_document_schema()` documents Draft 2020-12 structural constraints for `ProjectDocument`, yet leaves nearly every nested payload field as `any_json_schema()` (`{}`) and omits multiple runtime limits:
  - `SheetId::json_schema()` (`513-519`) emits only `minLength: 1` and `pattern: r"\S"`, omitting `maxLength: 256` (`MAX_SHEET_ID_BYTES`) and permitting strings with internal spaces, control chars, `:`, `{{`, `}}` that `SheetId::try_new` forbids.
  - `NamedCheckpoint::id` (`632`) specifies no `maxLength`, whereas runtime `validate_title` caps it at `4_096` bytes (`crates/format/src/archive.rs:2193`).
  - `data_sheet_schema`, `info_sheet_schema`, `result_sheet_schema`, `graph_sheet_schema`, `layout_sheet_schema` (`659-772`) leave `archetype`, `constants`, `linked_regions`, `floating_notes`, `exclusion_notes`, `analysis`, `tabs`, `scalars`, `pairwise_comparisons`, `fitted_curves`, and `spec` unconstrained (`any_json_schema()`).
  - `ProjectDocument::image_assets` (`818`) omits `maxItems: 256` (`MAX_PROJECT_IMAGE_ASSETS`).
- **Failure scenario:** External tools validating against the exported `document.schema.json` accept invalid `archetype`/`analysis`/`spec` payloads, oversized `SheetId`s (`> 256` bytes), `SheetId`s with `:`, `{{`, `}}`, or whitespace, and `> 256` `image_assets` that runtime archive readers or constructors reject.
- **Evidence:**
  ```rust
  // crates/doc/src/schema.rs:513-519, 659-675
  fn json_schema() -> SchemaNode {
      string_schema()
          .with_min_length(1)
          .with_pattern(r"\S")
          .with_title("SheetId")
          .with_description("Non-whitespace sheet identifier (trimmed non-empty).")
  }
  ...
  fn data_sheet_schema() -> SchemaNode {
      object_schema(
          vec![
              ("id", ref_schema(SheetId::schema_name())),
              ("title", title_string_schema()),
              ("archetype", any_json_schema()),
              ...
  ```
- **Suggested fix:** Add `maxLength: MAX_SHEET_ID_BYTES` and a strict non-whitespace/no-control/no-`:`/no-`{{`/`}}` regex on `SheetId::json_schema()` (aligned with `try_new`), add `maxLength` on `NamedCheckpoint::id` and `maxItems: 256` on `image_assets`, and progressively model `$defs` for `TableArchetype`, `AnalysisSpec`, `GraphSpec`, `LayoutSpec`, `FloatingNote`, and `CellExclusionNote`.
- **Related:** `crates/format/src/schemas.rs:75`, `crates/doc/tests/audit_r2_s05_s07.rs:75-108`.

---

### `FA-SEC02-13` · `S4` · Dead finiteness check and misnamed `SampleSizeBelowOne` error variant for fractional `N` in `SummarizedPoint::reconstruct`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/cell.rs:325-338`; `crates/doc/src/error.rs:124-128, 144`
- **Category:** `cleanup`
- **Defect:** In `SummarizedPoint::reconstruct`:
  1. Line `325` extracts `mean` via `Cell::active_number`, which already requires `v.is_finite()` (`cell.rs:41, 99-100`), making `if !mean.is_finite()` at line `328` unreachable dead code.
  2. Line `336-338` maps fractional `n` (`n.fract() != 0.0`) onto `SummaryPointError::SampleSizeBelowOne { n }`, which prints `"pre-summarized sample size N must be >= 1, got 2.5"` (`error.rs:144`).
- **Failure scenario:** A user entering `N = 2.5` in a pre-summarized (`MeanSdN` / `MeanSemN` / `MeanCvN`) subcolumn receives a misleading error message stating `N must be >= 1, got 2.5`.
- **Evidence:**
  ```rust
  // crates/doc/src/cell.rs:325-338
  let Some(mean) = subcells.first().and_then(Cell::active_number) else {
      return Ok(None);
  };
  if !mean.is_finite() {
      return Err(SummaryPointError::MissingSpread);
  }
  ...
  if !n.is_finite() || n < 1.0 || n.fract() != 0.0 {
      return Err(SummaryPointError::SampleSizeBelowOne { n });
  }
  ```
- **Suggested fix:** Remove the redundant `if !mean.is_finite()` branch and split `n.fract() != 0.0` into a dedicated `NonIntegerSampleSize { n }` variant (or reword `SampleSizeBelowOne` / message to `"must be an integer >= 1"`).
- **Related:** None.

---

### `FA-SEC02-14` · `S4` · `DocError::TooManyCells` formats project-wide `MAX_PROJECT_CELLS` rejections with `"sheet cell count ..."` text

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/doc/src/error.rs:97-103, 183-189, 301`
- **Category:** `cleanup`
- **Defect:** `validate_project_cells` (`error.rs:97-103`) returns `DocError::TooManyCells { count: total, max: MAX_PROJECT_CELLS }`, which `Display` unconditionally renders as `"sheet cell count {count} exceeds maximum {max}"` (`error.rs:301`), mislabeling a project-wide cumulative budget failure as a single-sheet limit.
- **Failure scenario:** Attempting to create or clone a sheet that would push cumulative project cells past `50_000_000` (e.g., adding a second `26_000_000`-cell sheet) prints `"sheet cell count 52000000 exceeds maximum 50000000"` even though neither sheet violates `MAX_SHEET_CELLS` (`30_000_000`).
- **Evidence:**
  ```rust
  // crates/doc/src/error.rs:97-101, 301
  pub fn validate_project_cells(existing_cells: usize, added_cells: usize) -> Result<usize, DocError> {
      let total = existing_cells.checked_add(added_cells).ok_or(DocError::TooManyCells { count: usize::MAX, max: MAX_PROJECT_CELLS })?;
      if total > MAX_PROJECT_CELLS {
          return Err(DocError::TooManyCells { count: total, max: MAX_PROJECT_CELLS });
      }
      Ok(total)
  }
  ...
  Self::TooManyCells { count, max } => write!(f, "sheet cell count {count} exceeds maximum {max}"),
  ```
- **Suggested fix:** Distinguish project vs sheet cell overflow in `Display` (e.g. compare `max == MAX_PROJECT_CELLS`, or add `DocError::TooManyProjectCells` if breaking-variant changes are acceptable).
- **Related:** `crates/engine/tests/budgets_and_guards.rs:61-77`.
