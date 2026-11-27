# Files, persistence, crash recovery and interchange formats

<!-- Audience: Tier 2 Technical Reference — exhaustive container schemas, recovery limits, and interchange details belong here; link to Tier 1 guides (../open-pzfx-files.html or get-started.md) for scientist walkthroughs. -->

This technical reference specifies CurveBook's native project containers, schema versioning, write-ahead crash recovery, and the rules for importing and exporting other formats. For a plain-language guide to opening `.pzfx` and `.prism` files, see [Open `.pzfx` and `.prism` files](../open-pzfx-files.html). For setup, see [Get started and installation](get-started.md).

For wire-level format details, see the [Empirical `.prism` and `.pzfx` format specification](../pzfx-format-specification.html), the [File Formats Specification](../file-formats.md), and the [JSON Schemas](../schemas/document.schema.json).

---

## Native project containers and templates {#native-formats}

### `.curvebook` deterministic ZIP archive {#curvebook-zip}

A `.curvebook` file is a standard ZIP archive. CurveBook writes it with a deterministic entry order and timestamps, and validates its contents with CRC-32 checksums:

| Entry path | Contents | Integrity and limits |
|---|---|---|
| `manifest.json` | Format tag, schema version, sheet index, and asset descriptors. | Validated against `archive-manifest.schema.json` and `document_crc32`. |
| `document.json` | Canonical project JSON, indented with 2 spaces, without inline image bytes. | Validated against `document.schema.json`, and checked for finite floats, limits, and an acyclic DAG. |
| `sheets/<id>.csv` | An RFC-4180 CSV snapshot of each data table (`*` marks excluded cells). | Per-sheet CRC-32, verified against `manifest.json`. |
| `assets/<asset_id>.<ext>` | Content-addressed embedded images in PNG, JPEG, or TIFF format. | Up to 32 MiB and 67,108,864 pixels; verified with CRC-32 and SHA-256. |

Archive structure and safety limits:

- **Manifest metadata:** `manifest.json` records `format: "curvebook-archive"`, `schema_version`, `kind`, `title`, and `document_crc32`. It also lists a descriptor for each sheet (`id`, `section`, `title`, `csv_path`, `crc32`) and for each asset (`asset_id`, `media_type`, `pixel_size`, `asset_path`, `crc32`).
- **Hard size and path caps:** When CurveBook opens an archive, it enforces caps of 128 MiB archive size, 4,096 ZIP entries, 128 MiB of uncompressed data in total, and 64 MiB per entry. Strict path checks reject `..` traversal, absolute paths, and symlink escapes.
- **Atomic writes:** Every save to disk writes a temporary file, syncs it to disk, and then renames it into place atomically.

### `.curvebook.d/` unpacked project directory {#unpacked-dir}

CurveBook can also read and write a project as an unpacked `.curvebook.d/` directory, which gives clean line-by-line Git diffs:

- The directory mirrors the `.curvebook` layout exactly on disk: `manifest.json`, the 2-space-indented `document.json`, `sheets/<id>.csv`, and `assets/<asset_id>.<ext>`.
- CurveBook keeps files that are not part of the archive, such as `.git/`, READMEs, or notes. It prunes stale sheet CSVs and unused image assets that earlier manifests tracked.
- On open, CurveBook canonicalizes the root path and applies symlink boundary checks, archive size limits, and CRC-32 validation.

### `.cbtemplate` reusable templates {#cbtemplate}

Saving a template (`project.saveTemplate`) writes a `.cbtemplate` archive with the project-specific data removed. The removed items are raw table observations, exclusion notes, plate well origins, navigator sheet origin badges, cached result numbers, and imported result snapshots. CurveBook also prunes dangling graph overlays, layout items, and active or anchor selection links that referred to the dropped imported results.

A template keeps:

- Table archetypes, column headers, subcolumn modes, and `=` Calculated Variable formulas.
- Info sheet constants that hooks refer to.
- Complete analysis pipelines, parameter rules and sharing, graph styling, axis settings, live equations, and page layout geometry.
- Embedded images that are referenced. Unreferenced images are dropped.

Instantiating a template (`project.instantiateTemplate`) creates a new, independent project that is ready for incoming data.

### `.cbplate` microplate layouts {#cbplate}

Reusable role layouts for 96-well (8×12) and 384-well (16×24) microplates are saved as UTF-8 JSON files with the single extension `.cbplate` (`format_version = 1`; see [`plate-map.schema.json`](../schemas/plate-map.schema.json)). These files are separate from `.cbtemplate` project archives and from `.json` result exports.

Legacy plate JSON without a version number is upgraded in memory from `v0` to `v1`. Files with `format_version > 1` are refused on open and never overwritten on save. Every write is atomic.

### Complete-workspace sharing vs. linked data {#sharing}

A `.curvebook` archive bundles all data tables, exclusion notes, plate well origins, info constants, results, graphs, and page layouts into one self-contained file. Projects with import links to external files on disk keep the provenance metadata of those sources. When the headless CLI or the MCP server runs with `--workdir` containment, external links outside the allowed workspace root are rejected.

---

## Schema versioning and forward-overwrite refusal {#schema-versioning}

| Format or record | Current version | Migration and compatibility |
|---|---:|---|
| Project and template (`document.json`, `manifest.json`) | 9 | Upgrades schemas 0 through 9 automatically. The additive `v8 -> v9` migration adds display metadata, result provenance, `MeanSd`/`MeanSem`, Column asymmetric summary modes, and sheet origin badges. On load, a dangling `active_sheet_id` or `family_anchor_sheet_id` is cleared with a warning. Newer versions (`> 9`) are rejected. |
| Plate map layout (`.cbplate`) | 1 | Upgrades legacy `v0` to `v1` in memory; rejects `format_version > 1`. |
| User preferences (`preferences.json`) | 1 | Preserves unknown extension keys; refuses to overwrite a file with `version > 1`. |
| Recovery WAL (`ops.journal` / `curvebook.lock`) | 2 / 1 | Validates the NDJSON and the CRC-32 of each line; upgrades `v1` journals to `v2` in memory. |

Before CurveBook overwrites an existing `.curvebook`, `.cbtemplate`, `.curvebook.d/`, or `.cbplate` target, this forward-overwrite guard inspects the metadata on disk. If the file has a higher schema version than the running binary supports, CurveBook refuses the write (`unsupported_version`) and leaves the newer file byte-for-byte untouched.

---

## WAL crash recovery and autosave {#crash-recovery}

Desktop GUI sessions (`curvebook`) keep a write-ahead crash recovery journal in the platform recovery directory, or in `<portable>/curvebook-data/recovery/`. Setting `CURVEBOOK_NO_PREFS=1` turns the journal off. The journal uses three files:

| Recovery file | Purpose | Limits and integrity |
|---|---|---|
| `curvebook.lock` | A lock lease that stops concurrent processes from corrupting the journal. | Reclaims expired leases and leases held by dead local PIDs; keeps parallel sessions isolated. |
| `snapshot.curvebook` | A periodic baseline archive, written on open and at structural checkpoints. | The same CRC-32 checks and archive caps as a project file (64 MiB JSON cap in web storage). |
| `ops.journal` | An append-only NDJSON log of the engine commands that change the project. | Capped at 32 MiB in total, 16 MiB per line, and 65,536 records. |

Recovery journal details:

- `curvebook.lock` records `owner_id`, `host_id`, `acquired_epoch_ms`, and `ttl_ms`.
- Each `ops.journal` record has the form `{ format_version, seq, crc32, command_json }` (journal version 2). Legacy `v1` records are checked against their CRC-32 and upgraded in memory before replay.
- If a crash truncates the end of the journal or a CRC-32 check fails, replay stops at the last valid record and reports recovery diagnostics.

---

## Supported interchange formats and CSV parsing rules {#interchange-and-csv}

### Supported imports

- **Native formats:** `.curvebook` project archives, `.curvebook.d/` unpacked directories, and `.cbtemplate` templates.
- **Delimited text:** `.csv` and `.tsv` files, and pasted delimited text (`table.importCsv`, `table.paste`).
- **`.xlsx` workbooks (`table.importXlsx`):** CurveBook imports worksheet tables with bounded OOXML parsing (see [Tabular import options](tables.md#tabular-import-options)).
- **`.pzfx` files (`table.importPzfx`):**
  - CurveBook imports all eight table archetypes from `<Table>`, `<HugeTable>`, and `<Table1024>` elements, and from XSLT-wrapped files.
  - It preserves replicate, `MeanSd`/`MeanSem`, and parametric summary modes, and the Column asymmetric and quartile summary modes.
  - It also preserves rich text, the Unicode mapping of Symbol-font characters, row, column, and subcolumn titles, display decimals, `Excluded="1"` masks, floating notes, and info sheets, whether top-level or per table.
  - Legacy binary `.pzf` files are not supported. This includes `PCFF`-prefixed binary files that carry a `.pzfx` extension (observed in about 15–18% of public `.pzfx`-named files). These files fail with an error before XML decoding starts. Re-save them as XML `.pzfx` or `.prism` files before opening them.
- **`.prism` files (`table.importPrism`, import-only at Beta):**
  - CurveBook imports all eight table archetypes, together with info sheets, from the ID-indexed headerless CSVs. Table import covers generated X sequences, replicate, summary, and offset modes, subcolumn titles, decimals, exclusions, and rich text.
  - **Live recompute of verified analyses:** Analyses matching CurveBook's corpus-verified designs (with no untranslated options) import as live `Computed` result sheets and recompute automatically on open across desktop, CLI, MCP, and merge imports (`table.importPrism` with `merge: true`). Promoted designs are:
    - Two-tailed unpaired, paired, and Welch *t*-tests.
    - Ordinary one-way ANOVA with no post-test, Tukey, or Dunnett multiple comparisons.
    - Ordinary two-way ANOVA with interaction and no post-test.
    - Descriptive statistics, area under the curve, and outlier detection on single-subcolumn or pooled replicate tables.
  - **Read-only snapshots for other designs:** Other designs and analyses with untranslated options (such as one-tailed tests, additive two-way ANOVA without interaction, or row-averaged multi-subcolumn outlier detection) are imported as clearly labelled read-only snapshots with the status `Imported` or `ImportedStale`, never `UpToDate`. Imported snapshots never feed scalar hooks or upstream DAG inputs. Methods reports list them separately under *"Imported results (not computed by CurveBook)"*.
  - **Transient verification, disagreement warnings and fallback:** On open, CurveBook compares each live recomputed analysis against the file's stored results in memory only; cached `.prism` reference values are never persisted in `.curvebook` files. If a recomputed scalar or pairwise post-test value differs from the stored value beyond tolerance, CurveBook emits a per-file `RecomputedResultDisagreement` warning naming the analysis, quantity, and both values, and CurveBook's live result always stands. If live recomputation fails on degenerate inputs, CurveBook falls back automatically to the cached snapshot (`Imported`) and emits an `AnalysisRecomputeFallback` warning. Corpus verification evidence and tolerances live in the [interop format evidence ledger](../evidence/interop-format-notes.md).
  - For graphs and layouts stored as opaque binary entries, CurveBook extracts the raster preview PNGs, with bounded ZIP/ZIP64 validation.

### Project merging, origin badges and import warnings

- **Import warnings:** Warnings that do not stop a `.pzfx`, `.prism`, or `.xlsx` import appear in the desktop **Import Warnings** panel. Examples are skipped archive parts, opaque binary graph or layout entries, unsupported elements, recomputed-vs-stored result disagreements, and snapshot fallbacks. Reopen the panel from the status bar or with **File › Import › Import Warnings…**. Its contents are also available through `ui.inspect`.
- **Project merging:** **File › Merge Project…** (`project.merge`) copies sheets from another `.curvebook`, `.curvebook.d/`, `.cbtemplate`, `.pzfx`, `.prism`, or `.xlsx` file into the current project. Drag-and-drop onto an occupied workspace, CLI batch runs, and MCP `open_file` in append mode merge in the same way. A merge creates a one-time copy with no live links between the files.
- **ID remapping and origin badges:** When sheet IDs collide, CurveBook remaps them automatically in every typed and text-embedded reference. The open project's file path, active selection, and checkpoints stay unchanged. Each merged sheet gets a navigator origin badge, which you can remove (`sheet.clearOrigin`). The badge stores only the source filename and format, never the full directory path.
- **Replace-or-merge prompt:** If you open or import a `.prism`, `.pzfx`, or `.xlsx` file into an occupied or unsaved project, CurveBook asks before replacing the workspace. This applies to **File › Open…**, **File › Import**, and drag-and-drop. Choose **Add to current project** (`merge: true`) or **Open as new project** (`merge: false`).

### Supported exports

- **Native formats:** `.curvebook`, `.curvebook.d/`, and `.cbtemplate`.
- **`.pzfx` data-only export:** `.pzfx` export saves your data tables and info sheets only. Analyses, graphs and layouts stay in your `.curvebook` project. Each export writes an atomic copy. It never replaces the active `.curvebook` path or changes the unsaved-changes state. The desktop app shows a preflight summary before writing. The export also returns a structured report that lists every dropped item and every feature with insufficient corpus coverage. Written to match the `.pzfx` structure of real files; not tested in other applications. Files that match this structure may still be rejected by other applications for non-structural reasons, such as checksums, cross-references, or version gates.
- **`.prism` export status:** `.prism` export is not shipped in the Beta. Real `.prism` archives require undocumented binary and digest entries: `.dt` tokens, and `PCFF` binary `.bin` entries for graphs, layouts, and fonts. Use `.pzfx` export for data interchange instead.
- **CSV and JSON exports:** Export a single data table (`table.exportCsv`), or export all tables at once with **Export All Data Tables…** (`ui.export.allTablesCsv`). That command writes one file per table, named after the table's sanitized title. For results, export a single-tab or tidy CSV (`analysis.exportCsv`, which adds `provenance` as the last tidy column) or structured JSON (`analysis.exportJson`, which includes the result `provenance`).
- **Publication figures:** Export vector SVG, PDF 1.7, and ECMA-234 `.emf` files, or PNG and TIFF rasters (see [Graphs and export capabilities](graphs.md#export-capabilities)).
- **Methods reports:** **Save Methods Report…** (`result.generateReport`) writes an analytical summary as `plain`, `markdown`, or `latex`, chosen by the file extension (`.txt`, `.md`, `.tex`). Imported `.prism` snapshots appear in a separate section, *"Imported results (not computed by CurveBook)"*.

### CSV and TSV parsing and formatting rules {#csv-rules}

- **Size limits:** Up to 64 MiB per CSV or TSV file and 10,000,000 dense imported cells.
- **RFC-4180 quotes and BOM:** The parser strips a leading UTF-8 BOM (`\u{FEFF}`), honors standard `""` quote escaping, and keeps newlines that are embedded inside quoted fields.
- **Delimiter auto-detection:** The parser inspects the first 10 non-empty lines to choose between `\t` (TSV), `;` (European CSV with `,` decimals), and `,` (standard CSV with `.` decimals).
- **Exclusions and missing values:** When `parse_asterisk_exclusions` is `true` (the default), numbers prefixed with `*` (such as `*4.52`) import as excluded values. Empty cells and the missing-value sentinels `NA`, `NaN`, and `null` import as empty cells.
- **Default export delimiters:** By default, `analysis.exportCsv` exports all result tabs in tidy format. It exports a single tab when you specify `tab_id` or `tab_index`. Both `analysis.exportCsv` and `table.exportCsv` pair the active locale's decimal separator with a matching delimiter: a `,` delimiter with `.` decimals, or a `;` delimiter with `,` decimals. Saving with a `.tsv` extension selects `\t` automatically.
