//! Interactive tool state machines for CurveBook: spreadsheet cell selection, graph object
//! manipulation, axis range/break dragging, pairwise significance bracket placement, text/arrow
//! callouts and publication layout alignment.
//!
//! Layer L4 crate: pointer and keyboard events in, engine command specs and overlays out.
//! Governing spec: `docs/development.md` (§Cursors) and `docs/evidence/user-interface.md`.
#![forbid(unsafe_code)]

pub mod canvas;
pub mod grid;

use serde::{Deserialize, Serialize};

pub use canvas::{
    CanvasToolMode, CanvasToolState, JournalWidthPreset, NavigatorDragState, PanZoomInertiaState, PhysicalUnit, RulerTick, ToolCommandIntent,
    build_ruler_ticks,
};
pub use grid::{GridCellCoord, GridToolState, GridViewportSpec, GridVisibleWindow, SelectionQuickStats};

/// Logical cursor shapes requested by interactive tools. Every variant is documented in
/// `crates/tools/src/cursors.toml` (checked by `cargo xtask cursors`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Cursor {
    /// Standard selection pointer over graphs, layouts and inspector chrome.
    Select,
    /// Spreadsheet cell selection cursor over data and results tables.
    CellGrid,
    /// Precision crosshair for placing significance brackets, lines and annotations.
    Crosshair,
    /// Horizontal or vertical resize grip on axis ticks, axis breaks and column headers.
    AxisResize,
    /// Panning hand over a zoomed graph or multi-panel layout sheet.
    HandGrab,
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{Cell, CellCoord, DataSheet, SheetId, SubcolumnMode, TableArchetype, XyTable};

    use super::{CanvasToolMode, CanvasToolState, Cursor, GridCellCoord, GridToolState, SelectionQuickStats};

    #[test]
    fn default_tool_cursors_are_distinct() {
        assert_ne!(Cursor::Select, Cursor::CellGrid);
        assert_ne!(Cursor::Crosshair, Cursor::AxisResize);
    }

    #[test]
    fn grid_window_and_quick_stats_smoke() {
        let archetype = TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 2, 10).expect("xy table"));
        let mut sheet = DataSheet::new(SheetId::new("data-1"), "Measurements", archetype).expect("datasheet");
        sheet.set_cell(CellCoord::new(0, 0, 0), Cell::number(10.0)).expect("set 0,0");
        sheet.set_cell(CellCoord::new(1, 0, 0), Cell::number(20.0)).expect("set 1,0");
        sheet.set_cell(CellCoord::new(2, 0, 0), Cell::excluded_number(99.0)).expect("set 2,0");

        let stats = SelectionQuickStats::from_data_sheet(&sheet, GridCellCoord::new(0, 0, 0), GridCellCoord::new(2, 0, 0));
        assert_eq!(stats.selected_cells, 3);
        assert_eq!(stats.numeric_count, 2);
        assert_eq!(stats.excluded_count, 1);
        assert_eq!(stats.mean, Some(15.0));

        let grid = GridToolState::default();
        let win = grid.visible_window(100, 12);
        assert_eq!(win.start_row, 0);
        assert!(win.end_row > 0);

        let canvas = CanvasToolState { mode: CanvasToolMode::Bracket, ..CanvasToolState::default() };
        assert_eq!(canvas.cursor(), Cursor::Crosshair);
    }
}
