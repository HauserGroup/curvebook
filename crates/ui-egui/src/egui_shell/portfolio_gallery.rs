//! Portfolio Gallery center-stage browser (`R10.35`): live scene/table thumbnails,
//! selection & range sync, collapsible folder sections (`NavigatorOrder` / `Type` vs flat
//! `NameAsc` / `SeriesOrRows`), batch workflows, card context menu, and keyboard grid nav.

use std::collections::BTreeSet;

use curvebook_doc::{ProjectDocument, SheetFolderKind, SheetId, TableArchetype};
use egui::{Color32, CornerRadius, FontId, Id, Key, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2};
use serde_json::json;

use crate::UiApp;
use crate::egui_shell::center_stage::inspect_input;
use crate::egui_shell::ui_kit::{ActionMenuAnchor, ActionMenuEntry, ActionMenuItem, ActionMenuSpec, TooltipSpec, action_menu_popup, show_tooltip};
use crate::egui_shell::widgets::dropdown;
use crate::egui_shell::{
    EguiShellViewState, FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, UiIntent, doc_sheet_title, rgba8_to_color32,
};
use crate::i18n::{tr, tr_or_fallback};
use crate::inspect::ZLayer;
use crate::workspace_nav::{GalleryFolderCounts, GallerySortOrder, PortfolioCard};

pub(super) struct GalleryFrameIo<'a> {
    pub(super) views: &'a mut EguiShellViewState,
    pub(super) meta: &'a mut FrameInspectMeta,
    pub(super) intents: &'a mut PendingUiIntents,
}

pub(super) const GALLERY_DEFAULT_CARD_W_PT: f32 = 148.0;

/// Compute a responsive Portfolio Gallery card width so visible cards fit neatly across `stage_w_pt`.
#[must_use]
pub(super) fn compute_gallery_fit_zoom_pt(stage_w_pt: f32, card_count: usize) -> f32 {
    let inner_w = (stage_w_pt - 32.0).max(140.0);
    let spacing = 10.0_f32;
    let max_cols = ((inner_w + spacing) / (150.0 + spacing)).floor().clamp(1.0, 6.0) as usize;
    let target_cols = if card_count == 0 { max_cols.min(3) } else { card_count.min(max_cols).max(1) };
    let fitted = (inner_w - spacing * (target_cols.saturating_sub(1) as f32)) / (target_cols as f32);
    fitted.clamp(120.0, 260.0)
}

/// Active right-click context menu anchored to a Portfolio Gallery card (`R10.35` / `R20.16`).
#[derive(Debug, Clone, PartialEq)]
pub struct GalleryContextMenuState {
    pub sheet_id: SheetId,
    pub anchor_pos: Pos2,
    pub folder: SheetFolderKind,
    pub is_frozen: bool,
}

/// Active inline card rename draft inside Portfolio Gallery (`R10.35`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GalleryInlineRenameState {
    pub sheet_id: SheetId,
    pub draft: String,
    pub request_focus: bool,
}

fn gallery_folder_slug(folder: SheetFolderKind) -> &'static str {
    match folder {
        SheetFolderKind::Data => "data",
        SheetFolderKind::Info => "info",
        SheetFolderKind::Results => "results",
        SheetFolderKind::Graphs => "graphs",
        SheetFolderKind::Layouts => "layouts",
    }
}

const fn gallery_folder_section_label_key(folder: SheetFolderKind) -> &'static str {
    match folder {
        SheetFolderKind::Data => "nav.data_tables",
        SheetFolderKind::Info => "nav.info",
        SheetFolderKind::Results => "nav.results",
        SheetFolderKind::Graphs => "nav.graphs",
        SheetFolderKind::Layouts => "nav.layouts",
    }
}

const GALLERY_NAV_SECTION_ORDER: [SheetFolderKind; 5] =
    [SheetFolderKind::Data, SheetFolderKind::Info, SheetFolderKind::Results, SheetFolderKind::Graphs, SheetFolderKind::Layouts];

const GALLERY_TYPE_SECTION_ORDER: [SheetFolderKind; 5] =
    [SheetFolderKind::Graphs, SheetFolderKind::Layouts, SheetFolderKind::Data, SheetFolderKind::Results, SheetFolderKind::Info];

struct GalleryBtnSpec<'a> {
    widget_id: &'a str,
    label: &'a str,
    tooltip: &'a str,
    enabled: bool,
    selected: bool,
    command_id: Option<&'a str>,
    disabled_reason: Option<&'a str>,
}

fn gallery_action_button(ui: &mut egui::Ui, tokens: &Tokens, spec: GalleryBtnSpec<'_>, meta: &mut FrameInspectMeta) -> egui::Response {
    let min_h = tokens.metrics.min_hit_target_pt;
    let fill = if spec.selected { tokens.accent } else { tokens.panel_bg };
    let fg = if !spec.enabled {
        tokens.text_disabled
    } else if spec.selected {
        tokens.on_accent
    } else {
        tokens.text_primary
    };
    let stroke_col = if spec.selected { tokens.accent } else { tokens.border };
    let btn = egui::Button::new(RichText::new(spec.label).small().color(fg))
        .min_size(Vec2::new(min_h, min_h))
        .fill(fill)
        .stroke(Stroke::new(tokens.metrics.stroke_thin_pt, stroke_col));
    let mut resp = ui.add_enabled(spec.enabled, btn);
    let tip = if !spec.enabled { spec.disabled_reason.unwrap_or(spec.tooltip) } else { spec.tooltip };
    resp = show_tooltip(&resp, tokens, TooltipSpec::new(tip));
    meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: spec.widget_id,
            role: "button",
            label: spec.label,
            command_id: spec.command_id,
            value: spec.selected.then_some(json!(true)),
            enabled: spec.enabled,
            disabled_reason: (!spec.enabled).then(|| tip.to_owned()).as_deref(),
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );
    resp
}

fn trigger_gallery_delete(
    doc: &ProjectDocument,
    locale: &str,
    targets: &[SheetId],
    views: &mut EguiShellViewState,
    intents: &mut PendingUiIntents,
    ctx: &egui::Context,
) {
    if targets.is_empty() {
        return;
    }
    let roots = super::leading_dock::deduplicate_delete_roots(doc, targets);
    let dependents = super::leading_dock::collect_multi_dependents(doc, &roots);
    if !dependents.is_empty() {
        let titles: Vec<String> = targets.iter().map(|id| doc_sheet_title(doc, id).unwrap_or(id.as_str()).to_owned()).collect();
        let summary = if targets.len() == 1 {
            doc_sheet_title(doc, &roots[0]).unwrap_or(roots[0].as_str()).to_owned()
        } else {
            format!("{} {}", targets.len(), tr(locale, "nav.selected_suffix"))
        };
        views.leading_dock.delete_confirm = Some(super::leading_dock::NavigatorDeleteConfirmState {
            sheet_id: roots[0].clone(),
            sheet_ids: roots,
            title: summary,
            titles,
            dependents,
        });
        ctx.request_repaint();
    } else {
        let cmds: Vec<(String, serde_json::Value)> =
            roots.iter().map(|id| (String::from("sheet.delete"), json!({ "sheet_id": id.as_str() }))).collect();
        intents.push(UiIntent::EngineCommandGroup { commands: cmds });
    }
}

fn gallery_text_item_width(ui: &egui::Ui, tokens: &Tokens, label: &str) -> f32 {
    let min_h = tokens.metrics.min_hit_target_pt;
    let text_w = egui::WidgetText::from(RichText::new(label).small())
        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button)
        .size()
        .x;
    (text_w + ui.spacing().button_padding.x * 2.0 + 4.0).max(min_h)
}

fn pack_flow_rows(widths: &[f32], max_w: f32, gap: f32) -> Vec<std::ops::Range<usize>> {
    let mut rows = Vec::new();
    let mut start = 0usize;
    let mut used = 0.0_f32;
    let limit = max_w.max(120.0);
    for (idx, &w) in widths.iter().enumerate() {
        let step = if idx == start { w } else { gap + w };
        if idx > start && used + step > limit {
            rows.push(start..idx);
            start = idx;
            used = w;
        } else {
            used += step;
        }
    }
    if start < widths.len() {
        rows.push(start..widths.len());
    }
    rows
}

struct GalleryPrimaryToolbarSpec<'a> {
    inner_w: f32,
    tokens: &'a Tokens,
    enabled: bool,
    visible_ids: &'a [SheetId],
    selected_count: usize,
    counts: GalleryFolderCounts,
}

fn render_gallery_primary_toolbar(ui: &mut egui::Ui, app: &UiApp, spec: GalleryPrimaryToolbarSpec<'_>, io: &mut GalleryFrameIo<'_>) {
    let GalleryPrimaryToolbarSpec { inner_w, tokens, enabled, visible_ids, selected_count, counts } = spec;
    let GalleryFrameIo { views, meta, intents } = io;
    let locale = app.locale.as_str();
    let gap = tokens.metrics.space_xs + 1.0;

    let heading = tr_or_fallback(locale, "w5.gallery.title", "Portfolio Gallery");
    let heading_w = egui::WidgetText::from(RichText::new(heading).strong())
        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body)
        .size()
        .x
        + 6.0;

    let sel_lbl = (selected_count > 0).then(|| format!("{selected_count} {}", tr(locale, "nav.selected_suffix")));
    let sel_badge_w = sel_lbl.as_deref().map(|lbl| {
        egui::WidgetText::from(RichText::new(lbl).small().strong())
            .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Small)
            .size()
            .x
            + tokens.metrics.space_sm * 2.0
            + 6.0
    });

    let sel_all_lbl = tr_or_fallback(locale, "gallery.select_all", "Select All");
    let sel_all_enabled = enabled && !visible_ids.is_empty() && selected_count < visible_ids.len();
    let clear_lbl = tr_or_fallback(locale, "gallery.clear_selection", "Clear");
    let clear_enabled = enabled && selected_count > 0;
    let search_hint = tr_or_fallback(locale, "w5.gallery.search", "Search sheets");
    let search_desired_w = (inner_w * 0.16).clamp(88.0, 128.0);

    let filter_defs: [(Option<SheetFolderKind>, &str, &str, &str); 6] = [
        (None, "all", "gallery.filter.all", "gallery.filter.all_label"),
        (Some(SheetFolderKind::Graphs), "graphs", "gallery.filter.graphs", "nav.graphs"),
        (Some(SheetFolderKind::Layouts), "layouts", "gallery.filter.layouts", "nav.layouts"),
        (Some(SheetFolderKind::Data), "data", "gallery.filter.data", "nav.data_tables"),
        (Some(SheetFolderKind::Results), "results", "gallery.filter.results", "nav.results"),
        (Some(SheetFolderKind::Info), "info", "gallery.filter.info", "nav.info"),
    ];
    let filter_labels: Vec<String> = filter_defs
        .iter()
        .map(|(folder_opt, _, _, key)| {
            let count = counts.for_filter(*folder_opt);
            let base = if *key == "gallery.filter.all_label" { tr_or_fallback(locale, key, "All") } else { tr(locale, key) };
            format!("{base} ({count})")
        })
        .collect();

    let family_active = app.nav.gallery.current_family_only;
    let fam_lbl = tr_or_fallback(locale, "gallery.filter.current_family", "Current Family");
    let sort_prefix = tr_or_fallback(locale, "gallery.sort.label", "Sort");
    let cur_sort = app.nav.gallery.sort_order;
    let sort_text = format!("{sort_prefix}: {}", tr_or_fallback(locale, cur_sort.label_key(), cur_sort.slug()));
    let close_lbl = tr_or_fallback(locale, "w5.gallery.close", "Close Gallery");

    #[derive(Clone, Copy)]
    enum PrimaryToken {
        Heading,
        SelectionBadge,
        SelectAll,
        Clear,
        Search,
        Filter(usize),
        CurrentFamily,
        Sort,
        ZoomSlider,
        Close,
    }

    let mut items = Vec::with_capacity(15);
    let mut widths = Vec::with_capacity(15);
    items.push(PrimaryToken::Heading);
    widths.push(heading_w);
    if let Some(bw) = sel_badge_w {
        items.push(PrimaryToken::SelectionBadge);
        widths.push(bw);
    }
    items.push(PrimaryToken::SelectAll);
    widths.push(gallery_text_item_width(ui, tokens, sel_all_lbl));
    items.push(PrimaryToken::Clear);
    widths.push(gallery_text_item_width(ui, tokens, clear_lbl));
    items.push(PrimaryToken::Search);
    widths.push(search_desired_w + 10.0);
    for (idx, lbl) in filter_labels.iter().enumerate() {
        items.push(PrimaryToken::Filter(idx));
        widths.push(gallery_text_item_width(ui, tokens, lbl));
    }
    items.push(PrimaryToken::CurrentFamily);
    widths.push(gallery_text_item_width(ui, tokens, fam_lbl));
    items.push(PrimaryToken::Sort);
    widths.push(146.0);
    items.push(PrimaryToken::ZoomSlider);
    widths.push(84.0);
    items.push(PrimaryToken::Close);
    widths.push(gallery_text_item_width(ui, tokens, close_lbl));

    let rows = pack_flow_rows(&widths, inner_w, gap);
    for (row_idx, row_range) in rows.into_iter().enumerate() {
        if row_idx > 0 {
            ui.add_space(tokens.metrics.space_xs);
        }
        ui.horizontal(|bar| {
            bar.spacing_mut().item_spacing.x = gap;
            bar.spacing_mut().slider_width = 76.0;
            for item_idx in row_range {
                match items[item_idx] {
                    PrimaryToken::Heading => {
                        bar.label(RichText::new(heading).strong().color(tokens.text_strong));
                    }
                    PrimaryToken::SelectionBadge => {
                        if let Some(ref lbl_text) = sel_lbl {
                            let badge_frame = egui::Frame::NONE
                                .fill(tokens.surface_recessed)
                                .stroke(Stroke::new(tokens.metrics.stroke_thin_pt, tokens.border))
                                .corner_radius(CornerRadius::same(tokens.metrics.radius_md))
                                .inner_margin(egui::Margin::symmetric(tokens.metrics.space_sm as i8, 3));
                            let lbl_resp = badge_frame
                                .show(bar, |bui| {
                                    let r = bui
                                        .add(egui::Label::new(RichText::new(lbl_text).small().strong().color(tokens.text_strong)).selectable(false));
                                    let a11y = lbl_text.clone();
                                    r.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Label, true, a11y.clone()));
                                    r
                                })
                                .inner;
                            meta.record_response(
                                bar,
                                &lbl_resp,
                                inspect_input(
                                    "gallery.selection_count",
                                    "Label",
                                    lbl_text,
                                    None,
                                    Some(json!(selected_count)),
                                    true,
                                    ZLayer::Workspace,
                                ),
                            );
                        }
                    }
                    PrimaryToken::SelectAll => {
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.select_all",
                                label: sel_all_lbl,
                                tooltip: tr_or_fallback(locale, "gallery.select_all.tip", "Select all visible sheets"),
                                enabled: sel_all_enabled,
                                selected: false,
                                command_id: None,
                                disabled_reason: Some(tr_or_fallback(
                                    locale,
                                    "gallery.select_all.disabled",
                                    "All visible sheets are already selected",
                                )),
                            },
                            meta,
                        );
                        if resp.clicked() && sel_all_enabled {
                            views.center_stage.gallery_anchor_id = visible_ids.first().cloned();
                            intents.push(UiIntent::SetGallerySelection { sheet_ids: visible_ids.to_vec() });
                        }
                    }
                    PrimaryToken::Clear => {
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.clear_selection",
                                label: clear_lbl,
                                tooltip: tr_or_fallback(locale, "gallery.clear_selection.tip", "Clear gallery selection"),
                                enabled: clear_enabled,
                                selected: false,
                                command_id: None,
                                disabled_reason: Some(tr_or_fallback(locale, "gallery.batch.need_selection", "Select one or more sheets first")),
                            },
                            meta,
                        );
                        if resp.clicked() && clear_enabled {
                            views.center_stage.gallery_anchor_id = None;
                            intents.push(UiIntent::SetGallerySelection { sheet_ids: Vec::new() });
                        }
                    }
                    PrimaryToken::Search => {
                        let search_resp = bar.add_enabled(
                            enabled,
                            egui::TextEdit::singleline(&mut views.center_stage.gallery_search)
                                .id(Id::unique("gallery.search_input"))
                                .desired_width(search_desired_w)
                                .hint_text(search_hint),
                        );
                        if search_resp.changed() && enabled {
                            views.center_stage.last_synced_gallery_search = views.center_stage.gallery_search.clone();
                            intents.push(UiIntent::SetGalleryViewOptions {
                                search_query: Some(views.center_stage.gallery_search.clone()),
                                sort_order: None,
                                current_family_only: None,
                            });
                        }
                        meta.record_response(
                            bar,
                            &search_resp,
                            inspect_input("gallery.search_input", "textbox", search_hint, None, None, enabled, ZLayer::Workspace),
                        );
                    }
                    PrimaryToken::Filter(f_idx) => {
                        let (folder_opt, folder_param, wid, _) = filter_defs[f_idx];
                        let active = app.nav.gallery.filter_folder == folder_opt;
                        let lbl = &filter_labels[f_idx];
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: wid,
                                label: lbl,
                                tooltip: lbl,
                                enabled,
                                selected: active,
                                command_id: Some("ui.gallery.toggle"),
                                disabled_reason: None,
                            },
                            meta,
                        );
                        if resp.clicked() && enabled {
                            intents.push_ui("ui.gallery.toggle", json!({ "open": true, "folder": folder_param }));
                        }
                    }
                    PrimaryToken::CurrentFamily => {
                        let fam_resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.filter.current_family",
                                label: fam_lbl,
                                tooltip: tr_or_fallback(
                                    locale,
                                    "gallery.filter.current_family.tip",
                                    "Show sheets linked to the active sheet's lineage",
                                ),
                                enabled,
                                selected: family_active,
                                command_id: None,
                                disabled_reason: None,
                            },
                            meta,
                        );
                        if fam_resp.clicked() && enabled {
                            intents.push(UiIntent::SetGalleryViewOptions {
                                search_query: None,
                                sort_order: None,
                                current_family_only: Some(!family_active),
                            });
                        }
                    }
                    PrimaryToken::Sort => {
                        let mut chosen_sort: Option<GallerySortOrder> = None;
                        let sort_drop = dropdown("gallery.sort", RichText::new(&sort_text).small().color(tokens.text_primary))
                            .width(144.0)
                            .height(tokens.metrics.min_hit_target_pt)
                            .enabled(enabled)
                            .accessible_name(sort_text.clone())
                            .show_ui(bar, |pop_ui| {
                                let row_w = pop_ui.available_width().max(148.0);
                                for opt in GallerySortOrder::ALL {
                                    let is_sel = opt == cur_sort;
                                    let opt_lbl = tr_or_fallback(locale, opt.label_key(), opt.slug());
                                    let (row_r, row_resp) = pop_ui.allocate_exact_size(
                                        Vec2::new(row_w, tokens.metrics.min_hit_target_pt),
                                        if enabled { Sense::click() } else { Sense::hover() },
                                    );
                                    super::widgets::paint_hover_row(
                                        pop_ui,
                                        row_r,
                                        CornerRadius::same(tokens.metrics.radius_md),
                                        row_resp.hovered(),
                                        is_sel,
                                        tokens,
                                    );
                                    let col = if is_sel { tokens.selected_accent_text() } else { tokens.text_primary };
                                    pop_ui.painter().text(
                                        Pos2::new(row_r.left() + tokens.metrics.space_sm, row_r.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        opt_lbl,
                                        FontId::proportional(tokens.metrics.font_ui_small_pt),
                                        col,
                                    );
                                    meta.record_response(
                                        pop_ui,
                                        &row_resp,
                                        FrameInspectEntryInput {
                                            widget_id: &format!("gallery.sort.{}", opt.slug()),
                                            role: "option",
                                            label: opt_lbl,
                                            command_id: None,
                                            value: is_sel.then_some(json!(true)),
                                            enabled,
                                            disabled_reason: None,
                                            z_layer: ZLayer::DropdownMenu,
                                            ellipsized: false,
                                            full_label: None,
                                        },
                                    );
                                    if row_resp.clicked() && enabled {
                                        chosen_sort = Some(opt);
                                        pop_ui.close();
                                    }
                                }
                            });
                        meta.record_response(
                            bar,
                            &sort_drop.response,
                            inspect_input("gallery.sort", "combobox", &sort_text, None, Some(json!(cur_sort.slug())), enabled, ZLayer::Workspace),
                        );
                        if let Some(next_sort) = chosen_sort {
                            intents.push(UiIntent::SetGalleryViewOptions {
                                search_query: None,
                                sort_order: Some(next_sort),
                                current_family_only: None,
                            });
                        }
                    }
                    PrimaryToken::ZoomSlider => {
                        let zoom_lbl = "Thumbnail Zoom";
                        let slider =
                            bar.add_enabled(enabled, egui::Slider::new(&mut views.center_stage.gallery_zoom_pt, 120.0..=260.0).show_value(false));
                        bar.ctx().accesskit_node_builder(slider.id, |node| node.set_label(zoom_lbl));
                        meta.record_response(
                            bar,
                            &slider,
                            inspect_input(
                                "gallery.zoom_slider",
                                "slider",
                                zoom_lbl,
                                None,
                                Some(json!(views.center_stage.gallery_zoom_pt.round())),
                                enabled,
                                ZLayer::Workspace,
                            ),
                        );
                    }
                    PrimaryToken::Close => {
                        let close_resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.close",
                                label: close_lbl,
                                tooltip: close_lbl,
                                enabled,
                                selected: false,
                                command_id: Some("ui.gallery.toggle"),
                                disabled_reason: None,
                            },
                            meta,
                        );
                        if close_resp.clicked() && enabled {
                            intents.push_ui("ui.gallery.toggle", json!({ "open": false }));
                        }
                    }
                }
            }
        });
    }
}

struct GalleryBatchToolbarSpec<'a> {
    doc: &'a ProjectDocument,
    inner_w: f32,
    tokens: &'a Tokens,
    enabled: bool,
    selected_ordered: &'a [SheetId],
}

fn render_gallery_batch_toolbar(ui: &mut egui::Ui, app: &UiApp, spec: GalleryBatchToolbarSpec<'_>, io: &mut GalleryFrameIo<'_>) {
    let GalleryBatchToolbarSpec { doc, inner_w, tokens, enabled, selected_ordered } = spec;
    let GalleryFrameIo { views, meta, intents } = io;
    let locale = app.locale.as_str();
    let gap = tokens.metrics.space_xs + 1.0;
    let selected_graphs: Vec<SheetId> = selected_ordered.iter().filter(|id| doc.graph_sheet(id).is_some()).cloned().collect();
    let selected_renderables: Vec<SheetId> =
        selected_ordered.iter().filter(|id| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some()).cloned().collect();
    let selected_data: Vec<SheetId> = selected_ordered.iter().filter(|id| doc.data_sheet(id).is_some()).cloned().collect();
    let all_data_frozen = !selected_data.is_empty() && selected_data.iter().all(|id| doc.data_sheet(id).is_some_and(|ds| ds.frozen));

    let split_en = enabled && selected_ordered.len() == 2;
    let split_lbl = tr(locale, "nav.ctx.compare_split");
    let place_en = enabled && !selected_graphs.is_empty();
    let place_lbl = tr_or_fallback(locale, "gallery.batch.place_layout", "Place on New Layout");
    let style_en = enabled && !selected_graphs.is_empty() && doc.graph_sheets.len() >= 2;
    let style_lbl = tr(locale, "r13.gallery.apply_style_from");
    let export_en = enabled && !selected_renderables.is_empty();
    let export_lbl = tr_or_fallback(locale, "gallery.batch.export", "Export Selected");
    let print_lbl = tr_or_fallback(locale, "gallery.batch.print", "Print Selected");
    let csv_en = enabled && !selected_data.is_empty();
    let csv_lbl = tr_or_fallback(locale, "gallery.batch.export_csv", "Export CSV");
    let dup_en = enabled && !selected_ordered.is_empty();
    let dup_lbl = tr_or_fallback(locale, "gallery.batch.duplicate", "Duplicate");
    let freeze_cmd = if all_data_frozen { "sheet.unfreeze" } else { "sheet.freeze" };
    let freeze_lbl = if all_data_frozen {
        tr_or_fallback(locale, "gallery.batch.unfreeze", "Unfreeze")
    } else {
        tr_or_fallback(locale, "gallery.batch.freeze", "Freeze")
    };
    let del_lbl = tr_or_fallback(locale, "gallery.batch.delete", "Delete");

    let labels = [split_lbl, place_lbl, style_lbl, export_lbl, print_lbl, csv_lbl, dup_lbl, freeze_lbl, del_lbl];
    let widths: Vec<f32> = labels.iter().map(|l| gallery_text_item_width(ui, tokens, l)).collect();
    let rows = pack_flow_rows(&widths, inner_w, gap);

    for (row_idx, row_range) in rows.into_iter().enumerate() {
        if row_idx > 0 {
            ui.add_space(tokens.metrics.space_xs);
        }
        ui.horizontal(|bar| {
            bar.spacing_mut().item_spacing.x = gap;
            for idx in row_range {
                match idx {
                    0 => {
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.batch.compare_split",
                                label: split_lbl,
                                tooltip: split_lbl,
                                enabled: split_en,
                                selected: false,
                                command_id: Some("ui.split.toggle"),
                                disabled_reason: Some(tr_or_fallback(locale, "gallery.batch.need_two", "Select 2 sheets to compare side by side")),
                            },
                            meta,
                        );
                        if resp.clicked() && split_en {
                            intents.push_ui(
                                "ui.split.toggle",
                                json!({
                                    "enabled": true,
                                    "primary_sheet_id": selected_ordered[0].as_str(),
                                    "secondary_sheet_id": selected_ordered[1].as_str(),
                                }),
                            );
                            intents.push_ui("ui.gallery.toggle", json!({ "open": false }));
                        }
                    }
                    1 => {
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.batch.place_layout",
                                label: place_lbl,
                                tooltip: tr_or_fallback(
                                    locale,
                                    "gallery.batch.place_layout.tip",
                                    "Create a grid layout containing the selected graphs",
                                ),
                                enabled: place_en,
                                selected: false,
                                command_id: Some("layout.create"),
                                disabled_reason: Some(tr_or_fallback(locale, "gallery.batch.need_graphs", "Select one or more Graph sheets")),
                            },
                            meta,
                        );
                        if resp.clicked() && place_en {
                            intents.push(UiIntent::PlaceSelectedOntoNewLayout { graph_ids: selected_graphs.clone() });
                        }
                    }
                    2 => {
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.apply_style_from",
                                label: style_lbl,
                                tooltip: style_lbl,
                                enabled: style_en,
                                selected: false,
                                command_id: Some("ui.applyStyleFrom.open"),
                                disabled_reason: Some(tr_or_fallback(
                                    locale,
                                    "gallery.batch.need_two_graphs",
                                    "Requires a selected graph and at least 2 graphs in the project",
                                )),
                            },
                            meta,
                        );
                        if resp.clicked() && style_en {
                            let target_ids: Vec<&str> = selected_graphs.iter().map(SheetId::as_str).collect();
                            intents.push(UiIntent::SetLauncherOpen { open: false });
                            intents.push_ui("ui.gallery.toggle", json!({ "open": false }));
                            intents.push_ui("ui.applyStyleFrom.open", json!({ "target_sheet_ids": target_ids }));
                        }
                    }
                    3 => {
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.batch.export",
                                label: export_lbl,
                                tooltip: export_lbl,
                                enabled: export_en,
                                selected: false,
                                command_id: Some("ui.export.figure.open"),
                                disabled_reason: Some(tr_or_fallback(
                                    locale,
                                    "gallery.batch.need_renderables",
                                    "Select one or more Graph or Layout sheets",
                                )),
                            },
                            meta,
                        );
                        if resp.clicked()
                            && export_en
                            && let Some(first) = selected_renderables.first()
                        {
                            intents.push(UiIntent::OpenExportFigure {
                                sheet_id: first.clone(),
                                target_selected: selected_renderables.len() > 1 || selected_ordered.len() > 1,
                            });
                        }
                    }
                    4 => {
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.batch.print",
                                label: print_lbl,
                                tooltip: print_lbl,
                                enabled: export_en,
                                selected: false,
                                command_id: Some("ui.print.open"),
                                disabled_reason: Some(tr_or_fallback(
                                    locale,
                                    "gallery.batch.need_renderables",
                                    "Select one or more Graph or Layout sheets",
                                )),
                            },
                            meta,
                        );
                        if resp.clicked() && export_en {
                            intents.push(UiIntent::OpenPrintForSheets { sheet_ids: selected_renderables.clone() });
                        }
                    }
                    5 => {
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.batch.export_csv",
                                label: csv_lbl,
                                tooltip: tr_or_fallback(locale, "gallery.batch.export_csv.tip", "Export selected Data tables to CSV or TSV"),
                                enabled: csv_en,
                                selected: false,
                                command_id: Some("ui.export.tableCsv.open"),
                                disabled_reason: Some(tr_or_fallback(locale, "gallery.batch.need_data", "Select one or more Data tables")),
                            },
                            meta,
                        );
                        if resp.clicked() && csv_en {
                            intents.push(UiIntent::OpenCsvExportForSheets { sheet_ids: selected_data.clone() });
                        }
                    }
                    6 => {
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.batch.duplicate",
                                label: dup_lbl,
                                tooltip: dup_lbl,
                                enabled: dup_en,
                                selected: false,
                                command_id: Some("sheet.duplicate"),
                                disabled_reason: Some(tr_or_fallback(locale, "gallery.batch.need_selection", "Select one or more sheets first")),
                            },
                            meta,
                        );
                        if resp.clicked() && dup_en {
                            let cmds =
                                selected_ordered.iter().map(|id| (String::from("sheet.duplicate"), json!({ "sheet_id": id.as_str() }))).collect();
                            intents.push(UiIntent::EngineCommandGroup { commands: cmds });
                        }
                    }
                    7 => {
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.batch.freeze",
                                label: freeze_lbl,
                                tooltip: freeze_lbl,
                                enabled: csv_en,
                                selected: false,
                                command_id: Some(freeze_cmd),
                                disabled_reason: Some(tr_or_fallback(locale, "gallery.batch.need_data", "Select one or more Data tables")),
                            },
                            meta,
                        );
                        if resp.clicked() && csv_en {
                            let cmds = selected_data.iter().map(|id| (String::from(freeze_cmd), json!({ "sheet_id": id.as_str() }))).collect();
                            intents.push(UiIntent::EngineCommandGroup { commands: cmds });
                        }
                    }
                    _ => {
                        let resp = gallery_action_button(
                            bar,
                            tokens,
                            GalleryBtnSpec {
                                widget_id: "gallery.batch.delete",
                                label: del_lbl,
                                tooltip: del_lbl,
                                enabled: dup_en,
                                selected: false,
                                command_id: Some("sheet.delete"),
                                disabled_reason: Some(tr_or_fallback(locale, "gallery.batch.need_selection", "Select one or more sheets first")),
                            },
                            meta,
                        );
                        if resp.clicked() && dup_en {
                            trigger_gallery_delete(doc, locale, selected_ordered, views, intents, bar.ctx());
                        }
                    }
                }
            }
        });
    }
}

fn handle_gallery_keyboard(
    ui: &egui::Ui,
    doc: &ProjectDocument,
    views: &mut EguiShellViewState,
    visible_ids: &[SheetId],
    cols: usize,
    intents: &mut PendingUiIntents,
) {
    if visible_ids.is_empty()
        || views.center_stage.gallery_context_menu.is_some()
        || views.center_stage.gallery_renaming.is_some()
        || ui.ctx().egui_wants_keyboard_input()
    {
        return;
    }

    let cols = cols.max(1);
    let select_all = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::A) || i.consume_key(egui::Modifiers::CTRL, Key::A));
    if select_all {
        views.center_stage.gallery_anchor_id = visible_ids.first().cloned();
        intents.push(UiIntent::SetGallerySelection { sheet_ids: visible_ids.to_vec() });
        return;
    }

    let delta = ui.input(|i| {
        let arrow_shift = |k: Key| -> Option<bool> {
            i.key_pressed(k).then(|| {
                i.modifiers.shift
                    || i.events.iter().any(|ev| matches!(ev, egui::Event::Key { key, pressed: true, modifiers, .. } if *key == k && modifiers.shift))
            })
        };
        [(Key::ArrowRight, 1isize), (Key::ArrowLeft, -1isize), (Key::ArrowDown, cols as isize), (Key::ArrowUp, -(cols as isize))]
            .into_iter()
            .find_map(|(k, step)| arrow_shift(k).map(|shift| (step, shift)))
    });

    if let Some((step, shift)) = delta {
        let cur_idx = views
            .center_stage
            .gallery_focused_id
            .as_ref()
            .and_then(|id| visible_ids.iter().position(|v| v == id))
            .or_else(|| doc.active_sheet_id.as_ref().and_then(|id| visible_ids.iter().position(|v| v == id)))
            .unwrap_or(0);
        let max_idx = visible_ids.len().saturating_sub(1) as isize;
        let next_idx = (cur_idx as isize + step).clamp(0, max_idx) as usize;
        let next_id = visible_ids[next_idx].clone();
        views.center_stage.gallery_focused_id = Some(next_id.clone());
        if shift {
            let anchor_idx = views.center_stage.gallery_anchor_id.as_ref().and_then(|id| visible_ids.iter().position(|v| v == id)).unwrap_or(cur_idx);
            let (lo, hi) = if anchor_idx <= next_idx { (anchor_idx, next_idx) } else { (next_idx, anchor_idx) };
            views.center_stage.gallery_anchor_id = Some(visible_ids[anchor_idx].clone());
            intents.push(UiIntent::SetGallerySelection { sheet_ids: visible_ids[lo..=hi].to_vec() });
        } else {
            views.center_stage.gallery_anchor_id = Some(next_id);
        }
        return;
    }

    let target_id = views
        .center_stage
        .gallery_focused_id
        .clone()
        .filter(|id| visible_ids.contains(id))
        .or_else(|| doc.active_sheet_id.as_ref().filter(|id| visible_ids.contains(id)).cloned())
        .or_else(|| visible_ids.first().cloned());

    if ui.input(|i| i.key_pressed(egui::Key::Space))
        && let Some(id) = target_id
    {
        views.center_stage.gallery_focused_id = Some(id.clone());
        views.center_stage.gallery_anchor_id = Some(id);
        return;
    }

    if ui.input(|i| i.key_pressed(egui::Key::Enter))
        && let Some(id) = target_id
    {
        views.center_stage.gallery_anchor_id = Some(id.clone());
        intents.push(UiIntent::SetLauncherOpen { open: false });
        intents.push(UiIntent::SetGallerySelection { sheet_ids: vec![id.clone()] });
        intents.push_engine("sheet.select", json!({ "sheet_id": id.as_str() }));
        intents.push_ui("ui.gallery.toggle", json!({ "open": false }));
    }
}

struct GalleryCardGridSpec<'a> {
    doc: &'a ProjectDocument,
    cards_slice: &'a [&'a PortfolioCard],
    visible_ids: &'a [SheetId],
    selected_ids_snapshot: &'a BTreeSet<SheetId>,
    cols: usize,
    card_size: Vec2,
    spacing: f32,
    inner_w: f32,
    enabled: bool,
}

fn render_gallery_card_grid_chunked(grid_ui: &mut egui::Ui, app: &UiApp, spec: GalleryCardGridSpec<'_>, io: &mut GalleryFrameIo<'_>) {
    let GalleryCardGridSpec { doc, cards_slice, visible_ids, selected_ids_snapshot, cols, card_size, spacing, inner_w, enabled } = spec;
    let GalleryFrameIo { views, meta, intents } = io;
    let clip = grid_ui.clip_rect();
    for row_cards in cards_slice.chunks(cols.max(1)) {
        let pred_row = Rect::from_min_size(grid_ui.cursor().min, Vec2::new(inner_w, card_size.y));
        let vis_row = pred_row.intersect(clip);
        if vis_row.height() < 4.0 || vis_row.width() < 4.0 {
            grid_ui.allocate_space(Vec2::new(inner_w, card_size.y));
            continue;
        }
        grid_ui.horizontal(|row_ui| {
            row_ui.spacing_mut().item_spacing.x = spacing;
            for card in row_cards {
                render_gallery_card(
                    row_ui,
                    app,
                    doc,
                    GalleryCardSpec { card, visible_ids, selected_ids_snapshot, views, size: card_size, tokens: &Tokens::get(row_ui.ctx()), enabled },
                    meta,
                    intents,
                );
            }
        });
    }
}

fn render_gallery_context_menu(
    ctx: &egui::Context,
    app: &UiApp,
    doc: &ProjectDocument,
    tokens: &Tokens,
    views: &mut EguiShellViewState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let Some(menu) = views.center_stage.gallery_context_menu.clone() else {
        return;
    };
    let locale = app.locale.as_str();
    let sid = menu.sheet_id.as_str();
    let title = doc_sheet_title(doc, &menu.sheet_id).unwrap_or(sid).to_owned();
    let is_data = matches!(menu.folder, SheetFolderKind::Data);
    let is_renderable = matches!(menu.folder, SheetFolderKind::Graphs | SheetFolderKind::Layouts);
    let is_csv_eligible = matches!(menu.folder, SheetFolderKind::Data | SheetFolderKind::Results);
    let freeze_cmd = if menu.is_frozen { "sheet.unfreeze" } else { "sheet.freeze" };
    let freeze_lbl = tr(locale, if menu.is_frozen { "cmd.sheet.unfreeze" } else { "cmd.sheet.freeze" });
    let frozen_reason = menu.is_frozen.then(|| tr(locale, "nav.frozen_badge"));

    #[derive(Clone, Copy)]
    enum GalCtxAction {
        Rename,
        Duplicate,
        DuplicateFamily,
        ToggleFreeze,
        ExportFigure,
        ExportCsv,
        Print,
        Delete,
    }

    let mut entries = vec![
        ActionMenuEntry::Item(
            ActionMenuItem::menu_item("gallery.ctx.rename", tr(locale, "cmd.sheet.rename"))
                .command_id(Some("sheet.rename"))
                .enabled(!menu.is_frozen)
                .disabled_reason(frozen_reason),
        ),
        ActionMenuEntry::Item(
            ActionMenuItem::menu_item("gallery.ctx.duplicate", tr(locale, "cmd.sheet.duplicate")).command_id(Some("sheet.duplicate")),
        ),
        ActionMenuEntry::Item(
            ActionMenuItem::menu_item("gallery.ctx.duplicate_family", tr(locale, "cmd.sheet.duplicateFamily"))
                .command_id(Some("sheet.duplicateFamily")),
        ),
        ActionMenuEntry::Item(
            ActionMenuItem::menu_item("gallery.ctx.freeze", freeze_lbl)
                .command_id(Some(freeze_cmd))
                .enabled(is_data)
                .disabled_reason((!is_data).then(|| tr_or_fallback(locale, "gallery.batch.need_data", "Applies only to Data tables"))),
        ),
        ActionMenuEntry::Separator,
    ];
    let mut actions =
        vec![Some(GalCtxAction::Rename), Some(GalCtxAction::Duplicate), Some(GalCtxAction::DuplicateFamily), Some(GalCtxAction::ToggleFreeze), None];

    if is_renderable {
        entries.push(ActionMenuEntry::Item(
            ActionMenuItem::menu_item("gallery.ctx.export", tr_or_fallback(locale, "gallery.ctx.export", "Export"))
                .command_id(Some("ui.export.figure.open")),
        ));
        actions.push(Some(GalCtxAction::ExportFigure));
        entries.push(ActionMenuEntry::Item(
            ActionMenuItem::menu_item("gallery.ctx.print", tr_or_fallback(locale, "gallery.ctx.print", "Print")).command_id(Some("ui.print.open")),
        ));
        actions.push(Some(GalCtxAction::Print));
    } else if is_csv_eligible {
        entries.push(ActionMenuEntry::Item(
            ActionMenuItem::menu_item("gallery.ctx.export_csv", tr_or_fallback(locale, "gallery.batch.export_csv", "Export CSV"))
                .command_id(Some("ui.export.tableCsv.open")),
        ));
        actions.push(Some(GalCtxAction::ExportCsv));
    }

    entries.push(ActionMenuEntry::Separator);
    actions.push(None);
    entries.push(ActionMenuEntry::Item(
        ActionMenuItem::menu_item("gallery.ctx.delete", tr(locale, "cmd.sheet.delete"))
            .command_id(Some("sheet.delete"))
            .enabled(!menu.is_frozen)
            .disabled_reason(frozen_reason),
    ));
    actions.push(Some(GalCtxAction::Delete));

    let out = action_menu_popup(
        ctx,
        tokens,
        Some(meta),
        ActionMenuSpec {
            area_id: Id::unique(("gallery.card.context_menu", sid)),
            widget_id: "gallery.card.context_menu",
            label: &title,
            anchor: ActionMenuAnchor::Pointer(menu.anchor_pos),
            z_layer: ZLayer::ContextMenu,
            min_w: 176.0,
            max_w: 280.0,
            max_height: None,
            allow_outside_dismiss: true,
        },
        &entries,
    );

    if let Some(hit) = out.clicked_entry
        && let Some(Some(action)) = actions.get(hit)
    {
        match action {
            GalCtxAction::Rename => {
                views.center_stage.gallery_renaming =
                    Some(GalleryInlineRenameState { sheet_id: menu.sheet_id.clone(), draft: title, request_focus: true });
            }
            GalCtxAction::Duplicate => intents.push_engine("sheet.duplicate", json!({ "sheet_id": sid })),
            GalCtxAction::DuplicateFamily => intents.push_engine("sheet.duplicateFamily", json!({ "sheet_id": sid })),
            GalCtxAction::ToggleFreeze => intents.push_engine(freeze_cmd, json!({ "sheet_id": sid })),
            GalCtxAction::ExportFigure => {
                intents.push(UiIntent::OpenExportFigure { sheet_id: menu.sheet_id.clone(), target_selected: false });
            }
            GalCtxAction::ExportCsv => {
                if matches!(menu.folder, SheetFolderKind::Results) {
                    intents.push_ui("ui.export.resultsCsv.open", json!({ "sheet_id": sid }));
                } else {
                    intents.push(UiIntent::OpenCsvExportForSheets { sheet_ids: vec![menu.sheet_id.clone()] });
                }
            }
            GalCtxAction::Print => intents.push(UiIntent::OpenPrintForSheets { sheet_ids: vec![menu.sheet_id.clone()] }),
            GalCtxAction::Delete => {
                trigger_gallery_delete(doc, locale, &[menu.sheet_id], views, intents, ctx);
            }
        }
        views.center_stage.gallery_context_menu = None;
    } else if out.dismissed {
        views.center_stage.gallery_context_menu = None;
    }
}

pub(super) fn render_portfolio_gallery(
    ui: &mut egui::Ui,
    app: &UiApp,
    rect: Rect,
    tokens: &Tokens,
    enabled: bool,
    io: &mut GalleryFrameIo<'_>,
) -> Option<Rect> {
    let doc = app.document();
    let locale = app.locale.as_str();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, tokens.surface_bg);
    io.views.center_stage.gallery_last_stage_w_pt = rect.width();

    if io.views.center_stage.gallery_search == io.views.center_stage.last_synced_gallery_search
        && app.nav.gallery.search_query != io.views.center_stage.gallery_search
    {
        io.views.center_stage.gallery_search = app.nav.gallery.search_query.clone();
        io.views.center_stage.last_synced_gallery_search = app.nav.gallery.search_query.clone();
    }
    let effective_search = io.views.center_stage.gallery_search.clone();
    let cards = app.nav.build_gallery_cards_with(doc, &effective_search, app.nav.gallery.sort_order, app.nav.gallery.current_family_only);
    let counts = GalleryFolderCounts::from_document(doc);
    let selected_ids_snapshot = app.nav.gallery.selected_ids.clone();
    let visible_ids: Vec<SheetId> = cards.iter().map(|c| c.sheet_id.clone()).collect();
    let mut selected_ordered: Vec<SheetId> =
        cards.iter().filter(|c| selected_ids_snapshot.contains(&c.sheet_id)).map(|c| c.sheet_id.clone()).collect();
    for id in &selected_ids_snapshot {
        if !selected_ordered.contains(id) {
            selected_ordered.push(id.clone());
        }
    }

    let inner_rect = rect.shrink(tokens.metrics.space_sm);
    let mut gal_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner_rect).layout(egui::Layout::top_down(egui::Align::Min)));
    gal_ui.set_clip_rect(rect);
    gal_ui.spacing_mut().item_spacing.y = 3.0;

    render_gallery_primary_toolbar(
        &mut gal_ui,
        app,
        GalleryPrimaryToolbarSpec {
            inner_w: inner_rect.width(),
            tokens,
            enabled,
            visible_ids: &visible_ids,
            selected_count: selected_ids_snapshot.len(),
            counts,
        },
        io,
    );
    render_gallery_batch_toolbar(
        &mut gal_ui,
        app,
        GalleryBatchToolbarSpec { doc, inner_w: inner_rect.width(), tokens, enabled, selected_ordered: &selected_ordered },
        io,
    );

    gal_ui.separator();

    if cards.is_empty() {
        gal_ui.label(RichText::new(tr_or_fallback(locale, "w5.gallery.empty", "No matching sheets in this view.")).color(tokens.text_muted));
        render_gallery_context_menu(ui.ctx(), app, doc, tokens, io.views, io.meta, io.intents);
        return None;
    }

    let est_inner_w = (inner_rect.width() - 16.0).max(140.0);
    let spacing = 8.0_f32;
    let est_desired_w = io.views.center_stage.gallery_zoom_pt.clamp(120.0, est_inner_w);
    let est_cols = ((est_inner_w + spacing) / (est_desired_w + spacing)).floor().max(1.0) as usize;

    if enabled {
        let prev_focused = io.views.center_stage.gallery_focused_id.clone();
        let space_toggle_requested = gal_ui.input(|i| i.key_pressed(egui::Key::Space))
            && io.views.center_stage.gallery_context_menu.is_none()
            && io.views.center_stage.gallery_renaming.is_none()
            && !gal_ui.ctx().egui_wants_keyboard_input();
        handle_gallery_keyboard(&gal_ui, doc, io.views, &visible_ids, est_cols, io.intents);
        if space_toggle_requested && let Some(fid) = io.views.center_stage.gallery_focused_id.clone().or(prev_focused) {
            let mut next = selected_ids_snapshot.clone();
            if !next.remove(&fid) {
                next.insert(fid);
            }
            io.intents.push(UiIntent::SetGallerySelection { sheet_ids: next.into_iter().collect() });
        }
    }

    let sort_order = app.nav.gallery.sort_order;
    let show_sections = app.nav.gallery.filter_folder.is_none() && sort_order.uses_collapsible_sections();

    let scroll = egui::ScrollArea::vertical().id_salt("center_stage.gallery.scroll").auto_shrink([false, false]).show(&mut gal_ui, |grid_ui| {
        grid_ui.spacing_mut().item_spacing.y = 3.0;
        let inner_w = (inner_rect.width().min(grid_ui.available_width()) - 16.0).max(140.0);
        let desired_card_w = io.views.center_stage.gallery_zoom_pt.clamp(120.0, inner_w);
        let cols = ((inner_w + spacing) / (desired_card_w + spacing)).floor().max(1.0) as usize;
        let card_w = ((inner_w - spacing * (cols.saturating_sub(1) as f32)) / (cols as f32)).max(120.0);
        let card_h = (desired_card_w * 0.54).clamp(80.0, 148.0);
        let card_size = Vec2::new(card_w, card_h);

        if show_sections {
            let section_order = match sort_order {
                GallerySortOrder::Type => GALLERY_TYPE_SECTION_ORDER,
                _ => GALLERY_NAV_SECTION_ORDER,
            };
            for folder in section_order {
                let bucket: Vec<&PortfolioCard> = cards.iter().filter(|c| c.folder == folder).collect();
                if bucket.is_empty() {
                    continue;
                }
                let collapsed = io.views.center_stage.gallery_collapsed_sections.contains(&folder);
                let header_h = tokens.metrics.min_hit_target_pt;
                let pred_h = Rect::from_min_size(grid_ui.cursor().min, Vec2::new(inner_w, header_h));
                let vis_h = pred_h.intersect(grid_ui.clip_rect());
                if vis_h.height() < 4.0 || vis_h.width() < 4.0 {
                    grid_ui.allocate_space(Vec2::new(inner_w, header_h));
                } else {
                    let (h_rect, h_resp) =
                        grid_ui.allocate_exact_size(Vec2::new(inner_w, header_h), if enabled { Sense::click() } else { Sense::hover() });
                    if h_resp.hovered() && enabled {
                        grid_ui.painter().rect_filled(h_rect, CornerRadius::same(tokens.metrics.radius_md), tokens.hover_bg);
                    }
                    let chev = if collapsed { "▸" } else { "▾" };
                    let sec_title = format!("{chev} {} ({})", tr(locale, gallery_folder_section_label_key(folder)), bucket.len());
                    grid_ui.painter().text(
                        Pos2::new(h_rect.left() + tokens.metrics.space_xs, h_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &sec_title,
                        FontId::proportional(tokens.metrics.font_ui_body_pt),
                        tokens.text_strong,
                    );
                    let sec_wid = format!("gallery.section.{}", gallery_folder_slug(folder));
                    let sec_a11y = sec_title.clone();
                    h_resp.widget_info(move || egui::WidgetInfo::selected(egui::Role::TreeItem, enabled, !collapsed, sec_a11y.clone()));
                    io.meta.record_response(
                        grid_ui,
                        &h_resp,
                        FrameInspectEntryInput {
                            widget_id: &sec_wid,
                            role: "treeitem",
                            label: &sec_title,
                            command_id: None,
                            value: Some(json!({ "collapsed": collapsed, "count": bucket.len() })),
                            enabled,
                            disabled_reason: None,
                            z_layer: ZLayer::Workspace,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if h_resp.clicked() && enabled {
                        if collapsed {
                            io.views.center_stage.gallery_collapsed_sections.retain(|f| *f != folder);
                        } else if !io.views.center_stage.gallery_collapsed_sections.contains(&folder) {
                            io.views.center_stage.gallery_collapsed_sections.push(folder);
                        }
                    }
                }
                if !collapsed {
                    render_gallery_card_grid_chunked(
                        grid_ui,
                        app,
                        GalleryCardGridSpec {
                            doc,
                            cards_slice: &bucket,
                            visible_ids: &visible_ids,
                            selected_ids_snapshot: &selected_ids_snapshot,
                            cols,
                            card_size,
                            spacing,
                            inner_w,
                            enabled,
                        },
                        io,
                    );
                }
            }
        } else {
            let flat: Vec<&PortfolioCard> = cards.iter().collect();
            render_gallery_card_grid_chunked(
                grid_ui,
                app,
                GalleryCardGridSpec {
                    doc,
                    cards_slice: &flat,
                    visible_ids: &visible_ids,
                    selected_ids_snapshot: &selected_ids_snapshot,
                    cols,
                    card_size,
                    spacing,
                    inner_w,
                    enabled,
                },
                io,
            );
        }
    });

    let scrollbar_rect_pt = (scroll.content_size.y > scroll.inner_rect.height() + 0.5).then(|| {
        let bar_w = gal_ui.spacing().scroll.bar_width.max(10.0);
        Rect::from_min_max(
            pos2((scroll.inner_rect.right() + 2.0).min(inner_rect.right()), scroll.inner_rect.top()),
            pos2((scroll.inner_rect.right() + 2.0 + bar_w).min(inner_rect.right()), scroll.inner_rect.bottom()),
        )
    });
    render_gallery_context_menu(ui.ctx(), app, doc, tokens, io.views, io.meta, io.intents);
    scrollbar_rect_pt
}

struct GalleryCardSpec<'a> {
    card: &'a PortfolioCard,
    visible_ids: &'a [SheetId],
    selected_ids_snapshot: &'a BTreeSet<SheetId>,
    views: &'a mut EguiShellViewState,
    size: Vec2,
    tokens: &'a Tokens,
    enabled: bool,
}

#[derive(Clone, Copy)]
struct GalleryCardDrawCtx<'a> {
    locale: &'a str,
    card: &'a PortfolioCard,
    card_rect: Rect,
    tokens: &'a Tokens,
}

fn render_gallery_card_label_area(
    ui: &mut egui::Ui,
    draw: GalleryCardDrawCtx<'_>,
    views: &mut EguiShellViewState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let GalleryCardDrawCtx { locale, card, card_rect, tokens } = draw;
    let renaming_this = views.center_stage.gallery_renaming.as_ref().is_some_and(|r| r.sheet_id == card.sheet_id);
    let text_clip = card_rect.shrink(4.0).intersect(ui.clip_rect());
    if renaming_this && let Some(ref mut ren) = views.center_stage.gallery_renaming {
        let edit_rect =
            Rect::from_min_max(pos2(card_rect.left() + 6.0, card_rect.bottom() - 34.0), pos2(card_rect.right() - 6.0, card_rect.bottom() - 14.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(edit_rect));
        let edit_resp = child.add(egui::TextEdit::singleline(&mut ren.draft).id(Id::unique(("gallery.rename_input", card.sheet_id.as_str()))));
        let just_focused = ren.request_focus;
        if just_focused {
            edit_resp.request_focus();
            if let Some(mut te_state) = egui::TextEdit::load_state(child.ctx(), edit_resp.id) {
                let end = egui::text::CCursor::new(ren.draft.chars().count());
                te_state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), end)));
                te_state.store(child.ctx(), edit_resp.id);
            }
            ren.request_focus = false;
        }
        meta.record_response(
            &child,
            &edit_resp,
            inspect_input("gallery.card.rename_input", "textbox", &ren.draft, Some("sheet.rename"), None, true, ZLayer::Workspace),
        );
        let cancel = child.input(|i| i.key_pressed(egui::Key::Escape));
        let enter = child.input(|i| i.key_pressed(egui::Key::Enter));
        if cancel {
            views.center_stage.gallery_renaming = None;
        } else if !just_focused && (enter || edit_resp.lost_focus()) {
            let trimmed = ren.draft.trim().to_owned();
            if !trimmed.is_empty() {
                intents.push_engine("sheet.rename", json!({ "sheet_id": card.sheet_id.as_str(), "title": trimmed }));
            }
            views.center_stage.gallery_renaming = None;
        }
        return;
    }
    if !text_clip.is_positive() || text_clip.width() < 20.0 {
        return;
    }

    let title_pos = pos2(card_rect.left() + 8.0, card_rect.bottom() - 31.0);
    let active_pill_w = if card.active { 38.0 } else { 0.0 };
    let max_title_w = (text_clip.right() - title_pos.x - 4.0 - active_pill_w).min(card_rect.width() - 16.0).max(16.0);
    let mut title_job =
        egui::text::LayoutJob::single_section(card.title.clone(), egui::text::TextFormat::simple(FontId::proportional(11.5), tokens.text_primary));
    title_job.wrap = egui::text::TextWrapping { max_width: max_title_w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = ui.painter().layout_job(title_job);
    ui.painter().with_clip_rect(text_clip).galley(title_pos, galley, tokens.text_primary);

    if card.active {
        let active_lbl = tr_or_fallback(locale, "gallery.badge.active", "Active");
        let pill_rect = Rect::from_min_max(
            pos2((card_rect.right() - 44.0).max(title_pos.x + 12.0), card_rect.bottom() - 31.0),
            pos2(card_rect.right() - 6.0, card_rect.bottom() - 17.0),
        );
        ui.painter().rect(
            pill_rect,
            CornerRadius::same(3),
            tokens.surface_recessed,
            Stroke::new(tokens.metrics.stroke_thin_pt, tokens.border),
            StrokeKind::Inside,
        );
        ui.painter().text(pill_rect.center(), egui::Align2::CENTER_CENTER, active_lbl, FontId::proportional(9.0), tokens.text_strong);
    }

    let sub_pos = pos2(card_rect.left() + 8.0, card_rect.bottom() - 15.0);
    let sub_text = format!("{} · {}", card.badge_label, card.subtitle);
    let mut sub_job = egui::text::LayoutJob::single_section(
        sub_text,
        egui::text::TextFormat::simple(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted),
    );
    sub_job.wrap = egui::text::TextWrapping {
        max_width: (text_clip.right() - sub_pos.x - 4.0).min(card_rect.width() - 16.0).max(16.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let sub_galley = ui.painter().layout_job(sub_job);
    ui.painter().with_clip_rect(text_clip).galley(sub_pos, sub_galley, tokens.text_muted);
}

fn render_gallery_card_checkbox(
    ui: &mut egui::Ui,
    draw: GalleryCardDrawCtx<'_>,
    chk_rect: Rect,
    card_hovered: bool,
    enabled: bool,
    meta: &mut FrameInspectMeta,
) -> Option<egui::Response> {
    let GalleryCardDrawCtx { locale, card, tokens, .. } = draw;
    let vis_chk = chk_rect.intersect(ui.clip_rect());
    if vis_chk.height() < 4.0 || vis_chk.width() < 4.0 {
        return None;
    }
    let r = ui.interact(
        chk_rect,
        ui.make_persistent_id(("gallery_card_chk", card.sheet_id.as_str())),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    if card.selected || card_hovered || r.hovered() {
        let box_rect = Rect::from_center_size(chk_rect.center(), Vec2::splat(16.0));
        let box_fill = if card.selected { tokens.accent } else { tokens.panel_bg };
        let box_stroke = if card.selected { tokens.accent } else { tokens.border };
        ui.painter().rect(box_rect, CornerRadius::same(3), box_fill, Stroke::new(1.2, box_stroke), StrokeKind::Inside);
        if card.selected {
            let p1 = pos2(box_rect.left() + 3.5, box_rect.center().y);
            let p2 = pos2(box_rect.left() + 6.8, box_rect.bottom() - 4.0);
            let p3 = pos2(box_rect.right() - 3.5, box_rect.top() + 4.0);
            ui.painter().line_segment([p1, p2], Stroke::new(1.8, tokens.on_accent));
            ui.painter().line_segment([p2, p3], Stroke::new(1.8, tokens.on_accent));
        }
    }
    let chk_wid = format!("gallery.card.select.{}", card.sheet_id.as_str());
    let chk_lbl = format!("{}: {}", tr_or_fallback(locale, "gallery.select_card", "Select"), card.title);
    let chk_a11y = chk_lbl.clone();
    let chk_selected = card.selected;
    r.widget_info(move || egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, chk_selected, chk_a11y.clone()));
    meta.record_response(
        ui,
        &r,
        FrameInspectEntryInput {
            widget_id: &chk_wid,
            role: "checkbox",
            label: &chk_lbl,
            command_id: None,
            value: Some(json!(card.selected)),
            enabled,
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            ellipsized: false,
            full_label: None,
        },
    );
    Some(r)
}

fn render_gallery_card(
    ui: &mut egui::Ui,
    app: &UiApp,
    doc: &ProjectDocument,
    spec: GalleryCardSpec<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let GalleryCardSpec { card, visible_ids, selected_ids_snapshot, views, size, tokens, enabled } = spec;
    let locale = app.locale.as_str();
    let (card_rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let is_focused = views.center_stage.gallery_focused_id.as_ref() == Some(&card.sheet_id);

    let bg = if card.selected {
        tokens.selection_unemphasized_bg
    } else if card.active {
        tokens.active_bg
    } else if resp.hovered() {
        tokens.hover_bg
    } else {
        tokens.panel_bg
    };
    let (border_w, border_col) = if card.selected {
        (2.0, tokens.accent)
    } else if card.active {
        (1.5, tokens.text_strong)
    } else {
        (1.0, tokens.border)
    };
    ui.painter().rect(card_rect, CornerRadius::same(4), bg, Stroke::new(border_w, border_col), StrokeKind::Inside);
    if is_focused {
        ui.painter().rect_stroke(card_rect.expand(1.5), CornerRadius::same(5), Stroke::new(1.5, tokens.focus_ring), StrokeKind::Outside);
    }

    let synchronous_raster = views.canvas_view.synchronous_raster;
    let thumb_rect = Rect::from_min_max(
        pos2(card_rect.left() + tokens.metrics.space_sm, card_rect.top() + tokens.metrics.space_xs),
        pos2(card_rect.right() - tokens.metrics.space_sm, card_rect.bottom() - 34.0),
    );
    paint_gallery_thumbnail(ui, app, doc, card, thumb_rect, tokens, synchronous_raster);
    let draw = GalleryCardDrawCtx { locale, card, card_rect, tokens };
    render_gallery_card_label_area(ui, draw, views, meta, intents);

    let wid = format!("gallery.card.{}", card.sheet_id.as_str());
    let card_a11y = format!("{} ({})", card.title, card.badge_label);
    resp.widget_info(move || egui::WidgetInfo::labeled(egui::Role::Button, enabled, card_a11y.clone()));
    meta.record_response(
        ui,
        &resp,
        inspect_input(
            &wid,
            "button",
            &card.title,
            Some("sheet.select"),
            Some(json!({ "sheet_id": card.sheet_id.as_str() })),
            enabled,
            ZLayer::Workspace,
        ),
    );

    // Paint and interact with corner selection checkbox LAST so pointer hits inside `chk_rect`
    // cleanly toggle batch selection without navigating or dismissing the gallery.
    let chk_rect = Rect::from_min_size(Pos2::new(card_rect.left() + 4.0, card_rect.top() + 4.0), Vec2::splat(tokens.metrics.min_hit_target_pt))
        .intersect(card_rect);
    let chk_resp = render_gallery_card_checkbox(ui, draw, chk_rect, resp.hovered(), enabled, meta);

    if (resp.secondary_clicked() || chk_resp.as_ref().is_some_and(egui::Response::secondary_clicked)) && enabled {
        let anchor_pos = ui.ctx().pointer_latest_pos().unwrap_or_else(|| card_rect.center());
        views.center_stage.gallery_context_menu =
            Some(GalleryContextMenuState { sheet_id: card.sheet_id.clone(), anchor_pos, folder: card.folder, is_frozen: card.is_frozen });
    } else if chk_resp.as_ref().is_some_and(egui::Response::clicked) && enabled {
        let mut next = selected_ids_snapshot.clone();
        if !next.remove(&card.sheet_id) {
            next.insert(card.sheet_id.clone());
        }
        views.center_stage.gallery_anchor_id = Some(card.sheet_id.clone());
        views.center_stage.gallery_focused_id = Some(card.sheet_id.clone());
        intents.push(UiIntent::SetGallerySelection { sheet_ids: next.into_iter().collect() });
    } else if resp.clicked() && enabled {
        let pointer_in_chk = ui.ctx().pointer_latest_pos().is_some_and(|p| chk_rect.contains(p));
        if pointer_in_chk {
            return;
        }
        let modifiers = ui.input(|i| {
            i.events
                .iter()
                .rev()
                .find_map(|ev| match ev {
                    egui::Event::PointerButton { pressed: false, modifiers, .. } => Some(*modifiers),
                    _ => None,
                })
                .unwrap_or(i.modifiers)
        });
        if modifiers.command || modifiers.ctrl {
            let mut next = selected_ids_snapshot.clone();
            if !next.remove(&card.sheet_id) {
                next.insert(card.sheet_id.clone());
            }
            views.center_stage.gallery_anchor_id = Some(card.sheet_id.clone());
            intents.push(UiIntent::SetGallerySelection { sheet_ids: next.into_iter().collect() });
        } else if modifiers.shift {
            let anchor_pos = views
                .center_stage
                .gallery_anchor_id
                .as_ref()
                .and_then(|aid| visible_ids.iter().position(|id| id == aid))
                .or_else(|| visible_ids.iter().position(|id| selected_ids_snapshot.contains(id)))
                .or_else(|| doc.active_sheet_id.as_ref().and_then(|aid| visible_ids.iter().position(|id| id == aid)))
                .unwrap_or(0);
            let click_pos = visible_ids.iter().position(|id| id == &card.sheet_id).unwrap_or(anchor_pos);
            let (lo, hi) = if anchor_pos <= click_pos { (anchor_pos, click_pos) } else { (click_pos, anchor_pos) };
            let range_ids: Vec<SheetId> = visible_ids[lo..=hi].to_vec();
            if views.center_stage.gallery_anchor_id.is_none() {
                views.center_stage.gallery_anchor_id = visible_ids.get(anchor_pos).cloned();
            }
            intents.push(UiIntent::SetGallerySelection { sheet_ids: range_ids });
        } else {
            views.center_stage.gallery_anchor_id = Some(card.sheet_id.clone());
            intents.push(UiIntent::SetLauncherOpen { open: false });
            intents.push(UiIntent::SetGallerySelection { sheet_ids: vec![card.sheet_id.clone()] });
            intents.push_engine("sheet.select", json!({ "sheet_id": card.sheet_id.as_str() }));
            intents.push_ui("ui.gallery.toggle", json!({ "open": false }));
        }
    }
}

fn truncate_thumb_text(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_owned()
    } else {
        let mut out: String = trimmed.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn data_sheet_preview_headers(arch: &TableArchetype) -> Vec<String> {
    match arch {
        TableArchetype::Xy(t) => {
            let mut cols = vec![if t.x_title.trim().is_empty() { String::from("X") } else { t.x_title.clone() }];
            for d in t.data_sets.iter().take(2) {
                cols.push(d.title.clone());
            }
            cols
        }
        TableArchetype::Survival(t) => {
            let mut cols = vec![String::from("Time")];
            for c in t.cohorts.iter().take(2) {
                cols.push(c.title.clone());
            }
            cols
        }
        TableArchetype::Column(t) => t.data_sets.iter().take(3).map(|d| d.title.clone()).collect(),
        TableArchetype::Grouped(t) => t.data_sets.iter().take(3).map(|d| d.title.clone()).collect(),
        TableArchetype::Contingency(t) => t.data_sets.iter().take(3).map(|d| d.title.clone()).collect(),
        TableArchetype::PartsOfWhole(t) => t.data_sets.iter().take(3).map(|d| d.title.clone()).collect(),
        TableArchetype::MultipleVariables(t) => t.variables.iter().take(3).map(|v| v.title.clone()).collect(),
        TableArchetype::Nested(t) => t.groups.iter().take(3).map(|g| g.title.clone()).collect(),
    }
}

fn data_sheet_preview_cell_text(arch: &TableArchetype, row: usize, visual_col: usize, app: &UiApp) -> String {
    let has_leading_x = matches!(arch, TableArchetype::Xy(_) | TableArchetype::Survival(_));
    let cell_opt = if has_leading_x && visual_col == 0 {
        arch.x_or_time_cell(row, 0)
    } else {
        let data_col = if has_leading_x { visual_col.saturating_sub(1) } else { visual_col };
        arch.data_cell(curvebook_doc::CellCoord::new(row, data_col, 0))
    };
    cell_opt.map(|c| crate::grid_view::format_cell_display(c, &app.locale_preset)).unwrap_or_default()
}

fn paint_gallery_status_cues(painter: &egui::Painter, rect: Rect, card: &PortfolioCard, scene_warn: bool, tokens: &Tokens) {
    let mut right_cursor = rect.right() - 6.0;
    if let Some(tag) = card.color_tag {
        let dot_center = pos2(right_cursor - 4.0, rect.top() + 8.0);
        painter.circle_filled(dot_center, 4.5, rgba8_to_color32(tag));
        painter.circle_stroke(dot_center, 4.5, Stroke::new(0.8, tokens.border));
        right_cursor -= 13.0;
    }

    let badge: Option<(&str, Color32)> = if card.is_frozen {
        Some(("Frozen", tokens.stale))
    } else if card.is_uninitialized {
        Some(("Uninit", tokens.border))
    } else if card.is_warning_or_stale || scene_warn {
        Some(("Warn", tokens.warning))
    } else {
        None
    };

    if let Some((txt, stroke_col)) = badge
        && right_cursor - rect.left() >= 48.0
    {
        let pill_w = (txt.len() as f32) * 5.6 + 8.0;
        let pill = Rect::from_min_max(pos2((right_cursor - pill_w).max(rect.left() + 26.0), rect.top() + 3.0), pos2(right_cursor, rect.top() + 15.0));
        painter.rect(
            pill,
            CornerRadius::same(3),
            tokens.surface_recessed,
            Stroke::new(tokens.metrics.stroke_thin_pt, stroke_col),
            StrokeKind::Inside,
        );
        painter.text(pill.center(), egui::Align2::CENTER_CENTER, txt, FontId::proportional(8.5), tokens.text_strong);
    }
}

fn paint_gallery_thumbnail(
    ui: &egui::Ui,
    app: &UiApp,
    doc: &ProjectDocument,
    card: &PortfolioCard,
    rect: Rect,
    tokens: &Tokens,
    synchronous_raster: bool,
) {
    if !rect.is_positive() {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect(rect, CornerRadius::same(2), tokens.surface_bg, Stroke::new(0.8, tokens.border), StrokeKind::Inside);
    let mut scene_warn = false;

    match card.folder {
        SheetFolderKind::Graphs | SheetFolderKind::Layouts => {
            if card.is_uninitialized {
                painter.rect_filled(rect.shrink(2.0), CornerRadius::same(2), tokens.surface_recessed);
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Uninitialized", FontId::proportional(10.0), tokens.text_muted);
            } else {
                scene_warn =
                    super::canvas_view::render_gallery_scene_thumbnail(ui, app, &card.sheet_id, rect.shrink(2.0), tokens, synchronous_raster);
            }
        }
        SheetFolderKind::Data => {
            if let Some(ds) = doc.data_sheet(&card.sheet_id) {
                let rows = ds.archetype.row_count();
                let cols = ds.archetype.column_count();
                let headers = data_sheet_preview_headers(&ds.archetype);
                let col_n = headers.len().clamp(1, 3);
                let header_h = 13.0_f32;
                let hdr_rect = Rect::from_min_max(rect.min, pos2(rect.right(), (rect.top() + header_h).min(rect.bottom())));
                painter.rect_filled(hdr_rect, CornerRadius::ZERO, tokens.header_bg);
                let col_w = (rect.width() / (col_n as f32)).max(18.0);
                for (c_idx, hdr) in headers.iter().take(col_n).enumerate() {
                    let x = rect.left() + (c_idx as f32) * col_w + 4.0;
                    painter.text(
                        pos2(x, hdr_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        truncate_thumb_text(hdr, 8),
                        FontId::proportional(8.5),
                        tokens.text_strong,
                    );
                }
                let row_h = 10.0_f32;
                for r_idx in 0..rows.min(3) {
                    let y0 = hdr_rect.bottom() + (r_idx as f32) * row_h;
                    if y0 + row_h > rect.bottom() - 9.0 {
                        break;
                    }
                    painter.line_segment([pos2(rect.left() + 2.0, y0), pos2(rect.right() - 2.0, y0)], Stroke::new(0.6, tokens.divider));
                    for c_idx in 0..col_n {
                        let cell_txt = data_sheet_preview_cell_text(&ds.archetype, r_idx, c_idx, app);
                        let x = rect.left() + (c_idx as f32) * col_w + 4.0;
                        painter.text(
                            pos2(x, y0 + row_h * 0.5),
                            egui::Align2::LEFT_CENTER,
                            truncate_thumb_text(&cell_txt, 8),
                            FontId::proportional(8.5),
                            tokens.text_primary,
                        );
                    }
                }
                painter.text(
                    pos2(rect.left() + 5.0, rect.bottom() - 2.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{rows} × {cols}"),
                    FontId::proportional(8.0),
                    tokens.text_muted,
                );
            }
        }
        SheetFolderKind::Results => {
            if let Some(rs) = doc.result_sheet(&card.sheet_id) {
                let kind_title = truncate_thumb_text(&rs.analysis.kind.derive_auto_title(), 26);
                painter.text(
                    pos2(rect.left() + 6.0, rect.top() + 5.0),
                    egui::Align2::LEFT_TOP,
                    kind_title,
                    FontId::proportional(9.0),
                    tokens.text_strong,
                );

                let (status_lbl, status_col) = match &rs.status {
                    curvebook_doc::ResultStatus::Failed { .. } => ("Failed", tokens.link_red),
                    curvebook_doc::ResultStatus::Dirty => ("Pending", tokens.warning),
                    curvebook_doc::ResultStatus::Imported => ("Imported", tokens.accent),
                    curvebook_doc::ResultStatus::ImportedStale => ("Imported (stale)", tokens.warning),
                    curvebook_doc::ResultStatus::UpToDate => {
                        if rs.checklist.iter().any(|c| c.status == curvebook_doc::ChecklistStatus::Violated) {
                            ("Violated", tokens.link_red)
                        } else if rs.checklist.iter().any(|c| c.status == curvebook_doc::ChecklistStatus::Warn) {
                            ("Warn", tokens.warning)
                        } else {
                            ("Pass", tokens.link_green)
                        }
                    }
                };
                let badge_r = Rect::from_min_max(
                    pos2(rect.left() + 6.0, rect.top() + 17.0),
                    pos2((rect.left() + 64.0).min(rect.right() - 6.0), rect.top() + 29.0),
                );
                if badge_r.is_positive() {
                    painter.rect(badge_r, CornerRadius::same(3), tokens.surface_recessed, Stroke::new(0.8, status_col), StrokeKind::Inside);
                    painter.text(badge_r.center(), egui::Align2::CENTER_CENTER, status_lbl, FontId::proportional(8.0), tokens.text_strong);
                }

                let headline = if let Some(sc) = rs.scalars.first() {
                    format!("{} = {:.4}", sc.key, sc.value)
                } else if let Some(tab) = rs.tabs.first()
                    && let Some(row) = tab.rows.first()
                    && let Some(cell) = row.iter().find(|c| !matches!(c, curvebook_doc::ResultCell::Empty))
                {
                    let val = crate::egui_shell::results_view::format_result_cell_display_text(cell, rs, tab);
                    let lbl = tab.row_labels.first().map(String::as_str).unwrap_or("");
                    if lbl.is_empty() { val } else { format!("{lbl}: {val}") }
                } else {
                    String::from(status_lbl)
                };
                painter.text(
                    pos2(rect.left() + 6.0, (rect.top() + 32.0).min(rect.bottom() - 8.0)),
                    egui::Align2::LEFT_TOP,
                    truncate_thumb_text(&headline, 26),
                    FontId::proportional(8.5),
                    tokens.text_primary,
                );
            }
        }
        SheetFolderKind::Info => {
            if let Some(info) = doc.info_sheet(&card.sheet_id) {
                let mut y = rect.top() + 5.0;
                for c in info.constants.iter().filter(|c| !c.value.display_text().is_empty()).chain(info.constants.iter()).take(2) {
                    if y > rect.bottom() - 12.0 {
                        break;
                    }
                    let val_txt = c.value.display_text();
                    let line = if val_txt.is_empty() { format!("{}: —", c.name) } else { format!("{}: {val_txt}", c.name) };
                    painter.text(
                        pos2(rect.left() + 6.0, y),
                        egui::Align2::LEFT_TOP,
                        truncate_thumb_text(&line, 26),
                        FontId::proportional(8.5),
                        tokens.text_primary,
                    );
                    y += 10.5;
                }
                if let Some(note_line) = info.notes_markdown.lines().map(str::trim).find(|l| !l.is_empty())
                    && y <= rect.bottom() - 8.0
                {
                    painter.text(
                        pos2(rect.left() + 6.0, y + 1.0),
                        egui::Align2::LEFT_TOP,
                        truncate_thumb_text(note_line, 28),
                        FontId::proportional(8.0),
                        tokens.text_muted,
                    );
                }
            }
        }
    }

    paint_gallery_status_cues(&painter, rect, card, scene_warn, tokens);
}
