# Persisted File Formats & Versioning

LumenCraft versions every persisted artefact so older projects migrate forward deterministically and newer projects are never silently damaged by an older binary. Machine-readable schemas generated from source live under `docs/schemas/*.schema.json` (`archive-manifest.schema.json`, `document.schema.json`, `batch-v1.schema.json`, `preferences.schema.json`, `journal-record.schema.json` via `cargo xtask schemas`).

---

## 1. Persisted Format Inventory

| File / Directory | Contents | Owner Crate(s) | Current Version | Notes |
| :--- | :--- | :--- | :--- | :--- |
| **`.lumencraft`** | Compressed archive (`manifest.json`, `document.json`, `sheets/*.csv`) | `lumencraft-doc`, `lumencraft-format` (`crates/format/src/archive.rs`) | `CURRENT_SCHEMA_VERSION = 1` (`crates/doc/src/error.rs`; `FORMAT_MAGIC = "lumencraft-archive"`) | CRC32-checked entries; saved via atomic write (`lumencraft-platform` / temp + rename). |
| **`.lumencraft.d/`** | Unpacked project directory (`manifest.json`, `document.json`, `sheets/*.csv`) | `lumencraft-doc`, `lumencraft-format` (`crates/format/src/archive.rs`) | `CURRENT_SCHEMA_VERSION = 1` | Version-control friendly workspace layout with the same manifest/document versioning. |
| **`.lctemplate`** | Reusable project template ZIP (`ArchiveKind::Template`) | `lumencraft-doc`, `lumencraft-format` (`crates/format/src/archive.rs`) | `CURRENT_SCHEMA_VERSION = 1` | Data cell values and cached results stripped; formulas, analyses, graphs, layouts, and info constants preserved. |
| **`preferences.json`** *(and `shortcuts.json`)* | Desktop UI settings (`PreferencesFile`: locale, theme, custom shortcut bindings, preserved unknown fields) | `lumencraft` (`apps/lumencraft/src/main.rs`), `lumencraft-platform` | `CURRENT_PREFERENCES_VERSION = 1` (`crates/format/src/versioned.rs`) | Atomic writes only; must never overwrite a newer version file written by a newer release (`docs/schemas/preferences.schema.json`). |
| **`ops.journal` & `lumencraft.lock`** | Crash-recovery NDJSON command WAL (`{ format_version, seq, crc32, command_json }`) + `JournalLockLease` JSON (`{ format_version, owner_id, acquired_epoch_ms, ttl_ms }`) *(with optional `snapshot.lumencraft`)* | `lumencraft-format` (`crates/format/src/recovery.rs`) | `CURRENT_JOURNAL_VERSION = 1`, `CURRENT_LOCK_VERSION = 1` | `ops.journal` replays serialized engine commands through `Session::execute` (see [docs/command-ids.md](command-ids.md)); schemas in `docs/schemas/journal-record.schema.json`. |
| **`lumencraft.batch.v1`** | Headless CLI batch execution manifest (`{ schema, template \| project, inputs, commands, exports }`) | `lumencraft-cli` (`apps/lumencraft-cli/src/cmd_run_batch.rs`) | `BATCH_SCHEMA_V1 = "lumencraft.batch.v1"` | Exact `"schema"` tag verified before execution (`docs/schemas/batch-v1.schema.json`); breaking contract changes increment schema suffix (`v2`). |

*(External interchange importers/exporters such as `.pzfx`, `.prism`, `.csv`, `.tsv`, `.svg`, `.pdf`, `.png`, `.tiff`, and `.emf` are read/exported defensively with bounds checks and clean-room parsers, and do not own LumenCraft's native schema history.)*

---

## 2. Forward & Backward Compatibility Policy

1. **Backward compatibility (older files in newer LumenCraft):**
   - Files tagged with `0 <= version <= CURRENT` are upgraded sequentially in memory (`v0 -> v1 -> ... -> CURRENT`) through registered migration steps before deserializing typed Rust models.
   - Defaults for newly added optional fields are applied explicitly inside the migration step or versioned loader—never silently fabricated during error recovery.
2. **Forward safety (newer files in older LumenCraft):**
   - Reading a manifest or settings wrapper whose `version` exceeds what the running binary understands **must fail cleanly** with a typed error (`FormatError::UnsupportedSchemaVersion { version, max_supported }`) telling the user a newer version of LumenCraft created the file.
   - **Never overwrite a newer version:** Neither project save paths nor desktop settings/preferences savers may overwrite an on-disk file whose persisted version is higher than `max_supported`, preventing silent loss of fields added in newer releases.
3. **Atomic persistence:**
   - All project, template, settings, and manifest saves write through a temporary sibling file -> flush/sync -> atomic rename (`lumencraft-platform`), so crashes never truncate existing files ([docs/pitfalls.md](pitfalls.md)).

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
