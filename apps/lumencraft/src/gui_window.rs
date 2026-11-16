#![forbid(unsafe_code)]

//! Interactive `eframe` + `egui` + `wgpu` native desktop window host for LumenCraft.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Instant;

use eframe::egui;
use lumencraft_doc::{ProjectDocument, SheetId, SheetSection};
use lumencraft_text::{BUNDLED_FONTS, TrueTypeFont, build_minimal_truetype_font};
use lumencraft_ui_egui::dialogs_canvas::ActiveDialog;
use lumencraft_ui_egui::interaction::{KeyModifiers, PointerButton};
use lumencraft_ui_egui::{ControlAuth, ControlPostRequestContext, LOCALES, UiApp, locale_display_name};
use serde_json::json;

use crate::desktop_host::{
    CloseDisposition, DesktopFrameOutput, DesktopHostEvent, DesktopHostRuntime, DesktopMenuChromeState, DesktopMenuItem, DesktopWindowId,
    DesktopWindowSession, LiveControlFrameOutput,
};

/// Canonical file extensions accepted by native project/data `Open...` dialogs (`classify_and_load_path`).
pub const NATIVE_OPEN_FILE_EXTENSIONS: &[&str] = &["lumencraft", "lctemplate", "prism", "pzfx", "csv", "tsv"];
/// Canonical file extensions accepted by native template save and instantiate dialogs.
pub const NATIVE_TEMPLATE_FILE_EXTENSIONS: &[&str] = &["lctemplate"];
/// Default filename suggested when saving a reusable `.lctemplate` archive.
pub const DEFAULT_TEMPLATE_FILE_NAME: &str = "untitled.lctemplate";

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

/// Non-blocking channel and repaint/shutdown handles wiring a live control server into [`LumenCraftNativeApp`].
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
    if let Some(key) = insert_valid_sfnt(&mut defs, "lumencraft-minimal-fallback", &minimal_ttf) {
        proportional.push(key.clone());
        monospace.push(key);
    } else {
        let name = String::from("lumencraft-minimal-fallback");
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
        let Ok(bytes) = lumencraft_platform::fs::read_regular_file_bytes(Path::new(path), MAX_HOST_FONT_BYTES) else {
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

/// Interactive `eframe::App` wrapping a [`DesktopHostRuntime`] session.
pub struct LumenCraftNativeApp {
    runtime: DesktopHostRuntime,
    primary_window_id: DesktopWindowId,
    texture: Option<egui::TextureHandle>,
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
}

impl LumenCraftNativeApp {
    /// Construct a native app host wrapping an already-initialized [`DesktopHostRuntime`].
    pub fn new(runtime: DesktopHostRuntime, primary_window_id: DesktopWindowId, on_exit_cleanup: Option<CleanupHook>) -> Self {
        Self {
            runtime,
            primary_window_id,
            texture: None,
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
        let quit_requested = self.dispatch_pending_live_requests(pending);
        if quit_requested {
            self.close_confirmed = true;
            self.close_dialog_open = false;
            if let Some(ref ctrl) = self.live_control {
                ctrl.shutdown.store(true, Ordering::Release);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
            let win_id = self.active_window_id();
            let res = match self.runtime.window_mut(win_id) {
                Ok(win) => win.dispatch_live_control_frame_with_hook(&req.raw_bytes, &req.auth, req.conn_authenticated, |app, hook_ctx| {
                    if let Some(ref mut hook) = post_hook {
                        hook(app, hook_ctx)?;
                    }
                    Ok(())
                }),
                Err(err) => Err(err),
            };
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

    fn drain_pending_control_on_exit(&mut self) {
        let Some(ctrl) = self.live_control.take() else {
            return;
        };
        ctrl.shutdown.store(true, Ordering::Release);
        while let Ok(req) = ctrl.request_rx.try_recv() {
            let _ = req.reply_tx.send(Err(String::from("desktop window is closing")));
        }
    }

    /// Execute one UI frame (polling live control requests, handling OS events, and rendering).
    pub fn step_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let _ = self.poll_live_control(&ctx);
        let window_id = self.active_window_id();

        self.handle_close_request(&ctx, window_id);
        self.render_top_menu_bar(ui, window_id);

        egui::CentralPanel::no_frame().show(ui, |ui| {
            let canvas_rect = ui.available_rect_before_wrap();
            let ppp = ctx.pixels_per_point().max(1.0);
            self.sync_canvas_viewport(window_id, canvas_rect, ppp);
            self.process_canvas_input(ui, window_id, canvas_rect);

            if let Some(frame) = self.tick_animations_and_redraw(&ctx, window_id) {
                self.upload_and_paint_frame(ui, canvas_rect, &frame);
            }
        });

        self.render_unsaved_close_modal(&ctx, window_id);
        self.sync_window_title(&ctx, window_id);
    }

    /// Run shutdown cleanup, signal any attached live-control server to stop, and close all windows.
    pub fn finish_exit(&mut self) {
        self.drain_pending_control_on_exit();
        if let Some(cleanup) = self.on_exit_cleanup.take()
            && let Err(err) = cleanup(&mut self.runtime)
        {
            eprintln!("lumencraft shutdown cleanup warning: {err}");
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

    fn handle_close_request(&mut self, ctx: &egui::Context, window_id: DesktopWindowId) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.close_confirmed {
            return;
        }
        match self.runtime.dispatch(DesktopHostEvent::WindowCloseRequested { window_id, force: false }) {
            Ok(outcome) => {
                if matches!(outcome.close_disposition, Some(CloseDisposition::PromptUnsaved { .. })) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    self.close_dialog_open = true;
                } else {
                    self.close_confirmed = true;
                }
            }
            Err(err) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.status_banner = Some(err);
                self.close_dialog_open = true;
            }
        }
    }

    fn render_top_menu_bar(&mut self, ui: &mut egui::Ui, window_id: DesktopWindowId) {
        let Ok(win) = self.runtime.window(window_id) else {
            return;
        };
        if win.app().presentation_mode {
            return;
        }
        let chrome: DesktopMenuChromeState = win.chrome_state();
        if chrome.sections.is_empty() {
            return;
        }

        let mut pending_command: Option<String> = None;
        egui::Panel::top("lumencraft_menu_bar").show(ui, |ui| {
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
                if let Some(err) = self.status_banner.as_deref() {
                    ui.separator();
                    ui.colored_label(egui::Color32::from_rgb(200, 60, 60), err);
                }
            });
        });

        if let Some(command_id) = pending_command {
            self.execute_menu_command(window_id, &command_id);
        }
    }

    fn execute_menu_command(&mut self, window_id: DesktopWindowId, command_id: &str) {
        self.status_banner = None;
        let result = if let Some(native_result) = self.try_handle_native_file_command(window_id, command_id) {
            native_result
        } else {
            self.runtime.dispatch(DesktopHostEvent::MenuAction { window_id, command_id: command_id.to_string() }).map(|_| ())
        };
        if let Err(err) = result {
            self.status_banner = Some(err);
        }
    }

    fn try_handle_native_file_command(&mut self, window_id: DesktopWindowId, command_id: &str) -> Option<Result<(), String>> {
        let res = match command_id {
            "project.open" | "file.open" => self.handle_native_open(window_id),
            "project.save" | "file.save" => self.handle_native_save(window_id, false),
            "project.saveAs" | "file.save_as" => self.handle_native_save(window_id, true),
            "project.saveTemplate" => self.handle_native_save_template(window_id),
            "project.instantiateTemplate" => self.handle_native_instantiate_template(window_id),
            "table.importCsv" => self.handle_native_import_file(window_id, "table.importCsv", "Delimited Table", &["csv", "tsv", "txt"]),
            "table.importPrism" => self.handle_native_import_file(window_id, "table.importPrism", "External Archive", &["prism"]),
            "table.importPzfx" => self.handle_native_import_file(window_id, "table.importPzfx", "XML Project", &["pzfx"]),
            _ => {
                let spec = native_export_spec_for_command(command_id)?;
                self.handle_native_export(window_id, spec)
            }
        };
        Some(res)
    }

    fn handle_native_open(&mut self, window_id: DesktopWindowId) -> Result<(), String> {
        let Some(path) = rfd::FileDialog::new().add_filter("LumenCraft / Data", NATIVE_OPEN_FILE_EXTENSIONS).pick_file() else {
            return Ok(());
        };
        self.runtime.dispatch(DesktopHostEvent::FileDropped { window_id, path }).map(|_| ())
    }

    fn handle_native_save(&mut self, window_id: DesktopWindowId, force_dialog: bool) -> Result<(), String> {
        let existing_path = self.runtime.window(window_id).ok().and_then(|win| win.app().session().file_path().map(Path::to_path_buf));
        let target_path = if !force_dialog {
            match existing_path {
                Some(path) => path,
                None => match pick_save_lumencraft_path() {
                    Some(path) => path,
                    None => return Ok(()),
                },
            }
        } else {
            match pick_save_lumencraft_path() {
                Some(path) => path,
                None => return Ok(()),
            }
        };
        let win = self.runtime.window_mut(window_id)?;
        let path_str = target_path.to_string_lossy().into_owned();
        win.execute_and_journal("project.save", json!({ "path": path_str, "unpacked": false }))?;
        win.app_mut().session_mut().set_file_path(Some(target_path), true);
        Ok(())
    }

    fn handle_native_save_template(&mut self, window_id: DesktopWindowId) -> Result<(), String> {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("LumenCraft Template", NATIVE_TEMPLATE_FILE_EXTENSIONS)
            .set_file_name(DEFAULT_TEMPLATE_FILE_NAME)
            .save_file()
        else {
            return Ok(());
        };
        let win = self.runtime.window_mut(window_id)?;
        let path_str = path.to_string_lossy().into_owned();
        win.execute_and_journal("project.saveTemplate", json!({ "path": path_str }))?;
        Ok(())
    }

    fn handle_native_instantiate_template(&mut self, window_id: DesktopWindowId) -> Result<(), String> {
        let Some(path) = rfd::FileDialog::new().add_filter("LumenCraft Template", NATIVE_TEMPLATE_FILE_EXTENSIONS).pick_file() else {
            return Ok(());
        };
        let win = self.runtime.window_mut(window_id)?;
        execute_native_import_at_path(win, "project.instantiateTemplate", &path)
    }

    fn handle_native_import_file(
        &mut self,
        window_id: DesktopWindowId,
        command: &str,
        filter_label: &str,
        extensions: &[&str],
    ) -> Result<(), String> {
        let Some(path) = rfd::FileDialog::new().add_filter(filter_label, extensions).pick_file() else {
            return Ok(());
        };
        let win = self.runtime.window_mut(window_id)?;
        execute_native_import_at_path(win, command, &path)
    }

    fn handle_native_export(&mut self, window_id: DesktopWindowId, spec: NativeExportSpec) -> Result<(), String> {
        let target_sheet_id = {
            let win = self.runtime.window(window_id)?;
            resolve_native_export_sheet_id(win.app().document(), spec.target_kind)
                .ok_or_else(|| format!("no matching sheet available for `{}`", spec.command))?
        };
        let Some(path) = rfd::FileDialog::new().add_filter(spec.filter_label, spec.extensions).set_file_name(spec.default_name).save_file() else {
            return Ok(());
        };
        let win = self.runtime.window_mut(window_id)?;
        let path_str = path.to_string_lossy().into_owned();
        let params = if spec.command == "result.generateReport" {
            let report_fmt = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
                Some("tex") => "latex",
                Some("txt") => "plain",
                _ => "markdown",
            };
            json!({ "result_sheet_id": target_sheet_id.as_str(), "format": report_fmt, "path": path_str })
        } else {
            json!({ "sheet_id": target_sheet_id.as_str(), "path": path_str })
        };
        win.execute_and_journal(spec.command, params)?;
        Ok(())
    }

    fn render_unsaved_close_modal(&mut self, ctx: &egui::Context, window_id: DesktopWindowId) {
        if !self.close_dialog_open {
            return;
        }
        egui::Modal::new(egui::Id::unique("lumencraft_unsaved_close_modal")).show(ctx, |ui| {
            ui.heading("Unsaved Changes");
            ui.label("Save changes to this project before closing?");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    match self.handle_native_save(window_id, false) {
                        Ok(()) => {
                            let still_dirty = self.runtime.window(window_id).map(|w| w.has_unsaved_changes()).unwrap_or(false);
                            if !still_dirty {
                                self.close_dialog_open = false;
                                self.close_confirmed = true;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        Err(err) => {
                            self.status_banner = Some(err);
                        }
                    }
                }
                if ui.button("Don't Save").clicked() {
                    let _ = self.runtime.dispatch(DesktopHostEvent::WindowCloseRequested { window_id, force: true });
                    self.close_dialog_open = false;
                    self.close_confirmed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.button("Cancel").clicked() {
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

        if (zoom_delta - 1.0).abs() > 1e-4
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
            let _ = self.runtime.dispatch(DesktopHostEvent::FileDropped { window_id, path: file.path().to_path_buf() });
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

        for event in events {
            match event {
                egui::Event::PointerMoved(pos) => {
                    if canvas_rect.contains(*pos) || self.pointer_captured {
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
                    self.dispatch_pointer_button(CanvasPointerButtonArgs {
                        window_id,
                        canvas_rect,
                        pos: *pos,
                        button: *button,
                        pressed: *pressed,
                        modifiers: *modifiers,
                    });
                }
                egui::Event::MouseWheel { unit, delta, modifiers, .. } => {
                    let anchor = hover_pos.unwrap_or(canvas_rect.center());
                    if canvas_rect.contains(anchor) {
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
                    if let Some(inserted) = self.dispatch_key_event(window_id, *key, *pressed, *modifiers) {
                        suppressed_text_char = Some(inserted);
                    }
                }
                egui::Event::Text(text) => {
                    if suppressed_text_char.as_deref() == Some(text.as_str()) {
                        suppressed_text_char = None;
                        continue;
                    }
                    let (inline_active, _) = self.inline_editor_snapshot(window_id);
                    if inline_active {
                        let _ = self.runtime.dispatch(DesktopHostEvent::ImeCommit { window_id, text: text.clone() });
                    }
                }
                egui::Event::Ime(ime) => {
                    self.dispatch_ime_event(window_id, ime);
                }
                egui::Event::Copy => {
                    self.dispatch_clipboard_copy(ctx, window_id);
                }
                egui::Event::Cut => {
                    self.dispatch_clipboard_copy(ctx, window_id);
                    let _ = self.runtime.dispatch(DesktopHostEvent::KeyboardInput {
                        window_id,
                        key: String::from("Backspace"),
                        pressed: true,
                        modifiers: KeyModifiers::default(),
                        text: None,
                    });
                }
                egui::Event::Paste(text) => {
                    let _ = self.runtime.dispatch(DesktopHostEvent::ClipboardPaste { window_id, text_override: Some(text.clone()) });
                }
                _ => {}
            }
        }
    }

    fn dispatch_clipboard_copy(&mut self, ctx: &egui::Context, window_id: DesktopWindowId) {
        if self.runtime.dispatch(DesktopHostEvent::ClipboardCopy { window_id }).is_ok()
            && let Some(copied) = self.runtime.clipboard_text()
        {
            ctx.copy_text(copied.to_owned());
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
            let _ = self.runtime.dispatch(DesktopHostEvent::PointerButton {
                window_id: args.window_id,
                x_pt,
                y_pt,
                button: to_pointer_button(args.button),
                pressed: false,
                click_count: self.click_count,
                modifiers: to_key_modifiers(args.modifiers),
            });
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

    fn dispatch_key_event(&mut self, window_id: DesktopWindowId, key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> Option<String> {
        let key_name = egui_key_to_name(key)?;
        let (inline_active_before, inline_text_before) = self.inline_editor_snapshot(window_id);
        let dialog_inactive =
            self.runtime.window(window_id).is_ok_and(|w| w.app().canvas_dialogs.active_dialog == ActiveDialog::None && !w.app().nav.welcome.open);
        let primary_cmd = modifiers.command || modifiers.mac_cmd || modifiers.ctrl;
        if pressed && !inline_active_before && dialog_inactive && primary_cmd && !modifiers.alt {
            if key == egui::Key::O && !modifiers.shift {
                self.execute_menu_command(window_id, "project.open");
                return None;
            }
            if key == egui::Key::S {
                let cmd = if modifiers.shift { "project.saveAs" } else { "project.save" };
                self.execute_menu_command(window_id, cmd);
                return None;
            }
        }
        let has_cmd_or_ctrl = modifiers.ctrl || modifiers.alt || modifiers.command || modifiers.mac_cmd;
        let is_single_char_key = key_name.chars().count() == 1;

        if inline_active_before && pressed && is_single_char_key && !has_cmd_or_ctrl {
            // Let `egui::Event::Text` handle printable character insertion once when inline editor is active.
            return None;
        }

        let _ = self.runtime.dispatch(DesktopHostEvent::KeyboardInput {
            window_id,
            key: key_name,
            pressed,
            modifiers: to_key_modifiers(modifiers),
            text: None,
        });

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

    fn upload_and_paint_frame(&mut self, ui: &mut egui::Ui, canvas_rect: egui::Rect, frame: &DesktopFrameOutput) {
        let size = [frame.width_px as usize, frame.height_px as usize];
        if size[0] == 0 || size[1] == 0 || frame.rgba_bytes.len() < size[0] * size[1] * 4 {
            return;
        }
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba_bytes);
        let tex_id = match &mut self.texture {
            Some(tex) => {
                tex.set(color_image, egui::TextureOptions::NEAREST);
                tex.id()
            }
            None => {
                let tex = ui.ctx().load_texture("lumencraft_canvas_rgba", color_image, egui::TextureOptions::NEAREST);
                let id = tex.id();
                self.texture = Some(tex);
                id
            }
        };
        let _response = ui.allocate_rect(canvas_rect, egui::Sense::click_and_drag());
        ui.painter().image(tex_id, canvas_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

impl Drop for LumenCraftNativeApp {
    fn drop(&mut self) {
        self.drain_pending_control_on_exit();
    }
}

impl eframe::App for LumenCraftNativeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.step_ui(ui);
    }

    fn on_exit(&mut self) {
        self.finish_exit();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeExportTargetKind {
    Renderable,
    Graph,
    Layout,
    TableOrResult,
    Result,
}

#[derive(Debug, Clone, Copy)]
struct NativeExportSpec {
    command: &'static str,
    filter_label: &'static str,
    extensions: &'static [&'static str],
    default_name: &'static str,
    target_kind: NativeExportTargetKind,
}

fn native_export_spec_for_command(command_id: &str) -> Option<NativeExportSpec> {
    let spec = match command_id {
        "export.svg" | "file.export_svg" => NativeExportSpec {
            command: "export.svg",
            filter_label: "SVG Vector",
            extensions: &["svg"],
            default_name: "figure.svg",
            target_kind: NativeExportTargetKind::Renderable,
        },
        "export.pdf" | "file.export_pdf" => NativeExportSpec {
            command: "export.pdf",
            filter_label: "PDF Document",
            extensions: &["pdf"],
            default_name: "figure.pdf",
            target_kind: NativeExportTargetKind::Renderable,
        },
        "export.png" | "file.export_png" => NativeExportSpec {
            command: "export.png",
            filter_label: "PNG Image",
            extensions: &["png"],
            default_name: "figure.png",
            target_kind: NativeExportTargetKind::Renderable,
        },
        "export.tiff" => NativeExportSpec {
            command: "export.tiff",
            filter_label: "TIFF Image",
            extensions: &["tiff", "tif"],
            default_name: "figure.tiff",
            target_kind: NativeExportTargetKind::Renderable,
        },
        "graph.exportSvg" => NativeExportSpec {
            command: "graph.exportSvg",
            filter_label: "SVG Vector",
            extensions: &["svg"],
            default_name: "figure.svg",
            target_kind: NativeExportTargetKind::Graph,
        },
        "graph.exportPdf" => NativeExportSpec {
            command: "graph.exportPdf",
            filter_label: "PDF Document",
            extensions: &["pdf"],
            default_name: "figure.pdf",
            target_kind: NativeExportTargetKind::Graph,
        },
        "graph.exportPng" => NativeExportSpec {
            command: "graph.exportPng",
            filter_label: "PNG Image",
            extensions: &["png"],
            default_name: "figure.png",
            target_kind: NativeExportTargetKind::Graph,
        },
        "graph.exportEmf" => NativeExportSpec {
            command: "graph.exportEmf",
            filter_label: "EMF Vector",
            extensions: &["emf"],
            default_name: "figure.emf",
            target_kind: NativeExportTargetKind::Graph,
        },
        "layout.exportEmf" => NativeExportSpec {
            command: "layout.exportEmf",
            filter_label: "EMF Vector",
            extensions: &["emf"],
            default_name: "layout.emf",
            target_kind: NativeExportTargetKind::Layout,
        },
        "table.exportCsv" => NativeExportSpec {
            command: "table.exportCsv",
            filter_label: "CSV Table",
            extensions: &["csv", "tsv"],
            default_name: "table.csv",
            target_kind: NativeExportTargetKind::TableOrResult,
        },
        "analysis.exportJson" => NativeExportSpec {
            command: "analysis.exportJson",
            filter_label: "JSON Results",
            extensions: &["json"],
            default_name: "analysis.json",
            target_kind: NativeExportTargetKind::Result,
        },
        "analysis.exportCsv" => NativeExportSpec {
            command: "analysis.exportCsv",
            filter_label: "CSV Results",
            extensions: &["csv"],
            default_name: "analysis.csv",
            target_kind: NativeExportTargetKind::Result,
        },
        "result.generateReport" => NativeExportSpec {
            command: "result.generateReport",
            filter_label: "Methods Report",
            extensions: &["md", "txt", "tex"],
            default_name: "methods-report.md",
            target_kind: NativeExportTargetKind::Result,
        },
        _ => return None,
    };
    Some(spec)
}

/// Returns `true` when clicking `command_id` in the native desktop window opens an OS `rfd::FileDialog`.
#[must_use]
pub fn menu_action_triggers_native_file_dialog(command_id: &str) -> bool {
    matches!(
        command_id,
        "project.open"
            | "file.open"
            | "project.save"
            | "file.save"
            | "project.saveAs"
            | "file.save_as"
            | "project.saveTemplate"
            | "project.instantiateTemplate"
            | "table.importCsv"
            | "table.importPrism"
            | "table.importPzfx"
    ) || native_export_spec_for_command(command_id).is_some()
}

/// Returns `true` when an import command replaces the full active document and must detach any
/// previously associated `.lumencraft` archive path so subsequent `Cmd+S` prompts cleanly.
#[must_use]
pub fn native_import_replaces_project_file_path(command_id: &str) -> bool {
    matches!(command_id, "project.instantiateTemplate" | "table.importPrism" | "table.importPzfx")
}

/// Execute a file-backed import or template instantiation command at `path`, clearing the active
/// session's archive association when the command replaces the full document.
///
/// # Errors
/// Returns an error string if executing the underlying engine command fails.
pub fn execute_native_import_at_path(win: &mut DesktopWindowSession, command: &str, path: &Path) -> Result<(), String> {
    let path_str = path.to_string_lossy().into_owned();
    win.execute_and_journal(command, json!({ "path": path_str }))?;
    if native_import_replaces_project_file_path(command) {
        win.app_mut().session_mut().set_file_path(None, false);
    }
    Ok(())
}

fn resolve_native_export_sheet_id(doc: &ProjectDocument, kind: NativeExportTargetKind) -> Option<SheetId> {
    let active = doc.active_sheet_id.as_ref();
    match kind {
        NativeExportTargetKind::Renderable => active
            .filter(|id| matches!(doc.sheet_section(id), Some(SheetSection::Graphs | SheetSection::Layouts)))
            .cloned()
            .or_else(|| doc.graph_sheets.first().map(|g| g.id.clone()))
            .or_else(|| doc.layout_sheets.first().map(|l| l.id.clone())),
        NativeExportTargetKind::Graph => {
            active.filter(|id| doc.graph_sheet(id).is_some()).cloned().or_else(|| doc.graph_sheets.first().map(|g| g.id.clone()))
        }
        NativeExportTargetKind::Layout => {
            active.filter(|id| doc.layout_sheet(id).is_some()).cloned().or_else(|| doc.layout_sheets.first().map(|l| l.id.clone()))
        }
        NativeExportTargetKind::TableOrResult => active
            .filter(|id| doc.data_sheet(id).is_some() || doc.result_sheet(id).is_some())
            .cloned()
            .or_else(|| doc.data_sheets.first().map(|d| d.id.clone()))
            .or_else(|| doc.result_sheets.first().map(|r| r.id.clone())),
        NativeExportTargetKind::Result => {
            active.filter(|id| doc.result_sheet(id).is_some()).cloned().or_else(|| doc.result_sheets.first().map(|r| r.id.clone()))
        }
    }
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
            for &loc in LOCALES {
                let caption = format!("{} ({loc})", locale_display_name(loc));
                if ui.button(caption).on_hover_text(&tooltip).clicked() {
                    *pending_command = Some(format!("ui.locale.set:{loc}"));
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

fn pick_save_lumencraft_path() -> Option<PathBuf> {
    rfd::FileDialog::new().add_filter("LumenCraft Project", &["lumencraft"]).set_file_name("untitled.lumencraft").save_file()
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
    let initial_title = runtime.window(primary_window_id).map(|w| w.window_title()).unwrap_or_else(|_| String::from("LumenCraft"));

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([DEFAULT_WINDOW_WIDTH_PT, DEFAULT_WINDOW_HEIGHT_PT])
            .with_min_inner_size([MIN_WINDOW_WIDTH_PT, MIN_WINDOW_HEIGHT_PT])
            .with_title(initial_title),
        ..Default::default()
    };

    let font_bytes = extra_ttf;
    eframe::run_native(
        "LumenCraft",
        native_options,
        Box::new(move |cc| {
            let fonts = build_egui_font_definitions(font_bytes.as_deref());
            cc.egui_ctx.set_fonts(fonts);
            let mut app = LumenCraftNativeApp::new(runtime, primary_window_id, Some(on_exit_cleanup));
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
    use lumencraft_ui_egui::interaction::ViewportMetrics;
    use serde_json::json;

    use super::*;
    use crate::desktop_host::DesktopHostConfig;

    fn test_runtime(viewport: ViewportMetrics) -> Result<(DesktopHostRuntime, DesktopWindowId), String> {
        let mut runtime = DesktopHostRuntime::new(DesktopHostConfig {
            in_window_menus: true,
            new_instance: true,
            gui_only: false,
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
        let mut app = LumenCraftNativeApp::new(runtime, id, None);

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
        let mut app = LumenCraftNativeApp::new(runtime, id, None);
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
        let mut app = LumenCraftNativeApp::new(runtime, id, None);
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
        let mut app = LumenCraftNativeApp::new(runtime, id, None);
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
}
