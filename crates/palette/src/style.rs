//! Data-point marker glyphs and stroke dash styles shared across `lumencraft-doc`,
//! `lumencraft-chart`, `lumencraft-svg`, `lumencraft-pdf`, and `lumencraft-render`.

use crate::space::Rgba8;

/// Geometric shape of scatter / summary data-point symbols (`docs/graphing-and-layouts.md` §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MarkerShape {
    #[default]
    Circle,
    Square,
    TriangleUp,
    TriangleDown,
    Diamond,
    Cross,
    Plus,
    Star,
    Hexagon,
    HorizontalBar,
    VerticalBar,
}

/// Fill treatment of a data-point symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum MarkerFillMode {
    #[default]
    Filled,
    Open,
    SemiFilledTop,
    SemiFilledBottom,
    SemiFilledLeft,
    SemiFilledRight,
}

/// Complete visual styling for a scatter or summary marker.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MarkerStyle {
    pub shape: MarkerShape,
    pub fill_mode: MarkerFillMode,
    pub size_pt: f64,
    pub fill_color: Rgba8,
    pub stroke_color: Rgba8,
    pub stroke_width_pt: f64,
}

impl Default for MarkerStyle {
    fn default() -> Self {
        Self {
            shape: MarkerShape::Circle,
            fill_mode: MarkerFillMode::Filled,
            size_pt: 5.0,
            fill_color: crate::categorical::OKABE_ITO_BLUE,
            stroke_color: crate::categorical::OKABE_ITO_BLUE,
            stroke_width_pt: 1.0,
        }
    }
}

/// Stroke dash cadence for connecting lines, regression curves, thresholds, and error bars
/// (including dashed/dotted error bars per `docs/graphing-and-layouts.md` §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum LineDashStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
    DashDot,
    DashDotDot,
    LongDash,
}

impl LineDashStyle {
    /// Alternating `[dash, gap, ...]` multipliers relative to stroke width (`empty` for solid).
    #[must_use]
    pub const fn dash_multipliers(self) -> &'static [f64] {
        match self {
            Self::Solid => &[],
            Self::Dashed => &[4.0, 2.5],
            Self::Dotted => &[1.0, 2.0],
            Self::DashDot => &[4.5, 2.0, 1.0, 2.0],
            Self::DashDotDot => &[4.5, 2.0, 1.0, 2.0, 1.0, 2.0],
            Self::LongDash => &[7.0, 3.0],
        }
    }
}
