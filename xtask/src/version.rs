//! `cargo xtask version [set <X.Y.Z[-pre]>]`: inspect or atomically update `[workspace.package]
//! version` in the root `Cargo.toml`, synchronize `CITATION.cff`, and refresh `Cargo.lock`.

use std::path::Path;
use std::process::Command;

const CITATION_CFF_REL: &str = "CITATION.cff";

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
            let cff_path = root.join(CITATION_CFF_REL);
            let cff_backup = if cff_path.is_file() {
                let raw = std::fs::read_to_string(&cff_path).map_err(|e| format!("{CITATION_CFF_REL}: {e}"))?;
                let next = replace_citation_cff_version(&raw, ver)?;
                Some((raw, next))
            } else {
                None
            };
            curvebook_platform::fs::write_atomic(&manifest, updated.as_bytes()).map_err(|e| format!("write Cargo.toml: {e}"))?;
            if let Some((_, ref next_cff)) = cff_backup
                && let Err(err) = curvebook_platform::fs::write_atomic(&cff_path, next_cff.as_bytes())
            {
                let _ = curvebook_platform::fs::write_atomic(&manifest, text.as_bytes());
                return Err(format!("write {CITATION_CFF_REL}: {err}"));
            }
            if let Err(err) = refresh_lockfile(root) {
                let _ = curvebook_platform::fs::write_atomic(&manifest, text.as_bytes());
                if let Some((ref orig_cff, _)) = cff_backup {
                    let _ = curvebook_platform::fs::write_atomic(&cff_path, orig_cff.as_bytes());
                }
                return Err(err);
            }
            println!("version: set workspace version to {ver}");
            Ok(())
        }
        _ => Err(String::from("usage: cargo xtask version [set <X.Y.Z[-pre]>]")),
    }
}

/// Verify root `CITATION.cff` exists, declares required CFF 1.2.0 metadata, and keeps its
/// `version` in lockstep with `[workspace.package] version` in `Cargo.toml` (`R14.7`).
pub fn check_citation_cff(root: &Path) -> Result<(), String> {
    let cargo_toml = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?;
    let ws_ver = read_workspace_version(&cargo_toml)?;
    let cff_path = root.join(CITATION_CFF_REL);
    let cff_raw = std::fs::read_to_string(&cff_path).map_err(|e| format!("{CITATION_CFF_REL}: {e}"))?;
    let cff_ver = read_citation_cff_version(&cff_raw)?;
    if cff_ver != ws_ver {
        return Err(format!(
            "{CITATION_CFF_REL}: `version` `{cff_ver}` does not match Cargo.toml `[workspace.package] version` `{ws_ver}`; run `cargo xtask version set {ws_ver}`"
        ));
    }
    for required in
        ["cff-version: 1.2.0", "title: CurveBook", "repository-code: https://github.com/HauserGroup/curvebook", "  - MIT", "  - Apache-2.0"]
    {
        if !cff_raw.lines().any(|line| line.trim_end() == required) {
            return Err(format!("{CITATION_CFF_REL}: missing required metadata line `{required}`"));
        }
    }
    let date = cff_raw
        .lines()
        .find_map(|line| line.trim().strip_prefix("date-released:").map(|v| v.trim().trim_matches(['"', '\''])))
        .ok_or_else(|| format!("{CITATION_CFF_REL}: missing `date-released`"))?;
    if !is_iso_date_yyyy_mm_dd(date) {
        return Err(format!("{CITATION_CFF_REL}: invalid `date-released` `{date}` (expected YYYY-MM-DD)"));
    }
    println!("citation: {CITATION_CFF_REL} version {ws_ver} matches Cargo.toml");
    Ok(())
}

fn is_iso_date_yyyy_mm_dd(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' && bytes.iter().enumerate().all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
}

pub fn read_citation_cff_version(cff: &str) -> Result<String, String> {
    for line in cff.lines() {
        if let Some(rhs) = line.strip_prefix("version:") {
            let unquoted = rhs.trim().trim_matches(['"', '\'']);
            if !unquoted.is_empty() {
                return Ok(unquoted.to_owned());
            }
        }
    }
    Err(format!("{CITATION_CFF_REL}: top-level `version:` not found"))
}

pub fn replace_citation_cff_version(cff: &str, new_ver: &str) -> Result<String, String> {
    let mut out = String::with_capacity(cff.len() + 16);
    let mut replaced = false;
    for line in cff.split_inclusive('\n') {
        if !replaced && line.starts_with("version:") {
            let newline = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            out.push_str(&format!("version: {new_ver}{newline}"));
            replaced = true;
            continue;
        }
        out.push_str(line);
    }
    if !replaced {
        return Err(format!("{CITATION_CFF_REL}: failed to locate top-level `version:`"));
    }
    Ok(out)
}

fn is_toml_version_rhs(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("version")?;
    let trimmed_rest = rest.trim_start();
    trimmed_rest.strip_prefix('=')
}

pub fn read_workspace_version(cargo_toml: &str) -> Result<String, String> {
    let mut in_ws_pkg = false;
    for line in cargo_toml.lines() {
        let no_comment = crate::scan_util::strip_toml_comment(line).trim();
        if no_comment.starts_with('[') && no_comment.ends_with(']') {
            in_ws_pkg = no_comment == "[workspace.package]";
            continue;
        }
        if in_ws_pkg && let Some(rhs) = is_toml_version_rhs(no_comment) {
            let trimmed = rhs.trim();
            let unquoted =
                trimmed.strip_prefix('"').and_then(|s| s.strip_suffix('"')).or_else(|| trimmed.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')));
            if let Some(v) = unquoted
                && !v.is_empty()
            {
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
        let no_comment = crate::scan_util::strip_toml_comment(line).trim();
        if no_comment.starts_with('[') && no_comment.ends_with(']') {
            in_ws_pkg = no_comment == "[workspace.package]";
        } else if in_ws_pkg && !replaced && is_toml_version_rhs(no_comment).is_some() {
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
    use super::{
        check_citation_cff, read_citation_cff_version, read_workspace_version, replace_citation_cff_version, replace_workspace_version,
        validate_semver,
    };
    use crate::scan_util::test_support::TestDir;

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

        // AUD-12-51: single-quoted TOML string version must unquote cleanly.
        let single_quoted = "[workspace.package]\nversion = '0.3.0-beta.2' # comment\n";
        assert_eq!(read_workspace_version(single_quoted).unwrap(), "0.3.0-beta.2");
    }

    #[test]
    fn citation_cff_version_check_and_replacement_enforce_parity() {
        let dir = TestDir::new("version-citation-cff");
        dir.write("Cargo.toml", "[workspace.package]\nversion = \"0.2.0-beta.1\"\n");
        let cff = "cff-version: 1.2.0\ntitle: CurveBook\nrepository-code: https://github.com/HauserGroup/curvebook\nlicense:\n  - MIT\n  - Apache-2.0\nversion: 0.2.0-beta.1\ndate-released: 2026-11-27\n";
        dir.write("CITATION.cff", cff);
        check_citation_cff(dir.path()).unwrap();

        let bumped = replace_citation_cff_version(cff, "0.3.0").unwrap();
        assert_eq!(read_citation_cff_version(&bumped).unwrap(), "0.3.0");
        dir.write("CITATION.cff", &bumped);
        assert!(check_citation_cff(dir.path()).unwrap_err().contains("does not match"));
    }
}
