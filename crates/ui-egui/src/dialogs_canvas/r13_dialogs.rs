//! Modal dialog state machines for R13 graph/layout workflows (`Create New Graph`, `Equalize Scaling`,
//! `Format Embedded Table`, and `Format Image`).

use curvebook_doc::{
    CropInsetsPt, EmbeddedTableStyle, GraphAnnotation, GraphSheet, ImageBorderSpec, LayoutItem, LayoutSheet, ProjectDocument, RectPt, SheetId,
};
use curvebook_palette::{LineDashStyle, Rgba8};
use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// State for the `Create New Graph...` modal dialog (`WF-56` / `R13.3`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateGraphDialogState {
    pub source_sheet_id: Option<SheetId>,
    pub source_candidates: Vec<(SheetId, String)>,
    pub kind_slug: String,
    pub one_per_data_set: bool,
    pub y_axis_title: String,
    pub graph_title: String,
}

impl Default for CreateGraphDialogState {
    fn default() -> Self {
        Self {
            source_sheet_id: None,
            source_candidates: Vec::new(),
            kind_slug: String::from("auto"),
            one_per_data_set: false,
            y_axis_title: String::new(),
            graph_title: String::new(),
        }
    }
}

impl CreateGraphDialogState {
    /// Populate candidate upstream tables/analyzable results and select `preferred_source` or the first valid source.
    pub fn open_for_document(&mut self, doc: &ProjectDocument, preferred_source: Option<&SheetId>) {
        self.source_candidates.clear();
        for ds in &doc.data_sheets {
            self.source_candidates.push((ds.id.clone(), ds.title.clone()));
        }
        for rs in &doc.result_sheets {
            if rs.analyzable_data.is_some() {
                self.source_candidates.push((rs.id.clone(), rs.title().to_owned()));
            }
        }
        let chosen = preferred_source
            .filter(|id| self.source_candidates.iter().any(|(cid, _)| cid == *id))
            .cloned()
            .or_else(|| doc.active_sheet_id.as_ref().filter(|id| self.source_candidates.iter().any(|(cid, _)| cid == *id)).cloned())
            .or_else(|| self.source_candidates.first().map(|(id, _)| id.clone()));
        self.source_sheet_id = chosen;
        self.kind_slug = String::from("auto");
        self.one_per_data_set = false;
        self.y_axis_title.clear();
        self.graph_title.clear();
    }

    /// Build the `graph.create` engine command intent on confirmation.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let src = self.source_sheet_id.as_ref()?;
        let mut obj = serde_json::Map::new();
        obj.insert(String::from("data_sheet_id"), json!(src.as_str()));
        if !self.kind_slug.is_empty() && self.kind_slug != "auto" {
            obj.insert(String::from("kind"), json!(self.kind_slug));
        }
        if self.one_per_data_set {
            obj.insert(String::from("one_per_data_set"), json!(true));
        }
        if !self.y_axis_title.trim().is_empty() {
            obj.insert(String::from("y_axis_title"), json!(self.y_axis_title.trim()));
        }
        if !self.graph_title.trim().is_empty() {
            obj.insert(String::from("title"), json!(self.graph_title.trim()));
        }
        Some(ToolCommandIntent::new("graph.create", Value::Object(obj)))
    }
}

/// State for the `Equalize Scaling...` modal dialog (`WF-63` / `R13.5`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqualizeScalingDialogState {
    pub layout_sheet_id: Option<SheetId>,
    pub mode: String,
    pub explicit_percent: f64,
    pub selected_panel_ids: Vec<String>,
    pub available_panels: Vec<(String, String)>,
}

impl Default for EqualizeScalingDialogState {
    fn default() -> Self {
        Self {
            layout_sheet_id: None,
            mode: String::from("match_smallest"),
            explicit_percent: 100.0,
            selected_panel_ids: Vec::new(),
            available_panels: Vec::new(),
        }
    }
}

impl EqualizeScalingDialogState {
    /// Populate available live graph panels on `layout` and reset mode defaults.
    pub fn open_for_layout(&mut self, doc: &ProjectDocument, layout: &LayoutSheet) {
        self.layout_sheet_id = Some(layout.id.clone());
        self.mode = String::from("match_smallest");
        self.explicit_percent = 100.0;
        self.selected_panel_ids.clear();
        self.available_panels.clear();
        for (lg_idx, item) in layout.spec.items.iter().filter(|it| matches!(it, LayoutItem::LiveGraph { .. })).enumerate() {
            if let LayoutItem::LiveGraph { id, graph_sheet_id, badge_override, .. } = item {
                let title = doc.graph_sheet(graph_sheet_id).map_or_else(|| graph_sheet_id.as_str().to_owned(), |g| g.title.clone());
                let effective_badge = badge_override.clone().or_else(|| layout.spec.panel_badges.badge_for(lg_idx));
                let display = match effective_badge {
                    Some(b) if !b.is_empty() => format!("{b}: {title}"),
                    _ => title,
                };
                self.available_panels.push((id.clone(), display));
            }
        }
    }

    /// Build the `layout.equalizeScaling` engine command intent on confirmation.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let lid = self.layout_sheet_id.as_ref()?;
        if self.available_panels.is_empty() {
            return None;
        }
        let mut obj = serde_json::Map::new();
        obj.insert(String::from("sheet_id"), json!(lid.as_str()));
        obj.insert(String::from("mode"), json!(self.mode));
        if self.mode == "explicit_percent" {
            obj.insert(String::from("percent"), json!(self.explicit_percent));
        }
        if !self.selected_panel_ids.is_empty() {
            obj.insert(String::from("panel_ids"), json!(self.selected_panel_ids));
        }
        Some(ToolCommandIntent::new("layout.equalizeScaling", Value::Object(obj)))
    }
}

/// Modal state for `Format Embedded Table...` (`WF-58` / `R13.4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatEmbeddedTableDialogState {
    pub sheet_id: Option<SheetId>,
    pub target_index: usize,
    pub is_layout: bool,
    pub source_label: String,
    pub bounds_pt: RectPt,
    pub style: EmbeddedTableStyle,
}

impl Default for FormatEmbeddedTableDialogState {
    fn default() -> Self {
        Self {
            sheet_id: None,
            target_index: 0,
            is_layout: false,
            source_label: String::new(),
            bounds_pt: RectPt::new(180.0, 40.0, 180.0, 90.0),
            style: EmbeddedTableStyle::default(),
        }
    }
}

impl FormatEmbeddedTableDialogState {
    /// Populate draft state from `gs.spec.annotations[ann_idx]`.
    pub fn open_for_graph(&mut self, doc: &ProjectDocument, gs: &GraphSheet, ann_idx: usize) -> bool {
        let Some(GraphAnnotation::EmbeddedResultTable { result_sheet_id, tab_id, bounds_pt, style, .. }) = gs.spec.annotations.get(ann_idx) else {
            return false;
        };
        let rs_title = doc.result_sheet(result_sheet_id).map_or_else(|| result_sheet_id.as_str().to_owned(), |r| r.title().to_owned());
        self.sheet_id = Some(gs.id.clone());
        self.target_index = ann_idx;
        self.is_layout = false;
        self.source_label = tab_id.as_ref().map_or(rs_title.clone(), |t| format!("{rs_title} · {t}"));
        self.bounds_pt = *bounds_pt;
        self.style = *style;
        true
    }

    /// Populate draft state from `ls.spec.items[item_idx]`.
    pub fn open_for_layout(&mut self, doc: &ProjectDocument, ls: &LayoutSheet, item_idx: usize) -> bool {
        let Some(LayoutItem::LiveTable { source_sheet_id, tab_id, bounds_pt, style, .. }) = ls.spec.items.get(item_idx) else {
            return false;
        };
        let src_title = doc
            .result_sheet(source_sheet_id)
            .map(|r| r.title().to_owned())
            .or_else(|| doc.data_sheet(source_sheet_id).map(|d| d.title.clone()))
            .or_else(|| doc.info_sheet(source_sheet_id).map(|i| i.title.clone()))
            .unwrap_or_else(|| source_sheet_id.as_str().to_owned());
        self.sheet_id = Some(ls.id.clone());
        self.target_index = item_idx;
        self.is_layout = true;
        self.source_label = tab_id.as_ref().map_or(src_title.clone(), |t| format!("{src_title} · {t}"));
        self.bounds_pt = *bounds_pt;
        self.style = *style;
        true
    }

    /// Build single atomic `graph.setSpec` or `layout.setSpec` intent updating the target table item.
    #[must_use]
    pub fn commit_intent(&self, doc: &ProjectDocument) -> Option<ToolCommandIntent> {
        let sid = self.sheet_id.as_ref()?;
        if self.is_layout {
            let ls = doc.layout_sheet(sid)?;
            let mut items = ls.spec.items.clone();
            let item = items.get_mut(self.target_index)?;
            let LayoutItem::LiveTable { bounds_pt, style, .. } = item else {
                return None;
            };
            *bounds_pt = self.bounds_pt;
            *style = self.style;
            Some(ToolCommandIntent::new("layout.setSpec", json!({ "sheet_id": sid.as_str(), "spec": { "items": items } })))
        } else {
            let gs = doc.graph_sheet(sid)?;
            let mut annotations = gs.spec.annotations.clone();
            let ann = annotations.get_mut(self.target_index)?;
            let GraphAnnotation::EmbeddedResultTable { bounds_pt, style, .. } = ann else {
                return None;
            };
            *bounds_pt = self.bounds_pt;
            *style = self.style;
            Some(ToolCommandIntent::new("graph.setSpec", json!({ "sheet_id": sid.as_str(), "spec": { "annotations": annotations } })))
        }
    }
}

/// Modal state for `Format Image...` (`WF-57` / `R13.7`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatImageDialogState {
    pub sheet_id: Option<SheetId>,
    pub target_index: usize,
    pub is_layout: bool,
    pub asset_id: String,
    pub pixel_size: Option<(u32, u32)>,
    pub bounds_pt: RectPt,
    pub rotation_deg: f64,
    pub lock_aspect_ratio: bool,
    pub crop: CropInsetsPt,
    pub border_enabled: bool,
    pub border_width_pt: f64,
    pub border_color: Rgba8,
    pub border_dash: LineDashStyle,
}

impl Default for FormatImageDialogState {
    fn default() -> Self {
        Self {
            sheet_id: None,
            target_index: 0,
            is_layout: false,
            asset_id: String::new(),
            pixel_size: None,
            bounds_pt: RectPt::new(24.0, 24.0, 120.0, 90.0),
            rotation_deg: 0.0,
            lock_aspect_ratio: true,
            crop: CropInsetsPt::default(),
            border_enabled: false,
            border_width_pt: 0.75,
            border_color: Rgba8::BLACK,
            border_dash: LineDashStyle::Solid,
        }
    }
}

struct FormatImageSeed<'a> {
    doc: &'a ProjectDocument,
    sheet_id: SheetId,
    target_index: usize,
    is_layout: bool,
    asset_id: &'a str,
    bounds_pt: RectPt,
    rotation_deg: f64,
    lock_aspect_ratio: bool,
    crop: CropInsetsPt,
    border: Option<ImageBorderSpec>,
}

impl FormatImageDialogState {
    fn populate_common(&mut self, seed: FormatImageSeed<'_>) {
        self.sheet_id = Some(seed.sheet_id);
        self.target_index = seed.target_index;
        self.is_layout = seed.is_layout;
        self.asset_id = seed.asset_id.to_owned();
        self.pixel_size = seed.doc.image_asset(seed.asset_id).map(|a| a.pixel_size);
        self.bounds_pt = seed.bounds_pt;
        self.rotation_deg = seed.rotation_deg;
        self.lock_aspect_ratio = seed.lock_aspect_ratio;
        self.crop = seed.crop;
        if let Some(b) = seed.border {
            self.border_enabled = true;
            self.border_width_pt = b.width_pt;
            self.border_color = b.color;
            self.border_dash = b.dash;
        } else {
            self.border_enabled = false;
            self.border_width_pt = 0.75;
            self.border_color = Rgba8::BLACK;
            self.border_dash = LineDashStyle::Solid;
        }
    }

    /// Populate draft state from `gs.spec.annotations[ann_idx]`.
    pub fn open_for_graph(&mut self, doc: &ProjectDocument, gs: &GraphSheet, ann_idx: usize) -> bool {
        let Some(GraphAnnotation::EmbeddedImage { asset_id, bounds_pt, rotation_deg, lock_aspect_ratio, crop, border }) =
            gs.spec.annotations.get(ann_idx)
        else {
            return false;
        };
        self.populate_common(FormatImageSeed {
            doc,
            sheet_id: gs.id.clone(),
            target_index: ann_idx,
            is_layout: false,
            asset_id,
            bounds_pt: *bounds_pt,
            rotation_deg: *rotation_deg,
            lock_aspect_ratio: *lock_aspect_ratio,
            crop: *crop,
            border: *border,
        });
        true
    }

    /// Populate draft state from `ls.spec.items[item_idx]`.
    pub fn open_for_layout(&mut self, doc: &ProjectDocument, ls: &LayoutSheet, item_idx: usize) -> bool {
        let Some(LayoutItem::EmbeddedImage { asset_id, bounds_pt, rotation_deg, lock_aspect_ratio, crop, border, .. }) = ls.spec.items.get(item_idx)
        else {
            return false;
        };
        self.populate_common(FormatImageSeed {
            doc,
            sheet_id: ls.id.clone(),
            target_index: item_idx,
            is_layout: true,
            asset_id,
            bounds_pt: *bounds_pt,
            rotation_deg: *rotation_deg,
            lock_aspect_ratio: *lock_aspect_ratio,
            crop: *crop,
            border: *border,
        });
        true
    }

    /// Intrinsic height/width aspect ratio falling back to current bounds aspect ratio.
    #[must_use]
    pub fn aspect_h_over_w(&self) -> f64 {
        self.pixel_size
            .filter(|&(w, h)| w > 0 && h > 0)
            .map(|(w, h)| f64::from(h) / f64::from(w))
            .unwrap_or_else(|| if self.bounds_pt.width > 0.0 { self.bounds_pt.height / self.bounds_pt.width } else { 1.0 })
    }

    /// Build `graph.updateImage` or `layout.updateImage` engine intent cleanly.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let sid = self.sheet_id.as_ref()?;
        let dash_slug = match self.border_dash {
            LineDashStyle::Solid => "solid",
            LineDashStyle::Dashed => "dashed",
            LineDashStyle::Dotted => "dotted",
            LineDashStyle::DashDot => "dash_dot",
            LineDashStyle::DashDotDot => "dash_dot_dot",
            LineDashStyle::LongDash => "long_dash",
        };
        let border_val = if self.border_enabled {
            let c = self.border_color;
            json!({
                "width_pt": self.border_width_pt,
                "color": format!("#{:02X}{:02X}{:02X}{:02X}", c.r, c.g, c.b, c.a),
                "dash": dash_slug,
            })
        } else {
            Value::Null
        };
        let idx_key = if self.is_layout { "item_index" } else { "annotation_index" };
        let cmd_id = if self.is_layout { "layout.updateImage" } else { "graph.updateImage" };
        let mut obj = serde_json::Map::new();
        obj.insert(String::from("sheet_id"), json!(sid.as_str()));
        obj.insert(String::from(idx_key), json!(self.target_index));
        obj.insert(String::from("bounds_pt"), json!([self.bounds_pt.x, self.bounds_pt.y, self.bounds_pt.width, self.bounds_pt.height]));
        obj.insert(String::from("rotation_deg"), json!(self.rotation_deg));
        obj.insert(String::from("lock_aspect_ratio"), json!(self.lock_aspect_ratio));
        obj.insert(
            String::from("crop"),
            json!({
                "left_pt": self.crop.left_pt,
                "right_pt": self.crop.right_pt,
                "top_pt": self.crop.top_pt,
                "bottom_pt": self.crop.bottom_pt,
            }),
        );
        obj.insert(String::from("border"), border_val);
        Some(ToolCommandIntent::new(cmd_id, Value::Object(obj)))
    }
}
