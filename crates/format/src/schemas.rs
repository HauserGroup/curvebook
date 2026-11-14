//! Draft 2020-12 JSON Schemas for all persisted LumenCraft file formats (`B5`).

use lumencraft_doc::schema::{
    DRAFT_2020_12_SCHEMA_URI, JsonSchema, SchemaNode, array_schema, bool_schema, const_string_schema, enum_schema, integer_schema, map_schema,
    nullable_schema, number_schema, object_schema, one_of_schema, project_document_schema, ref_schema, string_schema,
};
use lumencraft_doc::{CURRENT_SCHEMA_VERSION, MAX_TITLE_BYTES, SheetId};

use crate::recovery::{CURRENT_JOURNAL_VERSION, CURRENT_LOCK_VERSION};
use crate::versioned::{BATCH_SCHEMA_V1, CURRENT_PREFERENCES_VERSION};

const ARCHIVE_MANIFEST_SCHEMA_ID: &str = "https://lumencraft.dev/schemas/archive-manifest.schema.json";
const BATCH_V1_SCHEMA_ID: &str = "https://lumencraft.dev/schemas/batch-v1.schema.json";
const PREFERENCES_SCHEMA_ID: &str = "https://lumencraft.dev/schemas/preferences.schema.json";
const JOURNAL_RECORD_SCHEMA_ID: &str = "https://lumencraft.dev/schemas/journal-record.schema.json";

fn to_json_value(node: &SchemaNode) -> serde_json::Value {
    serde_json::to_value(node).unwrap_or_else(|_| serde_json::Value::Object(serde_json::Map::new()))
}

/// Draft 2020-12 JSON Schema for native archive `manifest.json` (`ArchiveManifest`).
pub fn archive_manifest_schema() -> serde_json::Value {
    let sheet_entry = object_schema(
        vec![
            ("id", ref_schema(SheetId::schema_name())),
            ("section", ref_schema("SheetSection")),
            ("title", string_schema().with_max_length(MAX_TITLE_BYTES as u64)),
            ("csv_path", nullable_schema(string_schema())),
            ("crc32", nullable_schema(integer_schema().with_minimum_i64(0).with_maximum_i64(i64::from(u32::MAX)))),
        ],
        &["id", "section", "title", "csv_path", "crc32"],
        false,
    )
    .with_title("SheetManifestEntry");

    let node = object_schema(
        vec![
            ("format", const_string_schema("lumencraft-archive")),
            ("schema_version", integer_schema().with_minimum_i64(0).with_maximum_i64(i64::from(CURRENT_SCHEMA_VERSION))),
            ("kind", enum_schema(&["Project", "Template"])),
            ("title", string_schema().with_max_length(MAX_TITLE_BYTES as u64)),
            ("document_crc32", integer_schema().with_minimum_i64(0).with_maximum_i64(i64::from(u32::MAX))),
            ("sheets", array_schema(ref_schema("SheetManifestEntry"))),
        ],
        &["format", "schema_version", "kind", "title", "document_crc32", "sheets"],
        false,
    )
    .with_schema_uri(DRAFT_2020_12_SCHEMA_URI)
    .with_id(ARCHIVE_MANIFEST_SCHEMA_ID)
    .with_title("ArchiveManifest")
    .with_description("Draft 2020-12 schema for LumenCraft .lumencraft / .lctemplate archive manifest.json files.")
    .with_def(SheetId::schema_name(), SheetId::json_schema())
    .with_def("SheetSection", enum_schema(&["Data", "Info", "Analyses", "Graphs", "Layouts"]))
    .with_def("SheetManifestEntry", sheet_entry);

    to_json_value(&node)
}

/// Draft 2020-12 JSON Schema for persisted `document.json` (`ProjectDocument`).
pub fn document_schema() -> serde_json::Value {
    to_json_value(&project_document_schema())
}

/// Draft 2020-12 JSON Schema for headless batch runner manifests (`lumencraft.batch.v1`).
pub fn batch_v1_schema() -> serde_json::Value {
    let batch_input = object_schema(
        vec![
            ("csv", string_schema().with_min_length(1)),
            ("tsv", string_schema().with_min_length(1)),
            ("path", string_schema().with_min_length(1)),
            ("file", string_schema().with_min_length(1)),
            ("sheet", string_schema().with_min_length(1)),
            ("kind", string_schema().with_min_length(1)),
            ("subcolumn_mode", string_schema().with_min_length(1)),
            ("delimiter", string_schema().with_min_length(1).with_max_length(1)),
            ("decimal_separator", string_schema().with_min_length(1).with_max_length(1)),
        ],
        &[],
        false,
    )
    .with_title("BatchInput");

    let batch_command = object_schema(
        vec![
            ("cmd", string_schema().with_min_length(1)),
            ("command", string_schema().with_min_length(1)),
            ("method", string_schema().with_min_length(1)),
            ("id", SchemaNode::default()),
            ("params", SchemaNode::default()),
        ],
        &[],
        false,
    )
    .with_title("BatchCommand");

    let batch_export = object_schema(
        vec![
            ("out", string_schema().with_min_length(1)),
            ("path", string_schema().with_min_length(1)),
            ("format", enum_schema(&["lumencraft", "lumencraft.d", "lctemplate", "csv", "tsv", "json", "svg", "pdf", "png", "tiff", "emf"])),
            ("sheet", string_schema().with_min_length(1)),
            ("dpi", number_schema().with_minimum_f64(72.0).with_maximum_f64(1200.0)),
            ("pretty", bool_schema()),
            ("tab_index", integer_schema().with_minimum_i64(0)),
            ("pdf_version", string_schema()),
            ("ttf_font", string_schema().with_min_length(1)),
        ],
        &[],
        false,
    )
    .with_title("BatchExport");

    let node = object_schema(
        vec![
            ("schema", const_string_schema(BATCH_SCHEMA_V1)),
            ("template", string_schema().with_min_length(1)),
            ("project", string_schema().with_min_length(1)),
            ("inputs", array_schema(ref_schema("BatchInput"))),
            ("commands", array_schema(ref_schema("BatchCommand"))),
            ("exports", array_schema(ref_schema("BatchExport"))),
        ],
        &["schema"],
        false,
    )
    .with_schema_uri(DRAFT_2020_12_SCHEMA_URI)
    .with_id(BATCH_V1_SCHEMA_ID)
    .with_title("BatchManifestV1")
    .with_description("Draft 2020-12 schema for lumencraft.batch.v1 headless batch manifests.")
    .with_def("BatchInput", batch_input)
    .with_def("BatchCommand", batch_command)
    .with_def("BatchExport", batch_export);

    let mut value = to_json_value(&node);
    if let Some(obj) = value.as_object_mut() {
        obj.insert(String::from("not"), serde_json::json!({ "required": ["template", "project"] }));
    }
    value
}

/// Draft 2020-12 JSON Schema for persisted user preferences (`preferences.json`).
pub fn preferences_schema() -> serde_json::Value {
    let node = object_schema(
        vec![
            ("version", integer_schema().with_minimum_i64(1).with_maximum_i64(i64::from(CURRENT_PREFERENCES_VERSION))),
            ("locale", nullable_schema(string_schema())),
            ("theme", nullable_schema(string_schema())),
            ("shortcuts", map_schema(string_schema())),
        ],
        &["version"],
        true,
    )
    .with_schema_uri(DRAFT_2020_12_SCHEMA_URI)
    .with_id(PREFERENCES_SCHEMA_ID)
    .with_title("PreferencesFile")
    .with_description("Draft 2020-12 schema for versioned user preferences files with forward-compatible extension fields.");

    to_json_value(&node)
}

/// Draft 2020-12 JSON Schema covering both [`JournalOpRecord`](crate::recovery::JournalOpRecord)
/// (`ops.journal` lines) and [`JournalLockLease`](crate::recovery::JournalLockLease) (`lumencraft.lock`).
pub fn journal_record_schema() -> serde_json::Value {
    let op_record = object_schema(
        vec![
            ("format_version", integer_schema().with_minimum_i64(1).with_maximum_i64(i64::from(CURRENT_JOURNAL_VERSION))),
            ("seq", integer_schema().with_minimum_i64(1)),
            ("crc32", integer_schema().with_minimum_i64(0).with_maximum_i64(i64::from(u32::MAX))),
            ("command_json", SchemaNode::default()),
        ],
        &["format_version", "seq", "crc32", "command_json"],
        false,
    )
    .with_title("JournalOpRecord");

    let lock_lease = object_schema(
        vec![
            ("format_version", integer_schema().with_minimum_i64(1).with_maximum_i64(i64::from(CURRENT_LOCK_VERSION))),
            ("owner_id", string_schema().with_min_length(1)),
            ("acquired_epoch_ms", integer_schema().with_minimum_i64(0)),
            ("ttl_ms", integer_schema().with_minimum_i64(0)),
        ],
        &["format_version", "owner_id", "acquired_epoch_ms", "ttl_ms"],
        false,
    )
    .with_title("JournalLockLease");

    let node = one_of_schema(vec![ref_schema("JournalOpRecord"), ref_schema("JournalLockLease")])
        .with_schema_uri(DRAFT_2020_12_SCHEMA_URI)
        .with_id(JOURNAL_RECORD_SCHEMA_ID)
        .with_title("JournalPersistenceRecord")
        .with_description("Draft 2020-12 schema for crash-recovery ops.journal NDJSON lines and lumencraft.lock leases.")
        .with_def("JournalOpRecord", op_record)
        .with_def("JournalLockLease", lock_lease);

    to_json_value(&node)
}

/// Return all five persisted file format JSON Schemas paired with their canonical filenames.
pub fn all_persisted_schemas() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("archive-manifest.schema.json", archive_manifest_schema()),
        ("document.schema.json", document_schema()),
        ("batch-v1.schema.json", batch_v1_schema()),
        ("preferences.schema.json", preferences_schema()),
        ("journal-record.schema.json", journal_record_schema()),
    ]
}
