//! `cargo xtask contributors`: refresh `contributors/contributors.json` from `git log` and
//! `contributors/people.toml` without external CI workflows or third-party crates.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

use curvebook_platform::write_atomic;

use crate::scan_util::{TomlEntry, TomlVal, parse_toml_entries};

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
    let repo = std::env::var("GITHUB_REPOSITORY")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_owned())
        .or_else(|| std::fs::read_to_string(root.join("Cargo.toml")).ok().and_then(|t| parse_cargo_repo_slug(&t)))
        .unwrap_or_else(|| String::from(DEFAULT_REPO));

    let inside = git_stdout(root, &["rev-parse", "--is-inside-work-tree"])
        .map_err(|e| format!("contributors: git worktree check failed (refusing to overwrite {OUT_REL}): {e}"))?;
    if inside != "true" {
        return Err(format!("contributors: `{}` is not inside a git work tree (refusing to overwrite {OUT_REL})", root.display()));
    }
    let branch = git_stdout(root, &["symbolic-ref", "--short", "HEAD"]).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| String::from("main"));
    let out_path = root.join(OUT_REL);
    let fallback_date = std::fs::read_to_string(&out_path)
        .ok()
        .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
        .and_then(|v| v.get("head_date").and_then(serde_json::Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| String::from("1970-01-01T00:00:00Z"));

    let (head, head_date, commits, unattributed, roster) = match git_stdout(root, &["rev-parse", "--verify", "HEAD"]) {
        Ok(sha) if !sha.is_empty() => {
            let date = git_stdout(root, &["show", "-s", "--format=%cI", "HEAD"])
                .map_err(|e| format!("contributors: failed to read HEAD commit date (refusing to overwrite {OUT_REL}): {e}"))?;
            if date.is_empty() {
                return Err(format!("contributors: empty HEAD commit date (refusing to overwrite {OUT_REL})"));
            }
            let log = git_stdout(root, &["log", "--format=%an\x1f%ae"])
                .map_err(|e| format!("contributors: failed to read git log (refusing to overwrite {OUT_REL}): {e}"))?;
            let (total, unattr, entries) = summarize_log(&log, &overrides);
            (sha, date, total, unattr, entries)
        }
        Ok(_) => return Err(format!("contributors: `git rev-parse --verify HEAD` returned empty SHA (refusing to overwrite {OUT_REL})")),
        Err(_)
            if git_stdout(root, &["symbolic-ref", "--short", "HEAD"]).is_ok_and(|s| !s.is_empty())
                && git_stdout(root, &["rev-list", "--all", "--count"]).is_ok_and(|c| c == "0") =>
        {
            (String::from(ZERO_HEAD), fallback_date, 0, 0, Vec::new())
        }
        Err(err) => return Err(format!("contributors: cannot resolve HEAD (refusing to overwrite {OUT_REL}): {err}")),
    };

    let rendered = render_contributors_json(&repo, &branch, &head, &head_date, commits, unattributed, &roster);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    write_atomic(&out_path, rendered.as_bytes()).map_err(|e| format!("write {OUT_REL}: {e}"))?;
    println!("contributors: wrote {OUT_REL} ({commits} commit(s), {} listed contributor(s))", roster.len());
    Ok(())
}

#[must_use]
pub fn parse_cargo_repo_slug(cargo_toml: &str) -> Option<String> {
    parse_toml_entries(cargo_toml).into_iter().find_map(|entry| match entry {
        TomlEntry::Leaf { path, value, .. } if path == ["workspace", "package", "repository"] => {
            let rest = value.as_str()?.trim_end_matches('/').trim_end_matches(".git").strip_prefix("https://github.com/")?;
            (rest.split('/').count() == 2).then(|| rest.to_owned())
        }
        _ => None,
    })
}

fn git_stdout(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = crate::dev_tool_command("git")
        .current_dir(root)
        .args(args.iter().copied())
        .wall_timeout(Duration::from_secs(60))
        .idle_timeout(Duration::from_secs(30))
        .run()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.success {
        return Err(format!("git {} exited with {:?}: {}", args.join(" "), out.status_code, String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

#[must_use]
pub fn parse_people_toml(text: &str) -> BTreeMap<String, PersonOverride> {
    let mut map: BTreeMap<String, PersonOverride> = BTreeMap::new();
    for entry in parse_toml_entries(text) {
        match entry {
            TomlEntry::Section { path, .. } if path.len() == 2 && path[0] == "people" => {
                map.entry(path[1].to_ascii_lowercase()).or_default();
            }
            TomlEntry::Leaf { path, value, .. } if path.len() == 3 && path[0] == "people" => {
                let ov = map.entry(path[1].to_ascii_lowercase()).or_default();
                match path[2].as_str() {
                    "real_name" | "name" => ov.real_name = value.as_str().filter(|s| !s.is_empty()).map(str::to_owned),
                    "display_name" => ov.display_name = value.as_str().filter(|s| !s.is_empty()).map(str::to_owned),
                    "unlisted" => ov.unlisted = matches!(value, TomlVal::Other(s) if s == "true"),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    map
}

#[must_use]
pub fn summarize_log(log: &str, overrides: &BTreeMap<String, PersonOverride>) -> (usize, usize, Vec<ContributorEntry>) {
    let (mut counts, mut total, mut unattributed): (BTreeMap<String, (String, usize)>, usize, usize) = (BTreeMap::new(), 0, 0);
    for line in log.lines().filter(|l| !l.trim().is_empty()) {
        total += 1;
        let (name, email) = line.split_once('\x1f').map_or((line.trim(), ""), |(n, e)| (n.trim(), e.trim()));
        if let Some(login) = resolve_github_login(name, email, overrides) {
            counts.entry(login.to_ascii_lowercase()).or_insert_with(|| (login, 0)).1 += 1;
        } else {
            unattributed += 1;
        }
    }
    let mut entries: Vec<ContributorEntry> = counts
        .into_iter()
        .filter_map(|(key, (login, commits))| {
            let ov = overrides.get(&key);
            (!ov.is_some_and(|o| o.unlisted)).then(|| ContributorEntry {
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
    let is_handle = |s: &str| {
        !s.is_empty() && s.len() <= 39 && !s.starts_with('-') && !s.ends_with('-') && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if let Some(local) = email.to_ascii_lowercase().strip_suffix("@users.noreply.github.com") {
        let login = local.rsplit('+').next().unwrap_or(local);
        if is_handle(login) {
            return Some(login.to_owned());
        }
    }
    (is_handle(name) || overrides.contains_key(&name.to_ascii_lowercase())).then(|| name.to_owned())
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
    let q = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| String::from("\"\""));
    let mut out = format!(
        "{{\n  \"schema\": 1,\n  \"repo\": {},\n  \"branch\": {},\n  \"head\": {},\n  \"head_date\": {},\n  \"commits\": {commits},\n  \"unattributed_commits\": {unattributed_commits},\n",
        q(repo),
        q(branch),
        q(head),
        q(head_date)
    );
    if contributors.is_empty() {
        out.push_str("  \"contributors\": [],\n");
    } else {
        out.push_str("  \"contributors\": [\n");
        for (idx, c) in contributors.iter().enumerate() {
            let _ = writeln!(out, "    {{\n      \"login\": {},", q(&c.login));
            if let Some(dn) = &c.display_name {
                let _ = writeln!(out, "      \"display_name\": {},", q(dn));
            }
            if let Some(rn) = &c.real_name {
                let _ = writeln!(out, "      \"real_name\": {},", q(rn));
            }
            let comma = if idx + 1 == contributors.len() { "" } else { "," };
            let _ = writeln!(out, "      \"commits\": {}\n    }}{comma}", c.commits);
        }
        out.push_str("  ],\n");
    }
    out.push_str("  \"models\": []\n}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::{ContributorEntry, parse_cargo_repo_slug, parse_people_toml, render_contributors_json, run, summarize_log};
    use crate::scan_util::test_support::TestDir;

    #[test]
    fn overrides_repo_slug_log_summary_and_fail_closed_git_guards() {
        let overrides =
            parse_people_toml("[people.ada-lovelace]\nreal_name = \"Ada Lovelace\"\ndisplay_name = \"Ada\"\n\n[people.quiet-dev]\nunlisted = true\n");
        let log = "Ada Lovelace\x1f12345+ada-lovelace@users.noreply.github.com\nAda Lovelace\x1fada-lovelace@users.noreply.github.com\nQuiet\x1f999+quiet-dev@users.noreply.github.com\nLocal User\x1flocal@example.org\n";
        let (total, unattributed, entries) = summarize_log(log, &overrides);
        assert_eq!(
            (total, unattributed, &entries),
            (
                4,
                1,
                &vec![ContributorEntry {
                    login: "ada-lovelace".into(),
                    commits: 2,
                    display_name: Some("Ada".into()),
                    real_name: Some("Ada Lovelace".into())
                }]
            )
        );
        let parsed: serde_json::Value = serde_json::from_str(&render_contributors_json(
            "HauserGroup/curvebook",
            "main",
            "abc",
            "2026-11-13T00:00:00Z",
            total,
            unattributed,
            &entries,
        ))
        .unwrap();
        assert_eq!(parsed.get("repo").and_then(serde_json::Value::as_str), Some("HauserGroup/curvebook"));

        assert_eq!(parse_cargo_repo_slug("[workspace.package] # header\nrepository_mirror = 'https://evil.example/a/b'\nrepository='https://github.com/HauserGroup/curvebook' # ok\n").as_deref(), Some("HauserGroup/curvebook"));
        let map = parse_people_toml("[people.'ada-lovelace']\nreal_name='Ada # Byron'\ndisplay_name='Ada' # ok\n");
        assert_eq!((map["ada-lovelace"].real_name.as_deref(), map["ada-lovelace"].display_name.as_deref()), (Some("Ada # Byron"), Some("Ada")));

        let non_git = TestDir::new("contributors-nogit");
        let existing = "{\"schema\":1,\"commits\":244,\"contributors\":[{\"login\":\"alice\",\"commits\":244}]}\n";
        non_git.write("contributors/contributors.json", existing);
        assert!(run(non_git.path()).unwrap_err().contains("refusing to overwrite"));
        assert_eq!(std::fs::read_to_string(non_git.join("contributors/contributors.json")).unwrap(), existing);

        let unborn = TestDir::git_repo("contributors-unborn");
        run(unborn.path()).unwrap();
        assert!(std::fs::read_to_string(unborn.join("contributors/contributors.json")).unwrap().contains("\"commits\": 0"));
    }
}
