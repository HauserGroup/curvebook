#![forbid(unsafe_code)]

//! Interactive `eframe` + `egui` + `wgpu` native desktop window host for CurveBook.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};

use curvebook_text::TrueTypeFont;
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::rasterizer::EguiScreenshotRegions;
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, GeometryStamp, PendingUiIntents, Tokens, UiIntent, inspect_bridge};
use curvebook_ui_egui::interaction::KeyModifiers;
use curvebook_ui_egui::{ControlAuth, ControlPostRequestContext, ControlRequest, UiApp, is_desktop_host_routed_command};
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

struct DeferredLivePointerGesture {
    req: LiveControlRequest,
    response_payload: serde_json::Value,
}

/// Interactive `eframe::App` wrapping a [`DesktopHostRuntime`] session.
pub struct CurveBookNativeApp {
    runtime: DesktopHostRuntime,
    primary_window_id: DesktopWindowId,
    last_active_sheet_id: Option<curvebook_doc::SheetId>,
    last_frame_size: [u32; 2],
    last_scale_factor: f32,
    last_title: String,
    hovered_paths_active: bool,
    close_dialog_open: bool,
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
    deferred_live_requests: Vec<LiveControlRequest>,
    pending_synthetic_egui_batches: VecDeque<Vec<egui::Event>>,
    pending_live_gesture: Option<DeferredLivePointerGesture>,
    menu_popup_was_open: bool,
    #[cfg(target_os = "macos")]
    mac_menu_host: crate::mac_menu::MacNativeMenuHost,
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
            pending_live_gesture: None,
            menu_popup_was_open: false,
            #[cfg(target_os = "macos")]
            mac_menu_host: crate::mac_menu::MacNativeMenuHost::new(),
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
        let has_deferred_in_flight =
            !self.deferred_live_requests.is_empty() || !self.pending_synthetic_egui_batches.is_empty() || self.pending_live_gesture.is_some();
        let mut immediate = Vec::new();
        for req in pending {
            if should_defer_to_settled_frame(&req) || (has_deferred_in_flight && classify_control_frame(&req.raw_bytes) == ControlFrameKind::Command)
            {
                self.deferred_live_requests.push(req);
            } else {
                immediate.push(req);
            }
        }
        let quit_requested = self.dispatch_pending_live_requests(immediate);
        if quit_requested {
            self.fail_deferred_live_requests("desktop window is closing");
            self.close_confirmed = true;
            self.close_dialog_open = false;
            if let Some(ref ctrl) = self.live_control {
                ctrl.shutdown.store(true, Ordering::Release);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if !self.deferred_live_requests.is_empty() || !self.pending_synthetic_egui_batches.is_empty() || self.pending_live_gesture.is_some() {
            ctx.request_repaint();
        }
        count
    }

    fn dispatch_pending_live_requests(&mut self, pending: Vec<LiveControlRequest>) -> bool {
        let mut quit_requested = false;
        let mut post_hook = self.live_control.as_mut().and_then(|ctrl| ctrl.post_hook.take());
        for req in pending {
            if quit_requested {
                let _ = req.reply_tx.send(Err(String::from("desktop window is closing")));
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

    fn complete_pending_live_gesture(&mut self) {
        let Some(pending) = self.pending_live_gesture.take() else {
            return;
        };
        let Ok(req) = serde_json::from_str::<ControlRequest>(std::str::from_utf8(&pending.req.raw_bytes).unwrap_or("").trim()) else {
            let _ = pending.req.reply_tx.send(Err(String::from("invalid queued pointer gesture payload")));
            return;
        };
        let reply = curvebook_ui_egui::encode_egui_control_response(req.id, Ok(pending.response_payload));
        let Ok(mut response_bytes) = serde_json::to_vec(&reply) else {
            let _ = pending.req.reply_tx.send(Err(String::from("failed to serialize pointer gesture reply")));
            return;
        };
        response_bytes.push(b'\n');
        let _ = pending.req.reply_tx.send(Ok(LiveControlFrameOutput { response_bytes, quit: false, close_connection: false, auth_succeeded: None }));
    }

    fn flush_deferred_live_requests(&mut self, ctx: &egui::Context) {
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
        self.complete_pending_live_gesture();

        if self.deferred_live_requests.is_empty() {
            return;
        }
        let queued = std::mem::take(&mut self.deferred_live_requests);
        let mut iter = queued.into_iter();
        let mut post_hook = self.live_control.as_mut().and_then(|ctrl| ctrl.post_hook.take());
        while let Some(req) = iter.next() {
            if is_pointer_gesture_request(&req) {
                match self.try_enqueue_live_pointer_gesture(ctx, req, &mut post_hook) {
                    LiveGestureDispatch::EguiQueued => {
                        self.deferred_live_requests.extend(iter);
                        ctx.request_repaint();
                        break;
                    }
                    LiveGestureDispatch::ImmediateReply => {}
                }
                continue;
            }
            let injected_bytes = self.prepare_settled_live_request_bytes(&req.raw_bytes, stamp);
            let res = self.dispatch_single_live_request(&injected_bytes, &req.auth, req.conn_authenticated, &mut post_hook);
            let _ = req.reply_tx.send(res);
        }
        if let Some(ref mut ctrl) = self.live_control {
            ctrl.post_hook = post_hook;
        }
    }

    fn try_enqueue_live_pointer_gesture(
        &mut self,
        ctx: &egui::Context,
        req: LiveControlRequest,
        post_hook: &mut Option<LiveControlPostHook>,
    ) -> LiveGestureDispatch {
        let Ok(text) = std::str::from_utf8(&req.raw_bytes) else {
            let res = self.dispatch_single_live_request(&req.raw_bytes, &req.auth, req.conn_authenticated, post_hook);
            let _ = req.reply_tx.send(res);
            return LiveGestureDispatch::ImmediateReply;
        };
        let Ok(parsed) = serde_json::from_str::<ControlRequest>(text.trim()) else {
            let res = self.dispatch_single_live_request(&req.raw_bytes, &req.auth, req.conn_authenticated, post_hook);
            let _ = req.reply_tx.send(res);
            return LiveGestureDispatch::ImmediateReply;
        };
        let params = &parsed.params;
        let action = params
            .get("action")
            .or_else(|| params.get("kind"))
            .or_else(|| params.get("gesture"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("click")
            .trim()
            .to_ascii_lowercase();
        let widget_id =
            params.get("widget_id").or_else(|| params.get("target_id")).and_then(serde_json::Value::as_str).map(str::trim).filter(|s| !s.is_empty());

        let meta_entry = widget_id.and_then(|wid| self.inspect_meta.find_by_widget_id(wid).cloned());
        if let Some(ref entry) = meta_entry
            && !entry.enabled
            && matches!(action.as_str(), "click" | "double_click")
        {
            let msg = entry.disabled_reason.clone().unwrap_or_else(|| format!("widget `{}` is disabled", entry.widget_id));
            let err = curvebook_engine::EngineError::with_code(curvebook_doc::ErrorCode::Conflict, msg);
            let reply = curvebook_ui_egui::encode_egui_control_response(parsed.id, Err(err));
            let bytes = serde_json::to_vec(&reply).map(|mut b| {
                b.push(b'\n');
                LiveControlFrameOutput { response_bytes: b, quit: false, close_connection: false, auth_succeeded: None }
            });
            let _ = req.reply_tx.send(bytes.map_err(|e| e.to_string()));
            return LiveGestureDispatch::ImmediateReply;
        }

        let explicit_xy = params.get("x").and_then(serde_json::Value::as_f64).zip(params.get("y").and_then(serde_json::Value::as_f64));
        let target_pos = meta_entry.as_ref().map(|e| e.clipped_rect.center()).or_else(|| {
            #[allow(clippy::cast_possible_truncation)]
            explicit_xy.map(|(x, y)| egui::pos2(x as f32, y as f32))
        });

        let pos = target_pos.unwrap_or_else(|| self.last_regions.canvas_rect_pt.map_or_else(|| ctx.content_rect().center(), |r| r.center()));
        let egui_mods = parse_egui_modifiers_param(params.get("modifiers"));
        let batches = build_synthetic_egui_gesture_batches(&action, pos, params, egui_mods);
        self.pending_synthetic_egui_batches.extend(batches);
        // Append one empty settle pass so post-gesture layout & `PendingUiIntents` commit before reply/inspect.
        self.pending_synthetic_egui_batches.push_back(Vec::new());
        self.pending_live_gesture = Some(DeferredLivePointerGesture {
            req,
            response_payload: serde_json::json!({
                "action": action,
                "widget_id": widget_id,
                "x": f64::from(pos.x),
                "y": f64::from(pos.y),
                "egui_dispatched": true,
            }),
        });
        LiveGestureDispatch::EguiQueued
    }

    fn prepare_settled_live_request_bytes(&self, raw_bytes: &[u8], stamp: GeometryStamp) -> Vec<u8> {
        let Ok(text) = std::str::from_utf8(raw_bytes) else {
            return raw_bytes.to_vec();
        };
        let Ok(mut req) = serde_json::from_str::<ControlRequest>(text.trim()) else {
            return raw_bytes.to_vec();
        };
        let scale = f64::from(stamp.scale_factor());
        let mut updated = false;
        match req.method.as_str() {
            "ui.screenshot" => {
                let obj = ensure_params_object(&mut req.params);
                if let Some(map) = obj {
                    if !map.contains_key("width_px") {
                        map.insert(String::from("width_px"), serde_json::json!(stamp.physical_width_px.max(64)));
                        updated = true;
                    }
                    if !map.contains_key("height_px") {
                        map.insert(String::from("height_px"), serde_json::json!(stamp.physical_height_px.max(64)));
                        updated = true;
                    }
                    if !map.contains_key("dpi") {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let dpi = (scale * 96.0).round().clamp(72.0, 600.0) as u16;
                        map.insert(String::from("dpi"), serde_json::json!(dpi));
                        updated = true;
                    }
                }
            }
            "ui.inspect" => {
                let obj = ensure_params_object(&mut req.params);
                if let Some(map) = obj
                    && !map.contains_key("metrics")
                {
                    let logical_w = (f64::from(stamp.physical_width_px) / scale).max(160.0);
                    let logical_h = (f64::from(stamp.physical_height_px) / scale).max(120.0);
                    map.insert(
                        String::from("metrics"),
                        serde_json::json!({
                            "logical_width": logical_w,
                            "logical_height": logical_h,
                            "scale_factor": scale,
                        }),
                    );
                    updated = true;
                }
            }
            _ => {}
        }
        if !updated {
            return raw_bytes.to_vec();
        }
        serde_json::to_vec(&req).map_or_else(
            |_| raw_bytes.to_vec(),
            |mut bytes| {
                bytes.push(b'\n');
                bytes
            },
        )
    }

    fn fail_deferred_live_requests(&mut self, reason: &str) {
        for req in self.deferred_live_requests.drain(..) {
            let _ = req.reply_tx.send(Err(String::from(reason)));
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

    fn commit_pending_intents(&mut self, ctx: &egui::Context, window_id: DesktopWindowId, will_discard: bool) {
        if self.pending_intents.is_empty() || will_discard {
            // Preserve pass-0 intents across discarded layout passes (`R15-GUI-05`) until settled.
            return;
        }
        let raw_intents = std::mem::take(&mut self.pending_intents);

        let mut remaining_intents = PendingUiIntents::default();
        let mut host_commands: Vec<(String, serde_json::Value)> = Vec::new();
        let mut host_copy = false;
        let mut host_cut = false;
        let mut host_paste = false;
        let active_is_result = self.runtime.window(window_id).ok().is_some_and(|w| {
            let doc = w.app().document();
            doc.active_sheet_id.as_ref().is_some_and(|id| doc.result_sheet(id).is_some())
        });
        for intent in raw_intents.intents().iter().cloned() {
            match &intent {
                UiIntent::UiCommand { command_id, params } | UiIntent::EngineCommand { command_id, params }
                    if is_desktop_host_routed_command(command_id) =>
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
                UiIntent::CustomViewAction { domain, payload } if domain == "welcome" && payload == "action:open_file" => {
                    host_commands.push((String::from("project.open"), serde_json::json!({})));
                }
                UiIntent::WorkspaceCopy { .. } if !active_is_result => {
                    host_copy = true;
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
            let mut batch_err = batch.first_error().map(ToString::to_string);
            let command_hint = batch.first_succeeded_command_id().map(str::to_owned).or(fallback_command_hint);
            if let Err(err) = win.record_ui_mutation_checkpoint(command_hint.as_deref(), prev_undo, prev_redo, prev_dirty, prev_mutation_seq)
                && batch_err.is_none()
            {
                batch_err = Some(err);
            }
            if let Some(err) = batch_err {
                self.status_banner = Some(err);
            }
            unsaved_action = win.app_mut().egui_views.app_modals.pending_unsaved_action.take();
            let changed = Some(win.app().ui_scale()) != prev_scale
                || Some(win.app().theme.mode) != prev_theme
                || Some(win.app().locale.clone()) != prev_locale
                || Some(&win.app().shortcuts) != prev_shortcuts.as_ref();
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
        if host_cut {
            self.dispatch_clipboard_cut(ctx, window_id);
        }
        if host_paste && let Err(err) = self.runtime.dispatch(DesktopHostEvent::ClipboardPaste { window_id, text_override: None }) {
            self.status_banner = Some(err);
        }
        for (cmd_id, params) in host_commands {
            self.execute_menu_command_with_params(Some(ctx), window_id, &cmd_id, &params);
        }
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
                    if !still_dirty {
                        match self.runtime.dispatch(DesktopHostEvent::WindowCloseRequested { window_id, force: false }) {
                            Ok(outcome) => self.apply_close_outcome(Some(ctx), &outcome),
                            Err(err) => self.status_banner = Some(err),
                        }
                    }
                }
                Err(err) => self.status_banner = Some(err),
            },
            UnsavedCloseAction::DontSave => match self.runtime.dispatch(DesktopHostEvent::WindowCloseRequested { window_id, force: true }) {
                Ok(outcome) => self.apply_close_outcome(Some(ctx), &outcome),
                Err(err) => self.status_banner = Some(err),
            },
            UnsavedCloseAction::Cancel => {
                self.close_dialog_open = false;
            }
        }
    }

    /// Execute one UI frame (polling live control requests, handling OS events, and rendering).
    pub fn step_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.frame_seq = self.frame_seq.wrapping_add(1);
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
        {
            self.sync_macos_menu_for_window(bar_window_id);
            if self.mac_menu_host.is_installed_for_nsapp() {
                while let Some(cmd_id) = self.mac_menu_host.poll_triggered_command_id() {
                    self.execute_menu_command(Some(&ctx), bar_window_id, &cmd_id);
                }
            }
        }
        if let Ok(win) = self.runtime.window_mut(bar_window_id) {
            let in_window_menus = win.chrome_state().menu_bar_visible;
            win.app_mut().host_draws_menu_bar = !in_window_menus;
        }

        let input_window_id = self.active_window_id();
        let ppp = ctx.pixels_per_point().max(1.0);
        let available_rect = ui.available_rect_before_wrap();
        let active_sheet_id = self.runtime.window(input_window_id).map(|w| w.app().document().active_sheet_id.clone()).unwrap_or(None);

        if active_sheet_id != self.last_active_sheet_id {
            self.last_active_sheet_id = active_sheet_id;
            ctx.request_repaint();
        }

        let mut next_intents = PendingUiIntents::default();
        let mut next_meta = FrameInspectMeta::default();
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
        self.inspect_meta = next_meta;
        self.last_regions = report.regions;
        self.pending_intents.merge_pass(next_intents);
        self.menu_popup_was_open |= report.menu_popup_open;

        let canvas_rect = report.regions.canvas_rect_pt.unwrap_or(available_rect);
        self.sync_canvas_viewport(input_window_id, canvas_rect, ppp);
        self.process_canvas_input(ui, input_window_id);

        let post_window_id = self.active_window_id();
        self.sync_window_title(&ctx, post_window_id);

        let will_discard = ctx.will_discard();
        let force_commit = will_discard && self.discarded_carry_passes >= 7;
        if will_discard && !force_commit {
            self.discarded_carry_passes = self.discarded_carry_passes.saturating_add(1);
            self.commit_pending_intents(&ctx, post_window_id, true);
            ctx.request_repaint();
        } else {
            self.discarded_carry_passes = 0;
            self.commit_pending_intents(&ctx, post_window_id, false);
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
        if !will_discard {
            self.flush_deferred_live_requests(&ctx);
        }
        if !self.pending_synthetic_egui_batches.is_empty() || self.pending_live_gesture.is_some() {
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
    }

    fn apply_close_outcome(&mut self, ctx: Option<&egui::Context>, outcome: &DesktopEventOutcome) {
        if matches!(outcome.close_disposition, Some(CloseDisposition::PromptUnsaved { .. })) {
            if let Some(ctx) = ctx {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
            self.close_dialog_open = true;
            return;
        }
        if matches!(outcome.close_disposition, Some(CloseDisposition::Closed)) {
            self.close_dialog_open = false;
            if self.runtime.window_count() > 0 {
                if let Some(ctx) = ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
                self.close_confirmed = false;
                self.on_active_window_switched();
            } else {
                self.close_confirmed = true;
                if let Some(ctx) = ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn handle_close_request(&mut self, ctx: &egui::Context, window_id: DesktopWindowId) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.close_confirmed {
            return;
        }
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

    fn execute_menu_command(&mut self, ctx: Option<&egui::Context>, window_id: DesktopWindowId, command_id: &str) {
        self.execute_menu_command_with_params(ctx, window_id, command_id, &serde_json::json!({}));
    }

    fn execute_menu_command_with_params(
        &mut self,
        ctx: Option<&egui::Context>,
        window_id: DesktopWindowId,
        command_id: &str,
        params: &serde_json::Value,
    ) {
        self.status_banner = None;
        if command_id == "app.preferences" {
            if let Ok(win) = self.runtime.window_mut(window_id) {
                let shortcuts = win.app().shortcuts.clone();
                win.app_mut().egui_views.app_modals.open_preferences(&shortcuts, None);
            }
            return;
        }
        if let Some(native_result) = self.try_handle_native_file_command_with_params(window_id, command_id, params) {
            if let Err(err) = native_result {
                self.status_banner = Some(err);
            }
            return;
        }
        match self.runtime.dispatch(DesktopHostEvent::MenuAction { window_id, command_id: command_id.to_string() }) {
            Ok(outcome) => {
                if outcome.close_disposition.is_some() {
                    self.apply_close_outcome(ctx, &outcome);
                } else if outcome.opened_window_id.is_some() {
                    self.on_active_window_switched();
                }
                if outcome.command_id.as_deref().is_some_and(|c| matches!(c, "edit.copy" | "edit.cut")) {
                    self.apply_clipboard_outcome_status(&outcome);
                    if let Some(ctx_ref) = ctx {
                        self.maybe_sync_plain_text_fallback(ctx_ref, &outcome);
                    }
                }
            }
            Err(err) => {
                self.status_banner = Some(err);
            }
        }
    }

    fn try_handle_native_file_command_with_params(
        &mut self,
        window_id: DesktopWindowId,
        command_id: &str,
        params: &serde_json::Value,
    ) -> Option<Result<(), String>> {
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
        Some(Ok(()))
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
        let (events, raw_hovered, raw_dropped) = ui.ctx().input(|i| (i.events.clone(), i.raw.hovered_files.clone(), i.raw.dropped_files.clone()));

        self.dispatch_file_dnd(window_id, &raw_hovered, &raw_dropped);
        self.dispatch_egui_events(ui.ctx(), window_id, &events);
    }

    fn dispatch_file_dnd(&mut self, window_id: DesktopWindowId, hovered: &[egui::HoveredFile], dropped: &[egui::DroppedFileHandle]) {
        let hovered_paths: Vec<PathBuf> = hovered.iter().filter_map(|f| f.path.clone()).collect();
        if !hovered_paths.is_empty() {
            self.hovered_paths_active = true;
            let _ = self.runtime.dispatch(DesktopHostEvent::FileHovered { window_id, paths: hovered_paths });
        } else if self.hovered_paths_active {
            self.hovered_paths_active = false;
            let _ = self.runtime.dispatch(DesktopHostEvent::FileHoverCancelled { window_id });
        }

        for file in dropped {
            self.hovered_paths_active = false;
            match self.runtime.dispatch(DesktopHostEvent::FileDropped { window_id, path: file.path().to_path_buf() }) {
                Ok(outcome) => {
                    if outcome.opened_window_id.is_some() {
                        self.on_active_window_switched();
                    }
                }
                Err(err) => {
                    self.status_banner = Some(err);
                }
            }
        }
    }

    fn dispatch_egui_events(&mut self, ctx: &egui::Context, window_id: DesktopWindowId, events: &[egui::Event]) {
        let popup_open = self.menu_popup_was_open || egui::Popup::is_any_open(ctx);

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
                self.dispatch_key_event(ctx, window_id, *key, *pressed, *modifiers);
            }
        }
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

    fn dispatch_key_event(&mut self, ctx: &egui::Context, window_id: DesktopWindowId, key: egui::Key, pressed: bool, modifiers: egui::Modifiers) {
        let Some(key_name) = egui_key_to_name(key) else {
            return;
        };
        let dialog_inactive = !self.close_dialog_open
            && self.runtime.window(window_id).is_ok_and(|w| {
                let app = w.app();
                app.canvas_dialogs.active_dialog == ActiveDialog::None
                    && !app.nav.welcome.open
                    && !app.grid.import_wizard.open
                    && !app.grid.variables_dialog.open
            });
        let primary_cmd = modifiers.command || modifiers.mac_cmd || modifiers.ctrl;
        if pressed && primary_cmd && !modifiers.alt && !ctx.text_edit_focused() {
            if key == egui::Key::Plus || key == egui::Key::Equals {
                self.apply_ui_scale_adjustment(ctx, window_id, UiScaleAdjust::In);
                return;
            }
            if key == egui::Key::Minus {
                self.apply_ui_scale_adjustment(ctx, window_id, UiScaleAdjust::Out);
                return;
            }
            if key == egui::Key::Num0 && !modifiers.shift {
                self.apply_ui_scale_adjustment(ctx, window_id, UiScaleAdjust::Reset);
                return;
            }
        }
        if pressed && dialog_inactive && primary_cmd && !ctx.text_edit_focused() {
            if !modifiers.alt {
                if key == egui::Key::O && !modifiers.shift {
                    self.execute_menu_command(Some(ctx), window_id, "project.open");
                    return;
                }
                if key == egui::Key::S {
                    let cmd = if modifiers.shift { "project.saveAs" } else { "project.save" };
                    self.execute_menu_command(Some(ctx), window_id, cmd);
                    return;
                }
            }
            let routed_cmd =
                self.runtime.window(window_id).ok().and_then(|w| match_routed_desktop_shortcut(w.app(), &key_name, to_key_modifiers(modifiers)));
            if let Some(cmd) = routed_cmd {
                let already_queued_edit_params = cmd == "ui.editParams.open"
                    && self.pending_intents.intents().iter().any(|intent| {
                        matches!(intent, curvebook_ui_egui::egui_shell::UiIntent::UiCommand { command_id, .. } if command_id == "ui.editParams.open")
                    });
                if !already_queued_edit_params
                    && (menu_action_triggers_native_file_dialog(&cmd)
                        || matches!(
                            cmd.as_str(),
                            "window.close" | "file.close" | "window.new" | "file.newWindow" | "ui.editParams.open" | "ui.analyze.open"
                        ))
                {
                    self.execute_menu_command(Some(ctx), window_id, &cmd);
                }
            }
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveGestureDispatch {
    ImmediateReply,
    EguiQueued,
}

fn is_pointer_gesture_request(req: &LiveControlRequest) -> bool {
    if !req.conn_authenticated || classify_control_frame(&req.raw_bytes) != ControlFrameKind::Command {
        return false;
    }
    let Ok(text) = std::str::from_utf8(&req.raw_bytes) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<ControlRequest>(text.trim()) else {
        return false;
    };
    parsed.method == "ui.pointer_gesture"
}

fn parse_egui_modifiers_param(val: Option<&serde_json::Value>) -> egui::Modifiers {
    let Some(obj) = val.and_then(serde_json::Value::as_object) else {
        return egui::Modifiers::NONE;
    };
    let flag = |k: &str| obj.get(k).and_then(serde_json::Value::as_bool).unwrap_or(false);
    let ctrl = flag("ctrl");
    let meta = flag("meta");
    egui::Modifiers { alt: flag("alt"), ctrl, shift: flag("shift"), mac_cmd: meta, command: ctrl || meta || flag("command") }
}

fn build_synthetic_egui_gesture_batches(
    action: &str,
    pos: egui::Pos2,
    params: &serde_json::Value,
    modifiers: egui::Modifiers,
) -> Vec<Vec<egui::Event>> {
    match action {
        "hover" => vec![vec![egui::Event::PointerMoved(pos)]],
        "scroll" | "wheel" => {
            #[allow(clippy::cast_possible_truncation)]
            let dx = params.get("delta_x").and_then(serde_json::Value::as_f64).unwrap_or(0.0) as f32;
            #[allow(clippy::cast_possible_truncation)]
            let dy = params.get("delta_y").and_then(serde_json::Value::as_f64).unwrap_or(if params.get("delta_x").is_none() { -120.0 } else { 0.0 })
                as f32;
            vec![vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(dx, dy),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers,
                },
            ]]
        }
        "right_click" => vec![
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed: true, modifiers },
            ],
            vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed: false, modifiers }],
        ],
        "double_click" => vec![
            vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers }],
            vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers }],
            vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers }],
            vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers }],
        ],
        "drag" => {
            #[allow(clippy::cast_possible_truncation)]
            let end_x = params.get("end_x").and_then(serde_json::Value::as_f64).map_or(pos.x, |v| v as f32);
            #[allow(clippy::cast_possible_truncation)]
            let end_y = params.get("end_y").and_then(serde_json::Value::as_f64).map_or(pos.y, |v| v as f32);
            let end_pos = egui::pos2(end_x, end_y);
            vec![
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers },
                ],
                vec![egui::Event::PointerMoved(end_pos)],
                vec![egui::Event::PointerButton { pos: end_pos, button: egui::PointerButton::Primary, pressed: false, modifiers }],
            ]
        }
        _ => vec![
            vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers }],
            vec![egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers }],
        ],
    }
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
    if !req.conn_authenticated || classify_control_frame(&req.raw_bytes) != ControlFrameKind::Command {
        return false;
    }
    let Ok(text) = std::str::from_utf8(&req.raw_bytes) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<ControlRequest>(text.trim()) else {
        return false;
    };
    matches!(parsed.method.as_str(), "ui.inspect" | "ui.screenshot" | "ui.pointer_gesture")
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
        ctrl: modifiers.ctrl || modifiers.command || modifiers.mac_cmd,
        alt: modifiers.alt,
        meta: modifiers.command || modifiers.mac_cmd,
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
        ..Default::default()
    };

    let font_bytes = extra_ttf;
    eframe::run_native(
        "CurveBook",
        native_options,
        Box::new(move |cc| {
            let fonts = build_egui_font_definitions(font_bytes.as_deref());
            cc.egui_ctx.set_fonts(fonts);
            let mut app = CurveBookNativeApp::new(runtime, primary_window_id, Some(on_exit_cleanup));
            #[cfg(target_os = "macos")]
            app.attach_macos_menus_on_windowed_startup();
            if let Some(ctrl) = control {
                let _ = ctrl.egui_ctx_cell.set(cc.egui_ctx.clone());
                cc.egui_ctx.request_repaint();
                app = app.with_live_control(ctrl);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|err| format!("failed to start native desktop window: {err}"))
}

#[cfg(test)]
mod tests {
    use curvebook_doc::SheetId;
    use curvebook_ui_egui::egui_shell::UiBackend;
    use curvebook_ui_egui::interaction::ViewportMetrics;
    use curvebook_ui_egui::{BETA_ACTIVE_LOCALES, locale_picker_label, normalize_beta_gui_locale};
    use serde_json::json;

    use super::*;
    use crate::desktop_host::DesktopHostConfig;

    fn test_runtime(viewport: ViewportMetrics) -> Result<(DesktopHostRuntime, DesktopWindowId), String> {
        let mut runtime = DesktopHostRuntime::new(DesktopHostConfig {
            in_window_menus: true,
            new_instance: true,
            gui_only: false,
            ui_backend: UiBackend::Egui,
            locale_override: None,
            theme_override: None,
            recovery_root: None,
            initial_viewport: viewport,
        });
        let outcome = runtime.dispatch(DesktopHostEvent::WindowOpened { initial_file: None, viewport })?;
        let id = outcome.window_id.ok_or_else(|| String::from("missing opened window id"))?;
        Ok((runtime, id))
    }

    #[test]
    fn egui_headless_frame_with_built_fonts_succeeds() -> Result<(), String> {
        let ctx = egui::Context::default();
        ctx.set_fonts(build_egui_font_definitions(None));

        let (runtime, id) = test_runtime(ViewportMetrics::new(960.0, 640.0, 1.0))?;
        let mut app = CurveBookNativeApp::new(runtime, id, None);

        let raw_input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(960.0, 640.0))), ..Default::default() };

        let mut output = ctx.run_ui(raw_input, |ui| {
            app.step_ui(ui);
        });
        output.textures_delta.clear();

        assert!(!output.shapes.is_empty());
        assert!(app.last_frame_size[0] > 0 && app.last_frame_size[1] > 0);
        Ok(())
    }

    #[test]
    fn inline_editor_42_key_plus_text_does_not_duplicate() -> Result<(), String> {
        let (mut runtime, id) = test_runtime(ViewportMetrics::new(960.0, 640.0, 1.0))?;
        {
            let win = runtime.window_mut(id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-inline", "kind": "column", "cols": 1, "rows": 3 }))?;
        }
        let mut app = CurveBookNativeApp::new(runtime, id, None);
        let ctx = egui::Context::default();
        ctx.set_fonts(build_egui_font_definitions(None));

        let step = |native: &mut CurveBookNativeApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 640.0))),
                    events,
                    ..Default::default()
                },
                |ui| native.step_ui(ui),
            );
            out.textures_delta.clear();
        };

        for _ in 0..2 {
            step(&mut app, Vec::new());
        }
        let cell_center = app.inspect_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing grid.cell.0.0.0")?.clipped_rect.center();
        step(
            &mut app,
            vec![
                egui::Event::PointerMoved(cell_center),
                egui::Event::PointerButton {
                    pos: cell_center,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: cell_center,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: cell_center,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos: cell_center,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        step(&mut app, Vec::new());
        step(
            &mut app,
            vec![
                egui::Event::Key { key: egui::Key::Num4, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
                egui::Event::Text(String::from("4")),
                egui::Event::Key { key: egui::Key::Num2, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
                egui::Event::Text(String::from("2")),
                egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
            ],
        );
        step(&mut app, Vec::new());

        let win = app.runtime.window(id)?;
        let ds = win.app().document().data_sheet(&SheetId::new("ds-inline")).ok_or("missing ds-inline")?;
        assert_eq!(ds.cell(curvebook_doc::CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(42.0));
        Ok(())
    }

    #[test]
    fn hidpi_canvas_resize_and_relative_pointer_offset() -> Result<(), String> {
        let (runtime, id) = test_runtime(ViewportMetrics::new(400.0, 300.0, 1.0))?;
        let mut app = CurveBookNativeApp::new(runtime, id, None);
        let ctx = egui::Context::default();
        ctx.set_fonts(build_egui_font_definitions(None));
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 500.0));

        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(rect),
                viewports: [(egui::ViewportId::ROOT, egui::ViewportInfo { native_pixels_per_point: Some(2.0), ..Default::default() })]
                    .into_iter()
                    .collect(),
                events: vec![egui::Event::PointerMoved(egui::pos2(120.0, 74.0))],
                ..Default::default()
            },
            |ui| app.step_ui(ui),
        );
        out.textures_delta.clear();

        let win = app.runtime.window(id)?;
        assert_eq!(win.viewport().scale_factor, 2.0);
        assert!(app.last_frame_size[0] > 0 && app.last_frame_size[1] > 0);
        assert!(win.viewport().logical_width > 0.0 && win.viewport().logical_width <= 800.0);
        assert!(win.viewport().logical_height > 0.0 && win.viewport().logical_height <= 500.0);
        Ok(())
    }

    #[test]
    fn unsaved_close_request_opens_modal_and_cancels_os_close() -> Result<(), String> {
        let (mut runtime, id) = test_runtime(ViewportMetrics::new(960.0, 640.0, 1.0))?;
        {
            let win = runtime.window_mut(id)?;
            win.execute_and_journal("table.importCsv", json!({ "csv": "X,Y\n1,2\n" }))?;
        }
        let mut app = CurveBookNativeApp::new(runtime, id, None);
        let ctx = egui::Context::default();
        let mut raw_input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(960.0, 640.0))), ..Default::default() };
        raw_input.viewports.entry(egui::ViewportId::ROOT).or_default().events.push(egui::ViewportEvent::Close);

        let mut output = ctx.run_ui(raw_input, |ui| {
            app.handle_close_request(ui.ctx(), id);
        });
        output.textures_delta.clear();

        assert!(app.close_dialog_open);
        assert!(!app.close_confirmed);
        let cancel_sent =
            output.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(cancel_sent);
        Ok(())
    }

    #[derive(Debug)]
    struct TestGuiClipboard {
        fail_writes: bool,
        available: bool,
        writes_text: curvebook_platform::sync::RwLock<Vec<String>>,
        writes_bundle_count: std::sync::atomic::AtomicUsize,
    }

    impl TestGuiClipboard {
        fn new(available: bool, fail_writes: bool) -> Self {
            Self {
                fail_writes,
                available,
                writes_text: curvebook_platform::sync::RwLock::named("gui_window.test_clipboard", Vec::new()),
                writes_bundle_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl crate::desktop_host::OsClipboardAdapter for TestGuiClipboard {
        fn write_text(&self, text: &str) -> Result<curvebook_ui_egui::OsAdapterStatus, String> {
            if self.fail_writes {
                return Err(String::from("pasteboard write denied"));
            }
            if let Ok(mut guard) = self.writes_text.write() {
                guard.push(text.to_owned());
            }
            Ok(curvebook_ui_egui::OsAdapterStatus {
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
            })
        }

        fn write_bundle(&self, _bundle: &curvebook_ui_egui::ClipboardBundle) -> Result<curvebook_ui_egui::OsAdapterStatus, String> {
            if self.fail_writes {
                return Err(String::from("bundle write denied"));
            }
            self.writes_bundle_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(curvebook_ui_egui::OsAdapterStatus {
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
            })
        }

        fn read_text(&self) -> Result<curvebook_ui_egui::OsAdapterStatus, String> {
            Ok(curvebook_ui_egui::OsAdapterStatus::unavailable("mock", "empty"))
        }
    }

    #[test]
    fn r2_s11_07_graph_copy_skips_ctx_copy_text_surfaces_errors_and_cut_clears_selection() -> Result<(), String> {
        let (mut runtime, id) = test_runtime(ViewportMetrics::new(960.0, 640.0, 1.0))?;
        runtime.set_clipboard_adapter(Box::new(TestGuiClipboard::new(true, false)));
        let graph_id = {
            let win = runtime.window_mut(id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-1", "kind": "column", "cols": 1, "rows": 3, "auto_graph": true }))?;
            win.execute_and_journal("table.setCell", json!({ "sheet_id": "ds-1", "row": 0, "col": 0, "value": 11.0 }))?;
            win.execute_and_journal("table.setCell", json!({ "sheet_id": "ds-1", "row": 1, "col": 0, "value": 22.0 }))?;
            let graph_id = win.app().document().graph_sheets.first().ok_or("missing graph")?.id.clone();
            win.execute_and_journal("sheet.select", json!({ "sheet_id": graph_id.as_str() }))?;
            graph_id
        };
        let mut app = CurveBookNativeApp::new(runtime, id, None);
        let ctx = egui::Context::default();
        ctx.set_fonts(build_egui_font_definitions(None));

        let run_events = |native: &mut CurveBookNativeApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 640.0))),
                    events,
                    ..Default::default()
                },
                |ui| native.step_ui(ui),
            );
            out.textures_delta.clear();
            out
        };

        // 1. Copy on GraphSheet must NOT emit OutputCommand::CopyText (preserves PDF/PNG/HTML bundle).
        run_events(&mut app, Vec::new());
        let out_graph = run_events(&mut app, vec![egui::Event::Copy]);
        assert!(app.runtime.clipboard_bundle().is_some());
        assert!(app.runtime.clipboard_text().is_none());
        let emitted_copy_text = out_graph.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_)));
        assert!(!emitted_copy_text, "graph bundle copy must NOT overwrite OS multi-flavor clipboard with ctx.copy_text");

        // 2. Cut on DataSheet clears ONLY the selected cell, leaving unselected rows intact.
        {
            let win = app.runtime.window_mut(id)?;
            win.execute_and_journal("sheet.select", json!({ "sheet_id": "ds-1" }))?;
        }
        run_events(&mut app, Vec::new());
        let cell_pt = app.inspect_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing grid.cell.0.0.0")?.clipped_rect.center();
        run_events(
            &mut app,
            vec![
                egui::Event::PointerMoved(cell_pt),
                egui::Event::PointerButton { pos: cell_pt, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                egui::Event::PointerButton { pos: cell_pt, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
            ],
        );
        run_events(&mut app, vec![egui::Event::Cut]);
        let ds = app.runtime.window(id)?.app().document().data_sheet(&SheetId::new("ds-1")).ok_or("missing ds-1")?;
        assert_eq!(ds.cell(curvebook_doc::CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), None);
        assert_eq!(ds.cell(curvebook_doc::CellCoord::new(1, 0, 0)).and_then(|c| c.active_number()), Some(22.0));

        // 3. Failing OS clipboard surfaced in status_banner.
        app.runtime.window_mut(id)?.execute_and_journal("sheet.select", json!({ "sheet_id": graph_id.as_str() }))?;
        app.runtime.set_clipboard_adapter(Box::new(TestGuiClipboard::new(true, true)));
        run_events(&mut app, Vec::new());
        run_events(&mut app, vec![egui::Event::Copy]);
        assert!(app.status_banner().is_some_and(|s| s.contains("denied")), "banner: {:?}", app.status_banner());
        Ok(())
    }

    #[test]
    fn r2_s11_02_cmd_s_writes_archive_and_unpacked_dir_and_respects_all_modals() -> Result<(), String> {
        let base_dir = std::env::temp_dir().join(format!("cb-gui-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base_dir);
        std::fs::create_dir_all(&base_dir).map_err(|e| e.to_string())?;
        let archive_path = base_dir.join("project.curvebook");
        let unpacked_path = base_dir.join("project.curvebook.d");

        let (mut runtime, id) = test_runtime(ViewportMetrics::new(960.0, 640.0, 1.0))?;
        {
            let win = runtime.window_mut(id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "sv-1", "kind": "column", "cols": 1, "rows": 2 }))?;
            win.execute_and_journal("project.save", json!({ "path": archive_path.to_string_lossy(), "unpacked": false }))?;
            win.app_mut().session_mut().set_file_path(Some(archive_path.clone()), true);
            win.execute_and_journal("table.setCell", json!({ "sheet_id": "sv-1", "row": 0, "col": 0, "value": 77.25 }))?;
        }

        let mut app = CurveBookNativeApp::new(runtime, id, None);
        let ctx = egui::Context::default();
        let cmd_s = egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { command: true, ..egui::Modifiers::NONE },
        };

        // Blocked while import_wizard / variables_dialog / close_dialog_open are open.
        app.runtime.window_mut(id)?.app_mut().grid.import_wizard.open = true;
        app.dispatch_egui_events(&ctx, id, std::slice::from_ref(&cmd_s));
        assert!(app.runtime.window(id)?.has_unsaved_changes(), "Cmd+S must be blocked while ImportWizard is open");
        app.runtime.window_mut(id)?.app_mut().grid.import_wizard.open = false;

        app.runtime.window_mut(id)?.app_mut().grid.variables_dialog.open = true;
        app.dispatch_egui_events(&ctx, id, std::slice::from_ref(&cmd_s));
        assert!(app.runtime.window(id)?.has_unsaved_changes(), "Cmd+S must be blocked while VariablesDialog is open");
        app.runtime.window_mut(id)?.app_mut().grid.variables_dialog.open = false;

        // With all modals closed, Cmd+S saves archive to disk cleanly.
        app.dispatch_egui_events(&ctx, id, std::slice::from_ref(&cmd_s));
        assert!(!app.runtime.window(id)?.has_unsaved_changes(), "Cmd+S must persist dirty archive");
        let reopened = curvebook_format::open_project_file(&archive_path).map_err(|e| e.to_string())?;
        assert_eq!(
            reopened.data_sheet(&SheetId::new("sv-1")).and_then(|s| s.cell(curvebook_doc::CellCoord::new(0, 0, 0))).and_then(|c| c.active_number()),
            Some(77.25)
        );

        // Save as unpacked `.curvebook.d`, dirty again, and confirm Cmd+S saves back as unpacked dir (`unpacked: true`).
        {
            let win = app.runtime.window_mut(id)?;
            win.execute_and_journal("project.save", json!({ "path": unpacked_path.to_string_lossy(), "unpacked": true }))?;
            win.app_mut().session_mut().set_file_path(Some(unpacked_path.clone()), true);
            win.execute_and_journal("table.setCell", json!({ "sheet_id": "sv-1", "row": 1, "col": 0, "value": 88.5 }))?;
        }
        app.dispatch_egui_events(&ctx, id, &[cmd_s]);
        assert!(!app.runtime.window(id)?.has_unsaved_changes(), "Cmd+S must persist dirty unpacked dir");
        let reopened_dir = curvebook_format::open_project_unpacked_dir(&unpacked_path).map_err(|e| e.to_string())?;
        assert_eq!(
            reopened_dir
                .data_sheet(&SheetId::new("sv-1"))
                .and_then(|s| s.cell(curvebook_doc::CellCoord::new(1, 0, 0)))
                .and_then(|c| c.active_number()),
            Some(88.5)
        );

        let _ = std::fs::remove_dir_all(&base_dir);
        Ok(())
    }

    #[cfg(feature = "accessibility")]
    #[test]
    fn r10_w5_accesskit_tree_sync_preserves_single_parent_and_focus() -> Result<(), String> {
        let (mut runtime, id) = test_runtime(ViewportMetrics::new(1024.0, 640.0, 1.0))?;
        {
            let win = runtime.window_mut(id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "ak-1", "kind": "column", "cols": 2, "rows": 3 }))?;
        }
        let mut app = CurveBookNativeApp::new(runtime, id, None);
        let ctx = egui::Context::default();
        ctx.set_fonts(build_egui_font_definitions(None));
        ctx.enable_accesskit();
        let raw_input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1024.0, 640.0))), ..Default::default() };

        let mut last_update = None;
        for _ in 0..3 {
            let mut output = ctx.run_ui(raw_input.clone(), |ui| {
                app.step_ui(ui);
            });
            output.textures_delta.clear();
            last_update = output.platform_output.accesskit_update;
        }
        let update = last_update.ok_or_else(|| String::from("expected accesskit_update when accesskit is enabled"))?;
        assert!(!update.nodes.is_empty(), "expected non-empty accesskit nodes");

        let mut parent_counts: std::collections::BTreeMap<egui::accesskit::NodeId, usize> = std::collections::BTreeMap::new();
        for (_, node) in &update.nodes {
            for &child_id in node.children() {
                *parent_counts.entry(child_id).or_default() += 1;
            }
        }
        for (nid, count) in &parent_counts {
            assert_eq!(*count, 1, "accesskit node {nid:?} has {count} parents (expected 1)");
        }
        let analyze_entry = app.inspect_meta().find_by_widget_id("toolbar.analyze").ok_or("missing toolbar.analyze in inspect_meta")?.clone();
        assert!(
            update.nodes.iter().any(|(nid, _)| *nid == analyze_entry.accesskit_id),
            "expected toolbar.analyze accesskit node in AccessKit TreeUpdate"
        );

        // Dispatch AccessKit Click action on `toolbar.analyze` -> opens AnalyzeSheet dialog.
        let req = egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: analyze_entry.accesskit_id,
            data: None,
        };
        let mut click_in = raw_input.clone();
        click_in.events.push(egui::Event::AccessKitActionRequest(req));
        let mut out_click = ctx.run_ui(click_in, |ui| app.step_ui(ui));
        out_click.textures_delta.clear();
        let mut out_settle = ctx.run_ui(raw_input, |ui| app.step_ui(ui));
        out_settle.textures_delta.clear();
        assert!(
            matches!(app.runtime.window(id)?.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }),
            "expected AccessKit Click on toolbar.analyze to open AnalyzeSheet"
        );
        Ok(())
    }

    #[test]
    fn language_picker_labels_mark_15_non_english_locales_as_draft() {
        use curvebook_ui_egui::LOCALES;
        assert_eq!(LOCALES.len(), 16);
        assert_eq!(locale_picker_label("en"), "English (en)");
        let draft_labels: Vec<String> = LOCALES.iter().skip(1).map(|&loc| locale_picker_label(loc)).collect();
        assert_eq!(draft_labels.len(), 15);
        for (&loc, label) in LOCALES.iter().skip(1).zip(&draft_labels) {
            assert!(label.ends_with(" [Draft]"), "expected `[Draft]` suffix on `{loc}`, got `{label}`");
            assert!(label.contains(&format!("({loc})")), "expected locale tag `({loc})` in `{label}`");
        }

        assert_eq!(BETA_ACTIVE_LOCALES.len(), 14);
        assert_eq!(BETA_ACTIVE_LOCALES.first().copied(), Some("en"));
        let beta_drafts: Vec<String> = BETA_ACTIVE_LOCALES.iter().skip(1).map(|&loc| locale_picker_label(loc)).collect();
        assert_eq!(beta_drafts.len(), 13);
        for (&loc, label) in BETA_ACTIVE_LOCALES.iter().skip(1).zip(&beta_drafts) {
            assert!(!curvebook_ui_egui::i18n::is_rtl_locale(loc), "RTL locale `{loc}` must be excluded from BETA_ACTIVE_LOCALES");
            assert!(label.ends_with(" [Draft]"), "expected `[Draft]` suffix on `{loc}`, got `{label}`");
        }
        assert_eq!(normalize_beta_gui_locale("ar"), "en");
        assert_eq!(normalize_beta_gui_locale("he"), "en");
        assert_eq!(normalize_beta_gui_locale("fa"), "en");
        assert_eq!(normalize_beta_gui_locale("ur"), "en");
        assert_eq!(normalize_beta_gui_locale("pt"), "pt-BR");
    }

    #[test]
    fn r15_5_gui_window_pure_egui_live_gesture_analyze_and_edit_params() -> Result<(), String> {
        let mut runtime = DesktopHostRuntime::new(DesktopHostConfig {
            in_window_menus: true,
            new_instance: true,
            gui_only: true,
            ui_backend: UiBackend::Egui,
            locale_override: None,
            theme_override: None,
            recovery_root: None,
            initial_viewport: ViewportMetrics::new(1024.0, 640.0, 1.0),
        });
        let opened = runtime.dispatch(DesktopHostEvent::WindowOpened { initial_file: None, viewport: ViewportMetrics::new(1024.0, 640.0, 1.0) })?;
        let id = opened.window_id.ok_or_else(|| String::from("missing window id"))?;
        {
            let win = runtime.window_mut(id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "col-h1", "kind": "column", "cols": 3, "rows": 4 }))?;
            for col in 0..3 {
                for row in 0..4 {
                    let val = 10.0 + (col as f64) * 3.0 + (row as f64) * 0.25;
                    win.execute_and_journal("table.setCell", json!({ "sheet_id": "col-h1", "row": row, "col": col, "value": val }))?;
                }
            }
        }

        let (req_tx, req_rx) = mpsc::channel();
        let ctrl = LiveControlChannel {
            request_rx: req_rx,
            egui_ctx_cell: Arc::new(OnceLock::new()),
            shutdown: Arc::new(AtomicBool::new(false)),
            post_hook: None,
        };
        let mut app = CurveBookNativeApp::new(runtime, id, None).with_live_control(ctrl);
        let ctx = egui::Context::default();
        ctx.set_fonts(build_egui_font_definitions(None));

        let step_once = |native: &mut CurveBookNativeApp, extra: Vec<egui::Event>| {
            let mut raw_input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 640.0))),
                events: extra,
                ..Default::default()
            };
            native.drain_synthetic_events_into(&mut raw_input);
            let mut out = ctx.run_ui(raw_input, |ui| native.step_ui(ui));
            out.textures_delta.clear();
        };

        // Settle initial Column grid frame so inspect_meta captures `toolbar.analyze`.
        for _ in 0..3 {
            step_once(&mut app, Vec::new());
        }
        assert!(app.inspect_meta().find_by_widget_id("toolbar.analyze").is_some(), "expected settled toolbar.analyze");

        let auth = ControlAuth::from_token("test-token-r15-5").map_err(|e| e.to_string())?;
        let send_ctrl = |native: &mut CurveBookNativeApp, req_json: Vec<u8>| -> Result<serde_json::Value, String> {
            let (tx, rx) = mpsc::channel();
            req_tx
                .send(LiveControlRequest { raw_bytes: req_json, auth: auth.clone(), conn_authenticated: true, reply_tx: tx })
                .map_err(|e| e.to_string())?;
            for _ in 0..8 {
                step_once(native, Vec::new());
                if let Ok(res) = rx.try_recv() {
                    let out = res?;
                    let txt = String::from_utf8_lossy(&out.response_bytes);
                    return serde_json::from_str(&txt).map_err(|e| format!("invalid control reply `{txt}`: {e}"));
                }
            }
            Err(String::from("timed out waiting for LiveControlRequest reply"))
        };

        let gesture_val =
            send_ctrl(&mut app, br#"{"id":"g1","method":"ui.pointer_gesture","params":{"action":"click","widget_id":"toolbar.analyze"}}"#.to_vec())?;
        assert_eq!(gesture_val.pointer("/result/egui_dispatched").and_then(serde_json::Value::as_bool), Some(true), "got {gesture_val}");
        step_once(&mut app, Vec::new());

        // Native egui AnalyzeSheet modal rendered cleanly over Column grid.
        assert!(matches!(app.runtime().window(id)?.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));

        // Select `analyze.entry.stats.one_way_anova` via live `egui` widget pointer click & confirm!
        let sel_val = send_ctrl(
            &mut app,
            br#"{"id":"sel","method":"ui.pointer_gesture","params":{"action":"click","widget_id":"analyze.entry.stats.one_way_anova"}}"#.to_vec(),
        )?;
        assert_eq!(sel_val.get("ok").and_then(serde_json::Value::as_bool), Some(true), "select one_way_anova failed: {sel_val}");

        let ok_val =
            send_ctrl(&mut app, br#"{"id":"run","method":"ui.answer_dialog","params":{"dialog_id":"analyze_sheet","action":"ok"}}"#.to_vec())?;
        assert_eq!(ok_val.get("ok").and_then(serde_json::Value::as_bool), Some(true), "confirm analyze_sheet failed: {ok_val}");
        step_once(&mut app, Vec::new());
        let active_rs = app.runtime().window(id)?.app().document().active_sheet_id.clone().ok_or("expected active ResultSheet")?;
        assert!(app.runtime().window(id)?.app().document().result_sheet(&active_rs).is_some());

        // Press Cmd+T on ResultSheet -> opens `egui` One-Way ANOVA modal!
        step_once(
            &mut app,
            vec![
                egui::Event::Key { key: egui::Key::T, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND },
                egui::Event::Key { key: egui::Key::T, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::COMMAND },
            ],
        );
        step_once(&mut app, Vec::new());
        assert!(
            matches!(app.runtime().window(id)?.app().canvas_dialogs.active_dialog, ActiveDialog::EditAnalysisParameters { .. }),
            "Cmd+T on ResultSheet must open egui EditAnalysisParameters modal"
        );
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
        assert_eq!(icon.rgba[3], 0, "outer corner should be transparent");
        assert!(icon.rgba.as_chunks::<4>().0.iter().any(|px| px[3] == 255), "icon must contain opaque logo pixels");
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

        assert!(!super::is_macos_app_bundle_exe(&exe), "bundle without curvebook.icns must fall back to runtime icon");
        std::fs::write(resources.join("curvebook.icns"), b"icns")?;
        assert!(super::is_macos_app_bundle_exe(&exe), "bundle with curvebook.icns must keep native CFBundleIconFile");
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
        let pref_calls_hook = Arc::clone(&pref_calls);
        runtime.set_preferences_persist_hook(move |_| {
            pref_calls_hook.fetch_add(1, Ordering::Relaxed);
        });
        let win_id = runtime.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
        let mut native = CurveBookNativeApp::new(runtime, win_id, None);
        let ctx = egui::Context::default();

        native.pending_intents.push_engine(
            "sheet.createData",
            serde_json::json!({ "sheet_id": "ds-a1-host", "title": "A1 Host", "kind": "column", "auto_graph": false }),
        );
        native.pending_intents.push_engine("sheet.delete", serde_json::json!({ "sheet_id": "missing-sheet" }));
        native.pending_intents.push(UiIntent::SetUiScale(1.25));

        native.commit_pending_intents(&ctx, win_id, false);

        assert!(native.status_banner().is_some(), "failing intent must be surfaced in host status_banner");
        let win = native.runtime().window(win_id)?;
        assert!(win.app().document().data_sheet(&curvebook_doc::SheetId::from("ds-a1-host")).is_some(), "succeeding intent must land");
        assert!(
            win.dirty_commands().iter().any(|c| c == "sheet.createData"),
            "record_ui_mutation_checkpoint must run on partial error: {:?}",
            win.dirty_commands()
        );
        assert!((win.app().ui_scale() - 1.25).abs() < 1e-4, "trailing preference intent must apply");
        assert!(pref_calls.load(Ordering::Relaxed) >= 1, "notify_preferences_changed must still run");

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

        // Build and save a real `.curvebook` archive with a data sheet and graph sheet.
        {
            let mut seed = UiApp::new();
            let _ = seed.execute_command(
                "sheet.createData",
                serde_json::json!({ "sheet_id": "ds-saved", "title": "Saved Assay", "kind": "xy", "data_sets": 1, "rows": 3, "auto_graph": false }),
            )?;
            let _ = seed.execute_command("table.setXCells", serde_json::json!({ "sheet_id": "ds-saved", "values": [1.0, 2.0, 3.0] }))?;
            let _ = seed.execute_command("table.setCells", serde_json::json!({ "sheet_id": "ds-saved", "rows": [[5.0], [10.0], [15.0]] }))?;
            let _ =
                seed.execute_command("graph.create", serde_json::json!({ "sheet_id": "gs-emf", "data_sheet_id": "ds-saved", "title": "EMF Graph" }))?;
            let _ = seed.execute_command("project.save", serde_json::json!({ "path": saved_project.to_string_lossy(), "unpacked": false }))?;
        }
        assert!(saved_project.exists(), "fixture .curvebook archive must exist");

        let adapter = Arc::new(StubNativeFileDialogAdapter::new());
        let mut runtime = DesktopHostRuntime::new(DesktopHostConfig::default());
        runtime.set_file_dialog_adapter(Arc::clone(&adapter) as Arc<dyn NativeFileDialogAdapter>);
        let win_id = runtime.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
        let mut native = CurveBookNativeApp::new(runtime, win_id, None);
        let ctx = egui::Context::default();

        // 1. Clean window: Welcome Hub `project.open` with explicit `path` loads directly without opening OS picker.
        native.pending_intents.push_engine("project.open", serde_json::json!({ "path": saved_project.to_string_lossy() }));
        native.commit_pending_intents(&ctx, win_id, false);
        assert!(native.status_banner().is_none(), "clean open-by-path failed: {:?}", native.status_banner());
        assert!(adapter.requests().is_empty(), "open-by-path must NOT invoke the OS file picker");
        assert!(
            native.runtime().window(win_id)?.app().document().data_sheet(&curvebook_doc::SheetId::from("ds-saved")).is_some(),
            "saved project must load into clean window"
        );
        assert_eq!(native.runtime().window_count(), 1);

        // 2. Dirty window: mutate a cell so `has_unsaved_changes() == true`; opening by path must preserve the dirty
        //    document and open the target project in a new window rather than silently overwriting unsaved work.
        {
            let win = native.runtime_mut().window_mut(win_id)?;
            win.execute_and_journal("table.setCell", serde_json::json!({ "sheet_id": "ds-saved", "row": 0, "col": 0, "value": 999.0 }))?;
            assert!(win.has_unsaved_changes());
        }
        native.pending_intents.push_engine("project.open", serde_json::json!({ "path": saved_project.to_string_lossy() }));
        native.commit_pending_intents(&ctx, win_id, false);
        assert!(native.status_banner().is_none(), "dirty open-by-path failed: {:?}", native.status_banner());
        assert!(adapter.requests().is_empty(), "dirty open-by-path must NOT invoke the OS file picker");
        assert_eq!(native.runtime().window_count(), 2, "dirty window must be preserved while target opens in a new window");
        let orig_cell = native
            .runtime()
            .window(win_id)?
            .app()
            .document()
            .data_sheet(&curvebook_doc::SheetId::from("ds-saved"))
            .and_then(|ds| ds.cell(curvebook_doc::CellCoord::new(0, 0, 0)))
            .ok_or("missing original dirty cell")?;
        assert_eq!(orig_cell.value, curvebook_doc::CellValue::Number(999.0), "dirty window edit must remain intact");

        // 3. Export dialog EMF option routes `graph.exportEmf` through the host SaveFile dialog and writes EMF bytes to disk.
        adapter.push_response("graph.exportEmf", Some(emf_out.clone()));
        native.pending_intents.push_engine("graph.exportEmf", serde_json::json!({ "sheet_id": "gs-emf" }));
        native.commit_pending_intents(&ctx, win_id, false);
        assert!(native.status_banner().is_none(), "EMF host export failed: {:?}", native.status_banner());
        let reqs = adapter.requests();
        assert_eq!(reqs.len(), 1, "expected 1 SaveFile request for graph.exportEmf");
        assert_eq!(reqs[0].command_id, "graph.exportEmf");
        assert_eq!(reqs[0].kind, NativeFileDialogKind::SaveFile);
        let emf_bytes = std::fs::read(&emf_out)?;
        assert!(emf_bytes.len() > 80, "expected non-empty EMF output written to disk, got {} bytes", emf_bytes.len());

        let _ = std::fs::remove_dir_all(&tmp_dir);
        Ok(())
    }
}
