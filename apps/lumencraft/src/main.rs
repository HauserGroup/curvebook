//! Native desktop entry point for LumenCraft.
#![forbid(unsafe_code)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lumencraft_doc::{ProjectDocument, SheetId, SubcolumnMode, TableKind};
use lumencraft_engine::Session;
use lumencraft_format::{CsvOptions, import_csv_to_datasheet, import_prism_bytes, import_pzfx_bytes, instantiate_template_bytes, open_project_file};
use lumencraft_render::RenderTargetSpec;
use lumencraft_ui_egui::dialogs_canvas::render_workspace_frame_png;
use lumencraft_ui_egui::theme::install_fonts;
use lumencraft_ui_egui::{UiApp, parse_control_port, run_control_tcp_server};

const USAGE: &str = "\
usage: lumencraft [options] [file...]

options:
  --control <port>    listen on 127.0.0.1:<port> for JSON-lines automation
  --in-window-menus   render the menu bar inside the window on macOS
  --new-instance      open files in a separate application window
  --headless-once     render one software compositor frame and exit
  -V, --version       print version and exit
  -h, --help          print this help message and exit
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
    in_window_menus: bool,
    new_instance: bool,
    headless_once: bool,
    files: Vec<PathBuf>,
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

fn launch_application(cfg: &LaunchConfig) -> Result<(), String> {
    let _loaded_fonts = install_fonts();
    let _ = lumencraft_ui_egui::bundled_fonts_enabled();
    let _ = lumencraft_format::EXTENSION;

    let session = load_startup_session(&cfg.files)?;
    let mut app = UiApp::from_session(session);
    app.sync_views();

    if cfg.headless_once {
        let png = render_workspace_frame_png(&app, RenderTargetSpec { width_px: 800, height_px: 520, dpi: 144 })
            .map_err(|err| format!("headless frame render failed: {err}"))?;
        if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(String::from("headless compositor produced invalid PNG signature"));
        }
    }

    if let Some(port) = cfg.control_port {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        run_control_tcp_server(&mut app, addr)?;
    }

    Ok(())
}

fn load_startup_session(files: &[PathBuf]) -> Result<Session, String> {
    if files.is_empty() {
        return Ok(Session::new());
    }

    let mut primary_doc: Option<ProjectDocument> = None;
    let mut primary_archive_path: Option<PathBuf> = None;
    let mut clean_archive = false;

    for (idx, path) in files.iter().enumerate() {
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        match ext.as_str() {
            "lumencraft" => {
                let doc = open_project_file(path).map_err(|err| format!("failed to open `{}`: {err}", path.display()))?;
                primary_doc = Some(doc);
                primary_archive_path = Some(path.clone());
                clean_archive = true;
            }
            "lctemplate" => {
                let bytes = std::fs::read(path).map_err(|err| format!("failed to read `{}`: {err}", path.display()))?;
                let doc =
                    instantiate_template_bytes(&bytes, None).map_err(|err| format!("failed to instantiate template `{}`: {err}", path.display()))?;
                primary_doc = Some(doc);
                primary_archive_path = None;
                clean_archive = false;
            }
            "prism" => {
                let bytes = std::fs::read(path).map_err(|err| format!("failed to read `{}`: {err}", path.display()))?;
                let report = import_prism_bytes(&bytes).map_err(|err| format!("failed to import `{}`: {err}", path.display()))?;
                primary_doc = Some(report.document);
                primary_archive_path = None;
                clean_archive = false;
            }
            "pzfx" => {
                let bytes = std::fs::read(path).map_err(|err| format!("failed to read `{}`: {err}", path.display()))?;
                let report = import_pzfx_bytes(&bytes).map_err(|err| format!("failed to import `{}`: {err}", path.display()))?;
                primary_doc = Some(report.document);
                primary_archive_path = None;
                clean_archive = false;
            }
            "csv" | "tsv" => {
                let text = std::fs::read_to_string(path).map_err(|err| format!("failed to read `{}`: {err}", path.display()))?;
                let opts = if ext == "tsv" { CsvOptions::tsv() } else { CsvOptions::detect(&text) };
                let doc = primary_doc.get_or_insert_with(ProjectDocument::default);
                let sheet_id = next_csv_sheet_id(doc, idx);
                let title = file_stem_title(path);
                let sheet = import_csv_to_datasheet(sheet_id.clone(), title, &text, TableKind::Column, SubcolumnMode::Single, &opts)
                    .map_err(|err| format!("failed to import `{}`: {err}", path.display()))?;
                doc.add_data_sheet(sheet).map_err(|err| err.to_string())?;
                if doc.active_sheet_id.is_none() {
                    let _ = doc.select_in_navigator(&sheet_id);
                }
            }
            other => {
                return Err(format!(
                    "unsupported file extension `.{other}` for `{}`; expected .lumencraft, .lctemplate, .prism, .pzfx, .csv, or .tsv",
                    path.display()
                ));
            }
        }
    }

    let doc = primary_doc.unwrap_or_default();
    let mut session = Session::from_document(doc);
    if let Some(path) = primary_archive_path {
        session.set_file_path(Some(path), clean_archive);
    }
    Ok(session)
}

fn next_csv_sheet_id(doc: &ProjectDocument, hint_idx: usize) -> SheetId {
    let mut seq = hint_idx.saturating_add(1);
    loop {
        let candidate = SheetId::new(format!("data-{seq}"));
        if doc.sheet_section(&candidate).is_none() {
            return candidate;
        }
        seq = seq.saturating_add(1);
    }
}

fn file_stem_title(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| String::from("Imported Table"), ToOwned::to_owned)
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
                in_window_menus: true,
                new_instance: true,
                headless_once: false,
                files: vec![PathBuf::from("study.lumencraft")],
            }))
        );
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
}
