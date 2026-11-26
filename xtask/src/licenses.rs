//! License policy parity (`deny.toml` <-> `about.toml`) and thin wrapper over `cargo deny` /
//! `cargo about` (`docs/development.md` §Third-party licensing).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::scan_util::strip_toml_comment;

const USAGE: &str = "usage: cargo xtask licenses [--check | --html [path]]";
const CARGO_LICENSE_WALL_TIMEOUT: Duration = Duration::from_secs(900);
const CARGO_LICENSE_IDLE_TIMEOUT: Duration = Duration::from_secs(900);

fn parse_args(root: &Path, args: &[String]) -> Result<Option<Option<PathBuf>>, String> {
    let mut html_out: Option<PathBuf> = None;
    let mut saw_check = false;
    let mut idx = 0usize;
    while let Some(arg) = args.get(idx).map(String::as_str) {
        match arg {
            "--check" => {
                saw_check = true;
                idx += 1;
            }
            "--html" => {
                if html_out.is_some() {
                    return Err(format!("duplicate `--html` flag\n{USAGE}"));
                }
                let target = match args.get(idx + 1).map(String::as_str) {
                    None => {
                        idx += 1;
                        root.join("licenses/THIRD-PARTY-LICENSES.html")
                    }
                    Some(next) if next.starts_with('-') => {
                        return Err(format!("unexpected flag `{next}` after `--html` (expected output path)\n{USAGE}"));
                    }
                    Some(next) if next.trim().is_empty() => {
                        return Err(format!("empty output path after `--html`\n{USAGE}"));
                    }
                    Some(next) => {
                        idx += 2;
                        PathBuf::from(next)
                    }
                };
                html_out = Some(target);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            other => return Err(format!("unknown `cargo xtask licenses` flag `{other}`\n{USAGE}")),
        }
    }
    if saw_check && html_out.is_some() {
        return Err(format!("cannot combine `--check` and `--html`\n{USAGE}"));
    }
    Ok(Some(html_out))
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let Some(html_out) = parse_args(root, args)? else {
        return Ok(());
    };

    let allow = check_allowlist_parity(root)?;
    println!("licenses: deny.toml <-> about.toml allowlist parity verified ({} SPDX identifiers)", allow.len());

    if let Some(out_path) = html_out {
        return run_cargo_about_html(root, &out_path);
    }

    let out = crate::dev_tool_command("cargo")
        .current_dir(root)
        .args(["deny", "check", "licenses"])
        .wall_timeout(CARGO_LICENSE_WALL_TIMEOUT)
        .idle_timeout(CARGO_LICENSE_IDLE_TIMEOUT)
        .run();
    match out {
        Ok(res) if res.success => {
            if !res.stdout.is_empty() {
                print!("{}", String::from_utf8_lossy(&res.stdout));
            }
            println!("licenses: `cargo deny check licenses` passed");
            Ok(())
        }
        Ok(res) => {
            let stderr = String::from_utf8_lossy(&res.stderr);
            if !crate::workflows::is_ci_env() && stderr.contains("no such command: `deny`") {
                eprintln!("licenses: warning: `cargo-deny` not installed locally; skipping external check");
                return Ok(());
            }
            Err(format!("`cargo deny check licenses` failed ({:?}):\n{}", res.status_code, stderr.trim()))
        }
        Err(err) => Err(format!("failed running `cargo deny check licenses`: {err}")),
    }
}

fn run_cargo_about_html(root: &Path, out_path: &Path) -> Result<(), String> {
    let abs_out = if out_path.is_absolute() { out_path.to_path_buf() } else { root.join(out_path) };
    if let Some(parent) = abs_out.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create_dir_all {}: {e}", parent.display()))?;
    }
    let out_str = abs_out.to_str().ok_or_else(|| format!("non-UTF-8 license output path {}", abs_out.display()))?;
    let res = crate::dev_tool_command("cargo")
        .current_dir(root)
        .args(["about", "generate", "about.hbs", "-o", out_str])
        .wall_timeout(CARGO_LICENSE_WALL_TIMEOUT)
        .idle_timeout(CARGO_LICENSE_IDLE_TIMEOUT)
        .run()
        .map_err(|e| format!("failed running `cargo about generate`: {e}"))?;
    if !res.success {
        return Err(format!("`cargo about generate` failed ({:?}):\n{}", res.status_code, String::from_utf8_lossy(&res.stderr).trim()));
    }
    println!("licenses: wrote {}", abs_out.display());
    Ok(())
}

fn check_allowlist_parity(root: &Path) -> Result<BTreeSet<String>, String> {
    let deny_raw = fs::read_to_string(root.join("deny.toml")).map_err(|e| format!("read deny.toml: {e}"))?;
    let about_raw = fs::read_to_string(root.join("about.toml")).map_err(|e| format!("read about.toml: {e}"))?;
    let deny_allow = parse_toml_string_array(&deny_raw, Some("licenses"), "allow")?;
    let about_accepted = parse_toml_string_array(&about_raw, None, "accepted")?;
    if deny_allow.is_empty() || deny_allow != about_accepted {
        let only_deny: Vec<_> = deny_allow.difference(&about_accepted).cloned().collect();
        let only_about: Vec<_> = about_accepted.difference(&deny_allow).cloned().collect();
        return Err(format!("deny.toml allow <-> about.toml accepted mismatch: only_deny={only_deny:?}, only_about={only_about:?}"));
    }
    Ok(deny_allow)
}

fn parse_toml_string_array(raw: &str, expected_section: Option<&str>, key: &str) -> Result<BTreeSet<String>, String> {
    let mut current_section: Option<String> = None;
    let mut in_array = false;
    let mut found = false;
    let mut items = BTreeSet::new();

    for line in raw.lines() {
        let clean = strip_toml_comment(line).trim();
        if clean.is_empty() {
            continue;
        }
        if !in_array {
            if let Some(sec) = parse_toml_section_header(clean) {
                current_section = Some(sec);
                continue;
            }
            if current_section.as_deref() != expected_section {
                continue;
            }
            if let Some((lhs, rhs)) = clean.split_once('=')
                && lhs.trim() == key
            {
                if found {
                    return Err(format!("duplicate `{key}` key in TOML section {expected_section:?}"));
                }
                let rhs = rhs.trim();
                let Some(after_open) = rhs.strip_prefix('[') else {
                    return Err(format!("expected `[` opening array for `{key}`"));
                };
                in_array = true;
                if collect_toml_array_slice(after_open, &mut items)? {
                    in_array = false;
                    found = true;
                }
            }
        } else if collect_toml_array_slice(clean, &mut items)? {
            in_array = false;
            found = true;
        }
    }
    if in_array {
        return Err(format!("unclosed `{key} = [...]` TOML array"));
    }
    if !found {
        return Err(format!("missing `{key} = [...]` TOML array in section {expected_section:?}"));
    }
    Ok(items)
}

fn parse_toml_section_header(clean: &str) -> Option<String> {
    if clean.starts_with("[[") && clean.ends_with("]]") {
        let inner = clean.get(2..clean.len().saturating_sub(2))?.trim();
        return Some(format!("[[{inner}]]"));
    }
    if clean.starts_with('[') && clean.ends_with(']') {
        let inner = clean.get(1..clean.len().saturating_sub(1))?.trim();
        return Some(inner.to_owned());
    }
    None
}

/// Parse single- and double-quoted TOML string elements up to an unquoted `]`.
/// Returns `Ok(true)` if the closing `]` was encountered on this line.
fn collect_toml_array_slice(slice: &str, out: &mut BTreeSet<String>) -> Result<bool, String> {
    let bytes = slice.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() || b == b',' {
            i += 1;
        } else if b == b']' {
            return Ok(true);
        } else if b == b'"' {
            let (val, next_i) = read_double_quoted(slice, i)?;
            out.insert(val);
            i = next_i;
        } else if b == b'\'' {
            let (val, next_i) = read_single_quoted(slice, i)?;
            out.insert(val);
            i = next_i;
        } else {
            return Err(format!("unexpected token in TOML string array near `{}`", slice.get(i..).unwrap_or("")));
        }
    }
    Ok(false)
}

fn read_double_quoted(slice: &str, start: usize) -> Result<(String, usize), String> {
    let mut out = String::new();
    let mut escaped = false;
    for (rel, ch) in slice.get(start + 1..).unwrap_or("").char_indices() {
        if escaped {
            match ch {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => out.push(other),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Ok((out, start + 1 + rel + 1));
        } else {
            out.push(ch);
        }
    }
    Err(String::from("unterminated double-quoted TOML string in license array"))
}

fn read_single_quoted(slice: &str, start: usize) -> Result<(String, usize), String> {
    let tail = slice.get(start + 1..).unwrap_or("");
    let Some(rel) = tail.find('\'') else {
        return Err(String::from("unterminated single-quoted TOML string in license array"));
    };
    let val = tail.get(..rel).unwrap_or("").to_owned();
    Ok((val, start + 1 + rel + 1))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{check_allowlist_parity, parse_args, parse_toml_string_array};

    #[test]
    fn deny_and_about_allowlists_match_on_workspace() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
        let allow = check_allowlist_parity(root).expect("allowlist parity");
        assert!(allow.contains("MIT") && allow.contains("Apache-2.0"), "{allow:?}");
    }

    #[test]
    fn aud_12_35_tracks_toml_sections_single_quoted_strings_and_rejects_flag_after_html() {
        let deny = "\
[bans]
allow = [\"decoy-crate\"]

[licenses]
allow = [
    \"MIT\", # inline comment
    'Apache-2.0',
    'GPL-3.0-only', # literal single-quoted SPDX id with # inside comment
]
";
        let parsed = parse_toml_string_array(deny, Some("licenses"), "allow").unwrap();
        assert!(parsed.contains("MIT") && parsed.contains("Apache-2.0") && parsed.contains("GPL-3.0-only"));
        assert!(!parsed.contains("decoy-crate"), "must not read `[bans].allow`");

        let about = "accepted = [\n  \"MIT\",\n  'Apache-2.0',\n]\n[other]\naccepted = [\"Decoy\"]\n";
        let about_parsed = parse_toml_string_array(about, None, "accepted").unwrap();
        assert_eq!(about_parsed.len(), 2);
        assert!(!about_parsed.contains("Decoy"));

        let root = Path::new(".");
        assert!(parse_args(root, &[String::from("--html"), String::from("--check")]).is_err());
        assert!(parse_args(root, &[String::from("--check"), String::from("--html")]).is_err());
        assert!(parse_args(root, &[String::from("--html"), String::from("out.html")]).unwrap().flatten().is_some());
    }
}
