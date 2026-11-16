//! Project Navigator, Family lineage tree, and Info / Floating Notes view builders (`R10.17`).

use curvebook_doc::{InfoSheet, SheetFolderKind, SheetSection};
use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::interaction::{InfoRowHitRect, NavigatorEntryHitRect, UiEventOutcome};
use crate::layout::a11y::{FocusScope, WidgetRole};
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldSpec, emit_button, emit_scrollbars_for_meta, emit_text_field, paint_text_fitted, widget_state,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::UiTextStyle;

use super::a11y::{disclosure_meta, labeled_meta, selectable_meta};

/// Transient view state owned by the navigator / notes shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NavNotesViewState {
    pub filter_query: String,
    pub active_note_id: Option<String>,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `nav_notes` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut NavNotesViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Build the Project Navigator folder tree and bottom Family pane inside `rect`.
pub fn build_navigator_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, None, rect, builder.palette.border, 1);

    let fam_h = if builder.app.family_open && rect.height >= 160 { rect.height / 3 } else { 24u32.min(rect.height) };
    let tree_h = rect.height.saturating_sub(fam_h);
    let tree_rect = LayoutRectPx::new(rect.x, rect.y, rect.width, tree_h);
    let fam_rect = LayoutRectPx::new(rect.x, rect.y + i32::try_from(tree_h).unwrap_or(0), rect.width, fam_h);

    let row_h = 20u32;
    let total_items = 5usize + builder.app.nav.folders.iter().map(|f| f.entries.len()).sum::<usize>();
    let content_h = (total_items as u32) * row_h + 28;
    let scroll_id = String::from("scroll.navigator");
    let scroll_off = builder.scroll_offset(&scroll_id);
    let scroll_meta = ScrollMeta::new(scroll_id.clone(), tree_rect, [tree_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let mut tree_a11y = labeled_meta(WidgetRole::TreeItem, "Project Navigator", FocusScope::Workspace, "Cmd+Shift+N");
    tree_a11y.expanded = Some(true);
    let tree_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("navigator.tree"),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: tree_rect,
        a11y: tree_a11y,
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&tree_wid, scroll_meta.clone());
    builder.paint_fill(ZLayer::Workspace, Some(&tree_wid), tree_rect, builder.palette.panel);

    let inner = tree_rect.inset(4);
    builder.push_clip(inner);
    let y_scroll = scroll_meta.scroll_offset_px[1].round() as i32;
    let mut cur_y = inner.y + 4 - y_scroll;
    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    let bold_style = UiTextStyle::ui_bold(size_px);
    let item_style = UiTextStyle::ui_sans(size_px);

    for (f_idx, folder) in builder.app.nav.folders.iter().enumerate() {
        let collapsed = folder.collapsed || builder.app.nav.collapsed_sections.contains(&folder.section);
        let folder_kind = folder_kind_for_section(folder.section);
        if cur_y + i32::try_from(row_h).unwrap_or(20) > inner.y && cur_y < inner.bottom() {
            let f_rect = LayoutRectPx::new(inner.x, cur_y, inner.width.saturating_sub(12), row_h);
            let fid = format!("nav.folder.{f_idx}");
            let fst = widget_state(builder, &fid, ControlEnablement::enabled());
            let fa = disclosure_meta(folder.label.clone(), FocusScope::Workspace, format!("nav.folder.{f_idx}"), !collapsed);
            let fwid = builder.push_node(NodeSpec {
                id: WidgetId::from(fid),
                parent: Some(tree_wid.clone()),
                z_layer: ZLayer::Workspace,
                rect_px: f_rect,
                a11y: fa,
                state: fst.clone(),
                action: WidgetAction::ToggleFolder(folder.section),
            });
            let bg = if fst.hovered { builder.palette.header } else { builder.palette.panel };
            builder.paint_fill(ZLayer::Workspace, Some(&fwid), f_rect, bg);
            let marker = if collapsed { "+" } else { "-" };
            let txt = format!("{marker} {}", folder.label);
            paint_text_fitted(
                builder,
                &fwid,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: f_rect,
                    pad_x: 4,
                    text: &txt,
                    style: &bold_style,
                    color: builder.palette.text,
                    scroll_container: Some(&scroll_id),
                },
            );
        }
        cur_y += i32::try_from(row_h).unwrap_or(20);
        if collapsed {
            continue;
        }
        for (e_idx, entry) in folder.entries.iter().enumerate() {
            if cur_y + i32::try_from(row_h).unwrap_or(20) > inner.y && cur_y < inner.bottom() {
                let row_rect = LayoutRectPx::new(inner.x + 8, cur_y, inner.width.saturating_sub(20), row_h);
                let section = folder.section;
                let eid = format!("nav.sheet.{}", entry.sheet_id.as_str());
                let selected = builder.app.document().active_sheet_id.as_ref() == Some(&entry.sheet_id);
                let est = widget_state(builder, &eid, ControlEnablement::enabled());
                let ea = selectable_meta(WidgetRole::TreeItem, entry.title.clone(), FocusScope::Workspace, format!("nav.{f_idx}.{e_idx}"), selected);
                let ewid = builder.push_node(NodeSpec {
                    id: WidgetId::from(eid),
                    parent: Some(tree_wid.clone()),
                    z_layer: ZLayer::Workspace,
                    rect_px: row_rect,
                    a11y: ea,
                    state: est.clone(),
                    action: WidgetAction::SwitchSheet { sheet_id: entry.sheet_id.clone(), section },
                });
                let bg = if selected {
                    builder.palette.accent
                } else if est.hovered {
                    builder.palette.header
                } else {
                    builder.palette.panel
                };
                let fg = if selected { curvebook_palette::Rgba8::WHITE } else { builder.palette.text };
                builder.paint_fill(ZLayer::Workspace, Some(&ewid), row_rect, bg);
                paint_text_fitted(
                    builder,
                    &ewid,
                    FittedTextSpec {
                        z_layer: ZLayer::Workspace,
                        rect: row_rect,
                        pad_x: 6,
                        text: &entry.title,
                        style: &item_style,
                        color: fg,
                        scroll_container: Some(&scroll_id),
                    },
                );
                builder.tree.navigator_entries.push(NavigatorEntryHitRect {
                    sheet_id: entry.sheet_id.clone(),
                    folder: folder_kind,
                    index_in_folder: e_idx,
                    rect: row_rect.to_workspace_rect(),
                });
            }
            cur_y += i32::try_from(row_h).unwrap_or(20);
        }
    }
    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(tree_wid));

    build_family_pane(builder, fam_rect);
}

/// Build the Family lineage strip inside `rect`.
pub fn build_family_pane(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width == 0 || rect.height < 16 {
        return;
    }
    builder.paint_line(ZLayer::Workspace, None, [rect.x, rect.y], [rect.right(), rect.y], builder.palette.border, 1);
    let hdr_h = 22u32.min(rect.height);
    let hdr = LayoutRectPx::new(rect.x + 2, rect.y + 1, rect.width.saturating_sub(4), hdr_h);
    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "navigator.family_toggle",
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: hdr,
            label: if builder.app.family_open { "Family (-)" } else { "Family (+)" },
            keyboard_path: "Cmd+Shift+F",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: builder.app.family_open,
            shortcut: Some("Cmd+Shift+F"),
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::ToggleFamilyPane,
        },
    );

    if builder.app.family_open && rect.height > hdr_h + 18 {
        builder.push_clip(rect);
        let mut cy = hdr.bottom() + 4;
        let size_px = (11.5 * builder.scale()).clamp(9.0, 20.0);
        let st = UiTextStyle::ui_sans(size_px);
        for (idx, item) in builder.app.nav.family_nodes.iter().enumerate() {
            if cy + 18 >= rect.bottom() {
                break;
            }
            let r = LayoutRectPx::new(rect.x + 6, cy, rect.width.saturating_sub(12), 18);
            let id = format!("family.item.{idx}");
            let a11y = selectable_meta(
                WidgetRole::TreeItem,
                item.title.clone(),
                FocusScope::Workspace,
                format!("family.{idx}"),
                builder.app.document().active_sheet_id.as_ref() == Some(&item.sheet_id),
            );
            let wid = builder.push_node(NodeSpec {
                id: WidgetId::from(id),
                parent: None,
                z_layer: ZLayer::Workspace,
                rect_px: r,
                a11y,
                state: WidgetVisualState::default(),
                action: WidgetAction::SwitchSheet { sheet_id: item.sheet_id.clone(), section: item.section },
            });
            builder.paint_fill(ZLayer::Workspace, Some(&wid), r, builder.palette.panel);
            paint_text_fitted(
                builder,
                &wid,
                FittedTextSpec {
                    z_layer: ZLayer::Workspace,
                    rect: r,
                    pad_x: 4,
                    text: &item.title,
                    style: &st,
                    color: builder.palette.text,
                    scroll_container: None,
                },
            );
            cy += 19;
        }
        builder.pop_clip();
    }
}

/// Build the Info Sheet constants + notes editor view inside `rect`.
pub fn build_info_sheet_view(builder: &mut LayoutBuilder<'_>, info: &InfoSheet, rect: LayoutRectPx, slot_tag: &str) {
    let box_rect = rect.inset(8);
    if box_rect.width < 60 || box_rect.height < 60 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, box_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, None, box_rect, builder.palette.border, 1);

    let half_w = box_rect.width / 2;
    let const_x = box_rect.x + i32::try_from(crate::i18n::mirror_x_for_rtl(0, half_w, box_rect.width, builder.tree.is_rtl)).unwrap_or(0);
    let notes_x =
        box_rect.x + i32::try_from(crate::i18n::mirror_x_for_rtl(half_w, box_rect.width - half_w, box_rect.width, builder.tree.is_rtl)).unwrap_or(0);
    let notes_rect = LayoutRectPx::new(notes_x + 6, box_rect.y + 24, box_rect.width.saturating_sub(half_w + 12), box_rect.height.saturating_sub(30));

    if slot_tag != "secondary" || builder.tree.info_notes_rect.is_none() {
        builder.tree.info_notes_rect = Some(notes_rect.to_workspace_rect());
        builder.tree.info_sheet_id = Some(info.id.clone());
    }

    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    let st = UiTextStyle::ui_sans(size_px);
    let mut y = box_rect.y + 24;
    for (idx, c) in info.constants.iter().enumerate() {
        if y + 20 >= box_rect.bottom() {
            break;
        }
        let r = LayoutRectPx::new(const_x + 6, y, half_w.saturating_sub(12), 18);
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(format!("info.const.{idx}")),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: r,
            a11y: labeled_meta(WidgetRole::TextBox, c.name.clone(), FocusScope::Workspace, format!("info.{idx}")),
            state: WidgetVisualState::default(),
            action: WidgetAction::FocusWidget(format!("info.const.{idx}")),
        });
        builder.paint_fill(ZLayer::Workspace, Some(&wid), r, builder.palette.panel);
        let txt = format!("{}: {}", c.name, c.value.display_text());
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: r,
                pad_x: 4,
                text: &txt,
                style: &st,
                color: builder.palette.text,
                scroll_container: None,
            },
        );
        builder.tree.info_constant_rows.push(InfoRowHitRect {
            sheet_id: info.id.clone(),
            name: c.name.clone(),
            value_text: c.value.display_text(),
            rect: r.to_workspace_rect(),
        });
        y += 20;
    }

    let n_wid = emit_text_field(
        builder,
        TextFieldSpec {
            id: "info.notes",
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: notes_rect,
            label: "Experiment Notes",
            fallback_text: &info.notes_markdown,
            keyboard_path: "info.notes",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            multiline: true,
            enablement: ControlEnablement::enabled(),
        },
    );
    let _ = n_wid;
}

/// Map [`SheetSection`] onto [`SheetFolderKind`].
#[must_use]
pub const fn folder_kind_for_section(section: SheetSection) -> SheetFolderKind {
    match section {
        SheetSection::Data => SheetFolderKind::Data,
        SheetSection::Info => SheetFolderKind::Info,
        SheetSection::Analyses => SheetFolderKind::Results,
        SheetSection::Graphs => SheetFolderKind::Graphs,
        SheetSection::Layouts => SheetFolderKind::Layouts,
    }
}

/// Map [`SheetFolderKind`] onto [`SheetSection`].
#[must_use]
pub const fn section_for_folder_kind(folder: SheetFolderKind) -> SheetSection {
    match folder {
        SheetFolderKind::Data => SheetSection::Data,
        SheetFolderKind::Info => SheetSection::Info,
        SheetFolderKind::Results => SheetSection::Analyses,
        SheetFolderKind::Graphs => SheetSection::Graphs,
        SheetFolderKind::Layouts => SheetSection::Layouts,
    }
}
