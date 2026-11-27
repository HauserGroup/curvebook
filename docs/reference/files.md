# Files, persistence, crash recovery & interchange formats

<!-- Audience: Tier 2 Technical Reference — exhaustive container schemas, recovery limits, and interchange details belong here; link to Tier 1 guides (../open-pzfx-files.html or get-started.md) for scientist walkthroughs. -->

This technical reference specifies CurveBook's native project containers, schema versioning, write-ahead crash recovery, and interchange import/export rules. For a plain-language guide to opening `.pzfx` and `.prism` files, see [Open `.pzfx` and `.prism` files](../open-pzfx-files.html) (or [Get started & installation](get-started.md) for setup).

For wire-level format details, see the [Empirical `.prism` and `.pzfx` format specification](../pzfx-format-specification.html), [File Formats Specification](../file-formats.md), and [JSON Schemas](../schemas/document.schema.json).

---

## Native project containers & templates {#native-formats}

### `.curvebook` deterministic ZIP archive {#curvebook-zip}

A `.curvebook` file is a standard ZIP archive written with deterministic entry order, timestamps, and CRC-32 validation:

| Entry path | Contents | Integrity & limits |
|---|---|---|
| `manifest.json` | Format tag, schema version, sheet index, and asset descriptors. | Validated against `archive-manifest.schema.json` and `document_crc32`. |
| `document.json` | Canonical 2-space-indented project JSON without inline image bytes. | Validated against `document.schema.json`, finite floats, limits, and acyclic DAG. |
| `sheets/<id>.csv` | RFC-4180 CSV snapshot for every data table (`*` marks excluded cells). | Per-sheet CRC-32 verified against `manifest.json`. |
| `assets/<asset_id>.<ext>` | Content-addressed PNG, JPEG, or TIFF embedded images. | Up to 32 MiB and 67,108,864 pixels; CRC-32 and SHA-256 verified. |

Archive structure and safety limits:

- **Manifest metadata:** `manifest.json` records `format: "curvebook-archive"`, `schema_version`, `kind`, `title`, and `document_crc32`. It also lists per-sheet descriptors (`id`, `section`, `title`, `csv_path`, `crc32`) and asset descriptors (`asset_id`, `media_type`, `pixel_size`, `asset_path`, `crc32`).
- **Hard size and path caps:** Opening an archive enforces caps of 128 MiB archive size, 4,096 ZIP entries, 128 MiB total uncompressed bytes, and 64 MiB per entry. Strict path checks reject `..` traversal, absolute paths, and symlink escapes.
- **Atomic writes:** Every disk save writes to a temporary file, syncs to disk, and renames atomically into place.

### `.curvebook.d/` unpacked project directory {#unpacked-dir}

CurveBook also reads and writes unpacked `.curvebook.d/` directories for clean line-by-line Git diffs:

- Mirrors the exact `.curvebook` layout (`manifest.json`, 2-space-indented `document.json`, `sheets/<id>.csv`, `assets/<asset_id>.<ext>`) directly on disk.
- Preserves non-archive files (such as `.git/`, READMEs, or notes) while pruning stale sheet CSVs and unused image assets tracked by previous manifests.
- Enforces root canonicalization, symlink boundary checks, archive size limits, and CRC-32 validation on open.

### `.cbtemplate` reusable templates {#cbtemplate}

Saving a template (`project.saveTemplate`) writes a `.cbtemplate` archive stripped of project-specific data. Stripped items include raw table observations, exclusion notes, plate well origins, navigator sheet origin badges, cached result numbers, and imported result snapshots. CurveBook also prunes dangling graph overlays, layout items, and active/anchor selection links that referenced dropped imported results.

Templates retain:

- Table archetypes, column headers, subcolumn modes, and `=` Calculated Variable formulas.
- Info sheet constants used as hook targets.
- Complete analysis pipelines, parameter rules and sharing, graph styling, axis settings, live equations, and page layout geometry.
- Referenced embedded images (unreferenced images are dropped).

Instantiating a template (`project.instantiateTemplate`) creates a fresh independent project ready for incoming data.

### `.cbplate` microplate layouts {#cbplate}

Reusable 96-well (8×12) and 384-well (16×24) microplate role layouts persist as single-extension `.cbplate` UTF-8 JSON files (`format_version = 1`; see [`plate-map.schema.json`](../schemas/plate-map.schema.json)). They are distinct from `.cbtemplate` project archives and `.json` result exports.

Legacy unversioned plate JSON upgrades cleanly in memory (`v0` to `v1`). Files with `format_version > 1` are refused on open and never overwritten on save. Every write is atomic.

### Complete-workspace sharing vs. linked data {#sharing}

A `.curvebook` archive bundles all data tables, exclusion notes, plate well origins, info constants, results, graphs, and page layouts into one self-contained file. Projects with external disk import links keep their source provenance metadata. Under headless CLI and MCP `--workdir` containment, external links outside the allowed workspace root are rejected.

---

## Schema versioning & forward-overwrite refusal {#schema-versioning}

| Format / record | Current version | Migration & compatibility |
|---|---:|---|
| Project & template (`document.json`, `manifest.json`) | 9 | Upgrades schemas 0 through 9 automatically (`v8 -> v9` additive migration for display metadata, result provenance, `MeanSd`/`MeanSem`, Column asymmetric summary modes, and sheet origin badges); clears dangling `active_sheet_id`/`family_anchor_sheet_id` on load with a warning; rejects newer versions (`> 9`). |
| Plate map layout (`.cbplate`) | 1 | Upgrades legacy `v0` to `v1` in memory; rejects `format_version > 1`. |
| User preferences (`preferences.json`) | 1 | Preserves unknown extension keys; refuses overwrite if `version > 1`. |
| Recovery WAL (`ops.journal` / `curvebook.lock`) | 2 / 1 | Validates NDJSON and CRC-32 per line; upgrades `v1` journals to `v2` in memory. |

**Forward-overwrite guard:** Before overwriting an existing `.curvebook`, `.cbtemplate`, `.curvebook.d/`, or `.cbplate` target, CurveBook inspects the metadata on disk. If the file carries a schema version higher than the running binary supports, CurveBook refuses the write (`unsupported_version`) and leaves the newer file untouched byte-for-byte.

---

## WAL crash recovery & autosave {#crash-recovery}

Desktop GUI sessions (`curvebook`, unless `CURVEBOOK_NO_PREFS=1` is set) maintain a write-ahead crash recovery journal in the platform recovery directory (or `<portable>/curvebook-data/recovery/`):

| Recovery file | Purpose | Limits & integrity |
|---|---|---|
| `curvebook.lock` | Lock lease preventing concurrent processes from corrupting the journal. | Reclaims expired leases and dead local PIDs safely; isolates parallel sessions. |
| `snapshot.curvebook` | Periodic baseline archive written on open and at structural checkpoints. | Same CRC-32 and archive caps (64 MiB JSON cap in web storage). |
| `ops.journal` | Append-only NDJSON log of mutating engine commands. | Caps at 32 MiB total, 16 MiB per line, and 65,536 records. |

Recovery journal details:

- `curvebook.lock` tracks `owner_id`, `host_id`, `acquired_epoch_ms`, and `ttl_ms`.
- `ops.journal` writes `{ format_version, seq, crc32, command_json }` (journal version 2). Legacy `v1` records upgrade in memory after CRC-32 verification before replay.
- If a crash truncates the journal tail or triggers a CRC-32 mismatch, replay stops cleanly at the last valid record and emits recovery diagnostics.

---

## Supported interchange formats & CSV parsing rules {#interchange-and-csv}

### Supported imports

- **Native formats:** `.curvebook` project archives, `.curvebook.d/` unpacked directories, and `.cbtemplate` templates.
- **Delimited text:** `.csv`, `.tsv`, and pasted delimited text (`table.importCsv`, `table.paste`).
- **`.xlsx` workbooks (`table.importXlsx`):** Imports worksheet tables with bounded OOXML parsing (see [Tabular import options](tables.md#tabular-import-options)).
- **`.pzfx` files (`table.importPzfx`):**
  - Imports all eight table archetypes across `<Table>`, `<HugeTable>`, `<Table1024>`, and XSLT-wrapped files.
  - Preserves replicate, `MeanSd`/`MeanSem`, parametric summary, and Column asymmetric/quartile summary modes.
  - Preserves rich text, Symbol-font Unicode mapping, row/column/subcolumn titles, display decimals, `Excluded="1"` masks, floating notes, and top-level or per-table info sheets.
  - Legacy binary `.pzf` files—including `PCFF`-prefixed binary files carrying a `.pzfx` extension (observed in ~15–18% of public `.pzfx`-named files)—are not supported and fail cleanly before XML decoding. Re-save them as XML `.pzfx` or `.prism` files before opening.
- **`.prism` files (`table.importPrism`, import-only at Beta):**
  - Imports all eight table archetypes from ID-indexed headerless CSVs (including generated X sequences, replicate/summary/offset modes, subcolumn titles, decimals, exclusions, and rich text) alongside info sheets.
  - Imports preferred analysis results as clearly labelled read-only snapshots (`Imported` or `ImportedStale`, never `UpToDate`). Imported snapshots never feed scalar hooks or upstream DAG inputs, and methods reports isolate them under *"Imported results (not computed by CurveBook)"*.
  - Extracts raster preview PNGs for opaque binary graphs and layouts with bounded ZIP/ZIP64 validation.

### Project merging, origin badges & import warnings

- **Import warnings:** Non-fatal warnings from `.pzfx`, `.prism`, and `.xlsx` imports (such as skipped archive parts, opaque binary graph/layout entries, or unsupported elements) appear in the desktop **Import Warnings** panel. Reopen it from the status bar or **File › Import › Import Warnings…** (also exposed via `ui.inspect`).
- **Project merging:** **File › Merge Project…** (`project.merge`), drag-and-drop onto an occupied workspace, CLI batch, and MCP `open_file` append mode copy sheets from another `.curvebook`, `.curvebook.d/`, `.cbtemplate`, `.pzfx`, `.prism`, or `.xlsx` file into the current project. Merging creates a one-time copy with no live cross-file links.
- **ID remapping & origin badges:** Colliding sheet IDs remap automatically across every typed and text-embedded reference, while the open project's file path, active selection, and checkpoints stay untouched. Merged sheets receive a removable navigator origin badge (`sheet.clearOrigin`) storing only the source filename and format (never full directory paths).
- **Replace-or-merge prompt:** Opening or importing `.prism`, `.pzfx`, or `.xlsx` files via **File › Open…**, **File › Import**, or drag-and-drop into an occupied or unsaved project prompts before replacing the workspace. Choose **Add to current project** (`merge: true`) or **Open as new project** (`merge: false`).

### Supported exports

- **Native formats:** `.curvebook`, `.curvebook.d/`, and `.cbtemplate`.
- **`.pzfx` data-only export:** `.pzfx` export saves your data tables and info sheets only. Analyses, graphs and layouts stay in your `.curvebook` project. Writes are atomic export copies that never replace the active `.curvebook` path or unsaved-changes state. The desktop app shows a preflight summary before writing, and export returns a structured report listing every dropped item and any feature with insufficient corpus coverage. Written to match the `.pzfx` structure of real files; not tested in other applications — structure-conformant files may still be rejected by other applications for non-structural reasons such as checksums, cross-references, or version gates.
- **`.prism` export status:** `.prism` export is not shipped at Beta. Real `.prism` archives require undocumented binary and digest entries (`.dt` tokens and `PCFF` binary `.bin` graph, layout, and font payloads); use `.pzfx` export for data interchange instead.
- **CSV & JSON exports:** Export individual data tables (`table.exportCsv`) or all tables at once via **Export All Data Tables…** (`ui.export.allTablesCsv`, writing one file per table named from its sanitized title). Export single-tab or tidy results CSV (`analysis.exportCsv`, appending `provenance` as the final tidy column) and structured results JSON (`analysis.exportJson`, including result `provenance`).
- **Publication figures:** Export vector **SVG**, **PDF 1.7**, **PNG**, **TIFF**, and ECMA-234 **`.emf`** (see [Graphs & Export Capabilities](graphs.md#export-capabilities)).
- **Methods reports:** **Save Methods Report…** (`result.generateReport`) writes `plain`, `markdown`, or `latex` analytical summaries selected by extension (`.txt`, `.md`, `.tex`). Imported `.prism` snapshots appear separately under *"Imported results (not computed by CurveBook)"*.

### CSV / TSV parsing & formatting rules {#csv-rules}

- **Size limits:** Up to 64 MiB per CSV/TSV file and 10,000,000 dense imported cells.
- **RFC-4180 quotes and BOM:** Strips leading UTF-8 BOMs (`\u{FEFF}`), honors standard `""` quote escaping, and preserves embedded newlines inside quoted fields.
- **Delimiter auto-detection:** Inspects the first 10 non-empty lines to distinguish `\t` (TSV), `;` (European CSV with `,` decimals), and `,` (standard CSV with `.` decimals).
- **Exclusions and missing sentinels:** When `parse_asterisk_exclusions` is `true` (default), numeric values prefixed with `*` (such as `*4.52`) import as excluded numbers. Empty cells and missing sentinels (`NA`, `NaN`, `null`) import as empty cells.
- **Default export delimiters:** `analysis.exportCsv` defaults to exporting all result tabs in tidy format (or one tab when `tab_id` or `tab_index` is specified). Both `analysis.exportCsv` and `table.exportCsv` pair the active locale's decimal separator with its matching delimiter (`,` with `.`, or `;` with `,`). Saving with a `.tsv` extension selects `\t` automatically.
