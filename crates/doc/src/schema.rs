//! Pure-Rust JSON Schema (Draft 2020-12) model and helper constructors (`C2`).
//!
//! `crates/doc` is an L1 model crate without a direct `serde_json` runtime dependency, so schemas
//! are constructed as strongly typed, deterministically ordered [`SchemaNode`] values that derive
//! `serde::Serialize` / `serde::Deserialize` and convert trivially via `serde_json::to_value` in
//! downstream crates (`curvebook-format`, `curvebook-engine`).

use std::collections::BTreeMap;

use crate::TableKind;
use crate::error::{CURRENT_SCHEMA_VERSION, MAX_TITLE_BYTES};
use crate::ids::SheetId;
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
    String,
    Boolean,
    Number,
    Integer,
    Array,
    Object,
    Null,
}

/// JSON Schema `type` keyword supporting either a single primitive type or a union array (e.g. `["string", "null"]`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum SchemaTypeSpec {
    Single(SchemaType),
    Multiple(Vec<SchemaType>),
}

/// Literal JSON scalar used in `enum`, `const`, `minimum`, and `maximum` schema constraints.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum SchemaLiteral {
    Null,
    Bool(bool),
    Integer(i64),
    Number(f64),
    String(String),
}

/// JSON Schema `additionalProperties` constraint: either a boolean or a value sub-schema.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum AdditionalProperties {
    Allowed(bool),
    Schema(Box<SchemaNode>),
}

/// Pure-Rust representation of a JSON Schema Draft 2020-12 node.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SchemaNode {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(rename = "$id", default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "$ref", default, skip_serializing_if = "Option::is_none")]
    pub schema_ref: Option<String>,
    #[serde(rename = "$defs", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defs: BTreeMap<String, SchemaNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub schema_type: Option<SchemaTypeSpec>,
    #[serde(rename = "const", default, skip_serializing_if = "Option::is_none")]
    pub const_value: Option<SchemaLiteral>,
    #[serde(rename = "enum", default, skip_serializing_if = "Vec::is_empty")]
    pub enum_values: Vec<SchemaLiteral>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, SchemaNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    #[serde(rename = "additionalProperties", default, skip_serializing_if = "Option::is_none")]
    pub additional_properties: Option<AdditionalProperties>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<SchemaNode>>,
    #[serde(rename = "minItems", default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u64>,
    #[serde(rename = "maxItems", default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<SchemaLiteral>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<SchemaLiteral>,
    #[serde(rename = "minLength", default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u64>,
    #[serde(rename = "maxLength", default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(rename = "oneOf", default, skip_serializing_if = "Vec::is_empty")]
    pub one_of: Vec<SchemaNode>,
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
            .with_pattern(r"\S")
            .with_title("SheetId")
            .with_description("Non-whitespace sheet identifier (trimmed non-empty).")
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
                ("id", string_schema().with_min_length(1).with_pattern(r"\S")),
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

fn data_sheet_schema() -> SchemaNode {
    object_schema(
        vec![
            ("id", ref_schema(SheetId::schema_name())),
            ("title", title_string_schema()),
            ("archetype", any_json_schema()),
            ("frozen", bool_schema()),
            ("info_sheet_id", nullable_schema(ref_schema(SheetId::schema_name()))),
            ("linked_regions", array_schema(any_json_schema())),
            ("floating_notes", array_schema(any_json_schema())),
            ("color_tag", nullable_schema(any_json_schema())),
            ("exclusion_notes", array_schema(any_json_schema())),
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
            ("constants", array_schema(any_json_schema())),
            ("notes_markdown", string_schema()),
            ("floating_notes", array_schema(any_json_schema())),
            ("color_tag", nullable_schema(any_json_schema())),
        ],
        &["id", "title", "linked_data_sheets", "constants", "notes_markdown", "floating_notes", "color_tag"],
        false,
    )
    .with_title("InfoSheet")
}

fn result_sheet_schema() -> SchemaNode {
    object_schema(
        vec![
            ("id", ref_schema(SheetId::schema_name())),
            ("analysis", any_json_schema()),
            ("status", ref_schema(ResultStatus::schema_name())),
            ("analyzable_data", nullable_schema(ref_schema("DataSheet"))),
            ("tabs", array_schema(any_json_schema())),
            ("scalars", array_schema(any_json_schema())),
            ("pairwise_comparisons", array_schema(any_json_schema())),
            ("fitted_curves", array_schema(any_json_schema())),
            ("checklist", array_schema(ref_schema(ChecklistItem::schema_name()))),
            ("floating_notes", array_schema(any_json_schema())),
            ("color_tag", nullable_schema(any_json_schema())),
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
            ("spec", any_json_schema()),
            ("floating_notes", array_schema(any_json_schema())),
            ("color_tag", nullable_schema(any_json_schema())),
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
            ("spec", any_json_schema()),
            ("floating_notes", array_schema(any_json_schema())),
            ("color_tag", nullable_schema(any_json_schema())),
        ],
        &["id", "title", "spec", "floating_notes", "color_tag"],
        false,
    )
    .with_title("LayoutSheet")
}

fn any_json_schema() -> SchemaNode {
    SchemaNode::default()
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
    object_schema(
        vec![
            ("schema_version", integer_schema().with_minimum_i64(1).with_maximum_i64(i64::from(CURRENT_SCHEMA_VERSION))),
            ("title", title_string_schema()),
            ("data_sheets", array_schema(ref_schema("DataSheet"))),
            ("info_sheets", array_schema(ref_schema("InfoSheet"))),
            ("result_sheets", array_schema(ref_schema("ResultSheet"))),
            ("graph_sheets", array_schema(ref_schema("GraphSheet"))),
            ("layout_sheets", array_schema(ref_schema("LayoutSheet"))),
            ("active_sheet_id", nullable_schema(ref_schema(SheetId::schema_name()))),
            ("family_anchor_sheet_id", nullable_schema(ref_schema(SheetId::schema_name()))),
            ("checkpoints", array_schema(ref_schema(NamedCheckpoint::schema_name()))),
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
    .with_def("DataSheet", data_sheet_schema())
    .with_def("InfoSheet", info_sheet_schema())
    .with_def("ResultSheet", result_sheet_schema())
    .with_def("GraphSheet", graph_sheet_schema())
    .with_def("LayoutSheet", layout_sheet_schema())
}
