# SEC-04 Audit Report — File Formats, Import & Recovery (`crates/format`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Scope:** `SEC-04` (`crates/format/Cargo.toml` + 14 source files in `crates/format/src/`, `11,918` total lines)
- **Date:** `2026-11-22`
- **Reviewer:** `rev-sci`
- **Mode:** Report-only static audit + read-only verification (`cargo test --locked --offline -p curvebook-format`: 19 unit tests + 116 integration tests across 16 test binaries passed, 0 failed, 0 ignored; `wasm32-unknown-unknown` std target not installed in environment, so `wasm32` `#[cfg]` gating verified by inspection)
- **Summary Counts:** `S1: 3` · `S2: 7` · `S3: 3` · `S4: 1` (Total: `14`)

---

## File Coverage Table

| File | Lines | Inspected | Concrete Invariants Checked | Findings / Status |
|---|---:|---|---|---|
| `crates/format/Cargo.toml` | 27 | `1..=27` | `[lints] workspace = true`; permissive-only dependency allowlist (`crc32fast`, `flate2`, `image`, `quick-xml`, `serde`, `serde_json`, workspace crates); zero `chrono` or network/telemetry crates | `Clean` — workspace lints, clean dependencies, no `chrono`/egress |
| `crates/format/src/archive.rs` | 3114 | `1..=3114` | ZIP/unpacked round-trips, `MAX_ARCHIVE_FILE_BYTES` (128 MiB) & `ZipLimits`, symlink containment & path traversal (`resolve_contained_path`), `.curvebook.tx/` crash recovery & rollback, stale file pruning, newer-schema overwrite refusal, circular dependency/hook sanitization, schema migration (`v0..=v7`), JSON DOM amplification bounds | `FA-SEC04-05` |
| `crates/format/src/csv.rs` | 1649 | `1..=1649` | RFC-4180 quote/delimiter/BOM/EOL handling, `MAX_CSV_FILE_BYTES` (64 MiB) & `MAX_IMPORT_CELLS` (10M) pre-allocation caps, formula-injection prefix neutralization (`=`, `+`, `-`, `@`, `\t`, `\r`), `apply_csv_import_link` checked arithmetic & freeze guards, multi-archetype headers/exclusions/subcolumns | `FA-SEC04-06`, `FA-SEC04-07`, `FA-SEC04-08`, `FA-SEC04-09`, `FA-SEC04-11` |
| `crates/format/src/error.rs` | 137 | `1..=137` | Structured `FormatError` variants (`Display`/`Error`), `From<PlatformError>`/`From<DocError>`/`From<std::io::Error>`/`From<serde_json::Error>`, non-panicking warning types | `Clean` — exhaustive error mappings without panicking paths |
| `crates/format/src/export.rs` | 431 | `1..=431` | `ResultStatus::Failed` refusal helpers (`try_export_result_*`), CSV formula neutralization on headers/labels/text/p-values, non-finite float rejection across tabs/scalars/comparisons/curves/analysis | `FA-SEC04-10` (see also `FA-SEC04-04`) |
| `crates/format/src/image_asset.rs` | 154 | `1..=154` | `MAX_IMAGE_ASSET_BYTES` (32 MiB) & pixel/dimension limits, SVG/XML sniff rejection, PNG/JPEG/TIFF magic validation, full bitstream decode on ingest, SHA-256 content addressing & verification | `Clean` — bounded ingest, SVG exclusion, digest/dimension checks |
| `crates/format/src/lib.rs` | 71 | `1..=71` | `#![forbid(unsafe_code)]`, `#[cfg(not(target_arch = "wasm32"))]` on `DirectoryJournalBackend`, public re-export completeness | `FA-SEC04-04` |
| `crates/format/src/prism.rs` | 681 | `1..=681` | Bounded `.prism` container/manifest peeking, cumulative `MAX_PROJECT_CELLS` (10M) across CSV + `.pzfx` tables, legacy-format rejection, embedded `document.json` migration/hydration/validation, `SheetId` sanitization & reference remapping | `FA-SEC04-01`, `FA-SEC04-05`, `FA-SEC04-12`, `FA-SEC04-13` |
| `crates/format/src/pzfx.rs` | 1040 | `1..=1040` | Bounded XML parsing (`XmlLimits`), pre-allocation `estimate_table_cells` overflow & `MAX_IMPORT_CELLS` checks across all 8 archetypes, title clamping on UTF-8 boundaries, exclusion/warning diagnostics, Nested subcolumn title handling | `FA-SEC04-11` |
| `crates/format/src/recovery.rs` | 2090 | `1..=2090` | NDJSON CRC-32 `(seq, command_json)` integrity, torn-tail truncation & quarantine, `snapshot_covered_seq` double-apply prevention, host-fingerprint + PID lease eviction, eviction guard protocol, `MemoryJournalBackend` web storage caps, `DirectoryJournalBackend` regular-file/size guards | `FA-SEC04-02`, `FA-SEC04-03`, `FA-SEC04-14` |
| `crates/format/src/registry.rs` | 670 | `1..=670` | Extension & magic-byte format detection, `.curvebook.d` discrimination, `read_regular_file_bytes` path imports, newer-version refusal before overwrite, `Importer`/`Exporter` result & document routing | `FA-SEC04-04` |
| `crates/format/src/schemas.rs` | 411 | `1..=411` | Draft 2020-12 schema parity across archive manifest, document, `curvebook.batch.v1`, preferences, and journal/lock records against `versioned.rs`, `archive.rs`, `recovery.rs`, and CLI contracts | `Clean` — schemas and `BATCH_MANIFEST_*` inventories align cleanly |
| `crates/format/src/versioned.rs` | 400 | `1..=400` | Monotonic `upgrade()` step execution & future-version refusal, regular-file preferences I/O & overwrite guard, `wasm32` error stubs, legacy magic rejection, batch v1 schema & command alias normalization | `Clean` — bounded preferences I/O, monotonic migrations, `wasm32` stubs |
| `crates/format/src/xml.rs` | 509 | `1..=509` | `DOCTYPE`/`ENTITY` rejection (zero XXE expansion), `MAX_XML_BYTES` (64 MiB), `MAX_XML_DEPTH` (128), `MAX_XML_NODES` (2M across elements/attrs/text/comments/CDATA), duplicate-attribute rejection, UTF-8 BOM strip, safe entity decoding | `Clean` — bounded XML DOM parser without XXE, panics, or unchecked slicing |
| `crates/format/src/zip.rs` | 534 | `1..=534` | `MAX_ZIP_ENTRIES` (4096), `MAX_ZIP_ENTRY_BYTES` (64 MiB), `MAX_ZIP_UNCOMPRESSED_BYTES` (128 MiB), `DeflateDecoder::take(cap + 1)` against forged headers, `sanitize_zip_path` traversal/backslash/drive/duplicate blocks, checked EOCD/CD bounds & CRC-32 | `Clean` — strict Zip Slip, Zip Bomb, duplicate, and CRC verification |

---

## Findings (`S1` -> `S4`)

### FA-SEC04-01 · S1 · Embedded `.prism` `document.json` clones image assets unboundedly before validation and deduplication

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/prism.rs:247-257`
- **Category:** `resources`
- **Defect:** `try_load_embedded_project_json` iterates `doc.image_assets` and copies archive payload bytes (`asset.bytes = bytes.to_vec()`) **before** `validate_project_document(&doc)` checks `MAX_PROJECT_IMAGE_ASSETS` (`256`), unique `asset_id`s, or `MAX_IMAGE_ASSET_BYTES` (`32 MiB`).
- **Failure scenario:** A crafted `.prism` ZIP containing one `32 MiB` PNG entry `assets/img-0000000000000000.png` (within `MAX_ZIP_ENTRY_BYTES` and `MAX_ZIP_UNCOMPRESSED_BYTES`) and a `document.json` listing 10,000 `ProjectImageAsset` stubs with `bytes: []` referencing that same `asset_id` (~1.5 MiB JSON, passing `parse_bounded_json_outcome`) causes `try_load_embedded_project_json` to allocate ~320 GiB (`10,000 * 32 MiB`, or up to `64 MiB` per entry if sized at `MAX_ZIP_ENTRY_BYTES`) before `validate_project_document` runs at line `257`, aborting the process via OOM (and even within 256 entries, duplicate `asset_id`s amplify memory up to 8 GiB instead of failing immediately).
- **Evidence:**
```rust
// crates/format/src/prism.rs:247-257
    for asset in &mut doc.image_assets {
        if asset.bytes.is_empty() {
            let expected_path = format!("assets/{}.{}", asset.asset_id, asset.media_type.extension());
            if let Some(bytes) = archive.get(&expected_path) {
                asset.bytes = bytes.to_vec();
            }
        }
    }
    doc.data_sheets.iter().try_fold(0_usize, accumulate_project_cells)?;
    sanitize_invalid_document_sheet_ids(&mut doc, warnings);
    validate_project_document(&doc)?;
```
  Contrast `crates/format/src/archive.rs:801-825` (`hydrate_archive_image_assets`), which checks `doc.image_assets.len() > MAX_PROJECT_IMAGE_ASSETS`, tracks `seen_ids`, and removes each loaded asset once.
- **Suggested fix:** Enforce `doc.image_assets.len() <= MAX_PROJECT_IMAGE_ASSETS`, reject duplicate `asset_id`s, and validate `bytes.len() <= MAX_IMAGE_ASSET_BYTES` before cloning `bytes.to_vec()`.
- **Related:** `crates/format/src/archive.rs:801-826` (`hydrate_archive_image_assets`), `SEC-X1`.

---

### FA-SEC04-02 · S1 · Oversized `curvebook.lock.evict` triggers unbounded read and infinitely restores itself instead of evicting

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/recovery.rs:1121-1141, 1159-1165`
- **Category:** `security`
- **Defect:** When `curvebook.lock.evict` exceeds `MAX_LOCKFILE_BYTES`, `try_acquire_eviction_guard` calls `reclaim_stale_eviction_lock(&evict_path, &Vec::new())`, which reads the entire renamed file with an unbounded `std::fs::read` and restores it via `hard_link` whenever `actual_bytes != observed_bytes`.
- **Failure scenario:** If an oversized (>64 KiB, or multi-GiB) `curvebook.lock.evict` exists in a recovery directory alongside a stale `curvebook.lock`, `actual_bytes != observed_bytes` (`actual_bytes != []`) is guaranteed `true`: (1) `std::fs::read(&reclaim_path)` allocates the entire arbitrarily large file into RAM (OOM crash hazard), and (2) `std::fs::hard_link(&reclaim_path, evict_path)` immediately puts the oversized eviction lock back in place so every retry in `acquire_eviction_guard_with_retry` repeats and fails, permanently wedging lock acquisition and recovery.
- **Evidence:**
```rust
// crates/format/src/recovery.rs:1121-1140
    fn reclaim_stale_eviction_lock(&self, evict_path: &Path, observed_bytes: &[u8]) -> Result<(), FormatError> {
        let reclaim_path = self.next_unique_sibling_path("curvebook.lock.evict.reclaim");
        match std::fs::rename(evict_path, &reclaim_path) {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(FormatError::from(err)),
        }
        let mut reclaim_guard = TempFileUnlinkGuard::new(reclaim_path.clone());
        let actual_bytes = match std::fs::read(&reclaim_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(FormatError::from(err)),
        };
        if actual_bytes != observed_bytes {
            // Another contender installed a fresh guard just before rename; restore best-effort without clobbering.
            let _ = std::fs::hard_link(&reclaim_path, evict_path);
        }

// crates/format/src/recovery.rs:1159-1165
        let observed_bytes = match read_bounded_lock_bytes(&evict_path, MAX_LOCKFILE_BYTES)? {
            BoundedLockRead::Missing => return Ok(None),
            BoundedLockRead::Oversized(_) => {
                let raw = Vec::new();
                self.reclaim_stale_eviction_lock(&evict_path, &raw)?;
                return Ok(None);
            }
            BoundedLockRead::Bytes(bytes) => bytes,
        };
```
- **Suggested fix:** Bound the post-rename verification read in `reclaim_stale_eviction_lock` to `MAX_LOCKFILE_BYTES` (using `read_bounded_lock_bytes`), and pass an `ExpectedEvictionState::Oversized` marker (or remove unconditionally when oversized) so oversized eviction locks are deleted rather than read unboundedly and re-linked back.
- **Related:** `crates/format/tests/audit_r1_s4b_recovery.rs`.

---

### FA-SEC04-03 · S1 · `DirectoryJournalBackend` reads snapshots, journals, and lockfiles without regular-file checks (FIFO hang) or snapshot byte caps (OOM)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/recovery.rs:1354-1360, 1423-1429, 970-988, 1451-1466`
- **Category:** `security`
- **Defect:** `DirectoryJournalBackend::read_snapshot` (`1354`) and `read_journal_bytes` (`1423`) call raw `std::fs::read` with no symlink/regular-file check or byte size limit, while `read_bounded_lock_bytes` (`970`) and `open_journal_stream` (`1451`) call `File::open` and inspect `file.metadata()?.len()` without rejecting symlinks, FIFOs, or special device files (`symlink_metadata` / `file_type().is_file()`).
- **Failure scenario:** Placing a named pipe (FIFO), symlink to `/dev/zero` (or outside the workspace), or multi-GiB `snapshot.curvebook` in a journal directory causes `CrashRecoveryJournal::open` / `recover_journal_contents` either to hang indefinitely inside `File::open` / `read_to_end` (on a FIFO whose `len()` is `0`) or to OOM via unbounded `std::fs::read`, bypassing the regular-file guards enforced for archive readers (`#1`, `R1-X1-03`).
- **Evidence:**
```rust
// crates/format/src/recovery.rs:1354-1360
    fn read_snapshot(&self) -> Result<Option<Vec<u8>>, FormatError> {
        match std::fs::read(self.snapshot_path()) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(FormatError::from(err)),
        }
    }

// crates/format/src/recovery.rs:970-983
fn read_bounded_lock_bytes(path: &Path, max_bytes: usize) -> Result<BoundedLockRead, FormatError> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(BoundedLockRead::Missing),
        Err(err) => return Err(FormatError::from(err)),
    };
    let file_len = file.metadata()?.len();

// crates/format/src/recovery.rs:1451-1465
    fn open_journal_stream(&self, max_bytes: usize) -> Result<Option<Box<dyn std::io::BufRead + '_>>, FormatError> {
        let file = match std::fs::File::open(self.journal_path()) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Some(Box::new(std::io::empty()))),
            Err(err) => return Err(FormatError::from(err)),
        };
        let file_len = file.metadata()?.len();
```
- **Suggested fix:** Check `std::fs::symlink_metadata` before `File::open` in `read_bounded_lock_bytes`, `open_journal_stream`, and `read_matching_snapshot_meta` to reject symlinks/FIFOs/devices (`!meta.file_type().is_file()`), and replace raw `std::fs::read` in `read_snapshot` / `read_journal_bytes` with `curvebook_platform::fs::read_regular_file_bytes_if_exists` capped at `MAX_ARCHIVE_FILE_BYTES` / `MAX_JOURNAL_BYTES`.
- **Related:** `R1-X1-03` (`crates/format/tests/audit_r1_s4a_x1_03.rs`), `SEC-01` (`crates/platform/src/fs.rs`).

---

### FA-SEC04-04 · S2 · `FormatRegistry` CSV/TSV exporters and `lib.rs` re-exports bypass `#71` failed-result guard (`REGRESSION`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/registry.rs:639-669`, `crates/format/src/lib.rs:36`
- **Category:** `wrong-result`
- **Defect:** Wave 1 added `ensure_result_sheet_csv_exportable`, `try_export_result_tab_csv`, and `try_export_result_sheet_tidy_csv` in `crates/format/src/export.rs:29, 66, 91` to close `#71`, but left `registry.rs` (`CsvAdapter` / `TsvAdapter` via `export_delimited_bytes` and `export_document_delimited_text`) calling unguarded `export_result_sheet_tidy_csv`, and `lib.rs:36` omits the `try_export_result_*` functions from crate re-exports.
- **Failure scenario:** Exporting a failed `ResultSheet` (`ResultStatus::Failed { .. }`) to CSV or TSV via `FormatRegistry::default_registry().exporter_by_format("csv")` (as well as downstream callers `apps/curvebook-cli/src/cmd_convert.rs:680-686`, `crates/engine/src/cmd/table.rs:2669`, `crates/engine/src/cmd/analysis.rs:1040-1048`, `crates/mcp/src/formats.rs:1167-1179`, and `crates/ui-egui/src/control_io.rs:2351-2362`) silently returns `Ok(...)` with header-only CSV bytes instead of failing with `FormatError::InvalidPipeline`.
- **Evidence:**
```rust
// crates/format/src/registry.rs:639-669
fn export_delimited_bytes(source: ExportSource<'_>, opts: &ExportOptions, csv_opts: &CsvOptions, format_id: &str) -> Result<Vec<u8>, FormatError> {
    let text = match source {
        ExportSource::DataSheet(sheet) => export_datasheet_to_csv(sheet, csv_opts)?,
        ExportSource::ResultSheet(sheet) => export_result_sheet_tidy_csv(sheet, csv_opts),
        ExportSource::Document(doc) => export_document_delimited_text(doc, opts, csv_opts, format_id)?,
    };
    Ok(text.into_bytes())
}
...
        if let Some(rs) = doc.result_sheets.iter().find(|s| s.id.as_str() == target_id) {
            return Ok(export_result_sheet_tidy_csv(rs, csv_opts));
        }
...
    if let Some(rs) = doc.result_sheets.first() {
        return Ok(export_result_sheet_tidy_csv(rs, csv_opts));
    }

// crates/format/src/lib.rs:36
pub use export::{ResultSheetExportJson, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv};
```
  Only `crates/format/tests/audit_wave1_format_io.rs:379-392` calls `try_export_result_*`.
- **Suggested fix:** Switch `registry.rs:642, 659, 667` to `try_export_result_sheet_tidy_csv`, re-export `ensure_result_sheet_csv_exportable` and `try_export_result_*` in `crates/format/src/lib.rs:36`, and update CLI/Engine/MCP/UI callers (or enforce `ensure_result_sheet_csv_exportable` inside a single checked entry point).
- **Related:** `#71` (`docs/audit/local-code-audit-2026-11-18.md:713-719`, marked closed on `main`), `SEC-11`, `SEC-12`, `SEC-13`, `SEC-21`.

---

### FA-SEC04-05 · S2 · Migration DOM budget (`1_000_000` nodes / `64 MiB`) rejects valid `v0..=v6` projects above ~250k cells

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/archive.rs:631-633, 756-777, 790`, `crates/format/src/prism.rs:233`
- **Category:** `data-loss`
- **Defect:** `parse_and_migrate_document` streams `CURRENT_SCHEMA_VERSION` (`7`) archives directly into `ProjectDocument` (`786`), whereas any `v0..=v6` `.curvebook`/`.cbtemplate` archive (`790`) and any embedded `.prism` `document.json` (`prism.rs:233`) must first deserialize an untyped `serde_json::Value` DOM capped at `MAX_MIGRATION_JSON_NODES = 1_000_000` and `min(6 * doc_bytes.len(), 64 MiB)`.
- **Failure scenario:** Each populated `Cell` (`{"value":{"Number":1.0},"excluded":false}`, see `crates/doc/src/cell.rs:8-58`) charges 4 nodes in `BoundedValueVisitor` (outer map + `"value"` variant map + number + `excluded` bool, plus 1 row-array node). Consequently, any valid `v0..=v6` project (or `.prism` embedded `document.json`) with more than ~250,000 populated cells—just `2.5%` of `MAX_IMPORT_CELLS` (`10_000_000`) and `0.5%` of `MAX_PROJECT_CELLS` (`50_000_000` in `crates/doc/src/error.rs:35`)—or exceeding ~11 MiB of serialized JSON fails to open with `JSON DOM node count exceeds migration limit 1000000` or `JSON DOM allocation exceeds migration memory budget`.
- **Evidence:**
```rust
// crates/format/src/archive.rs:631-633, 756-760, 785-793
const MAX_MIGRATION_JSON_NODES: usize = 1_000_000;
const MIN_MIGRATION_JSON_BYTES: usize = 1_048_576;
const MAX_MIGRATION_JSON_BYTES: usize = 64 * 1024 * 1024;
...
pub(crate) fn parse_bounded_json_outcome(doc_bytes: &[u8]) -> BoundedJsonOutcome {
    let max_bytes = doc_bytes.len().saturating_mul(6).clamp(MIN_MIGRATION_JSON_BYTES, MAX_MIGRATION_JSON_BYTES);
    let mut budget = JsonDomBudget { remaining_nodes: MAX_MIGRATION_JSON_NODES, remaining_bytes: max_bytes, budget_exceeded: false };
...
    if doc_version == CURRENT_SCHEMA_VERSION && peek.schema_version.is_some() {
        let mut parsed: ProjectDocument = serde_json::from_slice(doc_bytes)?;
        parsed.schema_version = CURRENT_SCHEMA_VERSION;
        Ok(parsed)
    } else {
        let raw_value = parse_bounded_json_value(doc_bytes)?;
        let migrated_value = migrate_document_json(raw_value, doc_version)?;
        Ok(serde_json::from_value(migrated_value)?)
    }
```
- **Suggested fix:** Stream large cell arrays directly via typed versioned deserializers / custom `DeserializeSeed` for `data_sheets` (since `migrate_v1_to_v7` only modifies top-level metadata and `result_sheets`, never `Cell` payloads) or raise DOM budgets to match `MAX_ARCHIVE_FILE_BYTES` / `MAX_PROJECT_CELLS` without loading every `Cell` as a boxed `serde_json::Value` tree.
- **Related:** `crates/format/tests/audit_r1_s4a.rs:r1_s06_05_document_json_amplification_bounded_and_legacy_v0_loads`.

---

### FA-SEC04-06 · S2 · `apply_csv_import_link` corrupts `XyTable` rectangularity when growing rows on `GeneratedSequence` tables with exclusions

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/csv.rs:1445-1456`
- **Category:** `data-loss`
- **Defect:** `ensure_sheet_capacity` resizes `XyTable::x_cells` using `vec![Cell::empty(); t.x_mode.subcolumn_count()]`; under `XColumnMode::GeneratedSequence`, `subcolumn_count()` returns `0`, whereas existing rows can store 1-element exclusion-carrier vectors (`x_cells[r].len() == 1`).
- **Failure scenario:** Importing additional rows via `apply_csv_import_link` into a `GeneratedSequence` `XyTable` that already carries excluded sequence cells (`width == 1` per `crates/doc/src/archetypes.rs:252, 492-511`) appends empty `[]` rows (`len == 0`) after existing `[Cell]` rows (`len == 1`). `XyTable::validate()` and subsequent saves then fail with `DocError::InvalidCellValue("X cell row widths are not rectangular")` (and line `1451` undercounts existing exclusion-carrier slots as `0` in `check_datasets_growth_budget`).
- **Evidence:**
```rust
// crates/format/src/csv.rs:1445-1456
        TableArchetype::Xy(t) => {
            t.x_mode.validate()?;
            t.y_subcolumn_mode.validate()?;
            let subs = t.y_subcolumn_mode.subcolumn_count();
            let needed_cols = compute_needed_cols_with_subs(start_col, start_subcol, grid_cols, subs)?;
            validate_data_sets(needed_cols)?;
            let x_cells = t.x_cells.len().max(needed_rows).checked_mul(t.x_mode.subcolumn_count());
            check_datasets_growth_budget(&t.data_sets, needed_cols, needed_rows, subs, x_cells)?;
            grow_datasets(&mut t.data_sets, needed_cols, needed_rows, subs)?;
            if t.x_cells.len() < needed_rows {
                t.x_cells.resize_with(needed_rows, || vec![Cell::empty(); t.x_mode.subcolumn_count()]);
            }
```
  Per `crates/doc/src/archetypes.rs:231, 493-508`: `GeneratedSequence` has `subcolumn_count() == 0`, while `XyTable::validate` requires every `x_cells` row to match `self.x_cells.first().map_or(0, Vec::len)`.
- **Suggested fix:** Compute `x_width = match &t.x_mode { XColumnMode::GeneratedSequence { .. } => t.x_cells.first().map_or(0, Vec::len), _ => t.x_mode.subcolumn_count() }` and use `x_width` in both `x_cells` budget estimation and `resize_with`.
- **Related:** `crates/doc/src/archetypes.rs:231, 252, 492-511` (`SEC-02`).

---

### FA-SEC04-07 · S2 · `apply_csv_import_link` hardcodes `subs = 1` in `resolve_target_coord` and capacity growth for `Nested` tables

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/csv.rs:1536-1585`
- **Category:** `wrong-result`
- **Defect:** `resolve_target_coord` (`1571-1577`) and `ensure_sheet_capacity` (`1536-1543`) treat `TableArchetype::Nested` as having `1` subcolumn (`_ => 1`) rather than each group's `biological_unit_count()`, and also allow nonzero `anchor.subcol` on single-subcolumn archetypes (`Contingency`, `Survival`, `PartsOfWhole`, `MultipleVariables`) to silently overflow into subsequent `col` indices.
- **Failure scenario:** Calling `apply_csv_import_link` on a `NestedTable` with multiple biological units per treatment group (`A:1`, `A:2`, `B:1`, `B:2`):
  1. Consecutive CSV columns skip secondary biological units (`subcol >= 1`) and land on the first unit of consecutive treatment groups (`col = 0, 1, ...`, `subcol = 0`), allocating extra 1-unit groups if `grid_cols > groups.len()`; and
  2. Supplying `placement.anchor = CellCoord::new(0, 0, 1)` silently writes to `(col = 1, subcol = 0)` instead of `(col = 0, subcol = 1)` (or shifts single-subcolumn tables into column `col + 1` instead of returning `CellOutOfBounds`).
- **Evidence:**
```rust
// crates/format/src/csv.rs:1536-1543
        TableArchetype::Nested(t) => {
            let needed_cols = compute_needed_cols_with_subs(start_col, start_subcol, grid_cols, 1)?;
            validate_data_sets(needed_cols)?;
            check_nested_growth_budget(&t.groups, needed_cols, needed_rows)?;
            while t.groups.len() < needed_cols {
                let idx = t.groups.len();
                t.groups.push(NestedGroupColumn::new(default_column_label(idx), 1, needed_rows)?);
            }

// crates/format/src/csv.rs:1571-1585
fn resolve_target_coord(arch: &TableArchetype, start_col: usize, start_subcol: usize, flat_offset: usize) -> Result<(usize, usize), FormatError> {
    let subs = match arch {
        TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count(),
        TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count(),
        TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count(),
        _ => 1,
    };
    validate_subcolumns(subs)?;
    let flat = start_col
        .checked_mul(subs)
        .and_then(|v| v.checked_add(start_subcol))
        .and_then(|v| v.checked_add(flat_offset))
        .ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("target cell coordinate overflow") })?;
    Ok((flat / subs, flat % subs))
}
```
- **Suggested fix:** Walk `NestedTable::groups` sequentially using each group's `biological_unit_count()` starting at `(start_col, start_subcol)` (creating new groups only past the last group's biological units), and reject `start_subcol > 0` on single-subcolumn archetypes (`Contingency`, `Survival`, `PartsOfWhole`, `MultipleVariables`).
- **Related:** `crates/format/tests/audit_r1_s4b_doc_fmt.rs:r1_s06_13_apply_headers_to_sheet_respects_subcolumns_across_archetypes`.

---

### FA-SEC04-08 · S2 · `apply_csv_import_link` mutates `DataSheet` irreversibly before post-growth checks can fail

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/csv.rs:1306-1313, 1349-1367`
- **Category:** `data-loss`
- **Defect:** `place_grid_in_datasheet` expands columns/rows (`1349`), shifts existing data and row titles downward on `PlacementMode::InsertRows` (`1351`), writes leading column titles (`1354`), and writes cells (`1363`) in-place before `validate_title`, `resolve_target_coord`, `target.set_cell`, or `target.add_linked_region` (`1307`) finishes checking all inputs.
- **Failure scenario:** Calling `apply_csv_import_link` with `PlacementMode::InsertRows` and `propagate_title_header: true` when an interior CSV column header exceeds `MAX_TITLE_BYTES` (or when `target.set_cell` fails on an out-of-bounds subcolumn in a heterogeneous `NestedTable`) returns `Err(...)` while leaving the caller's `DataSheet` permanently expanded, existing rows shifted down, and earlier column titles/cells overwritten.
- **Evidence:**
```rust
// crates/format/src/csv.rs:1349-1367
    ensure_sheet_capacity(target, needed_rows, start_col, placement.anchor.subcol, grid_cols)?;
    if placement.mode == PlacementMode::InsertRows && start_row < existing_rows {
        target.archetype.shift_rows_down(start_row, grid_rows, existing_rows);
    }
    if let Some(hdrs) = headers {
        apply_headers_to_sheet(target, start_col, placement.anchor.subcol, hdrs)?;
    }

    let mut end_coord = CellCoord::new(start_row, start_col, placement.anchor.subcol);
    for (r_off, row) in grid.iter().enumerate() {
        let r = start_row.checked_add(r_off).ok_or_else(|| FormatError::InvalidPipeline { reason: String::from("row offset overflow") })?;
        for (c_off, cell) in row.iter().enumerate() {
            let (col, subcol) = resolve_target_coord(&target.archetype, start_col, placement.anchor.subcol, c_off)?;
            let coord = CellCoord::new(r, col, subcol);
            target.set_cell(coord, cell.clone())?;
            end_coord = coord;
        }
    }
```
- **Suggested fix:** Validate all promoted headers (`validate_title`) and target coordinates before mutating `target`, or stage mutations into a cloned `DataSheet` (within cell budget) and swap on success.
- **Related:** `crates/format/tests/archive_and_csv.rs`, `SEC-12` (`crates/engine/src/cmd/table.rs`).

---

### FA-SEC04-09 · S2 · Excluded `CellValue::Text` tokens export unquoted as `<text>*` and coerce type or lose whitespace on CSV re-import (`REGRESSION`)

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/csv.rs:306-308, 383-398`
- **Category:** `wrong-result`
- **Defect:** `CsvExportField::cell` sets `force_quote: false` (`Self::unquoted(text)`) for excluded text cells whenever `opts.parse_asterisk_exclusions` is enabled (`388-391`), while `parse_csv_field_cell` only preserves quoted fields as verbatim text (`307`) and runs `parse_cell_token` on all unquoted fields.
- **Failure scenario:** Exporting and re-importing a `DataSheet` containing excluded categorical or identifier strings—such as `"0012"` (`"0012*"` -> `Cell::excluded_number(12.0)`), `"NA"` / `"NULL"` (`"NA*"` -> `Cell { value: CellValue::Empty, excluded: true }`), or `" a "` (`" a *"` -> `Cell::excluded_text("a")`)—silently converts `Text` to `Number`, erases sentinel-named text to `Empty`, or strips outer whitespace.
- **Evidence:**
```rust
// crates/format/src/csv.rs:306-308
fn parse_csv_field_cell(field: &ParsedCsvField, opts: &CsvOptions) -> Cell {
    if field.quoted { Cell::text(field.text.clone()) } else { parse_cell_token(&field.text, opts) }
}

// crates/format/src/csv.rs:383-398
    fn cell(cell: &Cell, opts: &CsvOptions) -> Self {
        match &cell.value {
            CellValue::Empty | CellValue::Number(_) => Self::unquoted(format_cell_token(cell, opts)),
            CellValue::Text(raw) => {
                let safe = neutralize_csv_formula_text(raw);
                if cell.excluded && opts.parse_asterisk_exclusions {
                    let mut text = safe;
                    text.push('*');
                    Self::unquoted(text)
                } else {
                    let force_quote = is_ambiguous_csv_text(&safe, opts);
                    Self { text: safe, force_quote }
                }
            }
        }
    }
```
- **Suggested fix:** Support a lossless representation for excluded text on CSV round-trip (or quote excluded text tokens such as `"..."*` outside RFC-4180 quotes and recognize it in `parse_csv_field_cell` before falling back to `Cell::text`), or surface an explicit `ImportWarning` / typed error when unrepresentable excluded text must be exported as CSV.
- **Related:** `#73` (`docs/audit/local-code-audit-2026-11-18.md:729-735`, marked closed on `main`).

---

### FA-SEC04-10 · S2 · `export_result_sheet_json` fails to validate `sheet.analysis` (and `analyzable_data`) for non-finite floats before JSON serialization

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/export.rs:98-113, 386-418`
- **Category:** `wrong-result`
- **Defect:** `export_result_sheet_json` asserts that it validates no non-finite floats are silently lost (`96-97`), yet `validate_result_floats` validates only `scalars`, `tabs`, `pairwise_comparisons`, and `fitted_curves`, omitting `sheet.analysis.first_non_finite_float()` (enforced by `archive.rs:2139-2141`) and `sheet.analyzable_data` while line `104` serializes `analysis: &sheet.analysis`.
- **Failure scenario:** Exporting an in-memory `ResultSheet` containing `f64::NAN` or `f64::INFINITY` inside `sheet.analysis` (e.g., `LinearRegression.plot_x_min: Some(f64::NAN)` or `NormalizeAnchor::FixedValue(f64::INFINITY)`) returns `Ok(json)` in which `serde_json` silently emits JSON `null` for each non-finite float, dropping optional bounds or producing corrupt JSON that fails deserialization.
- **Evidence:**
```rust
// crates/format/src/export.rs:98-113
pub fn export_result_sheet_json(sheet: &ResultSheet, pretty: bool) -> Result<String, FormatError> {
    validate_result_floats(sheet)?;
    let payload = ResultSheetExportJson {
        id: &sheet.id,
        title: sheet.title(),
        status: &sheet.status,
        analysis: &sheet.analysis,
        ...
    };
    let json = if pretty { serde_json::to_string_pretty(&payload)? } else { serde_json::to_string(&payload)? };
    Ok(json)
}
```
  Whereas `archive.rs:2139-2141` calls `if let Some(field) = res.analysis.first_non_finite_float() { return Err(FormatError::NonFiniteFloat { ... }); }` (and validates `res.analyzable_data`).
- **Suggested fix:** Call `sheet.analysis.first_non_finite_float()` (returning `FormatError::NonFiniteFloat`) inside `validate_result_floats(sheet)` and validate `sheet.analyzable_data` if present.
- **Related:** `crates/doc/src/analysis/finite.rs:36-38`, `crates/format/src/archive.rs:2135-2141`.

---

### FA-SEC04-11 · S3 · `export_nested_rows` omits `NestedGroupColumn::title` when subcolumn titles lack a `Group:` prefix, breaking CSV round-trips

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/csv.rs:593-604`, `crates/format/src/pzfx.rs:621-650`
- **Category:** `data-loss`
- **Defect:** `export_nested_rows` exports each `NestedGroupColumn`'s `subcolumn_titles` verbatim without `g.title` (`599-603`), unlike `push_dataset_headers` (`628-631`), even though `.pzfx` Nested imports with `<Subcolumn><Title>` (`pzfx.rs:625-649`) store bare biological-unit labels without a `"Group:"` prefix.
- **Failure scenario:** Importing a `.pzfx` Nested table with treatment group `"Control"` and subcolumn titles `"Mouse 1"`, `"Mouse 2"` and exporting it to CSV omits `"Control"` from the CSV headers; re-importing that CSV via `import_csv_to_datasheet(..., TableKind::Nested, ...)` (`parse_nested_column_groups`, `csv.rs:901-968`) fails to match adjacent colon prefixes and splits every biological unit into separate 1-unit groups (`"Mouse 1"`, `"Mouse 2"`).
- **Evidence:**
```rust
// crates/format/src/csv.rs:599-603
    for g in &t.groups {
        for sub_title in &g.subcolumn_titles {
            header.push(CsvExportField::header(sub_title));
        }
    }

// crates/format/src/pzfx.rs:642-648
        for (s_idx, maybe_title) in titles.iter().enumerate() {
            if let Some(sub_title) = maybe_title
                && let Some(slot) = grp.subcolumn_titles.get_mut(s_idx)
            {
                *slot = sub_title.clone();
            }
        }
```
- **Suggested fix:** When exporting `NestedTable` headers in `export_nested_rows`, emit `"{}:{}"` (`g.title` + `:` + stripped `sub_title`) whenever `sub_title` does not already start with `"{}:", g.title`, matching `parse_nested_column_groups` (`csv.rs:906-914`).
- **Related:** `crates/format/tests/audit_wave1_format_io.rs:audit_73_quote_aware_csv_round_trip_and_colon_subcolumn_headers`.

---

### FA-SEC04-12 · S3 · `remap_document_sheet_references` misses `HookSource` references in analyses and inline templates

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/prism.rs:618-681`
- **Category:** `wrong-result`
- **Defect:** When `sanitize_invalid_document_sheet_ids` rewrites an invalid `InfoSheet`, `ResultSheet`, or `DataSheet` ID in embedded `.prism` `document.json`, `remap_document_sheet_references` updates `source_sheet_id` and `PowerAnalysis.info_sheet_id` (`629-638`) but omits `HookSource` targets inside `ResultSheet.analysis.kind` (`Normalize` anchors and `NonlinearRegression` `ParameterScope::HookedConstant`) and inline `{{info:...}}`/`{{result:...}}` tokens in `GraphSpec`/`LayoutSpec`.
- **Failure scenario:** Importing a `.prism` archive whose `document.json` names an `InfoSheet` or `ResultSheet` with whitespace or colons (e.g. `"Info 1"`) alongside a `Normalize` or `NonlinearRegression` analysis referencing `HookSource::InfoConstant { info_sheet_id: Some("Info 1"), .. }` sanitizes the sheet ID to `"Info-1"` while leaving the `HookSource` pointing to the deleted `"Info 1"` ID, silently breaking hook resolution.
- **Evidence:**
```rust
// crates/format/src/prism.rs:629-638
    for rs in &mut doc.result_sheets {
        apply_sheet_remap(&mut rs.analysis.id, remap);
        apply_sheet_remap(&mut rs.analysis.source_sheet_id, remap);
        if let AnalysisKind::PowerAnalysis(ref mut power) = rs.analysis.kind {
            apply_opt_sheet_remap(&mut power.info_sheet_id, remap);
        }
        if let Some(ref mut derived) = rs.analyzable_data {
            remap_datasheet_references(derived, remap);
        }
    }
```
  (Compare `crates/format/src/archive.rs:2714-2773`, which visits all of those `HookSource` carriers.)
- **Suggested fix:** Remap explicit `HookSource` sheet IDs in `TransformAnalysisSpec::Normalize`, `NonlinearFitSpec::parameter_rules`, and `{{info:...}}`/`{{result:...}}` template hooks in `GraphSpec`/`LayoutSpec` inside `remap_document_sheet_references`.
- **Related:** `crates/format/tests/audit_wave1_format_io.rs:audit_29_pzfx_and_prism_validate_and_sanitize_invalid_sheet_ids`.

---

### FA-SEC04-13 · S3 · Embedded `.prism` `document.json` maps non-`u32` / non-integer `schema_version` to `CURRENT_SCHEMA_VERSION` instead of `UnsupportedSchemaVersion`

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/prism.rs:241-246`
- **Category:** `contract-drift`
- **Defect:** Unlike `archive.rs:242-244, 781` (`clamp_peek_schema_version`), `try_load_embedded_project_json` converts `schema_version` with `.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(curvebook_doc::CURRENT_SCHEMA_VERSION)`, falling back to `CURRENT_SCHEMA_VERSION` when `schema_version` overflows `u32` or is non-integer.
- **Failure scenario:** A `.prism` archive carrying `"schema_version": 5000000000` in embedded `document.json` skips `migrate_document_json`'s `UnsupportedSchemaVersion` check and fails later at `serde_json::from_value(migrated)` (`line 246`) with an opaque `FormatError::Json` integer-range error instead of `FormatError::UnsupportedSchemaVersion`.
- **Evidence:**
```rust
// crates/format/src/prism.rs:241-246
    let version = match val.get("schema_version") {
        None | Some(serde_json::Value::Null) => 0,
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(curvebook_doc::CURRENT_SCHEMA_VERSION),
    };
    let migrated = migrate_document_json(val, version)?;
    let mut doc: ProjectDocument = serde_json::from_value(migrated)?;

// crates/format/src/archive.rs:242-244
fn clamp_peek_schema_version(raw: u64) -> u32 {
    u32::try_from(raw).unwrap_or(u32::MAX)
}
```
- **Suggested fix:** Use `u32::try_from(n).unwrap_or(u32::MAX)` for `u64` `schema_version` values (matching `clamp_peek_schema_version`) and return a clear `FormatError::Json` error when `schema_version` is present with a non-unsigned-integer type.
- **Related:** `crates/format/src/archive.rs:242-248, 781`.

---

### FA-SEC04-14 · S4 · `build_cleaned_journal_bytes` emits duplicate trailing newlines between repaired NDJSON records

- **Base:** `7c85a8e516165ddadb101236a88eec937bf1f095`
- **Location:** `crates/format/src/recovery.rs:1874-1883`
- **Category:** `cleanup`
- **Defect:** `JournalOpRecord::to_ndjson_line` already appends `'\n'` (`334`), and `build_cleaned_journal_bytes` unconditionally pushes a second `b'\n'` (`1880`).
- **Failure scenario:** Rewriting `ops.journal` during `truncate_trailing_corrupt_tail` or `upgrade_all_records_to_current` inserts a blank line between every serialized NDJSON record on disk instead of compact newline-delimited JSON.
- **Evidence:**
```rust
// crates/format/src/recovery.rs:331-336
    pub fn to_ndjson_line(&self) -> Result<String, FormatError> {
        let mut line = serde_json::to_string(self)?;
        line.push('\n');
        Ok(line)
    }

// crates/format/src/recovery.rs:1874-1883
fn build_cleaned_journal_bytes(seqs: &[u64], crcs: &[u32], commands: &[serde_json::Value]) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    for ((&seq, &crc32), command_json) in seqs.iter().zip(crcs.iter()).zip(commands.iter()) {
        let record = JournalOpRecord { format_version: CURRENT_JOURNAL_VERSION, seq, crc32, command_json: command_json.clone() };
        let line = record.to_ndjson_line()?;
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}
```
- **Suggested fix:** Remove `out.push(b'\n')` at `crates/format/src/recovery.rs:1880` (or guard with `if !line.ends_with('\n')`).
- **Related:** `none`.
