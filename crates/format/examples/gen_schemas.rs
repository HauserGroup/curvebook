//! Emit persisted JSON schemas from `curvebook_format::schemas` for `docs/schemas/*.schema.json`.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::env;
use std::path::Path;
use std::process::ExitCode;

use curvebook_format::schemas::all_persisted_schemas;
use curvebook_platform::write_atomic;

fn main() -> ExitCode {
    let mut out_path: Option<String> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--out" {
            let Some(path) = args.next() else {
                eprintln!("usage: gen_schemas [--out <path>]");
                return ExitCode::from(2);
            };
            out_path = Some(path);
        } else {
            eprintln!("unknown argument `{arg}`\nusage: gen_schemas [--out <path>]");
            return ExitCode::from(2);
        }
    }

    let map: BTreeMap<String, serde_json::Value> = all_persisted_schemas().into_iter().map(|(name, value)| (name.to_owned(), value)).collect();
    let json = match serde_json::to_string_pretty(&map) {
        Ok(mut text) => {
            text.push('\n');
            text
        }
        Err(err) => {
            eprintln!("gen_schemas: serialization failed: {err}");
            return ExitCode::from(1);
        }
    };
    if let Some(path) = out_path {
        if let Err(err) = write_atomic(Path::new(&path), json.as_bytes()) {
            eprintln!("gen_schemas: failed writing `{path}`: {err}");
            return ExitCode::from(1);
        }
    } else {
        print!("{json}");
    }
    ExitCode::SUCCESS
}
