//! Draft 2020-12 JSON Schemas for all persisted LumenCraft file formats (`B5`).

use lumencraft_doc::schema::{
    DRAFT_2020_12_SCHEMA_URI, JsonSchema, SchemaNode, SchemaType, SchemaTypeSpec, array_schema, bool_schema, const_string_schema, enum_schema,
    integer_schema, map_schema, nullable_schema, object_schema, one_of_schema, project_document_schema, ref_schema, string_schema,
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

fn nullable_object_schema() -> SchemaNode {
    SchemaNode { schema_type: Some(SchemaTypeSpec::Multiple(vec![SchemaType::Object, SchemaType::Null])), ..SchemaNode::default() }
}

fn insert_non_null_string_any_of(def_node: &mut serde_json::Value, keys: &[&str], require_non_whitespace: bool) {
    let Some(obj) = def_node.as_object_mut() else {
        return;
    };
    let branches: Vec<serde_json::Value> = keys
        .iter()
        .map(|key| {
            let prop_rule = if require_non_whitespace {
                serde_json::json!({ "type": "string", "minLength": 1, "pattern": "\\S" })
            } else {
                serde_json::json!({ "type": "string", "minLength": 1 })
            };
            serde_json::json!({
                "required": [key],
                "properties": { (*key): prop_rule }
            })
        })
        .collect();
    obj.insert(String::from("anyOf"), serde_json::Value::Array(branches));
}

fn set_nullable_enum_property(def_node: &mut serde_json::Value, prop: &str, values: &[&str]) {
    let Some(prop_obj) = def_node
        .as_object_mut()
        .and_then(|o| o.get_mut("properties"))
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|p| p.get_mut(prop))
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    prop_obj.insert(String::from("type"), serde_json::json!(["string", "null"]));
    let mut enum_arr: Vec<serde_json::Value> = values.iter().map(|s| serde_json::Value::String((*s).to_owned())).collect();
    enum_arr.push(serde_json::Value::Null);
    prop_obj.insert(String::from("enum"), serde_json::Value::Array(enum_arr));
}

fn set_property_pattern(def_node: &mut serde_json::Value, props: &[&str], pattern: &str) {
    let Some(prop_map) = def_node.as_object_mut().and_then(|o| o.get_mut("properties")).and_then(serde_json::Value::as_object_mut) else {
        return;
    };
    for prop in props {
        if let Some(prop_obj) = prop_map.get_mut(*prop).and_then(serde_json::Value::as_object_mut) {
            prop_obj.insert(String::from("pattern"), serde_json::Value::String(pattern.to_owned()));
        }
    }
}

fn set_u64_max(node: &mut serde_json::Value, def_name: &str, field_name: &str) {
    if let Some(prop_obj) = node
        .as_object_mut()
        .and_then(|o| o.get_mut("$defs"))
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|d| d.get_mut(def_name))
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|def| def.get_mut("properties"))
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|p| p.get_mut(field_name))
        .and_then(serde_json::Value::as_object_mut)
    {
        prop_obj.insert(String::from("maximum"), serde_json::Value::from(u64::MAX));
    }
}

/// Draft 2020-12 JSON Schema for headless batch runner manifests (`lumencraft.batch.v1`).
pub fn batch_v1_schema() -> serde_json::Value {
    let batch_input = object_schema(
        vec![
            ("csv", nullable_schema(string_schema().with_min_length(1))),
            ("tsv", nullable_schema(string_schema().with_min_length(1))),
            ("path", nullable_schema(string_schema().with_min_length(1))),
            ("file", nullable_schema(string_schema().with_min_length(1))),
            ("sheet", nullable_schema(string_schema().with_min_length(1))),
            ("sheet_id", nullable_schema(string_schema().with_min_length(1))),
            ("sheetId", nullable_schema(string_schema().with_min_length(1))),
            ("kind", nullable_schema(string_schema().with_min_length(1))),
            ("table_kind", nullable_schema(string_schema().with_min_length(1))),
            ("tableKind", nullable_schema(string_schema().with_min_length(1))),
            ("subcolumn_mode", nullable_schema(string_schema().with_min_length(1))),
            ("subcolumnMode", nullable_schema(string_schema().with_min_length(1))),
            ("delimiter", nullable_schema(string_schema().with_min_length(1))),
            ("decimal_separator", nullable_schema(string_schema().with_min_length(1))),
            ("decimalSeparator", nullable_schema(string_schema().with_min_length(1))),
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
            ("params", nullable_object_schema()),
        ],
        &[],
        false,
    )
    .with_title("BatchCommand");

    let batch_export = object_schema(
        vec![
            ("out", nullable_schema(string_schema().with_min_length(1))),
            ("path", nullable_schema(string_schema().with_min_length(1))),
            (
                "format",
                enum_schema(&[
                    "lumencraft",
                    "lumencraft.d",
                    "lumencraft-dir",
                    "unpacked",
                    "dir",
                    "csv",
                    "tsv",
                    "json",
                    "svg",
                    "pdf",
                    "png",
                    "tiff",
                    "tif",
                    "emf",
                ]),
            ),
            ("sheet", nullable_schema(string_schema().with_min_length(1))),
            ("sheet_id", nullable_schema(string_schema().with_min_length(1))),
            ("sheetId", nullable_schema(string_schema().with_min_length(1))),
            ("dpi", nullable_schema(integer_schema().with_minimum_i64(1).with_maximum_i64(i64::from(u16::MAX)))),
            ("pretty", nullable_schema(bool_schema())),
            ("tab_index", nullable_schema(integer_schema().with_minimum_i64(0))),
            ("tabIndex", nullable_schema(integer_schema().with_minimum_i64(0))),
            ("pdf_version", string_schema()),
            ("pdfVersion", string_schema()),
            ("ttf_font", nullable_schema(string_schema().with_min_length(1))),
            ("ttfFont", nullable_schema(string_schema().with_min_length(1))),
        ],
        &[],
        false,
    )
    .with_title("BatchExport");

    let node = object_schema(
        vec![
            ("schema", const_string_schema(BATCH_SCHEMA_V1)),
            ("template", nullable_schema(string_schema().with_min_length(1))),
            ("project", nullable_schema(string_schema().with_min_length(1))),
            ("inputs", nullable_schema(array_schema(ref_schema("BatchInput")))),
            ("commands", nullable_schema(array_schema(ref_schema("BatchCommand")))),
            ("exports", nullable_schema(array_schema(ref_schema("BatchExport")))),
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
        obj.insert(
            String::from("not"),
            serde_json::json!({
                "required": ["template", "project"],
                "properties": {
                    "template": { "type": "string" },
                    "project": { "type": "string" }
                }
            }),
        );
    }
    if let Some(defs) = value.get_mut("$defs").and_then(serde_json::Value::as_object_mut) {
        if let Some(input_def) = defs.get_mut("BatchInput") {
            insert_non_null_string_any_of(input_def, &["csv", "tsv", "path", "file"], false);
        }
        if let Some(cmd_def) = defs.get_mut("BatchCommand") {
            set_property_pattern(cmd_def, &["cmd", "command", "method"], r"\S");
            insert_non_null_string_any_of(cmd_def, &["cmd", "command", "method"], true);
        }
        if let Some(exp_def) = defs.get_mut("BatchExport") {
            insert_non_null_string_any_of(exp_def, &["out", "path"], false);
            set_nullable_enum_property(
                exp_def,
                "format",
                &["lumencraft", "lumencraft.d", "lumencraft-dir", "unpacked", "dir", "csv", "tsv", "json", "svg", "pdf", "png", "tiff", "tif", "emf"],
            );
            set_nullable_enum_property(exp_def, "pdf_version", &["1.4", "1.7"]);
            set_nullable_enum_property(exp_def, "pdfVersion", &["1.4", "1.7"]);
        }
    }
    value
}

/// Draft 2020-12 JSON Schema for persisted user preferences (`preferences.json`).
pub fn preferences_schema() -> serde_json::Value {
    let node = object_schema(
        vec![
            ("version", integer_schema().with_minimum_i64(0).with_maximum_i64(i64::from(CURRENT_PREFERENCES_VERSION))),
            ("locale", nullable_schema(string_schema())),
            ("theme", nullable_schema(string_schema())),
            ("shortcuts", nullable_schema(map_schema(string_schema()))),
        ],
        &[],
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
        &["seq", "crc32", "command_json"],
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
        &["owner_id", "acquired_epoch_ms", "ttl_ms"],
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

    let mut value = to_json_value(&node);
    set_u64_max(&mut value, "JournalOpRecord", "seq");
    set_u64_max(&mut value, "JournalLockLease", "acquired_epoch_ms");
    set_u64_max(&mut value, "JournalLockLease", "ttl_ms");
    value
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
