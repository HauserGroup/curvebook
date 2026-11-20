//! Multi-panel `LayoutSpec` and `LayoutSheet` publication page model: physical journal column
//! presets (`mm`, `cm`, `in`, `pt`), live placed graphs and embedded data/result tables, plot-origin
//! baseline alignment, and automatic panel lettering (`docs/implementation/graphing-and-layouts.md` §6).

use curvebook_palette::Rgba8;

use crate::error::{DocError, validate_title};
use crate::graph::EmbeddedTableStyle;
use crate::ids::{FloatingNote, HookSource, RectPt, SheetId, SizePt, extract_template_hooks};
use crate::image::{CropInsetsPt, ImageBorderSpec};

const fn default_true() -> bool {
    true
}

/// Physical measurement unit for figure and page dimensions (`docs/implementation/graphing-and-layouts.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PhysicalUnit {
    /// Millimeters (`25.4 mm = 1 in = 72 pt`).
    #[default]
    Millimeters,
    /// Centimeters (`2.54 cm = 1 in = 72 pt`).
    Centimeters,
    /// Imperial inches (`1 in = 72 pt`).
    Inches,
    /// PostScript / PDF typographic points (`1 pt = 1/72 in`).
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

/// Standard academic journal figure-width and page presets (`docs/implementation/graphing-and-layouts.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum JournalWidthPreset {
    /// Single-column journal figure (`85 mm x 85 mm`).
    SingleColumn85Mm,
    /// Single-column journal figure (`89 mm x 89 mm`).
    SingleColumn89Mm,
    /// 1.5-column journal figure (`120 mm x 90 mm`).
    OneAndHalfColumn120Mm,
    /// 1.5-column journal figure (`136 mm x 102 mm`).
    OneAndHalfColumn136Mm,
    /// Double-column journal figure (`174 mm x 120 mm`; default for [`LayoutSpec`]).
    DoubleColumn174Mm,
    /// Double-column journal figure (`183 mm x 125 mm`).
    DoubleColumn183Mm,
    /// US Letter portrait page (`8.5 in x 11.0 in`).
    LetterPortrait,
    /// US Letter landscape page (`11.0 in x 8.5 in`).
    LetterLandscape,
    /// ISO 216 A4 portrait page (`210 mm x 297 mm`).
    A4Portrait,
    /// ISO 216 A4 landscape page (`297 mm x 210 mm`).
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
    /// Standard single-column print figure (`88 mm x 66 mm`).
    SingleColumn88mm,
    /// Standard 1.5-column print figure (`130 mm x 97.5 mm`).
    ColumnAndHalf130mm,
    /// Standard double-column print figure (`180 mm x 120 mm`).
    DoubleColumn180mm,
    /// Full-page portrait journal figure (`180 mm x 240 mm`).
    FullPagePortrait,
    /// Widescreen 16:9 presentation slide canvas (`720 pt x 405 pt`).
    Slide16x9,
    /// Large conference poster panel canvas (`864 pt x 648 pt`).
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
    /// Preserve exact font sizes, line widths, and marker sizes in typographic points so text stays legible at print size.
    #[default]
    KeepTypographyPt,
    /// Scale font sizes, strokes, and annotations proportionally with the canvas size change.
    ProportionalScale,
}

impl CanvasResizeMode {
    /// Parse a case- and punctuation-insensitive [`CanvasResizeMode`] name from CLI/RPC inputs.
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

/// Page aspect orientation of a [`LayoutSpec`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PageOrientation {
    /// Vertical page (`height > width`).
    Portrait,
    /// Horizontal page (`width >= height`).
    #[default]
    Landscape,
}

/// Automatic panel lettering badge style (`A, B, C` vs `a, b, c`; `docs/implementation/graphing-and-layouts.md` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PanelBadgeStyle {
    /// Uppercase alphabetical panel badges (`"A"`, `"B"`, `"C"`, ...).
    #[default]
    UpperCaseLetters,
    /// Lowercase alphabetical panel badges (`"a"`, `"b"`, `"c"`, ...).
    LowerCaseLetters,
    /// Suppress automatic panel lettering badges.
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
    /// Live vector projection of an upstream [`crate::GraphSheet`].
    LiveGraph {
        /// Stable item identifier within `LayoutSpec::items`.
        id: String,
        /// Upstream [`crate::GraphSheet`] rendered inside `bounds_pt`.
        graph_sheet_id: SheetId,
        /// Bounding box on the layout page in typographic points (`width > 0`, `height > 0`).
        bounds_pt: RectPt,
        /// Magnification percentage (`> 0.0`, `100.0` = unscaled 1:1 typography/stroke sizing).
        scale_percent: f64,
        /// Custom panel badge text overriding [`LayoutSpec::panel_badges`]; `None` uses automatic sequence lettering.
        badge_override: Option<String>,
    },
    /// Live tabular projection of an upstream [`crate::DataSheet`] or [`crate::ResultSheet`] sub-tab.
    LiveTable {
        /// Stable item identifier within `LayoutSpec::items`.
        id: String,
        /// Upstream [`crate::DataSheet`] or [`crate::ResultSheet`] identifier.
        source_sheet_id: SheetId,
        /// Specific [`crate::ResultTab::id`] when `source_sheet_id` names a multi-tab result sheet; `None` selects the primary tab or full data table.
        tab_id: Option<String>,
        /// Bounding box on the layout page in typographic points (`width > 0`, `height > 0`).
        bounds_pt: RectPt,
        /// Optional zero-based row subset to display; `None` includes all rows.
        #[serde(default)]
        selected_rows: Option<Vec<usize>>,
        /// Optional zero-based column subset to display; `None` includes all columns.
        #[serde(default)]
        selected_cols: Option<Vec<usize>>,
        /// Table formatting, header visibility, transposition, and border styling.
        #[serde(default)]
        style: EmbeddedTableStyle,
    },
    /// Free-placed Markdown caption or callout box supporting inline `{{info:...}}` and `{{result:...}}` hooks.
    TextCallout {
        /// Stable item identifier within `LayoutSpec::items`.
        id: String,
        /// Markdown text content (with optional inline constant/result hooks).
        markdown: String,
        /// Bounding box on the layout page in typographic points (`width > 0`, `height > 0`).
        bounds_pt: RectPt,
        /// Base font size in typographic points (`> 0.0`).
        font_size_pt: f64,
    },
    /// Embedded PNG/JPEG/TIFF raster image referencing [`crate::ProjectImageAsset::asset_id`].
    EmbeddedImage {
        /// Stable item identifier within `LayoutSpec::items`.
        id: String,
        /// Content-addressed [`crate::ProjectImageAsset::asset_id`] stored on [`crate::ProjectDocument::image_assets`].
        asset_id: String,
        /// Placement bounding rectangle on the layout page in typographic points (`width > 0`, `height > 0`).
        bounds_pt: RectPt,
        /// Clockwise rotation angle in degrees around the center of `bounds_pt`.
        #[serde(default)]
        rotation_deg: f64,
        /// When `true`, interactive corner handles preserve the intrinsic aspect ratio.
        #[serde(default = "default_true")]
        lock_aspect_ratio: bool,
        /// Four-edge inward crop insets measured in typographic points.
        #[serde(default)]
        crop: CropInsetsPt,
        /// Optional outline stroke framing the cropped image region.
        #[serde(default)]
        border: Option<ImageBorderSpec>,
        /// Custom panel badge text overriding [`LayoutSpec::panel_badges`]; `None` omits badge text.
        #[serde(default)]
        badge_override: Option<String>,
    },
}

impl LayoutItem {
    /// Return the stable item identifier within `LayoutSpec::items`.
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::LiveGraph { id, .. } | Self::LiveTable { id, .. } | Self::TextCallout { id, .. } | Self::EmbeddedImage { id, .. } => id.as_str(),
        }
    }
}

/// Maximum number of items allowed in a single [`LayoutSpec`] page layout.
pub const MAX_LAYOUT_ITEMS: usize = 64;

/// Error returned when a [`LayoutSpec`] exceeds [`MAX_LAYOUT_ITEMS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutItemLimitError {
    /// Attempted layout item count.
    pub count: usize,
    /// Maximum allowed layout item count ([`MAX_LAYOUT_ITEMS`]).
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
    /// Page width expressed in [`Self::unit`] (`> 0.0`, finite).
    pub width: f64,
    /// Page height expressed in [`Self::unit`] (`> 0.0`, finite).
    pub height: f64,
    /// Physical unit of [`Self::width`] and [`Self::height`].
    pub unit: PhysicalUnit,
    /// Portrait vs landscape page orientation metadata.
    pub orientation: PageOrientation,
    /// Optional `(rows, cols)` arrangement template (`rows >= 1`, `cols >= 1`); `None` for freeform placement.
    pub grid_template: Option<(u8, u8)>,
    /// Align plot-area origins across adjacent panels even when Y-tick label widths differ.
    pub align_plot_origins: bool,
    /// Automatic panel badge lettering mode (`A, B, C`, `a, b, c`, or `None`).
    pub panel_badges: PanelBadgeStyle,
    /// Solid page background color; `None` exports a transparent canvas background.
    pub background_fill: Option<Rgba8>,
    /// Ordered z-stack (back-to-front) of placed graphs, tables, and text callouts (`<= MAX_LAYOUT_ITEMS`).
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
                LayoutItem::LiveTable { bounds_pt, style, .. } => {
                    if !rect_is_finite(*bounds_pt) {
                        return Some("spec.items.live_table.bounds_pt");
                    }
                    if let Some(field) = style.first_non_finite_float("spec.items.live_table.style") {
                        return Some(field);
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
                LayoutItem::EmbeddedImage { bounds_pt, rotation_deg, crop, border, .. } => {
                    if !rect_is_finite(*bounds_pt) {
                        return Some("spec.items.embedded_image.bounds_pt");
                    }
                    if !rotation_deg.is_finite() {
                        return Some("spec.items.embedded_image.rotation_deg");
                    }
                    if let Some(field) = crop.first_non_finite_float("spec.items.embedded_image.crop") {
                        return Some(field);
                    }
                    if border.is_some_and(|b| !b.width_pt.is_finite()) {
                        return Some("spec.items.embedded_image.border.width_pt");
                    }
                }
            }
        }
        None
    }

    /// Validate item budget, float finiteness, and positive page/item geometry for this layout specification.
    pub fn validate(&self) -> Result<(), DocError> {
        self.validate_item_count()?;
        if let Some(reason) = self.first_non_finite_float() {
            return Err(DocError::InvalidCellValue { archetype: crate::TableKind::Column, row: 0, col: 0, reason });
        }
        let geom_err = |reason: &'static str| DocError::InvalidCellValue { archetype: crate::TableKind::Column, row: 0, col: 0, reason };
        if self.width <= 0.0 || self.height <= 0.0 {
            return Err(geom_err("layout width and height must be finite and > 0"));
        }
        if let Some((rows, cols)) = self.grid_template
            && (rows == 0 || cols == 0)
        {
            return Err(geom_err("layout grid_template rows and columns must be >= 1"));
        }
        for item in &self.items {
            match item {
                LayoutItem::LiveGraph { bounds_pt, scale_percent, .. } => {
                    if bounds_pt.width <= 0.0 || bounds_pt.height <= 0.0 {
                        return Err(geom_err("layout item bounds width and height must be finite and > 0"));
                    }
                    if *scale_percent <= 0.0 {
                        return Err(geom_err("layout live_graph scale_percent must be finite and > 0"));
                    }
                }
                LayoutItem::LiveTable { bounds_pt, style, .. } => {
                    if bounds_pt.width <= 0.0 || bounds_pt.height <= 0.0 {
                        return Err(geom_err("layout item bounds width and height must be finite and > 0"));
                    }
                    style.validate(crate::TableKind::Column)?;
                }
                LayoutItem::TextCallout { bounds_pt, font_size_pt, .. } => {
                    if bounds_pt.width <= 0.0 || bounds_pt.height <= 0.0 {
                        return Err(geom_err("layout item bounds width and height must be finite and > 0"));
                    }
                    if *font_size_pt <= 0.0 {
                        return Err(geom_err("layout text_callout font_size_pt must be finite and > 0"));
                    }
                }
                LayoutItem::EmbeddedImage { bounds_pt, crop, border, .. } => {
                    if bounds_pt.width <= 0.0 || bounds_pt.height <= 0.0 {
                        return Err(geom_err("layout item bounds width and height must be finite and > 0"));
                    }
                    crop.validate(crate::TableKind::Column)?;
                    if let Some(b) = border {
                        b.validate(crate::TableKind::Column)?;
                    }
                }
            }
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
    /// Project-unique sheet identifier.
    pub id: SheetId,
    /// Display title shown in the `Layouts` navigator folder.
    pub title: String,
    /// Physical page dimensions, grid alignment settings, and placed items.
    pub spec: LayoutSpec,
    /// Sticky-note callouts attached to this layout sheet.
    pub floating_notes: Vec<FloatingNote>,
    /// Optional navigator color swatch; `None` uses no color badge.
    pub color_tag: Option<Rgba8>,
}

fn rect_is_finite(rect: RectPt) -> bool {
    rect.x.is_finite() && rect.y.is_finite() && rect.width.is_finite() && rect.height.is_finite()
}

impl LayoutSheet {
    /// Construct a validated `LayoutSheet`, rejecting empty IDs, oversized titles, or invalid [`LayoutSpec`] geometry.
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
