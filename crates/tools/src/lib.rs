//! Interactive tool state machines for LumenCraft: spreadsheet cell selection, graph object
//! manipulation, axis range/break dragging, pairwise significance bracket placement, text/arrow
//! callouts and publication layout alignment.
//!
//! Layer L4 crate: pointer and keyboard events in, engine command specs and overlays out.
//! Governing spec: `docs/development.md` (§Cursors) and `docs/ui-parity.md`.
#![forbid(unsafe_code)]

/// Logical cursor shapes requested by interactive tools. Every variant is documented in
/// `crates/tools/src/cursors.toml` (checked by `cargo xtask cursors`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    use super::Cursor;

    #[test]
    fn default_tool_cursors_are_distinct() {
        assert_ne!(Cursor::Select, Cursor::CellGrid);
        assert_ne!(Cursor::Crosshair, Cursor::AxisResize);
    }
}
