//! Welcome launcher & portfolio gallery view builders (`R10.4`).

use serde::{Deserialize, Serialize};

use crate::UiApp;
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{FocusScope, WidgetRole};
use crate::layout::widgets::{FittedTextSpec, paint_text_fitted};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::text_bridge::UiTextStyle;

use super::a11y::labeled_meta;
use super::nav_notes::section_for_folder_kind;

/// Transient view state owned by the welcome / portfolio gallery shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WelcomeViewState {
    pub search_query: String,
    pub active_template_index: Option<usize>,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `welcome` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut WelcomeViewState, payload: &str) -> Result<UiEventOutcome, String> {
    let _ = (app, state, payload);
    Ok(UiEventOutcome::default())
}

/// Render the portfolio gallery grid inside `rect`.
pub fn build_portfolio_gallery(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    let cards = &builder.app.nav.gallery.cards;
    builder.paint_fill(ZLayer::Workspace, None, rect, builder.palette.surface);
    builder.push_clip(rect);
    let cols = (rect.width / 190).max(1) as usize;
    let cw = (rect.width.saturating_sub(24)) / (cols as u32).max(1);
    let ch = 120u32;
    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    for (idx, card) in cards.iter().enumerate() {
        let col = idx % cols;
        let row = idx / cols;
        let cx = rect.x + 8 + i32::try_from(col as u32 * cw).unwrap_or(0);
        let cy = rect.y + 8 + i32::try_from(row as u32 * (ch + 8)).unwrap_or(0);
        if cy >= rect.bottom() {
            break;
        }
        let card_rect = LayoutRectPx::new(cx, cy, cw.saturating_sub(8), ch);
        let cid = format!("gallery.card.{idx}");
        let wid = builder.push_node(NodeSpec {
            id: WidgetId::from(cid),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: card_rect,
            a11y: labeled_meta(WidgetRole::Button, card.title.clone(), FocusScope::Workspace, format!("gallery.{idx}")),
            state: WidgetVisualState::default(),
            action: WidgetAction::SwitchSheet { sheet_id: card.sheet_id.clone(), section: section_for_folder_kind(card.folder) },
        });
        let is_sel = builder.app.nav.gallery.selected_ids.contains(&card.sheet_id);
        let fill_col = if is_sel { builder.palette.header } else { builder.palette.panel };
        let stroke_col = if is_sel { builder.palette.accent } else { builder.palette.border };
        builder.paint_fill(ZLayer::Workspace, Some(&wid), card_rect, fill_col);
        builder.paint_stroke(ZLayer::Workspace, Some(&wid), card_rect, stroke_col, if is_sel { 2 } else { 1 });
        paint_text_fitted(
            builder,
            &wid,
            FittedTextSpec {
                z_layer: ZLayer::Workspace,
                rect: card_rect,
                pad_x: 8,
                text: &card.title,
                style: &UiTextStyle::ui_bold(size_px),
                color: builder.palette.text,
                scroll_container: None,
            },
        );
    }
    builder.pop_clip();
}
