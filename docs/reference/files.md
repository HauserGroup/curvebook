# Files, Persistence, Crash Recovery & Interchange Formats

CurveBook stores studies in deterministic archives and directories, protects unsaved edits with a CRC-checked Write-Ahead Log (WAL), enforces explicit schema-version safety guards against accidental downgrades/overwrites, and imports delimited and project interchange formats strictly at bounded format edges.

See [File Formats Specification](../file-formats.md) and [JSON Schemas](../schemas/document.schema.json) for wire-level definitions.

---

## Native Project Containers & Templates {#native-formats}

### `.curvebook` Deterministic ZIP Archive {#curvebook-zip}

A `.curvebook` file (`ArchiveKind::Project`) is a standard ZIP container compressed reproducibly with deterministic entry order, timestamps, and CRC-32 validation (`crates/format/src/archive.rs`):

| Entry Path | Contents | Integrity / Limits |
|---|---|---|
| `manifest.json` | `ArchiveManifest`: `format: "curvebook-archive"`, `schema_version`, `kind`, `title`, `document_crc32`, and per-sheet descriptors (`id`, `section`, `title`, `csv_path`, `crc32`). | Validated against `archive-manifest.schema.json` and checked against `document_crc32`. |
| `document.json` | Canonical 2-space-indented `ProjectDocument` JSON (`schema_version = CURRENT_SCHEMA_VERSION`). | Validated against `document.schema.json`, finite floats, dimensions, and acyclic DAGs. |
| `sheets/<id>.csv` | RFC-4180 CSV snapshots for every `DataSheet` (`*` marks excluded cells). | Per-sheet CRC-32 recorded in `manifest.json`. |

**Archive Safety Limits (`ZipLimits::default()` & `archive.rs`):** `MAX_ARCHIVE_FILE_BYTES = 128 MiB`, `MAX_ZIP_ENTRIES = 4,096`, `MAX_ZIP_UNCOMPRESSED_BYTES = 128 MiB`, `MAX_ZIP_ENTRY_BYTES = 64 MiB`, strict path sanitization rejecting `..` traversal, absolute paths, and symlink escapes. All disk writes go through atomic temp-file write + rename (`platform_write_atomic`).

### `.curvebook.d/` Unpacked Project Directory {#unpacked-dir}

CurveBook also supports reading and writing unpacked, Git-friendly project directories (`.curvebook.d/`, via `open_project_unpacked_dir` / `save_project_unpacked_dir` / `is_unpacked_project_dir`):

- Mirrors the exact `.curvebook` internal layout (`manifest.json`, 2-space-indented `document.json`, `sheets/<id>.csv`) directly on disk for clean line-by-line version-control diffs.
- Preserves non-archive repository files (such as `.git/`, READMEs, or notes) while pruning only stale sheet CSVs tracked by previous manifests.
- Enforces strict root canonicalization, symlink boundary checks, `ZipLimits`, and CRC-32 verification on load.

### `.cbtemplate` Reusable Templates {#cbtemplate}

Saving a template (`project.saveTemplate` -> `.cbtemplate`, `ArchiveKind::Template`) calls `strip_project_data_for_template`, which clears project-specific raw table cell observations and cached computed `ResultSheet` tab numbers while preserving:

- Table archetypes, column headers, subcolumn layouts, and `=` Calculated Variable formulas
- `InfoSheet` constants (`HookSource` targets)
- Complete `AnalysisSpec` pipelines, parameter rules/sharing, `GraphSpec` styling/axes/equations, and `LayoutSpec` panel geometry

Instantiating (`project.instantiateTemplate`) creates a fresh independent project ready for incoming data.

### Complete-Workspace Sharing vs. Linked Data {#sharing}

A `.curvebook` archive embeds all `DataSheet` values, exclusion notes, `InfoSheet` constants, `ResultSheet` outputs, `GraphSheet`s, and `LayoutSheet`s inside a single self-contained file for collaborator sharing. Projects referencing external disk imports via `ImportLinkSpec` retain provenance metadata; headless/MCP opens under `--workdir` reject stored external link escapes outside the allowed workspace root.

---

## Schema Versioning & Forward-Overwrite Refusal {#schema-versioning}

| Format / Record | Current Version Constant | Migration & Safety Rule |
|---|---:|---|
| Project / Template (`document.json`, `manifest.json`) | `CURRENT_SCHEMA_VERSION = 5` | Migrations (`migrate_document_json`, `VersionStep`) automatically upgrade schemas `0 -> 1 -> 2 -> 3 -> 4 -> 5`. Files with `schema_version > CURRENT_SCHEMA_VERSION` fail on open with `UnsupportedSchemaVersion` (`unsupported_version`). |
| User Preferences (`preferences.json`) | `CURRENT_PREFERENCES_VERSION = 1` | Unknown extension fields preserved via `additionalProperties`; `version > 1` triggers warning/refusal without silent overwrite. |
| Recovery WAL (`ops.journal` & `curvebook.lock`) | `CURRENT_JOURNAL_VERSION = 1`<br>`CURRENT_LOCK_VERSION = 1` | Strict NDJSON line validation and per-record CRC-32 checking. |

**Forward-Overwrite Guard:** Before writing to an existing `.curvebook`, `.cbtemplate`, or `.curvebook.d/` target (`save_project_file`, `save_template_file`, `save_project_unpacked_dir`), CurveBook inspects the existing target's metadata on disk and **refuses to overwrite** any file or directory carrying `schema_version > CURRENT_SCHEMA_VERSION`, leaving newer collaborator files intact byte-for-byte.

---

## WAL Crash Recovery (`CrashRecoveryJournal`) & Autosave {#crash-recovery}

Desktop GUI sessions (`curvebook`, unless `CURVEBOOK_NO_PREFS=1` is set) attach a directory-backed WAL `CrashRecoveryJournal` (`crates/format/src/recovery.rs`) under the platform recovery directory (or `<portable>/curvebook-data/recovery/`):

| Recovery File | Purpose | Limits & Integrity |
|---|---|---|
| `curvebook.lock` | `JournalLockLease` (`owner_id`, `host_id`, `acquired_epoch_ms`, `ttl_ms`) preventing concurrent journal corruption across active processes. | Automatically detects expired leases and dead local PIDs on the same host for safe reclaim (`is_dead_same_host_owner`), or isolates parallel instances into separate directories. |
| `snapshot.curvebook` | Full periodic baseline archive snapshot written on open and at structural/checkpoint commands (`command_requires_wal_checkpoint`, or bounded journal rotation). | Same CRC-32 & archive limits (`64 MiB` web storage JSON cap). |
| `ops.journal` | Append-only NDJSON log of `JournalOpRecord { format_version, seq, crc32, command_json }` flushed after mutating engine commands. | `MAX_JOURNAL_BYTES = 32 MiB`, `MAX_JOURNAL_LINE_BYTES = 16 MiB`, `MAX_JOURNAL_RECORDS = 65,536`. Tail truncation or CRC-32 mismatch stops replay cleanly at the last valid record and emits `JournalRecoveryWarning` diagnostics. |

---

## Supported Interchange Formats & CSV Parsing Rules {#interchange-and-csv}

All import and export adapters register through `FormatRegistry` (`crates/format/src/registry.rs`):

- **Supported Imports:**
  - `.curvebook` (native project archive) and `.curvebook.d/` (unpacked directory)
  - `.cbtemplate` (native template archive)
  - `.csv` / `.tsv` / delimited text (`table.importCsv`, `table.paste`)
  - `.pzfx` XML files (`table.importPzfx`, parsing tables, subcolumn modes, row/column titles, `Excluded="1"` masks, and Info constants strictly at the format boundary)
  - `.prism` ZIP containers (`table.importPrism`, bounded ZIP reader extracting tabular CSVs, optional table/info metadata JSON, or embedded XML tables while emitting non-fatal `ImportWarning` entries for unrecognized binary or script chunks)
- **Supported Exports:**
  - `.curvebook`, `.curvebook.d/`, `.cbtemplate`
  - Table CSV (`export_datasheet_to_csv`), Result sheet single-tab or tidy CSV (`analysis.exportCsv`), Result sheet structured JSON (`analysis.exportJson`)
  - Publication figures: **SVG**, **`%PDF-1.7`**, **PNG**, **TIFF**, and ECMA-234 **`.emf`** ([Graphs & Export Capabilities](graphs.md#export-capabilities))
  - Analytical methods summary (`result.generateReport`: `plain`, `markdown`, `latex`).

### CSV / TSV Parsing & Formatting Rules (`CsvOptions`) {#csv-rules}

- **Limits:** `MAX_CSV_FILE_BYTES = 64 MiB`, `MAX_IMPORT_CELLS = 10,000,000` dense cells.
- **RFC-4180 Quotes & BOM:** Strips leading UTF-8 BOM (`\u{FEFF}`), honors standard double-quote escaping (`""`), and preserves embedded newlines within quoted fields.
- **Auto-Detection (`CsvOptions::detect`):** Inspects the first `10` non-empty lines to distinguish `\t` (TSV), `;` (European CSV with `,` decimal), and `,` (standard CSV with `.` decimal). Presets include `CsvOptions::csv()`, `CsvOptions::tsv()`, and `CsvOptions::european_csv()`.
- **Exclusion Syntax & Missing Sentinels:** When `parse_asterisk_exclusions == true` (default), numeric tokens prefixed with `*` (e.g., `*4.52`) import as excluded numeric cells (`Cell { value: Number(4.52), excluded: true }`). Empty cells and recognized missing sentinels (e.g., `NA`, `NaN`, `null`) map cleanly to `CellValue::Empty`.
