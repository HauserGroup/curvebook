# Persisted File Formats & Versioning

CurveBook versions every persisted artefact so older projects migrate forward deterministically and newer projects are never silently damaged by an older binary. Machine-readable schemas generated from source live under `docs/schemas/*.schema.json` (`archive-manifest.schema.json`, `document.schema.json`, `batch-v1.schema.json`, `preferences.schema.json`, `journal-record.schema.json` via `cargo xtask schemas`, with schema `$id` URIs under `https://github.com/HauserGroup/curvebook/blob/main/docs/schemas/<filename>`).

---

## 1. Persisted Format Inventory

| File / Directory | Contents | Owner Crate(s) | Current Version | Notes |
| :--- | :--- | :--- | :--- | :--- |
| **`.curvebook`** | Compressed archive (`manifest.json`, `document.json`, `sheets/*.csv`, `assets/<asset_id>.<ext>`) | `curvebook-doc`, `curvebook-format` (`crates/format/src/archive.rs`) | `CURRENT_SCHEMA_VERSION = 7` (`crates/doc/src/error.rs`; `FORMAT_MAGIC = "curvebook-archive"`) | Format ID `curvebook` (`application/vnd.curvebook.project`, UTI `org.curvebook.project`); CRC32-checked entries + SHA-256 content-addressed PNG/JPEG/TIFF image assets; saved via atomic write (`curvebook-platform` / temp + rename). |
| **`.curvebook.d/`** | Unpacked project directory (`manifest.json`, `document.json`, `sheets/*.csv`, `assets/<asset_id>.<ext>`) | `curvebook-doc`, `curvebook-format` (`crates/format/src/archive.rs`) | `CURRENT_SCHEMA_VERSION = 7` | Format ID `curvebook.d`; version-control friendly workspace layout with the same manifest/document versioning and asset storage. |
| **`.cbtemplate`** | Reusable project template ZIP (`ArchiveKind::Template`) | `curvebook-doc`, `curvebook-format` (`crates/format/src/archive.rs`) | `CURRENT_SCHEMA_VERSION = 7` | Format ID `cbtemplate` (`application/vnd.curvebook.template`, UTI `org.curvebook.template`); data cell values and cached results stripped; formulas, analyses, graphs, layouts, info constants, and referenced embedded image assets preserved (unreferenced assets dropped). |
| **`preferences.json`** *(and `shortcuts.json`)* | Desktop UI settings (`PreferencesFile`: locale, theme, custom shortcut bindings, preserved unknown fields) | `curvebook` (`apps/curvebook/src/main.rs`), `curvebook-platform` | `CURRENT_PREFERENCES_VERSION = 1` (`crates/format/src/versioned.rs`) | Atomic writes only; must never overwrite a newer version file written by a newer release (`docs/schemas/preferences.schema.json`). |
| **`ops.journal` & `curvebook.lock`** | Crash-recovery NDJSON command WAL (`{ format_version, seq, crc32, command_json }`) + `JournalLockLease` JSON (`{ format_version, owner_id, acquired_epoch_ms, ttl_ms }`) *(with optional `snapshot.curvebook`)* | `curvebook-format` (`crates/format/src/recovery.rs`, `crates/format/src/versioned.rs`) | `CURRENT_JOURNAL_VERSION = 2`, `CURRENT_LOCK_VERSION = 1` | Reads both `v1` and `v2`; `v1` records verify raw CRC32 **before** upgrading retired command parameter aliases to canonical `v2` spellings; new writes emit `format_version = 2`; recovered commands replay via `Caller::Replay` (`Session::open_or_recover` / `Session::open_with_best_effort_recovery`); schemas in `docs/schemas/journal-record.schema.json`. |
| **`curvebook.batch.v1`** | Headless CLI batch execution manifest (`{ schema, template \| project, inputs, commands, exports }`) | `curvebook-format` (`crates/format/src/versioned.rs`), `curvebook-cli` (`apps/curvebook-cli/src/cmd_run_batch.rs`) | `BATCH_SCHEMA_V1 = "curvebook.batch.v1"` | Exact `"schema"` tag verified (`validate_batch_manifest_version`) and deprecated section/command aliases upgraded in memory (`normalize_batch_manifest_v1`) before execution (`docs/schemas/batch-v1.schema.json`); unsupported `curvebook.batch.vN` versions are rejected cleanly. |

*(External interchange importers/exporters such as `.pzfx`, `.prism`, `.csv`, `.tsv`, `.svg`, `.pdf`, `.png`, `.tiff`, and `.emf` are read/exported defensively with bounds checks and clean-room parsers, and do not own CurveBook's native schema history.)*

---

Schema v6 adds the source cell coordinates for cached ROUT exclusions. In v5 projects, the exclusion list starts empty until the fit is recalculated.

Schema v7 adds project-level content-addressed embedded raster image assets (`image_assets[]` in `document.json` with empty inline bytes, binary PNG/JPEG/TIFF payloads stored under `assets/<asset_id>.<ext>`, and CRC-32/SHA-256 metadata tracked in `ArchiveManifest::assets`), together with `GraphAnnotation::EmbeddedImage` and `LayoutItem::EmbeddedImage`. Projects saved under `v6` migrate forward cleanly (`v6 -> v7`) with an empty `image_assets` catalog; SVG image import is intentionally omitted under CurveBook's permissive-only dependency policy ([`licences.md`](licences.md)).

`ops.journal` `v2` (paired with `curvebook.batch.v1` normalization via `normalize_batch_manifest_v1`) upgrades retired command parameter spellings from `v1` records after validating CRC32 over the raw `v1` `(seq, command_json)` payload and recomputing the in-memory CRC32 for the canonicalized `v2` `command_json`. New `ops.journal` appends write `format_version = 2` with canonical `snake_case` parameters.

## 2. Forward & Backward Compatibility Policy

1. **Backward compatibility (older files in newer CurveBook):**
   - Files tagged with `0 <= version <= CURRENT` are upgraded sequentially in memory (`v0 -> v1 -> ... -> CURRENT`) through registered migration steps before deserializing typed Rust models.
   - Defaults for newly added optional fields are applied explicitly inside the migration step or versioned loader—never silently fabricated during error recovery.
2. **Forward safety (newer files in older CurveBook):**
   - Reading a manifest or settings wrapper whose `version` exceeds what the running binary understands **must fail cleanly** with a typed error (`FormatError::UnsupportedSchemaVersion { version, max_supported }`) telling the user a newer version of CurveBook created the file.
   - **Never overwrite a newer version:** Neither project save paths nor desktop settings/preferences savers may overwrite an on-disk file whose persisted version is higher than `max_supported`, preventing silent loss of fields added in newer releases.
3. **Atomic persistence:**
   - All project, template, settings, and manifest saves write through a temporary sibling file -> flush/sync -> atomic rename (`curvebook-platform`), so crashes never truncate existing files ([docs/pitfalls.md](pitfalls.md)).

---

## 3. Adding a `VersionStep` Migration & Updating Schemas

Whenever a change alters persisted JSON structure, adds required fields, or changes serialized semantics:

1. **Bump the version constant:** Increment `CURRENT_SCHEMA_VERSION` in `crates/doc/src/error.rs` (used by `crates/format/src/archive.rs`), or the target format's `CURRENT_PREFERENCES_VERSION` in `crates/format/src/versioned.rs` / `CURRENT_JOURNAL_VERSION` / `CURRENT_LOCK_VERSION` in `crates/format/src/recovery.rs`.
2. **Add a `VersionStep`:** Implement a pure `from_version -> from_version + 1` migration step transforming the serialized JSON payload (updating `"schema_version"`/`"version"` alongside migrated fields) and append it sequentially to `MIGRATION_STEPS` in `crates/format/src/archive.rs` or `PREFERENCES_MIGRATION_STEPS` in `crates/format/src/versioned.rs` (a `const` assert ties each slice's length to its version constant). Never skip step numbers or mutate files in place during migration tests before validation succeeds.
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
