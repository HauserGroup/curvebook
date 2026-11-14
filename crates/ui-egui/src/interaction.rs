//! HiDPI viewport coordinate transforms, interactive pointer/keyboard event dispatch,
//! inline editing for grid cells (`=` formulas and localized numbers) / `InfoSheet` / titles,
//! and in-memory WASM-clean [`VirtualWorkspaceFolder`] (`G6`, `M7`, `QOL-4`, `QOL-5`, `QOL-10`).

use std::collections::BTreeMap;

use lumencraft_chart::PointPt;
use lumencraft_doc::{
    CellCoord, CellValue, ConstantValue, DataSheet, GraphAnnotation, LayoutItem, LayoutSpec, ProjectDocument, RectPt, SheetFolderKind, SheetId,
    TableArchetype, XColumnMode,
};
use lumencraft_format::{export_datasheet_to_csv, open_project_bytes, save_project_bytes};
use lumencraft_render::RenderTargetSpec;
use lumencraft_tools::canvas::{CanvasHitTarget, ResizeHandleCorner, SnapGuide, SnapOrientation, SnapSource};
use lumencraft_tools::{CanvasToolMode, GridCellCoord, ToolCommandIntent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::compositor::{WorkspaceFrameLayout, WorkspacePixelRect, compute_workspace_layout_map};
use crate::dialogs_canvas::ActiveDialog;
use crate::grid_view::{GridViewState, format_cell_display, parse_localized_number_input, resolve_flat_coord};
use crate::workspace_nav::{ActiveWorkspacePane, build_set_info_constant_command, build_set_info_notes_command};

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
        let root_name = if raw.trim().is_empty() { String::from("workspace.lumencraft.d") } else { raw };
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

    /// Serialize `doc` as a `.lumencraft` archive at `file_name` inside this virtual workspace.
    ///
    /// # Errors
    /// Returns an error if archive serialization or path validation fails.
    pub fn save_project_archive(&mut self, file_name: &str, doc: &ProjectDocument) -> Result<usize, String> {
        let bytes = save_project_bytes(doc).map_err(|err| err.to_string())?;
        let len = bytes.len();
        self.write_file(file_name, bytes)?;
        Ok(len)
    }

    /// Deserialize a `.lumencraft` archive stored at `file_name` inside this virtual workspace.
    ///
    /// # Errors
    /// Returns an error if `file_name` is missing or archive decoding fails.
    pub fn open_project_archive(&self, file_name: &str) -> Result<ProjectDocument, String> {
        let bytes = self.read_file(file_name).ok_or_else(|| format!("virtual file `{file_name}` not found"))?;
        open_project_bytes(bytes).map_err(|err| err.to_string())
    }

    /// Write an unpacked `.lumencraft.d`-compatible virtual snapshot (`document.json`, `manifest.json`,
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
            let csv_text = export_datasheet_to_csv(ds, &lumencraft_format::CsvOptions::csv()).map_err(|err| err.to_string())?;
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
        serde_json::from_slice(bytes).map_err(|err| format!("failed decoding virtual `document.json`: {err}"))
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
        let s_off = u32::try_from(subcol).ok()?;
        let cy = self.body_y.saturating_add(r_off.saturating_mul(self.row_h));
        if cy >= self.sheet_rect.bottom() {
            return None;
        }
        let cw = self.x_col_w.saturating_sub(1);
        let ch = self.row_h.saturating_sub(1);
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
        let c_off = u32::try_from(flat_col - self.start_col).ok()?;
        if usize::try_from(c_off).ok().is_none_or(|idx| idx >= self.visible_cols.len()) {
            return None;
        }
        let cy = self.body_y.saturating_add(r_off.saturating_mul(self.row_h));
        if cy >= self.sheet_rect.bottom() {
            return None;
        }
        let cw = self.col_w.saturating_sub(1);
        let ch = self.row_h.saturating_sub(1);
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
        for (c_off, (flat_col, base_coord)) in self.visible_cols.iter().enumerate() {
            let cw = self.col_w.saturating_sub(1);
            let local_raw = origin.saturating_add((c_off as u32).saturating_mul(self.col_w));
            let local_x = crate::i18n::mirror_x_for_rtl(local_raw, cw, self.sheet_rect.width, self.is_rtl);
            let cx = self.sheet_rect.x.saturating_add(local_x);
            if phys_x >= cx && phys_x < cx.saturating_add(cw) {
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

/// Ephemeral pointer drag tracking across frames (`G6`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct InteractionControllerState {
    pub pointer_down: bool,
    pub last_logical_pos: Option<[f64; 2]>,
    pub dragging_grid_sheet: Option<SheetId>,
    pub dragging_split_divider: bool,
    pub dragging_navigator_sheet: Option<SheetId>,
    pub canvas_drag: Option<CanvasDragTarget>,
    #[serde(default)]
    pub context_menu_target: Option<String>,
}

/// Structured outcome emitted by [`dispatch_ui_event`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UiEventOutcome {
    pub handled: bool,
    pub action: String,
    pub command_id: Option<String>,
    pub command_result: Option<Value>,
}

/// Dispatch an interactive [`UiInputEvent`] against `app` under `metrics` (`G6`, `M7`).
///
/// # Errors
/// Returns a descriptive error string if a triggered engine or UI command fails.
pub fn dispatch_ui_event(app: &mut UiApp, event: UiInputEvent, metrics: ViewportMetrics) -> Result<UiEventOutcome, String> {
    let outcome = match event {
        UiInputEvent::PointerDown { x, y, button, modifiers } => handle_pointer_down(app, metrics, [x, y], button, modifiers)?,
        UiInputEvent::PointerMove { x, y, .. } => handle_pointer_move(app, metrics, [x, y]),
        UiInputEvent::PointerUp { x, y, button, .. } => handle_pointer_up(app, metrics, [x, y], button)?,
        UiInputEvent::DoubleClick { x, y, button, .. } => handle_double_click(app, metrics, [x, y], button),
        UiInputEvent::WheelZoom { x, y, delta_y, .. } => handle_wheel_zoom(app, metrics, [x, y], delta_y),
        UiInputEvent::PinchZoom { x, y, scale_delta } => handle_pinch_zoom(app, metrics, [x, y], scale_delta),
        UiInputEvent::InertialPan { delta_x, delta_y, dt_sec, step_coast } => {
            handle_inertial_pan(app, metrics, [delta_x, delta_y], dt_sec, step_coast)
        }
        UiInputEvent::KeyDown { key, modifiers } => handle_key_down(app, &key, modifiers)?,
        UiInputEvent::TextInput { text } => handle_text_input(app, &text),
        UiInputEvent::CommitInlineEdit => commit_inline_edit(app)?,
        UiInputEvent::CancelInlineEdit => cancel_inline_edit(app),
    };
    app.sync_views();
    Ok(outcome)
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
        return Ok(handle_secondary_pointer_down(app, &layout, logical_pos));
    }
    if button != PointerButton::Primary {
        return Ok(UiEventOutcome { handled: false, action: String::from("pointer_down_ignored"), ..Default::default() });
    }
    app.interaction.context_menu_target = None;
    app.interaction.pointer_down = true;
    app.interaction.last_logical_pos = Some(logical_pos);
    let target = layout.hit_test_logical(logical_pos[0], logical_pos[1]);

    match target {
        WorkspaceHitTarget::SplitDivider => {
            app.interaction.dragging_split_divider = true;
            app.split.dragging_divider = true;
            Ok(UiEventOutcome { handled: true, action: String::from("split_divider.drag_start"), ..Default::default() })
        }
        WorkspaceHitTarget::NavigatorEntry { sheet_id, folder, index_in_folder } => {
            app.nav.active_pane = ActiveWorkspacePane::Navigator;
            let _ = app.execute_command("sheet.select", json!({ "sheet_id": sheet_id.as_str() }));
            app.nav.sync_from_document(app.session.document());
            app.canvas_dialogs.on_sheet_selected(app.session.document(), &sheet_id);
            app.nav.begin_sheet_drag(folder, sheet_id.as_str(), index_in_folder);
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
                crate::control_io::dispatch_ui_command(app, "ui.result.applyRemediation", &json!({ "result_sheet_id": result_sheet_id.as_str() }))?
                    .unwrap_or(Value::Null);
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("result.apply_remediation"),
                command_id: Some(String::from("ui.result.applyRemediation")),
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

fn handle_secondary_pointer_down(app: &mut UiApp, layout: &WorkspaceLayoutMap, logical_pos: [f64; 2]) -> UiEventOutcome {
    match layout.hit_test_logical(logical_pos[0], logical_pos[1]) {
        WorkspaceHitTarget::GridXCell { row, subcol, .. } => {
            app.grid.select_x_cell_sync(row, subcol);
            app.grid.refresh_selection_stats(app.session.document());
            app.interaction.context_menu_target = Some(String::from("grid"));
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
            app.interaction.context_menu_target = Some(String::from("grid"));
            UiEventOutcome {
                handled: true,
                action: String::from("grid.context_menu"),
                command_result: Some(json!({ "target": "grid", "selected_rows": app.grid.selected_rows })),
                ..Default::default()
            }
        }
        WorkspaceHitTarget::ScenePaper { .. } => {
            app.interaction.context_menu_target = Some(String::from("canvas"));
            UiEventOutcome {
                handled: true,
                action: String::from("canvas.context_menu"),
                command_result: Some(json!({ "target": "canvas", "brushed_count": app.brushed_points.len() })),
                ..Default::default()
            }
        }
        _ => UiEventOutcome { handled: false, action: String::from("secondary_click_ignored"), ..Default::default() },
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
    if let Ok(scene) = app.session().render_sheet_scene(sheet_id)
        && let Some(hit) = scene.hit_test_point(scene_pt, POINT_BRUSH_TOLERANCE_PT)
    {
        brushed = Some(hit.source_cell.clone());
    }
    if let Some(cell) = brushed {
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
        if let WorkspaceHitTarget::NavigatorEntry { index_in_folder, .. } = layout.hit_test_physical(phys_x, phys_y) {
            app.nav.update_sheet_hover(index_in_folder);
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

    if let WorkspaceHitTarget::GridCell { sheet_id, coord, .. } = layout.hit_test_physical(phys_x, phys_y)
        && let Some(ds) = app.session.document().data_sheet(&sheet_id)
    {
        app.grid.update_hover_exclusion_tooltip(ds, Some(coord));
        return UiEventOutcome { handled: true, action: String::from("grid.hover"), ..Default::default() };
    }

    UiEventOutcome { handled: false, action: String::from("pointer_move"), ..Default::default() }
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
        if let WorkspaceHitTarget::NavigatorEntry { index_in_folder, .. } = layout.hit_test_logical(logical_pos[0], logical_pos[1]) {
            app.nav.update_sheet_hover(index_in_folder);
        }
        if let Some((cmd_id, params)) = app.nav.commit_sheet_drop_command() {
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

    if let Some(drag_target) = app.interaction.canvas_drag.take() {
        return commit_canvas_object_drag(app, &layout, logical_pos, drag_target);
    }

    if let WorkspaceHitTarget::ScenePaper { sheet_id, scene_pt } = layout.hit_test_logical(logical_pos[0], logical_pos[1])
        && let Some(intent) = app.canvas_dialogs.canvas.end_drag(&sheet_id, scene_pt)
    {
        let res = execute_engine_value(app, &intent.command_id, intent.params)?;
        return Ok(UiEventOutcome {
            handled: true,
            action: String::from("canvas.tool_intent_committed"),
            command_id: Some(intent.command_id),
            command_result: Some(res),
        });
    }
    app.canvas_dialogs.canvas.drag_start_pt = None;
    Ok(UiEventOutcome { handled: true, action: String::from("pointer_up"), ..Default::default() })
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
    let initial_for = |cell: Option<&lumencraft_doc::Cell>| -> String {
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

fn handle_text_input(app: &mut UiApp, text: &str) -> UiEventOutcome {
    if !app.inline_editor.active || text.is_empty() {
        return UiEventOutcome { handled: false, action: String::from("text_input_ignored"), ..Default::default() };
    }
    app.inline_editor.insert_text(text);
    if matches!(app.inline_editor.target, Some(InlineEditorTarget::GridCell { .. } | InlineEditorTarget::GridXCell { .. })) {
        app.grid.tool.editing_buffer = Some(app.inline_editor.buffer.clone());
    }
    UiEventOutcome { handled: true, action: String::from("inline_edit.text_input"), ..Default::default() }
}

fn handle_key_down(app: &mut UiApp, key: &str, modifiers: KeyModifiers) -> Result<UiEventOutcome, String> {
    let trimmed = key.trim();
    if app.inline_editor.active {
        match trimmed {
            "Enter" | "Return" => return commit_inline_edit(app),
            "Escape" | "Esc" => return Ok(cancel_inline_edit(app)),
            "Backspace" => {
                app.inline_editor.backspace();
                return Ok(UiEventOutcome { handled: true, action: String::from("inline_edit.backspace"), ..Default::default() });
            }
            _ => {
                if !modifiers.ctrl && !modifiers.meta && !modifiers.alt && trimmed.chars().count() == 1 {
                    return Ok(handle_text_input(app, trimmed));
                }
            }
        }
    }

    if app.canvas_dialogs.active_dialog != ActiveDialog::None && matches!(trimmed, "Escape" | "Esc") {
        app.canvas_dialogs.close_dialog();
        return Ok(UiEventOutcome { handled: true, action: String::from("dialog.cancelled"), ..Default::default() });
    }

    for candidate in build_shortcut_candidates(trimmed, modifiers) {
        if let Ok(Some(exec)) = app.handle_key_combo(&candidate) {
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

fn build_shortcut_candidates(key: &str, modifiers: KeyModifiers) -> Vec<String> {
    if key.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(2);
    let build = |use_cmd_for_ctrl: bool| -> String {
        let mut parts = Vec::with_capacity(5);
        if modifiers.meta || (use_cmd_for_ctrl && modifiers.ctrl) {
            parts.push("Cmd");
        } else if modifiers.ctrl {
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
    canvas: &lumencraft_tools::CanvasToolState,
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
    canvas: &lumencraft_tools::CanvasToolState,
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
        | GraphAnnotation::EmbeddedResultTable { bounds_pt, .. } => Some(*bounds_pt),
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
        | GraphAnnotation::EmbeddedResultTable { bounds_pt, .. } => *bounds_pt = bounds,
        GraphAnnotation::LineOrArrow { start_pt, end_pt, .. } => {
            *start_pt = (bounds.x, bounds.y);
            *end_pt = (bounds.x + bounds.width, bounds.y + bounds.height);
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
