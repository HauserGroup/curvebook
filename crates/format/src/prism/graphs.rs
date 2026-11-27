//! Basic editable graph reconstruction, floating-note annotation import, and raster preview
//! ingestion for `.prism` archives (§5.3).
//!
//! Corpus observations show that `.prism` graph visual styling lives in opaque binary
//! `graphs/<uid>/data.bin` (`PCFFGRA4`) blobs, which CurveBook intentionally never decodes.
//! Instead, this module reads open `graphs/<uid>/sheet.json` metadata (`title`, `inputDataSets`,
//! `floatingNotes`, `mediaCollection`, `highlightColor`) and `graphs/<uid>/floating_notes/*.json`:
//!
//! 1. When one or more `inputDataSets` resolve to imported CurveBook `DataSheet` columns, it
//!    rebuilds an initialized, editable [`GraphSheet`] using the matching archetype default from
//!    [`GraphSpec::default_for_data_sheet`], bound to the resolved datasets, titled from
//!    `sheet.json` / `sheetAttributesMap`, and populated with any resolvable `floatingNotes` as
//!    [`GraphAnnotation::TextBox`] annotations.
//! 2. It continues ingesting available raster `graphs/<uid>/preview.png` and supported raster
//!    `media/<uid>.{png,jpg,tiff}` representations via [`crate::image_asset::ingest_image_asset`].
//! 3. It emits one structured [`ImportWarningKind::UnsupportedGraphOrLayout`] warning per graph:
//!    rebuilt graphs are explicitly labelled `"rebuilt from data, original styling not imported"`,
//!    while graphs whose `inputDataSets` are empty or unresolvable emit a clear fallback warning.

use std::collections::{BTreeMap, HashSet};

use curvebook_doc::graph::MAX_GRAPH_ANNOTATIONS;
use curvebook_doc::{
    DataSheet, GraphAnnotation, GraphSheet, GraphSpec, GraphicalDataObject, MAX_DATA_SETS, MAX_TITLE_BYTES, ProjectDocument, RectPt,
    SeriesOffsetSpec, SeriesSpec, SheetId, SizePt, TableKind,
};
use curvebook_palette::{MarkerShape, Rgba8};

use crate::archive::{BoundedJsonOutcome, parse_bounded_json_outcome};
use crate::error::{FormatError, ImportWarning, ImportWarningKind};
use crate::image_asset::ingest_image_asset;
use crate::prism::analyses::ExternalDataSetPositions;
use crate::prism::schema::{
    ArchiveIdIndex, ExternalDocumentIndex, IdLookup, extract_media_representations, parse_archive_color, parse_rich_text_value,
};
use crate::pzfx::{allocate_valid_unique_sheet_id, clamp_title};
use crate::zip::ZipIndex;

/// Explicit warning label recorded for every `.prism` graph rebuilt from open dataset bindings
/// without decoding `graphs/<uid>/data.bin`.
pub const REBUILT_GRAPH_WARNING_LABEL: &str = "rebuilt from data, original styling not imported";

/// Shared lookup tables and mutable project state passed into [`import_external_graphs`].
pub(crate) struct GraphImportContext<'a> {
    pub(crate) id_index: &'a ArchiveIdIndex,
    pub(crate) doc_index: &'a ExternalDocumentIndex,
    pub(crate) data_uid_to_sheet_id: &'a BTreeMap<String, SheetId>,
    pub(crate) data_set_positions: &'a ExternalDataSetPositions,
    pub(crate) uid_to_sheet_id: &'a mut BTreeMap<String, SheetId>,
    pub(crate) used_ids: &'a mut HashSet<String>,
    pub(crate) warnings: &'a mut Vec<ImportWarning>,
}

/// Inspect all listed `.prism` graphs, rebuilding basic editable graphs from resolvable
/// `inputDataSets`, importing resolvable `floatingNotes` as text annotations, ingesting raster
/// previews/media, and reporting original binary styling omission cleanly.
pub(crate) fn import_external_graphs(
    zip_index: &mut ZipIndex<'_>,
    doc: &mut ProjectDocument,
    ctx: &mut GraphImportContext<'_>,
) -> Result<(), FormatError> {
    for (idx, graph_uid) in ctx.doc_index.graph_sheet_ids.iter().enumerate() {
        let attr = ctx.doc_index.sheet_attrs.get(graph_uid);
        let mut title = attr.and_then(|a| a.title.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| graph_uid.clone());
        let mut color_tag = attr.and_then(|a| a.highlight_color);

        let mut media_refs = Vec::new();
        let mut input_dataset_uids: Vec<String> = Vec::new();
        let mut floating_note_uids: Vec<String> = Vec::new();
        match ctx.id_index.find_graph_entry(graph_uid, "sheet.json") {
            IdLookup::Found(sheet_path) => {
                if let Some(payload) = zip_index.read_entry(&sheet_path)? {
                    match parse_bounded_json_outcome(&payload) {
                        BoundedJsonOutcome::Value(val) => {
                            if let Some(t) = val.get("title").map(|v| parse_rich_text_value(v).1).filter(|s| !s.trim().is_empty())
                                && title == *graph_uid
                            {
                                title = t;
                            }
                            if color_tag.is_none() {
                                color_tag = val.get("highlightColor").and_then(serde_json::Value::as_str).and_then(parse_archive_color);
                            }
                            media_refs = extract_media_representations(&val);
                            input_dataset_uids = extract_uid_list(&val, "inputDataSets");
                            floating_note_uids = extract_uid_list(&val, "floatingNotes");
                        }
                        BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
                        BoundedJsonOutcome::SyntaxOrType(err) => {
                            ctx.warnings.push(ImportWarning::new(
                                ImportWarningKind::MissingMetadata,
                                sheet_path,
                                format!("could not parse graph sheet JSON for `{graph_uid}`: {err}"),
                            ));
                        }
                    }
                }
            }
            IdLookup::Ambiguous(candidates) => {
                ctx.warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    graph_uid.clone(),
                    format!("ambiguous graph sheet.json paths for `{graph_uid}` ({} candidates); skipping metadata", candidates.len()),
                ));
            }
            IdLookup::Missing => {}
        }

        let mut ingested_assets: Vec<String> = Vec::new();
        if let IdLookup::Found(preview_path) = ctx.id_index.find_graph_entry(graph_uid, "preview.png")
            && let Some(asset_id) = try_ingest_zip_raster(zip_index, &preview_path, doc, ctx.warnings)?
        {
            ingested_assets.push(asset_id);
        }

        for media in &media_refs {
            if !is_supported_raster_format(&media.format) {
                continue;
            }
            if let IdLookup::Found(media_path) = ctx.id_index.find_media_entry(&media.uid, Some(&media.format))
                && let Some(asset_id) = try_ingest_zip_raster(zip_index, &media_path, doc, ctx.warnings)?
                && !ingested_assets.contains(&asset_id)
            {
                ingested_assets.push(asset_id);
            }
        }

        let has_opaque_bin = matches!(ctx.id_index.find_graph_entry(graph_uid, "data.bin"), IdLookup::Found(_));
        let preview_note = if ingested_assets.is_empty() {
            String::from("no raster preview image was present")
        } else {
            format!("ingested {} raster preview/media asset(s): {}", ingested_assets.len(), ingested_assets.join(", "))
        };
        let bin_note = if has_opaque_bin { "opaque binary graph definition (`data.bin`)" } else { "external graph definition" };

        let resolved = resolve_graph_input_bindings(&input_dataset_uids, doc, ctx.data_set_positions, ctx.data_uid_to_sheet_id);
        if let Some((primary_sheet_id, _)) = resolved.bindings.first()
            && let Some(primary_ds) = doc.data_sheet(primary_sheet_id)
        {
            let sheet_id = allocate_valid_unique_sheet_id(Some(graph_uid), "graph", idx + 1, ctx.used_ids, ctx.warnings);
            let clamped_title = clamp_title(&title, sheet_id.as_str(), ctx.warnings);
            let mut spec = build_rebuilt_graph_spec(primary_ds, &resolved.bindings, &clamped_title);
            import_graph_floating_notes(
                zip_index,
                ctx.id_index,
                graph_uid,
                &floating_note_uids,
                spec.canvas_size_pt,
                &mut spec.annotations,
                ctx.warnings,
            )?;

            match GraphSheet::new_uninitialized_for_data(sheet_id.clone(), primary_ds) {
                Ok(mut sheet) => {
                    sheet.title = clamped_title.clone();
                    sheet.initialized = true;
                    sheet.spec = spec;
                    sheet.color_tag = color_tag;
                    match doc.add_graph_sheet(sheet) {
                        Ok(()) => {
                            ctx.uid_to_sheet_id.insert(graph_uid.clone(), sheet_id);
                            let mut detail_parts: Vec<String> = Vec::new();
                            if resolved.unresolved > 0 {
                                detail_parts.push(format!("{} unresolved/non-table input dataset(s) omitted", resolved.unresolved));
                            }
                            if resolved.incompatible > 0 {
                                detail_parts.push(format!("{} incompatible-archetype input dataset(s) omitted", resolved.incompatible));
                            }
                            if resolved.capped > 0 {
                                detail_parts.push(format!("{} input dataset(s) exceeded per-graph series cap", resolved.capped));
                            }
                            let detail_suffix = if detail_parts.is_empty() { String::new() } else { format!(" ({})", detail_parts.join("; ")) };
                            ctx.warnings.push(ImportWarning::new(
                                ImportWarningKind::UnsupportedGraphOrLayout,
                                format!("graphs/{graph_uid}"),
                                format!("graph `{clamped_title}` (`{graph_uid}`) {REBUILT_GRAPH_WARNING_LABEL}{detail_suffix}; {preview_note}"),
                            ));
                        }
                        Err(err) => {
                            ctx.warnings.push(ImportWarning::new(
                                ImportWarningKind::UnsupportedGraphOrLayout,
                                format!("graphs/{graph_uid}"),
                                format!(
                                    "graph `{clamped_title}` (`{graph_uid}`) uses an {bin_note} and could not be rebuilt ({err}); {preview_note}"
                                ),
                            ));
                        }
                    }
                }
                Err(err) => {
                    ctx.warnings.push(ImportWarning::new(
                        ImportWarningKind::UnsupportedGraphOrLayout,
                        format!("graphs/{graph_uid}"),
                        format!("graph `{clamped_title}` (`{graph_uid}`) uses an {bin_note} and could not be rebuilt ({err}); {preview_note}"),
                    ));
                }
            }
            continue;
        }

        let fallback_reason = if input_dataset_uids.is_empty() {
            format!("uses an {bin_note} with no input data sets and could not be rebuilt")
        } else {
            format!(
                "uses an {bin_note} whose {} input data set(s) did not resolve to imported data tables and could not be rebuilt",
                input_dataset_uids.len()
            )
        };
        ctx.warnings.push(ImportWarning::new(
            ImportWarningKind::UnsupportedGraphOrLayout,
            format!("graphs/{graph_uid}"),
            format!("graph `{title}` (`{graph_uid}`) {fallback_reason}; {preview_note}"),
        ));
    }
    Ok(())
}

struct ResolvedGraphInputs {
    bindings: Vec<(SheetId, usize)>,
    unresolved: usize,
    incompatible: usize,
    capped: usize,
}

fn extract_uid_list(val: &serde_json::Value, key: &str) -> Vec<String> {
    let Some(arr) = val.get(key).and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(uid) = item.as_str() {
            if !uid.trim().is_empty() {
                out.push(uid.to_owned());
            }
        } else if let Some(uid) = item.get("uid").and_then(serde_json::Value::as_str)
            && !uid.trim().is_empty()
        {
            out.push(uid.to_owned());
        }
    }
    out
}

fn resolve_graph_input_bindings(
    input_dataset_uids: &[String],
    doc: &ProjectDocument,
    data_set_positions: &ExternalDataSetPositions,
    data_uid_to_sheet_id: &BTreeMap<String, SheetId>,
) -> ResolvedGraphInputs {
    let mut bindings: Vec<(SheetId, usize)> = Vec::new();
    let mut seen: HashSet<(SheetId, usize)> = HashSet::new();
    let mut primary_kind: Option<TableKind> = None;
    let mut unresolved = 0_usize;
    let mut incompatible = 0_usize;
    let mut capped = 0_usize;

    for ds_uid in input_dataset_uids {
        let Some((data_sheet_uid, col_idx)) = data_set_positions.get(ds_uid) else {
            unresolved += 1;
            continue;
        };
        let Some(sheet_id) = data_uid_to_sheet_id.get(data_sheet_uid) else {
            unresolved += 1;
            continue;
        };
        let Some(ds) = doc.data_sheet(sheet_id) else {
            unresolved += 1;
            continue;
        };
        if *col_idx >= ds.archetype.column_count() {
            unresolved += 1;
            continue;
        }
        let kind = ds.archetype.kind();
        if let Some(expected) = primary_kind {
            if kind != expected {
                incompatible += 1;
                continue;
            }
        } else {
            primary_kind = Some(kind);
        }
        let binding = (sheet_id.clone(), *col_idx);
        if seen.contains(&binding) {
            continue;
        }
        if bindings.len() >= MAX_DATA_SETS {
            capped += 1;
            continue;
        }
        seen.insert(binding.clone());
        bindings.push(binding);
    }

    ResolvedGraphInputs { bindings, unresolved, incompatible, capped }
}

fn build_rebuilt_graph_spec(primary_ds: &DataSheet, bindings: &[(SheetId, usize)], title: &str) -> GraphSpec {
    let mut spec = GraphSpec::default_for_data_sheet(primary_ds);
    spec.title = title.to_owned();

    let Some(template_series) = spec.series.first().cloned() else {
        return spec;
    };

    let mut rebuilt_series = Vec::with_capacity(bindings.len());
    for (series_pos, (source_sheet_id, data_set_index)) in bindings.iter().enumerate() {
        let swatch = spec.palette.color_at(series_pos);
        let shape = default_rebuilt_marker_shape(series_pos);
        let mut gdo = template_series.gdo.clone();
        style_rebuilt_gdo(&mut gdo, swatch, shape);
        rebuilt_series.push(SeriesSpec {
            source_sheet_id: source_sheet_id.clone(),
            data_set_index: *data_set_index,
            legend_label: None,
            bind_to_right_y2: false,
            gdo,
            y_error_bars: template_series.y_error_bars,
            x_error_bars: None,
            regression_overlay: None,
            offset: SeriesOffsetSpec::default(),
            bar_width_override_pt: None,
            show_raw_data: true,
        });
    }
    spec.series = rebuilt_series;
    spec
}

fn default_rebuilt_marker_shape(index: usize) -> MarkerShape {
    const SHAPES: [MarkerShape; 6] =
        [MarkerShape::Circle, MarkerShape::Square, MarkerShape::TriangleUp, MarkerShape::TriangleDown, MarkerShape::Diamond, MarkerShape::Hexagon];
    SHAPES.get(index % SHAPES.len()).copied().unwrap_or(MarkerShape::Circle)
}

fn style_rebuilt_gdo(gdo: &mut GraphicalDataObject, swatch: Rgba8, shape: MarkerShape) {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            if let Some(m) = marker {
                m.shape = shape;
                m.fill_color = swatch;
                m.stroke_color = swatch;
            }
            if let Some(l) = line {
                l.color = swatch;
            }
        }
        GraphicalDataObject::BarWithReplicates { fill_color, border_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, border_color, .. } => {
            *fill_color = swatch.with_alpha(fill_color.a);
            *border_color = swatch;
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            marker.shape = shape;
            marker.fill_color = swatch;
            marker.stroke_color = swatch;
        }
        GraphicalDataObject::BoxAndWhisker { fill_color, stroke_color, .. } | GraphicalDataObject::Violin { fill_color, stroke_color, .. } => {
            *fill_color = swatch.with_alpha(fill_color.a);
            *stroke_color = swatch;
        }
        GraphicalDataObject::BeforeAfterLines { marker, line_color, .. } => {
            *line_color = swatch;
            if let Some(m) = marker {
                m.shape = shape;
                m.fill_color = swatch;
                m.stroke_color = swatch;
            }
        }
        GraphicalDataObject::KaplanMeierStep { line_color, .. } => {
            *line_color = swatch;
        }
        GraphicalDataObject::PartsOfWhole { .. } => {}
    }
}

fn import_graph_floating_notes(
    zip_index: &mut ZipIndex<'_>,
    id_index: &ArchiveIdIndex,
    graph_uid: &str,
    note_uids: &[String],
    canvas_size_pt: SizePt,
    annotations: &mut Vec<GraphAnnotation>,
    warnings: &mut Vec<ImportWarning>,
) -> Result<(), FormatError> {
    let mut seen_notes: HashSet<&str> = HashSet::new();
    for note_uid in note_uids {
        if !seen_notes.insert(note_uid.as_str()) {
            continue;
        }
        if annotations.len() >= MAX_GRAPH_ANNOTATIONS {
            warnings.push(ImportWarning::new(
                ImportWarningKind::MissingMetadata,
                format!("graphs/{graph_uid}"),
                format!("graph `{graph_uid}` has more than {MAX_GRAPH_ANNOTATIONS} floating notes; remaining notes were omitted"),
            ));
            break;
        }

        let note_path = match id_index.find_graph_floating_note_json(graph_uid, note_uid) {
            IdLookup::Found(p) => p,
            IdLookup::Missing => {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    format!("graphs/{graph_uid}/floating_notes/{note_uid}"),
                    format!("floating note `{note_uid}` referenced by graph `{graph_uid}` was not found in archive"),
                ));
                continue;
            }
            IdLookup::Ambiguous(candidates) => {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    format!("graphs/{graph_uid}/floating_notes/{note_uid}"),
                    format!("ambiguous floating note paths for `{note_uid}` on graph `{graph_uid}` ({} candidates); skipping note", candidates.len()),
                ));
                continue;
            }
        };

        let Some(payload) = zip_index.read_entry(&note_path)? else {
            continue;
        };
        let note_val = match parse_bounded_json_outcome(&payload) {
            BoundedJsonOutcome::Value(v) => v,
            BoundedJsonOutcome::BudgetExceeded(err) => return Err(err),
            BoundedJsonOutcome::SyntaxOrType(err) => {
                warnings.push(ImportWarning::new(
                    ImportWarningKind::MissingMetadata,
                    note_path,
                    format!("could not parse floating note JSON `{note_uid}` on graph `{graph_uid}`: {err}"),
                ));
                continue;
            }
        };

        let Some(text_val) = note_val.get("text") else {
            continue;
        };
        let (markup_raw, plain_raw) = parse_rich_text_value(text_val);
        if plain_raw.trim().is_empty() {
            continue;
        }
        let normalized = normalize_note_markdown(&markup_raw);
        if normalized.is_empty() {
            continue;
        }
        let markdown = clamp_utf8_bytes(&normalized, MAX_TITLE_BYTES);
        let bounds_pt = compute_note_annotation_bounds(&note_val, &markdown, canvas_size_pt, annotations.len());
        annotations.push(GraphAnnotation::TextBox { bounds_pt, markdown, bordered: true });
    }
    Ok(())
}

fn normalize_note_markdown(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n").trim().to_owned()
}

fn clamp_utf8_bytes(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default().to_owned()
}

fn compute_note_annotation_bounds(note_val: &serde_json::Value, markdown: &str, canvas_size_pt: SizePt, note_idx: usize) -> RectPt {
    let canvas_w = if canvas_size_pt.width.is_finite() && canvas_size_pt.width > 0.0 { canvas_size_pt.width } else { 360.0 };
    let canvas_h = if canvas_size_pt.height.is_finite() && canvas_size_pt.height > 0.0 { canvas_size_pt.height } else { 260.0 };

    let max_line_chars = markdown.lines().map(|line| line.chars().count()).max().unwrap_or(1);
    let line_count = markdown.lines().count().max(1);

    let max_w = (canvas_w - 20.0).clamp(60.0, 220.0);
    let max_h = (canvas_h - 36.0).clamp(22.0, 96.0);
    let width = ((max_line_chars as f64) * 4.5 + 14.0).clamp(60.0_f64.min(max_w), max_w);
    let height = ((line_count as f64) * 11.5 + 10.0).clamp(22.0_f64.min(max_h), max_h);

    let x_min = 10.0_f64.min((canvas_w - width).max(0.0));
    let x_max = (canvas_w - width - 10.0).max(x_min);
    let y_min = 26.0_f64.min((canvas_h - height).max(0.0));
    let y_max = (canvas_h - height - 10.0).max(y_min);

    if let Some((px, py)) = parse_note_position(note_val.get("position"))
        && let Some((sw, sh)) = parse_screen_size(note_val.get("screenWidth"))
    {
        let rx = (px / sw).clamp(0.0, 1.0);
        let ry = (py / sh).clamp(0.0, 1.0);
        let x = x_min + rx * (x_max - x_min);
        let y = y_min + ry * (y_max - y_min);
        return RectPt::new(x, y, width, height);
    }

    let step = (height + 6.0).max(28.0);
    let y_span = (y_max - y_min).max(1.0);
    let y = y_min + (((note_idx as f64) * step) % y_span);
    RectPt::new(x_min, y.clamp(y_min, y_max), width, height)
}

fn parse_note_position(val: Option<&serde_json::Value>) -> Option<(f64, f64)> {
    let raw = val?.as_str()?.trim();
    let (xs, ys) = raw.split_once(';')?;
    let x = xs.trim().parse::<f64>().ok()?;
    let y = ys.trim().parse::<f64>().ok()?;
    if x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 { Some((x, y)) } else { None }
}

fn parse_screen_size(val: Option<&serde_json::Value>) -> Option<(f64, f64)> {
    let obj = val?.as_object()?;
    let w = obj.get("width")?.as_f64()?;
    let h = obj.get("height")?.as_f64()?;
    if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 { Some((w, h)) } else { None }
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
