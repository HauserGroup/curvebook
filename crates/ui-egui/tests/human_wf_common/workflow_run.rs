//! Five-stage GUI workflow completion harness (`R10.26`).
//!
//! [`WorkflowRun`] wraps [`EguiWfHarness`] privately and enforces the strict five-stage contract:
//! 1. **Create**: starting from an empty `UiApp`, create sheets via Welcome Hub or GUI menus.
//! 2. **Enter**: populate grid cells, titles, clipboard TSV paste, or Power Wizard inputs via real events.
//! 3. **Configure**: run Analyze / Format / Power Studio dialogs and commit via `modal.ok` / `modal.apply`.
//! 4. **Verify**: call [`WorkflowRun::verify_reference`] to cross-check GUI results against a fresh
//!    headless `curvebook_engine::Session` over the same dataset within documented tolerances (`D1`).
//! 5. **Live edit**: call [`WorkflowRun::live_edit`] to perform an upstream GUI edit and assert
//!    `ResultStatus::UpToDate` plus expected scalar, tab-cell, or compiled-SVG deltas/invariance.
//!
//! Bypasses (`from_app`, `app_mut`, `state_mut`, `dispatch_gui_action`) are intentionally absent.

use std::sync::Arc;
use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};

use curvebook_chart::compile_graph;
use curvebook_doc::{CiBound, DataSheet, GraphSheet, ProjectDocument, ResultCell, ResultSheet, ResultStatus, SheetId};
use curvebook_engine::Session;
use curvebook_svg::{SvgExportOptions, render_svg};
use curvebook_testkit::metrics::approx_eq;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::FrameInspectMeta;
use curvebook_ui_egui::inspect::ScreenshotRegionTarget;
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton, ViewportMetrics};
use curvebook_ui_egui::os_integration::StubPlatformClipboardAdapter;
use egui::{Key, Modifiers, Pos2, Vec2};

use super::{EguiWfHarness, TestRes, WfShellState, assert_input_to_paint_budget, assert_zero_shortcut_compliance, default_viewport, ensure};

static PANIC_HOOK_ONCE: Once = Once::new();
static BG_PANIC_COUNT: AtomicUsize = AtomicUsize::new(0);

fn install_bg_panic_hook() {
    PANIC_HOOK_ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let cur = std::thread::current();
            let is_bg_worker = match cur.name() {
                None => true,
                Some(name) => name.starts_with("cb-") || name.starts_with("raster") || name.starts_with("worker"),
            };
            if is_bg_worker {
                BG_PANIC_COUNT.fetch_add(1, Ordering::SeqCst);
            }
            prev(info);
        }));
    });
}

/// Ordered completion stages required by `WorkflowRun::finish`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkflowStages {
    pub created: bool,
    pub entered: bool,
    pub configured: bool,
    pub verified: bool,
    pub live_edited: bool,
}

/// Named scalar expectation comparing a GUI `ResultSheet` against a headless `Session` oracle.
#[derive(Clone, Debug)]
pub struct ReferenceScalar {
    pub key: String,
    pub data_set_index: Option<usize>,
    pub abs_tol: f64,
    pub rel_tol: f64,
}

/// Indexed result-tab cell expectation comparing a GUI `ResultSheet` against a headless `Session`.
#[derive(Clone, Debug)]
pub struct ReferenceTabCell {
    pub tab_index: usize,
    pub row: usize,
    pub col: usize,
    pub abs_tol: f64,
    pub rel_tol: f64,
}

/// Builder callback executed on a fresh headless `Session` during [`WorkflowRun::verify_reference`].
pub type ReferenceBuilderFn<'a> = Box<dyn FnOnce(&mut Session) -> Result<(), String> + 'a>;

/// Custom comparison callback between GUI and headless reference [`GraphSheet`] specs.
pub type GraphCheckFn<'a> = Box<dyn FnOnce(&GraphSheet, &GraphSheet) -> TestRes + 'a>;

/// Declarative headless-Session oracle check passed to [`WorkflowRun::verify_reference`].
pub struct ReferenceSpec<'a> {
    pub gui_result_id: Option<SheetId>,
    pub gui_graph_id: Option<SheetId>,
    pub ref_result_id: Option<SheetId>,
    pub ref_graph_id: Option<SheetId>,
    pub initial_doc: Option<ProjectDocument>,
    pub scalars: Vec<ReferenceScalar>,
    pub tab_cells: Vec<ReferenceTabCell>,
    pub build_reference: ReferenceBuilderFn<'a>,
    pub check_graph: Option<GraphCheckFn<'a>>,
}

impl<'a> ReferenceSpec<'a> {
    /// Construct a verification spec anchored on GUI [`ResultSheet`] `gui_result_id`.
    #[must_use]
    pub fn for_result(gui_result_id: SheetId, build_reference: impl FnOnce(&mut Session) -> Result<(), String> + 'a) -> Self {
        Self {
            gui_result_id: Some(gui_result_id),
            gui_graph_id: None,
            ref_result_id: None,
            ref_graph_id: None,
            initial_doc: None,
            scalars: Vec::new(),
            tab_cells: Vec::new(),
            build_reference: Box::new(build_reference),
            check_graph: None,
        }
    }

    /// Construct a verification spec anchored on GUI [`GraphSheet`] `gui_graph_id`.
    #[must_use]
    pub fn for_graph(gui_graph_id: SheetId, build_reference: impl FnOnce(&mut Session) -> Result<(), String> + 'a) -> Self {
        Self {
            gui_result_id: None,
            gui_graph_id: Some(gui_graph_id),
            ref_result_id: None,
            ref_graph_id: None,
            initial_doc: None,
            scalars: Vec::new(),
            tab_cells: Vec::new(),
            build_reference: Box::new(build_reference),
            check_graph: None,
        }
    }

    /// Pre-load a [`DataSheet`] into the fresh reference [`Session`] so tests do not need `Session::from_document` directly.
    pub fn with_reference_sheet(mut self, sheet: DataSheet) -> Result<Self, String> {
        let mut doc = self.initial_doc.take().unwrap_or_default();
        doc.add_data_sheet(sheet).map_err(|e| e.to_string())?;
        self.initial_doc = Some(doc);
        Ok(self)
    }

    /// Pre-load an initial [`ProjectDocument`] into the reference [`Session`].
    #[must_use]
    pub fn with_reference_document(mut self, doc: ProjectDocument) -> Self {
        self.initial_doc = Some(doc);
        self
    }

    /// Optionally override the target [`ResultSheet`] id in the reference [`Session`].
    #[must_use]
    pub fn ref_result_id(mut self, id: SheetId) -> Self {
        self.ref_result_id = Some(id);
        self
    }

    /// Also verify GUI [`GraphSheet`] `gui_graph_id` in this reference check.
    #[must_use]
    pub fn with_graph(mut self, gui_graph_id: SheetId) -> Self {
        self.gui_graph_id = Some(gui_graph_id);
        self
    }

    /// Optionally override the target [`GraphSheet`] id in the reference [`Session`].
    #[must_use]
    pub fn ref_graph_id(mut self, id: SheetId) -> Self {
        self.ref_graph_id = Some(id);
        self
    }

    /// Add a named scalar comparison (`lookup_scalar(key, data_set_index)`) with `(abs_tol, rel_tol)`.
    #[must_use]
    pub fn scalar(mut self, key: impl Into<String>, data_set_index: Option<usize>, abs_tol: f64, rel_tol: f64) -> Self {
        self.scalars.push(ReferenceScalar { key: key.into(), data_set_index, abs_tol, rel_tol });
        self
    }

    /// Add a tab-cell comparison at `(tab_index, row, col)` with `(abs_tol, rel_tol)`.
    #[must_use]
    pub fn tab_cell(mut self, tab_index: usize, row: usize, col: usize, abs_tol: f64, rel_tol: f64) -> Self {
        self.tab_cells.push(ReferenceTabCell { tab_index, row, col, abs_tol, rel_tol });
        self
    }

    /// Attach a custom [`GraphSheet`] comparison hook between GUI and headless reference graphs.
    #[must_use]
    pub fn check_graph(mut self, f: impl FnOnce(&GraphSheet, &GraphSheet) -> TestRes + 'a) -> Self {
        self.check_graph = Some(Box::new(f));
        self
    }
}

/// Snapshot of scalars, result tab cells, and compiled graph SVGs captured around [`WorkflowRun::live_edit`].
#[derive(Clone, Debug)]
pub struct LiveEditSnapshot {
    pub scalars: Vec<(SheetId, String, Option<usize>, f64)>,
    pub tab_cells: Vec<(SheetId, usize, usize, usize, ResultCell)>,
    pub graph_svgs: Vec<(SheetId, String)>,
}

impl LiveEditSnapshot {
    fn capture(doc: &ProjectDocument) -> Result<Self, String> {
        let mut scalars = Vec::new();
        let mut tab_cells = Vec::new();
        for rs in &doc.result_sheets {
            for s in &rs.scalars {
                scalars.push((rs.id.clone(), s.key.clone(), s.data_set_index, s.value));
            }
            for (t_idx, tab) in rs.tabs.iter().enumerate() {
                for (r_idx, row) in tab.rows.iter().enumerate() {
                    for (c_idx, cell) in row.iter().enumerate() {
                        tab_cells.push((rs.id.clone(), t_idx, r_idx, c_idx, cell.clone()));
                    }
                }
            }
        }
        let mut graph_svgs = Vec::new();
        let svg_opts = SvgExportOptions::default();
        for gs in &doc.graph_sheets {
            let scene = compile_graph(gs, doc).map_err(|e| format!("compile_graph `{}` failed: {e}", gs.id.as_str()))?;
            let svg = render_svg(&scene, &svg_opts).map_err(|e| format!("render_svg `{}` failed: {e}", gs.id.as_str()))?;
            graph_svgs.push((gs.id.clone(), svg));
        }
        Ok(Self { scalars, tab_cells, graph_svgs })
    }

    /// Look up a captured scalar by `(result_id, key, data_set_index)`.
    #[must_use]
    pub fn scalar(&self, result_id: &SheetId, key: &str, data_set_index: Option<usize>) -> Option<f64> {
        let needle = key.trim();
        self.scalars
            .iter()
            .find(|(id, k, ds, _)| id == result_id && k.trim().eq_ignore_ascii_case(needle) && (data_set_index.is_none() || *ds == data_set_index))
            .map(|(_, _, _, v)| *v)
            .filter(|v| v.is_finite())
    }

    /// Look up a captured [`ResultCell`] by `(result_id, tab_index, row, col)`.
    #[must_use]
    pub fn tab_cell(&self, result_id: &SheetId, tab_index: usize, row: usize, col: usize) -> Option<&ResultCell> {
        self.tab_cells.iter().find(|(id, t, r, c, _)| id == result_id && *t == tab_index && *r == row && *c == col).map(|(_, _, _, _, cell)| cell)
    }

    /// Look up compiled SVG text for [`GraphSheet`] `graph_id`.
    #[must_use]
    pub fn graph_svg(&self, graph_id: &SheetId) -> Option<&str> {
        self.graph_svgs.iter().find(|(id, _)| id == graph_id).map(|(_, svg)| svg.as_str())
    }
}

/// Upstream GUI edit applied during [`WorkflowRun::live_edit`].
pub enum LiveEditTarget {
    GridCell { sheet_id: Option<SheetId>, row: usize, col: usize, subcol: usize, text: String },
    XCell { sheet_id: Option<SheetId>, row: usize, subcol: usize, text: String },
    RowTitle { sheet_id: Option<SheetId>, row: usize, title: String },
    ColumnTitle { sheet_id: Option<SheetId>, col: usize, title: String },
    GuiAction(Box<dyn FnOnce(&mut WorkflowRun) -> TestRes>),
}

impl LiveEditTarget {
    /// Edit cell `(row, col, subcol)` on the currently active sheet.
    #[must_use]
    pub fn grid_cell(row: usize, col: usize, subcol: usize, text: impl Into<String>) -> Self {
        Self::GridCell { sheet_id: None, row, col, subcol, text: text.into() }
    }

    /// Select `sheet_id` then edit cell `(row, col, subcol)`.
    #[must_use]
    pub fn sheet_grid_cell(sheet_id: SheetId, row: usize, col: usize, subcol: usize, text: impl Into<String>) -> Self {
        Self::GridCell { sheet_id: Some(sheet_id), row, col, subcol, text: text.into() }
    }

    /// Edit X cell `(row, subcol)` on the currently active sheet.
    #[must_use]
    pub fn x_cell(row: usize, subcol: usize, text: impl Into<String>) -> Self {
        Self::XCell { sheet_id: None, row, subcol, text: text.into() }
    }

    /// Select `sheet_id` then edit X cell `(row, subcol)`.
    #[must_use]
    pub fn sheet_x_cell(sheet_id: SheetId, row: usize, subcol: usize, text: impl Into<String>) -> Self {
        Self::XCell { sheet_id: Some(sheet_id), row, subcol, text: text.into() }
    }

    /// Select `sheet_id` then edit row title `row`.
    #[must_use]
    pub fn sheet_row_title(sheet_id: SheetId, row: usize, title: impl Into<String>) -> Self {
        Self::RowTitle { sheet_id: Some(sheet_id), row, title: title.into() }
    }

    /// Select `sheet_id` then edit column title `col`.
    #[must_use]
    pub fn sheet_column_title(sheet_id: SheetId, col: usize, title: impl Into<String>) -> Self {
        Self::ColumnTitle { sheet_id: Some(sheet_id), col, title: title.into() }
    }

    /// Perform an arbitrary sequence of strict [`WorkflowRun`] GUI actions as the upstream edit.
    #[must_use]
    pub fn gui(f: impl FnOnce(&mut WorkflowRun) -> TestRes + 'static) -> Self {
        Self::GuiAction(Box::new(f))
    }
}

/// Custom assertion callback comparing before/after [`LiveEditSnapshot`]s and live [`ProjectDocument`].
pub type LiveEditCustomFn = Box<dyn FnOnce(&LiveEditSnapshot, &LiveEditSnapshot, &ProjectDocument) -> TestRes>;

/// Assertion verified after [`WorkflowRun::live_edit`] recomputation settles.
pub enum LiveEditExpectation {
    ScalarChanged {
        result_id: SheetId,
        key: String,
        data_set_index: Option<usize>,
        min_delta: f64,
    },
    TabCellChanged {
        result_id: SheetId,
        tab_index: usize,
        row: usize,
        col: usize,
    },
    Invariant {
        result_id: Option<SheetId>,
        scalar_key: Option<(String, Option<usize>)>,
        tab_cell: Option<(usize, usize, usize)>,
        graph_id: Option<SheetId>,
        abs_tol: f64,
        rel_tol: f64,
    },
    GraphSvgChanged {
        graph_id: SheetId,
    },
    Custom(LiveEditCustomFn),
}

impl LiveEditExpectation {
    /// Convenience constructor asserting a scalar changes by at least `min_delta` (or any non-zero amount when `min_delta <= 0.0`).
    #[must_use]
    pub fn scalar_changed(result_id: SheetId, key: impl Into<String>, data_set_index: Option<usize>, min_delta: f64) -> Self {
        Self::ScalarChanged { result_id, key: key.into(), data_set_index, min_delta }
    }

    /// Convenience constructor asserting a tab cell changes value/text.
    #[must_use]
    pub fn tab_cell_changed(result_id: SheetId, tab_index: usize, row: usize, col: usize) -> Self {
        Self::TabCellChanged { result_id, tab_index, row, col }
    }

    /// Convenience constructor asserting a scalar stays invariant within `(abs_tol, rel_tol)`.
    #[must_use]
    pub fn scalar_invariant(result_id: SheetId, key: impl Into<String>, data_set_index: Option<usize>, abs_tol: f64, rel_tol: f64) -> Self {
        Self::Invariant {
            result_id: Some(result_id),
            scalar_key: Some((key.into(), data_set_index)),
            tab_cell: None,
            graph_id: None,
            abs_tol,
            rel_tol,
        }
    }

    /// Convenience constructor asserting a compiled graph SVG stays identical.
    #[must_use]
    pub fn graph_svg_invariant(graph_id: SheetId) -> Self {
        Self::Invariant { result_id: None, scalar_key: None, tab_cell: None, graph_id: Some(graph_id), abs_tol: 0.0, rel_tol: 0.0 }
    }

    /// Convenience constructor asserting a compiled graph SVG changes.
    #[must_use]
    pub fn graph_svg_changed(graph_id: SheetId) -> Self {
        Self::GraphSvgChanged { graph_id }
    }

    /// Convenience constructor for custom live-edit snapshot verification.
    #[must_use]
    pub fn custom(f: impl FnOnce(&LiveEditSnapshot, &LiveEditSnapshot, &ProjectDocument) -> TestRes + 'static) -> Self {
        Self::Custom(Box::new(f))
    }
}

/// Strict five-stage GUI workflow completion harness (`R10.26`).
pub struct WorkflowRun {
    wf_id: String,
    harness: EguiWfHarness,
    clipboard: Arc<StubPlatformClipboardAdapter>,
    stages: WorkflowStages,
    reference_verified: bool,
    bg_panics_at_start: usize,
}

impl WorkflowRun {
    /// Start a five-stage GUI workflow test for `wf_id` (e.g. `"WF-01"`) from an empty `UiApp`.
    #[must_use]
    pub fn start(wf_id: &str) -> Self {
        Self::start_with_clipboard(wf_id, Arc::new(StubPlatformClipboardAdapter::new()))
    }

    /// Start a five-stage GUI workflow test for `wf_id` wired to an OS-boundary clipboard stub.
    #[must_use]
    pub fn start_with_clipboard(wf_id: &str, clipboard: Arc<StubPlatformClipboardAdapter>) -> Self {
        install_bg_panic_hook();
        let bg_panics_at_start = BG_PANIC_COUNT.load(Ordering::SeqCst);
        let harness = EguiWfHarness::new_with_clipboard(default_viewport(), Arc::clone(&clipboard));
        Self { wf_id: wf_id.to_owned(), harness, clipboard, stages: WorkflowStages::default(), reference_verified: false, bg_panics_at_start }
    }

    /// Workflow identifier passed to [`WorkflowRun::start`].
    #[must_use]
    pub fn wf_id(&self) -> &str {
        &self.wf_id
    }

    /// Current stage flags (`created`, `entered`, `configured`, `verified`, `live_edited`).
    #[must_use]
    pub const fn stages(&self) -> WorkflowStages {
        self.stages
    }

    /// Whether [`WorkflowRun::verify_reference`] has succeeded at least once.
    #[must_use]
    pub const fn is_reference_verified(&self) -> bool {
        self.reference_verified
    }

    /// Read-only access to the underlying [`UiApp`].
    #[must_use]
    pub fn app(&self) -> &UiApp {
        self.harness.app()
    }

    /// Read-only access to the active [`ProjectDocument`].
    #[must_use]
    pub fn document(&self) -> &ProjectDocument {
        self.harness.app().document()
    }

    /// Read-only access to the shell state wrapper.
    #[must_use]
    pub fn state(&self) -> &WfShellState {
        self.harness.driver.state()
    }

    /// Read-only settled [`FrameInspectMeta`] from the latest frame.
    #[must_use]
    pub fn frame_meta(&self) -> &FrameInspectMeta {
        self.harness.driver.frame_meta()
    }

    /// Read-only currently active modal dialog state.
    #[must_use]
    pub fn active_dialog(&self) -> &ActiveDialog {
        &self.harness.app().canvas_dialogs.active_dialog
    }

    /// Accumulated intent commit errors observed across settled frames.
    #[must_use]
    pub fn intent_errors(&self) -> &[String] {
        &self.harness.driver.state().intent_errors
    }

    /// Read-only access to the attached clipboard stub.
    #[must_use]
    pub fn clipboard(&self) -> &StubPlatformClipboardAdapter {
        &self.clipboard
    }

    /// Center point in logical points of settled widget `wid`.
    pub fn widget_center(&self, wid: &str) -> Result<Pos2, String> {
        self.harness.widget_center(wid)
    }

    /// Update viewport metrics and re-settle.
    pub fn set_viewport(&mut self, metrics: ViewportMetrics) {
        self.harness.driver.set_viewport(metrics);
    }

    /// Render a PNG screenshot of `region` via the software rasterizer.
    pub fn screenshot_png(&mut self, region: ScreenshotRegionTarget) -> Result<Vec<u8>, String> {
        self.harness.screenshot_png(region)
    }

    /// Click widget `wid` via synthesized pointer move/down/up.
    pub fn click(&mut self, wid: &str) -> TestRes {
        let before_ds = self.document().data_sheets.len();
        let before_fp = self.data_fingerprint();
        self.harness.click(wid)?;
        self.observe_passive_progress(before_ds, before_fp.as_deref(), wid.starts_with("power."));
        Ok(())
    }

    /// Double-click widget `wid` via synthesized pointer events.
    pub fn dbl_click(&mut self, wid: &str) -> TestRes {
        let before_ds = self.document().data_sheets.len();
        let before_fp = self.data_fingerprint();
        self.harness.dbl_click_widget(wid)?;
        self.observe_passive_progress(before_ds, before_fp.as_deref(), wid.starts_with("power."));
        Ok(())
    }

    /// Right-click widget `wid` at its rendered center.
    pub fn right_click(&mut self, wid: &str) -> TestRes {
        let pos = self.widget_center(wid)?;
        self.click_at_button(pos, PointerButton::Secondary, KeyModifiers::default())
    }

    /// Left-click at logical point `pos`.
    pub fn click_at(&mut self, pos: Pos2) -> TestRes {
        let before_ds = self.document().data_sheets.len();
        let before_fp = self.data_fingerprint();
        self.harness.driver.click_at(pos)?;
        self.observe_passive_progress(before_ds, before_fp.as_deref(), false);
        Ok(())
    }

    /// Click at logical point `pos` with explicit `button` and `modifiers`.
    pub fn click_at_button(&mut self, pos: Pos2, button: PointerButton, modifiers: KeyModifiers) -> TestRes {
        let before_ds = self.document().data_sheets.len();
        let before_fp = self.data_fingerprint();
        self.harness.driver.click_at_button(pos, button, modifiers)?;
        self.observe_passive_progress(before_ds, before_fp.as_deref(), false);
        Ok(())
    }

    /// Hover widget `wid` at its rendered center.
    pub fn hover(&mut self, wid: &str) -> TestRes {
        self.harness.driver.hover_widget_id(wid)
    }

    /// Hover at logical point `pos`.
    pub fn hover_at(&mut self, pos: Pos2) -> TestRes {
        self.harness.driver.hover_at(pos)
    }

    /// Drag pointer from `from` to `to`.
    pub fn drag(&mut self, from: Pos2, to: Pos2) -> TestRes {
        self.harness.driver.drag(from, to)
    }

    /// Drag pointer from `from` to `to` with modifier keys held.
    pub fn drag_with_modifiers(&mut self, from: Pos2, to: Pos2, modifiers: KeyModifiers) -> TestRes {
        self.harness.driver.drag_with_modifiers(from, to, modifiers)
    }

    /// Dispatch a pointer wheel delta `delta` at logical point `pos`.
    pub fn wheel(&mut self, pos: Pos2, delta: Vec2) -> TestRes {
        self.harness.driver.wheel_at(pos, delta)
    }

    /// Type UTF-8 text into the currently focused widget.
    pub fn type_text(&mut self, text: &str) -> TestRes {
        let before_ds = self.document().data_sheets.len();
        let before_fp = self.data_fingerprint();
        self.harness.driver.type_text(text)?;
        self.observe_passive_progress(before_ds, before_fp.as_deref(), false);
        Ok(())
    }

    /// Press key named `key` with `modifiers`.
    pub fn press_key(&mut self, key: &str, modifiers: KeyModifiers) -> TestRes {
        let before_ds = self.document().data_sheets.len();
        let before_fp = self.data_fingerprint();
        self.harness.driver.press_key(key, modifiers)?;
        self.observe_passive_progress(before_ds, before_fp.as_deref(), false);
        Ok(())
    }

    /// Press `egui::Key` with `egui::Modifiers`.
    pub fn press_egui_key(&mut self, key: Key, modifiers: Modifiers) -> TestRes {
        let before_ds = self.document().data_sheets.len();
        let before_fp = self.data_fingerprint();
        self.harness.driver.press_egui_key(key, modifiers);
        self.observe_passive_progress(before_ds, before_fp.as_deref(), false);
        Ok(())
    }

    /// Replace textbox `wid` contents via click + `Cmd+A` + type/backspace.
    pub fn replace_textbox(&mut self, wid: &str, text: &str) -> TestRes {
        let before_ds = self.document().data_sheets.len();
        let before_fp = self.data_fingerprint();
        self.harness.replace_textbox(wid, text)?;
        self.observe_passive_progress(before_ds, before_fp.as_deref(), wid.starts_with("power."));
        Ok(())
    }

    /// Create a data table via Welcome Hub (`Create` stage).
    pub fn create_table_via_welcome(&mut self, arch_wid: &str, title: &str, extra_clicks: &[&str]) -> Result<SheetId, String> {
        let sid = self.harness.create_table_via_welcome(arch_wid, title, extra_clicks)?;
        self.stages.created = true;
        Ok(sid)
    }

    /// Open Power & Sample Size Studio via the Analyze menu (`Create` stage for Power workflows).
    pub fn open_power_wizard(&mut self) -> TestRes {
        self.open_menu_and_click("menu.analyze", &[], "menu.ui.powerWizard.open")?;
        ensure(
            matches!(self.active_dialog(), ActiveDialog::PowerAndSampleSize),
            format!("{}: expected ActiveDialog::PowerAndSampleSize, found {:?}", self.wf_id, self.active_dialog()),
        )?;
        self.stages.created = true;
        Ok(())
    }

    /// Navigate Power & Sample Size Studio to stage `1..=4`.
    pub fn power_wizard_stage(&mut self, stage_1_to_4: usize) -> TestRes {
        self.require_created("power_wizard_stage")?;
        ensure((1..=4).contains(&stage_1_to_4), format!("{}: Power Wizard stage {stage_1_to_4} must be in 1..=4", self.wf_id))?;
        self.click(&format!("power.stage.{stage_1_to_4}"))?;
        self.stages.entered = true;
        Ok(())
    }

    /// Advance Power & Sample Size Studio by clicking `modal.next`.
    pub fn power_wizard_next(&mut self) -> TestRes {
        self.require_created("power_wizard_next")?;
        self.click("modal.next")?;
        self.stages.entered = true;
        Ok(())
    }

    /// Step backward in Power & Sample Size Studio by clicking `modal.back`.
    pub fn power_wizard_back(&mut self) -> TestRes {
        self.require_created("power_wizard_back")?;
        self.click("modal.back")?;
        self.stages.entered = true;
        Ok(())
    }

    /// Create the Power & Sample Size `ResultSheet` from the open wizard (`Configure` stage).
    pub fn power_wizard_create(&mut self) -> Result<SheetId, String> {
        self.require_created("power_wizard_create")?;
        self.require_entered("power_wizard_create")?;
        let before = self.document().result_sheets.len();
        let before_errs = self.intent_errors().len();
        self.harness.click("modal.ok")?;
        self.fail_on_new_intent_errors("power_wizard_create", before_errs)?;
        ensure(self.document().result_sheets.len() > before, format!("{}: Power Wizard modal.ok did not create a ResultSheet", self.wf_id))?;
        self.stages.configured = true;
        self.document().result_sheets.last().map(|r| r.id.clone()).ok_or_else(|| String::from("missing Power ResultSheet"))
    }

    /// Open Power & Sample Size Studio, click `config_clicks` (or `power.compute_defaults` if empty), and create its `ResultSheet`.
    pub fn open_power_wizard_create(&mut self, config_clicks: &[&str]) -> Result<SheetId, String> {
        self.open_power_wizard()?;
        if config_clicks.is_empty() {
            self.click("power.compute_defaults")?;
        } else {
            for wid in config_clicks {
                self.click(wid)?;
            }
        }
        self.stages.entered = true;
        self.power_wizard_create()
    }

    /// Enter `text` into data cell `(row, col, subcol)` (`Enter` stage). Supports `""` to clear a cell.
    pub fn enter_grid_cell(&mut self, row: usize, col: usize, subcol: usize, text: &str) -> TestRes {
        self.require_created("enter_grid_cell")?;
        let wid = format!("grid.cell.{row}.{col}.{subcol}");
        self.type_replace_with_empty_support(&wid, text)?;
        self.stages.entered = true;
        Ok(())
    }

    /// Enter `text` into X-column cell `(row, subcol)` (`Enter` stage). Supports `""` to clear a cell.
    pub fn enter_x_cell(&mut self, row: usize, subcol: usize, text: &str) -> TestRes {
        self.require_created("enter_x_cell")?;
        let wid = format!("grid.xcell.{row}.{subcol}");
        self.type_replace_with_empty_support(&wid, text)?;
        self.stages.entered = true;
        Ok(())
    }

    /// Edit row title `row` inline across both single-click (`Multi`) and double-click (`Column`) grids (`Enter` stage).
    pub fn enter_row_title(&mut self, row: usize, title: &str) -> TestRes {
        self.require_created("enter_row_title")?;
        self.edit_header_inline(&format!("grid.row_title.{row}"), title)?;
        self.stages.entered = true;
        Ok(())
    }

    /// Edit column title `col` inline across both single-click (`Multi`) and double-click (`Column`) grids (`Enter` stage).
    pub fn enter_column_title(&mut self, col: usize, title: &str) -> TestRes {
        self.require_created("enter_column_title")?;
        self.edit_header_inline(&format!("grid.col_title.{col}"), title)?;
        self.stages.entered = true;
        Ok(())
    }

    /// Seed `tsv` into the attached clipboard stub and trigger `Cmd+V` paste (`Enter` stage).
    pub fn paste_clipboard(&mut self, tsv: &str) -> TestRes {
        self.require_created("paste_clipboard")?;
        self.clipboard.seed_text(tsv);
        self.paste_from_clipboard()
    }

    /// Trigger `Cmd+V` paste using whatever payload is currently on the attached clipboard stub.
    pub fn paste_from_clipboard(&mut self) -> TestRes {
        self.require_created("paste_from_clipboard")?;
        let before_errs = self.intent_errors().len();
        let before_fp = self.data_fingerprint();
        self.harness.driver.press_key("V", KeyModifiers { meta: true, ..Default::default() })?;
        self.fail_on_new_intent_errors("paste_from_clipboard", before_errs)?;
        if self.data_fingerprint() != before_fp {
            self.stages.entered = true;
        }
        Ok(())
    }

    /// Assign Multiple Variables column `col` role via badge popup click.
    pub fn set_mv_role(&mut self, col: usize, role_slug: &str) -> TestRes {
        self.require_created("set_mv_role")?;
        self.harness.set_mv_role(col, role_slug)
    }

    /// Select sheet `id` via Project Navigator chip & row click.
    pub fn select_sheet(&mut self, id: &SheetId) -> TestRes {
        self.harness.select_sheet(id)
    }

    /// Run an Analyze dialog entry (`toolbar.analyze` -> filter `search_query` -> `entry_wid` -> `modal.ok`) (`Configure` stage).
    pub fn run_analyze_entry(&mut self, search_query: &str, entry_wid: &str) -> Result<SheetId, String> {
        self.require_created("run_analyze_entry")?;
        self.require_entered("run_analyze_entry")?;
        let before_errs = self.intent_errors().len();
        let rs_id = self.harness.run_analyze_entry(search_query, entry_wid)?;
        self.fail_on_new_intent_errors("run_analyze_entry", before_errs)?;
        self.stages.configured = true;
        Ok(rs_id)
    }

    /// Open `EditAnalysisParameters` via `Cmd+T`.
    pub fn open_cmd_t(&mut self) -> TestRes {
        self.require_created("open_cmd_t")?;
        self.require_entered("open_cmd_t")?;
        self.harness.open_cmd_t()
    }

    /// Open top menu `menu_wid`, optionally hover each submenu in `submenu_hovers`, and click `item_wid`.
    pub fn open_menu_and_click(&mut self, menu_wid: &str, submenu_hovers: &[&str], item_wid: &str) -> TestRes {
        let before_ds = self.document().data_sheets.len();
        let before_rs = self.document().result_sheets.len();
        let before_gs = self.document().graph_sheets.len();
        let before_ls = self.document().layout_sheets.len();
        let before_errs = self.intent_errors().len();

        self.harness.click(menu_wid)?;
        for sub in submenu_hovers {
            self.harness.driver.hover_widget_id(sub)?;
        }
        self.harness.click(item_wid)?;
        self.fail_on_new_intent_errors("open_menu_and_click", before_errs)?;

        if self.document().data_sheets.len() > before_ds || matches!(self.active_dialog(), ActiveDialog::PowerAndSampleSize) {
            self.stages.created = true;
        }
        if self.stages.created
            && self.stages.entered
            && (self.document().result_sheets.len() > before_rs
                || self.document().graph_sheets.len() > before_gs
                || self.document().layout_sheets.len() > before_ls)
        {
            self.stages.configured = true;
        }
        Ok(())
    }

    /// Commit an open modal dialog by clicking `modal.ok` (`Configure` stage, or Import `Create` stage).
    pub fn commit_modal_ok(&mut self) -> TestRes {
        self.commit_modal_button("modal.ok", "commit_modal_ok")
    }

    /// Commit an open modal dialog without closing it by clicking `modal.apply` (`Configure` stage).
    pub fn commit_modal_apply(&mut self) -> TestRes {
        self.commit_modal_button("modal.apply", "commit_modal_apply")
    }

    /// Stage 4 (`Verify`): run `spec` on a fresh headless `curvebook_engine::Session` over the same dataset
    /// and verify GUI scalars, tab cells, and/or compiled `GraphSpec` match within documented tolerances (`D1`).
    pub fn verify_reference(&mut self, spec: ReferenceSpec<'_>) -> TestRes {
        self.require_created("verify_reference")?;
        self.require_entered("verify_reference")?;
        self.require_configured("verify_reference")?;

        let has_checks = !spec.scalars.is_empty() || !spec.tab_cells.is_empty() || spec.gui_graph_id.is_some() || spec.check_graph.is_some();
        ensure(has_checks, format!("{}: verify_reference requires at least one scalar, tab_cell, or graph check", self.wf_id))?;

        let mut ref_session = match spec.initial_doc {
            Some(doc) => Session::from_document(doc),
            None => Session::new(),
        };
        (spec.build_reference)(&mut ref_session).map_err(|e| format!("{}: reference builder failed: {e}", self.wf_id))?;

        if !spec.scalars.is_empty() || !spec.tab_cells.is_empty() {
            let gui_rs = resolve_gui_result(self.document(), spec.gui_result_id.as_ref(), &self.wf_id)?;
            ensure(
                matches!(gui_rs.status, ResultStatus::UpToDate),
                format!("{}: verify_reference GUI ResultSheet `{}` is not UpToDate: {:?}", self.wf_id, gui_rs.id.as_str(), gui_rs.status),
            )?;
            let ref_rs = resolve_ref_result(ref_session.document(), spec.ref_result_id.as_ref(), &self.wf_id)?;
            ensure(
                matches!(ref_rs.status, ResultStatus::UpToDate),
                format!("{}: verify_reference ref ResultSheet `{}` is not UpToDate: {:?}", self.wf_id, ref_rs.id.as_str(), ref_rs.status),
            )?;
            verify_scalars(&self.wf_id, gui_rs, ref_rs, &spec.scalars)?;
            verify_tab_cells(&self.wf_id, gui_rs, ref_rs, &spec.tab_cells)?;
        }

        if let Some(gui_gid) = spec.gui_graph_id.as_ref() {
            let gui_gs =
                self.document().graph_sheet(gui_gid).ok_or_else(|| format!("{}: GUI GraphSheet `{}` not found", self.wf_id, gui_gid.as_str()))?;
            let ref_gs = resolve_ref_graph(ref_session.document(), spec.ref_graph_id.as_ref(), &self.wf_id)?;
            ensure(
                gui_gs.spec.kind == ref_gs.spec.kind,
                format!("{}: GraphSpec kind mismatch: gui={:?}, ref={:?}", self.wf_id, gui_gs.spec.kind, ref_gs.spec.kind),
            )?;
            ensure(
                gui_gs.spec.series.len() == ref_gs.spec.series.len(),
                format!("{}: GraphSpec series count mismatch: gui={}, ref={}", self.wf_id, gui_gs.spec.series.len(), ref_gs.spec.series.len()),
            )?;
            if let Some(check) = spec.check_graph {
                check(gui_gs, ref_gs)?;
            }
        }

        self.stages.verified = true;
        self.reference_verified = true;
        Ok(())
    }

    /// Stage 5 (`LiveEdit`): snapshot scalars, tab cells, and graph SVGs; apply upstream GUI `edit`;
    /// verify every `ResultSheet` is `ResultStatus::UpToDate`; and check `expect`.
    pub fn live_edit(&mut self, edit: LiveEditTarget, expect: LiveEditExpectation) -> TestRes {
        self.live_edit_all(edit, vec![expect])
    }

    /// Stage 5 (`LiveEdit`) variant validating multiple expectations after a single upstream GUI edit.
    pub fn live_edit_all(&mut self, edit: LiveEditTarget, expectations: Vec<LiveEditExpectation>) -> TestRes {
        self.require_created("live_edit")?;
        self.require_entered("live_edit")?;
        self.require_configured("live_edit")?;
        ensure(
            self.stages.verified && self.reference_verified,
            format!("{}: live_edit requires Stage 4 (`verify_reference`) to pass first", self.wf_id),
        )?;
        ensure(
            !self.document().result_sheets.is_empty() || !self.document().graph_sheets.is_empty(),
            format!("{}: live_edit requires an existing ResultSheet or GraphSheet", self.wf_id),
        )?;
        ensure(!expectations.is_empty(), format!("{}: live_edit requires at least one LiveEditExpectation", self.wf_id))?;

        let before = LiveEditSnapshot::capture(self.document())?;
        let before_errs = self.intent_errors().len();

        match edit {
            LiveEditTarget::GridCell { sheet_id, row, col, subcol, text } => {
                if let Some(id) = sheet_id.as_ref() {
                    self.select_sheet(id)?;
                }
                self.enter_grid_cell(row, col, subcol, &text)?;
            }
            LiveEditTarget::XCell { sheet_id, row, subcol, text } => {
                if let Some(id) = sheet_id.as_ref() {
                    self.select_sheet(id)?;
                }
                self.enter_x_cell(row, subcol, &text)?;
            }
            LiveEditTarget::RowTitle { sheet_id, row, title } => {
                if let Some(id) = sheet_id.as_ref() {
                    self.select_sheet(id)?;
                }
                self.enter_row_title(row, &title)?;
            }
            LiveEditTarget::ColumnTitle { sheet_id, col, title } => {
                if let Some(id) = sheet_id.as_ref() {
                    self.select_sheet(id)?;
                }
                self.enter_column_title(col, &title)?;
            }
            LiveEditTarget::GuiAction(f) => f(self)?,
        }

        self.fail_on_new_intent_errors("live_edit", before_errs)?;
        assert_all_results_up_to_date(self.document(), &self.wf_id, "live_edit")?;
        let after = LiveEditSnapshot::capture(self.document())?;

        for exp in expectations {
            verify_live_edit_expectation(&self.wf_id, &before, &after, self.document(), exp)?;
        }

        self.stages.live_edited = true;
        Ok(())
    }

    /// Finalize the workflow run, enforcing the complete five-stage contract, zero failures/panics/intent errors,
    /// `assert_zero_shortcut_compliance`, `verify_14_locales_section_9_4`, and `assert_input_to_paint_budget(50.0)`.
    pub fn finish(mut self) -> TestRes {
        ensure(self.stages.created, format!("{}: incomplete workflow — Stage 1 (`Create`) never ran", self.wf_id))?;
        ensure(self.stages.entered, format!("{}: incomplete workflow — Stage 2 (`Enter`) never ran", self.wf_id))?;
        ensure(self.stages.configured, format!("{}: incomplete workflow — Stage 3 (`Configure`) never ran", self.wf_id))?;
        ensure(self.stages.verified, format!("{}: incomplete workflow — Stage 4 (`Verify`) never ran", self.wf_id))?;
        ensure(self.reference_verified, format!("{}: incomplete workflow — `verify_reference` was never executed", self.wf_id))?;
        ensure(self.stages.live_edited, format!("{}: incomplete workflow — Stage 5 (`LiveEdit`) never ran", self.wf_id))?;
        ensure(
            !self.document().result_sheets.is_empty() || !self.document().graph_sheets.is_empty(),
            format!("{}: finish requires at least one ResultSheet or GraphSheet", self.wf_id),
        )?;

        assert_all_results_up_to_date(self.document(), &self.wf_id, "finish")?;
        self.assert_no_intent_errors("finish")?;
        self.assert_no_bg_panics("finish")?;

        assert_zero_shortcut_compliance(&mut self.harness)?;
        self.harness.verify_14_locales_section_9_4(&self.wf_id)?;
        assert_input_to_paint_budget(&self.harness, 50.0)?;

        self.assert_no_intent_errors("finish post-locale sweep")?;
        self.assert_no_bg_panics("finish post-locale sweep")?;
        Ok(())
    }

    fn data_fingerprint(&self) -> Option<String> {
        serde_json::to_string(&self.document().data_sheets).ok()
    }

    fn observe_passive_progress(&mut self, before_ds: usize, before_fp: Option<&str>, power_widget: bool) {
        if self.document().data_sheets.len() > before_ds || matches!(self.active_dialog(), ActiveDialog::PowerAndSampleSize) {
            self.stages.created = true;
        }
        if self.stages.created
            && ((power_widget && matches!(self.active_dialog(), ActiveDialog::PowerAndSampleSize)) || self.data_fingerprint().as_deref() != before_fp)
        {
            self.stages.entered = true;
        }
    }

    fn type_replace_with_empty_support(&mut self, wid: &str, text: &str) -> TestRes {
        self.harness.dbl_click_widget(wid)?;
        self.harness.driver.press_egui_key(Key::A, Modifiers::COMMAND);
        if text.is_empty() {
            self.harness.driver.press_key("Backspace", KeyModifiers::default())?;
        } else {
            self.harness.driver.type_text(text)?;
        }
        self.harness.driver.press_key("Enter", KeyModifiers::default())?;
        Ok(())
    }

    fn edit_header_inline(&mut self, wid: &str, title: &str) -> TestRes {
        self.harness.click(wid)?;
        if !self.frame_meta().find_by_widget_id(wid).is_some_and(|e| e.role == "textbox") {
            self.harness.dbl_click_widget(wid)?;
        }
        let meta = self.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("{}: missing header widget `{wid}`", self.wf_id))?;
        ensure(meta.role == "textbox", format!("{}: expected `{wid}` to enter inline textbox edit mode, found role `{}`", self.wf_id, meta.role))?;
        self.harness.driver.press_egui_key(Key::A, Modifiers::COMMAND);
        if title.is_empty() {
            self.harness.driver.press_key("Backspace", KeyModifiers::default())?;
        } else {
            self.harness.driver.type_text(title)?;
        }
        self.harness.driver.press_key("Enter", KeyModifiers::default())?;
        Ok(())
    }

    fn commit_modal_button(&mut self, wid: &str, caller: &str) -> TestRes {
        let dialog_before = self.active_dialog().clone();
        ensure(!matches!(dialog_before, ActiveDialog::None), format!("{}: {caller} called with ActiveDialog::None", self.wf_id))?;
        let is_import_creator = matches!(dialog_before, ActiveDialog::ImportReshape | ActiveDialog::SimulateDistributions);
        if !is_import_creator {
            self.require_created(caller)?;
            self.require_entered(caller)?;
        }
        let before_ds = self.document().data_sheets.len();
        let before_errs = self.intent_errors().len();
        self.harness.click(wid)?;
        self.fail_on_new_intent_errors(caller, before_errs)?;
        if is_import_creator && self.document().data_sheets.len() > before_ds {
            self.stages.created = true;
            self.stages.entered = true;
        } else if self.stages.created && self.stages.entered {
            self.stages.configured = true;
        }
        Ok(())
    }

    fn require_created(&self, caller: &str) -> TestRes {
        ensure(self.stages.created, format!("{}: `{caller}` requires Stage 1 (`Create`) first", self.wf_id))
    }

    fn require_entered(&self, caller: &str) -> TestRes {
        ensure(self.stages.entered, format!("{}: `{caller}` requires Stage 2 (`Enter`) first", self.wf_id))
    }

    fn require_configured(&self, caller: &str) -> TestRes {
        ensure(self.stages.configured, format!("{}: `{caller}` requires Stage 3 (`Configure`) first", self.wf_id))
    }

    fn fail_on_new_intent_errors(&self, caller: &str, before_count: usize) -> TestRes {
        let errs = self.intent_errors();
        if errs.len() > before_count {
            return Err(format!("{}: `{caller}` recorded intent error(s): {}", self.wf_id, errs[before_count..].join("; ")));
        }
        Ok(())
    }

    fn assert_no_intent_errors(&self, caller: &str) -> TestRes {
        let errs = self.intent_errors();
        ensure(errs.is_empty(), format!("{}: `{caller}` found accumulated intent error(s): {}", self.wf_id, errs.join("; ")))
    }

    fn assert_no_bg_panics(&self, caller: &str) -> TestRes {
        let now = BG_PANIC_COUNT.load(Ordering::SeqCst);
        ensure(
            now == self.bg_panics_at_start,
            format!("{}: `{caller}` detected {} background-thread panic(s)", self.wf_id, now.saturating_sub(self.bg_panics_at_start)),
        )
    }
}

fn assert_all_results_up_to_date(doc: &ProjectDocument, wf_id: &str, phase: &str) -> TestRes {
    for rs in &doc.result_sheets {
        match &rs.status {
            ResultStatus::UpToDate => {}
            ResultStatus::Dirty => {
                return Err(format!("{wf_id}: `{phase}` found Dirty ResultSheet `{}`", rs.id.as_str()));
            }
            ResultStatus::Failed { code, message } => {
                return Err(format!("{wf_id}: `{phase}` found Failed ResultSheet `{}` ({code:?}: {message})", rs.id.as_str()));
            }
        }
    }
    Ok(())
}

fn resolve_gui_result<'a>(doc: &'a ProjectDocument, id: Option<&SheetId>, wf_id: &str) -> Result<&'a ResultSheet, String> {
    match id {
        Some(sid) => doc.result_sheet(sid).ok_or_else(|| format!("{wf_id}: GUI ResultSheet `{}` not found", sid.as_str())),
        None => doc.result_sheets.last().ok_or_else(|| format!("{wf_id}: GUI document has no ResultSheet")),
    }
}

fn resolve_ref_result<'a>(doc: &'a ProjectDocument, id: Option<&SheetId>, wf_id: &str) -> Result<&'a ResultSheet, String> {
    match id {
        Some(sid) => doc.result_sheet(sid).ok_or_else(|| format!("{wf_id}: reference ResultSheet `{}` not found", sid.as_str())),
        None => doc.result_sheets.last().ok_or_else(|| format!("{wf_id}: reference Session produced no ResultSheet")),
    }
}

fn resolve_ref_graph<'a>(doc: &'a ProjectDocument, id: Option<&SheetId>, wf_id: &str) -> Result<&'a GraphSheet, String> {
    match id {
        Some(sid) => doc.graph_sheet(sid).ok_or_else(|| format!("{wf_id}: reference GraphSheet `{}` not found", sid.as_str())),
        None => doc.graph_sheets.last().ok_or_else(|| format!("{wf_id}: reference Session produced no GraphSheet")),
    }
}

fn verify_scalars(wf_id: &str, gui_rs: &ResultSheet, ref_rs: &ResultSheet, scalars: &[ReferenceScalar]) -> TestRes {
    for spec in scalars {
        let gui_val = gui_rs.lookup_scalar(&spec.key, spec.data_set_index).ok_or_else(|| {
            let keys: Vec<String> = gui_rs.scalars.iter().map(|s| format!("{}@{:?}", s.key, s.data_set_index)).collect();
            format!(
                "{wf_id}: verify_reference GUI ResultSheet `{}` missing scalar `{}` (ds={:?}); available=[{}]",
                gui_rs.id.as_str(),
                spec.key,
                spec.data_set_index,
                keys.join(", ")
            )
        })?;
        let ref_val = ref_rs.lookup_scalar(&spec.key, spec.data_set_index).ok_or_else(|| {
            let keys: Vec<String> = ref_rs.scalars.iter().map(|s| format!("{}@{:?}", s.key, s.data_set_index)).collect();
            format!(
                "{wf_id}: verify_reference ref ResultSheet `{}` missing scalar `{}` (ds={:?}); available=[{}]",
                ref_rs.id.as_str(),
                spec.key,
                spec.data_set_index,
                keys.join(", ")
            )
        })?;
        ensure(
            approx_eq(gui_val, ref_val, spec.abs_tol, spec.rel_tol),
            format!(
                "{wf_id}: verify_reference scalar mismatch for `{}` (ds={:?}): gui={gui_val:?}, ref={ref_val:?}, abs_tol={}, rel_tol={}",
                spec.key, spec.data_set_index, spec.abs_tol, spec.rel_tol
            ),
        )?;
    }
    Ok(())
}

fn verify_tab_cells(wf_id: &str, gui_rs: &ResultSheet, ref_rs: &ResultSheet, tab_cells: &[ReferenceTabCell]) -> TestRes {
    for tc in tab_cells {
        let gui_cell = gui_rs.tabs.get(tc.tab_index).and_then(|t| t.rows.get(tc.row)).and_then(|r| r.get(tc.col)).ok_or_else(|| {
            format!("{wf_id}: GUI ResultSheet `{}` missing tab cell ({}, {}, {})", gui_rs.id.as_str(), tc.tab_index, tc.row, tc.col)
        })?;
        let ref_cell = ref_rs.tabs.get(tc.tab_index).and_then(|t| t.rows.get(tc.row)).and_then(|r| r.get(tc.col)).ok_or_else(|| {
            format!("{wf_id}: ref ResultSheet `{}` missing tab cell ({}, {}, {})", ref_rs.id.as_str(), tc.tab_index, tc.row, tc.col)
        })?;
        compare_result_cell(wf_id, tc, gui_cell, ref_cell)?;
    }
    Ok(())
}

fn compare_ci_bound(wf_id: &str, tc: &ReferenceTabCell, gui_b: CiBound, ref_b: CiBound) -> TestRes {
    match (gui_b, ref_b) {
        (CiBound::Finite(g), CiBound::Finite(r)) => ensure(
            approx_eq(g, r, tc.abs_tol, tc.rel_tol),
            format!("{wf_id}: tab ({},{},{}) CI bound mismatch: gui={g:?}, ref={r:?}", tc.tab_index, tc.row, tc.col),
        ),
        (CiBound::Open, CiBound::Open) => Ok(()),
        _ => Err(format!("{wf_id}: tab ({},{},{}) CI bound kind mismatch: gui={gui_b:?}, ref={ref_b:?}", tc.tab_index, tc.row, tc.col)),
    }
}

fn compare_result_cell(wf_id: &str, tc: &ReferenceTabCell, gui_cell: &ResultCell, ref_cell: &ResultCell) -> TestRes {
    match (gui_cell, ref_cell) {
        (ResultCell::Empty, ResultCell::Empty) => Ok(()),
        (ResultCell::Number { value: g, .. }, ResultCell::Number { value: r, .. }) => ensure(
            g.is_finite() && r.is_finite() && approx_eq(*g, *r, tc.abs_tol, tc.rel_tol),
            format!("{wf_id}: tab ({},{},{}) number mismatch: gui={g:?}, ref={r:?}", tc.tab_index, tc.row, tc.col),
        ),
        (ResultCell::PValue { raw: g, .. }, ResultCell::PValue { raw: r, .. }) => ensure(
            g.is_finite() && r.is_finite() && approx_eq(*g, *r, tc.abs_tol, tc.rel_tol),
            format!("{wf_id}: tab ({},{},{}) p-value mismatch: gui={g:?}, ref={r:?}", tc.tab_index, tc.row, tc.col),
        ),
        (ResultCell::ConfidenceInterval { lower: gl, upper: gu }, ResultCell::ConfidenceInterval { lower: rl, upper: ru }) => {
            compare_ci_bound(wf_id, tc, *gl, *rl)?;
            compare_ci_bound(wf_id, tc, *gu, *ru)
        }
        (ResultCell::Text(g), ResultCell::Text(r)) => {
            ensure(g == r, format!("{wf_id}: tab ({},{},{}) text mismatch: gui={g:?}, ref={r:?}", tc.tab_index, tc.row, tc.col))
        }
        (ResultCell::Boolean(g), ResultCell::Boolean(r)) => {
            ensure(g == r, format!("{wf_id}: tab ({},{},{}) boolean mismatch: gui={g:?}, ref={r:?}", tc.tab_index, tc.row, tc.col))
        }
        _ => Err(format!("{wf_id}: tab ({},{},{}) variant mismatch: gui={gui_cell:?}, ref={ref_cell:?}", tc.tab_index, tc.row, tc.col)),
    }
}

fn verify_live_edit_expectation(
    wf_id: &str,
    before: &LiveEditSnapshot,
    after: &LiveEditSnapshot,
    doc: &ProjectDocument,
    exp: LiveEditExpectation,
) -> TestRes {
    match exp {
        LiveEditExpectation::ScalarChanged { result_id, key, data_set_index, min_delta } => {
            let b = before
                .scalar(&result_id, &key, data_set_index)
                .ok_or_else(|| format!("{wf_id}: live_edit before-snapshot missing scalar `{key}` on `{}`", result_id.as_str()))?;
            let a = after
                .scalar(&result_id, &key, data_set_index)
                .ok_or_else(|| format!("{wf_id}: live_edit after-snapshot missing scalar `{key}` on `{}`", result_id.as_str()))?;
            let diff = (a - b).abs();
            let threshold = if min_delta <= 0.0 { f64::EPSILON } else { min_delta };
            ensure(
                diff >= threshold,
                format!(
                    "{wf_id}: live_edit expected scalar `{key}` on `{}` to change (before={b:?}, after={a:?}, diff={diff:?}, min_delta={min_delta:?})",
                    result_id.as_str()
                ),
            )
        }
        LiveEditExpectation::TabCellChanged { result_id, tab_index, row, col } => {
            let b = before.tab_cell(&result_id, tab_index, row, col).ok_or_else(|| {
                format!("{wf_id}: live_edit before-snapshot missing tab cell ({tab_index},{row},{col}) on `{}`", result_id.as_str())
            })?;
            let a = after
                .tab_cell(&result_id, tab_index, row, col)
                .ok_or_else(|| format!("{wf_id}: live_edit after-snapshot missing tab cell ({tab_index},{row},{col}) on `{}`", result_id.as_str()))?;
            ensure(
                b != a,
                format!("{wf_id}: live_edit expected tab cell ({tab_index},{row},{col}) on `{}` to change, stayed {a:?}", result_id.as_str()),
            )
        }
        LiveEditExpectation::Invariant { result_id, scalar_key, tab_cell, graph_id, abs_tol, rel_tol } => {
            verify_invariant(wf_id, before, after, result_id.as_ref(), scalar_key.as_ref(), tab_cell, graph_id.as_ref(), abs_tol, rel_tol)
        }
        LiveEditExpectation::GraphSvgChanged { graph_id } => {
            let b = before
                .graph_svg(&graph_id)
                .ok_or_else(|| format!("{wf_id}: live_edit before-snapshot missing GraphSheet `{}`", graph_id.as_str()))?;
            let a =
                after.graph_svg(&graph_id).ok_or_else(|| format!("{wf_id}: live_edit after-snapshot missing GraphSheet `{}`", graph_id.as_str()))?;
            ensure(b != a, format!("{wf_id}: live_edit expected GraphSheet `{}` SVG to change, but remained identical", graph_id.as_str()))
        }
        LiveEditExpectation::Custom(check) => check(before, after, doc),
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_invariant(
    wf_id: &str,
    before: &LiveEditSnapshot,
    after: &LiveEditSnapshot,
    result_id: Option<&SheetId>,
    scalar_key: Option<&(String, Option<usize>)>,
    tab_cell: Option<(usize, usize, usize)>,
    graph_id: Option<&SheetId>,
    abs_tol: f64,
    rel_tol: f64,
) -> TestRes {
    let mut checked = false;
    if let (Some(rid), Some((key, ds))) = (result_id, scalar_key) {
        let b =
            before.scalar(rid, key, *ds).ok_or_else(|| format!("{wf_id}: invariant before-snapshot missing scalar `{key}` on `{}`", rid.as_str()))?;
        let a =
            after.scalar(rid, key, *ds).ok_or_else(|| format!("{wf_id}: invariant after-snapshot missing scalar `{key}` on `{}`", rid.as_str()))?;
        ensure(
            approx_eq(a, b, abs_tol, rel_tol),
            format!("{wf_id}: invariant violated for scalar `{key}` on `{}` (before={b:?}, after={a:?})", rid.as_str()),
        )?;
        checked = true;
    }
    if let (Some(rid), Some((tab_index, row, col))) = (result_id, tab_cell) {
        let b = before
            .tab_cell(rid, tab_index, row, col)
            .ok_or_else(|| format!("{wf_id}: invariant before-snapshot missing tab cell ({tab_index},{row},{col}) on `{}`", rid.as_str()))?;
        let a = after
            .tab_cell(rid, tab_index, row, col)
            .ok_or_else(|| format!("{wf_id}: invariant after-snapshot missing tab cell ({tab_index},{row},{col}) on `{}`", rid.as_str()))?;
        let tc = ReferenceTabCell { tab_index, row, col, abs_tol, rel_tol };
        compare_result_cell(wf_id, &tc, a, b)?;
        checked = true;
    }
    if let Some(gid) = graph_id {
        let b = before.graph_svg(gid).ok_or_else(|| format!("{wf_id}: invariant before-snapshot missing GraphSheet `{}`", gid.as_str()))?;
        let a = after.graph_svg(gid).ok_or_else(|| format!("{wf_id}: invariant after-snapshot missing GraphSheet `{}`", gid.as_str()))?;
        ensure(b == a, format!("{wf_id}: invariant expected GraphSheet `{}` SVG to remain identical, but it changed", gid.as_str()))?;
        checked = true;
    }
    ensure(checked, format!("{wf_id}: LiveEditExpectation::Invariant specified no target scalar, tab_cell, or graph_id"))
}
