# SEC-02 Verification Verdicts — Document Model (`crates/doc`)

- **Base Checked:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-backend` (independent source inspection + cross-crate caller trace + isolated `.lc-worktrees/ver-backend` dynamic probe `ver_sec02_probe`, cleaned up after execution)
- **Source Report:** [SEC-02.md](SEC-02.md) (`FA-SEC02-01`..`FA-SEC02-14`)

## Summary Table

| Verdict | Count | Settled Severity Breakdown |
|---|---:|---|
| `CONFIRMED` | 14 | `S1: 0`, `S2: 4`, `S3: 8`, `S4: 2` |
| `PLAUSIBLE` | 0 | `—` |
| `REFUTED` | 0 | `—` |
| `DUPLICATE` | 0 | `—` |
| **Total** | **14** | **`S1=0, S2=4, S3=8, S4=2`** |

Dynamic reproducer run against an isolated detached worktree at `7c85a8e516165ddadb101236a88eec937bf1f095` (removed cleanly afterward):
- Command: `CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend cargo test --manifest-path .lc-worktrees/ver-backend/Cargo.toml --locked --offline -p curvebook-doc --test ver_sec02_probe -- --nocapture`
- Result: `running 5 tests` -> `test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.14s` (`probe_01_param_spellings_shorthand_polarity`, `probe_02_shift_rows_down_drops_exclusion_note`, `probe_03_04_project_deps_and_image_deserialize`, `probe_05_06_07_08_09_model_and_validation`, `probe_10_11_12_13_14_misc_validation_and_schema`).

---

## File Coverage Completeness Check

Verified `100%` coverage against `crates/doc` at `7c85a8e516165ddadb101236a88eec937bf1f095` (`19` files = `Cargo.toml` + `18` files in `crates/doc/src/`, `15,369` total lines, zero files omitted):

- [Cargo.toml](../../../../crates/doc/Cargo.toml) (`21`), [analysis.rs](../../../../crates/doc/src/analysis.rs) (`2,228`), [descriptor.rs](../../../../crates/doc/src/analysis/descriptor.rs) (`1,295`), [finite.rs](../../../../crates/doc/src/analysis/finite.rs) (`186`), [archetypes.rs](../../../../crates/doc/src/archetypes.rs) (`2,182`), [cell.rs](../../../../crates/doc/src/cell.rs) (`358`), [delimited.rs](../../../../crates/doc/src/delimited.rs) (`319`), [error.rs](../../../../crates/doc/src/error.rs) (`341`), [graph.rs](../../../../crates/doc/src/graph.rs) (`2,102`), [ids.rs](../../../../crates/doc/src/ids.rs) (`257`), [image.rs](../../../../crates/doc/src/image.rs) (`491`), [info.rs](../../../../crates/doc/src/info.rs) (`151`), [layout.rs](../../../../crates/doc/src/layout.rs) (`667`), [lib.rs](../../../../crates/doc/src/lib.rs) (`765`), [links.rs](../../../../crates/doc/src/links.rs) (`695`), [param_spellings.rs](../../../../crates/doc/src/param_spellings.rs) (`1,453`), [project.rs](../../../../crates/doc/src/project.rs) (`444`), [result.rs](../../../../crates/doc/src/result.rs) (`560`), [schema.rs](../../../../crates/doc/src/schema.rs) (`854`).

Every file and line count matches `SEC-02.md`'s File Coverage Table.

---

## Per-Finding Verdicts

### Verdict `FA-SEC02-01` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [analysis_shorthand_alias](../../../../crates/doc/src/param_spellings.rs#L460-L489) (and `#L528-L534`) matches only narrow kind strings (`"pvalue_stack"`, `"glm"`, `"multiple_imputation"`, etc.), missing valid shorthands declared in [descriptor.rs](../../../../crates/doc/src/analysis/descriptor.rs#L837) (`#L866-L867`, `#L1003`) and [engine/src/cmd/analysis.rs](../../../../crates/engine/src/cmd/analysis.rs#L2535-L2603). Meanwhile [apply_glm_overrides](../../../../crates/engine/src/cmd/analysis.rs#L3181) reads `&["response_column"]`, [apply_multivariate_overrides](../../../../crates/engine/src/cmd/analysis.rs#L3242) reads `MULTIVARIATE_RESPONSE_COLUMN`, and archive/batch normalization invokes [canonical_command_param_alias](../../../../crates/format/src/versioned.rs#L273). Dynamic `probe_01`: passing `"linear_glm"`, `"multiple_linear"`, `"logistic_glm"`, `"multiple_logistic"`, `"poisson_glm"`, `"multiple_poisson"`, `"stats.multiple_imputation"`, or `"mice"` to `canonical_command_param_alias("stats.run", &[], Some(kind), "response_column")` returned `Some("mv_response_column")`, while passing `"stats.pvalue_stack"`, `"p_value_stack"`, `"pvalues"`, `"pvaluestack"`, or `"stats.pvaluestack"` inverted `"threshold"` -> `Some("alpha")` and `"multiplicity_method"` -> `Some("multiplicity")`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silently drops/inverts canonical parameters during command normalization, causing wrong default column/method behavior).
- **Still present on main:** yes

### Verdict `FA-SEC02-02` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [shift_rows_down](../../../../crates/doc/src/archetypes.rs#L1983-L2045) shifts only `TableArchetype` cell buffers, leaving `DataSheet::exclusion_notes` and `linked_regions` at pre-shift row coordinates, while [DataSheet::set_cell](../../../../crates/doc/src/links.rs#L322-L330) deletes `exclusion_notes` on unexcluded writes. Live caller [place_grid_in_datasheet](../../../../crates/format/src/csv.rs#L1350-L1366) calls `target.archetype.shift_rows_down(...)` followed by `target.set_cell(...)` without shifting `exclusion_notes`; [import.rs](../../../../crates/transform/src/import.rs#L591-L598) manually updates `exclusion_notes` while leaving `linked_regions` unshifted. Dynamic `probe_02`: after shifting row `0` (marked excluded with a `CellExclusionNote`) down to row `1`, overwriting row `0` via `set_cell(..., Cell::number(99.0))` cleared `ds.exclusion_notes` completely.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent audit metadata loss and save/open `validate_exclusion_notes` failures in `crates/format`).
- **Still present on main:** yes

### Verdict `FA-SEC02-03` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [ProjectDocument::upstream_dependencies](../../../../crates/doc/src/project.rs#L340-L410) explicitly executes `deps.retain(|d| d != id);` at [line 406](../../../../crates/doc/src/project.rs#L406), rendering the explicit `if &dep == id` guard in [dag.rs](../../../../crates/engine/src/dag.rs#L277-L286) dead code and hiding self-edges from dependency queries. Dynamic `probe_03_04`: adding a `ResultSheet` `"r1"` with `source_sheet_id = "r1"` yielded `doc.upstream_dependencies(&SheetId::new("r1")).is_empty()`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (breaks `upstream_dependencies` self-cycle contract for DAG/delete/inspect callers).
- **Still present on main:** yes

### Verdict `FA-SEC02-04` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [ProjectDocument::ensure_can_insert](../../../../crates/doc/src/project.rs#L139-L162) (`#L58`) enforces `MAX_SHEETS_PER_PROJECT` and derives raw `Deserialize` without calling `validate_project_cells` (`MAX_PROJECT_CELLS = 50_000_000`), and workspace inspection confirms [validate_project_document_inner](../../../../crates/format/src/archive.rs#L2115-L2174) also omits `validate_project_cells`. Additionally, [serde_base64_bytes::deserialize](../../../../crates/doc/src/image.rs#L353-L379) on [ProjectImageAsset](../../../../crates/doc/src/image.rs#L413-L425) allocates `String` and `Vec::with_capacity((raw.len() / 4) * 3)` before enforcing `MAX_IMAGE_ASSET_BYTES`, dimensions, or `asset_id` syntax. Dynamic `probe_03_04`: deserializing a `ProjectImageAsset` with `bytes > MAX_IMAGE_ASSET_BYTES`, `asset_id = "invalid asset id!"`, and `pixel_size = [0, 0]` succeeded via `serde_json::from_str`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (unbounded project-wide cell accumulation and unbounded pre-validation base64 decode allocation).
- **Still present on main:** yes

### Verdict `FA-SEC02-05` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [VariableColumn::recode_levels](../../../../crates/doc/src/archetypes.rs#L1447-L1467) checks `current == *from` without `.trim()` and writes untrimmed `to.clone()`, and [ordered_levels_with](../../../../crates/doc/src/archetypes.rs#L1514-L1529) inserts untrimmed `level.as_str()` keys for `Explicit`, diverging from [LevelOrderSpec::recode_levels](../../../../crates/doc/src/archetypes.rs#L1348-L1355). Dynamic `probe_05_06_07_08_09`: `recode_levels(&[(" Control ", " Vehicle ")])` returned `0`, `recode_levels(&[("Control", " Vehicle ")])` stored `" Vehicle "`, and `Explicit([" Alpha ", " Zebra "])` kept `"Zebra"` ordered ahead of `"Alpha"`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (whitespace asymmetry breaks level matching and ordering parity between cells and metadata).
- **Still present on main:** yes

### Verdict `FA-SEC02-06` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [PowerAnalysisSpec](../../../../crates/doc/src/analysis.rs#L1397-L1434) and [AnalysisSpec](../../../../crates/doc/src/analysis.rs#L1996-L2066) derive raw `Deserialize` without calling [PowerAnalysisSpec::validate](../../../../crates/doc/src/analysis.rs#L1517-L1603), and cross-crate `rg` shows `PowerAnalysisSpec::validate` has no production callers (`AnalysisSpec::{new, update_kind}` never invoke it). Dynamic `probe_05_06_07_08_09`: `bad_power.validate()` failed while `AnalysisSpec::new`, `update_kind`, and `serde_json::from_str::<PowerAnalysisSpec>` all accepted `alpha = 1.5`, `OneWayAnova + OneSided`, `num_groups = Some(1)`, and `RSquared(1.0)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (model/deserialization contract allows invalid power specs until execution time).
- **Still present on main:** yes

### Verdict `FA-SEC02-07` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [SheetId](../../../../crates/doc/src/ids.rs#L12-L39) derives `#[serde(transparent)] Deserialize` while [DataSheet::validate](../../../../crates/doc/src/links.rs#L283-L289), [InfoSheet::new_preseeded](../../../../crates/doc/src/info.rs#L99-L105), [ResultSheet::new](../../../../crates/doc/src/result.rs#L521-L525), [AnalysisSpec::new](../../../../crates/doc/src/analysis.rs#L2018-L2023), [GraphSheet::validate](../../../../crates/doc/src/graph.rs#L1995-L2015), [LayoutSheet::validate](../../../../crates/doc/src/layout.rs#L599-L625), [ProjectDocument::ensure_can_insert](../../../../crates/doc/src/project.rs#L139-L145), and [register_unique_sheet_id](../../../../crates/format/src/archive.rs#L2241-L2249) check only `!id.as_str().trim().is_empty()`. Dynamic `probe_05_06_07_08_09`: `SheetId::try_new` rejected `"exp:1"`, `"a {{b}}"`, and 257-byte strings, whereas `serde_json::from_str::<SheetId>("\"exp:1\"")`, `InfoSheet::new_preseeded`, and `DataSheet::new` accepted `:` and `\n`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (hook delimiter and byte/control invariants enforced in `try_new` can be bypassed across L1 constructors and deserialization).
- **Still present on main:** yes

### Verdict `FA-SEC02-08` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [ResultSheet::lookup_scalar](../../../../crates/doc/src/result.rs#L549-L559) returns early only on `ResultStatus::Failed { .. }`, omitting `ResultStatus::Dirty`. Dynamic `probe_05_06_07_08_09`: pushing `ResultScalarEntry { key: "EC50", value: 42.0, .. }` into a fresh `ResultSheet` (`status == ResultStatus::Dirty`) returned `Some(42.0)` from `lookup_scalar("EC50", None)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (can expose stale pre-edit scalar results while a result sheet is dirty).
- **Still present on main:** yes

### Verdict `FA-SEC02-09` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [apply_clone_options](../../../../crates/doc/src/links.rs#L646-L655) unconditionally executes `g.subcolumn_titles = (1..=units).map(|i| format!("{}:{i}", g.title)).collect();` inside the `TableArchetype::Nested` arm regardless of `opts.clear_column_titles` or `opts.override_replicates`. Dynamic `probe_05_06_07_08_09`: cloning a `Nested` `DataSheet` with `clear_column_titles: false, override_replicates: None` replaced `["Mouse 1", "Mouse 2"]` with `["A:1", "A:2"]`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (data loss of biological-unit subcolumn headers during `clone_structure` / `clonePipeline`).
- **Still present on main:** yes

### Verdict `FA-SEC02-10` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [XyTable::new_with_x_mode](../../../../crates/doc/src/archetypes.rs#L307-L317) allocates `0`-wide `x_cells` rows for `XColumnMode::GeneratedSequence` (`subcolumn_count() == 0` at `#L231`), so [DataSheet::set_x_or_time_excluded](../../../../crates/doc/src/links.rs#L408-L415) receives `None` from `x_or_time_cell_mut`; meanwhile [XColumnMode::validate](../../../../crates/doc/src/archetypes.rs#L240) and [GroupedTable::validate](../../../../crates/doc/src/archetypes.rs#L876-L885) omit finiteness checks on `GeneratedSequence` `start`/`step` and `row_numeric_spacing` (deferred to [archive.rs](../../../../crates/format/src/archive.rs#L2231-L2236, #L2256-L2259)). Dynamic `probe_10_11_12_13_14`: `set_x_or_time_excluded(0, 0, true)` returned `Err(DocError::CellOutOfBounds { .. })`, `GeneratedSequence { start: f64::NAN, step: f64::INFINITY, .. }` constructed `Ok`, and `GroupedTable` with `row_numeric_spacing = Some(vec![Some(f64::NAN)])` passed `validate()`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (X-cell exclusion fails unconditionally on generated-X sheets and L1 validation accepts non-finite sequence/spacing parameters).
- **Still present on main:** yes

### Verdict `FA-SEC02-11` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [LayoutSpec::Deserialize](../../../../crates/doc/src/layout.rs#L550-L591) checks only `validate_item_count()` without calling `LayoutSpec::validate()`, while `LayoutSheet`, `GraphSpec`, [GraphSheet](../../../../crates/doc/src/graph.rs#L1964-L1979), [InfoSheet](../../../../crates/doc/src/info.rs#L79-L95), and [ResultSheet](../../../../crates/doc/src/result.rs#L489-L517) derive raw `Deserialize`; [InfoConstant::new](../../../../crates/doc/src/info.rs#L64-L74) accepts `ConstantValue::Number(f64::NAN)`; [DataSheet::validate](../../../../crates/doc/src/links.rs#L283-L289) omits cell/note/exclusion validation; and [AxisSpec::validate](../../../../crates/doc/src/graph.rs#L490-L504) / [GraphSpec::validate](../../../../crates/doc/src/graph.rs#L1569-L1718) omit documented bounds (`length_share > 0`, `stroke_width_pt >= 0`, etc.). Dynamic `probe_10_11_12_13_14`: negative `width = -10.0` deserialized into `LayoutSpec` cleanly while failing `.validate()`, `InfoConstant::new("C", ConstantValue::Number(f64::NAN), None)` returned `Ok`, and `GraphSpec` with `stroke_width_pt = -5.0` and `length_share = -1.0` passed `validate()`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (deserialization and L1 validators accept out-of-domain geometry, float, and styling values).
- **Still present on main:** yes

### Verdict `FA-SEC02-12` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [schema.rs](../../../../crates/doc/src/schema.rs#L513-L519, #L630-L786, #L805-L832) emits `SheetId` without `maxLength: 256` or colon/brace/whitespace exclusions, omits `NamedCheckpoint::id` `maxLength` and `image_assets` `maxItems: 256`, and leaves nested table/analysis/graph/layout specs as empty `{}` schemas. Dynamic `probe_10_11_12_13_14`: serialized `project_document_schema()` confirmed missing `maxLength` on `$defs.SheetId`, missing `maxItems` on `properties.image_assets`, and `$defs.DataSheet.properties.archetype == {}`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (JSON Schema contract under-constrains payloads that runtime archive validation rejects).
- **Still present on main:** yes

### Verdict `FA-SEC02-13` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [SummarizedPoint::reconstruct](../../../../crates/doc/src/cell.rs#L325-L338) extracts `mean` via `Cell::active_number` (`!mean.is_finite()` at line `328` is unreachable dead code because `Cell::active_number` already checks `is_finite()`) and maps `n.fract() != 0.0` to [SummaryPointError::SampleSizeBelowOne](../../../../crates/doc/src/error.rs#L124-L128, #L144). Dynamic `probe_10_11_12_13_14`: passing `N = 2.5` returned `SummaryPointError::SampleSizeBelowOne { n: 2.5 }`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (dead branch and misleading error variant text on non-integer `N`).
- **Still present on main:** yes

### Verdict `FA-SEC02-14` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:** [validate_project_cells](../../../../crates/doc/src/error.rs#L97-L103) returns `DocError::TooManyCells { count: total, max: MAX_PROJECT_CELLS }`, formatted by [Display](../../../../crates/doc/src/error.rs#L301) as `"sheet cell count {count} exceeds maximum {max}"`. Dynamic `probe_10_11_12_13_14`: `validate_project_cells(26_000_000, 26_000_000)` formatted `"sheet cell count 52000000 exceeds maximum 50000000"`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (misleading single-sheet wording on project-wide cell budget overflow).
- **Still present on main:** yes
