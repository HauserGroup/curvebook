//! Native desktop entry point for LumenCraft.
#![forbid(unsafe_code)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lumencraft::desktop_host::{DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime, open_document_from_files};
use lumencraft_engine::Session;
use lumencraft_format::recovery::{CrashRecoveryJournal, DirectoryJournalBackend, RecoveryReport};
use lumencraft_format::versioned::{CURRENT_PREFERENCES_VERSION, PreferencesFile, load_preferences_file, save_preferences_file};
use lumencraft_render::RenderTargetSpec;
use lumencraft_text::{TrueTypeFontInfo, parse_truetype_font_info};
use lumencraft_ui_egui::dialogs_canvas::render_workspace_frame_png;
use lumencraft_ui_egui::interaction::ViewportMetrics;
use lumencraft_ui_egui::theme::install_fonts;
use lumencraft_ui_egui::{UiApp, parse_control_port, run_control_tcp_server};

const RECOVERY_LOCK_TTL_MS: u64 = 300_000;

const USAGE: &str = "\
usage: lumencraft [options] [file...]

options:
  --control <port>      listen on 127.0.0.1:<port> for JSON-lines automation
  --gui-only            restrict automation (--control) to interactive GUI actions only
  --in-window-menus     render the menu bar inside the window on macOS
  --new-instance        open files in a separate application window
  --headless-once       render one software compositor frame and exit
  --portable-dir <dir>  store config, shortcuts, and crash-recovery journal under <dir>/lumencraft-data
  --ttf-font <path>     load and validate an external TrueType/SFNT (.ttf) font file
  --rulers              show physical canvas rulers on startup
  --event-script <path> replay newline-delimited DesktopHostEvent JSONL commands
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
    gui_only: bool,
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
    let env_port = std::env::var("LUMENCRAFT_CONTROL_PORT").ok();
    match run_with_env_port(&args, env_port.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
fn run(args: &[String]) -> Result<(), String> {
    run_with_env_port(args, None)
}

fn run_with_env_port(args: &[String], env_port: Option<&str>) -> Result<(), String> {
    let action = parse_args(args, env_port)?;
    match action {
        CliAction::PrintVersion => {
            println!("LumenCraft {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        CliAction::PrintHelp => {
            print!("{USAGE}");
            Ok(())
        }
        CliAction::Launch(cfg) => launch_application(&cfg),
    }
}

fn parse_args(args: &[String], env_port: Option<&str>) -> Result<CliAction, String> {
    let mut cfg = LaunchConfig::default();
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
            "--gui-only" => {
                cfg.gui_only = true;
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
            flag if flag.starts_with('-') => {
                return Err(format!("unknown option `{flag}`\n\n{USAGE}"));
            }
            path => {
                cfg.files.push(PathBuf::from(path));
                i += 1;
            }
        }
    }

    if cfg.control_port.is_none()
        && let Some(raw_env) = env_port.map(str::trim).filter(|s| !s.is_empty())
    {
        cfg.control_port = Some(parse_port(raw_env)?);
    }

    Ok(CliAction::Launch(cfg))
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

fn configure_window_preferences(app: &mut UiApp, app_paths: &AppPaths, cfg: &LaunchConfig) -> Result<(), String> {
    load_shortcut_preferences(app, app_paths)?;
    if cfg.rulers {
        app.canvas_dialogs.rulers_visible = true;
    }
    lumencraft::configure_gui_only(app, cfg.gui_only);
    app.sync_views();
    Ok(())
}

fn launch_application(cfg: &LaunchConfig) -> Result<(), String> {
    let _loaded_fonts = install_fonts();
    let _ = lumencraft_ui_egui::bundled_fonts_enabled();
    let _ = lumencraft_format::EXTENSION;

    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let app_paths = resolve_app_paths(exe_dir.as_deref(), cfg.portable_dir.as_deref());
    let _validated_font = cfg.ttf_font.as_deref().map(load_validated_ttf_font).transpose()?;

    let interactive_gui = !cfg.headless_once && cfg.event_script.is_none() && cfg.control_port.is_none();
    let enable_recovery = app_paths.portable || !cfg.headless_once;
    let initial_viewport = if interactive_gui { ViewportMetrics::new(1280.0, 800.0, 2.0) } else { ViewportMetrics::new(400.0, 260.0, 2.0) };
    let recovery_root = if cfg.new_instance && enable_recovery && can_create_recovery_root(&app_paths.recovery_dir)? {
        Some(app_paths.recovery_dir.clone())
    } else {
        None
    };

    let host_cfg = DesktopHostConfig {
        in_window_menus: cfg.in_window_menus,
        new_instance: cfg.new_instance,
        gui_only: cfg.gui_only,
        locale_override: None,
        theme_override: None,
        recovery_root,
        initial_viewport,
    };
    let mut runtime = DesktopHostRuntime::new(host_cfg);
    let mut single_instance_journal: Option<CrashRecoveryJournal<DirectoryJournalBackend>> = None;

    if cfg.new_instance {
        if cfg.files.is_empty() {
            let _ = runtime.dispatch(DesktopHostEvent::WindowOpened { initial_file: None, viewport: initial_viewport })?;
        } else {
            for path in &cfg.files {
                let _ = runtime.dispatch(DesktopHostEvent::WindowOpened { initial_file: Some(path.clone()), viewport: initial_viewport })?;
            }
        }
    } else {
        let startup = open_startup_session_with_recovery(&cfg.files, &app_paths, enable_recovery, now_epoch_ms())?;
        let _recovered_session = startup.report.as_ref().is_some_and(|r| r.recovered_from_stale_lock.is_some() || !r.recovered_commands.is_empty());
        single_instance_journal = startup.journal;
        let _ = runtime.open_window_with_session(startup.session, initial_viewport, false)?;
    }

    for win_id in runtime.window_ids() {
        let win = runtime.window_mut(win_id)?;
        configure_window_preferences(win.app_mut(), &app_paths, cfg)?;
    }

    if interactive_gui {
        let extra_ttf_bytes = cfg
            .ttf_font
            .as_deref()
            .map(|path| std::fs::read(path).map_err(|err| format!("failed to read `--ttf-font` `{}`: {err}", path.display())))
            .transpose()?;
        let cleanup = Box::new(move |rt: &mut DesktopHostRuntime| -> Result<(), String> {
            let mut journal = single_instance_journal;
            if let Some(focused_id) = rt.focused_window_id() {
                let win = rt.window_mut(focused_id)?;
                if should_persist_preferences(win.app(), &app_paths)? {
                    save_shortcut_preferences(win.app(), &app_paths)?;
                }
                close_startup_recovery_cleanly(win.app_mut(), journal.take())?;
            } else if let Some(pending_journal) = journal.take() {
                let mut fallback = Session::new();
                let _backend = fallback
                    .close_journal_cleanly(pending_journal)
                    .map_err(|err| format!("failed to close crash-recovery journal cleanly: {err}"))?;
            }
            Ok(())
        });
        return lumencraft::gui_window::run_native_desktop(runtime, extra_ttf_bytes, cleanup);
    }

    if let Some(script_path) = cfg.event_script.as_deref() {
        let script =
            std::fs::read_to_string(script_path).map_err(|err| format!("failed to read --event-script `{}`: {err}", script_path.display()))?;
        let _ = runtime.run_event_script_jsonl(&script)?;
    }

    if cfg.headless_once {
        for win_id in runtime.window_ids() {
            let redraw = runtime.dispatch(DesktopHostEvent::RedrawRequested { window_id: win_id })?;
            let frame = redraw.frame.ok_or_else(|| format!("missing headless frame for window `{win_id}`"))?;
            if frame.rgba_bytes.is_empty() || frame.width_px == 0 || frame.height_px == 0 {
                return Err(String::from("headless desktop host produced empty RGBA framebuffer"));
            }
            let win = runtime.window(win_id)?;
            let png = render_workspace_frame_png(win.app(), RenderTargetSpec { width_px: 800, height_px: 520, dpi: 144 })
                .map_err(|err| format!("headless frame render failed: {err}"))?;
            if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
                return Err(String::from("headless compositor produced invalid PNG signature"));
            }
        }
    }

    if let Some(port) = cfg.control_port {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let focused_id = runtime.focused_window_id().ok_or_else(|| String::from("no open window available for --control server"))?;
        let win = runtime.window_mut(focused_id)?;
        run_control_tcp_server(win.app_mut(), addr)?;
    }

    if let Some(focused_id) = runtime.focused_window_id() {
        let win = runtime.window_mut(focused_id)?;
        if should_persist_preferences(win.app(), &app_paths)? {
            save_shortcut_preferences(win.app(), &app_paths)?;
        }
        close_startup_recovery_cleanly(win.app_mut(), single_instance_journal.take())?;
    } else if let Some(journal) = single_instance_journal.take() {
        let mut fallback = Session::new();
        let _backend = fallback.close_journal_cleanly(journal).map_err(|err| format!("failed to close crash-recovery journal cleanly: {err}"))?;
    }

    runtime.close_all_force()?;
    Ok(())
}

fn should_persist_preferences(app: &UiApp, paths: &AppPaths) -> Result<bool, String> {
    if paths.preferences_file.is_file() {
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
}

fn now_epoch_ms() -> u64 {
    u64::try_from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())).unwrap_or(u64::MAX)
}

fn recovery_owner_id() -> String {
    format!("lumencraft-pid-{}", std::process::id())
}

fn open_startup_session_with_recovery(
    files: &[PathBuf],
    app_paths: &AppPaths,
    enable_recovery: bool,
    now_ms: u64,
) -> Result<StartupRecoveryState, String> {
    if !files.is_empty() || !enable_recovery {
        let session = load_startup_session(files)?;
        return Ok(StartupRecoveryState { session, journal: None, report: None });
    }

    if app_paths.portable || app_paths.recovery_dir.exists() {
        let (session, journal, report) = Session::open_dir_with_recovery(&app_paths.recovery_dir, recovery_owner_id(), RECOVERY_LOCK_TTL_MS, now_ms)
            .map_err(|err| format!("failed to open recovery journal in `{}`: {err}", app_paths.recovery_dir.display()))?;
        return Ok(StartupRecoveryState { session, journal: Some(journal), report: Some(report) });
    }

    match std::fs::create_dir_all(&app_paths.recovery_dir) {
        Ok(()) => {
            let (session, journal, report) =
                Session::open_dir_with_recovery(&app_paths.recovery_dir, recovery_owner_id(), RECOVERY_LOCK_TTL_MS, now_ms)
                    .map_err(|err| format!("failed to open recovery journal in `{}`: {err}", app_paths.recovery_dir.display()))?;
            Ok(StartupRecoveryState { session, journal: Some(journal), report: Some(report) })
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            Ok(StartupRecoveryState { session: Session::new(), journal: None, report: None })
        }
        Err(err) => Err(format!("failed to create recovery directory `{}`: {err}", app_paths.recovery_dir.display())),
    }
}

fn close_startup_recovery_cleanly(app: &mut UiApp, recovery_journal: Option<CrashRecoveryJournal<DirectoryJournalBackend>>) -> Result<(), String> {
    if let Some(journal) = recovery_journal {
        let _backend =
            app.session_mut().close_journal_cleanly(journal).map_err(|err| format!("failed to close crash-recovery journal cleanly: {err}"))?;
    }
    Ok(())
}

fn resolve_app_paths(exe_dir: Option<&Path>, explicit_portable: Option<&Path>) -> AppPaths {
    resolve_app_paths_with_env(exe_dir, explicit_portable, |key| std::env::var_os(key))
}

fn resolve_app_paths_with_env(exe_dir: Option<&Path>, explicit_portable: Option<&Path>, env_get: impl Fn(&str) -> Option<OsString>) -> AppPaths {
    if let Some(dir) = explicit_portable {
        return portable_app_paths(dir);
    }
    if let Some(dir) = exe_dir
        && dir.join("portable.txt").is_file()
    {
        return portable_app_paths(dir);
    }

    let (config_dir, data_dir) = resolve_platform_standard_dirs(&env_get);
    AppPaths {
        portable: false,
        preferences_file: config_dir.join("preferences.json"),
        shortcuts_file: config_dir.join("shortcuts.json"),
        recovery_dir: data_dir.join("recovery"),
        config_dir,
        data_dir,
    }
}

fn portable_app_paths(base_dir: &Path) -> AppPaths {
    let root =
        if base_dir.file_name().is_some_and(|name| name == "lumencraft-data") { base_dir.to_path_buf() } else { base_dir.join("lumencraft-data") };
    AppPaths {
        portable: true,
        preferences_file: root.join("preferences.json"),
        shortcuts_file: root.join("shortcuts.json"),
        recovery_dir: root.join("recovery"),
        config_dir: root.clone(),
        data_dir: root,
    }
}

fn resolve_platform_standard_dirs(env_get: &impl Fn(&str) -> Option<OsString>) -> (PathBuf, PathBuf) {
    let nonempty_path = |key: &str| env_get(key).filter(|v| !v.is_empty()).map(PathBuf::from);

    #[cfg(target_os = "macos")]
    {
        let base = nonempty_path("HOME")
            .map(|h| h.join("Library").join("Application Support").join("LumenCraft"))
            .unwrap_or_else(|| std::env::temp_dir().join("lumencraft"));
        (base.clone(), base)
    }
    #[cfg(target_os = "windows")]
    {
        let fallback_base = nonempty_path("USERPROFILE")
            .map(|h| h.join("AppData").join("Roaming").join("LumenCraft"))
            .unwrap_or_else(|| std::env::temp_dir().join("lumencraft"));
        let config_dir = nonempty_path("APPDATA").map(|p| p.join("LumenCraft")).unwrap_or(fallback_base);
        let data_dir = nonempty_path("LOCALAPPDATA").map(|p| p.join("LumenCraft")).unwrap_or_else(|| config_dir.clone());
        (config_dir, data_dir)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let home = nonempty_path("HOME");
        let config_dir = nonempty_path("XDG_CONFIG_HOME")
            .map(|p| p.join("lumencraft"))
            .or_else(|| home.as_ref().map(|h| h.join(".config").join("lumencraft")))
            .unwrap_or_else(|| std::env::temp_dir().join("lumencraft-config"));
        let data_dir = nonempty_path("XDG_DATA_HOME")
            .map(|p| p.join("lumencraft"))
            .or_else(|| home.as_ref().map(|h| h.join(".local").join("share").join("lumencraft")))
            .unwrap_or_else(|| std::env::temp_dir().join("lumencraft-data"));
        (config_dir, data_dir)
    }
}

fn load_validated_ttf_font(path: &Path) -> Result<(Vec<u8>, TrueTypeFontInfo), String> {
    let bytes = std::fs::read(path).map_err(|err| format!("failed to read --ttf-font `{}`: {err}", path.display()))?;
    let info = parse_truetype_font_info(&bytes).map_err(|err| format!("invalid --ttf-font `{}`: {err}", path.display()))?;
    Ok((bytes, info))
}

fn load_shortcut_preferences(app: &mut UiApp, paths: &AppPaths) -> Result<usize, String> {
    if paths.preferences_file.is_file() {
        let prefs = load_preferences_file(&paths.preferences_file)
            .map_err(|err| format!("failed to load preferences `{}`: {err}", paths.preferences_file.display()))?;
        if prefs.shortcuts.is_empty() {
            return Ok(0);
        }
        let text = serde_json::to_string(&prefs.shortcuts).map_err(|err| err.to_string())?;
        return app.shortcuts.import_overrides_json(&text);
    }
    if paths.shortcuts_file.is_file() {
        let raw = std::fs::read_to_string(&paths.shortcuts_file)
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
    let raw_shortcuts = app.shortcuts.export_overrides_json()?;
    let shortcuts: BTreeMap<String, String> = serde_json::from_str(&raw_shortcuts).map_err(|err| err.to_string())?;
    prefs.version = CURRENT_PREFERENCES_VERSION;
    prefs.shortcuts = shortcuts;
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

    use lumencraft_doc::{DataSheet, ProjectDocument, SubcolumnMode, TableArchetype, XyTable};
    use lumencraft_format::save_project_file;

    use super::{CliAction, LaunchConfig, load_startup_session, parse_args, parse_port, run};

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
            String::from("study.lumencraft"),
        ];
        assert_eq!(
            parse_args(&args, None).ok(),
            Some(CliAction::Launch(LaunchConfig {
                control_port: Some(9123),
                gui_only: false,
                in_window_menus: true,
                new_instance: true,
                headless_once: false,
                portable_dir: None,
                ttf_font: None,
                rulers: false,
                event_script: None,
                files: vec![PathBuf::from("study.lumencraft")],
            }))
        );

        let gui_only_args = vec![String::from("--control"), String::from("9124"), String::from("--gui-only"), String::from("--headless-once")];
        assert_eq!(
            parse_args(&gui_only_args, None).ok(),
            Some(CliAction::Launch(LaunchConfig {
                control_port: Some(9124),
                gui_only: true,
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

        let portable_args = vec![
            String::from("--portable-dir"),
            String::from("/tmp/lc-usb"),
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
                gui_only: false,
                in_window_menus: false,
                new_instance: false,
                headless_once: false,
                portable_dir: Some(PathBuf::from("/tmp/lc-usb")),
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
        let dir = std::env::temp_dir().join(format!("lc-app-paths-{}-{unique}", std::process::id()));
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
        });
        assert!(!std_paths.portable);
        assert!(!std_paths.config_dir.ends_with("lumencraft-data"));

        // Explicit --portable-dir routes config, shortcuts, preferences, and recovery into ./lumencraft-data/
        let explicit_dir = dir.join("usb-stick");
        let explicit_paths = super::resolve_app_paths(Some(&exe_dir), Some(&explicit_dir));
        assert!(explicit_paths.portable);
        let expected_explicit_root = explicit_dir.join("lumencraft-data");
        assert_eq!(explicit_paths.config_dir, expected_explicit_root);
        assert_eq!(explicit_paths.data_dir, expected_explicit_root);
        assert_eq!(explicit_paths.preferences_file, expected_explicit_root.join("preferences.json"));
        assert_eq!(explicit_paths.shortcuts_file, expected_explicit_root.join("shortcuts.json"));
        assert_eq!(explicit_paths.recovery_dir, expected_explicit_root.join("recovery"));

        // portable.txt next to executable activates ./lumencraft-data/
        std::fs::write(exe_dir.join("portable.txt"), "LumenCraft portable mode\n")?;
        let marker_paths = super::resolve_app_paths(Some(&exe_dir), None);
        assert!(marker_paths.portable);
        assert_eq!(marker_paths.config_dir, exe_dir.join("lumencraft-data"));
        assert_eq!(marker_paths.recovery_dir, exe_dir.join("lumencraft-data").join("recovery"));

        // Shortcut preferences save & reload round-trip in portable mode (versioned preferences.json)
        let mut app = lumencraft_ui_egui::UiApp::new();
        app.shortcuts.set_shortcut("ui.gallery.toggle", "Ctrl+Shift+L")?;
        super::save_shortcut_preferences(&app, &marker_paths)?;
        assert!(marker_paths.preferences_file.is_file());
        let saved_prefs_raw = std::fs::read_to_string(&marker_paths.preferences_file)?;
        let saved_prefs_val: serde_json::Value = serde_json::from_str(&saved_prefs_raw)?;
        assert_eq!(saved_prefs_val["version"], serde_json::json!(lumencraft_format::versioned::CURRENT_PREFERENCES_VERSION));
        let mut reloaded = lumencraft_ui_egui::UiApp::new();
        let imported = super::load_shortcut_preferences(&mut reloaded, &marker_paths)?;
        assert_eq!(imported, 1);
        assert_eq!(reloaded.shortcuts.active_combo("ui.gallery.toggle"), Some("Ctrl+Shift+L"));

        // Valid & invalid --ttf-font loading
        let valid_ttf = dir.join("minimal.ttf");
        std::fs::write(&valid_ttf, lumencraft_text::build_minimal_truetype_font())?;
        let (_, font_info) = super::load_validated_ttf_font(&valid_ttf)?;
        assert!(!font_info.family_name.is_empty());

        let invalid_ttf = dir.join("bad.ttf");
        std::fs::write(&invalid_ttf, b"not-a-ttf")?;
        assert!(super::load_validated_ttf_font(&invalid_ttf).is_err());

        // Portable-dir recovery journal startup -> simulated crash -> WAL recovery -> clean shutdown
        let recovery_root = dir.join("portable-recovery");
        let rec_paths = super::resolve_app_paths(None, Some(&recovery_root));
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

        let mut app2 = lumencraft_ui_egui::UiApp::from_session(s2.session);
        super::close_startup_recovery_cleanly(&mut app2, s2.journal)?;
        assert!(!app2.session().is_dirty());

        let s3 = super::open_startup_session_with_recovery(&[], &rec_paths, true, 2_000 + super::RECOVERY_LOCK_TTL_MS)?;
        let report3 = s3.report.ok_or("reopen after clean close should return report")?;
        assert!(report3.recovered_from_stale_lock.is_none());
        assert!(report3.recovered_commands.is_empty());
        assert!(s3.session.document().data_sheets.iter().any(|s| s.id.as_str() == "rec-1"));
        let mut app3 = lumencraft_ui_egui::UiApp::from_session(s3.session);
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
    fn startup_loads_project_and_csv_files() -> Result<(), Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("lc-main-startup-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;

        let project_path = dir.join("sample.lumencraft");
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
        let dir = std::env::temp_dir().join(format!("lc-main-unpacked-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)?;

        let unpacked_dot_d = dir.join("study.lumencraft.d");
        let mut doc = ProjectDocument::new("Unpacked Startup")?;
        let xy = XyTable::new(SubcolumnMode::Single, 1, 2)?;
        doc.add_data_sheet(DataSheet::new("data-1", "Dose", TableArchetype::Xy(xy))?)?;
        lumencraft_format::save_project_unpacked_dir(&doc, &unpacked_dot_d)?;

        let session = load_startup_session(std::slice::from_ref(&unpacked_dot_d))?;
        assert_eq!(session.document().title, "Unpacked Startup");
        assert_eq!(session.file_path(), Some(unpacked_dot_d.as_path()));
        assert!(!session.is_dirty());

        let mut mut_session = session;
        // 1. Auto-detect `.lumencraft.d` suffix when saving without explicit `unpacked: true`.
        let saved_dot_d = dir.join("saved.lumencraft.d");
        let save_resp =
            mut_session.execute("project.save", serde_json::json!({ "path": saved_dot_d.display().to_string(), "return_bytes": false }))?;
        assert_eq!(save_resp.payload.get("unpacked").and_then(|v| v.as_bool()), Some(true));
        assert!(lumencraft_format::is_unpacked_project_dir(&saved_dot_d));

        let reopened_dot_d = load_startup_session(std::slice::from_ref(&saved_dot_d))?;
        assert_eq!(reopened_dot_d.document().title, "Unpacked Startup");

        // 2. Also auto-detect an unpacked directory without `.lumencraft.d` suffix via `is_unpacked_project_dir`.
        let plain_unpacked = dir.join("plain_project_dir");
        let save_plain_resp = mut_session
            .execute("project.save", serde_json::json!({ "path": plain_unpacked.display().to_string(), "unpacked": true, "return_bytes": false }))?;
        assert_eq!(save_plain_resp.payload.get("unpacked").and_then(|v| v.as_bool()), Some(true));
        assert!(lumencraft_format::is_unpacked_project_dir(&plain_unpacked));

        let reopened = load_startup_session(std::slice::from_ref(&plain_unpacked))?;
        assert_eq!(reopened.document().title, "Unpacked Startup");
        assert_eq!(reopened.document().data_sheets.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn preferences_preserve_forward_fields_and_refuse_newer_version_overwrite() -> Result<(), Box<dyn std::error::Error>> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("lc-main-prefs-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let paths = super::resolve_app_paths(None, Some(&dir));
        std::fs::create_dir_all(&paths.config_dir)?;

        // 1. Legacy v0 preferences file without explicit `version` upgrades and preserves metadata.
        let legacy_raw = serde_json::json!({
            "locale": "en-US",
            "theme": "light",
            "shortcuts": { "ui.gallery.toggle": "Ctrl+Alt+G" },
            "future_flag": true
        });
        std::fs::write(&paths.preferences_file, serde_json::to_string_pretty(&legacy_raw)?)?;
        let mut app = lumencraft_ui_egui::UiApp::new();
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

        let mut future_app = lumencraft_ui_egui::UiApp::new();
        assert!(super::load_shortcut_preferences(&mut future_app, &paths).is_err());
        future_app.shortcuts.set_shortcut("ui.gallery.toggle", "Ctrl+Shift+X")?;
        assert!(super::save_shortcut_preferences(&future_app, &paths).is_err());
        assert_eq!(std::fs::read_to_string(&paths.preferences_file)?, future_text, "newer-version preferences file must remain untouched");

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
