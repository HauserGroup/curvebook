//! Draft 2020-12 JSON Schema builders and RFC-6901 validating decoder for command parameters.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::{EngineError, ErrorCode};

const MAX_SCHEMA_DEPTH: usize = 32;

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

/// Build an object schema documenting accepted legacy aliases via `x-aliases`.
///
/// Each entry `(canonical_key, aliases)` allows any listed alias to populate
/// `canonical_key` during [`parse_params`] before validation and deserialization.
#[must_use]
pub fn obj_schema_with_aliases(props: Vec<(&str, Value)>, required: &[&str], aliases: Vec<(&str, &[&str])>, additional_properties: bool) -> Value {
    let mut schema = obj_schema(props, required, additional_properties);
    let mut alias_map = Map::new();
    for (canonical, list) in aliases {
        let items: Vec<Value> = list.iter().map(|a| Value::String((*a).to_owned())).collect();
        alias_map.insert(canonical.to_owned(), Value::Array(items));
    }
    if let Value::Object(ref mut map) = schema
        && !alias_map.is_empty()
    {
        map.insert(String::from("x-aliases"), Value::Object(alias_map));
    }
    schema
}

/// Attach `x-aliases` mappings to an existing object schema value.
#[must_use]
pub fn with_aliases(mut schema: Value, aliases: Vec<(&str, &[&str])>) -> Value {
    let mut alias_map = Map::new();
    for (canonical, list) in aliases {
        let items: Vec<Value> = list.iter().map(|a| Value::String((*a).to_owned())).collect();
        alias_map.insert(canonical.to_owned(), Value::Array(items));
    }
    if let Value::Object(ref mut map) = schema
        && !alias_map.is_empty()
    {
        map.insert(String::from("x-aliases"), Value::Object(alias_map));
    }
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

/// Convert a [`lumencraft_doc::SchemaNode`] into a `serde_json::Value` schema document.
///
/// # Errors
/// Returns [`EngineError`] if serialization fails.
pub fn schema_node_to_value(node: &lumencraft_doc::SchemaNode) -> Result<Value, EngineError> {
    serde_json::to_value(node).map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("failed to serialize schema node: {err}")))
}

/// Validate `raw` against a Draft 2020-12 JSON Schema, reporting RFC-6901 JSON-pointer errors,
/// and deserialize into `T`.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] and `details: { "pointer": "..." }`
/// if validation or deserialization fails.
pub fn parse_params<T: DeserializeOwned>(schema: &Value, mut raw: Value) -> Result<T, EngineError> {
    validate_value(schema, schema, &mut raw, "", 0)?;
    serde_json::from_value::<T>(raw).map_err(|err| {
        EngineError::with_code(ErrorCode::BadInput, format!("invalid command parameters: {err}")).with_details(json!({ "pointer": "" }))
    })
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

fn validate_value(root: &Value, schema: &Value, val: &mut Value, pointer: &str, depth: usize) -> Result<(), EngineError> {
    if depth > MAX_SCHEMA_DEPTH {
        return Err(pointer_error(pointer, "parameter validation exceeded schema depth limit"));
    }
    match schema {
        Value::Bool(true) => Ok(()),
        Value::Bool(false) => Err(pointer_error(pointer, "parameter rejected by schema")),
        Value::Object(schema_obj) => validate_schema_object(root, schema_obj, val, pointer, depth),
        _ => Ok(()),
    }
}

fn validate_schema_object(root: &Value, schema_obj: &Map<String, Value>, val: &mut Value, pointer: &str, depth: usize) -> Result<(), EngineError> {
    if let Some(Value::String(ref_str)) = schema_obj.get("$ref") {
        let target = resolve_ref(root, ref_str, pointer)?;
        validate_value(root, target, val, pointer, depth.saturating_add(1))?;
    }
    if let Some(Value::Array(all_of)) = schema_obj.get("allOf") {
        for sub in all_of {
            validate_value(root, sub, val, pointer, depth.saturating_add(1))?;
        }
    }
    if let Some(Value::Array(any_of)) = schema_obj.get("anyOf") {
        validate_any_of(root, any_of, val, pointer, depth)?;
    }
    if let Some(Value::Array(one_of)) = schema_obj.get("oneOf") {
        validate_one_of(root, one_of, val, pointer, depth)?;
    }
    if let Some(expected_type) = schema_obj.get("type") {
        validate_type_constraint(expected_type, val, pointer)?;
    }
    validate_enum_and_const(schema_obj, val, pointer)?;
    validate_typed_keywords(root, schema_obj, val, pointer, depth)
}

fn validate_any_of(root: &Value, variants: &[Value], val: &mut Value, pointer: &str, depth: usize) -> Result<(), EngineError> {
    let mut deepest: Option<(usize, EngineError)> = None;
    for sub in variants {
        let mut candidate = val.clone();
        match validate_value(root, sub, &mut candidate, pointer, depth.saturating_add(1)) {
            Ok(()) => {
                *val = candidate;
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

fn validate_one_of(root: &Value, variants: &[Value], val: &mut Value, pointer: &str, depth: usize) -> Result<(), EngineError> {
    let mut matched: Option<Value> = None;
    let mut match_count = 0usize;
    let mut deepest: Option<(usize, EngineError)> = None;
    for sub in variants {
        let mut candidate = val.clone();
        match validate_value(root, sub, &mut candidate, pointer, depth.saturating_add(1)) {
            Ok(()) => {
                match_count = match_count.saturating_add(1);
                if matched.is_none() {
                    matched = Some(candidate);
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
        if let Some(canon) = matched {
            *val = canon;
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

fn validate_typed_keywords(root: &Value, schema_obj: &Map<String, Value>, val: &mut Value, pointer: &str, depth: usize) -> Result<(), EngineError> {
    match val {
        Value::String(text) => validate_string_keywords(schema_obj, text, pointer),
        Value::Number(num) => validate_number_keywords(schema_obj, num, pointer),
        Value::Array(arr) => validate_array_keywords(root, schema_obj, arr, pointer, depth),
        Value::Object(obj) => validate_object_keywords(root, schema_obj, obj, pointer, depth),
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

fn validate_array_keywords(root: &Value, schema_obj: &Map<String, Value>, arr: &mut [Value], pointer: &str, depth: usize) -> Result<(), EngineError> {
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
            validate_value(root, prefix_schema, item, &item_ptr, depth.saturating_add(1))?;
            continue;
        }
        if let Some(item_schema) = items_schema {
            validate_value(root, item_schema, item, &item_ptr, depth.saturating_add(1))?;
        }
    }
    Ok(())
}

fn canonicalize_aliases(obj: &mut Map<String, Value>, schema_obj: &Map<String, Value>) {
    let Some(Value::Object(alias_map)) = schema_obj.get("x-aliases") else {
        return;
    };
    let props = schema_obj.get("properties").and_then(Value::as_object);
    for (canonical, aliases_val) in alias_map {
        let Some(alias_list) = aliases_val.as_array() else {
            continue;
        };
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
) -> Result<(), EngineError> {
    canonicalize_aliases(obj, schema_obj);
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
            validate_value(root, prop_schema, child_val, &child_ptr, depth.saturating_add(1))?;
        } else if let Some(extra_rule) = additional {
            match extra_rule {
                Value::Bool(false) => {
                    return Err(pointer_error(&child_ptr, format!("unknown field `{key}`")));
                }
                Value::Bool(true) => {}
                extra_schema => {
                    validate_value(root, extra_schema, child_val, &child_ptr, depth.saturating_add(1))?;
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
        let parsed = parse_params::<SetCellsParams>(
            &schema,
            json!({
                "sheetId": "data_1",
                "cells": [{ "row": 0, "col": 1, "value": "42" }]
            }),
        )
        .expect("alias should decode cleanly");
        assert_eq!(
            parsed,
            SetCellsParams { sheet_id: String::from("data_1"), cells: vec![CellUpdate { row: 0, col: 1, value: String::from("42") }] }
        );

        let bad_alias = parse_params::<SetCellsParams>(
            &schema,
            json!({
                "sheetId": "",
                "cells": [{ "row": 0, "col": 0, "value": "x" }]
            }),
        )
        .expect_err("empty aliased sheetId must still trigger minLength");
        assert_eq!(bad_alias.details.as_ref().and_then(|d| d.get("pointer")).and_then(Value::as_str), Some("/sheet_id"));
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
}
