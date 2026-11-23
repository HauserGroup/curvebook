//! Native desktop entry point for CurveBook.
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::{BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use curvebook::desktop_host::{
    DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, DesktopWindowId, DesktopWindowSession, LiveControlFrameOutput,
    allocate_fresh_isolated_journal_dir, dir_has_active_lease, dir_has_unrecovered_crash, open_document_from_files,
};
use curvebook::gui_window::{LiveControlChannel, LiveControlPostHook, run_native_desktop_with_control};
use curvebook::live_control::{
    LiveControlBridge, LiveControlServerGuard, serve_live_control_io, spawn_live_control_accept_loop_with_reject, write_busy_control_error,
};
use curvebook_engine::Session;
use curvebook_format::recovery::{CrashRecoveryJournal, DirectoryJournalBackend, JournalBackend, RecoveryReport};
use curvebook_format::versioned::{
    CURRENT_PREFERENCES_VERSION, MAX_PREFERENCES_FILE_BYTES, PreferencesFile, load_preferences_file, save_preferences_file,
};
use curvebook_render::RenderTargetSpec;
use curvebook_text::{TrueTypeFontInfo, parse_truetype_font_info};
use curvebook_ui_egui::control_io::CONTROL_CLIENT_IDLE_TIMEOUT;
use curvebook_ui_egui::egui_shell::UiBackend;
use curvebook_ui_egui::interaction::ViewportMetrics;
use curvebook_ui_egui::theme::install_fonts;
use curvebook_ui_egui::{
    ControlAuth, ControlPostRequestContext, DEFAULT_UI_SCALE, ScreenshotRegionTarget, UiApp, UiThemeMode, bind_control_tcp_listener,
    extract_ui_scale_from_preferences_extra, normalize_beta_gui_locale, normalize_locale, parse_control_port, serve_control_listener_with_hook,
    validate_loopback_addr, write_ui_scale_to_preferences_extra,
};

const RECOVERY_LOCK_TTL_MS: u64 = 300_000;
const MAX_FONT_FILE_BYTES: usize = 64 * 1024 * 1024;
const MAX_EVENT_SCRIPT_BYTES: usize = 64 * 1024 * 1024;

const USAGE: &str = "\
usage: curvebook [options] [--] [file...]

options:
  --control <port>         listen on 127.0.0.1:<port> for authenticated JSON-lines automation (or CURVEBOOK_CONTROL_PORT; headless unless --window)
  --control-token <token>  bearer token for --control authentication (prefer CURVEBOOK_CONTROL_TOKEN on multi-user systems; CLI args are visible in ps)
  --window                 attach a visible desktop window to --control <port> or CURVEBOOK_CONTROL_PORT
  --gui-only               restrict automation (--control) to interactive GUI actions only
  --ui-backend <name>      select UI backend (`egui` sole desktop backend; `legacy` is rejected; overrides CURVEBOOK_UI_BACKEND)
  --in-window-menus     render the menu bar inside the window on macOS
  --new-instance        open files in a separate application window
  --headless-once       render one egui software frame and exit
  --portable-dir <dir>  store config, shortcuts, and crash-recovery journal under <dir>/curvebook-data
  --ttf-font <path>     load and validate an external TrueType/SFNT (.ttf) font file
  --rulers              show physical canvas rulers on startup
  --event-script <path> replay newline-delimited DesktopHostEvent JSONL commands
  --                    stop parsing options; treat remaining arguments as file paths
  -V, --version         print version and exit
  -h, --help            print this help message and exit
";

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    PrintVersion,
    PrintHelp,
    Launch(LaunchConfig),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LaunchConfig {
    control_port: Option<u16>,
    control_token: Option<String>,
    window: bool,
    gui_only: bool,
    ui_backend: UiBackend,
    in_window_menus: bool,
    new_instance: bool,
    headless_once: bool,
    portable_dir: Option<PathBuf>,
    ttf_font: Option<PathBuf>,
    rulers: bool,
    event_script: Option<PathBuf>,
    files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppPaths {
    portable: bool,
    config_dir: PathBuf,
    data_dir: PathBuf,
    preferences_file: PathBuf,
    shortcuts_file: PathBuf,
    recovery_dir: PathBuf,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env_port = std::env::var("CURVEBOOK_CONTROL_PORT").ok();
    let env_token = std::env::var("CURVEBOOK_CONTROL_TOKEN").ok();
    match run_with_process_env(&args, env_port.as_deref(), env_token.as_deref(), |key| std::env::var_os(key)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
fn run(args: &[String]) -> Result<(), String> {
    run_with_process_env(args, None, None, |_| None)
}

fn run_with_process_env(
    args: &[String],
    env_port: Option<&str>,
    env_token: Option<&str>,
    env_get: impl Fn(&str) -> Option<OsString>,
) -> Result<(), String> {
    let mut stderr = std::io::stderr();
    run_with_process_env_and_stderr(args, env_port, env_token, env_get, &mut stderr)
}

fn run_with_process_env_and_stderr(
    args: &[String],
    env_port: Option<&str>,
    env_token: Option<&str>,
    env_get: impl Fn(&str) -> Option<OsString>,
    stderr: &mut dyn Write,
) -> Result<(), String> {
    let env_backend_os = env_get("CURVEBOOK_UI_BACKEND");
    let env_backend = match env_backend_os {
        Some(raw) => Some(raw.into_string().map_err(|_| String::from("non-UTF-8 CURVEBOOK_UI_BACKEND environment variable"))?),
        None => None,
    };
    let action = parse_args_with_env_and_backend(args, env_port, env_token, env_backend.as_deref())?;
    match action {
        CliAction::PrintVersion => {
            println!("CurveBook {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        CliAction::PrintHelp => {
            print!("{USAGE}");
            Ok(())
        }
        CliAction::Launch(cfg) => {
            if args_contain_cli_control_token(args) {
                let _ = writeln!(
                    stderr,
                    "curvebook warning: --control-token exposes the secret in process arguments (ps); prefer CURVEBOOK_CONTROL_TOKEN on multi-user systems"
                );
            }
            warn_or_reject_non_utf8_control_env(&cfg, env_port, env_token, &env_get)?;
            launch_application_with_env(&cfg, env_get)
        }
    }
}

fn args_contain_cli_control_token(args: &[String]) -> bool {
    let mut i = 0usize;
    while let Some(arg) = args.get(i) {
        match arg.as_str() {
            "--" => return false,
            "--control-token" => return true,
            "--control" | "--portable-dir" | "--ttf-font" | "--event-script" | "--ui-backend" => i += 2,
            _ => i += 1,
        }
    }
    false
}

fn warn_or_reject_non_utf8_control_env(
    cfg: &LaunchConfig,
    env_port: Option<&str>,
    env_token: Option<&str>,
    env_get: &impl Fn(&str) -> Option<OsString>,
) -> Result<(), String> {
    if cfg.control_port.is_none()
        && env_port.is_none()
        && let Some(raw_os) = env_get("CURVEBOOK_CONTROL_PORT")
        && raw_os.to_str().is_none()
    {
        eprintln!("curvebook warning: ignoring non-UTF-8 CURVEBOOK_CONTROL_PORT");
    }
    if cfg.control_token.is_none()
        && env_token.is_none()
        && let Some(raw_os) = env_get("CURVEBOOK_CONTROL_TOKEN")
        && raw_os.to_str().is_none()
    {
        if cfg.control_port.is_some() {
            return Err(String::from("invalid CURVEBOOK_CONTROL_TOKEN: must be valid UTF-8"));
        }
        eprintln!("curvebook warning: ignoring non-UTF-8 CURVEBOOK_CONTROL_TOKEN");
    }
    Ok(())
}

#[cfg(test)]
fn parse_args(args: &[String], env_port: Option<&str>) -> Result<CliAction, String> {
    parse_args_with_env(args, env_port, None)
}

#[cfg(test)]
fn parse_args_with_env(args: &[String], env_port: Option<&str>, env_token: Option<&str>) -> Result<CliAction, String> {
    parse_args_with_env_and_backend(args, env_port, env_token, None)
}

fn parse_args_with_env_and_backend(
    args: &[String],
    env_port: Option<&str>,
    env_token: Option<&str>,
    env_ui_backend: Option<&str>,
) -> Result<CliAction, String> {
    let mut cfg = LaunchConfig::default();
    let mut cli_ui_backend: Option<String> = None;
    let mut i = 0usize;
    while let Some(arg) = args.get(i) {
        match arg.as_str() {
            "-V" | "--version" => return Ok(CliAction::PrintVersion),
            "-h" | "--help" => return Ok(CliAction::PrintHelp),
            "--control" => {
                let raw = args.get(i + 1).ok_or_else(|| String::from("--control requires a TCP port (1..=65535)"))?;
                cfg.control_port = Some(parse_port(raw)?);
                i += 2;
            }
            "--control-token" => {
                let raw = args.get(i + 1).ok_or_else(|| String::from("--control-token requires a non-empty token"))?;
                let validated = ControlAuth::from_token(raw.as_str()).map_err(|err| format!("invalid --control-token: {err}"))?;
                cfg.control_token = Some(validated.as_str().to_owned());
                i += 2;
            }
            "--window" => {
                cfg.window = true;
                i += 1;
            }
            "--gui-only" => {
                cfg.gui_only = true;
                i += 1;
            }
            "--ui-backend" => {
                let raw = args.get(i + 1).ok_or_else(|| String::from("--ui-backend requires `egui` (`legacy` has been removed)"))?;
                cli_ui_backend = Some(raw.clone());
                i += 2;
            }
            flag if flag.starts_with("--ui-backend=") => {
                let raw = flag.trim_start_matches("--ui-backend=");
                cli_ui_backend = Some(raw.to_owned());
                i += 1;
            }
            "--in-window-menus" => {
                cfg.in_window_menus = true;
                i += 1;
            }
            "--new-instance" => {
                cfg.new_instance = true;
                i += 1;
            }
            "--headless-once" => {
                cfg.headless_once = true;
                i += 1;
            }
            "--portable-dir" => {
                let raw = args.get(i + 1).ok_or_else(|| String::from("--portable-dir requires a directory path"))?;
                if raw.trim().is_empty() {
                    return Err(String::from("--portable-dir requires a non-empty directory path"));
                }
                cfg.portable_dir = Some(PathBuf::from(raw));
                i += 2;
            }
            "--ttf-font" => {
                let raw = args.get(i + 1).ok_or_else(|| String::from("--ttf-font requires a .ttf font file path"))?;
                if raw.trim().is_empty() {
                    return Err(String::from("--ttf-font requires a non-empty file path"));
                }
                cfg.ttf_font = Some(PathBuf::from(raw));
                i += 2;
            }
            "--rulers" => {
                cfg.rulers = true;
                i += 1;
            }
            "--event-script" => {
                let raw = args.get(i + 1).ok_or_else(|| String::from("--event-script requires a JSONL file path"))?;
                if raw.trim().is_empty() {
                    return Err(String::from("--event-script requires a non-empty file path"));
                }
                cfg.event_script = Some(PathBuf::from(raw));
                i += 2;
            }
            "--" => {
                if let Some(rest) = args.get(i + 1..) {
                    cfg.files.extend(rest.iter().map(PathBuf::from));
                }
                break;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("unknown option `{flag}` (use `--` before file paths starting with `-`)\n\n{USAGE}"));
            }
            path => {
                cfg.files.push(PathBuf::from(path));
                i += 1;
            }
        }
    }

    cfg.ui_backend = UiBackend::from_env_and_args(env_ui_backend, cli_ui_backend.as_deref())?;
    apply_env_control_overrides(&mut cfg, env_port, env_token)?;
    validate_launch_config(&cfg)?;

    Ok(CliAction::Launch(cfg))
}

fn validate_launch_config(cfg: &LaunchConfig) -> Result<(), String> {
    if cfg.window && cfg.headless_once {
        return Err(String::from("--window cannot be combined with --headless-once"));
    }
    Ok(())
}

fn apply_env_control_overrides(cfg: &mut LaunchConfig, env_port: Option<&str>, env_token: Option<&str>) -> Result<(), String> {
    if cfg.control_port.is_none()
        && let Some(raw_env) = env_port.map(str::trim).filter(|s| !s.is_empty())
    {
        match parse_port(raw_env) {
            Ok(port) => cfg.control_port = Some(port),
            Err(err) => {
                eprintln!("curvebook warning: ignoring invalid CURVEBOOK_CONTROL_PORT `{raw_env}`: {err}");
            }
        }
    }
    if cfg.control_token.is_none()
        && let Some(raw_tok) = env_token.map(str::trim).filter(|s| !s.is_empty())
    {
        match ControlAuth::from_token(raw_tok) {
            Ok(validated) => cfg.control_token = Some(validated.as_str().to_owned()),
            Err(err) if cfg.control_port.is_some() => {
                return Err(format!("invalid CURVEBOOK_CONTROL_TOKEN: {err}"));
            }
            Err(err) => {
                eprintln!("curvebook warning: ignoring invalid CURVEBOOK_CONTROL_TOKEN: {err}");
            }
        }
    }
    Ok(())
}

fn can_create_recovery_root(path: &Path) -> Result<bool, String> {
    if path.exists() {
        return Ok(true);
    }
    match std::fs::create_dir_all(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => Ok(false),
        Err(err) => Err(format!("failed to create recovery directory `{}`: {err}", path.display())),
    }
}

struct RuntimeShutdownGuard {
    runtime: Option<DesktopHostRuntime>,
}

impl RuntimeShutdownGuard {
    fn new(runtime: DesktopHostRuntime) -> Self {
        Self { runtime: Some(runtime) }
    }

    fn get_mut(&mut self) -> Result<&mut DesktopHostRuntime, String> {
        self.runtime.as_mut().ok_or_else(|| String::from("desktop runtime already consumed"))
    }

    fn take(&mut self) -> Result<DesktopHostRuntime, String> {
        self.runtime.take().ok_or_else(|| String::from("desktop runtime already consumed"))
    }

    fn close_cleanly(&mut self) -> Result<(), String> {
        match self.runtime.take() {
            Some(mut rt) => rt.close_all_force(),
            None => Ok(()),
        }
    }
}

impl Drop for RuntimeShutdownGuard {
    fn drop(&mut self) {
        if let Some(mut rt) = self.runtime.take() {
            let _ = rt.close_all_force();
        }
    }
}

fn configure_window_preferences(app: &mut UiApp, app_paths: &AppPaths, cfg: &LaunchConfig, no_prefs: bool) {
    if !no_prefs && let Err(err) = load_shortcut_preferences(app, app_paths) {
        eprintln!("curvebook preferences warning: {err}; using default preferences");
        app.shortcuts.reset_all();
    }
    if cfg.rulers {
        app.canvas_dialogs.rulers_visible = true;
    }
    curvebook::configure_gui_only(app, cfg.gui_only);
    app.sync_views();
}

fn persist_window_preferences_best_effort(app: &UiApp, app_paths: &AppPaths, no_prefs: bool) {
    if no_prefs {
        return;
    }
    match should_persist_preferences(app, app_paths) {
        Ok(true) => {
            if let Err(err) = save_shortcut_preferences(app, app_paths) {
                eprintln!("curvebook preferences warning: {err}; leaving preferences untouched");
            }
        }
        Ok(false) => {}
        Err(err) => {
            eprintln!("curvebook preferences warning: {err}; leaving preferences untouched");
        }
    }
}

#[cfg(test)]
fn launch_application(cfg: &LaunchConfig) -> Result<(), String> {
    launch_application_with_env(cfg, |_| None)
}

fn should_launch_window(cfg: &LaunchConfig) -> bool {
    !cfg.headless_once && (cfg.window || (cfg.event_script.is_none() && cfg.control_port.is_none()))
}

fn launch_application_with_env(cfg: &LaunchConfig, env_get: impl Fn(&str) -> Option<OsString>) -> Result<(), String> {
    validate_launch_config(cfg)?;
    let _loaded_fonts = install_fonts();
    let _ = curvebook_ui_egui::bundled_fonts_enabled();
    let _ = curvebook_format::EXTENSION;

    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let no_prefs = is_no_prefs_enabled(&env_get);
    let app_paths = resolve_app_paths_with_env(exe_dir.as_deref(), cfg.portable_dir.as_deref(), &env_get)?;
    let _validated_font = cfg.ttf_font.as_deref().map(load_validated_ttf_font).transpose()?;

    let launch_window = should_launch_window(cfg);
    let enable_recovery = !no_prefs && (app_paths.portable || !cfg.headless_once);
    let initial_viewport = if launch_window { ViewportMetrics::new(1280.0, 800.0, 2.0) } else { ViewportMetrics::new(400.0, 260.0, 2.0) };
    let recovery_root =
        if enable_recovery && can_create_recovery_root(&app_paths.recovery_dir)? { Some(app_paths.recovery_dir.clone()) } else { None };

    let host_cfg = DesktopHostConfig {
        in_window_menus: cfg.in_window_menus,
        new_instance: cfg.new_instance,
        gui_only: cfg.gui_only,
        ui_backend: cfg.ui_backend,
        locale_override: None,
        theme_override: None,
        recovery_root: recovery_root.clone(),
        initial_viewport,
        ..DesktopHostConfig::default()
    };
    let mut runtime_guard = RuntimeShutdownGuard::new(DesktopHostRuntime::new(host_cfg));

    let initial_win_id = if cfg.new_instance {
        let session = load_startup_session(&cfg.files)?;
        let attach_recovery = recovery_root.is_some();
        runtime_guard.get_mut()?.open_window_with_session(session, initial_viewport, attach_recovery)?
    } else {
        let startup = open_startup_session_with_recovery(&cfg.files, &app_paths, enable_recovery, now_epoch_ms())?;
        let recovery_dir = startup.recovery_dir.unwrap_or_else(|| app_paths.recovery_dir.clone());
        if let Some(journal) = startup.journal {
            runtime_guard.get_mut()?.open_window_with_recovery(startup.session, initial_viewport, recovery_dir, journal, startup.report)?
        } else {
            runtime_guard.get_mut()?.open_window_with_session(startup.session, initial_viewport, false)?
        }
    };
    if enable_recovery && !cfg.headless_once {
        let _ = runtime_guard.get_mut()?.recover_stale_journals(initial_viewport, now_epoch_ms())?;
        if !cfg.files.is_empty() {
            let _ = runtime_guard.get_mut()?.focus_window(initial_win_id);
        }
    }

    let window_ids = runtime_guard.get_mut()?.window_ids();
    for win_id in window_ids {
        let win = runtime_guard.get_mut()?.window_mut(win_id)?;
        configure_window_preferences(win.app_mut(), &app_paths, cfg, no_prefs);
    }
    if !no_prefs {
        let persist_paths = app_paths.clone();
        runtime_guard.get_mut()?.set_preferences_persist_hook(move |app| {
            persist_window_preferences_best_effort(app, &persist_paths, false);
        });
    }

    if let Some(script_path) = cfg.event_script.as_deref() {
        let script = curvebook_platform::fs::read_regular_file_utf8(script_path, MAX_EVENT_SCRIPT_BYTES)
            .map_err(|err| format!("failed to read --event-script `{}`: {err}", script_path.display()))?;
        let _ = runtime_guard.get_mut()?.run_event_script_jsonl(&script)?;
    }

    if launch_window {
        let extra_ttf_bytes = cfg
            .ttf_font
            .as_deref()
            .map(|path| {
                curvebook_platform::fs::read_regular_file_bytes(path, MAX_FONT_FILE_BYTES)
                    .map_err(|err| format!("failed to read `--ttf-font` `{}`: {err}", path.display()))
            })
            .transpose()?;
        let cleanup_paths = app_paths.clone();
        let cleanup = Box::new(move |rt: &mut DesktopHostRuntime| -> Result<(), String> {
            let focused_id = rt.focused_window_id();
            for win_id in rt.window_ids() {
                if Some(win_id) != focused_id
                    && let Ok(win) = rt.window(win_id)
                {
                    persist_window_preferences_best_effort(win.app(), &cleanup_paths, no_prefs);
                }
            }
            if let Some(focused_id) = focused_id
                && let Ok(win) = rt.window(focused_id)
            {
                persist_window_preferences_best_effort(win.app(), &cleanup_paths, no_prefs);
            }
            Ok(())
        });
        let (live_guards, live_control) = match cfg.control_port {
            Some(port) => {
                let (guards, channel) = start_live_window_control_server(port, cfg, &app_paths, no_prefs)?;
                (Some(guards), Some(channel))
            }
            None => (None, None),
        };
        let runtime = runtime_guard.take()?;
        let res = run_native_desktop_with_control(runtime, extra_ttf_bytes, cleanup, live_control);
        drop(live_guards);
        return res;
    }

    if cfg.headless_once {
        let headless_ids = runtime_guard.get_mut()?.window_ids();
        for win_id in headless_ids {
            let redraw = runtime_guard.get_mut()?.dispatch(DesktopHostEvent::RedrawRequested { window_id: win_id })?;
            let frame = redraw.frame.ok_or_else(|| format!("missing headless frame for window `{win_id}`"))?;
            if frame.rgba_bytes.is_empty() || frame.width_px == 0 || frame.height_px == 0 {
                return Err(String::from("headless desktop host produced empty RGBA framebuffer"));
            }
            let rt = runtime_guard.get_mut()?;
            let win = rt.window_mut(win_id)?;
            let (png, _, _) = win
                .app_mut()
                .render_egui_shell_screenshot_png(RenderTargetSpec { width_px: 800, height_px: 520, dpi: 144 }, ScreenshotRegionTarget::FullWindow)
                .map_err(|err| format!("headless frame render failed: {err}"))?;
            if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err(String::from("headless egui rasterizer produced invalid PNG signature"));
            }
        }
    }

    if let Some(port) = cfg.control_port {
        let rt = runtime_guard.get_mut()?;
        let focused_id = rt.focused_window_id().ok_or_else(|| String::from("no open window available for --control server"))?;
        let win = rt.window_mut(focused_id)?;
        run_authenticated_control_server(win, port, cfg, &app_paths, no_prefs)?;
    }

    {
        let rt = runtime_guard.get_mut()?;
        let focused_id = rt.focused_window_id();
        for win_id in rt.window_ids() {
            if Some(win_id) != focused_id
                && let Ok(win) = rt.window(win_id)
            {
                persist_window_preferences_best_effort(win.app(), &app_paths, no_prefs);
            }
        }
        if let Some(focused_id) = focused_id
            && let Ok(win) = rt.window(focused_id)
        {
            persist_window_preferences_best_effort(win.app(), &app_paths, no_prefs);
        }
    }

    runtime_guard.close_cleanly()?;
    Ok(())
}

struct ControlTokenFileGuard {
    path: PathBuf,
}

impl Drop for ControlTokenFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn control_token_file_path(config_dir: &Path, port: u16) -> PathBuf {
    config_dir.join(format!("control-{port}.token"))
}

fn resolve_control_auth(cfg: &LaunchConfig) -> Result<ControlAuth, String> {
    match cfg.control_token.as_deref() {
        Some(token) => ControlAuth::from_token(token).map_err(|err| format!("invalid control token: {err}")),
        None => ControlAuth::generate().map_err(|err| format!("failed to generate control authentication token: {err}")),
    }
}

fn write_control_token_file(path: &Path, auth: &ControlAuth) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("failed to create control token directory `{}`: {err}", parent.display()))?;
    }
    curvebook_platform::fs::write_atomic_with_hook(path, auth.as_str().as_bytes(), |tmp_path| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(tmp_path, std::fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(not(unix))]
        let _ = tmp_path;
        Ok(())
    })
    .map_err(|err| format!("failed to write control token file `{}`: {err}", path.display()))
}

fn capture_control_pref_fingerprint(app: &UiApp) -> (String, UiThemeMode, u32, Option<String>) {
    (normalize_locale(&app.locale).to_owned(), app.theme.mode, app.ui_scale().to_bits(), app.shortcuts.export_overrides_json().ok())
}

fn theme_mode_pref_str(mode: UiThemeMode) -> &'static str {
    match mode {
        UiThemeMode::Light => "light",
        UiThemeMode::Dark => "dark",
        UiThemeMode::HighContrast => "high-contrast",
    }
}

fn maybe_persist_changed_control_prefs(
    app: &UiApp,
    ctx: &ControlPostRequestContext<'_>,
    prev_prefs: &mut Option<(String, UiThemeMode, u32, Option<String>)>,
    app_paths: &AppPaths,
    no_prefs: bool,
) {
    if no_prefs {
        return;
    }
    let baseline = prev_prefs.get_or_insert_with(|| capture_control_pref_fingerprint(app));
    if ctx.request_ok && !ctx.rolled_back {
        let current = capture_control_pref_fingerprint(app);
        if *baseline != current {
            *baseline = current;
            persist_window_preferences_best_effort(app, app_paths, false);
        }
    }
}

fn make_control_pref_persist_hook(app_paths: &AppPaths, no_prefs: bool) -> Option<LiveControlPostHook> {
    if no_prefs {
        return None;
    }
    let paths = app_paths.clone();
    let mut prev_prefs = None;
    Some(Box::new(move |app: &UiApp, ctx: &ControlPostRequestContext<'_>| {
        maybe_persist_changed_control_prefs(app, ctx, &mut prev_prefs, &paths, false);
        Ok(())
    }))
}

fn serve_control_with_pref_persistence<F>(win: &mut DesktopWindowSession, app_paths: &AppPaths, no_prefs: bool, mut runner: F) -> Result<(), String>
where
    F: FnMut(&mut UiApp, &mut dyn FnMut(&UiApp, &ControlPostRequestContext<'_>) -> Result<(), String>) -> std::io::Result<()>,
{
    let mut prev_prefs: Option<(String, UiThemeMode, u32, Option<String>)> = None;
    win.with_control_journal_hook(|app, on_post_request| {
        if !no_prefs && prev_prefs.is_none() {
            prev_prefs = Some(capture_control_pref_fingerprint(app));
        }
        let mut persist_after_control = |app: &UiApp, ctx: &ControlPostRequestContext<'_>| -> Result<(), String> {
            on_post_request(app, ctx)?;
            maybe_persist_changed_control_prefs(app, ctx, &mut prev_prefs, app_paths, no_prefs);
            Ok(())
        };
        runner(app, &mut persist_after_control)
    })
    .map_err(|err| format!("control server I/O error: {err}"))
}

fn run_authenticated_control_server(
    win: &mut DesktopWindowSession,
    port: u16,
    cfg: &LaunchConfig,
    app_paths: &AppPaths,
    no_prefs: bool,
) -> Result<(), String> {
    let auth = resolve_control_auth(cfg)?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let checked = validate_loopback_addr(addr)?;
    let listener = bind_control_tcp_listener(checked).map_err(|err| format!("failed to bind control socket {checked}: {err}"))?;
    let token_path = control_token_file_path(&app_paths.config_dir, port);
    write_control_token_file(&token_path, &auth)?;
    let token_guard = ControlTokenFileGuard { path: token_path };
    if win.app().ui_backend() == UiBackend::Egui {
        let (mut guards, mut channel) = start_live_window_control_on_listener(listener, auth.clone(), app_paths, no_prefs, token_guard)?;
        let mut post_hook = channel.post_hook.take();
        while let Ok(req) = channel.request_rx.recv() {
            let res: Result<LiveControlFrameOutput, String> =
                win.dispatch_live_control_frame_with_hook(&req.raw_bytes, &req.auth, req.conn_authenticated, |app, hook_ctx| {
                    if let Some(ref mut hook) = post_hook {
                        hook(app, hook_ctx)?;
                    }
                    Ok(())
                });
            let quit = res.as_ref().is_ok_and(|out| out.quit);
            let _ = req.reply_tx.send(res);
            if quit {
                channel.shutdown.store(true, std::sync::atomic::Ordering::Release);
                break;
            }
        }
        drop(channel);
        guards._server_guard.stop();
        return Ok(());
    }
    let _token_guard = token_guard;
    serve_control_with_pref_persistence(win, app_paths, no_prefs, |app, hook| serve_control_listener_with_hook(app, &listener, &auth, hook))
}

struct LiveControlServerGuards {
    _server_guard: LiveControlServerGuard,
    _token_guard: ControlTokenFileGuard,
}

fn start_live_window_control_server(
    port: u16,
    cfg: &LaunchConfig,
    app_paths: &AppPaths,
    no_prefs: bool,
) -> Result<(LiveControlServerGuards, LiveControlChannel), String> {
    let auth = resolve_control_auth(cfg)?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let checked = validate_loopback_addr(addr)?;
    let listener = bind_control_tcp_listener(checked).map_err(|err| format!("failed to bind control socket {checked}: {err}"))?;
    let token_path = control_token_file_path(&app_paths.config_dir, port);
    write_control_token_file(&token_path, &auth)?;
    let token_guard = ControlTokenFileGuard { path: token_path };
    start_live_window_control_on_listener(listener, auth, app_paths, no_prefs, token_guard)
}

fn start_live_window_control_on_listener(
    listener: TcpListener,
    auth: ControlAuth,
    app_paths: &AppPaths,
    no_prefs: bool,
    token_guard: ControlTokenFileGuard,
) -> Result<(LiveControlServerGuards, LiveControlChannel), String> {
    listener.set_nonblocking(true).map_err(|err| format!("failed to configure nonblocking control listener: {err}"))?;
    let (bridge, mut channel) = LiveControlBridge::new(auth);
    if let Some(hook) = make_control_pref_persist_hook(app_paths, no_prefs) {
        channel = channel.with_post_hook(hook);
    }
    let server_guard = spawn_live_control_accept_loop_with_reject(
        bridge,
        move || listener.accept().map(|(stream, _)| stream),
        serve_live_control_tcp_stream,
        reject_busy_control_tcp_stream,
    )?;
    let guards = LiveControlServerGuards { _server_guard: server_guard, _token_guard: token_guard };
    Ok((guards, channel))
}

fn reject_busy_control_tcp_stream(mut stream: TcpStream) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_write_timeout(Some(Duration::from_millis(50)));
    write_busy_control_error(&mut stream);
    let _ = stream.shutdown(Shutdown::Both);
}

fn serve_live_control_tcp_stream(mut stream: TcpStream, bridge: &LiveControlBridge) -> std::io::Result<()> {
    // Accepted sockets inherit non-blocking mode from the listener on macOS/BSD.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    stream.set_write_timeout(Some(CONTROL_CLIENT_IDLE_TIMEOUT))?;
    let reader_stream = stream.try_clone()?;
    let mut reader = BufReader::new(reader_stream);
    serve_live_control_io(&mut reader, &mut stream, bridge, drain_oversized_tcp_stream)
}

fn drain_oversized_tcp_stream(tcp: &mut TcpStream) {
    let _ = tcp.shutdown(Shutdown::Write);
    // Drain bounded trailing frame bytes briefly after FIN so BSD/macOS sockets do not send TCP RST
    // and discard the JSON error response still in the client's receive buffer.
    let _ = tcp.set_read_timeout(Some(Duration::from_millis(20)));
    let mut discard = [0u8; 8192];
    let mut drained = 0usize;
    let deadline = Instant::now() + Duration::from_millis(50);
    while drained < 2 * 1024 * 1024 && Instant::now() < deadline {
        match tcp.read(&mut discard) {
            Ok(0) | Err(_) => break,
            Ok(n) => drained += n,
        }
    }
}

fn should_persist_preferences(app: &UiApp, paths: &AppPaths) -> Result<bool, String> {
    if paths.preferences_file.is_file() {
        return Ok(true);
    }
    if normalize_locale(&app.locale) != "en" || app.theme.mode != UiThemeMode::default() || (app.ui_scale() - DEFAULT_UI_SCALE).abs() > 0.001 {
        return Ok(true);
    }
    let raw = app.shortcuts.export_overrides_json()?;
    let map: BTreeMap<String, String> = serde_json::from_str(&raw).map_err(|err| err.to_string())?;
    Ok(!map.is_empty())
}

struct StartupRecoveryState {
    session: Session,
    journal: Option<CrashRecoveryJournal<DirectoryJournalBackend>>,
    report: Option<RecoveryReport>,
    recovery_dir: Option<PathBuf>,
}

fn now_epoch_ms() -> u64 {
    u64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())).unwrap_or(u64::MAX)
}

fn recovery_owner_id() -> String {
    format!("curvebook-pid-{}", std::process::id())
}

fn ensure_primary_recovery_dir(app_paths: &AppPaths) -> Result<bool, String> {
    if app_paths.portable || app_paths.recovery_dir.exists() {
        return Ok(true);
    }
    match std::fs::create_dir_all(&app_paths.recovery_dir) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => Ok(false),
        Err(err) => Err(format!("failed to create recovery directory `{}`: {err}", app_paths.recovery_dir.display())),
    }
}

fn open_startup_session_with_recovery(
    files: &[PathBuf],
    app_paths: &AppPaths,
    enable_recovery: bool,
    now_ms: u64,
) -> Result<StartupRecoveryState, String> {
    if !enable_recovery {
        let session = load_startup_session(files)?;
        return Ok(StartupRecoveryState { session, journal: None, report: None, recovery_dir: None });
    }

    let session = load_startup_session(files)?;
    if !ensure_primary_recovery_dir(app_paths)? {
        return Ok(StartupRecoveryState { session, journal: None, report: None, recovery_dir: None });
    }

    if files.is_empty() {
        let (recovered_session, journal, report) =
            Session::open_dir_with_recovery(&app_paths.recovery_dir, recovery_owner_id(), RECOVERY_LOCK_TTL_MS, now_ms)
                .map_err(|err| format!("failed to open recovery journal in `{}`: {err}", app_paths.recovery_dir.display()))?;
        log_recovery_warnings(&app_paths.recovery_dir, &report);
        return Ok(StartupRecoveryState {
            session: recovered_session,
            journal: Some(journal),
            report: Some(report),
            recovery_dir: Some(app_paths.recovery_dir.clone()),
        });
    }

    if dir_has_active_lease(&app_paths.recovery_dir, now_ms) {
        let _ = Session::open_dir_with_recovery(&app_paths.recovery_dir, recovery_owner_id(), RECOVERY_LOCK_TTL_MS, now_ms)
            .map_err(|err| format!("failed to open recovery journal in `{}`: {err}", app_paths.recovery_dir.display()))?;
    }

    let target_dir = if dir_has_unrecovered_crash(&app_paths.recovery_dir, now_ms) {
        allocate_fresh_isolated_journal_dir(&app_paths.recovery_dir, DesktopWindowId(1), now_ms)?
    } else {
        app_paths.recovery_dir.clone()
    };

    let (_, mut journal, _) = Session::open_dir_with_recovery(&target_dir, recovery_owner_id(), RECOVERY_LOCK_TTL_MS, now_ms)
        .map_err(|err| format!("failed to open recovery journal in `{}`: {err}", target_dir.display()))?;
    if let Err(err) = session.checkpoint_journal(&mut journal) {
        let owner_id = journal.lease().owner_id.clone();
        let mut backend = journal.abandon();
        let _ = backend.release_lease(&owner_id);
        return Err(format!("failed to seed startup recovery snapshot in `{}`: {err}", target_dir.display()));
    }
    Ok(StartupRecoveryState { session, journal: Some(journal), report: None, recovery_dir: Some(target_dir) })
}

fn log_recovery_warnings(dir: &Path, report: &RecoveryReport) {
    for warning in &report.warnings {
        eprintln!("curvebook recovery warning in `{}` (line {}): {}", dir.display(), warning.line, warning.message);
    }
}

#[cfg(test)]
fn close_startup_recovery_cleanly(app: &mut UiApp, recovery_journal: Option<CrashRecoveryJournal<DirectoryJournalBackend>>) -> Result<(), String> {
    if let Some(journal) = recovery_journal {
        let _backend =
            app.session_mut().close_journal_cleanly(journal).map_err(|err| format!("failed to close crash-recovery journal cleanly: {err}"))?;
    }
    Ok(())
}

fn parse_env_truthy(val: Option<&OsStr>) -> bool {
    val.and_then(|s| s.to_str()).map(str::trim).is_some_and(|s| {
        s.eq_ignore_ascii_case("1") || s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("yes") || s.eq_ignore_ascii_case("on")
    })
}

fn is_no_prefs_enabled(env_get: &impl Fn(&str) -> Option<OsString>) -> bool {
    parse_env_truthy(env_get("CURVEBOOK_NO_PREFS").as_deref())
}

#[cfg(test)]
fn resolve_app_paths(exe_dir: Option<&Path>, explicit_portable: Option<&Path>) -> Result<AppPaths, String> {
    resolve_app_paths_with_env(exe_dir, explicit_portable, |_| None)
}

fn resolve_app_paths_with_env(
    exe_dir: Option<&Path>,
    explicit_portable: Option<&Path>,
    env_get: impl Fn(&str) -> Option<OsString>,
) -> Result<AppPaths, String> {
    if let Some(dir) = explicit_portable {
        return Ok(portable_app_paths(dir));
    }
    if let Some(raw_cfg) = env_get("CURVEBOOK_CONFIG_DIR").filter(|v| !v.to_string_lossy().trim().is_empty()) {
        let base = PathBuf::from(raw_cfg);
        return Ok(AppPaths {
            portable: false,
            preferences_file: base.join("preferences.json"),
            shortcuts_file: base.join("shortcuts.json"),
            recovery_dir: base.join("recovery"),
            config_dir: base.clone(),
            data_dir: base,
        });
    }
    if let Some(dir) = exe_dir
        && dir.join("portable.txt").is_file()
    {
        return Ok(portable_app_paths(dir));
    }

    let (config_dir, data_dir) = resolve_platform_standard_dirs(&env_get)?;
    Ok(AppPaths {
        portable: false,
        preferences_file: config_dir.join("preferences.json"),
        shortcuts_file: config_dir.join("shortcuts.json"),
        recovery_dir: data_dir.join("recovery"),
        config_dir,
        data_dir,
    })
}

fn portable_app_paths(base_dir: &Path) -> AppPaths {
    let root =
        if base_dir.file_name().is_some_and(|name| name == "curvebook-data") { base_dir.to_path_buf() } else { base_dir.join("curvebook-data") };
    AppPaths {
        portable: true,
        preferences_file: root.join("preferences.json"),
        shortcuts_file: root.join("shortcuts.json"),
        recovery_dir: root.join("recovery"),
        config_dir: root.clone(),
        data_dir: root,
    }
}

#[cfg(unix)]
fn current_unix_uid() -> Result<u32, String> {
    use std::os::unix::fs::MetadataExt;
    static CACHED_UID: std::sync::OnceLock<Result<u32, String>> = std::sync::OnceLock::new();
    CACHED_UID
        .get_or_init(|| {
            if let Ok(meta) = std::fs::metadata("/proc/self") {
                return Ok(meta.uid());
            }
            for bin in ["/usr/bin/id", "/bin/id", "id"] {
                if let Ok(output) = curvebook_platform::ProcessCommand::new(bin).arg("-u").run()
                    && output.success
                {
                    let text = String::from_utf8_lossy(&output.stdout);
                    if let Ok(uid) = text.trim().parse::<u32>() {
                        return Ok(uid);
                    }
                }
            }
            Err(String::from("unable to resolve current OS UID for private user directory fallback"))
        })
        .clone()
}

#[cfg(unix)]
fn verify_private_existing_dir(path: &Path, expected_uid: u32, label: &str) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path).map_err(|err| format!("failed to inspect {label} `{}`: {err}", path.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!("refusing unsafe non-directory or symlink {label} `{}`", path.display()));
    }
    if meta.uid() != expected_uid {
        return Err(format!("refusing {label} `{}` owned by uid {} (expected uid {expected_uid})", path.display(), meta.uid()));
    }
    if (meta.mode() & 0o077) != 0 {
        return Err(format!("refusing {label} `{}` with group/world permissions {:04o} (must be private 0700)", path.display(), meta.mode() & 0o777));
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_private_uid_dir(path: &Path, expected_uid: u32, label: &str) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::symlink_metadata(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(path) {
                Ok(()) => {}
                Err(create_err) if create_err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(create_err) => {
                    return Err(format!("failed to create private {label} `{}`: {create_err}", path.display()));
                }
            }
        }
        Err(err) => return Err(format!("failed to inspect {label} `{}`: {err}", path.display())),
        Ok(_) => {}
    }
    verify_private_existing_dir(path, expected_uid, label)
}

#[cfg(unix)]
fn resolve_unix_private_fallback_root(env_get: &impl Fn(&str) -> Option<OsString>) -> Result<(u32, PathBuf), String> {
    let uid = current_unix_uid()?;
    if let Some(raw_xdg_rt) = env_get("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        let rt_dir = PathBuf::from(raw_xdg_rt);
        verify_private_existing_dir(&rt_dir, uid, "XDG_RUNTIME_DIR")?;
        let app_root = rt_dir.join("curvebook");
        ensure_private_uid_dir(&app_root, uid, "runtime directory")?;
        return Ok((uid, app_root));
    }
    let tmp_base = env_get("TMPDIR").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let uid_root = tmp_base.join(format!("curvebook-uid-{uid}"));
    ensure_private_uid_dir(&uid_root, uid, "per-user fallback directory")?;
    Ok((uid, uid_root))
}

fn resolve_platform_standard_dirs(env_get: &impl Fn(&str) -> Option<OsString>) -> Result<(PathBuf, PathBuf), String> {
    let nonempty_path = |key: &str| env_get(key).filter(|v| !v.is_empty()).map(PathBuf::from);

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = nonempty_path("HOME") {
            let base = home.join("Library").join("Application Support").join("CurveBook");
            return Ok((base.clone(), base));
        }
        let (_uid, fallback_base) = resolve_unix_private_fallback_root(env_get)?;
        Ok((fallback_base.clone(), fallback_base))
    }
    #[cfg(target_os = "windows")]
    {
        let fallback_base = nonempty_path("USERPROFILE").map(|h| h.join("AppData").join("Roaming").join("CurveBook"));
        let config_dir = nonempty_path("APPDATA").map(|p| p.join("CurveBook")).or(fallback_base).ok_or_else(|| {
            String::from("cannot determine CurveBook config directory: APPDATA and USERPROFILE are unset (set CURVEBOOK_CONFIG_DIR or --portable)")
        })?;
        let data_dir = nonempty_path("LOCALAPPDATA").map(|p| p.join("CurveBook")).unwrap_or_else(|| config_dir.clone());
        Ok((config_dir, data_dir))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let home = nonempty_path("HOME");
        let mut fallback: Option<(u32, PathBuf)> = None;
        let mut get_fallback = || -> Result<(u32, PathBuf), String> {
            if let Some(ref existing) = fallback {
                return Ok(existing.clone());
            }
            let resolved = resolve_unix_private_fallback_root(env_get)?;
            fallback = Some(resolved.clone());
            Ok(resolved)
        };
        let config_dir = match nonempty_path("XDG_CONFIG_HOME")
            .map(|p| p.join("curvebook"))
            .or_else(|| home.as_ref().map(|h| h.join(".config").join("curvebook")))
        {
            Some(dir) => dir,
            None => {
                let (uid, root) = get_fallback()?;
                let dir = root.join("config");
                ensure_private_uid_dir(&dir, uid, "per-user config fallback directory")?;
                dir
            }
        };
        let data_dir = match nonempty_path("XDG_DATA_HOME")
            .map(|p| p.join("curvebook"))
            .or_else(|| home.as_ref().map(|h| h.join(".local").join("share").join("curvebook")))
        {
            Some(dir) => dir,
            None => {
                let (uid, root) = get_fallback()?;
                let dir = root.join("data");
                ensure_private_uid_dir(&dir, uid, "per-user data fallback directory")?;
                dir
            }
        };
        Ok((config_dir, data_dir))
    }
}

fn load_validated_ttf_font(path: &Path) -> Result<(Vec<u8>, TrueTypeFontInfo), String> {
    let bytes = curvebook_platform::fs::read_regular_file_bytes(path, MAX_FONT_FILE_BYTES)
        .map_err(|err| format!("failed to read --ttf-font `{}`: {err}", path.display()))?;
    let info = parse_truetype_font_info(&bytes).map_err(|err| format!("invalid --ttf-font `{}`: {err}", path.display()))?;
    Ok((bytes, info))
}

fn load_shortcut_preferences(app: &mut UiApp, paths: &AppPaths) -> Result<usize, String> {
    if paths.preferences_file.is_file() {
        let prefs = load_preferences_file(&paths.preferences_file)
            .map_err(|err| format!("failed to load preferences `{}`: {err}", paths.preferences_file.display()))?;
        if let Some(ref raw_locale) = prefs.locale {
            let trimmed = raw_locale.trim();
            if !trimmed.is_empty() {
                app.set_locale(normalize_beta_gui_locale(trimmed));
            }
        }
        if let Some(ref raw_theme) = prefs.theme {
            match curvebook::desktop_host::parse_theme_mode(raw_theme) {
                Ok(mode) => app.set_theme_mode(mode),
                Err(err) => {
                    eprintln!("curvebook preferences warning: ignoring invalid theme in `{}`: {err}", paths.preferences_file.display());
                }
            }
        }
        match extract_ui_scale_from_preferences_extra(&prefs.extra) {
            Ok(Some(scale)) => {
                let _ = app.set_ui_scale(scale);
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!("curvebook preferences warning: ignoring invalid ui_scale in `{}`: {err}", paths.preferences_file.display());
            }
        }
        if prefs.shortcuts.is_empty() {
            return Ok(0);
        }
        let text = serde_json::to_string(&prefs.shortcuts).map_err(|err| err.to_string())?;
        return app.shortcuts.import_overrides_json(&text);
    }
    if paths.shortcuts_file.is_file() {
        let raw = curvebook_platform::fs::read_regular_file_utf8(&paths.shortcuts_file, MAX_PREFERENCES_FILE_BYTES)
            .map_err(|err| format!("failed to read shortcuts file `{}`: {err}", paths.shortcuts_file.display()))?;
        return app.shortcuts.import_overrides_json(&raw);
    }
    Ok(0)
}

fn save_shortcut_preferences(app: &UiApp, paths: &AppPaths) -> Result<(), String> {
    let mut prefs = if paths.preferences_file.is_file() {
        load_preferences_file(&paths.preferences_file)
            .map_err(|err| format!("failed to read existing preferences `{}`: {err}", paths.preferences_file.display()))?
    } else {
        PreferencesFile::default()
    };
    let current_locale = normalize_locale(&app.locale);
    match prefs.locale.as_deref() {
        Some(existing) if normalize_locale(existing) == current_locale => {}
        Some(_) => prefs.locale = Some(current_locale.to_owned()),
        None if current_locale != "en" => prefs.locale = Some(current_locale.to_owned()),
        None => {}
    }
    let current_theme = app.theme.mode;
    match prefs.theme.as_deref() {
        Some(existing) if curvebook::desktop_host::parse_theme_mode(existing).ok() == Some(current_theme) => {}
        Some(_) => prefs.theme = Some(theme_mode_pref_str(current_theme).to_owned()),
        None if current_theme != UiThemeMode::default() => {
            prefs.theme = Some(theme_mode_pref_str(current_theme).to_owned());
        }
        None => {}
    }
    let raw_shortcuts = app.shortcuts.export_overrides_json()?;
    let shortcuts: BTreeMap<String, String> = serde_json::from_str(&raw_shortcuts).map_err(|err| err.to_string())?;
    prefs.version = CURRENT_PREFERENCES_VERSION;
    prefs.shortcuts = shortcuts;
    write_ui_scale_to_preferences_extra(&mut prefs.extra, app.ui_scale());
    save_preferences_file(&paths.preferences_file, &prefs)
        .map_err(|err| format!("failed to write preferences `{}`: {err}", paths.preferences_file.display()))
}

fn load_startup_session(files: &[PathBuf]) -> Result<Session, String> {
    open_document_from_files(files)
}

fn parse_port(raw: &str) -> Result<u16, String> {
    parse_control_port(raw)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use curvebook_doc::{DataSheet, ProjectDocument, SubcolumnMode, TableArchetype, XyTable};
    use curvebook_format::save_project_file;

    use super::{
        CURRENT_PREFERENCES_VERSION, CliAction, LaunchConfig, UiApp, UiBackend, UiThemeMode, load_startup_session, parse_args, parse_port, run,
    };

    #[test]
    fn cli_version_and_help_and_flags() {
        assert!(run(&[String::from("--version")]).is_ok());
        assert!(run(&[String::from("--help")]).is_ok());
        assert!(run(&[String::from("--in-window-menus"), String::from("--new-instance"), String::from("--headless-once")]).is_ok());
        assert!(run(&[String::from("--definitely-unknown-flag")]).is_err());

        assert!(parse_port("0").is_err());
        assert!(parse_port("65536").is_err());
        assert_eq!(parse_port("9001").ok(), Some(9001));

        let args = vec![
            String::from("--control"),
            String::from("9123"),
            String::from("--in-window-menus"),
            String::from("--new-instance"),
            String::from("study.curvebook"),
        ];
        assert_eq!(
            parse_args(&args, None).ok(),
            Some(CliAction::Launch(LaunchConfig {
                control_port: Some(9123),
                control_token: None,
                window: false,
                gui_only: false,
                ui_backend: UiBackend::Egui,
                in_window_menus: true,
                new_instance: true,
                headless_once: false,
                portable_dir: None,
                ttf_font: None,
                rulers: false,
                event_script: None,
                files: vec![PathBuf::from("study.curvebook")],
            }))
        );

        let gui_only_args = vec![String::from("--control"), String::from("9124"), String::from("--gui-only"), String::from("--headless-once")];
        assert_eq!(
            parse_args(&gui_only_args, None).ok(),
            Some(CliAction::Launch(LaunchConfig {
                control_port: Some(9124),
                control_token: None,
                window: false,
                gui_only: true,
                ui_backend: UiBackend::Egui,
                in_window_menus: false,
                new_instance: false,
                headless_once: true,
                portable_dir: None,
                ttf_font: None,
                rulers: false,
                event_script: None,
                files: Vec::new(),
            }))
        );
        assert!(super::USAGE.contains("--gui-only"));
        assert!(super::USAGE.contains("--window"));

        let window_args = vec![String::from("--control"), String::from("9125"), String::from("--window"), String::from("--gui-only")];
        assert_eq!(
            parse_args(&window_args, None).ok(),
            Some(CliAction::Launch(LaunchConfig {
                control_port: Some(9125),
                control_token: None,
                window: true,
                gui_only: true,
                ui_backend: UiBackend::Egui,
                in_window_menus: false,
                new_instance: false,
                headless_once: false,
                portable_dir: None,
                ttf_font: None,
                rulers: false,
                event_script: None,
                files: Vec::new(),
            }))
        );
        assert!(
            super::parse_args(&[String::from("--control"), String::from("9125"), String::from("--window"), String::from("--headless-once")], None)
                .is_err()
        );

        let portable_args = vec![
            String::from("--portable-dir"),
            String::from("/tmp/cb-usb"),
            String::from("--ttf-font"),
            String::from("/tmp/sans.ttf"),
            String::from("--rulers"),
            String::from("--event-script"),
            String::from("/tmp/script.jsonl"),
        ];
        assert_eq!(
            parse_args(&portable_args, None).ok(),
            Some(CliAction::Launch(LaunchConfig {
                control_port: None,
                control_token: None,
                window: false,
                gui_only: false,
                ui_backend: UiBackend::Egui,
                in_window_menus: false,
                new_instance: false,
                headless_once: false,
                portable_dir: Some(PathBuf::from("/tmp/cb-usb")),
                ttf_font: Some(PathBuf::from("/tmp/sans.ttf")),
                rulers: true,
                event_script: Some(PathBuf::from("/tmp/script.jsonl")),
                files: Vec::new(),
            }))
        );
    }

    #[test]
    fn portable_mode_detection_and_standard_app_paths() -> Result<(), Box<dyn std::error::Error>> {
        use std::ffi::OsString;

        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cb-app-paths-{}-{unique}", std::process::id()));
        let exe_dir = dir.join("bin");
        std::fs::create_dir_all(&exe_dir)?;

        // Standard (non-portable) mode resolves platform dirs via env without touching portable.txt
        let std_paths = super::resolve_app_paths_with_env(Some(&exe_dir), None, |key| match key {
            "HOME" | "USERPROFILE" => Some(OsString::from("/mock/home")),
            "XDG_CONFIG_HOME" => Some(OsString::from("/mock/xdg-config")),
            "XDG_DATA_HOME" => Some(OsString::from("/mock/xdg-data")),
            "APPDATA" => Some(OsString::from("/mock/appdata")),
            "LOCALAPPDATA" => Some(OsString::from("/mock/localappdata")),
            _ => None,
        })?;
        assert!(!std_paths.portable);
        assert!(!std_paths.config_dir.ends_with("curvebook-data"));

        // Explicit --portable-dir routes config, shortcuts, preferences, and recovery into ./curvebook-data/
        let explicit_dir = dir.join("usb-stick");
        let explicit_paths = super::resolve_app_paths(Some(&exe_dir), Some(&explicit_dir))?;
        assert!(explicit_paths.portable);
        let expected_explicit_root = explicit_dir.join("curvebook-data");
        assert_eq!(explicit_paths.config_dir, expected_explicit_root);
        assert_eq!(explicit_paths.data_dir, expected_explicit_root);
        assert_eq!(explicit_paths.preferences_file, expected_explicit_root.join("preferences.json"));
        assert_eq!(explicit_paths.shortcuts_file, expected_explicit_root.join("shortcuts.json"));
        assert_eq!(explicit_paths.recovery_dir, expected_explicit_root.join("recovery"));

        // portable.txt next to executable activates ./curvebook-data/
        std::fs::write(exe_dir.join("portable.txt"), "CurveBook portable mode\n")?;
        let marker_paths = super::resolve_app_paths(Some(&exe_dir), None)?;
        assert!(marker_paths.portable);
        assert_eq!(marker_paths.config_dir, exe_dir.join("curvebook-data"));
        assert_eq!(marker_paths.recovery_dir, exe_dir.join("curvebook-data").join("recovery"));

        // Shortcut preferences save & reload round-trip in portable mode (versioned preferences.json)
        let mut app = curvebook_ui_egui::UiApp::new();
        app.shortcuts.set_shortcut("ui.gallery.toggle", "Ctrl+Shift+L")?;
        super::save_shortcut_preferences(&app, &marker_paths)?;
        assert!(marker_paths.preferences_file.is_file());
        let saved_prefs_raw = std::fs::read_to_string(&marker_paths.preferences_file)?;
        let saved_prefs_val: serde_json::Value = serde_json::from_str(&saved_prefs_raw)?;
        assert_eq!(saved_prefs_val["version"], serde_json::json!(curvebook_format::versioned::CURRENT_PREFERENCES_VERSION));
        let mut reloaded = curvebook_ui_egui::UiApp::new();
        let imported = super::load_shortcut_preferences(&mut reloaded, &marker_paths)?;
        assert_eq!(imported, 1);
        assert_eq!(reloaded.shortcuts.active_combo("ui.gallery.toggle"), Some("Ctrl+Shift+L"));

        // Valid & invalid --ttf-font loading
        let valid_ttf = dir.join("minimal.ttf");
        std::fs::write(&valid_ttf, curvebook_text::build_minimal_truetype_font())?;
        let (_, font_info) = super::load_validated_ttf_font(&valid_ttf)?;
        assert!(!font_info.family_name.is_empty());

        let invalid_ttf = dir.join("bad.ttf");
        std::fs::write(&invalid_ttf, b"not-a-ttf")?;
        assert!(super::load_validated_ttf_font(&invalid_ttf).is_err());

        // Portable-dir recovery journal startup -> simulated crash -> WAL recovery -> clean shutdown
        let recovery_root = dir.join("portable-recovery");
        let rec_paths = super::resolve_app_paths(None, Some(&recovery_root))?;
        let mut s1 = super::open_startup_session_with_recovery(&[], &rec_paths, true, 1_000)?;
        let mut journal1 = s1.journal.ok_or("portable recovery should attach journal")?;
        assert!(s1.report.is_some_and(|r| r.recovered_commands.is_empty()));
        s1.session.execute_and_record(&mut journal1, "sheet.createData", serde_json::json!({ "sheet_id": "rec-1", "title": "Recovered" }))?;
        drop(journal1.abandon());

        let s2 = super::open_startup_session_with_recovery(&[], &rec_paths, true, 1_000 + super::RECOVERY_LOCK_TTL_MS + 10)?;
        let report2 = s2.report.ok_or("recovery report expected on reopen")?;
        assert!(report2.recovered_from_stale_lock.is_some());
        assert_eq!(report2.recovered_commands.len(), 1);
        assert!(s2.session.document().data_sheets.iter().any(|s| s.id.as_str() == "rec-1"));

        let mut app2 = curvebook_ui_egui::UiApp::from_session(s2.session);
        super::close_startup_recovery_cleanly(&mut app2, s2.journal)?;
        assert!(!app2.session().is_dirty());

        let s3 = super::open_startup_session_with_recovery(&[], &rec_paths, true, 2_000 + super::RECOVERY_LOCK_TTL_MS)?;
        let report3 = s3.report.ok_or("reopen after clean close should return report")?;
        assert!(report3.recovered_from_stale_lock.is_none());
        assert!(report3.recovered_commands.is_empty());
        assert!(s3.session.document().data_sheets.iter().any(|s| s.id.as_str() == "rec-1"));
        let mut app3 = curvebook_ui_egui::UiApp::from_session(s3.session);
        super::close_startup_recovery_cleanly(&mut app3, s3.journal)?;

        // Headless launch wiring test with --portable-dir, --ttf-font, and --rulers
        let headless_cfg = super::LaunchConfig {
            headless_once: true,
            portable_dir: Some(recovery_root),
            ttf_font: Some(valid_ttf),
            rulers: true,
            ..super::LaunchConfig::default()
        };
        super::launch_application(&headless_cfg)?;
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn cli_file_startup_attaches_recovery_and_control_port_window_routing() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("cb-main-new12-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;

        // NEW-1 + NEW-2: CLI file startup attaches recovery journal AND protects existing primary crash.
        let cli_rec_root = dir.join("cli-file-portable");
        let cli_rec_paths = super::resolve_app_paths(None, Some(&cli_rec_root))?;
        let cli_project = dir.join("startup-file.curvebook");
        let mut cli_doc = curvebook_doc::ProjectDocument::new("Startup File")?;
        cli_doc.add_data_sheet(curvebook_doc::DataSheet::new(
            "cli-s1",
            "CLI Data",
            curvebook_doc::TableArchetype::Column(curvebook_doc::ColumnTable::new(curvebook_doc::ColumnDesignHint::default(), 1, 2)?),
        )?)?;
        curvebook_format::save_project_file(&cli_project, &cli_doc)?;

        let mut cli_start = super::open_startup_session_with_recovery(std::slice::from_ref(&cli_project), &cli_rec_paths, true, 5_000)?;
        let mut cli_journal = cli_start.journal.ok_or("NEW-1: CLI file startup must attach live recovery journal")?;
        assert_eq!(cli_start.recovery_dir.as_deref(), Some(cli_rec_paths.recovery_dir.as_path()));
        cli_start.session.execute_and_record(
            &mut cli_journal,
            "table.setCell",
            serde_json::json!({ "sheet_id": "cli-s1", "row": 0, "col": 0, "value": 98.75 }),
        )?;
        drop(cli_journal.abandon());

        let cli_recovered = super::open_startup_session_with_recovery(&[], &cli_rec_paths, true, 5_000 + super::RECOVERY_LOCK_TTL_MS + 10)?;
        let crashed_sheet =
            cli_recovered.session.document().data_sheet(&curvebook_doc::SheetId::new("cli-s1")).ok_or("missing recovered CLI sheet")?;
        assert_eq!(crashed_sheet.cell(curvebook_doc::CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(98.75));
        let mut rec_app = curvebook_ui_egui::UiApp::from_session(cli_recovered.session);
        super::close_startup_recovery_cleanly(&mut rec_app, cli_recovered.journal)?;

        // NEW-2: If primary recovery_dir holds an unrecovered crash, launching with another CLI file
        // preserves the crashed primary journal intact and runs in an isolated journal directory.
        {
            let (mut s_crash, mut j_crash, _) =
                curvebook_engine::Session::open_dir_with_recovery(&cli_rec_paths.recovery_dir, "crashed-proc", super::RECOVERY_LOCK_TTL_MS, 10_000)?;
            s_crash.execute_and_record(&mut j_crash, "sheet.createData", serde_json::json!({ "sheet_id": "keep-crash", "title": "Preserved" }))?;
            drop(j_crash.abandon());
        }
        let isolated_cli = super::open_startup_session_with_recovery(
            std::slice::from_ref(&cli_project),
            &cli_rec_paths,
            true,
            10_000 + super::RECOVERY_LOCK_TTL_MS + 10,
        )?;
        let iso_journal = isolated_cli.journal.ok_or("NEW-2: isolated CLI file launch must still attach a journal")?;
        assert_ne!(isolated_cli.recovery_dir.as_deref(), Some(cli_rec_paths.recovery_dir.as_path()));
        let mut iso_app = curvebook_ui_egui::UiApp::from_session(isolated_cli.session);
        super::close_startup_recovery_cleanly(&mut iso_app, Some(iso_journal))?;

        let primary_still_has_crash =
            super::open_startup_session_with_recovery(&[], &cli_rec_paths, true, 10_000 + super::RECOVERY_LOCK_TTL_MS + 20)?;
        assert!(
            primary_still_has_crash.session.document().data_sheet(&curvebook_doc::SheetId::new("keep-crash")).is_some(),
            "crashed primary journal must not be overwritten when launching with a CLI file"
        );
        let mut clean_primary_app = curvebook_ui_egui::UiApp::from_session(primary_still_has_crash.session);
        super::close_startup_recovery_cleanly(&mut clean_primary_app, primary_still_has_crash.journal)?;

        // R2-S11-09 (#362): CURVEBOOK_CONTROL_PORT matches --control behavior (headless unless --window).
        let env_headless = super::parse_args(&[], Some("9125"))?;
        let super::CliAction::Launch(cfg_headless) = env_headless else {
            return Err("expected Launch for CURVEBOOK_CONTROL_PORT".into());
        };
        assert_eq!(cfg_headless.control_port, Some(9125));
        assert!(!cfg_headless.window);
        assert!(!super::should_launch_window(&cfg_headless));

        let env_windowed = super::parse_args(&[String::from("--window")], Some("9125"))?;
        let super::CliAction::Launch(cfg_windowed) = env_windowed else {
            return Err("expected Launch for CURVEBOOK_CONTROL_PORT + --window".into());
        };
        assert_eq!(cfg_windowed.control_port, Some(9125));
        assert!(cfg_windowed.window);
        assert!(super::should_launch_window(&cfg_windowed));

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn startup_loads_project_and_csv_files() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("cb-main-startup-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;

        let project_path = dir.join("sample.curvebook");
        let mut doc = ProjectDocument::new("Startup Assay")?;
        let xy = XyTable::new(SubcolumnMode::Single, 1, 3)?;
        doc.add_data_sheet(DataSheet::new("data-1", "Dose", TableArchetype::Xy(xy))?)?;
        save_project_file(&project_path, &doc)?;

        let session = load_startup_session(std::slice::from_ref(&project_path))?;
        assert_eq!(session.document().title, "Startup Assay");
        assert_eq!(session.document().data_sheets.len(), 1);
        assert!(!session.is_dirty());

        let csv_path = dir.join("plate.csv");
        std::fs::write(&csv_path, "Ctrl,Treated\n1.0,2.5\n3.0,4.5\n")?;
        let csv_session = load_startup_session(&[csv_path])?;
        assert_eq!(csv_session.document().data_sheets.len(), 1);
        assert_eq!(csv_session.document().data_sheets[0].title, "plate");

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn startup_and_session_auto_detect_unpacked_project_dirs() -> Result<(), Box<dyn std::error::Error>> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-main-unpacked-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)?;

        let unpacked_dot_d = dir.join("study.curvebook.d");
        let mut doc = ProjectDocument::new("Unpacked Startup")?;
        let xy = XyTable::new(SubcolumnMode::Single, 1, 2)?;
        doc.add_data_sheet(DataSheet::new("data-1", "Dose", TableArchetype::Xy(xy))?)?;
        curvebook_format::save_project_unpacked_dir(&doc, &unpacked_dot_d)?;

        let session = load_startup_session(std::slice::from_ref(&unpacked_dot_d))?;
        assert_eq!(session.document().title, "Unpacked Startup");
        assert_eq!(session.file_path(), Some(unpacked_dot_d.as_path()));
        assert!(!session.is_dirty());

        let mut mut_session = session;
        // 1. Auto-detect `.curvebook.d` suffix when saving without explicit `unpacked: true`.
        let saved_dot_d = dir.join("saved.curvebook.d");
        let save_resp =
            mut_session.execute("project.save", serde_json::json!({ "path": saved_dot_d.display().to_string(), "return_bytes": false }))?;
        assert_eq!(save_resp.payload.get("unpacked").and_then(|v| v.as_bool()), Some(true));
        assert!(curvebook_format::is_unpacked_project_dir(&saved_dot_d));

        let reopened_dot_d = load_startup_session(std::slice::from_ref(&saved_dot_d))?;
        assert_eq!(reopened_dot_d.document().title, "Unpacked Startup");

        // 2. Also auto-detect an unpacked directory without `.curvebook.d` suffix via `is_unpacked_project_dir`.
        let plain_unpacked = dir.join("plain_project_dir");
        let save_plain_resp = mut_session
            .execute("project.save", serde_json::json!({ "path": plain_unpacked.display().to_string(), "unpacked": true, "return_bytes": false }))?;
        assert_eq!(save_plain_resp.payload.get("unpacked").and_then(|v| v.as_bool()), Some(true));
        assert!(curvebook_format::is_unpacked_project_dir(&plain_unpacked));

        let reopened = load_startup_session(std::slice::from_ref(&plain_unpacked))?;
        assert_eq!(reopened.document().title, "Unpacked Startup");
        assert_eq!(reopened.document().data_sheets.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn preferences_preserve_forward_fields_and_refuse_newer_version_overwrite() -> Result<(), Box<dyn std::error::Error>> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-main-prefs-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let paths = super::resolve_app_paths(None, Some(&dir))?;
        std::fs::create_dir_all(&paths.config_dir)?;

        // 1. Legacy v0 preferences file without explicit `version` upgrades and preserves metadata.
        let legacy_raw = serde_json::json!({
            "locale": "en-US",
            "theme": "light",
            "shortcuts": { "ui.gallery.toggle": "Ctrl+Alt+G" },
            "future_flag": true
        });
        std::fs::write(&paths.preferences_file, serde_json::to_string_pretty(&legacy_raw)?)?;
        let mut app = curvebook_ui_egui::UiApp::new();
        let count = super::load_shortcut_preferences(&mut app, &paths)?;
        assert_eq!(count, 1);
        assert_eq!(app.shortcuts.active_combo("ui.gallery.toggle"), Some("Ctrl+Alt+G"));

        app.shortcuts.set_shortcut("ui.gallery.toggle", "Ctrl+Shift+G")?;
        super::save_shortcut_preferences(&app, &paths)?;
        let after_save: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&paths.preferences_file)?)?;
        assert_eq!(after_save["version"], serde_json::json!(super::CURRENT_PREFERENCES_VERSION));
        assert_eq!(after_save["locale"], serde_json::json!("en-US"));
        assert_eq!(after_save["future_flag"], serde_json::json!(true));

        // 2. Future version file on disk must never be overwritten.
        let future_raw = serde_json::json!({
            "version": super::CURRENT_PREFERENCES_VERSION + 42,
            "shortcuts": { "ui.gallery.toggle": "Ctrl+Alt+F" },
            "custom_future_setting": 123
        });
        let future_text = serde_json::to_string_pretty(&future_raw)?;
        std::fs::write(&paths.preferences_file, &future_text)?;

        let mut future_app = curvebook_ui_egui::UiApp::new();
        assert!(super::load_shortcut_preferences(&mut future_app, &paths).is_err());
        future_app.shortcuts.set_shortcut("ui.gallery.toggle", "Ctrl+Shift+X")?;
        assert!(super::save_shortcut_preferences(&future_app, &paths).is_err());
        assert_eq!(std::fs::read_to_string(&paths.preferences_file)?, future_text, "newer-version preferences file must remain untouched");

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn control_token_cli_env_override_and_restricted_file_cleanup() -> Result<(), Box<dyn std::error::Error>> {
        let env_only = super::parse_args_with_env(&[String::from("--control"), String::from("9125")], None, Some("  sample-env-token  "));
        let Ok(CliAction::Launch(env_cfg)) = env_only else {
            return Err(String::from("expected launch config from env token").into());
        };
        assert_eq!(env_cfg.control_port, Some(9125));
        assert_eq!(env_cfg.control_token.as_deref(), Some("sample-env-token"));

        let cli_wins = super::parse_args_with_env(
            &[String::from("--control"), String::from("9126"), String::from("--control-token"), String::from("sample-cli-token")],
            None,
            Some("sample-env-token"),
        );
        let Ok(CliAction::Launch(cli_cfg)) = cli_wins else {
            return Err(String::from("expected launch config from cli token").into());
        };
        assert_eq!(cli_cfg.control_token.as_deref(), Some("sample-cli-token"));
        assert!(super::parse_args_with_env(&[String::from("--control-token"), String::from("")], None, None).is_err());
        assert!(super::parse_args_with_env(&[String::from("--control-token")], None, None).is_err());

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-ctrl-tok-file-{}-{nanos}", std::process::id()));
        let paths = super::resolve_app_paths(None, Some(&dir))?;
        let token_path = super::control_token_file_path(&paths.config_dir, 9127);
        let auth = super::resolve_control_auth(&LaunchConfig::default())?;
        assert_eq!(auth.as_str().len(), curvebook_ui_egui::CONTROL_AUTH_TOKEN_BYTES * 2);

        super::write_control_token_file(&token_path, &auth)?;
        {
            let _guard = super::ControlTokenFileGuard { path: token_path.clone() };
            assert_eq!(std::fs::read_to_string(&token_path)?, auth.as_str());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&token_path)?.permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "control token file must use 0600 permissions");
            }
        }
        assert!(!token_path.exists(), "ControlTokenFileGuard must remove token file on drop");

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn r1_s11_02_launch_failure_releases_acquired_recovery_lock() -> Result<(), Box<dyn std::error::Error>> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-main-lock-cleanup-{}-{nanos}", std::process::id()));
        let recovery_dir = dir.join("curvebook-data").join("recovery");

        // Trigger failure AFTER opening window + acquiring recovery lock (`--event-script` missing file).
        let failing_cfg = LaunchConfig {
            portable_dir: Some(dir.clone()),
            event_script: Some(dir.join("definitely-missing-script.jsonl")),
            ..LaunchConfig::default()
        };
        let err = super::launch_application_with_env(&failing_cfg, |_| None).expect_err("missing --event-script should fail launch");
        assert!(err.contains("--event-script"));
        assert!(!recovery_dir.join(curvebook_format::LOCKFILE_NAME).exists(), "failed launch must still release curvebook.lock");

        // Immediate relaunch without waiting for TTL must succeed cleanly.
        let ok_cfg = LaunchConfig { headless_once: true, portable_dir: Some(dir.clone()), ..LaunchConfig::default() };
        super::launch_application_with_env(&ok_cfg, |_| None)?;
        assert!(!recovery_dir.join(curvebook_format::LOCKFILE_NAME).exists());

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn r1_s11_04_multi_file_args_and_csv_dirty_flag() -> Result<(), Box<dyn std::error::Error>> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-main-s11-04-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)?;

        let proj_a = dir.join("a.curvebook");
        let mut doc_a = ProjectDocument::new("Project Alpha")?;
        doc_a.add_data_sheet(DataSheet::new("data-a", "Alpha Table", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2)?))?)?;
        save_project_file(&proj_a, &doc_a)?;

        let proj_b = dir.join("b.curvebook");
        let doc_b = ProjectDocument::new("Project Beta")?;
        save_project_file(&proj_b, &doc_b)?;

        let csv_path = dir.join("extra.csv");
        std::fs::write(&csv_path, "X,Y\n1,2\n3,4\n")?;

        // 1. Multiple project archives must return an explicit error rather than silently dropping the first.
        let multi_err = load_startup_session(&[proj_a.clone(), proj_b]).expect_err("multiple project archives must fail");
        assert!(multi_err.contains("cannot open multiple project files"));

        // 2. CSV before project archive loads archive AND merges CSV, marking session dirty.
        let before = load_startup_session(&[csv_path.clone(), proj_a.clone()])?;
        assert_eq!(before.document().title, "Project Alpha");
        assert_eq!(before.document().data_sheets.len(), 2);
        assert_eq!(before.file_path(), Some(proj_a.as_path()));
        assert!(before.is_dirty(), "merging CSV before project archive must mark session dirty");

        // 3. CSV after project archive loads archive AND merges CSV, marking session dirty.
        let after = load_startup_session(&[proj_a.clone(), csv_path])?;
        assert_eq!(after.document().title, "Project Alpha");
        assert_eq!(after.document().data_sheets.len(), 2);
        assert_eq!(after.file_path(), Some(proj_a.as_path()));
        assert!(after.is_dirty(), "merging CSV after project archive must mark session dirty");

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn r1_s11_05_corrupt_prefs_and_shortcuts_warn_instead_of_aborting_launch() -> Result<(), Box<dyn std::error::Error>> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-main-s11-05-{}-{nanos}", std::process::id()));
        let data_dir = dir.join("curvebook-data");
        std::fs::create_dir_all(&data_dir)?;

        // Corrupt shortcuts.json must warn and fall back to defaults instead of aborting launch.
        std::fs::write(data_dir.join("shortcuts.json"), "{definitely invalid json")?;
        let headless_cfg = LaunchConfig { headless_once: true, portable_dir: Some(dir.clone()), ..LaunchConfig::default() };
        super::launch_application_with_env(&headless_cfg, |_| None)?;

        // Corrupt preferences.json must warn at launch and on exit without aborting cleanup.
        std::fs::write(data_dir.join("preferences.json"), "{broken preferences json")?;
        super::launch_application_with_env(&headless_cfg, |_| None)?;

        // Swapped shortcuts.json loads cleanly through configure_window_preferences.
        std::fs::remove_file(data_dir.join("preferences.json"))?;
        std::fs::write(data_dir.join("shortcuts.json"), r#"{"ui.palette.open":"F6","ui.pane.cycle":"Cmd+K"}"#)?;
        let app_paths = super::resolve_app_paths(None, Some(&dir))?;
        let mut app = curvebook_ui_egui::UiApp::new();
        super::configure_window_preferences(&mut app, &app_paths, &LaunchConfig::default(), false);
        assert_eq!(app.shortcuts.active_combo("ui.palette.open"), Some("F6"));
        assert_eq!(app.shortcuts.active_combo("ui.pane.cycle"), Some("Cmd+K"));

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn r1_s11_06_config_dir_and_no_prefs_env_vars() -> Result<(), Box<dyn std::error::Error>> {
        use std::ffi::OsString;

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-main-s11-06-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let env_cfg_dir = dir.join("custom-cfg");
        std::fs::create_dir_all(&env_cfg_dir)?;

        let paths =
            super::resolve_app_paths_with_env(
                None,
                None,
                |key| {
                    if key == "CURVEBOOK_CONFIG_DIR" { Some(OsString::from(&env_cfg_dir)) } else { None }
                },
            )?;
        assert_eq!(paths.config_dir, env_cfg_dir);
        assert_eq!(paths.preferences_file, env_cfg_dir.join("preferences.json"));
        assert_eq!(paths.shortcuts_file, env_cfg_dir.join("shortcuts.json"));
        assert_eq!(paths.recovery_dir, env_cfg_dir.join("recovery"));

        // With CURVEBOOK_NO_PREFS=1 (and true/yes), corrupt prefs are skipped, no prefs are saved,
        // and no shared recovery lock/journal is touched even on non-headless event script execution.
        let mut prefs = curvebook_format::versioned::PreferencesFile::default();
        prefs.shortcuts.insert(String::from("ui.gallery.toggle"), String::from("Ctrl+Alt+G"));
        curvebook_format::versioned::save_preferences_file(&paths.preferences_file, &prefs)?;

        let script_path = dir.join("noop.jsonl");
        std::fs::write(&script_path, "{\"WindowFocused\":{\"window_id\":1}}\n")?;
        let cfg = LaunchConfig { event_script: Some(script_path), ..LaunchConfig::default() };

        for val in ["1", "true", "YES"] {
            let env_dir = env_cfg_dir.clone();
            let flag = OsString::from(val);
            super::launch_application_with_env(&cfg, move |key| match key {
                "CURVEBOOK_CONFIG_DIR" => Some(OsString::from(&env_dir)),
                "CURVEBOOK_NO_PREFS" => Some(flag.clone()),
                _ => None,
            })?;
            assert!(!paths.recovery_dir.exists(), "CURVEBOOK_NO_PREFS={val} must not touch recovery_dir");
        }

        // Verify a customized app does NOT write preferences when CURVEBOOK_NO_PREFS is true.
        let mut custom_app = curvebook_ui_egui::UiApp::new();
        custom_app.shortcuts.set_shortcut("ui.gallery.toggle", "Ctrl+Shift+Y")?;
        let fresh_paths = super::resolve_app_paths(None, Some(&dir.join("unused-portable")))?;
        super::persist_window_preferences_best_effort(&custom_app, &fresh_paths, true);
        assert!(!fresh_paths.preferences_file.exists());

        // Contrast: without CURVEBOOK_NO_PREFS, CURVEBOOK_CONFIG_DIR is used for recovery_dir.
        let env_dir_enabled = env_cfg_dir.clone();
        super::launch_application_with_env(
            &cfg,
            move |key| {
                if key == "CURVEBOOK_CONFIG_DIR" { Some(OsString::from(&env_dir_enabled)) } else { None }
            },
        )?;
        assert!(paths.recovery_dir.is_dir(), "expected CURVEBOOK_CONFIG_DIR/recovery when CURVEBOOK_NO_PREFS is unset");

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn r1_s11_07_persists_locale_theme_and_shortcuts_across_control_and_gui_close() -> Result<(), Box<dyn std::error::Error>> {
        use curvebook::desktop_host::{DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime};
        use curvebook_ui_egui::{ControlAuth, UiThemeMode, ViewportMetrics, serve_control_stream_with_quit_and_hook};

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-main-s11-07-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let app_paths = super::resolve_app_paths(None, Some(&dir))?;
        std::fs::create_dir_all(&app_paths.config_dir)?;

        // Part A: Versioned PreferencesFile handles `"shortcuts": null` and swapped shortcuts + locale/theme cleanly.
        std::fs::write(&app_paths.preferences_file, r#"{"version":1,"locale":"fr","theme":"dark","shortcuts":null}"#)?;
        let mut reloaded_null = curvebook_ui_egui::UiApp::new();
        super::configure_window_preferences(&mut reloaded_null, &app_paths, &LaunchConfig::default(), false);
        assert_eq!(reloaded_null.locale, "fr");
        assert_eq!(reloaded_null.theme.mode, UiThemeMode::Dark);
        reloaded_null.shortcuts.set_shortcut("ui.palette.open", "Ctrl+Shift+F9")?;
        reloaded_null.shortcuts.set_shortcut("ui.pane.cycle", "Cmd+K")?;
        reloaded_null.shortcuts.set_shortcut("ui.palette.open", "F6")?;
        super::persist_window_preferences_best_effort(&reloaded_null, &app_paths, false);

        let mut swapped_app = curvebook_ui_egui::UiApp::new();
        super::configure_window_preferences(&mut swapped_app, &app_paths, &LaunchConfig::default(), false);
        assert_eq!(swapped_app.locale, "fr");
        assert_eq!(swapped_app.theme.mode, UiThemeMode::Dark);
        assert_eq!(swapped_app.shortcuts.active_combo("ui.palette.open"), Some("F6"));
        assert_eq!(swapped_app.shortcuts.active_combo("ui.pane.cycle"), Some("Cmd+K"));

        // Part B: GUI DesktopHostRuntime close path persists locale, theme, and shortcuts even on dirty close ("Don't Save").
        let mut rt =
            DesktopHostRuntime::new(DesktopHostConfig { initial_viewport: ViewportMetrics::new(800.0, 600.0, 2.0), ..DesktopHostConfig::default() });
        let win_id = rt.open_window(None, ViewportMetrics::new(800.0, 600.0, 2.0))?;
        let hook_paths = app_paths.clone();
        rt.set_preferences_persist_hook(move |app| {
            super::persist_window_preferences_best_effort(app, &hook_paths, false);
        });
        {
            let win = rt.window_mut(win_id)?;
            win.app_mut().set_locale("de");
            win.app_mut().set_theme_mode(UiThemeMode::HighContrast);
            win.app_mut().shortcuts.set_shortcut("ui.gallery.toggle", "Ctrl+Shift+U")?;
            let _ = win.app_mut().session_mut().execute("sheet.createData", serde_json::json!({ "sheet_id": "dirty-s11", "title": "DirtyTable" }))?;
        }
        let _ = rt.dispatch(DesktopHostEvent::WindowCloseRequested { window_id: win_id, force: false })?;
        let mut post_gui = curvebook_ui_egui::UiApp::new();
        super::configure_window_preferences(&mut post_gui, &app_paths, &LaunchConfig::default(), false);
        assert_eq!(post_gui.locale, "de");
        assert_eq!(post_gui.theme.mode, UiThemeMode::HighContrast);
        assert_eq!(post_gui.shortcuts.active_combo("ui.gallery.toggle"), Some("Ctrl+Shift+U"));

        // Discard dirty prompt via close_all_force after another preference change; must still persist.
        rt.window_mut(win_id)?.app_mut().set_theme_mode(UiThemeMode::Light);
        rt.close_all_force()?;
        let mut post_force = curvebook_ui_egui::UiApp::new();
        super::configure_window_preferences(&mut post_force, &app_paths, &LaunchConfig::default(), false);
        assert_eq!(post_force.theme.mode, UiThemeMode::Light);
        assert_eq!(post_force.locale, "de");

        // Part C: Authenticated `--control` post-request hook immediately persists locale, theme, and shortcuts.
        let mut ctrl_rt =
            DesktopHostRuntime::new(DesktopHostConfig { initial_viewport: ViewportMetrics::new(800.0, 600.0, 2.0), ..DesktopHostConfig::default() });
        let ctrl_win_id = ctrl_rt.open_window(None, ViewportMetrics::new(800.0, 600.0, 2.0))?;
        let auth_tok = ControlAuth::generate()?;
        let script = [
            serde_json::json!({"id": 1, "method": "auth", "params": {"token": auth_tok.as_str()}}).to_string(),
            String::from(r#"{"id":2,"method":"ui.locale.set","params":{"locale":"ja"}}"#),
            String::from(r#"{"id":3,"method":"ui.theme.set","params":{"mode":"dark"}}"#),
            String::from(r#"{"id":4,"method":"ui.shortcut.set","params":{"command_id":"ui.palette.open","shortcut":"F7"}}"#),
            String::from(r#"{"id":5,"method":"app.quit","params":{}}"#),
        ]
        .join("\n")
            + "\n";
        let mut out_bytes = Vec::new();
        let mut observed_immediate_pre_quit = false;
        super::serve_control_with_pref_persistence(ctrl_rt.window_mut(ctrl_win_id)?, &app_paths, false, |app, hook| {
            let mut reader = std::io::BufReader::new(std::io::Cursor::new(script.as_bytes()));
            let quit = serve_control_stream_with_quit_and_hook(app, &mut reader, &mut out_bytes, &auth_tok, |app_ref, ctx| {
                hook(app_ref, ctx)?;
                if ctx.method == "ui.shortcut.set" {
                    let saved = std::fs::read_to_string(&app_paths.preferences_file).map_err(|e| e.to_string())?;
                    if saved.contains("\"ja\"") && saved.contains("\"dark\"") && saved.contains("\"F7\"") {
                        observed_immediate_pre_quit = true;
                    }
                }
                Ok(())
            })
            .map_err(std::io::Error::other)?;
            assert!(quit, "expected app.quit to stop control stream");
            Ok(())
        })?;
        assert!(observed_immediate_pre_quit, "expected immediate pre-quit preference persistence during control stream");
        let resp_text = String::from_utf8(out_bytes)?;
        assert_eq!(resp_text.lines().count(), 5);
        for line in resp_text.lines() {
            assert!(line.contains("\"ok\":true"), "control response error: {line}");
        }
        ctrl_rt.close_all_force()?;

        let mut post_ctrl = curvebook_ui_egui::UiApp::new();
        super::configure_window_preferences(&mut post_ctrl, &app_paths, &LaunchConfig::default(), false);
        assert_eq!(post_ctrl.locale, "ja");
        assert_eq!(post_ctrl.theme.mode, UiThemeMode::Dark);
        assert_eq!(post_ctrl.shortcuts.active_combo("ui.palette.open"), Some("F7"));

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn r1_s11_09_unset_home_uses_private_uid_dirs_and_rejects_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        use std::ffi::OsString;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-main-s11-09-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)?;

        let uid = super::current_unix_uid()?;
        let uid_dir_name = format!("curvebook-uid-{uid}");

        // Case 1: HOME/XDG unset creates per-UID 0700 dirs (never fixed shared /tmp/curvebook).
        let tmp_ok = dir.join("tmp-ok");
        std::fs::create_dir_all(&tmp_ok)?;
        let tmp_ok_os = OsString::from(&tmp_ok);
        let paths = super::resolve_app_paths_with_env(None, None, |key| if key == "TMPDIR" { Some(tmp_ok_os.clone()) } else { None })?;
        assert!(paths.config_dir.to_string_lossy().contains(&uid_dir_name));
        assert!(paths.recovery_dir.to_string_lossy().contains(&uid_dir_name));
        assert_ne!(paths.config_dir, std::env::temp_dir().join("curvebook"));
        let uid_root = tmp_ok.join(&uid_dir_name);
        let root_meta = std::fs::symlink_metadata(&uid_root)?;
        assert!(!root_meta.file_type().is_symlink());
        assert!(root_meta.is_dir());
        assert_eq!(root_meta.uid(), uid);
        assert_eq!(root_meta.mode() & 0o777, 0o700);

        // Case 2: Pre-planted symlink at curvebook-uid-<uid> is rejected explicitly.
        let tmp_sym = dir.join("tmp-sym");
        std::fs::create_dir_all(&tmp_sym)?;
        let decoy = dir.join("decoy");
        std::fs::create_dir_all(&decoy)?;
        std::os::unix::fs::symlink(&decoy, tmp_sym.join(&uid_dir_name))?;
        let tmp_sym_os = OsString::from(&tmp_sym);
        assert!(super::resolve_app_paths_with_env(None, None, |key| if key == "TMPDIR" { Some(tmp_sym_os.clone()) } else { None }).is_err());

        // Case 3: Permissive 0755 fallback dir is rejected explicitly.
        let tmp_perm = dir.join("tmp-perm");
        std::fs::create_dir_all(&tmp_perm)?;
        let bad_dir = tmp_perm.join(&uid_dir_name);
        std::fs::create_dir_all(&bad_dir)?;
        std::fs::set_permissions(&bad_dir, std::fs::Permissions::from_mode(0o755))?;
        let tmp_perm_os = OsString::from(&tmp_perm);
        assert!(super::resolve_app_paths_with_env(None, None, |key| if key == "TMPDIR" { Some(tmp_perm_os.clone()) } else { None }).is_err());

        // Case 4: Valid 0700 XDG_RUNTIME_DIR works; permissive or symlinked XDG_RUNTIME_DIR fails explicitly.
        let xdg_ok = dir.join("xdg-ok");
        std::fs::create_dir_all(&xdg_ok)?;
        std::fs::set_permissions(&xdg_ok, std::fs::Permissions::from_mode(0o700))?;
        let xdg_ok_os = OsString::from(&xdg_ok);
        let xdg_paths = super::resolve_app_paths_with_env(None, None, |key| if key == "XDG_RUNTIME_DIR" { Some(xdg_ok_os.clone()) } else { None })?;
        assert!(xdg_paths.config_dir.starts_with(xdg_ok.join("curvebook")));

        std::fs::set_permissions(&xdg_ok, std::fs::Permissions::from_mode(0o755))?;
        assert!(super::resolve_app_paths_with_env(None, None, |key| if key == "XDG_RUNTIME_DIR" { Some(xdg_ok_os.clone()) } else { None }).is_err());

        let xdg_sym = dir.join("xdg-sym");
        std::os::unix::fs::symlink(&decoy, &xdg_sym)?;
        let xdg_sym_os = OsString::from(&xdg_sym);
        assert!(super::resolve_app_paths_with_env(None, None, |key| if key == "XDG_RUNTIME_DIR" { Some(xdg_sym_os.clone()) } else { None }).is_err());

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn r1_s11_13_env_control_port_double_dash_and_new_instance() -> Result<(), Box<dyn std::error::Error>> {
        use std::ffi::OsString;

        assert!(super::USAGE.contains("[--]"));
        assert!(super::USAGE.contains("--new-instance"));

        // 1. Invalid CURVEBOOK_CONTROL_PORT is ignored on uncontrolled launches instead of aborting.
        for bad_port in ["abc", "0", "65536", "-1", "127.0.0.1:9125"] {
            let parsed = super::parse_args_with_env(&[String::from("--headless-once")], Some(bad_port), None)?;
            let CliAction::Launch(cfg) = parsed else {
                return Err(String::from("expected Launch action").into());
            };
            assert!(cfg.headless_once);
            assert_eq!(cfg.control_port, None, "bad env port `{bad_port}` should be ignored");
        }

        // Valid CURVEBOOK_CONTROL_PORT still applies; CLI --control takes precedence and stays strict.
        let env_valid = super::parse_args_with_env(&[], Some(" 9125 "), None)?;
        let CliAction::Launch(env_cfg) = env_valid else {
            return Err(String::from("expected Launch action").into());
        };
        assert_eq!(env_cfg.control_port, Some(9125));

        let cli_override = super::parse_args_with_env(&[String::from("--control"), String::from("9126")], Some("abc"), None)?;
        let CliAction::Launch(cli_cfg) = cli_override else {
            return Err(String::from("expected Launch action").into());
        };
        assert_eq!(cli_cfg.control_port, Some(9126));
        assert!(super::parse_args_with_env(&[String::from("--control"), String::from("0")], None, None).is_err());
        assert!(super::parse_args_with_env(&[String::from("--control"), String::from("abc")], None, None).is_err());

        // Invalid CURVEBOOK_CONTROL_TOKEN only aborts when control mode is actually enabled.
        let ignored_tok = super::parse_args_with_env(&[String::from("--headless-once")], Some("abc"), Some("bad token"))?;
        let CliAction::Launch(ignored_cfg) = ignored_tok else {
            return Err(String::from("expected Launch action").into());
        };
        assert_eq!(ignored_cfg.control_port, None);
        assert_eq!(ignored_cfg.control_token, None);
        assert!(super::parse_args_with_env(&[String::from("--control"), String::from("9126")], None, Some("bad token")).is_err());
        assert!(super::parse_args_with_env(&[], Some("9125"), Some("bad token")).is_err());

        // Non-UTF-8 CURVEBOOK_CONTROL_PORT is ignored on uncontrolled launches; non-UTF-8 token fails when control is enabled.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let non_utf8 = OsString::from_vec(vec![0xff, 0xfe]);
            super::run_with_process_env(&[String::from("--headless-once")], None, None, |key| {
                if key == "CURVEBOOK_CONTROL_PORT" { Some(non_utf8.clone()) } else { None }
            })?;
            let token_err = super::run_with_process_env(&[String::from("--control"), String::from("9126")], None, None, |key| {
                if key == "CURVEBOOK_CONTROL_TOKEN" { Some(non_utf8.clone()) } else { None }
            })
            .expect_err("non-UTF-8 token must fail when --control is enabled");
            assert!(token_err.contains("CURVEBOOK_CONTROL_TOKEN"));
        }

        // 2. `--` ends option parsing so `-`-prefixed file paths are accepted, while bare `-` flags are still rejected.
        let dash_err = super::parse_args_with_env(&[String::from("-data.csv")], None, None).expect_err("bare -data.csv must be rejected");
        assert!(dash_err.contains("unknown option"));
        assert!(dash_err.contains("--"));

        let terminated = super::parse_args_with_env(
            &[
                String::from("--headless-once"),
                String::from("--"),
                String::from("-data.csv"),
                String::from("--help"),
                String::from("-V"),
                String::from("--control"),
                String::from("--"),
            ],
            None,
            None,
        )?;
        let CliAction::Launch(term_cfg) = terminated else {
            return Err(String::from("expected Launch action for post-`--` args").into());
        };
        assert!(term_cfg.headless_once);
        assert_eq!(
            term_cfg.files,
            vec![PathBuf::from("-data.csv"), PathBuf::from("--help"), PathBuf::from("-V"), PathBuf::from("--control"), PathBuf::from("--"),]
        );

        let empty_terminated = super::parse_args_with_env(&[String::from("--")], None, None)?;
        let CliAction::Launch(empty_term_cfg) = empty_terminated else {
            return Err(String::from("expected Launch action for bare `--`").into());
        };
        assert!(empty_term_cfg.files.is_empty());

        // 3. `--new-instance` opens an isolated recovery dir without colliding on an active primary lock.
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-main-s11-13-{}-{nanos}", std::process::id()));
        let paths = super::resolve_app_paths(None, Some(&dir))?;
        std::fs::create_dir_all(&paths.recovery_dir)?;

        let (_primary_session, primary_journal, _report) =
            curvebook_engine::Session::open_dir_with_recovery(&paths.recovery_dir, "active-primary", 300_000, super::now_epoch_ms())?;

        let single_cfg = LaunchConfig { headless_once: true, portable_dir: Some(dir.clone()), new_instance: false, ..LaunchConfig::default() };
        assert!(super::launch_application_with_env(&single_cfg, |_| None).is_err(), "single-instance launch must fail when primary lock is held");

        let dash_csv = dir.join("-data.csv");
        std::fs::write(&dash_csv, "X,Y\n1,2\n3,4\n")?;
        let new_inst_cfg = LaunchConfig {
            headless_once: true,
            portable_dir: Some(dir.clone()),
            new_instance: true,
            files: vec![dash_csv],
            ..LaunchConfig::default()
        };
        super::launch_application_with_env(&new_inst_cfg, |_| None)?;
        assert!(paths.recovery_dir.join(curvebook_format::LOCKFILE_NAME).is_file(), "primary lock must remain untouched");

        drop(primary_journal.abandon());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn live_window_control_server_roundtrip_and_token_cleanup() -> Result<(), Box<dyn std::error::Error>> {
        use std::io::{BufRead, BufReader, BufWriter, ErrorKind, Write};
        use std::net::{SocketAddr, TcpStream};
        use std::time::{Duration, Instant};

        use curvebook::desktop_host::{DesktopHostConfig, DesktopHostRuntime};
        use curvebook::gui_window::{CurveBookNativeApp, build_egui_font_definitions};
        use curvebook_ui_egui::{ControlAuth, ViewportMetrics, bind_control_tcp_listener};

        let listener = match bind_control_tcp_listener(SocketAddr::from(([127, 0, 0, 1], 0))) {
            Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
            other => other?,
        };
        let addr = listener.local_addr()?;
        match TcpStream::connect_timeout(&addr, Duration::from_millis(250)) {
            Ok(probe) => {
                drop(probe);
                let _ = listener.accept();
            }
            Err(err) if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::TimedOut) => return Ok(()),
            Err(err) => return Err(Box::new(err)),
        }

        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cb-main-live-win-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let app_paths = super::resolve_app_paths(None, Some(&dir))?;
        let token_path = super::control_token_file_path(&app_paths.config_dir, addr.port());
        let auth = ControlAuth::generate()?;
        super::write_control_token_file(&token_path, &auth)?;
        let token_guard = super::ControlTokenFileGuard { path: token_path.clone() };

        let mut rt = DesktopHostRuntime::new(DesktopHostConfig {
            in_window_menus: true,
            new_instance: false,
            gui_only: false,
            ui_backend: UiBackend::Egui,
            locale_override: None,
            theme_override: None,
            recovery_root: None,
            initial_viewport: ViewportMetrics::new(960.0, 640.0, 1.0),
            ..DesktopHostConfig::default()
        });
        let win_id = rt.open_window(None, ViewportMetrics::new(960.0, 640.0, 1.0))?;
        let (guards, channel) = super::start_live_window_control_on_listener(listener, auth.clone(), &app_paths, false, token_guard)?;

        let ctx = egui::Context::default();
        ctx.set_fonts(build_egui_font_definitions(None));
        let mut native_app = CurveBookNativeApp::new(rt, win_id, None).with_live_control(channel);

        let token_owned = auth.as_str().to_owned();
        let client = std::thread::spawn(move || -> Result<Vec<serde_json::Value>, String> {
            let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
            let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
            let mut writer = BufWriter::new(stream);
            let reqs = [
                format!("{{\"id\":1,\"method\":\"auth\",\"params\":{{\"token\":\"{token_owned}\"}}}}\n"),
                String::from("{\"id\":2,\"method\":\"sheet.createData\",\"params\":{\"sheet_id\":\"live-1\",\"title\":\"Live Window\"}}\n"),
                String::from("{\"id\":3,\"method\":\"ui.theme.set\",\"params\":{\"mode\":\"dark\"}}\n"),
                String::from("{\"id\":4,\"method\":\"app.quit\"}\n"),
            ];
            let mut replies = Vec::new();
            for req in reqs {
                writer.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
                writer.flush().map_err(|e| e.to_string())?;
                let mut line = String::new();
                reader.read_line(&mut line).map_err(|e| e.to_string())?;
                let parsed: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
                replies.push(parsed);
            }
            Ok(replies)
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        while !native_app.close_confirmed() && Instant::now() < deadline {
            let raw_input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(960.0, 640.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw_input, |ui| native_app.step_ui(ui));
            out.textures_delta.clear();
            std::thread::sleep(Duration::from_millis(5));
        }

        let replies = client.join().map_err(|_| "client thread panicked")??;
        assert_eq!(replies.len(), 4);
        assert!(replies.iter().all(|r| r["ok"] == serde_json::json!(true)));
        assert!(native_app.close_confirmed());
        let win = native_app.runtime().window(win_id)?;
        assert!(win.app().document().data_sheet(&curvebook_doc::SheetId::new("live-1")).is_some());
        assert_eq!(win.app().theme.mode, curvebook_ui_egui::UiThemeMode::Dark);
        assert!(app_paths.preferences_file.is_file());

        native_app.finish_exit();
        drop(guards);
        assert!(!token_path.exists(), "token file must be removed when live server guards drop");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn rv51_control_token_help_and_cli_argv_warning_semantics() -> Result<(), Box<dyn std::error::Error>> {
        assert!(super::USAGE.contains("prefer CURVEBOOK_CONTROL_TOKEN"), "USAGE should recommend CURVEBOOK_CONTROL_TOKEN");
        assert!(super::USAGE.contains("ps"), "USAGE should mention process list (`ps`) visibility");

        assert!(super::args_contain_cli_control_token(&[
            String::from("--control"),
            String::from("9125"),
            String::from("--control-token"),
            String::from("secret-tok"),
        ]));
        assert!(super::args_contain_cli_control_token(&[
            String::from("--ui-backend"),
            String::from("egui"),
            String::from("--control-token"),
            String::from("secret-tok"),
        ]));
        assert!(!super::args_contain_cli_control_token(&[String::from("--ui-backend"), String::from("--control-token")]));
        assert!(!super::args_contain_cli_control_token(&[String::from("--control"), String::from("9125")]));
        assert!(!super::args_contain_cli_control_token(&[String::from("--headless-once"), String::from("--"), String::from("--control-token"),]));

        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cb-rv51-tok-warn-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir)?;

        let mut stderr_buf = Vec::new();
        super::run_with_process_env_and_stderr(
            &[
                String::from("--portable-dir"),
                dir.display().to_string(),
                String::from("--headless-once"),
                String::from("--control-token"),
                String::from("cli-visible-token-123"),
            ],
            None,
            None,
            |_| None,
            &mut stderr_buf,
        )?;
        let stderr = String::from_utf8(stderr_buf)?;
        assert!(
            stderr.contains("curvebook warning:") && stderr.contains("CURVEBOOK_CONTROL_TOKEN"),
            "expected stderr warning recommending CURVEBOOK_CONTROL_TOKEN, got: {stderr}"
        );
        assert!(!stderr.contains("cli-visible-token-123"), "warning must never echo the secret token value");

        let mut help_stderr = Vec::new();
        super::run_with_process_env_and_stderr(
            &[String::from("--control-token"), String::from("cli-visible-token-123"), String::from("--help")],
            None,
            None,
            |_| None,
            &mut help_stderr,
        )?;
        assert!(help_stderr.is_empty(), "--help must keep stderr empty");

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn r15_1_ui_backend_flag_env_and_preferences_ui_scale_migration() -> Result<(), Box<dyn std::error::Error>> {
        let default_egui = super::parse_args_with_env_and_backend(&[], None, None, None)?;
        assert!(matches!(default_egui, CliAction::Launch(ref c) if c.ui_backend == UiBackend::Egui));

        let env_egui = super::parse_args_with_env_and_backend(&[], None, None, Some("egui"))?;
        assert!(matches!(env_egui, CliAction::Launch(ref c) if c.ui_backend == UiBackend::Egui));

        let cli_legacy_err = super::parse_args_with_env_and_backend(&[String::from("--ui-backend=legacy")], None, None, Some("egui"));
        assert!(cli_legacy_err.is_err(), "--ui-backend=legacy must be rejected post-R15.5 cutover");

        let env_legacy_err = super::parse_args_with_env_and_backend(&[], None, None, Some("legacy"));
        assert!(env_legacy_err.is_err(), "CURVEBOOK_UI_BACKEND=legacy must be rejected post-R15.5 cutover");

        let cli_split_flag = super::parse_args_with_env_and_backend(&[String::from("--ui-backend"), String::from("egui")], None, None, None)?;
        assert!(matches!(cli_split_flag, CliAction::Launch(ref c) if c.ui_backend == UiBackend::Egui));

        assert!(super::parse_args_with_env_and_backend(&[String::from("--ui-backend=bogus")], None, None, None).is_err());

        // Preferences migration for `ui_scale`, RTL locale fallback (`ar` -> `en`), and newer version refusal.
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cb-r15-prefs-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let paths = super::resolve_app_paths(None, Some(&dir))?;
        if let Some(parent) = paths.preferences_file.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Legacy v0 file with `"scale": 1.35` and `"locale": "ar"` migrates safely.
        std::fs::write(&paths.preferences_file, r#"{"locale":"ar","theme":"dark","scale":1.35}"#)?;
        let mut app = UiApp::new();
        super::load_shortcut_preferences(&mut app, &paths)?;
        assert_eq!(app.locale, "en", "RTL locale `ar` must normalize to `en` on Beta desktop host load");
        assert_eq!(app.theme.mode, UiThemeMode::Dark);
        assert!((app.ui_scale() - 1.35).abs() < 1e-4);

        app.zoom_ui_scale_in();
        super::save_shortcut_preferences(&app, &paths)?;
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&paths.preferences_file)?)?;
        assert_eq!(saved["version"], serde_json::json!(CURRENT_PREFERENCES_VERSION));
        assert_eq!(saved["ui_scale"], serde_json::json!(1.45));
        assert!(saved.get("scale").is_none());

        // Newer version file must be refused cleanly without overwriting.
        let future_body = r#"{"version":999,"locale":"en","theme":"light","ui_scale":2.0}"#;
        std::fs::write(&paths.preferences_file, future_body)?;
        let mut future_app = UiApp::new();
        assert!(super::load_shortcut_preferences(&mut future_app, &paths).is_err());
        assert!(super::save_shortcut_preferences(&future_app, &paths).is_err());
        assert_eq!(std::fs::read_to_string(&paths.preferences_file)?, future_body);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
