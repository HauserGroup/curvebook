//! Unit tests for the virtualized `Column` / multi-archetype data grid.

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, ImportLinkSpec, LinkSource, LinkedDataRegion, ProjectDocument, SheetCellRange, SheetId,
    TableArchetype,
};
use curvebook_engine::Session;
use curvebook_tools::GridCellCoord;

use super::{ColumnGridEguiState, ColumnGridRenderContext, GridInlineEditBuffer, GridInlineEditTarget, render_column_grid};
use crate::UiApp;
use crate::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use crate::egui_shell::{PendingUiIntents, Tokens};
use crate::interaction::ViewportMetrics;

fn build_column_app(cols: usize, rows: usize) -> UiApp {
    let mut doc = ProjectDocument::default();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows).expect("column table");
    for c in 0..cols {
        col.data_sets[c].title = format!("Group {}", c + 1);
        for r in 0..rows {
            col.data_sets[c].cells[r][0] = Cell::number((r as f64) + (c as f64) * 0.1);
        }
    }
    let ds = DataSheet::new("col-1", "Column Test", TableArchetype::Column(col)).expect("data sheet");
    doc.add_data_sheet(ds).expect("add data sheet");
    doc.select_in_navigator(&SheetId::from("col-1")).expect("select sheet");
    UiApp::from_session(Session::from_document(doc))
}

#[test]
fn default_grid_state_enables_inspect_meta_and_inherits_metrics() {
    let state = ColumnGridEguiState::default();
    assert!(state.record_inspect_meta);
    assert!(!state.row_titles_collapsed);
    let tokens = Tokens::default();
    let ctx = ColumnGridRenderContext::new(&tokens, false);
    assert_eq!(ctx.metrics.grid_row_h, 24.0);
    assert!(!ctx.modal_blocked);
}

#[test]
fn one_million_cell_column_grid_renders_strictly_visible_window() {
    let app = build_column_app(1_000, 1_000);
    let tokens = Tokens::default();
    let state = (app, ColumnGridEguiState::default());
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        state,
        ViewportMetrics::new(1280.0, 800.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, (app, grid_state)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let rep = render_column_grid(ui, app, grid_state, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
            assert!(rep.rendered_cells < 600, "must virtualize 1M cells, rendered {}", rep.rendered_cells);
            assert!(rep.rendered_cells > 100);
            assert!(rep.vertical_scrollbar_rect_pt.is_some());
            assert!(rep.horizontal_scrollbar_rect_pt.is_some());
            let _ = intents.commit_if_settled(app, ui.ctx().will_discard());
        },
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    let parity = driver.verify_pixel_to_hit_box_parity();
    assert!(parity.is_clean(), "1M-cell parity violations: {:?}", parity.violations);
    assert!(driver.check_scrollbar_non_overlap().is_empty());
}

#[test]
fn linked_region_guard_opens_on_double_click_and_blocks_outside_clicks() {
    let mut app = build_column_app(3, 8);
    {
        let mut doc = app.document().clone();
        let ds = doc.data_sheet_mut(&SheetId::from("col-1")).expect("sheet");
        ds.add_linked_region(LinkedDataRegion {
            id: String::from("link-1"),
            spec: ImportLinkSpec { source: LinkSource::Clipboard, filter: Default::default(), placement: Default::default() },
            target_range: SheetCellRange::new(curvebook_doc::CellCoord::new(0, 0, 0), curvebook_doc::CellCoord::new(2, 1, 0)),
            provenance_summary: String::from("Raw Plate › A1:B3"),
        })
        .expect("add link");
        *app.session_mut() = Session::from_document(doc);
        app.sync_views();
    }
    let tokens = Tokens::default();
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        (app, ColumnGridEguiState::default()),
        ViewportMetrics::new(1024.0, 640.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, (app, grid_state)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_column_grid(ui, app, grid_state, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
            let _ = intents.commit_if_settled(app, ui.ctx().will_discard());
        },
    );
    let center = driver.frame_meta().find_by_widget_id("grid.cell.1.0.0").expect("cell meta").clipped_rect.center();
    driver.click_at(center).expect("click 1");
    driver.click_at(center).expect("click 2");
    assert!(driver.state().1.active_link_popover.is_some());
    assert!(driver.frame_meta().find_by_widget_id("grid.link_popover.panel").is_some());
}

#[test]
fn buffered_inline_edit_commits_once_on_enter_and_cancels_on_escape() {
    let app = build_column_app(2, 5);
    let tokens = Tokens::default();
    let grid_state = ColumnGridEguiState {
        inline_edit: Some(GridInlineEditBuffer {
            target: GridInlineEditTarget::Cell(GridCellCoord::new(0, 0, 0)),
            draft: String::from("42.75"),
            focused_once: false,
        }),
        ..Default::default()
    };
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        (app, grid_state),
        ViewportMetrics::new(900.0, 560.0, 1.0),
        tokens,
        build_driver_font_definitions(),
        |ui, meta, (app, grid_state)| {
            let mut intents = PendingUiIntents::new();
            let t = Tokens::get(ui.ctx());
            let _ = render_column_grid(ui, app, grid_state, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
            let _ = intents.commit_if_settled(app, ui.ctx().will_discard());
        },
    );
    driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
    assert!(driver.state().1.inline_edit.is_none());
    assert_eq!(driver.state().0.session().undo_depth(), 1);
    let cell = driver.state().0.document().data_sheet(&SheetId::from("col-1")).unwrap().cell(curvebook_doc::CellCoord::new(0, 0, 0)).unwrap();
    assert_eq!(cell.value, curvebook_doc::CellValue::Number(42.75));
}
