//! Pure-Rust JSON Schema (Draft 2020-12) model and helper constructors (`C2`).
//!
//! `crates/doc` is an L1 model crate without a direct `serde_json` runtime dependency, so schemas
//! are constructed as strongly typed, deterministically ordered [`SchemaNode`] values that derive
//! `serde::Serialize` / `serde::Deserialize` and convert trivially via `serde_json::to_value` in
//! downstream crates (`curvebook-format`, `curvebook-engine`).

use std::collections::BTreeMap;

use crate::TableKind;
use crate::analysis::PowerAnalysisSpec;
use crate::error::{CURRENT_SCHEMA_VERSION, MAX_SHEET_ID_BYTES, MAX_SHEETS_PER_PROJECT, MAX_TITLE_BYTES};
use crate::ids::SheetId;
use crate::image::MAX_PROJECT_IMAGE_ASSETS;
use crate::layout::MAX_LAYOUT_ITEMS;
use crate::project::{NamedCheckpoint, ProjectDocument, SheetFolderKind};
use crate::result::{ChecklistItem, ChecklistStatus, ErrorCategory, ErrorCode, RemediationKind, ResultStatus};

/// Canonical JSON Schema Draft 2020-12 dialect URI.
pub const DRAFT_2020_12_SCHEMA_URI: &str = "https://json-schema.org/draft/2020-12/schema";

/// Canonical `$id` URI for serialized [`ProjectDocument`] payloads.
pub const PROJECT_DOCUMENT_SCHEMA_ID: &str = "https://github.com/HauserGroup/curvebook/blob/main/docs/schemas/document.schema.json";

/// JSON Schema primitive type discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SchemaType {
    /// JSON `"string"` primitive type.
    String,
    /// JSON `"boolean"` primitive type.
    Boolean,
    /// JSON `"number"` floating-point or numeric primitive type.
    Number,
    /// JSON `"integer"` integral number primitive type.
    Integer,
    /// JSON `"array"` ordered sequence type.
    Array,
    /// JSON `"object"` key-value map type.
    Object,
    /// JSON `"null"` singleton type.
    Null,
}

/// JSON Schema `type` keyword supporting either a single primitive type or a union array (e.g. `["string", "null"]`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum SchemaTypeSpec {
    /// Single primitive JSON Schema `"type"` string (for example `"string"`).
    Single(SchemaType),
    /// Union array of primitive JSON Schema `"type"` strings (for example `["string", "null"]`).
    Multiple(Vec<SchemaType>),
}

/// Literal JSON scalar used in `enum`, `const`, `minimum`, and `maximum` schema constraints.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum SchemaLiteral {
    /// JSON `null` literal.
    Null,
    /// JSON `true` or `false` boolean literal.
    Bool(bool),
    /// Signed 64-bit integer literal.
    Integer(i64),
    /// IEEE-754 floating-point literal.
    Number(f64),
    /// UTF-8 string literal.
    String(String),
}

/// JSON Schema `additionalProperties` constraint: either a boolean or a value sub-schema.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum AdditionalProperties {
    /// Boolean flag (`false` forbids undeclared object keys; `true` allows arbitrary extra keys).
    Allowed(bool),
    /// Value schema applied to every undeclared/dynamic property key in a map object.
    Schema(Box<SchemaNode>),
}

/// Pure-Rust representation of a JSON Schema Draft 2020-12 node.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SchemaNode {
    /// Optional `$schema` dialect URI (for example [`DRAFT_2020_12_SCHEMA_URI`]); `None` on nested sub-schemas.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Optional canonical `$id` resource URI; `None` on local sub-schemas.
    #[serde(rename = "$id", default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Optional `$ref` JSON Pointer or URI reference (for example `"#/$defs/SheetId"`); `None` when defined inline.
    #[serde(rename = "$ref", default, skip_serializing_if = "Option::is_none")]
    pub schema_ref: Option<String>,
    /// Named reusable sub-schema definitions serialized under `$defs` (omitted when empty).
    #[serde(rename = "$defs", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defs: BTreeMap<String, SchemaNode>,
    /// Optional short human-readable schema `title`; `None` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional human-readable `description` prose; `None` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional JSON `type` constraint (`Single` or `Multiple`); `None` on `$ref`, `oneOf`, `anyOf`, or unconstrained nodes.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub schema_type: Option<SchemaTypeSpec>,
    /// Optional exact `const` scalar value constraint; `None` when unconstrained.
    #[serde(rename = "const", default, skip_serializing_if = "Option::is_none")]
    pub const_value: Option<SchemaLiteral>,
    /// Closed set of allowed `enum` scalar values (omitted when empty).
    #[serde(rename = "enum", default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<SchemaLiteral>,
    /// Deterministically sorted object property schemas keyed by JSON property name (omitted when empty).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, SchemaNode>,
    /// Property names that must be present on matching JSON objects (omitted when empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    /// Policy or value schema governing object keys absent from `properties`; `None` leaves `additionalProperties` unspecified.
    #[serde(rename = "additionalProperties", default, skip_serializing_if = "Option::is_none")]
    pub additional_properties: Option<AdditionalProperties>,
    /// Element schema for homogeneous `"array"` instances; `None` for non-array schemas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<SchemaNode>>,
    /// Inclusive minimum element count (`minItems`) for arrays; `None` when unbounded below.
    #[serde(rename = "minItems", default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u64>,
    /// Inclusive maximum element count (`maxItems`) for arrays; `None` when unbounded above.
    #[serde(rename = "maxItems", default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u64>,
    /// Inclusive numeric lower bound (`minimum`); `None` when unbounded below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<SchemaLiteral>,
    /// Inclusive numeric upper bound (`maximum`); `None` when unbounded above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<SchemaLiteral>,
    /// Inclusive minimum UTF-8 character length (`minLength`) for strings; `None` when unbounded.
    #[serde(rename = "minLength", default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u64>,
    /// Inclusive maximum length (`maxLength`) for strings; `None` when unbounded.
    #[serde(rename = "maxLength", default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u64>,
    /// Optional ECMA-262 regular expression `pattern` constraint for strings; `None` when unconstrained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// Optional semantic `format` annotation (for example `"date-time"` or `"uri"`); `None` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Exclusive union (`oneOf`) requiring instances to match exactly one sub-schema (omitted when empty).
    #[serde(rename = "oneOf", default, skip_serializing_if = "Vec::is_empty")]
    pub one_of: Vec<SchemaNode>,
    /// Inclusive union (`anyOf`) requiring instances to match at least one sub-schema (omitted when empty).
    #[serde(rename = "anyOf", default, skip_serializing_if = "Vec::is_empty")]
    pub any_of: Vec<SchemaNode>,
}

/// Alias emphasizing object-level JSON Schema nodes.
pub type SchemaObject = SchemaNode;

impl SchemaNode {
    /// Attach `$schema` dialect URI to this schema node.
    #[must_use]
    pub fn with_schema_uri(mut self, uri: impl Into<String>) -> Self {
        self.schema = Some(uri.into());
        self
    }

    /// Attach `$id` URI to this schema node.
    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Attach human-readable `title` metadata to this schema node.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Attach human-readable `description` metadata to this schema node.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Insert a named definition under `$defs`.
    #[must_use]
    pub fn with_def(mut self, name: impl Into<String>, schema: SchemaNode) -> Self {
        self.defs.insert(name.into(), schema);
        self
    }

    /// Set inclusive integer lower bound `minimum`.
    #[must_use]
    pub fn with_minimum_i64(mut self, min: i64) -> Self {
        self.minimum = Some(SchemaLiteral::Integer(min));
        self
    }

    /// Set inclusive numeric lower bound `minimum`.
    #[must_use]
    pub fn with_minimum_f64(mut self, min: f64) -> Self {
        self.minimum = Some(SchemaLiteral::Number(min));
        self
    }

    /// Set inclusive integer upper bound `maximum`.
    #[must_use]
    pub fn with_maximum_i64(mut self, max: i64) -> Self {
        self.maximum = Some(SchemaLiteral::Integer(max));
        self
    }

    /// Set inclusive numeric upper bound `maximum`.
    #[must_use]
    pub fn with_maximum_f64(mut self, max: f64) -> Self {
        self.maximum = Some(SchemaLiteral::Number(max));
        self
    }

    /// Set `minLength` on string schemas.
    #[must_use]
    pub fn with_min_length(mut self, min_length: u64) -> Self {
        self.min_length = Some(min_length);
        self
    }

    /// Set `maxLength` on string schemas.
    #[must_use]
    pub fn with_max_length(mut self, max_length: u64) -> Self {
        self.max_length = Some(max_length);
        self
    }

    /// Set string regex `pattern` constraint.
    #[must_use]
    pub fn with_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.pattern = Some(pattern.into());
        self
    }

    /// Set string `format` annotation (e.g. `"date-time"` or `"uri"`).
    #[must_use]
    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }
}

/// Types able to describe their serialized JSON structure via Draft 2020-12 [`SchemaNode`].
pub trait JsonSchema {
    /// Canonical schema definition key suitable for `#/$defs/<name>`.
    fn schema_name() -> &'static str;

    /// Construct the Draft 2020-12 [`SchemaNode`] describing `Self`.
    fn json_schema() -> SchemaNode;
}

/// Build a `"type": "string"` schema.
#[must_use]
pub fn string_schema() -> SchemaNode {
    SchemaNode { schema_type: Some(SchemaTypeSpec::Single(SchemaType::String)), ..SchemaNode::default() }
}

/// Build a `"type": "boolean"` schema.
#[must_use]
pub fn bool_schema() -> SchemaNode {
    SchemaNode { schema_type: Some(SchemaTypeSpec::Single(SchemaType::Boolean)), ..SchemaNode::default() }
}

/// Build a `"type": "number"` schema.
#[must_use]
pub fn number_schema() -> SchemaNode {
    SchemaNode { schema_type: Some(SchemaTypeSpec::Single(SchemaType::Number)), ..SchemaNode::default() }
}

/// Build a `"type": "integer"` schema.
#[must_use]
pub fn integer_schema() -> SchemaNode {
    SchemaNode { schema_type: Some(SchemaTypeSpec::Single(SchemaType::Integer)), ..SchemaNode::default() }
}

/// Build a `"type": "null"` schema.
#[must_use]
pub fn null_schema() -> SchemaNode {
    SchemaNode { schema_type: Some(SchemaTypeSpec::Single(SchemaType::Null)), ..SchemaNode::default() }
}

/// Build a `"type": "array"` schema with element `items`.
#[must_use]
pub fn array_schema(items: SchemaNode) -> SchemaNode {
    SchemaNode { schema_type: Some(SchemaTypeSpec::Single(SchemaType::Array)), items: Some(Box::new(items)), ..SchemaNode::default() }
}

/// Build a `"type": "string"` schema constrained to a fixed set of `enum` string literals.
#[must_use]
pub fn enum_schema(variants: &[&str]) -> SchemaNode {
    SchemaNode {
        schema_type: Some(SchemaTypeSpec::Single(SchemaType::String)),
        enum_values: variants.iter().map(|v| SchemaLiteral::String((*v).to_owned())).collect(),
        ..SchemaNode::default()
    }
}

/// Build a schema matching an exact constant string literal (`type: "string"`, `const: value`).
#[must_use]
pub fn const_string_schema(value: &str) -> SchemaNode {
    SchemaNode {
        schema_type: Some(SchemaTypeSpec::Single(SchemaType::String)),
        const_value: Some(SchemaLiteral::String(value.to_owned())),
        ..SchemaNode::default()
    }
}

/// Build a schema matching an exact constant integer literal (`type: "integer"`, `const: value`).
#[must_use]
pub fn const_integer_schema(value: i64) -> SchemaNode {
    SchemaNode {
        schema_type: Some(SchemaTypeSpec::Single(SchemaType::Integer)),
        const_value: Some(SchemaLiteral::Integer(value)),
        ..SchemaNode::default()
    }
}

/// Build an internal JSON Pointer reference (`$ref: "#/$defs/<def_name>"`).
#[must_use]
pub fn ref_schema(def_name: &str) -> SchemaNode {
    SchemaNode { schema_ref: Some(format!("#/$defs/{def_name}")), ..SchemaNode::default() }
}

/// Build a `type: "object"` schema with explicit properties, required field list, and `additionalProperties` policy.
#[must_use]
pub fn object_schema(properties: Vec<(&str, SchemaNode)>, required: &[&str], additional_properties: bool) -> SchemaNode {
    let mut prop_map = BTreeMap::new();
    for (name, schema) in properties {
        prop_map.insert(name.to_owned(), schema);
    }
    SchemaNode {
        schema_type: Some(SchemaTypeSpec::Single(SchemaType::Object)),
        properties: prop_map,
        required: required.iter().map(|r| (*r).to_owned()).collect(),
        additional_properties: Some(AdditionalProperties::Allowed(additional_properties)),
        ..SchemaNode::default()
    }
}

/// Build an unconstrained-keys JSON dictionary schema (`type: "object"` with typed `additionalProperties`).
#[must_use]
pub fn map_schema(value_schema: SchemaNode) -> SchemaNode {
    SchemaNode {
        schema_type: Some(SchemaTypeSpec::Single(SchemaType::Object)),
        additional_properties: Some(AdditionalProperties::Schema(Box::new(value_schema))),
        ..SchemaNode::default()
    }
}

/// Wrap `inner` to allow `null` (using `[type, "null"]` for plain primitives or `anyOf` for `$ref`/enums/unions).
#[must_use]
pub fn nullable_schema(mut inner: SchemaNode) -> SchemaNode {
    let can_widen_type = inner.schema_ref.is_none()
        && inner.one_of.is_empty()
        && inner.any_of.is_empty()
        && inner.enum_values.is_empty()
        && inner.const_value.is_none();
    if can_widen_type {
        match inner.schema_type.take() {
            Some(SchemaTypeSpec::Single(SchemaType::Null)) => {
                inner.schema_type = Some(SchemaTypeSpec::Single(SchemaType::Null));
                return inner;
            }
            Some(SchemaTypeSpec::Single(kind)) => {
                inner.schema_type = Some(SchemaTypeSpec::Multiple(vec![kind, SchemaType::Null]));
                return inner;
            }
            Some(SchemaTypeSpec::Multiple(mut kinds)) => {
                if !kinds.contains(&SchemaType::Null) {
                    kinds.push(SchemaType::Null);
                }
                inner.schema_type = Some(SchemaTypeSpec::Multiple(kinds));
                return inner;
            }
            None => {}
        }
    }
    any_of_schema(vec![inner, null_schema()])
}

/// Combine mutually exclusive sub-schemas with `oneOf`.
#[must_use]
pub fn one_of_schema(variants: Vec<SchemaNode>) -> SchemaNode {
    SchemaNode { one_of: variants, ..SchemaNode::default() }
}

/// Combine sub-schemas with `anyOf`.
#[must_use]
pub fn any_of_schema(variants: Vec<SchemaNode>) -> SchemaNode {
    SchemaNode { any_of: variants, ..SchemaNode::default() }
}

impl JsonSchema for String {
    fn schema_name() -> &'static str {
        "string"
    }

    fn json_schema() -> SchemaNode {
        string_schema()
    }
}

impl JsonSchema for bool {
    fn schema_name() -> &'static str {
        "boolean"
    }

    fn json_schema() -> SchemaNode {
        bool_schema()
    }
}

impl JsonSchema for f64 {
    fn schema_name() -> &'static str {
        "number"
    }

    fn json_schema() -> SchemaNode {
        number_schema()
    }
}

impl JsonSchema for f32 {
    fn schema_name() -> &'static str {
        "number"
    }

    fn json_schema() -> SchemaNode {
        number_schema()
    }
}

impl JsonSchema for u32 {
    fn schema_name() -> &'static str {
        "u32"
    }

    fn json_schema() -> SchemaNode {
        integer_schema().with_minimum_i64(0).with_maximum_i64(i64::from(u32::MAX))
    }
}

impl JsonSchema for u64 {
    fn schema_name() -> &'static str {
        "u64"
    }

    fn json_schema() -> SchemaNode {
        integer_schema().with_minimum_i64(0)
    }
}

impl JsonSchema for usize {
    fn schema_name() -> &'static str {
        "usize"
    }

    fn json_schema() -> SchemaNode {
        integer_schema().with_minimum_i64(0)
    }
}

impl JsonSchema for i64 {
    fn schema_name() -> &'static str {
        "i64"
    }

    fn json_schema() -> SchemaNode {
        integer_schema()
    }
}

impl<T: JsonSchema> JsonSchema for Vec<T> {
    fn schema_name() -> &'static str {
        "array"
    }

    fn json_schema() -> SchemaNode {
        array_schema(T::json_schema())
    }
}

impl<T: JsonSchema> JsonSchema for Option<T> {
    fn schema_name() -> &'static str {
        T::schema_name()
    }

    fn json_schema() -> SchemaNode {
        nullable_schema(T::json_schema())
    }
}

impl<T: JsonSchema> JsonSchema for BTreeMap<String, T> {
    fn schema_name() -> &'static str {
        "map"
    }

    fn json_schema() -> SchemaNode {
        map_schema(T::json_schema())
    }
}

impl JsonSchema for SheetId {
    fn schema_name() -> &'static str {
        "SheetId"
    }

    fn json_schema() -> SchemaNode {
        string_schema()
            .with_min_length(1)
            .with_max_length(MAX_SHEET_ID_BYTES as u64)
            .with_pattern(r"^(?!.*(?:\{\{|\}\}))[^\s:\x00-\x1f\x7f]{1,256}$")
            .with_title("SheetId")
            .with_description("Non-empty hook-safe sheet identifier (1..=256 bytes, no whitespace, control characters, `:`, `{{`, or `}}`).")
    }
}

impl JsonSchema for ErrorCategory {
    fn schema_name() -> &'static str {
        "ErrorCategory"
    }

    fn json_schema() -> SchemaNode {
        let variants: Vec<&str> = Self::ALL.iter().map(|c| c.as_str()).collect();
        enum_schema(&variants).with_title("ErrorCategory")
    }
}

impl JsonSchema for ErrorCode {
    fn schema_name() -> &'static str {
        "ErrorCode"
    }

    fn json_schema() -> SchemaNode {
        let variants: Vec<&str> = Self::ALL.iter().map(|c| c.as_str()).collect();
        enum_schema(&variants).with_title("ErrorCode")
    }
}

impl JsonSchema for RemediationKind {
    fn schema_name() -> &'static str {
        "RemediationKind"
    }

    fn json_schema() -> SchemaNode {
        enum_schema(&["WelchT", "MannWhitneyU", "WilcoxonSignedRank", "OneSampleWilcoxon", "WelchAnova", "KruskalWallis", "Friedman"])
            .with_title("RemediationKind")
    }
}

impl JsonSchema for ChecklistStatus {
    fn schema_name() -> &'static str {
        "ChecklistStatus"
    }

    fn json_schema() -> SchemaNode {
        enum_schema(&["Pass", "Warn", "Violated", "Info"]).with_title("ChecklistStatus")
    }
}

impl JsonSchema for TableKind {
    fn schema_name() -> &'static str {
        "TableKind"
    }

    fn json_schema() -> SchemaNode {
        enum_schema(&["Xy", "Column", "Grouped", "Contingency", "Survival", "PartsOfWhole", "MultipleVariables", "Nested"]).with_title("TableKind")
    }
}

impl JsonSchema for SheetFolderKind {
    fn schema_name() -> &'static str {
        "SheetFolderKind"
    }

    fn json_schema() -> SchemaNode {
        enum_schema(&["Data", "Info", "Results", "Graphs", "Layouts"]).with_title("SheetFolderKind")
    }
}

impl JsonSchema for ResultStatus {
    fn schema_name() -> &'static str {
        "ResultStatus"
    }

    fn json_schema() -> SchemaNode {
        let code_schema = any_of_schema(vec![
            ref_schema(ErrorCode::schema_name()),
            string_schema().with_description(
                "Legacy or historical error code accepted by ErrorCode::from_code_str (normalizes unknown strings to internal_error).",
            ),
        ]);
        let failed_payload = object_schema(vec![("code", code_schema), ("message", string_schema())], &["code", "message"], false);
        let failed_variant = object_schema(vec![("Failed", failed_payload)], &["Failed"], false);
        one_of_schema(vec![const_string_schema("Dirty"), const_string_schema("UpToDate"), failed_variant]).with_title("ResultStatus")
    }
}

impl JsonSchema for ChecklistItem {
    fn schema_name() -> &'static str {
        "ChecklistItem"
    }

    fn json_schema() -> SchemaNode {
        object_schema(
            vec![
                ("assumption", string_schema()),
                ("status", ref_schema(ChecklistStatus::schema_name())),
                ("diagnostic_metric", nullable_schema(string_schema())),
                ("explanation", string_schema()),
                ("remediation", nullable_schema(ref_schema(RemediationKind::schema_name()))),
            ],
            &["assumption", "status", "diagnostic_metric", "explanation"],
            false,
        )
        .with_title("ChecklistItem")
    }
}

impl JsonSchema for NamedCheckpoint {
    fn schema_name() -> &'static str {
        "NamedCheckpoint"
    }

    fn json_schema() -> SchemaNode {
        object_schema(
            vec![
                ("id", string_schema().with_min_length(1).with_max_length(MAX_SHEET_ID_BYTES as u64).with_pattern(r"\S")),
                ("name", title_string_schema()),
                ("note", title_string_schema()),
                (
                    "created_utc",
                    string_schema()
                        .with_min_length(1)
                        .with_pattern(r"\S")
                        .with_description("Creation timestamp string (RFC 3339 UTC or `unix:<seconds>`)."),
                ),
                ("snapshot_json", string_schema().with_min_length(1)),
            ],
            &["id", "name", "note", "created_utc", "snapshot_json"],
            false,
        )
        .with_title("NamedCheckpoint")
    }
}

fn title_string_schema() -> SchemaNode {
    string_schema()
        .with_max_length(MAX_TITLE_BYTES as u64)
        .with_description(
            "Title text capped at 4,096 UTF-8 bytes by runtime validate_title (JSON Schema maxLength bounds code points; multi-byte characters also count toward the 4,096 UTF-8 byte ceiling).",
        )
}

fn rgba8_schema() -> SchemaNode {
    let channel = integer_schema().with_minimum_i64(0).with_maximum_i64(255);
    object_schema(vec![("r", channel.clone()), ("g", channel.clone()), ("b", channel.clone()), ("a", channel)], &["r", "g", "b", "a"], false)
        .with_title("Rgba8")
}

fn size_pt_schema() -> SchemaNode {
    object_schema(
        vec![("width", number_schema().with_minimum_f64(0.0)), ("height", number_schema().with_minimum_f64(0.0))],
        &["width", "height"],
        false,
    )
    .with_title("SizePt")
}

fn rect_pt_schema() -> SchemaNode {
    object_schema(
        vec![
            ("x", number_schema()),
            ("y", number_schema()),
            ("width", number_schema().with_minimum_f64(0.0)),
            ("height", number_schema().with_minimum_f64(0.0)),
        ],
        &["x", "y", "width", "height"],
        false,
    )
    .with_title("RectPt")
}

fn floating_note_schema() -> SchemaNode {
    object_schema(
        vec![
            ("id", string_schema().with_min_length(1).with_max_length(MAX_SHEET_ID_BYTES as u64).with_pattern(r"\S")),
            ("fill_color", ref_schema("Rgba8")),
            ("bounds_pt", ref_schema("RectPt")),
            ("minimized", bool_schema()),
            ("body_markdown", string_schema()),
        ],
        &["id", "fill_color", "bounds_pt", "minimized", "body_markdown"],
        false,
    )
    .with_title("FloatingNote")
}

fn cell_coord_schema() -> SchemaNode {
    let idx = integer_schema().with_minimum_i64(0);
    object_schema(vec![("row", idx.clone()), ("col", idx.clone()), ("subcol", idx)], &["row", "col", "subcol"], false).with_title("CellCoord")
}

fn cell_exclusion_note_schema() -> SchemaNode {
    object_schema(
        vec![("coord", ref_schema("CellCoord")), ("reason_tag", string_schema()), ("note", string_schema()), ("timestamp_utc", string_schema())],
        &["coord", "reason_tag", "note", "timestamp_utc"],
        false,
    )
    .with_title("CellExclusionNote")
}

fn plate_cell_origin_schema() -> SchemaNode {
    object_schema(vec![("coord", ref_schema("CellCoord")), ("well", title_string_schema().with_min_length(1))], &["coord", "well"], false)
        .with_title("PlateCellOrigin")
}

fn linked_data_region_schema() -> SchemaNode {
    let range = object_schema(vec![("start", ref_schema("CellCoord")), ("end", ref_schema("CellCoord"))], &["start", "end"], false);
    let link_spec = SchemaNode { schema_type: Some(SchemaTypeSpec::Single(SchemaType::Object)), ..SchemaNode::default() };
    object_schema(
        vec![("id", string_schema().with_min_length(1)), ("spec", link_spec), ("target_range", range), ("provenance_summary", string_schema())],
        &["id", "spec", "target_range", "provenance_summary"],
        false,
    )
    .with_title("LinkedDataRegion")
}

fn table_archetype_schema() -> SchemaNode {
    let table_body = SchemaNode { schema_type: Some(SchemaTypeSpec::Single(SchemaType::Object)), ..SchemaNode::default() };
    let variants = ["Xy", "Column", "Grouped", "Contingency", "Survival", "PartsOfWhole", "MultipleVariables", "Nested"]
        .into_iter()
        .map(|tag| object_schema(vec![(tag, table_body.clone())], &[tag], false))
        .collect();
    one_of_schema(variants).with_title("TableArchetype")
}

fn info_constant_schema() -> SchemaNode {
    let value_schema = one_of_schema(vec![
        const_string_schema("Empty"),
        object_schema(vec![("Number", number_schema())], &["Number"], false),
        object_schema(vec![("Text", string_schema())], &["Text"], false),
        object_schema(vec![("Date", string_schema())], &["Date"], false),
    ]);
    object_schema(
        vec![("name", title_string_schema().with_min_length(1)), ("value", value_schema), ("units", nullable_schema(title_string_schema()))],
        &["name", "value", "units"],
        false,
    )
    .with_title("InfoConstant")
}

fn analysis_spec_schema() -> SchemaNode {
    let kind_obj = SchemaNode { schema_type: Some(SchemaTypeSpec::Single(SchemaType::Object)), ..SchemaNode::default() };
    let power_variant = object_schema(vec![("PowerAnalysis", ref_schema(PowerAnalysisSpec::schema_name()))], &["PowerAnalysis"], false);
    let other_kind = object_schema(
        crate::analysis::ALL_ANALYSIS_DESCRIPTORS
            .iter()
            .filter(|desc| desc.id != crate::analysis::AnalysisKindId::PowerAnalysis)
            .map(|desc| (desc.serde_variant, kind_obj.clone()))
            .collect(),
        &[],
        false,
    );
    let kind_schema = any_of_schema(vec![power_variant, other_kind]);
    let p_value_format = one_of_schema(vec![
        const_string_schema("FourDecimal"),
        const_string_schema("Apa"),
        const_string_schema("Nejm"),
        object_schema(
            vec![(
                "Custom",
                object_schema(
                    vec![
                        ("decimals", integer_schema().with_minimum_i64(1).with_maximum_i64(15)),
                        ("scientific_below", nullable_schema(number_schema().with_minimum_f64(0.0))),
                    ],
                    &["decimals", "scientific_below"],
                    false,
                ),
            )],
            &["Custom"],
            false,
        ),
    ]);
    object_schema(
        vec![
            ("id", ref_schema(SheetId::schema_name())),
            ("source_sheet_id", ref_schema(SheetId::schema_name())),
            ("selected_data_sets", array_schema(integer_schema().with_minimum_i64(0))),
            ("auto_title", string_schema()),
            ("custom_title", nullable_schema(title_string_schema())),
            ("confidence_level", number_schema().with_minimum_f64(0.01).with_maximum_f64(99.99)),
            ("p_value_format", p_value_format),
            ("kind", kind_schema),
        ],
        &["id", "source_sheet_id", "selected_data_sets", "auto_title", "custom_title", "confidence_level", "p_value_format", "kind"],
        false,
    )
    .with_title("AnalysisSpec")
}

fn result_tab_schema() -> SchemaNode {
    let cell = SchemaNode { schema_type: Some(SchemaTypeSpec::Multiple(vec![SchemaType::String, SchemaType::Object])), ..SchemaNode::default() };
    let tab_kind = enum_schema(&[
        "MainSummary",
        "AnovaTable",
        "MultipleComparisons",
        "ParametersAndConfidenceIntervals",
        "ModelComparison",
        "Residuals",
        "DescriptiveSummary",
        "InterpolatedUnknowns",
        "CurveCoordinates",
        "NumberAtRisk",
        "Diagnostics",
        "CovarianceMatrix",
        "NormalQq",
        "BaselineSurvival",
        "LogMinusLog",
    ]);
    object_schema(
        vec![
            ("id", string_schema().with_min_length(1)),
            ("title", title_string_schema()),
            ("kind", tab_kind),
            ("column_headers", array_schema(string_schema())),
            ("row_labels", array_schema(string_schema())),
            ("rows", array_schema(array_schema(cell))),
            ("footnotes", array_schema(string_schema())),
        ],
        &["id", "title", "kind", "column_headers", "row_labels", "rows", "footnotes"],
        false,
    )
    .with_title("ResultTab")
}

fn result_scalar_entry_schema() -> SchemaNode {
    object_schema(
        vec![
            ("key", string_schema().with_min_length(1)),
            ("data_set_index", nullable_schema(integer_schema().with_minimum_i64(0))),
            ("value", number_schema()),
            ("units", nullable_schema(string_schema())),
        ],
        &["key", "data_set_index", "value", "units"],
        false,
    )
    .with_title("ResultScalarEntry")
}

fn pairwise_comparison_record_schema() -> SchemaNode {
    object_schema(
        vec![
            ("group_a", integer_schema().with_minimum_i64(0)),
            ("group_b", integer_schema().with_minimum_i64(0)),
            ("label_a", string_schema()),
            ("label_b", string_schema()),
            ("estimate_diff", nullable_schema(number_schema())),
            ("raw_p_or_q", number_schema().with_minimum_f64(0.0).with_maximum_f64(1.0)),
            ("is_fdr_q", bool_schema()),
            ("significant", bool_schema()),
        ],
        &["group_a", "group_b", "label_a", "label_b", "estimate_diff", "raw_p_or_q", "is_fdr_q", "significant"],
        false,
    )
    .with_title("PairwiseComparisonRecord")
}

fn fitted_curve_overlay_schema() -> SchemaNode {
    let pt = object_schema(
        vec![
            ("x", number_schema()),
            ("y", number_schema()),
            ("ci_lower", nullable_schema(number_schema())),
            ("ci_upper", nullable_schema(number_schema())),
            ("pred_lower", nullable_schema(number_schema())),
            ("pred_upper", nullable_schema(number_schema())),
        ],
        &["x", "y", "ci_lower", "ci_upper", "pred_lower", "pred_upper"],
        false,
    );
    object_schema(
        vec![
            ("data_set_index", integer_schema().with_minimum_i64(0)),
            ("equation_id", string_schema()),
            ("converged", bool_schema()),
            ("ambiguous", bool_schema()),
            ("points", array_schema(pt)),
        ],
        &["data_set_index", "equation_id", "converged", "ambiguous", "points"],
        false,
    )
    .with_title("FittedCurveOverlay")
}

fn graph_spec_schema() -> SchemaNode {
    let axis = object_schema(
        vec![
            ("title", title_string_schema()),
            (
                "segments",
                SchemaNode {
                    min_items: Some(1),
                    max_items: Some(3),
                    ..array_schema(SchemaNode { schema_type: Some(SchemaTypeSpec::Single(SchemaType::Object)), ..SchemaNode::default() })
                },
            ),
            ("stroke_width_pt", number_schema().with_minimum_f64(0.0)),
        ],
        &["title", "segments", "stroke_width_pt"],
        true,
    );
    object_schema(
        vec![
            ("primary_source_sheet_id", ref_schema(SheetId::schema_name())),
            ("canvas_size_pt", ref_schema("SizePt")),
            ("paper_fill", nullable_schema(ref_schema("Rgba8"))),
            ("title", title_string_schema()),
            ("x_axis", axis.clone()),
            ("y_axis", axis.clone()),
            ("y2_axis", nullable_schema(axis)),
        ],
        &[
            "kind",
            "primary_source_sheet_id",
            "palette",
            "canvas_size_pt",
            "paper_fill",
            "title",
            "frame_preset",
            "x_axis",
            "y_axis",
            "series",
            "legend",
        ],
        true,
    )
    .with_title("GraphSpec")
}

fn layout_spec_schema() -> SchemaNode {
    let item = SchemaNode { schema_type: Some(SchemaTypeSpec::Single(SchemaType::Object)), ..SchemaNode::default() };
    let grid = SchemaNode { min_items: Some(2), max_items: Some(2), ..array_schema(integer_schema().with_minimum_i64(1).with_maximum_i64(255)) };
    object_schema(
        vec![
            ("width", number_schema().with_minimum_f64(f64::MIN_POSITIVE)),
            ("height", number_schema().with_minimum_f64(f64::MIN_POSITIVE)),
            ("unit", enum_schema(&["Millimeters", "Centimeters", "Inches", "Points"])),
            ("orientation", enum_schema(&["Portrait", "Landscape"])),
            ("grid_template", nullable_schema(grid)),
            ("align_plot_origins", bool_schema()),
            ("panel_badges", enum_schema(&["UpperCaseLetters", "LowerCaseLetters", "None"])),
            ("background_fill", nullable_schema(ref_schema("Rgba8"))),
            ("items", SchemaNode { max_items: Some(MAX_LAYOUT_ITEMS as u64), ..array_schema(item) }),
        ],
        &["width", "height", "unit", "orientation", "grid_template", "align_plot_origins", "panel_badges", "background_fill", "items"],
        false,
    )
    .with_title("LayoutSpec")
}

fn data_sheet_schema() -> SchemaNode {
    object_schema(
        vec![
            ("id", ref_schema(SheetId::schema_name())),
            ("title", title_string_schema()),
            ("archetype", ref_schema("TableArchetype")),
            ("frozen", bool_schema()),
            ("info_sheet_id", nullable_schema(ref_schema(SheetId::schema_name()))),
            ("linked_regions", array_schema(ref_schema("LinkedDataRegion"))),
            ("floating_notes", array_schema(ref_schema("FloatingNote"))),
            ("color_tag", nullable_schema(ref_schema("Rgba8"))),
            ("exclusion_notes", array_schema(ref_schema("CellExclusionNote"))),
            ("plate_well_origins", array_schema(ref_schema("PlateCellOrigin"))),
        ],
        &["id", "title", "archetype", "frozen", "info_sheet_id", "linked_regions", "floating_notes", "color_tag"],
        false,
    )
    .with_title("DataSheet")
}

fn info_sheet_schema() -> SchemaNode {
    object_schema(
        vec![
            ("id", ref_schema(SheetId::schema_name())),
            ("title", title_string_schema()),
            ("linked_data_sheets", array_schema(ref_schema(SheetId::schema_name()))),
            ("constants", array_schema(ref_schema("InfoConstant"))),
            ("notes_markdown", string_schema()),
            ("floating_notes", array_schema(ref_schema("FloatingNote"))),
            ("color_tag", nullable_schema(ref_schema("Rgba8"))),
        ],
        &["id", "title", "linked_data_sheets", "constants", "notes_markdown", "floating_notes", "color_tag"],
        false,
    )
    .with_title("InfoSheet")
}

fn source_cell_id_schema() -> SchemaNode {
    object_schema(
        vec![
            ("sheet_id", ref_schema(SheetId::schema_name())),
            ("row", integer_schema().with_minimum_i64(0)),
            ("col", integer_schema().with_minimum_i64(0)),
            ("subcol", integer_schema().with_minimum_i64(0)),
        ],
        &["sheet_id", "row", "col", "subcol"],
        false,
    )
    .with_title("SourceCellId")
}

fn result_sheet_schema() -> SchemaNode {
    object_schema(
        vec![
            ("id", ref_schema(SheetId::schema_name())),
            ("analysis", ref_schema("AnalysisSpec")),
            ("status", ref_schema(ResultStatus::schema_name())),
            ("analyzable_data", nullable_schema(ref_schema("DataSheet"))),
            ("tabs", array_schema(ref_schema("ResultTab"))),
            ("scalars", array_schema(ref_schema("ResultScalarEntry"))),
            ("pairwise_comparisons", array_schema(ref_schema("PairwiseComparisonRecord"))),
            ("fitted_curves", array_schema(ref_schema("FittedCurveOverlay"))),
            ("eliminated_outliers", array_schema(ref_schema("SourceCellId"))),
            ("checklist", array_schema(ref_schema(ChecklistItem::schema_name()))),
            ("floating_notes", array_schema(ref_schema("FloatingNote"))),
            ("color_tag", nullable_schema(ref_schema("Rgba8"))),
        ],
        &[
            "id",
            "analysis",
            "status",
            "analyzable_data",
            "tabs",
            "scalars",
            "pairwise_comparisons",
            "fitted_curves",
            "checklist",
            "floating_notes",
            "color_tag",
        ],
        false,
    )
    .with_title("ResultSheet")
}

fn graph_sheet_schema() -> SchemaNode {
    object_schema(
        vec![
            ("id", ref_schema(SheetId::schema_name())),
            ("title", title_string_schema()),
            ("initialized", bool_schema()),
            ("spec", ref_schema("GraphSpec")),
            ("floating_notes", array_schema(ref_schema("FloatingNote"))),
            ("color_tag", nullable_schema(ref_schema("Rgba8"))),
        ],
        &["id", "title", "initialized", "spec", "floating_notes", "color_tag"],
        false,
    )
    .with_title("GraphSheet")
}

fn layout_sheet_schema() -> SchemaNode {
    object_schema(
        vec![
            ("id", ref_schema(SheetId::schema_name())),
            ("title", title_string_schema()),
            ("spec", ref_schema("LayoutSpec")),
            ("floating_notes", array_schema(ref_schema("FloatingNote"))),
            ("color_tag", nullable_schema(ref_schema("Rgba8"))),
        ],
        &["id", "title", "spec", "floating_notes", "color_tag"],
        false,
    )
    .with_title("LayoutSheet")
}

fn project_image_asset_schema() -> SchemaNode {
    let pixel_size_schema = SchemaNode { min_items: Some(2), max_items: Some(2), ..array_schema(integer_schema().with_minimum_i64(1)) };
    object_schema(
        vec![
            ("asset_id", string_schema().with_min_length(4).with_pattern(r"^img-[0-9a-f]{16}$")),
            ("media_type", enum_schema(&["Png", "Jpeg", "Tiff"])),
            ("pixel_size", pixel_size_schema),
            ("bytes", string_schema()),
        ],
        &["asset_id", "media_type", "pixel_size"],
        false,
    )
    .with_title("ProjectImageAsset")
}

impl JsonSchema for ProjectDocument {
    fn schema_name() -> &'static str {
        "ProjectDocument"
    }

    fn json_schema() -> SchemaNode {
        project_document_schema()
    }
}

/// Construct the canonical JSON Schema (Draft 2020-12) for [`ProjectDocument`].
#[must_use]
pub fn project_document_schema() -> SchemaNode {
    let max_sheets = Some(MAX_SHEETS_PER_PROJECT as u64);
    object_schema(
        vec![
            ("schema_version", integer_schema().with_minimum_i64(1).with_maximum_i64(i64::from(CURRENT_SCHEMA_VERSION))),
            ("title", title_string_schema()),
            ("data_sheets", SchemaNode { max_items: max_sheets, ..array_schema(ref_schema("DataSheet")) }),
            ("info_sheets", SchemaNode { max_items: max_sheets, ..array_schema(ref_schema("InfoSheet")) }),
            ("result_sheets", SchemaNode { max_items: max_sheets, ..array_schema(ref_schema("ResultSheet")) }),
            ("graph_sheets", SchemaNode { max_items: max_sheets, ..array_schema(ref_schema("GraphSheet")) }),
            ("layout_sheets", SchemaNode { max_items: max_sheets, ..array_schema(ref_schema("LayoutSheet")) }),
            ("active_sheet_id", nullable_schema(ref_schema(SheetId::schema_name()))),
            ("family_anchor_sheet_id", nullable_schema(ref_schema(SheetId::schema_name()))),
            ("checkpoints", array_schema(ref_schema(NamedCheckpoint::schema_name()))),
            ("image_assets", SchemaNode { max_items: Some(MAX_PROJECT_IMAGE_ASSETS as u64), ..array_schema(ref_schema("ProjectImageAsset")) }),
        ],
        &[
            "schema_version",
            "title",
            "data_sheets",
            "info_sheets",
            "result_sheets",
            "graph_sheets",
            "layout_sheets",
            "active_sheet_id",
            "family_anchor_sheet_id",
        ],
        false,
    )
    .with_schema_uri(DRAFT_2020_12_SCHEMA_URI)
    .with_id(PROJECT_DOCUMENT_SCHEMA_ID)
    .with_title("ProjectDocument")
    .with_description("Canonical CurveBook L1 ProjectDocument JSON schema (Draft 2020-12).")
    .with_def(SheetId::schema_name(), SheetId::json_schema())
    .with_def(ErrorCategory::schema_name(), ErrorCategory::json_schema())
    .with_def(ErrorCode::schema_name(), ErrorCode::json_schema())
    .with_def(RemediationKind::schema_name(), RemediationKind::json_schema())
    .with_def(ChecklistStatus::schema_name(), ChecklistStatus::json_schema())
    .with_def(ChecklistItem::schema_name(), ChecklistItem::json_schema())
    .with_def(ResultStatus::schema_name(), ResultStatus::json_schema())
    .with_def(TableKind::schema_name(), TableKind::json_schema())
    .with_def(SheetFolderKind::schema_name(), SheetFolderKind::json_schema())
    .with_def(NamedCheckpoint::schema_name(), NamedCheckpoint::json_schema())
    .with_def(PowerAnalysisSpec::schema_name(), PowerAnalysisSpec::json_schema())
    .with_def("Rgba8", rgba8_schema())
    .with_def("SizePt", size_pt_schema())
    .with_def("RectPt", rect_pt_schema())
    .with_def("FloatingNote", floating_note_schema())
    .with_def("CellCoord", cell_coord_schema())
    .with_def("CellExclusionNote", cell_exclusion_note_schema())
    .with_def("PlateCellOrigin", plate_cell_origin_schema())
    .with_def("LinkedDataRegion", linked_data_region_schema())
    .with_def("TableArchetype", table_archetype_schema())
    .with_def("InfoConstant", info_constant_schema())
    .with_def("AnalysisSpec", analysis_spec_schema())
    .with_def("ResultTab", result_tab_schema())
    .with_def("ResultScalarEntry", result_scalar_entry_schema())
    .with_def("PairwiseComparisonRecord", pairwise_comparison_record_schema())
    .with_def("FittedCurveOverlay", fitted_curve_overlay_schema())
    .with_def("GraphSpec", graph_spec_schema())
    .with_def("LayoutSpec", layout_spec_schema())
    .with_def("DataSheet", data_sheet_schema())
    .with_def("InfoSheet", info_sheet_schema())
    .with_def("SourceCellId", source_cell_id_schema())
    .with_def("ResultSheet", result_sheet_schema())
    .with_def("GraphSheet", graph_sheet_schema())
    .with_def("LayoutSheet", layout_sheet_schema())
    .with_def("ProjectImageAsset", project_image_asset_schema())
}
