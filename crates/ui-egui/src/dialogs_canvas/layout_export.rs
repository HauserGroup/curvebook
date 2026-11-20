//! Multi-panel `Page Layout Composer` and `Export Figure` dialog state machines.

use curvebook_doc::{JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, PageOrientation, PanelBadgeStyle, ProjectDocument, RectPt, SheetId};
use curvebook_render::TiffCompression;

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dialogs_r9::{parse_export_format, parse_journal_preset_str, parse_page_orientation_str, parse_panel_badge_style_str, parse_usize_min};

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
    }

    /// Switch physical journal/page size preset.
    pub fn apply_preset(&mut self, preset: JournalWidthPreset) {
        self.preset = preset;
        let derived = LayoutSpec::from_preset(preset);
        self.orientation = derived.orientation;
    }

    /// Place `graph_ids` neatly across `rows x cols` slots inside `LayoutSpec::from_preset(preset)`.
    pub fn arrange_grid(&mut self, graph_ids: &[SheetId], margin_pt: f64, gutter_pt: f64) {
        let page = LayoutSpec::from_preset(self.preset).page_size_pt();
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
        self.items.push(LayoutItem::LiveTable { id: format!("table-{next}"), source_sheet_id, tab_id, bounds_pt });
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

    /// Apply modal/inspector field patches for `Page Layout` (`WF-48`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>, doc: Option<&ProjectDocument>) -> Result<(), String> {
        let mut grid_changed = false;
        let mut explicit_graph_ids: Option<Vec<SheetId>> = None;
        for (key, val) in fields {
            match key.as_str() {
                "preset" | "layout.preset" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`preset` must be a string"))?;
                    self.apply_preset(parse_journal_preset_str(raw)?);
                }
                "orientation" | "layout.orientation" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`orientation` must be a string"))?;
                    self.orientation = parse_page_orientation_str(raw)?;
                }
                "rows" | "layout.rows" => {
                    self.rows = u8::try_from(parse_usize_min(val, key, 1)?.min(12)).unwrap_or(12);
                    grid_changed = true;
                }
                "cols" | "columns" | "layout.cols" => {
                    self.cols = u8::try_from(parse_usize_min(val, key, 1)?.min(12)).unwrap_or(12);
                    grid_changed = true;
                }
                "align_plot_origins" | "layout.align_plot_origins" => {
                    self.align_plot_origins = val.as_bool().ok_or_else(|| String::from("`align_plot_origins` must be boolean"))?;
                }
                "panel_badges" | "layout.panel_badges" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`panel_badges` must be a string"))?;
                    self.panel_badges = parse_panel_badge_style_str(raw)?;
                }
                "new_callout_markdown" | "callout" | "layout.new_callout_markdown" => {
                    if let Some(s) = val.as_str() {
                        self.new_callout_markdown = s.to_owned();
                    }
                }
                "arrange_graph_ids" | "graph_ids" | "layout.arrange_graph_ids" => {
                    let arr = val.as_array().ok_or_else(|| format!("`{key}` must be an array of sheet IDs"))?;
                    let mut ids = Vec::with_capacity(arr.len());
                    for (idx, item) in arr.iter().enumerate() {
                        let raw = item.as_str().ok_or_else(|| format!("`{key}[{idx}]` must be a string"))?;
                        ids.push(SheetId::try_new(raw).map_err(|e| e.to_string())?);
                    }
                    explicit_graph_ids = Some(ids);
                }
                other => return Err(format!("unknown Page Layout field `{other}`")),
            }
        }
        if let Some(ids) = explicit_graph_ids {
            self.arrange_grid(&ids, 18.0, 12.0);
        } else if grid_changed && let Some(d) = doc {
            let graph_ids: Vec<SheetId> = d.graph_sheets.iter().map(|g| g.id.clone()).collect();
            self.arrange_grid(&graph_ids, 18.0, 12.0);
        }
        Ok(())
    }
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

/// Publication figure export modal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportFigureState {
    pub format: ExportFigureFormat,
    pub width_px: u32,
    pub height_px: u32,
    pub dpi: u16,
}

impl Default for ExportFigureState {
    fn default() -> Self {
        Self { format: ExportFigureFormat::Png, width_px: 1800, height_px: 1200, dpi: 300 }
    }
}

impl ExportFigureState {
    /// Emit engine export intent (`graph.exportSvg`, `graph.exportPdf`, `graph.exportPng`, or `export.tiff`).
    #[must_use]
    pub fn commit_intent(&self, sheet_id: &SheetId) -> ToolCommandIntent {
        match self.format {
            ExportFigureFormat::Svg => ToolCommandIntent::new("graph.exportSvg", json!({ "sheet_id": sheet_id.as_str() })),
            ExportFigureFormat::Pdf => ToolCommandIntent::new("graph.exportPdf", json!({ "sheet_id": sheet_id.as_str() })),
            ExportFigureFormat::Png => ToolCommandIntent::new(
                "graph.exportPng",
                json!({
                    "sheet_id": sheet_id.as_str(),
                    "width_px": self.width_px.max(16),
                    "height_px": self.height_px.max(16),
                    "dpi": self.dpi.max(72),
                }),
            ),
            ExportFigureFormat::Tiff(compression) => ToolCommandIntent::new(
                "export.tiff",
                json!({
                    "sheet_id": sheet_id.as_str(),
                    "width_px": self.width_px.max(16),
                    "height_px": self.height_px.max(16),
                    "dpi": self.dpi.max(72),
                    "compression": format!("{compression:?}"),
                }),
            ),
        }
    }

    /// Apply modal field patches for `Export Figure...` (`WF-48`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "format" | "export.format" => self.format = parse_export_format(val)?,
                "width_px" | "export.width_px" => {
                    let w = parse_usize_min(val, key, 16)?;
                    self.width_px = u32::try_from(w).unwrap_or(u32::MAX);
                }
                "height_px" | "export.height_px" => {
                    let h = parse_usize_min(val, key, 16)?;
                    self.height_px = u32::try_from(h).unwrap_or(u32::MAX);
                }
                "dpi" | "export.dpi" => {
                    let d = parse_usize_min(val, key, 72)?;
                    self.dpi = u16::try_from(d.clamp(72, 2400)).unwrap_or(600);
                }
                other => return Err(format!("unknown Export Figure field `{other}`")),
            }
        }
        Ok(())
    }
}
