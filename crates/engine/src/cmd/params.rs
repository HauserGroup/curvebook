//! Draft 2020-12 JSON Schema builders and RFC-6901 validating decoder for command parameters.

use std::cell::RefCell;

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::{EngineError, ErrorCode};

const MAX_SCHEMA_DEPTH: usize = 32;

/// Retired parameter spelling detected during command parameter parsing or lookup,
/// paired with its canonical `snake_case` replacement.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeprecationHit {
    /// Retired parameter spelling supplied by the caller.
    pub deprecated: String,
    /// Canonical `snake_case` replacement key.
    pub canonical: String,
}

thread_local! {
    static DEPRECATION_SCOPES: RefCell<Vec<Vec<DeprecationHit>>> = const { RefCell::new(Vec::new()) };
}

/// RAII scope collecting retired parameter spellings observed while executing one command.
pub(crate) struct DeprecationScope {
    active: bool,
}

impl DeprecationScope {
    /// Push a new empty collection frame onto the thread-local deprecation stack.
    #[must_use]
    pub(crate) fn enter() -> Self {
        DEPRECATION_SCOPES.with(|stack| {
            stack.borrow_mut().push(Vec::new());
        });
        Self { active: true }
    }

    /// Pop the active collection frame and format deduplicated deprecation warnings for `command_id`.
    #[must_use]
    pub(crate) fn finish_warnings(mut self, command_id: &str) -> Vec<String> {
        self.active = false;
        let hits = DEPRECATION_SCOPES.with(|stack| stack.borrow_mut().pop().unwrap_or_default());
        hits.into_iter().map(|hit| format_deprecation_warning(command_id, &hit)).collect()
    }
}

impl Drop for DeprecationScope {
    fn drop(&mut self) {
        if self.active {
            let _ = DEPRECATION_SCOPES.with(|stack| stack.borrow_mut().pop());
        }
    }
}

fn push_unique_hit(out: &mut Vec<DeprecationHit>, hit: DeprecationHit) {
    if hit.deprecated != hit.canonical && !out.iter().any(|existing| existing == &hit) {
        out.push(hit);
    }
}

fn push_hits_to_active_scope(hits: &[DeprecationHit]) {
    if hits.is_empty() {
        return;
    }
    DEPRECATION_SCOPES.with(|stack| {
        if let Some(frame) = stack.borrow_mut().last_mut() {
            for hit in hits {
                push_unique_hit(frame, hit.clone());
            }
        }
    });
}

/// Format a deterministic user-facing deprecation notice for a retired parameter spelling.
// Retained until Stage 4 retires r16_5_cli_and_callers.rs
#[must_use]
pub fn format_deprecation_warning(command_id: &str, hit: &DeprecationHit) -> String {
    format!("parameter `{}` is deprecated for `{command_id}`; use canonical `{}` instead", hit.deprecated, hit.canonical)
}

/// Schema for commands taking no parameters (an empty object).
#[must_use]
pub fn empty_object_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

/// Generate a self-contained Draft 2020-12 command parameter schema from `T`.
///
/// Inlines all local `$defs` references so command registry consumers
/// (`summarize_schema_properties`, docs generation, and parameter validators)
/// can inspect top-level `type`, `properties`, and `required` directly, ensures
/// closed object schemas carry `"additionalProperties": false`, and preserves
/// open maps/catch-all fields.
#[must_use]
pub fn command_schema<T: JsonSchema>() -> Value {
    let mut root = SchemaSettings::draft2020_12().into_generator().into_root_schema_for::<T>().to_value();
    postprocess_command_schema(&mut root);
    root
}

/// Generate a Draft 2020-12 command parameter schema from `T` with retired legacy
/// aliases documented via `x-deprecated-aliases`.
#[must_use]
pub fn command_schema_with_aliases<T: JsonSchema>(aliases: &[(&str, &[&str])]) -> Value {
    let mut schema = command_schema::<T>();
    attach_deprecated_aliases(&mut schema, aliases);
    schema
}

/// Marker type for `#[schemars(with = "NonEmptyStrSchema")]` (and inside `Vec<NonEmptyStrSchema>`)
/// to emit `{ "type": "string", "minLength": 1 }` without duplicating custom schema closures.
pub(crate) struct NonEmptyStrSchema;

impl JsonSchema for NonEmptyStrSchema {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("NonEmptyString")
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "minLength": 1
        })
    }
}

/// Attach `x-deprecated-aliases` metadata to an object schema.
pub fn attach_deprecated_aliases(schema: &mut Value, aliases: &[(&str, &[&str])]) {
    let mut alias_map = Map::new();
    for (canonical, list) in aliases {
        let items: Vec<Value> = list.iter().map(|a| Value::String((*a).to_owned())).collect();
        alias_map.insert((*canonical).to_owned(), Value::Array(items));
    }
    if let Value::Object(map) = schema
        && !alias_map.is_empty()
    {
        map.insert(String::from("x-deprecated-aliases"), Value::Object(alias_map));
    }
}

fn postprocess_command_schema(root: &mut Value) {
    let defs = root
        .as_object_mut()
        .and_then(|obj| {
            obj.remove("title");
            obj.remove("$defs").or_else(|| obj.remove("definitions"))
        })
        .and_then(|v| match v {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();

    let mut visiting = Vec::new();
    inline_and_close_schema(root, &defs, &mut visiting, 0, true);
}

fn schema_allows_object(obj: &Map<String, Value>) -> bool {
    match obj.get("type") {
        Some(Value::String(s)) => s == "object",
        Some(Value::Array(arr)) => arr.iter().any(|v| v.as_str() == Some("object")),
        _ => false,
    }
}

fn inline_and_close_schema(schema: &mut Value, defs: &Map<String, Value>, visiting: &mut Vec<String>, depth: usize, is_root: bool) {
    if depth >= MAX_SCHEMA_DEPTH {
        return;
    }
    let Some(obj) = schema.as_object_mut() else {
        return;
    };

    if !is_root {
        obj.remove("$schema");
        obj.remove("title");
    }

    if let Some(def_name) = obj.get("$ref").and_then(Value::as_str).and_then(|r| r.strip_prefix("#/$defs/")).map(str::to_owned)
        && !visiting.iter().any(|seen| seen == &def_name)
        && let Some(mut resolved) = defs.get(&def_name).cloned()
    {
        visiting.push(def_name);
        inline_and_close_schema(&mut resolved, defs, visiting, depth.saturating_add(1), false);
        visiting.pop();
        obj.remove("$ref");
        if let Value::Object(resolved_map) = resolved {
            for (k, v) in resolved_map {
                obj.entry(k).or_insert(v);
            }
        }
    }

    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(Value::Array(items)) = obj.get_mut(key) {
            for item in items.iter_mut() {
                inline_and_close_schema(item, defs, visiting, depth.saturating_add(1), false);
            }
        }
    }

    if let Some(Value::Object(props)) = obj.get_mut("properties") {
        for prop in props.values_mut() {
            inline_and_close_schema(prop, defs, visiting, depth.saturating_add(1), false);
        }
        if schema_allows_object(obj) && !obj.contains_key("additionalProperties") {
            obj.insert(String::from("additionalProperties"), Value::Bool(false));
        }
    } else if schema_allows_object(obj) && obj.get("additionalProperties") == Some(&Value::Bool(false)) {
        obj.insert(String::from("properties"), Value::Object(Map::new()));
    }

    for key in ["items", "additionalProperties"] {
        if let Some(child) = obj.get_mut(key) {
            inline_and_close_schema(child, defs, visiting, depth.saturating_add(1), false);
        }
    }
    if let Some(Value::Array(prefix)) = obj.get_mut("prefixItems") {
        for item in prefix.iter_mut() {
            inline_and_close_schema(item, defs, visiting, depth.saturating_add(1), false);
        }
    }
}

/// Build a Draft 2020-12 object schema from `(field_name, property_schema)` pairs.
#[must_use]
pub fn obj_schema(props: Vec<(&str, Value)>, required: &[&str], additional_properties: bool) -> Value {
    let mut prop_map = Map::new();
    for (name, schema) in props {
        prop_map.insert(name.to_owned(), schema);
    }
    let req_arr: Vec<Value> = required.iter().map(|r| Value::String((*r).to_owned())).collect();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": Value::Object(prop_map),
        "required": Value::Array(req_arr),
        "additionalProperties": additional_properties
    })
}

/// Build an object schema documenting retired legacy aliases via `x-deprecated-aliases`.
///
/// Each entry `(canonical_key, aliases)` allows any listed alias to populate
/// `canonical_key` during [`parse_params`] (while emitting a deprecation warning)
/// before validation and deserialization.
// Retained until Stage 4 retires r16_5_cli_and_callers.rs
#[must_use]
pub fn obj_schema_with_aliases(props: Vec<(&str, Value)>, required: &[&str], aliases: Vec<(&str, &[&str])>, additional_properties: bool) -> Value {
    let mut schema = obj_schema(props, required, additional_properties);
    attach_deprecated_aliases(&mut schema, &aliases);
    schema
}

/// Build a string property schema.
#[must_use]
pub fn str_schema(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

/// Build a non-empty string (`minLength: 1`) property schema.
#[must_use]
pub fn non_empty_str_schema(description: &str) -> Value {
    json!({ "type": "string", "minLength": 1, "description": description })
}

/// Build a string enum property schema.
#[must_use]
pub fn enum_str_schema(description: &str, variants: &[&str]) -> Value {
    let vals: Vec<Value> = variants.iter().map(|v| Value::String((*v).to_owned())).collect();
    json!({ "type": "string", "enum": Value::Array(vals), "description": description })
}

/// Build a boolean property schema.
#[must_use]
pub fn bool_schema(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
}

/// Build an integer property schema with optional inclusive bounds.
#[must_use]
pub fn int_schema(description: &str, min: Option<i64>, max: Option<i64>) -> Value {
    let mut map = Map::new();
    map.insert(String::from("type"), json!("integer"));
    map.insert(String::from("description"), json!(description));
    if let Some(v) = min {
        map.insert(String::from("minimum"), json!(v));
    }
    if let Some(v) = max {
        map.insert(String::from("maximum"), json!(v));
    }
    Value::Object(map)
}

/// Build a numeric property schema with optional inclusive finite bounds.
#[must_use]
pub fn num_schema(description: &str, min: Option<f64>, max: Option<f64>) -> Value {
    let mut map = Map::new();
    map.insert(String::from("type"), json!("number"));
    map.insert(String::from("description"), json!(description));
    if let Some(v) = min.filter(|x| x.is_finite()) {
        map.insert(String::from("minimum"), json!(v));
    }
    if let Some(v) = max.filter(|x| x.is_finite()) {
        map.insert(String::from("maximum"), json!(v));
    }
    Value::Object(map)
}

/// Build an array property schema with optional length bounds.
#[must_use]
pub fn array_schema(description: &str, items: Value, min_items: Option<usize>, max_items: Option<usize>) -> Value {
    let mut map = Map::new();
    map.insert(String::from("type"), json!("array"));
    map.insert(String::from("description"), json!(description));
    map.insert(String::from("items"), items);
    if let Some(v) = min_items {
        map.insert(String::from("minItems"), json!(v));
    }
    if let Some(v) = max_items {
        map.insert(String::from("maxItems"), json!(v));
    }
    Value::Object(map)
}

/// Allow `null` alongside `inner` using `anyOf`.
#[must_use]
pub fn nullable_schema(inner: Value) -> Value {
    json!({ "anyOf": [inner, { "type": "null" }] })
}

/// Build an unconstrained JSON value property schema with a description.
#[must_use]
pub fn any_json_schema(description: &str) -> Value {
    json!({ "description": description })
}

/// Convert a [`curvebook_doc::SchemaNode`] into a `serde_json::Value` schema document.
///
/// # Errors
/// Returns [`EngineError`] if serialization fails.
pub fn schema_node_to_value(node: &curvebook_doc::SchemaNode) -> Result<Value, EngineError> {
    serde_json::to_value(node).map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("failed to serialize schema node: {err}")))
}

/// Validate `raw` against a Draft 2020-12 JSON Schema, reporting RFC-6901 JSON-pointer errors,
/// and deserialize into `T` while rejecting undeclared fields on closed parameter structs and
/// returning any retired `x-deprecated-aliases` spellings supplied in `raw`.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] and `details: { "pointer": "..." }`
/// if validation or deserialization fails.
pub fn parse_params_with_warnings<T: DeserializeOwned>(schema: &Value, mut raw: Value) -> Result<(T, Vec<DeprecationHit>), EngineError> {
    let mut hits = Vec::new();
    validate_value_with_hits(schema, schema, &mut raw, "", 0, &mut hits)?;
    push_hits_to_active_scope(&hits);
    let de = StrictValueDeserializer { value: &raw, pointer: String::new(), field_name: None, allow_ignored_fields: is_open_catchall_schema(schema) };
    let parsed = T::deserialize(de).map_err(StrictDeError::into_engine_error)?;
    Ok((parsed, hits))
}

/// Validate `raw` against a Draft 2020-12 JSON Schema, reporting RFC-6901 JSON-pointer errors,
/// and deserialize into `T` while rejecting undeclared fields on closed parameter structs.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] and `details: { "pointer": "..." }`
/// if validation or deserialization fails.
pub fn parse_params<T: DeserializeOwned>(schema: &Value, raw: Value) -> Result<T, EngineError> {
    parse_params_with_warnings(schema, raw).map(|(parsed, _)| parsed)
}

/// Strictly deserialize a JSON [`Value`] into `T`, rejecting unknown struct fields at any nesting depth.
pub(super) fn deserialize_strict<T: DeserializeOwned>(value: &Value, pointer: &str) -> Result<T, EngineError> {
    let de = StrictValueDeserializer { value, pointer: pointer.to_owned(), field_name: None, allow_ignored_fields: false };
    T::deserialize(de).map_err(StrictDeError::into_engine_error)
}

fn is_open_catchall_schema(schema: &Value) -> bool {
    let Some(obj) = schema.as_object() else {
        return false;
    };
    let props_empty = obj.get("properties").and_then(Value::as_object).is_some_and(Map::is_empty);
    let additional_true = obj.get("additionalProperties") == Some(&Value::Bool(true));
    props_empty && additional_true
}

#[derive(Debug)]
struct StrictDeError {
    pointer: String,
    message: String,
    is_unknown_field: bool,
}

impl StrictDeError {
    fn from_serde(pointer: String, err: impl std::fmt::Display) -> Self {
        Self { pointer, message: err.to_string(), is_unknown_field: false }
    }

    fn unknown_field(pointer: String, field_name: Option<String>) -> Self {
        let message = match field_name {
            Some(name) => format!("unknown field `{name}`"),
            None => String::from("unexpected extra parameter"),
        };
        Self { pointer, message, is_unknown_field: true }
    }

    fn with_fallback_pointer(mut self, fallback: &str) -> Self {
        if self.pointer.is_empty() && !fallback.is_empty() {
            self.pointer = fallback.to_owned();
        }
        self
    }

    fn into_engine_error(self) -> EngineError {
        if self.is_unknown_field {
            pointer_error(&self.pointer, self.message)
        } else {
            pointer_error(&self.pointer, format!("invalid command parameters: {}", self.message))
        }
    }
}

impl std::fmt::Display for StrictDeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StrictDeError {}

impl serde::de::Error for StrictDeError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        Self { pointer: String::new(), message: msg.to_string(), is_unknown_field: false }
    }
}

macro_rules! forward_strict_scalar {
    ($($method:ident),* $(,)?) => {
        $(
            fn $method<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'de>,
            {
                let ptr = self.pointer;
                serde::Deserializer::$method(self.value, visitor).map_err(|err| StrictDeError::from_serde(ptr, err))
            }
        )*
    };
}

struct StrictValueDeserializer<'de> {
    value: &'de Value,
    pointer: String,
    field_name: Option<String>,
    allow_ignored_fields: bool,
}

impl<'de> StrictValueDeserializer<'de> {
    fn visit_array<V>(self, arr: &'de [Value], visitor: V) -> Result<V::Value, StrictDeError>
    where
        V: serde::de::Visitor<'de>,
    {
        let ptr = self.pointer;
        let seq = StrictSeqAccess { iter: arr.iter().enumerate(), base_pointer: ptr.clone(), allow_ignored_fields: self.allow_ignored_fields };
        visitor.visit_seq(seq).map_err(|err| err.with_fallback_pointer(&ptr))
    }

    fn visit_object<V>(self, map: &'de Map<String, Value>, visitor: V) -> Result<V::Value, StrictDeError>
    where
        V: serde::de::Visitor<'de>,
    {
        let ptr = self.pointer;
        let access = StrictMapAccess { iter: map.iter(), pending: None, base_pointer: ptr.clone(), allow_ignored_fields: self.allow_ignored_fields };
        visitor.visit_map(access).map_err(|err| err.with_fallback_pointer(&ptr))
    }
}

impl<'de> serde::Deserializer<'de> for StrictValueDeserializer<'de> {
    type Error = StrictDeError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.value {
            Value::Array(arr) => self.visit_array(arr, visitor),
            Value::Object(map) => self.visit_object(map, visitor),
            _ => {
                let ptr = self.pointer;
                serde::Deserializer::deserialize_any(self.value, visitor).map_err(|err| StrictDeError::from_serde(ptr, err))
            }
        }
    }

    forward_strict_scalar!(
        deserialize_bool,
        deserialize_i8,
        deserialize_i16,
        deserialize_i32,
        deserialize_i64,
        deserialize_i128,
        deserialize_u8,
        deserialize_u16,
        deserialize_u32,
        deserialize_u64,
        deserialize_u128,
        deserialize_f32,
        deserialize_f64,
        deserialize_char,
        deserialize_str,
        deserialize_string,
        deserialize_bytes,
        deserialize_byte_buf,
        deserialize_unit,
        deserialize_identifier,
    );

    fn deserialize_unit_struct<V>(self, name: &'static str, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let ptr = self.pointer;
        serde::Deserializer::deserialize_unit_struct(self.value, name, visitor).map_err(|err| StrictDeError::from_serde(ptr, err))
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        if self.value.is_null() {
            let ptr = self.pointer;
            visitor.visit_none().map_err(|err: StrictDeError| err.with_fallback_pointer(&ptr))
        } else {
            let ptr = self.pointer.clone();
            visitor.visit_some(self).map_err(|err| err.with_fallback_pointer(&ptr))
        }
    }

    fn deserialize_newtype_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        let ptr = self.pointer.clone();
        visitor.visit_newtype_struct(self).map_err(|err| err.with_fallback_pointer(&ptr))
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.value {
            Value::Array(arr) => self.visit_array(arr, visitor),
            _ => {
                let ptr = self.pointer;
                serde::Deserializer::deserialize_seq(self.value, visitor).map_err(|err| StrictDeError::from_serde(ptr, err))
            }
        }
    }

    fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.value {
            Value::Array(arr) => self.visit_array(arr, visitor),
            _ => {
                let ptr = self.pointer;
                serde::Deserializer::deserialize_tuple(self.value, len, visitor).map_err(|err| StrictDeError::from_serde(ptr, err))
            }
        }
    }

    fn deserialize_tuple_struct<V>(self, name: &'static str, len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.value {
            Value::Array(arr) => self.visit_array(arr, visitor),
            _ => {
                let ptr = self.pointer;
                serde::Deserializer::deserialize_tuple_struct(self.value, name, len, visitor).map_err(|err| StrictDeError::from_serde(ptr, err))
            }
        }
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.value {
            Value::Object(map) => self.visit_object(map, visitor),
            _ => {
                let ptr = self.pointer;
                serde::Deserializer::deserialize_map(self.value, visitor).map_err(|err| StrictDeError::from_serde(ptr, err))
            }
        }
    }

    fn deserialize_struct<V>(self, name: &'static str, fields: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.value {
            Value::Array(arr) => self.visit_array(arr, visitor),
            Value::Object(map) => self.visit_object(map, visitor),
            _ => {
                let ptr = self.pointer;
                serde::Deserializer::deserialize_struct(self.value, name, fields, visitor).map_err(|err| StrictDeError::from_serde(ptr, err))
            }
        }
    }

    fn deserialize_enum<V>(self, name: &'static str, variants: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        match self.value {
            Value::String(variant) => {
                let ptr = self.pointer;
                visitor
                    .visit_enum(serde::de::value::BorrowedStrDeserializer::<StrictDeError>::new(variant.as_str()))
                    .map_err(|err| err.with_fallback_pointer(&ptr))
            }
            Value::Object(map) if map.len() == 1 => {
                if let Some((variant, payload)) = map.iter().next() {
                    let variant_ptr = append_pointer(&self.pointer, variant);
                    let ptr = self.pointer;
                    return visitor
                        .visit_enum(StrictEnumAccess {
                            variant: variant.as_str(),
                            payload,
                            pointer: variant_ptr,
                            allow_ignored_fields: self.allow_ignored_fields,
                        })
                        .map_err(|err| err.with_fallback_pointer(&ptr));
                }
                let ptr = self.pointer;
                serde::Deserializer::deserialize_enum(self.value, name, variants, visitor).map_err(|err| StrictDeError::from_serde(ptr, err))
            }
            _ => {
                let ptr = self.pointer;
                serde::Deserializer::deserialize_enum(self.value, name, variants, visitor).map_err(|err| StrictDeError::from_serde(ptr, err))
            }
        }
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        if self.allow_ignored_fields {
            let ptr = self.pointer;
            serde::Deserializer::deserialize_ignored_any(self.value, visitor).map_err(|err| StrictDeError::from_serde(ptr, err))
        } else {
            Err(StrictDeError::unknown_field(self.pointer, self.field_name))
        }
    }
}

struct StrictSeqAccess<I> {
    iter: I,
    base_pointer: String,
    allow_ignored_fields: bool,
}

impl<'de, I> serde::de::SeqAccess<'de> for StrictSeqAccess<I>
where
    I: Iterator<Item = (usize, &'de Value)>,
{
    type Error = StrictDeError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: serde::de::DeserializeSeed<'de>,
    {
        let Some((idx, item)) = self.iter.next() else {
            return Ok(None);
        };
        let child_ptr = append_pointer(&self.base_pointer, &idx.to_string());
        let de =
            StrictValueDeserializer { value: item, pointer: child_ptr.clone(), field_name: None, allow_ignored_fields: self.allow_ignored_fields };
        seed.deserialize(de).map(Some).map_err(|err| err.with_fallback_pointer(&child_ptr))
    }
}

struct StrictMapAccess<'de, I> {
    iter: I,
    pending: Option<(&'de String, &'de Value)>,
    base_pointer: String,
    allow_ignored_fields: bool,
}

impl<'de, I> serde::de::MapAccess<'de> for StrictMapAccess<'de, I>
where
    I: Iterator<Item = (&'de String, &'de Value)>,
{
    type Error = StrictDeError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: serde::de::DeserializeSeed<'de>,
    {
        let Some((key, val)) = self.iter.next() else {
            return Ok(None);
        };
        self.pending = Some((key, val));
        let key_ptr = append_pointer(&self.base_pointer, key);
        let key_de = StrictMapKeyDeserializer { key: key.as_str(), pointer: &key_ptr };
        seed.deserialize(key_de).map(Some).map_err(|err| err.with_fallback_pointer(&key_ptr))
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::DeserializeSeed<'de>,
    {
        let Some((key, val)) = self.pending.take() else {
            return Err(StrictDeError::from_serde(self.base_pointer.clone(), "map value requested before key"));
        };
        let child_ptr = append_pointer(&self.base_pointer, key);
        let de = StrictValueDeserializer {
            value: val,
            pointer: child_ptr.clone(),
            field_name: Some(key.clone()),
            allow_ignored_fields: self.allow_ignored_fields,
        };
        seed.deserialize(de).map_err(|err| err.with_fallback_pointer(&child_ptr))
    }
}

#[derive(Clone, Copy)]
struct StrictMapKeyDeserializer<'de, 'p> {
    key: &'de str,
    pointer: &'p str,
}

impl<'de> serde::Deserializer<'de> for StrictMapKeyDeserializer<'de, '_> {
    type Error = StrictDeError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.key).map_err(|err: StrictDeError| err.with_fallback_pointer(self.pointer))
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_some(self).map_err(|err| err.with_fallback_pointer(self.pointer))
    }

    fn deserialize_newtype_struct<V>(self, _name: &'static str, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor.visit_newtype_struct(self).map_err(|err| err.with_fallback_pointer(self.pointer))
    }

    fn deserialize_enum<V>(self, _name: &'static str, _variants: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        visitor
            .visit_enum(serde::de::value::BorrowedStrDeserializer::<StrictDeError>::new(self.key))
            .map_err(|err| err.with_fallback_pointer(self.pointer))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct seq tuple tuple_struct map struct
        identifier ignored_any
    }
}

struct StrictEnumAccess<'de> {
    variant: &'de str,
    payload: &'de Value,
    pointer: String,
    allow_ignored_fields: bool,
}

impl<'de> StrictEnumAccess<'de> {
    fn into_deserializer(self) -> StrictValueDeserializer<'de> {
        StrictValueDeserializer { value: self.payload, pointer: self.pointer, field_name: None, allow_ignored_fields: self.allow_ignored_fields }
    }
}

impl<'de> serde::de::EnumAccess<'de> for StrictEnumAccess<'de> {
    type Error = StrictDeError;
    type Variant = Self;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
    where
        V: serde::de::DeserializeSeed<'de>,
    {
        let variant_de = serde::de::value::BorrowedStrDeserializer::<StrictDeError>::new(self.variant);
        let val = seed.deserialize(variant_de).map_err(|err| err.with_fallback_pointer(&self.pointer))?;
        Ok((val, self))
    }
}

impl<'de> serde::de::VariantAccess<'de> for StrictEnumAccess<'de> {
    type Error = StrictDeError;

    fn unit_variant(self) -> Result<(), Self::Error> {
        if self.payload.is_null() {
            Ok(())
        } else {
            let ptr = self.pointer;
            serde::Deserialize::deserialize(self.payload).map_err(|err| StrictDeError::from_serde(ptr, err))
        }
    }

    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, Self::Error>
    where
        T: serde::de::DeserializeSeed<'de>,
    {
        let ptr = self.pointer.clone();
        seed.deserialize(self.into_deserializer()).map_err(|err| err.with_fallback_pointer(&ptr))
    }

    fn tuple_variant<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        serde::Deserializer::deserialize_tuple(self.into_deserializer(), len, visitor)
    }

    fn struct_variant<V>(self, fields: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error>
    where
        V: serde::de::Visitor<'de>,
    {
        serde::Deserializer::deserialize_struct(self.into_deserializer(), "", fields, visitor)
    }
}

fn pointer_error(pointer: &str, message: impl Into<String>) -> EngineError {
    EngineError::with_code(ErrorCode::BadInput, message).with_details(json!({ "pointer": pointer }))
}

fn append_pointer(base: &str, token: &str) -> String {
    let escaped = token.replace('~', "~0").replace('/', "~1");
    format!("{base}/{escaped}")
}

fn json_type_name(val: &Value) -> &'static str {
    match val {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn value_matches_type(val: &Value, expected: &str) -> bool {
    match expected {
        "null" => val.is_null(),
        "boolean" => val.is_boolean(),
        "string" => val.is_string(),
        "array" => val.is_array(),
        "object" => val.is_object(),
        "number" => val.is_number(),
        "integer" => match val {
            Value::Number(n) => n.is_i64() || n.is_u64() || n.as_f64().is_some_and(|f| f.is_finite() && f.fract() == 0.0),
            _ => false,
        },
        _ => true,
    }
}

fn resolve_ref<'a>(root: &'a Value, ref_str: &str, pointer: &str) -> Result<&'a Value, EngineError> {
    if ref_str == "#" {
        return Ok(root);
    }
    let Some(frag) = ref_str.strip_prefix("#/") else {
        return Err(pointer_error(pointer, format!("unsupported JSON schema $ref `{ref_str}`")));
    };
    let mut cur = root;
    for raw_seg in frag.split('/') {
        let seg = raw_seg.replace("~1", "/").replace("~0", "~");
        cur = match cur {
            Value::Object(map) => map.get(&seg),
            Value::Array(arr) => seg.parse::<usize>().ok().and_then(|idx| arr.get(idx)),
            _ => None,
        }
        .ok_or_else(|| pointer_error(pointer, format!("unresolved JSON schema $ref `{ref_str}`")))?;
    }
    Ok(cur)
}

fn validate_value_with_hits(
    root: &Value,
    schema: &Value,
    val: &mut Value,
    pointer: &str,
    depth: usize,
    hits: &mut Vec<DeprecationHit>,
) -> Result<(), EngineError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(pointer_error(pointer, "parameter validation exceeded schema depth limit"));
    }
    match schema {
        Value::Bool(true) => Ok(()),
        Value::Bool(false) => Err(pointer_error(pointer, "parameter rejected by schema")),
        Value::Object(schema_obj) => validate_schema_object(root, schema_obj, val, pointer, depth, hits),
        _ => Ok(()),
    }
}

fn validate_schema_object(
    root: &Value,
    schema_obj: &Map<String, Value>,
    val: &mut Value,
    pointer: &str,
    depth: usize,
    hits: &mut Vec<DeprecationHit>,
) -> Result<(), EngineError> {
    if let Some(Value::String(ref_str)) = schema_obj.get("$ref") {
        let target = resolve_ref(root, ref_str, pointer)?;
        validate_value_with_hits(root, target, val, pointer, depth.saturating_add(1), hits)?;
    }
    if let Some(Value::Array(all_of)) = schema_obj.get("allOf") {
        for sub in all_of {
            validate_value_with_hits(root, sub, val, pointer, depth.saturating_add(1), hits)?;
        }
    }
    if let Some(Value::Array(any_of)) = schema_obj.get("anyOf") {
        validate_any_of(root, any_of, val, pointer, depth, hits)?;
    }
    if let Some(Value::Array(one_of)) = schema_obj.get("oneOf") {
        validate_one_of(root, one_of, val, pointer, depth, hits)?;
    }
    if let Some(expected_type) = schema_obj.get("type") {
        validate_type_constraint(expected_type, val, pointer)?;
    }
    validate_enum_and_const(schema_obj, val, pointer)?;
    validate_typed_keywords(root, schema_obj, val, pointer, depth, hits)
}

fn validate_any_of(
    root: &Value,
    variants: &[Value],
    val: &mut Value,
    pointer: &str,
    depth: usize,
    hits: &mut Vec<DeprecationHit>,
) -> Result<(), EngineError> {
    let mut deepest: Option<(usize, EngineError)> = None;
    for sub in variants {
        let mut candidate = val.clone();
        let mut branch_hits = Vec::new();
        match validate_value_with_hits(root, sub, &mut candidate, pointer, depth.saturating_add(1), &mut branch_hits) {
            Ok(()) => {
                *val = candidate;
                for hit in branch_hits {
                    push_unique_hit(hits, hit);
                }
                return Ok(());
            }
            Err(err) => {
                let err_depth = pointer_depth(&err);
                if deepest.as_ref().is_none_or(|(best, _)| err_depth > *best) {
                    deepest = Some((err_depth, err));
                }
            }
        }
    }
    Err(deepest.map_or_else(|| pointer_error(pointer, "value did not match anyOf schema variants"), |(_, err)| err))
}

fn validate_one_of(
    root: &Value,
    variants: &[Value],
    val: &mut Value,
    pointer: &str,
    depth: usize,
    hits: &mut Vec<DeprecationHit>,
) -> Result<(), EngineError> {
    let mut matched: Option<(Value, Vec<DeprecationHit>)> = None;
    let mut match_count = 0usize;
    let mut deepest: Option<(usize, EngineError)> = None;
    for sub in variants {
        let mut candidate = val.clone();
        let mut branch_hits = Vec::new();
        match validate_value_with_hits(root, sub, &mut candidate, pointer, depth.saturating_add(1), &mut branch_hits) {
            Ok(()) => {
                match_count = match_count.saturating_add(1);
                if matched.is_none() {
                    matched = Some((candidate, branch_hits));
                }
            }
            Err(err) => {
                let err_depth = pointer_depth(&err);
                if deepest.as_ref().is_none_or(|(best, _)| err_depth > *best) {
                    deepest = Some((err_depth, err));
                }
            }
        }
    }
    if match_count == 1 {
        if let Some((canon, branch_hits)) = matched {
            *val = canon;
            for hit in branch_hits {
                push_unique_hit(hits, hit);
            }
        }
        return Ok(());
    }
    if match_count > 1 {
        return Err(pointer_error(pointer, "value matched multiple oneOf schema variants"));
    }
    Err(deepest.map_or_else(|| pointer_error(pointer, "value did not match any oneOf schema variant"), |(_, err)| err))
}

fn pointer_depth(err: &EngineError) -> usize {
    err.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str).map_or(0, str::len)
}

fn validate_type_constraint(expected: &Value, val: &Value, pointer: &str) -> Result<(), EngineError> {
    match expected {
        Value::String(kind) => {
            if !value_matches_type(val, kind) {
                return Err(pointer_error(pointer, format!("expected {kind}, got {}", json_type_name(val))));
            }
        }
        Value::Array(kinds) => {
            let names: Vec<&str> = kinds.iter().filter_map(Value::as_str).collect();
            if !names.is_empty() && !names.iter().any(|k| value_matches_type(val, k)) {
                return Err(pointer_error(pointer, format!("expected one of [{}], got {}", names.join(", "), json_type_name(val))));
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_enum_and_const(schema_obj: &Map<String, Value>, val: &Value, pointer: &str) -> Result<(), EngineError> {
    if let Some(expected_const) = schema_obj.get("const")
        && val != expected_const
    {
        return Err(pointer_error(pointer, format!("value must equal {expected_const}")));
    }
    if let Some(Value::Array(allowed)) = schema_obj.get("enum")
        && !allowed.iter().any(|item| item == val)
    {
        return Err(pointer_error(pointer, format!("value {val} is not one of allowed enum values")));
    }
    Ok(())
}

fn validate_typed_keywords(
    root: &Value,
    schema_obj: &Map<String, Value>,
    val: &mut Value,
    pointer: &str,
    depth: usize,
    hits: &mut Vec<DeprecationHit>,
) -> Result<(), EngineError> {
    match val {
        Value::String(text) => validate_string_keywords(schema_obj, text, pointer),
        Value::Number(num) => validate_number_keywords(schema_obj, num, pointer),
        Value::Array(arr) => validate_array_keywords(root, schema_obj, arr, pointer, depth, hits),
        Value::Object(obj) => validate_object_keywords(root, schema_obj, obj, pointer, depth, hits),
        Value::Null | Value::Bool(_) => Ok(()),
    }
}

fn validate_string_keywords(schema_obj: &Map<String, Value>, text: &str, pointer: &str) -> Result<(), EngineError> {
    let char_len = text.chars().count();
    if let Some(min_len) = schema_obj.get("minLength").and_then(Value::as_u64)
        && (char_len as u64) < min_len
    {
        return Err(pointer_error(pointer, format!("string length {char_len} is shorter than minLength {min_len}")));
    }
    if let Some(max_len) = schema_obj.get("maxLength").and_then(Value::as_u64)
        && (char_len as u64) > max_len
    {
        return Err(pointer_error(pointer, format!("string length {char_len} exceeds maxLength {max_len}")));
    }
    Ok(())
}

fn validate_number_keywords(schema_obj: &Map<String, Value>, num: &serde_json::Number, pointer: &str) -> Result<(), EngineError> {
    let Some(v) = num.as_f64() else {
        return Err(pointer_error(pointer, "non-finite numeric parameter"));
    };
    if let Some(min) = schema_obj.get("minimum").and_then(Value::as_f64)
        && v < min
    {
        return Err(pointer_error(pointer, format!("value {v} is below minimum {min}")));
    }
    if let Some(ex_min) = schema_obj.get("exclusiveMinimum").and_then(Value::as_f64)
        && v <= ex_min
    {
        return Err(pointer_error(pointer, format!("value {v} must be > {ex_min}")));
    }
    if let Some(max) = schema_obj.get("maximum").and_then(Value::as_f64)
        && v > max
    {
        return Err(pointer_error(pointer, format!("value {v} exceeds maximum {max}")));
    }
    if let Some(ex_max) = schema_obj.get("exclusiveMaximum").and_then(Value::as_f64)
        && v >= ex_max
    {
        return Err(pointer_error(pointer, format!("value {v} must be < {ex_max}")));
    }
    Ok(())
}

fn validate_array_keywords(
    root: &Value,
    schema_obj: &Map<String, Value>,
    arr: &mut [Value],
    pointer: &str,
    depth: usize,
    hits: &mut Vec<DeprecationHit>,
) -> Result<(), EngineError> {
    if let Some(min_items) = schema_obj.get("minItems").and_then(Value::as_u64)
        && (arr.len() as u64) < min_items
    {
        return Err(pointer_error(pointer, format!("array length {} is less than minItems {min_items}", arr.len())));
    }
    if let Some(max_items) = schema_obj.get("maxItems").and_then(Value::as_u64)
        && (arr.len() as u64) > max_items
    {
        return Err(pointer_error(pointer, format!("array length {} exceeds maxItems {max_items}", arr.len())));
    }
    let prefix = schema_obj.get("prefixItems").and_then(Value::as_array);
    let items_schema = schema_obj.get("items");
    for (idx, item) in arr.iter_mut().enumerate() {
        let item_ptr = append_pointer(pointer, &idx.to_string());
        if let Some(prefix_arr) = prefix
            && let Some(prefix_schema) = prefix_arr.get(idx)
        {
            validate_value_with_hits(root, prefix_schema, item, &item_ptr, depth.saturating_add(1), hits)?;
            continue;
        }
        if let Some(item_schema) = items_schema {
            validate_value_with_hits(root, item_schema, item, &item_ptr, depth.saturating_add(1), hits)?;
        }
    }
    Ok(())
}

// Retained until Stage 4 retires r16_5_cli_and_callers.rs
fn canonicalize_aliases(obj: &mut Map<String, Value>, schema_obj: &Map<String, Value>, hits: &mut Vec<DeprecationHit>) {
    let props = schema_obj.get("properties").and_then(Value::as_object);
    let Some(Value::Object(alias_map)) = schema_obj.get("x-deprecated-aliases") else {
        return;
    };
    for (canonical, aliases_val) in alias_map {
        let Some(alias_list) = aliases_val.as_array() else {
            continue;
        };
        for alias in alias_list.iter().filter_map(Value::as_str) {
            let is_declared_prop = props.is_some_and(|p| p.contains_key(alias));
            if alias != canonical && !is_declared_prop && obj.contains_key(alias) {
                push_unique_hit(hits, DeprecationHit { deprecated: alias.to_owned(), canonical: canonical.clone() });
            }
        }
        if !obj.contains_key(canonical) {
            for alias in alias_list.iter().filter_map(Value::as_str) {
                if let Some(moved) = obj.remove(alias) {
                    obj.insert(canonical.clone(), moved);
                    break;
                }
            }
        }
        for alias in alias_list.iter().filter_map(Value::as_str) {
            let is_declared_prop = props.is_some_and(|p| p.contains_key(alias));
            if alias != canonical && !is_declared_prop {
                obj.remove(alias);
            }
        }
    }
}

fn validate_object_keywords(
    root: &Value,
    schema_obj: &Map<String, Value>,
    obj: &mut Map<String, Value>,
    pointer: &str,
    depth: usize,
    hits: &mut Vec<DeprecationHit>,
) -> Result<(), EngineError> {
    canonicalize_aliases(obj, schema_obj, hits);
    if let Some(Value::Array(required)) = schema_obj.get("required") {
        for req_key in required.iter().filter_map(Value::as_str) {
            if !obj.contains_key(req_key) {
                let req_ptr = append_pointer(pointer, req_key);
                return Err(pointer_error(&req_ptr, format!("missing required field `{req_key}`")));
            }
        }
    }
    let props = schema_obj.get("properties").and_then(Value::as_object);
    let additional = schema_obj.get("additionalProperties");
    for (key, child_val) in obj.iter_mut() {
        let child_ptr = append_pointer(pointer, key);
        if let Some(prop_schema) = props.and_then(|p| p.get(key)) {
            validate_value_with_hits(root, prop_schema, child_val, &child_ptr, depth.saturating_add(1), hits)?;
        } else if let Some(extra_rule) = additional {
            match extra_rule {
                Value::Bool(false) => {
                    return Err(pointer_error(&child_ptr, format!("unknown field `{key}`")));
                }
                Value::Bool(true) => {}
                extra_schema => {
                    validate_value_with_hits(root, extra_schema, child_val, &child_ptr, depth.saturating_add(1), hits)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct CellUpdate {
        row: usize,
        col: usize,
        value: String,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct SetCellsParams {
        sheet_id: String,
        cells: Vec<CellUpdate>,
    }

    fn set_cells_schema() -> Value {
        let cell_schema = obj_schema(
            vec![
                ("row", int_schema("Zero-based row index", Some(0), None)),
                ("col", int_schema("Zero-based column index", Some(0), None)),
                ("value", str_schema("Cell text or numeric string")),
            ],
            &["row", "col", "value"],
            false,
        );
        obj_schema_with_aliases(
            vec![("sheet_id", non_empty_str_schema("Target sheet id")), ("cells", array_schema("Cells to update", cell_schema, Some(1), None))],
            &["sheet_id", "cells"],
            vec![("sheet_id", &["sheetId", "id"])],
            false,
        )
    }

    #[test]
    fn reports_exact_json_pointer_for_missing_required_and_nested_array_errors() {
        let schema = set_cells_schema();

        let missing_sheet = parse_params::<SetCellsParams>(&schema, json!({ "cells": [] })).expect_err("missing sheet_id should fail");
        assert_eq!(missing_sheet.code, ErrorCode::BadInput);
        assert_eq!(missing_sheet.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str), Some("/sheet_id"));

        let bad_nested = parse_params::<SetCellsParams>(
            &schema,
            json!({
                "sheet_id": "data_1",
                "cells": [
                    { "row": "bad", "col": 0, "value": "1.2" }
                ]
            }),
        )
        .expect_err("bad nested integer should fail");
        assert_eq!(bad_nested.code, ErrorCode::BadInput);
        assert_eq!(bad_nested.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str), Some("/cells/0/row"));
    }

    #[test]
    fn resolves_aliases_and_preserves_canonical_validation() {
        let schema = set_cells_schema();
        assert!(schema.get("x-aliases").is_none(), "retired x-aliases key must not be emitted");
        assert!(schema.get("x-deprecated-aliases").is_some(), "schema must emit x-deprecated-aliases");

        let (canonical_parsed, canonical_hits) = parse_params_with_warnings::<SetCellsParams>(
            &schema,
            json!({
                "sheet_id": "data_1",
                "cells": [{ "row": 0, "col": 1, "value": "42" }]
            }),
        )
        .expect("canonical input should decode cleanly");
        assert!(canonical_hits.is_empty(), "canonical params must emit zero deprecation hits");

        let (parsed, hits) = parse_params_with_warnings::<SetCellsParams>(
            &schema,
            json!({
                "sheetId": "data_1",
                "cells": [{ "row": 0, "col": 1, "value": "42" }]
            }),
        )
        .expect("alias should decode cleanly");
        assert_eq!(parsed, canonical_parsed);
        assert_eq!(
            parsed,
            SetCellsParams { sheet_id: String::from("data_1"), cells: vec![CellUpdate { row: 0, col: 1, value: String::from("42") }] }
        );
        assert_eq!(hits, vec![DeprecationHit { deprecated: String::from("sheetId"), canonical: String::from("sheet_id") }]);

        let bad_alias = parse_params::<SetCellsParams>(
            &schema,
            json!({
                "sheetId": "",
                "cells": [{ "row": 0, "col": 0, "value": "x" }]
            }),
        )
        .expect_err("empty aliased sheetId must still trigger minLength");
        assert_eq!(bad_alias.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str), Some("/sheet_id"));

        // Rejected anyOf variants must not leak deprecation hits.
        let variant_a = obj_schema_with_aliases(
            vec![("kind", json!({ "const": "a" })), ("target", str_schema("Target A"))],
            &["kind", "target"],
            vec![("target", &["legacy_a"])],
            false,
        );
        let variant_b = obj_schema_with_aliases(
            vec![("kind", json!({ "const": "b" })), ("target", str_schema("Target B"))],
            &["kind", "target"],
            vec![("target", &["legacy_b"])],
            false,
        );
        let any_schema = json!({ "anyOf": [variant_a, variant_b] });
        let (_v, variant_hits) =
            parse_params_with_warnings::<Value>(&any_schema, json!({ "kind": "b", "legacy_b": "ok" })).expect("second anyOf variant should validate");
        assert_eq!(variant_hits, vec![DeprecationHit { deprecated: String::from("legacy_b"), canonical: String::from("target") }]);
    }

    #[test]
    fn escapes_rfc6901_special_characters_and_handles_refs() {
        let schema = json!({
            "$defs": {
                "positiveInt": { "type": "integer", "minimum": 1 }
            },
            "type": "object",
            "properties": {
                "a/b~c": { "$ref": "#/$defs/positiveInt" }
            },
            "required": ["a/b~c"]
        });
        let err = parse_params::<serde_json::Value>(&schema, json!({ "a/b~c": 0 })).expect_err("zero violates minimum 1");
        assert_eq!(err.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str), Some("/a~1b~0c"));
    }

    #[test]
    fn rejects_unknown_fields_on_closed_structs_even_if_schema_was_permissive() {
        #[derive(Debug, Deserialize, PartialEq)]
        struct LooseSchemaClosedStruct {
            #[serde(alias = "label")]
            title: Option<String>,
            nested: Option<CellUpdate>,
        }

        #[derive(Debug, Deserialize, PartialEq)]
        struct OpenPatchStruct {
            sheet_id: String,
            #[serde(flatten)]
            extra: Map<String, Value>,
        }

        #[derive(Debug, Deserialize, PartialEq)]
        struct EmptyCatchAllStruct {}

        let permissive_schema =
            obj_schema(vec![("title", nullable_schema(str_schema("Title"))), ("nested", nullable_schema(json!({ "type": "object" })))], &[], true);

        // Declared serde alias still works cleanly.
        let ok = parse_params::<LooseSchemaClosedStruct>(&permissive_schema, json!({ "label": "Dose" })).expect("serde alias accepted");
        assert_eq!(ok.title.as_deref(), Some("Dose"));

        // Unknown top-level field is rejected with exact RFC-6901 pointer.
        let top_err = parse_params::<LooseSchemaClosedStruct>(&permissive_schema, json!({ "title": "Dose", "titel": "Typo" }))
            .expect_err("unknown top-level field must fail");
        assert_eq!(top_err.code, ErrorCode::BadInput);
        assert!(top_err.message.contains("unknown field `titel`"), "got: {}", top_err.message);
        assert_eq!(top_err.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str), Some("/titel"));

        // Unknown nested struct field is rejected with exact nested RFC-6901 pointer.
        let nested_err =
            parse_params::<LooseSchemaClosedStruct>(&permissive_schema, json!({ "nested": { "row": 0, "col": 1, "value": "5", "bogus": true } }))
                .expect_err("unknown nested field must fail");
        assert_eq!(nested_err.code, ErrorCode::BadInput);
        assert_eq!(nested_err.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str), Some("/nested/bogus"));

        // Explicit `#[serde(flatten)] Map<String, Value>` patch structs continue to collect extra keys.
        let patch_schema = obj_schema(vec![("sheet_id", non_empty_str_schema("Sheet"))], &["sheet_id"], true);
        let patch = parse_params::<OpenPatchStruct>(&patch_schema, json!({ "sheet_id": "g-1", "min": 0.0 })).expect("patch struct collects extra");
        assert_eq!(patch.extra.get("min"), Some(&json!(0.0)));

        // Explicit empty-properties catch-all schema (`{"properties": {}, "additionalProperties": true}`) stays permissive.
        let catchall_schema = json!({ "type": "object", "properties": {}, "additionalProperties": true });
        parse_params::<EmptyCatchAllStruct>(&catchall_schema, json!({ "sheet_id": "res-1" })).expect("catch-all schema allowed");
    }

    #[test]
    fn command_schema_inlines_defs_enforces_closed_objects_and_supports_aliases() {
        #[derive(Debug, Deserialize, JsonSchema, PartialEq)]
        #[serde(rename_all = "snake_case")]
        enum Kind {
            Xy,
            Column,
        }

        #[derive(Debug, Deserialize, JsonSchema, PartialEq)]
        struct NestedCrop {
            #[schemars(range(min = 0.0))]
            left_pt: Option<f64>,
        }

        #[derive(Debug, Deserialize, JsonSchema, PartialEq)]
        #[serde(deny_unknown_fields)]
        struct SampleCmd {
            #[schemars(length(min = 1))]
            sheet_id: String,
            kind: Option<Kind>,
            #[schemars(range(min = 0))]
            row: Option<i64>,
            crop: Option<NestedCrop>,
        }

        #[derive(Debug, Deserialize, JsonSchema, PartialEq)]
        struct FlatTail {
            title: Option<String>,
        }

        #[derive(Debug, Deserialize, JsonSchema, PartialEq)]
        struct FlattenedCmd {
            #[schemars(length(min = 1))]
            sheet_id: String,
            #[serde(flatten)]
            tail: FlatTail,
        }

        #[derive(Debug, Deserialize, JsonSchema, PartialEq)]
        #[serde(deny_unknown_fields)]
        struct EmptyCmd {}

        let schema = command_schema_with_aliases::<SampleCmd>(&[("sheet_id", &["id"])]);
        assert!(schema.get("$defs").is_none(), "inlined schema must remove $defs: {schema}");
        assert_eq!(schema.get("additionalProperties"), Some(&json!(false)));
        assert!(schema.pointer("/properties/crop/anyOf/0/properties/left_pt").is_some(), "nested $ref must be inlined: {schema}");
        assert_eq!(schema.pointer("/properties/crop/anyOf/0/additionalProperties"), Some(&json!(false)));

        let ok = parse_params::<SampleCmd>(&schema, json!({ "id": "d1", "kind": null, "row": null, "crop": { "left_pt": 2.0 } }))
            .expect("valid null and alias");
        assert_eq!(ok.sheet_id, "d1");
        assert_eq!(ok.kind, None);
        assert_eq!(ok.row, None);

        let neg_row = parse_params::<SampleCmd>(&schema, json!({ "sheet_id": "d1", "row": -1 })).expect_err("negative row rejected");
        assert_eq!(neg_row.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str), Some("/row"));

        let flat_schema = command_schema::<FlattenedCmd>();
        assert_eq!(flat_schema.get("additionalProperties"), Some(&json!(false)), "flattened fixed schema must be closed: {flat_schema}");
        assert!(flat_schema.pointer("/properties/title").is_some(), "flattened field missing: {flat_schema}");

        #[derive(Debug, Deserialize, JsonSchema, PartialEq)]
        struct LocalOpenPatch {
            #[schemars(length(min = 1))]
            sheet_id: String,
            #[serde(flatten)]
            extra: Map<String, Value>,
        }

        let open_schema = command_schema::<LocalOpenPatch>();
        assert_ne!(open_schema.get("additionalProperties"), Some(&json!(false)), "open flattened Map schema must stay open: {open_schema}");

        let empty_schema = command_schema::<EmptyCmd>();
        assert_eq!(empty_schema.get("additionalProperties"), Some(&json!(false)));
        assert_eq!(empty_schema.get("properties"), Some(&json!({})));
    }
}
