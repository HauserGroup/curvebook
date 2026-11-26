//! `Page Layout Composer` modal body (`gui-layout-spec.md` §8.12).

use curvebook_doc::{JournalWidthPreset, LayoutItem, LayoutSpec, PageOrientation, PanelBadgeStyle, RectPt, SheetId, SizePt};
use egui::Ui;

use crate::dialogs_canvas::PageLayoutComposerState;
use crate::egui_shell::UiIntent;
use crate::i18n::tr;

use super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox, record_input};
use super::state::GraphModalEguiState;

fn layout_item_id(item: &LayoutItem) -> &str {
    match item {
        LayoutItem::LiveGraph { id, .. }
        | LayoutItem::LiveTable { id, .. }
        | LayoutItem::TextCallout { id, .. }
        | LayoutItem::EmbeddedImage { id, .. } => id.as_str(),
    }
}

/// Generate a unique callout item ID avoiding collisions with any existing `items` (`AUD-10-53`).
pub(super) fn next_unique_callout_id(items: &[LayoutItem]) -> String {
    let mut n = items.len().saturating_add(1);
    loop {
        let candidate = format!("callout-{n}");
        if !items.iter().any(|it| layout_item_id(it) == candidate) {
            return candidate;
        }
        n = n.saturating_add(1);
    }
}

/// Compute orientation-aware page dimensions in points for `preset` and `orientation` (`AUD-10-53`).
pub(super) fn oriented_page_size_pt(preset: JournalWidthPreset, orientation: PageOrientation) -> SizePt {
    let mut spec = LayoutSpec::from_preset(preset);
    spec.orientation = orientation;
    let raw = spec.page_size_pt();
    match orientation {
        PageOrientation::Portrait if raw.width > raw.height => SizePt::new(raw.height, raw.width),
        PageOrientation::Landscape if raw.height > raw.width => SizePt::new(raw.height, raw.width),
        _ => raw,
    }
}

/// Arrange `graph_ids` across `rows x cols` using the orientation-aware page size (`AUD-10-53`).
pub(super) fn arrange_layout_grid_oriented(draft: &mut PageLayoutComposerState, graph_ids: &[SheetId], margin_pt: f64, gutter_pt: f64) {
    let page = oriented_page_size_pt(draft.preset, draft.orientation);
    let rows = usize::from(draft.rows.max(1));
    let cols = usize::from(draft.cols.max(1));
    let margin = if margin_pt.is_finite() { margin_pt.max(0.0) } else { 14.0 };
    let gutter = if gutter_pt.is_finite() { gutter_pt.max(0.0) } else { 10.0 };
    let usable_w = (page.width - 2.0 * margin - gutter * ((cols.saturating_sub(1)) as f64)).max(24.0);
    let usable_h = (page.height - 2.0 * margin - gutter * ((rows.saturating_sub(1)) as f64)).max(24.0);
    let cell_w = usable_w / (cols as f64);
    let cell_h = usable_h / (rows as f64);

    draft.items.retain(|item| !matches!(item, LayoutItem::LiveGraph { .. }));
    for (idx, gid) in graph_ids.iter().take(rows * cols).enumerate() {
        let r = idx / cols;
        let c = idx % cols;
        let x = margin + (c as f64) * (cell_w + gutter);
        let y = margin + (r as f64) * (cell_h + gutter);
        let badge = draft.panel_badges.badge_for(idx);
        draft.items.push(LayoutItem::LiveGraph {
            id: format!("panel-{}", idx + 1),
            graph_sheet_id: gid.clone(),
            bounds_pt: RectPt::new(x, y, cell_w, cell_h),
            scale_percent: 100.0,
            badge_override: badge,
        });
    }
}

pub(super) fn render_page_layout_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let mut draft = gctx.app.canvas_dialogs.page_layout.clone();
    let mut dirty = false;

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Journal Width Preset:");
        for (lbl, preset) in [
            ("85 mm", JournalWidthPreset::SingleColumn85Mm),
            ("120 mm", JournalWidthPreset::OneAndHalfColumn120Mm),
            ("174 mm", JournalWidthPreset::DoubleColumn174Mm),
            ("A4 Portrait", JournalWidthPreset::A4Portrait),
            ("A4 Landscape", JournalWidthPreset::A4Landscape),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("layout.preset.{preset:?}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.preset == preset,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.apply_preset(preset);
                dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    render_grid_and_orientation_row(ui, gctx, &mut draft, &mut dirty);

    ui.add_space(4.0);
    let mut align_origins = draft.align_plot_origins;
    let chk = ui.checkbox(&mut align_origins, "Align Plot Origins Across Grid");
    record_checkbox(ui, &chk, gctx.meta, "layout.align_plot_origins", "Align Plot Origins Across Grid", align_origins);
    if chk.changed() {
        draft.align_plot_origins = align_origins;
        dirty = true;
    }

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Panel Badges:");
        for (lbl, st) in
            [("A, B, C", PanelBadgeStyle::UpperCaseLetters), ("a, b, c", PanelBadgeStyle::LowerCaseLetters), ("None", PanelBadgeStyle::None)]
        {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("layout.badge.{st:?}"),
                    label: lbl,
                    command_id: None,
                    selected: draft.panel_badges == st,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                draft.panel_badges = st;
                dirty = true;
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let edit = ui.add(egui::TextEdit::singleline(&mut state.page_callout_draft).desired_width((gctx.body_w - 130.0).max(110.0)));
        record_input(ui, &edit, gctx.meta, "layout.callout_input", "Callout text", &state.page_callout_draft, true);
        let has_callout_text = !state.page_callout_draft.trim().is_empty();
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "layout.add_callout",
                label: tr(gctx.locale, "layout.add_callout"),
                command_id: None,
                selected: false,
                enabled: has_callout_text,
                disabled_reason: (!has_callout_text).then_some("Enter callout text first"),
            },
            gctx.tokens,
        )
        .clicked()
            && has_callout_text
        {
            let id = next_unique_callout_id(&draft.items);
            draft.items.push(LayoutItem::TextCallout {
                id,
                markdown: state.page_callout_draft.trim().to_owned(),
                bounds_pt: RectPt::new(24.0, 24.0, 180.0, 28.0),
                font_size_pt: 11.0,
            });
            state.page_callout_draft.clear();
            dirty = true;
        }
    });
    ui.label(format!("Layout items: {}", draft.items.len()));

    if dirty {
        gctx.intents.push(UiIntent::UpdatePageLayoutState(Box::new(draft)));
    }
}

fn render_grid_and_orientation_row(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, draft: &mut PageLayoutComposerState, dirty: &mut bool) {
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!("Grid: {} x {}", draft.rows, draft.cols));

        ui.label("Rows:");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "layout.rows.minus",
                label: "-",
                command_id: None,
                selected: false,
                enabled: draft.rows > 1,
                disabled_reason: (draft.rows <= 1).then_some("Minimum 1 row"),
            },
            gctx.tokens,
        )
        .clicked()
            && draft.rows > 1
        {
            draft.rows = draft.rows.saturating_sub(1).max(1);
            *dirty = true;
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "layout.rows.plus",
                label: "+",
                command_id: None,
                selected: false,
                enabled: draft.rows < 6,
                disabled_reason: (draft.rows >= 6).then_some("Maximum 6 rows"),
            },
            gctx.tokens,
        )
        .clicked()
            && draft.rows < 6
        {
            draft.rows = draft.rows.saturating_add(1).min(6);
            *dirty = true;
        }

        ui.label("Cols:");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "layout.cols.minus",
                label: "-",
                command_id: None,
                selected: false,
                enabled: draft.cols > 1,
                disabled_reason: (draft.cols <= 1).then_some("Minimum 1 column"),
            },
            gctx.tokens,
        )
        .clicked()
            && draft.cols > 1
        {
            draft.cols = draft.cols.saturating_sub(1).max(1);
            *dirty = true;
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "layout.cols.plus",
                label: "+",
                command_id: None,
                selected: false,
                enabled: draft.cols < 6,
                disabled_reason: (draft.cols >= 6).then_some("Maximum 6 columns"),
            },
            gctx.tokens,
        )
        .clicked()
            && draft.cols < 6
        {
            draft.cols = draft.cols.saturating_add(1).min(6);
            *dirty = true;
        }

        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "layout.arrange",
                label: tr(gctx.locale, "layout.auto_arrange"),
                command_id: None,
                selected: false,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let gids: Vec<SheetId> = gctx.app.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
            arrange_layout_grid_oriented(draft, &gids, 24.0, 12.0);
            *dirty = true;
        }
        let orient_lbl = match draft.orientation {
            PageOrientation::Portrait => "Portrait",
            PageOrientation::Landscape => "Landscape",
        };
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "layout.orientation",
                label: orient_lbl,
                command_id: None,
                selected: true,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            draft.orientation = match draft.orientation {
                PageOrientation::Portrait => PageOrientation::Landscape,
                PageOrientation::Landscape => PageOrientation::Portrait,
            };
            *dirty = true;
        }
    });
}
