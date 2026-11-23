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
            curvebook_platform::fs::write_atomic(&manifest, updated.as_bytes()).map_err(|e| format!("write Cargo.toml: {e}"))?;
            if let Err(err) = refresh_lockfile(root) {
                let _ = curvebook_platform::fs::write_atomic(&manifest, text.as_bytes());
                return Err(err);
            }
            println!("version: set workspace version to {ver}");
            Ok(())
        }
        _ => Err(String::from("usage: cargo xtask version [set <X.Y.Z[-pre]>]")),
    }
}

fn is_toml_version_rhs(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("version")?;
    let trimmed_rest = rest.trim_start();
    trimmed_rest.strip_prefix('=')
}

pub fn read_workspace_version(cargo_toml: &str) -> Result<String, String> {
    let mut in_ws_pkg = false;
    for line in cargo_toml.lines() {
        let t = line.trim();
        let no_comment = t.split('#').next().unwrap_or_default().trim();
        if no_comment.starts_with('[') && no_comment.ends_with(']') {
            in_ws_pkg = no_comment == "[workspace.package]";
            continue;
        }
        if in_ws_pkg && let Some(rhs) = is_toml_version_rhs(t) {
            let v = rhs.split('#').next().unwrap_or_default().trim().trim_matches('"');
            if !v.is_empty() {
                return Ok(v.to_owned());
            }
        }
    }
    Err(String::from("Cargo.toml: `[workspace.package] version` not found"))
}

pub fn validate_semver(ver: &str) -> Result<(), String> {
    let parsed = semver::Version::parse(ver).map_err(|err| format!("invalid version `{ver}`: expected X.Y.Z or X.Y.Z-prerelease ({err})"))?;
    if !parsed.build.is_empty() {
        return Err(format!("invalid version `{ver}`: build metadata is not supported for workspace version"));
    }
    Ok(())
}

pub fn replace_workspace_version(cargo_toml: &str, new_ver: &str) -> Result<String, String> {
    let mut out = String::with_capacity(cargo_toml.len() + 16);
    let mut in_ws_pkg = false;
    let mut replaced = false;
    for line in cargo_toml.split_inclusive('\n') {
        let t = line.trim();
        let no_comment = t.split('#').next().unwrap_or_default().trim();
        if no_comment.starts_with('[') && no_comment.ends_with(']') {
            in_ws_pkg = no_comment == "[workspace.package]";
        } else if in_ws_pkg && !replaced && is_toml_version_rhs(t).is_some() {
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
        let toml = "[workspace]\nmembers = []\n\n[workspace.package] # trailing comment\nversion_extra = \"9.9.9\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
        assert_eq!(read_workspace_version(toml).unwrap(), "0.1.0");
        for good in ["0.1.0", "0.2.0-rc.1", "1.0.0-alpha.2", "1.2.3-beta-1.0"] {
            validate_semver(good).unwrap();
        }
        for bad in ["", "1.2", "v0.1.0", "0.1.0+build.1", "0.1.0-", "0.1.0-alpha..1", "01.2.3", "0.1.0-01", " 0.1.0 "] {
            assert!(validate_semver(bad).is_err(), "expected failure for `{bad}`");
        }
        let next = replace_workspace_version(toml, "0.2.0-rc.1").unwrap();
        assert!(next.contains("version_extra = \"9.9.9\""), "unrelated `version_*` key must stay untouched");
        assert_eq!(read_workspace_version(&next).unwrap(), "0.2.0-rc.1");
    }
}
