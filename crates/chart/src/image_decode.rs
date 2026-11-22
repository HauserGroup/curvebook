//! Bounded decoding and scene emission for embedded PNG, JPEG, and TIFF raster images.

use std::collections::BTreeMap;
use std::io::Cursor;

use curvebook_doc::{
    CropInsetsPt, ImageBorderSpec, ImageMimeKind, MAX_IMAGE_ASSET_BYTES, MAX_IMAGE_DIM_PX, MAX_IMAGE_PIXELS, ProjectDocument, ProjectImageAsset,
    RectPt as DocRectPt,
};
use curvebook_text::parse_markup;
use image::{ImageFormat, ImageReader, Limits};

use crate::scene::{
    ChartScene, ChartWarning, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, SceneRasterImage, StrokePaint, TextAnchor,
};

const fn image_format_for_mime(kind: ImageMimeKind) -> ImageFormat {
    match kind {
        ImageMimeKind::Png => ImageFormat::Png,
        ImageMimeKind::Jpeg => ImageFormat::Jpeg,
        ImageMimeKind::Tiff => ImageFormat::Tiff,
    }
}

fn bounded_reader(bytes: &[u8], kind: ImageMimeKind) -> ImageReader<Cursor<&[u8]>> {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIM_PX);
    limits.max_image_height = Some(MAX_IMAGE_DIM_PX);
    limits.max_alloc = Some(MAX_IMAGE_PIXELS.saturating_mul(16));
    let mut reader = ImageReader::with_format(Cursor::new(bytes), image_format_for_mime(kind));
    reader.limits(limits);
    reader
}

/// Decode a stored [`ProjectImageAsset`] into straight-alpha RGBA8 pixels under strict dimension
/// and allocation bounds.
pub fn decode_asset_rgba8(asset: &ProjectImageAsset) -> Result<((u32, u32), Vec<u8>), String> {
    if asset.bytes.is_empty() {
        return Err(format!("embedded image asset `{}` has empty payload", asset.asset_id));
    }
    if asset.bytes.len() > MAX_IMAGE_ASSET_BYTES {
        return Err(format!("embedded image asset `{}` byte length {} exceeds limit {MAX_IMAGE_ASSET_BYTES}", asset.asset_id, asset.bytes.len()));
    }
    let Some(sniffed) = ImageMimeKind::sniff_header(&asset.bytes) else {
        return Err(format!("embedded image asset `{}` has unrecognized magic bytes", asset.asset_id));
    };
    if sniffed != asset.media_type {
        return Err(format!("embedded image asset `{}` declares {:?} but magic bytes indicate {:?}", asset.asset_id, asset.media_type, sniffed));
    }
    let (w, h) = bounded_reader(&asset.bytes, asset.media_type)
        .into_dimensions()
        .map_err(|err| format!("failed to probe image asset `{}` dimensions: {err}", asset.asset_id))?;
    if w == 0 || h == 0 || w > MAX_IMAGE_DIM_PX || h > MAX_IMAGE_DIM_PX {
        return Err(format!("embedded image `{}` dimensions {w}x{h} exceed limit {MAX_IMAGE_DIM_PX}", asset.asset_id));
    }
    let pixels = u64::from(w).saturating_mul(u64::from(h));
    if pixels > MAX_IMAGE_PIXELS {
        return Err(format!("embedded image `{}` pixel count {pixels} exceeds limit {MAX_IMAGE_PIXELS}", asset.asset_id));
    }
    let decoded = bounded_reader(&asset.bytes, asset.media_type)
        .decode()
        .map_err(|err| format!("failed to decode embedded image `{}`: {err}", asset.asset_id))?;
    let rgba = decoded.into_rgba8();
    Ok(((rgba.width(), rgba.height()), rgba.into_raw()))
}

/// Draw specification for an embedded graph or layout image primitive.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedImageDrawSpec<'a> {
    pub asset_id: &'a str,
    pub bounds_pt: DocRectPt,
    pub rotation_deg: f64,
    pub crop: CropInsetsPt,
    pub border: Option<ImageBorderSpec>,
    pub clip: Option<RectPt>,
    pub badge_override: Option<&'a str>,
}

type CachedImageDecode = Result<((u32, u32), Vec<u8>), String>;

/// Per-scene decoded RGBA cache keyed by `asset_id` so repeated references decode once.
#[derive(Debug, Default)]
pub(crate) struct DecodedImageCache {
    entries: BTreeMap<String, CachedImageDecode>,
    decode_calls: usize,
}

impl DecodedImageCache {
    #[cfg(test)]
    pub(crate) fn decode_calls(&self) -> usize {
        self.decode_calls
    }
}

/// Resolve, bounded-decode, and push a [`ScenePrimitive::RasterImage`] (plus optional rotated
/// border [`ScenePrimitive::Path`] and optional badge [`ScenePrimitive::Text`]) onto `scene`.
pub fn emit_embedded_image_primitive(
    scene: &mut ChartScene,
    doc: &ProjectDocument,
    spec: &EmbeddedImageDrawSpec<'_>,
    pixel_budget: Option<&mut u64>,
) {
    emit_embedded_image_primitive_cached(scene, doc, spec, pixel_budget, None);
}

/// Same as [`emit_embedded_image_primitive`], reusing a per-scene [`DecodedImageCache`] when provided
/// while still counting every materialized raster against `pixel_budget`.
pub(crate) fn emit_embedded_image_primitive_cached(
    scene: &mut ChartScene,
    doc: &ProjectDocument,
    spec: &EmbeddedImageDrawSpec<'_>,
    pixel_budget: Option<&mut u64>,
    cache: Option<&mut DecodedImageCache>,
) {
    if !spec.bounds_pt.x.is_finite()
        || !spec.bounds_pt.y.is_finite()
        || !spec.bounds_pt.width.is_finite()
        || !spec.bounds_pt.height.is_finite()
        || spec.bounds_pt.width <= 0.0
        || spec.bounds_pt.height <= 0.0
        || !spec.rotation_deg.is_finite()
    {
        return;
    }
    let Some(vis_doc) = spec.crop.visible_rect(spec.bounds_pt) else {
        return;
    };
    let Some(asset) = doc.image_asset(spec.asset_id) else {
        scene.warnings.push(ChartWarning::new("missing_embedded_image", format!("embedded image asset `{}` not found", spec.asset_id)));
        return;
    };
    let declared_px = u64::from(asset.pixel_size.0).saturating_mul(u64::from(asset.pixel_size.1));
    if let Some(used) = pixel_budget.as_deref()
        && used.saturating_add(declared_px) > MAX_IMAGE_PIXELS
    {
        scene.warnings.push(ChartWarning::new(
            "embedded_image_budget_exceeded",
            format!("embedded image `{}` exceeds cumulative pixel budget {MAX_IMAGE_PIXELS}", spec.asset_id),
        ));
        return;
    }

    let decoded_res: CachedImageDecode = match cache {
        Some(c) => {
            if let Some(existing) = c.entries.get(&asset.asset_id) {
                match existing {
                    Ok(((w, h), bytes)) => {
                        let actual_px = u64::from(*w).saturating_mul(u64::from(*h));
                        if let Some(used) = pixel_budget.as_deref()
                            && used.saturating_add(actual_px) > MAX_IMAGE_PIXELS
                        {
                            scene.warnings.push(ChartWarning::new(
                                "embedded_image_budget_exceeded",
                                format!("embedded image `{}` exceeds cumulative pixel budget {MAX_IMAGE_PIXELS}", spec.asset_id),
                            ));
                            return;
                        }
                        Ok(((*w, *h), bytes.clone()))
                    }
                    Err(msg) => Err(msg.clone()),
                }
            } else {
                c.decode_calls += 1;
                let res = decode_asset_rgba8(asset);
                let out = res.clone();
                c.entries.insert(asset.asset_id.clone(), res);
                out
            }
        }
        None => decode_asset_rgba8(asset),
    };

    let ((w, h), rgba) = match decoded_res {
        Ok(decoded) => decoded,
        Err(msg) => {
            scene.warnings.push(ChartWarning::new("invalid_embedded_image", msg));
            return;
        }
    };
    let actual_px = u64::from(w).saturating_mul(u64::from(h));
    if let Some(used) = pixel_budget {
        let next = used.saturating_add(actual_px);
        if next > MAX_IMAGE_PIXELS {
            scene.warnings.push(ChartWarning::new(
                "embedded_image_budget_exceeded",
                format!("embedded image `{}` exceeds cumulative pixel budget {MAX_IMAGE_PIXELS}", spec.asset_id),
            ));
            return;
        }
        *used = next;
    }

    let bounds = RectPt::from(spec.bounds_pt);
    scene.primitives.push(ScenePrimitive::RasterImage(SceneRasterImage {
        asset_id: asset.asset_id.clone(),
        media_type: asset.media_type,
        pixel_size: (w, h),
        rgba,
        bounds_pt: bounds,
        crop: spec.crop,
        rotation_deg: spec.rotation_deg,
        clip: spec.clip,
    }));

    if let Some(border) = spec.border
        && border.width_pt.is_finite()
        && border.width_pt > 0.0
        && border.color.a > 0
    {
        let vis = RectPt::from(vis_doc);
        let cx = bounds.x + 0.5 * bounds.width;
        let cy = bounds.y + 0.5 * bounds.height;
        let (sin_t, cos_t) = spec.rotation_deg.to_radians().sin_cos();
        let rot = |x: f64, y: f64| -> PointPt {
            let dx = x - cx;
            let dy = y - cy;
            PointPt::new(cx + dx * cos_t - dy * sin_t, cy + dx * sin_t + dy * cos_t)
        };
        let path = Path2D {
            commands: vec![
                PathCmd::MoveTo(rot(vis.x, vis.y)),
                PathCmd::LineTo(rot(vis.right(), vis.y)),
                PathCmd::LineTo(rot(vis.right(), vis.bottom())),
                PathCmd::LineTo(rot(vis.x, vis.bottom())),
                PathCmd::Close,
            ],
        };
        scene.primitives.push(ScenePrimitive::Path {
            path,
            fill: None,
            stroke: Some(StrokePaint { color: border.color, width_pt: border.width_pt, dash: border.dash, ..StrokePaint::default() }),
            clip: spec.clip,
        });
    }

    if let Some(badge) = spec.badge_override
        && !badge.trim().is_empty()
    {
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&format!("<b>{badge}</b>")),
            anchor_pt: PointPt::new(bounds.x + 4.0, bounds.y + 12.0),
            anchor: TextAnchor::Start,
            font_size_pt: 11.5,
            rotation_deg: 0.0,
            color: curvebook_palette::Rgba8::BLACK,
        }));
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use curvebook_doc::{
        CropInsetsPt, ImageMimeKind, MAX_IMAGE_PIXELS, ProjectDocument, ProjectImageAsset, RectPt as DocRectPt, content_addressed_asset_id,
    };
    use image::{ImageEncoder, RgbaImage, codecs::png::PngEncoder};

    use super::{DecodedImageCache, EmbeddedImageDrawSpec, emit_embedded_image_primitive_cached};
    use crate::scene::{ChartScene, ScenePrimitive};

    fn make_png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = RgbaImage::from_pixel(w, h, image::Rgba([10, 120, 200, 255]));
        let mut buf = Vec::new();
        PngEncoder::new(Cursor::new(&mut buf)).write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8).expect("encode png");
        buf
    }

    #[test]
    fn decoded_image_cache_decodes_repeated_asset_id_once_while_enforcing_pixel_budget() {
        let bytes = make_png_bytes(32, 32);
        let asset_id = content_addressed_asset_id(&bytes);
        let asset = ProjectImageAsset { asset_id: asset_id.clone(), media_type: ImageMimeKind::Png, pixel_size: (32, 32), bytes };
        let mut doc = ProjectDocument::default();
        let inserted_id = doc.upsert_image_asset(asset).expect("add asset");

        let mut scene = ChartScene::new("cache-test", "Cache Test", 400.0, 300.0, None);
        let mut budget: u64 = 0;
        let mut cache = DecodedImageCache::default();
        let spec = EmbeddedImageDrawSpec {
            asset_id: &inserted_id,
            bounds_pt: DocRectPt::new(10.0, 10.0, 50.0, 50.0),
            rotation_deg: 0.0,
            crop: CropInsetsPt::default(),
            border: None,
            clip: None,
            badge_override: None,
        };
        for _ in 0..4 {
            emit_embedded_image_primitive_cached(&mut scene, &doc, &spec, Some(&mut budget), Some(&mut cache));
        }
        assert_eq!(cache.decode_calls(), 1, "repeated asset_id should decode exactly once per scene");
        let raster_count = scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::RasterImage(_))).count();
        assert_eq!(raster_count, 4);
        assert_eq!(budget, 4 * 32 * 32);
        assert!(budget <= MAX_IMAGE_PIXELS);
    }
}
