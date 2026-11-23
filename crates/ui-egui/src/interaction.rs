//! HiDPI viewport coordinate metrics, keyboard/shortcut/inline-edit dispatch,
//! inertial pan/zoom state, and in-memory WASM-clean [`VirtualWorkspaceFolder`] (`G6`, `M7`, `QOL-4`, `QOL-5`, `QOL-10`).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use curvebook_chart::PointPt;
use curvebook_doc::{
    CURRENT_SCHEMA_VERSION, CellCoord, ConstantValue, ImageMimeKind, MAX_IMAGE_ASSET_BYTES, MAX_PROJECT_IMAGE_ASSETS, ProjectDocument, RectPt,
    SheetCellRange, SheetId, SheetSection,
};
use curvebook_format::{
    ArchiveKind, ArchiveManifest, AssetManifestEntry, ZipLimits, export_datasheet_to_csv, migrate_document_json, open_project_bytes,
    save_project_bytes, validate_project_document,
};
use curvebook_render::RenderTargetSpec;
use curvebook_tools::canvas::ResizeHandleCorner;
use curvebook_tools::{GridCellCoord, ToolCommandIntent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::dialogs_canvas::ActiveDialog;
use crate::grid_view::parse_localized_number_input;
use crate::workspace_nav::{ActiveWorkspacePane, build_set_info_constant_command, build_set_info_notes_command};

const MIN_SCALE_FACTOR: f64 = 0.5;
const MAX_SCALE_FACTOR: f64 = 8.0;

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
    SheetTitle { sheet_id: SheetId },
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
        if let Some((prev_idx, _)) = self.buffer.get(..end).and_then(|s| s.char_indices().last()) {
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
            InlineEditorTarget::SheetTitle { sheet_id } => {
                let next_title = text.trim();
                if next_title.is_empty() {
                    return None;
                }
                Some(ToolCommandIntent::new(
                    "sheet.rename",
                    json!({
                        "sheet_id": sheet_id.as_str(),
                        "title": next_title,
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
    /// Write a relative file inside the virtual workspace while enforcing `ZipLimits::default()` caps.
    pub fn write_file(&mut self, rel_path: &str, bytes: Vec<u8>) -> Result<(), String> {
        let clean = Self::normalize_rel_path(rel_path)?;
        let limits = ZipLimits::default();
        let bytes_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if bytes_len > limits.max_entry_uncompressed_bytes {
            return Err(format!(
                "virtual workspace entry `{clean}` exceeds per-entry byte limit ({} > {})",
                bytes.len(),
                limits.max_entry_uncompressed_bytes
            ));
        }
        if !self.files.contains_key(&clean) && self.files.len() >= limits.max_entries {
            return Err(format!("virtual workspace exceeds maximum entry count ({})", limits.max_entries));
        }
        let current_total: u64 = self.files.values().fold(0u64, |acc, data| acc.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX)));
        let existing_len = u64::try_from(self.files.get(&clean).map_or(0usize, Vec::len)).unwrap_or(u64::MAX);
        let _next_total = current_total
            .saturating_sub(existing_len)
            .checked_add(bytes_len)
            .filter(|&sum| sum <= limits.max_total_uncompressed_bytes)
            .ok_or_else(|| format!("virtual workspace exceeds total byte budget ({} bytes max)", limits.max_total_uncompressed_bytes))?;
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

    /// Validate entry-count, per-entry, and total byte caps against `ZipLimits::default()`.
    ///
    /// # Errors
    /// Returns an error if any path is invalid or any storage limit is exceeded.
    pub fn validate_limits(&self) -> Result<(), String> {
        let limits = ZipLimits::default();
        if self.files.len() > limits.max_entries {
            return Err(format!("virtual workspace exceeds maximum entry count ({} > {})", self.files.len(), limits.max_entries));
        }
        let mut total_bytes = 0u64;
        for (path, bytes) in &self.files {
            let _ = Self::normalize_rel_path(path)?;
            let entry_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            if entry_len > limits.max_entry_uncompressed_bytes {
                return Err(format!(
                    "virtual workspace entry `{path}` exceeds per-entry byte limit ({} > {})",
                    bytes.len(),
                    limits.max_entry_uncompressed_bytes
                ));
            }
            total_bytes = total_bytes
                .checked_add(entry_len)
                .filter(|&sum| sum <= limits.max_total_uncompressed_bytes)
                .ok_or_else(|| format!("virtual workspace exceeds total byte budget ({} bytes max)", limits.max_total_uncompressed_bytes))?;
        }
        let _ = total_bytes;
        Ok(())
    }

    fn validate_manifest_and_assets(&self, doc_bytes: &[u8], raw_doc: &Value, from_version: u32) -> Result<ValidatedVirtualManifest, String> {
        let manifest_bytes = self.read_file("manifest.json").ok_or_else(|| String::from("virtual snapshot missing `manifest.json`"))?;
        let manifest_val: Value = serde_json::from_slice(manifest_bytes).map_err(|err| format!("failed decoding virtual `manifest.json`: {err}"))?;
        let manifest_obj = manifest_val.as_object().ok_or_else(|| String::from("virtual `manifest.json` must be a JSON object"))?;

        let manifest_schema_version = manifest_obj
            .get("schema_version")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| String::from("virtual `manifest.json` missing valid `schema_version`"))?;
        if manifest_schema_version > CURRENT_SCHEMA_VERSION {
            return Err(format!("unsupported virtual `manifest.json` schema_version {manifest_schema_version}"));
        }
        if manifest_schema_version != from_version {
            return Err(format!(
                "virtual `manifest.json` schema_version ({manifest_schema_version}) does not match `document.json` ({from_version})"
            ));
        }
        if let Some(title_val) = manifest_obj.get("title") {
            let manifest_title = title_val.as_str().ok_or_else(|| String::from("virtual `manifest.json` has non-string `title`"))?;
            if let Some(raw_title) = raw_doc.get("title").and_then(Value::as_str)
                && manifest_title != raw_title
            {
                return Err(String::from("virtual `manifest.json` title does not match `document.json`"));
            }
        }

        let is_archive_manifest = manifest_obj.contains_key("format") || manifest_obj.contains_key("kind") || manifest_obj.contains_key("assets");
        if is_archive_manifest {
            if !manifest_obj.contains_key("format") || !manifest_obj.contains_key("document_crc32") {
                return Err(String::from("archive virtual `manifest.json` missing `format` or `document_crc32`"));
            }
            let archive_manifest: ArchiveManifest =
                serde_json::from_value(manifest_val.clone()).map_err(|err| format!("invalid archive virtual `manifest.json`: {err}"))?;
            if archive_manifest.format != "curvebook-archive" {
                return Err(format!("unsupported virtual `manifest.json` format `{}`", archive_manifest.format));
            }
            if archive_manifest.kind != ArchiveKind::Project {
                return Err(format!("virtual workspace requires project archive, got `{}`", archive_manifest.kind.as_str()));
            }
            let actual_doc_crc = virtual_crc32_ieee(doc_bytes);
            if archive_manifest.document_crc32 != actual_doc_crc {
                return Err(format!(
                    "virtual `document.json` CRC-32 mismatch (manifest {:08x} != actual {actual_doc_crc:08x})",
                    archive_manifest.document_crc32
                ));
            }
            let limits = ZipLimits::default();
            if archive_manifest.sheets.len() > limits.max_entries {
                return Err(format!(
                    "archive virtual `manifest.json` sheet count {} exceeds limit {}",
                    archive_manifest.sheets.len(),
                    limits.max_entries
                ));
            }
            let mut seen_sections = HashSet::with_capacity(archive_manifest.sheets.len());
            for entry in &archive_manifest.sheets {
                if !seen_sections.insert((entry.section, entry.id.as_str().to_owned())) {
                    return Err(format!("archive virtual `manifest.json` contains duplicate sheet `{}`", entry.id.as_str()));
                }
                if entry.section == SheetSection::Data {
                    let csv_path = entry
                        .csv_path
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| format!("archive virtual `manifest.json` data sheet `{}` missing `csv_path`", entry.id.as_str()))?;
                    let clean_csv = Self::normalize_rel_path(csv_path)?;
                    let csv_bytes =
                        self.read_file(&clean_csv).ok_or_else(|| format!("virtual `manifest.json` references missing sheet `{clean_csv}`"))?;
                    let expected_crc =
                        entry.crc32.ok_or_else(|| format!("archive virtual `manifest.json` data sheet `{}` missing `crc32`", entry.id.as_str()))?;
                    let actual_crc = virtual_crc32_ieee(csv_bytes);
                    if expected_crc != actual_crc {
                        return Err(format!("virtual `{clean_csv}` CRC-32 mismatch (manifest {expected_crc:08x} != actual {actual_crc:08x})"));
                    }
                } else if let Some(csv_path) = entry.csv_path.as_deref().filter(|s| !s.is_empty()) {
                    let clean_csv = Self::normalize_rel_path(csv_path)?;
                    let csv_bytes =
                        self.read_file(&clean_csv).ok_or_else(|| format!("virtual `manifest.json` references missing sheet `{clean_csv}`"))?;
                    if let Some(expected_crc) = entry.crc32 {
                        let actual_crc = virtual_crc32_ieee(csv_bytes);
                        if expected_crc != actual_crc {
                            return Err(format!("virtual `{clean_csv}` CRC-32 mismatch (manifest {expected_crc:08x} != actual {actual_crc:08x})"));
                        }
                    }
                }
            }
            if archive_manifest.assets.len() > MAX_PROJECT_IMAGE_ASSETS {
                return Err(format!(
                    "archive virtual `manifest.json` asset count {} exceeds limit {MAX_PROJECT_IMAGE_ASSETS}",
                    archive_manifest.assets.len()
                ));
            }
            let mut seen_asset_paths = BTreeSet::new();
            let mut hydrated_assets = BTreeMap::new();
            for asset in &archive_manifest.assets {
                let clean_asset = Self::normalize_rel_path(&asset.path)?;
                if !seen_asset_paths.insert(clean_asset.clone()) {
                    return Err(format!("archive virtual `manifest.json` contains duplicate asset path `{clean_asset}`"));
                }
                if hydrated_assets.contains_key(&asset.asset_id) {
                    return Err(format!("archive virtual `manifest.json` contains duplicate asset id `{}`", asset.asset_id));
                }
                let asset_bytes =
                    self.read_file(&clean_asset).ok_or_else(|| format!("virtual `manifest.json` references missing asset `{clean_asset}`"))?;
                if asset_bytes.len() > MAX_IMAGE_ASSET_BYTES {
                    return Err(format!("virtual asset `{clean_asset}` exceeds maximum asset size ({MAX_IMAGE_ASSET_BYTES} bytes)"));
                }
                if u64::try_from(asset_bytes.len()).unwrap_or(u64::MAX) != asset.byte_len {
                    return Err(format!(
                        "virtual asset `{clean_asset}` byte length {} disagrees with manifest {}",
                        asset_bytes.len(),
                        asset.byte_len
                    ));
                }
                let actual_crc = virtual_crc32_ieee(asset_bytes);
                if asset.crc32 != actual_crc {
                    return Err(format!("virtual asset `{clean_asset}` CRC-32 mismatch (manifest {:08x} != actual {actual_crc:08x})", asset.crc32));
                }
                hydrated_assets.insert(asset.asset_id.clone(), (asset.media_type, asset_bytes.to_vec()));
            }
            return Ok(ValidatedVirtualManifest::Archive { manifest: archive_manifest, hydrated_assets });
        }

        if let Some(root_val) = manifest_obj.get("root_name")
            && !root_val.is_string()
        {
            return Err(String::from("virtual `manifest.json` has non-string `root_name`"));
        }
        if let Some(crc_val) = manifest_obj.get("document_crc32") {
            verify_manifest_crc32("document.json", crc_val, doc_bytes)?;
        }

        let sheets_arr =
            manifest_obj.get("sheets").and_then(Value::as_array).ok_or_else(|| String::from("virtual `manifest.json` missing `sheets` array"))?;
        let mut seen_sheet_ids = BTreeSet::new();
        for sheet_entry in sheets_arr {
            let obj = sheet_entry.as_object().ok_or_else(|| String::from("virtual `manifest.json` sheet entry must be an object"))?;
            let sheet_id = obj
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| String::from("virtual `manifest.json` sheet entry missing `id`"))?;
            if !seen_sheet_ids.insert(sheet_id.to_owned()) {
                return Err(format!("virtual `manifest.json` contains duplicate sheet `{sheet_id}`"));
            }
            let csv_path = obj
                .get("csv_path")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| String::from("virtual `manifest.json` sheet entry missing `csv_path`"))?;
            let clean_csv = Self::normalize_rel_path(csv_path)?;
            let csv_bytes = self.read_file(&clean_csv).ok_or_else(|| format!("virtual `manifest.json` references missing sheet `{clean_csv}`"))?;
            if let Some(crc_val) = obj.get("crc32") {
                verify_manifest_crc32(&clean_csv, crc_val, csv_bytes)?;
            }
        }
        Ok(ValidatedVirtualManifest::Unpacked { sheet_ids: seen_sheet_ids })
    }

    /// Write an unpacked `.curvebook.d`-compatible virtual snapshot (`document.json`, `manifest.json`,
    /// and `sheets/<id>.csv`) directly into memory with zero filesystem access (`QOL-10`).
    ///
    /// # Errors
    /// Returns an error if serialization fails or storage caps are exceeded.
    pub fn save_unpacked_snapshot(&mut self, doc: &ProjectDocument) -> Result<usize, String> {
        doc.validate_structural_limits().map_err(|err| format!("invalid project structural limits: {err}"))?;
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
        self.validate_limits()?;
        Ok(self.total_bytes())
    }

    /// Load and validate a [`ProjectDocument`] from `manifest.json` + `document.json` in this virtual workspace (`QOL-10`).
    ///
    /// # Errors
    /// Returns an error if limits are exceeded or `manifest.json`, `document.json`, or declared sheets are missing/corrupt.
    pub fn open_unpacked_snapshot(&self) -> Result<ProjectDocument, String> {
        self.validate_limits()?;
        let bytes = self.read_file("document.json").ok_or_else(|| String::from("virtual snapshot missing `document.json`"))?;
        let raw: Value = serde_json::from_slice(bytes).map_err(|err| format!("failed decoding virtual `document.json`: {err}"))?;
        let from_version = match raw.get("schema_version") {
            None | Some(Value::Null) => 0_u32,
            Some(v) => {
                v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| String::from("virtual `document.json` has invalid `schema_version`"))?
            }
        };
        if from_version > CURRENT_SCHEMA_VERSION {
            return Err(format!("unsupported virtual `document.json` schema_version {from_version} (max {CURRENT_SCHEMA_VERSION})"));
        }
        let validated_manifest = self.validate_manifest_and_assets(bytes, &raw, from_version)?;
        let migrated = migrate_document_json(raw, from_version).map_err(|err| format!("failed migrating virtual `document.json`: {err}"))?;
        let mut doc: ProjectDocument = serde_json::from_value(migrated).map_err(|err| format!("failed decoding virtual `document.json`: {err}"))?;
        match validated_manifest {
            ValidatedVirtualManifest::Archive { manifest, hydrated_assets } => {
                hydrate_virtual_image_assets(&mut doc, &manifest.assets, hydrated_assets)?;
                validate_archive_manifest_sections(&doc, &manifest)?;
            }
            ValidatedVirtualManifest::Unpacked { sheet_ids } => {
                if doc.data_sheets.len() != sheet_ids.len() || doc.data_sheets.iter().any(|ds| !sheet_ids.contains(ds.id.as_str())) {
                    return Err(String::from("virtual `manifest.json` sheet entries do not match `document.json` data sheets"));
                }
            }
        }
        doc.validate_structural_limits().map_err(|err| format!("invalid virtual `document.json`: {err}"))?;
        validate_project_document(&doc).map_err(|err| format!("invalid virtual `document.json`: {err}"))?;
        Ok(doc)
    }
}

enum ValidatedVirtualManifest {
    Archive { manifest: ArchiveManifest, hydrated_assets: BTreeMap<String, (ImageMimeKind, Vec<u8>)> },
    Unpacked { sheet_ids: BTreeSet<String> },
}

fn hydrate_virtual_image_assets(
    doc: &mut ProjectDocument,
    manifest_assets: &[AssetManifestEntry],
    mut hydrated_assets: BTreeMap<String, (ImageMimeKind, Vec<u8>)>,
) -> Result<(), String> {
    if doc.image_assets.len() > MAX_PROJECT_IMAGE_ASSETS {
        return Err(format!("virtual `document.json` image asset count {} exceeds limit {MAX_PROJECT_IMAGE_ASSETS}", doc.image_assets.len()));
    }
    let mut seen_ids = BTreeSet::new();
    for asset in &mut doc.image_assets {
        if !seen_ids.insert(asset.asset_id.clone()) {
            return Err(format!("virtual `document.json` contains duplicate image asset `{}`", asset.asset_id));
        }
        let Some((loaded_mime, loaded_bytes)) = hydrated_assets.remove(&asset.asset_id) else {
            return Err(format!("virtual `manifest.json` missing image asset `{}` declared in `document.json`", asset.asset_id));
        };
        if loaded_mime != asset.media_type {
            return Err(format!(
                "virtual image asset `{}` media_type {:?} in `document.json` disagrees with {:?} in `manifest.json`",
                asset.asset_id, asset.media_type, loaded_mime
            ));
        }
        if !asset.bytes.is_empty() && asset.bytes != loaded_bytes {
            return Err(format!("virtual image asset `{}` inline bytes disagree with external payload", asset.asset_id));
        }
        asset.bytes = loaded_bytes;
    }
    if let Some(orphan) = manifest_assets.iter().find(|entry| hydrated_assets.contains_key(&entry.asset_id)) {
        return Err(format!("virtual `manifest.json` declares image asset `{}` missing from `document.json`", orphan.asset_id));
    }
    Ok(())
}

fn validate_archive_manifest_sections(doc: &ProjectDocument, manifest: &ArchiveManifest) -> Result<(), String> {
    let mut expected = HashSet::with_capacity(doc.total_sheet_count());
    for ds in &doc.data_sheets {
        expected.insert((SheetSection::Data, ds.id.as_str().to_owned()));
    }
    for info in &doc.info_sheets {
        expected.insert((SheetSection::Info, info.id.as_str().to_owned()));
    }
    for res in &doc.result_sheets {
        expected.insert((SheetSection::Analyses, res.id.as_str().to_owned()));
    }
    for graph in &doc.graph_sheets {
        expected.insert((SheetSection::Graphs, graph.id.as_str().to_owned()));
    }
    for layout in &doc.layout_sheets {
        expected.insert((SheetSection::Layouts, layout.id.as_str().to_owned()));
    }
    let actual: HashSet<(SheetSection, String)> = manifest.sheets.iter().map(|s| (s.section, s.id.as_str().to_owned())).collect();
    if actual != expected {
        return Err(String::from("archive virtual `manifest.json` sheet entries do not match `document.json`"));
    }
    Ok(())
}

fn virtual_crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn verify_manifest_crc32(entry_name: &str, crc_val: &Value, bytes: &[u8]) -> Result<(), String> {
    let expected = crc_val
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("virtual `manifest.json` entry `{entry_name}` has invalid `crc32`"))?;
    let actual = virtual_crc32_ieee(bytes);
    if expected != actual {
        return Err(format!("virtual `{entry_name}` CRC-32 mismatch (manifest {expected:08x} != actual {actual:08x})"));
    }
    Ok(())
}

/// Persistent container scroll offsets stored on `InteractionControllerState`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ScrollStateStore {
    pub offsets: BTreeMap<String, [f64; 2]>,
}

impl ScrollStateStore {
    /// Read current `[x, y]` scroll offset for `container_id`.
    #[must_use]
    pub fn offset(&self, container_id: &str) -> [f64; 2] {
        self.offsets.get(container_id).copied().unwrap_or([0.0, 0.0])
    }
}

/// Option item inside a scrollable [`OpenDropdownState`] popup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropdownOption {
    pub value: String,
    pub label: String,
    pub enabled: bool,
}

/// Currently open dropdown menu popup state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenDropdownState {
    pub dropdown_id: String,
    pub label: String,
    pub selected_value: String,
    pub options: Vec<DropdownOption>,
}

/// Interactive single/multi-line text editing buffer (caret, selection, clipboard, IME preedit).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextFieldEditState {
    pub text: String,
    pub cursor_char: usize,
    pub selection_anchor: Option<usize>,
    pub preedit: Option<String>,
    pub multiline: bool,
}

pub type TextFieldBufferMap = BTreeMap<String, TextFieldEditState>;

impl TextFieldEditState {
    #[must_use]
    pub fn new(initial: impl Into<String>, multiline: bool) -> Self {
        let text = initial.into();
        let len = text.chars().count();
        Self { text, cursor_char: len, selection_anchor: None, preedit: None, multiline }
    }

    pub fn select_all(&mut self) {
        let len = self.text.chars().count();
        self.selection_anchor = Some(0);
        self.cursor_char = len;
    }

    pub fn move_left(&mut self, extend_selection: bool) {
        if !extend_selection {
            self.selection_anchor = None;
        } else if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor_char);
        }
        self.cursor_char = self.cursor_char.saturating_sub(1);
    }

    pub fn move_right(&mut self, extend_selection: bool) {
        let len = self.text.chars().count();
        if !extend_selection {
            self.selection_anchor = None;
        } else if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor_char);
        }
        self.cursor_char = (self.cursor_char + 1).min(len);
    }

    pub fn insert_text(&mut self, input: &str) {
        self.delete_selection_internal();
        let sanitized = if self.multiline { input.to_owned() } else { input.replace(['\r', '\n'], " ") };
        let byte_pos = char_to_byte_idx(&self.text, self.cursor_char);
        self.text.insert_str(byte_pos, &sanitized);
        self.cursor_char += sanitized.chars().count();
        self.selection_anchor = None;
        self.preedit = None;
    }

    pub fn backspace(&mut self) {
        if self.delete_selection_internal() || self.cursor_char == 0 {
            return;
        }
        let end_b = char_to_byte_idx(&self.text, self.cursor_char);
        let start_b = char_to_byte_idx(&self.text, self.cursor_char - 1);
        self.text.replace_range(start_b..end_b, "");
        self.cursor_char -= 1;
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection_internal() {
            return;
        }
        let len = self.text.chars().count();
        if self.cursor_char >= len {
            return;
        }
        let start_b = char_to_byte_idx(&self.text, self.cursor_char);
        let end_b = char_to_byte_idx(&self.text, self.cursor_char + 1);
        self.text.replace_range(start_b..end_b, "");
    }

    #[must_use]
    pub fn copy_selection(&self) -> Option<String> {
        let anchor = self.selection_anchor?;
        let lo = anchor.min(self.cursor_char);
        let hi = anchor.max(self.cursor_char);
        if lo == hi {
            return None;
        }
        let sb = char_to_byte_idx(&self.text, lo);
        let eb = char_to_byte_idx(&self.text, hi);
        self.text.get(sb..eb).map(str::to_owned)
    }

    /// Cut the active selection (when non-empty), returning the removed text.
    pub fn cut_selection(&mut self) -> Option<String> {
        let copied = self.copy_selection()?;
        self.delete_selection_internal();
        Some(copied)
    }

    pub fn set_preedit(&mut self, preedit: Option<String>) {
        self.preedit = preedit.filter(|s| !s.is_empty());
    }

    pub fn commit_preedit(&mut self) {
        if let Some(p) = self.preedit.take() {
            self.insert_text(&p);
        }
    }

    fn delete_selection_internal(&mut self) -> bool {
        let Some(anchor) = self.selection_anchor.take() else {
            return false;
        };
        let lo = anchor.min(self.cursor_char);
        let hi = anchor.max(self.cursor_char);
        if lo == hi {
            return false;
        }
        let sb = char_to_byte_idx(&self.text, lo);
        let eb = char_to_byte_idx(&self.text, hi);
        self.text.replace_range(sb..eb, "");
        self.cursor_char = lo;
        true
    }
}

fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map_or(s.len(), |(idx, _)| idx)
}

/// Supported zoom presets exposed in the canvas viewport Zoom-% selector.
pub const CANVAS_ZOOM_PRESETS: &[f64] = &[0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

/// Cycle to the next standard zoom percentage in [`CANVAS_ZOOM_PRESETS`].
#[must_use]
pub fn next_zoom_preset(current_zoom: f64) -> f64 {
    for &preset in CANVAS_ZOOM_PRESETS {
        if preset > current_zoom + 0.05 {
            return preset;
        }
    }
    CANVAS_ZOOM_PRESETS[0]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_copied_result_table: Option<ResultTableClipboard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_clipboard_image_png: Option<Vec<u8>>,
}

/// Structured copy reference to a [`curvebook_doc::ResultSheet`] selection for graph pasting (`WF-58`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultTableClipboard {
    pub source_sheet_id: SheetId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_rows: Option<Vec<usize>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_cols: Option<Vec<usize>>,
}

/// Structured outcome emitted by [`dispatch_ui_event`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UiEventOutcome {
    pub handled: bool,
    pub action: String,
    pub command_id: Option<String>,
    pub command_result: Option<Value>,
}

fn dispatch_ui_event_inner(app: &mut UiApp, event: UiInputEvent, metrics: ViewportMetrics) -> Result<UiEventOutcome, String> {
    let outcome = match event {
        UiInputEvent::PointerDown { x, y, button, .. } => {
            app.interaction.last_logical_pos = Some([x, y]);
            if button == PointerButton::Primary {
                app.interaction.pointer_down = true;
                UiEventOutcome { handled: true, action: String::from("pointer_down"), ..Default::default() }
            } else if button == PointerButton::Secondary {
                clear_context_menu_state(app);
                UiEventOutcome { handled: false, action: String::from("secondary_click_ignored"), ..Default::default() }
            } else {
                UiEventOutcome { handled: false, action: String::from("pointer_down_ignored"), ..Default::default() }
            }
        }
        UiInputEvent::PointerMove { x, y, .. } => {
            app.interaction.last_logical_pos = Some([x, y]);
            UiEventOutcome { handled: false, action: String::from("pointer_move"), ..Default::default() }
        }
        UiInputEvent::PointerUp { x, y, button, .. } => {
            app.interaction.last_logical_pos = Some([x, y]);
            if button == PointerButton::Primary {
                app.interaction.pointer_down = false;
            }
            UiEventOutcome { handled: true, action: String::from("pointer_up"), ..Default::default() }
        }
        UiInputEvent::DoubleClick { x, y, .. } => {
            app.interaction.last_logical_pos = Some([x, y]);
            UiEventOutcome { handled: false, action: String::from("double_click"), ..Default::default() }
        }
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
    dispatch_ui_event_inner(app, event, metrics)
}

pub(crate) fn clear_inspector_axis_buffers(app: &mut UiApp) {
    app.interaction.text_field_buffers.remove("inspector.axes.min.input");
    app.interaction.text_field_buffers.remove("inspector.axes.max.input");
    app.interaction.text_field_buffers.remove("inspector.axes.interval.input");
    app.interaction.text_field_buffers.remove("inspector.axes.title.input");
}

/// Clear transient modal textbox buffers so stale values never bleed across separate dialog sessions.
pub fn clear_modal_text_buffers(app: &mut UiApp) {
    for &id in crate::inspect::TEXTBOX_WIDGET_IDS {
        if id != "formula_bar.input" {
            app.interaction.text_field_buffers.remove(id);
        }
    }
    for id in [
        "exclude.note_input",
        "page_layout.callout_input",
        "rand_groups.arms_input",
        "rand_groups.title_input",
        "fit.eq.custom_dsl",
        "fit.dsl.input",
        "fit.eq.search",
        "navigator.search_input",
        "hook.search_input",
        "feedback.title",
        "feedback.summary",
        "feedback.details",
        "feedback.extra",
    ] {
        app.interaction.text_field_buffers.remove(id);
    }
}

pub(crate) fn close_active_modal(app: &mut UiApp) {
    if matches!(app.canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }) {
        crate::views::fit_params::clear_fit_parameter_inputs(app);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.egui_views.stats_modals.open_session_sheet_id = None;
        app.egui_views.fit_modal.open_session_sheet_id = None;
    }
    app.canvas_dialogs.close_dialog();
}

fn clear_context_menu_state(app: &mut UiApp) {
    app.interaction.context_menu_target = None;
    app.view_shards.toolbar_menus.active_context_menu_id = None;
    app.view_shards.toolbar_menus.context_menu_anchor_px = None;
    app.view_shards.toolbar_menus.context_sheet_id = None;
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

/// Dispatch a mouse-wheel or trackpad scroll event across data sheets and graph/layout canvases.
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
    if app.canvas_dialogs.active_dialog != ActiveDialog::None {
        return Ok(UiEventOutcome { handled: false, action: String::from("wheel.dialog_open_ignored"), ..Default::default() });
    }
    if modifiers.ctrl || modifiers.meta {
        return Ok(handle_wheel_zoom(app, metrics, [x, y], delta_y));
    }
    if let Some(active_id) = app.document().active_sheet_id.clone() {
        let doc = app.document();
        if let Some(ds) = doc.data_sheet(&active_id).cloned() {
            app.grid.tool.scroll_for_sheet(&ds, -delta_x, -delta_y);
            #[cfg(not(target_arch = "wasm32"))]
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            {
                let max_x = ((ds.archetype.column_count() as f32) * 140.0).max(2_000.0);
                let max_y = ((ds.archetype.row_count() as f32) * 28.0).max(2_000.0);
                let pt = &mut app.egui_views.column_grid.scroll_offset_pt;
                pt.x = (pt.x - (delta_x as f32)).clamp(0.0, max_x);
                pt.y = (pt.y - (delta_y as f32)).clamp(0.0, max_y);
            }
            app.sync_views();
            return Ok(UiEventOutcome { handled: true, action: String::from("grid.scroll"), ..Default::default() });
        }
        if doc.graph_sheet(&active_id).is_some() || doc.layout_sheet(&active_id).is_some() {
            return Ok(handle_inertial_pan(app, metrics, [delta_x, delta_y], 0.016, false));
        }
    }
    Ok(UiEventOutcome { handled: false, action: String::from("mouse_wheel_ignored"), ..Default::default() })
}

fn execute_engine_value(app: &mut UiApp, command_id: &str, params: Value) -> Result<Value, String> {
    let outcome = app.execute_command(command_id, params).map_err(|err| err.to_string())?;
    serde_json::to_value(outcome).map_err(|err| format!("failed to serialize command outcome: {err}"))
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
    let _ = metrics;
    let anchor_pt = [logical_pos[0], logical_pos[1]];

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

fn is_multiline_text_widget_id(wid: &str) -> bool {
    matches!(
        wid,
        "info.notes"
            | "info.secondary.notes"
            | "info.notes_editor"
            | "secondary.info.notes_editor"
            | "import_paste.input.csv"
            | "import_paste.input.notes"
            | "import_paste.input.plate_values"
            | "import.tidy_csv"
            | "page_layout.callout_input"
            | "exclude.note_input"
            | "exclude.note"
            | "checkpoints.note"
            | "fit.eq.custom_dsl"
            | "fit.dsl.input"
            | "edit.fit.custom_dsl"
            | "analyze.fit.custom_dsl"
            | "feedback.summary"
            | "feedback.details"
            | "feedback.extra"
    )
}

fn is_info_notes_widget_id(wid: &str) -> bool {
    matches!(wid, "info.notes" | "info.secondary.notes" | "info.notes_editor" | "secondary.info.notes_editor")
}

fn resolve_info_sheet_for_textbox(app: &UiApp, wid: &str) -> Option<SheetId> {
    let doc = app.document();
    let is_secondary = wid == "info.secondary.notes" || wid == "secondary.info.notes_editor";
    let candidate = if app.split.enabled {
        let (prim, sec) = app.split.resolve_sheets(doc);
        if is_secondary { sec } else { prim }
    } else if is_secondary {
        None
    } else {
        doc.active_sheet_id.clone()
    };
    candidate.filter(|id| doc.info_sheet(id).is_some())
}

fn initial_text_for_widget(app: &UiApp, wid: &str) -> String {
    if is_info_notes_widget_id(wid) {
        if let Some(info_id) = resolve_info_sheet_for_textbox(app, wid) {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let view = if wid == "info.secondary.notes" || wid == "secondary.info.notes_editor" {
                    &app.egui_views.center_stage.secondary_info_view
                } else {
                    &app.egui_views.info_view
                };
                if let Some((draft_id, draft)) = &view.notes_draft
                    && draft_id == &info_id
                {
                    return draft.clone();
                }
            }
            if let Some(info) = app.document().info_sheet(&info_id) {
                return info.notes_markdown.clone();
            }
        }
        return String::new();
    }
    match wid {
        "formula_bar.input" => app.grid.formula_bar.input.clone(),
        "palette.query" => app.canvas_dialogs.command_palette.query.clone(),
        "navigator.search_input" => app.nav.search_query.clone(),
        "welcome.title.input" => app.nav.welcome.table_draft.title.clone(),
        "welcome.clone.title.input" => app.nav.welcome.clone_draft.options.title.clone(),
        "analyze.search" => app.canvas_dialogs.analyze.search_query.clone(),
        "hook.search_query" | "hook.search_input" => app.canvas_dialogs.hook_picker.search_query.clone(),
        "exclude.note" | "exclude.note_input" => app.canvas_dialogs.exclude_with_reason.note.clone(),
        "checkpoints.name" => app.canvas_dialogs.checkpoints.name_draft.clone(),
        "checkpoints.note" => app.canvas_dialogs.checkpoints.note_draft.clone(),
        "feedback.title" => app.canvas_dialogs.feedback.title.clone(),
        "feedback.summary" => app.canvas_dialogs.feedback.summary.clone(),
        "feedback.details" => app.canvas_dialogs.feedback.details.clone(),
        "feedback.extra" => app.canvas_dialogs.feedback.extra.clone(),
        "import.title" => app.grid.import_reshape.title.clone(),
        "import.x_title" => app.grid.import_reshape.x_title.clone(),
        "import.tidy_csv" => app.grid.import_reshape.tidy_csv.clone(),
        "import.group_col" => app.grid.import_reshape.group_col.clone(),
        "import.value_col" => app.grid.import_reshape.value_col.clone(),
        "import.x_col" => app.grid.import_reshape.x_col.clone().unwrap_or_default(),
        "import.replicate_col" => app.grid.import_reshape.replicate_col.clone().unwrap_or_default(),
        "axes.title" => app.canvas_dialogs.format_axes.axis_draft.title.clone(),
        "create_graph.title" => app.canvas_dialogs.create_graph.graph_title.clone(),
        "create_graph.y_axis_title" => app.canvas_dialogs.create_graph.y_axis_title.clone(),
        "randomize.title" => app.canvas_dialogs.randomize_groups.title.clone(),
        "sample.title" => app.canvas_dialogs.random_sample_rows.title.clone(),
        "simulate.title" => app.canvas_dialogs.simulate_distributions.title.clone(),
        _ => String::new(),
    }
}

fn ensure_text_field_buffer(app: &mut UiApp, wid: &str, initial: &str) {
    if app.interaction.text_field_buffers.contains_key(wid) {
        return;
    }
    let multiline = is_multiline_text_widget_id(wid);
    let seed = if !initial.is_empty() {
        initial.to_owned()
    } else if is_info_notes_widget_id(wid) {
        initial_text_for_widget(app, wid)
    } else {
        String::new()
    };
    app.interaction.text_field_buffers.insert(wid.to_owned(), TextFieldEditState::new(&seed, multiline));
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

fn parse_f64_list_text(raw: &str, preset: &crate::i18n::LocalePreset) -> Option<Vec<f64>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(Vec::new());
    }
    if trimmed.starts_with('[') {
        let parsed = serde_json::from_str::<Vec<f64>>(trimmed).ok()?;
        return parsed.iter().all(|v| v.is_finite()).then_some(parsed);
    }
    let sep = if preset.decimal_separator == ',' { ';' } else { ',' };
    let mut out = Vec::new();
    for tok in trimmed.split(|c: char| c == sep || c.is_whitespace()).filter(|s| !s.is_empty()) {
        let val = parse_localized_number_input(tok, preset).filter(|v| v.is_finite())?;
        out.push(val);
    }
    Some(out)
}

fn sync_active_dialog_value(app: &mut UiApp, key: &str, val: Value) {
    let mut map = serde_json::Map::new();
    map.insert(key.to_owned(), val);
    let _ = crate::gui_actions::patch_active_dialog(app, &map);
}

fn sync_info_notes_text_field(app: &mut UiApp, fid: &str, text: &str) {
    let Some(info_id) = resolve_info_sheet_for_textbox(app, fid) else {
        return;
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let view = if fid == "info.secondary.notes" || fid == "secondary.info.notes_editor" {
            &mut app.egui_views.center_stage.secondary_info_view
        } else {
            &mut app.egui_views.info_view
        };
        view.notes_draft = Some((info_id.clone(), text.to_owned()));
        view.notes_dirty = false;
    }
    if app.document().info_sheet(&info_id).is_some_and(|s| s.notes_markdown != text) {
        let intent = crate::workspace_nav::build_set_info_notes_command(&info_id, text, false);
        let _ = app.execute_command(&intent.command_id, intent.params);
    }
}

fn sync_edit_params_text_field(app: &mut UiApp, fid: &str, text: &str) {
    let mut map = serde_json::Map::new();
    match fid {
        "edit.custom_title" => {
            let val = if text.is_empty() { Value::Null } else { Value::String(text.to_owned()) };
            map.insert(String::from("edit.custom_title"), val);
        }
        "edit.transform.formula" | "edit.transform.filter_formula" | "edit.fit.model_id" | "edit.fit.custom_dsl" => {
            map.insert(fid.to_owned(), Value::String(text.to_owned()));
        }
        "edit.transform.selected_data_sets" | "edit.transform.baseline_columns" | "edit.glm.predictor_columns" | "edit.cox.predictor_columns" => {
            let Some(cols) = parse_usize_list_text(text) else {
                return;
            };
            map.insert(fid.to_owned(), serde_json::json!(cols));
        }
        "edit.contingency.expected_proportions" => {
            if text.trim().is_empty() {
                map.insert(fid.to_owned(), Value::Null);
            } else {
                let Some(vals) = parse_f64_list_text(text, &app.locale_preset) else {
                    return;
                };
                map.insert(fid.to_owned(), serde_json::json!(vals));
            }
        }
        _ => return,
    }
    let _ = app.canvas_dialogs.edit_params.apply_field_patch_localized(&map, &app.locale_preset);
}

fn sync_format_axes_text_field(app: &mut UiApp, fid: &str, text: &str) {
    let doc = app.document().clone();
    let mut map = serde_json::Map::new();
    match fid {
        "axes.title" => {
            map.insert(String::from("axes.title"), Value::String(text.to_owned()));
        }
        "axes.thresholds" | "axes.y_thresholds" | "axes.x_thresholds" => {
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

/// Flush any buffered `FormatAxes` textbox edits (`axes.title`, `axes.thresholds`, `axes.y_thresholds`, `axes.x_thresholds`) into `format_axes`.
pub fn sync_format_axes_text_buffers(app: &mut UiApp) {
    for fid in ["axes.title", "axes.thresholds", "axes.y_thresholds", "axes.x_thresholds"] {
        if let Some(text) = app.interaction.text_field_buffers.get(fid).map(|b| b.text.clone()) {
            sync_format_axes_text_field(app, fid, &text);
        }
    }
}

/// Flush any buffered `EditAnalysisParameters` textbox edits into `edit_params`.
pub fn sync_edit_params_text_buffers(app: &mut UiApp) {
    for fid in [
        "edit.custom_title",
        "edit.transform.formula",
        "edit.transform.filter_formula",
        "edit.transform.selected_data_sets",
        "edit.transform.baseline_columns",
        "edit.contingency.expected_proportions",
        "edit.glm.predictor_columns",
        "edit.cox.predictor_columns",
        "edit.fit.model_id",
        "edit.fit.custom_dsl",
    ] {
        if let Some(text) = app.interaction.text_field_buffers.get(fid).map(|b| b.text.clone()) {
            sync_edit_params_text_field(app, fid, &text);
        }
    }
}

fn sync_power_or_format_points_text_field(app: &mut UiApp, fid: &str, text: &str) -> bool {
    match fid {
        "power.standardized_effect" | "power.within_pair_correlation" | "power.alpha" | "power.target_power" => {
            if let Some(val) = parse_localized_number_input(text.trim(), &app.locale_preset).filter(|v| v.is_finite()) {
                sync_active_dialog_value(app, fid, serde_json::json!(val));
            }
            true
        }
        "format_points.color" => {
            let v = if text.trim().is_empty() { Value::Null } else { Value::String(text.to_owned()) };
            sync_active_dialog_value(app, fid, v);
            true
        }
        "format_points.size_pt" => {
            if text.trim().is_empty() {
                sync_active_dialog_value(app, fid, Value::Null);
            } else if let Some(val) = parse_localized_number_input(text.trim(), &app.locale_preset).filter(|v| v.is_finite()) {
                sync_active_dialog_value(app, fid, serde_json::json!(val));
            }
            true
        }
        _ => false,
    }
}

fn sync_modal_text_field(app: &mut UiApp, fid: &str, text: &str) -> bool {
    if sync_power_or_format_points_text_field(app, fid, text) {
        return true;
    }
    match fid {
        "analyze.transform.formula" | "analyze.transform.filter_formula" | "analyze.fit.custom_dsl" => {
            sync_active_dialog_value(app, fid, Value::String(text.to_owned()));
        }
        "analyze.fit.equation_id" => sync_active_dialog_value(app, "equation_id", Value::String(text.to_owned())),
        "analyze.transform.selected_data_sets" | "analyze.transform.baseline_columns" => {
            if let Some(cols) = parse_usize_list_text(text) {
                sync_active_dialog_value(app, fid, serde_json::json!(cols));
            }
        }
        "analyze.expected_proportions" => {
            if let Some(vals) = parse_f64_list_text(text, &app.locale_preset).filter(|v| !v.is_empty()) {
                sync_active_dialog_value(app, fid, serde_json::json!(vals));
            }
        }
        "edit.custom_title"
        | "edit.transform.formula"
        | "edit.transform.filter_formula"
        | "edit.transform.selected_data_sets"
        | "edit.transform.baseline_columns"
        | "edit.contingency.expected_proportions"
        | "edit.glm.predictor_columns"
        | "edit.cox.predictor_columns"
        | "edit.fit.model_id"
        | "edit.fit.custom_dsl" => sync_edit_params_text_field(app, fid, text),
        "import.title" | "import.x_title" | "import.tidy_csv" | "import.group_col" | "import.value_col" => {
            sync_active_dialog_value(app, fid, Value::String(text.to_owned()));
        }
        "import.x_col" | "import.replicate_col" => {
            let v = if text.trim().is_empty() { Value::Null } else { Value::String(text.to_owned()) };
            sync_active_dialog_value(app, fid, v);
        }
        "axes.title" | "axes.thresholds" | "axes.y_thresholds" | "axes.x_thresholds" => sync_format_axes_text_field(app, fid, text),
        "create_graph.y_axis_title" | "create_graph.title" | "simulate.title" | "simulate.column_name" | "sample.title" => {
            sync_active_dialog_value(app, fid, Value::String(text.to_owned()));
        }
        "randomize.groups" => {
            let _ = app.canvas_dialogs.randomize_groups.set_groups_from_str(text);
        }
        "randomize.block_sizes" => {
            if let Some(cols) = parse_usize_list_text(text).filter(|c| !c.is_empty()) {
                sync_active_dialog_value(app, fid, serde_json::json!(cols));
            }
        }
        "randomize.title" => app.canvas_dialogs.randomize_groups.title = text.to_owned(),
        "change_data.selected_data_sets" => {
            if let Some(cols) = parse_usize_list_text(text) {
                sync_active_dialog_value(app, fid, serde_json::json!(cols));
            }
        }
        _ => return false,
    }
    true
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
    if is_info_notes_widget_id(fid) {
        sync_info_notes_text_field(app, fid, &text);
        return;
    }
    if sync_modal_text_field(app, fid, &text) {
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
        "hook.search_query" | "hook.search_input" => app.canvas_dialogs.hook_picker.search_query = text,
        "exclude.note" | "exclude.note_input" => app.canvas_dialogs.exclude_with_reason.note = text,
        "checkpoints.name" => app.canvas_dialogs.checkpoints.name_draft = text,
        "checkpoints.note" => app.canvas_dialogs.checkpoints.note_draft = text,
        "feedback.title" => app.canvas_dialogs.feedback.title = text,
        "feedback.summary" => app.canvas_dialogs.feedback.summary = text,
        "feedback.details" => app.canvas_dialogs.feedback.details = text,
        "feedback.extra" => app.canvas_dialogs.feedback.extra = text,
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
        "inspector.axes.title.input" | "inspector.mv.color_clamp_min.input" | "inspector.mv.color_clamp_max.input" => {}
        _ => {}
    }
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
        let _ = metrics;
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
    crate::inspect::is_textbox_widget_id(wid)
        || is_multiline_text_widget_id(wid)
        || matches!(
            wid,
            "palette.query"
                | "formula_bar.input"
                | "info.notes"
                | "info.secondary.notes"
                | "info.notes_editor"
                | "secondary.info.notes_editor"
                | "info.const.editor_value"
                | "edit.transform.constant_k"
                | "edit.transform.formula"
                | "edit.transform.filter_formula"
                | "edit.transform.selected_data_sets"
                | "edit.transform.baseline_columns"
                | "edit.transform.simulate.x_min"
                | "edit.transform.simulate.x_max"
                | "edit.transform.simulate.row_count"
                | "edit.transform.simulate.seed"
                | "edit.transform.simulate.params"
                | "edit.transform.random_sample.seed"
                | "exclude.reason_tag"
                | "axes.y_thresholds"
                | "axes.x_thresholds"
                | "axes.thresholds"
                | "axes.title"
                | "feedback.title"
                | "feedback.summary"
                | "feedback.details"
                | "feedback.extra"
                | "fit.wr.x_min"
                | "fit.wr.x_max"
        )
        || wid.starts_with("fit.con.")
        || wid.starts_with("fit.init.")
        || wid.contains("search")
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

pub(crate) fn resolve_figure_sheet_for_shortcut(app: &UiApp) -> Option<SheetId> {
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
    copy_figure_sheet_bundle_dpi(app, sheet_id, metrics.dpi_for_base(144))
}

pub(crate) fn copy_figure_sheet_bundle_dpi(app: &mut UiApp, sheet_id: &SheetId, dpi: u16) -> Result<UiEventOutcome, String> {
    let bundle = crate::os_integration::ClipboardBundle::from_sheet(app.session(), sheet_id, dpi.max(72))?;
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

fn read_clipboard_text_for_paste(app: &UiApp) -> Option<String> {
    if let Ok(status) = app.read_clipboard_text()
        && status.available
        && let Some(text) = status.text
        && !text.is_empty()
    {
        return Some(text);
    }
    app.interaction.last_clipboard_text.clone().filter(|t| !t.is_empty())
}

fn try_handle_workspace_paste_shortcut(app: &mut UiApp) -> Result<Option<UiEventOutcome>, String> {
    if app.nav.welcome.open || app.nav.gallery.open {
        return Ok(None);
    }
    let Some(ds_id) = app.document().active_sheet_id.as_ref().filter(|id| app.document().data_sheet(id).is_some_and(|ds| !ds.frozen)).cloned() else {
        return Ok(None);
    };
    let Some(clip) = read_clipboard_text_for_paste(app) else {
        return Ok(None);
    };
    if let Some((x_row, x_subcol)) = app.grid.active_x_cell {
        let res = crate::gui_actions::paste_text_into_active_x_cell(app, &ds_id, x_row, x_subcol, &clip).map_err(|e| e.to_string())?;
        app.grid.refresh_selection_stats(app.session.document());
        return Ok(Some(UiEventOutcome {
            handled: true,
            action: String::from("shortcut:table.paste"),
            command_id: Some(String::from("table.paste")),
            command_result: Some(res),
        }));
    }
    let anchor = app.grid.tool.active.unwrap_or(GridCellCoord::new(0, 0, 0));
    let res = execute_engine_value(
        app,
        "table.paste",
        json!({
            "sheet_id": ds_id.as_str(),
            "text": clip,
            "start_row": anchor.row,
            "start_col": anchor.col,
            "start_subcol": anchor.subcol,
            "propagate_title_header": false,
        }),
    )?;
    app.grid.refresh_selection_stats(app.session.document());
    Ok(Some(UiEventOutcome {
        handled: true,
        action: String::from("shortcut:table.paste"),
        command_id: Some(String::from("table.paste")),
        command_result: Some(res),
    }))
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

    if !modal_open && app.inline_editor.active {
        if (modifiers.meta || modifiers.ctrl) && !modifiers.shift && !modifiers.alt && trimmed.eq_ignore_ascii_case("v") {
            if let Some(clip) = read_clipboard_text_for_paste(app) {
                app.inline_editor.insert_text(&clip);
                if matches!(app.inline_editor.target, Some(InlineEditorTarget::GridCell { .. } | InlineEditorTarget::GridXCell { .. })) {
                    app.grid.tool.editing_buffer = Some(app.inline_editor.buffer.clone());
                }
            }
            return Ok(UiEventOutcome { handled: true, action: String::from("inline_edit.paste"), ..Default::default() });
        }
        if !modifiers.meta && !modifiers.ctrl {
            return handle_inline_editor_key(app, metrics, trimmed, modifiers);
        }
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

    if !text_focused
        && (modifiers.meta || modifiers.ctrl)
        && !modifiers.shift
        && !modifiers.alt
        && trimmed.eq_ignore_ascii_case("v")
        && let Some(out) = try_handle_workspace_paste_shortcut(app)?
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
        if app.view_shards.grid_core.editing_title.is_some() {
            app.view_shards.grid_core.editing_title = None;
            app.interaction.text_field_buffers.remove("grid.title_editor.input");
            if app.focus_widget_id.as_deref() == Some("grid.title_editor.input") {
                app.focus_widget_id = None;
            }
            return Ok(UiEventOutcome { handled: true, action: String::from("grid_core.title_cancel"), ..Default::default() });
        }
        if app.interaction.open_dropdown.take().is_some() || app.interaction.open_menu.take().is_some() {
            return Ok(UiEventOutcome { handled: true, action: String::from("popup.dismissed"), ..Default::default() });
        }
        if app.nav.welcome.open {
            app.nav.welcome.open = false;
            return Ok(UiEventOutcome { handled: true, action: String::from("welcome.dismissed"), ..Default::default() });
        }
        if app.nav.gallery.open {
            app.nav.gallery.open = false;
            return Ok(UiEventOutcome { handled: true, action: String::from("gallery.dismissed"), ..Default::default() });
        }
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
        let _ = modifiers;
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
        && let Some(out) = handle_textbox_key(app, metrics, &fid, raw_key, modifiers)?
    {
        return Ok(out);
    }

    if !modifiers.alt && matches!(trimmed, "Enter" | "Return") {
        let res = crate::gui_actions::submit_active_dialog_ok(app).map_err(|e| e.to_string())?;
        if app.canvas_dialogs.active_dialog == ActiveDialog::None {
            clear_context_menu_state(app);
        }
        return Ok(UiEventOutcome { handled: true, action: String::from("dialog.confirmed"), command_result: Some(res), ..Default::default() });
    }

    let _ = is_space;

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
    if !modifiers.ctrl && !modifiers.meta && !modifiers.alt && matches!(key, "Enter" | "Return") && is_multiline_text_widget_id(fid) {
        if let Some(b) = app.interaction.text_field_buffers.get_mut(fid) {
            b.insert_text("\n");
        }
        sync_focused_text_field(app, fid);
        return Ok(Some(UiEventOutcome { handled: true, action: format!("textbox.newline:{fid}"), ..Default::default() }));
    }
    if (modifiers.ctrl || modifiers.meta)
        && !modifiers.alt
        && matches!(key, "Enter" | "Return")
        && app.canvas_dialogs.active_dialog == ActiveDialog::None
        && is_info_notes_widget_id(fid)
    {
        sync_focused_text_field(app, fid);
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("info_notes.committed"), ..Default::default() }));
    }
    if (modifiers.ctrl || modifiers.meta) && key.eq_ignore_ascii_case("a") {
        if let Some(b) = app.interaction.text_field_buffers.get_mut(fid) {
            b.select_all();
        }
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("textbox.select_all"), ..Default::default() }));
    }
    if (modifiers.ctrl || modifiers.meta) && key.eq_ignore_ascii_case("c") {
        if let Some(sel) = app.interaction.text_field_buffers.get(fid).and_then(TextFieldEditState::copy_selection) {
            app.interaction.last_clipboard_text = Some(sel.clone());
            let _ = app.write_clipboard_text(&sel);
        }
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("textbox.copy"), ..Default::default() }));
    }
    if (modifiers.ctrl || modifiers.meta) && key.eq_ignore_ascii_case("x") {
        if let Some(sel) = app.interaction.text_field_buffers.get_mut(fid).and_then(TextFieldEditState::cut_selection) {
            app.interaction.last_clipboard_text = Some(sel.clone());
            let _ = app.write_clipboard_text(&sel);
            sync_focused_text_field(app, fid);
        }
        return Ok(Some(UiEventOutcome { handled: true, action: String::from("textbox.cut"), ..Default::default() }));
    }
    if (modifiers.ctrl || modifiers.meta) && key.eq_ignore_ascii_case("v") {
        if let Some(clip) = read_clipboard_text_for_paste(app) {
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

/// Build the ordered list of canonical shortcut strings for `(key, modifiers)`, including the
/// `Ctrl`-to-`Cmd` fallback candidate when `ctrl` is pressed without `meta`.
///
/// Modifier order is deterministic (`Cmd`, `Ctrl`, `Alt`, `Shift`, `key`). Returns an empty vector
/// when `key` is empty.
#[must_use]
pub fn build_shortcut_candidates(key: &str, modifiers: KeyModifiers) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::{KeyModifiers, build_shortcut_candidates};

    #[test]
    fn build_shortcut_candidates_preserves_order_ctrl_to_cmd_fallback_and_combined_ctrl_cmd() {
        assert!(build_shortcut_candidates("", KeyModifiers::default()).is_empty());
        assert_eq!(build_shortcut_candidates("Escape", KeyModifiers::default()), vec!["Escape"]);
        assert_eq!(build_shortcut_candidates("s", KeyModifiers { meta: true, ..Default::default() }), vec!["Cmd+s"]);
        assert_eq!(build_shortcut_candidates("s", KeyModifiers { ctrl: true, ..Default::default() }), vec!["Ctrl+s", "Cmd+s"]);
        assert_eq!(
            build_shortcut_candidates("k", KeyModifiers { ctrl: true, alt: true, shift: true, meta: false }),
            vec!["Ctrl+Alt+Shift+k", "Cmd+Alt+Shift+k"]
        );
        assert_eq!(build_shortcut_candidates("k", KeyModifiers { ctrl: true, meta: true, alt: true, shift: true }), vec!["Cmd+Ctrl+Alt+Shift+k"]);
    }
}
