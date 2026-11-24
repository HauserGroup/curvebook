# SEC-11 Verification Verdicts — Engine Core, DAG & Commands (Part 1)

- **Base Checked:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-backend-3` (independent source inspection + cross-crate caller/upstream guard trace + isolated `.lc-worktrees/ver-backend-3` dynamic probe `ver_sec11_probe`, removed after execution)
- **Source Report:** [SEC-11.md](SEC-11.md) (`FA-SEC11-01`..`FA-SEC11-10`)

## Summary Table

| Verdict | Count | Settled Severity Breakdown |
|---|---:|---|
| `CONFIRMED` | 10 | `S1: 0`, `S2: 3`, `S3: 6`, `S4: 1` |
| `PLAUSIBLE` | 0 | `—` |
| `REFUTED` | 0 | `—` |
| `DUPLICATE` | 0 | `—` |
| **Total** | **10** | **`S1=0, S2=3, S3=6, S4=1`** |

Dynamic verification run against an isolated detached worktree at `7c85a8e516165ddadb101236a88eec937bf1f095` (probe test removed afterward; worktree clean via `git status -s`):
- Clippy check (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-3 cargo clippy --manifest-path .lc-worktrees/ver-backend-3/Cargo.toml --locked --offline -p curvebook-engine --all-targets -- -D warnings`): exited `0` (zero Clippy warnings; Cargo emitted `warning: unused dependency 'curvebook-tools' --> crates/engine/Cargo.toml:30:1`).
- Baseline suite (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-3 cargo test --manifest-path .lc-worktrees/ver-backend-3/Cargo.toml --locked --offline -p curvebook-engine --quiet`): exited `0` with `269 passed; 0 failed; 0 ignored` (`56` unit tests + `213` integration tests across `34` test binaries + `0` doc-tests).
- Targeted reproducer suite (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend-3 cargo test --manifest-path .lc-worktrees/ver-backend-3/Cargo.toml --locked --offline -p curvebook-engine --test ver_sec11_probe -- --nocapture`): `running 8 tests` -> `test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 22.94s`.

Cross-reference & precision notes (without refuting any defect):
- `FA-SEC11-03`: `SEC-11.md` cites sibling `FA-SEC02-03` for `ProjectDocument::Deserialize` omitting `MAX_PROJECT_CELLS`; in settled [SEC-02-verdicts.md](SEC-02-verdicts.md#L59-L65), that finding is numbered `FA-SEC02-04`.
- `FA-SEC11-05`: `SEC-11.md` states the minted timestamp also propagates into the `snapshot_json` of any subsequent checkpoint; `run_project_create_checkpoint` actually calls `snapshot_doc.checkpoints.clear()` before serializing `snapshot_json`, so only `doc.checkpoints[i].created_utc` itself diverges on replay (whereas sibling `table.excludeCellsWithReason` in `SEC-12` stores timestamps inside `ds.exclusion_notes`, which would propagate).

---

## File Coverage Completeness Check

Verified `100%` file coverage against `SEC-11` scope at `7c85a8e516165ddadb101236a88eec937bf1f095` (`1` Cargo manifest + `14` Rust source files = `15` files, `13,757` total lines; `find crates/engine/src -type f | sort` returns `21` `.rs` files, with the remaining `7` in `cmd/{analysis,export,graph,image,layout,table}.rs` and `cmd/table/import.rs` assigned to `SEC-12`):

- Manifest: [Cargo.toml](../../../../crates/engine/Cargo.toml) (`38`)
- `crates/engine/src/`: [dag.rs](../../../../crates/engine/src/dag.rs) (`1,472`), [guard.rs](../../../../crates/engine/src/guard.rs) (`101`), [lib.rs](../../../../crates/engine/src/lib.rs) (`1,563`), [methods_report.rs](../../../../crates/engine/src/methods_report.rs) (`2,754`), [path_policy.rs](../../../../crates/engine/src/path_policy.rs) (`394`)
- `crates/engine/src/cmd/`: [mod.rs](../../../../crates/engine/src/cmd/mod.rs) (`380`), [edit.rs](../../../../crates/engine/src/cmd/edit.rs) (`488`), [effects.rs](../../../../crates/engine/src/cmd/effects.rs) (`196`), [helpers.rs](../../../../crates/engine/src/cmd/helpers.rs) (`959`), [info.rs](../../../../crates/engine/src/cmd/info.rs) (`348`), [params.rs](../../../../crates/engine/src/cmd/params.rs) (`1,340`), [project.rs](../../../../crates/engine/src/cmd/project.rs) (`1,210`), [sheet.rs](../../../../crates/engine/src/cmd/sheet.rs) (`2,447`), [spellings.rs](../../../../crates/engine/src/cmd/spellings.rs) (`67`)

Every file and line count matches `SEC-11.md`'s File Coverage Table with zero omitted files.

---

## Per-Finding Verdicts

### Verdict `FA-SEC11-01` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [describe_bland_altman](../../../../crates/engine/src/methods_report.rs#L786-L800) resolves both method labels unconditionally through [column_title_at](../../../../crates/engine/src/methods_report.rs#L2040-L2058) (`TableArchetype::Xy(t) => t.data_sets.get(col_idx)`), ignoring `xy.x_title`. In contrast, [extract_bland_altman_pairs](../../../../crates/stats/src/two_group.rs#L2386-L2408) enables X-vs-Y[0] comparison whenever `ba.xy_compare_x_vs_y.unwrap_or(xy.data_sets.len() == 1)` is `true`, pairing `xy.x_title` (`column_a == 0`) with `xy.data_sets[0].title` (`column_b == 1`) and emitting `"{lbl_a} vs. {lbl_b}"` in `ResultTab` ([two_group.rs](../../../../crates/stats/src/two_group.rs#L1948)). Dynamic `probe_01_bland_altman_xy_methods_report_labels` confirmed: on a 1-dataset XY table (`Reference`, `Assay`) `ResultTab` header was `"Reference vs. Assay"` while `PlainText` report printed `"Method agreement between Assay and Column 2"`; on a 2-dataset XY table with `xy_compare_x_vs_y: true` (`Reference`, `AssayA`, `AssayB`) `ResultTab` header was `"Reference vs. AssayA"` while `PlainText` report printed `"Method agreement between AssayA and AssayB"`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (wrong method attribution in published prose, contradicting the `ResultSheet` header and source table columns).
- **Still present on main:** yes

### Verdict `FA-SEC11-02` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [decode_base64](../../../../crates/engine/src/cmd/helpers.rs#L361-L391) filters whitespace into an uncapped `Vec<u8>`, allocates `Vec::with_capacity(chunks.len().saturating_mul(3))` with no byte limit, and accepts empty/whitespace strings as `Ok(vec![])`. Callers in [project.rs](../../../../crates/engine/src/cmd/project.rs#L162-L164) (`project.open`), [project.rs](../../../../crates/engine/src/cmd/project.rs#L330-L334) (`project.instantiateTemplate`), and [import.rs](../../../../crates/engine/src/cmd/table/import.rs#L803-L805) / [import.rs](../../../../crates/engine/src/cmd/table/import.rs#L864-L866) (`table.importPrism` / `table.importPzfx`) invoke it before format byte caps apply (unlike sibling [decode_base64_capped](../../../../crates/engine/src/cmd/image.rs#L141-L158)). Upstream transport caps exist only on stdio MCP JSON-RPC (`16 MiB` in [protocol.rs](../../../../crates/mcp/src/protocol.rs#L54)) and TCP control lines, not on CLI batch scripts (`MAX_SCRIPT_FILE_BYTES = 64 MiB` at [cmd_run_batch.rs](../../../../crates/tools/src/cmd_run_batch.rs#L26)), direct `Session::execute`, or the `wasm32` web bridge. Dynamic `probe_02_decode_base64_unbounded_and_accepts_empty` verified `decode_base64("") == Ok(vec![])`, `decode_base64("  \t\r\n  ") == Ok(vec![])`, and `table.importPzfx` with a `65 MiB` decoded `bytes_base64` payload (`> MAX_XML_BYTES = 64 MiB`) allocated and passed `decode_base64` without `ErrorCode::ResourceLimit`. (Related `FA-SEC02-04` covers `crates/doc/src/image.rs` `serde_base64_bytes`, not a duplicate.)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (unbounded base64 buffer allocation prior to archive/XML size enforcement, creating a DoS / `wasm32` OOM vector on large `bytes_base64` inputs).
- **Still present on main:** yes

### Verdict `FA-SEC11-03` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [run_sheet_extract_result_tab](../../../../crates/engine/src/cmd/sheet.rs#L1918-L2019) constructs a `ColumnTable`, `GroupedTable`, or `MultipleVariablesTable` (which checks only the per-sheet `MAX_SHEET_CELLS = 30_000_000` ceiling) and calls `ctx.session.doc.add_data_sheet(sheet)?` without invoking `validate_project_cells(document_used_cells(&ctx.session.doc), ...)`, unlike sibling handlers at [sheet.rs](../../../../crates/engine/src/cmd/sheet.rs#L287) (`create_data_with_options`), [sheet.rs](../../../../crates/engine/src/cmd/sheet.rs#L985) (`run_sheet_duplicate`), and [sheet.rs](../../../../crates/engine/src/cmd/sheet.rs#L1299) (`clone_family_into_doc`). Upstream/post-dispatch guards also omit this check: [ProjectDocument::ensure_can_insert](../../../../crates/doc/src/project.rs#L139-L151) checks only `MAX_SHEETS_PER_PROJECT`, and [validate_project_document_inner](../../../../crates/format/src/archive.rs#L2115-L2174) (called from [lib.rs](../../../../crates/engine/src/lib.rs#L1397)) does not check `MAX_PROJECT_CELLS` (`50_000_000`). Dynamic `probe_03_extract_result_tab_bypasses_max_project_cells` confirmed `sheet.createData` rejected adding `200,000` cells atop `49,900,000` cells with `ErrorCode::ResourceLimit`, while `sheet.extractResultTab` on a `100,000 x 2` `ResultTab` succeeded (`mutated: true`), raising `allocated_cell_slots().sum()` to `50_100_000 > MAX_PROJECT_CELLS`. (*Cross-ref note:* `SEC-11.md` cites sibling `FA-SEC02-03`, which is numbered `FA-SEC02-04` in [SEC-02-verdicts.md](SEC-02-verdicts.md#L59-L65).)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (bypasses the project-wide `50,000,000`-cell resource ceiling via valid result extraction).
- **Still present on main:** yes

### Verdict `FA-SEC11-04` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [run_project_open](../../../../crates/engine/src/cmd/project.rs#L170-L182) calls `ctx.session.set_file_path(opened_path, false)` and returns `CommandOutcome::mutated(...)`, after which [execute_mutating_spec_with_hook](../../../../crates/engine/src/lib.rs#L1403-L1406) unconditionally executes `self.dirty = true;` (contradicting `Session::dirty`'s docstring at [lib.rs](../../../../crates/engine/src/lib.rs#L1171)). Meanwhile [run_project_save](../../../../crates/engine/src/cmd/project.rs#L245-L256) returns `CommandOutcome::read_only(...)` without clearing `ctx.session.dirty`. While desktop host ([desktop_host.rs](../../../../apps/curvebook/src/desktop_host.rs#L1155-L1158), `#L1965`) and MCP file-tool wrappers ([formats.rs](../../../../crates/mcp/src/formats.rs#L262), `#L645`) compensate manually via `set_file_path(..., true)`, MCP `command_run` ([tools.rs](../../../../crates/mcp/src/tools.rs#L1800-L1809)), live-attached MCP helpers ([formats.rs](../../../../crates/mcp/src/formats.rs#L593-L605)), and direct `Session::execute` callers observe `is_dirty() == true` after opening or saving a file (and [egui_shell/mod.rs](../../../../crates/ui-egui/src/egui_shell/mod.rs#L1873) gates `project.open` on `is_dirty()`). Dynamic `probe_04_project_open_and_save_dirty_semantics` confirmed `project.save { path }` kept `is_dirty() == true` and `file_path() == None`, and `project.open { path }` produced `is_dirty() == true`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (clean-state contract drift in core `Session`, worked around manually in desktop/file-MCP wrappers).
- **Still present on main:** yes

### Verdict `FA-SEC11-05` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [command_requires_wal_checkpoint](../../../../crates/engine/src/lib.rs#L920-L925) returns `true` only for `"edit.undo"`, `"edit.redo"`, and `Effects::READS_FILESYSTEM`. `project.createCheckpoint` declares `Effects::MUTATES_DOCUMENT` only ([project.rs](../../../../crates/engine/src/cmd/project.rs#L959)), omits WAL checkpointing, journals un-resolved `journal_params` ([lib.rs](../../../../crates/engine/src/lib.rs#L973-L977)), and invokes `current_utc_timestamp()` inside [run_project_create_checkpoint](../../../../crates/engine/src/cmd/project.rs#L667-L688) when `timestamp_utc` is omitted, minting a new `unix:<secs>` timestamp during [replay_recovered_commands](../../../../crates/engine/src/lib.rs#L1042). Dynamic `probe_05_create_checkpoint_wal_replay_parity` confirmed `!Session::command_requires_wal_checkpoint("project.createCheckpoint")` and showed `live.document() != replayed.document()` (`checkpoints[0].created_utc` diverged) when replaying an abandoned WAL in a later epoch second. (*Precision note:* `run_project_create_checkpoint` calls `snapshot_doc.checkpoints.clear()` at [project.rs](../../../../crates/engine/src/cmd/project.rs#L686-L688) before serializing `snapshot_json`, so an earlier checkpoint's `created_utc` does not propagate into subsequent checkpoints' `snapshot_json`, though `doc.checkpoints[i].created_utc` itself still breaks WAL replay parity; sibling `table.excludeCellsWithReason` at `cmd/table.rs:2107` does write wall-clock timestamps into `ds.exclusion_notes`.)
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (breaks deterministic WAL replay parity on checkpoint metadata when `timestamp_utc` is omitted; workaround is passing explicit `timestamp_utc`).
- **Still present on main:** yes

### Verdict `FA-SEC11-06` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [clone_downstream_graphs](../../../../crates/engine/src/cmd/sheet.rs#L1483-L1492) inspects only `primary_source_sheet_id`, `series[].source_sheet_id`, `series[].regression_overlay`, `compact_letter_display`, and `comparisons`, ignoring `annotations` (`EmbeddedResultTable`, `ModelEquation`, `TextBox` inline hooks) and inline hooks inside graph/axis titles (`title`, `x_axis.title`, `y_axis.title`, `y2_axis.title`) that both [ProjectDocument::upstream_dependencies](../../../../crates/doc/src/project.rs#L364-L392) and [remap_graph_spec_references](../../../../crates/engine/src/cmd/helpers.rs#L563-L595) handle (note also `graph_references_sheet` at [sheet.rs](../../../../crates/engine/src/cmd/sheet.rs#L744-L746) delegates to `upstream_dependencies`, whereas `GraphSpec::inline_hooks` at [graph.rs](../../../../crates/doc/src/graph.rs#L1722-L1735) omits `series[].legend_label`). Dynamic `probe_06_clone_downstream_graphs_omits_annotation_and_hook_graphs` constructed two `GraphSheet`s whose `primary_source_sheet_id` was an unrelated sheet (`"xy-other"`), one with an `EmbeddedResultTable` referencing `"fit-1"` and one with `title: "Alpha={{info:info-1:Alpha}}"`: `doc.upstream_dependencies` included `"fit-1"` and `"info-1"`, yet `sheet.duplicateFamily` returned `graph_sheet_ids: []`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (omits annotation-only or hook-only downstream graphs during `duplicateFamily`/`clonePipeline`; workaround is manual `graph.duplicate`).
- **Still present on main:** yes

### Verdict `FA-SEC11-07` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** Both [describe_parameter_rule](../../../../crates/engine/src/methods_report.rs#L1434) (`ParameterScope::HookedConstant(hook) => format!("constrained to hooked constant ({hook:?})")`) and [format_anchor](../../../../crates/engine/src/methods_report.rs#L2132) (`NormalizeAnchor::Hooked(h) => format!("hooked scalar ({h:?})")`) interpolate `HookSource` via Rust `Debug` formatting into `PlainText`/`Markdown`/`Latex` prose (`HookSource` has no `Display` impl; no other `HookSource` `:?` formatters exist in `methods_report.rs`). Dynamic `probe_07_methods_report_debug_hook_source` verified `generate_methods_report(..., ReportFormat::PlainText)` emits `InfoConstant { info_sheet_id: Some(SheetId("info-1")), constant_name: "IC50" }` and `ResultScalar { result_sheet_id: SheetId("fit-hook") ... }` verbatim.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (leaks raw internal Rust `Debug` representation in user-facing methods reports).
- **Still present on main:** yes

### Verdict `FA-SEC11-08` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** Nine schema helpers in `cmd/sheet.rs` ([sheet.rs](../../../../crates/engine/src/cmd/sheet.rs#L177), `#L570`, `#L656`, `#L944`, `#L1099`, `#L1168`, `#L1597`, `#L1670`, `#L1826`) pass `additional_properties = true` to `obj_schema_with_aliases` for `10` of the `11` `sheet.*` commands (only `extract_result_tab_schema` at `#L1891` passes `false`). Because every one of those schemas has non-empty `properties`, [is_open_catchall_schema](../../../../crates/engine/src/cmd/params.rs#L306-L313) returns `false`, so [StrictValueDeserializer](../../../../crates/engine/src/cmd/params.rs#L285) rejects unknown fields at runtime despite `"additionalProperties": true` in the published JSON Schema. Dynamic `probe_08_sheet_schemas_additional_properties_vs_strict_deserializer` confirmed all `10` commands advertise `"additionalProperties": true` while failing with `ErrorCode::BadInput` (`unknown field \`bogus\``) when given `"bogus": 1`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (published JSON Schema contract disagrees with strict runtime parameter deserialization; passing only declared keys works around it).
- **Still present on main:** yes

### Verdict `FA-SEC11-09` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [write_output_file](../../../../crates/engine/src/cmd/helpers.rs#L301-L318) is exported under `pub mod helpers` ([cmd/mod.rs](../../../../crates/engine/src/cmd/mod.rs#L15)) and directly runs `std::fs::create_dir_all` + non-atomic `std::fs::write` without `ExternalPathPolicy` or `curvebook_platform::fs::write_atomic` (contrast [ExternalPathPolicy::write_atomic_bytes](../../../../crates/engine/src/path_policy.rs#L181-L197)). Native builds in [export.rs](../../../../crates/engine/src/cmd/export.rs#L169-L190) use `curvebook_platform::fs::write_atomic` (with pre-dispatch path policy checks at [lib.rs](../../../../crates/engine/src/lib.rs#L1298)), while the `#[cfg(target_arch = "wasm32")]` arm at [export.rs](../../../../crates/engine/src/cmd/export.rs#L191-L194) delegates to `helpers::write_output_file`, emitting `IoError` instead of structured `UnsupportedFormat` / `PathEscape`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (latent non-atomic/policy-bypass hazard on direct `helpers::write_output_file` callers and `wasm32` export error-code inconsistency; native `export.*` command path already uses `write_atomic` + pre-dispatch policy checks).
- **Still present on main:** yes

### Verdict `FA-SEC11-10` · `CONFIRMED`

- **Verifier:** `ver-backend-3`
- **Evidence:** [Cargo.toml](../../../../crates/engine/Cargo.toml#L30) declares `curvebook-tools = { workspace = true }`, workspace `rg "curvebook_tools" curvebook/crates/engine/` yields `0` matches, and `cargo clippy --locked --offline -p curvebook-engine --all-targets -- -D warnings` (as well as `cargo test`) emitted `warning: unused dependency 'curvebook-tools' --> crates/engine/Cargo.toml:30:1`. Related to `FA-SEC01-22` (`curvebook-palette` / `curvebook-platform`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (unused Cargo dependency noise with no functional user impact).
- **Still present on main:** yes
