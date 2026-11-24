//! Core transactional `serde_json` object-merge primitives for dialog state patching.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

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
