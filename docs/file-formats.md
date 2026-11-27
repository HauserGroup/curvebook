# Persisted File Formats & Versioning

CurveBook versions every persisted artefact so older projects migrate forward deterministically and newer projects are never silently damaged by an older binary. Machine-readable schemas generated from source live under `docs/schemas/*.schema.json` (`archive-manifest.schema.json`, `document.schema.json`, `batch-v1.schema.json`, `preferences.schema.json`, `journal-record.schema.json`, `plate-map.schema.json` via `cargo xtask schemas`, with schema `$id` URIs under `https://github.com/HauserGroup/curvebook/blob/main/docs/schemas/<filename>`).

---

## 1. Persisted Format Inventory

| File / Directory | Contents | Owner Crate(s) | Current Version | Notes |
| :--- | :--- | :--- | :--- | :--- |
| **`.curvebook`** | Compressed archive (`manifest.json`, `document.json`, `sheets/*.csv`, `assets/<asset_id>.<ext>`) | `curvebook-doc`, `curvebook-format` (`crates/format/src/archive.rs`) | `CURRENT_SCHEMA_VERSION = 9` (`crates/doc/src/error.rs`; `FORMAT_MAGIC = "curvebook-archive"`) | Format ID `curvebook` (`application/vnd.curvebook.project`, UTI `org.curvebook.project`); CRC32-checked entries + SHA-256 content-addressed PNG/JPEG/TIFF image assets; saved via atomic write (`curvebook-platform` / temp + rename). |
| **`.curvebook.d/`** | Unpacked project directory (`manifest.json`, `document.json`, `sheets/*.csv`, `assets/<asset_id>.<ext>`) | `curvebook-doc`, `curvebook-format` (`crates/format/src/archive.rs`) | `CURRENT_SCHEMA_VERSION = 9` | Format ID `curvebook.d`; version-control friendly workspace layout with the same manifest/document versioning and asset storage. |
| **`.cbtemplate`** | Reusable project template ZIP (`ArchiveKind::Template`) | `curvebook-doc`, `curvebook-format` (`crates/format/src/archive.rs`) | `CURRENT_SCHEMA_VERSION = 9` | Format ID `cbtemplate` (`application/vnd.curvebook.template`, UTI `org.curvebook.template`); data cell values, `exclusion_notes`, `plate_well_origins`, sheet `origin` badges, cached results, and imported result snapshots stripped (with dangling graph overlays, layout items, and active/anchor sheet references pruned); formulas, analyses, graphs, layouts, info constants, and referenced embedded image assets preserved (unreferenced assets dropped). |
| **`.cbplate`** | Reusable microplate layout UTF-8 JSON (`PlateMapFile`: `{ format_version, name, geometry, x_title, wells }`) | `curvebook-doc`, `curvebook-format` (`crates/format/src/plate.rs`, `crates/format/src/registry.rs`) | `CURRENT_PLATE_MAP_VERSION = 1` (`crates/format/src/plate.rs`) | Format ID `cbplate` (`CurveBook Plate Map Layout`, `DESC_PLATE_MAP`, `docs/schemas/plate-map.schema.json`); single-extension versioned JSON distinct from `.cbtemplate` ZIP project templates and `.json` result exports; migrates legacy unversioned `v0 -> v1`, refuses `format_version > 1` on load and refuses overwriting newer files on save (`FormatError::UnsupportedSchemaVersion`), and writes atomically via `curvebook_platform::fs::write_atomic`. |
| **`preferences.json`** *(and `shortcuts.json`)* | Desktop UI settings (`PreferencesFile`: locale, theme, custom shortcut bindings, preserved unknown fields) | `curvebook` (`apps/curvebook/src/main.rs`), `curvebook-platform` | `CURRENT_PREFERENCES_VERSION = 1` (`crates/format/src/versioned.rs`) | Atomic writes only; must never overwrite a newer version file written by a newer release (`docs/schemas/preferences.schema.json`). |
| **`ops.journal` & `curvebook.lock`** | Crash-recovery NDJSON command WAL (`{ format_version, seq, crc32, command_json }`) + `JournalLockLease` JSON (`{ format_version, owner_id, acquired_epoch_ms, ttl_ms }`) *(with optional `snapshot.curvebook`)* | `curvebook-format` (`crates/format/src/recovery.rs`, `crates/format/src/versioned.rs`) | `CURRENT_JOURNAL_VERSION = 2`, `CURRENT_LOCK_VERSION = 1` | Reads both `v1` and `v2`; `v1` records verify raw CRC32 **before** upgrading retired command parameter aliases to canonical `v2` spellings; new writes emit `format_version = 2`; recovered commands replay via `Caller::Replay` (`Session::open_or_recover` / `Session::open_with_best_effort_recovery`); schemas in `docs/schemas/journal-record.schema.json`. |
| **`curvebook.batch.v1`** | Headless CLI batch execution manifest (`{ schema, template \| project, inputs, commands, exports }`) | `curvebook-format` (`crates/format/src/versioned.rs`), `curvebook-cli` (`apps/curvebook-cli/src/cmd_run_batch.rs`) | `BATCH_SCHEMA_V1 = "curvebook.batch.v1"` | Exact `"schema"` tag verified (`validate_batch_manifest_version`) and deprecated section/command aliases upgraded in memory (`normalize_batch_manifest_v1`) before execution (`docs/schemas/batch-v1.schema.json`); unsupported `curvebook.batch.vN` versions are rejected cleanly. |

*(External interchange formats — `.pzfx`, `.prism`, `.xlsx`, `.csv`, `.tsv`, `.svg`, `.pdf`, `.png`, `.tiff`, and `.emf` — are read and exported defensively with bounded clean-room parsers and do not own CurveBook's native schema history. See §1.1 below for `.pzfx`, `.prism`, legacy `.pzf`, and project-merge boundaries.)*

### 1.1 External `.pzfx`, `.prism`, legacy `.pzf`, and Project Merging

- **`.pzfx` XML import:** Supports all eight table archetypes (`XY`, `Column`, `Grouped`, `Nested`, `Contingency`, `Survival`, `PartsOfWhole`, `MultipleVariables`), `<HugeTable>` and `<Table1024>`, XSLT-wrapped files, rich text and Symbol-font Unicode mapping, subcolumn titles, display decimals, `Excluded="1"` cell masks, floating notes, top-level and per-table info sheets, `MeanSd`/`MeanSem` two-subcolumn modes, and Column asymmetric/quartile summary modes (`MeanLowHigh`, `UpperLowerLimits`, `MedianIqr`). Non-tabular XML sections (graphs, analyses, layouts, opaque `<Template>` payloads) are skipped with typed warnings.
- **Legacy binary `.pzf` refusal:** Older binary `.pzf` files — including `PCFF`-prefixed binary files carrying a `.pzfx` extension (observed in ~15–18% of public `.pzfx`-named files; `18.1%` in [`docs/evidence/interop-format-notes.md`](evidence/interop-format-notes.md)) — are **not supported** and are refused before UTF-8 decoding with `FormatError::UnsupportedFormat`. Re-save them as XML `.pzfx` or `.prism` before opening.
- **`.pzfx` data-only export:** `.pzfx` export saves your data tables and info sheets only. Analyses, graphs and layouts stay in your `.curvebook` project. Writes are atomic and always act as an export copy (never altering the open `.curvebook` file path or unsaved state), sharing a preflight summary and structured `ExportReport` listing every dropped item and any feature with insufficient corpus coverage. Written to match the `.pzfx` structure of real files; not tested in other applications — residual risk remains that structure-conformant files may still be rejected by other applications for non-structural reasons such as checksums, cross-references, or version gates.
- **`.prism` import (import-only at Beta):** Reads data tables across all eight archetypes (ID-indexed headerless CSVs, generated X sequences, exclusions, rich text, subcolumn titles, decimals), info sheets, preferred analysis results as clearly labelled read-only snapshots (`ResultProvenance::Imported`, `ResultStatus::Imported` / `ImportedStale`, never `UpToDate`, never feeding CurveBook calculations or hooks, and reported under *"Imported results (not computed by CurveBook)"* in methods reports), and raster preview PNGs for opaque binary graphs/layouts. **`.prism` export is not shipped as a supported format at Beta** because real `.prism` archives require undocumented 128-bit `.dt` tokens and legacy `PCFF` binary `.bin` entries ([`docs/evidence/interop-format-notes.md`](evidence/interop-format-notes.md)); use `.pzfx` export for data interchange instead.
- **Project merging & navigator origin badges:** `project.merge` (**File › Merge Project…**, drag-and-drop merge, CLI batch, and MCP `open_file` append mode) copies data, info, results (preserving imported snapshot provenance), graphs, layouts, and referenced images into the open project as a one-time copy with consolidated sheet-ID remapping and no live cross-file links. Merged sheets carry an optional `SheetOrigin { source_label, source_format, merged_at }` badge (storing only the source filename without path separators, removable via `sheet.clearOrigin` and stripped when saving `.cbtemplate`).

### 1.2 Archive, Save, Reopen, and Import Size Limits

Every native save path (`.curvebook`, `.cbtemplate`, `.curvebook.d/`), native reopen path, and external project importer (`.pzfx`, `.prism`, `.xlsx`) enforces one consistent set of native archive safety limits so that **any project accepted by an importer saves and reopens cleanly**, and any input that would exceed native archive limits is refused up front at import with the same actionable `FormatError::ZipBomb` or `FormatError::FileTooLarge` error.

| Surface / Container | Compressed / Raw Input Cap | ZIP Entries Cap | Per-Entry Uncompressed Cap | Cumulative Uncompressed Cap | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Native `.curvebook` / `.cbtemplate`** (`native_archive_zip_limits()` in `crates/format/src/archive.rs`) | `MAX_ARCHIVE_FILE_BYTES = 128 MiB` (enforced on both save and open) | `MAX_ZIP_ENTRIES = 4,096` | `MAX_ARCHIVE_ENTRY_BYTES = 256 MiB` | `MAX_ARCHIVE_UNCOMPRESSED_BYTES = 256 MiB` | `document.json` writes canonical 2-space pretty JSON whenever it fits within `64 MiB` per entry and `128 MiB` total uncompressed, and falls back deterministically to compact schema-v9 JSON for larger projects up to `256 MiB` (still `CURRENT_SCHEMA_VERSION = 9`). |
| **Native `.curvebook.d/` unpacked directory** (`crates/format/src/archive.rs`) | N/A (directory entries) | `MAX_ZIP_ENTRIES = 4,096` | `MAX_ARCHIVE_ENTRY_BYTES = 256 MiB` | `MAX_ARCHIVE_UNCOMPRESSED_BYTES = 256 MiB` | Shares `build_archive_entries`, compact-JSON fallback, and `native_archive_zip_limits()` with `.curvebook` so ZIP and unpacked directory save/reopen behave identically. |
| **Generic ZIP container default** (`ZipLimits::default()` in `crates/format/src/zip.rs`) | `MAX_ARCHIVE_FILE_BYTES = 128 MiB` | `MAX_ZIP_ENTRIES = 4,096` | `MAX_ZIP_ENTRY_BYTES = 64 MiB` | `MAX_ZIP_UNCOMPRESSED_BYTES = 128 MiB` | Enforced on raw `ZipArchive::from_bytes`, `.xlsx` ZIP extraction, and web `VirtualWorkspaceFolder`. |
| **External `.prism` import** (`external_archive_zip_limits()` in `curvebook-format`) | `MAX_ARCHIVE_FILE_BYTES = 128 MiB` | `MAX_ARCHIVE_ZIP_ENTRIES = 32,768` (lazy central-directory index) | `MAX_ZIP_ENTRY_BYTES = 64 MiB` (read entries) | `MAX_ZIP_UNCOMPRESSED_BYTES = 128 MiB` (cumulative read bytes) | Also enforces `MAX_PROJECT_CELLS = 10,000,000`, `validate_project_document`, and `validate_archive_save_limits` (native `256 MiB` uncompressed / `128 MiB` compressed save limits) up front before returning `Ok`. |
| **External `.pzfx` import** (`crates/format/src/pzfx/import.rs`, `crates/format/src/xml.rs`) | `MAX_XML_BYTES = 64 MiB` (`FormatRegistry` disk reader); up to `MAX_XML_OPAQUE_BYTES = 128 MiB` raw XML when skipping opaque `<Template>` bodies | N/A (`MAX_XML_NODES = 2,000,000`, `MAX_XML_DEPTH = 128`) | N/A | `MAX_IMPORT_CELLS = 10,000,000` dense cells | Enforces `validate_project_document` and `validate_archive_save_limits` (native `256 MiB` uncompressed / `128 MiB` compressed save limits) up front before returning `Ok`. |
| **External `.xlsx` import** (`crates/format/src/xlsx.rs`) | `MAX_ARCHIVE_FILE_BYTES = 128 MiB` | `MAX_ZIP_ENTRIES = 4,096` | `MAX_ZIP_ENTRY_BYTES = 64 MiB` | `MAX_ZIP_UNCOMPRESSED_BYTES = 128 MiB` | Also enforces `MAX_IMPORT_CELLS = 10,000,000`, `validate_project_document`, and `validate_archive_save_limits` up front before returning `Ok`. |
| **Delimited `.csv` / `.tsv` import** (`crates/format/src/csv.rs`) | `MAX_CSV_FILE_BYTES = 64 MiB` | N/A | N/A | `MAX_IMPORT_CELLS = 10,000,000` dense cells | Produces a single `DataSheet`; saving the enclosing project still enforces `native_archive_zip_limits()` and `MAX_ARCHIVE_FILE_BYTES`. |

---

Schema v6 adds the source cell coordinates for cached ROUT exclusions. In v5 projects, the exclusion list starts empty until the fit is recalculated.

Schema v7 adds project-level content-addressed embedded raster image assets (`image_assets[]` in `document.json` with empty inline bytes, binary PNG/JPEG/TIFF payloads stored under `assets/<asset_id>.<ext>`, and CRC-32/SHA-256 metadata tracked in `ArchiveManifest::assets`), together with `GraphAnnotation::EmbeddedImage` and `LayoutItem::EmbeddedImage`. Projects saved under `v6` migrate forward cleanly (`v6 -> v7`) with an empty `image_assets` catalog; SVG image import is intentionally omitted under CurveBook's permissive-only dependency policy ([`licences.md`](licences.md)).

Schema v8 (`migrate_v7_to_v8`) adds:
- `AnalysisKind::LeveneTest` (`LeveneTestSpec { center: LeveneCenter::Median | LeveneCenter::Mean }` for standalone Levene / Brown–Forsythe homogeneity-of-variance analysis),
- `ImportFilterSpec::row_match` (`Option<ColumnEqualsFilter>` for exact trimmed column-equality filtering on `.xlsx`, `.csv`, and live-linked imports),
- `AnalysisKind::AreaUnderCurve` (`AreaUnderCurveSpec` and `AucBaseline` for standalone partial-window trapezoidal AUC analysis), and
- `DataSheet::plate_well_origins` (`Vec<PlateCellOrigin>` mapping microplate-imported cells back to physical well IDs, preserved during `.curvebook` streaming archive deserialization in `DataSheetVisitor` and cleared when stripping project templates via `strip_project_data_for_template`).

Schema v9 (`CURRENT_SCHEMA_VERSION = 9`, `migrate_v8_to_v9`, all additive with `serde(default)`) adds:
- Table display metadata: `DataSetColumn.{subcolumn_titles, display_decimals}`, `XyTable.{x_subcolumn_titles, x_display_decimals}`, and `VariableColumn.display_decimals` (validated `0..=15` decimals and bounded subcolumn title lengths),
- Existence validation for `ProjectDocument.{active_sheet_id, family_anchor_sheet_id}` (dangling IDs fail `validate()`, and project loaders clear dangling selections before validation with a warning),
- Imported result provenance: `ResultSheet.provenance` (`ResultProvenance::Computed` default, or `ResultProvenance::Imported`), `ResultStatus::{Imported, ImportedStale}`, and `AnalysisKind::ImportedSnapshot` (non-user-creatable, `analyzable_data: None` invariant, excluded from scalar hooks and upstream DAG inputs, dropped in `.cbtemplate` with dangling references pruned),
- Two-subcolumn no-`N` summary modes `SubcolumnMode::{MeanSd, MeanSem}` across XY (Y and `XColumnMode::Numbers`), Column, and Grouped tables, plus `MeanLowHigh`, `UpperLowerLimits`, and `MedianIqr` on `ColumnTable` governed by the canonical summary-mode matrix, and
- `origin: Option<SheetOrigin>` on all five sheet structs for removable navigator origin badges on merged sheets (wired into `DataSheetVisitor`, `RawResultSheet`, and cleared in `strip_project_data_for_template`).

Projects saved under `v0` through `v8` migrate forward cleanly (`v8 -> v9`), while older `v8` binaries refuse `v9` archives cleanly with `FormatError::UnsupportedSchemaVersion`.

`ops.journal` `v2` (paired with `curvebook.batch.v1` normalization via `normalize_batch_manifest_v1`) upgrades retired command parameter spellings from `v1` records after validating CRC32 over the raw `v1` `(seq, command_json)` payload and recomputing the in-memory CRC32 for the canonicalized `v2` `command_json`. New `ops.journal` appends write `format_version = 2` with canonical `snake_case` parameters.

## 2. Forward & Backward Compatibility Policy

1. **Backward compatibility (older files in newer CurveBook):**
   - Files tagged with `0 <= version <= CURRENT` are upgraded sequentially in memory (`v0 -> v1 -> ... -> CURRENT`) through registered migration steps before deserializing typed Rust models.
   - Defaults for newly added optional fields are applied explicitly inside the migration step or versioned loader—never silently fabricated during error recovery.
2. **Forward safety (newer files in older CurveBook):**
   - Reading a manifest, plate layout, or settings wrapper whose `version` exceeds what the running binary understands **must fail cleanly** with a typed error (`FormatError::UnsupportedSchemaVersion { version, max_supported }`) telling the user a newer version of CurveBook created the file.
   - **Never overwrite a newer version:** Neither project save paths, `.cbplate` savers, nor desktop settings/preferences savers may overwrite an on-disk file whose persisted version is higher than `max_supported`, preventing silent loss of fields added in newer releases.
3. **Atomic persistence:**
   - All project, template, plate layout, settings, and manifest saves write through a temporary sibling file -> flush/sync -> atomic rename (`curvebook-platform`), so crashes never truncate existing files ([docs/pitfalls.md](pitfalls.md)).

---

## 3. Adding a `VersionStep` Migration & Updating Schemas

Whenever a change alters persisted JSON structure, adds required fields, or changes serialized semantics:

1. **Bump the version constant:** Increment `CURRENT_SCHEMA_VERSION` in `crates/doc/src/error.rs` (used by `crates/format/src/archive.rs`), or the target format's `CURRENT_PLATE_MAP_VERSION` in `crates/format/src/plate.rs` / `CURRENT_PREFERENCES_VERSION` in `crates/format/src/versioned.rs` / `CURRENT_JOURNAL_VERSION` / `CURRENT_LOCK_VERSION` in `crates/format/src/recovery.rs`.
2. **Add a `VersionStep`:** Implement a pure `from_version -> from_version + 1` migration step transforming the serialized JSON payload (updating `"schema_version"`/`"version"`/`"format_version"` alongside migrated fields) and append it sequentially to `MIGRATION_STEPS` in `crates/format/src/archive.rs`, `PLATE_MAP_MIGRATION_STEPS` in `crates/format/src/plate.rs`, or `PREFERENCES_MIGRATION_STEPS` in `crates/format/src/versioned.rs` (a `const` assert ties each slice's length to its version constant). Never skip step numbers or mutate files in place during migration tests before validation succeeds.
3. **Add golden round-trip & forward-refusal tests:** Keep a minimal representative fixture of the previous version under `crates/format/tests/` (and register any fixture rows in `ASSETS.md` if outside test code). Assert both successful upgrade to current and clean refusal (`UnsupportedSchemaVersion`) on `CURRENT + 1`.
4. **Regenerate & verify schemas:**

```sh
cargo xtask schemas
cargo xtask schemas --check
```

`cargo xtask schemas` updates `docs/schemas/*.schema.json`; `cargo xtask schemas --check` fails in CI if persisted structs/manifests changed without regenerating the schema snapshots.

---

## 4. Breaking Cutover, Legacy Rejection & Residual-Name Allowlist

1. **Full breaking cutover (`R0.20`):**
   - CurveBook is a **full breaking cutover** with **zero LumenCraft file readers, executable or environment-variable aliases, settings/recovery migrations, or browser-storage migrations**.
   - **Renaming does not convert:** Renaming an older `.lumencraft`, `.lumencraft.d/`, or `.lctemplate` file or directory to `.curvebook`, `.curvebook.d/`, or `.cbtemplate` does **not** convert old archives. `lumencraft-archive` manifest magic, `.lumencraft` / `.lumencraft.d` / `.lctemplate` native format IDs, and `lumencraft.batch.v1` batch manifests are **explicitly rejected** (including against `.pzfx` / interchange importer fallbacks).
   - Existing user `.lumencraft` files, old settings/recovery directories, old browser `localStorage` / Web Lock namespaces, and separate LumenCraft Windows installations are left untouched; any conversion utility or migration service is outside this cutover.
2. **Narrow residual-name allowlist (`plan/curvebook-rename.md`):**
   - Residual case-insensitive `lumencraft`, `lctemplate`, or `lc-` / `lc_` references in tracked text exist solely for:
     - [`plan/curvebook-rename.md`](../plan/curvebook-rename.md) (the before/after cutover specification);
     - Legacy-rejection negative tests (synthesized in test code without committed binary fixtures) and breaking-change / install-separation notes (`docs/file-formats.md`, `docs/release/releasing.md`, `docs/STATUS.md`, `packaging/windows/INSTALLATION.md`);
     - Historical provenance records (`docs/history/roadmap.md`, dated pre-cutover baseline records in `docs/ROADMAP.md` and `docs/STATUS.md`, `R0.20`'s heading/anchor `#r020-rename-lumencraft-to-curvebook`, `assets/app-icon/source/seven-pane-win95.prompt.txt`, archived mole attribution, and `xtask` guard negative tests).
   - Residual old names never create runtime compatibility, fallbacks, or current product instructions.
