//! Core transactional `serde_json` object-merge primitives and declarative field-rule helpers for dialog state patching.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use super::value_parsers::{
    parse_bool_field, parse_finite_f64_with_locale, parse_nonneg_f64, parse_opt_finite_f64_with_locale, parse_opt_nonempty_string,
    parse_opt_usize_min, parse_str_field, parse_u64_val, parse_usize_min,
};
use crate::i18n::LocalePreset;

/// Merge a pre-normalized JSON object `patch` into `target` via
/// `serde_json::to_value` -> recursive/atomic map merge -> `serde_json::from_value`.
///
/// `nested_struct_keys` lists top-level field names whose JSON objects are plain structs
/// (safe to merge field-by-field, e.g. `&["job"]`, `&["bounds_pt", "crop"]`, `&["style"]`).
/// All other object-valued fields (such as tagged enums or replacement specs) replace atomically.
///
/// Transactional: `*target` is overwritten only after `serde_json::from_value` succeeds.
pub(super) fn merge_struct_json<T: Serialize + DeserializeOwned>(
    target: &mut T,
    patch: &Map<String, Value>,
    nested_struct_keys: &[&str],
) -> Result<(), String> {
    if patch.is_empty() {
        return Ok(());
    }
    let mut base = serde_json::to_value(&*target).map_err(|e| format!("failed to serialize dialog state: {e}"))?;
    let Some(base_map) = base.as_object_mut() else {
        return Err(String::from("dialog state did not serialize to a JSON object"));
    };
    merge_object_maps(base_map, patch, nested_struct_keys);
    let updated: T = serde_json::from_value(base).map_err(|e| format!("invalid dialog patch: {e}"))?;
    *target = updated;
    Ok(())
}

pub(super) fn merge_object_maps(dst: &mut Map<String, Value>, src: &Map<String, Value>, nested_struct_keys: &[&str]) {
    for (k, v) in src {
        if nested_struct_keys.contains(&k.as_str())
            && let Some(src_obj) = v.as_object()
            && let Some(Value::Object(dst_obj)) = dst.get_mut(k)
        {
            for (nk, nv) in src_obj {
                dst_obj.insert(nk.clone(), nv.clone());
            }
            continue;
        }
        dst.insert(k.clone(), v.clone());
    }
}

/// Insert `val` into nested plain-struct object `root[outer][inner]` inside a normalized patch map.
pub(super) fn insert_nested_field(root: &mut Map<String, Value>, outer: &str, inner: &str, val: Value) {
    let entry = root.entry(outer.to_owned()).or_insert_with(|| Value::Object(Map::new()));
    if let Some(obj) = entry.as_object_mut() {
        obj.insert(inner.to_owned(), val);
    } else {
        let mut obj = Map::new();
        obj.insert(inner.to_owned(), val);
        *entry = Value::Object(obj);
    }
}

pub(super) fn insert_target_field(root: &mut Map<String, Value>, target: &str, val: Value) {
    if let Some((outer, inner)) = target.split_once('.') {
        insert_nested_field(root, outer, inner, val);
    } else {
        root.insert(target.to_owned(), val);
    }
}

#[derive(Clone, Copy)]
pub(super) enum FieldKind {
    Bool,
    OptBool,
    Str,
    OptStr,
    StrOrNullEmpty,
    StrTrimNonEmpty,
    OptNonEmptyStr,
    FiniteF64,
    OptFiniteF64,
    NonNegF64,
    UsizeMin(usize),
    OptUsizeMin(usize),
    U64,
}

#[derive(Clone, Copy)]
pub(super) struct FieldRule {
    pub aliases: &'static [&'static str],
    pub target: &'static str,
    pub kind: FieldKind,
    pub dirty_flag: Option<&'static str>,
}

impl FieldRule {
    pub(super) const fn new(aliases: &'static [&'static str], target: &'static str, kind: FieldKind) -> Self {
        Self { aliases, target, kind, dirty_flag: None }
    }

    pub(super) const fn with_dirty(aliases: &'static [&'static str], target: &'static str, kind: FieldKind, dirty: &'static str) -> Self {
        Self { aliases, target, kind, dirty_flag: Some(dirty) }
    }
}

pub(super) fn try_apply_rule(
    key: &str,
    val: &Value,
    rules: &[FieldRule],
    preset: &LocalePreset,
    patch: &mut Map<String, Value>,
) -> Result<bool, String> {
    let Some(rule) = rules.iter().find(|r| r.aliases.contains(&key)) else {
        return Ok(false);
    };
    let parsed: Option<Value> = match rule.kind {
        FieldKind::Bool => Some(json!(parse_bool_field(val, key)?)),
        FieldKind::OptBool => Some(json!(match val {
            Value::Null => None,
            Value::Bool(b) => Some(*b),
            _ => return Err(format!("`{key}` must be a boolean or null")),
        })),
        FieldKind::Str => Some(json!(parse_str_field(val, key)?)),
        FieldKind::OptStr => Some(json!(match val {
            Value::Null => None,
            Value::String(s) => Some(s.as_str()),
            _ => return Err(format!("`{key}` must be a string or null")),
        })),
        FieldKind::StrOrNullEmpty => Some(json!(if val.is_null() { "" } else { parse_str_field(val, key)? })),
        FieldKind::StrTrimNonEmpty => {
            let s = parse_str_field(val, key)?.trim();
            (!s.is_empty()).then(|| json!(s))
        }
        FieldKind::OptNonEmptyStr => Some(json!(parse_opt_nonempty_string(val, key)?)),
        FieldKind::FiniteF64 => Some(json!(parse_finite_f64_with_locale(val, key, preset)?)),
        FieldKind::OptFiniteF64 => Some(json!(parse_opt_finite_f64_with_locale(val, key, preset)?)),
        FieldKind::NonNegF64 => Some(json!(parse_nonneg_f64(val, key, preset)?)),
        FieldKind::UsizeMin(min) => Some(json!(parse_usize_min(val, key, min)?)),
        FieldKind::OptUsizeMin(min) => Some(json!(parse_opt_usize_min(val, key, min)?)),
        FieldKind::U64 => Some(json!(parse_u64_val(val, key)?)),
    };
    if let Some(v) = parsed {
        insert_target_field(patch, rule.target, v);
        if let Some(flag) = rule.dirty_flag {
            patch.insert(flag.to_owned(), json!(true));
        }
    }
    Ok(true)
}

pub(super) fn apply_rules_strict(
    fields: &Map<String, Value>,
    rules: &[FieldRule],
    preset: &LocalePreset,
    dialog_name: &str,
) -> Result<Map<String, Value>, String> {
    let mut patch = Map::new();
    for (key, val) in fields {
        if !try_apply_rule(key, val, rules, preset, &mut patch)? {
            return Err(format!("unknown {dialog_name} field `{key}`"));
        }
    }
    Ok(patch)
}
