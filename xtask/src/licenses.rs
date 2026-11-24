//! License policy parity (`deny.toml` <-> `about.toml`) and thin wrapper over `cargo deny` /
//! `cargo about` (`docs/development.md` §Third-party licensing).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const USAGE: &str = "usage: cargo xtask licenses [--check | --html [path]]";
const CARGO_LICENSE_WALL_TIMEOUT: Duration = Duration::from_secs(900);
const CARGO_LICENSE_IDLE_TIMEOUT: Duration = Duration::from_secs(900);

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let mut html_out: Option<PathBuf> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--check" => {}
            "--html" => {
                let target = iter.next().map_or_else(|| root.join("licenses/THIRD-PARTY-LICENSES.html"), PathBuf::from);
                html_out = Some(target);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unknown `cargo xtask licenses` flag `{other}`\n{USAGE}")),
        }
    }

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
    let deny_allow = parse_toml_string_array(&deny_raw, "allow")?;
    let about_accepted = parse_toml_string_array(&about_raw, "accepted")?;
    if deny_allow.is_empty() || deny_allow != about_accepted {
        let only_deny: Vec<_> = deny_allow.difference(&about_accepted).cloned().collect();
        let only_about: Vec<_> = about_accepted.difference(&deny_allow).cloned().collect();
        return Err(format!("deny.toml allow <-> about.toml accepted mismatch: only_deny={only_deny:?}, only_about={only_about:?}"));
    }
    Ok(deny_allow)
}

fn parse_toml_string_array(raw: &str, key: &str) -> Result<BTreeSet<String>, String> {
    let mut in_array = false;
    let mut items = BTreeSet::new();
    for line in raw.lines() {
        let clean = line.split('#').next().unwrap_or("").trim();
        if clean.is_empty() {
            continue;
        }
        if !in_array {
            if let Some((lhs, rhs)) = clean.split_once('=')
                && lhs.trim() == key
            {
                in_array = true;
                collect_quoted(rhs, &mut items);
                if rhs.contains(']') {
                    return Ok(items);
                }
            }
        } else {
            collect_quoted(clean, &mut items);
            if clean.contains(']') {
                return Ok(items);
            }
        }
    }
    Err(format!("missing `{key} = [...]` TOML array"))
}

fn collect_quoted(slice: &str, out: &mut BTreeSet<String>) {
    let mut rest = slice;
    while let Some((_, after_open)) = rest.split_once('"') {
        if let Some((item, after_close)) = after_open.split_once('"') {
            out.insert(item.to_owned());
            rest = after_close;
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::check_allowlist_parity;

    #[test]
    fn deny_and_about_allowlists_match_on_workspace() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root");
        let allow = check_allowlist_parity(root).expect("allowlist parity");
        assert!(allow.contains("MIT") && allow.contains("Apache-2.0"), "{allow:?}");
    }
}
