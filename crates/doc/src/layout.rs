//! Multi-panel `LayoutSpec` and `LayoutSheet` publication page model: physical journal column
//! presets (`mm`, `cm`, `in`, `pt`), live placed graphs and embedded data/result tables, plot-origin
//! baseline alignment, and automatic panel lettering (`docs/graphing-and-layouts.md` §6).

use lumencraft_palette::Rgba8;

use crate::error::{DocError, validate_title};
use crate::ids::{FloatingNote, HookSource, RectPt, SheetId, SizePt, extract_template_hooks};

/// Physical measurement unit for figure and page dimensions (`docs/graphing-and-layouts.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PhysicalUnit {
    #[default]
    Millimeters,
    Centimeters,
    Inches,
    Points,
}

impl PhysicalUnit {
    /// Convert `value` in this unit to typographic points (`1 in = 72 pt`, `1 in = 25.4 mm`).
    #[must_use]
    pub fn to_points(self, value: f64) -> f64 {
        if !value.is_finite() {
            return 0.0;
        }
        let pts = match self {
            Self::Points => value,
            Self::Inches => value * 72.0,
            Self::Millimeters => value * (72.0 / 25.4),
            Self::Centimeters => value * (720.0 / 25.4),
        };
        pts.max(0.0)
    }

    /// Convert typographic points into this physical unit.
    #[must_use]
    pub fn from_points(self, points: f64) -> f64 {
        if !points.is_finite() {
            return 0.0;
        }
        match self {
            Self::Points => points,
            Self::Inches => points / 72.0,
            Self::Millimeters => points * (25.4 / 72.0),
            Self::Centimeters => points * (2.54 / 72.0),
        }
    }
}

/// Standard academic journal figure-width and page presets (`docs/graphing-and-layouts.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum JournalWidthPreset {
    SingleColumn85Mm,
    SingleColumn89Mm,
    OneAndHalfColumn120Mm,
    OneAndHalfColumn136Mm,
    DoubleColumn174Mm,
    DoubleColumn183Mm,
    LetterPortrait,
    LetterLandscape,
    A4Portrait,
    A4Landscape,
}

impl JournalWidthPreset {
    /// `(width, height, unit)` tuple for this journal / page preset.
    #[must_use]
    pub const fn dimensions(self) -> (f64, f64, PhysicalUnit) {
        match self {
            Self::SingleColumn85Mm => (85.0, 85.0, PhysicalUnit::Millimeters),
            Self::SingleColumn89Mm => (89.0, 89.0, PhysicalUnit::Millimeters),
            Self::OneAndHalfColumn120Mm => (120.0, 90.0, PhysicalUnit::Millimeters),
            Self::OneAndHalfColumn136Mm => (136.0, 102.0, PhysicalUnit::Millimeters),
            Self::DoubleColumn174Mm => (174.0, 120.0, PhysicalUnit::Millimeters),
            Self::DoubleColumn183Mm => (183.0, 125.0, PhysicalUnit::Millimeters),
            Self::LetterPortrait => (8.5, 11.0, PhysicalUnit::Inches),
            Self::LetterLandscape => (11.0, 8.5, PhysicalUnit::Inches),
            Self::A4Portrait => (210.0, 297.0, PhysicalUnit::Millimeters),
            Self::A4Landscape => (297.0, 210.0, PhysicalUnit::Millimeters),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PageOrientation {
    Portrait,
    #[default]
    Landscape,
}

/// Automatic panel lettering badge style (`A, B, C` vs `a, b, c`; `docs/graphing-and-layouts.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PanelBadgeStyle {
    #[default]
    UpperCaseLetters,
    LowerCaseLetters,
    None,
}

impl PanelBadgeStyle {
    /// Format the badge string for 0-based panel index `panel_index`.
    #[must_use]
    pub fn badge_for(self, panel_index: usize) -> Option<String> {
        let base = crate::archetypes::default_column_label(panel_index);
        match self {
            Self::UpperCaseLetters => Some(base),
            Self::LowerCaseLetters => Some(base.to_ascii_lowercase()),
            Self::None => None,
        }
    }
}

/// Live element placed onto a [`LayoutSpec`] page.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum LayoutItem {
    LiveGraph { id: String, graph_sheet_id: SheetId, bounds_pt: RectPt, scale_percent: f64, badge_override: Option<String> },
    LiveTable { id: String, source_sheet_id: SheetId, tab_id: Option<String>, bounds_pt: RectPt },
    TextCallout { id: String, markdown: String, bounds_pt: RectPt, font_size_pt: f64 },
}

/// Multi-panel publication layout specification.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LayoutSpec {
    pub width: f64,
    pub height: f64,
    pub unit: PhysicalUnit,
    pub orientation: PageOrientation,
    /// Optional `(rows, cols)` arrangement template.
    pub grid_template: Option<(u8, u8)>,
    /// Align plot-area origins across adjacent panels even when Y-tick label widths differ.
    pub align_plot_origins: bool,
    pub panel_badges: PanelBadgeStyle,
    pub background_fill: Option<Rgba8>,
    pub items: Vec<LayoutItem>,
}

impl LayoutSpec {
    /// Construct a layout from a [`JournalWidthPreset`].
    #[must_use]
    pub fn from_preset(preset: JournalWidthPreset) -> Self {
        let (width, height, unit) = preset.dimensions();
        let orientation = if width >= height { PageOrientation::Landscape } else { PageOrientation::Portrait };
        Self {
            width,
            height,
            unit,
            orientation,
            grid_template: Some((1, 2)),
            align_plot_origins: true,
            panel_badges: PanelBadgeStyle::UpperCaseLetters,
            background_fill: Some(Rgba8::WHITE),
            items: Vec::new(),
        }
    }

    /// Physical page size converted to typographic points (`MediaBox` / `TrimBox` geometry).
    #[must_use]
    pub fn page_size_pt(&self) -> SizePt {
        SizePt::new(self.unit.to_points(self.width), self.unit.to_points(self.height))
    }

    /// Collect all inline template hooks (`{{info:...}}`, `{{result:...}}`) in layout callouts.
    #[must_use]
    pub fn inline_hooks(&self) -> Vec<HookSource> {
        let mut hooks = Vec::new();
        for item in &self.items {
            if let LayoutItem::TextCallout { markdown, .. } = item {
                hooks.extend(extract_template_hooks(markdown));
            }
        }
        hooks
    }
}

impl Default for LayoutSpec {
    fn default() -> Self {
        Self::from_preset(JournalWidthPreset::DoubleColumn174Mm)
    }
}

/// Navigator sheet wrapping a [`LayoutSpec`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LayoutSheet {
    pub id: SheetId,
    pub title: String,
    pub spec: LayoutSpec,
    pub floating_notes: Vec<FloatingNote>,
    pub color_tag: Option<Rgba8>,
}

impl LayoutSheet {
    pub fn new(id: impl Into<SheetId>, title: impl Into<String>, spec: LayoutSpec) -> Result<Self, DocError> {
        let id = id.into();
        if id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        let title = title.into();
        validate_title(&title)?;
        Ok(Self { id, title, spec, floating_notes: Vec::new(), color_tag: None })
    }
}
