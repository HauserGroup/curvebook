#![forbid(unsafe_code)]

//! Interactive `eframe` + `egui` + `wgpu` native desktop window host for CurveBook.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};

use curvebook_render::RenderTargetSpec;
use curvebook_text::TrueTypeFont;
use curvebook_ui_egui::control_io::MAX_CONTROL_LINE_BYTES;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::rasterizer::EguiScreenshotRegions;
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, GeometryStamp, PendingUiIntents, Tokens, UiIntent, UiIntentOutcome, inspect_bridge};
use curvebook_ui_egui::interaction::KeyModifiers;
use curvebook_ui_egui::{
    ControlAuth, ControlPostRequestContext, ControlRequest, ScreenshotRegionTarget, UiApp, is_desktop_host_routed_command, parse_control_request,
};
use eframe::egui;

use crate::desktop_host::{
    CloseDisposition, ControlFrameKind, DesktopEventOutcome, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, LiveControlFrameOutput,
    classify_control_frame, match_routed_desktop_shortcut,
};
pub use crate::desktop_host::{
    DEFAULT_TEMPLATE_FILE_NAME, NATIVE_OPEN_FILE_EXTENSIONS, NATIVE_TEMPLATE_FILE_EXTENSIONS, NativeFileDialogAdapter, NativeFileDialogKind,
    NativeFileDialogRequest, execute_native_import_at_path, menu_action_triggers_native_file_dialog, native_import_replaces_project_file_path,
};

/// Production OS file-dialog backend powered by [`rfd::FileDialog`].
#[derive(Debug, Default)]
pub struct RfdNativeFileDialogAdapter;

impl NativeFileDialogAdapter for RfdNativeFileDialogAdapter {
    fn pick(&self, request: &NativeFileDialogRequest) -> Result<Option<PathBuf>, String> {
        let mut dialog = rfd::FileDialog::new();
        if !request.extensions.is_empty() && request.kind != NativeFileDialogKind::PickFolder {
            let ext_refs: Vec<&str> = request.extensions.iter().map(String::as_str).collect();
            dialog = dialog.add_filter(&request.filter_label, &ext_refs);
        }
        if let Some(ref name) = request.default_file_name
            && request.kind == NativeFileDialogKind::SaveFile
        {
            dialog = dialog.set_file_name(name);
        }
        Ok(match request.kind {
            NativeFileDialogKind::OpenFile => dialog.pick_file(),
            NativeFileDialogKind::PickFolder => dialog.pick_folder(),
            NativeFileDialogKind::SaveFile => dialog.save_file(),
        })
    }
}

const MIN_WINDOW_WIDTH_PT: f32 = 640.0;
const MIN_WINDOW_HEIGHT_PT: f32 = 420.0;
const DEFAULT_WINDOW_WIDTH_PT: f32 = 1280.0;
const DEFAULT_WINDOW_HEIGHT_PT: f32 = 800.0;

type CleanupHook = Box<dyn FnOnce(&mut DesktopHostRuntime) -> Result<(), String>>;

/// UI-thread callback invoked after each live-control request executes.
pub type LiveControlPostHook = Box<dyn FnMut(&UiApp, &ControlPostRequestContext<'_>) -> Result<(), String>>;

/// One inbound control request queued for execution on the live UI thread.
pub struct LiveControlRequest {
    pub raw_bytes: Vec<u8>,
    pub auth: ControlAuth,
    pub conn_authenticated: bool,
    pub reply_tx: mpsc::Sender<Result<LiveControlFrameOutput, String>>,
}

/// Non-blocking channel and repaint/shutdown handles wiring a live control server into [`CurveBookNativeApp`].
pub struct LiveControlChannel {
    pub request_rx: mpsc::Receiver<LiveControlRequest>,
    pub egui_ctx_cell: Arc<OnceLock<egui::Context>>,
    pub shutdown: Arc<AtomicBool>,
    pub post_hook: Option<LiveControlPostHook>,
}

impl LiveControlChannel {
    /// Attach a UI-thread post-request hook invoked after each live-control request executes.
    #[must_use]
    pub fn with_post_hook(mut self, hook: LiveControlPostHook) -> Self {
        self.post_hook = Some(hook);
        self
    }
}

/// Build `egui` font definitions using `curvebook_ui_egui::egui_shell::fonts`, optionally prepending an extra CLI TTF override.
#[must_use]
pub fn build_egui_font_definitions(extra_ttf: Option<&[u8]>) -> egui::FontDefinitions {
    let mut defs = curvebook_ui_egui::egui_shell::fonts::build_egui_font_definitions().definitions;
    if let Some(bytes) = extra_ttf
        && TrueTypeFont::parse(bytes).is_some()
    {
        let key = String::from("cli-ttf-override");
        defs.font_data.insert(key.clone(), Arc::new(egui::FontData::from_owned(bytes.to_vec())));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            defs.families.entry(family).or_default().insert(0, key.clone());
        }
    }
    defs
}

/// Build and install CurveBook's desktop font definitions (including `BOLD_FONT_FAMILY` and
/// `ALIGNED_NUMERAL_FONT_FAMILY` metadata) into `ctx`.
pub fn install_desktop_font_definitions(ctx: &egui::Context, extra_ttf: Option<&[u8]>) {
    let fonts = build_egui_font_definitions(extra_ttf);
    curvebook_ui_egui::egui_shell::fonts::install_font_definitions(ctx, fonts);
}

struct DeferredLiveRequest {
    req: LiveControlRequest,
    scroll_attempted: bool,
}

enum PendingLiveUiActionKind {
    PointerGesture {
        action: String,
        widget_id: Option<String>,
        pos: egui::Pos2,
        power_hover_meta: Option<(f64, Option<f64>, Option<f64>)>,
        is_right_click_coords: bool,
    },
    TypeText {
        text: String,
        widget_id: Option<String>,
        submit: bool,
    },
    PressKey {
        key_raw: String,
    },
    SwitchTab {
        response_payload: serde_json::Value,
    },
}

struct PendingLiveUiAction {
    req: LiveControlRequest,
    request_id: serde_json::Value,
    method: String,
    kind: PendingLiveUiActionKind,
    scroll_settle_budget: u8,
    prev_undo_depth: usize,
    prev_redo_depth: usize,
    prev_dirty: bool,
    prev_mutation_seq: u64,
}

/// Interactive `eframe::App` wrapping a [`DesktopHostRuntime`] session.
fn has_non_null_field(params: &serde_json::Value, key: &str) -> bool {
    params.get(key).is_some_and(|v| !v.is_null())
}

fn should_bypass_native_file_picker(command_id: &str, params: &serde_json::Value) -> bool {
    match command_id {
        "project.merge" => has_non_null_field(params, "bytes_base64"),
        "project.exportPzfx" => has_non_null_field(params, "path") || has_non_null_field(params, "return_bytes"),
        "project.instantiateTemplate" => {
            has_non_null_field(params, "bytes_base64") || (has_non_null_field(params, "path") && !has_non_null_field(params, "new_window"))
        }
        "graph.insertImage" | "layout.insertImage" => {
            has_non_null_field(params, "path") || has_non_null_field(params, "bytes_base64") || has_non_null_field(params, "asset_id")
        }
        "table.importCsv" => has_non_null_field(params, "path") || has_non_null_field(params, "csv"),
        "table.importXlsx" => {
            (has_non_null_field(params, "path") && !has_non_null_field(params, "merge") && !has_non_null_field(params, "new_window"))
                || has_non_null_field(params, "bytes_base64")
                || has_non_null_field(params, "bytes")
        }
        "table.importPrism" | "table.importPzfx" => {
            (has_non_null_field(params, "path") && !has_non_null_field(params, "merge") && !has_non_null_field(params, "new_window"))
                || has_non_null_field(params, "bytes_base64")
                || has_non_null_field(params, "xml")
        }
        "export.svg"
        | "file.export_svg"
        | "graph.exportSvg"
        | "export.pdf"
        | "file.export_pdf"
        | "graph.exportPdf"
        | "export.png"
        | "file.export_png"
        | "graph.exportPng"
        | "export.tiff"
        | "graph.exportEmf"
        | "layout.exportEmf"
        | "table.exportCsv"
        | "analysis.exportJson"
        | "analysis.exportCsv"
        | "result.generateReport"
        | "project.saveTemplate" => has_non_null_field(params, "path"),
        _ => false,
    }
}

pub struct CurveBookNativeApp {
    runtime: DesktopHostRuntime,
    primary_window_id: DesktopWindowId,
    last_active_sheet_id: Option<curvebook_doc::SheetId>,
    last_frame_size: [u32; 2],
    last_scale_factor: f32,
    last_title: String,
    hovered_paths_active: bool,
    close_dialog_open: bool,
    quit_in_progress: bool,
    window_switcher_rects: std::collections::BTreeMap<DesktopWindowId, (egui::Rect, egui::Rect)>,
    close_confirmed: bool,
    status_banner: Option<String>,
    on_exit_cleanup: Option<CleanupHook>,
    live_control: Option<LiveControlChannel>,
    frame_seq: u64,
    last_geometry_stamp: Option<GeometryStamp>,
    inspect_meta: FrameInspectMeta,
    last_regions: EguiScreenshotRegions,
    pending_intents: PendingUiIntents,
    discarded_carry_passes: u8,
    deferred_live_requests: Vec<DeferredLiveRequest>,
    pending_synthetic_egui_batches: VecDeque<Vec<egui::Event>>,
    pending_live_action: Option<PendingLiveUiAction>,
    pending_live_outcomes: Vec<UiIntentOutcome>,
    pending_live_error: Option<curvebook_engine::EngineError>,
    pending_live_ever_wanted_kb: bool,
    last_egui_wants_kb: bool,
    menu_popup_was_open: bool,
    pending_device_lost: Option<Arc<curvebook_platform::sync::Mutex<Option<String>>>>,
    interactive_graphics_dialogs: bool,
    #[cfg(target_os = "macos")]
    mac_menu_host: crate::mac_menu::MacNativeMenuHost,
    #[cfg(target_os = "macos")]
    last_mac_menu_fingerprint: Option<MacMenuFingerprint>,
}

#[cfg(target_os = "macos")]
type MacMenuFingerprint =
    (DesktopWindowId, u64, String, curvebook_ui_egui::menus::ShortcutRegistry, bool, Option<curvebook_doc::SheetId>, bool, usize, usize, bool, bool);

#[cfg(target_os = "macos")]
fn mac_menu_fingerprint(win: &crate::desktop_host::DesktopWindowSession) -> MacMenuFingerprint {
    let app = win.app();
    (
        win.id(),
        app.mutation_seq(),
        app.locale.clone(),
        app.shortcuts.clone(),
        win.in_window_menus(),
        app.document().active_sheet_id.clone(),
        app.inline_editor.active,
        app.session().undo_depth(),
        app.session().redo_depth(),
        app.session().is_dirty(),
        app.grid.tool.active.is_some(),
    )
}

impl CurveBookNativeApp {
    /// Construct a native app host wrapping an already-initialized [`DesktopHostRuntime`].
    pub fn new(mut runtime: DesktopHostRuntime, primary_window_id: DesktopWindowId, on_exit_cleanup: Option<CleanupHook>) -> Self {
        if !runtime.has_file_dialog_adapter() {
            runtime.set_file_dialog_adapter(Arc::new(RfdNativeFileDialogAdapter));
        }
        Self {
            runtime,
            primary_window_id,
            last_active_sheet_id: None,
            last_frame_size: [0, 0],
            last_scale_factor: 0.0,
            last_title: String::new(),
            hovered_paths_active: false,
            close_dialog_open: false,
            quit_in_progress: false,
            window_switcher_rects: std::collections::BTreeMap::new(),
            close_confirmed: false,
            status_banner: None,
            on_exit_cleanup,
            live_control: None,
            frame_seq: 0,
            last_geometry_stamp: None,
            inspect_meta: FrameInspectMeta::default(),
            last_regions: EguiScreenshotRegions::default(),
            pending_intents: PendingUiIntents::default(),
            discarded_carry_passes: 0,
            deferred_live_requests: Vec::new(),
            pending_synthetic_egui_batches: VecDeque::new(),
            pending_live_action: None,
            pending_live_outcomes: Vec::new(),
            pending_live_error: None,
            pending_live_ever_wanted_kb: false,
            last_egui_wants_kb: false,
            menu_popup_was_open: false,
            pending_device_lost: None,
            interactive_graphics_dialogs: false,
            #[cfg(target_os = "macos")]
            mac_menu_host: crate::mac_menu::MacNativeMenuHost::new(),
            #[cfg(target_os = "macos")]
            last_mac_menu_fingerprint: None,
        }
    }

    /// Borrow the macOS native menu host mirror (for headless parity inspection and windowed sync).
    #[cfg(target_os = "macos")]
    #[must_use]
    pub const fn mac_menu_host(&self) -> &crate::mac_menu::MacNativeMenuHost {
        &self.mac_menu_host
    }

    /// Synchronize macOS `muda` menu descriptors (and live `NSMenu` only if attached on main thread).
    #[cfg(target_os = "macos")]
    pub fn sync_macos_menu_for_window(&mut self, window_id: DesktopWindowId) {
        if let Ok(win) = self.runtime.window(window_id) {
            let chrome = win.chrome_state();
            if let Err(err) = self.mac_menu_host.sync_from_chrome(&chrome) {
                self.status_banner = Some(err);
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn attach_macos_menus_on_windowed_startup(&mut self) {
        let win_id = self.active_window_id();
        if let Ok(win) = self.runtime.window(win_id) {
            let chrome = win.chrome_state();
            if !chrome.menu_bar_visible
                && let Err(err) = self.mac_menu_host.attach_to_nsapp_for_windowed_host(&chrome)
            {
                self.status_banner = Some(err);
            }
        }
    }

    /// Attach a [`LiveControlChannel`] so inbound control requests share this live desktop window.
    #[must_use]
    pub fn with_live_control(mut self, control: LiveControlChannel) -> Self {
        self.live_control = Some(control);
        self
    }

    fn with_device_lost_slot(mut self, slot: Arc<curvebook_platform::sync::Mutex<Option<String>>>, interactive_dialogs: bool) -> Self {
        self.pending_device_lost = Some(slot);
        self.interactive_graphics_dialogs = interactive_dialogs;
        self
    }

    fn poll_pending_device_lost(&mut self) {
        let diagnostic = self.pending_device_lost.as_ref().and_then(|slot| slot.lock().ok().and_then(|mut guard| guard.take()));
        if let Some(msg) = diagnostic {
            self.status_banner = Some(msg.clone());
            if self.interactive_graphics_dialogs {
                show_native_graphics_error_dialog("CurveBook Graphics Device Lost", &msg);
            }
        }
    }

    /// Borrow the underlying [`DesktopHostRuntime`].
    #[must_use]
    pub const fn runtime(&self) -> &DesktopHostRuntime {
        &self.runtime
    }

    /// Mutably borrow the underlying [`DesktopHostRuntime`].
    pub fn runtime_mut(&mut self) -> &mut DesktopHostRuntime {
        &mut self.runtime
    }

    /// Whether shutdown was confirmed (for example via `app.quit` over `--control`).
    #[must_use]
    pub const fn close_confirmed(&self) -> bool {
        self.close_confirmed
    }

    /// Whether the unsaved-changes confirmation modal is currently open.
    #[must_use]
    pub const fn close_dialog_open(&self) -> bool {
        self.close_dialog_open
    }

    /// Whether OS files are currently hovered over the desktop window drop target.
    #[must_use]
    pub const fn is_drop_hover_active(&self) -> bool {
        self.hovered_paths_active
    }

    /// Current top-bar error or warning banner text, if any.
    #[must_use]
    pub fn status_banner(&self) -> Option<&str> {
        self.status_banner.as_deref()
    }

    /// Most recent frame geometry synchronization stamp (`GeometryStamp`).
    #[must_use]
    pub const fn geometry_stamp(&self) -> Option<GeometryStamp> {
        self.last_geometry_stamp
    }

    /// Borrow the latest settled [`FrameInspectMeta`] captured by the `egui` shell pass.
    #[must_use]
    pub const fn inspect_meta(&self) -> &FrameInspectMeta {
        &self.inspect_meta
    }

    /// Drain and process all currently queued live-control requests without blocking rendering.
    pub fn poll_live_control(&mut self, ctx: &egui::Context) -> usize {
        let mut pending = Vec::new();
        if let Some(ref ctrl) = self.live_control {
            let _ = ctrl.egui_ctx_cell.set(ctx.clone());
            while let Ok(req) = ctrl.request_rx.try_recv() {
                pending.push(req);
            }
        }
        if pending.is_empty() {
            return 0;
        }
        let count = pending.len();
        let mut defer_subsequent_commands =
            !self.deferred_live_requests.is_empty() || !self.pending_synthetic_egui_batches.is_empty() || self.pending_live_action.is_some();
        let mut immediate = Vec::new();
        for req in pending {
            if should_defer_to_settled_frame(&req)
                || (defer_subsequent_commands && classify_control_frame(&req.raw_bytes) == ControlFrameKind::Command)
            {
                self.deferred_live_requests.push(DeferredLiveRequest { req, scroll_attempted: false });
                defer_subsequent_commands = true;
            } else {
                immediate.push(req);
            }
        }
        let quit_requested = self.dispatch_pending_live_requests(immediate);
        if quit_requested {
            self.finish_live_quit(ctx);
        } else if !self.deferred_live_requests.is_empty() || !self.pending_synthetic_egui_batches.is_empty() || self.pending_live_action.is_some() {
            ctx.request_repaint();
        }
        count
    }

    fn finish_live_quit(&mut self, ctx: &egui::Context) {
        self.fail_deferred_live_requests("desktop window is closing");
        self.close_confirmed = true;
        self.close_dialog_open = false;
        if let Some(ref ctrl) = self.live_control {
            ctrl.shutdown.store(true, Ordering::Release);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn dispatch_pending_live_requests(&mut self, pending: Vec<LiveControlRequest>) -> bool {
        let mut quit_requested = false;
        let mut post_hook = self.live_control.as_mut().and_then(|ctrl| ctrl.post_hook.take());
        for req in pending {
            if quit_requested {
                let _ = req.reply_tx.send(Err(String::from("desktop window is closing")));
                continue;
            }
            if self.close_dialog_open
                && is_request_authorized(&req)
                && let Some(parsed) = parse_control_request_from_bytes(&req.raw_bytes)
                && parsed.method == "ui.invoke_menu"
            {
                let _ = Self::send_live_engine_error(
                    req,
                    parsed.id,
                    curvebook_doc::ErrorCode::Conflict,
                    "cannot invoke menu command while unsaved changes dialog is open",
                );
                continue;
            }
            let res = self.dispatch_single_live_request(&req.raw_bytes, &req.auth, req.conn_authenticated, &mut post_hook);
            if res.as_ref().is_ok_and(|out| out.quit) {
                quit_requested = true;
            }
            let _ = req.reply_tx.send(res);
        }
        if let Some(ref mut ctrl) = self.live_control {
            ctrl.post_hook = post_hook;
        }
        quit_requested
    }

    fn dispatch_single_live_request(
        &mut self,
        raw_bytes: &[u8],
        auth: &ControlAuth,
        conn_authenticated: bool,
        post_hook: &mut Option<LiveControlPostHook>,
    ) -> Result<LiveControlFrameOutput, String> {
        let win_id = self.active_window_id();
        self.runtime.window_mut(win_id)?.dispatch_live_control_frame_with_hook(raw_bytes, auth, conn_authenticated, |app, hook_ctx| {
            if let Some(ref mut hook) = *post_hook {
                hook(app, hook_ctx)?;
            }
            Ok(())
        })
    }

    /// Pop the next queued synthetic `egui` event batch into `raw_input` for headless or live frame stepping.
    pub fn drain_synthetic_events_into(&mut self, raw_input: &mut egui::RawInput) {
        if let Some(batch) = self.pending_synthetic_egui_batches.pop_front() {
            raw_input.events.extend(batch);
        }
    }

    fn send_live_control_reply(req: LiveControlRequest, id: serde_json::Value, result: Result<serde_json::Value, curvebook_engine::EngineError>) {
        // Promote socket framing cap from 64 KiB to 16 MiB when a deferred GUI command authenticates
        // via per-request `"token"` (`AUD-13-21`).
        let auth_succeeded = (!req.conn_authenticated && is_request_authorized(&req)).then_some(true);
        let reply = curvebook_ui_egui::encode_egui_control_response(id, result);
        let bytes = serde_json::to_vec(&reply).map(|mut b| {
            b.push(b'\n');
            LiveControlFrameOutput { response_bytes: b, quit: false, close_connection: false, auth_succeeded }
        });
        let _ = req.reply_tx.send(bytes.map_err(|e| e.to_string()));
    }

    fn send_live_engine_error(
        req: LiveControlRequest,
        id: serde_json::Value,
        code: curvebook_doc::ErrorCode,
        msg: impl Into<String>,
    ) -> LiveActionDispatch {
        Self::send_live_control_reply(req, id, Err(curvebook_engine::EngineError::with_code(code, msg)));
        LiveActionDispatch::ImmediateReply
    }

    fn send_live_engine_err_val(req: LiveControlRequest, id: serde_json::Value, err: curvebook_engine::EngineError) -> LiveActionDispatch {
        Self::send_live_control_reply(req, id, Err(err));
        LiveActionDispatch::ImmediateReply
    }

    fn capture_mutation_baseline(&self) -> (usize, usize, bool, u64) {
        let win_id = self.active_window_id();
        self.runtime.window(win_id).ok().map_or((0, 0, false, 0), |w| {
            (w.app().session().undo_depth(), w.app().session().redo_depth(), w.app().session().is_dirty(), w.app().mutation_seq())
        })
    }

    fn invoke_live_post_hook(
        &self,
        post_hook: &mut Option<LiveControlPostHook>,
        method: &str,
        baseline: (usize, usize, bool, u64),
        request_ok: bool,
    ) {
        let Some(ref mut hook) = *post_hook else {
            return;
        };
        let win_id = self.active_window_id();
        let Ok(win) = self.runtime.window(win_id) else {
            return;
        };
        let (prev_undo_depth, prev_redo_depth, prev_dirty, prev_mutation_seq) = baseline;
        let mutated = win.app().mutation_seq() != prev_mutation_seq
            || win.app().session().undo_depth() != prev_undo_depth
            || win.app().session().redo_depth() != prev_redo_depth
            || (!prev_dirty && win.app().session().is_dirty());
        let steps = if mutated {
            vec![curvebook_ui_egui::ControlRecordedStep { command_id: method.to_owned(), params: serde_json::Value::Null, mutated: true }]
        } else {
            Vec::new()
        };
        let hook_ctx = ControlPostRequestContext {
            method,
            steps: &steps,
            prev_undo_depth,
            prev_redo_depth,
            prev_dirty,
            prev_mutation_seq,
            rolled_back: false,
            request_ok,
        };
        let _ = hook(win.app(), &hook_ctx);
    }

    fn record_direct_live_mutation(&mut self, method: &str, baseline: (usize, usize, bool, u64)) {
        let win_id = self.active_window_id();
        if let Ok(win) = self.runtime.window_mut(win_id) {
            win.app_mut().sync_views();
            let _ = win.record_ui_mutation_checkpoint(Some(method), baseline.0, baseline.1, baseline.2, baseline.3);
        }
    }

    fn begin_pending_live_action(
        &mut self,
        req: LiveControlRequest,
        request_id: serde_json::Value,
        method: &str,
        kind: PendingLiveUiActionKind,
        batches: Vec<Vec<egui::Event>>,
    ) -> LiveActionDispatch {
        let baseline = self.capture_mutation_baseline();
        self.pending_live_outcomes.clear();
        self.pending_live_error = None;
        self.pending_live_ever_wanted_kb = false;
        self.pending_synthetic_egui_batches.extend(batches);
        // Append one empty settle pass so post-event layout & `PendingUiIntents` commit before reply/inspect.
        self.pending_synthetic_egui_batches.push_back(Vec::new());
        let scroll_settle_budget = match &kind {
            PendingLiveUiActionKind::PointerGesture { action, .. } if action == "scroll" || action == "wheel" => 96,
            _ => 0,
        };
        self.pending_live_action = Some(PendingLiveUiAction {
            req,
            request_id,
            method: method.to_owned(),
            kind,
            scroll_settle_budget,
            prev_undo_depth: baseline.0,
            prev_redo_depth: baseline.1,
            prev_dirty: baseline.2,
            prev_mutation_seq: baseline.3,
        });
        LiveActionDispatch::EguiQueued
    }

    fn complete_pending_live_action(&mut self, ctx: &egui::Context, post_hook: &mut Option<LiveControlPostHook>) -> bool {
        let Some(pending) = self.pending_live_action.take() else {
            return false;
        };
        let outcomes = std::mem::take(&mut self.pending_live_outcomes);
        let pending_err = self.pending_live_error.take();
        let ever_wanted_kb = std::mem::take(&mut self.pending_live_ever_wanted_kb);
        let baseline = (pending.prev_undo_depth, pending.prev_redo_depth, pending.prev_dirty, pending.prev_mutation_seq);

        let win_id = self.active_window_id();
        if let Ok(win) = self.runtime.window_mut(win_id) {
            inspect_bridge::clear_stale_non_grid_textbox_focus(
                win.app_mut(),
                &self.inspect_meta,
                ever_wanted_kb || self.last_egui_wants_kb,
                ctx.egui_wants_keyboard_input(),
            );
            win.app_mut().sync_views();
        }

        if let Some(err) = pending_err {
            Self::send_live_control_reply(pending.req, pending.request_id, Err(err));
            self.invoke_live_post_hook(post_hook, &pending.method, baseline, false);
            return true;
        }

        let result_val = match pending.kind {
            PendingLiveUiActionKind::PointerGesture { action, widget_id, pos, power_hover_meta, is_right_click_coords } => {
                if is_right_click_coords
                    && !outcomes.iter().any(|o| !matches!(o, UiIntentOutcome::ShellUpdated))
                    && let Ok(win) = self.runtime.window_mut(win_id)
                {
                    inspect_bridge::clear_right_click_miss_context_menu(win.app_mut());
                }
                let mut payload = inspect_bridge::format_pointer_gesture_result(&action, widget_id.as_deref(), pos, &outcomes);
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert(String::from("egui_dispatched"), serde_json::json!(true));
                    if let Some((frac, hn, hp)) = power_hover_meta {
                        obj.insert(String::from("hover_fraction_x"), serde_json::json!(frac));
                        obj.insert(String::from("hover_n"), serde_json::json!(hn));
                        obj.insert(String::from("hover_power"), serde_json::json!(hp));
                    }
                }
                payload
            }
            PendingLiveUiActionKind::TypeText { text, widget_id, submit } => {
                let Ok(win) = self.runtime.window(win_id) else {
                    Self::send_live_control_reply(
                        pending.req,
                        pending.request_id,
                        Err(curvebook_engine::EngineError::with_code(curvebook_doc::ErrorCode::InternalError, "missing active window")),
                    );
                    return true;
                };
                inspect_bridge::format_type_text_result(win.app(), &text, widget_id.as_deref(), submit, &outcomes)
            }
            PendingLiveUiActionKind::PressKey { key_raw } => inspect_bridge::format_press_key_result(&key_raw, &outcomes),
            PendingLiveUiActionKind::SwitchTab { response_payload } => response_payload,
        };

        Self::send_live_control_reply(pending.req, pending.request_id, Ok(result_val));
        self.invoke_live_post_hook(post_hook, &pending.method, baseline, true);
        true
    }

    fn flush_deferred_live_requests(&mut self, ctx: &egui::Context, scroll_active_this_frame: bool) {
        if ctx.will_discard() {
            return;
        }
        let Some(stamp) = self.last_geometry_stamp.filter(|s| s.settled) else {
            return;
        };
        if !self.pending_synthetic_egui_batches.is_empty() {
            ctx.request_repaint();
            return;
        }
        if let Some(ref mut pending) = self.pending_live_action
            && pending.scroll_settle_budget > 0
            && scroll_active_this_frame
        {
            pending.scroll_settle_budget = pending.scroll_settle_budget.saturating_sub(1);
            ctx.request_repaint();
            return;
        }

        let mut post_hook = self.live_control.as_mut().and_then(|ctrl| ctrl.post_hook.take());
        let completed_pending = self.complete_pending_live_action(ctx, &mut post_hook);
        if completed_pending {
            ctx.request_repaint();
        }

        if self.deferred_live_requests.is_empty() {
            if let Some(ref mut ctrl) = self.live_control {
                ctrl.post_hook = post_hook;
            }
            return;
        }

        let queued = std::mem::take(&mut self.deferred_live_requests);
        let mut iter = queued.into_iter();
        let mut quit_requested = false;
        let mut needs_repaint = false;
        while let Some(deferred) = iter.next() {
            if quit_requested {
                let _ = deferred.req.reply_tx.send(Err(String::from("desktop window is closing")));
                continue;
            }
            if !is_request_authorized(&deferred.req) {
                let res =
                    self.dispatch_single_live_request(&deferred.req.raw_bytes, &deferred.req.auth, deferred.req.conn_authenticated, &mut post_hook);
                let _ = deferred.req.reply_tx.send(res);
                continue;
            }
            let Some(parsed) = parse_control_request_from_bytes(&deferred.req.raw_bytes) else {
                let res =
                    self.dispatch_single_live_request(&deferred.req.raw_bytes, &deferred.req.auth, deferred.req.conn_authenticated, &mut post_hook);
                if res.as_ref().is_ok_and(|out| out.quit) {
                    quit_requested = true;
                }
                let _ = deferred.req.reply_tx.send(res);
                needs_repaint = true;
                continue;
            };

            let dispatch = match parsed.method.as_str() {
                "ui.inspect" => {
                    let res = self.handle_live_ui_inspect(ctx, stamp, &parsed.params);
                    Self::send_live_control_reply(deferred.req, parsed.id, res);
                    LiveActionDispatch::ImmediateReply
                }
                "ui.screenshot" => {
                    let settled_bytes = self.prepare_settled_live_request_bytes(&deferred.req.raw_bytes, stamp);
                    let settled_params = parse_control_request_from_bytes(&settled_bytes).map_or(parsed.params, |req| req.params);
                    let res = self.handle_live_ui_screenshot(&parsed.id, stamp, &settled_params);
                    Self::send_live_control_reply(deferred.req, parsed.id, res);
                    LiveActionDispatch::ImmediateReply
                }
                "ui.pointer_gesture" => self.try_enqueue_live_pointer_gesture(ctx, deferred, parsed, &mut post_hook),
                "ui.type_text" => self.try_enqueue_live_type_text(ctx, deferred, parsed),
                "ui.press_key" => self.try_enqueue_live_press_key(ctx, deferred.req, parsed),
                "ui.switch_tab" => self.try_dispatch_live_switch_tab(deferred, parsed, &mut post_hook),
                _ => {
                    let res = self.dispatch_single_live_request(
                        &deferred.req.raw_bytes,
                        &deferred.req.auth,
                        deferred.req.conn_authenticated,
                        &mut post_hook,
                    );
                    if res.as_ref().is_ok_and(|out| out.quit) {
                        quit_requested = true;
                    }
                    let _ = deferred.req.reply_tx.send(res);
                    needs_repaint = true;
                    if !iter.as_slice().is_empty() { LiveActionDispatch::YieldFrame } else { LiveActionDispatch::ImmediateReply }
                }
            };

            match dispatch {
                LiveActionDispatch::ImmediateReply => {}
                LiveActionDispatch::EguiQueued | LiveActionDispatch::YieldFrame => {
                    self.deferred_live_requests.extend(iter);
                    ctx.request_repaint();
                    break;
                }
                LiveActionDispatch::NeedsScrollRetry(retry_req) => {
                    self.deferred_live_requests.push(retry_req);
                    self.deferred_live_requests.extend(iter);
                    ctx.request_repaint();
                    break;
                }
            }
        }
        if needs_repaint {
            ctx.request_repaint();
        }
        if let Some(ref mut ctrl) = self.live_control {
            ctrl.post_hook = post_hook;
        }
        if quit_requested {
            self.finish_live_quit(ctx);
        }
    }

    fn handle_live_ui_screenshot(
        &mut self,
        request_id: &serde_json::Value,
        stamp: GeometryStamp,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, curvebook_engine::EngineError> {
        use curvebook_doc::ErrorCode;
        use curvebook_engine::EngineError;

        let empty_map = serde_json::Map::new();
        let obj = match params {
            serde_json::Value::Null => &empty_map,
            serde_json::Value::Object(map) => map,
            _ => return Err(EngineError::with_code(ErrorCode::BadInput, "`ui.screenshot` params must be a JSON object")),
        };

        let region_raw = match obj.get("region").or_else(|| obj.get("target")) {
            None | Some(serde_json::Value::Null) => "full_window",
            Some(v) => v
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`region`/`target` must be a non-empty string"))?,
        };
        let region = match region_raw.to_ascii_lowercase().as_str() {
            "full_window" | "full" | "window" | "workspace" | "viewport" => ScreenshotRegionTarget::FullWindow,
            "canvas" => ScreenshotRegionTarget::Canvas,
            "dialog" => ScreenshotRegionTarget::Dialog,
            other => {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported screenshot region `{other}`")));
            }
        };

        let parse_dim = |key: &str, default_val: u32| -> Result<u32, EngineError> {
            match obj.get(key) {
                None | Some(serde_json::Value::Null) => Ok(default_val),
                Some(v) => {
                    let n = v
                        .as_u64()
                        .and_then(|u| u32::try_from(u).ok())
                        .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, format!("`{key}` must be a positive integer")))?;
                    if !(64..=4096).contains(&n) {
                        return Err(EngineError::with_code(ErrorCode::BadInput, format!("`{key}` must be in 64..=4096 (got {n})")));
                    }
                    Ok(n)
                }
            }
        };
        let width_px = parse_dim("width_px", stamp.physical_width_px.clamp(64, 4096))?;
        let height_px = parse_dim("height_px", stamp.physical_height_px.clamp(64, 4096))?;
        let dpi = if let Some(v) = obj.get("dpi").filter(|v| !v.is_null()) {
            let n = v
                .as_u64()
                .and_then(|u| u16::try_from(u).ok())
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`dpi` must be an integer in 72..=600"))?;
            if !(72..=600).contains(&n) {
                return Err(EngineError::with_code(ErrorCode::BadInput, format!("`dpi` must be in 72..=600 (got {n})")));
            }
            n
        } else if let Some(v) = obj.get("scale_factor").filter(|v| !v.is_null()) {
            let sf = v
                .as_f64()
                .filter(|x| x.is_finite())
                .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`scale_factor` must be a finite number"))?;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let computed = (96.0 * sf.clamp(0.5, 6.0)).round().clamp(72.0, 600.0) as u16;
            computed
        } else {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let computed = (96.0 * f64::from(stamp.scale_factor()).clamp(0.5, 6.0)).round().clamp(72.0, 600.0) as u16;
            computed
        };

        let raw_dest_path = match obj.get("path") {
            None | Some(serde_json::Value::Null) => None,
            Some(v) => {
                let s = v
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| EngineError::with_code(ErrorCode::BadInput, "`path` must be a non-empty string"))?;
                if s.contains('\0') {
                    return Err(EngineError::with_code(ErrorCode::BadInput, "`path` must not contain NUL bytes"));
                }
                Some(s.to_owned())
            }
        };

        let win_id = self.active_window_id();
        let win = self.runtime.window_mut(win_id).map_err(|e| EngineError::with_code(ErrorCode::InternalError, e))?;

        let validated_path = if let Some(raw) = raw_dest_path.as_deref() {
            let validated = win.app().session().validate_host_write_path(raw)?;
            if let Ok(meta) = std::fs::symlink_metadata(&validated)
                && (meta.file_type().is_symlink() || meta.is_dir())
            {
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("refusing to write screenshot to symlink or directory `{validated}`"),
                ));
            }
            Some(validated)
        } else {
            None
        };

        let (png, actual_w, actual_h) =
            win.app_mut().render_egui_shell_screenshot_png(RenderTargetSpec { width_px, height_px, dpi }, region).map_err(|err| {
                let msg = err.to_string();
                if msg.contains("no modal dialog") || msg.contains("unavailable") {
                    EngineError::with_code(ErrorCode::Conflict, msg)
                } else {
                    EngineError::with_code(ErrorCode::BadInput, msg)
                }
            })?;

        let final_payload = finalize_live_screenshot_payload_with_limit(
            request_id,
            region_raw,
            actual_w,
            actual_h,
            dpi,
            &png,
            validated_path.as_deref(),
            MAX_CONTROL_LINE_BYTES,
        )?;

        if let Some(ref dest) = validated_path {
            win.app().session().write_host_file_atomic(dest, &png)?;
        }

        Ok(final_payload)
    }

    fn handle_live_ui_inspect(
        &self,
        ctx: &egui::Context,
        stamp: GeometryStamp,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, curvebook_engine::EngineError> {
        let mut parsed_params = inspect_bridge::parse_inspect_params(params)?;
        if params.get("metrics").is_none() && params.get("viewport").is_none() {
            let scale = f64::from(stamp.scale_factor());
            let logical_w = (f64::from(stamp.physical_width_px) / scale).max(160.0);
            let logical_h = (f64::from(stamp.physical_height_px) / scale).max(120.0);
            parsed_params.metrics = curvebook_ui_egui::interaction::ViewportMetrics::new(logical_w, logical_h, scale);
        }
        let win = self
            .runtime
            .window(self.active_window_id())
            .map_err(|e| curvebook_engine::EngineError::with_code(curvebook_doc::ErrorCode::InternalError, e))?;
        let focused_egui_id = ctx.memory(|m| m.focused());
        let snap = inspect_bridge::build_live_ui_tree_snapshot(inspect_bridge::LiveInspectSnapshotInput {
            app: win.app(),
            views: &win.app().egui_views,
            meta: &self.inspect_meta,
            focused_egui_id,
            host_close_dialog_open: self.close_dialog_open,
            scope: parsed_params.scope,
            include_bounds: parsed_params.include_bounds,
            metrics: parsed_params.metrics,
        });
        serde_json::to_value(snap).map_err(|e| curvebook_engine::EngineError::with_code(curvebook_doc::ErrorCode::InternalError, e.to_string()))
    }

    fn try_enqueue_live_pointer_gesture(
        &mut self,
        _ctx: &egui::Context,
        mut deferred: DeferredLiveRequest,
        parsed: ControlRequest,
        post_hook: &mut Option<LiveControlPostHook>,
    ) -> LiveActionDispatch {
        let params = &parsed.params;
        let target = match inspect_bridge::parse_pointer_gesture_params(params, if params.get("delta_x").is_none() { -120.0 } else { 0.0 }) {
            Ok(p) => p,
            Err(err) => return Self::send_live_engine_err_val(deferred.req, parsed.id, err),
        };

        let win_id = self.active_window_id();
        let is_human_path_only = self.runtime.window(win_id).is_ok_and(|w| w.app().is_human_path_only());
        if is_human_path_only && (target.widget_id.is_some() || target.to_widget_id.is_some()) {
            return Self::send_live_engine_err_val(
                deferred.req,
                parsed.id,
                curvebook_engine::EngineError::with_code(
                    curvebook_doc::ErrorCode::Conflict,
                    "direct `ui.pointer_gesture(widget_id)` is blocked in human-path-only mode",
                )
                .with_details(serde_json::json!({ "code": "human_path_only_policy" })),
            );
        }

        if let Some(wid) = target.widget_id
            && target.explicit_xy.is_none()
            && !wid.starts_with("formula_bar.open_col.")
            && !wid.starts_with("canvas.point.")
            && inspect_bridge::resolve_widget_rect_in_meta(&self.inspect_meta, wid).is_none()
            && !deferred.scroll_attempted
            && let Ok(win) = self.runtime.window_mut(win_id)
            && inspect_bridge::ensure_widget_scrolled_into_view_state(win.app_mut(), target.metrics, wid)
        {
            deferred.scroll_attempted = true;
            return LiveActionDispatch::NeedsScrollRetry(deferred);
        }

        if let Some(wid) = target.widget_id
            && let Some(col_str) = wid.strip_prefix("formula_bar.open_col.")
            && let Ok(col) = col_str.parse::<usize>()
        {
            return self.try_open_live_formula_bar(deferred.req, parsed.id, wid, col, &target, post_hook);
        }

        let mut power_hover_meta: Option<(f64, Option<f64>, Option<f64>)> = None;
        let pos = if target.widget_id == Some("power.curve_hover") {
            let Ok(win) = self.runtime.window_mut(win_id) else {
                return Self::send_live_engine_error(deferred.req, parsed.id, curvebook_doc::ErrorCode::InternalError, "missing active window");
            };
            match inspect_bridge::resolve_power_curve_hover_target(win.app_mut(), &self.inspect_meta, params) {
                Ok(hover) => {
                    power_hover_meta = Some((hover.fraction_x, hover.hovered_pair.map(|(n, _)| n), hover.hovered_pair.map(|(_, p)| p)));
                    hover.pos
                }
                Err(err) => return Self::send_live_engine_err_val(deferred.req, parsed.id, err),
            }
        } else if let Some((x, y)) = target.explicit_xy {
            #[allow(clippy::cast_possible_truncation)]
            egui::pos2(x as f32, y as f32)
        } else if let Some(wid) = target.widget_id {
            let Ok(win) = self.runtime.window(win_id) else {
                return Self::send_live_engine_error(deferred.req, parsed.id, curvebook_doc::ErrorCode::InternalError, "missing active window");
            };
            match inspect_bridge::resolve_widget_center_in_meta(win.app(), &self.inspect_meta, target.metrics, wid) {
                Ok((center, enabled, reason)) => {
                    if !enabled && !matches!(target.kind.as_str(), "hover" | "move") {
                        let msg = reason.unwrap_or_else(|| format!("widget `{wid}` is disabled"));
                        return Self::send_live_engine_error(deferred.req, parsed.id, curvebook_doc::ErrorCode::Conflict, msg);
                    }
                    center
                }
                Err(err) => return Self::send_live_engine_err_val(deferred.req, parsed.id, err),
            }
        } else {
            return Self::send_live_engine_error(
                deferred.req,
                parsed.id,
                curvebook_doc::ErrorCode::BadInput,
                "ui.pointer_gesture requires either `widget_id` or finite `x`/`y` coordinates",
            );
        };

        if !pos.x.is_finite() || !pos.y.is_finite() {
            return Self::send_live_engine_error(
                deferred.req,
                parsed.id,
                curvebook_doc::ErrorCode::BadInput,
                "resolved pointer coordinates must be finite",
            );
        }

        let end_pos = if target.kind == "drag" {
            if let Some((tx, ty)) = target.to_xy {
                #[allow(clippy::cast_possible_truncation)]
                Some(egui::pos2(tx as f32, ty as f32))
            } else if let Some(to_wid) = target.to_widget_id {
                let Ok(win) = self.runtime.window(win_id) else {
                    return Self::send_live_engine_error(deferred.req, parsed.id, curvebook_doc::ErrorCode::InternalError, "missing active window");
                };
                match inspect_bridge::resolve_widget_center_in_meta(win.app(), &self.inspect_meta, target.metrics, to_wid) {
                    Ok((center, _, _)) => Some(center),
                    Err(err) => return Self::send_live_engine_err_val(deferred.req, parsed.id, err),
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Ok(win) = self.runtime.window_mut(win_id) {
            let tracked = end_pos.unwrap_or(pos);
            win.app_mut().interaction.last_logical_pos = Some([f64::from(tracked.x), f64::from(tracked.y)]);
        }

        let batches = inspect_bridge::build_pointer_gesture_batches(&target, pos, end_pos);
        let is_right_click_coords = target.kind == "right_click" && target.widget_id.is_none();
        let widget_id = target.widget_id.map(str::to_owned);
        let action = target.kind;
        self.begin_pending_live_action(
            deferred.req,
            parsed.id,
            &parsed.method,
            PendingLiveUiActionKind::PointerGesture { action, widget_id, pos, power_hover_meta, is_right_click_coords },
            batches,
        )
    }

    fn try_open_live_formula_bar(
        &mut self,
        req: LiveControlRequest,
        request_id: serde_json::Value,
        wid: &str,
        col: usize,
        target: &inspect_bridge::ParsedPointerGestureParams<'_>,
        post_hook: &mut Option<LiveControlPostHook>,
    ) -> LiveActionDispatch {
        let win_id = self.active_window_id();
        let baseline = self.capture_mutation_baseline();
        let badge_wid = format!("mv_formula.role_badge.{col}");
        if let Some(entry) = inspect_bridge::resolve_widget_rect_in_meta(&self.inspect_meta, &badge_wid)
            && !entry.enabled
        {
            let msg = entry.disabled_reason.clone().unwrap_or_else(|| format!("widget `{wid}` is disabled"));
            return Self::send_live_engine_error(req, request_id, curvebook_doc::ErrorCode::Conflict, msg);
        }
        let Ok(win) = self.runtime.window_mut(win_id) else {
            return Self::send_live_engine_error(req, request_id, curvebook_doc::ErrorCode::InternalError, "missing active window");
        };
        if !win.app().grid.formula_bar.active
            && let Err(err) = curvebook_ui_egui::dispatch_ui_command(win.app_mut(), "ui.table.openFormulaBar", &serde_json::json!({ "col": col }))
        {
            return Self::send_live_engine_error(req, request_id, curvebook_doc::ErrorCode::Conflict, err);
        }
        let active = win.app().grid.formula_bar.active;
        self.record_direct_live_mutation("ui.pointer_gesture", baseline);
        Self::send_live_control_reply(
            req,
            request_id,
            Ok(serde_json::json!({
                "handled": true,
                "kind": target.kind,
                "action": target.kind,
                "widget_id": wid,
                "formula_bar_active": active,
                "egui_dispatched": true,
            })),
        );
        self.invoke_live_post_hook(post_hook, "ui.pointer_gesture", baseline, true);
        LiveActionDispatch::YieldFrame
    }

    fn try_enqueue_live_type_text(&mut self, ctx: &egui::Context, mut deferred: DeferredLiveRequest, parsed: ControlRequest) -> LiveActionDispatch {
        let type_params = match inspect_bridge::parse_type_text_params(&parsed.params) {
            Ok(p) => p,
            Err(err) => return Self::send_live_engine_err_val(deferred.req, parsed.id, err),
        };
        let text = type_params.text.to_owned();
        let clear_param = type_params.clear_param;
        let submit = type_params.submit;
        let widget_id = type_params.widget_id.map(str::to_owned);
        let metrics = type_params.metrics;

        let win_id = self.active_window_id();
        let egui_wants_kb = ctx.egui_wants_keyboard_input();
        let focused_egui_id = ctx.memory(|m| m.focused());
        let live_focus_wid: Option<String> = focused_egui_id
            .and_then(|eid| self.inspect_meta.find_by_egui_id(eid))
            .map(|e| e.widget_id.clone())
            .or_else(|| self.runtime.window(win_id).ok().and_then(|w| w.app().focus_widget_id.clone()));

        let is_human_path_only = self.runtime.window(win_id).is_ok_and(|w| w.app().is_human_path_only());
        if is_human_path_only
            && let Some(ref wid) = widget_id
            && live_focus_wid.as_deref() != Some(wid.as_str())
        {
            return Self::send_live_engine_err_val(
                deferred.req,
                parsed.id,
                curvebook_engine::EngineError::with_code(
                    curvebook_doc::ErrorCode::Conflict,
                    format!("focus-jumping `ui.type_text(widget_id = {wid})` is blocked in human-path-only mode"),
                )
                .with_details(serde_json::json!({ "code": "human_path_only_policy" })),
            );
        }

        if let Some(ref wid) = widget_id
            && !is_human_path_only
            && inspect_bridge::resolve_widget_rect_in_meta(&self.inspect_meta, wid).is_none()
            && !deferred.scroll_attempted
            && let Ok(win) = self.runtime.window_mut(win_id)
            && inspect_bridge::ensure_widget_scrolled_into_view_state(win.app_mut(), metrics, wid)
        {
            deferred.scroll_attempted = true;
            return LiveActionDispatch::NeedsScrollRetry(deferred);
        }

        let modal_backdrop_active = self.runtime.window(win_id).ok().is_some_and(|w| {
            inspect_bridge::HybridCoexistenceArbiter::evaluate_with_views(w.app(), &w.app().egui_views, self.close_dialog_open).modal_backdrop_active
        });

        let mut click_prefix_batches: Vec<Vec<egui::Event>> = Vec::new();
        if let Some(ref wid) = widget_id {
            if is_human_path_only {
                if let Ok(win) = self.runtime.window(win_id)
                    && let Some(pos) = inspect_bridge::find_non_grid_textbox_focus_center(win.app(), &self.inspect_meta, metrics, egui_wants_kb)
                {
                    click_prefix_batches.extend(inspect_bridge::click_pos_batches(pos, egui::PointerButton::Primary, egui::Modifiers::NONE));
                    click_prefix_batches.push(Vec::new());
                }
            } else {
                let Ok(win) = self.runtime.window(win_id) else {
                    return Self::send_live_engine_error(deferred.req, parsed.id, curvebook_doc::ErrorCode::InternalError, "missing active window");
                };
                let already_selected_grid_cell =
                    inspect_bridge::is_grid_cell_widget_selected_without_inline_edit(win.app(), wid) && !egui_wants_kb && !modal_backdrop_active;
                if !already_selected_grid_cell {
                    match inspect_bridge::resolve_widget_center_in_meta(win.app(), &self.inspect_meta, metrics, wid) {
                        Ok((pos, enabled, reason)) => {
                            if !enabled {
                                let msg = reason.unwrap_or_else(|| format!("widget `{wid}` is disabled"));
                                return Self::send_live_engine_error(deferred.req, parsed.id, curvebook_doc::ErrorCode::Conflict, msg);
                            }
                            click_prefix_batches.extend(inspect_bridge::click_pos_batches(pos, egui::PointerButton::Primary, egui::Modifiers::NONE));
                            click_prefix_batches.push(Vec::new());
                        }
                        Err(err) => return Self::send_live_engine_err_val(deferred.req, parsed.id, err),
                    }
                }
            }
        } else if let Ok(win) = self.runtime.window(win_id)
            && let Some(pos) = inspect_bridge::find_non_grid_textbox_focus_center(win.app(), &self.inspect_meta, metrics, egui_wants_kb)
        {
            click_prefix_batches.extend(inspect_bridge::click_pos_batches(pos, egui::PointerButton::Primary, egui::Modifiers::NONE));
            click_prefix_batches.push(Vec::new());
        }

        let Ok(win) = self.runtime.window_mut(win_id) else {
            return Self::send_live_engine_error(deferred.req, parsed.id, curvebook_doc::ErrorCode::InternalError, "missing active window");
        };
        let (is_x_cell_target, is_y_cell_target) = match inspect_bridge::prepare_grid_type_text_target(
            win.app_mut(),
            widget_id.as_deref(),
            live_focus_wid.as_deref(),
            modal_backdrop_active,
            egui_wants_kb,
        ) {
            Ok(pair) => pair,
            Err(err) => return Self::send_live_engine_err_val(deferred.req, parsed.id, err),
        };
        let clear_first = clear_param.unwrap_or_else(|| widget_id.is_some() && !(is_x_cell_target || is_y_cell_target));
        let inline_open = win.app().egui_views.column_grid.inline_edit.is_some();
        let mut batches = click_prefix_batches;
        batches.extend(inspect_bridge::build_type_text_batches(&text, clear_first, submit, is_x_cell_target, is_y_cell_target, inline_open));
        self.begin_pending_live_action(
            deferred.req,
            parsed.id,
            &parsed.method,
            PendingLiveUiActionKind::TypeText { text, widget_id, submit },
            batches,
        )
    }

    fn try_enqueue_live_press_key(&mut self, ctx: &egui::Context, req: LiveControlRequest, parsed: ControlRequest) -> LiveActionDispatch {
        let key_params = match inspect_bridge::parse_press_key_params(&parsed.params) {
            Ok(p) => p,
            Err(err) => return Self::send_live_engine_err_val(req, parsed.id, err),
        };
        let key_raw = key_params.key_raw.to_owned();
        let mut batches = Vec::new();
        let win_id = self.active_window_id();
        if let Ok(win) = self.runtime.window(win_id)
            && let Some(pos) =
                inspect_bridge::find_non_grid_textbox_focus_center(win.app(), &self.inspect_meta, key_params.metrics, ctx.egui_wants_keyboard_input())
        {
            batches.extend(inspect_bridge::click_pos_batches(pos, egui::PointerButton::Primary, egui::Modifiers::NONE));
            batches.push(Vec::new());
        }
        batches.extend(inspect_bridge::build_press_key_batches(key_params.key, key_params.modifiers, key_params.repeat));
        self.begin_pending_live_action(req, parsed.id, &parsed.method, PendingLiveUiActionKind::PressKey { key_raw }, batches)
    }

    fn try_dispatch_live_switch_tab(
        &mut self,
        deferred: DeferredLiveRequest,
        parsed: ControlRequest,
        post_hook: &mut Option<LiveControlPostHook>,
    ) -> LiveActionDispatch {
        let win_id = self.active_window_id();
        if self.runtime.window(win_id).is_ok_and(|w| w.app().is_human_path_only()) {
            return Self::send_live_engine_err_val(
                deferred.req,
                parsed.id,
                curvebook_engine::EngineError::with_code(
                    curvebook_doc::ErrorCode::Conflict,
                    "direct `ui.switch_tab` is blocked in human-path-only mode",
                )
                .with_details(serde_json::json!({ "code": "human_path_only_policy" })),
            );
        }
        let (normalized_params, numeric_tab_index) = match normalize_live_switch_tab_params(&parsed.params) {
            Ok(pair) => pair,
            Err(err) => return Self::send_live_engine_err_val(deferred.req, parsed.id, err),
        };
        let tab_params = match inspect_bridge::parse_switch_tab_params(&normalized_params) {
            Ok(p) => p,
            Err(err) => return Self::send_live_engine_err_val(deferred.req, parsed.id, err),
        };
        let baseline = self.capture_mutation_baseline();

        if matches!(tab_params.target, inspect_bridge::ParsedSwitchTabTarget::Sheet(_))
            && let Ok(win) = self.runtime.window_mut(win_id)
        {
            inspect_bridge::commit_active_grid_inline_edit(win.app_mut());
        }

        if let inspect_bridge::ParsedSwitchTabTarget::Tab(tab_id) = tab_params.target {
            let pw_res = self.runtime.window_mut(win_id).ok().map(|win| inspect_bridge::try_switch_power_wizard_stage(win.app_mut(), tab_id));
            match pw_res {
                Some(Ok(Some(res_json))) => {
                    self.record_direct_live_mutation("ui.switch_tab", baseline);
                    Self::send_live_control_reply(deferred.req, parsed.id, Ok(res_json));
                    self.invoke_live_post_hook(post_hook, "ui.switch_tab", baseline, true);
                    return LiveActionDispatch::YieldFrame;
                }
                Some(Err(err)) => {
                    self.invoke_live_post_hook(post_hook, "ui.switch_tab", baseline, false);
                    return Self::send_live_engine_err_val(deferred.req, parsed.id, err);
                }
                _ => {}
            }
        }

        if let Some(entry) = inspect_bridge::find_switch_tab_widget_in_meta(&self.inspect_meta, &tab_params.target)
            .or_else(|| numeric_tab_index.and_then(|idx| find_indexed_tab_widget_in_meta(&self.inspect_meta, idx)))
        {
            if !entry.enabled {
                let tab_label = match tab_params.target {
                    inspect_bridge::ParsedSwitchTabTarget::Sheet(s) => s,
                    inspect_bridge::ParsedSwitchTabTarget::Tab(t) => t,
                };
                let msg = entry.disabled_reason.clone().unwrap_or_else(|| format!("tab `{tab_label}` is disabled"));
                return Self::send_live_engine_error(deferred.req, parsed.id, curvebook_doc::ErrorCode::Conflict, msg);
            }
            let pos = entry.clipped_rect.center();
            let wid = entry.widget_id.clone();
            let response_payload = match tab_params.target {
                inspect_bridge::ParsedSwitchTabTarget::Sheet(sheet_id) => {
                    serde_json::json!({ "switched": true, "sheet_id": sheet_id, "widget_id": wid })
                }
                inspect_bridge::ParsedSwitchTabTarget::Tab(tab_id) => {
                    let resolved_tab_id = wid
                        .strip_prefix("result.tab.")
                        .or_else(|| wid.strip_prefix("edit.tab."))
                        .or_else(|| wid.strip_prefix("fit.tab."))
                        .filter(|s| !s.is_empty())
                        .unwrap_or(tab_id);
                    serde_json::json!({ "switched": true, "tab_id": resolved_tab_id, "widget_id": wid })
                }
            };
            let batches = inspect_bridge::click_pos_batches(pos, egui::PointerButton::Primary, egui::Modifiers::NONE);
            return self.begin_pending_live_action(
                deferred.req,
                parsed.id,
                &parsed.method,
                PendingLiveUiActionKind::SwitchTab { response_payload },
                batches,
            );
        }

        let Ok(win) = self.runtime.window_mut(win_id) else {
            return Self::send_live_engine_error(deferred.req, parsed.id, curvebook_doc::ErrorCode::InternalError, "missing active window");
        };
        // Support 0-based numeric tab indices on active `ResultSheet` sub-tabs (`AUD-13-11`).
        if let Some(idx) = numeric_tab_index
            && let Some(rs_id) = win.app().document().active_sheet_id.clone()
            && let Some(tab_id) = win.app().document().result_sheet(&rs_id).and_then(|rs| rs.tabs.get(idx)).map(|t| t.id.clone())
        {
            win.app_mut().canvas_dialogs.result_view.active_tab_index = idx;
            let out = serde_json::json!({
                "switched": true,
                "tab_id": tab_id,
                "tab_index": idx,
                "widget_id": format!("result.tab.{tab_id}"),
            });
            self.record_direct_live_mutation("ui.switch_tab", baseline);
            Self::send_live_control_reply(deferred.req, parsed.id, Ok(out));
            self.invoke_live_post_hook(post_hook, "ui.switch_tab", baseline, true);
            return LiveActionDispatch::YieldFrame;
        }
        match inspect_bridge::execute_switch_tab_fallback(win.app_mut(), &tab_params.target) {
            Ok(out) => {
                self.record_direct_live_mutation("ui.switch_tab", baseline);
                Self::send_live_control_reply(deferred.req, parsed.id, Ok(out));
                self.invoke_live_post_hook(post_hook, "ui.switch_tab", baseline, true);
                LiveActionDispatch::YieldFrame
            }
            Err(err) => {
                self.invoke_live_post_hook(post_hook, "ui.switch_tab", baseline, false);
                Self::send_live_engine_err_val(deferred.req, parsed.id, err)
            }
        }
    }

    fn prepare_settled_live_request_bytes(&self, raw_bytes: &[u8], stamp: GeometryStamp) -> Vec<u8> {
        // Mutate the raw JSON Value in place so per-request `"token"` and all top-level envelope
        // fields are preserved verbatim (`AUD-06-03`).
        let Ok(text) = std::str::from_utf8(raw_bytes) else {
            return raw_bytes.to_vec();
        };
        let Ok(mut val) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
            return raw_bytes.to_vec();
        };
        let Some(obj) = val.as_object_mut() else {
            return raw_bytes.to_vec();
        };
        let method = obj.get("method").or_else(|| obj.get("op")).and_then(serde_json::Value::as_str).map(str::trim);
        if method != Some("ui.screenshot") {
            return raw_bytes.to_vec();
        }
        let scale = f64::from(stamp.scale_factor());
        let mut updated = false;
        let params_entry = obj.entry(String::from("params")).or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let Some(map) = ensure_params_object(params_entry) {
            if !map.contains_key("width_px") {
                map.insert(String::from("width_px"), serde_json::json!(stamp.physical_width_px.clamp(64, 4096)));
                updated = true;
            }
            if !map.contains_key("height_px") {
                map.insert(String::from("height_px"), serde_json::json!(stamp.physical_height_px.clamp(64, 4096)));
                updated = true;
            }
            let has_dpi = map.get("dpi").is_some_and(|v| !v.is_null());
            let has_scale = map.get("scale_factor").is_some_and(|v| !v.is_null());
            if !has_dpi && !has_scale {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let dpi = (scale * 96.0).round().clamp(72.0, 600.0) as u16;
                map.insert(String::from("dpi"), serde_json::json!(dpi));
                updated = true;
            }
        }
        if !updated {
            return raw_bytes.to_vec();
        }
        serde_json::to_vec(&val).map_or_else(
            |_| raw_bytes.to_vec(),
            |mut bytes| {
                bytes.push(b'\n');
                bytes
            },
        )
    }

    fn fail_deferred_live_requests(&mut self, reason: &str) {
        if let Some(pending) = self.pending_live_action.take() {
            let _ = pending.req.reply_tx.send(Err(String::from(reason)));
        }
        for deferred in self.deferred_live_requests.drain(..) {
            let _ = deferred.req.reply_tx.send(Err(String::from(reason)));
        }
    }

    fn drain_pending_control_on_exit(&mut self) {
        self.fail_deferred_live_requests("desktop window is closing");
        let Some(ctrl) = self.live_control.take() else {
            return;
        };
        ctrl.shutdown.store(true, Ordering::Release);
        while let Ok(req) = ctrl.request_rx.try_recv() {
            let _ = req.reply_tx.send(Err(String::from("desktop window is closing")));
        }
    }

    fn commit_pending_intents(&mut self, ctx: &egui::Context, window_id: DesktopWindowId, will_discard: bool) -> bool {
        if self.pending_intents.is_empty() || will_discard {
            // Preserve pass-0 intents across discarded layout passes (`R15-GUI-05`) until settled.
            return false;
        }
        let raw_intents = std::mem::take(&mut self.pending_intents);

        let mut remaining_intents = PendingUiIntents::default();
        let mut host_commands: Vec<(String, serde_json::Value)> = Vec::new();
        let mut close_modals_after_host_commands = false;
        let mut host_copy = false;
        let mut host_copy_for_slides: Option<u16> = None;
        let mut host_cut = false;
        let mut host_paste = false;
        let active_is_result = self.runtime.window(window_id).ok().is_some_and(|w| {
            let doc = w.app().document();
            doc.active_sheet_id.as_ref().is_some_and(|id| doc.result_sheet(id).is_some())
        });
        for intent in raw_intents.intents().iter().cloned() {
            match &intent {
                UiIntent::UiCommand { command_id, params } | UiIntent::EngineCommand { command_id, params }
                    if is_desktop_host_routed_command(command_id) && !should_bypass_native_file_picker(command_id, params) =>
                {
                    host_commands.push((command_id.clone(), params.clone()));
                }
                UiIntent::EngineCommand { command_id, params }
                    if (command_id == "graph.insertImage" || command_id == "layout.insertImage")
                        && params.get("path").is_none()
                        && params.get("bytes_base64").is_none()
                        && params.get("asset_id").is_none() =>
                {
                    host_commands.push((command_id.clone(), params.clone()));
                }
                UiIntent::CloseAllModals if !host_commands.is_empty() => {
                    // Defer modal dismissal until after host-routed commands finish cleanly
                    // (`AUD-10-22` ExportFigure / CsvExportOptions error & cancel preservation).
                    close_modals_after_host_commands = true;
                }
                UiIntent::OpenProjectFile => {
                    host_commands.push((String::from("project.open"), serde_json::json!({})));
                }
                UiIntent::WorkspaceCopy { .. } if !active_is_result => {
                    host_copy = true;
                }
                UiIntent::WorkspaceCopyForSlides { dpi } => {
                    host_copy_for_slides = Some(*dpi);
                }
                UiIntent::GridCopySelection => {
                    host_copy = true;
                }
                UiIntent::GridCutSelection => {
                    host_cut = true;
                }
                UiIntent::GridPasteFromClipboard { .. } => {
                    host_paste = true;
                }
                _ => remaining_intents.push(intent),
            }
        }

        let prev_scale = self.runtime.window(window_id).ok().map(|w| w.app().ui_scale());
        let prev_theme = self.runtime.window(window_id).ok().map(|w| w.app().theme.mode);
        let prev_locale = self.runtime.window(window_id).ok().map(|w| w.app().locale.clone());
        let prev_shortcuts = self.runtime.window(window_id).ok().map(|w| w.app().shortcuts.clone());
        let prev_desc_default = self.runtime.window(window_id).ok().and_then(|w| w.app().saved_descriptive_default().cloned());
        let mut unsaved_action = None;
        if !remaining_intents.is_empty()
            && let Ok(win) = self.runtime.window_mut(window_id)
        {
            let prev_undo = win.app().session().undo_depth();
            let prev_redo = win.app().session().redo_depth();
            let prev_dirty = win.app().session().is_dirty();
            let prev_mutation_seq = win.app().mutation_seq();
            let fallback_command_hint = remaining_intents.intents().iter().find_map(|intent| match intent {
                UiIntent::EngineCommand { command_id, .. } | UiIntent::UiCommand { command_id, .. } => Some(command_id.clone()),
                _ => None,
            });
            let batch = remaining_intents.commit_all_if_settled(win.app_mut(), false);
            if self.pending_live_action.is_some() {
                self.pending_live_outcomes.extend(batch.outcomes().iter().cloned());
                if self.pending_live_error.is_none()
                    && let Some(err) = batch.first_error()
                {
                    self.pending_live_error = Some(err.clone());
                }
            }
            let mut batch_err = batch.first_error().map(ToString::to_string);
            let command_hint = batch.first_succeeded_command_id().map(str::to_owned).or(fallback_command_hint);
            if let Err(err) = win.record_ui_mutation_checkpoint(command_hint.as_deref(), prev_undo, prev_redo, prev_dirty, prev_mutation_seq)
                && batch_err.is_none()
            {
                batch_err = Some(err);
            }
            if let Some(err) = batch_err {
                self.status_banner = Some(err);
            } else {
                let mut export_warnings: Vec<String> = Vec::new();
                let mut pzfx_summary: Option<String> = None;
                for out in batch.outcomes() {
                    if let curvebook_ui_egui::egui_shell::UiIntentOutcome::Engine { command_id, outcome } = out {
                        if command_id == "project.exportPzfx" {
                            let path_opt = outcome.payload.get("path").and_then(serde_json::Value::as_str).map(std::path::Path::new);
                            pzfx_summary = Some(curvebook_ui_egui::format_pzfx_export_status_summary(path_opt, &outcome.payload, &outcome.warnings));
                        } else if curvebook_ui_egui::is_figure_export_command(&command_id) && !outcome.warnings.is_empty() {
                            export_warnings.extend(outcome.warnings);
                        }
                    }
                }
                if let Some(summary) = pzfx_summary {
                    self.status_banner = Some(summary);
                } else if let Some(warn_banner) = curvebook_ui_egui::format_figure_export_warnings_banner(&export_warnings) {
                    self.status_banner = Some(warn_banner);
                }
            }
            unsaved_action = win.app_mut().egui_views.app_modals.pending_unsaved_action.take();
            let changed = Some(win.app().ui_scale()) != prev_scale
                || Some(win.app().theme.mode) != prev_theme
                || Some(win.app().locale.clone()) != prev_locale
                || Some(&win.app().shortcuts) != prev_shortcuts.as_ref()
                || win.app().saved_descriptive_default() != prev_desc_default.as_ref();
            if changed {
                self.runtime.notify_preferences_changed(window_id);
            }
        }
        if let Some(action) = unsaved_action {
            self.apply_egui_unsaved_close_action(ctx, window_id, action);
        }
        if host_copy {
            self.dispatch_clipboard_copy(ctx, window_id);
        }
        if let Some(dpi) = host_copy_for_slides {
            self.dispatch_clipboard_copy_for_slides(window_id, dpi);
        }
        if host_cut {
            self.dispatch_clipboard_cut(ctx, window_id);
        }
        if host_paste && let Err(err) = self.runtime.dispatch(DesktopHostEvent::ClipboardPaste { window_id, text_override: None }) {
            self.status_banner = Some(err);
        }
        let dialog_before_host = if close_modals_after_host_commands {
            self.runtime.window(window_id).ok().map(|w| w.app().canvas_dialogs.active_dialog.clone())
        } else {
            None
        };
        let mut all_host_cmds_completed = true;
        for (cmd_id, params) in host_commands {
            if !self.execute_menu_command_with_params(Some(ctx), window_id, &cmd_id, &params) {
                all_host_cmds_completed = false;
            }
        }
        if close_modals_after_host_commands
            && all_host_cmds_completed
            && let Ok(win) = self.runtime.window_mut(window_id)
            && dialog_before_host.as_ref().is_none_or(|before| win.app().canvas_dialogs.active_dialog == *before)
        {
            let mut close_intents = PendingUiIntents::default();
            close_intents.push(UiIntent::CloseAllModals);
            let _ = close_intents.commit_all_if_settled(win.app_mut(), false);
        }
        true
    }

    fn apply_egui_unsaved_close_action(
        &mut self,
        ctx: &egui::Context,
        window_id: DesktopWindowId,
        action: curvebook_ui_egui::egui_shell::app_modals::UnsavedCloseAction,
    ) {
        use curvebook_ui_egui::egui_shell::app_modals::UnsavedCloseAction;
        match action {
            UnsavedCloseAction::Save => match self.handle_native_save(window_id, false) {
                Ok(()) => {
                    let still_dirty = self.runtime.window(window_id).map(|w| w.has_unsaved_changes()).unwrap_or(false);
                    if still_dirty {
                        self.quit_in_progress = false;
                    } else {
                        match self.runtime.dispatch(DesktopHostEvent::WindowCloseRequested { window_id, force: false }) {
                            Ok(outcome) => self.apply_close_outcome(Some(ctx), &outcome),
                            Err(err) => {
                                self.quit_in_progress = false;
                                self.status_banner = Some(err);
                            }
                        }
                    }
                }
                Err(err) => {
                    self.quit_in_progress = false;
                    self.status_banner = Some(err);
                }
            },
            UnsavedCloseAction::DontSave => match self.runtime.dispatch(DesktopHostEvent::WindowCloseRequested { window_id, force: true }) {
                Ok(outcome) => self.apply_close_outcome(Some(ctx), &outcome),
                Err(err) => {
                    self.quit_in_progress = false;
                    self.status_banner = Some(err);
                }
            },
            UnsavedCloseAction::Cancel => {
                self.close_dialog_open = false;
                self.quit_in_progress = false;
            }
        }
    }

    /// Execute one UI frame (polling live control requests, handling OS events, and rendering).
    pub fn step_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.frame_seq = self.frame_seq.wrapping_add(1);
        self.poll_pending_device_lost();
        let _ = self.poll_live_control(&ctx);
        let initial_window_id = self.active_window_id();

        if let Ok(win) = self.runtime.window(initial_window_id) {
            let tokens = Tokens::from_theme_spec(&win.app().theme, &win.app().locale);
            tokens.apply_to_ctx(&ctx);
            tokens.apply_to_ui(ui);
        }
        ctx.options_mut(|o| o.zoom_with_keyboard = false);

        self.handle_close_request(&ctx, initial_window_id);
        self.menu_popup_was_open = egui::Popup::is_any_open(&ctx);
        let bar_window_id = self.active_window_id();
        #[cfg(target_os = "macos")]
        if self.mac_menu_host.is_installed_for_nsapp() {
            let fp = self.runtime.window(bar_window_id).ok().map(mac_menu_fingerprint);
            if fp.is_some() && fp != self.last_mac_menu_fingerprint {
                self.sync_macos_menu_for_window(bar_window_id);
                self.last_mac_menu_fingerprint = fp;
            }
            while let Some(cmd_id) = self.mac_menu_host.poll_triggered_command_id() {
                self.execute_menu_command(Some(&ctx), bar_window_id, &cmd_id);
            }
        }
        if let Ok(win) = self.runtime.window_mut(bar_window_id) {
            let in_window_menus = win.in_window_menus();
            win.app_mut().host_draws_menu_bar = !in_window_menus;
        }
        self.render_window_switcher_strip(ui);
        if !self.close_dialog_open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Q)) {
            let quit_win_id = self.active_window_id();
            self.execute_menu_command(Some(&ctx), quit_win_id, "app.quit");
        }

        let input_window_id = self.active_window_id();
        let ppp = ctx.pixels_per_point().max(1.0);
        let available_rect = ui.available_rect_before_wrap();
        let active_sheet_id = self.runtime.window(input_window_id).map(|w| w.app().document().active_sheet_id.clone()).unwrap_or(None);

        if active_sheet_id != self.last_active_sheet_id {
            self.last_active_sheet_id = active_sheet_id;
            ctx.request_repaint();
        }

        let frame_had_scroll_input = ctx.input(|i| i.smooth_scroll_delta.length_sq() > 0.01);
        let prev_grid_scroll = self
            .runtime
            .window(input_window_id)
            .ok()
            .map(|w| (w.app().egui_views.column_grid.scroll_offset_pt, w.app().egui_views.center_stage.secondary_column_grid.scroll_offset_pt));
        let mut next_intents = PendingUiIntents::default();
        let mut next_meta = FrameInspectMeta::default();
        self.adopt_figure_export_warning(input_window_id);
        let report = if let Ok(win) = self.runtime.window_mut(input_window_id) {
            let mut views = std::mem::take(&mut win.app_mut().egui_views);
            let rep = inspect_bridge::render_egui_shell_frame_with_host(
                ui,
                win.app(),
                &mut views,
                &mut next_meta,
                &mut next_intents,
                self.close_dialog_open,
            );
            win.app_mut().egui_views = views;
            rep
        } else {
            inspect_bridge::ShellAboutFrameReport::default()
        };
        let post_grid_scroll = self
            .runtime
            .window(input_window_id)
            .ok()
            .map(|w| (w.app().egui_views.column_grid.scroll_offset_pt, w.app().egui_views.center_stage.secondary_column_grid.scroll_offset_pt));
        let scroll_active_this_frame = frame_had_scroll_input || (prev_grid_scroll.is_some() && post_grid_scroll != prev_grid_scroll);
        self.inspect_meta = next_meta;
        self.last_regions = report.regions;
        self.pending_intents.merge_pass(next_intents);
        self.menu_popup_was_open |= report.menu_popup_open;

        let canvas_rect = report.regions.canvas_rect_pt.unwrap_or(available_rect);
        self.sync_canvas_viewport(input_window_id, canvas_rect, ppp);
        if self.process_canvas_input_inner(ui, input_window_id) {
            ctx.request_repaint();
        }
        self.render_dnd_hover_overlay(ui);
        self.render_status_banner_toast(&ctx);

        let post_window_id = self.active_window_id();
        let will_discard = ctx.will_discard();
        let force_commit = will_discard && self.discarded_carry_passes >= 7;
        if will_discard && !force_commit {
            self.discarded_carry_passes = self.discarded_carry_passes.saturating_add(1);
            let _ = self.commit_pending_intents(&ctx, post_window_id, true);
            ctx.request_repaint();
        } else {
            self.discarded_carry_passes = 0;
            if self.commit_pending_intents(&ctx, post_window_id, false) {
                ctx.request_repaint();
            }
            if will_discard {
                ctx.request_repaint();
            }
        }
        let content_rect = ctx.content_rect();
        let ppp = ctx.pixels_per_point().clamp(0.5, 8.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let phys_w = (content_rect.width() * ppp).round().max(1.0) as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let phys_h = (content_rect.height() * ppp).round().max(1.0) as u32;
        self.last_geometry_stamp = Some(GeometryStamp::from_context(self.frame_seq, &ctx, [phys_w, phys_h], !will_discard));
        let now_wants_kb = ctx.egui_wants_keyboard_input();
        if self.pending_live_action.is_some() {
            self.pending_live_ever_wanted_kb |= now_wants_kb;
        }
        if !will_discard {
            let had_pending = self.pending_live_action.is_some();
            self.flush_deferred_live_requests(&ctx, scroll_active_this_frame);
            if !had_pending && let Ok(win) = self.runtime.window_mut(post_window_id) {
                inspect_bridge::clear_stale_non_grid_textbox_focus(win.app_mut(), &self.inspect_meta, self.last_egui_wants_kb, now_wants_kb);
            }
            self.last_egui_wants_kb = now_wants_kb;
        }
        // Sync native window title AFTER post-render intent commits and deferred live-control
        // actions so title/dirty-marker updates take effect on the same frame (`AUD-06-06`).
        let final_window_id = self.active_window_id();
        self.sync_window_title(&ctx, final_window_id);
        if !self.pending_synthetic_egui_batches.is_empty() || self.pending_live_action.is_some() {
            ctx.request_repaint();
        }
    }

    /// Run shutdown cleanup, signal any attached live-control server to stop, and close all windows.
    pub fn finish_exit(&mut self) {
        self.drain_pending_control_on_exit();
        if let Some(cleanup) = self.on_exit_cleanup.take()
            && let Err(err) = cleanup(&mut self.runtime)
        {
            eprintln!("curvebook shutdown cleanup warning: {err}");
        }
        let _ = self.runtime.close_all_force();
    }

    fn active_window_id(&self) -> DesktopWindowId {
        self.runtime.focused_window_id().unwrap_or(self.primary_window_id)
    }

    fn sync_window_title(&mut self, ctx: &egui::Context, window_id: DesktopWindowId) {
        let Ok(win) = self.runtime.window(window_id) else {
            return;
        };
        let title = win.window_title();
        if title != self.last_title {
            self.last_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }

    fn on_active_window_switched(&mut self) {
        self.last_frame_size = [0, 0];
        self.last_scale_factor = 0.0;
        self.last_title.clear();
        self.hovered_paths_active = false;
        self.close_dialog_open = false;
        #[cfg(target_os = "macos")]
        {
            self.last_mac_menu_fingerprint = None;
        }
    }

    /// Returns `true` when the multi-window switcher strip is currently displayed (`window_count() > 1`).
    #[must_use]
    pub fn is_window_switcher_visible(&self) -> bool {
        self.runtime.window_count() > 1
    }

    /// Returns `(switch_button_rect, close_button_rect)` in logical points for `window_id` from the last rendered frame.
    #[must_use]
    pub fn window_switcher_button_rects(&self, window_id: DesktopWindowId) -> Option<(egui::Rect, egui::Rect)> {
        self.window_switcher_rects.get(&window_id).copied()
    }

    fn window_switcher_labels(win: &crate::desktop_host::DesktopWindowSession, win_id: DesktopWindowId) -> (String, String) {
        let base_name = win
            .app()
            .session()
            .file_path()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map_or_else(|| format!("Untitled {}", win_id.0), ToOwned::to_owned);
        let dirty_mark = if win.has_unsaved_changes() { " *" } else { "" };
        let recovered_mark = if win.recovered_from_crash() { " [Recovered]" } else { "" };
        (format!("{base_name}{dirty_mark}{recovered_mark}"), base_name)
    }

    fn apply_window_switcher_action(&mut self, ctx: &egui::Context, switch_target: Option<DesktopWindowId>, close_target: Option<DesktopWindowId>) {
        if let Some(target_id) = switch_target {
            if let Ok(changed) = self.runtime.focus_window(target_id)
                && changed
            {
                self.close_dialog_open = false;
                self.quit_in_progress = false;
                self.on_active_window_switched();
            }
            ctx.request_repaint();
        }
        if let Some(target_id) = close_target {
            self.quit_in_progress = false;
            match self.runtime.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: target_id, force: false }) {
                Ok(outcome) => self.apply_close_outcome(Some(ctx), &outcome),
                Err(err) => self.status_banner = Some(err),
            }
            ctx.request_repaint();
        }
    }

    fn render_window_switcher_strip(&mut self, ui: &mut egui::Ui) {
        self.window_switcher_rects.clear();
        if self.runtime.window_count() <= 1 {
            return;
        }
        let ids = self.runtime.window_ids();
        let focused_id = self.runtime.focused_window_id();
        let mut switch_target: Option<DesktopWindowId> = None;
        let mut close_target: Option<DesktopWindowId> = None;

        egui::Panel::top("desktop.window_switcher").exact_size(26.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(egui::RichText::new(format!("Windows ({}):", ids.len())).small().strong());
                for win_id in ids {
                    let Ok(win) = self.runtime.window(win_id) else {
                        continue;
                    };
                    let (label, base_name) = Self::window_switcher_labels(win, win_id);
                    let is_focused = focused_id == Some(win_id);

                    let switch_btn = egui::Button::new(egui::RichText::new(&label).small()).selected(is_focused);
                    let switch_resp = ui.add(switch_btn);
                    if switch_resp.clicked() {
                        switch_target = Some(win_id);
                    }
                    let close_resp = ui.small_button("×").on_hover_text(format!("Close {base_name}"));
                    if close_resp.clicked() {
                        close_target = Some(win_id);
                    }
                    self.window_switcher_rects.insert(win_id, (switch_resp.rect, close_resp.rect));
                }
            });
        });

        self.apply_window_switcher_action(ui.ctx(), switch_target, close_target);
    }

    fn focus_and_prompt_unsaved(&mut self, ctx: Option<&egui::Context>, window_id: DesktopWindowId) {
        if self.runtime.focused_window_id() != Some(window_id) && self.runtime.focus_window(window_id).unwrap_or(false) {
            self.on_active_window_switched();
        }
        if let Some(ctx) = ctx {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        self.close_dialog_open = true;
    }

    fn apply_close_outcome(&mut self, ctx: Option<&egui::Context>, outcome: &DesktopEventOutcome) {
        if let Some(CloseDisposition::PromptUnsaved { window_id, .. }) = outcome.close_disposition {
            self.focus_and_prompt_unsaved(ctx, window_id);
            return;
        }
        if !matches!(outcome.close_disposition, Some(CloseDisposition::Closed)) {
            return;
        }
        self.close_dialog_open = false;
        if self.runtime.window_count() > 0 && self.quit_in_progress {
            match self.runtime.request_app_quit(None, false) {
                Ok(next_outcome) => {
                    if let Some(CloseDisposition::PromptUnsaved { window_id, .. }) = next_outcome.close_disposition {
                        self.focus_and_prompt_unsaved(ctx, window_id);
                        return;
                    }
                }
                Err(err) => {
                    self.quit_in_progress = false;
                    self.status_banner = Some(err);
                }
            }
        }
        if self.runtime.window_count() > 0 {
            if let Some(ctx) = ctx {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
            self.close_confirmed = false;
            self.on_active_window_switched();
        } else {
            self.quit_in_progress = false;
            self.close_confirmed = true;
            if let Some(ctx) = ctx {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context, window_id: DesktopWindowId) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.close_confirmed {
            return;
        }
        self.quit_in_progress = false;
        match self.runtime.dispatch(DesktopHostEvent::WindowCloseRequested { window_id, force: false }) {
            Ok(outcome) => {
                self.apply_close_outcome(Some(ctx), &outcome);
            }
            Err(err) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.status_banner = Some(err);
                self.close_dialog_open = true;
            }
        }
    }

    /// Dispatch a desktop host or UI menu command for the currently active window inside a live frame.
    pub fn trigger_menu_action(&mut self, ctx: &egui::Context, command_id: &str) {
        let win_id = self.active_window_id();
        self.execute_menu_command(Some(ctx), win_id, command_id);
    }

    fn execute_menu_command(&mut self, ctx: Option<&egui::Context>, window_id: DesktopWindowId, command_id: &str) {
        let _ = self.execute_menu_command_with_params(ctx, window_id, command_id, &serde_json::json!({}));
    }

    fn execute_menu_command_with_params(
        &mut self,
        ctx: Option<&egui::Context>,
        window_id: DesktopWindowId,
        command_id: &str,
        params: &serde_json::Value,
    ) -> bool {
        self.status_banner = None;
        if command_id == "app.preferences" {
            if let Ok(win) = self.runtime.window_mut(window_id) {
                let shortcuts = win.app().shortcuts.clone();
                win.app_mut().egui_views.app_modals.open_preferences(&shortcuts, None);
            }
            return true;
        }
        if let Some(native_result) = self.try_handle_native_file_command_with_params(window_id, command_id, params) {
            match native_result {
                Ok(completed) => {
                    if self.active_window_id() != window_id {
                        self.on_active_window_switched();
                    }
                    return completed;
                }
                Err(err) => {
                    self.status_banner = Some(err);
                    return false;
                }
            }
        }
        if matches!(command_id, "app.quit" | "file.quit") {
            self.quit_in_progress = true;
        } else if matches!(command_id, "window.close" | "file.close") {
            self.quit_in_progress = false;
        }
        match self.runtime.dispatch(DesktopHostEvent::MenuAction { window_id, command_id: command_id.to_string() }) {
            Ok(outcome) => {
                if outcome.close_disposition.is_some() {
                    self.apply_close_outcome(ctx, &outcome);
                } else if outcome.opened_window_id.is_some() {
                    self.quit_in_progress = false;
                    self.on_active_window_switched();
                }
                if outcome.command_id.as_deref().is_some_and(|c| matches!(c, "edit.copy" | "edit.cut" | "ui.clipboard.copyForSlides")) {
                    self.apply_clipboard_outcome_status(&outcome);
                    if outcome.command_id.as_deref() != Some("ui.clipboard.copyForSlides")
                        && let Some(ctx_ref) = ctx
                    {
                        self.maybe_sync_plain_text_fallback(ctx_ref, &outcome);
                    }
                }
                true
            }
            Err(err) => {
                self.quit_in_progress = false;
                self.status_banner = Some(err);
                false
            }
        }
    }

    fn try_handle_native_file_command_with_params(
        &mut self,
        window_id: DesktopWindowId,
        command_id: &str,
        params: &serde_json::Value,
    ) -> Option<Result<bool, String>> {
        if !menu_action_triggers_native_file_dialog(command_id) {
            return None;
        }
        let outcome = match self.runtime.dispatch_native_file_command_with_params(window_id, command_id, params) {
            Ok(Some(outcome)) => outcome,
            Ok(None) => return None,
            Err(err) => return Some(Err(err)),
        };
        if outcome.opened_window_id.is_some() {
            self.on_active_window_switched();
        }
        if let Some(summary) = outcome.command_result.as_ref().and_then(|res| res.get("status_summary")).and_then(serde_json::Value::as_str) {
            self.status_banner = Some(summary.to_owned());
        } else if let Some(warnings) = outcome.command_result.as_ref().and_then(|res| res.get("warnings")).and_then(serde_json::Value::as_array) {
            let warning_lines: Vec<String> = warnings.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
            if let Some(warn_banner) = curvebook_ui_egui::format_figure_export_warnings_banner(&warning_lines) {
                self.status_banner = Some(warn_banner);
            }
        }
        let completed = !outcome.action.starts_with("native_file_dialog.cancelled:");
        Some(Ok(completed))
    }

    fn handle_native_save(&mut self, window_id: DesktopWindowId, force_dialog: bool) -> Result<(), String> {
        let cmd = if force_dialog { "file.save_as" } else { "project.save" };
        let _ = self.runtime.dispatch_native_file_command(window_id, cmd)?;
        Ok(())
    }

    fn sync_canvas_viewport(&mut self, window_id: DesktopWindowId, canvas_rect: egui::Rect, ppp: f32) {
        let clamped_ppp = ppp.max(1.0);
        if (clamped_ppp - self.last_scale_factor).abs() > 0.001 {
            self.last_scale_factor = clamped_ppp;
            let _ = self.runtime.dispatch(DesktopHostEvent::ScaleFactorChanged { window_id, scale_factor: clamped_ppp });
        }

        let width_px = ((canvas_rect.width() * clamped_ppp).round().max(1.0)) as u32;
        let height_px = ((canvas_rect.height() * clamped_ppp).round().max(1.0)) as u32;
        if [width_px, height_px] != self.last_frame_size {
            self.last_frame_size = [width_px, height_px];
            let _ = self.runtime.dispatch(DesktopHostEvent::Resized { window_id, width_px, height_px });
        }
    }

    /// Process host-level `egui` file drag-and-drop and global desktop shortcut events.
    pub fn process_canvas_input(&mut self, ui: &egui::Ui, window_id: DesktopWindowId) {
        let _ = self.process_canvas_input_inner(ui, window_id);
    }

    fn process_canvas_input_inner(&mut self, ui: &egui::Ui, window_id: DesktopWindowId) -> bool {
        let (events, raw_hovered, raw_dropped) = ui.ctx().input(|i| (i.events.clone(), i.raw.hovered_files.clone(), i.raw.dropped_files.clone()));

        let dnd_acted = self.dispatch_file_dnd(window_id, &raw_hovered, &raw_dropped);
        let key_acted = self.dispatch_egui_events(ui.ctx(), window_id, &events);
        dnd_acted || key_acted
    }

    fn window_has_blocking_modal(&self, window_id: DesktopWindowId) -> bool {
        self.close_dialog_open || self.runtime.window(window_id).is_ok_and(|w| w.app().has_open_modal())
    }

    fn dispatch_file_dnd(&mut self, window_id: DesktopWindowId, hovered: &[egui::HoveredFile], dropped: &[egui::DroppedFileHandle]) -> bool {
        let mut acted = false;
        let modal_open_before = self.window_has_blocking_modal(window_id);
        let hovered_paths: Vec<PathBuf> = hovered.iter().filter_map(|f| f.path.clone()).collect();
        if !hovered.is_empty() && !modal_open_before {
            self.hovered_paths_active = true;
            let _ = self.runtime.dispatch(DesktopHostEvent::FileHovered { window_id, paths: hovered_paths });
            acted = true;
        } else if self.hovered_paths_active {
            self.hovered_paths_active = false;
            let _ = self.runtime.dispatch(DesktopHostEvent::FileHoverCancelled { window_id });
            acted = true;
        }

        if dropped.is_empty() {
            return acted;
        }
        self.hovered_paths_active = false;
        let _ = self.runtime.dispatch(DesktopHostEvent::FileHoverCancelled { window_id });
        if modal_open_before {
            self.status_banner = Some(String::from("Cannot drop files while a dialog is open. Close or finish the current dialog first."));
            return true;
        }

        self.status_banner = None;
        acted = true;
        // Track the active drop target across a multi-file batch so dropping a project + CSV
        // routes follow-up files into the newly opened project window, join all errors rather than
        // overwriting earlier failures (`AUD-06-13`), and stop immediately if an earlier drop opens
        // a modal prompt so subsequent files are never imported behind an active modal (`GAP-07-04`).
        let mut target_window_id = window_id;
        let mut drop_errors: Vec<String> = Vec::new();
        for (idx, file) in dropped.iter().enumerate() {
            if idx > 0 && self.window_has_blocking_modal(target_window_id) {
                let skipped: Vec<String> = dropped
                    .get(idx..)
                    .unwrap_or_default()
                    .iter()
                    .map(|f| f.path().file_name().and_then(|n| n.to_str()).map_or_else(|| f.path().display().to_string(), str::to_owned))
                    .collect();
                drop_errors.push(format!(
                    "Skipped {} while an import dialog is open; finish the dialog before dropping additional files.",
                    skipped.join(", ")
                ));
                break;
            }
            match self.runtime.dispatch(DesktopHostEvent::FileDropped { window_id: target_window_id, path: file.path().to_path_buf() }) {
                Ok(outcome) => {
                    if let Some(opened_id) = outcome.opened_window_id {
                        target_window_id = opened_id;
                        self.on_active_window_switched();
                    }
                }
                Err(err) => {
                    drop_errors.push(err);
                }
            }
        }
        if !drop_errors.is_empty() {
            self.status_banner = Some(drop_errors.join("; "));
        }
        acted
    }

    fn render_dnd_hover_overlay(&self, ui: &egui::Ui) {
        if !self.hovered_paths_active || self.window_has_blocking_modal(self.active_window_id()) {
            return;
        }
        let ctx = ui.ctx();
        let rect = ctx.content_rect();
        let hovered_names: Vec<String> = self
            .runtime
            .window(self.active_window_id())
            .map(|w| w.hovered_files().iter().filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned)).collect())
            .unwrap_or_default();
        let detail = if hovered_names.is_empty() {
            String::from("Drop .curvebook, .cbtemplate, .pzfx, .prism, .xlsx, .csv, or .tsv files to open or import")
        } else {
            format!("Drop to open or import: {}", hovered_names.join(", "))
        };
        egui::Area::new(egui::Id::unique("desktop.dnd_hover_overlay")).order(egui::Order::Foreground).interactable(false).fixed_pos(rect.min).show(
            ctx,
            |area_ui| {
                area_ui.set_min_size(rect.size());
                let painter = area_ui.painter();
                painter.rect_filled(rect, 0.0, egui::Color32::from_rgba_unmultiplied(15, 23, 42, 110));
                let card_rect = rect.shrink2(egui::vec2((rect.width() * 0.15).min(80.0), (rect.height() * 0.25).min(100.0)));
                painter.rect_filled(card_rect, 12.0, egui::Color32::from_rgba_unmultiplied(248, 250, 252, 240));
                painter.rect_stroke(card_rect, 12.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(37, 99, 235)), egui::StrokeKind::Inside);
                painter.text(
                    card_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    detail,
                    egui::FontId::proportional(15.0),
                    egui::Color32::from_rgb(15, 23, 42),
                );
            },
        );
    }

    /// Move pending figure-export warnings and non-Welcome-hub shell status notices from the
    /// window's shared shell state into the desktop toast before the shell draws, so each notice
    /// is shown once rather than duplicated across both the toast and the status footer (`R0.36`).
    fn adopt_figure_export_warning(&mut self, window_id: DesktopWindowId) {
        if let Ok(win) = self.runtime.window_mut(window_id) {
            if let Some(warning) = win.app_mut().canvas_dialogs.figure_export_warning.take() {
                self.status_banner = Some(warning);
            }
            if !win.app().egui_views.launcher_open
                && win.app().document().total_sheet_count() > 0
                && let Some(banner) = win.app_mut().canvas_dialogs.status_banner.take()
            {
                self.status_banner = Some(banner);
            }
        }
    }

    fn render_status_banner_toast(&mut self, ctx: &egui::Context) {
        let Some(msg) = self.status_banner.clone() else {
            return;
        };
        let rect = ctx.content_rect();
        let mut dismiss = false;
        let is_warning = msg.starts_with(curvebook_ui_egui::FIGURE_EXPORT_WARNING_PREFIX);
        let (fill_color, stroke_color) = if is_warning {
            (egui::Color32::from_rgb(146, 64, 14), egui::Color32::from_rgb(251, 191, 36))
        } else {
            (egui::Color32::from_rgb(127, 29, 29), egui::Color32::from_rgb(248, 113, 113))
        };
        egui::Area::new(egui::Id::unique("desktop.status_banner"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -14.0))
            .show(ctx, |ui| {
                egui::Frame::window(ui.style())
                    .fill(fill_color)
                    .stroke(egui::Stroke::new(1.0, stroke_color))
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .show(ui, |row| {
                        row.set_max_width((rect.width() - 48.0).max(220.0));
                        row.horizontal(|h| {
                            h.label(egui::RichText::new(&msg).color(egui::Color32::WHITE).small());
                            if h.small_button("×").clicked() {
                                dismiss = true;
                            }
                        });
                    });
            });
        if dismiss {
            self.status_banner = None;
        }
    }

    fn dispatch_egui_events(&mut self, ctx: &egui::Context, window_id: DesktopWindowId, events: &[egui::Event]) -> bool {
        let popup_open = self.menu_popup_was_open || egui::Popup::is_any_open(ctx);
        let mut acted = false;

        for event in events {
            if let egui::Event::Key { key, pressed, modifiers, .. } = event {
                let is_menu_nav_key = matches!(
                    key,
                    egui::Key::ArrowLeft
                        | egui::Key::ArrowRight
                        | egui::Key::ArrowUp
                        | egui::Key::ArrowDown
                        | egui::Key::Enter
                        | egui::Key::Space
                        | egui::Key::Escape
                        | egui::Key::Tab
                );
                if popup_open && is_menu_nav_key {
                    continue;
                }
                acted |= self.dispatch_key_event(ctx, window_id, *key, *pressed, *modifiers);
            }
        }
        acted
    }

    fn apply_clipboard_outcome_status(&mut self, outcome: &DesktopEventOutcome) {
        let os_status = outcome.command_result.as_ref().and_then(|v| v.get("os_clipboard"));
        if let Some(err) = os_status.and_then(|s| s.get("error")).and_then(serde_json::Value::as_str) {
            self.status_banner = Some(format!("OS clipboard error: {err}"));
        } else if os_status.and_then(|s| s.get("available")).and_then(serde_json::Value::as_bool) == Some(false)
            && let Some(detail) = os_status.and_then(|s| s.get("detail")).and_then(serde_json::Value::as_str)
        {
            self.status_banner = Some(format!("OS clipboard unavailable: {detail}"));
        }
    }

    fn maybe_sync_plain_text_fallback(&self, ctx: &egui::Context, outcome: &DesktopEventOutcome) {
        let win_id = outcome.window_id.unwrap_or_else(|| self.active_window_id());
        if self.runtime.config().human_path_only || self.runtime.window(win_id).is_ok_and(|w| w.app().is_human_path_only()) {
            return;
        }
        let os_available =
            outcome.command_result.as_ref().and_then(|v| v.get("os_clipboard")).and_then(|s| s.get("available")).and_then(serde_json::Value::as_bool)
                == Some(true);
        // Never overwrite multi-flavor ClipboardBundle flavors on the OS pasteboard with `ctx.copy_text`,
        // and avoid redundant `ctx.copy_text` clobbering whenever OS write succeeded cleanly.
        if !os_available
            && self.runtime.clipboard_bundle().is_none()
            && let Some(copied) = self.runtime.clipboard_text()
        {
            ctx.copy_text(copied.to_owned());
        }
    }

    fn dispatch_clipboard_copy(&mut self, ctx: &egui::Context, window_id: DesktopWindowId) {
        match self.runtime.dispatch(DesktopHostEvent::ClipboardCopy { window_id }) {
            Ok(outcome) => {
                self.apply_clipboard_outcome_status(&outcome);
                self.maybe_sync_plain_text_fallback(ctx, &outcome);
            }
            Err(err) => {
                self.status_banner = Some(err);
            }
        }
    }

    fn dispatch_clipboard_copy_for_slides(&mut self, window_id: DesktopWindowId, dpi: u16) {
        match self.runtime.handle_clipboard_copy_for_slides(window_id, dpi) {
            Ok(outcome) => {
                self.apply_clipboard_outcome_status(&outcome);
            }
            Err(err) => {
                self.status_banner = Some(err);
            }
        }
    }

    fn dispatch_clipboard_cut(&mut self, ctx: &egui::Context, window_id: DesktopWindowId) {
        match self.runtime.cut_selection(window_id) {
            Ok(outcome) => {
                self.apply_clipboard_outcome_status(&outcome);
                self.maybe_sync_plain_text_fallback(ctx, &outcome);
            }
            Err(err) => {
                self.status_banner = Some(err);
            }
        }
    }

    fn dispatch_key_event(
        &mut self,
        ctx: &egui::Context,
        window_id: DesktopWindowId,
        key: egui::Key,
        pressed: bool,
        modifiers: egui::Modifiers,
    ) -> bool {
        if !pressed || ctx.text_edit_focused() {
            return false;
        }
        let Some(key_name) = egui_key_to_name(key) else {
            return false;
        };
        let (dialog_inactive, inline_active, shortcut_cmd, should_fit_sheet_on_cmd_zero) =
            self.runtime.window(window_id).ok().map_or((false, false, None, false), |w| {
                let app = w.app();
                let dialog_ok = !self.close_dialog_open
                    && app.canvas_dialogs.active_dialog == ActiveDialog::None
                    && !app.nav.welcome.open
                    && !app.grid.import_wizard.open
                    && !app.grid.variables_dialog.open;
                if !dialog_ok {
                    return (false, app.inline_editor.active, None, false);
                }
                let cmd = match_routed_desktop_shortcut(app, &key_name, to_key_modifiers(modifiers));
                let fit_sheet = if cmd.as_deref() == Some("ui.zoom.fitSheet")
                    && (modifiers.command || modifiers.mac_cmd || modifiers.ctrl)
                    && !modifiers.shift
                    && !modifiers.alt
                    && key == egui::Key::Num0
                {
                    let doc = app.document();
                    let canvas_moved = (app.canvas_dialogs.canvas.zoom - 1.0).abs() > 0.001
                        || app.canvas_dialogs.canvas.pan_pt.x.abs() > 0.001
                        || app.canvas_dialogs.canvas.pan_pt.y.abs() > 0.001;
                    let ui_zoomed = (app.ui_scale() - 1.0).abs() > 0.001;
                    doc.total_sheet_count() > 0 && (canvas_moved || !ui_zoomed)
                } else {
                    true
                };
                (true, app.inline_editor.active, cmd, fit_sheet)
            });
        if !dialog_inactive || inline_active {
            return false;
        }

        let primary_cmd = modifiers.command || modifiers.mac_cmd || modifiers.ctrl;
        let is_cmd_zero_reset = primary_cmd && !modifiers.alt && !modifiers.shift && key == egui::Key::Num0;

        // 1. Consult customizable `app.shortcuts` BEFORE hardcoded fallbacks (supporting F-keys and Alt bindings too).
        if let Some(cmd) = shortcut_cmd {
            if cmd == "ui.zoom.fitSheet" && is_cmd_zero_reset && !should_fit_sheet_on_cmd_zero {
                self.apply_ui_scale_adjustment(ctx, window_id, UiScaleAdjust::Reset);
                return true;
            }
            let already_queued_edit_params = cmd == "ui.editParams.open"
                && self.pending_intents.intents().iter().any(|intent| {
                    matches!(intent, curvebook_ui_egui::egui_shell::UiIntent::UiCommand { command_id, .. } if command_id == "ui.editParams.open")
                });
            if !already_queued_edit_params
                && (menu_action_triggers_native_file_dialog(&cmd)
                    || matches!(
                        cmd.as_str(),
                        "window.close"
                            | "file.close"
                            | "app.quit"
                            | "file.quit"
                            | "window.new"
                            | "file.newWindow"
                            | "app.preferences"
                            | "ui.editParams.open"
                            | "ui.analyze.open"
                            | "ui.print.open"
                            | "ui.changeGraphType.open"
                            | "ui.zoom.fitSheet"
                    ))
            {
                self.execute_menu_command(Some(ctx), window_id, &cmd);
                return true;
            }
            if cmd != "ui.editParams.open" {
                self.execute_menu_command(Some(ctx), window_id, &cmd);
                return true;
            }
        }

        // 2. Hardcoded fallbacks when `app.shortcuts` did not claim this combination.
        if primary_cmd && !modifiers.alt {
            if key == egui::Key::Plus || key == egui::Key::Equals {
                self.apply_ui_scale_adjustment(ctx, window_id, UiScaleAdjust::In);
                return true;
            }
            if key == egui::Key::Minus {
                self.apply_ui_scale_adjustment(ctx, window_id, UiScaleAdjust::Out);
                return true;
            }
            if key == egui::Key::Num0 && !modifiers.shift {
                self.apply_ui_scale_adjustment(ctx, window_id, UiScaleAdjust::Reset);
                return true;
            }
            if key == egui::Key::O && !modifiers.shift {
                self.execute_menu_command(Some(ctx), window_id, "project.open");
                return true;
            }
            if key == egui::Key::S {
                let cmd = if modifiers.shift { "project.saveAs" } else { "project.save" };
                self.execute_menu_command(Some(ctx), window_id, cmd);
                return true;
            }
        }
        false
    }
}

impl Drop for CurveBookNativeApp {
    fn drop(&mut self) {
        self.drain_pending_control_on_exit();
    }
}

impl eframe::App for CurveBookNativeApp {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.drain_synthetic_events_into(raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.step_ui(ui);
    }

    fn on_exit(&mut self) {
        self.finish_exit();
    }
}

enum LiveActionDispatch {
    ImmediateReply,
    EguiQueued,
    YieldFrame,
    NeedsScrollRetry(DeferredLiveRequest),
}

fn parse_control_request_from_bytes(raw_bytes: &[u8]) -> Option<ControlRequest> {
    let text = std::str::from_utf8(raw_bytes).ok()?;
    parse_control_request(text.trim()).ok()
}

fn is_request_authorized(req: &LiveControlRequest) -> bool {
    if req.conn_authenticated {
        return true;
    }
    let Ok(text) = std::str::from_utf8(&req.raw_bytes) else {
        return false;
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return false;
    };
    let Some(token) = val.get("token").and_then(serde_json::Value::as_str) else {
        return false;
    };
    req.auth.verify(token)
}

#[derive(Debug, Clone, Copy)]
enum UiScaleAdjust {
    In,
    Out,
    Reset,
}

impl CurveBookNativeApp {
    fn apply_ui_scale_adjustment(&mut self, ctx: &egui::Context, window_id: DesktopWindowId, adjust: UiScaleAdjust) {
        let mut applied_scale = None;
        let changed = if let Ok(win) = self.runtime.window_mut(window_id) {
            let prev = win.app().ui_scale();
            let next = match adjust {
                UiScaleAdjust::In => win.app_mut().zoom_ui_scale_in(),
                UiScaleAdjust::Out => win.app_mut().zoom_ui_scale_out(),
                UiScaleAdjust::Reset => win.app_mut().reset_ui_scale(),
            };
            applied_scale = Some(next);
            (next - prev).abs() > 0.001
        } else {
            false
        };
        if let Some(next_scale) = applied_scale {
            ctx.set_zoom_factor(next_scale);
        }
        if changed {
            self.runtime.notify_preferences_changed(window_id);
        }
        ctx.request_repaint();
    }
}

fn should_defer_to_settled_frame(req: &LiveControlRequest) -> bool {
    if !is_request_authorized(req) || classify_control_frame(&req.raw_bytes) != ControlFrameKind::Command {
        return false;
    }
    let Some(parsed) = parse_control_request_from_bytes(&req.raw_bytes) else {
        return false;
    };
    matches!(parsed.method.as_str(), "ui.inspect" | "ui.screenshot" | "ui.pointer_gesture" | "ui.type_text" | "ui.press_key" | "ui.switch_tab")
}

fn serialized_control_reply_len(id: &serde_json::Value, payload: &serde_json::Value) -> Result<usize, curvebook_engine::EngineError> {
    let reply = curvebook_ui_egui::encode_egui_control_response(id.clone(), Ok(payload.clone()));
    let bytes = serde_json::to_vec(&reply).map_err(|err| {
        curvebook_engine::EngineError::with_code(curvebook_doc::ErrorCode::InternalError, format!("serialize control reply: {err}"))
    })?;
    Ok(bytes.len().saturating_add(1))
}

#[allow(clippy::too_many_arguments)]
fn finalize_live_screenshot_payload_with_limit(
    request_id: &serde_json::Value,
    region_raw: &str,
    width_px: u32,
    height_px: u32,
    dpi: u16,
    png: &[u8],
    validated_path: Option<&str>,
    max_line_bytes: usize,
) -> Result<serde_json::Value, curvebook_engine::EngineError> {
    use curvebook_doc::ErrorCode;
    use curvebook_engine::EngineError;

    // Emit a SINGLE `data_base64` field (never duplicate as `png_base64`) and verify serialized
    // control-frame size against `max_line_bytes` BEFORE writing `validated_path` to disk (`AUD-13-12`).
    let mut base_payload = serde_json::json!({
        "region": region_raw,
        "width_px": width_px,
        "height_px": height_px,
        "dpi": dpi,
        "format": "png",
        "mime_type": "image/png",
        "bytes_len": png.len(),
        "path": validated_path,
    });

    let b64 = curvebook_platform::base64::encode(png);
    let mut with_inline = base_payload.clone();
    if let Some(map) = with_inline.as_object_mut() {
        map.insert(String::from("data_base64"), serde_json::Value::String(b64));
    }

    let inline_frame_len = serialized_control_reply_len(request_id, &with_inline)?;
    if inline_frame_len <= max_line_bytes {
        return Ok(with_inline);
    }

    if validated_path.is_some() {
        if let Some(map) = base_payload.as_object_mut() {
            map.insert(String::from("inline_omitted"), serde_json::Value::Bool(true));
        }
        let meta_len = serialized_control_reply_len(request_id, &base_payload)?;
        if meta_len > max_line_bytes {
            return Err(EngineError::with_code(
                ErrorCode::ResourceLimit,
                format!("screenshot response metadata ({meta_len} bytes) exceeds control frame limit ({max_line_bytes} bytes)"),
            ));
        }
        Ok(base_payload)
    } else {
        Err(EngineError::with_code(
            ErrorCode::ResourceLimit,
            format!(
                "screenshot frame ({inline_frame_len} bytes) exceeds control line limit ({max_line_bytes} bytes); supply `path` or reduce dimensions"
            ),
        ))
    }
}

fn normalize_live_switch_tab_params(params: &serde_json::Value) -> Result<(serde_json::Value, Option<usize>), curvebook_engine::EngineError> {
    let mut cloned = params.clone();
    let mut numeric_index = None;
    if let Some(obj) = cloned.as_object_mut() {
        for key in ["tab", "tab_id"] {
            if let Some(val) = obj.get_mut(key) {
                if let Some(num) = val.as_number() {
                    let Some(idx_u64) = num.as_u64() else {
                        return Err(curvebook_engine::EngineError::with_code(
                            curvebook_doc::ErrorCode::BadInput,
                            format!("`{key}` index must be a non-negative integer"),
                        ));
                    };
                    let idx = usize::try_from(idx_u64).map_err(|_| {
                        curvebook_engine::EngineError::with_code(curvebook_doc::ErrorCode::BadInput, format!("`{key}` index out of range"))
                    })?;
                    *val = serde_json::Value::String(idx.to_string());
                    if numeric_index.is_none() {
                        numeric_index = Some(idx);
                    }
                } else if let Some(s) = val.as_str()
                    && numeric_index.is_none()
                    && let Ok(idx) = s.trim().parse::<usize>()
                {
                    numeric_index = Some(idx);
                }
            }
        }
    }
    Ok((cloned, numeric_index))
}

fn find_indexed_tab_widget_in_meta(meta: &FrameInspectMeta, idx: usize) -> Option<&curvebook_ui_egui::egui_shell::FrameInspectNodeMeta> {
    for prefix in ["edit.tab.", "fit.tab.", "result.tab."] {
        let mut matches: Vec<&curvebook_ui_egui::egui_shell::FrameInspectNodeMeta> =
            meta.entries.iter().filter(|w| w.widget_id.starts_with(prefix)).collect();
        if !matches.is_empty() {
            if let Some(&entry) = matches.get(idx) {
                return Some(entry);
            }
            matches.clear();
        }
    }
    None
}

fn ensure_params_object(params: &mut serde_json::Value) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    if params.is_null() {
        *params = serde_json::Value::Object(serde_json::Map::new());
    }
    params.as_object_mut()
}

fn to_key_modifiers(modifiers: egui::Modifiers) -> KeyModifiers {
    KeyModifiers {
        shift: modifiers.shift,
        ctrl: modifiers.ctrl,
        alt: modifiers.alt,
        meta: modifiers.mac_cmd || (modifiers.command && !modifiers.ctrl),
    }
}

fn egui_key_to_name(key: egui::Key) -> Option<String> {
    let literal = match key {
        egui::Key::Enter => "Enter",
        egui::Key::Escape => "Escape",
        egui::Key::Backspace => "Backspace",
        egui::Key::Delete => "Delete",
        egui::Key::Tab => "Tab",
        egui::Key::ArrowLeft => "ArrowLeft",
        egui::Key::ArrowRight => "ArrowRight",
        egui::Key::ArrowUp => "ArrowUp",
        egui::Key::ArrowDown => "ArrowDown",
        egui::Key::Home => "Home",
        egui::Key::End => "End",
        egui::Key::PageUp => "PageUp",
        egui::Key::PageDown => "PageDown",
        egui::Key::Space => " ",
        egui::Key::F1 => "F1",
        egui::Key::F2 => "F2",
        egui::Key::F3 => "F3",
        egui::Key::F4 => "F4",
        egui::Key::F5 => "F5",
        egui::Key::F6 => "F6",
        egui::Key::F7 => "F7",
        egui::Key::F8 => "F8",
        egui::Key::F9 => "F9",
        egui::Key::F10 => "F10",
        egui::Key::F11 => "F11",
        egui::Key::F12 => "F12",
        egui::Key::Plus => "+",
        egui::Key::Minus => "-",
        egui::Key::Period => ".",
        egui::Key::Comma => ",",
        egui::Key::Slash => "/",
        egui::Key::Backslash => "\\",
        egui::Key::Equals => "=",
        egui::Key::Num0 => "0",
        egui::Key::Num1 => "1",
        egui::Key::Num2 => "2",
        egui::Key::Num3 => "3",
        egui::Key::Num4 => "4",
        egui::Key::Num5 => "5",
        egui::Key::Num6 => "6",
        egui::Key::Num7 => "7",
        egui::Key::Num8 => "8",
        egui::Key::Num9 => "9",
        _ => return Some(key.symbol_or_name().to_owned()),
    };
    Some(String::from(literal))
}

/// Launch the interactive native desktop window for a prepared [`DesktopHostRuntime`].
///
/// # Errors
/// Returns an error if no open window is present or `eframe` window startup fails.
pub fn run_native_desktop(runtime: DesktopHostRuntime, extra_ttf: Option<Vec<u8>>, on_exit_cleanup: CleanupHook) -> Result<(), String> {
    run_native_desktop_with_control(runtime, extra_ttf, on_exit_cleanup, None)
}

/// Runtime window, macOS Dock, taskbar, and app-switcher icon (`assets/app-icon/`, see its README).
///
/// macOS uses `curvebook-macos-512.png` (which carries Apple's icon-grid inset); other platforms
/// use the full-bleed 256 px hicolor PNG.
fn app_icon() -> egui::IconData {
    #[cfg(target_os = "macos")]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/curvebook-macos-512.png");
    #[cfg(not(target_os = "macos"))]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/org.curvebook.CurveBook.png");
    eframe::icon_data::from_png_bytes(png).unwrap_or_default()
}

/// Detect whether `exe_path` is running from inside a packaged macOS `.app` bundle whose
/// `Contents/Resources/curvebook.icns` is already active via `CFBundleIconFile`.
#[cfg(any(target_os = "macos", test))]
fn is_macos_app_bundle_exe(exe_path: &std::path::Path) -> bool {
    let Some(macos_dir) = exe_path.parent() else {
        return false;
    };
    if macos_dir.file_name().and_then(std::ffi::OsStr::to_str) != Some("MacOS") {
        return false;
    }
    let Some(contents_dir) = macos_dir.parent() else {
        return false;
    };
    if contents_dir.file_name().and_then(std::ffi::OsStr::to_str) != Some("Contents") {
        return false;
    }
    let Some(bundle_dir) = contents_dir.parent() else {
        return false;
    };
    bundle_dir.extension().and_then(std::ffi::OsStr::to_str) == Some("app") && contents_dir.join("Resources").join("curvebook.icns").is_file()
}

/// Select the `ViewportBuilder::with_icon` payload for native desktop launch.
///
/// Inside a packaged macOS `.app` bundle, passing [`egui::IconData::default`] instructs `eframe`
/// not to override the bundle's native `curvebook.icns` (whereas omitting `.with_icon` would fall
/// back to egui's default icon). Unbundled launches (`cargo run` / bare binary) and non-macOS
/// platforms use [`app_icon`].
fn viewport_icon_for_launch() -> egui::IconData {
    #[cfg(target_os = "macos")]
    if std::env::current_exe().ok().is_some_and(|exe| is_macos_app_bundle_exe(&exe)) {
        return egui::IconData::default();
    }
    app_icon()
}

const GRAPHICS_TROUBLESHOOTING_HINT: &str =
    "See docs/reference/troubleshooting.md#gpu-graphics for GPU/Metal driver compatibility and fallback guidance.";

fn format_graphics_init_diagnostic_for_platform(is_macos: bool, stage: &str, detail: &str) -> String {
    let backend_step = if is_macos {
        "- On macOS, CurveBook compiles wgpu with Metal only (`WGPU_BACKEND=metal`; OpenGL is unavailable on macOS builds). Try `WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1` if a legacy or virtualized GPU adapter is rejected."
    } else {
        "- Try launching with an explicit backend override: WGPU_BACKEND=vulkan, WGPU_BACKEND=dx12 (Windows), or WGPU_BACKEND=gl (Linux/Windows), or WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1."
    };
    format!(
        "CurveBook could not initialize GPU graphics ({stage}): {detail}\n\
         Cause: Your system's GPU or graphics driver (such as older macOS / non-native Metal, virtualized GPUs, or incomplete Vulkan/DirectX 12 drivers) may not support the required wgpu features.\n\
         Next steps:\n\
         - Update your OS and GPU drivers (on macOS, ensure native Metal GPU support is available).\n\
         {backend_step}\n\
         - For CPU-only statistical analysis and SVG/PDF/PNG/TIFF figure export without a GPU, use `curvebook-cli` or `curvebook-cli mcp` (`curvebook --headless-once` renders a single discarded 800x520 smoke frame and exits).\n\
         {GRAPHICS_TROUBLESHOOTING_HINT}"
    )
}

fn format_graphics_init_diagnostic(stage: &str, detail: &str) -> String {
    format_graphics_init_diagnostic_for_platform(cfg!(target_os = "macos"), stage, detail)
}

fn format_graphics_runtime_diagnostic_for_platform(is_macos: bool, event: &str, detail: &str) -> String {
    let recovery_step = if is_macos {
        "If the window becomes unresponsive, relaunch with WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1 (macOS builds use Metal only), or use `curvebook-cli` for CPU-only analysis and export."
    } else {
        "If the window becomes unresponsive, relaunch with WGPU_BACKEND=vulkan/dx12/gl or WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1, or use `curvebook-cli` for CPU-only analysis and export."
    };
    format!(
        "CurveBook graphics runtime warning ({event}): {detail}\n\
         This can occur on older macOS / non-native Metal GPUs, virtualized displays, or after a GPU driver reset.\n\
         {recovery_step}\n\
         {GRAPHICS_TROUBLESHOOTING_HINT}"
    )
}

fn format_graphics_runtime_diagnostic(event: &str, detail: &str) -> String {
    format_graphics_runtime_diagnostic_for_platform(cfg!(target_os = "macos"), event, detail)
}

fn format_graphics_init_error(err: &eframe::Error) -> String {
    match err {
        eframe::Error::Wgpu(wgpu_err) => {
            let stage = match wgpu_err {
                eframe::egui_wgpu::WgpuError::RequestAdapterError(_) => "GPU adapter selection failed",
                eframe::egui_wgpu::WgpuError::CustomNativeAdapterSelectionError(_) => "custom GPU adapter selection failed",
                eframe::egui_wgpu::WgpuError::NoSurfaceFormatsAvailable => "no compatible GPU surface format found",
                eframe::egui_wgpu::WgpuError::RequestDeviceError(_) => "GPU device creation failed",
                eframe::egui_wgpu::WgpuError::CreateSurfaceError(_) => "GPU surface creation failed",
                eframe::egui_wgpu::WgpuError::HandleError(_) => "native window handle unavailable for GPU surface",
            };
            format_graphics_init_diagnostic(stage, &wgpu_err.to_string())
        }
        eframe::Error::Winit(os_err) => format_graphics_init_diagnostic("native window creation failed", &os_err.to_string()),
        eframe::Error::WinitEventLoop(loop_err) => format_graphics_init_diagnostic("window event loop initialization failed", &loop_err.to_string()),
        eframe::Error::AppCreation(app_err) => format!("failed to start CurveBook desktop session: {app_err}"),
    }
}

fn classify_wgpu_surface_status(
    status: &eframe::egui_wgpu::wgpu::CurrentSurfaceTexture,
) -> (eframe::egui_wgpu::SurfaceErrorAction, Option<&'static str>) {
    use eframe::egui_wgpu::{SurfaceErrorAction, wgpu::CurrentSurfaceTexture};
    match status {
        CurrentSurfaceTexture::Outdated => (SurfaceErrorAction::Reconfigure, None),
        CurrentSurfaceTexture::Occluded => (SurfaceErrorAction::SkipFrame, None),
        CurrentSurfaceTexture::Lost => (SurfaceErrorAction::RecreateSurface, Some("GPU surface lost; attempting to recreate rendering surface")),
        CurrentSurfaceTexture::Timeout => (SurfaceErrorAction::SkipFrame, Some("GPU surface frame acquisition timed out")),
        CurrentSurfaceTexture::Validation => (SurfaceErrorAction::SkipFrame, Some("GPU surface validation error while acquiring frame")),
        CurrentSurfaceTexture::Success(_) | CurrentSurfaceTexture::Suboptimal(_) => (SurfaceErrorAction::SkipFrame, None),
    }
}

fn build_wgpu_configuration() -> eframe::egui_wgpu::WgpuConfiguration {
    let logged_surface_diagnostic = Arc::new(AtomicBool::new(false));
    eframe::egui_wgpu::WgpuConfiguration {
        on_surface_status: Arc::new(move |status| {
            let (action, detail) = classify_wgpu_surface_status(status);
            if let Some(reason) = detail {
                if !logged_surface_diagnostic.swap(true, Ordering::AcqRel) {
                    eprintln!("{}", format_graphics_runtime_diagnostic("surface error", reason));
                } else {
                    eprintln!("curvebook graphics warning: {reason}");
                }
            }
            action
        }),
        ..eframe::egui_wgpu::WgpuConfiguration::default()
    }
}

fn should_show_native_error_dialog() -> bool {
    let has_display = {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
        }
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        {
            true
        }
    };
    has_display && std::env::var_os("CI").is_none()
}

fn show_native_graphics_error_dialog(title: &str, message: &str) {
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(message)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

fn install_wgpu_device_error_handlers(
    render_state: &eframe::egui_wgpu::RenderState,
    egui_ctx: egui::Context,
    pending_device_lost: Arc<curvebook_platform::sync::Mutex<Option<String>>>,
) {
    let device_lost_reported = Arc::new(AtomicBool::new(false));
    render_state.device.set_device_lost_callback(move |reason, message| {
        if reason == eframe::egui_wgpu::wgpu::DeviceLostReason::Destroyed {
            return;
        }
        let detail = if message.trim().is_empty() { format!("{reason:?}") } else { format!("{reason:?}: {message}") };
        let diagnostic = format_graphics_runtime_diagnostic("GPU device lost", &detail);
        eprintln!("{diagnostic}");
        // Never invoke AppKit/OS dialogs from a background wgpu/Metal callback thread; queue onto the main UI thread instead.
        if !device_lost_reported.swap(true, Ordering::AcqRel) {
            if let Ok(mut slot) = pending_device_lost.lock() {
                *slot = Some(diagnostic);
            }
            egui_ctx.request_repaint();
        }
    });

    let uncaptured_error_reported = Arc::new(AtomicBool::new(false));
    render_state.device.on_uncaptured_error(Arc::new(move |err| {
        if !uncaptured_error_reported.swap(true, Ordering::AcqRel) {
            eprintln!("{}", format_graphics_runtime_diagnostic("uncaptured GPU error", &err.to_string()));
        } else {
            eprintln!("curvebook graphics warning (uncaptured GPU error): {err}");
        }
    }));
}

/// Launch the interactive native desktop window while optionally attaching a [`LiveControlChannel`]
/// so external `--control` automation and human window input share the same live `UiApp` session.
///
/// # Errors
/// Returns an error if no open window is present or `eframe` window startup fails.
pub fn run_native_desktop_with_control(
    mut runtime: DesktopHostRuntime,
    extra_ttf: Option<Vec<u8>>,
    on_exit_cleanup: CleanupHook,
    control: Option<LiveControlChannel>,
) -> Result<(), String> {
    let interactive_dialogs = control.is_none() && should_show_native_error_dialog();
    if control.is_none() {
        runtime.set_url_opener(std::sync::Arc::new(curvebook_ui_egui::SystemPlatformUrlOpener::live()));
    }
    let primary_window_id = runtime
        .focused_window_id()
        .or_else(|| runtime.window_ids().into_iter().next())
        .ok_or_else(|| String::from("no open window available for interactive desktop launch"))?;
    let initial_title = runtime.window(primary_window_id).map(|w| w.window_title()).unwrap_or_else(|_| String::from("CurveBook"));

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("org.curvebook.CurveBook")
            .with_icon(viewport_icon_for_launch())
            .with_inner_size([DEFAULT_WINDOW_WIDTH_PT, DEFAULT_WINDOW_HEIGHT_PT])
            .with_min_inner_size([MIN_WINDOW_WIDTH_PT, MIN_WINDOW_HEIGHT_PT])
            .with_title(initial_title),
        wgpu_options: build_wgpu_configuration(),
        ..Default::default()
    };

    let font_bytes = extra_ttf;
    eframe::run_native(
        "CurveBook",
        native_options,
        Box::new(move |cc| {
            let pending_device_lost = Arc::new(curvebook_platform::sync::Mutex::new(None));
            if let Some(ref render_state) = cc.wgpu_render_state {
                install_wgpu_device_error_handlers(render_state, cc.egui_ctx.clone(), Arc::clone(&pending_device_lost));
            }
            install_desktop_font_definitions(&cc.egui_ctx, font_bytes.as_deref());
            let mut app = CurveBookNativeApp::new(runtime, primary_window_id, Some(on_exit_cleanup))
                .with_device_lost_slot(pending_device_lost, interactive_dialogs);
            #[cfg(target_os = "macos")]
            {
                app.attach_macos_menus_on_windowed_startup();
                crate::mac_menu::install_egui_repaint_event_handler(cc.egui_ctx.clone());
            }
            if let Some(ctrl) = control {
                let _ = ctrl.egui_ctx_cell.set(cc.egui_ctx.clone());
                cc.egui_ctx.request_repaint();
                app = app.with_live_control(ctrl);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|err| {
        let diagnostic = format_graphics_init_error(&err);
        eprintln!("{diagnostic}");
        if interactive_dialogs {
            show_native_graphics_error_dialog("CurveBook Graphics Initialization Error", &diagnostic);
        }
        diagnostic
    })
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{CellCoord, CellValue, SheetId};
    use curvebook_ui_egui::interaction::ViewportMetrics;
    use curvebook_ui_egui::{BETA_ACTIVE_LOCALES, locale_picker_label, normalize_beta_gui_locale};
    use serde_json::{Value, json};

    use super::*;
    use crate::desktop_host::DesktopHostConfig;

    fn test_runtime(viewport: ViewportMetrics) -> Result<(DesktopHostRuntime, DesktopWindowId), String> {
        let mut runtime = DesktopHostRuntime::new(DesktopHostConfig {
            in_window_menus: true,
            new_instance: true,
            recovery_root: None,
            initial_viewport: viewport,
            ..DesktopHostConfig::default()
        });
        let outcome = runtime.dispatch(DesktopHostEvent::WindowOpened { initial_file: None, viewport })?;
        let id = outcome.window_id.ok_or_else(|| String::from("missing opened window id"))?;
        Ok((runtime, id))
    }

    struct GuiHarness {
        app: CurveBookNativeApp,
        win_id: DesktopWindowId,
        ctx: egui::Context,
        size: egui::Vec2,
        req_tx: Option<mpsc::Sender<LiveControlRequest>>,
        auth: ControlAuth,
    }

    impl GuiHarness {
        fn new<T>(
            w: f32,
            h: f32,
            with_control: bool,
            setup: impl FnOnce(&mut DesktopHostRuntime, DesktopWindowId) -> Result<T, String>,
        ) -> Result<Self, String> {
            let (mut runtime, win_id) = test_runtime(ViewportMetrics::new(f64::from(w), f64::from(h), 1.0))?;
            let _ = setup(&mut runtime, win_id)?;
            let mut app = CurveBookNativeApp::new(runtime, win_id, None);
            let req_tx = if with_control {
                let (tx, rx) = mpsc::channel();
                app = app.with_live_control(LiveControlChannel {
                    request_rx: rx,
                    egui_ctx_cell: Arc::new(OnceLock::new()),
                    shutdown: Arc::new(AtomicBool::new(false)),
                    post_hook: None,
                });
                Some(tx)
            } else {
                None
            };
            let ctx = egui::Context::default();
            install_desktop_font_definitions(&ctx, None);
            let auth = ControlAuth::from_token("test-token-gui-harness").map_err(|e| e.to_string())?;
            Ok(Self { app, win_id, ctx, size: egui::vec2(w, h), req_tx, auth })
        }

        fn step(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)), events, ..Default::default() };
            self.app.drain_synthetic_events_into(&mut raw);
            let mut out = self.ctx.run_ui(raw, |ui| self.app.step_ui(ui));
            out.textures_delta.clear();
            out
        }

        fn settle(&mut self, n: usize) {
            for _ in 0..n {
                let _ = self.step(Vec::new());
            }
        }

        fn send_ctrl(&mut self, req: Value) -> Result<Value, String> {
            let raw_bytes = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
            let (tx, rx) = mpsc::channel();
            self.req_tx
                .as_ref()
                .ok_or_else(|| String::from("live control not enabled"))?
                .send(LiveControlRequest { raw_bytes, auth: self.auth.clone(), conn_authenticated: true, reply_tx: tx })
                .map_err(|e| e.to_string())?;
            for _ in 0..24 {
                let _ = self.step(Vec::new());
                if let Ok(res) = rx.try_recv() {
                    let out = res?;
                    let txt = String::from_utf8_lossy(&out.response_bytes);
                    return serde_json::from_str(&txt).map_err(|e| format!("invalid control reply `{txt}`: {e}"));
                }
            }
            Err(format!("timed out waiting for LiveControlRequest reply to {req}"))
        }

        fn send_ok(&mut self, req: Value, msg: &str) -> Result<Value, String> {
            let resp = self.send_ctrl(req)?;
            assert_eq!(resp.get("ok").and_then(Value::as_bool), Some(true), "{msg}: {resp}");
            Ok(resp)
        }

        fn open_edit_modal(&mut self, ds_id: &str, entry_id: &str) -> Result<SheetId, String> {
            self.send_ok(json!({ "id": "sw", "method": "ui.switch_tab", "params": { "sheet_id": ds_id } }), ds_id)?;
            self.send_ok(
                json!({ "id": "az", "method": "ui.pointer_gesture", "params": { "action": "click", "widget_id": "toolbar.analyze" } }),
                "az",
            )?;
            let _ = self.step(Vec::new());
            self.send_ok(json!({ "id": "q", "method": "ui.pointer_gesture", "params": { "action": "click", "widget_id": "analyze.search" } }), "q")?;
            self.send_ok(json!({ "id": "qt", "method": "ui.type_text", "params": { "text": entry_id } }), "qt")?;
            let _ = self.step(Vec::new());
            self.send_ok(
                json!({ "id": "sel", "method": "ui.pointer_gesture", "params": { "action": "click", "widget_id": format!("analyze.entry.{entry_id}") } }),
                entry_id,
            )?;
            let _ = self.step(Vec::new());
            self.send_ok(json!({ "id": "ok", "method": "ui.pointer_gesture", "params": { "action": "click", "widget_id": "modal.ok" } }), "ok")?;
            let _ = self.step(Vec::new());
            let rs_id = if let ActiveDialog::EditAnalysisParameters { result_sheet_id } =
                &self.app.runtime().window(self.win_id)?.app().canvas_dialogs.active_dialog
            {
                result_sheet_id.clone()
            } else {
                let rs_id =
                    self.app.runtime().window(self.win_id)?.app().document().result_sheets.last().map(|r| r.id.clone()).ok_or("missing result")?;
                let doc = self.app.runtime().window(self.win_id)?.app().document().clone();
                assert!(self.app.runtime_mut().window_mut(self.win_id)?.app_mut().canvas_dialogs.open_edit_parameters(&doc, &rs_id));
                let _ = self.step(Vec::new());
                rs_id
            };
            assert!(matches!(
                self.app.runtime().window(self.win_id)?.app().canvas_dialogs.active_dialog,
                ActiveDialog::EditAnalysisParameters { .. }
            ));
            Ok(rs_id)
        }

        fn verify_two_enter_textbox_commit(&mut self, widget_id: &str, text: &str, by_coords: bool) -> Result<(), String> {
            if by_coords {
                let dlg =
                    self.send_ok(json!({ "id": "di", "method": "ui.inspect", "params": { "scope": "dialog", "include_bounds": true } }), "di")?;
                let b = dlg
                    .pointer("/result/dialog/controls")
                    .and_then(Value::as_array)
                    .and_then(|nodes| nodes.iter().find(|n| n.get("widget_id").and_then(Value::as_str) == Some(widget_id)))
                    .and_then(|n| n.get("bounds"))
                    .and_then(Value::as_array)
                    .ok_or_else(|| format!("missing {widget_id} bounds in {dlg}"))?;
                let x = b.first().and_then(Value::as_f64).ok_or("x")? + b.get(2).and_then(Value::as_f64).ok_or("w")? * 0.5;
                let y = b.get(1).and_then(Value::as_f64).ok_or("y")? + b.get(3).and_then(Value::as_f64).ok_or("h")? * 0.5;
                self.send_ok(json!({ "id": "cc", "method": "ui.pointer_gesture", "params": { "action": "click", "x": x, "y": y } }), "coord click")?;
                let fi = self.send_ok(json!({ "id": "fi", "method": "ui.inspect", "params": { "scope": "dialog" } }), "fi")?;
                assert_eq!(fi.pointer("/result/focus_widget_id").and_then(Value::as_str), Some(widget_id), "focus mismatch: {fi}");
            } else {
                self.send_ok(
                    json!({ "id": "wc", "method": "ui.pointer_gesture", "params": { "action": "click", "widget_id": widget_id } }),
                    widget_id,
                )?;
            }
            self.send_ok(json!({ "id": "tt", "method": "ui.type_text", "params": { "text": text, "clear_first": true } }), text)?;
            self.send_ok(json!({ "id": "e1", "method": "ui.press_key", "params": { "key": "Enter" } }), "1st Enter")?;
            assert!(matches!(
                self.app.runtime().window(self.win_id)?.app().canvas_dialogs.active_dialog,
                ActiveDialog::EditAnalysisParameters { .. }
            ));
            let ie1 = self.send_ok(json!({ "id": "ie1", "method": "ui.inspect", "params": { "scope": "dialog" } }), "ie1")?;
            assert_eq!(ie1.pointer("/result/focus_widget_id"), Some(&Value::Null));
            self.send_ok(json!({ "id": "e2", "method": "ui.press_key", "params": { "key": "Enter" } }), "2nd Enter")?;
            assert_eq!(self.app.runtime().window(self.win_id)?.app().canvas_dialogs.active_dialog, ActiveDialog::None);
            Ok(())
        }
    }

    fn ptr_btn(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    fn key_evt(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
    }

    #[test]
    fn inline_editor_42_key_plus_text_does_not_duplicate() -> Result<(), String> {
        let mut h = GuiHarness::new(960.0, 640.0, false, |rt, id| {
            rt.window_mut(id)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-inline", "kind": "column", "cols": 1, "rows": 3 }))
        })?;
        h.settle(2);
        let c = h.app.inspect_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing grid.cell.0.0.0")?.clipped_rect.center();
        let _ = h.step(vec![egui::Event::PointerMoved(c), ptr_btn(c, true), ptr_btn(c, false), ptr_btn(c, true), ptr_btn(c, false)]);
        let _ = h.step(Vec::new());
        let _ = h.step(vec![
            key_evt(egui::Key::Num4, true, egui::Modifiers::NONE),
            egui::Event::Text(String::from("4")),
            key_evt(egui::Key::Num2, true, egui::Modifiers::NONE),
            egui::Event::Text(String::from("2")),
            key_evt(egui::Key::Enter, true, egui::Modifiers::NONE),
        ]);
        let _ = h.step(Vec::new());
        let ds = h.app.runtime.window(h.win_id)?.app().document().data_sheet(&SheetId::new("ds-inline")).ok_or("missing ds-inline")?;
        assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(42.0));
        Ok(())
    }

    #[test]
    fn hidpi_canvas_resize_and_relative_pointer_offset() -> Result<(), String> {
        let mut h = GuiHarness::new(400.0, 300.0, false, |_, _| Ok(()))?;
        let mut out = h.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0))),
                viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(2.0), ..Default::default() })]
                    .into_iter()
                    .collect(),
                events: vec![egui::Event::PointerMoved(egui::pos2(120.0, 74.0))],
                ..Default::default()
            },
            |ui| h.app.step_ui(ui),
        );
        out.textures_delta.clear();
        assert!(!out.shapes.is_empty());
        let win = h.app.runtime.window(h.win_id)?;
        assert_eq!(win.viewport().scale_factor, 2.0);
        assert!(h.app.last_frame_size[0] > 0 && h.app.last_frame_size[1] > 0);
        assert!(win.viewport().logical_width > 0.0 && win.viewport().logical_width <= 800.0);
        assert!(win.viewport().logical_height > 0.0 && win.viewport().logical_height <= 500.0);
        Ok(())
    }

    #[test]
    fn unsaved_close_request_opens_modal_and_cancels_os_close() -> Result<(), String> {
        let mut h =
            GuiHarness::new(960.0, 640.0, false, |rt, id| rt.window_mut(id)?.execute_and_journal("table.importCsv", json!({ "csv": "X,Y\n1,2\n" })))?;
        let mut raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)), ..Default::default() };
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);
        let mut out = h.ctx.run_ui(raw, |ui| h.app.handle_close_request(ui.ctx(), h.win_id));
        out.textures_delta.clear();
        assert!(h.app.close_dialog_open && !h.app.close_confirmed);
        assert!(out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.contains(&egui::ViewportCommand::CancelClose)));
        Ok(())
    }

    #[derive(Debug)]
    struct TestGuiClipboard {
        fail_writes: bool,
        available: bool,
    }

    impl crate::desktop_host::OsClipboardAdapter for TestGuiClipboard {
        fn write_text(&self, _text: &str) -> Result<curvebook_ui_egui::OsAdapterStatus, String> {
            if self.fail_writes {
                return Err(String::from("pasteboard write denied"));
            }
            Ok(self.status())
        }

        fn write_bundle(&self, _bundle: &curvebook_ui_egui::ClipboardBundle) -> Result<curvebook_ui_egui::OsAdapterStatus, String> {
            if self.fail_writes {
                return Err(String::from("bundle write denied"));
            }
            Ok(self.status())
        }

        fn read_text(&self) -> Result<curvebook_ui_egui::OsAdapterStatus, String> {
            Ok(curvebook_ui_egui::OsAdapterStatus::unavailable("mock", "empty"))
        }
    }

    impl TestGuiClipboard {
        fn status(&self) -> curvebook_ui_egui::OsAdapterStatus {
            curvebook_ui_egui::OsAdapterStatus {
                available: self.available,
                availability: if self.available {
                    curvebook_ui_egui::OsAdapterAvailability::Submitted
                } else {
                    curvebook_ui_egui::OsAdapterAvailability::Unavailable
                },
                backend: String::from("mock"),
                detail: String::from(if self.available { "ok" } else { "headless" }),
                text: None,
                job_id: None,
            }
        }
    }

    #[test]
    fn r2_s11_07_graph_copy_skips_ctx_copy_text_surfaces_errors_and_cut_clears_selection() -> Result<(), String> {
        let mut graph_id = SheetId::new("g");
        let mut h = GuiHarness::new(960.0, 640.0, false, |rt, id| {
            rt.set_clipboard_adapter(Box::new(TestGuiClipboard { available: true, fail_writes: false }));
            let win = rt.window_mut(id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-1", "kind": "column", "cols": 1, "rows": 3, "auto_graph": true }))?;
            win.execute_and_journal("table.setCell", json!({ "sheet_id": "ds-1", "row": 0, "col": 0, "value": 11.0 }))?;
            win.execute_and_journal("table.setCell", json!({ "sheet_id": "ds-1", "row": 1, "col": 0, "value": 22.0 }))?;
            graph_id = win.app().document().graph_sheets.first().ok_or("missing graph")?.id.clone();
            win.execute_and_journal("sheet.select", json!({ "sheet_id": graph_id.as_str() }))
        })?;

        // 1. Copy on GraphSheet must NOT emit OutputCommand::CopyText.
        let _ = h.step(Vec::new());
        let out_graph = h.step(vec![egui::Event::Copy]);
        assert!(h.app.runtime.clipboard_bundle().is_some() && h.app.runtime.clipboard_text().is_none());
        assert!(!out_graph.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))));

        // 2. Cut on DataSheet clears ONLY the selected cell.
        h.app.runtime.window_mut(h.win_id)?.execute_and_journal("sheet.select", json!({ "sheet_id": "ds-1" }))?;
        let _ = h.step(Vec::new());
        let pt = h.app.inspect_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing grid.cell.0.0.0")?.clipped_rect.center();
        let _ = h.step(vec![egui::Event::PointerMoved(pt), ptr_btn(pt, true), ptr_btn(pt, false)]);
        let _ = h.step(vec![egui::Event::Cut]);
        let ds = h.app.runtime.window(h.win_id)?.app().document().data_sheet(&SheetId::new("ds-1")).ok_or("missing ds-1")?;
        assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), None);
        assert_eq!(ds.cell(CellCoord::new(1, 0, 0)).and_then(|c| c.active_number()), Some(22.0));

        // 3. Failing OS clipboard surfaced in status_banner.
        h.app.runtime.window_mut(h.win_id)?.execute_and_journal("sheet.select", json!({ "sheet_id": graph_id.as_str() }))?;
        h.app.runtime.set_clipboard_adapter(Box::new(TestGuiClipboard { available: true, fail_writes: true }));
        let _ = h.step(Vec::new());
        let _ = h.step(vec![egui::Event::Copy]);
        assert!(h.app.status_banner().is_some_and(|s| s.contains("denied")));
        Ok(())
    }

    #[test]
    fn r2_s11_02_cmd_s_writes_archive_and_unpacked_dir_and_respects_all_modals() -> Result<(), String> {
        let base_dir = std::env::temp_dir().join(format!("cb-gui-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base_dir);
        std::fs::create_dir_all(&base_dir).map_err(|e| e.to_string())?;
        let archive_path = base_dir.join("project.curvebook");
        let unpacked_path = base_dir.join("project.curvebook.d");

        let mut h = GuiHarness::new(960.0, 640.0, false, |rt, id| {
            let win = rt.window_mut(id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "sv-1", "kind": "column", "cols": 1, "rows": 2 }))?;
            win.execute_and_journal("project.save", json!({ "path": archive_path.to_string_lossy(), "unpacked": false }))?;
            win.app_mut().session_mut().set_file_path(Some(archive_path.clone()), true);
            win.execute_and_journal("table.setCell", json!({ "sheet_id": "sv-1", "row": 0, "col": 0, "value": 77.25 }))
        })?;
        let cmd_s = key_evt(egui::Key::S, true, egui::Modifiers::COMMAND);

        for is_vars in [false, true] {
            let g = &mut h.app.runtime.window_mut(h.win_id)?.app_mut().grid;
            if is_vars {
                g.variables_dialog.open = true;
            } else {
                g.import_wizard.open = true;
            }
            h.app.dispatch_egui_events(&h.ctx, h.win_id, std::slice::from_ref(&cmd_s));
            assert!(h.app.runtime.window(h.win_id)?.has_unsaved_changes());
            let g = &mut h.app.runtime.window_mut(h.win_id)?.app_mut().grid;
            if is_vars {
                g.variables_dialog.open = false;
            } else {
                g.import_wizard.open = false;
            }
        }

        h.app.dispatch_egui_events(&h.ctx, h.win_id, std::slice::from_ref(&cmd_s));
        assert!(!h.app.runtime.window(h.win_id)?.has_unsaved_changes());
        let reopened = curvebook_format::open_project_file(&archive_path).map_err(|e| e.to_string())?;
        assert_eq!(
            reopened.data_sheet(&SheetId::new("sv-1")).and_then(|s| s.cell(CellCoord::new(0, 0, 0))).and_then(|c| c.active_number()),
            Some(77.25)
        );

        {
            let win = h.app.runtime.window_mut(h.win_id)?;
            win.execute_and_journal("project.save", json!({ "path": unpacked_path.to_string_lossy(), "unpacked": true }))?;
            win.app_mut().session_mut().set_file_path(Some(unpacked_path.clone()), true);
            win.execute_and_journal("table.setCell", json!({ "sheet_id": "sv-1", "row": 1, "col": 0, "value": 88.5 }))?;
        }
        h.app.dispatch_egui_events(&h.ctx, h.win_id, &[cmd_s]);
        assert!(!h.app.runtime.window(h.win_id)?.has_unsaved_changes());
        let reopened_dir = curvebook_format::open_project_unpacked_dir(&unpacked_path).map_err(|e| e.to_string())?;
        assert_eq!(
            reopened_dir.data_sheet(&SheetId::new("sv-1")).and_then(|s| s.cell(CellCoord::new(1, 0, 0))).and_then(|c| c.active_number()),
            Some(88.5)
        );
        let _ = std::fs::remove_dir_all(&base_dir);
        Ok(())
    }

    #[cfg(feature = "accessibility")]
    #[test]
    fn r10_w5_accesskit_tree_sync_preserves_single_parent_and_focus() -> Result<(), String> {
        let mut h = GuiHarness::new(1024.0, 640.0, false, |rt, id| {
            rt.window_mut(id)?.execute_and_journal("sheet.createData", json!({ "sheet_id": "ak-1", "kind": "column", "cols": 2, "rows": 3 }))
        })?;
        h.ctx.enable_accesskit();
        let mut last_update = None;
        for _ in 0..3 {
            last_update = h.step(Vec::new()).platform_output.accesskit_update;
        }
        let update = last_update.ok_or("expected accesskit_update")?;
        assert!(!update.nodes.is_empty());
        let mut parent_counts: std::collections::BTreeMap<egui::accesskit::NodeId, usize> = std::collections::BTreeMap::new();
        for (_, node) in &update.nodes {
            for &child_id in node.children() {
                *parent_counts.entry(child_id).or_default() += 1;
            }
        }
        assert!(parent_counts.values().all(|&c| c == 1));
        let analyze = h.app.inspect_meta().find_by_widget_id("toolbar.analyze").ok_or("missing toolbar.analyze")?.clone();
        assert!(update.nodes.iter().any(|(nid, _)| *nid == analyze.accesskit_id));

        let req = egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: analyze.accesskit_id,
            data: None,
        };
        let _ = h.step(vec![egui::Event::AccessKitActionRequest(req)]);
        let _ = h.step(Vec::new());
        assert!(matches!(h.app.runtime.window(h.win_id)?.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
        Ok(())
    }

    #[test]
    fn language_picker_labels_mark_15_non_english_locales_as_draft() {
        use curvebook_ui_egui::LOCALES;
        assert_eq!((LOCALES.len(), locale_picker_label("en").as_str()), (16, "English (en)"));
        for &loc in LOCALES.iter().skip(1) {
            let label = locale_picker_label(loc);
            assert!(label.ends_with(" [Draft]") && label.contains(&format!("({loc})")));
        }
        assert_eq!((BETA_ACTIVE_LOCALES.len(), BETA_ACTIVE_LOCALES.first().copied()), (14, Some("en")));
        for &loc in BETA_ACTIVE_LOCALES.iter().skip(1) {
            assert!(!curvebook_ui_egui::i18n::is_rtl_locale(loc) && locale_picker_label(loc).ends_with(" [Draft]"));
        }
        for (raw, expected) in [("ar", "en"), ("he", "en"), ("fa", "en"), ("ur", "en"), ("pt", "pt-BR")] {
            assert_eq!(normalize_beta_gui_locale(raw), expected);
        }
    }

    #[test]
    fn r15_5_gui_window_pure_egui_live_gesture_analyze_and_edit_params() -> Result<(), String> {
        let mut h = GuiHarness::new(1024.0, 640.0, true, |rt, id| {
            let win = rt.window_mut(id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "col-h1", "kind": "column", "cols": 3, "rows": 4 }))?;
            for col in 0..3 {
                for row in 0..4 {
                    let val = 10.0 + (col as f64) * 3.0 + (row as f64) * 0.25;
                    win.execute_and_journal("table.setCell", json!({ "sheet_id": "col-h1", "row": row, "col": col, "value": val }))?;
                }
            }
            Ok(())
        })?;
        h.settle(3);
        assert!(h.app.inspect_meta().find_by_widget_id("toolbar.analyze").is_some());

        let g1 = h.send_ok(json!({"id":"g1","method":"ui.pointer_gesture","params":{"action":"click","widget_id":"toolbar.analyze"}}), "g1")?;
        assert_eq!(g1.pointer("/result/egui_dispatched").and_then(Value::as_bool), Some(true));
        let _ = h.step(Vec::new());
        assert!(matches!(h.app.runtime().window(h.win_id)?.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));

        h.send_ok(
            json!({"id":"sel","method":"ui.pointer_gesture","params":{"action":"click","widget_id":"analyze.entry.stats.one_way_anova"}}),
            "sel",
        )?;
        let _ = h.step(Vec::new());
        h.send_ok(json!({"id":"run","method":"ui.pointer_gesture","params":{"action":"click","widget_id":"modal.ok"}}), "run")?;
        let _ = h.step(Vec::new());
        let active_rs = h.app.runtime().window(h.win_id)?.app().document().active_sheet_id.clone().ok_or("expected ResultSheet")?;
        assert!(h.app.runtime().window(h.win_id)?.app().document().result_sheet(&active_rs).is_some());

        let _ = h.step(vec![
            key_evt(egui::Key::P, true, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT),
            key_evt(egui::Key::P, false, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT),
        ]);
        let _ = h.step(Vec::new());
        assert!(matches!(h.app.runtime().window(h.win_id)?.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }));
        Ok(())
    }

    #[test]
    fn runtime_app_icon_decodes_from_assets_logo() {
        let icon = super::app_icon();
        #[cfg(target_os = "macos")]
        assert_eq!((icon.width, icon.height), (512, 512));
        #[cfg(not(target_os = "macos"))]
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), (icon.width as usize) * (icon.height as usize) * 4);
        assert_eq!(icon.rgba[3], 0);
        assert!(icon.rgba.as_chunks::<4>().0.iter().any(|px| px[3] == 255));
    }

    #[test]
    fn macos_app_bundle_exe_detection_requires_icns() -> Result<(), Box<dyn std::error::Error>> {
        let base = std::env::temp_dir().join(format!("curvebook-bundle-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let exe = base.join("CurveBook.app").join("Contents").join("MacOS").join("curvebook");
        let resources = base.join("CurveBook.app").join("Contents").join("Resources");
        std::fs::create_dir_all(exe.parent().ok_or("missing parent")?)?;
        std::fs::create_dir_all(&resources)?;
        std::fs::write(&exe, b"")?;

        assert!(!super::is_macos_app_bundle_exe(&exe));
        std::fs::write(resources.join("curvebook.icns"), b"icns")?;
        assert!(super::is_macos_app_bundle_exe(&exe));
        assert!(!super::is_macos_app_bundle_exe(std::path::Path::new("/usr/local/bin/curvebook")));
        let _ = std::fs::remove_dir_all(&base);
        Ok(())
    }

    #[test]
    fn r15_6_a1_host_records_checkpoint_and_banner_on_partial_batch_error() -> Result<(), Box<dyn std::error::Error>> {
        use std::sync::atomic::AtomicUsize;

        let rec_dir = std::env::temp_dir().join(format!("curvebook-r15-6-a1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&rec_dir);
        let mut runtime = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(rec_dir.clone()), ..DesktopHostConfig::default() });
        let pref_calls = Arc::new(AtomicUsize::new(0));
        let pref_hook = Arc::clone(&pref_calls);
        runtime.set_preferences_persist_hook(move |_| {
            pref_hook.fetch_add(1, Ordering::Relaxed);
        });
        let win_id = runtime.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
        let mut native = CurveBookNativeApp::new(runtime, win_id, None);
        let ctx = egui::Context::default();

        native
            .pending_intents
            .push_engine("sheet.createData", json!({ "sheet_id": "ds-a1-host", "title": "A1 Host", "kind": "column", "auto_graph": false }));
        native.pending_intents.push_engine("sheet.delete", json!({ "sheet_id": "missing-sheet" }));
        native.pending_intents.push(UiIntent::SetUiScale(1.25));
        native.commit_pending_intents(&ctx, win_id, false);

        assert!(native.status_banner().is_some());
        let win = native.runtime().window(win_id)?;
        assert!(win.app().document().data_sheet(&SheetId::from("ds-a1-host")).is_some());
        assert!(win.dirty_commands().iter().any(|c| c == "sheet.createData"));
        assert!((win.app().ui_scale() - 1.25).abs() < 1e-4 && pref_calls.load(Ordering::Relaxed) >= 1);
        let _ = std::fs::remove_dir_all(&rec_dir);
        Ok(())
    }

    #[test]
    fn r15_6_a3_welcome_open_by_path_honors_dirty_check_and_emf_routes_to_host_save() -> Result<(), Box<dyn std::error::Error>> {
        use crate::desktop_host::{NativeFileDialogKind, StubNativeFileDialogAdapter};

        let tmp_dir = std::env::temp_dir().join(format!("curvebook-r15-6-a3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp_dir);
        std::fs::create_dir_all(&tmp_dir)?;
        let saved_project = tmp_dir.join("saved-assay.curvebook");
        let emf_out = tmp_dir.join("figure-out.emf");
        {
            let mut seed = UiApp::new();
            let _ = seed.execute_command(
                "sheet.createData",
                json!({ "sheet_id": "ds-saved", "title": "Saved Assay", "kind": "xy", "cols": 1, "rows": 3, "auto_graph": false }),
            )?;
            let _ = seed.execute_command("table.setXCells", json!({ "sheet_id": "ds-saved", "values": [1.0, 2.0, 3.0] }))?;
            let _ = seed.execute_command("table.setCells", json!({ "sheet_id": "ds-saved", "rows": [[5.0], [10.0], [15.0]] }))?;
            let _ = seed.execute_command("graph.create", json!({ "sheet_id": "gs-emf", "data_sheet_id": "ds-saved", "title": "EMF Graph" }))?;
            let _ = seed.execute_command("project.save", json!({ "path": saved_project.to_string_lossy(), "unpacked": false }))?;
        }

        let adapter = Arc::new(StubNativeFileDialogAdapter::new());
        let mut runtime = DesktopHostRuntime::new(DesktopHostConfig::default());
        runtime.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);
        let win_id = runtime.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
        let mut native = CurveBookNativeApp::new(runtime, win_id, None);
        let ctx = egui::Context::default();

        // 1. Clean window: Welcome Hub `project.open` with explicit `path` loads directly without OS picker.
        native.pending_intents.push_engine("project.open", json!({ "path": saved_project.to_string_lossy() }));
        native.commit_pending_intents(&ctx, win_id, false);
        assert!(native.status_banner().is_none() && adapter.requests().is_empty() && native.runtime().window_count() == 1);
        assert!(native.runtime().window(win_id)?.app().document().data_sheet(&SheetId::from("ds-saved")).is_some());

        // 2. Dirty window preserves edit and opens target in a new window.
        native
            .runtime_mut()
            .window_mut(win_id)?
            .execute_and_journal("table.setCell", json!({ "sheet_id": "ds-saved", "row": 0, "col": 0, "value": 999.0 }))?;
        native.pending_intents.push_engine("project.open", json!({ "path": saved_project.to_string_lossy() }));
        native.commit_pending_intents(&ctx, win_id, false);
        assert!(native.status_banner().is_none() && adapter.requests().is_empty() && native.runtime().window_count() == 2);
        let orig = native
            .runtime()
            .window(win_id)?
            .app()
            .document()
            .data_sheet(&SheetId::from("ds-saved"))
            .and_then(|d| d.cell(CellCoord::new(0, 0, 0)))
            .ok_or("missing")?;
        assert_eq!(orig.value, CellValue::Number(999.0));

        // 3. EMF option routes through SaveFile dialog and writes EMF bytes.
        adapter.push_response("graph.exportEmf", Some(emf_out.clone()));
        native.pending_intents.push_engine("graph.exportEmf", json!({ "sheet_id": "gs-emf" }));
        native.commit_pending_intents(&ctx, win_id, false);
        assert!(native.status_banner().is_none());
        let reqs = adapter.requests();
        assert!(reqs.len() == 1 && reqs[0].command_id == "graph.exportEmf" && reqs[0].kind == NativeFileDialogKind::SaveFile);
        assert!(std::fs::read(&emf_out)?.len() > 80);
        let _ = std::fs::remove_dir_all(&tmp_dir);
        Ok(())
    }

    #[test]
    fn m2_gui_window_remediation_guards_drag_alias_hover_and_pinch() -> Result<(), String> {
        let mut h = GuiHarness::new(1024.0, 640.0, true, |rt, id| {
            let win = rt.window_mut(id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-m2", "kind": "xy", "cols": 1, "rows": 3, "auto_graph": false }))?;
            win.execute_and_journal("table.setXCells", json!({ "sheet_id": "ds-m2", "values": [1.0, 2.0, 3.0] }))?;
            win.execute_and_journal("table.setCells", json!({ "sheet_id": "ds-m2", "rows": [[4.0], [5.0], [6.0]] }))?;
            win.execute_and_journal("graph.create", json!({ "sheet_id": "gs-m2", "data_sheet_id": "ds-m2" }))?;
            win.execute_and_journal("layout.create", json!({ "sheet_id": "ly-m2", "title": "Layout M2" }))?;
            let item = curvebook_doc::LayoutItem::LiveGraph {
                id: String::from("item-1"),
                graph_sheet_id: SheetId::from("gs-m2"),
                bounds_pt: curvebook_doc::RectPt::new(40.0, 40.0, 200.0, 150.0),
                scale_percent: 100.0,
                badge_override: None,
            };
            win.execute_and_journal("layout.setSpec", json!({ "sheet_id": "ly-m2", "spec": { "items": [item] } }))?;
            win.execute_and_journal("sheet.select", json!({ "sheet_id": "ly-m2" }))
        })?;
        h.settle(3);

        // 1. human_path_only blocks widget_id / to_widget_id.
        h.app.runtime_mut().window_mut(h.win_id)?.app_mut().set_human_path_only(true);
        for req in [
            json!({"id":"h1","method":"ui.pointer_gesture","params":{"action":"click","widget_id":"toolbar.analyze"}}),
            json!({"id":"h2","method":"ui.pointer_gesture","params":{"action":"drag","x":100.0,"y":100.0,"to_widget_id":"toolbar.analyze"}}),
        ] {
            let r = h.send_ctrl(req)?;
            assert_eq!(r.pointer("/error_detail/code").and_then(Value::as_str), Some("conflict"));
            assert_eq!(r.pointer("/error_detail/details/code").and_then(Value::as_str), Some("human_path_only_policy"));
        }
        h.app.runtime_mut().window_mut(h.win_id)?.app_mut().set_human_path_only(false);

        // 2. Drag rejects missing destination (BadInput) and unknown to_widget_id (NotFound).
        for (req, code) in [
            (json!({"id":"d1","method":"ui.pointer_gesture","params":{"action":"drag","widget_id":"canvas.layout_panel.0"}}), "bad_input"),
            (
                json!({"id":"d2","method":"ui.pointer_gesture","params":{"action":"drag","widget_id":"canvas.layout_panel.0","to_widget_id":"nonexistent.target"}}),
                "not_found",
            ),
        ] {
            assert_eq!(h.send_ctrl(req)?.pointer("/error_detail/code").and_then(Value::as_str), Some(code));
        }

        // 3. Short drags (7.5pt, 9.0pt) and diagonal drag genuinely translate layout item.
        let panel_bounds = |app: &CurveBookNativeApp, wid: DesktopWindowId| -> Result<curvebook_doc::RectPt, String> {
            app.runtime()
                .window(wid)?
                .app()
                .document()
                .layout_sheet(&SheetId::from("ly-m2"))
                .and_then(|ls| ls.spec.items.first())
                .and_then(|it| match it {
                    curvebook_doc::LayoutItem::LiveGraph { bounds_pt, .. } => Some(*bounds_pt),
                    _ => None,
                })
                .ok_or_else(|| String::from("missing layout item 0"))
        };
        for dist_pt in [7.5_f64, 9.0_f64] {
            let before_x = panel_bounds(&h.app, h.win_id)?.x;
            let c = h.app.inspect_meta().find_by_widget_id("canvas.layout_panel.0").ok_or("missing panel 0")?.clipped_rect.center();
            h.send_ok(json!({"id":format!("short_{dist_pt}"),"method":"ui.pointer_gesture","params":{"action":"drag","widget_id":"canvas.layout_panel.0","to_x":f64::from(c.x)+dist_pt,"to_y":f64::from(c.y)}}), "short drag")?;
            let _ = h.step(Vec::new());
            assert!(panel_bounds(&h.app, h.win_id)?.x > before_x);
        }
        let c = h.app.inspect_meta().find_by_widget_id("canvas.layout_panel.0").ok_or("missing panel 0")?.clipped_rect.center();
        h.send_ok(json!({"id":"d3","method":"ui.pointer_gesture","params":{"action":"drag","widget_id":"canvas.layout_panel.0","to_x":f64::from(c.x)+80.0,"to_y":f64::from(c.y)+50.0}}), "diag drag")?;
        let _ = h.step(Vec::new());
        let moved = panel_bounds(&h.app, h.win_id)?;
        assert!(moved.x > 60.0 && moved.y > 60.0);

        // 4. Pinch rejects non-positive scale_delta and accepts positive values.
        for bad in [0.0_f64, -1.5] {
            let r =
                h.send_ctrl(json!({"id":"pb","method":"ui.pointer_gesture","params":{"action":"pinch","x":400.0,"y":300.0,"scale_delta":bad}}))?;
            assert_eq!(r.pointer("/error_detail/code").and_then(Value::as_str), Some("bad_input"));
        }
        h.send_ok(json!({"id":"p1","method":"ui.pointer_gesture","params":{"action":"pinch","x":400.0,"y":300.0,"scale_delta":1.25}}), "pinch")?;

        // 5. Alias resolution, disabled hover, and `power.curve_hover` parity.
        h.app.runtime_mut().window_mut(h.win_id)?.app_mut().canvas_dialogs.open_power_wizard();
        let _ = h.step(Vec::new());
        h.send_ok(json!({"id":"hb","method":"ui.pointer_gesture","params":{"action":"hover","widget_id":"dialog.button.back"}}), "disabled hover")?;
        for _ in 0..3 {
            h.send_ok(json!({"id":"nxt","method":"ui.pointer_gesture","params":{"action":"click","widget_id":"dialog.button.next"}}), "next")?;
            let _ = h.step(Vec::new());
        }
        h.send_ok(json!({"id":"pt","method":"ui.pointer_gesture","params":{"action":"click","widget_id":"power.tab.curve"}}), "power tab")?;
        let _ = h.step(Vec::new());
        let ch = h.send_ok(
            json!({"id":"pch","method":"ui.pointer_gesture","params":{"action":"hover","widget_id":"power.curve_hover","hover_fraction_x":0.25}}),
            "curve hover",
        )?;
        assert!(
            ch.pointer("/result/hover_n").and_then(Value::as_f64).is_some() && ch.pointer("/result/hover_power").and_then(Value::as_f64).is_some()
        );
        assert!(h.app.runtime().window(h.win_id)?.app().canvas_dialogs.power_wizard.hover_n.is_some());
        h.send_ok(json!({"id":"c1","method":"ui.pointer_gesture","params":{"action":"click","widget_id":"dialog.button.cancel"}}), "cancel")?;
        let _ = h.step(Vec::new());
        assert_eq!(h.app.runtime().window(h.win_id)?.app().canvas_dialogs.active_dialog, ActiveDialog::None);
        Ok(())
    }

    #[test]
    fn m2_w5_gui_window_live_context_unified_focus_type_key_inspect_switch_tab() -> Result<(), String> {
        let mut h = GuiHarness::new(1280.0, 800.0, true, |rt, id| {
            let win = rt.window_mut(id)?;
            win.execute_and_journal(
                "sheet.createData",
                json!({ "sheet_id": "ds-col-w5", "kind": "column", "cols": 2, "rows": 4, "auto_graph": false }),
            )?;
            win.execute_and_journal("table.setCells", json!({ "sheet_id": "ds-col-w5", "rows": [[1.0, 4.0], [2.0, 5.0], [3.0, 6.0]] }))?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-fit-w5", "kind": "xy", "cols": 1, "rows": 6, "auto_graph": false }))?;
            win.execute_and_journal("table.setXCells", json!({ "sheet_id": "ds-fit-w5", "values": [0.01, 0.1, 1.0, 10.0, 100.0, 1000.0] }))?;
            win.execute_and_journal("table.setCells", json!({ "sheet_id": "ds-fit-w5", "rows": [[5.0], [12.0], [48.0], [88.0], [97.0], [99.0]] }))?;
            win.execute_and_journal("sheet.select", json!({ "sheet_id": "ds-col-w5" }))
        })?;
        h.settle(4);

        // 1. Grid click -> type_text(submit: true) -> ArrowDown.
        h.send_ok(
            json!({ "id": "gc1", "method": "ui.pointer_gesture", "params": { "action": "click", "widget_id": "grid.cell.0.0.0" } }),
            "grid click",
        )?;
        h.send_ok(json!({ "id": "gt1", "method": "ui.type_text", "params": { "text": "123", "submit": true } }), "grid type")?;
        h.send_ok(json!({ "id": "gd1", "method": "ui.press_key", "params": { "key": "ArrowDown" } }), "grid ArrowDown")?;
        let insp = h.send_ok(json!({ "id": "gi1", "method": "ui.inspect", "params": { "scope": "grid" } }), "grid inspect")?;
        assert_eq!(insp.pointer("/result/grid_selection/active_cell/row").and_then(Value::as_u64), Some(2));
        let cell_val = h
            .app
            .runtime()
            .window(h.win_id)?
            .app()
            .document()
            .data_sheet(&SheetId::from("ds-col-w5"))
            .and_then(|ds| ds.cell(CellCoord::new(0, 0, 0)))
            .map(|c| c.value.clone());
        assert_eq!(cell_val, Some(CellValue::Number(123.0)));

        // 2. `stats.two_group`: two-Enter textbox commit.
        let tg_rs_id = h.open_edit_modal("ds-col-w5", "stats.two_group")?;
        h.verify_two_enter_textbox_commit("edit.custom_title", "GwTitle", false)?;
        assert_eq!(
            h.app.runtime().window(h.win_id)?.app().document().result_sheet(&tg_rs_id).and_then(|rs| rs.analysis.custom_title.as_deref()),
            Some("GwTitle")
        );

        // 3. Coordinate-only human-path (`human_path_only = true`).
        let doc = h.app.runtime().window(h.win_id)?.app().document().clone();
        assert!(h.app.runtime_mut().window_mut(h.win_id)?.app_mut().canvas_dialogs.open_edit_parameters(&doc, &tg_rs_id));
        h.app.runtime_mut().window_mut(h.win_id)?.app_mut().set_human_path_only(true);
        let _ = h.step(Vec::new());
        let blocked = h.send_ctrl(json!({ "id": "gob", "method": "ui.type_text", "params": { "widget_id": "edit.custom_title", "text": "Bad" } }))?;
        assert_eq!(blocked.pointer("/error_detail/code").and_then(Value::as_str), Some("conflict"));
        assert_eq!(blocked.pointer("/error_detail/details/code").and_then(Value::as_str), Some("human_path_only_policy"));
        h.verify_two_enter_textbox_commit("edit.custom_title", "HumanPathGw", true)?;
        assert_eq!(
            h.app.runtime().window(h.win_id)?.app().document().result_sheet(&tg_rs_id).and_then(|rs| rs.analysis.custom_title.as_deref()),
            Some("HumanPathGw")
        );
        h.app.runtime_mut().window_mut(h.win_id)?.app_mut().set_human_path_only(false);

        // 4. `FitModal` -> `ui.switch_tab("fit.tab.output")` -> `"0.95"` -> Enter -> Enter (`R20.20` defers creation until commit).
        let _pending_nl_id = h.open_edit_modal("ds-fit-w5", "fit.nonlinear")?;
        h.send_ok(json!({ "id": "ft1", "method": "ui.switch_tab", "params": { "tab_id": "fit.tab.output" } }), "fit.tab.output")?;
        h.verify_two_enter_textbox_commit("fit.diagnostics.r2.input", "0.95", false)?;
        let committed_r2 = h.app.runtime().window(h.win_id)?.app().document().result_sheets.last().and_then(|rs| match &rs.analysis.kind {
            curvebook_doc::AnalysisKind::NonlinearRegression(fit) => fit.flag_r_squared_below,
            _ => None,
        });
        assert!(committed_r2.is_some_and(|v| (v - 0.95).abs() < 1e-9));
        Ok(())
    }

    #[test]
    fn graphics_init_and_runtime_diagnostics_include_actionable_gpu_metal_guidance() {
        let msg = super::format_graphics_init_error(&eframe::Error::Wgpu(eframe::egui_wgpu::WgpuError::NoSurfaceFormatsAvailable));
        for needle in ["no compatible GPU surface format found", "Metal", "WGPU_BACKEND", "curvebook-cli", "troubleshooting.md#gpu-graphics"] {
            assert!(msg.contains(needle), "missing `{needle}` in {msg}");
        }
        let adapter_msg = super::format_graphics_init_error(&eframe::Error::Wgpu(eframe::egui_wgpu::WgpuError::CustomNativeAdapterSelectionError(
            String::from("no Metal adapter"),
        )));
        assert!(adapter_msg.contains("no Metal adapter") && adapter_msg.contains("WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1"));
        let lost_msg = super::format_graphics_runtime_diagnostic("GPU device lost", "Unknown: Metal device reset");
        assert!(lost_msg.contains("GPU device lost") && lost_msg.contains("Metal") && lost_msg.contains("curvebook-cli"));

        // GAP-07-08: macOS diagnostics must never advise unavailable `WGPU_BACKEND=gl`, and must clarify `--headless-once`.
        let mac_init = super::format_graphics_init_diagnostic_for_platform(true, "GPU adapter selection failed", "no Metal adapter");
        assert!(!mac_init.contains("WGPU_BACKEND=gl"), "macOS init diagnostic must not advise WGPU_BACKEND=gl: {mac_init}");
        assert!(mac_init.contains("WGPU_BACKEND=metal") && mac_init.contains("WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1"));
        assert!(mac_init.contains("curvebook-cli mcp") && mac_init.contains("discarded 800x520 smoke frame"));

        let mac_runtime = super::format_graphics_runtime_diagnostic_for_platform(true, "GPU device lost", "Metal reset");
        assert!(!mac_runtime.contains("WGPU_BACKEND=gl"), "macOS runtime diagnostic must not advise WGPU_BACKEND=gl: {mac_runtime}");
        assert!(mac_runtime.contains("Metal") && mac_runtime.contains("WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=1"));

        let non_mac_init = super::format_graphics_init_diagnostic_for_platform(false, "GPU adapter selection failed", "no Vulkan adapter");
        assert!(non_mac_init.contains("WGPU_BACKEND=gl") && non_mac_init.contains("WGPU_BACKEND=vulkan"));
    }

    #[test]
    fn classify_wgpu_surface_status_maps_recovery_actions_and_diagnostics() {
        use eframe::egui_wgpu::{SurfaceErrorAction, wgpu::CurrentSurfaceTexture};
        for (status, expected_action, expect_msg) in [
            (CurrentSurfaceTexture::Outdated, SurfaceErrorAction::Reconfigure, false),
            (CurrentSurfaceTexture::Occluded, SurfaceErrorAction::SkipFrame, false),
            (CurrentSurfaceTexture::Lost, SurfaceErrorAction::RecreateSurface, true),
            (CurrentSurfaceTexture::Timeout, SurfaceErrorAction::SkipFrame, true),
            (CurrentSurfaceTexture::Validation, SurfaceErrorAction::SkipFrame, true),
        ] {
            let (action, msg) = super::classify_wgpu_surface_status(&status);
            assert_eq!(std::mem::discriminant(&action), std::mem::discriminant(&expected_action));
            assert_eq!(msg.is_some(), expect_msg);
        }
    }

    #[test]
    fn aud_13_12_live_screenshot_frame_budget_overflow_branches() {
        let req_id = serde_json::json!("shot-overflow");
        let png = vec![0x89u8; 512];

        // 1. Fits within frame limit -> emits exact parity schema with single `data_base64` (no `png_base64`).
        let ok_inline = super::finalize_live_screenshot_payload_with_limit(&req_id, "workspace", 640, 480, 144, &png, None, 16 * 1024)
            .expect("inline screenshot fits within limit");
        assert_eq!(ok_inline["region"], serde_json::json!("workspace"));
        assert_eq!(ok_inline["width_px"], serde_json::json!(640));
        assert_eq!(ok_inline["height_px"], serde_json::json!(480));
        assert_eq!(ok_inline["dpi"], serde_json::json!(144));
        assert_eq!(ok_inline["format"], serde_json::json!("png"));
        assert_eq!(ok_inline["mime_type"], serde_json::json!("image/png"));
        assert_eq!(ok_inline["bytes_len"], serde_json::json!(png.len()));
        assert!(ok_inline["path"].is_null());
        assert!(ok_inline["data_base64"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(ok_inline.get("png_base64").is_none());
        assert!(ok_inline.get("inline_omitted").is_none());

        // 2. Exceeds inline limit WITH `path` -> falls back cleanly to path-only metadata (`inline_omitted: true`).
        let path_only = super::finalize_live_screenshot_payload_with_limit(&req_id, "full_window", 640, 480, 144, &png, Some("/tmp/out.png"), 512)
            .expect("path-only fallback fits within metadata budget");
        assert_eq!(path_only["inline_omitted"], serde_json::json!(true));
        assert_eq!(path_only["path"], serde_json::json!("/tmp/out.png"));
        assert_eq!(path_only["bytes_len"], serde_json::json!(png.len()));
        assert!(path_only.get("data_base64").is_none());
        assert!(path_only.get("png_base64").is_none());

        // 3. Exceeds inline limit WITHOUT `path` -> ResourceLimit error (caller never writes disk).
        let pathless_err = super::finalize_live_screenshot_payload_with_limit(&req_id, "full_window", 640, 480, 144, &png, None, 512)
            .expect_err("pathless overflow must fail with ResourceLimit");
        assert_eq!(pathless_err.code, curvebook_doc::ErrorCode::ResourceLimit);

        // 4. Even metadata overflows limit WITH `path` -> ResourceLimit error BEFORE disk write!
        let meta_err = super::finalize_live_screenshot_payload_with_limit(&req_id, "full_window", 640, 480, 144, &png, Some("/tmp/out.png"), 64)
            .expect_err("metadata overflow must fail before write");
        assert_eq!(meta_err.code, curvebook_doc::ErrorCode::ResourceLimit);
    }

    #[test]
    fn r20_8_gui_window_grid_drag_commit_repaints_immediately_and_settles_zero_idle() -> Result<(), String> {
        let mut h = GuiHarness::new(1024.0, 640.0, true, |rt, id| {
            let win = rt.window_mut(id)?;
            win.execute_and_journal(
                "sheet.createData",
                json!({ "sheet_id": "ds-r208", "kind": "column", "cols": 3, "rows": 6, "auto_graph": false }),
            )?;
            win.execute_and_journal("sheet.select", json!({ "sheet_id": "ds-r208" }))
        })?;
        let mut sim_time = 1.0_f64;
        let step_at = |h: &mut GuiHarness, sim_time: &mut f64, dt: f64, events: Vec<egui::Event>| -> egui::FullOutput {
            *sim_time += dt;
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, h.size)),
                time: Some(*sim_time),
                predicted_dt: dt as f32,
                focused: true,
                events,
                ..Default::default()
            };
            h.app.drain_synthetic_events_into(&mut raw);
            let mut out = h.ctx.run_ui(raw, |ui| h.app.step_ui(ui));
            out.textures_delta.clear();
            out
        };
        for _ in 0..20 {
            let out = step_at(&mut h, &mut sim_time, 0.05, Vec::new());
            if out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay == std::time::Duration::MAX) {
                break;
            }
        }

        let c0 = h.app.inspect_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing cell 0,0")?.clipped_rect.center();
        let c1 = h.app.inspect_meta().find_by_widget_id("grid.cell.2.1.0").ok_or("missing cell 2,1")?.clipped_rect.center();

        let out = step_at(
            &mut h,
            &mut sim_time,
            1.0 / 60.0,
            vec![
                egui::Event::PointerMoved(c0),
                egui::Event::PointerButton { pos: c0, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerMoved(c1),
            ],
        );
        let immediate_repaint = out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay == std::time::Duration::ZERO);
        assert!(immediate_repaint, "grid drag intent commit must request immediate repaint");

        let bounds = h.app.runtime().window(h.win_id)?.app().grid.tool.selection_bounds();
        assert_eq!(bounds.map(|(a, b)| ((a.row, a.col, a.subcol), (b.row, b.col, b.subcol))), Some(((0, 0, 0), (2, 1, 0))));

        let _ = step_at(
            &mut h,
            &mut sim_time,
            1.0 / 60.0,
            vec![egui::Event::PointerButton { pos: c1, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }],
        );
        for _ in 0..24 {
            let settle_out = step_at(&mut h, &mut sim_time, 0.05, Vec::new());
            if settle_out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay == std::time::Duration::MAX) {
                break;
            }
        }
        let idle_out = step_at(&mut h, &mut sim_time, 0.1, Vec::new());
        let idle_delay = idle_out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay);
        assert_eq!(idle_delay, Some(std::time::Duration::MAX), "settled grid selection must request zero idle repaints");
        Ok(())
    }

    #[test]
    fn r20_13_gui_harness_installs_bold_and_numeral_font_metadata() -> Result<(), String> {
        let mut h = GuiHarness::new(1024.0, 640.0, false, |_, _| Ok(()))?;
        let _ = h.step(Vec::new());
        let expected_report = curvebook_ui_egui::egui_shell::fonts::build_egui_font_definitions().report;
        assert_eq!(
            curvebook_ui_egui::egui_shell::fonts::has_bold_font(&h.ctx),
            expected_report.bold_face_loaded,
            "live GuiHarness context must mirror BOLD_FONT_FAMILY availability"
        );
        assert!(
            curvebook_ui_egui::egui_shell::fonts::has_aligned_numeral_font(&h.ctx),
            "live GuiHarness context must register ALIGNED_NUMERAL_FONT_FAMILY"
        );
        if cfg!(target_os = "macos") {
            assert!(curvebook_ui_egui::egui_shell::fonts::has_bold_font(&h.ctx));
        }
        let tokens = curvebook_ui_egui::egui_shell::Tokens::default();
        let numeral_font = curvebook_ui_egui::egui_shell::fonts::aligned_numeral_font_id(&h.ctx, &tokens);
        assert_eq!(numeral_font.family, egui::FontFamily::Name(Arc::from(curvebook_ui_egui::egui_shell::fonts::ALIGNED_NUMERAL_FONT_FAMILY)));
        if expected_report.bold_face_loaded {
            let heading_font = curvebook_ui_egui::egui_shell::fonts::section_heading_font_id(&h.ctx, &tokens);
            assert_eq!(heading_font.family, egui::FontFamily::Name(Arc::from(curvebook_ui_egui::egui_shell::fonts::BOLD_FONT_FAMILY)));
        }
        Ok(())
    }
}
