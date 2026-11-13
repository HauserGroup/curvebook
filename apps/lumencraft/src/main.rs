//! Native desktop entry point for LumenCraft.
#![forbid(unsafe_code)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;

const USAGE: &str = "\
usage: lumencraft [options] [file...]

options:
  --control <port>    listen on 127.0.0.1:<port> for JSON-lines automation
  --in-window-menus   render the menu bar inside the window on macOS
  --new-instance      open files in a separate application window
  -V, --version       print version and exit
  -h, --help          print this help message and exit
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lumencraft: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let mut i = 0usize;
    while let Some(arg) = args.get(i) {
        match arg.as_str() {
            "-V" | "--version" => {
                println!("LumenCraft {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "--control" => {
                let raw = args.get(i + 1).ok_or_else(|| String::from("--control requires a TCP port (1..=65535)"))?;
                let _port = parse_port(raw)?;
                i += 2;
            }
            "--in-window-menus" | "--new-instance" => {
                i += 1;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("unknown option `{flag}`\n\n{USAGE}"));
            }
            _path => {
                i += 1;
            }
        }
    }
    let _ = lumencraft_ui_egui::bundled_fonts_enabled();
    let _ = lumencraft_format::EXTENSION;
    Ok(())
}

fn parse_port(raw: &str) -> Result<u16, String> {
    let port: u16 = raw.parse().map_err(|_| format!("invalid control port `{raw}`: expected 1..=65535"))?;
    if port == 0 {
        return Err(String::from("invalid control port `0`: expected 1..=65535"));
    }
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::{parse_port, run};

    #[test]
    fn parses_valid_control_ports_and_rejects_zero() {
        assert_eq!(parse_port("7979").unwrap(), 7979);
        assert!(parse_port("0").is_err());
        assert!(parse_port("abc").is_err());
    }

    #[test]
    fn version_and_help_flags_succeed() {
        assert!(run(&[String::from("--version")]).is_ok());
        assert!(run(&[String::from("--help")]).is_ok());
    }
}
