//! The set of files the GitHub Pages site publishes (`R12.9`, `R12.10`, `R21.4`) and `cargo xtask pages-stage`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use curvebook_platform::write_atomic;

use crate::scan_util::{
    collect_files_no_symlinks, is_real_dir, is_regular_file, parse_html_tag_attributes, parse_meta_refresh_url, read_dir_paths_sorted,
    walk_html_open_tags,
};

pub const SITE_BASE_URL: &str = "https://curvebook.org/";
pub const CUSTOM_DOMAIN: Option<&str> = Some("curvebook.org");
pub const LANDING_PAGES: &[&str] =
    &["index.html", "open-pzfx-files.html", "pzfx-format-specification.html", "linux.html", "faq.html", "privacy.html"];
pub const REDIRECT_PAGES: &[&str] = &["get-started.html", "troubleshooting.html", "manual/index.html"];
pub const COMMITTED_SITE_FILES: &[&str] = &["404.html", "site.css"];
pub const GENERATED_SITE_FILES: &[&str] = &["sitemap.xml", "llms.txt", "llms-full.txt"];
pub const MANUAL_ROOT_SUBDIR: &str = "_root";
pub const SITE_DIRS: &[&str] = &["images"];
const SNAPSHOT_EXCLUDED_FILES: &[&str] = &["manifest.json"];
const SYMLINK_ERR: &str = "symlinks are never published on Pages";
const USAGE: &str = "usage: cargo xtask pages-stage [--manual-dir <dir>] (<output-dir> | --list | --check)";

#[must_use]
pub fn site_base_path() -> &'static str {
    let rest = SITE_BASE_URL.strip_prefix("https://").or_else(|| SITE_BASE_URL.strip_prefix("http://")).unwrap_or(SITE_BASE_URL);
    rest.find('/').and_then(|idx| rest.get(idx..)).unwrap_or("/")
}

#[must_use]
pub fn default_manual_dir(root: &Path) -> PathBuf {
    root.join("target/docs-manual")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StageAction {
    List,
    Check,
    Help,
    Stage(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StageCli {
    manual_dir: Option<PathBuf>,
    action: StageAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StagedEntry {
    site_rel: String,
    source_path: PathBuf,
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let cli = parse_args(args)?;
    let manual_dir = cli.manual_dir.as_deref();
    match cli.action {
        StageAction::Help => println!("{USAGE}"),
        StageAction::List => check_publish_set_with_manual(root, manual_dir)?.into_iter().for_each(|r| println!("{r}")),
        StageAction::Check => {
            let files = check_publish_set_with_manual(root, manual_dir)?;
            println!("pages-stage: {} published files, every local link resolves inside the published set", files.len());
        }
        StageAction::Stage(out) => {
            let out_dir = if out.is_absolute() { out } else { root.join(out) };
            let count = stage_with_manual(root, manual_dir, &out_dir)?;
            println!("pages-stage: staged {count} files into {}", out_dir.display());
        }
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<StageCli, String> {
    let (mut manual_dir, mut action, mut it) = (None, None, args.iter().map(String::as_str));
    while let Some(arg) = it.next() {
        if matches!(arg, "-h" | "--help") {
            return Ok(StageCli { manual_dir: None, action: StageAction::Help });
        }
        if let Some(val) = arg.strip_prefix("--manual-dir=").or_else(|| (arg == "--manual-dir").then(|| it.next()).flatten()) {
            if val.trim().is_empty() || val.starts_with('-') || manual_dir.replace(PathBuf::from(val)).is_some() {
                return Err(String::from(USAGE));
            }
        } else {
            let next = match arg {
                "--list" => StageAction::List,
                "--check" => StageAction::Check,
                a if !a.starts_with('-') => StageAction::Stage(PathBuf::from(a)),
                _ => return Err(String::from(USAGE)),
            };
            if action.replace(next).is_some() {
                return Err(String::from(USAGE));
            }
        }
    }
    Ok(StageCli { manual_dir, action: action.ok_or_else(|| String::from(USAGE))? })
}

pub fn resolve_manual_dir(root: &Path, manual_dir: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(explicit) = manual_dir {
        let resolved = if explicit.is_absolute() { explicit.to_path_buf() } else { root.join(explicit) };
        if !is_real_dir(&resolved) {
            return Err(format!("manual directory `{}` does not exist (or is not a regular directory)", resolved.display()));
        }
        if !is_regular_file(&resolved.join("index.html")) {
            return Err(format!("manual directory `{}` is missing `index.html`", resolved.display()));
        }
        return Ok(resolved);
    }
    let default_dir = default_manual_dir(root);
    [&default_dir, &root.join("docs/manual")]
        .into_iter()
        .find(|c| is_real_dir(c) && is_regular_file(&c.join("index.html")))
        .cloned()
        .ok_or_else(|| format!("manual HTML is not built (checked `{}` and `docs/manual`); run `cargo run -p xtask --features docs-site -- docs-site --build-manual` first or pass `--manual-dir <dir>`", default_dir.display()))
}

#[cfg(test)]
pub fn publish_list(root: &Path) -> Result<Vec<String>, String> {
    Ok(publish_entries(root, None)?.into_iter().map(|e| e.site_rel).collect())
}

fn check_snapshot_dir_has_manifest(dir: &Path) -> Result<bool, String> {
    let Ok(meta) = std::fs::symlink_metadata(dir) else { return Ok(false) };
    if meta.file_type().is_symlink() {
        return Err(format!("{}: {SYMLINK_ERR}", dir.display()));
    }
    let manifest = dir.join("manifest.json");
    if std::fs::symlink_metadata(&manifest).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("{}: {SYMLINK_ERR}", manifest.display()));
    }
    Ok(meta.file_type().is_dir() && is_regular_file(&manifest))
}

fn publish_entries(root: &Path, manual_dir: Option<&Path>) -> Result<Vec<StagedEntry>, String> {
    let (docs, mut entries) = (root.join("docs"), BTreeMap::new());
    for &rel in LANDING_PAGES.iter().chain(REDIRECT_PAGES).chain(COMMITTED_SITE_FILES).filter(|&&r| r != "manual/index.html") {
        let path = docs.join(rel);
        if !is_regular_file(&path) {
            return Err(format!("docs/{rel} is on the Pages publish list but missing (or not a regular file)"));
        }
        entries.insert(rel.to_owned(), path);
    }
    for dir in SITE_DIRS {
        collect_entries(&docs, &docs.join(dir), "", &mut entries)?;
    }
    let manual_root = resolve_manual_dir(root, manual_dir)?;
    entries.insert(String::from("manual/index.html"), manual_root.join("index.html"));
    let generated_root = manual_root.join(MANUAL_ROOT_SUBDIR);
    for &rel in GENERATED_SITE_FILES {
        let path = generated_root.join(rel);
        if !is_regular_file(&path) {
            return Err(format!(
                "{}/{MANUAL_ROOT_SUBDIR}/{rel} is on the Pages publish list but missing (run `cargo run -p xtask --features docs-site -- docs-site --build-manual` first)",
                manual_root.display()
            ));
        }
        entries.insert(rel.to_owned(), path);
    }
    let mut snap_roots: Vec<PathBuf> = snapshot_dirs(&manual_root)?.into_iter().map(|s| manual_root.join(s)).collect();
    let latest_dir = manual_root.join("latest");
    if check_snapshot_dir_has_manifest(&latest_dir)? {
        snap_roots.push(latest_dir);
    }
    let mut manual_page_count = 0usize;
    for snap_dir in snap_roots {
        let mut snap_entries = BTreeMap::new();
        collect_entries(&manual_root, &snap_dir, "manual/", &mut snap_entries)?;
        for (rel, src) in snap_entries {
            if !SNAPSHOT_EXCLUDED_FILES.iter().any(|name| rel.rsplit('/').next() == Some(*name)) {
                manual_page_count += 1;
                entries.insert(rel, src);
            }
        }
    }
    if manual_page_count == 0 {
        return Err(format!("manual directory `{}` contains no snapshot pages (`latest/` or `v<major>.<minor>/`)", manual_root.display()));
    }
    if let Some(md) = entries.keys().find(|rel| rel.ends_with(".md")) {
        return Err(format!("{md}: Markdown sources are never published on Pages"));
    }
    Ok(entries.into_iter().map(|(site_rel, source_path)| StagedEntry { site_rel, source_path }).collect())
}

#[cfg(test)]
pub fn check_publish_set(root: &Path) -> Result<Vec<String>, String> {
    check_publish_set_with_manual(root, None)
}

pub fn check_publish_set_with_manual(root: &Path, manual_dir: Option<&Path>) -> Result<Vec<String>, String> {
    Ok(check_publish_entries(root, manual_dir)?.into_iter().map(|e| e.site_rel).collect())
}

fn check_publish_entries(root: &Path, manual_dir: Option<&Path>) -> Result<Vec<StagedEntry>, String> {
    let entries = publish_entries(root, manual_dir)?;
    let published: BTreeSet<&str> = entries.iter().map(|e| e.site_rel.as_str()).collect();
    let mut problems = Vec::new();
    for entry in entries.iter().filter(|e| e.site_rel.ends_with(".html")) {
        let rel = entry.site_rel.as_str();
        let html = std::fs::read_to_string(&entry.source_path).map_err(|e| format!("read {}: {e}", entry.source_path.display()))?;
        match local_references(&html) {
            Ok(targets) => {
                for target in targets {
                    match resolve(rel, &target) {
                        Some(r) if published.contains(r.as_str()) => {}
                        Some(r) => problems.push(format!("{rel} links to `{target}` ({r}), which is not published")),
                        None => problems.push(format!("{rel} links to `{target}`, which leaves the site root")),
                    }
                }
            }
            Err(err) => problems.push(format!("{rel}: {err}")),
        }
    }
    let sm = entries.iter().find(|e| e.site_rel == "sitemap.xml").ok_or("sitemap.xml missing from Pages publish list")?;
    let sitemap = std::fs::read_to_string(&sm.source_path).map_err(|e| format!("read {}: {e}", sm.source_path.display()))?;
    for loc in sitemap.split("<loc>").skip(1).filter_map(|chunk| chunk.split("</loc>").next()) {
        match loc.strip_prefix(SITE_BASE_URL) {
            None => problems.push(format!("sitemap.xml lists `{loc}` outside `{SITE_BASE_URL}`")),
            Some(r) if !published.contains(if r.is_empty() { "index.html" } else { r }) => {
                problems.push(format!("sitemap.xml lists `{loc}`, which is not published"))
            }
            Some(_) => {}
        }
    }
    if problems.is_empty() { Ok(entries) } else { Err(format!("Pages publish set has {} problem(s):\n  {}", problems.len(), problems.join("\n  "))) }
}

#[cfg(test)]
pub fn stage(root: &Path, out_dir: &Path) -> Result<usize, String> {
    stage_with_manual(root, None, out_dir)
}

#[must_use]
pub fn parse_minor_dir_version(name: &str) -> Option<(u64, u64)> {
    let (maj, min) = name.strip_prefix('v')?.split_once('.')?;
    (!maj.is_empty() && !min.is_empty() && maj.bytes().chain(min.bytes()).all(|b| b.is_ascii_digit()))
        .then(|| Some((maj.parse().ok()?, min.parse().ok()?)))
        .flatten()
}

pub(crate) fn resolve_path_for_containment(path: &Path) -> Result<PathBuf, String> {
    let mut cur = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => cur.push(p.as_os_str()),
            Component::RootDir => cur.push(comp.as_os_str()),
            Component::CurDir => {}
            Component::Normal(seg) => {
                cur.push(seg);
                if std::fs::symlink_metadata(&cur).is_ok_and(|m| m.file_type().is_symlink()) {
                    cur = std::fs::canonicalize(&cur).map_err(|e| format!("canonicalize {}: {e}", cur.display()))?;
                }
            }
            Component::ParentDir => {
                if std::fs::symlink_metadata(&cur).is_ok() {
                    cur = std::fs::canonicalize(&cur).map_err(|e| format!("canonicalize {}: {e}", cur.display()))?;
                }
                cur.pop();
            }
        }
    }
    let (mut tail, mut probe) = (Vec::new(), cur.clone());
    while !probe.as_os_str().is_empty() && std::fs::symlink_metadata(&probe).is_err() {
        if let Some(name) = probe.file_name() {
            tail.push(name.to_os_string());
        }
        if !probe.pop() {
            break;
        }
    }
    let mut resolved = if !probe.as_os_str().is_empty() && std::fs::symlink_metadata(&probe).is_ok() {
        std::fs::canonicalize(&probe).map_err(|e| format!("canonicalize {}: {e}", probe.display()))?
    } else {
        cur
    };
    tail.into_iter().rev().for_each(|seg| resolved.push(seg));
    Ok(resolved)
}

pub fn stage_with_manual(root: &Path, manual_dir: Option<&Path>, out_dir: &Path) -> Result<usize, String> {
    if let Ok(meta) = std::fs::symlink_metadata(out_dir) {
        if meta.file_type().is_symlink() || !meta.file_type().is_dir() {
            return Err(format!("staging directory {} must be a regular non-symlink directory", out_dir.display()));
        }
        if std::fs::read_dir(out_dir).map_err(|e| format!("read_dir {}: {e}", out_dir.display()))?.next().is_some() {
            return Err(format!("staging directory {} must be empty", out_dir.display()));
        }
    }
    let (norm_out, norm_docs, resolved_manual) =
        (resolve_path_for_containment(out_dir)?, resolve_path_for_containment(&root.join("docs"))?, resolve_manual_dir(root, manual_dir)?);
    if norm_out.starts_with(&norm_docs) || norm_docs.starts_with(&norm_out) {
        return Err(format!("staging directory {} must be outside docs/", out_dir.display()));
    }
    let norm_manual = resolve_path_for_containment(&resolved_manual)?;
    if norm_out.starts_with(&norm_manual) || norm_manual.starts_with(&norm_out) {
        return Err(format!("staging directory {} must be outside manual directory {}", out_dir.display(), resolved_manual.display()));
    }
    let entries = check_publish_entries(root, Some(&resolved_manual))?;
    for entry in &entries {
        let bytes = std::fs::read(&entry.source_path).map_err(|e| format!("read {}: {e}", entry.source_path.display()))?;
        let dst = out_dir.join(&entry.site_rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        write_atomic(&dst, &bytes).map_err(|e| format!("write {}: {e}", dst.display()))?;
    }
    let extra = if let Some(domain) = CUSTOM_DOMAIN {
        write_atomic(&out_dir.join("CNAME"), format!("{}\n", domain.trim()).as_bytes())
            .map_err(|e| format!("write {}/CNAME: {e}", out_dir.display()))?;
        let robots = format!("User-agent: *\nAllow: /\n\nSitemap: {}/sitemap.xml\n", SITE_BASE_URL.trim_end_matches('/'));
        write_atomic(&out_dir.join("robots.txt"), robots.as_bytes()).map_err(|e| format!("write {}/robots.txt: {e}", out_dir.display()))?;
        2
    } else {
        0
    };
    Ok(entries.len() + extra)
}

fn snapshot_dirs(manual: &Path) -> Result<Vec<String>, String> {
    if !is_real_dir(manual) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for path in read_dir_paths_sorted(manual)? {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if parse_minor_dir_version(&name).is_some() && check_snapshot_dir_has_manifest(&path)? {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}

fn collect_entries(base: &Path, dir: &Path, site_prefix: &str, out: &mut BTreeMap<String, PathBuf>) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("file type of {}: {e}", dir.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!("{}: {SYMLINK_ERR}", dir.display()));
    }
    if !meta.file_type().is_dir() {
        return Err(format!("{}: expected a regular directory for Pages staging", dir.display()));
    }
    let mut rels = Vec::new();
    collect_files_no_symlinks(base, dir, SYMLINK_ERR, &mut rels)?;
    rels.into_iter().for_each(|rel| {
        out.insert(format!("{site_prefix}{rel}"), base.join(&rel));
    });
    Ok(())
}

fn local_references(html: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    walk_html_open_tags(html, |tag_lower, trimmed| {
        let attrs = parse_html_tag_attributes(trimmed);
        if let Some((bad_key, _)) = attrs.iter().find(|(k, _)| k.contains(['"', '\'', '<'])) {
            return Err(format!("malformed HTML attribute name `{bad_key}` on `<{tag_lower}>` (unclosed quote or parse error)"));
        }
        let (mut is_refresh, mut saw_equiv, mut contents) = (false, false, Vec::new());
        for (k, v) in &attrs {
            match k.to_ascii_lowercase().as_str() {
                "href" | "src" | "poster" | "action" | "formaction" | "xlink:href" => push_local(v, &mut out),
                "srcset" | "imagesrcset" => v.split(',').filter_map(|c| c.split_whitespace().next()).for_each(|u| push_local(u, &mut out)),
                "ping" => v.split_whitespace().for_each(|u| push_local(u, &mut out)),
                "http-equiv" if tag_lower == "meta" && !saw_equiv => (saw_equiv, is_refresh) = (true, v.trim().eq_ignore_ascii_case("refresh")),
                "content" if tag_lower == "meta" => contents.push(v.as_str()),
                _ => {}
            }
        }
        if is_refresh {
            contents.into_iter().filter_map(parse_meta_refresh_url).for_each(|t| push_local(t, &mut out));
        }
        Ok(())
    })?;
    Ok(out)
}

fn push_local(raw: &str, out: &mut Vec<String>) {
    let val = raw.trim();
    if val.starts_with("//") {
        return;
    }
    let path = val.split(['#', '?']).next().unwrap_or("").trim();
    if !path.is_empty() && !path.split_once(':').is_some_and(|(s, _)| !s.is_empty() && !s.contains('/')) {
        out.push(path.to_owned());
    }
}

fn resolve(from_rel: &str, target: &str) -> Option<String> {
    if target.starts_with('/') {
        return (from_rel == "404.html").then(|| target.strip_prefix(site_base_path())).flatten().and_then(|s| resolve("index.html", s));
    }
    let mut parts: Vec<&str> = from_rel.split('/').collect();
    parts.pop();
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    Some(if target.ends_with('/') || joined.is_empty() {
        if joined.is_empty() { String::from("index.html") } else { format!("{joined}/index.html") }
    } else {
        joined
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan_util::test_support::TestDir;

    fn seed(label: &str) -> TestDir {
        let root = TestDir::new(&format!("pages-site-{label}"));
        for (rel, body) in [
            (
                "docs/index.html",
                "<a href=\"open-pzfx-files.html#c\">x</a><a href=\"faq.html\">f</a><img src=\"images/a.png\"><a href=\"https://github.com/x\">g</a><a href=\"#top\">t</a><a href=\"mailto:a@b.c\">m</a>",
            ),
            (
                "docs/open-pzfx-files.html",
                "<a href=\"pzfx-format-specification.html\">s</a><a href=\"manual/latest/files.html\">f</a><link rel=\"stylesheet\" href=\"site.css\">",
            ),
            (
                "docs/pzfx-format-specification.html",
                "<a href=\"open-pzfx-files.html\">b</a><a href=\"manual/latest/files.html\">f</a><link rel=\"stylesheet\" href=\"site.css\">",
            ),
            ("docs/linux.html", "<a href=\"index.html\">h</a><a href=\"manual/latest/get-started.html\">gs</a>"),
            ("docs/faq.html", "<a href=\"index.html\">h</a><a href=\"manual/latest/index.html\">m</a>"),
            ("docs/privacy.html", "<a href=\"index.html\">h</a>"),
            ("docs/get-started.html", "<meta http-equiv=\"refresh\" content=\"0; url=manual/latest/get-started.html\">"),
            ("docs/troubleshooting.html", "<meta http-equiv=\"refresh\" content=\"0; url=manual/latest/troubleshooting.html#k\">"),
            ("target/docs-manual/index.html", "<meta http-equiv=\"refresh\" content=\"0; url=latest/index.html\">"),
            ("target/docs-manual/scratch/notes.html", "ignored"),
            ("docs/site.css", "body{}"),
            ("docs/404.html", "<link rel=\"stylesheet\" href=\"/site.css\"><a href=\"/\">h</a><a href=\"/manual/latest/index.html\">m</a>"),
            ("docs/images/a.png", "png"),
            ("target/docs-manual/_root/llms.txt", "# CurveBook\n"),
            ("target/docs-manual/_root/llms-full.txt", "# Full\n"),
            (
                "target/docs-manual/_root/sitemap.xml",
                "<urlset><url><loc>https://curvebook.org/</loc></url><url><loc>https://curvebook.org/manual/latest/files.html</loc></url></urlset>",
            ),
            ("docs/ROADMAP.md", "internal"),
        ] {
            root.write(rel, body);
        }
        for snap in ["latest", "v0.1"] {
            for (rel, body) in [
                ("manifest.json", "{}"),
                ("index.html", "<a href=\"../../index.html\">h</a><a href=\"registry/formats.html\">r</a>"),
                ("files.html", "<link href=\"../../site.css\">"),
                ("get-started.html", "gs"),
                ("troubleshooting.html", "ts"),
                ("registry/formats.html", "<a href=\"../index.html\">up</a>"),
            ] {
                root.write(&format!("target/docs-manual/{snap}/{rel}"), body);
            }
        }
        root
    }

    #[test]
    fn publish_stage_manual_dir_and_html_guards() {
        let root = seed("stage");
        let files = check_publish_set(root.path()).unwrap();
        assert!(files.len() == 25 && !files.iter().any(|f| f.ends_with("manifest.json") || f.ends_with(".md")));
        let out = root.join("_site");
        assert_eq!(stage(root.path(), &out).unwrap(), 27);
        assert_eq!(std::fs::read_to_string(out.join("CNAME")).unwrap(), "curvebook.org\n");
        assert!(std::fs::read_to_string(out.join("robots.txt")).unwrap().contains("Sitemap: https://curvebook.org/sitemap.xml"));
        assert!(!out.join("ROADMAP.md").exists() && !out.join("manual/latest/manifest.json").exists());
        assert!(stage(root.path(), &out).unwrap_err().contains("must be empty"));
        assert!(stage(root.path(), &root.join("_site/../docs/sub")).unwrap_err().contains("outside docs/"));
        assert!(stage(root.path(), &root.join("_site/../target/docs-manual/sub")).unwrap_err().contains("outside manual directory"));

        let moved = root.join("build/custom");
        std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
        std::fs::rename(root.join("target/docs-manual"), &moved).unwrap();
        assert!(check_publish_set(root.path()).unwrap_err().contains("--build-manual"));
        assert_eq!(check_publish_set_with_manual(root.path(), Some(&moved)).unwrap().len(), 25);
        assert_eq!(
            parse_args(&["--manual-dir=x", "--check"].map(String::from)).unwrap(),
            StageCli { manual_dir: Some(PathBuf::from("x")), action: StageAction::Check }
        );
        assert_eq!((parse_minor_dir_version("v0.1"), parse_minor_dir_version("v+0.+1")), (Some((0, 1)), None));
        for (from, target, exp) in [
            ("manual/v0.1/registry/formats.html", "../index.html", Some("manual/v0.1/index.html")),
            ("index.html", "manual/", Some("manual/index.html")),
            ("index.html", "../x.html", None),
            ("404.html", "/", Some("index.html")),
            ("404.html", "/../site.css", None),
        ] {
            assert_eq!(resolve(from, target).as_deref(), exp);
        }
        let r = seed("bad");
        for snippet in [
            "<a href=\"ROADMAP.md\">x</a>",
            "<a href=ROADMAP.md>x</a>",
            "<a/href=\"ROADMAP.md\">x</a>",
            "<meta content=\"0; url=ROADMAP.md\" http-equiv=\"refresh\">",
            "<meta http-equiv=\"refresh\" content=\"0, ROADMAP.md\">",
            "<p title=x\"><a href=\"ROADMAP.md\">x</a><b z=1\">",
            "<!--><a href=\"ROADMAP.md\">x</a><!-- -->",
            "<!-- unclosed",
            "<title><!--</title><a href=\"ROADMAP.md\">x</a><!-- -->",
        ] {
            r.write("docs/privacy.html", snippet);
            assert!(check_publish_set(r.path()).is_err(), "{snippet}");
        }
        r.write("docs/privacy.html", "<a href=\"index.html\">h</a>");
        r.write("target/docs-manual/_root/sitemap.xml", "<urlset><url><loc>https://curvebook.org/ROADMAP.md</loc></url></urlset>");
        assert!(check_publish_set(r.path()).is_err());
        r.write("docs/images/notes.md", "oops");
        assert!(publish_list(r.path()).unwrap_err().contains("Markdown sources"));
        #[cfg(unix)]
        {
            let s = seed("sym");
            std::os::unix::fs::symlink(s.join("docs/ROADMAP.md"), s.join("docs/images/link.png")).unwrap();
            assert!(publish_list(s.path()).unwrap_err().contains("symlinks"));
        }
    }
}
