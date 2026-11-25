//! Shared integration test helpers for `curvebook-ui-egui` suites.
#![forbid(unsafe_code)]
#![allow(dead_code, unused_imports)]

pub use curvebook_testkit::harness::{TempDirGuard, assert_close, assert_rel_close};
use curvebook_ui_egui::{UiApp, ViewportMetrics};
use serde_json::{Value, json};

/// Create a fresh RAII temporary directory with a `String` error type.
pub fn temp_dir(label: &str) -> Result<TempDirGuard, String> {
    TempDirGuard::new(label).map_err(|err| format!("create temp dir `{label}`: {err}"))
}

/// Encode bytes as standard Base64 via `curvebook_platform::base64`.
#[must_use]
pub fn encode_base64(input: &[u8]) -> String {
    curvebook_platform::base64::encode(input)
}

/// Decode standard Base64 via `curvebook_platform::base64`.
pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    curvebook_platform::base64::decode(input).map_err(|err| err.to_string())
}

/// Return `Ok(())` when `cond` holds, otherwise `Err(msg)`.
pub fn ensure(cond: bool, msg: impl Into<String>) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.into()) }
}

/// Result-returning absolute-tolerance floating-point comparison for test workflows.
pub fn assert_close_res(actual: f64, expected: f64, tol: f64, label: &str) -> Result<(), String> {
    if !actual.is_finite() || (actual - expected).abs() > tol {
        return Err(format!("{label}: expected {expected} +/- {tol}, got {actual}"));
    }
    Ok(())
}

/// Default serialized [`ViewportMetrics`] payload for `ui.inspect` / `ui.screenshot` calls.
#[must_use]
pub fn default_metrics() -> Value {
    serde_json::to_value(ViewportMetrics::default()).unwrap_or_else(|_| json!({}))
}

/// Recursively locate a widget JSON object by `"widget_id"` inside a `ui.inspect` snapshot.
#[must_use]
pub fn find_widget<'a>(snapshot: &'a Value, wid: &str) -> Option<&'a Value> {
    match snapshot {
        Value::Object(map) => {
            if map.get("widget_id").and_then(Value::as_str) == Some(wid) {
                return Some(snapshot);
            }
            for child in map.values() {
                if let Some(hit) = find_widget(child, wid) {
                    return Some(hit);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(|item| find_widget(item, wid)),
        _ => None,
    }
}
