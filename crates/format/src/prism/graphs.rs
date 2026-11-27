//! Graph sheet preview ingestion and opaque binary diagnostics for `.prism` archives (§5.3).
//!
//! Corpus observations show that `.prism` graph visual definitions are stored as opaque binary
//! `graphs/<uid>/data.bin` (`PCFFGRA4`) blobs rather than open JSON. This module ingests any
//! available raster `graphs/<uid>/preview.png` and supported raster `media/<uid>.{png,jpg,tiff}`
//! representations via [`crate::image_asset::ingest_image_asset`] (respecting all image dimension
//! and asset-count caps) and emits one structured [`ImportWarningKind::UnsupportedGraphOrLayout`]
//! warning per opaque graph naming its title, ID, and ingested preview asset(s).

use curvebook_doc::ProjectDocument;

use crate::archive::{BoundedJsonOutcome, parse_bounded_json_outcome};
use crate::error::{FormatError, ImportWarning, ImportWarningKind};
use crate::image_asset::ingest_image_asset;
use crate::prism::schema::{ArchiveIdIndex, ExternalDocumentIndex, IdLookup, extract_media_representations, parse_rich_text_value};
use crate::zip::ZipIndex;

/// Inspect all listed `.prism` graphs, ingesting raster previews and supported raster media
/// while reporting opaque binary graph definitions cleanly.
pub(crate) fn import_external_graphs(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    doc_index: &ExternalDocumentIndex,
    doc: &mut ProjectDocument,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), FormatError> {
    for graph_uid in &doc_index.graph_sheet_ids {
        let mut title =
            doc_index.sheet_attrs.get(graph_uid).and_then(|a| a.title.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| graph_uid.clone());

        let mut media_refs = Vec::new();
        match id_index.find_graph_entry(graph_uid, "sheet.json") {
            IdLookup::Found(sheet_path) => {
                if let Some(payload) = zip_index.read_entry(&sheet_path)? {
                    match parse_bounded_json_outcome(&payload) {
                        BoundedJsonOutcome::Value(val) => {
                            if let Some(t) = val.get("title").map(|v| parse_rich_text_value(v).1).filter(|s| !s.trim().is_empty())
                                && title == *graph_uid
                            {
                                title = t;
                            }
                            media_refs = extract_media_representations(&val);
                        }
                        BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
                        BoundedJsonOutcome::SyntaxOrType(err) => {
                            warnings.push(ImportWarning::new(
                                ImportWarningKind::MissingMetadata,
                                sheet_path,
                                format!("could not parse graph sheet JSON for `{graph_uid}`: {err}"),
                            ));
                        }
                    }
                }
            }
            IdLookup::Ambiguous(candidates) => {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    graph_uid.clone(),
                    format!("ambiguous graph sheet.json paths for `{graph_uid}` ({} candidates); skipping metadata", candidates.len()),
                ));
            }
            IdLookup::Missing => {}
        }

        let mut ingested_assets: Vec<String> = Vec::new();
        if let IdLookup::Found(preview_path) = id_index.find_graph_entry(graph_uid, "preview.png")
            && let Some(asset_id) = try_ingest_zip_raster(zip_index, &preview_path, doc, warnings)?
        {
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

        let has_opaque_bin = matches!(id_index.find_graph_entry(graph_uid, "data.bin"), IdLookup::Found(_));
        let preview_note = if ingested_assets.is_empty() {
            String::from("no raster preview image was present")
        } else {
            format!("ingested {} raster preview/media asset(s): {}", ingested_assets.len(), ingested_assets.join(", "))
        };
        let bin_note = if has_opaque_bin { "opaque binary graph definition (`data.bin`)" } else { "external graph definition" };
        warnings.push(ImportWarning::new(
            ImportWarningKind::UnsupportedGraphOrLayout,
            format!("graphs/{graph_uid}"),
            format!("graph `{title}` (`{graph_uid}`) uses an {bin_note} that cannot be translated natively; {preview_note}"),
        ));
    }
    Ok(())
}

/// Read and validate one raster image entry from `zip_index` into `doc.image_assets`, returning
/// its content-addressed `asset_id` on success or recording a non-fatal warning if invalid/capped.
pub(crate) fn try_ingest_zip_raster(
    zip_index: &mut ZipIndex<'_>,
    entry_path: &str,
    doc: &mut ProjectDocument,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Option<String>, FormatError> {
    let Some(bytes) = zip_index.read_entry(entry_path)? else {
        return Ok(None);
    };
    let asset = match ingest_image_asset(&bytes) {
        Ok(a) => a,
        Err(err) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedChunk,
                entry_path,
                format!("skipped invalid raster preview/media `{entry_path}`: {err}"),
            ));
            return Ok(None);
        }
    };
    match doc.upsert_image_asset(asset) {
        Ok(id) => Ok(Some(id)),
        Err(err) => {
            warnings.push(ImportWarning::new(
                ImportWarningKind::UnsupportedChunk,
                entry_path,
                format!("could not store raster preview/media `{entry_path}` in project asset table: {err}"),
            ));
            Ok(None)
        }
    }
}

pub(crate) fn is_supported_raster_format(format: &str) -> bool {
    matches!(format.trim().to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "tif" | "tiff")
}
