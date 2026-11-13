//! `cargo xtask cursors`: warn about `Cursor` variants in `crates/tools/src/lib.rs` with no
//! `[[cursor]]` entry in `crates/tools/src/cursors.toml` (never fails CI).

use std::path::Path;

const TOOLS_LIB: &str = "crates/tools/src/lib.rs";
const MANIFEST: &str = "crates/tools/src/cursors.toml";

pub fn run(root: &Path) -> Result<(), String> {
    let lib = std::fs::read_to_string(root.join(TOOLS_LIB)).map_err(|e| format!("{TOOLS_LIB}: {e}"))?;
    let toml = std::fs::read_to_string(root.join(MANIFEST)).map_err(|e| format!("{MANIFEST}: {e}"))?;
    let variants = parse_cursor_variants(&lib)?;
    let documented = parse_manifest_names(&toml);
    let missing: Vec<&str> = variants.iter().map(String::as_str).filter(|v| !documented.iter().any(|d| d == v)).collect();
    if missing.is_empty() {
        println!("cursors: all {} Cursor variants documented in {MANIFEST}", variants.len());
    } else {
        eprintln!("warning: {} Cursor variant(s) missing from {MANIFEST}: {}", missing.len(), missing.join(", "));
    }
    Ok(())
}

fn parse_cursor_variants(src: &str) -> Result<Vec<String>, String> {
    let start = src.find("pub enum Cursor").ok_or_else(|| format!("{TOOLS_LIB}: `pub enum Cursor` not found"))?;
    let after = &src[start..];
    let open = after.find('{').ok_or_else(|| format!("{TOOLS_LIB}: unclosed `pub enum Cursor`"))?;
    let body_after = &after[open + 1..];
    let close = body_after.find('}').ok_or_else(|| format!("{TOOLS_LIB}: missing `}}` on `pub enum Cursor`"))?;
    let body = &body_after[..close];
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.split("//").next().unwrap_or_default().trim().trim_end_matches(',');
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let ident = t.split(['(', '{', '=']).next().unwrap_or_default().trim();
        if !ident.is_empty() {
            out.push(ident.to_owned());
        }
    }
    Ok(out)
}

fn parse_manifest_names(toml: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in toml.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("name")
            && let Some((_, val)) = rest.split_once('=')
        {
            let name = val.split('#').next().unwrap_or_default().trim().trim_matches('"');
            if !name.is_empty() {
                out.push(name.to_owned());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{parse_cursor_variants, parse_manifest_names};

    #[test]
    fn extracts_variants_and_manifest_entries() {
        let src = "pub enum Cursor {\n    /// Doc\n    Select,\n    Crosshair,\n}\n";
        let toml = "[[cursor]]\nname = \"Select\"\n";
        assert_eq!(parse_cursor_variants(src).unwrap(), vec!["Select", "Crosshair"]);
        assert_eq!(parse_manifest_names(toml), vec!["Select"]);
    }
}
