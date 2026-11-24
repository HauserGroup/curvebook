//! Emit `docs/commands.md` generated from `curvebook_engine::cmd`.
#![forbid(unsafe_code)]

use std::env;
use std::path::Path;
use std::process::ExitCode;

use curvebook_engine::cmd::render_commands_markdown;
use curvebook_platform::write_atomic;

fn main() -> ExitCode {
    let mut out_path: Option<String> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--out" {
            let Some(path) = args.next() else {
                eprintln!("usage: gen_commands [--out <path>]");
                return ExitCode::from(2);
            };
            out_path = Some(path);
        } else {
            eprintln!("unknown argument `{arg}`\nusage: gen_commands [--out <path>]");
            return ExitCode::from(2);
        }
    }

    let rendered = render_commands_markdown();
    if let Some(path) = out_path {
        if let Err(err) = write_atomic(Path::new(&path), rendered.as_bytes()) {
            eprintln!("gen_commands: failed writing `{path}`: {err}");
            return ExitCode::from(1);
        }
    } else {
        print!("{rendered}");
    }
    ExitCode::SUCCESS
}
