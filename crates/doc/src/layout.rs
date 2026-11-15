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

/// Standardized print / presentation figure size presets (`QOL-8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum FigureCanvasPreset {
    SingleColumn88mm,
    ColumnAndHalf130mm,
    DoubleColumn180mm,
    FullPagePortrait,
    Slide16x9,
    PosterPanel,
}

impl FigureCanvasPreset {
    /// Physical dimensions `(width, height, unit)` for this figure preset.
    #[must_use]
    pub const fn dimensions(self) -> (f64, f64, PhysicalUnit) {
        match self {
            Self::SingleColumn88mm => (88.0, 66.0, PhysicalUnit::Millimeters),
            Self::ColumnAndHalf130mm => (130.0, 97.5, PhysicalUnit::Millimeters),
            Self::DoubleColumn180mm => (180.0, 120.0, PhysicalUnit::Millimeters),
            Self::FullPagePortrait => (180.0, 240.0, PhysicalUnit::Millimeters),
            Self::Slide16x9 => (720.0, 405.0, PhysicalUnit::Points),
            Self::PosterPanel => (864.0, 648.0, PhysicalUnit::Points),
        }
    }

    /// Physical dimensions converted to typographic points (`1 pt = 1/72 in`).
    #[must_use]
    pub fn dimensions_pt(self) -> SizePt {
        let (w, h, unit) = self.dimensions();
        SizePt::new(unit.to_points(w), unit.to_points(h))
    }

    /// Physical dimensions in millimeters.
    #[must_use]
    pub fn dimensions_mm(self) -> (f64, f64) {
        let pt = self.dimensions_pt();
        (PhysicalUnit::Millimeters.from_points(pt.width), PhysicalUnit::Millimeters.from_points(pt.height))
    }

    /// Parse preset name from CLI / JSON command parameters.
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        match norm.as_str() {
            "singlecolumn88mm" | "singlecolumn88" | "88mm" => Some(Self::SingleColumn88mm),
            "columnandhalf130mm" | "columnandhalf130" | "130mm" => Some(Self::ColumnAndHalf130mm),
            "doublecolumn180mm" | "doublecolumn180" | "180mm" => Some(Self::DoubleColumn180mm),
            "fullpageportrait" | "fullpage" => Some(Self::FullPagePortrait),
            "slide16x9" | "16x9" | "slide" => Some(Self::Slide16x9),
            "posterpanel" | "poster" => Some(Self::PosterPanel),
            _ => None,
        }
    }
}

/// How strokes, annotations, and font sizes behave when applying a [`FigureCanvasPreset`] (`QOL-8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum CanvasResizeMode {
    #[default]
    KeepTypographyPt,
    ProportionalScale,
}

impl CanvasResizeMode {
    #[must_use]
    pub fn from_name(raw: &str) -> Option<Self> {
        let norm: String = raw.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        match norm.as_str() {
            "keeptypographypt" | "keeptypography" | "locktypography" | "keep" => Some(Self::KeepTypographyPt),
            "proportionalscale" | "proportional" | "scale" => Some(Self::ProportionalScale),
            _ => None,
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

/// Maximum number of items allowed in a single [`LayoutSpec`] page layout.
pub const MAX_LAYOUT_ITEMS: usize = 64;

/// Error returned when a [`LayoutSpec`] exceeds [`MAX_LAYOUT_ITEMS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutItemLimitError {
    pub count: usize,
    pub max: usize,
}

impl std::fmt::Display for LayoutItemLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "layout item count {} exceeds maximum allowed {}", self.count, self.max)
    }
}

impl std::error::Error for LayoutItemLimitError {}

/// Multi-panel publication layout specification.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
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

    /// Validate that [`Self::items`] does not exceed [`MAX_LAYOUT_ITEMS`].
    pub fn validate_item_count(&self) -> Result<(), LayoutItemLimitError> {
        if self.items.len() > MAX_LAYOUT_ITEMS {
            return Err(LayoutItemLimitError { count: self.items.len(), max: MAX_LAYOUT_ITEMS });
        }
        Ok(())
    }

    /// Return the first non-finite float field path in this layout specification, if any.
    #[must_use]
    pub fn first_non_finite_float(&self) -> Option<&'static str> {
        if !self.width.is_finite() {
            return Some("spec.width");
        }
        if !self.height.is_finite() {
            return Some("spec.height");
        }
        for item in &self.items {
            match item {
                LayoutItem::LiveGraph { bounds_pt, scale_percent, .. } => {
                    if !rect_is_finite(*bounds_pt) {
                        return Some("spec.items.live_graph.bounds_pt");
                    }
                    if !scale_percent.is_finite() {
                        return Some("spec.items.live_graph.scale_percent");
                    }
                }
                LayoutItem::LiveTable { bounds_pt, .. } => {
                    if !rect_is_finite(*bounds_pt) {
                        return Some("spec.items.live_table.bounds_pt");
                    }
                }
                LayoutItem::TextCallout { bounds_pt, font_size_pt, .. } => {
                    if !rect_is_finite(*bounds_pt) {
                        return Some("spec.items.text_callout.bounds_pt");
                    }
                    if !font_size_pt.is_finite() {
                        return Some("spec.items.text_callout.font_size_pt");
                    }
                }
            }
        }
        None
    }

    /// Validate item budget and float finiteness for this layout specification.
    pub fn validate(&self) -> Result<(), DocError> {
        self.validate_item_count()?;
        if let Some(reason) = self.first_non_finite_float() {
            return Err(DocError::InvalidCellValue { archetype: crate::TableKind::Column, row: 0, col: 0, reason });
        }
        Ok(())
    }

    /// Append a [`LayoutItem`] while enforcing [`MAX_LAYOUT_ITEMS`].
    pub fn add_item(&mut self, item: LayoutItem) -> Result<(), LayoutItemLimitError> {
        let next = self.items.len().saturating_add(1);
        if next > MAX_LAYOUT_ITEMS {
            return Err(LayoutItemLimitError { count: next, max: MAX_LAYOUT_ITEMS });
        }
        self.items.push(item);
        Ok(())
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

#[derive(serde::Deserialize)]
struct RawLayoutSpec {
    width: f64,
    height: f64,
    unit: PhysicalUnit,
    orientation: PageOrientation,
    grid_template: Option<(u8, u8)>,
    align_plot_origins: bool,
    panel_badges: PanelBadgeStyle,
    background_fill: Option<Rgba8>,
    items: Vec<LayoutItem>,
}

impl<'de> serde::Deserialize<'de> for LayoutSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawLayoutSpec::deserialize(deserializer)?;
        let spec = Self {
            width: raw.width,
            height: raw.height,
            unit: raw.unit,
            orientation: raw.orientation,
            grid_template: raw.grid_template,
            align_plot_origins: raw.align_plot_origins,
            panel_badges: raw.panel_badges,
            background_fill: raw.background_fill,
            items: raw.items,
        };
        spec.validate_item_count().map_err(serde::de::Error::custom)?;
        Ok(spec)
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

fn rect_is_finite(rect: RectPt) -> bool {
    rect.x.is_finite() && rect.y.is_finite() && rect.width.is_finite() && rect.height.is_finite()
}

impl LayoutSheet {
    pub fn new(id: impl Into<SheetId>, title: impl Into<String>, spec: LayoutSpec) -> Result<Self, DocError> {
        let id = id.into();
        if id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        let title = title.into();
        validate_title(&title)?;
        spec.validate()?;
        Ok(Self { id, title, spec, floating_notes: Vec::new(), color_tag: None })
    }

    /// Return the first non-finite float field path in this layout sheet, if any.
    #[must_use]
    pub fn first_non_finite_float(&self) -> Option<&'static str> {
        if let Some(field) = self.spec.first_non_finite_float() {
            return Some(field);
        }
        if self.floating_notes.iter().any(|note| !rect_is_finite(note.bounds_pt)) {
            return Some("floating_notes.bounds_pt");
        }
        None
    }

    /// Validate sheet identity, title, layout specification, and floating-note geometry.
    pub fn validate(&self) -> Result<(), DocError> {
        if self.id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        validate_title(&self.title)?;
        self.spec.validate()?;
        if self.floating_notes.iter().any(|note| !rect_is_finite(note.bounds_pt)) {
            return Err(DocError::InvalidCellValue { archetype: crate::TableKind::Column, row: 0, col: 0, reason: "floating_notes.bounds_pt" });
        }
        Ok(())
    }
}
