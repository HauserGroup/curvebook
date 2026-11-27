//! Layout sheet preview ingestion and opaque binary diagnostics for `.prism` archives (§5.3).
//!
//! Corpus observations show that `.prism` layout geometry is stored as opaque binary
//! `layouts/<uid>/data.bin` (`PCFFGRA4`) blobs. This module ingests `layouts/<uid>/preview.png`
//! and supported raster `media/<uid>.{png,jpg,tiff}` assets via
//! [`crate::image_asset::ingest_image_asset`], attaches a preview [`LayoutSheet`] when a valid
//! raster preview PNG is present, and emits one structured
//! [`ImportWarningKind::UnsupportedGraphOrLayout`] warning per opaque layout.

use std::collections::{BTreeMap, HashSet};

use curvebook_doc::{CropInsetsPt, LayoutItem, LayoutSheet, LayoutSpec, ProjectDocument, RectPt, SheetId};

use crate::archive::{BoundedJsonOutcome, parse_bounded_json_outcome};
use crate::error::{FormatError, ImportWarning, ImportWarningKind};
use crate::prism::graphs::{is_supported_raster_format, try_ingest_zip_raster};
use crate::prism::schema::{
    ArchiveIdIndex, ExternalDocumentIndex, IdLookup, extract_media_representations, parse_archive_color, parse_rich_text_value,
};
use crate::pzfx::{allocate_valid_unique_sheet_id, clamp_title};
use crate::zip::ZipIndex;

/// Inspect all listed `.prism` layouts, ingesting raster previews and supported raster media
/// while reporting opaque binary layout definitions cleanly.
pub(crate) fn import_external_layouts(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    doc_index: &ExternalDocumentIndex,
    doc: &mut ProjectDocument,
    uid_to_sheet_id: &mut BTreeMap<String, SheetId>,
    used_ids: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), FormatError> {
    for (idx, layout_uid) in doc_index.layout_sheet_ids.iter().enumerate() {
        let attr = doc_index.sheet_attrs.get(layout_uid);
        let mut title = attr.and_then(|a| a.title.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| layout_uid.clone());
        let mut color_tag = attr.and_then(|a| a.highlight_color);

        let mut media_refs = Vec::new();
        match id_index.find_layout_entry(layout_uid, "sheet.json") {
            IdLookup::Found(sheet_path) => {
                if let Some(payload) = zip_index.read_entry(&sheet_path)? {
                    match parse_bounded_json_outcome(&payload) {
                        BoundedJsonOutcome::Value(val) => {
                            if let Some(t) = val.get("title").map(|v| parse_rich_text_value(v).1).filter(|s| !s.trim().is_empty())
                                && title == *layout_uid
                            {
                                title = t;
                            }
                            if color_tag.is_none() {
                                color_tag = val.get("highlightColor").and_then(serde_json::Value::as_str).and_then(parse_archive_color);
                            }
                            media_refs = extract_media_representations(&val);
                        }
                        BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
                        BoundedJsonOutcome::SyntaxOrType(err) => {
                            warnings.push(ImportWarning::new(
                                ImportWarningKind::MissingMetadata,
                                sheet_path,
                                format!("could not parse layout sheet JSON for `{layout_uid}`: {err}"),
                            ));
                        }
                    }
                }
            }
            IdLookup::Ambiguous(candidates) => {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    layout_uid.clone(),
                    format!("ambiguous layout sheet.json paths for `{layout_uid}` ({} candidates); skipping metadata", candidates.len()),
                ));
            }
            IdLookup::Missing => {}
        }

        let mut primary_preview_asset: Option<String> = None;
        let mut ingested_assets: Vec<String> = Vec::new();
        if let IdLookup::Found(preview_path) = id_index.find_layout_entry(layout_uid, "preview.png")
            && let Some(asset_id) = try_ingest_zip_raster(zip_index, &preview_path, doc, warnings)?
        {
            primary_preview_asset = Some(asset_id.clone());
            ingested_assets.push(asset_id);
        }

        for media in &media_refs {
            if !is_supported_raster_format(&media.format) {
                continue;
            }
            if let IdLookup::Found(media_path) = id_index.find_media_entry(&media.uid, Some(&media.format))
                && let Some(asset_id) = try_ingest_zip_raster(zip_index, &media_path, doc, warnings)?
                && !ingested_assets.contains(&asset_id)
            {
                ingested_assets.push(asset_id);
            }
        }

        if let Some(ref preview_id) = primary_preview_asset {
            let sid = allocate_valid_unique_sheet_id(Some(layout_uid), "layout", idx + 1, used_ids, warnings);
            let clamped = clamp_title(&title, sid.as_str(), warnings);
            let mut spec = LayoutSpec::default();
            let (w_pt, h_pt) = preview_bounds_pt(doc, preview_id);
            let item = LayoutItem::EmbeddedImage {
                id: String::from("preview-1"),
                asset_id: preview_id.clone(),
                bounds_pt: RectPt::new(18.0, 18.0, w_pt, h_pt),
                rotation_deg: 0.0,
                lock_aspect_ratio: true,
                crop: CropInsetsPt::default(),
                border: None,
                badge_override: None,
            };
            let _ = spec.add_item(item);
            if let Ok(mut sheet) = LayoutSheet::new(sid.clone(), clamped, spec) {
                sheet.color_tag = color_tag;
                match doc.add_layout_sheet(sheet) {
                    Ok(()) => {
                        uid_to_sheet_id.insert(layout_uid.clone(), sid);
                    }
                    Err(err) => {
                        warnings.push(ImportWarning::new(
                            ImportWarningKind::UnsupportedGraphOrLayout,
                            format!("layouts/{layout_uid}"),
                            format!("could not attach preview layout sheet for `{title}` (`{layout_uid}`): {err}"),
                        ));
                    }
                }
            }
        }

        let has_opaque_bin = matches!(id_index.find_layout_entry(layout_uid, "data.bin"), IdLookup::Found(_));
        let preview_note = if ingested_assets.is_empty() {
            String::from("no raster preview image was present")
        } else {
            format!("ingested {} raster preview/media asset(s): {}", ingested_assets.len(), ingested_assets.join(", "))
        };
        let bin_note = if has_opaque_bin { "opaque binary layout definition (`data.bin`)" } else { "external layout definition" };
        warnings.push(ImportWarning::new(
            ImportWarningKind::UnsupportedGraphOrLayout,
            format!("layouts/{layout_uid}"),
            format!("layout `{title}` (`{layout_uid}`) uses an {bin_note} that cannot be translated natively; {preview_note}"),
        ));
    }
    Ok(())
}

fn preview_bounds_pt(doc: &ProjectDocument, asset_id: &str) -> (f64, f64) {
    let Some(asset) = doc.image_asset(asset_id) else {
        return (360.0, 240.0);
    };
    let (pw, ph) = asset.pixel_size;
    if pw == 0 || ph == 0 {
        return (360.0, 240.0);
    }
    let max_dim = 420.0_f64;
    let w = f64::from(pw);
    let h = f64::from(ph);
    let scale = (max_dim / w.max(h)).min(1.0);
    ((w * scale).max(24.0), (h * scale).max(24.0))
}
