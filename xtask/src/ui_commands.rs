//! `cargo xtask ui-commands [--check]`: generate `crates/mcp/src/ui_catalog.rs` from
//! `crates/ui-egui/src/menus/ui_commands.rs` so headless MCP command discovery stays in lockstep
//! with the desktop shell without introducing an L6 -> L6 dependency.

use std::path::Path;
use std::time::Duration;

use curvebook_platform::write_atomic;

const SRC_REL: &str = "crates/ui-egui/src/menus/ui_commands.rs";
const DST_REL: &str = "crates/mcp/src/ui_catalog.rs";

const HEADER: &str = "\
//! Desktop UI command catalogue for headless MCP discovery (`ui: true`).
//! Generated from `crates/ui-egui/src/menus/ui_commands.rs` by `cargo xtask ui-commands`.
//! Do not edit by hand; re-run the xtask when that table changes.

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

    write_atomic(&dst_path, formatted.as_bytes()).map_err(|e| format!("write {DST_REL}: {e}"))?;
    println!("ui-commands: wrote {DST_REL}");
    Ok(())
}

pub fn render_catalog(src: &str) -> Result<String, String> {
    let body = strip_leading_inner_doc(src);
    if body.trim().is_empty() {
        return Err(format!("{SRC_REL}: source module is empty"));
    }
    let mut out = String::with_capacity(HEADER.len() + body.len() + 1);
    out.push_str(HEADER);
    out.push_str(body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

fn strip_leading_inner_doc(src: &str) -> &str {
    let mut rest = src;
    loop {
        let Some(line_end) = rest.find('\n') else {
            let trimmed = rest.trim();
            return if trimmed.is_empty() || trimmed.starts_with("//!") { "" } else { rest };
        };
        let line = rest[..line_end].trim();
        if line.is_empty() || line.starts_with("//!") {
            rest = &rest[line_end + 1..];
        } else {
            return rest;
        }
    }
}

fn rustfmt(root: &Path, code: &str) -> Result<String, String> {
    let cfg = root.join("rustfmt.toml");
    let mut cmd = crate::dev_tool_command("rustfmt")
        .args(["--edition", "2024"])
        .stdin_bytes(code.as_bytes())
        .wall_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(10));
    if cfg.is_file() {
        let cfg_str = cfg.to_str().ok_or_else(|| format!("non-UTF-8 path {}", cfg.display()))?;
        cmd = cmd.args(["--config-path", cfg_str]);
    }
    let out = cmd.run().map_err(|e| format!("rustfmt: {e}"))?;
    if !out.success {
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
    fn prepends_generated_banner_and_copies_whole_module_body() {
        let src = "//! source docs\n//! second line\n\n/// Spec doc.\n#[derive(Copy, Clone, Debug, PartialEq, Eq)]\npub struct UiCommandSpec {\n    pub id: &'static str,\n    pub label: &'static str,\n    pub shortcut: &'static str,\n    pub params_doc: &'static str,\n    pub summary: &'static str,\n    pub beginner: &'static str,\n}\n\n/// Table doc.\npub const UI_COMMANDS: &[UiCommandSpec] = &[\n    UiCommandSpec { id: \"app.about\", label: \"About\", shortcut: \"\", params_doc: \"[]];\", summary: \"Show about.\", beginner: \"Opens about dialog.\" },\n];\n";
        let rendered = render_catalog(src).unwrap();
        assert!(rendered.starts_with("//! Desktop UI command catalogue for headless MCP discovery"));
        assert!(!rendered.contains("//! source docs"));
        assert!(rendered.contains("/// Spec doc.\n#[derive(Copy, Clone, Debug, PartialEq, Eq)]\npub struct UiCommandSpec"));
        assert!(rendered.contains("pub const UI_COMMANDS: &[UiCommandSpec]"));
        assert!(rendered.contains("params_doc: \"[]];\""));
        assert!(rendered.ends_with("];\n"));
    }

    #[test]
    fn handles_source_without_inner_doc_and_rejects_empty_body() {
        let bare = "pub const UI_COMMANDS: &[()] = &[];";
        let rendered = render_catalog(bare).unwrap();
        assert!(rendered.starts_with("//! Desktop UI command catalogue"));
        assert!(rendered.ends_with("pub const UI_COMMANDS: &[()] = &[];\n"));

        assert!(render_catalog("//! only inner docs\n\n").is_err());
        assert!(render_catalog("   \n\t").is_err());
    }
}
