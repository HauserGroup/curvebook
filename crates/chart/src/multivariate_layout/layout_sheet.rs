//! Multi-panel publication `LayoutSheet` scene compilation, origin alignment, and budget guards.

use std::collections::BTreeMap;

use curvebook_doc::{LayoutItem, LayoutSheet, MAX_IMAGE_PIXELS, ProjectDocument, SheetId};
use curvebook_palette::{LineDashStyle, Rgba8};
use curvebook_text::parse_markup;

use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PathCmd, PlottedPointMeta, PointPt, PositionedText, RectPt, ScenePrimitive, TextAnchor,
};
use crate::survival_cat::{resolve_graph_text, solid_stroke};
use crate::xy::{EmbeddedTableDrawSpec, render_styled_embedded_table};

// ============================================================================
// 3. Multi-Panel LayoutSheet Compilation (Gap G11)
// ============================================================================

/// Hard ceiling on total emitted scene primitives within a compiled multi-panel layout.
pub const MAX_LAYOUT_PRIMITIVES: usize = 500_000;

struct CompiledGraphEntry {
    scene: Option<ChartScene>,
    warning: Option<(&'static str, String)>,
}

struct PlacedPanel {
    panel_idx: usize,
    bounds: RectPt,
    badge: Option<String>,
    cache_idx: Option<usize>,
    scale: f64,
    tx: f64,
    ty: f64,
}

pub(crate) fn check_layout_primitive_budget(current: usize, added: usize, context: &str) -> Result<(), ChartError> {
    let next = current.saturating_add(added);
    if next > MAX_LAYOUT_PRIMITIVES {
        return Err(ChartError::ResourceLimit { message: format!("{context} primitive count {next} exceeds layout budget {MAX_LAYOUT_PRIMITIVES}") });
    }
    Ok(())
}

enum LayoutPaintOrder<'a> {
    Panel(usize),
    Other(&'a LayoutItem),
}

/// Compile a multi-panel [`LayoutSheet`] into a unified physical-page [`ChartScene`].
pub fn compile_layout_scene(layout: &LayoutSheet, doc: &ProjectDocument) -> Result<ChartScene, ChartError> {
    layout.spec.validate_item_count().map_err(|err| ChartError::ResourceLimit { message: err.to_string() })?;

    let page_pt = layout.spec.page_size_pt();
    let mut scene = ChartScene::new(layout.id.as_str(), layout.title.clone(), page_pt.width, page_pt.height, layout.spec.background_fill);

    let mut compiled_graphs: Vec<CompiledGraphEntry> = Vec::new();
    let mut graph_cache: BTreeMap<&SheetId, usize> = BTreeMap::new();
    let mut panels: Vec<PlacedPanel> = Vec::new();
    let mut paint_order: Vec<LayoutPaintOrder<'_>> = Vec::with_capacity(layout.spec.items.len());
    let mut panel_counter = 0_usize;

    for item in &layout.spec.items {
        match item {
            LayoutItem::LiveGraph { graph_sheet_id, bounds_pt, scale_percent, badge_override, .. } => {
                let bounds = RectPt::from(*bounds_pt);
                let badge = badge_override.clone().or_else(|| layout.spec.panel_badges.badge_for(panel_counter));
                let cache_idx = if let Some(&existing) = graph_cache.get(graph_sheet_id) {
                    existing
                } else {
                    let entry = match doc.graph_sheet(graph_sheet_id) {
                        Some(gs) => match crate::compile_graph(gs, doc) {
                            Ok(compiled) => CompiledGraphEntry { scene: Some(compiled), warning: None },
                            Err(e) => CompiledGraphEntry { scene: None, warning: Some(("panel_compile_error", e.to_string())) },
                        },
                        None => CompiledGraphEntry {
                            scene: None,
                            warning: Some(("missing_layout_graph", format!("referenced graph sheet '{graph_sheet_id}' not found"))),
                        },
                    };
                    let idx = compiled_graphs.len();
                    compiled_graphs.push(entry);
                    graph_cache.insert(graph_sheet_id, idx);
                    idx
                };
                if let Some(entry) = compiled_graphs.get(cache_idx)
                    && let Some((code, message)) = &entry.warning
                {
                    scene.warnings.push(ChartWarning::new(*code, message.clone()));
                }
                let scale = compiled_graphs.get(cache_idx).and_then(|entry| entry.scene.as_ref()).map_or(1.0, |sub| {
                    let pct = if scale_percent.is_finite() && *scale_percent > 0.0 { *scale_percent / 100.0 } else { 1.0 };
                    let fit = (bounds.width / sub.width_pt.max(1.0)).min(bounds.height / sub.height_pt.max(1.0));
                    (fit * pct).max(0.05)
                });
                let p_vec_idx = panels.len();
                panels.push(PlacedPanel { panel_idx: panel_counter, bounds, badge, cache_idx: Some(cache_idx), scale, tx: bounds.x, ty: bounds.y });
                paint_order.push(LayoutPaintOrder::Panel(p_vec_idx));
                panel_counter += 1;
            }
            LayoutItem::LiveTable { .. } | LayoutItem::TextCallout { .. } | LayoutItem::EmbeddedImage { .. } => {
                paint_order.push(LayoutPaintOrder::Other(item));
            }
        }
    }

    if layout.spec.align_plot_origins {
        align_placed_panel_origins(&mut panels, &compiled_graphs);
    }

    let mut extra_table_work: usize = 0;
    let mut layout_image_pixels: u64 = 0;
    let mut image_cache = crate::image_decode::DecodedImageCache::default();
    for entry in paint_order {
        match entry {
            LayoutPaintOrder::Panel(p_idx) => {
                let panel = &panels[p_idx];
                let sub = panel.cache_idx.and_then(|idx| compiled_graphs.get(idx)).and_then(|entry| entry.scene.as_ref());
                emit_placed_panel(&mut scene, panel, sub, &mut layout_image_pixels)?;
            }
            LayoutPaintOrder::Other(LayoutItem::LiveTable { source_sheet_id, tab_id, bounds_pt, selected_rows, selected_cols, style, .. }) => {
                render_styled_embedded_table(
                    &mut scene,
                    doc,
                    &EmbeddedTableDrawSpec {
                        sheet_id: source_sheet_id,
                        tab_id: tab_id.as_deref(),
                        bounds: RectPt::from(*bounds_pt),
                        selected_rows: selected_rows.as_deref(),
                        selected_cols: selected_cols.as_deref(),
                        style,
                        missing_warning_code: "missing_layout_table",
                    },
                    Some(&mut extra_table_work),
                )?;
            }
            LayoutPaintOrder::Other(LayoutItem::TextCallout { markdown, bounds_pt, font_size_pt, .. }) => {
                let expanded = resolve_graph_text(markdown, doc, None);
                let bounds = RectPt::from(*bounds_pt);
                let fs = if font_size_pt.is_finite() && *font_size_pt > 0.0 { *font_size_pt } else { 10.0 };
                let line_h = fs * 1.3;
                let line_count = expanded.lines().count();
                check_layout_primitive_budget(
                    scene.primitives.len().saturating_add(extra_table_work),
                    line_count.saturating_add(2),
                    "layout text callout",
                )?;
                scene.primitives.push(ScenePrimitive::PushClip { rect: bounds });
                for (line_idx, line) in expanded.lines().enumerate() {
                    scene.primitives.push(ScenePrimitive::Text(PositionedText {
                        rich: parse_markup(line),
                        anchor_pt: PointPt::new(bounds.x, bounds.y + fs + (line_idx as f64) * line_h),
                        anchor: TextAnchor::Start,
                        font_size_pt: fs,
                        rotation_deg: 0.0,
                        color: Rgba8::BLACK,
                    }));
                }
                scene.primitives.push(ScenePrimitive::PopClip);
            }
            LayoutPaintOrder::Other(LayoutItem::EmbeddedImage { asset_id, bounds_pt, rotation_deg, crop, border, badge_override, .. }) => {
                check_layout_primitive_budget(scene.primitives.len().saturating_add(extra_table_work), 3, "layout embedded image")?;
                crate::image_decode::emit_embedded_image_primitive_cached(
                    &mut scene,
                    doc,
                    &crate::image_decode::EmbeddedImageDrawSpec {
                        asset_id,
                        bounds_pt: *bounds_pt,
                        rotation_deg: *rotation_deg,
                        crop: *crop,
                        border: *border,
                        clip: None,
                        badge_override: badge_override.as_deref(),
                    },
                    Some(&mut layout_image_pixels),
                    Some(&mut image_cache),
                );
            }
            LayoutPaintOrder::Other(LayoutItem::LiveGraph { .. }) => {}
        }
    }

    Ok(scene)
}

/// Align inner `plot_rect_pt` origins across panels sharing a layout column (X) or layout row (Y).
fn align_placed_panel_origins(panels: &mut [PlacedPanel], compiled_graphs: &[CompiledGraphEntry]) {
    let sub_scene_of = |panel: &PlacedPanel| -> Option<&ChartScene> {
        panel.cache_idx.and_then(|idx| compiled_graphs.get(idx)).and_then(|entry| entry.scene.as_ref())
    };
    let mut col_targets: Vec<Option<f64>> = vec![None; panels.len()];
    for i in 0..panels.len() {
        let Some(sub_i) = sub_scene_of(&panels[i]) else { continue };
        let bx_i = panels[i].bounds.x;
        let peers: Vec<usize> =
            (0..panels.len()).filter(|&j| sub_scene_of(&panels[j]).is_some() && (panels[j].bounds.x - bx_i).abs() <= 16.0).collect();
        if peers.len() >= 2 {
            let max_origin_x = peers
                .iter()
                .filter_map(|&j| sub_scene_of(&panels[j]).map(|s| panels[j].bounds.x + panels[j].scale * s.plot_rect_pt.x))
                .fold(bx_i + panels[i].scale * sub_i.plot_rect_pt.x, f64::max);
            col_targets[i] = Some(max_origin_x);
            let avail_right = (panels[i].bounds.right() - max_origin_x).max(8.0);
            let span_right = (sub_i.width_pt - sub_i.plot_rect_pt.x).max(1.0);
            panels[i].scale = panels[i].scale.min(avail_right / span_right).max(0.05);
        }
    }

    let mut row_targets: Vec<Option<f64>> = vec![None; panels.len()];
    for i in 0..panels.len() {
        let Some(sub_i) = sub_scene_of(&panels[i]) else { continue };
        let by_i = panels[i].bounds.y;
        let peers: Vec<usize> =
            (0..panels.len()).filter(|&j| sub_scene_of(&panels[j]).is_some() && (panels[j].bounds.y - by_i).abs() <= 16.0).collect();
        if peers.len() >= 2 {
            let max_origin_y = peers
                .iter()
                .filter_map(|&j| sub_scene_of(&panels[j]).map(|s| panels[j].bounds.y + panels[j].scale * s.plot_rect_pt.y))
                .fold(by_i + panels[i].scale * sub_i.plot_rect_pt.y, f64::max);
            row_targets[i] = Some(max_origin_y);
            let avail_bot = (panels[i].bounds.bottom() - max_origin_y).max(8.0);
            let span_bot = (sub_i.height_pt - sub_i.plot_rect_pt.y).max(1.0);
            panels[i].scale = panels[i].scale.min(avail_bot / span_bot).max(0.05);
        }
    }

    for (i, panel) in panels.iter_mut().enumerate() {
        let Some(sub) = sub_scene_of(panel) else { continue };
        if let Some(target_x) = col_targets.get(i).copied().flatten() {
            panel.tx = target_x - panel.scale * sub.plot_rect_pt.x;
        }
        if let Some(target_y) = row_targets.get(i).copied().flatten() {
            panel.ty = target_y - panel.scale * sub.plot_rect_pt.y;
        }
    }
}

fn emit_placed_panel(scene: &mut ChartScene, panel: &PlacedPanel, sub_scene: Option<&ChartScene>, image_pixels: &mut u64) -> Result<(), ChartError> {
    let badge_prims = usize::from(panel.badge.is_some());
    let content_prims = sub_scene.map_or(1, |sub| usize::from(sub.background.is_some()).saturating_add(sub.primitives.len()));
    let added_prims = 2_usize.saturating_add(content_prims).saturating_add(badge_prims);
    check_layout_primitive_budget(scene.primitives.len(), added_prims, "layout panel")?;

    if let Some(sub) = sub_scene {
        let next_points = scene.plotted_points.len().saturating_add(sub.plotted_points.len());
        if next_points > MAX_LAYOUT_PRIMITIVES {
            return Err(ChartError::ResourceLimit {
                message: format!("layout plotted point metadata count {next_points} exceeds layout budget {MAX_LAYOUT_PRIMITIVES}"),
            });
        }
    }

    let bounds = panel.bounds;
    scene.primitives.push(ScenePrimitive::PushClip { rect: bounds });

    if let Some(sub) = sub_scene {
        if let Some(bg) = sub.background {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(RectPt::new(panel.tx, panel.ty, sub.width_pt * panel.scale, sub.height_pt * panel.scale)),
                fill: Some(FillPaint::solid(bg)),
                stroke: None,
                clip: None,
            });
        }
        for prim in &sub.primitives {
            if let ScenePrimitive::RasterImage(img) = prim {
                let declared_px = u64::from(img.pixel_size.0).saturating_mul(u64::from(img.pixel_size.1));
                let buf_px = (img.rgba.len() / 4) as u64;
                let px = declared_px.max(buf_px);
                let next = image_pixels.saturating_add(px);
                if next > MAX_IMAGE_PIXELS {
                    scene.warnings.push(ChartWarning::new(
                        "embedded_image_budget_exceeded",
                        format!("panel {} embedded image `{}` exceeds cumulative pixel budget {MAX_IMAGE_PIXELS}", panel.panel_idx + 1, img.asset_id),
                    ));
                    continue;
                }
                *image_pixels = next;
            }
            scene.primitives.push(transform_primitive(prim, panel.tx, panel.ty, panel.scale));
        }
        for p in &sub.plotted_points {
            scene.plotted_points.push(PlottedPointMeta {
                center_pt: transform_point(p.center_pt, panel.tx, panel.ty, panel.scale),
                radius_pt: (p.radius_pt * panel.scale).max(2.0),
                source_cell: p.source_cell.clone(),
                series_label: p.series_label.clone(),
                value_summary: p.value_summary.clone(),
            });
        }
        for w in &sub.warnings {
            scene.warnings.push(ChartWarning::new(w.code.clone(), format!("panel {}: {}", panel.panel_idx + 1, w.message)));
        }
    } else {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(bounds),
            fill: None,
            stroke: Some(solid_stroke(Rgba8::rgb(180, 180, 180), 0.75, LineDashStyle::Solid)),
            clip: None,
        });
    }

    scene.primitives.push(ScenePrimitive::PopClip);

    if let Some(label) = &panel.badge {
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&format!("<b>{label}</b>")),
            anchor_pt: PointPt::new(bounds.x + 4.0, bounds.y + 12.0),
            anchor: TextAnchor::Start,
            font_size_pt: 11.5,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
    Ok(())
}

fn transform_point(p: PointPt, tx: f64, ty: f64, s: f64) -> PointPt {
    PointPt::new(tx + p.x * s, ty + p.y * s)
}

fn transform_rect(r: RectPt, tx: f64, ty: f64, s: f64) -> RectPt {
    RectPt::new(tx + r.x * s, ty + r.y * s, r.width * s, r.height * s)
}

fn transform_primitive(prim: &ScenePrimitive, tx: f64, ty: f64, s: f64) -> ScenePrimitive {
    match prim {
        ScenePrimitive::Path { path, fill, stroke, clip } => {
            let cmds = path
                .commands
                .iter()
                .map(|c| match *c {
                    PathCmd::MoveTo(p) => PathCmd::MoveTo(transform_point(p, tx, ty, s)),
                    PathCmd::LineTo(p) => PathCmd::LineTo(transform_point(p, tx, ty, s)),
                    PathCmd::QuadTo { control, to } => {
                        PathCmd::QuadTo { control: transform_point(control, tx, ty, s), to: transform_point(to, tx, ty, s) }
                    }
                    PathCmd::CubicTo { c1, c2, to } => PathCmd::CubicTo {
                        c1: transform_point(c1, tx, ty, s),
                        c2: transform_point(c2, tx, ty, s),
                        to: transform_point(to, tx, ty, s),
                    },
                    PathCmd::Close => PathCmd::Close,
                })
                .collect();
            let mut scaled_stroke = *stroke;
            if let Some(st) = &mut scaled_stroke {
                st.width_pt = (st.width_pt * s).max(0.25);
            }
            ScenePrimitive::Path {
                path: Path2D { commands: cmds },
                fill: *fill,
                stroke: scaled_stroke,
                clip: clip.map(|c| transform_rect(c, tx, ty, s)),
            }
        }
        ScenePrimitive::Text(t) => ScenePrimitive::Text(PositionedText {
            rich: t.rich.clone(),
            anchor_pt: transform_point(t.anchor_pt, tx, ty, s),
            anchor: t.anchor,
            font_size_pt: (t.font_size_pt * s).max(4.0),
            rotation_deg: t.rotation_deg,
            color: t.color,
        }),
        ScenePrimitive::RasterImage(img) => ScenePrimitive::RasterImage(crate::scene::SceneRasterImage {
            asset_id: img.asset_id.clone(),
            media_type: img.media_type,
            pixel_size: img.pixel_size,
            rgba: img.rgba.clone(),
            bounds_pt: transform_rect(img.bounds_pt, tx, ty, s),
            crop: curvebook_doc::CropInsetsPt {
                left_pt: img.crop.left_pt * s,
                right_pt: img.crop.right_pt * s,
                top_pt: img.crop.top_pt * s,
                bottom_pt: img.crop.bottom_pt * s,
            },
            rotation_deg: img.rotation_deg,
            clip: img.clip.map(|c| transform_rect(c, tx, ty, s)),
        }),
        ScenePrimitive::PushClip { rect } => ScenePrimitive::PushClip { rect: transform_rect(*rect, tx, ty, s) },
        ScenePrimitive::PopClip => ScenePrimitive::PopClip,
    }
}
