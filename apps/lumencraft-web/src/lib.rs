//! Pure-Rust, WebAssembly-compatible workspace host for LumenCraft (`QOL-10`, `G6`).
//!
//! Zero JavaScript bindings and zero OS filesystem, process, or network calls: all archive
//! decoding, virtual `.lumencraft.d` folder editing, interactive pointer/inline-edit dispatch,
//! color-vision simulation filters, presentation mode, and RGBA framebuffer rendering stay 100%
//! self-contained in memory.
#![forbid(unsafe_code)]

use lumencraft_engine::Session;
use lumencraft_format::{
    ArchiveKind, ImportReport, import_prism_bytes, import_pzfx_bytes, instantiate_template_bytes, open_archive_bytes, save_project_bytes,
};
use lumencraft_text::{TrueTypeFontInfo, parse_truetype_font_info};
use lumencraft_ui_egui::serde_json::{self, Value, json};
use lumencraft_ui_egui::theme::install_fonts;
pub use lumencraft_ui_egui::{
    CanvasPreviewFilter, KeyModifiers, PointerButton, RasterImage, RenderError, RenderTargetSpec, UiApp, UiEventOutcome, UiInputEvent,
    ViewportMetrics, VirtualWorkspaceFolder, WorkspaceHitTarget, WorkspaceLayoutMap,
};

/// Default logical viewport width used when [`WebWorkspaceHost::new`] starts.
pub const DEFAULT_VIEWPORT_WIDTH: f64 = 1280.0;
/// Default logical viewport height used when [`WebWorkspaceHost::new`] starts.
pub const DEFAULT_VIEWPORT_HEIGHT: f64 = 800.0;

/// Structured summary of the document currently loaded in [`WebWorkspaceHost`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSummary {
    pub title: String,
    pub format_tag: String,
    pub sheet_count: usize,
    pub data_sheet_count: usize,
    pub info_sheet_count: usize,
    pub result_sheet_count: usize,
    pub graph_sheet_count: usize,
    pub layout_sheet_count: usize,
    pub active_sheet_id: Option<String>,
    pub warnings: Vec<String>,
    pub dirty: bool,
}

/// Pure-Rust in-memory web & WASM host wrapping [`UiApp`], [`ViewportMetrics`], and
/// [`VirtualWorkspaceFolder`].
pub struct WebWorkspaceHost {
    app: UiApp,
    viewport: ViewportMetrics,
    format_tag: String,
    warnings: Vec<String>,
}

impl Default for WebWorkspaceHost {
    fn default() -> Self {
        Self::new()
    }
}

impl WebWorkspaceHost {
    /// Construct a fresh browser workspace host with bundled fonts installed and an empty session.
    #[must_use]
    pub fn new() -> Self {
        Self::with_viewport(ViewportMetrics::new(DEFAULT_VIEWPORT_WIDTH, DEFAULT_VIEWPORT_HEIGHT, 1.0))
    }

    /// Construct a browser workspace host initialized with explicit HiDPI [`ViewportMetrics`].
    #[must_use]
    pub fn with_viewport(viewport: ViewportMetrics) -> Self {
        let _fonts_loaded = install_fonts();
        let _ = lumencraft_ui_egui::bundled_fonts_enabled();
        let _ = lumencraft_format::EXTENSION;
        let mut app = UiApp::new();
        app.sync_views();
        Self { app, viewport, format_tag: String::from("empty"), warnings: Vec::new() }
    }

    /// Borrow the underlying headless UI application state.
    #[must_use]
    pub const fn app(&self) -> &UiApp {
        &self.app
    }

    /// Mutably borrow the underlying headless UI application state.
    pub fn app_mut(&mut self) -> &mut UiApp {
        &mut self.app
    }

    /// Return the active logical viewport and HiDPI scale metrics.
    #[must_use]
    pub const fn viewport(&self) -> ViewportMetrics {
        self.viewport
    }

    /// Update the logical viewport and HiDPI scale metrics.
    pub fn set_viewport(&mut self, viewport: ViewportMetrics) {
        self.viewport = viewport;
    }

    /// Inspect the active project document and host warnings.
    #[must_use]
    pub fn inspect_summary(&self) -> WorkspaceSummary {
        let doc = self.app.document();
        WorkspaceSummary {
            title: doc.title.clone(),
            format_tag: self.format_tag.clone(),
            sheet_count: doc.total_sheet_count(),
            data_sheet_count: doc.data_sheets.len(),
            info_sheet_count: doc.info_sheets.len(),
            result_sheet_count: doc.result_sheets.len(),
            graph_sheet_count: doc.graph_sheets.len(),
            layout_sheet_count: doc.layout_sheets.len(),
            active_sheet_id: doc.active_sheet_id.as_ref().map(|id| id.as_str().to_owned()),
            warnings: self.warnings.clone(),
            dirty: self.app.session().is_dirty(),
        }
    }

    /// Serialize an inspection payload covering document structure, UI state, and virtual files.
    #[must_use]
    pub fn inspect_json(&self) -> Value {
        let summary = self.inspect_summary();
        json!({
            "title": summary.title,
            "format": summary.format_tag,
            "sheet_count": summary.sheet_count,
            "data_sheets": summary.data_sheet_count,
            "info_sheets": summary.info_sheet_count,
            "result_sheets": summary.result_sheet_count,
            "graph_sheets": summary.graph_sheet_count,
            "layout_sheets": summary.layout_sheet_count,
            "active_sheet_id": summary.active_sheet_id,
            "warnings": summary.warnings,
            "dirty": summary.dirty,
            "preview_filter": self.app.preview_filter.as_str(),
            "presentation_mode": self.app.presentation_mode,
            "locale": self.app.locale,
            "virtual_root": self.app.virtual_workspace.root_name,
            "virtual_files": self.app.virtual_workspace.list_files(),
        })
    }

    /// Open a `.lumencraft` project or `.lctemplate` archive strictly from in-memory bytes.
    ///
    /// # Errors
    /// Returns an error description if archive decoding or schema validation fails.
    pub fn open_lumencraft_bytes(&mut self, bytes: &[u8]) -> Result<WorkspaceSummary, String> {
        let (kind, doc) = open_archive_bytes(bytes).map_err(|err| err.to_string())?;
        let tag = match kind {
            ArchiveKind::Project => "lumencraft",
            ArchiveKind::Template => "lctemplate",
        };
        self.install_session(Session::from_document(doc), tag, Vec::new(), true);
        Ok(self.inspect_summary())
    }

    /// Instantiate a `.lctemplate` archive from in-memory bytes with an optional project title.
    ///
    /// # Errors
    /// Returns an error description if template decoding or instantiation fails.
    pub fn open_template_bytes(&mut self, bytes: &[u8], title_override: Option<&str>) -> Result<WorkspaceSummary, String> {
        let doc = instantiate_template_bytes(bytes, title_override).map_err(|err| err.to_string())?;
        self.install_session(Session::from_document(doc), "lctemplate", Vec::new(), false);
        Ok(self.inspect_summary())
    }

    /// Open a `.pzfx` XML payload from in-memory bytes.
    ///
    /// # Errors
    /// Returns an error description if UTF-8 decoding or XML import fails.
    pub fn open_pzfx_bytes(&mut self, bytes: &[u8]) -> Result<WorkspaceSummary, String> {
        let report = import_pzfx_bytes(bytes).map_err(|err| err.to_string())?;
        self.apply_import_report(report, "pzfx");
        Ok(self.inspect_summary())
    }

    /// Open a `.prism` ZIP container payload from in-memory bytes.
    ///
    /// # Errors
    /// Returns an error description if archive extraction or table import fails.
    pub fn open_prism_bytes(&mut self, bytes: &[u8]) -> Result<WorkspaceSummary, String> {
        let report = import_prism_bytes(bytes).map_err(|err| err.to_string())?;
        self.apply_import_report(report, "prism");
        Ok(self.inspect_summary())
    }

    /// Open a JSON-serialized project document from in-memory bytes.
    ///
    /// # Errors
    /// Returns an error description if JSON deserialization fails.
    pub fn open_project_json_bytes(&mut self, bytes: &[u8]) -> Result<WorkspaceSummary, String> {
        let doc = serde_json::from_slice(bytes).map_err(|err| format!("invalid project JSON: {err}"))?;
        self.install_session(Session::from_document(doc), "json", Vec::new(), false);
        Ok(self.inspect_summary())
    }

    /// Replace the workspace with a fresh project created from in-memory CSV or TSV bytes.
    ///
    /// # Errors
    /// Returns an error description if `bytes` are not valid UTF-8 or CSV import fails.
    pub fn open_delimited_bytes(&mut self, bytes: &[u8], is_tsv: bool, title: Option<&str>, kind: Option<&str>) -> Result<WorkspaceSummary, String> {
        *self.app.session_mut() = Session::new();
        self.app.sync_views();
        self.warnings.clear();
        let summary = self.import_delimited_bytes(bytes, is_tsv, title, kind)?;
        self.format_tag = String::from(if is_tsv { "tsv" } else { "csv" });
        Ok(self.inspect_summary_with_format(summary))
    }

    /// Import in-memory CSV or TSV bytes as a new sheet inside the currently active project.
    ///
    /// # Errors
    /// Returns an error description if `bytes` are not valid UTF-8 or CSV import fails.
    pub fn import_delimited_bytes(
        &mut self,
        bytes: &[u8],
        is_tsv: bool,
        title: Option<&str>,
        kind: Option<&str>,
    ) -> Result<WorkspaceSummary, String> {
        let text = std::str::from_utf8(bytes).map_err(|err| format!("delimited bytes are not valid UTF-8: {err}"))?;
        let mut params = json!({
            "text": text,
            "title": title.unwrap_or(if is_tsv { "Imported TSV" } else { "Imported CSV" }),
            "kind": kind.unwrap_or("xy"),
        });
        if is_tsv && let Some(obj) = params.as_object_mut() {
            obj.insert(String::from("delimiter"), Value::String(String::from("\t")));
        }
        let outcome = self.app.execute_command("table.importCsv", params).map_err(|err| err.to_string())?;
        if let Some(sheet_id) = outcome.payload.get("sheet_id").and_then(Value::as_str) {
            let _ = self.app.execute_command("sheet.select", json!({ "sheet_id": sheet_id }));
        }
        if self.format_tag == "empty" {
            self.format_tag = String::from(if is_tsv { "tsv" } else { "csv" });
        }
        Ok(self.inspect_summary())
    }

    /// Auto-detect format from `file_name` (or sniff content bytes when extension is absent) and
    /// open the payload completely in memory.
    ///
    /// # Errors
    /// Returns an error description if the format is unsupported or decoding fails.
    pub fn open_bytes_for_name(&mut self, file_name: &str, bytes: &[u8]) -> Result<WorkspaceSummary, String> {
        let ext = file_name.rsplit_once('.').map(|(_, tail)| tail.to_ascii_lowercase());
        let stem = file_name.rsplit_once('/').map_or(file_name, |(_, tail)| tail);
        let stem_title = stem.rsplit_once('.').map_or(stem, |(head, _)| head);
        let clean_title = (!stem_title.trim().is_empty()).then_some(stem_title);

        match ext.as_deref() {
            Some("lumencraft" | "lctemplate") => self.open_lumencraft_bytes(bytes),
            Some("pzfx") => self.open_pzfx_bytes(bytes),
            Some("prism") => self.open_prism_bytes(bytes),
            Some("json") => self.open_project_json_bytes(bytes),
            Some("csv") => self.open_delimited_bytes(bytes, false, clean_title, Some("column")),
            Some("tsv") => self.open_delimited_bytes(bytes, true, clean_title, Some("column")),
            _ => self.sniff_and_open_bytes(bytes, clean_title),
        }
    }

    /// Borrow the in-memory `.lumencraft.d` virtual workspace folder.
    #[must_use]
    pub const fn virtual_workspace(&self) -> &VirtualWorkspaceFolder {
        &self.app.virtual_workspace
    }

    /// Mutably borrow the in-memory `.lumencraft.d` virtual workspace folder.
    pub fn virtual_workspace_mut(&mut self) -> &mut VirtualWorkspaceFolder {
        &mut self.app.virtual_workspace
    }

    /// Populate [`VirtualWorkspaceFolder`] with virtual `.lumencraft.d` folder entries strictly in
    /// memory (validating against path traversal) and open its project payload.
    ///
    /// # Errors
    /// Returns an error description if any relative path is unsafe or no loadable payload exists.
    pub fn load_virtual_folder_entries<I, K, V>(&mut self, root_name: impl Into<String>, entries: I) -> Result<WorkspaceSummary, String>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<Vec<u8>>,
    {
        let mut folder = VirtualWorkspaceFolder::new(root_name);
        for (rel_path, bytes) in entries {
            folder.write_file(rel_path.as_ref(), bytes.into())?;
        }
        self.app.virtual_workspace = folder;

        if self.app.virtual_workspace.contains("document.json") {
            return self.open_virtual_unpacked_snapshot();
        }
        let archive_entry = self.app.virtual_workspace.list_files().into_iter().find(|name| !name.ends_with(".csv") && name != "manifest.json");
        if let Some(entry_name) = archive_entry
            && let Some(bytes) = self.app.virtual_workspace.read_file(&entry_name).map(<[u8]>::to_vec)
        {
            return self.open_bytes_for_name(&entry_name, &bytes);
        }
        Err(String::from("virtual folder does not contain `document.json` or a supported project entry"))
    }

    /// Load the `document.json` snapshot stored inside [`Self::virtual_workspace`].
    ///
    /// # Errors
    /// Returns an error description if `document.json` is absent or invalid.
    pub fn open_virtual_unpacked_snapshot(&mut self) -> Result<WorkspaceSummary, String> {
        let doc = self.app.virtual_workspace.open_unpacked_snapshot()?;
        self.install_session(Session::from_document(doc), "lumencraft.d", Vec::new(), true);
        Ok(self.inspect_summary())
    }

    /// Persist the active project as an unpacked `.lumencraft.d` snapshot (`manifest.json`,
    /// `document.json`, and `sheets/<id>.csv`) inside [`Self::virtual_workspace`].
    ///
    /// # Errors
    /// Returns an error description if serialization fails.
    pub fn save_virtual_unpacked_snapshot(&mut self) -> Result<usize, String> {
        let mut folder = std::mem::take(&mut self.app.virtual_workspace);
        let save_result = folder.save_unpacked_snapshot(self.app.document());
        self.app.virtual_workspace = folder;
        let total = save_result?;
        self.app.session_mut().set_file_path(None, true);
        self.format_tag = String::from("lumencraft.d");
        Ok(total)
    }

    /// Save the active project as an in-memory `.lumencraft` archive entry inside
    /// [`Self::virtual_workspace`].
    ///
    /// # Errors
    /// Returns an error description if serialization or relative-path validation fails.
    pub fn save_virtual_archive(&mut self, file_name: &str) -> Result<usize, String> {
        self.app.save_to_virtual_workspace(file_name)
    }

    /// Load a `.lumencraft` archive entry previously saved in [`Self::virtual_workspace`].
    ///
    /// # Errors
    /// Returns an error description if `file_name` is missing or invalid.
    pub fn open_virtual_archive(&mut self, file_name: &str) -> Result<WorkspaceSummary, String> {
        self.app.load_from_virtual_workspace(file_name)?;
        self.app.session_mut().set_file_path(None, true);
        self.format_tag = String::from("lumencraft");
        self.warnings.clear();
        Ok(self.inspect_summary())
    }

    /// Serialize the active project into `.lumencraft` ZIP bytes purely in memory.
    ///
    /// # Errors
    /// Returns an error description if archive serialization fails.
    pub fn export_lumencraft_bytes(&self) -> Result<Vec<u8>, String> {
        save_project_bytes(self.app.document()).map_err(|err| err.to_string())
    }

    /// Refresh the unpacked `.lumencraft.d` virtual snapshot and return `(relative_path, bytes)`
    /// entries for browser persistence or download.
    ///
    /// # Errors
    /// Returns an error description if snapshot serialization fails.
    pub fn export_virtual_entries(&mut self) -> Result<Vec<(String, Vec<u8>)>, String> {
        let _ = self.save_virtual_unpacked_snapshot()?;
        Ok(self.app.virtual_workspace.files.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
    }

    /// Execute a structured engine command and return its serialized payload.
    ///
    /// # Errors
    /// Returns an error description if engine execution or JSON conversion fails.
    pub fn execute_command(&mut self, command_id: &str, params: Value) -> Result<Value, String> {
        let outcome = self.app.execute_command(command_id, params).map_err(|err| err.to_string())?;
        serde_json::to_value(outcome).map_err(|err| format!("failed to serialize command outcome: {err}"))
    }

    /// Process one JSON-lines control-protocol request line on [`UiApp`].
    #[must_use]
    pub fn dispatch_control_line(&mut self, line: &str) -> String {
        self.app.dispatch_control_line(line)
    }

    /// Dispatch an interactive [`UiInputEvent`] using the host's active [`ViewportMetrics`].
    ///
    /// # Errors
    /// Returns an error description if a triggered command fails.
    pub fn dispatch_event(&mut self, event: UiInputEvent) -> Result<UiEventOutcome, String> {
        self.app.dispatch_ui_event(event, self.viewport)
    }

    /// Dispatch an interactive [`UiInputEvent`] under explicit [`ViewportMetrics`].
    ///
    /// # Errors
    /// Returns an error description if a triggered command fails.
    pub fn dispatch_event_with_metrics(&mut self, event: UiInputEvent, metrics: ViewportMetrics) -> Result<UiEventOutcome, String> {
        self.viewport = metrics;
        self.app.dispatch_ui_event(event, metrics)
    }

    /// Compute the deterministic HiDPI layout and hit-testing geometry for the current viewport.
    #[must_use]
    pub fn layout_map(&self) -> WorkspaceLayoutMap {
        self.app.layout_map(self.viewport)
    }

    /// Hit-test logical coordinates `(x, y)` against the current [`WorkspaceLayoutMap`].
    #[must_use]
    pub fn hit_test_logical(&self, x: f64, y: f64) -> WorkspaceHitTarget {
        self.layout_map().hit_test_logical(x, y)
    }

    /// Select the active color-vision deficiency or grayscale print preview filter.
    pub fn set_preview_filter(&mut self, filter: CanvasPreviewFilter) {
        self.app.preview_filter = filter;
    }

    /// Return the currently active [`CanvasPreviewFilter`].
    #[must_use]
    pub const fn preview_filter(&self) -> CanvasPreviewFilter {
        self.app.preview_filter
    }

    /// Explicitly enable or disable full-canvas presentation mode (`presentation_mode`).
    pub fn set_presentation_mode(&mut self, enabled: bool) {
        self.app.presentation_mode = enabled;
    }

    /// Toggle `presentation_mode` and return the resulting state.
    pub fn toggle_presentation_mode(&mut self) -> bool {
        self.app.presentation_mode = !self.app.presentation_mode;
        self.app.presentation_mode
    }

    /// Return whether `presentation_mode` is currently enabled.
    #[must_use]
    pub const fn presentation_mode(&self) -> bool {
        self.app.presentation_mode
    }

    /// Render the workspace framebuffer at the host's [`ViewportMetrics`] and `base_dpi`.
    ///
    /// # Errors
    /// Returns [`RenderError`] if rasterization fails.
    pub fn render_rgba(&self, base_dpi: u16) -> Result<RasterImage, RenderError> {
        self.app.render_rgba_for_metrics(self.viewport, base_dpi)
    }

    /// Render the workspace framebuffer using an explicit [`RenderTargetSpec`].
    ///
    /// # Errors
    /// Returns [`RenderError`] if rasterization fails.
    pub fn render_rgba_target(&self, target: RenderTargetSpec) -> Result<RasterImage, RenderError> {
        self.app.render_rgba(target)
    }

    /// Parse and validate TrueType/OpenType SFNT font bytes strictly in memory.
    ///
    /// # Errors
    /// Returns an error description if the font header or tables are malformed.
    pub fn validate_font_bytes(&self, sfnt_bytes: &[u8]) -> Result<TrueTypeFontInfo, String> {
        parse_truetype_font_info(sfnt_bytes).map_err(|err| err.to_string())
    }

    fn apply_import_report(&mut self, report: ImportReport, tag: &str) {
        let warnings = report.warnings.into_iter().map(|w| w.message).collect();
        self.install_session(Session::from_document(report.document), tag, warnings, false);
    }

    fn install_session(&mut self, mut session: Session, tag: &str, warnings: Vec<String>, mark_clean: bool) {
        if mark_clean {
            session.set_file_path(None, true);
        }
        *self.app.session_mut() = session;
        self.app.sync_views();
        self.format_tag = String::from(tag);
        self.warnings = warnings;
    }

    fn inspect_summary_with_format(&self, mut summary: WorkspaceSummary) -> WorkspaceSummary {
        summary.format_tag = self.format_tag.clone();
        summary
    }

    fn sniff_and_open_bytes(&mut self, bytes: &[u8], title: Option<&str>) -> Result<WorkspaceSummary, String> {
        if bytes.starts_with(b"PK") {
            return self.open_lumencraft_bytes(bytes).or_else(|_| self.open_prism_bytes(bytes));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| String::from("unrecognized binary project payload"))?;
        let trimmed = text.trim_start();
        if trimmed.starts_with('<') && trimmed.contains("PrismFile") {
            return self.open_pzfx_bytes(bytes);
        }
        if trimmed.starts_with('{') {
            return self.open_project_json_bytes(bytes);
        }
        if !trimmed.is_empty() {
            let looks_tsv = trimmed.lines().next().is_some_and(|line| line.contains('\t') && !line.contains(','));
            return self.open_delimited_bytes(bytes, looks_tsv, title, Some("column"));
        }
        Err(String::from("empty project payload"))
    }
}

#[cfg(test)]
mod tests {
    use lumencraft_format::{CsvOptions, ZipWriter, export_datasheet_to_csv};
    use lumencraft_ui_egui::serde_json::json;

    use super::{
        CanvasPreviewFilter, KeyModifiers, PointerButton, UiInputEvent, ViewportMetrics, VirtualWorkspaceFolder, WebWorkspaceHost, WorkspaceHitTarget,
    };

    fn build_populated_host() -> WebWorkspaceHost {
        let mut host = WebWorkspaceHost::with_viewport(ViewportMetrics::new(1280.0, 800.0, 2.0));
        let summary = host
            .open_delimited_bytes(b"Concentration,Response\n1.0,12.5\n2.0,25.0\n4.0,50.0\n", false, Some("Dose Assay"), Some("xy"))
            .expect("CSV import must succeed");
        assert_eq!(summary.data_sheet_count, 1);

        let sheet_id = summary.active_sheet_id.expect("imported CSV becomes active sheet");
        let graph =
            host.execute_command("graph.create", json!({ "source_sheet_id": sheet_id, "title": "Dose Plot" })).expect("graph.create must succeed");
        assert!(graph.get("payload").is_some());
        assert_eq!(host.app().document().graph_sheets.len(), 1);
        host
    }

    fn build_synthetic_prism_zip() -> Vec<u8> {
        let mut writer = ZipWriter::new();
        writer
            .add_file(
                "manifest.json",
                br#"{"title":"Synthetic Prism Study","sheets":[{"id":"tbl-1","title":"Group Comparison","kind":"column","csv_path":"sheets/tbl-1.csv"}]}"#
                    .to_vec(),
            )
            .expect("add manifest.json");
        writer.add_file("sheets/tbl-1.csv", b"Control,Treated\n1.1,2.4\n1.5,2.9\n".to_vec()).expect("add sheet csv");
        writer.finish().expect("finish synthetic prism zip")
    }

    #[test]
    fn opens_lumencraft_archive_template_pzfx_prism_and_delimited_in_memory() {
        let source = build_populated_host();
        let archive_bytes = source.export_lumencraft_bytes().expect("export .lumencraft bytes");

        // 1. `.lumencraft` and `.lctemplate` archive bytes round-trip
        let mut archive_host = WebWorkspaceHost::new();
        let sum = archive_host.open_bytes_for_name("assay.lumencraft", &archive_bytes).expect("open .lumencraft");
        assert_eq!(sum.format_tag, "lumencraft");
        assert_eq!(sum.data_sheet_count, 1);
        assert_eq!(sum.graph_sheet_count, 1);

        let template_bytes = lumencraft_format::save_template_bytes(source.app().document()).expect("export .lctemplate bytes");
        let mut tpl_host = WebWorkspaceHost::new();
        let tpl_sum = tpl_host.open_template_bytes(&template_bytes, Some("Template Assay")).expect("instantiate .lctemplate bytes");
        assert_eq!(tpl_sum.format_tag, "lctemplate");
        assert_eq!(tpl_sum.title, "Template Assay");
        assert_eq!(tpl_sum.data_sheet_count, 1);

        // 2. `.tsv` and `.csv` in-memory open + import
        let mut tsv_host = WebWorkspaceHost::new();
        let tsv_sum = tsv_host.open_bytes_for_name("plate.tsv", b"Alpha\tBeta\n3.5\t7.0\n4.5\t8.0\n").expect("open .tsv");
        assert_eq!(tsv_sum.format_tag, "tsv");
        assert_eq!(tsv_sum.data_sheet_count, 1);
        let csv_text = export_datasheet_to_csv(&tsv_host.app().document().data_sheets[0], &CsvOptions::csv()).expect("export table to csv");
        assert!(csv_text.contains("3.5"));
        assert!(csv_text.contains("7"));

        let imported = tsv_host.import_delimited_bytes(b"X,Y\n1,2\n3,4\n", false, Some("Extra Sheet"), Some("xy")).expect("import CSV sheet");
        assert_eq!(imported.data_sheet_count, 2);

        // 3. `.pzfx` clean-room XML bytes
        let pzfx_xml = br#"<PrismFile><Title>PZFX Web Study</Title><Table ID="t1" TableType="XY"><Title>Curve Data</Title><XColumn><Title>X</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn><YColumn><Title>Y</Title><Subcolumn><d>10</d><d>20</d></Subcolumn></YColumn></Table></PrismFile>"#;
        let mut pzfx_host = WebWorkspaceHost::new();
        let pzfx_sum = pzfx_host.open_bytes_for_name("study.pzfx", pzfx_xml).expect("open .pzfx bytes");
        assert_eq!(pzfx_sum.format_tag, "pzfx");
        assert_eq!(pzfx_sum.title, "PZFX Web Study");
        assert_eq!(pzfx_sum.data_sheet_count, 1);

        // 4. `.prism` ZIP archive bytes + extensionless sniffing
        let prism_bytes = build_synthetic_prism_zip();
        let mut prism_host = WebWorkspaceHost::new();
        let prism_sum = prism_host.open_bytes_for_name("experiment.prism", &prism_bytes).expect("open .prism bytes");
        assert_eq!(prism_sum.format_tag, "prism");
        assert_eq!(prism_sum.title, "Synthetic Prism Study");
        assert_eq!(prism_sum.data_sheet_count, 1);

        let mut sniff_host = WebWorkspaceHost::new();
        let sniff_sum = sniff_host.open_bytes_for_name("blob", &archive_bytes).expect("sniff PK archive bytes");
        assert_eq!(sniff_sum.format_tag, "lumencraft");

        assert!(sniff_host.validate_font_bytes(b"not-a-font").is_err());
    }

    #[test]
    fn virtual_workspace_folder_saves_loads_edits_and_rejects_traversal() {
        let mut host = build_populated_host();
        assert!(host.virtual_workspace_mut().write_text("../escape.txt", "blocked").is_err());

        let snapshot_bytes = host.save_virtual_unpacked_snapshot().expect("save virtual unpacked snapshot");
        assert!(snapshot_bytes > 0);
        let files = host.virtual_workspace().list_files();
        assert!(files.iter().any(|p| p == "manifest.json"));
        assert!(files.iter().any(|p| p == "document.json"));
        assert!(files.iter().any(|p| p.starts_with("sheets/") && p.ends_with(".csv")));

        let archive_size = host.save_virtual_archive("backup.lumencraft").expect("save virtual archive");
        assert!(archive_size > 0);

        let exported_entries = host.export_virtual_entries().expect("export virtual entries");
        let mut reloaded_host = WebWorkspaceHost::new();
        let summary =
            reloaded_host.load_virtual_folder_entries("assay.lumencraft.d", exported_entries).expect("load .lumencraft.d virtual folder entries");
        assert_eq!(summary.format_tag, "lumencraft.d");
        assert_eq!(summary.data_sheet_count, 1);
        assert_eq!(summary.graph_sheet_count, 1);
        assert_eq!(VirtualWorkspaceFolder::normalize_rel_path("sheets/data-1.csv").as_deref(), Ok("sheets/data-1.csv"));

        // Edit in reloaded host, save virtual archive, then reopen cleanly from virtual archive.
        let sheet_id = reloaded_host.app().document().data_sheets[0].id.as_str().to_owned();
        reloaded_host.execute_command("sheet.rename", json!({ "sheet_id": sheet_id, "title": "Edited Virtual Table" })).expect("rename sheet");
        reloaded_host.save_virtual_archive("edited.lumencraft").expect("save virtual archive after edit");
        let reopened_archive = reloaded_host.open_virtual_archive("edited.lumencraft").expect("reopen virtual archive");
        assert_eq!(reopened_archive.data_sheet_count, 1);
        assert_eq!(reloaded_host.app().document().data_sheets[0].title, "Edited Virtual Table");
    }

    #[test]
    fn preview_filter_presentation_mode_and_hidpi_rgba_render_behave_accurately() {
        let mut host = build_populated_host();
        // Ensure active sheet is the graph so preview filter transforms plot pixels visibly.
        let graph_id = host.app().document().graph_sheets[0].id.as_str().to_owned();
        host.execute_command("sheet.select", json!({ "sheet_id": graph_id })).expect("select graph sheet");

        let baseline = host.render_rgba(72).expect("baseline HiDPI RGBA render");
        assert_eq!(baseline.width_px, 2560);
        assert_eq!(baseline.height_px, 1600);
        assert_eq!(baseline.rgba.len(), (2560 * 1600 * 4) as usize);

        let layout_normal = host.layout_map();
        assert!(layout_normal.frame.navigator.width > 0);
        assert!(!layout_normal.navigator_entries.is_empty());

        let filter_reply = host.dispatch_control_line(r#"{"id":"f1","method":"ui.previewFilter.set","params":{"filter":"grayscale_print"}}"#);
        let filter_json: lumencraft_ui_egui::serde_json::Value =
            lumencraft_ui_egui::serde_json::from_str(&filter_reply).expect("valid JSON control reply");
        assert_eq!(filter_json.get("ok"), Some(&json!(true)), "unexpected filter reply: {filter_reply}");
        assert_eq!(host.preview_filter(), CanvasPreviewFilter::GrayscalePrint);
        let grayscale = host.render_rgba(72).expect("grayscale RGBA render");
        assert_ne!(baseline.rgba, grayscale.rgba, "preview filter must visibly alter rendered framebuffer");

        host.set_preview_filter(CanvasPreviewFilter::Deuteranopia);
        assert_eq!(host.preview_filter(), CanvasPreviewFilter::Deuteranopia);

        // Toggle presentation mode: collapses navigator chrome so canvas expands to full viewport.
        assert!(!host.presentation_mode());
        let toggled = host.toggle_presentation_mode();
        assert!(toggled);
        let layout_pres = host.layout_map();
        assert_eq!(layout_pres.frame.navigator.width, 0);
        assert!(layout_pres.navigator_entries.is_empty());
        assert_eq!(layout_pres.frame.canvas.width, 2560);

        let pres_rgba = host.render_rgba(72).expect("presentation mode RGBA render");
        assert_ne!(grayscale.rgba, pres_rgba.rgba);

        let pres_off_reply = host.dispatch_control_line(r#"{"id":"p1","method":"ui.presentation.toggle","params":{"enabled":false}}"#);
        assert!(pres_off_reply.contains("\"presentation_mode\":false"));
        assert!(!host.presentation_mode());
        assert!(host.inspect_json().get("virtual_root").is_some());
    }

    #[test]
    fn interactive_pointer_hit_testing_and_inline_cell_edit_events_update_document() {
        let mut host = build_populated_host();
        let data_id = host.app().document().data_sheets[0].id.as_str().to_owned();
        host.execute_command("sheet.select", json!({ "sheet_id": data_id })).expect("select data sheet");

        let layout = host.layout_map();
        let grid = layout.grid_geometry.as_ref().expect("data sheet renders interactive grid geometry");
        let &(first_flat_col, _) = grid.visible_cols.first().expect("at least one data column must be visible");
        let cell_rect = grid.cell_rect(0, first_flat_col).expect("first data cell (row 0) must be visible");
        let phys_center = [f64::from(cell_rect.x) + f64::from(cell_rect.width) * 0.5, f64::from(cell_rect.y) + f64::from(cell_rect.height) * 0.5];
        let [lx, ly] = host.viewport().physical_to_logical_px(phys_center);

        assert!(matches!(host.hit_test_logical(lx, ly), WorkspaceHitTarget::GridCell { .. }));

        let down = host
            .dispatch_event(UiInputEvent::PointerDown { x: lx, y: ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() })
            .expect("PointerDown succeeds");
        assert!(down.handled);
        assert!(host.app().brushed_cell.is_some());

        let up = host
            .dispatch_event(UiInputEvent::PointerUp { x: lx, y: ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() })
            .expect("PointerUp succeeds");
        assert!(up.handled);

        // Double-click cell, clear original value via Ctrl/Meta+A -> Backspace -> type replacement -> CommitInlineEdit.
        let dbl = host
            .dispatch_event(UiInputEvent::DoubleClick { x: lx, y: ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() })
            .expect("DoubleClick enters inline editor");
        assert_eq!(dbl.action, "grid.inline_edit_begin");
        assert!(host.app().inline_editor.active);

        // Clear buffer cleanly via KeyDown("Escape") cancel test, then re-open and replace buffer via TextInput.
        host.dispatch_event(UiInputEvent::CancelInlineEdit).expect("CancelInlineEdit");
        assert!(!host.app().inline_editor.active);

        host.dispatch_event(UiInputEvent::DoubleClick { x: lx, y: ly, button: PointerButton::Primary, modifiers: KeyModifiers::default() })
            .expect("DoubleClick re-enters inline editor");
        for _ in 0..16 {
            host.dispatch_event(UiInputEvent::KeyDown { key: String::from("Backspace"), modifiers: KeyModifiers::default() })
                .expect("Backspace succeeds");
        }
        host.dispatch_event(UiInputEvent::TextInput { text: String::from("99.5") }).expect("TextInput succeeds");
        let commit = host.dispatch_event(UiInputEvent::CommitInlineEdit).expect("CommitInlineEdit succeeds");
        assert!(commit.handled);
        assert!(!host.app().inline_editor.active);

        let updated_csv = export_datasheet_to_csv(&host.app().document().data_sheets[0], &CsvOptions::csv()).expect("export updated CSV");
        assert!(updated_csv.contains("99.5"), "expected inline edit 99.5 in exported CSV:\n{updated_csv}");

        // Graph sheet interactive zoom & inertial pan event dispatch
        let graph_id = host.app().document().graph_sheets[0].id.as_str().to_owned();
        host.execute_command("sheet.select", json!({ "sheet_id": graph_id })).expect("select graph sheet");
        let graph_layout = host.layout_map();
        let paper = graph_layout.scene_paper.as_ref().expect("graph sheet provides ScenePaperMapping");
        let paper_center =
            [f64::from(paper.paper.x) + f64::from(paper.paper.width) * 0.5, f64::from(paper.paper.y) + f64::from(paper.paper.height) * 0.5];
        let [gx, gy] = host.viewport().physical_to_logical_px(paper_center);
        let zoom_out = host
            .dispatch_event(UiInputEvent::WheelZoom { x: gx, y: gy, delta_y: 120.0, modifiers: KeyModifiers::default() })
            .expect("WheelZoom succeeds");
        assert!(zoom_out.handled);
        let pan_out = host
            .dispatch_event(UiInputEvent::InertialPan { delta_x: 24.0, delta_y: -12.0, dt_sec: 0.016, step_coast: true })
            .expect("InertialPan succeeds");
        assert!(pan_out.handled);
    }
}
