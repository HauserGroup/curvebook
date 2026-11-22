//! Shared image ingestion, placement, crop, border, and aspect-ratio helpers for embedded
//! PNG/JPEG/TIFF commands (`graph.insertImage`, `graph.updateImage`, `layout.insertImage`,
//! `layout.updateImage`, `edit.pasteImage`).

use curvebook_doc::{CropInsetsPt, ErrorCode, ImageBorderSpec, ImageMimeKind, MAX_IMAGE_ASSET_BYTES, ProjectDocument, RectPt, TableKind};
use curvebook_format::{FormatError, ingest_image_asset};
use curvebook_palette::{LineDashStyle, Rgba8};
use serde::Deserialize;
use serde_json::{Value, json};

use super::graph::{parse_line_dash_style, parse_rect_pt, parse_rgba8};
use super::helpers::decode_base64_capped;
use super::params::{any_json_schema, bool_schema, non_empty_str_schema, nullable_schema, num_schema, obj_schema};
use crate::EngineError;

/// Resolved image asset metadata after ingestion or project-store lookup.
#[derive(Debug, Clone)]
pub(super) struct ResolvedImageAsset {
    pub asset_id: String,
    pub media_type: ImageMimeKind,
    pub pixel_size: (u32, u32),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CropInsetsPatchParams {
    left_pt: Option<f64>,
    right_pt: Option<f64>,
    top_pt: Option<f64>,
    bottom_pt: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageBorderParams {
    width_pt: Option<f64>,
    color: Option<Value>,
    dash: Option<Value>,
}

/// JSON Schema snippet describing a non-negative crop insets object.
#[must_use]
pub(super) fn crop_schema() -> Value {
    obj_schema(
        vec![
            ("left_pt", nullable_schema(num_schema("Left crop inset in points (>= 0)", Some(0.0), None))),
            ("right_pt", nullable_schema(num_schema("Right crop inset in points (>= 0)", Some(0.0), None))),
            ("top_pt", nullable_schema(num_schema("Top crop inset in points (>= 0)", Some(0.0), None))),
            ("bottom_pt", nullable_schema(num_schema("Bottom crop inset in points (>= 0)", Some(0.0), None))),
        ],
        &[],
        false,
    )
}

/// JSON Schema snippet describing an optional image border specification.
#[must_use]
pub(super) fn border_schema() -> Value {
    obj_schema(
        vec![
            ("width_pt", nullable_schema(num_schema("Border stroke width in points (>= 0)", Some(0.0), None))),
            ("color", nullable_schema(any_json_schema("Border stroke color (`#RRGGBB` / `#RRGGBBAA` or `[r, g, b, a]`)"))),
            ("dash", nullable_schema(any_json_schema("Border dash style (`solid`, `dashed`, `dotted`, `dash_dot`, etc.)"))),
        ],
        &[],
        false,
    )
}

/// Common insert-image source and placement schema fields.
pub(super) fn insert_image_common_props() -> Vec<(&'static str, Value)> {
    vec![
        ("path", nullable_schema(non_empty_str_schema("Regular local file path to a PNG, JPEG, or TIFF image (URLs rejected)"))),
        ("bytes_base64", nullable_schema(non_empty_str_schema("Base64-encoded PNG, JPEG, or TIFF image payload"))),
        ("asset_id", nullable_schema(non_empty_str_schema("Existing content-addressed image `asset_id` already stored in the project"))),
        ("bounds_pt", nullable_schema(any_json_schema("Optional placement `[x, y, width, height]` or `RectPt` in points"))),
        ("rotation_deg", nullable_schema(num_schema("Clockwise rotation in degrees around the center of `bounds_pt`", Some(-3600.0), Some(3600.0)))),
        ("lock_aspect_ratio", nullable_schema(bool_schema("Preserve intrinsic image aspect ratio when resizing one dimension (default `true`)"))),
        ("crop", nullable_schema(crop_schema())),
        ("border", nullable_schema(border_schema())),
    ]
}

fn reject_uri_scheme(raw_path: &str) -> Result<(), EngineError> {
    let lower = raw_path.trim().to_ascii_lowercase();
    for prefix in ["http://", "https://", "file://", "ftp://", "data:"] {
        if lower.starts_with(prefix) {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                "remote URLs and URI schemes are disabled for image import; provide a regular file `path`, `bytes_base64`, or `asset_id`",
            )
            .with_details(json!({ "pointer": "/path" })));
        }
    }
    Ok(())
}

fn map_format_ingest_error(err: FormatError, pointer: &str) -> EngineError {
    let code = match err {
        FormatError::FileTooLarge { .. } | FormatError::ZipBomb { .. } => ErrorCode::ResourceLimit,
        FormatError::Io(_) => ErrorCode::IoError,
        _ => ErrorCode::BadInput,
    };
    EngineError::with_code(code, err.to_string()).with_details(json!({ "pointer": pointer }))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_image_file_bytes(raw_path: &str) -> Result<Vec<u8>, EngineError> {
    curvebook_platform::fs::read_regular_file_bytes(std::path::Path::new(raw_path), MAX_IMAGE_ASSET_BYTES).map_err(|err| {
        let code = match err {
            curvebook_platform::PlatformError::FileTooLarge { .. } => ErrorCode::ResourceLimit,
            curvebook_platform::PlatformError::PathEscape { .. } | curvebook_platform::PlatformError::InvalidPath { .. } => ErrorCode::BadInput,
            curvebook_platform::PlatformError::ProgramNotFound { .. }
            | curvebook_platform::PlatformError::TimedOut { .. }
            | curvebook_platform::PlatformError::OutputLimitExceeded { .. }
            | curvebook_platform::PlatformError::Io { .. } => ErrorCode::IoError,
        };
        EngineError::with_code(code, err.to_string()).with_details(json!({ "pointer": "/path" }))
    })
}

#[cfg(target_arch = "wasm32")]
fn read_image_file_bytes(_raw_path: &str) -> Result<Vec<u8>, EngineError> {
    Err(EngineError::with_code(
        ErrorCode::BadInput,
        "filesystem `path` reads are not supported on wasm32; pass `bytes_base64` or an existing `asset_id`",
    )
    .with_details(json!({ "pointer": "/path" })))
}

/// Resolve and ingest an image from exactly one of `path`, `bytes_base64`, or `asset_id`.
pub(super) fn resolve_image_source(
    doc: &mut ProjectDocument,
    path: Option<&str>,
    bytes_base64: Option<&str>,
    asset_id: Option<&str>,
) -> Result<ResolvedImageAsset, EngineError> {
    let source_count = usize::from(path.is_some()) + usize::from(bytes_base64.is_some()) + usize::from(asset_id.is_some());
    if source_count == 0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "provide exactly one of `path`, `bytes_base64`, or `asset_id`"));
    }
    if source_count > 1 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`path`, `bytes_base64`, and `asset_id` are mutually exclusive"));
    }

    if let Some(existing_id) = asset_id {
        let Some(asset) = doc.image_asset(existing_id) else {
            return Err(EngineError::with_code(ErrorCode::NotFound, format!("image asset `{existing_id}` not found in project"))
                .with_details(json!({ "pointer": "/asset_id" })));
        };
        return Ok(ResolvedImageAsset { asset_id: asset.asset_id.clone(), media_type: asset.media_type, pixel_size: asset.pixel_size });
    }

    let (raw_bytes, pointer) = if let Some(b64) = bytes_base64 {
        (decode_base64_capped(b64, MAX_IMAGE_ASSET_BYTES, "/bytes_base64")?, "/bytes_base64")
    } else if let Some(raw_path) = path {
        reject_uri_scheme(raw_path)?;
        (read_image_file_bytes(raw_path)?, "/path")
    } else {
        return Err(EngineError::with_code(ErrorCode::BadInput, "missing image source"));
    };

    let asset = ingest_image_asset(&raw_bytes).map_err(|err| map_format_ingest_error(err, pointer))?;
    let media_type = asset.media_type;
    let pixel_size = asset.pixel_size;
    let canonical_id = doc.upsert_image_asset(asset)?;
    Ok(ResolvedImageAsset { asset_id: canonical_id, media_type, pixel_size })
}

/// Compute aspect-preserving default bounds inside `canvas_w x canvas_h` points.
pub(super) fn default_image_bounds(pixel_size: (u32, u32), canvas_w: f64, canvas_h: f64) -> RectPt {
    let pw = f64::from(pixel_size.0.max(1));
    let ph = f64::from(pixel_size.1.max(1));
    let aspect = ph / pw;
    let mut w = (canvas_w * 0.45).clamp(40.0, 200.0);
    let mut h = w * aspect;
    let max_w = (canvas_w * 0.65).max(40.0);
    let max_h = (canvas_h * 0.65).max(40.0);
    if h > max_h && h > 0.0 {
        let scale = max_h / h;
        w = (w * scale).max(16.0);
        h = (h * scale).max(16.0);
    }
    if w > max_w && w > 0.0 {
        let scale = max_w / w;
        w = (w * scale).max(16.0);
        h = (h * scale).max(16.0);
    }
    RectPt::new(24.0, 24.0, w, h)
}

/// Parse and validate explicit or default placement bounds for an inserted image.
pub(super) fn parse_image_insert_bounds(val: Option<&Value>, pixel_size: (u32, u32), canvas_w: f64, canvas_h: f64) -> Result<RectPt, EngineError> {
    let fallback = default_image_bounds(pixel_size, canvas_w, canvas_h);
    let rect = parse_rect_pt(val, fallback)?;
    validate_positive_rect(rect, "/bounds_pt")?;
    Ok(rect)
}

/// Validate finite positive width/height for an image placement rectangle.
pub(super) fn validate_positive_rect(rect: RectPt, pointer: &str) -> Result<(), EngineError> {
    if !rect.x.is_finite() || !rect.y.is_finite() || !rect.width.is_finite() || !rect.height.is_finite() || rect.width <= 0.0 || rect.height <= 0.0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "image `bounds_pt` must be finite with width > 0 and height > 0")
            .with_details(json!({ "pointer": pointer })));
    }
    Ok(())
}

/// Apply optional `crop` patch onto `base` and verify the visible region does not collapse.
pub(super) fn apply_crop_patch(val: Option<&Value>, bounds: RectPt, base: CropInsetsPt) -> Result<CropInsetsPt, EngineError> {
    let mut next = base;
    if let Some(v) = val {
        let patch: CropInsetsPatchParams =
            crate::cmd::params::parse_params(&crop_schema(), v.clone()).map_err(|err| err.with_details(json!({ "pointer": "/crop" })))?;
        if let Some(l) = patch.left_pt {
            next.left_pt = l;
        }
        if let Some(r) = patch.right_pt {
            next.right_pt = r;
        }
        if let Some(t) = patch.top_pt {
            next.top_pt = t;
        }
        if let Some(b) = patch.bottom_pt {
            next.bottom_pt = b;
        }
    }
    next.validate(TableKind::Xy)?;
    if next.visible_rect(bounds).is_none() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "image crop insets exceed `bounds_pt` dimensions")
            .with_details(json!({ "pointer": "/crop" })));
    }
    Ok(next)
}

/// Parse optional border specification (supporting explicit `null` to clear an existing border).
pub(super) fn apply_border_patch(
    border_arg: Option<&Option<Value>>,
    current: Option<ImageBorderSpec>,
) -> Result<Option<ImageBorderSpec>, EngineError> {
    let Some(opt_val) = border_arg else {
        return Ok(current);
    };
    let Some(raw_obj) = opt_val else {
        return Ok(None);
    };
    let patch: ImageBorderParams = crate::cmd::params::parse_params(&border_schema(), raw_obj.clone())?;
    let mut next = current.unwrap_or(ImageBorderSpec { width_pt: 0.75, color: Rgba8::BLACK, dash: LineDashStyle::Solid });
    if let Some(w) = patch.width_pt {
        next.width_pt = w;
    }
    if let Some(ref c) = patch.color {
        next.color = parse_rgba8(c)?;
    }
    if let Some(ref d) = patch.dash {
        next.dash = parse_line_dash_style(d)?;
    }
    next.validate(TableKind::Xy)?;
    Ok(Some(next))
}

/// Update `bounds` given optional `bounds_pt`, `width_pt`, `height_pt`, and `lock_aspect_ratio`.
pub(super) fn apply_image_bounds_update(
    bounds: &mut RectPt,
    bounds_pt_arg: Option<&Value>,
    width_pt_arg: Option<f64>,
    height_pt_arg: Option<f64>,
    lock_aspect_ratio: bool,
    intrinsic_px: Option<(u32, u32)>,
) -> Result<(), EngineError> {
    if bounds_pt_arg.is_some() && (width_pt_arg.is_some() || height_pt_arg.is_some()) {
        return Err(EngineError::with_code(ErrorCode::BadInput, "cannot combine `bounds_pt` with `width_pt` or `height_pt`"));
    }
    if let Some(raw_rect) = bounds_pt_arg {
        let next = parse_rect_pt(Some(raw_rect), *bounds)?;
        validate_positive_rect(next, "/bounds_pt")?;
        *bounds = next;
        return Ok(());
    }
    let aspect_h_over_w = intrinsic_px
        .filter(|&(w, h)| w > 0 && h > 0)
        .map(|(w, h)| f64::from(h) / f64::from(w))
        .unwrap_or_else(|| if bounds.width > 0.0 { bounds.height / bounds.width } else { 1.0 });

    match (width_pt_arg, height_pt_arg) {
        (Some(w), Some(h)) => {
            if !w.is_finite() || w <= 0.0 || !h.is_finite() || h <= 0.0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`width_pt` and `height_pt` must be finite and > 0"));
            }
            bounds.width = w;
            bounds.height = h;
        }
        (Some(w), None) => {
            if !w.is_finite() || w <= 0.0 {
                return Err(
                    EngineError::with_code(ErrorCode::BadInput, "`width_pt` must be finite and > 0").with_details(json!({ "pointer": "/width_pt" }))
                );
            }
            bounds.width = w;
            if lock_aspect_ratio {
                bounds.height = (w * aspect_h_over_w).max(1e-3);
            }
        }
        (None, Some(h)) => {
            if !h.is_finite() || h <= 0.0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`height_pt` must be finite and > 0")
                    .with_details(json!({ "pointer": "/height_pt" })));
            }
            bounds.height = h;
            if lock_aspect_ratio {
                bounds.width = (h / aspect_h_over_w.max(1e-9)).max(1e-3);
            }
        }
        (None, None) => {}
    }
    Ok(())
}

/// Validate and normalize `rotation_deg` if provided.
pub(super) fn parse_rotation_deg(val: Option<f64>, current: f64) -> Result<f64, EngineError> {
    let Some(deg) = val else {
        return Ok(current);
    };
    if !deg.is_finite() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`rotation_deg` must be finite").with_details(json!({ "pointer": "/rotation_deg" })));
    }
    Ok(deg)
}
