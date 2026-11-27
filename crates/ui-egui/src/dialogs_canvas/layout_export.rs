//! Multi-panel `Page Layout Composer` and `Export Figure` dialog state machines.

use curvebook_doc::{
    EmbeddedTableStyle, JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, PageOrientation, PanelBadgeStyle, PhysicalUnit, RectPt, SheetId,
};
use curvebook_palette::Rgba8;
use curvebook_render::TiffCompression;

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Multi-panel publication figure composer (`LayoutSpec`, journal presets, panel badges, alignment).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageLayoutComposerState {
    pub layout_sheet_id: Option<SheetId>,
    pub preset: JournalWidthPreset,
    pub orientation: PageOrientation,
    pub rows: u8,
    pub cols: u8,
    pub align_plot_origins: bool,
    pub panel_badges: PanelBadgeStyle,
    pub items: Vec<LayoutItem>,
    pub new_callout_markdown: String,
    #[serde(default)]
    pub page_width: Option<f64>,
    #[serde(default)]
    pub page_height: Option<f64>,
    #[serde(default)]
    pub page_unit: Option<PhysicalUnit>,
    #[serde(default)]
    pub has_synced_background_fill: bool,
    #[serde(default)]
    pub background_fill: Option<Rgba8>,
    #[serde(default)]
    pub preset_dirty: bool,
}

impl Default for PageLayoutComposerState {
    fn default() -> Self {
        Self {
            layout_sheet_id: None,
            preset: JournalWidthPreset::DoubleColumn174Mm,
            orientation: PageOrientation::Landscape,
            rows: 1,
            cols: 2,
            align_plot_origins: true,
            panel_badges: PanelBadgeStyle::UpperCaseLetters,
            items: Vec::new(),
            new_callout_markdown: String::new(),
            page_width: None,
            page_height: None,
            page_unit: None,
            has_synced_background_fill: false,
            background_fill: Some(Rgba8::WHITE),
            preset_dirty: false,
        }
    }
}

impl PageLayoutComposerState {
    /// Load layout properties and items from `layout`.
    pub fn sync_from_layout(&mut self, layout: &LayoutSheet) {
        self.layout_sheet_id = Some(layout.id.clone());
        self.orientation = layout.spec.orientation;
        let (rows, cols) = layout.spec.grid_template.unwrap_or((1, 2));
        self.rows = rows.max(1);
        self.cols = cols.max(1);
        self.align_plot_origins = layout.spec.align_plot_origins;
        self.panel_badges = layout.spec.panel_badges;
        self.items = layout.spec.items.clone();
        self.page_width = Some(layout.spec.width);
        self.page_height = Some(layout.spec.height);
        self.page_unit = Some(layout.spec.unit);
        self.has_synced_background_fill = true;
        self.background_fill = layout.spec.background_fill;
        self.preset_dirty = false;
        if let Some(matched) = detect_matching_preset(&layout.spec) {
            self.preset = matched;
        }
    }

    /// Switch physical journal/page size preset.
    pub fn apply_preset(&mut self, preset: JournalWidthPreset) {
        self.preset = preset;
        let derived = LayoutSpec::from_preset(preset);
        self.orientation = derived.orientation;
        self.page_width = Some(derived.width);
        self.page_height = Some(derived.height);
        self.page_unit = Some(derived.unit);
        self.preset_dirty = true;
    }

    /// Place `graph_ids` neatly across `rows x cols` slots inside the current page dimensions.
    pub fn arrange_grid(&mut self, graph_ids: &[SheetId], margin_pt: f64, gutter_pt: f64) {
        let page = self.to_layout_spec().page_size_pt();
        let rows = usize::from(self.rows.max(1));
        let cols = usize::from(self.cols.max(1));
        let margin = if margin_pt.is_finite() { margin_pt.max(0.0) } else { 14.0 };
        let gutter = if gutter_pt.is_finite() { gutter_pt.max(0.0) } else { 10.0 };
        let usable_w = (page.width - 2.0 * margin - gutter * ((cols.saturating_sub(1)) as f64)).max(24.0);
        let usable_h = (page.height - 2.0 * margin - gutter * ((rows.saturating_sub(1)) as f64)).max(24.0);
        let cell_w = usable_w / (cols as f64);
        let cell_h = usable_h / (rows as f64);

        self.items.retain(|item| !matches!(item, LayoutItem::LiveGraph { .. }));
        for (idx, gid) in graph_ids.iter().take(rows * cols).enumerate() {
            let r = idx / cols;
            let c = idx % cols;
            let x = margin + (c as f64) * (cell_w + gutter);
            let y = margin + (r as f64) * (cell_h + gutter);
            let badge = self.panel_badges.badge_for(idx);
            self.items.push(LayoutItem::LiveGraph {
                id: format!("panel-{}", idx + 1),
                graph_sheet_id: gid.clone(),
                bounds_pt: RectPt::new(x, y, cell_w, cell_h),
                scale_percent: 100.0,
                badge_override: badge,
            });
        }
    }

    /// Append a live embedded table item.
    pub fn add_live_table(&mut self, source_sheet_id: SheetId, tab_id: Option<String>, bounds_pt: RectPt) {
        let next = self.items.len() + 1;
        self.items.push(LayoutItem::LiveTable {
            id: format!("table-{next}"),
            source_sheet_id,
            tab_id,
            bounds_pt,
            selected_rows: None,
            selected_cols: None,
            style: EmbeddedTableStyle::default(),
        });
    }

    /// Append a Markdown / hook-interpolated callout item.
    pub fn add_text_callout(&mut self, markdown: String, bounds_pt: RectPt, font_size_pt: f64) {
        let next = self.items.len() + 1;
        let size = if font_size_pt.is_finite() { font_size_pt.clamp(4.0, 72.0) } else { 10.0 };
        self.items.push(LayoutItem::TextCallout { id: format!("callout-{next}"), markdown, bounds_pt, font_size_pt: size });
    }

    /// Materialize a full [`LayoutSpec`].
    #[must_use]
    pub fn to_layout_spec(&self) -> LayoutSpec {
        let mut spec = LayoutSpec::from_preset(self.preset);
        if !self.preset_dirty
            && let (Some(width), Some(height), Some(unit)) = (self.page_width, self.page_height, self.page_unit)
            && width.is_finite()
            && height.is_finite()
            && width > 0.0
            && height > 0.0
        {
            spec.width = width;
            spec.height = height;
            spec.unit = unit;
        }
        if self.has_synced_background_fill {
            spec.background_fill = self.background_fill;
        }
        spec.orientation = self.orientation;
        spec.grid_template = Some((self.rows.max(1), self.cols.max(1)));
        spec.align_plot_origins = self.align_plot_origins;
        spec.panel_badges = self.panel_badges;
        spec.items = self.items.clone();
        spec
    }

    /// Emit `layout.setSpec` and `layout.alignOrigins` intents.
    #[must_use]
    pub fn commit_intents(&self) -> Vec<ToolCommandIntent> {
        let Some(id) = &self.layout_sheet_id else {
            return Vec::new();
        };
        let mut out = vec![ToolCommandIntent::new(
            "layout.setSpec",
            json!({
                "sheet_id": id.as_str(),
                "spec": self.to_layout_spec(),
            }),
        )];
        if self.align_plot_origins {
            out.push(ToolCommandIntent::new("layout.alignOrigins", json!({ "sheet_id": id.as_str() })));
        }
        out
    }
}

fn detect_matching_preset(spec: &LayoutSpec) -> Option<JournalWidthPreset> {
    const PRESETS: [JournalWidthPreset; 10] = [
        JournalWidthPreset::SingleColumn85Mm,
        JournalWidthPreset::SingleColumn89Mm,
        JournalWidthPreset::OneAndHalfColumn120Mm,
        JournalWidthPreset::OneAndHalfColumn136Mm,
        JournalWidthPreset::DoubleColumn174Mm,
        JournalWidthPreset::DoubleColumn183Mm,
        JournalWidthPreset::LetterPortrait,
        JournalWidthPreset::LetterLandscape,
        JournalWidthPreset::A4Portrait,
        JournalWidthPreset::A4Landscape,
    ];
    PRESETS.into_iter().find(|&preset| {
        let cand = LayoutSpec::from_preset(preset);
        cand.unit == spec.unit && (cand.width - spec.width).abs() < 1e-6 && (cand.height - spec.height).abs() < 1e-6
    })
}

/// Output format for [`ExportFigureState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExportFigureFormat {
    #[default]
    Svg,
    Pdf,
    Png,
    Tiff(TiffCompression),
}

fn default_export_color_space() -> String {
    String::from("sRGB")
}

fn default_export_unit() -> String {
    String::from("px")
}

/// Publication figure export modal state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportFigureState {
    pub format: ExportFigureFormat,
    pub width_px: u32,
    pub height_px: u32,
    pub dpi: u16,
    #[serde(default)]
    pub transparent_bg: bool,
    #[serde(default = "default_export_color_space")]
    pub color_space: String,
    #[serde(default = "default_export_unit")]
    pub unit: String,
}

impl Default for ExportFigureState {
    fn default() -> Self {
        Self {
            format: ExportFigureFormat::Png,
            width_px: 1800,
            height_px: 1200,
            dpi: 300,
            transparent_bg: false,
            color_space: default_export_color_space(),
            unit: default_export_unit(),
        }
    }
}

impl ExportFigureState {
    /// Emit engine export intent (`graph.exportSvg`, `graph.exportPdf`, `graph.exportPng`, or `export.tiff`).
    #[must_use]
    pub fn commit_intent(&self, sheet_id: &SheetId) -> ToolCommandIntent {
        match self.format {
            ExportFigureFormat::Svg => {
                let mut params = json!({ "sheet_id": sheet_id.as_str() });
                if self.transparent_bg
                    && let Some(obj) = params.as_object_mut()
                {
                    obj.insert(String::from("transparent_bg"), json!(true));
                }
                ToolCommandIntent::new("graph.exportSvg", params)
            }
            ExportFigureFormat::Pdf => ToolCommandIntent::new("graph.exportPdf", json!({ "sheet_id": sheet_id.as_str() })),
            ExportFigureFormat::Png => {
                let mut params = json!({
                    "sheet_id": sheet_id.as_str(),
                    "width_px": self.width_px.max(16),
                    "height_px": self.height_px.max(16),
                    "dpi": self.dpi.max(72),
                });
                if self.transparent_bg
                    && let Some(obj) = params.as_object_mut()
                {
                    obj.insert(String::from("transparent_bg"), json!(true));
                }
                ToolCommandIntent::new("graph.exportPng", params)
            }
            ExportFigureFormat::Tiff(compression) => {
                let mut params = json!({
                    "sheet_id": sheet_id.as_str(),
                    "width_px": self.width_px.max(16),
                    "height_px": self.height_px.max(16),
                    "dpi": self.dpi.max(72),
                    "compression": format!("{compression:?}"),
                });
                if self.transparent_bg
                    && let Some(obj) = params.as_object_mut()
                {
                    obj.insert(String::from("transparent_bg"), json!(true));
                }
                ToolCommandIntent::new("export.tiff", params)
            }
        }
    }
}

/// Target surface for the CSV Export Options modal (`R10.29`, `R10.35`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CsvExportTargetKind {
    ResultSheet { sheet_id: SheetId },
    DataTable { sheet_id: SheetId },
    DataTables { sheet_ids: Vec<SheetId> },
    AllDataTables,
}

const fn default_csv_delimiter() -> char {
    ','
}

const fn default_csv_decimal_separator() -> char {
    '.'
}

/// Editable options for CSV/TSV export (`R10.29`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvExportOptionsState {
    /// Selected result tab ID (`None` = All tabs tidy). Only used for [`CsvExportTargetKind::ResultSheet`].
    #[serde(default)]
    pub tab_id: Option<String>,
    /// Field delimiter (`','`, `';'`, or `'\t'`).
    #[serde(default = "default_csv_delimiter")]
    pub delimiter: char,
    /// Decimal separator (`'.'` or `','`).
    #[serde(default = "default_csv_decimal_separator")]
    pub decimal_separator: char,
}

impl Default for CsvExportOptionsState {
    fn default() -> Self {
        Self { tab_id: None, delimiter: ',', decimal_separator: '.' }
    }
}

impl CsvExportOptionsState {
    /// Reset options cleanly using the active locale's default decimal separator, pairing a comma
    /// decimal separator with a semicolon delimiter (`CsvOptions::european_csv()`).
    pub fn reset_for_locale(&mut self, decimal_separator: char) {
        self.tab_id = None;
        if decimal_separator == ',' {
            self.delimiter = ';';
            self.decimal_separator = ',';
        } else {
            self.delimiter = ',';
            self.decimal_separator = '.';
        }
    }

    /// Normalize delimiter into the supported set `[',', ';', '\t']`.
    #[must_use]
    pub const fn normalized_delimiter(&self) -> char {
        match self.delimiter {
            ';' => ';',
            '\t' => '\t',
            _ => ',',
        }
    }

    /// Normalize decimal separator into the supported set `['.', ',']`.
    #[must_use]
    pub const fn normalized_decimal_separator(&self) -> char {
        if self.decimal_separator == ',' { ',' } else { '.' }
    }
}
