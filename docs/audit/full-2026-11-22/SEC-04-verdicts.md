# SEC-04 Verification Verdicts — File Formats, Import & Recovery (`crates/format`)

- **Base Checked:** `7c85a8e516165ddadb101236a88eec937bf1f095` (`main`)
- **Verifier:** `ver-backend` (independent trace + isolated `.lc-worktrees/ver-backend` dynamic probe binary `ver_sec04_probe`, cleaned up after execution)
- **Source Report:** [SEC-04.md](../../../docs/audit/full-2026-11-22/SEC-04.md) (`FA-SEC04-01`..`FA-SEC04-14`)

## Summary Table

| Verdict | Count | Settled Severity Breakdown |
|---|---:|---|
| `CONFIRMED` | 14 | `S1: 3`, `S2: 7`, `S3: 3`, `S4: 1` |
| `PLAUSIBLE` | 0 | `—` |
| `REFUTED` | 0 | `—` |
| `DUPLICATE` | 0 | `—` |
| **Total** | **14** | **`S1=3, S2=7, S3=3, S4=1`** |

Dynamic reproducer run against isolated detached worktree at `7c85a8e516165ddadb101236a88eec937bf1f095` (`CARGO_TARGET_DIR=/tmp/cb-audit-ver-backend cargo test --locked --offline -p curvebook-format --test ver_sec04_probe -- --nocapture`): `11 passed; 0 failed; 0 ignored; finished in 0.82s`. (Note: `#71` and `#73` in [local-code-audit-2026-11-18.md](../../../docs/audit/local-code-audit-2026-11-18.md#L713-L736) are marked closed on `main`, so `FA-SEC04-04` and `FA-SEC04-09` are confirmed live regressions / incomplete closures rather than `DUPLICATE`.)

---

## File Coverage Completeness Check

Verified `100%` coverage against `crates/format` at `7c85a8e516165ddadb101236a88eec937bf1f095`:

- `crates/format/Cargo.toml` (`27` lines)
- `crates/format/src/`: `archive.rs` (`3114`), `csv.rs` (`1649`), `error.rs` (`137`), `export.rs` (`431`), `image_asset.rs` (`154`), `lib.rs` (`71`), `prism.rs` (`681`), `pzfx.rs` (`1040`), `recovery.rs` (`2090`), `registry.rs` (`670`), `schemas.rs` (`411`), `versioned.rs` (`400`), `xml.rs` (`509`), `zip.rs` (`534`)

All `15` scoped files (`11,918` total lines; zero files omitted) appear in `SEC-04.md`'s File Coverage Table with matching line counts and `1..=N` inspection ranges.

---

## Per-Finding Verdicts

### Verdict `FA-SEC04-01` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [prism.rs](../../../crates/format/src/prism.rs#L247-L257) clones `asset.bytes = bytes.to_vec()` for every entry in `doc.image_assets` inside `try_load_embedded_project_json` *before* `validate_project_document(&doc)?` (line `257`) enforces `MAX_PROJECT_IMAGE_ASSETS` (`256`), unique `asset_id`s, and `MAX_IMAGE_ASSET_BYTES` (`32 MiB`). Contrast [archive.rs](../../../crates/format/src/archive.rs#L801-L825) (`hydrate_archive_image_assets`), which checks length and duplicates first.
  - Dynamic probe (`probe_01_prism_duplicate_asset_cloning_before_validation`): `10,000` `ProjectImageAsset` stubs referencing a single archive entry bypassed `parse_bounded_json_outcome`, cloned all `10,000` vectors, and only then failed at `validate_project_document` (`"10000 ... 256"`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (multi-hundred-GiB OOM abort reachable from an untrusted `.prism` ZIP with one 32–64 MiB PNG and 10,000 stubs).
- **Still present on main:** yes

### Verdict `FA-SEC04-02` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [recovery.rs](../../../crates/format/src/recovery.rs#L1159-L1165) passes `&Vec::new()` to [reclaim_stale_eviction_lock](../../../crates/format/src/recovery.rs#L1121-L1141) whenever `read_bounded_lock_bytes(&evict_path, MAX_LOCKFILE_BYTES)` returns `Oversized(_)`. `reclaim_stale_eviction_lock` performs an unbounded `std::fs::read(&reclaim_path)` (line `1129`) and restores the oversized file via `std::fs::hard_link(&reclaim_path, evict_path)` (line `1136`) whenever `actual_bytes != observed_bytes` (guaranteed true for oversized non-empty files).
  - Dynamic probe (`probe_02_oversized_evict_lock_restores_itself`): placing a `70,000`-byte `curvebook.lock.evict` next to a stale `curvebook.lock` failed `CrashRecoveryJournal::open_or_recover` (`"concurrent"`) and preserved the `70,000`-byte eviction file on disk.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (unbounded allocation OOM plus persistent lock/recovery wedge from a malformed `.curvebook.lock.evict`).
- **Still present on main:** yes

### Verdict `FA-SEC04-03` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [recovery.rs](../../../crates/format/src/recovery.rs#L970-L988) (`read_bounded_lock_bytes`) and [recovery.rs](../../../crates/format/src/recovery.rs#L1451-L1466) (`open_journal_stream`) call `std::fs::File::open` before any `symlink_metadata` / `is_file()` check (hanging indefinitely on FIFOs whose `len()` is `0`). Meanwhile [read_snapshot](../../../crates/format/src/recovery.rs#L1354-L1360) and [read_journal_bytes](../../../crates/format/src/recovery.rs#L1423-L1429) call unbounded `std::fs::read` without regular-file or byte-cap checks.
  - Dynamic probe (`probe_03_directory_backend_fifo_hang_and_unbounded_snapshot`): `CrashRecoveryJournal::open_or_recover` hung past `250 ms` on FIFOs at both `curvebook.lock` and `ops.journal`, and loaded a sparse `snapshot.curvebook` of size `MAX_ARCHIVE_FILE_BYTES + 1` (`128 MiB + 1`) into memory without error.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S1` (process hang on FIFO and unbounded read OOM from untrusted journal directory entries, bypassing regular-file archive guards).
- **Still present on main:** yes

### Verdict `FA-SEC04-04` · `CONFIRMED` (`REGRESSION` of closed `#71`)

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [export.rs](../../../crates/format/src/export.rs#L29-L36) and [export.rs](../../../crates/format/src/export.rs#L66-L94) implement `ensure_result_sheet_csv_exportable` and `try_export_result_*` for `#71`, yet [lib.rs](../../../crates/format/src/lib.rs#L36) does not re-export them and [registry.rs](../../../crates/format/src/registry.rs#L642) / [registry.rs](../../../crates/format/src/registry.rs#L659) / [registry.rs](../../../crates/format/src/registry.rs#L667) invoke unguarded `export_result_sheet_tidy_csv`. Callsite grep across workspace confirms downstream CLI/Engine/MCP/GUI exporters (`apps/curvebook-cli/src/cmd_convert.rs:683`, `crates/engine/src/cmd/table.rs:2669`, `crates/engine/src/cmd/analysis.rs:1040,1046,1048`, `crates/mcp/src/formats.rs:1167,1174,1176,1178`, `crates/ui-egui/src/control_io.rs:2351,2358,2360,2362`) also call the unguarded functions; only `crates/format/tests/audit_wave1_format_io.rs` calls `try_export_result_*`.
  - Dynamic probe (`probe_04_registry_csv_tsv_bypass_failed_result_guard`): exporting a `ResultStatus::Failed` sheet via `FormatRegistry` (`"csv"` and `"tsv"`, both `ExportSource::ResultSheet` and `ExportSource::Document`) returned `Ok(...)` header-only CSV bytes starting with `"result_id"`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (failed analyses silently export misleading header-only CSV/TSV through the format registry and production export callers instead of returning `FormatError::InvalidPipeline`).
- **Still present on main:** yes

### Verdict `FA-SEC04-05` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [archive.rs](../../../crates/format/src/archive.rs#L631-L633), [archive.rs](../../../crates/format/src/archive.rs#L756-L777), [archive.rs](../../../crates/format/src/archive.rs#L790), and [prism.rs](../../../crates/format/src/prism.rs#L233) gate `v0..=v6` migrations and embedded `.prism` JSON through `parse_bounded_json_outcome` with `MAX_MIGRATION_JSON_NODES = 1_000_000` and `MAX_MIGRATION_JSON_BYTES = 64 MiB`. Every populated `Cell` allocates multiple DOM nodes, exhausting the node budget near ~250,000 cells (`2.5%` of `MAX_IMPORT_CELLS` / `0.5%` of `MAX_PROJECT_CELLS`).
  - Dynamic probe (`probe_05_migration_dom_budget_rejects_250k_cell_v6_project`): opening a valid `10 x 25,000` (`250,000`-cell) `ColumnTable` project tagged `schema_version: 6` via `open_project_bytes` failed with `"migration"` DOM budget exhaustion.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (valid large `v0..=v6` archives and `.prism` imports fail loudly with a false security/DoS budget error).
- **Still present on main:** yes

### Verdict `FA-SEC04-06` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [csv.rs](../../../crates/format/src/csv.rs#L1445-L1456) grows `XyTable::x_cells` using `t.x_mode.subcolumn_count()`, which returns `0` for `XColumnMode::GeneratedSequence`, whereas existing rows in a `GeneratedSequence` table with X exclusions store width-`1` carrier rows (`x_cells[r].len() == 1`).
  - Dynamic probe (`probe_06_generated_sequence_x_cells_rectangularity_corruption`): running `apply_csv_import_link` to append rows to a valid `GeneratedSequence` `XyTable` with `x_cells[0][0].excluded = true` appended empty `[]` rows, causing subsequent `sheet.archetype.validate()` to fail rectangularity.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (corrupts in-memory `DataSheet` rectangularity and breaks validation/persistence on `apply_csv_import_link`; note `crates/engine` routes live links via `curvebook_transform::import`, scoping impact to direct `curvebook_format::apply_csv_import_link` callers).
- **Still present on main:** yes

### Verdict `FA-SEC04-07` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [csv.rs](../../../crates/format/src/csv.rs#L1536-L1585) (`ensure_sheet_capacity` and `resolve_target_coord`) falls into `_ => 1` for `TableArchetype::Nested`, failing to walk each group's `biological_unit_count()` and allowing `anchor.subcol > 0` on single-subcolumn tables to overflow into column `col + 1`.
  - Dynamic probe (`probe_07_nested_apply_csv_import_link_subs_one`): importing a 2-column CSV into a 2-group, 2-unit-per-group `NestedTable` wrote the second CSV value to `(group=1, subcol=0)` instead of `(group=0, subcol=1)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (wrong column/subcolumn placement and spurious 1-unit group allocation through `apply_csv_import_link`).
- **Still present on main:** yes

### Verdict `FA-SEC04-08` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [csv.rs](../../../crates/format/src/csv.rs#L1306-L1313) and [csv.rs](../../../crates/format/src/csv.rs#L1349-L1367) execute `ensure_sheet_capacity`, `shift_rows_down`, `apply_headers_to_sheet`, and leading `target.set_cell` writes in place before later header `validate_title`, `set_cell`, or `add_linked_region` failures abort `apply_csv_import_link`.
  - Dynamic probe (`probe_08_apply_csv_import_link_partial_mutation_on_error`): calling `apply_csv_import_link` with `PlacementMode::InsertRows`, `propagate_title_header: true`, and an oversized second header (`> MAX_TITLE_BYTES`) returned `Err(...)` while leaving the `DataSheet` resized, existing rows shifted down, and the first group title overwritten.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (non-atomic error path corrupts caller's `DataSheet` in place).
- **Still present on main:** yes

### Verdict `FA-SEC04-09` · `CONFIRMED` (`REGRESSION` / incomplete closure of `#73`)

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [csv.rs](../../../crates/format/src/csv.rs#L383-L398) exports excluded `CellValue::Text` cells with `force_quote: false` (`Self::unquoted(text)` containing trailing `*`), and [csv.rs](../../../crates/format/src/csv.rs#L306-L308) runs `parse_cell_token` on all unquoted fields.
  - Dynamic probe (`probe_09_excluded_text_csv_roundtrip_corruption`): exporting and re-importing excluded text cells turned `"0012"` into `CellValue::Number(12.0)`, `"NA"` into `CellValue::Empty` (`excluded: true`), and `" a "` into `CellValue::Text("a")`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silent type coercion and text loss across CSV export/re-import round-trips for excluded categorical/identifier cells).
- **Still present on main:** yes

### Verdict `FA-SEC04-10` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [export.rs](../../../crates/format/src/export.rs#L98-L113) calls `validate_result_floats(sheet)` ([export.rs](../../../crates/format/src/export.rs#L386-L418)) and serializes `analysis: &sheet.analysis`, but `validate_result_floats` omits `sheet.analysis.first_non_finite_float()` (checked in `archive.rs:2139-2141`). *Scope note:* [ResultSheetExportJson](../../../crates/format/src/export.rs#L14-L24) does **not** include `analyzable_data`, so only `sheet.analysis` leaks non-finite floats into JSON `null` here (`analyzable_data` is only part of `archive.rs` validation).
  - Dynamic probe (`probe_10_export_result_sheet_json_non_finite_analysis`): `export_result_sheet_json` succeeded (`Ok(json)`) on both `LinearRegression.plot_x_min: Some(f64::NAN)` (silently dropping to `null` / `Some(0.0)` omitted bound in round-trip) and `NormalizeAnchor::FixedValue(f64::INFINITY)` (producing `"FixedValue":null` that failed subsequent `serde_json::from_str::<AnalysisSpec>`).
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S2` (silently emits invalid/lossy JSON for non-finite `AnalysisSpec` parameters despite documented non-finite float validation).
- **Still present on main:** yes

### Verdict `FA-SEC04-11` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [csv.rs](../../../crates/format/src/csv.rs#L593-L604) emits `NestedGroupColumn::subcolumn_titles` verbatim without prepending `g.title`, whereas `.pzfx` Nested imports ([pzfx.rs](../../../crates/format/src/pzfx.rs#L621-L650)) store bare subcolumn labels (e.g. `"Mouse 1"`, `"Mouse 2"`). On re-import, [parse_nested_column_groups](../../../crates/format/src/csv.rs#L901-L968) finds no colon prefix match and loses the original group title (`"Control"`), splitting into separate 1-unit groups under `RawReplicates(1)` or adopting `"Mouse 1"` as the group title under `RawReplicates(2)`.
  - Dynamic probe (`probe_11_12_13_14_remaining_findings`): verified header `"Mouse 1,Mouse 2\n"`, split into 2 groups under `RawReplicates(1)`, and wrong group title `"Mouse 1"` under `RawReplicates(2)`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (CSV round-trip data/structure loss on `.pzfx`-origin Nested tables, with workaround of renaming subcolumns with `"Group:"` prefixes).
- **Still present on main:** yes

### Verdict `FA-SEC04-12` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [prism.rs](../../../crates/format/src/prism.rs#L618-L681) (`remap_document_sheet_references`) remaps `rs.analysis.source_sheet_id` and `PowerAnalysis.info_sheet_id`, but omits `HookSource` sheet IDs carried inside `TransformAnalysisSpec::Normalize`, `NonlinearRegression` `ParameterScope::HookedConstant`, and inline `{{info:...}}`/`{{result:...}}` tokens in `GraphSpec`/`LayoutSpec`.
  - Dynamic probe (`probe_11_12_13_14_remaining_findings`): importing a `.prism` `document.json` containing `InfoSheet` `"Info 1"` and a `Normalize` analysis referencing `HookSource::InfoConstant { info_sheet_id: Some("Info 1"), .. }` sanitized the `InfoSheet` ID away from `"Info 1"` while leaving `Normalize`'s `info_sheet_id` dangling at `"Info 1"`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (triggered only by `.prism` files containing invalid sheet IDs combined with explicit hooked references).
- **Still present on main:** yes

### Verdict `FA-SEC04-13` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: [prism.rs](../../../crates/format/src/prism.rs#L241-L246) uses `.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(curvebook_doc::CURRENT_SCHEMA_VERSION)` instead of `clamp_peek_schema_version` (`u32::try_from(raw).unwrap_or(u32::MAX)`), bypassing `UnsupportedSchemaVersion`.
  - Dynamic probe (`probe_11_12_13_14_remaining_findings`): `"schema_version": 5_000_000_000` in embedded `.prism` `document.json` failed with `FormatError::Json(_)` instead of `FormatError::UnsupportedSchemaVersion`.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S3` (error-variant mismatch on malformed/extreme `.prism` `schema_version` payloads).
- **Still present on main:** yes

### Verdict `FA-SEC04-14` · `CONFIRMED`

- **Verifier:** `ver-backend`
- **Evidence:**
  - Code trace: `JournalOpRecord::to_ndjson_line` ([recovery.rs](../../../crates/format/src/recovery.rs#L331-L336)) already appends `'\n'`, and `build_cleaned_journal_bytes` ([recovery.rs](../../../crates/format/src/recovery.rs#L1874-L1883)) unconditionally appends a second `b'\n'`.
  - Dynamic probe (`probe_11_12_13_14_remaining_findings`): recovering a journal with a torn tail rewrote `ops.journal` containing `"\n\n"` blank separator lines.
- **Checked at:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Severity agreed:** `S4` (formatting/cleanup defect tolerated by blank-line-skipping NDJSON readers).
- **Still present on main:** yes
