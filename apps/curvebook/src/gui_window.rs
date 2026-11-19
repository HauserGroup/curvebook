#![forbid(unsafe_code)]

//! Interactive `eframe` + `egui` + `wgpu` native desktop window host for CurveBook.

use std::collections::VecDeque;
#[cfg(feature = "accessibility")]
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Instant;

use curvebook_text::{BUNDLED_FONTS, TrueTypeFont, build_minimal_truetype_font};
use curvebook_ui_egui::dialogs_canvas::ActiveDialog;
use curvebook_ui_egui::egui_shell::inspect_bridge::{HybridCoexistenceArbiter, HybridHostTextureFrame, HybridModalSurface};
use curvebook_ui_egui::egui_shell::rasterizer::EguiScreenshotRegions;
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, GeometryStamp, PendingUiIntents, Tokens, UiBackend, inspect_bridge};
use curvebook_ui_egui::interaction::{KeyModifiers, PointerButton};
use curvebook_ui_egui::{
    BETA_ACTIVE_LOCALES, ControlAuth, ControlPostRequestContext, ControlRequest, UiApp, compute_workspace_layout_for_dpi, locale_picker_label,
    normalize_beta_gui_locale,
};
use eframe::egui;

use crate::desktop_host::{
    CloseDisposition, ControlFrameKind, DesktopEventOutcome, DesktopFrameOutput, DesktopHostEvent, DesktopHostRuntime, DesktopMenuChromeState,
    DesktopMenuItem, DesktopWindowId, LiveControlFrameOutput, classify_control_frame, match_routed_desktop_shortcut,
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
const DOUBLE_CLICK_WINDOW_SEC: f64 = 0.40;
const DOUBLE_CLICK_RADIUS_PT: f32 = 5.0;
const MAX_HOST_FONT_BYTES: usize = 64 * 1024 * 1024;

const HOST_PROPORTIONAL_TTF_CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/System/Library/Fonts/Supplemental/Helvetica.ttf",
    "/System/Library/Fonts/SFNS.ttf",
    "/Library/Fonts/Arial.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "C:\\Windows\\Fonts\\arial.ttf",
    "C:\\Windows\\Fonts\\segoeui.ttf",
];

const HOST_MONOSPACE_TTF_CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Courier New.ttf",
    "/System/Library/Fonts/SFNSMono.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
    "C:\\Windows\\Fonts\\consola.ttf",
    "C:\\Windows\\Fonts\\cour.ttf",
];

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

struct CanvasPointerButtonArgs {
    window_id: DesktopWindowId,
    canvas_rect: egui::Rect,
    pos: egui::Pos2,
    button: egui::PointerButton,
    pressed: bool,
    modifiers: egui::Modifiers,
}

/// Build `egui` font definitions from validated single-face SFNT fonts and a deterministic minimal fallback.
#[must_use]
pub fn build_egui_font_definitions(extra_ttf: Option<&[u8]>) -> egui::FontDefinitions {
    let mut defs = egui::FontDefinitions::empty();
    let mut proportional = Vec::new();
    let mut monospace = Vec::new();

    if let Some(bytes) = extra_ttf
        && let Some(key) = insert_valid_sfnt(&mut defs, "cli-ttf-override", bytes)
    {
        proportional.push(key.clone());
        monospace.push(key);
    }

    for &(font_name, font_bytes) in BUNDLED_FONTS {
        if let Some(key) = insert_valid_sfnt(&mut defs, font_name, font_bytes) {
            if font_name.to_ascii_lowercase().contains("mono") {
                monospace.push(key);
            } else {
                proportional.push(key);
            }
        }
    }

    load_host_sfnt_candidates(&mut defs, HOST_PROPORTIONAL_TTF_CANDIDATES, &mut proportional);
    load_host_sfnt_candidates(&mut defs, HOST_MONOSPACE_TTF_CANDIDATES, &mut monospace);

    let minimal_ttf = build_minimal_truetype_font();
    if let Some(key) = insert_valid_sfnt(&mut defs, "curvebook-minimal-fallback", &minimal_ttf) {
        proportional.push(key.clone());
        monospace.push(key);
    } else {
        let name = String::from("curvebook-minimal-fallback");
        defs.font_data.insert(name.clone(), Arc::new(egui::FontData::from_owned(minimal_ttf)));
        proportional.push(name.clone());
        monospace.push(name);
    }

    defs.families.insert(egui::FontFamily::Proportional, proportional);
    defs.families.insert(egui::FontFamily::Monospace, monospace);
    defs
}

fn insert_valid_sfnt(defs: &mut egui::FontDefinitions, name: &str, bytes: &[u8]) -> Option<String> {
    TrueTypeFont::parse(bytes)?;
    let key = name.to_string();
    defs.font_data.insert(key.clone(), Arc::new(egui::FontData::from_owned(bytes.to_vec())));
    Some(key)
}

fn load_host_sfnt_candidates(defs: &mut egui::FontDefinitions, candidates: &[&str], family_list: &mut Vec<String>) {
    for (idx, path) in candidates.iter().enumerate() {
        let Ok(bytes) = curvebook_platform::fs::read_regular_file_bytes(Path::new(path), MAX_HOST_FONT_BYTES) else {
            continue;
        };
        let label = format!("host-font-{idx}-{}", path_stem_hint(path));
        if let Some(key) = insert_valid_sfnt(defs, &label, &bytes) {
            family_list.push(key);
        }
    }
}

fn path_stem_hint(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|s| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect())
        .filter(|s: &String| !s.is_empty())
        .unwrap_or_else(|| String::from("font"))
}

struct DeferredLivePointerGesture {
    req: LiveControlRequest,
    response_payload: serde_json::Value,
}

/// Interactive `eframe::App` wrapping a [`DesktopHostRuntime`] session.
pub struct CurveBookNativeApp {
    runtime: DesktopHostRuntime,
    primary_window_id: DesktopWindowId,
    texture: Option<egui::TextureHandle>,
    legacy_modal_texture: Option<egui::TextureHandle>,
    legacy_modal_rect_pt: Option<egui::Rect>,
    legacy_viewport_origin_pt: egui::Pos2,
    last_active_sheet_id: Option<curvebook_doc::SheetId>,
    last_modal_surface: Option<HybridModalSurface>,
    last_frame_size: [u32; 2],
    last_scale_factor: f32,
    last_title: String,
    last_frame_time: Option<Instant>,
    last_click_time: Option<Instant>,
    last_click_pos: Option<egui::Pos2>,
    click_count: u8,
    pointer_captured: bool,
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
    deferred_live_requests: Vec<LiveControlRequest>,
    pending_synthetic_egui_batches: VecDeque<Vec<egui::Event>>,
    pending_live_gesture: Option<DeferredLivePointerGesture>,
    menu_popup_was_open: bool,
    #[cfg(feature = "accessibility")]
    a11y_id_map: BTreeMap<egui::accesskit::NodeId, String>,
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
            texture: None,
            legacy_modal_texture: None,
            legacy_modal_rect_pt: None,
            legacy_viewport_origin_pt: egui::Pos2::ZERO,
            last_active_sheet_id: None,
            last_modal_surface: None,
            last_frame_size: [0, 0],
            last_scale_factor: 0.0,
            last_title: String::new(),
            last_frame_time: None,
            last_click_time: None,
            last_click_pos: None,
            click_count: 1,
            pointer_captured: false,
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
            deferred_live_requests: Vec::new(),
            pending_synthetic_egui_batches: VecDeque::new(),
            pending_live_gesture: None,
            menu_popup_was_open: false,
            #[cfg(feature = "accessibility")]
            a11y_id_map: BTreeMap::new(),
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
        let active_id = self.active_window_id();
        let is_egui_backend = self.runtime.window(active_id).is_ok_and(|w| w.app().ui_backend() == UiBackend::Egui);
        let has_deferred_in_flight =
            !self.deferred_live_requests.is_empty() || !self.pending_synthetic_egui_batches.is_empty() || self.pending_live_gesture.is_some();
        let mut immediate = Vec::new();
        for req in pending {
            if should_defer_to_settled_frame(&req, is_egui_backend)
                || (has_deferred_in_flight && classify_control_frame(&req.raw_bytes) == ControlFrameKind::Command)
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
            let win_id = self.active_window_id();
            let is_egui = self.runtime.window(win_id).is_ok_and(|w| w.app().ui_backend() == UiBackend::Egui);
            if is_egui && is_pointer_gesture_request(&req) {
                match self.try_enqueue_live_pointer_gesture(ctx, win_id, req, &mut post_hook) {
                    LiveGestureDispatch::LegacyRepaint => {
                        self.deferred_live_requests.extend(iter);
                        ctx.request_repaint();
                        break;
                    }
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
        win_id: DesktopWindowId,
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

        let decision = self.runtime.window(win_id).map(|w| HybridCoexistenceArbiter::evaluate(w.app(), self.close_dialog_open)).ok();

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

        let route_legacy = decision.is_some_and(|d| {
            if widget_id.is_some() && meta_entry.is_none() {
                d.modal_surface == HybridModalSurface::LegacyModal || (d.renders_legacy_canvas && d.modal_surface == HybridModalSurface::None)
            } else if let Some(pos) = target_pos {
                let canvas_rect = self.last_regions.canvas_rect_pt.unwrap_or_else(|| ctx.content_rect());
                let popup = self.menu_popup_was_open || egui::Popup::is_any_open(ctx);
                meta_entry.is_none() && HybridCoexistenceArbiter::pointer_routes_to_legacy(&d, canvas_rect, pos, popup, self.legacy_modal_rect_pt)
            } else {
                false
            }
        });
        if route_legacy {
            let res = self.dispatch_single_live_request(&req.raw_bytes, &req.auth, req.conn_authenticated, post_hook);
            let _ = req.reply_tx.send(res);
            return LiveGestureDispatch::LegacyRepaint;
        }

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
        let win_id = self.active_window_id();
        let is_egui = self.runtime.window(win_id).is_ok_and(|w| w.app().ui_backend() == UiBackend::Egui);
        if !is_egui {
            return raw_bytes.to_vec();
        }
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

    fn commit_pending_intents(&mut self, window_id: DesktopWindowId, will_discard: bool) {
        if self.pending_intents.is_empty() {
            return;
        }
        let mut intents = std::mem::take(&mut self.pending_intents);
        let prev_scale = self.runtime.window(window_id).ok().map(|w| w.app().ui_scale());
        let prev_theme = self.runtime.window(window_id).ok().map(|w| w.app().theme.mode);
        let prev_locale = self.runtime.window(window_id).ok().map(|w| w.app().locale.clone());
        if let Ok(win) = self.runtime.window_mut(window_id) {
            if let Err(err) = intents.commit_if_settled(win.app_mut(), will_discard) {
                self.status_banner = Some(err.to_string());
            } else if !will_discard {
                let changed = Some(win.app().ui_scale()) != prev_scale
                    || Some(win.app().theme.mode) != prev_theme
                    || Some(win.app().locale.clone()) != prev_locale;
                if changed {
                    self.runtime.notify_preferences_changed(window_id);
                }
            }
        }
    }

    /// Execute one UI frame (polling live control requests, handling OS events, and rendering).
    pub fn step_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.frame_seq = self.frame_seq.wrapping_add(1);
        self.pending_intents = PendingUiIntents::default();
        let _ = self.poll_live_control(&ctx);
        let initial_window_id = self.active_window_id();

        if let Ok(win) = self.runtime.window(initial_window_id) {
            let tokens = Tokens::from_theme_spec(&win.app().theme, &win.app().locale);
            tokens.apply_to_ctx(&ctx);
        }
        ctx.options_mut(|o| o.zoom_with_keyboard = false);

        self.handle_close_request(&ctx, initial_window_id);
        self.menu_popup_was_open = egui::Popup::is_any_open(&ctx);
        let bar_window_id = self.active_window_id();
        let host_drew_menu_bar = self.render_top_menu_bar(ui, bar_window_id);
        if let Ok(win) = self.runtime.window_mut(bar_window_id) {
            win.app_mut().host_draws_menu_bar = host_drew_menu_bar;
        }

        let input_window_id = self.active_window_id();
        let ui_backend = self.runtime.window(input_window_id).map(|w| w.app().ui_backend()).unwrap_or(UiBackend::Legacy);
        if ui_backend == UiBackend::Egui {
            let ppp = ctx.pixels_per_point().max(1.0);
            let available_legacy_rect = ui.available_rect_before_wrap();
            let full_content_rect = ctx.content_rect();
            let (decision, active_sheet_id) = self
                .runtime
                .window(input_window_id)
                .map(|w| (HybridCoexistenceArbiter::evaluate(w.app(), self.close_dialog_open), w.app().document().active_sheet_id.clone()))
                .unwrap_or_else(|_| (HybridCoexistenceArbiter::evaluate(&UiApp::new(), self.close_dialog_open), None));

            if active_sheet_id != self.last_active_sheet_id || Some(decision.modal_surface) != self.last_modal_surface {
                self.pointer_captured = false;
                self.last_click_pos = None;
                self.last_active_sheet_id = active_sheet_id;
                self.last_modal_surface = Some(decision.modal_surface);
                if decision.modal_backdrop_active || decision.renders_legacy_canvas {
                    ctx.request_repaint();
                }
            }

            let mut host_textures = HybridHostTextureFrame { canvas_texture: None, legacy_modal_texture: None, accesskit_canvas_id: None };
            if decision.renders_legacy_modal_overlay {
                self.sync_canvas_viewport(input_window_id, full_content_rect, ppp);
                self.legacy_viewport_origin_pt = full_content_rect.min;
                if let Some(frame) = self.tick_animations_and_redraw(&ctx, input_window_id)
                    && let Ok(win) = self.runtime.window(input_window_id)
                {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let dpi = (f64::from(ppp) * 96.0).round().clamp(72.0, 600.0) as u16;
                    let layout = compute_workspace_layout_for_dpi(win.app(), frame.width_px, frame.height_px, dpi);
                    if let Some(modal_px) = layout.modal_overlay
                        && modal_px.width > 0
                        && modal_px.height > 0
                        && let Some(modal_img) = crop_legacy_frame_modal_to_color_image(&frame, modal_px)
                    {
                        let tex_id = match &mut self.legacy_modal_texture {
                            Some(tex) => {
                                tex.set(modal_img, egui::TextureOptions::NEAREST);
                                tex.id()
                            }
                            None => {
                                let tex = ui.ctx().load_texture("curvebook_legacy_modal_rgba", modal_img, egui::TextureOptions::NEAREST);
                                let id = tex.id();
                                self.legacy_modal_texture = Some(tex);
                                id
                            }
                        };
                        #[allow(clippy::cast_precision_loss)]
                        let modal_rect_pt = egui::Rect::from_min_size(
                            egui::pos2(full_content_rect.min.x + (modal_px.x as f32) / ppp, full_content_rect.min.y + (modal_px.y as f32) / ppp),
                            egui::vec2((modal_px.width as f32) / ppp, (modal_px.height as f32) / ppp),
                        );
                        host_textures.legacy_modal_texture = Some((tex_id, modal_rect_pt));
                    }
                }
            } else if decision.renders_legacy_canvas {
                let target_rect = self.last_regions.canvas_rect_pt.unwrap_or(available_legacy_rect);
                self.sync_canvas_viewport(input_window_id, target_rect, ppp);
                self.legacy_viewport_origin_pt = target_rect.min;
                if let Some(frame) = self.tick_animations_and_redraw(&ctx, input_window_id) {
                    let size = [frame.width_px as usize, frame.height_px as usize];
                    if size[0] > 0 && size[1] > 0 && frame.rgba_bytes.len() >= size[0] * size[1] * 4 {
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba_bytes);
                        let tex_id = match &mut self.texture {
                            Some(tex) => {
                                tex.set(color_image, egui::TextureOptions::NEAREST);
                                tex.id()
                            }
                            None => {
                                let tex = ui.ctx().load_texture("curvebook_canvas_rgba", color_image, egui::TextureOptions::NEAREST);
                                let id = tex.id();
                                self.texture = Some(tex);
                                id
                            }
                        };
                        host_textures.canvas_texture = Some(tex_id);
                    }
                }
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
                    Some(&host_textures),
                );
                win.app_mut().egui_views = views;
                rep
            } else {
                inspect_bridge::ShellAboutFrameReport::default()
            };
            self.inspect_meta = next_meta;
            self.last_regions = report.regions;
            self.legacy_modal_rect_pt = report.legacy_modal_rect_pt;
            self.pending_intents = next_intents;

            let canvas_rect = report.regions.canvas_rect_pt.unwrap_or(available_legacy_rect);
            if !decision.renders_legacy_modal_overlay {
                self.sync_canvas_viewport(input_window_id, canvas_rect, ppp);
                self.legacy_viewport_origin_pt = canvas_rect.min;
            }
            if decision.renders_legacy_canvas
                && let Some(canvas_id) = report.legacy_canvas_widget_id
            {
                #[cfg(feature = "accessibility")]
                self.sync_accesskit_tree(ui, input_window_id, canvas_rect, canvas_id);
                #[cfg(not(feature = "accessibility"))]
                let _ = canvas_id;
            }
            let pointer_legacy_rect = if decision.renders_legacy_modal_overlay { full_content_rect } else { canvas_rect };
            self.process_canvas_input(ui, input_window_id, pointer_legacy_rect);
        } else {
            egui::CentralPanel::no_frame().show(ui, |ui| {
                let canvas_rect = ui.available_rect_before_wrap();
                let ppp = ctx.pixels_per_point().max(1.0);
                self.sync_canvas_viewport(input_window_id, canvas_rect, ppp);
                self.process_canvas_input(ui, input_window_id, canvas_rect);

                let paint_window_id = self.active_window_id();
                if let Some(frame) = self.tick_animations_and_redraw(&ctx, paint_window_id)
                    && let Some(canvas_resp) = self.upload_and_paint_frame(ui, canvas_rect, &frame)
                {
                    #[cfg(feature = "accessibility")]
                    self.sync_accesskit_tree(ui, paint_window_id, canvas_rect, canvas_resp.id);
                    #[cfg(not(feature = "accessibility"))]
                    let _ = canvas_resp;
                }
            });
        }

        let post_window_id = self.active_window_id();
        self.render_unsaved_close_modal(&ctx, post_window_id);
        self.sync_window_title(&ctx, post_window_id);

        let will_discard = ctx.will_discard();
        self.commit_pending_intents(post_window_id, will_discard);
        if will_discard {
            ctx.request_repaint();
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

    fn inline_editor_snapshot(&self, window_id: DesktopWindowId) -> (bool, String) {
        let Ok(win) = self.runtime.window(window_id) else {
            return (false, String::new());
        };
        let editor = &win.app().inline_editor;
        (editor.active, editor.buffer.clone())
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
        self.pointer_captured = false;
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

    /// Render the surviving desktop `egui::MenuBar`, returning `true` when the top menu bar was
    /// painted so `toolbar_menus` suppresses duplicate canvas menu widgets (`host_draws_menu_bar`).
    pub fn render_top_menu_bar(&mut self, ui: &mut egui::Ui, window_id: DesktopWindowId) -> bool {
        let Ok(win) = self.runtime.window(window_id) else {
            return false;
        };
        if win.app().presentation_mode {
            let _ = win;
            if let Ok(win_mut) = self.runtime.window_mut(window_id) {
                win_mut.app_mut().host_draws_menu_bar = false;
            }
            return false;
        }
        let win_locale = win.app().locale.clone();
        let chrome: DesktopMenuChromeState = win.chrome_state();
        if !chrome.menu_bar_visible || chrome.sections.is_empty() {
            let _ = win;
            if let Ok(win_mut) = self.runtime.window_mut(window_id) {
                win_mut.app_mut().host_draws_menu_bar = false;
            }
            if let Some(err) = self.status_banner.clone() {
                egui::Panel::top("curvebook_status_banner").show(ui, |ui| {
                    ui.colored_label(egui::Color32::from_rgb(200, 60, 60), err);
                });
            }
            return false;
        }
        let _ = win;
        if let Ok(win_mut) = self.runtime.window_mut(window_id) {
            win_mut.app_mut().host_draws_menu_bar = true;
        }

        let window_entries: Vec<(DesktopWindowId, String)> = if self.runtime.window_count() > 1 {
            self.runtime.window_ids().into_iter().filter_map(|id| self.runtime.window(id).ok().map(|w| (id, w.window_title()))).collect()
        } else {
            Vec::new()
        };

        let mut pending_command: Option<String> = None;
        let mut pending_focus_window: Option<DesktopWindowId> = None;
        egui::Panel::top("curvebook_menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                for section in &chrome.sections {
                    ui.menu_button(&section.title, |ui| {
                        for item in &section.items {
                            render_menu_item(ui, item, &mut pending_command);
                        }
                        if matches!(section.menu_key.as_str(), "menu.file" | "file") {
                            render_extra_file_actions(ui, &section.items, &mut pending_command);
                        }
                    });
                }
                if !window_entries.is_empty() {
                    ui.menu_button(curvebook_ui_egui::tr(&win_locale, "w6.view.shell.window_menu"), |ui| {
                        for (id, title) in &window_entries {
                            let label = if *id == window_id { format!("✓ Window {}: {title}", id.0) } else { format!("Window {}: {title}", id.0) };
                            if ui.button(label).clicked() {
                                pending_focus_window = Some(*id);
                                ui.close();
                            }
                        }
                    });
                }
                if let Some(err) = self.status_banner.as_deref() {
                    ui.separator();
                    ui.colored_label(egui::Color32::from_rgb(200, 60, 60), err);
                }
            });
        });

        if let Some(target_id) = pending_focus_window {
            let _ = self.runtime.dispatch(DesktopHostEvent::WindowFocused { window_id: target_id });
            self.on_active_window_switched();
        }
        if let Some(command_id) = pending_command {
            let ctx = ui.ctx().clone();
            self.execute_menu_command(Some(&ctx), window_id, &command_id);
        }
        true
    }

    fn execute_menu_command(&mut self, ctx: Option<&egui::Context>, window_id: DesktopWindowId, command_id: &str) {
        self.status_banner = None;
        if let Some(native_result) = self.try_handle_native_file_command(window_id, command_id) {
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
                if outcome.command_id.as_deref().is_some_and(|c| c == "edit.copy") {
                    self.apply_clipboard_outcome_status(&outcome);
                }
            }
            Err(err) => {
                self.status_banner = Some(err);
            }
        }
    }

    fn try_handle_native_file_command(&mut self, window_id: DesktopWindowId, command_id: &str) -> Option<Result<(), String>> {
        if !menu_action_triggers_native_file_dialog(command_id) {
            return None;
        }
        let outcome = match self.runtime.dispatch_native_file_command(window_id, command_id) {
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

    fn render_unsaved_close_modal(&mut self, ctx: &egui::Context, window_id: DesktopWindowId) {
        if !self.close_dialog_open {
            return;
        }
        let win_locale = self.runtime.window(window_id).map(|w| w.app().locale.clone()).unwrap_or_else(|_| String::from("en"));
        egui::Modal::new(egui::Id::unique("curvebook_unsaved_close_modal")).show(ctx, |ui| {
            ui.heading(curvebook_ui_egui::tr(&win_locale, "w6.view.shell.unsaved_title"));
            ui.label(curvebook_ui_egui::tr(&win_locale, "w6.view.shell.unsaved_prompt"));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(curvebook_ui_egui::tr(&win_locale, "menu.file.save")).clicked() {
                    match self.handle_native_save(window_id, false) {
                        Ok(()) => {
                            let still_dirty = self.runtime.window(window_id).map(|w| w.has_unsaved_changes()).unwrap_or(false);
                            if !still_dirty {
                                match self.runtime.dispatch(DesktopHostEvent::WindowCloseRequested { window_id, force: false }) {
                                    Ok(outcome) => self.apply_close_outcome(Some(ctx), &outcome),
                                    Err(err) => self.status_banner = Some(err),
                                }
                            }
                        }
                        Err(err) => {
                            self.status_banner = Some(err);
                        }
                    }
                }
                if ui.button(curvebook_ui_egui::tr(&win_locale, "w6.view.shell.dont_save")).clicked() {
                    match self.runtime.dispatch(DesktopHostEvent::WindowCloseRequested { window_id, force: true }) {
                        Ok(outcome) => self.apply_close_outcome(Some(ctx), &outcome),
                        Err(err) => self.status_banner = Some(err),
                    }
                }
                if ui.button(curvebook_ui_egui::tr(&win_locale, "btn.cancel")).clicked() {
                    self.close_dialog_open = false;
                }
            });
        });
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

    /// Process `egui` events against `canvas_rect` and translate them cleanly to [`DesktopHostEvent`].
    pub fn process_canvas_input(&mut self, ui: &egui::Ui, window_id: DesktopWindowId, canvas_rect: egui::Rect) {
        let (events, raw_hovered, raw_dropped, hover_pos, modifiers, zoom_delta) = ui.ctx().input(|i| {
            (i.events.clone(), i.raw.hovered_files.clone(), i.raw.dropped_files.clone(), i.pointer.hover_pos(), i.modifiers, i.zoom_delta())
        });

        self.dispatch_file_dnd(window_id, &raw_hovered, &raw_dropped);
        self.dispatch_egui_events(ui.ctx(), window_id, canvas_rect, hover_pos, modifiers, &events);

        let hybrid_decision = self.runtime.window(window_id).map(|w| HybridCoexistenceArbiter::evaluate(w.app(), self.close_dialog_open)).ok();
        let is_egui_backend = self.runtime.window(window_id).is_ok_and(|w| w.app().ui_backend() == UiBackend::Egui);
        let allows_legacy_magnify = !is_egui_backend || hybrid_decision.is_some_and(|d| d.renders_legacy_canvas && !d.modal_backdrop_active);
        if allows_legacy_magnify
            && (zoom_delta - 1.0).abs() > 1e-4
            && let Some(pos) = hover_pos
            && canvas_rect.contains(pos)
        {
            let (ax, ay) = to_canvas_pt(pos, canvas_rect.min);
            let _ = self.runtime.dispatch(DesktopHostEvent::TrackpadMagnify {
                window_id,
                anchor_x_pt: ax,
                anchor_y_pt: ay,
                magnification_delta: f64::from(zoom_delta - 1.0),
            });
        }
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

    fn dispatch_egui_events(
        &mut self,
        ctx: &egui::Context,
        window_id: DesktopWindowId,
        canvas_rect: egui::Rect,
        hover_pos: Option<egui::Pos2>,
        frame_modifiers: egui::Modifiers,
        events: &[egui::Event],
    ) {
        let mut suppressed_text_char: Option<String> = None;
        let popup_open = self.menu_popup_was_open || egui::Popup::is_any_open(ctx);
        let is_egui_backend = self.runtime.window(window_id).is_ok_and(|w| w.app().ui_backend() == UiBackend::Egui);
        let hybrid_decision = self.runtime.window(window_id).map(|w| HybridCoexistenceArbiter::evaluate(w.app(), self.close_dialog_open)).ok();
        let legacy_modal_active = is_egui_backend && hybrid_decision.is_some_and(|d| d.modal_surface == HybridModalSurface::LegacyModal);
        let egui_wants_kb = ctx.egui_wants_keyboard_input();
        let legacy_modal_blocking = self.close_dialog_open
            || self
                .runtime
                .window(window_id)
                .is_ok_and(|w| w.app().ui_backend() == UiBackend::Egui && w.app().canvas_dialogs.active_dialog != ActiveDialog::None);
        let legacy_modal_rect_pt = self.legacy_modal_rect_pt;
        let allows_legacy_pointer_at = |pos: egui::Pos2| -> bool {
            if is_egui_backend {
                hybrid_decision
                    .is_some_and(|d| HybridCoexistenceArbiter::pointer_routes_to_legacy(&d, canvas_rect, pos, popup_open, legacy_modal_rect_pt))
            } else {
                !legacy_modal_blocking && !popup_open
            }
        };
        if is_egui_backend && hybrid_decision.is_some_and(|d| !d.renders_legacy_canvas && d.modal_surface != HybridModalSurface::LegacyModal) {
            self.pointer_captured = false;
        }

        for event in events {
            match event {
                egui::Event::PointerMoved(pos) => {
                    let allow_drag = self.pointer_captured && (!is_egui_backend || hybrid_decision.is_some_and(|d| !d.blocks_legacy_background));
                    if (allows_legacy_pointer_at(*pos) && canvas_rect.contains(*pos)) || allow_drag {
                        let (x_pt, y_pt) = to_canvas_pt(*pos, canvas_rect.min);
                        let _ = self.runtime.dispatch(DesktopHostEvent::PointerMoved {
                            window_id,
                            x_pt,
                            y_pt,
                            modifiers: to_key_modifiers(frame_modifiers),
                        });
                    }
                }
                egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                    let allow_release_capture =
                        !*pressed && self.pointer_captured && (!is_egui_backend || hybrid_decision.is_some_and(|d| !d.blocks_legacy_background));
                    if allows_legacy_pointer_at(*pos) || allow_release_capture {
                        self.dispatch_pointer_button(CanvasPointerButtonArgs {
                            window_id,
                            canvas_rect,
                            pos: *pos,
                            button: *button,
                            pressed: *pressed,
                            modifiers: *modifiers,
                        });
                    }
                }
                egui::Event::MouseWheel { unit, delta, modifiers, .. } => {
                    let anchor = hover_pos.unwrap_or(canvas_rect.center());
                    if allows_legacy_pointer_at(anchor) && canvas_rect.contains(anchor) {
                        let (x_pt, y_pt) = to_canvas_pt(anchor, canvas_rect.min);
                        let scale = wheel_unit_scale(*unit);
                        let _ = self.runtime.dispatch(DesktopHostEvent::MouseWheel {
                            window_id,
                            x_pt,
                            y_pt,
                            delta_x: f64::from(delta.x) * scale,
                            delta_y: f64::from(delta.y) * scale,
                            modifiers: to_key_modifiers(*modifiers),
                        });
                    }
                }
                egui::Event::Key { key, pressed, modifiers, .. } => {
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
                    if let Some(inserted) = self.dispatch_key_event(ctx, window_id, *key, *pressed, *modifiers, is_egui_backend) {
                        suppressed_text_char = Some(inserted);
                    }
                }
                egui::Event::Text(text) => {
                    if suppressed_text_char.as_deref() == Some(text.as_str()) {
                        suppressed_text_char = None;
                        continue;
                    }
                    if (is_egui_backend && !legacy_modal_active) || egui_wants_kb {
                        continue;
                    }
                    let (inline_active, _) = self.inline_editor_snapshot(window_id);
                    if inline_active {
                        let _ = self.runtime.dispatch(DesktopHostEvent::ImeCommit { window_id, text: text.clone() });
                    }
                }
                egui::Event::Ime(ime) => {
                    if (!is_egui_backend || legacy_modal_active) && !egui_wants_kb {
                        self.dispatch_ime_event(window_id, ime);
                    }
                }
                egui::Event::Copy => {
                    if (!is_egui_backend || legacy_modal_active) && !egui_wants_kb {
                        self.dispatch_clipboard_copy(ctx, window_id);
                    }
                }
                egui::Event::Cut => {
                    if (!is_egui_backend || legacy_modal_active) && !egui_wants_kb {
                        self.dispatch_clipboard_cut(ctx, window_id);
                    }
                }
                egui::Event::Paste(text) => {
                    if (!is_egui_backend || legacy_modal_active)
                        && !egui_wants_kb
                        && let Err(err) = self.runtime.dispatch(DesktopHostEvent::ClipboardPaste { window_id, text_override: Some(text.clone()) })
                    {
                        self.status_banner = Some(err);
                    }
                }
                #[cfg(feature = "accessibility")]
                egui::Event::AccessKitActionRequest(req)
                    if !is_egui_backend
                        || hybrid_decision.is_some_and(|d| d.renders_legacy_canvas || d.modal_surface == HybridModalSurface::LegacyModal) =>
                {
                    self.handle_accesskit_action_request(ctx, window_id, req);
                }
                _ => {}
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

    fn dispatch_pointer_button(&mut self, args: CanvasPointerButtonArgs) {
        if args.pressed {
            if !args.canvas_rect.contains(args.pos) {
                return;
            }
            self.pointer_captured = true;
            let click_count = self.next_click_count(args.pos);
            let (x_pt, y_pt) = to_canvas_pt(args.pos, args.canvas_rect.min);
            let _ = self.runtime.dispatch(DesktopHostEvent::PointerButton {
                window_id: args.window_id,
                x_pt,
                y_pt,
                button: to_pointer_button(args.button),
                pressed: true,
                click_count,
                modifiers: to_key_modifiers(args.modifiers),
            });
        } else if self.pointer_captured {
            self.pointer_captured = false;
            let (x_pt, y_pt) = to_canvas_pt(args.pos, args.canvas_rect.min);
            match self.runtime.dispatch(DesktopHostEvent::PointerButton {
                window_id: args.window_id,
                x_pt,
                y_pt,
                button: to_pointer_button(args.button),
                pressed: false,
                click_count: self.click_count,
                modifiers: to_key_modifiers(args.modifiers),
            }) {
                Ok(outcome) if outcome.opened_window_id.is_some() => self.on_active_window_switched(),
                Err(err) => self.status_banner = Some(err),
                Ok(_) => {}
            }
        }
    }

    fn next_click_count(&mut self, pos: egui::Pos2) -> u8 {
        let now = Instant::now();
        let within_time = self.last_click_time.is_some_and(|prev| now.duration_since(prev).as_secs_f64() <= DOUBLE_CLICK_WINDOW_SEC);
        let within_dist = self.last_click_pos.is_some_and(|prev| prev.distance(pos) <= DOUBLE_CLICK_RADIUS_PT);
        self.click_count = if within_time && within_dist { self.click_count.saturating_add(1).min(3) } else { 1 };
        self.last_click_time = Some(now);
        self.last_click_pos = Some(pos);
        self.click_count
    }

    fn dispatch_key_event(
        &mut self,
        ctx: &egui::Context,
        window_id: DesktopWindowId,
        key: egui::Key,
        pressed: bool,
        modifiers: egui::Modifiers,
        is_egui_backend: bool,
    ) -> Option<String> {
        let key_name = egui_key_to_name(key)?;
        let (inline_active_before, inline_text_before) = self.inline_editor_snapshot(window_id);
        let dialog_inactive = !self.close_dialog_open
            && self.runtime.window(window_id).is_ok_and(|w| {
                let app = w.app();
                app.canvas_dialogs.active_dialog == ActiveDialog::None
                    && !app.nav.welcome.open
                    && !app.grid.import_wizard.open
                    && !app.grid.variables_dialog.open
            });
        let primary_cmd = modifiers.command || modifiers.mac_cmd || modifiers.ctrl;
        if pressed && !inline_active_before && primary_cmd && !modifiers.alt {
            if key == egui::Key::Plus || key == egui::Key::Equals {
                self.apply_ui_scale_adjustment(ctx, window_id, UiScaleAdjust::In);
                return None;
            }
            if key == egui::Key::Minus {
                self.apply_ui_scale_adjustment(ctx, window_id, UiScaleAdjust::Out);
                return None;
            }
            if key == egui::Key::Num0 && !modifiers.shift {
                let consume_for_scale =
                    self.runtime.window(window_id).is_ok_and(|w| (w.app().ui_scale() - 1.0).abs() > 0.001 || w.app().ui_backend() == UiBackend::Egui);
                if consume_for_scale {
                    self.apply_ui_scale_adjustment(ctx, window_id, UiScaleAdjust::Reset);
                    return None;
                }
            }
        }
        let allow_routed_shortcut = !is_egui_backend || (primary_cmd && !ctx.text_edit_focused());
        if pressed && !inline_active_before && dialog_inactive && allow_routed_shortcut {
            if primary_cmd && !modifiers.alt {
                if key == egui::Key::O && !modifiers.shift {
                    self.execute_menu_command(Some(ctx), window_id, "project.open");
                    return None;
                }
                if key == egui::Key::S {
                    let cmd = if modifiers.shift { "project.saveAs" } else { "project.save" };
                    self.execute_menu_command(Some(ctx), window_id, cmd);
                    return None;
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
                    return None;
                }
            }
        }
        let legacy_modal_active = is_egui_backend
            && self
                .runtime
                .window(window_id)
                .is_ok_and(|w| HybridCoexistenceArbiter::evaluate(w.app(), self.close_dialog_open).modal_surface == HybridModalSurface::LegacyModal);
        if (is_egui_backend && !legacy_modal_active) || ctx.egui_wants_keyboard_input() {
            return None;
        }
        let has_cmd_or_ctrl = modifiers.ctrl || modifiers.alt || modifiers.command || modifiers.mac_cmd;
        if pressed && !dialog_inactive && has_cmd_or_ctrl {
            return None;
        }
        let is_single_char_key = key_name.chars().count() == 1;

        if inline_active_before && pressed && is_single_char_key && !has_cmd_or_ctrl {
            // Let `egui::Event::Text` handle printable character insertion once when inline editor is active.
            return None;
        }

        match self.runtime.dispatch(DesktopHostEvent::KeyboardInput {
            window_id,
            key: key_name,
            pressed,
            modifiers: to_key_modifiers(modifiers),
            text: None,
        }) {
            Ok(outcome) if outcome.opened_window_id.is_some() => self.on_active_window_switched(),
            Err(err) => self.status_banner = Some(err),
            Ok(_) => {}
        }

        if pressed && is_single_char_key && !has_cmd_or_ctrl {
            let (inline_active_after, inline_text_after) = self.inline_editor_snapshot(window_id);
            if inline_active_after && (!inline_active_before || inline_text_after != inline_text_before) {
                return inline_text_after.strip_prefix(&inline_text_before).filter(|s| !s.is_empty()).map(str::to_string);
            }
        }
        None
    }

    fn dispatch_ime_event(&mut self, window_id: DesktopWindowId, ime: &egui::ImeEvent) {
        match ime {
            egui::ImeEvent::Preedit { text, active_range_chars } => {
                let cursor = active_range_chars.as_ref().map(|r| (r.start, r.end));
                let _ = self.runtime.dispatch(DesktopHostEvent::ImePreedit { window_id, text: text.clone(), cursor });
            }
            egui::ImeEvent::Commit(text) => {
                let _ = self.runtime.dispatch(DesktopHostEvent::ImeCommit { window_id, text: text.clone() });
            }
            _ => {}
        }
    }

    fn tick_animations_and_redraw(&mut self, ctx: &egui::Context, window_id: DesktopWindowId) -> Option<DesktopFrameOutput> {
        let now = Instant::now();
        let dt_sec = self.last_frame_time.map(|prev| now.duration_since(prev).as_secs_f64().clamp(0.0, 0.1)).unwrap_or(0.0);
        self.last_frame_time = Some(now);

        let coasting = self.runtime.window_ids().into_iter().any(|id| self.runtime.window(id).map(|w| w.is_coasting()).unwrap_or(false));
        if coasting && dt_sec > 0.0 {
            let _ = self.runtime.dispatch(DesktopHostEvent::AnimationTick { dt_sec });
            if self.runtime.window(window_id).map(|w| w.is_coasting()).unwrap_or(false) {
                ctx.request_repaint();
            }
        }

        self.runtime.dispatch(DesktopHostEvent::RedrawRequested { window_id }).ok().and_then(|outcome| outcome.frame)
    }

    fn upload_and_paint_frame(&mut self, ui: &mut egui::Ui, canvas_rect: egui::Rect, frame: &DesktopFrameOutput) -> Option<egui::Response> {
        let size = [frame.width_px as usize, frame.height_px as usize];
        if size[0] == 0 || size[1] == 0 || frame.rgba_bytes.len() < size[0] * size[1] * 4 {
            return None;
        }
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba_bytes);
        let tex_id = match &mut self.texture {
            Some(tex) => {
                tex.set(color_image, egui::TextureOptions::NEAREST);
                tex.id()
            }
            None => {
                let tex = ui.ctx().load_texture("curvebook_canvas_rgba", color_image, egui::TextureOptions::NEAREST);
                let id = tex.id();
                self.texture = Some(tex);
                id
            }
        };
        let response = ui.allocate_rect(canvas_rect, egui::Sense::click_and_drag());
        ui.painter().image(tex_id, canvas_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        Some(response)
    }

    #[cfg(feature = "accessibility")]
    fn sync_accesskit_tree(&mut self, ui: &mut egui::Ui, window_id: DesktopWindowId, canvas_rect: egui::Rect, canvas_id: egui::Id) {
        if ui.ctx().accesskit_node_builder(canvas_id, |_| {}).is_none() {
            return;
        }
        let tree_update = {
            let Ok(win) = self.runtime.window(window_id) else {
                return;
            };
            curvebook_ui_egui::views::a11y::sync_app_accessibility_tree(win.app(), win.viewport())
        };

        let a11y_id = |num: u64| egui::Id::unique(("curvebook_a11y", num));
        let focus_eid = (tree_update.focus_numeric_id != 0)
            .then(|| tree_update.node_by_numeric_id(tree_update.focus_numeric_id))
            .flatten()
            .map(|n| (a11y_id(n.numeric_id), n));
        if let Some((fe, fnode)) = focus_eid {
            #[allow(clippy::cast_possible_truncation)]
            let frect = egui::Rect::from_min_size(
                egui::pos2(canvas_rect.min.x + fnode.bounds_logical.x as f32, canvas_rect.min.y + fnode.bounds_logical.y as f32),
                egui::vec2((fnode.bounds_logical.w as f32).max(1.0), (fnode.bounds_logical.h as f32).max(1.0)),
            );
            let _ = ui.interact(frect, fe, egui::Sense::focusable_noninteractive());
            ui.ctx().memory_mut(|m| m.request_focus(fe));
        }

        let mut by_num: BTreeMap<u64, usize> = BTreeMap::new();
        for (idx, node) in tree_update.nodes.iter().enumerate() {
            by_num.insert(node.numeric_id, idx);
        }
        let mut emitted: BTreeSet<u64> = BTreeSet::new();
        let mut queue: VecDeque<u64> = tree_update.root_children_numeric_ids.iter().copied().collect();
        for node in &tree_update.nodes {
            if node.parent_numeric_id.is_none() {
                queue.push_back(node.numeric_id);
            }
        }

        self.a11y_id_map.clear();
        let mut owned_ak_ids: BTreeSet<egui::accesskit::NodeId> = BTreeSet::new();
        let mut root_ak_ids: Vec<egui::accesskit::NodeId> = Vec::new();
        let mut emit_node = |node: &curvebook_ui_egui::layout::a11y::AccessibleNodeUpdate| {
            let eid = a11y_id(node.numeric_id);
            let ak_id = eid.accesskit_id();
            owned_ak_ids.insert(ak_id);
            self.a11y_id_map.insert(ak_id, node.widget_id.clone());
            if node.parent_numeric_id.is_none() {
                root_ak_ids.push(ak_id);
            }
            let origin = canvas_rect.min;
            let _ = ui.ctx().accesskit_node_builder(eid, |ak| {
                populate_accesskit_node(ak, node, origin);
                let child_ak_ids: Vec<egui::accesskit::NodeId> = node.children_numeric_ids.iter().map(|c| a11y_id(*c).accesskit_id()).collect();
                ak.set_children(child_ak_ids);
            });
        };

        while let Some(num_id) = queue.pop_front() {
            if !emitted.insert(num_id) {
                continue;
            }
            let Some(&idx) = by_num.get(&num_id) else {
                continue;
            };
            let node = &tree_update.nodes[idx];
            emit_node(node);
            for &child_num in &node.children_numeric_ids {
                if !emitted.contains(&child_num) {
                    queue.push_back(child_num);
                }
            }
        }
        for node in &tree_update.nodes {
            if !emitted.insert(node.numeric_id) {
                continue;
            }
            emit_node(node);
        }

        if let Some(msg) = tree_update.live_announcement.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let politeness = map_accesskit_live(tree_update.live_politeness);
            if politeness != egui::accesskit::Live::Off {
                let live_id = egui::Id::unique("curvebook_a11y_live_region");
                let live_ak = live_id.accesskit_id();
                owned_ak_ids.insert(live_ak);
                root_ak_ids.push(live_ak);
                let _ = ui.ctx().accesskit_node_builder(live_id, |ak| {
                    ak.set_role(egui::accesskit::Role::Status);
                    ak.set_live(politeness);
                    ak.set_label(msg.to_owned());
                    ak.set_bounds(egui::accesskit::Rect {
                        x0: f64::from(canvas_rect.min.x),
                        y0: f64::from(canvas_rect.min.y),
                        x1: f64::from(canvas_rect.min.x) + 1.0,
                        y1: f64::from(canvas_rect.min.y) + 1.0,
                    });
                });
            }
        }

        // Remove any auto-parented edges from egui root/panel containers so every node has a single parent (`canvas_id` or its declared parent).
        for container_id in [egui::accesskit_root_id(), ui.unique_id()] {
            let _ = ui.ctx().accesskit_node_builder(container_id, |ak| {
                let kept: Vec<egui::accesskit::NodeId> = ak.children().iter().copied().filter(|id| !owned_ak_ids.contains(id)).collect();
                ak.set_children(kept);
            });
        }
        let _ = ui.ctx().accesskit_node_builder(canvas_id, |ak| {
            ak.set_role(egui::accesskit::Role::Canvas);
            ak.set_label(tree_update.root_label.clone());
            ak.set_bounds(egui::accesskit::Rect {
                x0: f64::from(canvas_rect.min.x),
                y0: f64::from(canvas_rect.min.y),
                x1: f64::from(canvas_rect.max.x),
                y1: f64::from(canvas_rect.max.y),
            });
            let mut kids: Vec<egui::accesskit::NodeId> = ak.children().iter().copied().filter(|id| !owned_ak_ids.contains(id)).collect();
            kids.extend(root_ak_ids);
            ak.set_children(kids);
        });
    }

    #[cfg(feature = "accessibility")]
    fn handle_accesskit_action_request(&mut self, ctx: &egui::Context, window_id: DesktopWindowId, req: &egui::accesskit::ActionRequest) {
        use egui::accesskit::{Action, ActionData};

        let Some(widget_id) = self.a11y_id_map.get(&req.target_node).cloned() else {
            return;
        };
        let (center_pt, viewport) = {
            let Ok(win) = self.runtime.window(window_id) else {
                return;
            };
            let vp = win.viewport();
            let scale = vp.clamped_scale_factor();
            let tree = win.app().layout_tree_cow(vp);
            let pt = tree.node(&widget_id).map(|n| {
                let r = n.bounds.rect_px;
                ((f64::from(r.x) + f64::from(r.width) * 0.5) / scale, (f64::from(r.y) + f64::from(r.height) * 0.5) / scale)
            });
            (pt, vp)
        };

        match req.action {
            Action::Focus => {
                if let Ok(win) = self.runtime.window_mut(window_id) {
                    win.app_mut().focus_widget_id = Some(widget_id);
                    let _ = win.app_mut().build_and_cache_layout_tree(viewport);
                }
                ctx.request_repaint();
            }
            Action::Click | Action::Expand | Action::Collapse => {
                if let Some((x_pt, y_pt)) = center_pt {
                    let mods = KeyModifiers::default();
                    let _ = self.runtime.dispatch(DesktopHostEvent::PointerButton {
                        window_id,
                        x_pt,
                        y_pt,
                        button: PointerButton::Primary,
                        pressed: true,
                        click_count: 1,
                        modifiers: mods,
                    });
                    match self.runtime.dispatch(DesktopHostEvent::PointerButton {
                        window_id,
                        x_pt,
                        y_pt,
                        button: PointerButton::Primary,
                        pressed: false,
                        click_count: 1,
                        modifiers: mods,
                    }) {
                        Ok(outcome) if outcome.opened_window_id.is_some() => self.on_active_window_switched(),
                        Err(err) => self.status_banner = Some(err),
                        Ok(_) => {}
                    }
                    ctx.request_repaint();
                }
            }
            Action::SetValue => {
                let next_text = match &req.data {
                    Some(ActionData::Value(val)) => Some(val.to_string()),
                    Some(ActionData::NumericValue(num)) => Some(format!("{num}")),
                    _ => None,
                };
                if let Some(text) = next_text {
                    if let Ok(win) = self.runtime.window_mut(window_id) {
                        let app = win.app_mut();
                        app.focus_widget_id = Some(widget_id.clone());
                        app.interaction.text_field_buffers.remove(&widget_id);
                    }
                    if !text.is_empty() {
                        let _ = self.runtime.dispatch(DesktopHostEvent::ImeCommit { window_id, text });
                    }
                    ctx.request_repaint();
                }
            }
            _ => {}
        }
    }
}

#[cfg(feature = "accessibility")]
fn map_accesskit_role(mapping: &curvebook_ui_egui::layout::a11y::AriaRoleMapping) -> egui::accesskit::Role {
    use egui::accesskit::Role;
    match mapping.accesskit_role {
        "MenuBar" => Role::MenuBar,
        "MenuItem" => Role::MenuItem,
        "Button" => Role::Button,
        "Tab" => Role::Tab,
        "TreeItem" => Role::TreeItem,
        "GridCell" => Role::GridCell,
        "TextInput" => Role::TextInput,
        "MultilineTextInput" => Role::MultilineTextInput,
        "ComboBox" => Role::ComboBox,
        "CheckBox" => Role::CheckBox,
        "RadioButton" => Role::RadioButton,
        "SpinButton" => Role::SpinButton,
        "Slider" => Role::Slider,
        "ScrollBar" => Role::ScrollBar,
        "Splitter" => Role::Splitter,
        "Canvas" => Role::Canvas,
        "GraphicsSymbol" => Role::GraphicsSymbol,
        "Dialog" => Role::Dialog,
        "Banner" => Role::Banner,
        "Tooltip" => Role::Tooltip,
        "Menu" => Role::Menu,
        "DisclosureTriangle" => Role::DisclosureTriangle,
        _ => Role::Label,
    }
}

#[cfg(feature = "accessibility")]
fn map_accesskit_live(politeness: curvebook_ui_egui::layout::a11y::AriaLivePoliteness) -> egui::accesskit::Live {
    use curvebook_ui_egui::layout::a11y::AriaLivePoliteness;
    use egui::accesskit::Live;
    match politeness {
        AriaLivePoliteness::Off => Live::Off,
        AriaLivePoliteness::Polite => Live::Polite,
        AriaLivePoliteness::Assertive => Live::Assertive,
    }
}

#[cfg(feature = "accessibility")]
fn populate_accesskit_node(ak: &mut egui::accesskit::Node, node: &curvebook_ui_egui::layout::a11y::AccessibleNodeUpdate, origin: egui::Pos2) {
    use curvebook_ui_egui::layout::a11y::AccessibleActionTag;
    use egui::accesskit::{Action, Toggled};

    ak.set_role(map_accesskit_role(&node.role_mapping));
    let x0 = f64::from(origin.x) + node.bounds_logical.x;
    let y0 = f64::from(origin.y) + node.bounds_logical.y;
    ak.set_bounds(egui::accesskit::Rect { x0, y0, x1: x0 + node.bounds_logical.w, y1: y0 + node.bounds_logical.h });
    ak.set_label(node.label.clone());
    if let Some(val) = &node.value_text {
        ak.set_value(val.clone());
    }
    if node.enabled {
        ak.clear_disabled();
    } else {
        ak.set_disabled();
        if let Some(reason) = &node.disabled_reason {
            ak.set_description(reason.clone());
        }
    }
    if let Some(sel) = node.selected {
        ak.set_selected(sel);
    }
    if let Some(exp) = node.expanded {
        ak.set_expanded(exp);
    }
    if let Some(chk) = node.checked {
        ak.set_toggled(if chk { Toggled::True } else { Toggled::False });
    }
    if node.hidden {
        ak.set_hidden();
    }
    if node.modal {
        ak.set_modal();
    }
    if let Some(live) = node.live_region {
        ak.set_live(map_accesskit_live(live));
    }
    if let Some(shortcut) = node.shortcut.as_deref().or(node.keyboard_path.as_deref()) {
        ak.set_keyboard_shortcut(shortcut.to_owned());
    }
    for act in &node.actions {
        let ak_act = match act {
            AccessibleActionTag::Focus => Action::Focus,
            AccessibleActionTag::Click => Action::Click,
            AccessibleActionTag::Expand => Action::Expand,
            AccessibleActionTag::Collapse => Action::Collapse,
            AccessibleActionTag::Increment => Action::Increment,
            AccessibleActionTag::Decrement => Action::Decrement,
            AccessibleActionTag::SetValue => Action::SetValue,
            AccessibleActionTag::Scroll => Action::ScrollIntoView,
        };
        ak.add_action(ak_act);
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
    LegacyRepaint,
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

fn crop_legacy_frame_modal_to_color_image(frame: &DesktopFrameOutput, rect: curvebook_ui_egui::WorkspacePixelRect) -> Option<egui::ColorImage> {
    if frame.width_px == 0 || frame.height_px == 0 || rect.width == 0 || rect.height == 0 {
        return None;
    }
    let x0 = rect.x.min(frame.width_px.saturating_sub(1));
    let y0 = rect.y.min(frame.height_px.saturating_sub(1));
    let w = rect.width.min(frame.width_px.saturating_sub(x0));
    let h = rect.height.min(frame.height_px.saturating_sub(y0));
    if w == 0 || h == 0 {
        return None;
    }
    let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
    for row in 0..h {
        let src_start = (((y0 + row) as usize) * (frame.width_px as usize) + (x0 as usize)) * 4;
        let dst_start = (row as usize) * (w as usize) * 4;
        let bytes = (w as usize) * 4;
        if let (Some(src_slice), Some(dst_slice)) = (frame.rgba_bytes.get(src_start..src_start + bytes), rgba.get_mut(dst_start..dst_start + bytes)) {
            dst_slice.copy_from_slice(src_slice);
        }
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba))
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

fn should_defer_to_settled_frame(req: &LiveControlRequest, is_egui_backend: bool) -> bool {
    if !req.conn_authenticated || classify_control_frame(&req.raw_bytes) != ControlFrameKind::Command {
        return false;
    }
    let Ok(text) = std::str::from_utf8(&req.raw_bytes) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<ControlRequest>(text.trim()) else {
        return false;
    };
    matches!(parsed.method.as_str(), "ui.inspect" | "ui.screenshot") || (is_egui_backend && parsed.method == "ui.pointer_gesture")
}

fn ensure_params_object(params: &mut serde_json::Value) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    if params.is_null() {
        *params = serde_json::Value::Object(serde_json::Map::new());
    }
    params.as_object_mut()
}

fn menu_item_tooltip(item: &DesktopMenuItem) -> String {
    if item.beginner.is_empty() { item.summary.clone() } else { format!("{}\n{}", item.summary, item.beginner) }
}

fn render_menu_item(ui: &mut egui::Ui, item: &DesktopMenuItem, pending_command: &mut Option<String>) {
    if item.command_id == "ui.event.dispatch" {
        return;
    }
    let tooltip = menu_item_tooltip(item);
    let effective_enabled = item.enabled && item.command_id != "ui.result.applyRemediation";

    if let Some(variants) = submenu_variants_for_command(&item.command_id) {
        ui.menu_button(&item.localized_label, |ui| {
            for (label, variant) in variants {
                let mut btn = egui::Button::new(*label);
                if item.command_id == "ui.canvas.applyPreset" && !item.shortcut.is_empty() && *variant == "SingleColumn88mm" {
                    btn = btn.shortcut_text(&item.shortcut);
                }
                let resp = ui.add_enabled(effective_enabled, btn).on_hover_text(&tooltip).on_disabled_hover_text(&tooltip);
                if resp.clicked() {
                    *pending_command = Some(format!("{}:{variant}", item.command_id));
                    ui.close();
                }
            }
        });
        return;
    }

    if item.command_id == "ui.locale.set" {
        ui.menu_button(&item.localized_label, |ui| {
            for &loc in BETA_ACTIVE_LOCALES {
                let canonical = normalize_beta_gui_locale(loc);
                let caption = locale_picker_label(canonical);
                if ui.button(caption).on_hover_text(&tooltip).clicked() {
                    *pending_command = Some(format!("ui.locale.set:{canonical}"));
                    ui.close();
                }
            }
        });
        return;
    }

    let mut button = egui::Button::new(&item.localized_label);
    if !item.shortcut.is_empty() {
        button = button.shortcut_text(&item.shortcut);
    }
    let resp = ui.add_enabled(effective_enabled, button).on_hover_text(&tooltip).on_disabled_hover_text(&tooltip);
    if resp.clicked() {
        *pending_command = Some(item.command_id.clone());
        ui.close();
    }
}

fn submenu_variants_for_command(command_id: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match command_id {
        "ui.theme.set" => Some(&[("Light", "light"), ("Dark", "dark"), ("High Contrast", "high_contrast")]),
        "ui.previewFilter.set" => Some(&[
            ("Normal Vision", "normal"),
            ("Deuteranopia", "deuteranopia"),
            ("Protanopia", "protanopia"),
            ("Tritanopia", "tritanopia"),
            ("Grayscale Print", "grayscale"),
        ]),
        "ui.canvas.applyPreset" | "graph.applyCanvasPreset" | "layout.applyCanvasPreset" => Some(&[
            ("Single Column (88 mm)", "SingleColumn88mm"),
            ("1.5 Column (130 mm)", "ColumnAndHalf130mm"),
            ("Double Column (180 mm)", "DoubleColumn180mm"),
            ("Full Page Portrait (180 × 240 mm)", "FullPagePortrait"),
            ("Presentation Slide 16:9", "Slide16x9"),
            ("Poster Panel (12 × 9 in)", "PosterPanel"),
        ]),
        _ => None,
    }
}

fn render_extra_file_actions(ui: &mut egui::Ui, existing_items: &[DesktopMenuItem], pending_command: &mut Option<String>) {
    let extras = [
        ("Open Unpacked Folder…", "file.open_unpacked_dir", &["file.open_unpacked_dir"][..]),
        ("Save As…", "file.save_as", &["project.saveAs", "file.save_as"][..]),
        ("Export PDF…", "export.pdf", &["export.pdf", "file.export_pdf"][..]),
        ("Export PNG…", "export.png", &["export.png", "file.export_png"][..]),
    ];
    let mut added_separator = false;
    for (label, cmd, aliases) in extras {
        if existing_items.iter().any(|item| aliases.contains(&item.command_id.as_str())) {
            continue;
        }
        if !added_separator {
            ui.separator();
            added_separator = true;
        }
        if ui.button(label).clicked() {
            *pending_command = Some(String::from(cmd));
            ui.close();
        }
    }
}

fn to_canvas_pt(pos: egui::Pos2, canvas_min: egui::Pos2) -> (f64, f64) {
    (f64::from((pos.x - canvas_min.x).max(0.0)), f64::from((pos.y - canvas_min.y).max(0.0)))
}

fn wheel_unit_scale(unit: egui::MouseWheelUnit) -> f64 {
    match unit {
        egui::MouseWheelUnit::Point => 1.0,
        egui::MouseWheelUnit::Line => 24.0,
        egui::MouseWheelUnit::Page => 120.0,
    }
}

fn to_pointer_button(button: egui::PointerButton) -> PointerButton {
    match button {
        egui::PointerButton::Primary => PointerButton::Primary,
        egui::PointerButton::Secondary => PointerButton::Secondary,
        egui::PointerButton::Middle | egui::PointerButton::Extra1 | egui::PointerButton::Extra2 => PointerButton::Middle,
    }
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

/// Launch the interactive native desktop window while optionally attaching a [`LiveControlChannel`]
/// so external `--control` automation and human window input share the same live `UiApp` session.
///
/// # Errors
/// Returns an error if no open window is present or `eframe` window startup fails.
pub fn run_native_desktop_with_control(
    runtime: DesktopHostRuntime,
    extra_ttf: Option<Vec<u8>>,
    on_exit_cleanup: CleanupHook,
    control: Option<LiveControlChannel>,
) -> Result<(), String> {
    let primary_window_id = runtime
        .focused_window_id()
        .or_else(|| runtime.window_ids().into_iter().next())
        .ok_or_else(|| String::from("no open window available for interactive desktop launch"))?;
    let initial_title = runtime.window(primary_window_id).map(|w| w.window_title()).unwrap_or_else(|_| String::from("CurveBook"));

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("org.curvebook.CurveBook")
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
    use curvebook_ui_egui::interaction::ViewportMetrics;
    use serde_json::json;

    use super::*;
    use crate::desktop_host::DesktopHostConfig;

    fn test_runtime(viewport: ViewportMetrics) -> Result<(DesktopHostRuntime, DesktopWindowId), String> {
        let mut runtime = DesktopHostRuntime::new(DesktopHostConfig {
            in_window_menus: true,
            new_instance: true,
            gui_only: false,
            ui_backend: UiBackend::Legacy,
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
            app.render_top_menu_bar(ui, id);
            egui::CentralPanel::no_frame().show(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                app.sync_canvas_viewport(id, rect, 1.0);
                if let Some(frame) = app.tick_animations_and_redraw(ui.ctx(), id) {
                    app.upload_and_paint_frame(ui, rect, &frame);
                }
            });
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
            win.app_mut().inline_editor.active = true;
            win.app_mut().inline_editor.buffer.clear();
        }
        let mut app = CurveBookNativeApp::new(runtime, id, None);
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 24.0), egui::vec2(960.0, 616.0));

        let events = vec![
            egui::Event::Key { key: egui::Key::Num4, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
            egui::Event::Text(String::from("4")),
            egui::Event::Key { key: egui::Key::Num2, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
            egui::Event::Text(String::from("2")),
        ];

        app.dispatch_egui_events(&ctx, id, rect, Some(egui::pos2(200.0, 200.0)), egui::Modifiers::NONE, &events);

        let win = app.runtime.window(id)?;
        assert_eq!(win.app().inline_editor.buffer, "42");
        Ok(())
    }

    #[test]
    fn hidpi_canvas_resize_and_relative_pointer_offset() -> Result<(), String> {
        let (runtime, id) = test_runtime(ViewportMetrics::new(400.0, 300.0, 1.0))?;
        let mut app = CurveBookNativeApp::new(runtime, id, None);
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 24.0), egui::vec2(800.0, 500.0));

        app.sync_canvas_viewport(id, rect, 2.0);
        assert_eq!(app.last_frame_size, [1600, 1000]);

        let events = vec![egui::Event::PointerMoved(egui::pos2(120.0, 74.0))];
        app.dispatch_egui_events(&ctx, id, rect, Some(egui::pos2(120.0, 74.0)), egui::Modifiers::NONE, &events);

        let win = app.runtime.window(id)?;
        assert_eq!(win.viewport().scale_factor, 2.0);
        assert_eq!(win.viewport().logical_width, 800.0);
        assert_eq!(win.viewport().logical_height, 500.0);
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
        writes_text: std::sync::RwLock<Vec<String>>,
        writes_bundle_count: std::sync::atomic::AtomicUsize,
    }

    impl TestGuiClipboard {
        fn new(available: bool, fail_writes: bool) -> Self {
            Self {
                fail_writes,
                available,
                writes_text: std::sync::RwLock::new(Vec::new()),
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
        {
            let win = runtime.window_mut(id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "ds-1", "kind": "column", "cols": 1, "rows": 3, "auto_graph": true }))?;
            win.execute_and_journal("table.setCell", json!({ "sheet_id": "ds-1", "row": 0, "col": 0, "value": 11.0 }))?;
            win.execute_and_journal("table.setCell", json!({ "sheet_id": "ds-1", "row": 1, "col": 0, "value": 22.0 }))?;
            let graph_id = win.app().document().graph_sheets.first().ok_or("missing graph")?.id.clone();
            win.execute_and_journal("sheet.select", json!({ "sheet_id": graph_id.as_str() }))?;
        }
        let mut app = CurveBookNativeApp::new(runtime, id, None);
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 24.0), egui::vec2(960.0, 616.0));

        // 1. Copy on GraphSheet must NOT emit OutputCommand::CopyText (preserves PDF/PNG/HTML bundle).
        let raw_input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(960.0, 640.0))), ..Default::default() };
        let mut out_graph = ctx.run_ui(raw_input.clone(), |ui| {
            app.dispatch_egui_events(ui.ctx(), id, rect, None, egui::Modifiers::NONE, &[egui::Event::Copy]);
        });
        out_graph.textures_delta.clear();
        assert!(app.runtime.clipboard_bundle().is_some());
        assert!(app.runtime.clipboard_text().is_none());
        let emitted_copy_text = out_graph.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_)));
        assert!(!emitted_copy_text, "graph bundle copy must NOT overwrite OS multi-flavor clipboard with ctx.copy_text");

        // 2. Cut on DataSheet clears ONLY the selected cell, leaving unselected rows intact.
        {
            let win = app.runtime.window_mut(id)?;
            win.execute_and_journal("sheet.select", json!({ "sheet_id": "ds-1" }))?;
            win.app_mut().grid.select_single_sync(curvebook_doc::CellCoord::new(0, 0, 0).into());
        }
        app.dispatch_egui_events(&ctx, id, rect, None, egui::Modifiers::NONE, &[egui::Event::Cut]);
        let ds = app.runtime.window(id)?.app().document().data_sheet(&SheetId::new("ds-1")).ok_or("missing ds-1")?;
        assert_eq!(ds.cell(curvebook_doc::CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), None);
        assert_eq!(ds.cell(curvebook_doc::CellCoord::new(1, 0, 0)).and_then(|c| c.active_number()), Some(22.0));

        // 3. Cut during inline editor clears inline_editor.buffer cleanly.
        {
            let win = app.runtime.window_mut(id)?;
            win.app_mut().inline_editor.active = true;
            win.app_mut().inline_editor.buffer = String::from("edited-inline");
            win.app_mut().inline_editor.caret_byte = 13;
        }
        app.dispatch_egui_events(&ctx, id, rect, None, egui::Modifiers::NONE, &[egui::Event::Cut]);
        assert_eq!(app.runtime.window(id)?.app().inline_editor.buffer, "");
        assert_eq!(app.runtime.clipboard_text(), Some("edited-inline"));

        // 4. Failing OS clipboard surfaced in status_banner.
        app.runtime.set_clipboard_adapter(Box::new(TestGuiClipboard::new(true, true)));
        app.dispatch_egui_events(&ctx, id, rect, None, egui::Modifiers::NONE, &[egui::Event::Copy]);
        assert!(app.status_banner().is_some_and(|s| s.contains("pasteboard write denied")), "banner: {:?}", app.status_banner());
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
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 24.0), egui::vec2(960.0, 616.0));
        let cmd_s = egui::Event::Key {
            key: egui::Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { command: true, ..egui::Modifiers::NONE },
        };

        // Blocked while import_wizard / variables_dialog / close_dialog_open are open.
        app.runtime.window_mut(id)?.app_mut().grid.import_wizard.open = true;
        app.dispatch_egui_events(&ctx, id, rect, None, egui::Modifiers::NONE, std::slice::from_ref(&cmd_s));
        assert!(app.runtime.window(id)?.has_unsaved_changes(), "Cmd+S must be blocked while ImportWizard is open");
        app.runtime.window_mut(id)?.app_mut().grid.import_wizard.open = false;

        app.runtime.window_mut(id)?.app_mut().grid.variables_dialog.open = true;
        app.dispatch_egui_events(&ctx, id, rect, None, egui::Modifiers::NONE, std::slice::from_ref(&cmd_s));
        assert!(app.runtime.window(id)?.has_unsaved_changes(), "Cmd+S must be blocked while VariablesDialog is open");
        app.runtime.window_mut(id)?.app_mut().grid.variables_dialog.open = false;

        // With all modals closed, Cmd+S saves archive to disk cleanly.
        app.dispatch_egui_events(&ctx, id, rect, None, egui::Modifiers::NONE, std::slice::from_ref(&cmd_s));
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
        app.dispatch_egui_events(&ctx, id, rect, None, egui::Modifiers::NONE, &[cmd_s]);
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
            win.app_mut().focus_widget_id = Some(String::from("toolbar.analyze"));
        }
        let mut app = CurveBookNativeApp::new(runtime, id, None);
        let ctx = egui::Context::default();
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
        assert!(!app.a11y_id_map.is_empty(), "expected populated a11y_id_map, frame_size={:?}", app.last_frame_size);

        let mut parent_counts: std::collections::BTreeMap<egui::accesskit::NodeId, usize> = std::collections::BTreeMap::new();
        for (_, node) in &update.nodes {
            for &child_id in node.children() {
                *parent_counts.entry(child_id).or_default() += 1;
            }
        }
        for (nid, count) in &parent_counts {
            assert_eq!(*count, 1, "accesskit node {nid:?} has {count} parents (expected 1)");
        }
        let focused_ak = app.a11y_id_map.iter().find_map(|(k, v)| (v == "toolbar.analyze").then_some(*k));
        assert!(
            focused_ak.is_some(),
            "missing toolbar.analyze in a11y_id_map (focus={:?}, nodes={:?})",
            app.runtime.window(id)?.app().focus_widget_id,
            app.a11y_id_map.values().collect::<Vec<_>>()
        );
        assert_eq!(Some(update.focus), focused_ak, "expected AccessKit focus on toolbar.analyze across multi-frame pass");

        // Dispatch AccessKit Click action on `toolbar.analyze` -> opens AnalyzeSheet dialog.
        let target_node = focused_ak.ok_or_else(|| String::from("missing toolbar.analyze ak id"))?;
        let req = egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node,
            data: None,
        };
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 24.0), egui::vec2(1024.0, 616.0));
        app.dispatch_egui_events(&ctx, id, rect, None, egui::Modifiers::NONE, &[egui::Event::AccessKitActionRequest(req)]);
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
    fn r15_2_gui_window_hybrid_arbiter_live_gesture_case_a_and_case_b() -> Result<(), String> {
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

        let auth = ControlAuth::from_token("test-token-r15-2").map_err(|e| e.to_string())?;
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

        // Case B settled: legacy AnalyzeSheet rendered over egui Column grid!
        assert!(matches!(app.runtime().window(id)?.app().canvas_dialogs.active_dialog, ActiveDialog::AnalyzeSheet { .. }));
        assert!(app.legacy_modal_rect_pt.is_some(), "Case B must render cropped legacy AnalyzeSheet overlay");

        // Scroll legacy `analyze.entries_list` inside modal via `ui.pointer_gesture` wheel until `analyze.entry.stats.one_way_anova` is visible, then click `(x, y)` + confirm!
        let mut entry_xy: Option<[f64; 2]> = None;
        for _ in 0..8 {
            let insp = send_ctrl(&mut app, br#"{"id":"i1","method":"ui.inspect","params":{"scope":"dialog"}}"#.to_vec())?;
            let controls = insp.pointer("/result/dialog/controls").and_then(serde_json::Value::as_array).ok_or("missing dialog controls")?;
            if let Some(b) = controls
                .iter()
                .find(|c| c.get("widget_id").and_then(serde_json::Value::as_str) == Some("analyze.entry.stats.one_way_anova"))
                .and_then(|c| c.get("bounds"))
                .and_then(serde_json::Value::as_array)
            {
                let x = b[0].as_f64().unwrap_or(0.0) + b[2].as_f64().unwrap_or(0.0) * 0.5;
                let y = b[1].as_f64().unwrap_or(0.0) + b[3].as_f64().unwrap_or(0.0) * 0.5;
                entry_xy = Some([x, y]);
                break;
            }
            let lb = controls
                .iter()
                .find(|c| c.get("widget_id").and_then(serde_json::Value::as_str) == Some("analyze.entries_list"))
                .and_then(|c| c.get("bounds"))
                .and_then(serde_json::Value::as_array)
                .ok_or("missing analyze.entries_list bounds")?;
            let lx = lb[0].as_f64().unwrap_or(0.0) + lb[2].as_f64().unwrap_or(0.0) * 0.5;
            let ly = lb[1].as_f64().unwrap_or(0.0) + lb[3].as_f64().unwrap_or(0.0) * 0.5;
            let w_cmd = format!(
                r#"{{"id":"w1","method":"ui.pointer_gesture","params":{{"action":"wheel","x":{lx},"y":{ly},"delta_x":0.0,"delta_y":140.0}}}}"#
            );
            let _ = send_ctrl(&mut app, w_cmd.into_bytes())?;
        }
        let [ex, ey] = entry_xy.ok_or("analyze.entry.stats.one_way_anova never scrolled into view")?;
        let click_cmd = format!(r#"{{"id":"sel","method":"ui.pointer_gesture","params":{{"action":"click","x":{ex},"y":{ey}}}}}"#);
        let sel_val = send_ctrl(&mut app, click_cmd.into_bytes())?;
        assert_eq!(sel_val.get("ok").and_then(serde_json::Value::as_bool), Some(true), "select one_way_anova failed: {sel_val}");

        let ok_val =
            send_ctrl(&mut app, br#"{"id":"run","method":"ui.answer_dialog","params":{"dialog_id":"analyze_sheet","action":"ok"}}"#.to_vec())?;
        assert_eq!(ok_val.get("ok").and_then(serde_json::Value::as_bool), Some(true), "confirm analyze_sheet failed: {ok_val}");
        step_once(&mut app, Vec::new());
        let active_rs = app.runtime().window(id)?.app().document().active_sheet_id.clone().ok_or("expected active ResultSheet")?;
        assert!(app.runtime().window(id)?.app().document().result_sheet(&active_rs).is_some());
        assert!(app.legacy_modal_rect_pt.is_none());

        // Press Cmd+T on legacy ResultSheet -> Case A opens `egui` One-Way ANOVA modal!
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
}
