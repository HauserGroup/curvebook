//! `cargo xtask contributors`: refresh `contributors/contributors.json` from `git log` and
//! `contributors/people.toml` without external CI workflows or third-party crates.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use curvebook_platform::write_atomic;

const PEOPLE_REL: &str = "contributors/people.toml";
const OUT_REL: &str = "contributors/contributors.json";
const ZERO_HEAD: &str = "0000000000000000000000000000000000000000";
const DEFAULT_REPO: &str = "HauserGroup/curvebook";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersonOverride {
    pub real_name: Option<String>,
    pub display_name: Option<String>,
    pub unlisted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributorEntry {
    pub login: String,
    pub commits: usize,
    pub display_name: Option<String>,
    pub real_name: Option<String>,
}

pub fn run(root: &Path) -> Result<(), String> {
    let people_path = root.join(PEOPLE_REL);
    let people_raw =
        if people_path.is_file() { std::fs::read_to_string(&people_path).map_err(|e| format!("{PEOPLE_REL}: {e}"))? } else { String::new() };
    let overrides = parse_people_toml(&people_raw);

    let repo = resolve_repo_slug(root);
    let branch = git_stdout(root, &["symbolic-ref", "--short", "HEAD"]).unwrap_or_else(|| String::from("main"));
    let out_path = root.join(OUT_REL);
    let fallback_date = existing_head_date(&out_path).unwrap_or_else(|| String::from("1970-01-01T00:00:00Z"));

    let (head, head_date, commits, unattributed, roster) = match git_stdout(root, &["rev-parse", "HEAD"]) {
        Some(sha) => {
            let date = git_stdout(root, &["show", "-s", "--format=%cI", "HEAD"]).unwrap_or(fallback_date);
            let log = git_stdout(root, &["log", "--format=%an\x1f%ae"]).unwrap_or_default();
            let (total, unattr, entries) = summarize_log(&log, &overrides);
            (sha, date, total, unattr, entries)
        }
        None => (String::from(ZERO_HEAD), fallback_date, 0, 0, Vec::new()),
    };

    let rendered = render_contributors_json(&repo, &branch, &head, &head_date, commits, unattributed, &roster);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    write_atomic(&out_path, rendered.as_bytes()).map_err(|e| format!("write {OUT_REL}: {e}"))?;
    println!("contributors: wrote {OUT_REL} ({commits} commit(s), {} listed contributor(s))", roster.len());
    Ok(())
}

fn resolve_repo_slug(root: &Path) -> String {
    if let Some(env_repo) = std::env::var("GITHUB_REPOSITORY").ok().filter(|s| !s.trim().is_empty()) {
        return env_repo.trim().to_owned();
    }
    if let Ok(cargo_toml) = std::fs::read_to_string(root.join("Cargo.toml"))
        && let Some(slug) = parse_cargo_repo_slug(&cargo_toml)
    {
        return slug;
    }
    String::from(DEFAULT_REPO)
}

#[must_use]
pub fn parse_cargo_repo_slug(cargo_toml: &str) -> Option<String> {
    let mut in_pkg = false;
    for raw in cargo_toml.lines() {
        let trimmed = strip_toml_comment(raw).trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('[') {
            in_pkg = trimmed == "[workspace.package]";
            continue;
        }
        if !in_pkg {
            continue;
        }
        let Some((key, rhs)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim() != "repository" {
            continue;
        }
        let val = parse_quoted_value(rhs.trim())?;
        let stripped = val.trim_end_matches('/').trim_end_matches(".git");
        if let Some(rest) = stripped.strip_prefix("https://github.com/")
            && rest.split('/').count() == 2
        {
            return Some(rest.to_owned());
        }
    }
    None
}

fn existing_head_date(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    parsed.get("head_date").and_then(serde_json::Value::as_str).map(str::to_owned)
}

fn git_stdout(root: &Path, args: &[&str]) -> Option<String> {
    let out = crate::dev_tool_command("git")
        .current_dir(root)
        .args(args.iter().copied())
        .wall_timeout(Duration::from_secs(60))
        .idle_timeout(Duration::from_secs(30))
        .run()
        .ok()?;
    if !out.success {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if text.is_empty() { None } else { Some(text) }
}

#[must_use]
pub fn parse_people_toml(text: &str) -> BTreeMap<String, PersonOverride> {
    let mut map: BTreeMap<String, PersonOverride> = BTreeMap::new();
    let mut current: Option<String> = None;
    for raw in text.lines() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current = header.trim().strip_prefix("people.").map(|s| s.trim().trim_matches(['"', '\'']).to_ascii_lowercase());
            if let Some(login) = &current {
                map.entry(login.clone()).or_default();
            }
            continue;
        }
        let Some(login) = &current else {
            continue;
        };
        let Some((key, rhs)) = line.split_once('=') else {
            continue;
        };
        let entry = map.entry(login.clone()).or_default();
        apply_person_field(entry, key.trim(), rhs.trim());
    }
    map
}

fn apply_person_field(entry: &mut PersonOverride, key: &str, rhs: &str) {
    match key {
        "real_name" | "name" => {
            entry.real_name = parse_quoted_value(rhs).filter(|s| !s.is_empty());
        }
        "display_name" => {
            entry.display_name = parse_quoted_value(rhs).filter(|s| !s.is_empty());
        }
        "unlisted" => {
            entry.unlisted = rhs == "true";
        }
        _ => {}
    }
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
        } else if in_single {
            if ch == '\'' {
                in_single = false;
            }
        } else {
            match ch {
                '"' => in_double = true,
                '\'' => in_single = true,
                '#' => return &line[..idx],
                _ => {}
            }
        }
    }
    line
}

fn parse_quoted_value(rhs: &str) -> Option<String> {
    if let Some(inner) = rhs.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return Some(inner.replace("\\\"", "\"").replace("\\\\", "\\"));
    }
    if let Some(inner) = rhs.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return Some(inner.to_owned());
    }
    None
}

#[must_use]
pub fn summarize_log(log: &str, overrides: &BTreeMap<String, PersonOverride>) -> (usize, usize, Vec<ContributorEntry>) {
    let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut total = 0usize;
    let mut unattributed = 0usize;

    for line in log.lines().filter(|l| !l.trim().is_empty()) {
        total += 1;
        let mut parts = line.split('\x1f');
        let name = parts.next().unwrap_or_default().trim();
        let email = parts.next().unwrap_or_default().trim();
        if let Some(login) = resolve_github_login(name, email, overrides) {
            let key = login.to_ascii_lowercase();
            let entry = counts.entry(key).or_insert_with(|| (login, 0));
            entry.1 += 1;
        } else {
            unattributed += 1;
        }
    }

    let mut entries: Vec<ContributorEntry> = counts
        .into_iter()
        .filter_map(|(key, (login, commits))| {
            let ov = overrides.get(&key);
            if ov.is_some_and(|o| o.unlisted) {
                return None;
            }
            Some(ContributorEntry {
                login,
                commits,
                display_name: ov.and_then(|o| o.display_name.clone()),
                real_name: ov.and_then(|o| o.real_name.clone()),
            })
        })
        .collect();
    entries.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.login.cmp(&b.login)));
    (total, unattributed, entries)
}

fn resolve_github_login(name: &str, email: &str, overrides: &BTreeMap<String, PersonOverride>) -> Option<String> {
    let lower_email = email.to_ascii_lowercase();
    if let Some(local) = lower_email.strip_suffix("@users.noreply.github.com") {
        let login = local.rsplit('+').next().unwrap_or(local);
        if is_github_handle(login) {
            return Some(login.to_owned());
        }
    }
    let trimmed = name.trim();
    if is_github_handle(trimmed) {
        return Some(trimmed.to_owned());
    }
    let key = trimmed.to_ascii_lowercase();
    if overrides.contains_key(&key) {
        return Some(trimmed.to_owned());
    }
    None
}

fn is_github_handle(s: &str) -> bool {
    !s.is_empty() && s.len() <= 39 && !s.starts_with('-') && !s.ends_with('-') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[must_use]
pub fn render_contributors_json(
    repo: &str,
    branch: &str,
    head: &str,
    head_date: &str,
    commits: usize,
    unattributed_commits: usize,
    contributors: &[ContributorEntry],
) -> String {
    let mut out = String::from("{\n");
    out.push_str("  \"schema\": 1,\n");
    out.push_str(&format!("  \"repo\": \"{}\",\n", escape_json(repo)));
    out.push_str(&format!("  \"branch\": \"{}\",\n", escape_json(branch)));
    out.push_str(&format!("  \"head\": \"{}\",\n", escape_json(head)));
    out.push_str(&format!("  \"head_date\": \"{}\",\n", escape_json(head_date)));
    out.push_str(&format!("  \"commits\": {commits},\n"));
    out.push_str(&format!("  \"unattributed_commits\": {unattributed_commits},\n"));
    if contributors.is_empty() {
        out.push_str("  \"contributors\": [],\n");
    } else {
        out.push_str("  \"contributors\": [\n");
        for (idx, c) in contributors.iter().enumerate() {
            out.push_str("    {\n");
            out.push_str(&format!("      \"login\": \"{}\",\n", escape_json(&c.login)));
            if let Some(dn) = &c.display_name {
                out.push_str(&format!("      \"display_name\": \"{}\",\n", escape_json(dn)));
            }
            if let Some(rn) = &c.real_name {
                out.push_str(&format!("      \"real_name\": \"{}\",\n", escape_json(rn)));
            }
            out.push_str(&format!("      \"commits\": {}\n", c.commits));
            let comma = if idx + 1 == contributors.len() { "" } else { "," };
            out.push_str(&format!("    }}{comma}\n"));
        }
        out.push_str("  ],\n");
    }
    out.push_str("  \"models\": []\n");
    out.push_str("}\n");
    out
}

fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{ContributorEntry, parse_cargo_repo_slug, parse_people_toml, render_contributors_json, summarize_log};

    #[test]
    fn parses_people_overrides_and_summarizes_git_log() {
        let people = "\
[people.ada-lovelace]
real_name = \"Ada Lovelace\"
display_name = \"Ada\"

[people.quiet-dev]
unlisted = true
";
        let overrides = parse_people_toml(people);
        let log = "\
Ada Lovelace\x1f12345+ada-lovelace@users.noreply.github.com
Ada Lovelace\x1fada-lovelace@users.noreply.github.com
Quiet\x1f999+quiet-dev@users.noreply.github.com
Local User\x1flocal@example.org
";
        let (total, unattributed, entries) = summarize_log(log, &overrides);
        assert_eq!(total, 4);
        assert_eq!(unattributed, 1);
        assert_eq!(
            entries,
            vec![ContributorEntry {
                login: String::from("ada-lovelace"),
                commits: 2,
                display_name: Some(String::from("Ada")),
                real_name: Some(String::from("Ada Lovelace")),
            }]
        );
        let json = render_contributors_json("HauserGroup/curvebook", "main", "abc", "2026-11-13T00:00:00Z", total, unattributed, &entries);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.get("repo").and_then(serde_json::Value::as_str), Some("HauserGroup/curvebook"));
    }

    #[test]
    fn extracts_repo_slug_from_workspace_cargo_toml() {
        let cargo = "[workspace.package]\nversion = \"0.1.0\"\nrepository = \"https://github.com/HauserGroup/curvebook\"\n";
        assert_eq!(parse_cargo_repo_slug(cargo).as_deref(), Some("HauserGroup/curvebook"));
    }

    #[test]
    fn fa_sec23_15_handles_single_quotes_compact_assign_and_comments() {
        let cargo = "[workspace.package] # header comment\nrepository_mirror = 'https://evil.example/a/b'\nrepository='https://github.com/HauserGroup/curvebook' # trailing\n";
        assert_eq!(parse_cargo_repo_slug(cargo).as_deref(), Some("HauserGroup/curvebook"));

        let people = "[people.'ada-lovelace']\nreal_name='Ada # Byron'\ndisplay_name='Ada' # ok\n";
        let map = parse_people_toml(people);
        assert_eq!(map["ada-lovelace"].real_name.as_deref(), Some("Ada # Byron"));
        assert_eq!(map["ada-lovelace"].display_name.as_deref(), Some("Ada"));
    }
}
