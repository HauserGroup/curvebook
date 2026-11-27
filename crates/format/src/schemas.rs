//! Draft 2020-12 JSON Schemas for all persisted CurveBook file formats (`B5`).

use curvebook_doc::schema::{
    DRAFT_2020_12_SCHEMA_URI, JsonSchema, SchemaNode, SchemaType, SchemaTypeSpec, array_schema, bool_schema, const_string_schema, enum_schema,
    integer_schema, map_schema, nullable_schema, number_schema, object_schema, one_of_schema, project_document_schema, ref_schema, string_schema,
};
use curvebook_doc::{CURRENT_SCHEMA_VERSION, MAX_IMAGE_ASSET_BYTES, MAX_PLATE_WELLS, MAX_TITLE_BYTES, SheetId};

use crate::plate::CURRENT_PLATE_MAP_VERSION;
use crate::recovery::{CURRENT_JOURNAL_VERSION, CURRENT_LOCK_VERSION};
use crate::versioned::{BATCH_SCHEMA_V1, CURRENT_PREFERENCES_VERSION};

const ARCHIVE_MANIFEST_SCHEMA_ID: &str = "https://github.com/HauserGroup/curvebook/blob/main/docs/schemas/archive-manifest.schema.json";
const BATCH_V1_SCHEMA_ID: &str = "https://github.com/HauserGroup/curvebook/blob/main/docs/schemas/batch-v1.schema.json";
const PREFERENCES_SCHEMA_ID: &str = "https://github.com/HauserGroup/curvebook/blob/main/docs/schemas/preferences.schema.json";
const JOURNAL_RECORD_SCHEMA_ID: &str = "https://github.com/HauserGroup/curvebook/blob/main/docs/schemas/journal-record.schema.json";
const PLATE_MAP_SCHEMA_ID: &str = "https://github.com/HauserGroup/curvebook/blob/main/docs/schemas/plate-map.schema.json";

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

    let asset_entry = object_schema(
        vec![
            ("asset_id", string_schema().with_pattern("^img-[0-9a-f]{16}$")),
            ("media_type", ref_schema("ImageMimeKind")),
            ("path", string_schema()),
            ("byte_len", integer_schema().with_minimum_i64(0).with_maximum_i64(MAX_IMAGE_ASSET_BYTES as i64)),
            ("crc32", integer_schema().with_minimum_i64(0).with_maximum_i64(i64::from(u32::MAX))),
        ],
        &["asset_id", "media_type", "path", "byte_len", "crc32"],
        false,
    )
    .with_title("AssetManifestEntry");

    let node = object_schema(
        vec![
            ("format", const_string_schema("curvebook-archive")),
            ("schema_version", integer_schema().with_minimum_i64(0).with_maximum_i64(i64::from(CURRENT_SCHEMA_VERSION))),
            ("kind", enum_schema(&["Project", "Template"])),
            ("title", string_schema().with_max_length(MAX_TITLE_BYTES as u64)),
            ("document_crc32", integer_schema().with_minimum_i64(0).with_maximum_i64(i64::from(u32::MAX))),
            ("sheets", array_schema(ref_schema("SheetManifestEntry"))),
            ("assets", array_schema(ref_schema("AssetManifestEntry"))),
        ],
        &["format", "schema_version", "kind", "title", "document_crc32", "sheets"],
        false,
    )
    .with_schema_uri(DRAFT_2020_12_SCHEMA_URI)
    .with_id(ARCHIVE_MANIFEST_SCHEMA_ID)
    .with_title("ArchiveManifest")
    .with_description("Draft 2020-12 schema for CurveBook .curvebook / .cbtemplate archive manifest.json files.")
    .with_def(SheetId::schema_name(), SheetId::json_schema())
    .with_def("SheetSection", enum_schema(&["Data", "Info", "Analyses", "Graphs", "Layouts"]))
    .with_def("SheetManifestEntry", sheet_entry)
    .with_def("ImageMimeKind", enum_schema(&["Png", "Jpeg", "Tiff"]))
    .with_def("AssetManifestEntry", asset_entry);

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

fn mark_deprecated_aliases(def_node: &mut serde_json::Value, aliases: &[(&str, &[&str])]) {
    let Some(obj) = def_node.as_object_mut() else {
        return;
    };
    let mut alias_map = serde_json::Map::new();
    for &(canonical, list) in aliases {
        let items: Vec<serde_json::Value> = list.iter().map(|a| serde_json::Value::String((*a).to_owned())).collect();
        alias_map.insert(canonical.to_owned(), serde_json::Value::Array(items));
        if let Some(props) = obj.get_mut("properties").and_then(serde_json::Value::as_object_mut) {
            for &alias in list {
                if let Some(prop_obj) = props.get_mut(alias).and_then(serde_json::Value::as_object_mut) {
                    prop_obj.insert(String::from("deprecated"), serde_json::Value::Bool(true));
                }
            }
        }
    }
    if !alias_map.is_empty() {
        obj.insert(String::from("x-deprecated-aliases"), serde_json::Value::Object(alias_map));
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

/// Allowed field names at the root of a `curvebook.batch.v1` manifest.
pub const BATCH_MANIFEST_ROOT_KEYS: &[&str] = &["schema", "template", "project", "inputs", "commands", "exports"];

/// Allowed field names on each entry of `inputs` in a `curvebook.batch.v1` manifest.
pub const BATCH_MANIFEST_INPUT_KEYS: &[&str] = &[
    "csv",
    "tsv",
    "path",
    "file",
    "sheet",
    "sheet_id",
    "sheetId",
    "kind",
    "table_kind",
    "tableKind",
    "subcolumn_mode",
    "subcolumnMode",
    "delimiter",
    "decimal_separator",
    "decimalSeparator",
];

/// Allowed field names on each entry of `commands` in a `curvebook.batch.v1` manifest.
pub const BATCH_MANIFEST_COMMAND_KEYS: &[&str] = &["cmd", "command", "method", "id", "params"];

/// Allowed field names on each entry of `exports` in a `curvebook.batch.v1` manifest.
pub const BATCH_MANIFEST_EXPORT_KEYS: &[&str] = &[
    "out",
    "path",
    "format",
    "sheet",
    "sheet_id",
    "sheetId",
    "dpi",
    "pretty",
    "experimental",
    "tab_index",
    "tabIndex",
    "pdf_version",
    "pdfVersion",
    "ttf_font",
    "ttfFont",
];

const BATCH_EXPORT_FORMATS: &[&str] =
    &["curvebook", "curvebook.d", "curvebook-dir", "unpacked", "dir", "pzfx", "csv", "tsv", "json", "svg", "pdf", "png", "tiff", "tif", "emf"];

fn batch_command_property_schema(key: &str) -> Option<SchemaNode> {
    match key {
        "cmd" | "command" | "method" => Some(string_schema().with_min_length(1)),
        "id" => Some(SchemaNode::default()),
        "params" => Some(nullable_object_schema()),
        _ => None,
    }
}

fn batch_export_property_schema(key: &str) -> Option<SchemaNode> {
    match key {
        "out" | "path" | "sheet" | "sheet_id" | "sheetId" | "ttf_font" | "ttfFont" => Some(nullable_schema(string_schema().with_min_length(1))),
        "format" => Some(enum_schema(BATCH_EXPORT_FORMATS)),
        "dpi" => Some(nullable_schema(integer_schema().with_minimum_i64(1).with_maximum_i64(i64::from(u16::MAX)))),
        "pretty" | "experimental" => Some(nullable_schema(bool_schema())),
        "tab_index" | "tabIndex" => Some(nullable_schema(integer_schema().with_minimum_i64(0))),
        "pdf_version" | "pdfVersion" => Some(string_schema()),
        _ => None,
    }
}

fn batch_root_property_schema(key: &str) -> Option<SchemaNode> {
    match key {
        "schema" => Some(const_string_schema(BATCH_SCHEMA_V1)),
        "template" | "project" => Some(nullable_schema(string_schema().with_min_length(1))),
        "inputs" => Some(nullable_schema(array_schema(ref_schema("BatchInput")))),
        "commands" => Some(nullable_schema(array_schema(ref_schema("BatchCommand")))),
        "exports" => Some(nullable_schema(array_schema(ref_schema("BatchExport")))),
        _ => None,
    }
}

/// Draft 2020-12 JSON Schema for headless batch runner manifests (`curvebook.batch.v1`).
pub fn batch_v1_schema() -> serde_json::Value {
    let input_props: Vec<(&str, SchemaNode)> =
        BATCH_MANIFEST_INPUT_KEYS.iter().map(|&key| (key, nullable_schema(string_schema().with_min_length(1)))).collect();
    let batch_input = object_schema(input_props, &[], false).with_title("BatchInput");

    let command_props: Vec<(&str, SchemaNode)> =
        BATCH_MANIFEST_COMMAND_KEYS.iter().filter_map(|&key| batch_command_property_schema(key).map(|schema| (key, schema))).collect();
    let batch_command = object_schema(command_props, &[], false).with_title("BatchCommand");

    let export_props: Vec<(&str, SchemaNode)> =
        BATCH_MANIFEST_EXPORT_KEYS.iter().filter_map(|&key| batch_export_property_schema(key).map(|schema| (key, schema))).collect();
    let batch_export = object_schema(export_props, &[], false).with_title("BatchExport");

    let root_props: Vec<(&str, SchemaNode)> =
        BATCH_MANIFEST_ROOT_KEYS.iter().filter_map(|&key| batch_root_property_schema(key).map(|schema| (key, schema))).collect();
    let node = object_schema(root_props, &["schema"], false)
        .with_schema_uri(DRAFT_2020_12_SCHEMA_URI)
        .with_id(BATCH_V1_SCHEMA_ID)
        .with_title("BatchManifestV1")
        .with_description("Draft 2020-12 schema for curvebook.batch.v1 headless batch manifests.")
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
            mark_deprecated_aliases(
                input_def,
                &[
                    ("sheet", &["sheet_id", "sheetId"]),
                    ("kind", &["table_kind", "tableKind"]),
                    ("subcolumn_mode", &["subcolumnMode"]),
                    ("decimal_separator", &["decimalSeparator"]),
                ],
            );
        }
        if let Some(cmd_def) = defs.get_mut("BatchCommand") {
            set_property_pattern(cmd_def, &["cmd", "command", "method"], r"\S");
            insert_non_null_string_any_of(cmd_def, &["cmd", "command", "method"], true);
            mark_deprecated_aliases(cmd_def, &[("cmd", &["command", "method"])]);
        }
        if let Some(exp_def) = defs.get_mut("BatchExport") {
            insert_non_null_string_any_of(exp_def, &["out", "path"], false);
            set_nullable_enum_property(exp_def, "format", BATCH_EXPORT_FORMATS);
            set_nullable_enum_property(exp_def, "pdf_version", &["1.4", "1.7"]);
            set_nullable_enum_property(exp_def, "pdfVersion", &["1.4", "1.7"]);
            mark_deprecated_aliases(
                exp_def,
                &[("sheet", &["sheet_id", "sheetId"]), ("tab_index", &["tabIndex"]), ("pdf_version", &["pdfVersion"]), ("ttf_font", &["ttfFont"])],
            );
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
/// (`ops.journal` lines) and [`JournalLockLease`](crate::recovery::JournalLockLease) (`curvebook.lock`).
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
            ("host_id", nullable_schema(string_schema().with_min_length(1))),
        ],
        &["owner_id", "acquired_epoch_ms", "ttl_ms"],
        false,
    )
    .with_title("JournalLockLease");

    let node = one_of_schema(vec![ref_schema("JournalOpRecord"), ref_schema("JournalLockLease")])
        .with_schema_uri(DRAFT_2020_12_SCHEMA_URI)
        .with_id(JOURNAL_RECORD_SCHEMA_ID)
        .with_title("JournalPersistenceRecord")
        .with_description("Draft 2020-12 schema for crash-recovery ops.journal NDJSON lines and curvebook.lock leases.")
        .with_def("JournalOpRecord", op_record)
        .with_def("JournalLockLease", lock_lease);

    let mut value = to_json_value(&node);
    set_u64_max(&mut value, "JournalOpRecord", "seq");
    set_u64_max(&mut value, "JournalLockLease", "acquired_epoch_ms");
    set_u64_max(&mut value, "JournalLockLease", "ttl_ms");
    value
}

/// Draft 2020-12 JSON Schema for versioned `.cbplate` microplate layout files (`PlateMapFile`).
pub fn plate_map_schema() -> serde_json::Value {
    let treatment_payload = object_schema(
        vec![
            ("group_name", string_schema().with_min_length(1).with_max_length(MAX_TITLE_BYTES as u64)),
            ("dilution_x", number_schema()),
            ("replicate", integer_schema().with_minimum_i64(0)),
        ],
        &["group_name", "dilution_x", "replicate"],
        false,
    );
    let well_role = one_of_schema(vec![
        const_string_schema("Blank"),
        const_string_schema("PositiveControl"),
        const_string_schema("Excluded"),
        object_schema(vec![("Treatment", treatment_payload)], &["Treatment"], false),
    ])
    .with_title("PlateWellRole");

    let node = object_schema(
        vec![
            ("format_version", integer_schema().with_minimum_i64(1).with_maximum_i64(i64::from(CURRENT_PLATE_MAP_VERSION))),
            ("name", string_schema().with_min_length(1).with_max_length(MAX_TITLE_BYTES as u64)),
            ("geometry", enum_schema(&["Wells96", "Wells384"])),
            ("x_title", string_schema().with_max_length(MAX_TITLE_BYTES as u64)),
            ("wells", SchemaNode { min_items: Some(96), max_items: Some(MAX_PLATE_WELLS as u64), ..array_schema(ref_schema("PlateWellRole")) }),
        ],
        &["format_version", "name", "geometry", "x_title", "wells"],
        false,
    )
    .with_schema_uri(DRAFT_2020_12_SCHEMA_URI)
    .with_id(PLATE_MAP_SCHEMA_ID)
    .with_title("PlateMapFile")
    .with_description("Draft 2020-12 schema for versioned CurveBook .cbplate microplate layout files.")
    .with_def("PlateWellRole", well_role);

    to_json_value(&node)
}

/// Return all persisted file format JSON Schemas paired with their canonical filenames.
pub fn all_persisted_schemas() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("archive-manifest.schema.json", archive_manifest_schema()),
        ("document.schema.json", document_schema()),
        ("batch-v1.schema.json", batch_v1_schema()),
        ("preferences.schema.json", preferences_schema()),
        ("journal-record.schema.json", journal_record_schema()),
        ("plate-map.schema.json", plate_map_schema()),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{BATCH_MANIFEST_COMMAND_KEYS, BATCH_MANIFEST_EXPORT_KEYS, BATCH_MANIFEST_INPUT_KEYS, BATCH_MANIFEST_ROOT_KEYS, batch_v1_schema};

    #[test]
    fn batch_manifest_key_inventories_match_schema_properties() {
        let schema = batch_v1_schema();
        let root_keys: BTreeSet<&str> = schema["properties"].as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(root_keys, BATCH_MANIFEST_ROOT_KEYS.iter().copied().collect());

        let defs = schema["$defs"].as_object().unwrap();
        for (def_name, expected) in
            [("BatchInput", BATCH_MANIFEST_INPUT_KEYS), ("BatchCommand", BATCH_MANIFEST_COMMAND_KEYS), ("BatchExport", BATCH_MANIFEST_EXPORT_KEYS)]
        {
            let actual: BTreeSet<&str> = defs[def_name]["properties"].as_object().unwrap().keys().map(String::as_str).collect();
            assert_eq!(actual, expected.iter().copied().collect(), "mismatch in {def_name} properties");
        }
    }
}
