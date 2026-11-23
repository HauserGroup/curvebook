# SEC-11 Audit Report — Engine Core, DAG & Commands (Part 1)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Scope:** `crates/engine/Cargo.toml` (`1` manifest, `38` lines) + `5` files in `crates/engine/src/` (`6,284` lines) + `9` files in `crates/engine/src/cmd/` (`7,435` lines) = `15` files (`13,757` lines total; leaving `cmd/table.rs`, `cmd/table/import.rs`, `cmd/analysis.rs`, `cmd/graph.rs`, `cmd/layout.rs`, `cmd/export.rs`, and `cmd/image.rs` for `SEC-12`)
- **Date:** `2026-11-22`
- **Reviewer:** `rev-engine` (report-only; line `1..=N` coverage on all `15` files)
- **Summary Counts:** `S1 = 0`, `S2 = 3`, `S3 = 6`, `S4 = 1` (Total: `10` findings)
- **Prior Remediation Check (`docs/audit/local-code-audit-2026-11-18.md`):**
  - Closed findings `#1`–`#126` affecting `SEC-11` (`#1`, `#2`, `#6`, `#70`, `#105`–`#114`, `#118`–`#122`): **0 regressions** verified across `dag.rs`, `lib.rs`, `methods_report.rs`, `cmd/effects.rs`, `cmd/project.rs`, and `cmd/sheet.rs` (`tests/audit_engine_core_fixes.rs` and `tests/audit_methods_report_fixes.rs` pass).
  - Open finding `#127` (`2026-11-20`) concerns nonlinear fit default model selection outside `SEC-11` scope.

## Baseline Verification Summary

- `cargo clippy --locked --offline -p curvebook-engine --all-targets -- -D warnings` (with `CARGO_TARGET_DIR=/tmp/cb-sec11-rev`): exited `0` with zero Clippy lint warnings; Cargo emitted `warning: unused dependency 'curvebook-tools'` on `crates/engine/Cargo.toml:30:1` (tracked in `FA-SEC11-10`).
- `cargo test --locked --offline -p curvebook-engine --quiet` (with `CARGO_TARGET_DIR=/tmp/cb-sec11-rev`): exited `0`. Observed `269` passed, `0` failed, `0` ignored across `35` test runs:
  - Unit tests (`src/lib.rs`): `56` passed, `0` failed, `0` ignored.
  - Integration suites (`crates/engine/tests/`): `213` passed across `34` test binaries (`11 + 16 + 9 + 3 + 5 + 4 + 2 + 6 + 19 + 4 + 11 + 7 + 4 + 9 + 6 + 11 + 4 + 10 + 3 + 3 + 6 + 4 + 4 + 1 + 3 + 4 + 4 + 3 + 10 + 2 + 5 + 10 + 6 + 4`), `0` failed, `0` ignored.
  - Doc-tests (`curvebook_engine`): `0` tests.
- `rustup target list --installed`: only `aarch64-apple-darwin` installed locally (`wasm32-unknown-unknown` target sysroot unavailable in this environment); source inspection verifies `#![forbid(unsafe_code)]`, zero `chrono` imports, and `#[cfg(target_arch = "wasm32")]` guards across `lib.rs` and `path_policy.rs`, except `cmd/helpers.rs::write_output_file` (`FA-SEC11-09`).

---

## File Coverage Table

| # | File | Lines | Inspected | Concrete Invariants & Dimensions Verified | Findings |
|---|---|---:|---|---|---|
| 1 | `crates/engine/Cargo.toml` | 38 | `1..=38` | `[lints] workspace = true`; `bundled-fonts` default feature; runtime deps vs actual crate imports; unused `curvebook-tools` warning emitted on build/clippy. | `FA-SEC11-10` |
| 2 | `crates/engine/src/dag.rs` | 1,472 | `1..=1,472` | `topological_order` cycle detection; scoped `compute_dirty_closure` & working-clone atomic swap; `recompute_result_sheet` clears stale outputs while preserving `floating_notes`; `resolve_refresh_source` enforces `ExternalPathPolicy` and `check_upstream_result_status` (`R1-X2-01`). | Clean — topological ordering, cycle rejection, failure isolation, and link refresh verified |
| 3 | `crates/engine/src/guard.rs` | 101 | `1..=101` | `run_guarded` wraps command/DAG callbacks in `catch_unwind(AssertUnwindSafe(...))`, extracting `&str`, `String`, whitespace, and non-string panic payloads into `ErrorCode::InternalPanic`. | Clean — panic boundary & error mapping verified |
| 4 | `crates/engine/src/lib.rs` | 1,563 | `1..=1,563` | `#![forbid(unsafe_code)]`; `GuiOnlyPolicy::Strict` & `ExternalPathPolicy` enforcement before dispatch; `MAX_UNDO_ENTRIES = 64` + cell/checkpoint-bytes budget eviction; atomic rollback of `doc`/`file_path`/`dirty` on errors; `project.open` clobbered by `self.dirty = true`; `command_requires_wal_checkpoint` skips wall-clock timestamp commands. | `FA-SEC11-04`, `FA-SEC11-05` |
| 5 | `crates/engine/src/methods_report.rs` | 2,754 | `1..=2,754` | PlainText/Markdown/LaTeX renderers (`escape_latex_text`, `escape_latex_inline_text`, `escape_latex_math_mode`, `[` row guard); `resolve_effective_power_spec` info overrides; `describe_bland_altman` + `column_title_at` X-vs-Y[0] misattribution; raw `{hook:?}`/`{h:?}` Debug formatting of `HookSource`. | `FA-SEC11-01`, `FA-SEC11-07` |
| 6 | `crates/engine/src/path_policy.rs` | 394 | `1..=394` | `ExternalPathPolicy` (`Unconstrained`, `Workdir`, `RejectExternal`), `ENGINE_PATH_PARAM_KEYS` traversal before dispatch, regular-file & size-capped reads, `write_atomic_archive_bytes` `ensure_existing_archive_not_newer` forward-version check, `wasm32` `UnsupportedFormat`/`PathEscape` gates. | Clean — path containment, symlink/FIFO rejection, forward-version check, and `wasm32` guards verified |
| 7 | `crates/engine/src/cmd/mod.rs` | 380 | `1..=380` | `CommandGroup` ordering, `Caller`, `Ctx`, `CommandSpec`, `Registry::from_specs` empty/duplicate ID rejection, `build_registry` completeness, markdown registry docs. | Clean — registry invariants & duplicate/empty ID checks verified |
| 8 | `crates/engine/src/cmd/edit.rs` | 488 | `1..=488` | `edit.undo`, `edit.redo`, `edit.pasteResultTable`, `edit.pasteImage`: closed schemas (`additionalProperties: false`) + `#[serde(deny_unknown_fields)]`, `MAX_LAYOUT_ITEMS` check, accurate `Effects` (`READS_FILESYSTEM` on `edit.pasteImage`). | Clean — edit schemas, undo/redo no-op behavior, and effects verified |
| 9 | `crates/engine/src/cmd/effects.rs` | 196 | `1..=196` | 7-flag `Effects` bitmask, `strict_gui_only_allowed`, `destructive_hint` (including `WRITES_FILESYSTEM`), `read_only_hint` (excluding `CHANGES_UI_STATE`), `idempotent_hint` combinators & unit assertions. | Clean — effect bitmask & safety/MCP hint predicates verified |
| 10 | `crates/engine/src/cmd/helpers.rs` | 959 | `1..=959` | `remap_graph_spec_references`, `sanitize_title_controls`, `clip_safe_title`, `parse_color_string`; `write_output_file` bare non-atomic `std::fs::write` bypassing `ExternalPathPolicy`; `decode_base64` lacking byte budget and accepting empty payloads. | `FA-SEC11-02`, `FA-SEC11-09` |
| 11 | `crates/engine/src/cmd/info.rs` | 348 | `1..=348` | `info.setNote`, `info.setConstant`, `info.removeConstant`: closed schemas (`obj_schema_with_aliases(..., false)`), finite checks, optional `units` preserve/clear semantics, `recalculate_dirty_with_links` propagation. | Clean — info note/constant schemas, validation, and DAG recalc verified |
| 12 | `crates/engine/src/cmd/params.rs` | 1,340 | `1..=1,340` | `parse_params_with_warnings`, panic-safe `DeprecationScope`, `StrictValueDeserializer` with exact RFC-6901 JSON-pointer errors, `is_open_catchall_schema` rule (`props_empty && additional_true`). | Clean — strict schema validation, RFC-6901 pointers, and alias canonicalization verified |
| 13 | `crates/engine/src/cmd/project.rs` | 1,210 | `1..=1,210` | `12` `CommandSpec`s (`additionalProperties: false`), `MAX_CHECKPOINTS_PER_PROJECT` / `MAX_TOTAL_CHECKPOINT_BYTES` caps, ISO-8601 UTC / `unix:` timestamp validation, forward-version save refusal; uncapped `decode_base64` callers, `open`/`save` dirty state drift, `createCheckpoint` uncheckpointed timestamp fallback. | `FA-SEC11-02`, `FA-SEC11-04`, `FA-SEC11-05` |
| 14 | `crates/engine/src/cmd/sheet.rs` | 2,447 | `1..=2,447` | `11` `CommandSpec`s, `validate_project_cells` on create/duplicate/clone, freeze/cascade conflict checks, cycle guards; `extractResultTab` missing `MAX_PROJECT_CELLS` check, `clone_downstream_graphs` annotation/title-hook omission, 9 schema helpers advertising `additionalProperties: true` vs strict `parse_params`. | `FA-SEC11-03`, `FA-SEC11-06`, `FA-SEC11-08` |
| 15 | `crates/engine/src/cmd/spellings.rs` | 67 | `1..=67` | Direct re-export of all `70` canonical-first parameter spelling tables and `ALL` from `curvebook_doc::param_spellings`; `first_present_with_key` precedence & deprecation hit recording. | Clean — zero divergence from L1 `param_spellings` source of truth |

---

## Findings (`S1` -> `S4`)

*(No `S1` Critical findings identified in `SEC-11`.)*

### `FA-SEC11-01` · `S2` · `describe_bland_altman` and `column_title_at` mislabel compared XY columns in X-vs-Y[0] mode

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/methods_report.rs:786-800`, `crates/engine/src/methods_report.rs:2040-2058`
- **Category:** `wrong-result` / `contract-drift`
- **Defect:** `describe_bland_altman` resolves method labels via `column_title_at(source_ds, s.column_a)` and `column_title_at(source_ds, s.column_b)`, while `column_title_at` for `TableArchetype::Xy(t)` unconditionally indexes `t.data_sets.get(col_idx)` (ignoring `t.x_title`). In `crates/stats/src/two_group.rs:2386-2409` (`extract_bland_altman_pairs`), when `ba.xy_compare_x_vs_y.unwrap_or(xy.data_sets.len() == 1)` is `true`, `column_a == 0` reads `xy.x_title` (`X`) and `column_b == 1` reads `xy.data_sets[0].title` (`Y[0]`).
- **Failure scenario:** Running `result.generateReport` on a Bland-Altman analysis over a 1-dataset XY table (`X = "Reference"`, `Y[0] = "Assay"`) or a multi-dataset XY table with `xy_compare_x_vs_y: true` prints `"Method agreement between Assay and Column 2 ..."` (for 1-dataset XY) or `"Method agreement between Assay and Y2 ..."` (for multi-dataset XY) instead of `"Method agreement between Reference and Assay ..."`, publishing misattributed method identities in scientific reports.
- **Evidence:**
  ```rust
  // crates/engine/src/methods_report.rs:786-796
  fn describe_bland_altman(s: &BlandAltmanSpec, source_ds: Option<&DataSheet>, conf_percent: f64) -> Vec<ReportBlock> {
      let col_a = column_title_at(source_ds, s.column_a);
      let col_b = column_title_at(source_ds, s.column_b);
  ...
  // crates/engine/src/methods_report.rs:2040-2046
  fn column_title_at(source_ds: Option<&DataSheet>, col_idx: usize) -> String {
      ...
      let raw = match &ds.archetype {
          TableArchetype::Xy(t) => t.data_sets.get(col_idx).map(|c| c.title.as_str()),
  ```
  Compare `crates/stats/src/two_group.rs:2386-2408`:
  ```rust
  let compare_x_vs_y = ba.xy_compare_x_vs_y.unwrap_or(xy.data_sets.len() == 1);
  if compare_x_vs_y {
      // X vs Y[0] mode: require canonical indices (0, 1) regardless of additional Y datasets.
      ...
      let ds = xy.data_sets.first().ok_or(StatsError::EmptyInput)?;
      ...
      Ok(BlandAltmanExtractedPairs {
          label_a: xy.x_title.clone(),
          label_b: ds.title.clone(),
  ```
- **Suggested fix:** Mirror `extract_bland_altman_pairs` inside `describe_bland_altman`: when `source_ds` is `Some(DataSheet { archetype: TableArchetype::Xy(xy), .. })` and `s.xy_compare_x_vs_y.unwrap_or(xy.data_sets.len() == 1)` holds, resolve `col_a` from `xy.x_title` (fallback `"X"`) and `col_b` from `xy.data_sets.first().map(|c| c.title.as_str())` (fallback `"Column 1"`).
- **Related:** None in `SEC-11`.

---

### `FA-SEC11-02` · `S2` · `cmd::helpers::decode_base64` lacks payload size limits and accepts empty inputs across `bytes_base64` importers

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/helpers.rs:361-391`, `crates/engine/src/cmd/project.rs:162-164`, `crates/engine/src/cmd/project.rs:332-334` (also called at `crates/engine/src/cmd/table/import.rs:804, 865`)
- **Category:** `resources` / `security`
- **Defect:** `cmd::helpers::decode_base64` collects all non-whitespace bytes into `cleaned: Vec<u8>` and allocates `out = Vec::with_capacity(chunks.len().saturating_mul(3))` with no maximum byte budget (`MAX_ARCHIVE_FILE_BYTES` or `MAX_XML_BYTES`), and accepts empty/whitespace-only inputs as `Ok(vec![])`. By contrast, filesystem `{ path }` paths enforce `MAX_ARCHIVE_FILE_BYTES` (`128 MiB`) or `MAX_XML_BYTES` (`64 MiB`) prior to reading (`read_binary_file`), and `cmd/image.rs::decode_base64_capped` enforces `MAX_IMAGE_ASSET_BYTES` and rejects empty payloads.
- **Failure scenario:** Calling `project.open`, `project.instantiateTemplate`, `table.importPrism`, or `table.importPzfx` with a multi-hundred-megabyte or multi-gigabyte `bytes_base64` payload forces `decode_base64` to allocate ~1.75x the payload size across `cleaned` and `out` before format limits or decompression ever run, causing OOM or `wasm32` allocation aborts.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/helpers.rs:361-368
  pub fn decode_base64(input: &str) -> Result<Vec<u8>, EngineError> {
      let cleaned: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
      if !cleaned.len().is_multiple_of(4) {
          return Err(EngineError::with_code(ErrorCode::BadInput, "base64 payload length must be a multiple of 4"));
      }
      let (chunks, _) = cleaned.as_chunks::<4>();
      let mut out = Vec::with_capacity(chunks.len().saturating_mul(3));
  ```
  Compare `crates/engine/src/cmd/project.rs:162-164` (`project.open { bytes_base64 }`) and `330-334` (`project.instantiateTemplate { path }` vs `{ bytes_base64 }`):
  ```rust
  ctx.session.path_policy.read_binary_file(path, "template file", MAX_ARCHIVE_FILE_BYTES)?
  } else if let Some(ref b64) = parsed.bytes_base64 {
      decode_base64(b64)?
  ```
- **Suggested fix:** Add `decode_base64_capped(input: &str, max_bytes: usize, pointer: &str)` (rejecting empty inputs with `ErrorCode::BadInput` and rejecting `cleaned.len() > max_b64_len` / decoded length `> max_bytes` with `ErrorCode::ResourceLimit` before allocating `Vec`), and enforce `MAX_ARCHIVE_FILE_BYTES` on archive `bytes_base64` entrypoints and `MAX_XML_BYTES` on `.pzfx` `bytes_base64`.
- **Related:** `FA-SEC02-04` (`crates/doc/src/image.rs` `serde_base64_bytes` unbounded decode).

---

### `FA-SEC11-03` · `S2` · `sheet.extractResultTab` bypasses the `MAX_PROJECT_CELLS` project cell budget

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/sheet.rs:1918-2019`
- **Category:** `resources`
- **Defect:** `run_sheet_extract_result_tab` allocates a `ColumnTable`, `GroupedTable`, or `MultipleVariablesTable` of `row_count * col_count` cells and appends it to `ctx.session.doc` without calling `validate_project_cells(document_used_cells(&ctx.session.doc), ...)`. Every sibling sheet-creation command (`create_data_with_options` at `:287`, `run_sheet_duplicate` at `:985`, and `clone_family_into_doc` at `:1299`) enforces `MAX_PROJECT_CELLS` (`50_000_000`) via `validate_project_cells`.
- **Failure scenario:** In a project already approaching `MAX_PROJECT_CELLS`, extracting large result tabs (such as Monte Carlo simulations, residuals, or large multi-column tables) repeatedly via `sheet.extractResultTab` exceeds the cumulative 50M-cell ceiling, bypassing undo/snapshot memory protection.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/sheet.rs:1934-1960, 1998
      let row_count = tab.rows.len();
      let col_count = tab.rows.iter().map(Vec::len).max().unwrap_or(0).max(tab.column_headers.len()).max(1);
      ...
      let archetype = match target_kind {
          TableKind::Column => TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, col_count, row_count)?),
          TableKind::Grouped => TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(1), col_count, row_count)?),
          TableKind::MultipleVariables => TableArchetype::MultipleVariables(MultipleVariablesTable::new(col_count, row_count)?),
          ...
      };
      let mut sheet = DataSheet::new(target_id.clone(), title.clone(), archetype)?;
      ...
      ctx.session.doc.add_data_sheet(sheet)?;
  ```
- **Suggested fix:** Compute `added_cells = row_count.checked_mul(col_count)` (with overflow error) and call `validate_project_cells(document_used_cells(&ctx.session.doc), added_cells)?` before allocating/adding the extracted `DataSheet`.
- **Related:** Sibling `FA-SEC02-03` (`ProjectDocument::Deserialize` missing `MAX_PROJECT_CELLS`).

---

### `FA-SEC11-04` · `S3` · `project.open { path }` always leaves `Session::is_dirty() == true` and `project.save` never clears `dirty`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/project.rs:170-172`, `crates/engine/src/cmd/project.rs:215-256`, `crates/engine/src/lib.rs:1406`
- **Category:** `contract-drift`
- **Defect:** `run_project_open` calls `ctx.session.set_file_path(opened_path, false)` (`mark_clean = false`) and returns `CommandOutcome::mutated("project.open", ...)`. Even if `mark_clean = true` were passed inside the handler, `Session::execute_mutating_spec_with_hook` unconditionally sets `self.dirty = true;` at `lib.rs:1406` whenever `res.mutated` is `true`. In parallel, `run_project_save` receives `&mut Ctx<'_>` and writes the project to disk, but returns `CommandOutcome::read_only(...)` without clearing `ctx.session.dirty`.
- **Failure scenario:** Opening an unmodified `.curvebook` archive (or `.curvebook.d/` directory) via `Session::execute("project.open", { "path": ... })` or saving via `project.save` leaves `session.is_dirty() == true`. Callers relying on `Session::is_dirty()` (such as the GUI shell unsaved-changes guard at `crates/ui-egui/src/egui_shell/mod.rs:1873` or `command_run` inspect workflows) falsely treat an untouched opened/saved file as having unsaved edits unless a wrapper (`crates/mcp/src/formats.rs:262, 645`) manually invokes `session.set_file_path(..., true)` afterward.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/project.rs:170-172
      ctx.session.doc = loaded;
      ctx.session.set_file_path(opened_path, false);
      Ok(CommandOutcome::mutated(
          "project.open",
  ```
  ```rust
  // crates/engine/src/lib.rs:1403-1406
      if clears_file_path_on_mutate {
          self.file_path = None;
      }
      self.dirty = true;
  ```
  ```rust
  // crates/engine/src/cmd/project.rs:245-248
      Ok(CommandOutcome::read_only(
          "project.save",
          "saved project",
  ```
- **Suggested fix:** When `project.open` loads from a clean filesystem `path` (not `bytes_base64`), keep `session.is_dirty() == false` by handling clean-load dirty semantics explicitly after commit in `execute_mutating_spec_with_hook` (while rolling back to `before_dirty` on error or undo). For `project.save`, decide and document explicitly whether `project.save { path }` clears `dirty` when writing to the active `file_path` (or provide an engine-level clean-mark path rather than relying solely on MCP wrapper workarounds).
- **Related:** None in `SEC-11`.

---

### `FA-SEC11-05` · `S3` · `project.createCheckpoint` defaults to wall-clock `current_utc_timestamp()` without WAL checkpointing, breaking WAL replay parity

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/lib.rs:920-925`, `crates/engine/src/lib.rs:973-976`, `crates/engine/src/cmd/project.rs:667-684`
- **Category:** `contract-drift` / `data-loss` (WAL replay parity)
- **Defect:** `Session::command_requires_wal_checkpoint` returns `true` only for `"edit.undo"`, `"edit.redo"`, and `Effects::READS_FILESYSTEM`. Meanwhile `execute_with_caller_and_record` journals the caller's alias-normalized input JSON (`journal_params`), and `run_project_create_checkpoint` (`Effects::MUTATES_DOCUMENT`) calls `current_utc_timestamp()` whenever `timestamp_utc` is omitted.
- **Failure scenario:** Calling `session.execute_and_record(&mut journal, "project.createCheckpoint", json!({ "name": "Baseline" }))` appends `{ "command_id": "project.createCheckpoint", "params": { "name": "Baseline" } }` to `ops.journal`. When a crash recovery session replays the WAL seconds or hours later (`replay_recovered_commands`, `lib.rs:1042`), `project.createCheckpoint` mints a different `created_utc` (`unix:<new_secs>`), which also propagates into the `snapshot_json` of any subsequent checkpoint, breaking deterministic WAL replay parity.
- **Evidence:**
  ```rust
  // crates/engine/src/lib.rs:920-925
  pub fn command_requires_wal_checkpoint(command_id: &str) -> bool {
      if command_id == "edit.undo" || command_id == "edit.redo" {
          return true;
      }
      cmd::registry().get(command_id).is_some_and(|spec| spec.effects.contains(cmd::Effects::READS_FILESYSTEM))
  }
  ```
  ```rust
  // crates/engine/src/lib.rs:973-977
  let mut journal_params = params.clone();
  let _ = normalize_command_params_in_place(command_id, &mut journal_params);
  self.execute_with_caller_and_hook(command_id, params, caller, cancelled, |session, outcome| {
      session.record_outcome_to_journal(journal, command_id, &journal_params, outcome.mutated)
  })
  ```
  ```rust
  // crates/engine/src/cmd/project.rs:667-684
  let created_utc = match parsed.timestamp_utc {
      Some(raw) => { ... }
      None => current_utc_timestamp(),
  };
  ```
- **Suggested fix:** Either include wall-clock-defaulting commands (`project.createCheckpoint` and `table.excludeCellsWithReason`) in `command_requires_wal_checkpoint` when `timestamp_utc` is omitted, or record the resolved `timestamp_utc` into `journal_params` (or checkpoint automatically whenever a command resolves a nondeterministic clock/seed default).
- **Related:** `cmd/table.rs:2107` (`table.excludeCellsWithReason` in `SEC-12` shares the same `current_utc_timestamp()` pattern).

---

### `FA-SEC11-06` · `S3` · `clone_downstream_graphs` omits graphs linked only via `annotations` or title/axis inline hooks

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/sheet.rs:1474-1506`
- **Category:** `wrong-result`
- **Defect:** `clone_downstream_graphs` decides which `GraphSheet`s belong to a duplicated/cloned family by inspecting only `primary_source_sheet_id`, `series[].source_sheet_id`, `series[].regression_overlay`, `compact_letter_display`, and `comparisons`. However, both `ProjectDocument::upstream_dependencies(&gs.id)` and `remap_graph_spec_references` (`crates/engine/src/cmd/helpers.rs:580-595`) also link graphs via `annotations` (`EmbeddedResultTable`, `ModelEquation`, `TextBox` inline hooks) and inline hooks in `title`, `x_axis.title`, `y_axis.title`, `y2_axis.title`, and `series[].legend_label`.
- **Failure scenario:** If a `GraphSheet` uses an external or separate `DataSheet` as its `primary_source_sheet_id` while embedding an `EmbeddedResultTable` / `ModelEquation` from a cloned analysis or referencing an inline `HookSource` on the duplicated family's `InfoSheet`/`ResultSheet`, `sheet.duplicateFamily` and `sheet.clonePipeline` skip cloning that graph (leaving the duplicated family without its graph, and any layout referencing that graph still pointing to the original graph).
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/sheet.rs:1483-1492
      for gs in &original_graphs {
          let references_family = id_map.contains_key(&gs.spec.primary_source_sheet_id)
              || gs.spec.series.iter().any(|s| {
                  id_map.contains_key(&s.source_sheet_id) || s.regression_overlay.as_ref().is_some_and(|ov| id_map.contains_key(&ov.result_sheet_id))
              })
              || gs.spec.compact_letter_display.as_ref().is_some_and(|cld| id_map.contains_key(&cld.result_sheet_id))
              || gs.spec.comparisons.iter().any(|cmp| id_map.contains_key(&cmp.result_sheet_id));
          if !references_family {
              continue;
          }
  ```
  Compare `crates/engine/src/cmd/helpers.rs:580-594` (`remap_graph_spec_references`):
  ```rust
      for ann in &mut spec.annotations {
          match ann {
              GraphAnnotation::EmbeddedResultTable { result_sheet_id, .. } | GraphAnnotation::ModelEquation { result_sheet_id, .. } => {
                  remap_sheet_id(result_sheet_id, id_map);
              }
              GraphAnnotation::TextBox { markdown, .. } => remap_inline_hook_tokens(markdown, id_map),
              ...
          }
      }
      remap_inline_hook_tokens(&mut spec.title, id_map);
  ```
- **Suggested fix:** Check `doc.upstream_dependencies(&gs.id).iter().any(|dep| id_map.contains_key(dep))` (or also check `annotations` and inline hook sheets in `references_family`) so graph discovery matches `doc.upstream_dependencies` and `remap_graph_spec_references`.
- **Related:** `cmd/sheet.rs:744-746` (`graph_references_sheet` already delegates to `doc.upstream_dependencies(&gs.id)`).

---

### `FA-SEC11-07` · `S3` · `methods_report` formats `HookSource` via Rust `Debug` (`{hook:?}` / `{h:?}`) in user-facing prose

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/methods_report.rs:1434`, `crates/engine/src/methods_report.rs:2132`
- **Category:** `contract-drift`
- **Defect:** `describe_parameter_rule` (`ParameterScope::HookedConstant(hook)`) and `format_anchor` (`NormalizeAnchor::Hooked(h)`) format `HookSource` using Rust `Debug` interpolation (`{hook:?}` and `{h:?}`), emitting raw internal Rust struct/enum representation in `PlainText`, `Markdown`, and `Latex` reports.
- **Failure scenario:** Generating a methods report via `result.generateReport` for any nonlinear regression with a `HookedConstant` parameter rule or any `Transform::Normalize` analysis using a `NormalizeAnchor::Hooked` anchor outputs raw text such as `constrained to hooked constant (InfoConstant { info_sheet_id: Some(SheetId("info-1")), constant_name: "IC50" })` instead of publication-ready prose.
- **Evidence:**
  ```rust
  // crates/engine/src/methods_report.rs:1434
  ParameterScope::HookedConstant(hook) => format!("constrained to hooked constant ({hook:?})"),
  ```
  ```rust
  // crates/engine/src/methods_report.rs:2132
  NormalizeAnchor::Hooked(h) => format!("hooked scalar ({h:?})"),
  ```
- **Suggested fix:** Add a dedicated `format_hook_source(hook: &HookSource) -> String` helper matching on `HookSource::InfoConstant`, `HookSource::ResultScalar`, and `HookSource::ColumnTitleValue` to render clean natural-language references (e.g., `"info constant 'IC50' on sheet 'info-1'"` or `"fitted scalar 'Slope' from 'lin-1'"`).
- **Related:** None in `SEC-11`.

---

### `FA-SEC11-08` · `S3` · `cmd/sheet.rs` publishes `"additionalProperties": true` on 10 command schemas whose runtime structs reject unknown keys

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/sheet.rs:177, 570, 656, 944, 1099, 1168, 1597, 1670, 1826`, `crates/engine/src/cmd/params.rs:285, 306-313`
- **Category:** `contract-drift`
- **Defect:** Nine schema builders in `cmd/sheet.rs` covering 10 of 11 `sheet.*` commands (`sheet.createData`, `sheet.rename`, `sheet.delete`, `sheet.duplicate`, `sheet.duplicateFamily`, `sheet.clonePipeline`, `sheet.freeze`, `sheet.unfreeze`, `sheet.reorder`, `sheet.select`) pass `additional_properties = true` to `obj_schema_with_aliases`. At runtime, `parse_params_with_warnings` sets `allow_ignored_fields = is_open_catchall_schema(schema)`, which returns `true` only when `properties` is empty (`props_empty && additional_true`). Consequently, `StrictValueDeserializer` rejects any undeclared property with `ErrorCode::BadInput`, contradicting the published Draft 2020-12 schema.
- **Failure scenario:** External automation, MCP clients, or batch schema validators consulting `CommandSpec::input_schema` (via `cmd::registry()` / `render_markdown`) treat extra keys as schema-valid because `"additionalProperties": true` is advertised, only for every invocation carrying an extra key to fail at execution with `BadInput` (`unknown field`). Notably, `cmd/project.rs` and `cmd/info.rs` both pass `false` to `obj_schema_with_aliases` while still preserving full `x-deprecated-aliases` support.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/params.rs:285, 306-313
  let de = StrictValueDeserializer { value: &raw, pointer: String::new(), field_name: None, allow_ignored_fields: is_open_catchall_schema(schema) };
  ...
  fn is_open_catchall_schema(schema: &Value) -> bool {
      let Some(obj) = schema.as_object() else { return false; };
      let props_empty = obj.get("properties").and_then(Value::as_object).is_some_and(Map::is_empty);
      let additional_true = obj.get("additionalProperties") == Some(&Value::Bool(true));
      props_empty && additional_true
  }
  ```
  ```rust
  // crates/engine/src/cmd/sheet.rs:1596-1598 (identical `true` flag at :177, :570, :656, :944, :1099, :1168, :1670, :1826)
  fn sheet_id_schema(desc: &str) -> Value {
      obj_schema_with_aliases(vec![("sheet_id", non_empty_str_schema(desc))], &["sheet_id"], vec![("sheet_id", &["sheetId", "id"])], true)
  }
  ```
- **Suggested fix:** Change `additional_properties` from `true` to `false` in the 9 `cmd/sheet.rs` schema builders (matching `extract_result_tab_schema` at `cmd/sheet.rs:1891`, `cmd/project.rs`, and `cmd/info.rs`) so advertised JSON Schemas agree with `StrictValueDeserializer`.
- **Related:** Sibling `obj_schema_with_aliases(..., true)` patterns in `SEC-12` command modules.

---

### `FA-SEC11-09` · `S3` · `cmd::helpers::write_output_file` uses raw non-atomic `std::fs::write` without `ExternalPathPolicy`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/src/cmd/helpers.rs:301-318` (and caller `crates/engine/src/cmd/export.rs:191-194`)
- **Category:** `architecture` / `security`
- **Defect:** Public helper `cmd::helpers::write_output_file` directly invokes `std::fs::create_dir_all` and non-atomic `std::fs::write` instead of `ExternalPathPolicy::write_atomic_bytes` / `curvebook_platform::fs::write_atomic`. Furthermore, `cmd/export.rs:191-194` routes `#[cfg(target_arch = "wasm32")]` export writes through `write_output_file`, producing a raw `std::fs` `IoError` on `wasm32` rather than `ExternalPathPolicy`'s `ErrorCode::UnsupportedFormat` / `RejectExternal` `ErrorCode::PathEscape`.
- **Failure scenario:** On write interruption or disk-full errors, `std::fs::write` leaves partially written/corrupt output files on disk; any caller invoking `write_output_file` directly bypasses `WorkdirGuard` path containment and regular-file checks; and on `wasm32`, export path writes emit generic `IoError` instead of structured `UnsupportedFormat` / `PathEscape`.
- **Evidence:**
  ```rust
  // crates/engine/src/cmd/helpers.rs:301-318
  pub fn write_output_file(path: &str, bytes: &[u8]) -> Result<(), EngineError> {
      if path.trim().is_empty() {
          return Err(EngineError::with_code(ErrorCode::BadInput, "output path must not be empty"));
      }
      let out = Path::new(path);
      if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
          std::fs::create_dir_all(parent).map_err(|err| { ... })?;
      }
      std::fs::write(out, bytes).map_err(|err| { ... })
  }
  ```
  ```rust
  // crates/engine/src/cmd/export.rs:191-194
      #[cfg(target_arch = "wasm32")]
      {
          super::helpers::write_output_file(path, bytes)
      }
  ```
- **Suggested fix:** Remove `write_output_file` (or restrict/replace it with `ctx.session.path_policy.write_atomic_bytes(path, bytes)` so native writes are policy-checked and atomic via `curvebook_platform::fs::write_atomic`, and `wasm32` targets uniformly return `UnsupportedFormat` / `PathEscape`).
- **Related:** None in `SEC-11`.

---

### `FA-SEC11-10` · `S4` · Unused `curvebook-tools` dependency in `crates/engine/Cargo.toml`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/engine/Cargo.toml:30`
- **Category:** `cleanup` / `architecture`
- **Defect:** `crates/engine/Cargo.toml` declares `curvebook-tools = { workspace = true }` under `[dependencies]`, while `crates/engine/src/` and `crates/engine/tests/` contain zero references to `curvebook_tools` (`rg` confirmed `0` matches).
- **Failure scenario:** Every `cargo test` and `cargo clippy --all-targets` build of `curvebook-engine` emits `warning: unused dependency 'curvebook-tools'`, adding an unnecessary build-graph edge and warning noise.
- **Evidence:**
  ```toml
  # crates/engine/Cargo.toml:29-31
  curvebook-text = { workspace = true }
  curvebook-tools = { workspace = true }
  curvebook-transform = { workspace = true }
  ```
  Observed compiler warning during `cargo clippy --locked --offline -p curvebook-engine --all-targets -- -D warnings` and `cargo test --locked --offline -p curvebook-engine`:
  `warning: unused dependency curvebook-tools --> crates/engine/Cargo.toml:30:1`.
- **Suggested fix:** Remove `curvebook-tools = { workspace = true }` from `crates/engine/Cargo.toml:30`.
- **Related:** `FA-SEC01-*` (`curvebook-palette` / `curvebook-platform` unused dependency findings).
