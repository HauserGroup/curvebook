//! HiDPI viewport coordinate transforms, interactive pointer/keyboard event dispatch,
//! inline editing for grid cells (`=` formulas and localized numbers) / `InfoSheet` / titles,
//! and in-memory WASM-clean [`VirtualWorkspaceFolder`] (`G6`, `M7`, `QOL-4`, `QOL-5`, `QOL-10`).

use std::collections::BTreeMap;

use curvebook_chart::PointPt;
use curvebook_doc::{
    CellCoord, CellValue, ConstantValue, DataSheet, GraphAnnotation, LayoutItem, LayoutSpec, ProjectDocument, RectPt, SheetCellRange,
    SheetFolderKind, SheetId, TableArchetype, XColumnMode,
};
use curvebook_format::{export_datasheet_to_csv, migrate_document_json, open_project_bytes, save_project_bytes, validate_project_document};
use curvebook_render::RenderTargetSpec;
use curvebook_tools::canvas::{CanvasHitTarget, ResizeHandleCorner, SnapGuide, SnapOrientation, SnapSource};
use curvebook_tools::{CanvasToolMode, GridCellCoord, ToolCommandIntent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::compositor::{WorkspaceFrameLayout, WorkspacePixelRect, compute_workspace_layout_map};
use crate::dialogs_canvas::{ActiveDialog, AnalysisCategory};
use crate::grid_view::{GridViewState, format_cell_display, parse_localized_number_input, resolve_flat_coord};
use crate::layout::a11y::{WidgetRole, cycle_focus_in_scope};
use crate::layout::hit_test::{find_scroll_container_at, hit_test_logical};
use crate::layout::scroll::ScrollStateStore;
use crate::layout::widgets::{DropdownOption, OpenDropdownState, TextFieldBufferMap, TextFieldEditState};
use crate::layout::{WidgetAction, ZLayer};
use crate::modal_predicates::evaluate_dialog_widget;
use crate::workspace_nav::{ActiveWorkspacePane, build_set_info_constant_command, build_set_info_notes_command, live_sheet_index_in_folder};
use curvebook_doc::GraphSheet;

const MIN_SCALE_FACTOR: f64 = 0.5;
const MAX_SCALE_FACTOR: f64 = 8.0;
const POINT_BRUSH_TOLERANCE_PT: f64 = 12.0;

/// HiDPI logical <-> physical coordinate metrics for a workspace viewport (`1.0x`, `1.5x`, `2.0x`, `3.0x`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ViewportMetrics {
    pub logical_width: f64,
    pub logical_height: f64,
    pub scale_factor: f64,
}

impl Default for ViewportMetrics {
    fn default() -> Self {
        Self { logical_width: 1024.0, logical_height: 640.0, scale_factor: 1.0 }
    }
}

impl ViewportMetrics {
    /// Construct validated [`ViewportMetrics`] from logical dimensions and HiDPI scale factor.
    #[must_use]
    pub fn new(logical_width: f64, logical_height: f64, scale_factor: f64) -> Self {
        let w = if logical_width.is_finite() { logical_width.max(1.0) } else { 1024.0 };
        let h = if logical_height.is_finite() { logical_height.max(1.0) } else { 640.0 };
        let scale = if scale_factor.is_finite() { scale_factor.clamp(MIN_SCALE_FACTOR, MAX_SCALE_FACTOR) } else { 1.0 };
        Self { logical_width: w, logical_height: h, scale_factor: scale }
    }

    /// Construct [`ViewportMetrics`] from physical framebuffer pixels and scale factor.
    #[must_use]
    pub fn from_physical(physical_width_px: u32, physical_height_px: u32, scale_factor: f64) -> Self {
        let scale = if scale_factor.is_finite() { scale_factor.clamp(MIN_SCALE_FACTOR, MAX_SCALE_FACTOR) } else { 1.0 };
        Self {
            logical_width: (f64::from(physical_width_px.max(1)) / scale).max(1.0),
            logical_height: (f64::from(physical_height_px.max(1)) / scale).max(1.0),
            scale_factor: scale,
        }
    }

    /// Sanitized scale factor clamped to `[0.5, 8.0]`.
    #[must_use]
    pub fn clamped_scale_factor(self) -> f64 {
        if self.scale_factor.is_finite() { self.scale_factor.clamp(MIN_SCALE_FACTOR, MAX_SCALE_FACTOR) } else { 1.0 }
    }

    /// Physical pixel width of the framebuffer (`round(logical_width * scale_factor)`).
    #[must_use]
    pub fn physical_width_px(self) -> u32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let w = (self.logical_width.max(1.0) * self.clamped_scale_factor()).round().clamp(1.0, 16_384.0) as u32;
        w
    }

    /// Physical pixel height of the framebuffer (`round(logical_height * scale_factor)`).
    #[must_use]
    pub fn physical_height_px(self) -> u32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let h = (self.logical_height.max(1.0) * self.clamped_scale_factor()).round().clamp(1.0, 16_384.0) as u32;
        h
    }

    /// Convert a logical coordinate `[x, y]` into physical framebuffer pixels.
    #[must_use]
    pub fn logical_to_physical_px(self, logical: [f64; 2]) -> [f64; 2] {
        let s = self.clamped_scale_factor();
        let x = if logical[0].is_finite() { logical[0] * s } else { 0.0 };
        let y = if logical[1].is_finite() { logical[1] * s } else { 0.0 };
        [x, y]
    }

    /// Convert a physical pixel coordinate `[x, y]` back into logical coordinates.
    #[must_use]
    pub fn physical_to_logical_px(self, physical: [f64; 2]) -> [f64; 2] {
        let s = self.clamped_scale_factor();
        let x = if physical[0].is_finite() { physical[0] / s } else { 0.0 };
        let y = if physical[1].is_finite() { physical[1] / s } else { 0.0 };
        [x, y]
    }

    /// Scale a logical span into physical pixels.
    #[must_use]
    pub fn logical_to_physical_scalar(self, val: f64) -> f64 {
        if val.is_finite() { val * self.clamped_scale_factor() } else { 0.0 }
    }

    /// Convert a physical pixel span into logical units.
    #[must_use]
    pub fn physical_to_logical_scalar(self, val: f64) -> f64 {
        if val.is_finite() { val / self.clamped_scale_factor() } else { 0.0 }
    }

    /// Effective physical DPI for a baseline logical DPI (default `72` or `96`), clamped to `[72, 1200]`.
    #[must_use]
    pub fn dpi_for_base(self, base_dpi: u16) -> u16 {
        let base = f64::from(base_dpi.max(72));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let dpi = (base * self.clamped_scale_factor()).round().clamp(72.0, 1200.0) as u16;
        dpi
    }

    /// Build a [`RenderTargetSpec`] matching physical pixel dimensions and HiDPI density.
    #[must_use]
    pub fn to_render_target_spec(self, base_dpi: u16) -> RenderTargetSpec {
        RenderTargetSpec { width_px: self.physical_width_px(), height_px: self.physical_height_px(), dpi: self.dpi_for_base(base_dpi) }
    }
}

/// Mouse / pointer button identifier for [`UiInputEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PointerButton {
    #[default]
    Primary,
    Secondary,
    Middle,
}

/// Active modifier key flags carried on pointer and key events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct KeyModifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Platform-agnostic interactive input events dispatched to [`UiApp`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UiInputEvent {
    PointerDown { x: f64, y: f64, button: PointerButton, modifiers: KeyModifiers },
    PointerMove { x: f64, y: f64, modifiers: KeyModifiers },
    PointerUp { x: f64, y: f64, button: PointerButton, modifiers: KeyModifiers },
    DoubleClick { x: f64, y: f64, button: PointerButton, modifiers: KeyModifiers },
    WheelZoom { x: f64, y: f64, delta_y: f64, modifiers: KeyModifiers },
    PinchZoom { x: f64, y: f64, scale_delta: f64 },
    InertialPan { delta_x: f64, delta_y: f64, dt_sec: f64, step_coast: bool },
    KeyDown { key: String, modifiers: KeyModifiers },
    TextInput { text: String },
    CommitInlineEdit,
    CancelInlineEdit,
}

/// Specific inline-editable target in the workspace (`G6`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InlineEditorTarget {
    GridCell { sheet_id: SheetId, coord: GridCellCoord },
    GridXCell { sheet_id: SheetId, row: usize, subcol: usize },
    InfoConstant { sheet_id: SheetId, name: String },
    InfoNotes { sheet_id: SheetId },
    GraphTitle { sheet_id: SheetId },
    AxisTitle { sheet_id: SheetId, axis: String },
}

/// Active inline editing buffer & target state (`G6`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InlineEditorState {
    pub active: bool,
    pub target: Option<InlineEditorTarget>,
    pub buffer: String,
    pub original_text: String,
    pub caret_byte: usize,
}

impl InlineEditorState {
    /// Start an inline edit session for `target` pre-populated with `initial_text`.
    pub fn begin(&mut self, target: InlineEditorTarget, initial_text: String) {
        self.caret_byte = initial_text.len();
        self.original_text = initial_text.clone();
        self.buffer = initial_text;
        self.target = Some(target);
        self.active = true;
    }

    /// Append UTF-8 `text` at the caret position when active.
    pub fn insert_text(&mut self, text: &str) {
        if !self.active || text.is_empty() {
            return;
        }
        let idx = self.caret_byte.min(self.buffer.len());
        let pos = if self.buffer.is_char_boundary(idx) { idx } else { self.buffer.len() };
        self.buffer.insert_str(pos, text);
        self.caret_byte = pos + text.len();
    }

    /// Delete one scalar character before the caret when active.
    pub fn backspace(&mut self) {
        if !self.active || self.caret_byte == 0 || self.buffer.is_empty() {
            return;
        }
        let end = self.caret_byte.min(self.buffer.len());
        let end = if self.buffer.is_char_boundary(end) { end } else { self.buffer.len() };
        if let Some((prev_idx, _)) = self.buffer[..end].char_indices().last() {
            self.buffer.drain(prev_idx..end);
            self.caret_byte = prev_idx;
        }
    }

    /// Discard edits and clear active state.
    pub fn cancel(&mut self) {
        self.active = false;
        self.target = None;
        self.buffer.clear();
        self.original_text.clear();
        self.caret_byte = 0;
    }

    /// Build the engine [`ToolCommandIntent`] to commit the current buffer, clearing inline state.
    pub fn take_commit_intent(&mut self, app: &UiApp) -> Option<ToolCommandIntent> {
        if !self.active {
            return None;
        }
        let target = self.target.take()?;
        let text = std::mem::take(&mut self.buffer);
        self.active = false;
        self.original_text.clear();
        self.caret_byte = 0;

        match target {
            InlineEditorTarget::GridCell { sheet_id, coord } => {
                let value = parse_inline_cell_input(app, text);
                Some(ToolCommandIntent::new(
                    "table.setCell",
                    json!({
                        "sheet_id": sheet_id.as_str(),
                        "row": coord.row,
                        "col": coord.col,
                        "subcol": coord.subcol,
                        "value": value,
                    }),
                ))
            }
            InlineEditorTarget::GridXCell { sheet_id, row, subcol } => {
                let value = parse_inline_cell_input(app, text);
                Some(ToolCommandIntent::new(
                    "table.setXCells",
                    json!({
                        "sheet_id": sheet_id.as_str(),
                        "row": row,
                        "subcol": subcol,
                        "value": value,
                    }),
                ))
            }
            InlineEditorTarget::InfoConstant { sheet_id, name } => {
                let existing_units = app.document().info_sheet(&sheet_id).and_then(|info| info.get_constant(&name)).and_then(|c| c.units.as_deref());
                let trimmed = text.trim();
                let value = if trimmed.is_empty() {
                    ConstantValue::Empty
                } else if let Some(num) = parse_localized_number_input(trimmed, &app.locale_preset) {
                    ConstantValue::Number(num)
                } else {
                    ConstantValue::Text(text)
                };
                Some(build_set_info_constant_command(&sheet_id, &name, &value, existing_units))
            }
            InlineEditorTarget::InfoNotes { sheet_id } => Some(build_set_info_notes_command(&sheet_id, &text, false)),
            InlineEditorTarget::GraphTitle { sheet_id } => {
                if app.document().layout_sheet(&sheet_id).is_some() {
                    Some(ToolCommandIntent::new("layout.setSpec", json!({ "sheet_id": sheet_id.as_str(), "title": text })))
                } else {
                    Some(ToolCommandIntent::new("graph.setSpec", json!({ "sheet_id": sheet_id.as_str(), "title": text, "spec": { "title": text } })))
                }
            }
            InlineEditorTarget::AxisTitle { sheet_id, axis } => {
                let axis_key = if axis == "y" {
                    "y_axis"
                } else if axis == "y2" {
                    "y2_axis"
                } else {
                    "x_axis"
                };
                Some(ToolCommandIntent::new("graph.setAxes", json!({ "sheet_id": sheet_id.as_str(), axis_key: { "title": text } })))
            }
        }
    }
}

fn parse_inline_cell_input(app: &UiApp, text: String) -> Value {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Value::Null
    } else if trimmed.starts_with('=') {
        Value::String(trimmed.to_owned())
    } else if let Some(num) = parse_localized_number_input(trimmed, &app.locale_preset) {
        json!(num)
    } else {
        Value::String(text)
    }
}

/// In-memory virtual workspace directory abstraction for browser/WASM & sandboxed hosts (`QOL-10`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VirtualWorkspaceFolder {
    pub root_name: String,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl VirtualWorkspaceFolder {
    /// Create an empty virtual workspace folder named `root_name`.
    #[must_use]
    pub fn new(root_name: impl Into<String>) -> Self {
        let raw = root_name.into();
        let root_name = if raw.trim().is_empty() { String::from("workspace.curvebook.d") } else { raw };
        Self { root_name, files: BTreeMap::new() }
    }

    /// Normalize and validate a clean relative virtual path (rejecting empty, root, or `..` segments).
    ///
    /// # Errors
    /// Returns an error if the relative path is empty, absolute, contains backslashes, or escapes root.
    pub fn normalize_rel_path(rel_path: &str) -> Result<String, String> {
        let trimmed = rel_path.trim();
        if trimmed.is_empty() || trimmed.starts_with('/') || trimmed.contains('\\') || trimmed.contains('\0') {
            return Err(format!("invalid virtual workspace path `{rel_path}`"));
        }
        let mut parts = Vec::new();
        for seg in trimmed.split('/') {
            if seg.is_empty() || seg == "." || seg == ".." {
                return Err(format!("unsafe virtual workspace path `{rel_path}`"));
            }
            parts.push(seg);
        }
        Ok(parts.join("/"))
    }

    /// Write raw bytes at `rel_path` inside the virtual folder.
    ///
    /// # Errors
    /// Returns an error if `rel_path` fails validation.
    pub fn write_file(&mut self, rel_path: &str, bytes: Vec<u8>) -> Result<(), String> {
        let clean = Self::normalize_rel_path(rel_path)?;
        self.files.insert(clean, bytes);
        Ok(())
    }

    /// Write UTF-8 text at `rel_path` inside the virtual folder.
    ///
    /// # Errors
    /// Returns an error if `rel_path` fails validation.
    pub fn write_text(&mut self, rel_path: &str, text: &str) -> Result<(), String> {
        self.write_file(rel_path, text.as_bytes().to_vec())
    }

    /// Read raw bytes from `rel_path`, if present.
    #[must_use]
    pub fn read_file(&self, rel_path: &str) -> Option<&[u8]> {
        let clean = Self::normalize_rel_path(rel_path).ok()?;
        self.files.get(&clean).map(Vec::as_slice)
    }

    /// Read UTF-8 text from `rel_path`, if present.
    ///
    /// # Errors
    /// Returns an error if the stored bytes are not valid UTF-8.
    pub fn read_text(&self, rel_path: &str) -> Result<Option<String>, String> {
        let Some(bytes) = self.read_file(rel_path) else {
            return Ok(None);
        };
        let text = std::str::from_utf8(bytes).map_err(|err| format!("virtual file `{rel_path}` is not valid UTF-8: {err}"))?;
        Ok(Some(text.to_owned()))
    }

    /// Remove `rel_path`, returning `true` when an entry existed.
    pub fn remove_file(&mut self, rel_path: &str) -> bool {
        Self::normalize_rel_path(rel_path).ok().and_then(|clean| self.files.remove(&clean)).is_some()
    }

    /// Check whether `rel_path` exists in the virtual workspace.
    #[must_use]
    pub fn contains(&self, rel_path: &str) -> bool {
        Self::normalize_rel_path(rel_path).ok().is_some_and(|clean| self.files.contains_key(&clean))
    }

    /// Sorted list of relative file paths in the virtual workspace.
    #[must_use]
    pub fn list_files(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    /// Total stored byte footprint across all virtual files.
    #[must_use]
    pub fn total_bytes(&self) -> usize {
        self.files.values().map(Vec::len).sum()
    }

    /// Remove all entries from the virtual workspace.
    pub fn clear(&mut self) {
        self.files.clear();
    }

    /// Serialize `doc` as a `.curvebook` archive at `file_name` inside this virtual workspace.
    ///
    /// # Errors
    /// Returns an error if archive serialization or path validation fails.
    pub fn save_project_archive(&mut self, file_name: &str, doc: &ProjectDocument) -> Result<usize, String> {
        let bytes = save_project_bytes(doc).map_err(|err| err.to_string())?;
        let len = bytes.len();
        self.write_file(file_name, bytes)?;
        Ok(len)
    }

    /// Deserialize a `.curvebook` archive stored at `file_name` inside this virtual workspace.
    ///
    /// # Errors
    /// Returns an error if `file_name` is missing or archive decoding fails.
    pub fn open_project_archive(&self, file_name: &str) -> Result<ProjectDocument, String> {
        let bytes = self.read_file(file_name).ok_or_else(|| format!("virtual file `{file_name}` not found"))?;
        open_project_bytes(bytes).map_err(|err| err.to_string())
    }

    /// Write an unpacked `.curvebook.d`-compatible virtual snapshot (`document.json`, `manifest.json`,
    /// and `sheets/<id>.csv`) directly into memory with zero filesystem access (`QOL-10`).
    ///
    /// # Errors
    /// Returns an error if serialization fails.
    pub fn save_unpacked_snapshot(&mut self, doc: &ProjectDocument) -> Result<usize, String> {
        let doc_bytes = serde_json::to_vec_pretty(doc).map_err(|err| err.to_string())?;
        let mut sheet_entries = Vec::new();
        self.files.retain(|k, _| !k.starts_with("sheets/"));
        for ds in &doc.data_sheets {
            let safe_id: String = ds.id.as_str().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
            let csv_rel = format!("sheets/{safe_id}.csv");
            let csv_text = export_datasheet_to_csv(ds, &curvebook_format::CsvOptions::csv()).map_err(|err| err.to_string())?;
            self.write_file(&csv_rel, csv_text.into_bytes())?;
            sheet_entries.push(json!({ "id": ds.id.as_str(), "title": ds.title, "csv_path": csv_rel }));
        }
        let manifest_bytes = serde_json::to_vec_pretty(&json!({
            "root_name": self.root_name,
            "title": doc.title,
            "schema_version": doc.schema_version,
            "sheets": sheet_entries,
        }))
        .map_err(|err| err.to_string())?;
        self.write_file("document.json", doc_bytes)?;
        self.write_file("manifest.json", manifest_bytes)?;
        Ok(self.total_bytes())
    }

    /// Load a [`ProjectDocument`] from `document.json` in this virtual workspace (`QOL-10`).
    ///
    /// # Errors
    /// Returns an error if `document.json` is missing or malformed.
    pub fn open_unpacked_snapshot(&self) -> Result<ProjectDocument, String> {
        let bytes = self.read_file("document.json").ok_or_else(|| String::from("virtual snapshot missing `document.json`"))?;
        let raw: Value = serde_json::from_slice(bytes).map_err(|err| format!("failed decoding virtual `document.json`: {err}"))?;
        let from_version = match raw.get("schema_version") {
            None | Some(Value::Null) => 0_u32,
            Some(v) => {
                v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| String::from("virtual `document.json` has invalid `schema_version`"))?
            }
        };
        let migrated = migrate_document_json(raw, from_version).map_err(|err| format!("failed migrating virtual `document.json`: {err}"))?;
        let doc: ProjectDocument = serde_json::from_value(migrated).map_err(|err| format!("failed decoding virtual `document.json`: {err}"))?;
        validate_project_document(&doc).map_err(|err| format!("invalid virtual `document.json`: {err}"))?;
        Ok(doc)
    }
}

/// Physical pixel hit region for a row in the Project Navigator list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigatorEntryHitRect {
    pub sheet_id: SheetId,
    pub folder: SheetFolderKind,
    pub index_in_folder: usize,
    pub rect: WorkspacePixelRect,
}

/// Physical pixel hit region for a row in an `InfoSheet` constants table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfoRowHitRect {
    pub sheet_id: SheetId,
    pub name: String,
    pub value_text: String,
    pub rect: WorkspacePixelRect,
}

/// Deterministic spreadsheet cell geometry mapping pixels <-> [`GridCellCoord`] in LTR and RTL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridHitGeometry {
    pub sheet_id: SheetId,
    pub sheet_rect: WorkspacePixelRect,
    pub hdr_y: u32,
    pub body_y: u32,
    pub row_h: u32,
    pub gutter_w: u32,
    pub col_w: u32,
    /// Number of visible X / Time subcolumns before the Y columns (e.g. `1` or `3` on XY, `1` on Survival).
    #[serde(default)]
    pub x_subcols: usize,
    /// Pixel width of each X / Time subcolumn (`0` when [`Self::x_subcols`] == 0).
    #[serde(default)]
    pub x_col_w: u32,
    pub start_row: usize,
    pub end_row: usize,
    pub start_col: usize,
    pub visible_cols: Vec<(usize, GridCellCoord)>,
    #[serde(default)]
    pub x_col_spans: Vec<(u32, u32)>,
    #[serde(default)]
    pub col_spans: Vec<(u32, u32)>,
    pub is_rtl: bool,
}

impl GridHitGeometry {
    /// Local pixel offset where Y columns begin (immediately after `#` gutter + visible X subcolumns).
    #[must_use]
    pub const fn data_x_origin(&self) -> u32 {
        #[allow(clippy::cast_possible_truncation)]
        let x_cnt = if self.x_subcols > 16 { 16 } else { self.x_subcols as u32 };
        self.gutter_w.saturating_add(x_cnt.saturating_mul(self.x_col_w))
    }

    /// Pixel rectangle for the leading `#` gutter of visible row offset `r_off`.
    #[must_use]
    pub const fn row_gutter_rect(&self, r_off: u32) -> WorkspacePixelRect {
        let local_x = crate::i18n::mirror_x_for_rtl(0, self.gutter_w, self.sheet_rect.width, self.is_rtl);
        WorkspacePixelRect::new(
            self.sheet_rect.x.saturating_add(local_x),
            self.body_y.saturating_add(r_off.saturating_mul(self.row_h)),
            self.gutter_w,
            self.row_h,
        )
    }

    /// Pixel rectangle for X / Time subcolumn header `subcol`.
    #[must_use]
    pub fn x_header_rect(&self, subcol: usize) -> Option<WorkspacePixelRect> {
        if subcol >= self.x_subcols || self.x_col_w == 0 {
            return None;
        }
        let s_off = u32::try_from(subcol).ok()?;
        let local_raw = self.gutter_w.saturating_add(s_off.saturating_mul(self.x_col_w));
        let w = self.x_col_w.saturating_sub(2);
        let local_x = crate::i18n::mirror_x_for_rtl(local_raw, w, self.sheet_rect.width, self.is_rtl);
        Some(WorkspacePixelRect::new(self.sheet_rect.x.saturating_add(local_x), self.hdr_y, w, self.row_h))
    }

    /// Exact physical pixel rectangle for X / Time cell `(row, subcol)` when currently visible.
    #[must_use]
    pub fn x_cell_rect(&self, row: usize, subcol: usize) -> Option<WorkspacePixelRect> {
        if subcol >= self.x_subcols || self.x_col_w == 0 || row < self.start_row || row >= self.end_row {
            return None;
        }
        let r_off = u32::try_from(row - self.start_row).ok()?;
        let cy = self.body_y.saturating_add(r_off.saturating_mul(self.row_h));
        if cy >= self.sheet_rect.bottom() {
            return None;
        }
        let ch = self.row_h.saturating_sub(1);
        if let Some(&(cx, span_w)) = self.x_col_spans.get(subcol)
            && span_w > 1
            && cx < self.sheet_rect.right()
        {
            return Some(WorkspacePixelRect::new(cx, cy, span_w.saturating_sub(1), ch));
        }
        let s_off = u32::try_from(subcol).ok()?;
        let cw = self.x_col_w.saturating_sub(1);
        let local_raw = self.gutter_w.saturating_add(s_off.saturating_mul(self.x_col_w));
        let local_x = crate::i18n::mirror_x_for_rtl(local_raw, cw, self.sheet_rect.width, self.is_rtl);
        let cx = self.sheet_rect.x.saturating_add(local_x);
        if cx >= self.sheet_rect.right() {
            return None;
        }
        Some(WorkspacePixelRect::new(cx, cy, cw, ch))
    }

    /// Hit-test physical `(phys_x, phys_y)` into `(row, subcol)` for visible X / Time cells.
    #[must_use]
    pub fn hit_test_x_cell(&self, phys_x: u32, phys_y: u32) -> Option<(usize, usize)> {
        if self.x_subcols == 0
            || self.x_col_w == 0
            || phys_x < self.sheet_rect.x
            || phys_x >= self.sheet_rect.right()
            || phys_y < self.body_y
            || phys_y >= self.sheet_rect.bottom()
            || self.row_h == 0
        {
            return None;
        }
        let r_off = usize::try_from((phys_y - self.body_y) / self.row_h).ok()?;
        let row = self.start_row.checked_add(r_off)?;
        if row >= self.end_row {
            return None;
        }
        for subcol in 0..self.x_subcols {
            if let Some(rect) = self.x_cell_rect(row, subcol)
                && rect.contains(phys_x, phys_y)
            {
                return Some((row, subcol));
            }
        }
        None
    }

    /// Pixel rectangle for header column offset `c_off`.
    #[must_use]
    pub const fn column_header_rect(&self, c_off: u32) -> WorkspacePixelRect {
        let local_raw = self.data_x_origin().saturating_add(c_off.saturating_mul(self.col_w));
        let w = self.col_w.saturating_sub(2);
        let local_x = crate::i18n::mirror_x_for_rtl(local_raw, w, self.sheet_rect.width, self.is_rtl);
        WorkspacePixelRect::new(self.sheet_rect.x.saturating_add(local_x), self.hdr_y, w, self.row_h)
    }

    /// Exact physical pixel rectangle for `(row, flat_col)` when currently visible.
    #[must_use]
    pub fn cell_rect(&self, row: usize, flat_col: usize) -> Option<WorkspacePixelRect> {
        if row < self.start_row || row >= self.end_row || flat_col < self.start_col {
            return None;
        }
        let r_off = u32::try_from(row - self.start_row).ok()?;
        let cy = self.body_y.saturating_add(r_off.saturating_mul(self.row_h));
        if cy >= self.sheet_rect.bottom() {
            return None;
        }
        let ch = self.row_h.saturating_sub(1);
        if self.col_spans.len() == self.visible_cols.len()
            && let Some(c_idx) = self.visible_cols.iter().position(|(fc, _)| *fc == flat_col)
            && let Some(&(cx, span_w)) = self.col_spans.get(c_idx)
        {
            if span_w <= 1 || cx >= self.sheet_rect.right() {
                return None;
            }
            return Some(WorkspacePixelRect::new(cx, cy, span_w.saturating_sub(1), ch));
        }
        let c_off = u32::try_from(flat_col - self.start_col).ok()?;
        if usize::try_from(c_off).ok().is_none_or(|idx| idx >= self.visible_cols.len()) {
            return None;
        }
        let cw = self.col_w.saturating_sub(1);
        let local_raw = self.data_x_origin().saturating_add(c_off.saturating_mul(self.col_w));
        let local_x = crate::i18n::mirror_x_for_rtl(local_raw, cw, self.sheet_rect.width, self.is_rtl);
        let cx = self.sheet_rect.x.saturating_add(local_x);
        if cx >= self.sheet_rect.right() {
            return None;
        }
        Some(WorkspacePixelRect::new(cx, cy, cw, ch))
    }

    /// Hit-test physical `(phys_x, phys_y)` into `(GridCellCoord, flat_col)`.
    #[must_use]
    pub fn hit_test_cell(&self, phys_x: u32, phys_y: u32) -> Option<(GridCellCoord, usize)> {
        if phys_x < self.sheet_rect.x
            || phys_x >= self.sheet_rect.right()
            || phys_y < self.body_y
            || phys_y >= self.sheet_rect.bottom()
            || self.row_h == 0
            || self.col_w == 0
        {
            return None;
        }
        let r_off = usize::try_from((phys_y - self.body_y) / self.row_h).ok()?;
        let row = self.start_row.checked_add(r_off)?;
        if row >= self.end_row {
            return None;
        }

        let origin = self.data_x_origin();
        let use_spans = self.col_spans.len() == self.visible_cols.len();
        for (c_off, (flat_col, base_coord)) in self.visible_cols.iter().enumerate() {
            let (cx, cw) = if use_spans {
                let (sx, sw) = self.col_spans[c_off];
                (sx, sw.saturating_sub(1))
            } else {
                let cw = self.col_w.saturating_sub(1);
                let local_raw = origin.saturating_add((c_off as u32).saturating_mul(self.col_w));
                let local_x = crate::i18n::mirror_x_for_rtl(local_raw, cw, self.sheet_rect.width, self.is_rtl);
                (self.sheet_rect.x.saturating_add(local_x), cw)
            };
            if cw > 0 && phys_x >= cx && phys_x < cx.saturating_add(cw) {
                return Some((GridCellCoord::new(row, base_coord.col, base_coord.subcol), *flat_col));
            }
        }
        None
    }
}

/// Physical pixel <-> scene typographic point (`PointPt`) mapping for the active graph/layout paper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenePaperMapping {
    pub sheet_id: SheetId,
    pub paper: WorkspacePixelRect,
    pub scene_width_pt: f64,
    pub scene_height_pt: f64,
    pub zoom: f64,
    pub pan_pt: [f64; 2],
}

impl ScenePaperMapping {
    /// Unproject a physical pixel `(phys_x, phys_y)` inside `paper` into scene coordinates (`PointPt`).
    #[must_use]
    pub fn pixel_to_scene_pt(&self, phys_x: f64, phys_y: f64) -> Option<PointPt> {
        if !phys_x.is_finite() || !phys_y.is_finite() || self.paper.width == 0 || self.paper.height == 0 {
            return None;
        }
        let px0 = f64::from(self.paper.x);
        let py0 = f64::from(self.paper.y);
        let pw = f64::from(self.paper.width);
        let ph = f64::from(self.paper.height);
        if phys_x < px0 || phys_x > px0 + pw || phys_y < py0 || phys_y > py0 + ph {
            return None;
        }
        let u = (phys_x - px0) / pw;
        let v = (phys_y - py0) / ph;
        let zoom = if self.zoom.is_finite() && self.zoom > 1e-6 { self.zoom } else { 1.0 };
        let base_x = u * self.scene_width_pt.max(1.0);
        let base_y = v * self.scene_height_pt.max(1.0);
        let sx = (base_x - self.pan_pt[0]) / zoom;
        let sy = (base_y - self.pan_pt[1]) / zoom;
        Some(PointPt::new(sx, sy))
    }

    /// Project a scene point (`PointPt`) to physical framebuffer pixels `[x, y]`.
    #[must_use]
    pub fn scene_pt_to_pixel(&self, scene_pt: PointPt) -> Option<[f64; 2]> {
        if self.paper.width == 0 || self.paper.height == 0 || self.scene_width_pt <= 0.0 || self.scene_height_pt <= 0.0 {
            return None;
        }
        let zoom = if self.zoom.is_finite() && self.zoom > 1e-6 { self.zoom } else { 1.0 };
        let view_x = scene_pt.x * zoom + self.pan_pt[0];
        let view_y = scene_pt.y * zoom + self.pan_pt[1];
        let px = f64::from(self.paper.x) + (view_x / self.scene_width_pt) * f64::from(self.paper.width);
        let py = f64::from(self.paper.y) + (view_y / self.scene_height_pt) * f64::from(self.paper.height);
        Some([px, py])
    }
}

/// Resolved target produced by hit-testing [`WorkspaceLayoutMap`].
#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceHitTarget {
    SplitDivider,
    NavigatorEntry { sheet_id: SheetId, folder: SheetFolderKind, index_in_folder: usize },
    GridXCell { sheet_id: SheetId, row: usize, subcol: usize },
    GridCell { sheet_id: SheetId, coord: GridCellCoord, flat_col: usize },
    ResultBannerAction { result_sheet_id: SheetId },
    InfoConstant { sheet_id: SheetId, name: String, value_text: String },
    InfoNotes { sheet_id: SheetId },
    CanvasTitle { sheet_id: SheetId },
    AxisTitle { sheet_id: SheetId, axis: String },
    ScenePaper { sheet_id: SheetId, scene_pt: PointPt },
    Pane(ActiveWorkspacePane),
    None,
}

/// Complete physical-pixel layout & hit-testing map for LTR and RTL frames (`G6`, `M7`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceLayoutMap {
    pub metrics: ViewportMetrics,
    pub is_rtl: bool,
    pub frame: WorkspaceFrameLayout,
    pub navigator_entries: Vec<NavigatorEntryHitRect>,
    pub split_primary: Option<WorkspacePixelRect>,
    pub split_divider: Option<WorkspacePixelRect>,
    pub split_secondary: Option<WorkspacePixelRect>,
    pub grid_geometry: Option<GridHitGeometry>,
    pub scene_paper: Option<ScenePaperMapping>,
    pub result_banner_rect: Option<WorkspacePixelRect>,
    pub result_banner_sheet_id: Option<SheetId>,
    pub canvas_title_rect: Option<WorkspacePixelRect>,
    pub x_axis_title_rect: Option<WorkspacePixelRect>,
    pub y_axis_title_rect: Option<WorkspacePixelRect>,
    pub info_constant_rows: Vec<InfoRowHitRect>,
    pub info_notes_rect: Option<WorkspacePixelRect>,
    pub info_sheet_id: Option<SheetId>,
}

impl WorkspaceLayoutMap {
    /// Compute the deterministic layout map for `app` and `metrics`.
    #[must_use]
    pub fn compute(app: &UiApp, metrics: ViewportMetrics) -> Self {
        compute_workspace_layout_map(app, metrics)
    }

    /// Hit-test physical coordinates `(phys_x, phys_y)`.
    #[must_use]
    pub fn hit_test_physical(&self, phys_x: f64, phys_y: f64) -> WorkspaceHitTarget {
        if !phys_x.is_finite() || !phys_y.is_finite() || phys_x < 0.0 || phys_y < 0.0 {
            return WorkspaceHitTarget::None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (px, py) = (phys_x.round() as u32, phys_y.round() as u32);

        if let Some(div) = self.split_divider
            && div.contains(px, py)
        {
            return WorkspaceHitTarget::SplitDivider;
        }

        if let Some(entry) = self.navigator_entries.iter().find(|e| e.rect.contains(px, py)) {
            return WorkspaceHitTarget::NavigatorEntry {
                sheet_id: entry.sheet_id.clone(),
                folder: entry.folder,
                index_in_folder: entry.index_in_folder,
            };
        }

        if let Some(banner_rect) = self.result_banner_rect
            && banner_rect.contains(px, py)
            && let Some(rs_id) = &self.result_banner_sheet_id
        {
            return WorkspaceHitTarget::ResultBannerAction { result_sheet_id: rs_id.clone() };
        }

        if let Some(grid) = &self.grid_geometry {
            if let Some((row, subcol)) = grid.hit_test_x_cell(px, py) {
                return WorkspaceHitTarget::GridXCell { sheet_id: grid.sheet_id.clone(), row, subcol };
            }
            if let Some((coord, flat_col)) = grid.hit_test_cell(px, py) {
                return WorkspaceHitTarget::GridCell { sheet_id: grid.sheet_id.clone(), coord, flat_col };
            }
        }

        if let Some(row) = self.info_constant_rows.iter().find(|r| r.rect.contains(px, py)) {
            return WorkspaceHitTarget::InfoConstant { sheet_id: row.sheet_id.clone(), name: row.name.clone(), value_text: row.value_text.clone() };
        }
        if let Some(notes_rect) = self.info_notes_rect
            && notes_rect.contains(px, py)
            && let Some(sheet_id) = &self.info_sheet_id
        {
            return WorkspaceHitTarget::InfoNotes { sheet_id: sheet_id.clone() };
        }

        if let Some(paper) = &self.scene_paper {
            if let Some(title_rect) = self.canvas_title_rect
                && title_rect.contains(px, py)
            {
                return WorkspaceHitTarget::CanvasTitle { sheet_id: paper.sheet_id.clone() };
            }
            if let Some(x_rect) = self.x_axis_title_rect
                && x_rect.contains(px, py)
            {
                return WorkspaceHitTarget::AxisTitle { sheet_id: paper.sheet_id.clone(), axis: String::from("x") };
            }
            if let Some(y_rect) = self.y_axis_title_rect
                && y_rect.contains(px, py)
            {
                return WorkspaceHitTarget::AxisTitle { sheet_id: paper.sheet_id.clone(), axis: String::from("y") };
            }
            if let Some(scene_pt) = paper.pixel_to_scene_pt(phys_x, phys_y) {
                return WorkspaceHitTarget::ScenePaper { sheet_id: paper.sheet_id.clone(), scene_pt };
            }
        }

        if self.frame.navigator.contains(px, py) {
            let fam_y = self.frame.navigator.y + (self.frame.navigator.height * 2 / 3);
            if py >= fam_y {
                return WorkspaceHitTarget::Pane(ActiveWorkspacePane::FamilyStrip);
            }
            return WorkspaceHitTarget::Pane(ActiveWorkspacePane::Navigator);
        }
        if self.frame.inspector.contains(px, py) {
            return WorkspaceHitTarget::Pane(ActiveWorkspacePane::Inspector);
        }
        if self.frame.canvas.contains(px, py) {
            return WorkspaceHitTarget::Pane(ActiveWorkspacePane::MainCanvasOrGrid);
        }
        WorkspaceHitTarget::None
    }

    /// Hit-test logical coordinates `(log_x, log_y)` using [`Self::metrics`].
    #[must_use]
    pub fn hit_test_logical(&self, log_x: f64, log_y: f64) -> WorkspaceHitTarget {
        let [px, py] = self.metrics.logical_to_physical_px([log_x, log_y]);
        self.hit_test_physical(px, py)
    }
}

/// Active layout item resize corner or edge handle tracked during live pointer resize drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayoutResizeHandle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Left,
    Right,
    Top,
    Bottom,
}

impl From<ResizeHandleCorner> for LayoutResizeHandle {
    fn from(value: ResizeHandleCorner) -> Self {
        match value {
            ResizeHandleCorner::TopLeft => Self::TopLeft,
            ResizeHandleCorner::TopRight => Self::TopRight,
            ResizeHandleCorner::BottomLeft => Self::BottomLeft,
            ResizeHandleCorner::BottomRight => Self::BottomRight,
            ResizeHandleCorner::Left => Self::Left,
            ResizeHandleCorner::Right => Self::Right,
            ResizeHandleCorner::Top => Self::Top,
            ResizeHandleCorner::Bottom => Self::Bottom,
        }
    }
}

/// Active canvas annotation or layout panel drag state (`G6`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CanvasDragTarget {
    LayoutItem {
        sheet_id: SheetId,
        item_index: usize,
        start_scene_pt: PointPt,
        initial_bounds_pt: RectPt,
        current_bounds_pt: RectPt,
        #[serde(default)]
        resize_handle: Option<LayoutResizeHandle>,
        #[serde(default)]
        preview_rect_pt: Option<RectPt>,
    },
    GraphAnnotation {
        sheet_id: SheetId,
        annotation_index: usize,
        start_scene_pt: PointPt,
        initial_bounds_pt: RectPt,
        current_bounds_pt: RectPt,
    },
}

/// Sub-target for active spreadsheet header divider drags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridDividerDragKind {
    RowTitles,
    XCol(usize),
    DataCol(usize, usize),
}

/// Active chrome widget drag (floating note card header or grid column divider).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ActiveDragHandle {
    FloatingNote { sheet_id: String, note_id: String, start_logical: [f64; 2], orig_pos_pt: [f64; 2], moved: bool },
    GridDivider { slot_tag: String, kind: GridDividerDragKind, last_logical_x: f64, is_rtl: bool, moved: bool },
}

/// Ephemeral pointer drag, hover, scroll, and overlay tracking across frames (`G6`, `R10.16`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct InteractionControllerState {
    pub pointer_down: bool,
    pub last_logical_pos: Option<[f64; 2]>,
    pub dragging_grid_sheet: Option<SheetId>,
    pub dragging_split_divider: bool,
    pub dragging_navigator_sheet: Option<SheetId>,
    pub canvas_drag: Option<CanvasDragTarget>,
    #[serde(default)]
    pub active_drag_handle: Option<ActiveDragHandle>,
    #[serde(default)]
    pub context_menu_target: Option<String>,
    #[serde(default)]
    pub hovered_widget_id: Option<String>,
    #[serde(default)]
    pub pressed_widget_id: Option<String>,
    #[serde(default)]
    pub open_menu: Option<String>,
    #[serde(default)]
    pub open_dropdown: Option<OpenDropdownState>,
    #[serde(default)]
    pub scroll_offsets: ScrollStateStore,
    #[serde(default)]
    pub text_field_buffers: TextFieldBufferMap,
    #[serde(default)]
    pub last_clipboard_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_clipboard_bundle: Option<crate::os_integration::ClipboardBundle>,
}

/// Structured outcome emitted by [`dispatch_ui_event`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UiEventOutcome {
    pub handled: bool,
    pub action: String,
    pub command_id: Option<String>,
    pub command_result: Option<Value>,
}

/// Dispatch [`UiInputEvent`] against `app` without rebuilding the retained [`crate::layout::LayoutTree`].
pub(crate) fn dispatch_ui_event_without_layout_rebuild(
    app: &mut UiApp,
    event: UiInputEvent,
    metrics: ViewportMetrics,
) -> Result<UiEventOutcome, String> {
    let outcome = match event {
        UiInputEvent::PointerDown { x, y, button, modifiers } => handle_pointer_down(app, metrics, [x, y], button, modifiers)?,
        UiInputEvent::PointerMove { x, y, .. } => handle_pointer_move(app, metrics, [x, y]),
        UiInputEvent::PointerUp { x, y, button, .. } => handle_pointer_up(app, metrics, [x, y], button)?,
        UiInputEvent::DoubleClick { x, y, button, .. } => handle_double_click(app, metrics, [x, y], button),
        UiInputEvent::WheelZoom { x, y, delta_y, modifiers } => dispatch_mouse_wheel(app, metrics, x, y, 0.0, delta_y, modifiers)?,
        UiInputEvent::PinchZoom { x, y, scale_delta } => handle_pinch_zoom(app, metrics, [x, y], scale_delta),
        UiInputEvent::InertialPan { delta_x, delta_y, dt_sec, step_coast } => {
            handle_inertial_pan(app, metrics, [delta_x, delta_y], dt_sec, step_coast)
        }
        UiInputEvent::KeyDown { key, modifiers } => handle_key_down(app, metrics, &key, modifiers)?,
        UiInputEvent::TextInput { text } => handle_text_input(app, metrics, &text),
        UiInputEvent::CommitInlineEdit => commit_inline_edit(app)?,
        UiInputEvent::CancelInlineEdit => cancel_inline_edit(app),
    };
    app.sync_views();
    Ok(outcome)
}

/// Dispatch an interactive [`UiInputEvent`] against `app` under `metrics` (`G6`, `M7`).
///
/// # Errors
/// Returns a descriptive error string if a triggered engine or UI command fails.
pub fn dispatch_ui_event(app: &mut UiApp, event: UiInputEvent, metrics: ViewportMetrics) -> Result<UiEventOutcome, String> {
    let outcome = dispatch_ui_event_without_layout_rebuild(app, event, metrics)?;
    let _ = app.build_and_cache_layout_tree(metrics);
    Ok(outcome)
}

pub(crate) fn clear_inspector_axis_buffers(app: &mut UiApp) {
    app.interaction.text_field_buffers.remove("inspector.axes.min.input");
    app.interaction.text_field_buffers.remove("inspector.axes.max.input");
    app.interaction.text_field_buffers.remove("inspector.axes.interval.input");
    app.interaction.text_field_buffers.remove("inspector.axes.title.input");
}

/// Clear transient modal textbox buffers so stale values never bleed across separate dialog sessions.
pub fn clear_modal_text_buffers(app: &mut UiApp) {
    for id in [
        "axes.title",
        "axes.y_thresholds",
        "axes.x_thresholds",
        "edit.transform.formula",
        "edit.transform.filter_formula",
        "edit.transform.selected_data_sets",
        "edit.transform.baseline_columns",
    ] {
        app.interaction.text_field_buffers.remove(id);
    }
}

pub(crate) fn close_active_modal(app: &mut UiApp) {
    if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        crate::views::fit_params::clear_linear_range_inputs(app);
    }
    app.canvas_dialogs.close_dialog();
}

fn clear_context_menu_state(app: &mut UiApp) {
    app.interaction.context_menu_target = None;
    app.view_shards.toolbar_menus.active_context_menu_id = None;
    app.view_shards.toolbar_menus.context_menu_anchor_px = None;
    app.view_shards.toolbar_menus.context_sheet_id = None;
}

fn set_context_menu_state(app: &mut UiApp, target: String, sheet_id: Option<String>, anchor_px: [i32; 2]) {
    app.interaction.context_menu_target = Some(target.clone());
    app.view_shards.toolbar_menus.active_context_menu_id = Some(target);
    app.view_shards.toolbar_menus.context_menu_anchor_px = Some(anchor_px);
    app.view_shards.toolbar_menus.context_sheet_id = sheet_id;
    app.view_shards.toolbar_menus.overflow_open = false;
    app.interaction.open_menu = None;
    app.interaction.open_dropdown = None;
}

/// Dismiss any stale sheet-bound modal (such as `ChangeGraphType` for a different graph) and synchronize dialog/inspector state for `sheet_id`.
pub(crate) fn sync_dialogs_on_sheet_switch(app: &mut UiApp, sheet_id: &SheetId) {
    if matches!(&app.canvas_dialogs.active_dialog, ActiveDialog::ChangeGraphType { graph_sheet_id } if graph_sheet_id != sheet_id) {
        app.canvas_dialogs.close_dialog();
    }
    clear_inspector_axis_buffers(app);
    let doc = app.document().clone();
    app.canvas_dialogs.on_sheet_selected(&doc, sheet_id);
}

/// Unified mouse-wheel handler scrolling hovered [`crate::layout::scroll::ScrollMeta`] containers
/// unless zoom-modified (`ctrl || meta`) or over [`ScenePaperMapping`] (`R10.16`, `R10.2`).
///
/// # Errors
/// Returns an error string only if a triggered state refresh fails.
pub fn dispatch_mouse_wheel(
    app: &mut UiApp,
    metrics: ViewportMetrics,
    x: f64,
    y: f64,
    delta_x: f64,
    delta_y: f64,
    modifiers: KeyModifiers,
) -> Result<UiEventOutcome, String> {
    if !delta_x.is_finite() || !delta_y.is_finite() {
        return Ok(UiEventOutcome { handled: false, action: String::from("mouse_wheel_invalid"), ..Default::default() });
    }
    let tree = app.build_and_cache_layout_tree(metrics);
    let [phys_x, phys_y] = metrics.logical_to_physical_px([x, y]);
    #[allow(clippy::cast_possible_truncation)]
    let (px, py) = (phys_x.round() as i32, phys_y.round() as i32);

    if !(modifiers.ctrl || modifiers.meta)
        && let Some(scroll) = find_scroll_container_at(&tree, px, py).cloned()
        && (scroll.has_vertical_overflow() || scroll.has_horizontal_overflow())
    {
        let next = app.interaction.scroll_offsets.apply_wheel(&scroll, delta_x, delta_y);
        app.sync_views();
        let _ = app.build_and_cache_layout_tree(metrics);
        return Ok(UiEventOutcome {
            handled: true,
            action: String::from("scroll_container.wheel"),
            command_id: None,
            command_result: Some(json!({
                "container_id": scroll.container_id.as_str(),
                "scroll_offset_px": next,
                "max_scroll_px": scroll.max_scroll_px,
            })),
        });
    }

    let over_scene_paper = px >= 0 && py >= 0 && tree.scene_papers.iter().any(|p| p.paper.contains(px as u32, py as u32));
    if app.canvas_dialogs.active_dialog == ActiveDialog::None && (modifiers.ctrl || modifiers.meta || over_scene_paper) {
        let out = handle_wheel_zoom(app, metrics, [x, y], delta_y);
        let _ = app.build_and_cache_layout_tree(metrics);
        return Ok(out);
    }

    Ok(UiEventOutcome { handled: false, action: String::from("mouse_wheel_ignored"), ..Default::default() })
}

fn try_begin_drag_handle(app: &UiApp, wid: &str, logical_pos: [f64; 2], is_rtl: bool) -> Option<ActiveDragHandle> {
    if let Some(rest) = wid.strip_prefix("floating_note.")
        && let Some(core) = rest.strip_suffix(".drag_handle")
        && let Some((sheet_id, note_id)) = core.split_once('.')
    {
        let key = format!("{sheet_id}::{note_id}");
        let sid = SheetId::new(sheet_id);
        let orig_pos_pt = app
            .view_shards
            .nav_notes
            .note_positions_pt
            .get(&key)
            .copied()
            .or_else(|| {
                crate::views::nav_notes::sheet_floating_notes(app.document(), &sid)
                    .iter()
                    .find(|n| n.id.as_str() == note_id)
                    .map(|n| [n.bounds_pt.x, n.bounds_pt.y])
            })
            .unwrap_or([12.0, 12.0]);
        return Some(ActiveDragHandle::FloatingNote {
            sheet_id: sheet_id.to_owned(),
            note_id: note_id.to_owned(),
            start_logical: logical_pos,
            orig_pos_pt,
            moved: false,
        });
    }
    let (slot_tag, base) = if let Some(rest) = wid.strip_prefix("secondary.") { ("secondary", rest) } else { ("main", wid) };
    let kind = if base == "grid.row_titles.resize" {
        Some(GridDividerDragKind::RowTitles)
    } else if let Some(raw_sub) = base.strip_prefix("grid.col_resize.x.") {
        raw_sub.parse::<usize>().ok().map(GridDividerDragKind::XCol)
    } else if let Some(rest) = base.strip_prefix("grid.col_resize.")
        && let Some((c, s)) = rest.split_once('.')
        && let (Ok(col), Ok(sub)) = (c.parse::<usize>(), s.parse::<usize>())
    {
        Some(GridDividerDragKind::DataCol(col, sub))
    } else {
        None
    }?;
    Some(ActiveDragHandle::GridDivider { slot_tag: slot_tag.to_owned(), kind, last_logical_x: logical_pos[0], is_rtl, moved: false })
}

fn handle_pointer_down(
    app: &mut UiApp,
    metrics: ViewportMetrics,
    logical_pos: [f64; 2],
    button: PointerButton,
    modifiers: KeyModifiers,
) -> Result<UiEventOutcome, String> {
    let layout = compute_workspace_layout_map(app, metrics);
    if button == PointerButton::Secondary {
        return Ok(handle_secondary_pointer_down(app, metrics, &layout, logical_pos));
    }
    if button != PointerButton::Primary {
        return Ok(UiEventOutcome { handled: false, action: String::from("pointer_down_ignored"), ..Default::default() });
    }
    app.interaction.pointer_down = true;
    app.interaction.last_logical_pos = Some(logical_pos);
    app.interaction.pressed_widget_id = None;
    app.interaction.active_drag_handle = None;

    let tree = app.build_and_cache_layout_tree(metrics);
    if let Some(node) = hit_test_logical(&tree, logical_pos[0], logical_pos[1]).cloned() {
        if (app.interaction.context_menu_target.is_some() || app.view_shards.toolbar_menus.active_context_menu_id.is_some())
            && node.z_layer < ZLayer::ContextMenu
        {
            clear_context_menu_state(app);
        }
        if app.view_shards.toolbar_menus.overflow_open && node.z_layer < ZLayer::DropdownMenu && node.id.as_str() != "toolbar.overflow" {
            app.view_shards.toolbar_menus.overflow_open = false;
        }
        if node.id.as_str() == "modal.backdrop" {
            app.interaction.open_dropdown = None;
            app.interaction.open_menu = None;
            return Ok(UiEventOutcome { handled: true, action: String::from("modal.backdrop_blocked"), ..Default::default() });
        }
        if app.interaction.open_menu.is_some() && node.z_layer < ZLayer::DropdownMenu && !node.id.as_str().starts_with("menu.") {
            app.interaction.open_menu = None;
        }
        if app.interaction.open_dropdown.is_some()
            && !matches!(node.z_layer, ZLayer::DropdownMenu | ZLayer::ModalDropdown)
            && !matches!(node.action, WidgetAction::OpenDropdown(_))
        {
            app.interaction.open_dropdown = None;
        }
        if should_handle_chrome_widget_click(&node) {
            if app.inline_editor.active && !node.id.as_str().starts_with("inline_editor") {
                commit_inline_edit_if_active(app)?;
            }
            if node.id.as_str() == "formula_bar.input" {
                app.grid.formula_bar.active = true;
            }
            if node.state.enablement.enabled && (node.a11y.tab_order.is_some() || node.a11y.role == WidgetRole::TextBox) {
                app.focus_widget_id = Some(node.id.0.clone());
                if node.a11y.role == WidgetRole::TextBox {
                    ensure_text_field_buffer(app, node.id.as_str(), node.a11y.value_text.as_deref().unwrap_or(""));
                }
            }
            if node.state.enablement.enabled && node.action != WidgetAction::NoOp {
                app.interaction.pressed_widget_id = Some(node.id.0.clone());
                app.interaction.active_drag_handle = try_begin_drag_handle(app, node.id.as_str(), logical_pos, tree.is_rtl);
            }
            return Ok(UiEventOutcome {
                handled: true,
                action: if node.state.enablement.enabled {
                    format!("widget.pressed:{}", node.id.as_str())
                } else {
                    format!("widget.disabled:{}", node.id.as_str())
                },
                ..Default::default()
            });
        }
        if node.z_layer >= ZLayer::ModalBackdrop {
            return Ok(UiEventOutcome { handled: true, action: format!("modal.surface:{}", node.id.as_str()), ..Default::default() });
        }
    } else if app.interaction.context_menu_target.is_some() || app.view_shards.toolbar_menus.active_context_menu_id.is_some() {
        clear_context_menu_state(app);
    }

    let target = layout.hit_test_logical(logical_pos[0], logical_pos[1]);
    let clicking_active_inline_target = match (&app.inline_editor.target, &target) {
        (Some(InlineEditorTarget::GridCell { sheet_id: es, coord: ec }), WorkspaceHitTarget::GridCell { sheet_id, coord, .. }) => {
            es == sheet_id && *ec == *coord
        }
        (Some(InlineEditorTarget::GridXCell { sheet_id: es, row: er, subcol: esc }), WorkspaceHitTarget::GridXCell { sheet_id, row, subcol }) => {
            es == sheet_id && *er == *row && *esc == *subcol
        }
        _ => false,
    };
    if app.inline_editor.active && !clicking_active_inline_target {
        commit_inline_edit_if_active(app)?;
    }

    match target {
        WorkspaceHitTarget::SplitDivider => {
            app.interaction.dragging_split_divider = true;
            app.split.dragging_divider = true;
            Ok(UiEventOutcome { handled: true, action: String::from("split_divider.drag_start"), ..Default::default() })
        }
        WorkspaceHitTarget::NavigatorEntry { sheet_id, folder, index_in_folder } => {
            app.nav.active_pane = ActiveWorkspacePane::Navigator;
            commit_inline_edit_if_active(app)?;
            app.interaction.context_menu_target = None;
            let _ = app.execute_command("sheet.select", json!({ "sheet_id": sheet_id.as_str() }));
            app.nav.sync_from_document(app.session.document());
            sync_dialogs_on_sheet_switch(app, &sheet_id);
            let live_index = live_sheet_index_in_folder(app.session.document(), folder, sheet_id.as_str()).unwrap_or(index_in_folder);
            app.nav.begin_sheet_drag(folder, sheet_id.as_str(), live_index);
            app.interaction.dragging_navigator_sheet = Some(sheet_id.clone());
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("navigator.select_and_drag_start"),
                command_id: Some(String::from("sheet.select")),
                command_result: Some(json!({ "sheet_id": sheet_id.as_str() })),
            })
        }
        WorkspaceHitTarget::GridXCell { sheet_id, row, subcol } => {
            app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
            app.grid.select_x_cell_sync(row, subcol);
            app.grid.refresh_selection_stats(app.session.document());
            app.interaction.dragging_grid_sheet = None;
            app.focus_widget_id = Some(format!("grid.xcell.{row}.{subcol}"));
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("grid.select_x_cell"),
                command_id: None,
                command_result: Some(json!({
                    "sheet_id": sheet_id.as_str(),
                    "row": row,
                    "subcol": subcol,
                    "selected_rows": app.grid.selected_rows,
                })),
            })
        }
        WorkspaceHitTarget::GridCell { sheet_id, coord, .. } => {
            app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
            let doc_sheet = app.session.document().data_sheet(&sheet_id).cloned();
            if modifiers.ctrl || modifiers.meta {
                if let Some(ref ds) = doc_sheet {
                    app.grid.toggle_cmd_selection(ds, coord);
                } else {
                    app.grid.select_single_sync(coord);
                }
            } else if modifiers.shift {
                if let Some(ref ds) = doc_sheet {
                    app.grid.extend_shift_range(ds, coord);
                } else {
                    app.grid.tool.extend_selection(coord);
                }
            } else {
                app.grid.select_single_sync(coord);
            }
            app.grid.refresh_selection_stats(app.session.document());
            app.interaction.dragging_grid_sheet = Some(sheet_id.clone());
            app.sync_brush_from_grid_selection(&sheet_id);
            if let Some(ref ds) = doc_sheet {
                app.grid.update_hover_exclusion_tooltip(ds, Some(coord));
            }
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("grid.select_cell"),
                command_id: Some(String::from("ui.brush.point")),
                command_result: Some(json!({
                    "sheet_id": sheet_id.as_str(),
                    "row": coord.row,
                    "col": coord.col,
                    "subcol": coord.subcol,
                    "selected_rows": app.grid.selected_rows,
                    "selected_count": app.grid.selected_coords.len(),
                })),
            })
        }
        WorkspaceHitTarget::ResultBannerAction { result_sheet_id } => {
            let res =
                crate::control_io::dispatch_ui_command(app, "ui.result.inspectDiagnostics", &json!({ "result_sheet_id": result_sheet_id.as_str() }))?
                    .unwrap_or(Value::Null);
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("result.inspect_diagnostics"),
                command_id: Some(String::from("ui.result.inspectDiagnostics")),
                command_result: Some(res),
            })
        }
        WorkspaceHitTarget::ScenePaper { sheet_id, scene_pt } => handle_scene_pointer_down(app, &sheet_id, scene_pt, modifiers),
        WorkspaceHitTarget::InfoConstant { sheet_id, name, value_text } => {
            app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
            app.inline_editor.begin(InlineEditorTarget::InfoConstant { sheet_id, name }, value_text);
            Ok(UiEventOutcome { handled: true, action: String::from("info.constant_edit_begin"), ..Default::default() })
        }
        WorkspaceHitTarget::InfoNotes { sheet_id } => {
            let notes = app.document().info_sheet(&sheet_id).map(|i| i.notes_markdown.clone()).unwrap_or_default();
            app.inline_editor.begin(InlineEditorTarget::InfoNotes { sheet_id }, notes);
            Ok(UiEventOutcome { handled: true, action: String::from("info.notes_edit_begin"), ..Default::default() })
        }
        WorkspaceHitTarget::CanvasTitle { sheet_id } | WorkspaceHitTarget::AxisTitle { sheet_id, .. } => {
            app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
            Ok(UiEventOutcome { handled: true, action: format!("title.focused:{}", sheet_id.as_str()), ..Default::default() })
        }
        WorkspaceHitTarget::Pane(pane) => {
            app.nav.active_pane = pane;
            Ok(UiEventOutcome { handled: true, action: String::from("pane.focus"), ..Default::default() })
        }
        WorkspaceHitTarget::None => Ok(UiEventOutcome { handled: false, action: String::from("pointer_down_miss"), ..Default::default() }),
    }
}

fn is_grid_column_header_widget(wid: &str) -> bool {
    let base = wid.strip_prefix("secondary.").unwrap_or(wid);
    base.starts_with("grid.col_hdr.")
        || base.starts_with("grid.x_hdr.")
        || base.starts_with("grid.col_super.")
        || base.starts_with("grid.subcol_hdr.")
        || base.starts_with("grid.col_title.")
        || base.starts_with("grid.x_title.")
        || base.starts_with("grid.subcol_title.")
}

fn handle_secondary_pointer_down(app: &mut UiApp, metrics: ViewportMetrics, layout: &WorkspaceLayoutMap, logical_pos: [f64; 2]) -> UiEventOutcome {
    let [px, py] = metrics.logical_to_physical_px(logical_pos);
    #[allow(clippy::cast_possible_truncation)]
    let anchor = [px.round() as i32, py.round() as i32];

    let tree = app.build_and_cache_layout_tree(metrics);
    if let Some(node) = hit_test_logical(&tree, logical_pos[0], logical_pos[1]) {
        if node.z_layer >= ZLayer::ModalBackdrop {
            clear_context_menu_state(app);
            return UiEventOutcome { handled: false, action: String::from("secondary_click_ignored"), ..Default::default() };
        }
        let wid = node.id.as_str();
        if let Some(sid_str) = wid.strip_prefix("nav.sheet.") {
            let target = format!("navigator:{sid_str}");
            set_context_menu_state(app, target.clone(), Some(sid_str.to_owned()), anchor);
            return UiEventOutcome {
                handled: true,
                action: String::from("navigator.context_menu"),
                command_result: Some(json!({ "target": target, "sheet_id": sid_str })),
                ..Default::default()
            };
        }
        if is_grid_column_header_widget(wid) {
            set_context_menu_state(app, String::from("column_header"), None, anchor);
            return UiEventOutcome {
                handled: true,
                action: String::from("column_header.context_menu"),
                command_result: Some(json!({ "target": "column_header", "widget_id": wid })),
                ..Default::default()
            };
        }
    }

    match layout.hit_test_logical(logical_pos[0], logical_pos[1]) {
        WorkspaceHitTarget::NavigatorEntry { sheet_id, .. } => {
            let target = format!("navigator:{}", sheet_id.as_str());
            set_context_menu_state(app, target.clone(), Some(sheet_id.as_str().to_owned()), anchor);
            UiEventOutcome {
                handled: true,
                action: String::from("navigator.context_menu"),
                command_result: Some(json!({ "target": target, "sheet_id": sheet_id.as_str() })),
                ..Default::default()
            }
        }
        WorkspaceHitTarget::GridXCell { row, subcol, .. } => {
            app.grid.select_x_cell_sync(row, subcol);
            app.grid.refresh_selection_stats(app.session.document());
            set_context_menu_state(app, String::from("grid"), None, anchor);
            UiEventOutcome {
                handled: true,
                action: String::from("grid.context_menu"),
                command_result: Some(json!({ "target": "grid", "row": row, "subcol": subcol, "selected_rows": app.grid.selected_rows })),
                ..Default::default()
            }
        }
        WorkspaceHitTarget::GridCell { sheet_id, coord, .. } => {
            if let Some(ds) = app.session.document().data_sheet(&sheet_id) {
                let existing = app.grid.all_selected_coords(ds);
                if existing.is_empty() || (existing.len() == 1 && existing[0] != coord) {
                    app.grid.select_single_sync(coord);
                    app.grid.refresh_selection_stats(app.session.document());
                    app.sync_brush_from_grid_selection(&sheet_id);
                }
            }
            let is_linked = app
                .session
                .document()
                .data_sheet(&sheet_id)
                .is_some_and(|ds| crate::views::grid_core::linked_region_for_cell(ds, coord.row, coord.col).is_some());
            if is_linked {
                set_context_menu_state(app, String::from("linked_region"), Some(sheet_id.as_str().to_owned()), anchor);
                UiEventOutcome {
                    handled: true,
                    action: String::from("linked_region.context_menu"),
                    command_result: Some(json!({ "target": "linked_region", "row": coord.row, "col": coord.col })),
                    ..Default::default()
                }
            } else {
                set_context_menu_state(app, String::from("grid"), Some(sheet_id.as_str().to_owned()), anchor);
                UiEventOutcome {
                    handled: true,
                    action: String::from("grid.context_menu"),
                    command_result: Some(json!({ "target": "grid", "selected_rows": app.grid.selected_rows })),
                    ..Default::default()
                }
            }
        }
        WorkspaceHitTarget::ScenePaper { .. } => {
            set_context_menu_state(app, String::from("graph_object"), None, anchor);
            UiEventOutcome {
                handled: true,
                action: String::from("graph_object.context_menu"),
                command_result: Some(json!({ "target": "graph_object", "brushed_count": app.brushed_points.len() })),
                ..Default::default()
            }
        }
        _ => {
            clear_context_menu_state(app);
            UiEventOutcome { handled: false, action: String::from("secondary_click_ignored"), ..Default::default() }
        }
    }
}

fn handle_scene_pointer_down(app: &mut UiApp, sheet_id: &SheetId, scene_pt: PointPt, modifiers: KeyModifiers) -> Result<UiEventOutcome, String> {
    app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;

    if let Some(ls) = app.session.document().layout_sheet(sheet_id) {
        let hit = app.canvas_dialogs.canvas.update_hover_layout(&ls.spec, scene_pt);
        let target_info = match hit {
            Some(CanvasHitTarget::PanelResizeHandle { index, corner }) => Some((index, Some(LayoutResizeHandle::from(corner)))),
            Some(CanvasHitTarget::LayoutPanel { index } | CanvasHitTarget::EmbeddedTable { index }) => Some((index, None)),
            _ => None,
        };
        if app.canvas_dialogs.canvas.mode == CanvasToolMode::Select
            && let Some((index, resize_handle)) = target_info
            && let Some(item) = ls.spec.items.get(index)
        {
            let bounds = layout_item_bounds(item);
            app.interaction.canvas_drag = Some(CanvasDragTarget::LayoutItem {
                sheet_id: sheet_id.clone(),
                item_index: index,
                start_scene_pt: scene_pt,
                initial_bounds_pt: bounds,
                current_bounds_pt: bounds,
                resize_handle,
                preview_rect_pt: Some(bounds),
            });
            return Ok(UiEventOutcome { handled: true, action: String::from("layout.item_drag_start"), ..Default::default() });
        }
    }

    let mut brushed = None;
    let mut brushed_summary = None;
    if let Ok(scene) = app.session().render_sheet_scene(sheet_id)
        && let Some(hit) = scene.hit_test_point(scene_pt, POINT_BRUSH_TOLERANCE_PT)
    {
        brushed = Some(hit.source_cell.clone());
        brushed_summary = Some(format!("{} ({})", hit.series_label, hit.value_summary));
    }
    if let Some(cell) = brushed {
        if let Some(summary) = brushed_summary {
            app.view_shards.inspector.selected_point_summary = summary;
        }
        app.brushed_cell = Some(cell.clone());
        if modifiers.shift || modifiers.ctrl || modifiers.meta {
            if let Some(pos) = app.brushed_points.iter().position(|c| c == &cell) {
                if app.brushed_points.len() > 1 {
                    app.brushed_points.remove(pos);
                }
            } else {
                app.brushed_points.push(cell.clone());
            }
        } else {
            app.brushed_points = vec![cell.clone()];
        }
        return Ok(UiEventOutcome {
            handled: true,
            action: String::from("canvas.brush_point"),
            command_id: Some(String::from("ui.brush.point")),
            command_result: Some(json!({
                "sheet_id": cell.sheet_id.as_str(),
                "row": cell.row,
                "col": cell.col,
                "subcol": cell.subcol,
                "brushed_count": app.brushed_points.len(),
            })),
        });
    }

    if let Some(gs) = app.session.document().graph_sheet(sheet_id)
        && let Ok(scene) = app.session.render_sheet_scene(sheet_id)
    {
        let hit = app.canvas_dialogs.canvas.update_hover_graph(&scene, &gs.spec, scene_pt);
        if app.canvas_dialogs.canvas.mode == CanvasToolMode::Select
            && let Some(CanvasHitTarget::Annotation { index } | CanvasHitTarget::EmbeddedTable { index }) = hit
            && let Some(bounds) = gs.spec.annotations.get(index).and_then(graph_annotation_bounds)
        {
            app.interaction.canvas_drag = Some(CanvasDragTarget::GraphAnnotation {
                sheet_id: sheet_id.clone(),
                annotation_index: index,
                start_scene_pt: scene_pt,
                initial_bounds_pt: bounds,
                current_bounds_pt: bounds,
            });
            return Ok(UiEventOutcome { handled: true, action: String::from("graph.annotation_drag_start"), ..Default::default() });
        }
    }

    app.canvas_dialogs.canvas.begin_drag(scene_pt);
    Ok(UiEventOutcome { handled: true, action: String::from("canvas.drag_start"), ..Default::default() })
}

fn handle_pointer_move(app: &mut UiApp, metrics: ViewportMetrics, logical_pos: [f64; 2]) -> UiEventOutcome {
    app.interaction.last_logical_pos = Some(logical_pos);
    let layout = compute_workspace_layout_map(app, metrics);
    let [phys_x, phys_y] = metrics.logical_to_physical_px(logical_pos);

    if app.interaction.dragging_split_divider && layout.frame.canvas.width > 0 {
        let cx = f64::from(layout.frame.canvas.x);
        let cw = f64::from(layout.frame.canvas.width.max(1));
        let raw_ratio = if layout.is_rtl { (cx + cw - phys_x) / cw } else { (phys_x - cx) / cw };
        app.split.set_ratio(raw_ratio);
        return UiEventOutcome { handled: true, action: String::from("split_divider.move"), ..Default::default() };
    }

    if app.interaction.dragging_navigator_sheet.is_some() {
        if let WorkspaceHitTarget::NavigatorEntry { sheet_id: target_sheet_id, folder, index_in_folder } = layout.hit_test_physical(phys_x, phys_y) {
            let target_idx = live_sheet_index_in_folder(app.session.document(), folder, target_sheet_id.as_str()).unwrap_or(index_in_folder);
            app.nav.update_sheet_hover_in_folder(folder, target_idx);
        }
        return UiEventOutcome { handled: true, action: String::from("navigator.drag_hover"), ..Default::default() };
    }

    if let Some(drag_sheet) = app.interaction.dragging_grid_sheet.clone() {
        if let WorkspaceHitTarget::GridCell { sheet_id, coord, .. } = layout.hit_test_physical(phys_x, phys_y)
            && sheet_id == drag_sheet
        {
            if let Some(ds) = app.session.document().data_sheet(&sheet_id).cloned() {
                app.grid.extend_shift_range(&ds, coord);
            } else {
                app.grid.tool.extend_selection(coord);
            }
            app.grid.refresh_selection_stats(app.session.document());
            app.sync_brush_from_grid_selection(&sheet_id);
        }
        return UiEventOutcome { handled: true, action: String::from("grid.drag_select"), ..Default::default() };
    }

    if let Some(drag_target) = app.interaction.canvas_drag.take() {
        let next_drag = update_canvas_object_drag(app, &layout, [phys_x, phys_y], drag_target);
        app.interaction.canvas_drag = Some(next_drag);
        return UiEventOutcome { handled: true, action: String::from("canvas.object_drag_snap"), ..Default::default() };
    }

    if let Some(mut handle) = app.interaction.active_drag_handle.take() {
        let mut outcome: Option<UiEventOutcome> = None;
        match &mut handle {
            ActiveDragHandle::FloatingNote { sheet_id, note_id, start_logical, orig_pos_pt, moved } => {
                let dx = logical_pos[0] - start_logical[0];
                let dy = logical_pos[1] - start_logical[1];
                if *moved || dx.hypot(dy) > 1.0 {
                    *moved = true;
                    app.interaction.pressed_widget_id = None;
                    let pt_per_logical = 72.0 / 96.0;
                    let nx = (orig_pos_pt[0] + dx * pt_per_logical).clamp(0.0, 2400.0);
                    let ny = (orig_pos_pt[1] + dy * pt_per_logical).clamp(0.0, 2400.0);
                    let payload = format!("note.set_pos|{sheet_id}|{note_id}|{nx:.2}|{ny:.2}");
                    let mut nav_notes = std::mem::take(&mut app.view_shards.nav_notes);
                    if let Ok(out) = crate::views::nav_notes::handle_custom_action(app, &mut nav_notes, &payload) {
                        outcome = Some(out);
                    }
                    app.view_shards.nav_notes = nav_notes;
                }
            }
            ActiveDragHandle::GridDivider { slot_tag, kind, last_logical_x, is_rtl, moved } => {
                let raw_dx = logical_pos[0] - *last_logical_x;
                if raw_dx.abs() >= 1.0 {
                    *moved = true;
                    #[allow(clippy::cast_possible_truncation)]
                    let step_i = raw_dx.trunc() as i32;
                    *last_logical_x += f64::from(step_i);
                    app.interaction.pressed_widget_id = None;
                    let dx = if *is_rtl { -step_i } else { step_i };
                    let core = match kind {
                        GridDividerDragKind::RowTitles => format!("row_titles:resize_delta:{dx}"),
                        GridDividerDragKind::XCol(sub) => format!("xcol:resize_delta:{sub}:{dx}"),
                        GridDividerDragKind::DataCol(col, sub) => format!("col:resize_delta:{col}:{sub}:{dx}"),
                    };
                    let payload = if slot_tag == "secondary" { format!("secondary.{core}") } else { core };
                    let mut grid_core = std::mem::take(&mut app.view_shards.grid_core);
                    if let Ok(out) = crate::views::grid_core::handle_custom_action(app, &mut grid_core, &payload) {
                        outcome = Some(out);
                    }
                    app.view_shards.grid_core = grid_core;
                }
            }
        }
        app.interaction.active_drag_handle = Some(handle);
        if let Some(out) = outcome {
            return out;
        }
    }

    let tree = app.build_and_cache_layout_tree(metrics);
    if let Some(node) = hit_test_logical(&tree, logical_pos[0], logical_pos[1]) {
        app.interaction.hovered_widget_id = Some(node.id.0.clone());
        app.hover_command_id = match &node.action {
            WidgetAction::InvokeCommand { command_id, .. } => Some(command_id.clone()),
            _ => None,
        };
    } else {
        app.interaction.hovered_widget_id = None;
        app.hover_command_id = None;
    }

    if app.canvas_dialogs.active_dialog == ActiveDialog::None
        && let WorkspaceHitTarget::GridCell { sheet_id, coord, .. } = layout.hit_test_physical(phys_x, phys_y)
        && app.session.document().active_sheet_id.as_ref() == Some(&sheet_id)
    {
        let mut grid_core = std::mem::take(&mut app.view_shards.grid_core);
        crate::views::grid_core::handle_grid_pointer_hover(app, &mut grid_core, Some(coord));
        app.view_shards.grid_core = grid_core;
        return UiEventOutcome { handled: true, action: String::from("grid.hover"), ..Default::default() };
    }
    if app.view_shards.grid_core.hovered_cell.is_some() || app.grid.hover_exclusion_tooltip.is_some() {
        let mut grid_core = std::mem::take(&mut app.view_shards.grid_core);
        crate::views::grid_core::handle_grid_pointer_hover(app, &mut grid_core, None);
        app.view_shards.grid_core = grid_core;
    }

    UiEventOutcome { handled: app.interaction.hovered_widget_id.is_some(), action: String::from("pointer_move"), ..Default::default() }
}

fn execute_engine_value(app: &mut UiApp, command_id: &str, params: Value) -> Result<Value, String> {
    let outcome = app.execute_command(command_id, params).map_err(|err| err.to_string())?;
    serde_json::to_value(outcome).map_err(|err| format!("failed to serialize command outcome: {err}"))
}

fn update_canvas_object_drag(app: &mut UiApp, layout: &WorkspaceLayoutMap, phys: [f64; 2], mut drag_target: CanvasDragTarget) -> CanvasDragTarget {
    let Some(paper) = &layout.scene_paper else {
        return drag_target;
    };
    let Some(scene_pt) = paper.pixel_to_scene_pt(phys[0], phys[1]) else {
        return drag_target;
    };

    match &mut drag_target {
        CanvasDragTarget::LayoutItem {
            sheet_id,
            item_index,
            start_scene_pt,
            initial_bounds_pt,
            current_bounds_pt,
            resize_handle,
            preview_rect_pt,
        } => {
            let dx = scene_pt.x - start_scene_pt.x;
            let dy = scene_pt.y - start_scene_pt.y;
            let Some(ls) = app.document().layout_sheet(sheet_id) else {
                let fallback = raw_layout_candidate(*initial_bounds_pt, dx, dy, *resize_handle);
                *current_bounds_pt = fallback;
                *preview_rect_pt = Some(fallback);
                return drag_target;
            };
            let spec = ls.spec.clone();
            let plot_origin_offsets = compute_layout_plot_origins(app, &spec);
            let (snapped, guides) = compute_live_layout_drag_snap(
                &app.canvas_dialogs.canvas,
                &spec,
                *item_index,
                *initial_bounds_pt,
                (dx, dy),
                *resize_handle,
                &plot_origin_offsets,
            );
            app.canvas_dialogs.canvas.active_snap_guides = guides;
            *current_bounds_pt = snapped;
            *preview_rect_pt = Some(snapped);
        }
        CanvasDragTarget::GraphAnnotation { start_scene_pt, initial_bounds_pt, current_bounds_pt, .. } => {
            let raw_x = initial_bounds_pt.x + (scene_pt.x - start_scene_pt.x);
            let raw_y = initial_bounds_pt.y + (scene_pt.y - start_scene_pt.y);
            let (snapped_x, snapped_y, guides) =
                snap_to_canvas_grid(raw_x, raw_y, app.canvas_dialogs.canvas.snap_to_grid, app.canvas_dialogs.canvas.snap_grid_pt);
            app.canvas_dialogs.canvas.active_snap_guides = guides;
            *current_bounds_pt = RectPt::new(snapped_x, snapped_y, initial_bounds_pt.width, initial_bounds_pt.height);
        }
    }
    drag_target
}

fn handle_pointer_up(app: &mut UiApp, metrics: ViewportMetrics, logical_pos: [f64; 2], button: PointerButton) -> Result<UiEventOutcome, String> {
    if button != PointerButton::Primary {
        return Ok(UiEventOutcome { handled: false, action: String::from("pointer_up_ignored"), ..Default::default() });
    }
    app.interaction.pointer_down = false;
    app.interaction.last_logical_pos = Some(logical_pos);
    let layout = compute_workspace_layout_map(app, metrics);

    if app.interaction.dragging_split_divider {
        app.interaction.dragging_split_divider = false;
        app.split.dragging_divider = false;
        return Ok(UiEventOutcome { handled: true, action: String::from("split_divider.drag_end"), ..Default::default() });
    }

    if app.interaction.dragging_navigator_sheet.take().is_some() {
        if let WorkspaceHitTarget::NavigatorEntry { sheet_id: target_sheet_id, folder, index_in_folder } =
            layout.hit_test_logical(logical_pos[0], logical_pos[1])
        {
            let target_idx = live_sheet_index_in_folder(app.session.document(), folder, target_sheet_id.as_str()).unwrap_or(index_in_folder);
            app.nav.update_sheet_hover_in_folder(folder, target_idx);
        }
        let doc_snapshot = app.session.document().clone();
        if let Some((cmd_id, params)) = app.nav.commit_sheet_drop_command_for_doc(&doc_snapshot) {
            let res = execute_engine_value(app, cmd_id, params)?;
            app.nav.sync_from_document(app.session.document());
            return Ok(UiEventOutcome {
                handled: true,
                action: String::from("navigator.reorder_committed"),
                command_id: Some(cmd_id.to_owned()),
                command_result: Some(res),
            });
        }
        app.nav.cancel_sheet_drag();
        return Ok(UiEventOutcome { handled: true, action: String::from("navigator.drag_cancelled"), ..Default::default() });
    }

    if app.interaction.dragging_grid_sheet.take().is_some() {
        return Ok(UiEventOutcome { handled: true, action: String::from("grid.drag_select_end"), ..Default::default() });
    }

    if let Some(drag_handle) = app.interaction.active_drag_handle.take() {
        let moved_action = match drag_handle {
            ActiveDragHandle::FloatingNote { moved: true, note_id, .. } => Some(format!("floating_note.drag_end:{note_id}")),
            ActiveDragHandle::GridDivider { moved: true, .. } => Some(String::from("grid_core.divider_drag_end")),
            _ => None,
        };
        if let Some(action) = moved_action {
            app.interaction.pressed_widget_id = None;
            return Ok(UiEventOutcome { handled: true, action, ..Default::default() });
        }
    }

    if let Some(drag_target) = app.interaction.canvas_drag.take() {
        return commit_canvas_object_drag(app, &layout, logical_pos, drag_target);
    }

    if let Some(pressed_id) = app.interaction.pressed_widget_id.take() {
        let tree = app.build_and_cache_layout_tree(metrics);
        if let Some(node) = hit_test_logical(&tree, logical_pos[0], logical_pos[1]).cloned()
            && node.id.as_str() == pressed_id
            && node.state.enablement.enabled
        {
            return execute_widget_action(app, &node.action, metrics);
        }
        return Ok(UiEventOutcome { handled: true, action: String::from("widget.release_outside"), ..Default::default() });
    }

    if app.canvas_dialogs.active_dialog != ActiveDialog::None {
        let tree = app.build_and_cache_layout_tree(metrics);
        let action = if hit_test_logical(&tree, logical_pos[0], logical_pos[1]).is_some_and(|n| n.id.as_str() == "modal.backdrop") {
            "modal.backdrop_blocked"
        } else {
            "modal.pointer_up"
        };
        return Ok(UiEventOutcome { handled: true, action: String::from(action), ..Default::default() });
    }

    if let WorkspaceHitTarget::ScenePaper { sheet_id, scene_pt } = layout.hit_test_logical(logical_pos[0], logical_pos[1]) {
        if app.canvas_dialogs.canvas.mode == CanvasToolMode::Bracket {
            return commit_bracket_drag_on_graph(app, &sheet_id, scene_pt);
        }
        if let Some(intent) = app.canvas_dialogs.canvas.end_drag(&sheet_id, scene_pt) {
            let res = execute_engine_value(app, &intent.command_id, intent.params)?;
            return Ok(UiEventOutcome {
                handled: true,
                action: String::from("canvas.tool_intent_committed"),
                command_id: Some(intent.command_id),
                command_result: Some(res),
            });
        }
    }
    app.canvas_dialogs.canvas.drag_start_pt = None;
    Ok(UiEventOutcome { handled: true, action: String::from("pointer_up"), ..Default::default() })
}

fn commit_bracket_drag_on_graph(app: &mut UiApp, sheet_id: &SheetId, end_pt: PointPt) -> Result<UiEventOutcome, String> {
    let Some(graph) = app.session.document().graph_sheet(sheet_id).cloned() else {
        app.canvas_dialogs.canvas.drag_start_pt = None;
        return Ok(UiEventOutcome { handled: true, action: String::from("canvas.bracket_cancelled"), ..Default::default() });
    };
    let scene = match app.session.render_sheet_scene(sheet_id) {
        Ok(scene) => scene,
        Err(_) => {
            app.canvas_dialogs.canvas.drag_start_pt = None;
            return Ok(UiEventOutcome { handled: true, action: String::from("canvas.bracket_cancelled"), ..Default::default() });
        }
    };
    let Some((group_a, group_b)) = app.canvas_dialogs.canvas.resolve_bracket_drag_pair(&scene, &graph, app.session.document(), end_pt) else {
        return Ok(UiEventOutcome { handled: true, action: String::from("canvas.bracket_cancelled"), ..Default::default() });
    };
    let Some(result_sheet_id) = resolve_pairwise_result_sheet_id(app.session.document(), &graph, group_a, group_b) else {
        return Ok(UiEventOutcome { handled: true, action: String::from("canvas.bracket_no_result_sheet"), ..Default::default() });
    };
    let params = json!({
        "sheet_id": sheet_id.as_str(),
        "result_sheet_id": result_sheet_id.as_str(),
        "group_a": group_a,
        "group_b": group_b,
        "style": "square",
        "label_mode": "stars",
        "hide_if_non_significant": false,
    });
    let res = execute_engine_value(app, "graph.addComparisonBrackets", params)?;
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("canvas.bracket_added"),
        command_id: Some(String::from("graph.addComparisonBrackets")),
        command_result: Some(res),
    })
}

fn resolve_pairwise_result_sheet_id(doc: &ProjectDocument, graph: &GraphSheet, group_a: usize, group_b: usize) -> Option<SheetId> {
    let matches_pair = |res: &&curvebook_doc::ResultSheet| {
        res.pairwise_comparisons.iter().any(|c| (c.group_a == group_a && c.group_b == group_b) || (c.group_a == group_b && c.group_b == group_a))
    };
    doc.result_sheets
        .iter()
        .find(|res| res.analysis.source_sheet_id == graph.spec.primary_source_sheet_id && matches_pair(res))
        .or_else(|| {
            doc.result_sheets
                .iter()
                .find(|res| res.analysis.source_sheet_id == graph.spec.primary_source_sheet_id && !res.pairwise_comparisons.is_empty())
        })
        .or_else(|| doc.result_sheets.iter().find(|res| matches_pair(res)))
        .or_else(|| doc.result_sheets.iter().find(|res| !res.pairwise_comparisons.is_empty()))
        .map(|res| res.id.clone())
}

fn commit_canvas_object_drag(
    app: &mut UiApp,
    layout: &WorkspaceLayoutMap,
    logical_pos: [f64; 2],
    drag_target: CanvasDragTarget,
) -> Result<UiEventOutcome, String> {
    let [phys_x, phys_y] = layout.metrics.logical_to_physical_px(logical_pos);
    let updated = update_canvas_object_drag(app, layout, [phys_x, phys_y], drag_target);
    app.canvas_dialogs.canvas.active_snap_guides.clear();

    match updated {
        CanvasDragTarget::LayoutItem { sheet_id, item_index, current_bounds_pt, .. } => {
            let Some(mut spec) = app.document().layout_sheet(&sheet_id).map(|l| l.spec.clone()) else {
                return Ok(UiEventOutcome { handled: false, action: String::from("layout.missing"), ..Default::default() });
            };
            if let Some(item) = spec.items.get_mut(item_index) {
                set_layout_item_bounds(item, current_bounds_pt);
            }
            let res = execute_engine_value(app, "layout.setSpec", json!({ "sheet_id": sheet_id.as_str(), "spec": spec }))?;
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("layout.item_drag_committed"),
                command_id: Some(String::from("layout.setSpec")),
                command_result: Some(res),
            })
        }
        CanvasDragTarget::GraphAnnotation { sheet_id, annotation_index, current_bounds_pt, .. } => {
            let Some(mut spec) = app.document().graph_sheet(&sheet_id).map(|g| g.spec.clone()) else {
                return Ok(UiEventOutcome { handled: false, action: String::from("graph.missing"), ..Default::default() });
            };
            if let Some(ann) = spec.annotations.get_mut(annotation_index) {
                set_graph_annotation_bounds(ann, current_bounds_pt);
            }
            let res = execute_engine_value(app, "graph.setSpec", json!({ "sheet_id": sheet_id.as_str(), "spec": spec }))?;
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("graph.annotation_drag_committed"),
                command_id: Some(String::from("graph.setSpec")),
                command_result: Some(res),
            })
        }
    }
}

fn handle_double_click(app: &mut UiApp, metrics: ViewportMetrics, logical_pos: [f64; 2], button: PointerButton) -> UiEventOutcome {
    if button != PointerButton::Primary {
        return UiEventOutcome { handled: false, action: String::from("double_click_ignored"), ..Default::default() };
    }
    let initial_for = |cell: Option<&curvebook_doc::Cell>| -> String {
        cell.map_or_else(String::new, |cell| match &cell.value {
            CellValue::Text(s) if s.starts_with('=') => s.clone(),
            _ => format_cell_display(cell, &app.locale_preset),
        })
    };
    let layout = compute_workspace_layout_map(app, metrics);
    match layout.hit_test_logical(logical_pos[0], logical_pos[1]) {
        WorkspaceHitTarget::GridXCell { sheet_id, row, subcol } => {
            app.grid.select_x_cell_sync(row, subcol);
            app.focus_widget_id = Some(format!("grid.xcell.{row}.{subcol}"));
            let is_frozen = app.document().data_sheet(&sheet_id).is_some_and(|ds| ds.frozen);
            if is_frozen {
                return UiEventOutcome { handled: true, action: String::from("grid_core.frozen_read_only"), ..Default::default() };
            }
            let is_generated = app
                .document()
                .data_sheet(&sheet_id)
                .is_some_and(|ds| matches!(&ds.archetype, TableArchetype::Xy(xy) if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. })));
            if is_generated {
                return UiEventOutcome { handled: true, action: String::from("grid.x_cell_readonly"), ..Default::default() };
            }
            let initial_text = initial_for(app.document().data_sheet(&sheet_id).and_then(|ds| ds.x_or_time_cell(row, subcol)));
            app.inline_editor.begin(InlineEditorTarget::GridXCell { sheet_id, row, subcol }, initial_text);
            UiEventOutcome { handled: true, action: String::from("grid.x_inline_edit_begin"), ..Default::default() }
        }
        WorkspaceHitTarget::GridCell { sheet_id, coord, .. } => {
            app.grid.select_single_sync(coord);
            let mut grid_core = std::mem::take(&mut app.view_shards.grid_core);
            let link_out = crate::views::grid_core::handle_grid_double_click(app, &mut grid_core, coord);
            app.view_shards.grid_core = grid_core;
            if link_out.handled {
                return link_out;
            }
            let initial_text = initial_for(app.document().data_sheet(&sheet_id).and_then(|ds| ds.cell(CellCoord::from(coord))));
            app.inline_editor.begin(InlineEditorTarget::GridCell { sheet_id, coord }, initial_text);
            UiEventOutcome { handled: true, action: String::from("grid.inline_edit_begin"), ..Default::default() }
        }
        WorkspaceHitTarget::InfoConstant { sheet_id, name, value_text } => {
            app.inline_editor.begin(InlineEditorTarget::InfoConstant { sheet_id, name }, value_text);
            UiEventOutcome { handled: true, action: String::from("info.constant_inline_edit_begin"), ..Default::default() }
        }
        WorkspaceHitTarget::InfoNotes { sheet_id } => {
            let notes = app.document().info_sheet(&sheet_id).map(|i| i.notes_markdown.clone()).unwrap_or_default();
            app.inline_editor.begin(InlineEditorTarget::InfoNotes { sheet_id }, notes);
            UiEventOutcome { handled: true, action: String::from("info.notes_inline_edit_begin"), ..Default::default() }
        }
        WorkspaceHitTarget::CanvasTitle { sheet_id } => {
            let title = app
                .document()
                .graph_sheet(&sheet_id)
                .map(|g| if g.spec.title.is_empty() { g.title.clone() } else { g.spec.title.clone() })
                .or_else(|| app.document().layout_sheet(&sheet_id).map(|l| l.title.clone()))
                .unwrap_or_default();
            app.inline_editor.begin(InlineEditorTarget::GraphTitle { sheet_id }, title);
            UiEventOutcome { handled: true, action: String::from("canvas.title_inline_edit_begin"), ..Default::default() }
        }
        WorkspaceHitTarget::AxisTitle { sheet_id, axis } => {
            let title = app
                .document()
                .graph_sheet(&sheet_id)
                .map(|g| if axis == "y" { g.spec.y_axis.title.clone() } else { g.spec.x_axis.title.clone() })
                .unwrap_or_default();
            app.inline_editor.begin(InlineEditorTarget::AxisTitle { sheet_id, axis }, title);
            UiEventOutcome { handled: true, action: String::from("canvas.axis_title_inline_edit_begin"), ..Default::default() }
        }
        _ => UiEventOutcome { handled: false, action: String::from("double_click_miss"), ..Default::default() },
    }
}

fn handle_wheel_zoom(app: &mut UiApp, metrics: ViewportMetrics, logical_pos: [f64; 2], delta_y: f64) -> UiEventOutcome {
    if !delta_y.is_finite() {
        return UiEventOutcome { handled: false, action: String::from("wheel_zoom_invalid"), ..Default::default() };
    }
    let scale_delta = (-delta_y * 0.002).exp().clamp(0.5, 2.0);
    handle_pinch_zoom(app, metrics, logical_pos, scale_delta)
}

fn handle_pinch_zoom(app: &mut UiApp, metrics: ViewportMetrics, logical_pos: [f64; 2], scale_delta: f64) -> UiEventOutcome {
    if !scale_delta.is_finite() || scale_delta <= 0.0 {
        return UiEventOutcome { handled: false, action: String::from("pinch_zoom_invalid"), ..Default::default() };
    }
    let layout = compute_workspace_layout_map(app, metrics);
    let [phys_x, phys_y] = metrics.logical_to_physical_px(logical_pos);
    let anchor_pt = if let Some(paper) = &layout.scene_paper
        && paper.paper.width > 0
        && paper.paper.height > 0
    {
        let ax = ((phys_x - f64::from(paper.paper.x)) / f64::from(paper.paper.width)) * paper.scene_width_pt;
        let ay = ((phys_y - f64::from(paper.paper.y)) / f64::from(paper.paper.height)) * paper.scene_height_pt;
        [ax, ay]
    } else {
        [logical_pos[0], logical_pos[1]]
    };

    app.canvas_dialogs.pan_zoom.apply_pinch_zoom(scale_delta, anchor_pt);
    sync_canvas_camera_from_inertia(app);
    UiEventOutcome {
        handled: true,
        action: String::from("canvas.pinch_zoom"),
        command_result: Some(json!({ "zoom": app.canvas_dialogs.pan_zoom.zoom, "pan_pt": app.canvas_dialogs.pan_zoom.pan_pt })),
        ..Default::default()
    }
}

fn handle_inertial_pan(app: &mut UiApp, metrics: ViewportMetrics, delta: [f64; 2], dt_sec: f64, step_coast: bool) -> UiEventOutcome {
    let _ = metrics;
    if step_coast {
        let still_moving = app.canvas_dialogs.pan_zoom.step_inertia(dt_sec);
        sync_canvas_camera_from_inertia(app);
        return UiEventOutcome {
            handled: true,
            action: String::from("canvas.inertial_coast"),
            command_result: Some(json!({ "moving": still_moving, "pan_pt": app.canvas_dialogs.pan_zoom.pan_pt })),
            ..Default::default()
        };
    }
    app.canvas_dialogs.pan_zoom.apply_pan_delta(delta, dt_sec);
    sync_canvas_camera_from_inertia(app);
    UiEventOutcome {
        handled: true,
        action: String::from("canvas.pan_delta"),
        command_result: Some(json!({ "pan_pt": app.canvas_dialogs.pan_zoom.pan_pt, "vel_pt_per_sec": app.canvas_dialogs.pan_zoom.vel_pt_per_sec })),
        ..Default::default()
    }
}

fn sync_canvas_camera_from_inertia(app: &mut UiApp) {
    app.canvas_dialogs.canvas.zoom = app.canvas_dialogs.pan_zoom.zoom;
    app.canvas_dialogs.canvas.pan_pt = PointPt::new(app.canvas_dialogs.pan_zoom.pan_pt[0], app.canvas_dialogs.pan_zoom.pan_pt[1]);
}

impl UiApp {
    /// Advance active inertial pan coasting by `dt_sec` and synchronize canvas camera offsets.
    pub fn tick_inertial_pan(&mut self, dt_sec: f64) -> bool {
        let moving = self.canvas_dialogs.pan_zoom.step_inertia(dt_sec);
        sync_canvas_camera_from_inertia(self);
        moving
    }
}

fn should_handle_chrome_widget_click(node: &crate::layout::WidgetNode) -> bool {
    if node.z_layer >= ZLayer::DropdownMenu {
        return node.action != WidgetAction::NoOp;
    }
    let id = node.id.as_str();
    if id == "result.diagnostic_banner" || id.starts_with("canvas.surface.") || id.starts_with("info.") {
        return false;
    }
    !matches!(node.action, WidgetAction::NoOp | WidgetAction::SelectGridCell { .. } | WidgetAction::SwitchSheet { .. } | WidgetAction::SwapSplitPanes)
}

fn ensure_text_field_buffer(app: &mut UiApp, wid: &str, initial: &str) {
    let multiline = matches!(
        wid,
        "info.notes" | "info.secondary.notes" | "import_paste.input.csv" | "import_paste.input.notes" | "import_paste.input.plate_values"
    );
    app.interaction.text_field_buffers.entry(wid.to_owned()).or_insert_with(|| TextFieldEditState::new(initial, multiline));
}

fn sync_power_pilot_text_field(app: &mut UiApp, fid: &str, text: &str) {
    let Some(val) = parse_localized_number_input(text.trim(), &app.locale_preset).filter(|v| v.is_finite()) else {
        return;
    };
    match fid {
        "power.pilot.mean1.input" => app.view_shards.power_wizard.pilot_mean1 = val,
        "power.pilot.mean2.input" => app.view_shards.power_wizard.pilot_mean2 = val,
        "power.pilot.sd.input" if val > 0.0 => app.canvas_dialogs.power_wizard.sd = val,
        "power.pilot.corr.input" => app.canvas_dialogs.power_wizard.within_pair_correlation = val.clamp(-0.99, 0.99),
        "power.pilot.p1.input" => app.canvas_dialogs.power_wizard.p1 = val.clamp(0.01, 0.99),
        "power.pilot.p2.input" => app.canvas_dialogs.power_wizard.p2 = val.clamp(0.01, 0.99),
        "power.pilot.hr.input" => app.view_shards.power_wizard.survival_hazard_ratio = val.max(0.05),
        "power.pilot.evprob.input" => app.view_shards.power_wizard.survival_event_prob = val.clamp(0.01, 0.99),
        "power.pilot.r2.input" => app.canvas_dialogs.power_wizard.r_squared = val.clamp(0.001, 0.999),
        "power.pilot.std.input" => app.canvas_dialogs.power_wizard.standardized_effect = val,
        _ => {}
    }
}

fn parse_usize_list_text(raw: &str) -> Option<Vec<usize>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(Vec::new());
    }
    if trimmed.starts_with('[') {
        return serde_json::from_str::<Vec<usize>>(trimmed).ok();
    }
    let mut out = Vec::new();
    for tok in trimmed.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()) {
        out.push(tok.parse::<usize>().ok()?);
    }
    Some(out)
}

fn sync_edit_params_text_field(app: &mut UiApp, fid: &str, text: &str) {
    let mut map = serde_json::Map::new();
    match fid {
        "edit.transform.formula" | "edit.transform.filter_formula" => {
            map.insert(fid.to_owned(), Value::String(text.to_owned()));
        }
        "edit.transform.selected_data_sets" | "edit.transform.baseline_columns" => {
            let Some(cols) = parse_usize_list_text(text) else {
                return;
            };
            map.insert(fid.to_owned(), serde_json::json!(cols));
        }
        _ => return,
    }
    let _ = app.canvas_dialogs.edit_params.apply_field_patch(&map);
}

fn sync_format_axes_text_field(app: &mut UiApp, fid: &str, text: &str) {
    let doc = app.document().clone();
    let mut map = serde_json::Map::new();
    match fid {
        "axes.title" => {
            map.insert(String::from("axes.title"), Value::String(text.to_owned()));
        }
        "axes.y_thresholds" | "axes.x_thresholds" => {
            let trimmed = text.trim();
            let val = if trimmed.is_empty() {
                Value::Array(Vec::new())
            } else if trimmed.starts_with('[') || trimmed.starts_with('{') {
                let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
                    return;
                };
                parsed
            } else {
                Value::String(trimmed.to_owned())
            };
            map.insert(fid.to_owned(), val);
        }
        _ => return,
    }
    let _ = app.canvas_dialogs.format_axes.apply_field_patch(&map, Some(&doc));
}

/// Flush any buffered `FormatAxes` textbox edits (`axes.title`, `axes.y_thresholds`, `axes.x_thresholds`) into `format_axes`.
pub fn sync_format_axes_text_buffers(app: &mut UiApp) {
    for fid in ["axes.title", "axes.y_thresholds", "axes.x_thresholds"] {
        if let Some(text) = app.interaction.text_field_buffers.get(fid).map(|b| b.text.clone()) {
            sync_format_axes_text_field(app, fid, &text);
        }
    }
}

/// Flush any buffered `EditAnalysisParameters` transform textbox edits into `edit_params`.
pub fn sync_edit_params_text_buffers(app: &mut UiApp) {
    for fid in ["edit.transform.formula", "edit.transform.filter_formula", "edit.transform.selected_data_sets", "edit.transform.baseline_columns"] {
        if let Some(text) = app.interaction.text_field_buffers.get(fid).map(|b| b.text.clone()) {
            sync_edit_params_text_field(app, fid, &text);
        }
    }
}

fn sync_focused_text_field(app: &mut UiApp, fid: &str) {
    let text = app.interaction.text_field_buffers.get(fid).map(|b| b.text.clone()).unwrap_or_default();
    if fid.starts_with("import_paste.input.") {
        crate::views::import_paste::on_text_field_changed(app, fid, &text);
        return;
    }
    if fid.starts_with("power.pilot.") && fid.ends_with(".input") {
        sync_power_pilot_text_field(app, fid, &text);
        return;
    }
    match fid {
        "palette.query" => app.canvas_dialogs.command_palette.query = text,
        "formula_bar.input" => {
            let doc = app.document().clone();
            app.grid.formula_bar.set_input(&text, None, &doc);
            app.view_shards.mv_formula.validation_feedback = None;
        }
        "analyze.search" => {
            app.view_shards.analyze_picker.search_query = text.clone();
            app.canvas_dialogs.analyze.search_query = text;
        }
        "navigator.search_input" => app.nav.search_query = text,
        "welcome.title.input" => app.nav.welcome.table_draft.title = text,
        "welcome.clone.title.input" => app.nav.welcome.clone_draft.options.title = text,
        "hook.search_input" => app.canvas_dialogs.hook_picker.search_query = text,
        "exclude.note_input" => app.canvas_dialogs.exclude_with_reason.note = text,
        "page_layout.callout_input" => app.canvas_dialogs.page_layout.new_callout_markdown = text,
        "rand_groups.arms_input" => {
            let _ = app.canvas_dialogs.randomize_groups.set_groups_from_str(&text);
        }
        "rand_groups.title_input" => app.canvas_dialogs.randomize_groups.title = text,
        "mv_formula.func.search" => app.view_shards.mv_formula.function_search = text,
        "mv_formula.name.input" => {
            app.grid.formula_bar.target_title = text;
            app.view_shards.mv_formula.validation_feedback = None;
        }
        "mv_formula.vars.recode_from.input" => app.view_shards.mv_formula.recode_from = text,
        "mv_formula.vars.recode_to.input" => app.view_shards.mv_formula.recode_to = text,
        "fit.eq.search" => app.view_shards.fit_params.model_search = text,
        "edit.transform.formula" | "edit.transform.filter_formula" | "edit.transform.selected_data_sets" | "edit.transform.baseline_columns" => {
            sync_edit_params_text_field(app, fid, &text)
        }
        "axes.title" | "axes.y_thresholds" | "axes.x_thresholds" => sync_format_axes_text_field(app, fid, &text),
        "inspector.axes.title.input" | "inspector.mv.color_clamp_min.input" | "inspector.mv.color_clamp_max.input" => {}
        _ => {}
    }
}

/// Execute any retained [`WidgetAction`] cleanly through the unified desktop/web/driver handler.
///
/// # Errors
/// Returns an error string if the triggered command or dialog action fails.
pub fn execute_widget_action(app: &mut UiApp, action: &WidgetAction, metrics: ViewportMetrics) -> Result<UiEventOutcome, String> {
    match action {
        WidgetAction::NoOp => Ok(UiEventOutcome { handled: false, action: String::from("widget.noop"), ..Default::default() }),
        WidgetAction::MenuOpen(key) => {
            app.interaction.open_dropdown = None;
            app.interaction.open_menu = if app.interaction.open_menu.as_deref() == Some(key.as_str()) { None } else { Some(key.clone()) };
            Ok(UiEventOutcome { handled: true, action: format!("menu.toggle:{key}"), ..Default::default() })
        }
        WidgetAction::InvokeCommand { command_id, params } => {
            if command_id == "ui.locale.set" && params.get("locale").and_then(Value::as_str).is_none_or(|s| s.trim().is_empty()) {
                let out = open_dropdown_for_id(app, "menu.ui.locale.set", metrics)?;
                app.interaction.open_menu = None;
                clear_context_menu_state(app);
                return Ok(out);
            }
            app.interaction.open_menu = None;
            app.interaction.open_dropdown = None;
            clear_context_menu_state(app);
            if matches!(command_id.as_str(), "project.open" | "file.open")
                && params.get("path").and_then(Value::as_str).is_none_or(|p| p.trim().is_empty())
            {
                return Ok(UiEventOutcome {
                    handled: true,
                    action: String::from("welcome.open_file_requested"),
                    command_id: Some(String::from("project.open")),
                    command_result: None,
                });
            }
            let res = if let Some(ui_val) = crate::control_io::dispatch_ui_command(app, command_id, params).map_err(|e| e.to_string())? {
                UiEventOutcome {
                    handled: true,
                    action: format!("command.ui:{command_id}"),
                    command_id: Some(command_id.clone()),
                    command_result: Some(ui_val),
                }
            } else {
                let res = execute_engine_value(app, command_id, params.clone())?;
                UiEventOutcome {
                    handled: true,
                    action: format!("command.engine:{command_id}"),
                    command_id: Some(command_id.clone()),
                    command_result: Some(res),
                }
            };
            if matches!(command_id.as_str(), "edit.undo" | "edit.redo" | "sheet.select") {
                clear_inspector_axis_buffers(app);
            }
            Ok(res)
        }
        WidgetAction::SwitchSheet { sheet_id, .. } => {
            commit_inline_edit_if_active(app)?;
            app.interaction.context_menu_target = None;
            let res = execute_engine_value(app, "sheet.select", json!({ "sheet_id": sheet_id.as_str() }))?;
            app.nav.sync_from_document(app.session.document());
            sync_dialogs_on_sheet_switch(app, sheet_id);
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("sheet.select"),
                command_id: Some(String::from("sheet.select")),
                command_result: Some(res),
            })
        }
        WidgetAction::ToggleFolder(section) => {
            if let Some(pos) = app.nav.collapsed_sections.iter().position(|s| s == section) {
                app.nav.collapsed_sections.remove(pos);
            } else {
                app.nav.collapsed_sections.push(*section);
            }
            for folder in &mut app.nav.folders {
                if folder.section == *section {
                    folder.collapsed = app.nav.collapsed_sections.contains(section);
                }
            }
            Ok(UiEventOutcome { handled: true, action: format!("nav.toggle_folder:{section:?}"), ..Default::default() })
        }
        WidgetAction::ToggleFamilyPane => {
            let _ = app.toggle_family_pane();
            Ok(UiEventOutcome { handled: true, action: String::from("nav.toggle_family"), ..Default::default() })
        }
        WidgetAction::SetSplitRatio(ratio) => {
            app.split.enabled = true;
            app.split.ratio = ratio.clamp(0.2, 0.8);
            Ok(UiEventOutcome { handled: true, action: String::from("split.ratio"), ..Default::default() })
        }
        WidgetAction::SwapSplitPanes => {
            std::mem::swap(&mut app.split.primary_sheet_id, &mut app.split.secondary_sheet_id);
            Ok(UiEventOutcome { handled: true, action: String::from("split.swap"), ..Default::default() })
        }
        WidgetAction::SelectGridCell { row, col, subcol } => {
            let coord = GridCellCoord::new(*row as usize, *col as usize, *subcol as usize);
            app.grid.select_single_sync(coord);
            app.grid.refresh_selection_stats(app.session.document());
            Ok(UiEventOutcome { handled: true, action: String::from("grid.select_cell"), ..Default::default() })
        }
        WidgetAction::StartInlineGridEdit { row, col, subcol } => {
            let coord = GridCellCoord::new(*row as usize, *col as usize, *subcol as usize);
            app.grid.select_single_sync(coord);
            Ok(UiEventOutcome { handled: true, action: String::from("grid.edit_begin"), ..Default::default() })
        }
        WidgetAction::FocusWidget(wid) => {
            app.focus_widget_id = Some(wid.clone());
            if wid == "formula_bar.input" {
                app.grid.formula_bar.active = true;
            }
            Ok(UiEventOutcome { handled: true, action: format!("widget.focus:{wid}"), ..Default::default() })
        }
        WidgetAction::ScrollBy { container_id, delta_x, delta_y } => {
            let tree = app.build_and_cache_layout_tree(metrics);
            if let Some(meta) = tree.nodes.iter().filter_map(|n| n.scroll.as_ref()).find(|s| s.container_id.as_str() == container_id) {
                app.interaction.scroll_offsets.apply_wheel(meta, *delta_x, *delta_y);
            }
            Ok(UiEventOutcome { handled: true, action: format!("scroll.by:{container_id}"), ..Default::default() })
        }
        WidgetAction::ScrollbarTrackJump { container_id, vertical, ratio } => {
            let tree = app.build_and_cache_layout_tree(metrics);
            if let Some(meta) = tree.nodes.iter().filter_map(|n| n.scroll.as_ref()).find(|s| s.container_id.as_str() == container_id) {
                app.interaction.scroll_offsets.apply_track_ratio(meta, *vertical, *ratio);
            }
            Ok(UiEventOutcome { handled: true, action: format!("scroll.jump:{container_id}"), ..Default::default() })
        }
        WidgetAction::SelectResultTab(tab_id) => {
            if let Some(rs_id) = app.document().active_sheet_id.clone()
                && let Some(rs) = app.document().result_sheet(&rs_id)
                && let Some(idx) = rs.tabs.iter().position(|t| &t.id == tab_id)
            {
                app.canvas_dialogs.result_view.active_tab_index = idx;
            }
            Ok(UiEventOutcome { handled: true, action: format!("result.select_tab:{tab_id}"), ..Default::default() })
        }
        WidgetAction::ToggleInspectorTab(mode) => {
            app.canvas_dialogs.active_inspector = *mode;
            Ok(UiEventOutcome { handled: true, action: format!("inspector.mode:{mode:?}"), ..Default::default() })
        }
        WidgetAction::SetCanvasZoomFit => {
            app.canvas_dialogs.canvas.zoom = 1.0;
            app.canvas_dialogs.canvas.pan_pt = PointPt::new(0.0, 0.0);
            app.canvas_dialogs.pan_zoom.zoom = 1.0;
            app.canvas_dialogs.pan_zoom.pan_pt = [0.0, 0.0];
            Ok(UiEventOutcome { handled: true, action: String::from("canvas.zoom_fit"), ..Default::default() })
        }
        WidgetAction::CycleCanvasZoomPreset => {
            let next = crate::layout::canvas_viewport::next_zoom_preset(app.canvas_dialogs.canvas.zoom);
            app.canvas_dialogs.canvas.zoom = next;
            app.canvas_dialogs.pan_zoom.zoom = next;
            Ok(UiEventOutcome { handled: true, action: format!("canvas.zoom_preset:{next}"), ..Default::default() })
        }
        WidgetAction::SelectCanvasTool(_) | WidgetAction::SelectLayoutPanel(_) => {
            Ok(UiEventOutcome { handled: true, action: String::from("canvas.tool_action"), ..Default::default() })
        }
        WidgetAction::ClosePopupOrModal => {
            if app.interaction.open_dropdown.take().is_none() && app.interaction.open_menu.take().is_none() {
                close_active_modal(app);
            }
            app.interaction.context_menu_target = None;
            Ok(UiEventOutcome { handled: true, action: String::from("popup_or_modal.closed"), ..Default::default() })
        }
        WidgetAction::Custom { domain, payload } => {
            let mut shard_states = std::mem::take(&mut app.view_shards);
            let res = crate::views::dispatch_custom_view_action(app, &mut shard_states, domain, payload);
            app.view_shards = shard_states;
            res
        }
        other => execute_dialog_widget_action(app, other, metrics),
    }
}

fn execute_dialog_widget_action(app: &mut UiApp, action: &WidgetAction, metrics: ViewportMetrics) -> Result<UiEventOutcome, String> {
    match action {
        WidgetAction::SelectAnalyzeCategory(cat) => {
            app.canvas_dialogs.analyze.category_filter = match cat.as_str() {
                "transform" => Some(AnalysisCategory::Transform),
                "column" => Some(AnalysisCategory::Column),
                "grouped" => Some(AnalysisCategory::Grouped),
                "xy" => Some(AnalysisCategory::XyAndRegression),
                "survival" => Some(AnalysisCategory::Survival),
                "contingency" => Some(AnalysisCategory::Contingency),
                "multivariate" => Some(AnalysisCategory::Multivariate),
                _ => None,
            };
            let first = app.canvas_dialogs.analyze.compatible_entries().first().map(|e| e.id.to_owned());
            if let Some(id) = first {
                app.canvas_dialogs.analyze.select_catalog_entry(&id);
            }
            Ok(UiEventOutcome { handled: true, action: format!("analyze.category:{cat}"), ..Default::default() })
        }
        WidgetAction::SelectAnalyzeEntry(entry_id) => {
            app.canvas_dialogs.analyze.select_catalog_entry(entry_id);
            Ok(UiEventOutcome { handled: true, action: format!("analyze.entry:{entry_id}"), ..Default::default() })
        }
        WidgetAction::SelectEditParamsTab(tab_id) => {
            let mut map = serde_json::Map::new();
            map.insert(String::from("active_tab"), json!(tab_id));
            app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
            Ok(UiEventOutcome { handled: true, action: format!("edit.tab:{tab_id}"), ..Default::default() })
        }
        WidgetAction::ToggleDialogCheckbox(field) => {
            let next_val = match field.as_str() {
                "assume_gaussian" => !app.canvas_dialogs.edit_params.assume_gaussian,
                "paired_or_matched" => !app.canvas_dialogs.edit_params.paired_or_matched,
                "welch_correction" => !app.canvas_dialogs.edit_params.welch_correction,
                "compare_against_control" => !app.canvas_dialogs.edit_params.compare_against_control,
                _ => true,
            };
            let mut map = serde_json::Map::new();
            map.insert(field.clone(), Value::Bool(next_val));
            app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
            Ok(UiEventOutcome { handled: true, action: format!("dialog.toggle:{field}"), ..Default::default() })
        }
        WidgetAction::SelectDialogRadio { group, value } => {
            let mut map = serde_json::Map::new();
            map.insert(group.clone(), Value::String(value.clone()));
            app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
            Ok(UiEventOutcome { handled: true, action: format!("dialog.radio:{group}={value}"), ..Default::default() })
        }
        WidgetAction::OpenDropdown(dropdown_id) => open_dropdown_for_id(app, dropdown_id, metrics),
        WidgetAction::SelectDropdownOption { dropdown_id, value } => {
            app.interaction.open_dropdown = None;
            let mut map = serde_json::Map::new();
            if dropdown_id == "edit.multiplicity_method" {
                map.insert(String::from("multiplicity_method"), Value::String(value.clone()));
                app.canvas_dialogs.edit_params.apply_field_patch(&map)?;
            } else if dropdown_id == "inspector.pairwise_mode" {
                map.insert(String::from("display_mode"), Value::String(value.clone()));
                app.canvas_dialogs.pairwise.apply_field_patch(&map, None)?;
            } else if dropdown_id == "menu.ui.locale.set" {
                let ui_val = crate::control_io::dispatch_ui_command(app, "ui.locale.set", &json!({ "locale": value })).map_err(|e| e.to_string())?;
                return Ok(UiEventOutcome {
                    handled: true,
                    action: format!("dropdown.select:{dropdown_id}={value}"),
                    command_id: Some(String::from("ui.locale.set")),
                    command_result: ui_val,
                });
            }
            Ok(UiEventOutcome { handled: true, action: format!("dropdown.select:{dropdown_id}={value}"), ..Default::default() })
        }
        WidgetAction::StepSpinner { field_id, delta } => {
            if field_id == "edit.alpha"
                && let Some(cur_pct) = app.canvas_dialogs.edit_params.draft.as_ref().map(|d| d.confidence_level.percent())
            {
                let next_pct = (cur_pct - delta * 100.0).clamp(50.0, 99.9);
                let _ = app.canvas_dialogs.edit_params.set_confidence_percent(next_pct);
            }
            Ok(UiEventOutcome { handled: true, action: format!("spinner.step:{field_id}"), ..Default::default() })
        }
        WidgetAction::SetSlider { field_id, normalized } => {
            if field_id == "inspector.zoom_slider" {
                let z = (0.5 + normalized * 1.5).clamp(0.5, 2.0);
                app.canvas_dialogs.canvas.zoom = z;
                app.canvas_dialogs.pan_zoom.zoom = z;
            }
            Ok(UiEventOutcome { handled: true, action: format!("slider.set:{field_id}"), ..Default::default() })
        }
        WidgetAction::DialogCancel => {
            close_active_modal(app);
            app.interaction.open_dropdown = None;
            Ok(UiEventOutcome { handled: true, action: String::from("dialog.cancelled"), ..Default::default() })
        }
        WidgetAction::DialogConfirm => {
            let res = crate::gui_actions::submit_active_dialog_ok(app).map_err(|e| e.to_string())?;
            Ok(UiEventOutcome { handled: true, action: String::from("dialog.confirmed"), command_result: Some(res), ..Default::default() })
        }
        WidgetAction::InvokeDialogButton(btn) => {
            let res = crate::gui_actions::execute_dialog_button_action(app, btn).map_err(|e| e.to_string())?;
            app.interaction.open_dropdown = None;
            Ok(UiEventOutcome { handled: true, action: format!("dialog.button:{btn}"), command_result: Some(res), ..Default::default() })
        }
        WidgetAction::ClickDialogControl(wid) => {
            if let Some(res) = crate::gui_actions::apply_dialog_widget_click(app, wid).map_err(|e| e.to_string())? {
                return Ok(UiEventOutcome {
                    handled: true,
                    action: format!("dialog.control:{wid}"),
                    command_result: Some(res),
                    ..Default::default()
                });
            }
            Ok(UiEventOutcome { handled: true, action: format!("dialog.control_focus:{wid}"), ..Default::default() })
        }
        _ => Ok(UiEventOutcome { handled: false, action: String::from("widget.noop"), ..Default::default() }),
    }
}

fn open_dropdown_for_id(app: &mut UiApp, dropdown_id: &str, metrics: ViewportMetrics) -> Result<UiEventOutcome, String> {
    if app.interaction.open_dropdown.as_ref().is_some_and(|d| d.dropdown_id == dropdown_id) {
        app.interaction.open_dropdown = None;
        return Ok(UiEventOutcome { handled: true, action: format!("dropdown.closed:{dropdown_id}"), ..Default::default() });
    }
    let tree = app.build_and_cache_layout_tree(metrics);
    let anchor = tree.node(dropdown_id).map(|n| n.bounds.rect_px).unwrap_or_default();
    if dropdown_id == "edit.multiplicity_method" {
        let tukey_ok = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.post_hoc.tukey").enabled;
        let bonf_ok = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.post_hoc.bonferroni").enabled;
        let sidak_ok = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.post_hoc.sidak").enabled;
        let holm_ok = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.post_hoc.holm_sidak").enabled;
        let lsd_ok = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.post_hoc.fisher_lsd").enabled;
        let dunnett_ok = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.post_hoc.dunnett").enabled;
        let nonparam_ok = evaluate_dialog_widget(&app.canvas_dialogs, app.document(), "edit.post_hoc.dunn").enabled;
        let opts = vec![
            DropdownOption { value: String::from("none"), label: String::from("None (uncorrected)"), enabled: true },
            DropdownOption { value: String::from("tukey"), label: String::from("Tukey HSD"), enabled: tukey_ok },
            DropdownOption { value: String::from("bonferroni"), label: String::from("Bonferroni"), enabled: bonf_ok },
            DropdownOption { value: String::from("sidak"), label: String::from("Sidak"), enabled: sidak_ok },
            DropdownOption { value: String::from("holm_sidak"), label: String::from("Holm-Sidak"), enabled: holm_ok },
            DropdownOption { value: String::from("fisher_lsd"), label: String::from("Fisher's LSD"), enabled: lsd_ok },
            DropdownOption { value: String::from("bh"), label: String::from("Benjamini-Hochberg (FDR)"), enabled: true },
            DropdownOption { value: String::from("bky"), label: String::from("Two-Stage BKY (FDR)"), enabled: true },
            DropdownOption { value: String::from("by"), label: String::from("Benjamini-Yekutieli (FDR)"), enabled: true },
            DropdownOption { value: String::from("dunnett"), label: String::from("Dunnett"), enabled: dunnett_ok },
            DropdownOption { value: String::from("dunn"), label: String::from("Dunn (Rank Sum)"), enabled: nonparam_ok },
        ];
        app.interaction.open_dropdown = Some(OpenDropdownState {
            dropdown_id: dropdown_id.to_owned(),
            label: String::from("Multiple Comparisons"),
            z_layer: ZLayer::ModalDropdown,
            anchor_rect_px: anchor,
            selected_value: String::from("tukey"),
            options: opts,
        });
    } else if dropdown_id == "inspector.pairwise_mode" {
        let opts = vec![
            DropdownOption { value: String::from("stars"), label: String::from("Significance Stars"), enabled: true },
            DropdownOption { value: String::from("exact"), label: String::from("Numeric P Values"), enabled: true },
            DropdownOption { value: String::from("cld"), label: String::from("Compact Letter Display"), enabled: true },
        ];
        app.interaction.open_dropdown = Some(OpenDropdownState {
            dropdown_id: dropdown_id.to_owned(),
            label: String::from("Pairwise Display Mode"),
            z_layer: ZLayer::DropdownMenu,
            anchor_rect_px: anchor,
            selected_value: String::from("stars"),
            options: opts,
        });
    } else if dropdown_id == "menu.ui.locale.set" {
        let picker_w = (248.0 * metrics.scale_factor).round().clamp(210.0, 520.0) as u32;
        let toolbar_rect = crate::LayoutRectPx::from_workspace_rect(tree.frame.toolbar);
        let anchor_x = tree.node("menu.view").map_or(toolbar_rect.x + 8, |n| {
            if tree.is_rtl { n.bounds.rect_px.right() - i32::try_from(picker_w).unwrap_or(248) } else { n.bounds.rect_px.x }
        });
        let picker_anchor = crate::LayoutRectPx::new(anchor_x, toolbar_rect.bottom(), picker_w, 0);
        app.interaction.scroll_offsets.offsets.remove("dropdown_popup.menu.ui.locale.set");
        let opts: Vec<DropdownOption> = crate::i18n::LOCALES
            .iter()
            .map(|&loc| DropdownOption { value: String::from(loc), label: crate::i18n::locale_picker_label(loc), enabled: true })
            .collect();
        app.interaction.open_dropdown = Some(OpenDropdownState {
            dropdown_id: dropdown_id.to_owned(),
            label: String::from(crate::i18n::command_label(&app.locale, "ui.locale.set", "Language")),
            z_layer: ZLayer::DropdownMenu,
            anchor_rect_px: picker_anchor,
            selected_value: String::from(crate::i18n::normalize_locale(&app.locale)),
            options: opts,
        });
    }
    Ok(UiEventOutcome { handled: true, action: format!("dropdown.opened:{dropdown_id}"), ..Default::default() })
}

fn handle_text_input(app: &mut UiApp, metrics: ViewportMetrics, text: &str) -> UiEventOutcome {
    if text.is_empty() {
        return UiEventOutcome { handled: false, action: String::from("text_input_ignored"), ..Default::default() };
    }
    let modal_open = app.canvas_dialogs.active_dialog != ActiveDialog::None;
    if !modal_open && app.inline_editor.active {
        app.inline_editor.insert_text(text);
        if matches!(app.inline_editor.target, Some(InlineEditorTarget::GridCell { .. } | InlineEditorTarget::GridXCell { .. })) {
            app.grid.tool.editing_buffer = Some(app.inline_editor.buffer.clone());
        }
        return UiEventOutcome { handled: true, action: String::from("inline_edit.text_input"), ..Default::default() };
    }
    if let Some(fid) = app.focus_widget_id.clone()
        && is_text_widget_id(&fid)
    {
        if modal_open {
            let tree = app.build_and_cache_layout_tree(metrics);
            let modal_textbox_focused =
                tree.node(&fid).is_some_and(|n| n.z_layer >= ZLayer::ModalDialog && n.a11y.role == WidgetRole::TextBox && n.state.enablement.enabled);
            if !modal_textbox_focused {
                return UiEventOutcome { handled: false, action: String::from("modal.text_input_blocked"), ..Default::default() };
            }
        }
        ensure_text_field_buffer(app, &fid, "");
        if let Some(buf) = app.interaction.text_field_buffers.get_mut(&fid) {
            buf.insert_text(text);
        }
        sync_focused_text_field(app, &fid);
        return UiEventOutcome { handled: true, action: format!("textbox.text_input:{fid}"), ..Default::default() };
    }
    if !modal_open
        && !app.nav.welcome.open
        && !app.nav.gallery.open
        && app.nav.active_pane == ActiveWorkspacePane::MainCanvasOrGrid
        && app.document().active_sheet_id.as_ref().is_some_and(|sid| app.document().data_sheet(sid).is_some())
    {
        let mut grid_core = std::mem::take(&mut app.view_shards.grid_core);
        let grid_out = crate::views::grid_core::handle_grid_text_input(app, &mut grid_core, text);
        app.view_shards.grid_core = grid_core;
        if grid_out.handled {
            return grid_out;
        }
    }
    UiEventOutcome { handled: false, action: String::from("text_input_ignored"), ..Default::default() }
}

fn is_text_widget_id(wid: &str) -> bool {
    matches!(
        wid,
        "palette.query"
            | "formula_bar.input"
            | "info.notes"
            | "info.secondary.notes"
            | "edit.transform.formula"
            | "edit.transform.filter_formula"
            | "edit.transform.selected_data_sets"
            | "edit.transform.baseline_columns"
            | "axes.y_thresholds"
            | "axes.x_thresholds"
            | "axes.title"
    ) || wid.contains("search")
        || wid.contains("input")
}

pub(crate) fn resolve_result_sheet_for_shortcut(app: &UiApp, allow_any_split_fallback: bool) -> Option<SheetId> {
    let doc = app.document();
    if let Some(active) = doc.active_sheet_id.as_ref().filter(|id| doc.result_sheet(id).is_some()) {
        return Some(active.clone());
    }
    if !app.split.enabled {
        return None;
    }
    let (prim, sec) = app.split.resolve_sheets(doc);
    let focused = app.focus_widget_id.as_deref().unwrap_or("");
    let sec_focused = focused.starts_with("secondary.") || focused.starts_with("result.sec.");
    let result_focused = focused.starts_with("result.") || sec_focused;
    if sec_focused {
        if let Some(id) = sec.as_ref().filter(|id| doc.result_sheet(id).is_some()) {
            return Some(id.clone());
        }
    } else if result_focused {
        if let Some(id) = prim.as_ref().filter(|id| doc.result_sheet(id).is_some()) {
            return Some(id.clone());
        }
        if let Some(id) = sec.as_ref().filter(|id| doc.result_sheet(id).is_some()) {
            return Some(id.clone());
        }
    }
    if allow_any_split_fallback {
        return sec.filter(|id| doc.result_sheet(id).is_some()).or_else(|| prim.filter(|id| doc.result_sheet(id).is_some()));
    }
    None
}

fn resolve_figure_sheet_for_shortcut(app: &UiApp) -> Option<SheetId> {
    let doc = app.document();
    let is_figure = |id: &SheetId| doc.graph_sheet(id).is_some() || doc.layout_sheet(id).is_some();
    if app.split.enabled {
        let (_prim, sec) = app.split.resolve_sheets(doc);
        if app.focus_widget_id.as_deref().is_some_and(|f| f.starts_with("secondary."))
            && let Some(id) = sec.filter(is_figure)
        {
            return Some(id);
        }
    }
    doc.active_sheet_id.as_ref().filter(|id| is_figure(id)).cloned()
}

fn copy_figure_sheet_bundle(app: &mut UiApp, sheet_id: &SheetId, metrics: ViewportMetrics) -> Result<UiEventOutcome, String> {
    let dpi = metrics.dpi_for_base(144);
    let bundle = crate::os_integration::ClipboardBundle::from_sheet(app.session(), sheet_id, dpi)?;
    let flavors = bundle.available_flavors();
    let pdf_bytes = bundle.pdf_bytes.len();
    let svg_bytes = bundle.svg.len();
    let png_bytes = bundle.png_bytes.len();
    let emf_bytes = bundle.emf_bytes.as_ref().map_or(0, Vec::len);
    let write_res = app.write_clipboard_bundle(&bundle);
    app.interaction.last_clipboard_text = (!bundle.svg.is_empty()).then(|| bundle.svg.clone());
    app.interaction.last_clipboard_bundle = Some(bundle);

    match write_res {
        Ok(status) => {
            let action = if status.available { "clipboard.copied_bundle" } else { "clipboard.copied_bundle_os_unavailable" };
            Ok(UiEventOutcome {
                handled: true,
                action: String::from(action),
                command_id: None,
                command_result: Some(json!({
                    "sheet_id": sheet_id.as_str(),
                    "flavors": flavors,
                    "pdf_bytes": pdf_bytes,
                    "svg_bytes": svg_bytes,
                    "png_bytes": png_bytes,
                    "emf_bytes": emf_bytes,
                    "os_clipboard": status,
                })),
            })
        }
        Err(err) => Ok(UiEventOutcome {
            handled: true,
            action: String::from("clipboard.copied_bundle_os_failed"),
            command_id: None,
            command_result: Some(json!({
                "sheet_id": sheet_id.as_str(),
                "flavors": flavors,
                "pdf_bytes": pdf_bytes,
                "svg_bytes": svg_bytes,
                "png_bytes": png_bytes,
                "emf_bytes": emf_bytes,
                "os_clipboard": { "available": false, "error": err },
            })),
        }),
    }
}

fn try_handle_workspace_copy_shortcut(app: &mut UiApp, metrics: ViewportMetrics) -> Result<Option<UiEventOutcome>, String> {
    if app.inline_editor.active {
        return Ok(None);
    }
    if let Some(rs_id) = resolve_result_sheet_for_shortcut(app, false) {
        let mut results_state = std::mem::take(&mut app.view_shards.results);
        let out = crate::views::results::handle_custom_action(app, &mut results_state, &format!("copy_selection:{}", rs_id.as_str()));
        app.view_shards.results = results_state;
        if let Ok(outcome) = out
            && outcome.handled
        {
            return Ok(Some(outcome));
        }
    }
    if !app.nav.welcome.open
        && !app.nav.gallery.open
        && let Some(fig_id) = resolve_figure_sheet_for_shortcut(app)
    {
        return copy_figure_sheet_bundle(app, &fig_id, metrics).map(Some);
    }
    Ok(None)
}

fn toggle_exclude_active_grid_cells(app: &mut UiApp) -> Result<Option<UiEventOutcome>, String> {
    let doc = app.document().clone();
    let Some(sid) = doc.active_sheet_id.as_ref() else {
        return Ok(None);
    };
    let Some(ds) = doc.data_sheet(sid).cloned() else {
        return Ok(None);
    };
    if ds.frozen {
        return Ok(None);
    }
    let coords = app.grid.all_selected_coords(&ds);
    if !coords.is_empty() {
        let all_excluded = coords.iter().all(|c| ds.cell(CellCoord::from(*c)).is_some_and(|cell| cell.excluded));
        let target_excluded = !all_excluded;
        app.grid.tool.pending_intents.clear();
        let _ = app.grid.emit_toggle_exclude_intents(&ds, target_excluded);
        let intents = std::mem::take(&mut app.grid.tool.pending_intents);
        for intent in intents {
            app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
        }
        app.grid.refresh_selection_stats(app.session.document());
        return Ok(Some(UiEventOutcome {
            handled: true,
            action: String::from("shortcut:grid.toggle_exclude"),
            command_id: Some(String::from("table.excludeCells")),
            command_result: Some(json!({ "sheet_id": ds.id.as_str(), "excluded": target_excluded, "count": coords.len() })),
        }));
    }
    if let Some((row, subcol)) = app.grid.active_x_cell {
        let cur_excl = ds.x_or_time_cell(row, subcol).is_some_and(|c| c.excluded);
        let next_excl = !cur_excl;
        app.execute_command(
            "table.excludeCells",
            json!({ "sheet_id": ds.id.as_str(), "target": "x", "excluded": next_excl, "row": row, "subcol": subcol }),
        )
        .map_err(|e| e.to_string())?;
        return Ok(Some(UiEventOutcome {
            handled: true,
            action: String::from("shortcut:grid.toggle_exclude_x"),
            command_id: Some(String::from("table.excludeCells")),
            command_result: Some(json!({ "sheet_id": ds.id.as_str(), "excluded": next_excl, "row": row, "subcol": subcol })),
        }));
    }
    if let Some(c) = app.grid.tool.active {
        let cur_excl = ds.cell(CellCoord::from(c)).is_some_and(|cell| cell.excluded);
        let next_excl = !cur_excl;
        let single = SheetCellRange::new(CellCoord::from(c), CellCoord::from(c));
        app.execute_command("table.excludeCells", json!({ "sheet_id": ds.id.as_str(), "excluded": next_excl, "range": single }))
            .map_err(|e| e.to_string())?;
        app.grid.refresh_selection_stats(app.session.document());
        return Ok(Some(UiEventOutcome {
            handled: true,
            action: String::from("shortcut:grid.toggle_exclude"),
            command_id: Some(String::from("table.excludeCells")),
            command_result: Some(json!({ "sheet_id": ds.id.as_str(), "excluded": next_excl })),
        }));
    }
    Ok(None)
}

fn handle_key_down(app: &mut UiApp, metrics: ViewportMetrics, key: &str, modifiers: KeyModifiers) -> Result<UiEventOutcome, String> {
    let trimmed = key.trim();
    let is_space = key == " " || trimmed.eq_ignore_ascii_case("space");
    let modal_open = app.canvas_dialogs.active_dialog != ActiveDialog::None;

    if !modal_open && app.inline_editor.active && !modifiers.meta && !modifiers.ctrl {
        return handle_inline_editor_key(app, metrics, trimmed, modifiers);
    }

    if !modal_open
        && app.grid.formula_bar.active
        && app.focus_widget_id.as_deref() == Some("formula_bar.input")
        && !modifiers.ctrl
        && !modifiers.meta
        && !modifiers.alt
    {
        if trimmed.eq_ignore_ascii_case("tab") {
            let doc = app.document().clone();
            let accepted = app.grid.formula_bar.accept_completion(&doc);
            return Ok(UiEventOutcome {
                handled: true,
                action: String::from("formula_bar.tab_complete"),
                command_result: Some(json!({ "completion_accepted": accepted, "input": app.grid.formula_bar.input })),
                ..Default::default()
            });
        }
        if let Some(out) = handle_textbox_key(app, metrics, "formula_bar.input", key, modifiers)? {
            return Ok(out);
        }
    }

    if modal_open {
        return handle_modal_key_down(app, metrics, key, trimmed, is_space, modifiers);
    }

    if trimmed.eq_ignore_ascii_case("tab") && !modifiers.ctrl && !modifiers.meta && !modifiers.alt {
        if app.grid.formula_bar.active {
            let doc = app.document().clone();
            let accepted = app.grid.formula_bar.accept_completion(&doc);
            return Ok(UiEventOutcome {
                handled: true,
                action: String::from("formula_bar.tab_complete"),
                command_result: Some(json!({ "completion_accepted": accepted, "input": app.grid.formula_bar.input })),
                ..Default::default()
            });
        }
        let tree = app.build_and_cache_layout_tree(metrics);
        if let Some(next_id) = cycle_focus_in_scope(&tree.nodes, tree.active_focus_scope, app.focus_widget_id.as_deref(), modifiers.shift) {
            app.focus_widget_id = Some(next_id.clone());
            if let Some(node) = tree.node(&next_id)
                && node.a11y.role == WidgetRole::TextBox
            {
                ensure_text_field_buffer(app, &next_id, node.a11y.value_text.as_deref().unwrap_or(""));
            }
            return Ok(UiEventOutcome { handled: true, action: format!("focus.cycle:{next_id}"), ..Default::default() });
        }
    }

    // `Cmd/Ctrl+T` is context-scoped (`ui.editParams.open` vs `sheet.createData`) via `ShortcutRegistry` (`RV-75`).
    let text_focused = app.focus_widget_id.as_deref().is_some_and(is_text_widget_id);
    if !text_focused
        && (modifiers.meta || modifiers.ctrl)
        && !modifiers.shift
        && !modifiers.alt
        && trimmed.eq_ignore_ascii_case("e")
        && let Some(out) = toggle_exclude_active_grid_cells(app)?
    {
        return Ok(out);
    }

    if !text_focused
        && (modifiers.meta || modifiers.ctrl)
        && !modifiers.shift
        && !modifiers.alt
        && trimmed.eq_ignore_ascii_case("c")
        && let Some(out) = try_handle_workspace_copy_shortcut(app, metrics)?
    {
        return Ok(out);
    }

    if let Some(fid) = app.focus_widget_id.clone()
        && is_text_widget_id(&fid)
        && let Some(out) = handle_textbox_key(app, metrics, &fid, key, modifiers)?
    {
        return Ok(out);
    }

    if matches!(trimmed, "Escape" | "Esc") {
        clear_context_menu_state(app);
        app.view_shards.toolbar_menus.overflow_open = false;
        if app.interaction.open_dropdown.take().is_some() || app.interaction.open_menu.take().is_some() {
            return Ok(UiEventOutcome { handled: true, action: String::from("popup.dismissed"), ..Default::default() });
        }
        if app.nav.welcome.open {
            app.nav.welcome.open = false;
            return Ok(UiEventOutcome { handled: true, action: String::from("welcome.dismissed"), ..Default::default() });
        }
    }

    let activate_focused_workspace = !modifiers.ctrl
        && !modifiers.meta
        && !modifiers.alt
        && (is_space
            || (matches!(trimmed, "Enter" | "Return")
                && app.view_shards.grid_core.editing_title.is_none()
                && app.focus_widget_id.as_deref().is_some_and(|f| !f.starts_with("grid.") && !f.starts_with("secondary.grid."))));
    if activate_focused_workspace
        && let Some(fid) = app.focus_widget_id.clone()
        && let Some(node) = app.build_and_cache_layout_tree(metrics).node(&fid).cloned()
        && node.a11y.role != WidgetRole::TextBox
        && node.state.enablement.enabled
        && node.action != WidgetAction::NoOp
    {
        return execute_widget_action(app, &node.action, metrics);
    }

    let editing_grid_title = app.view_shards.grid_core.editing_title.is_some();
    let grid_nav_key =
        matches!(trimmed, "ArrowUp" | "Up" | "ArrowDown" | "Down" | "ArrowLeft" | "Left" | "ArrowRight" | "Right" | "Delete" | "Backspace")
            || (matches!(trimmed, "Enter" | "Return")
                && (editing_grid_title
                    || (app.nav.active_pane == ActiveWorkspacePane::MainCanvasOrGrid
                        && app.focus_widget_id.as_deref().is_none_or(|f| f.starts_with("grid.") || f.starts_with("secondary.grid.")))));
    if grid_nav_key
        && !modifiers.alt
        && !app.nav.welcome.open
        && !app.nav.gallery.open
        && app.interaction.open_menu.is_none()
        && app.interaction.open_dropdown.is_none()
        && (editing_grid_title
            || (!text_focused && app.document().active_sheet_id.as_ref().is_some_and(|sid| app.document().data_sheet(sid).is_some())))
    {
        let mut grid_core = std::mem::take(&mut app.view_shards.grid_core);
        let grid_out = crate::views::grid_core::handle_grid_key_down(app, &mut grid_core, trimmed, modifiers);
        app.view_shards.grid_core = grid_core;
        let outcome = grid_out?;
        if outcome.handled {
            return Ok(outcome);
        }
    }

    for candidate in build_shortcut_candidates(trimmed, modifiers) {
        let on_result_sheet = resolve_result_sheet_for_shortcut(app, true).is_some();
        if let Ok(Some(exec)) = app.handle_key_combo_for_context(&candidate, on_result_sheet) {
            if matches!(exec.command_id.as_str(), "edit.undo" | "edit.redo" | "sheet.select") {
                clear_inspector_axis_buffers(app);
            }
            return Ok(UiEventOutcome {
                handled: true,
                action: format!("shortcut:{}", exec.command_id),
                command_id: Some(exec.command_id),
                command_result: Some(exec.result),
            });
        }
    }

    Ok(UiEventOutcome { handled: false, action: String::from("key_down_unhandled"), ..Default::default() })
}

fn handle_modal_key_down(
    app: &mut UiApp,
    metrics: ViewportMetrics,
    raw_key: &str,
    trimmed: &str,
    is_space: bool,
    modifiers: KeyModifiers,
) -> Result<UiEventOutcome, String> {
    if matches!(trimmed, "Escape" | "Esc") {
        clear_context_menu_state(app);
        if app.interaction.open_dropdown.take().is_some() || app.interaction.open_menu.take().is_some() {
            return Ok(UiEventOutcome { handled: true, action: String::from("popup.dismissed"), ..Default::default() });
        }
        close_active_modal(app);
        return Ok(UiEventOutcome { handled: true, action: String::from("dialog.cancelled"), ..Default::default() });
    }

    if trimmed.eq_ignore_ascii_case("tab") && !modifiers.ctrl && !modifiers.meta && !modifiers.alt {
        let tree = app.build_and_cache_layout_tree(metrics);
        if let Some(next_id) = cycle_focus_in_scope(&tree.nodes, tree.active_focus_scope, app.focus_widget_id.as_deref(), modifiers.shift) {
            app.focus_widget_id = Some(next_id.clone());
            if let Some(node) = tree.node(&next_id)
                && node.a11y.role == WidgetRole::TextBox
            {
                ensure_text_field_buffer(app, &next_id, node.a11y.value_text.as_deref().unwrap_or(""));
            }
            return Ok(UiEventOutcome { handled: true, action: format!("focus.cycle:{next_id}"), ..Default::default() });
        }
        return Ok(UiEventOutcome { handled: true, action: String::from("modal.tab_contained"), ..Default::default() });
    }

    if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::CommandPalette)
        && !modifiers.ctrl
        && !modifiers.meta
        && !modifiers.alt
        && matches!(trimmed, "ArrowUp" | "Up" | "ArrowDown" | "Down")
    {
        let count = app.canvas_dialogs.command_palette.matches_for_session(app.session()).len();
        if count > 0 {
            let cur = app.canvas_dialogs.command_palette.selected_index.min(count - 1);
            app.canvas_dialogs.command_palette.selected_index =
                if matches!(trimmed, "ArrowUp" | "Up") { if cur == 0 { count - 1 } else { cur - 1 } } else { (cur + 1) % count };
        }
        return Ok(UiEventOutcome { handled: true, action: String::from("palette.navigate"), ..Default::default() });
    }

    if let Some(fid) = app.focus_widget_id.clone()
        && is_text_widget_id(&fid)
    {
        let tree = app.build_and_cache_layout_tree(metrics);
        let modal_textbox_focused =
            tree.node(&fid).is_some_and(|n| n.z_layer >= ZLayer::ModalDialog && n.a11y.role == WidgetRole::TextBox && n.state.enablement.enabled);
        if modal_textbox_focused && let Some(out) = handle_textbox_key(app, metrics, &fid, raw_key, modifiers)? {
            return Ok(out);
        }
    }

    if !modifiers.ctrl && !modifiers.meta && !modifiers.alt && matches!(trimmed, "Enter" | "Return") {
        let res = crate::gui_actions::submit_active_dialog_ok(app).map_err(|e| e.to_string())?;
        if app.canvas_dialogs.active_dialog == ActiveDialog::None {
            clear_context_menu_state(app);
        }
        return Ok(UiEventOutcome { handled: true, action: String::from("dialog.confirmed"), command_result: Some(res), ..Default::default() });
    }

    if is_space
        && !modifiers.ctrl
        && !modifiers.meta
        && !modifiers.alt
        && let Some(fid) = app.focus_widget_id.clone()
        && let Some(node) = app.build_and_cache_layout_tree(metrics).node(&fid).cloned()
        && node.z_layer >= ZLayer::ModalDialog
        && node.state.enablement.enabled
        && node.action != WidgetAction::NoOp
    {
        return execute_widget_action(app, &node.action, metrics);
    }

    Ok(UiEventOutcome { handled: false, action: String::from("modal.key_blocked"), ..Default::default() })
}

fn handle_inline_editor_key(app: &mut UiApp, metrics: ViewportMetrics, key: &str, modifiers: KeyModifiers) -> Result<UiEventOutcome, String> {
    match key {
        "Enter" | "Return" => commit_inline_edit(app),
        "Escape" | "Esc" => Ok(cancel_inline_edit(app)),
        "Backspace" => {
            app.inline_editor.backspace();
            if matches!(app.inline_editor.target, Some(InlineEditorTarget::GridCell { .. } | InlineEditorTarget::GridXCell { .. })) {
                app.grid.tool.editing_buffer = Some(app.inline_editor.buffer.clone());
            }
            Ok(UiEventOutcome { handled: true, action: String::from("inline_edit.backspace"), ..Default::default() })
        }
        _ if !modifiers.ctrl && !modifiers.meta && !modifiers.alt && key.chars().count() == 1 => Ok(handle_text_input(app, metrics, key)),
        _ => Ok(UiEventOutcome { handled: false, action: String::from("key_down_unhandled"), ..Default::default() }),
    }
}

fn handle_textbox_key(
    app: &mut UiApp,
    metrics: ViewportMetrics,
    fid: &str,
    key: &str,
    modifiers: KeyModifiers,
) -> Result<Option<UiEventOutcome>, String> {
    if matches!(fid, "formula_bar.input" | "mv_formula.name.input") && !modifiers.ctrl && !modifiers.meta && !modifiers.alt {
        if matches!(key, "Enter" | "Return") {
            if app.view_shards.mv_formula.right_panel_open {
                return crate::views::mv_formula::commit_from_textbox_enter(app).map(Some);
            }
            if fid == "formula_bar.input" {
                let res = crate::gui_actions::commit_formula_bar(app).map_err(|e| e.to_string())?;
                return Ok(Some(UiEventOutcome {
                    handled: true,
                    action: String::from("formula_bar.committed"),
                    command_result: Some(res),
                    ..Default::default()
                }));
            }
        }
        if fid == "formula_bar.input" && matches!(key, "Escape" | "Esc") {
            crate::gui_actions::cancel_formula_bar(app);
            return Ok(Some(UiEventOutcome { handled: true, action: String::from("formula_bar.cancelled"), ..Default::default() }));
        }
    }
    ensure_text_field_buffer(app, fid, "");
    if (modifiers.ctrl || modifiers.meta) && key.eq_ignore_ascii_case("a") {
        if let Some(b) = app.interaction.text_field_buffers.get_mut(fid) {
            b.select_all();
        }
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("textbox.select_all"), ..Default::default() }));
    }
    if (modifiers.ctrl || modifiers.meta) && key.eq_ignore_ascii_case("c") {
        if let Some(sel) = app.interaction.text_field_buffers.get(fid).and_then(TextFieldEditState::copy_selection) {
            app.interaction.last_clipboard_text = Some(sel);
        }
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("textbox.copy"), ..Default::default() }));
    }
    if (modifiers.ctrl || modifiers.meta) && key.eq_ignore_ascii_case("v") {
        if let Some(clip) = app.interaction.last_clipboard_text.clone() {
            if let Some(b) = app.interaction.text_field_buffers.get_mut(fid) {
                b.insert_text(&clip);
            }
            sync_focused_text_field(app, fid);
        }
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("textbox.paste"), ..Default::default() }));
    }
    let handled = match key {
        "Backspace" => {
            if let Some(b) = app.interaction.text_field_buffers.get_mut(fid) {
                b.backspace();
            }
            sync_focused_text_field(app, fid);
            true
        }
        "Delete" => {
            if let Some(b) = app.interaction.text_field_buffers.get_mut(fid) {
                b.delete_forward();
            }
            sync_focused_text_field(app, fid);
            true
        }
        "ArrowLeft" | "Left" => {
            if let Some(b) = app.interaction.text_field_buffers.get_mut(fid) {
                b.move_left(modifiers.shift);
            }
            true
        }
        "ArrowRight" | "Right" => {
            if let Some(b) = app.interaction.text_field_buffers.get_mut(fid) {
                b.move_right(modifiers.shift);
            }
            true
        }
        _ if !modifiers.ctrl && !modifiers.meta && !modifiers.alt && key.chars().count() == 1 => {
            return Ok(Some(handle_text_input(app, metrics, key)));
        }
        _ => false,
    };
    Ok(handled.then(|| UiEventOutcome { handled: true, action: format!("textbox.key:{key}"), ..Default::default() }))
}

fn build_shortcut_candidates(key: &str, modifiers: KeyModifiers) -> Vec<String> {
    if key.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(2);
    let build = |use_cmd_for_ctrl: bool| -> String {
        let mut parts = Vec::with_capacity(5);
        if modifiers.meta || (use_cmd_for_ctrl && modifiers.ctrl) {
            parts.push("Cmd");
        }
        if modifiers.ctrl && !(use_cmd_for_ctrl && !modifiers.meta) {
            parts.push("Ctrl");
        }
        if modifiers.alt {
            parts.push("Alt");
        }
        if modifiers.shift {
            parts.push("Shift");
        }
        parts.push(key);
        parts.join("+")
    };
    out.push(build(false));
    if modifiers.ctrl && !modifiers.meta {
        out.push(build(true));
    }
    out
}

pub(crate) fn commit_inline_edit_if_active(app: &mut UiApp) -> Result<bool, String> {
    if !app.inline_editor.active {
        return Ok(false);
    }
    let _ = commit_inline_edit(app)?;
    Ok(true)
}

fn commit_inline_edit(app: &mut UiApp) -> Result<UiEventOutcome, String> {
    let mut editor = std::mem::take(&mut app.inline_editor);
    let intent = editor.take_commit_intent(app);
    app.inline_editor = editor;
    let Some(intent) = intent else {
        return Ok(UiEventOutcome { handled: false, action: String::from("inline_edit_not_active"), ..Default::default() });
    };
    app.grid.tool.editing_buffer = None;
    let res = execute_engine_value(app, &intent.command_id, intent.params)?;
    app.grid.refresh_selection_stats(app.session.document());
    Ok(UiEventOutcome {
        handled: true,
        action: String::from("inline_edit.committed"),
        command_id: Some(intent.command_id),
        command_result: Some(res),
    })
}

fn cancel_inline_edit(app: &mut UiApp) -> UiEventOutcome {
    if !app.inline_editor.active {
        return UiEventOutcome { handled: false, action: String::from("inline_edit_not_active"), ..Default::default() };
    }
    app.inline_editor.cancel();
    app.grid.tool.editing_buffer = None;
    UiEventOutcome { handled: true, action: String::from("inline_edit.cancelled"), ..Default::default() }
}

fn layout_item_bounds(item: &LayoutItem) -> RectPt {
    match item {
        LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => *bounds_pt,
    }
}

const MIN_LAYOUT_PANEL_DIM_PT: f64 = 18.0;

fn raw_layout_candidate(initial: RectPt, dx: f64, dy: f64, resize_handle: Option<LayoutResizeHandle>) -> RectPt {
    let Some(handle) = resize_handle else {
        return RectPt::new(initial.x + dx, initial.y + dy, initial.width, initial.height);
    };
    let mut left = initial.x;
    let mut right = initial.x + initial.width;
    let mut top = initial.y;
    let mut bottom = initial.y + initial.height;
    if matches!(handle, LayoutResizeHandle::TopLeft | LayoutResizeHandle::Left | LayoutResizeHandle::BottomLeft) {
        left = (initial.x + dx).min(right - MIN_LAYOUT_PANEL_DIM_PT);
    }
    if matches!(handle, LayoutResizeHandle::TopRight | LayoutResizeHandle::Right | LayoutResizeHandle::BottomRight) {
        right = (initial.x + initial.width + dx).max(left + MIN_LAYOUT_PANEL_DIM_PT);
    }
    if matches!(handle, LayoutResizeHandle::TopLeft | LayoutResizeHandle::Top | LayoutResizeHandle::TopRight) {
        top = (initial.y + dy).min(bottom - MIN_LAYOUT_PANEL_DIM_PT);
    }
    if matches!(handle, LayoutResizeHandle::BottomLeft | LayoutResizeHandle::Bottom | LayoutResizeHandle::BottomRight) {
        bottom = (initial.y + initial.height + dy).max(top + MIN_LAYOUT_PANEL_DIM_PT);
    }
    RectPt::new(left, top, (right - left).max(MIN_LAYOUT_PANEL_DIM_PT), (bottom - top).max(MIN_LAYOUT_PANEL_DIM_PT))
}

fn compute_layout_plot_origins(app: &UiApp, spec: &LayoutSpec) -> Vec<(usize, PointPt)> {
    let mut origins = Vec::new();
    for (idx, item) in spec.items.iter().enumerate() {
        let LayoutItem::LiveGraph { graph_sheet_id, bounds_pt, scale_percent, .. } = item else {
            continue;
        };
        let Ok(sub) = app.session().render_sheet_scene(graph_sheet_id) else {
            continue;
        };
        let pct = if scale_percent.is_finite() && *scale_percent > 0.0 { *scale_percent / 100.0 } else { 1.0 };
        let fit = (bounds_pt.width / sub.width_pt.max(1.0)).min(bounds_pt.height / sub.height_pt.max(1.0));
        let scale = (fit * pct).max(0.05);
        origins.push((idx, PointPt::new(sub.plot_rect_pt.x * scale, sub.plot_rect_pt.y * scale)));
    }
    origins
}

fn compute_live_layout_drag_snap(
    canvas: &curvebook_tools::CanvasToolState,
    spec: &LayoutSpec,
    moving_index: usize,
    initial: RectPt,
    delta_pt: (f64, f64),
    resize_handle: Option<LayoutResizeHandle>,
    plot_origin_offsets: &[(usize, PointPt)],
) -> (RectPt, Vec<SnapGuide>) {
    let candidate = raw_layout_candidate(initial, delta_pt.0, delta_pt.1, resize_handle);
    let sibling_plot_origins: Vec<PointPt> = plot_origin_offsets
        .iter()
        .filter(|(idx, _)| *idx != moving_index)
        .filter_map(|(idx, off)| {
            spec.items.get(*idx).map(|item| {
                let b = layout_item_bounds(item);
                PointPt::new(b.x + off.x, b.y + off.y)
            })
        })
        .collect();

    if let Some(handle) = resize_handle {
        return snap_layout_resize(canvas, spec, moving_index, candidate, handle, &sibling_plot_origins);
    }

    let snap = canvas.compute_layout_snap(spec, Some(moving_index), candidate.into(), &sibling_plot_origins);
    let mut snapped: RectPt = snap.snapped_bounds_pt.into();
    let mut guides = snap.guides;

    if spec.align_plot_origins
        && let Some((_, own_off)) = plot_origin_offsets.iter().find(|(idx, _)| *idx == moving_index)
    {
        let page = spec.page_size_pt();
        let tol = canvas.snap_tolerance_pt.max(0.0);
        apply_plot_origin_axis_snap(
            PlotOriginAxis {
                orientation: SnapOrientation::Vertical,
                candidate_coord: candidate.x,
                own_off: own_off.x,
                cross_span_pt: page.height,
                tol,
            },
            &sibling_plot_origins,
            &mut snapped.x,
            &mut guides,
        );
        apply_plot_origin_axis_snap(
            PlotOriginAxis {
                orientation: SnapOrientation::Horizontal,
                candidate_coord: candidate.y,
                own_off: own_off.y,
                cross_span_pt: page.width,
                tol,
            },
            &sibling_plot_origins,
            &mut snapped.y,
            &mut guides,
        );
    }

    (snapped, guides)
}

struct PlotOriginAxis {
    orientation: SnapOrientation,
    candidate_coord: f64,
    own_off: f64,
    cross_span_pt: f64,
    tol: f64,
}

fn apply_plot_origin_axis_snap(axis: PlotOriginAxis, sibling_plot_origins: &[PointPt], snapped_coord: &mut f64, guides: &mut Vec<SnapGuide>) {
    let vertical = axis.orientation == SnapOrientation::Vertical;
    let Some((target, delta)) = nearest_plot_origin_coord(axis.candidate_coord + axis.own_off, sibling_plot_origins, vertical, axis.tol) else {
        return;
    };
    let cur_delta = (*snapped_coord - axis.candidate_coord).abs();
    let has_semantic_guide = guides.iter().any(|g| g.orientation == axis.orientation && g.source != SnapSource::Grid);
    let coincident = ((*snapped_coord + axis.own_off) - target).abs() <= 0.01;
    if !has_semantic_guide || delta.abs() < cur_delta {
        *snapped_coord = target - axis.own_off;
        guides.retain(|g| g.orientation != axis.orientation);
        guides.push(SnapGuide {
            orientation: axis.orientation,
            position_pt: target,
            span_start_pt: 0.0,
            span_end_pt: axis.cross_span_pt,
            source: SnapSource::PlotOrigin,
        });
    } else if coincident {
        guides.push(SnapGuide {
            orientation: axis.orientation,
            position_pt: target,
            span_start_pt: 0.0,
            span_end_pt: axis.cross_span_pt,
            source: SnapSource::PlotOrigin,
        });
    }
}

fn nearest_plot_origin_coord(coord: f64, origins: &[PointPt], vertical: bool, tol: f64) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    for origin in origins {
        let target = if vertical { origin.x } else { origin.y };
        let delta = target - coord;
        if delta.abs() <= tol && best.is_none_or(|(_, prev)| delta.abs() < prev.abs()) {
            best = Some((target, delta));
        }
    }
    best
}

fn snap_layout_resize(
    canvas: &curvebook_tools::CanvasToolState,
    spec: &LayoutSpec,
    moving_index: usize,
    candidate: RectPt,
    handle: LayoutResizeHandle,
    sibling_plot_origins: &[PointPt],
) -> (RectPt, Vec<SnapGuide>) {
    let mut left = candidate.x;
    let mut right = candidate.x + candidate.width;
    let mut top = candidate.y;
    let mut bottom = candidate.y + candidate.height;
    let tol = canvas.snap_tolerance_pt.max(0.0);
    let mut guides = Vec::new();
    let page = spec.page_size_pt();

    let (x_targets, y_targets) = collect_resize_targets(spec, moving_index, sibling_plot_origins, canvas.snap_to_grid, canvas.snap_grid_pt);
    if matches!(handle, LayoutResizeHandle::TopLeft | LayoutResizeHandle::Left | LayoutResizeHandle::BottomLeft)
        && let Some((pos, src)) = best_snap_target(left, &x_targets, tol)
        && right - pos >= MIN_LAYOUT_PANEL_DIM_PT
    {
        left = pos;
        guides.push(SnapGuide {
            orientation: SnapOrientation::Vertical,
            position_pt: pos,
            span_start_pt: 0.0,
            span_end_pt: page.height,
            source: src,
        });
    }
    if matches!(handle, LayoutResizeHandle::TopRight | LayoutResizeHandle::Right | LayoutResizeHandle::BottomRight)
        && let Some((pos, src)) = best_snap_target(right, &x_targets, tol)
        && pos - left >= MIN_LAYOUT_PANEL_DIM_PT
    {
        right = pos;
        guides.push(SnapGuide {
            orientation: SnapOrientation::Vertical,
            position_pt: pos,
            span_start_pt: 0.0,
            span_end_pt: page.height,
            source: src,
        });
    }
    if matches!(handle, LayoutResizeHandle::TopLeft | LayoutResizeHandle::Top | LayoutResizeHandle::TopRight)
        && let Some((pos, src)) = best_snap_target(top, &y_targets, tol)
        && bottom - pos >= MIN_LAYOUT_PANEL_DIM_PT
    {
        top = pos;
        guides.push(SnapGuide {
            orientation: SnapOrientation::Horizontal,
            position_pt: pos,
            span_start_pt: 0.0,
            span_end_pt: page.width,
            source: src,
        });
    }
    if matches!(handle, LayoutResizeHandle::BottomLeft | LayoutResizeHandle::Bottom | LayoutResizeHandle::BottomRight)
        && let Some((pos, src)) = best_snap_target(bottom, &y_targets, tol)
        && pos - top >= MIN_LAYOUT_PANEL_DIM_PT
    {
        bottom = pos;
        guides.push(SnapGuide {
            orientation: SnapOrientation::Horizontal,
            position_pt: pos,
            span_start_pt: 0.0,
            span_end_pt: page.width,
            source: src,
        });
    }

    (RectPt::new(left, top, right - left, bottom - top), guides)
}

type SnapTargets = Vec<(f64, SnapSource)>;

fn collect_resize_targets(
    spec: &LayoutSpec,
    moving_index: usize,
    sibling_plot_origins: &[PointPt],
    snap_to_grid: bool,
    grid_pt: f64,
) -> (SnapTargets, SnapTargets) {
    let page = spec.page_size_pt();
    let mut xs = vec![(0.0, SnapSource::PageEdge), (page.width * 0.5, SnapSource::PageCenter), (page.width, SnapSource::PageEdge)];
    let mut ys = vec![(0.0, SnapSource::PageEdge), (page.height * 0.5, SnapSource::PageCenter), (page.height, SnapSource::PageEdge)];
    for (idx, item) in spec.items.iter().enumerate() {
        if idx == moving_index {
            continue;
        }
        let b = layout_item_bounds(item);
        xs.extend([(b.x, SnapSource::SiblingEdge), (b.x + b.width * 0.5, SnapSource::SiblingCenter), (b.x + b.width, SnapSource::SiblingEdge)]);
        ys.extend([(b.y, SnapSource::SiblingEdge), (b.y + b.height * 0.5, SnapSource::SiblingCenter), (b.y + b.height, SnapSource::SiblingEdge)]);
    }
    if spec.align_plot_origins {
        for origin in sibling_plot_origins {
            if origin.x.is_finite() {
                xs.push((origin.x, SnapSource::PlotOrigin));
            }
            if origin.y.is_finite() {
                ys.push((origin.y, SnapSource::PlotOrigin));
            }
        }
    }
    if snap_to_grid && grid_pt.is_finite() && grid_pt > 0.0 {
        let cols = (page.width / grid_pt).floor() as usize;
        for c in 0..=cols.min(256) {
            xs.push((c as f64 * grid_pt, SnapSource::Grid));
        }
        let rows = (page.height / grid_pt).floor() as usize;
        for r in 0..=rows.min(256) {
            ys.push((r as f64 * grid_pt, SnapSource::Grid));
        }
    }
    (xs, ys)
}

fn best_snap_target(edge: f64, targets: &[(f64, SnapSource)], tol: f64) -> Option<(f64, SnapSource)> {
    let mut best: Option<(f64, SnapSource, f64)> = None;
    for &(pos, src) in targets {
        let dist = (pos - edge).abs();
        if dist <= tol && best.is_none_or(|(_, _, prev)| dist < prev) {
            best = Some((pos, src, dist));
        }
    }
    best.map(|(pos, src, _)| (pos, src))
}

fn set_layout_item_bounds(item: &mut LayoutItem, bounds: RectPt) {
    match item {
        LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => {
            *bounds_pt = bounds;
        }
    }
}

fn graph_annotation_bounds(ann: &GraphAnnotation) -> Option<RectPt> {
    match ann {
        GraphAnnotation::TextBox { bounds_pt, .. }
        | GraphAnnotation::Shape { bounds_pt, .. }
        | GraphAnnotation::EmbeddedResultTable { bounds_pt, .. }
        | GraphAnnotation::ModelEquation { bounds_pt, .. } => Some(*bounds_pt),
        GraphAnnotation::LineOrArrow { start_pt, end_pt, .. } => {
            let x = start_pt.0.min(end_pt.0);
            let y = start_pt.1.min(end_pt.1);
            Some(RectPt::new(x, y, (end_pt.0 - start_pt.0).abs(), (end_pt.1 - start_pt.1).abs()))
        }
    }
}

fn set_graph_annotation_bounds(ann: &mut GraphAnnotation, bounds: RectPt) {
    match ann {
        GraphAnnotation::TextBox { bounds_pt, .. }
        | GraphAnnotation::Shape { bounds_pt, .. }
        | GraphAnnotation::EmbeddedResultTable { bounds_pt, .. }
        | GraphAnnotation::ModelEquation { bounds_pt, .. } => *bounds_pt = bounds,
        GraphAnnotation::LineOrArrow { start_pt, end_pt, .. } => {
            let reverse_x = start_pt.0 > end_pt.0;
            let reverse_y = start_pt.1 > end_pt.1;
            let x0 = if reverse_x { bounds.x + bounds.width } else { bounds.x };
            let x1 = if reverse_x { bounds.x } else { bounds.x + bounds.width };
            let y0 = if reverse_y { bounds.y + bounds.height } else { bounds.y };
            let y1 = if reverse_y { bounds.y } else { bounds.y + bounds.height };
            *start_pt = (x0, y0);
            *end_pt = (x1, y1);
        }
    }
}

fn snap_to_canvas_grid(x: f64, y: f64, snap_enabled: bool, grid_pt: f64) -> (f64, f64, Vec<SnapGuide>) {
    if !snap_enabled || !grid_pt.is_finite() || grid_pt <= 0.0 {
        return (x, y, Vec::new());
    }
    let sx = (x / grid_pt).round() * grid_pt;
    let sy = (y / grid_pt).round() * grid_pt;
    let guides = vec![
        SnapGuide { orientation: SnapOrientation::Vertical, position_pt: sx, span_start_pt: 0.0, span_end_pt: 1000.0, source: SnapSource::Grid },
        SnapGuide { orientation: SnapOrientation::Horizontal, position_pt: sy, span_start_pt: 0.0, span_end_pt: 1000.0, source: SnapSource::Grid },
    ];
    (sx, sy, guides)
}

#[allow(dead_code)]
fn _grid_hit_coord_unused(_: &GridViewState, sheet: &DataSheet, flat_col: usize, row: usize) -> Option<GridCellCoord> {
    resolve_flat_coord(&sheet.archetype, flat_col).map(|(col, subcol)| GridCellCoord::new(row, col, subcol))
}
