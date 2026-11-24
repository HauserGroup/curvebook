//! Interactive tool state machines for CurveBook: spreadsheet cell selection, graph object
//! manipulation, axis range/break dragging, pairwise significance bracket placement, text/arrow
//! callouts and publication layout alignment.
//!
//! Layer L4 crate: pointer and keyboard events in, engine command specs and overlays out.
//! Governing spec: `docs/development.md` (§Cursors) and `docs/evidence/user-interface.md`.
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

pub mod canvas;
pub mod grid;

use serde::{Deserialize, Serialize};

pub use canvas::{
    CanvasToolMode, CanvasToolState, JournalWidthPreset, MAX_ZOOM, MIN_ZOOM, NavigatorDragState, PanZoomInertiaState, PhysicalUnit, RulerTick,
    ToolCommandIntent, build_ruler_ticks,
};
pub use grid::{GridCellCoord, GridToolState, GridViewportSpec, GridVisibleWindow, SelectionQuickStats};

/// Logical cursor shapes requested by interactive tools. Every variant is documented in
/// `crates/tools/src/cursors.toml` (checked by `cursor_enum_matches_cursors_toml` test).
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

impl Cursor {
    /// Every [`Cursor`] variant, in declaration order.
    pub const ALL: &[Self] = &[Self::Select, Self::CellGrid, Self::Crosshair, Self::AxisResize, Self::HandGrab];

    /// Canonical variant name matching `name` entries in `crates/tools/src/cursors.toml`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::CellGrid => "CellGrid",
            Self::Crosshair => "Crosshair",
            Self::AxisResize => "AxisResize",
            Self::HandGrab => "HandGrab",
        }
    }
}

#[cfg(test)]
mod cursor_toml_tests {
    use std::collections::BTreeSet;

    use super::Cursor;

    fn parse_toml_cursor_names(raw: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut in_cursor = false;
        for line in raw.lines() {
            let trimmed = line.split('#').next().unwrap_or("").trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('[') {
                in_cursor = trimmed == "[[cursor]]";
                continue;
            }
            if in_cursor
                && let Some((key, val)) = trimmed.split_once('=')
                && key.trim() == "name"
            {
                let clean = val.trim().trim_matches('"').trim_matches('\'');
                names.push(clean.to_owned());
            }
        }
        names
    }

    #[test]
    fn cursor_enum_matches_cursors_toml() {
        let doc = include_str!("cursors.toml");
        let toml_names = parse_toml_cursor_names(doc);
        assert_eq!(toml_names.len(), Cursor::ALL.len(), "cursors.toml count != Cursor::ALL: {toml_names:?}");

        let enum_set: BTreeSet<&str> = Cursor::ALL.iter().map(|c| c.name()).collect();
        let mut seen = BTreeSet::new();
        for name in &toml_names {
            assert!(seen.insert(name.as_str()), "duplicate [[cursor]] name `{name}` in cursors.toml");
            assert!(enum_set.contains(name.as_str()), "stale [[cursor]] entry `{name}` in cursors.toml");
        }
        for c in Cursor::ALL {
            assert!(seen.contains(c.name()), "missing [[cursor]] entry `{}` in cursors.toml", c.name());
        }
    }
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
