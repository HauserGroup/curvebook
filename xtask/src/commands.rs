//! Regenerate or verify `docs/commands.md` via `curvebook-engine`'s `gen_commands` example.

use std::fs;
use std::path::Path;

use curvebook_platform::write_atomic;

const USAGE: &str = "usage: cargo xtask commands [--check]";

pub fn parse_args(args: &[String]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [flag] if flag == "--check" => Ok(true),
        [flag] if flag == "-h" || flag == "--help" => {
            println!("{USAGE}");
            std::process::exit(0);
        }
        [other, ..] => Err(format!("unknown `cargo xtask commands` argument `{other}`\n{USAGE}")),
    }
}

pub fn run_cli(root: &Path, args: &[String]) -> Result<(), String> {
    let check = parse_args(args)?;
    run(root, check)
}

pub fn run(root: &Path, check: bool) -> Result<(), String> {
    let raw = crate::run_workspace_example(root, "curvebook-engine", "gen_commands", &[])?;
    let expected = String::from_utf8(raw).map_err(|e| format!("gen_commands emitted non-UTF-8 output: {e}"))?;
    let doc_path = root.join("docs/commands.md");

    if check {
        let existing = fs::read_to_string(&doc_path).map_err(|e| format!("read {}: {e}", doc_path.display()))?;
        if normalize_newlines(&existing) != normalize_newlines(&expected) {
            return Err(format!("commands: `{}` is out of sync with curvebook_engine::cmd; run `cargo xtask commands`", doc_path.display()));
        }
        let count = expected
            .lines()
            .find_map(|line| line.strip_prefix("Registered commands: ").and_then(|rest| rest.trim().parse::<usize>().ok()))
            .unwrap_or(0);
        println!("commands: docs/commands.md verified ({count} commands)");
        return Ok(());
    }

    write_atomic(&doc_path, expected.as_bytes()).map_err(|e| format!("write {}: {e}", doc_path.display()))?;
    println!("commands: regenerated {}", doc_path.display());
    Ok(())
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::parse_args;

    #[test]
    fn parse_args_accepts_default_or_check() {
        assert_eq!(parse_args(&[]), Ok(false));
        assert_eq!(parse_args(&[String::from("--check")]), Ok(true));
        assert!(parse_args(&[String::from("--bad")]).is_err());
    }

    #[test]
    fn parse_args_rejects_extra_positionals() {
        assert!(parse_args(&[String::from("--check"), String::from("extra")]).is_err());
    }
}
