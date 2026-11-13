//! `cargo xtask version [set <X.Y.Z[-pre]>]`: inspect or atomically update `[workspace.package]
//! version` in the root `Cargo.toml` and refresh `Cargo.lock`.

use std::path::Path;
use std::process::Command;

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let manifest = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("Cargo.toml: {e}"))?;
    match args {
        [] => {
            let current = read_workspace_version(&text)?;
            println!("{current}");
            Ok(())
        }
        [sub, ver] if sub == "set" => {
            validate_semver(ver)?;
            let updated = replace_workspace_version(&text, ver)?;
            let tmp = root.join("Cargo.toml.tmp");
            std::fs::write(&tmp, &updated).map_err(|e| format!("write Cargo.toml.tmp: {e}"))?;
            std::fs::rename(&tmp, &manifest).map_err(|e| format!("rename Cargo.toml.tmp: {e}"))?;
            refresh_lockfile(root)?;
            println!("version: set workspace version to {ver}");
            Ok(())
        }
        _ => Err(String::from("usage: cargo xtask version [set <X.Y.Z[-pre]>]")),
    }
}

pub fn read_workspace_version(cargo_toml: &str) -> Result<String, String> {
    let mut in_ws_pkg = false;
    for line in cargo_toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_ws_pkg = t == "[workspace.package]";
            continue;
        }
        if in_ws_pkg
            && let Some(rest) = t.strip_prefix("version")
            && let Some((_, rhs)) = rest.split_once('=')
        {
            let v = rhs.split('#').next().unwrap_or_default().trim().trim_matches('"');
            if !v.is_empty() {
                return Ok(v.to_owned());
            }
        }
    }
    Err(String::from("Cargo.toml: `[workspace.package] version` not found"))
}

pub fn validate_semver(ver: &str) -> Result<(), String> {
    let (core, pre) = match ver.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (ver, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 || !parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())) {
        return Err(format!("invalid version `{ver}`: expected X.Y.Z or X.Y.Z-prerelease"));
    }
    if let Some(p) = pre
        && (p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
    {
        return Err(format!("invalid prerelease `{p}` in `{ver}`"));
    }
    Ok(())
}

pub fn replace_workspace_version(cargo_toml: &str, new_ver: &str) -> Result<String, String> {
    let mut out = String::with_capacity(cargo_toml.len() + 16);
    let mut in_ws_pkg = false;
    let mut replaced = false;
    for line in cargo_toml.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with('[') {
            in_ws_pkg = t == "[workspace.package]";
        } else if in_ws_pkg && !replaced && t.starts_with("version") && t.contains('=') {
            let newline = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            out.push_str(&format!("version = \"{new_ver}\"{newline}"));
            replaced = true;
            continue;
        }
        out.push_str(line);
    }
    if !replaced {
        return Err(String::from("Cargo.toml: failed to locate `[workspace.package] version`"));
    }
    Ok(out)
}

fn refresh_lockfile(root: &Path) -> Result<(), String> {
    let status = Command::new("cargo")
        .current_dir(root)
        .args(["update", "--workspace", "--offline"])
        .status()
        .map_err(|e| format!("cargo update --workspace --offline: {e}"))?;
    if !status.success() {
        return Err(String::from("cargo update --workspace --offline exited non-zero"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{read_workspace_version, replace_workspace_version, validate_semver};

    #[test]
    fn reads_validates_and_replaces_workspace_version() {
        let toml = "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n";
        assert_eq!(read_workspace_version(toml).unwrap(), "0.1.0");
        validate_semver("0.2.0-rc.1").unwrap();
        assert!(validate_semver("1.2").is_err());
        let next = replace_workspace_version(toml, "0.2.0-rc.1").unwrap();
        assert_eq!(read_workspace_version(&next).unwrap(), "0.2.0-rc.1");
    }
}
