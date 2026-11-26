//! License policy parity (`deny.toml` <-> `about.toml`) and thin wrapper over `cargo deny` /
//! `cargo about` (`docs/development.md` §Third-party licensing).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::scan_util::{TomlEntry, TomlVal, parse_toml_entries};

const USAGE: &str = "usage: cargo xtask licenses [--check | --html [path]]";
const CARGO_LICENSE_TIMEOUT: Duration = Duration::from_secs(900);

fn parse_args(root: &Path, args: &[String]) -> Result<Option<Option<PathBuf>>, String> {
    let (mut html_out, mut saw_check, mut idx) = (None, false, 0usize);
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
                html_out = Some(match args.get(idx + 1).map(String::as_str) {
                    None => {
                        idx += 1;
                        root.join("licenses/THIRD-PARTY-LICENSES.html")
                    }
                    Some(next) if next.starts_with('-') || next.trim().is_empty() => {
                        return Err(format!("invalid output path `{next}` after `--html`\n{USAGE}"));
                    }
                    Some(next) => {
                        idx += 2;
                        PathBuf::from(next)
                    }
                });
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
    let Some(html_out) = parse_args(root, args)? else { return Ok(()) };
    let allow = check_allowlist_parity(root)?;
    println!("licenses: deny.toml <-> about.toml allowlist parity verified ({} SPDX identifiers)", allow.len());
    if let Some(out_path) = html_out {
        return run_cargo_about_html(root, &out_path);
    }
    let out = crate::dev_tool_command("cargo")
        .current_dir(root)
        .args(["deny", "check", "licenses"])
        .wall_timeout(CARGO_LICENSE_TIMEOUT)
        .idle_timeout(CARGO_LICENSE_TIMEOUT)
        .run()
        .map_err(|e| format!("failed running `cargo deny check licenses`: {e}"))?;
    if out.success {
        if !out.stdout.is_empty() {
            print!("{}", String::from_utf8_lossy(&out.stdout));
        }
        println!("licenses: `cargo deny check licenses` passed");
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !crate::workflows::is_ci_env() && stderr.contains("no such command: `deny`") {
        eprintln!("licenses: warning: `cargo-deny` not installed locally; skipping external check");
        return Ok(());
    }
    Err(format!("`cargo deny check licenses` failed ({:?}):\n{}", out.status_code, stderr.trim()))
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
        .wall_timeout(CARGO_LICENSE_TIMEOUT)
        .idle_timeout(CARGO_LICENSE_TIMEOUT)
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
    let target: Vec<&str> = expected_section.into_iter().chain(std::iter::once(key)).collect();
    let mut found = None;
    for entry in parse_toml_entries(raw) {
        if let TomlEntry::Leaf { path, value, .. } = entry
            && path.iter().map(String::as_str).collect::<Vec<_>>() == target
        {
            if found.is_some() {
                return Err(format!("duplicate `{key}` key in TOML section {expected_section:?}"));
            }
            let TomlVal::Array(elems) = value else {
                return Err(format!("expected `[` opening array for `{key}`"));
            };
            let mut items = BTreeSet::new();
            for elem in elems {
                let TomlVal::Str(s) = elem else { return Err(format!("unexpected non-string element in `{key}` array")) };
                items.insert(s);
            }
            found = Some(items);
        }
    }
    found.ok_or_else(|| format!("missing `{key} = [...]` TOML array in section {expected_section:?}"))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{check_allowlist_parity, parse_args, parse_toml_string_array};

    #[test]
    fn allowlist_parity_toml_sections_and_cli_flags() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
        let allow = check_allowlist_parity(root).expect("allowlist parity");
        assert!(allow.contains("MIT") && allow.contains("Apache-2.0"), "{allow:?}");

        let deny = "[bans]\nallow = [\"decoy\"]\n[licenses]\nallow = [\n  \"MIT\", # c\n  'Apache-2.0',\n  'GPL-3.0-only',\n]\n";
        let parsed = parse_toml_string_array(deny, Some("licenses"), "allow").unwrap();
        assert!(parsed.contains("MIT") && parsed.contains("Apache-2.0") && parsed.contains("GPL-3.0-only") && !parsed.contains("decoy"));

        let about = "accepted = [\n  \"MIT\",\n  'Apache-2.0',\n]\n[other]\naccepted = [\"Decoy\"]\n";
        let about_parsed = parse_toml_string_array(about, None, "accepted").unwrap();
        assert_eq!(about_parsed.len(), 2);
        assert!(!about_parsed.contains("Decoy"));

        assert!(parse_args(Path::new("."), &[String::from("--html"), String::from("--check")]).is_err());
        assert!(parse_args(Path::new("."), &[String::from("--check"), String::from("--html")]).is_err());
        assert!(parse_args(Path::new("."), &[String::from("--html"), String::from("out.html")]).unwrap().flatten().is_some());
    }
}
