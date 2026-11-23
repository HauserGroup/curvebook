//! `Page Layout Composer` modal body (`gui-layout-spec.md` §8.12).

use curvebook_doc::{JournalWidthPreset, LayoutItem, PageOrientation, PanelBadgeStyle, RectPt, SheetId};
use egui::Ui;

use crate::i18n::tr;

use super::helpers::{ButtonEntry, GraphModalCtx, record_button, record_checkbox, record_input};
use super::state::GraphModalEguiState;

pub(super) fn render_page_layout_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let pl = gctx.app.canvas_dialogs.page_layout.clone();
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
                    selected: pl.preset == preset,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = pl.clone();
                next.apply_preset(preset);
                gctx.intents.push_update_page_layout_state(next);
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!("Grid: {} x {}", pl.rows, pl.cols));
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
            let mut next = pl.clone();
            let gids: Vec<SheetId> = gctx.app.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
            next.arrange_grid(&gids, 24.0, 12.0);
            gctx.intents.push_update_page_layout_state(next);
        }
        let orient_lbl = match pl.orientation {
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
            let mut next = pl.clone();
            next.orientation = match next.orientation {
                PageOrientation::Portrait => PageOrientation::Landscape,
                PageOrientation::Landscape => PageOrientation::Portrait,
            };
            gctx.intents.push_update_page_layout_state(next);
        }
    });

    ui.add_space(4.0);
    let mut align_origins = pl.align_plot_origins;
    let chk = ui.checkbox(&mut align_origins, "Align Plot Origins Across Grid");
    record_checkbox(ui, &chk, gctx.meta, "layout.align_plot_origins", "Align Plot Origins Across Grid", align_origins);
    if chk.changed() {
        let mut next = pl.clone();
        next.align_plot_origins = align_origins;
        gctx.intents.push_update_page_layout_state(next);
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
                    selected: pl.panel_badges == st,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = pl.clone();
                next.panel_badges = st;
                gctx.intents.push_update_page_layout_state(next);
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let edit = ui.add(egui::TextEdit::singleline(&mut state.page_callout_draft).desired_width((gctx.body_w - 130.0).max(110.0)));
        record_input(ui, &edit, gctx.meta, "layout.callout_input", "Callout text", &state.page_callout_draft, true);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "layout.add_callout",
                label: tr(gctx.locale, "layout.add_callout"),
                command_id: None,
                selected: false,
                enabled: !state.page_callout_draft.trim().is_empty(),
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = pl.clone();
            next.items.push(LayoutItem::TextCallout {
                id: format!("callout-{}", next.items.len() + 1),
                markdown: state.page_callout_draft.trim().to_owned(),
                bounds_pt: RectPt::new(24.0, 24.0, 180.0, 28.0),
                font_size_pt: 11.0,
            });
            state.page_callout_draft.clear();
            gctx.intents.push_update_page_layout_state(next);
        }
    });
    ui.label(format!("Layout items: {}", pl.items.len()));
}
