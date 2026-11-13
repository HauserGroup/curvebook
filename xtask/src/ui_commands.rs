//! `cargo xtask ui-commands [--check]`: generate `crates/mcp/src/ui_catalog.rs` from
//! `crates/ui-egui/src/menus/ui_commands.rs` so headless MCP command discovery stays in lockstep
//! with the desktop shell without introducing an L6 -> L6 dependency.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const SRC_REL: &str = "crates/ui-egui/src/menus/ui_commands.rs";
const DST_REL: &str = "crates/mcp/src/ui_catalog.rs";

const HEADER: &str = "\
//! Desktop UI command catalogue for headless MCP discovery (`ui: true`).
//! Generated from `crates/ui-egui/src/menus/ui_commands.rs` by `cargo xtask ui-commands`.
//! Do not edit by hand; re-run the xtask when that table changes.

/// `(id, label, shortcut, params doc)` — same shape as the desktop `UI_COMMANDS` table.
";

pub fn run(root: &Path, check: bool) -> Result<(), String> {
    let src_path = root.join(SRC_REL);
    let dst_path = root.join(DST_REL);
    let src = std::fs::read_to_string(&src_path).map_err(|e| format!("{SRC_REL}: {e}"))?;
    let raw = render_catalog(&src)?;
    let formatted = rustfmt(root, &raw)?;

    if check {
        let existing = std::fs::read_to_string(&dst_path).map_err(|e| format!("{DST_REL}: {e}"))?;
        if normalize_newlines(&existing) != normalize_newlines(&formatted) {
            return Err(format!("{DST_REL} is out of date with {SRC_REL}; run `cargo xtask ui-commands`"));
        }
        println!("ui-commands: {DST_REL} is in sync with {SRC_REL}");
        return Ok(());
    }

    std::fs::write(&dst_path, formatted).map_err(|e| format!("write {DST_REL}: {e}"))?;
    println!("ui-commands: wrote {DST_REL}");
    Ok(())
}

pub fn render_catalog(src: &str) -> Result<String, String> {
    let start = src.find("pub const UI_COMMANDS:").ok_or_else(|| format!("{SRC_REL}: `pub const UI_COMMANDS:` not found"))?;
    let tail = &src[start..];
    let end_rel = tail.find("];").ok_or_else(|| format!("{SRC_REL}: closing `];` for `UI_COMMANDS` not found"))?;
    let decl = &tail[..end_rel + 2];
    Ok(format!("{HEADER}{decl}\n"))
}

fn rustfmt(root: &Path, code: &str) -> Result<String, String> {
    let cfg = root.join("rustfmt.toml");
    let mut cmd = Command::new("rustfmt");
    cmd.arg("--edition").arg("2024");
    if cfg.is_file() {
        cmd.arg("--config-path").arg(&cfg);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("spawn rustfmt: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(code.as_bytes()).map_err(|e| format!("rustfmt stdin: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("wait rustfmt: {e}"))?;
    if !out.status.success() {
        return Err(format!("rustfmt failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("rustfmt utf-8: {e}"))
}

fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::render_catalog;

    #[test]
    fn extracts_ui_commands_slice_declaration() {
        let src = "//! docs\npub const UI_COMMANDS: &[(&str, &str, &str, &str)] = &[\n    (\"app.about\", \"About\", \"\", \"{}\"),\n];\n";
        let rendered = render_catalog(src).unwrap();
        assert!(rendered.contains("Generated from `crates/ui-egui/src/menus/ui_commands.rs`"));
        assert!(rendered.contains("(\"app.about\", \"About\", \"\", \"{}\")"));
    }
}
