//! `cargo xtask ci-cache`: trim Cargo target directories to active build units and
//! restore unchanged workspace file modification times across CI cache restores (`R0.35`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

const USAGE: &str = "\
usage:
  cargo xtask ci-cache trim [--target-dir <dir>] [--profile <name>] [--messages <jsonl>]... [--base-sha <sha>]
  cargo xtask ci-cache restore-mtimes [--target-dir <dir>] [--base-sha <sha>]";

const BASE_SHA_FILE: &str = ".ci-cache-base-sha";
const TREE_HASHES_FILE: &str = ".ci-cache-tree-hashes";
const UNITS_FILE: &str = ".ci-cache-units.jsonl";

/// Deterministic pre-build baseline timestamp (`2020-01-01T00:00:00Z`) older than any CI cache artifact.
const DEFAULT_BASELINE_EPOCH_SECS: u64 = 1_577_836_800;

/// Summary of a `ci-cache trim` pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TrimStats {
    pub active_units: usize,
    pub removed_entries: usize,
    pub removed_bytes: u64,
}

/// Summary of a `ci-cache restore-mtimes` pass.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RestoreStats {
    pub restored_files: usize,
    pub skipped_modified: usize,
}

#[derive(Default)]
struct ActiveUnits {
    unit_hashes: BTreeSet<String>,
    fingerprint_dirs: BTreeSet<String>,
    build_dirs: BTreeSet<String>,
    top_files: BTreeSet<String>,
    example_files: BTreeSet<String>,
    top_targets: Vec<(Option<String>, String)>,
}

/// Execute `cargo xtask ci-cache <trim|restore-mtimes> ...`.
pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(USAGE.to_owned());
    };
    match sub.as_str() {
        "trim" => run_trim(root, rest),
        "restore-mtimes" => run_restore_mtimes(root, rest),
        other => Err(format!("unknown `ci-cache` subcommand `{other}`\n\n{USAGE}")),
    }
}

fn run_trim(root: &Path, args: &[String]) -> Result<(), String> {
    let opts = parse_trim_args(root, args)?;
    let target_dir = resolve_target_dir(root, opts.target_dir.as_deref());
    let base_sha = match opts.base_sha {
        Some(sha) => sha,
        None => git_head_sha(root)?,
    };
    let jsonl = load_or_record_messages(root, &target_dir, &opts.messages)?;
    let stats = trim_target_dir(root, &target_dir, &opts.profile, &jsonl, &base_sha)?;
    println!(
        "ci-cache trim: retained {} active unit(s), pruned {} stale entry/entries ({:.2} MiB); base SHA {}",
        stats.active_units,
        stats.removed_entries,
        bytes_to_mib(stats.removed_bytes),
        short_sha(&base_sha)
    );
    Ok(())
}

fn run_restore_mtimes(root: &Path, args: &[String]) -> Result<(), String> {
    let opts = parse_restore_args(root, args)?;
    let target_dir = resolve_target_dir(root, opts.target_dir.as_deref());
    let base_sha = match opts.base_sha {
        Some(sha) => Some(sha),
        None => read_cached_base_sha(&target_dir)?,
    };
    let Some(sha) = base_sha else {
        println!("ci-cache restore-mtimes: no cached base SHA at {}; skipping", target_dir.join(BASE_SHA_FILE).display());
        return Ok(());
    };
    let stats = restore_mtimes(root, &target_dir, &sha)?;
    println!(
        "ci-cache restore-mtimes: restored baseline mtime on {} unchanged file(s), kept {} modified file(s) (base {})",
        stats.restored_files,
        stats.skipped_modified,
        short_sha(&sha)
    );
    Ok(())
}

struct TrimArgs {
    target_dir: Option<PathBuf>,
    profile: String,
    messages: Vec<PathBuf>,
    base_sha: Option<String>,
}

struct RestoreArgs {
    target_dir: Option<PathBuf>,
    base_sha: Option<String>,
}

fn parse_trim_args(root: &Path, args: &[String]) -> Result<TrimArgs, String> {
    let mut parsed = TrimArgs { target_dir: None, profile: String::from("debug"), messages: Vec::new(), base_sha: None };
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--target-dir" => {
                let val = take_flag_value(args, &mut i, "--target-dir")?;
                parsed.target_dir = Some(resolve_input_path(root, val));
            }
            "--profile" => {
                let val = take_flag_value(args, &mut i, "--profile")?;
                parsed.profile = val.to_owned();
            }
            "--messages" => {
                let val = take_flag_value(args, &mut i, "--messages")?;
                parsed.messages.push(resolve_input_path(root, val));
            }
            "--base-sha" => {
                let val = take_flag_value(args, &mut i, "--base-sha")?;
                parsed.base_sha = Some(val.to_owned());
            }
            other => return Err(format!("unexpected `ci-cache trim` argument `{other}`\n\n{USAGE}")),
        }
        i += 1;
    }
    Ok(parsed)
}

fn parse_restore_args(root: &Path, args: &[String]) -> Result<RestoreArgs, String> {
    let mut parsed = RestoreArgs { target_dir: None, base_sha: None };
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--target-dir" => {
                let val = take_flag_value(args, &mut i, "--target-dir")?;
                parsed.target_dir = Some(resolve_input_path(root, val));
            }
            "--base-sha" => {
                let val = take_flag_value(args, &mut i, "--base-sha")?;
                parsed.base_sha = Some(val.to_owned());
            }
            other => return Err(format!("unexpected `ci-cache restore-mtimes` argument `{other}`\n\n{USAGE}")),
        }
        i += 1;
    }
    Ok(parsed)
}

fn take_flag_value<'a>(args: &'a [String], idx: &mut usize, flag: &str) -> Result<&'a str, String> {
    let next = idx.saturating_add(1);
    let Some(val) = args.get(next) else {
        return Err(format!("missing value for `{flag}`\n\n{USAGE}"));
    };
    if val.is_empty() {
        return Err(format!("empty value for `{flag}`"));
    }
    *idx = next;
    Ok(val.as_str())
}

fn resolve_input_path(root: &Path, raw: &str) -> PathBuf {
    let p = Path::new(raw);
    if p.is_absolute() { p.to_path_buf() } else { root.join(p) }
}

fn resolve_target_dir(root: &Path, explicit: Option<&Path>) -> PathBuf {
    if let Some(dir) = explicit {
        return dir.to_path_buf();
    }
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from)
}

fn load_or_record_messages(root: &Path, target_dir: &Path, message_files: &[PathBuf]) -> Result<String, String> {
    if !message_files.is_empty() {
        let mut combined = String::new();
        for path in message_files {
            let chunk = fs::read_to_string(path).map_err(|e| format!("read messages `{}`: {e}", path.display()))?;
            combined.push_str(&chunk);
            if !combined.ends_with('\n') {
                combined.push('\n');
            }
        }
        return Ok(combined);
    }

    let default_units = target_dir.join(UNITS_FILE);
    if default_units.is_file() {
        return fs::read_to_string(&default_units).map_err(|e| format!("read `{}`: {e}", default_units.display()));
    }

    record_workspace_cargo_messages(root, target_dir)
}

fn record_workspace_cargo_messages(root: &Path, target_dir: &Path) -> Result<String, String> {
    let mut combined = String::new();
    for args in [
        &["build", "--locked", "-p", "xtask", "--message-format=json-render-diagnostics"][..],
        &["test", "--locked", "--workspace", "--all-targets", "--no-run", "--message-format=json-render-diagnostics"][..],
    ] {
        let out = Command::new("cargo")
            .current_dir(root)
            .env("CARGO_TARGET_DIR", target_dir)
            .args(args)
            .output()
            .map_err(|e| format!("cargo {}: {e}", args.join(" ")))?;
        if !out.status.success() {
            return Err(format!("`cargo {}` failed ({}): {}", args.join(" "), out.status, String::from_utf8_lossy(&out.stderr)));
        }
        let stdout = String::from_utf8(out.stdout).map_err(|e| format!("cargo json utf-8: {e}"))?;
        combined.push_str(&stdout);
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }
    Ok(combined)
}

/// Trim `target_dir/<profile>` to the build units recorded in `messages_jsonl` and write `base_sha`.
pub fn trim_target_dir(root: &Path, target_dir: &Path, profile: &str, messages_jsonl: &str, base_sha: &str) -> Result<TrimStats, String> {
    let clean_sha = validate_sha_text(base_sha)?;
    let profile_dir = target_dir.join(profile);
    if !profile_dir.is_dir() {
        return Err(format!("profile directory `{}` does not exist", profile_dir.display()));
    }

    let mut active = parse_active_units(&profile_dir, messages_jsonl)?;
    resolve_top_level_units(&profile_dir, &mut active)?;
    if active.unit_hashes.is_empty() && active.fingerprint_dirs.is_empty() && active.build_dirs.is_empty() {
        return Err(String::from("refusing to trim: zero active build units found in Cargo JSON messages"));
    }

    let mut stats = TrimStats { active_units: active.unit_hashes.len().max(active.fingerprint_dirs.len()), removed_entries: 0, removed_bytes: 0 };

    prune_top_archives(target_dir, &mut stats)?;
    prune_named_unit_dir(&profile_dir.join(".fingerprint"), &active.fingerprint_dirs, &active.unit_hashes, &mut stats)?;
    prune_named_unit_dir(&profile_dir.join("build"), &active.build_dirs, &active.unit_hashes, &mut stats)?;
    prune_deps_dir(&profile_dir.join("deps"), &active.unit_hashes, &mut stats)?;
    prune_dir_children(&profile_dir.join("incremental"), &mut stats)?;
    prune_examples_dir(&profile_dir.join("examples"), &active.example_files, &active.unit_hashes, &mut stats)?;
    prune_profile_top_files(&profile_dir, &active.top_files, &mut stats)?;

    write_base_state(root, target_dir, &clean_sha)?;
    Ok(stats)
}

fn validate_sha_text(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 64 || !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid git commit SHA `{trimmed}`"));
    }
    Ok(trimmed.to_owned())
}

fn parse_active_units(profile_dir: &Path, messages_jsonl: &str) -> Result<ActiveUnits, String> {
    let mut active = ActiveUnits::default();
    let canonical_profile = fs::canonicalize(profile_dir).unwrap_or_else(|_| profile_dir.to_path_buf());

    for raw in messages_jsonl.lines() {
        let trimmed = raw.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        let val: Value = serde_json::from_str(trimmed).map_err(|e| format!("parse Cargo JSON message: {e}"))?;
        let Some(reason) = val.get("reason").and_then(Value::as_str) else {
            continue;
        };
        match reason {
            "compiler-artifact" => collect_artifact_units(&val, profile_dir, &canonical_profile, &mut active),
            "build-script-executed" => collect_build_script_units(&val, profile_dir, &canonical_profile, &mut active),
            _ => {}
        }
    }
    Ok(active)
}

fn collect_artifact_units(val: &Value, profile_dir: &Path, canonical_profile: &Path, active: &mut ActiveUnits) {
    let pkg_hint = val.get("package_id").and_then(Value::as_str).and_then(extract_pkg_name_from_id);
    let mut paths: Vec<&str> = Vec::new();
    if let Some(arr) = val.get("filenames").and_then(Value::as_array) {
        paths.extend(arr.iter().filter_map(Value::as_str));
    }
    if let Some(exe) = val.get("executable").and_then(Value::as_str) {
        paths.push(exe);
    }

    for path_str in paths {
        let Some(rel) = relative_to_profile(Path::new(path_str), profile_dir, canonical_profile) else {
            continue;
        };
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        match parts.as_slice() {
            [dir, name, ..] if dir == "deps" => {
                if let Some(hash) = extract_unit_hash(name) {
                    active.unit_hashes.insert(hash.to_owned());
                }
            }
            [dir, unit_dir, ..] if dir == "build" => {
                active.build_dirs.insert(unit_dir.clone());
                active.fingerprint_dirs.insert(unit_dir.clone());
                if let Some(hash) = extract_unit_hash(unit_dir) {
                    active.unit_hashes.insert(hash.to_owned());
                }
            }
            [dir, name, ..] if dir == "examples" => {
                active.example_files.insert(name.clone());
                if let Some(hash) = extract_unit_hash(name) {
                    active.unit_hashes.insert(hash.to_owned());
                }
            }
            [top_name] => {
                record_top_level_file(top_name, pkg_hint.as_deref(), active);
            }
            _ => {}
        }
    }
}

fn collect_build_script_units(val: &Value, profile_dir: &Path, canonical_profile: &Path, active: &mut ActiveUnits) {
    let Some(out_dir) = val.get("out_dir").and_then(Value::as_str) else {
        return;
    };
    let Some(rel) = relative_to_profile(Path::new(out_dir), profile_dir, canonical_profile) else {
        return;
    };
    let mut comps = rel.components();
    if let (Some(Component::Normal(dir)), Some(Component::Normal(unit_dir))) = (comps.next(), comps.next())
        && dir == "build"
    {
        let unit_name = unit_dir.to_string_lossy().into_owned();
        active.build_dirs.insert(unit_name.clone());
        active.fingerprint_dirs.insert(unit_name.clone());
        if let Some(hash) = extract_unit_hash(&unit_name) {
            active.unit_hashes.insert(hash.to_owned());
        }
    }
}

fn record_top_level_file(top_name: &str, pkg_hint: Option<&str>, active: &mut ActiveUnits) {
    active.top_files.insert(top_name.to_owned());
    let stem = top_name.strip_suffix(".exe").unwrap_or(top_name);
    active.top_files.insert(format!("{stem}.d"));
    active.top_files.insert(format!("{stem}.pdb"));
    active.top_files.insert(format!("{stem}.dSYM"));
    if let Some(raw_stem) =
        stem.strip_prefix("lib").and_then(|s| s.strip_suffix(".rlib").or_else(|| s.strip_suffix(".dylib")).or_else(|| s.strip_suffix(".so")))
    {
        active.top_files.insert(format!("{raw_stem}.d"));
    }
    active.top_targets.push((pkg_hint.map(str::to_owned), top_name.to_owned()));
}

fn resolve_top_level_units(profile_dir: &Path, active: &mut ActiveUnits) -> Result<(), String> {
    if active.top_targets.is_empty() {
        return Ok(());
    }
    let fp_root = profile_dir.join(".fingerprint");
    if !fp_root.is_dir() {
        return Ok(());
    }
    let mut fp_entries = Vec::new();
    for entry in fs::read_dir(&fp_root).map_err(|e| format!("read `{}`: {e}", fp_root.display()))? {
        let entry = entry.map_err(|e| format!("dir entry `{}`: {e}", fp_root.display()))?;
        if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
            fp_entries.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    fp_entries.sort();

    let deps_dir = profile_dir.join("deps");
    for (pkg_hint, top_name) in active.top_targets.clone() {
        if let Some((fp_dir, hash)) = find_matching_top_fingerprint(profile_dir, &fp_root, &deps_dir, &fp_entries, pkg_hint.as_deref(), &top_name) {
            active.fingerprint_dirs.insert(fp_dir);
            active.unit_hashes.insert(hash);
        }
    }
    Ok(())
}

fn find_matching_top_fingerprint(
    profile_dir: &Path,
    fp_root: &Path,
    deps_dir: &Path,
    fp_entries: &[String],
    pkg_hint: Option<&str>,
    top_name: &str,
) -> Option<(String, String)> {
    let top_path = profile_dir.join(top_name);
    let top_meta = fs::metadata(&top_path).ok();
    let stem = top_name.strip_suffix(".exe").unwrap_or(top_name);
    let stem_norm = stem.replace('-', "_");
    let mut best_match: Option<(SystemTime, String, String)> = None;

    for fp_name in fp_entries {
        let Some(hash) = extract_unit_hash(fp_name) else {
            continue;
        };
        if let Some(pkg) = pkg_hint
            && !fp_name.starts_with(&format!("{pkg}-"))
        {
            continue;
        }
        let fp_dir = fp_root.join(fp_name);
        let has_marker = fingerprint_matches_target(&fp_dir, stem, &stem_norm);
        if !has_marker {
            continue;
        }
        let candidate_dep = dep_artifact_for_top(deps_dir, top_name, stem, hash);
        if let (Some(tmeta), Some(dep_path)) = (&top_meta, &candidate_dep)
            && let Ok(dmeta) = fs::metadata(dep_path)
            && same_artifact_file(tmeta, &dmeta)
        {
            return Some((fp_name.clone(), hash.to_owned()));
        }
        let mtime = fs::metadata(fp_dir.join("invoked.timestamp")).and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
        let replace = best_match.as_ref().is_none_or(|(best_time, _, _)| mtime > *best_time);
        if replace {
            best_match = Some((mtime, fp_name.clone(), hash.to_owned()));
        }
    }
    best_match.map(|(_, fp, h)| (fp, h))
}

fn fingerprint_matches_target(fp_dir: &Path, stem: &str, stem_norm: &str) -> bool {
    let Ok(entries) = fs::read_dir(fp_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == format!("bin-{stem}") || name == format!("bin-{stem_norm}") {
            return true;
        }
        if let Some(lib_stem) = stem.strip_prefix("lib").and_then(|s| s.split('.').next()) {
            let lib_norm = lib_stem.replace('-', "_");
            if name == format!("lib-{lib_stem}") || name == format!("lib-{lib_norm}") {
                return true;
            }
        }
    }
    false
}

fn dep_artifact_for_top(deps_dir: &Path, top_name: &str, stem: &str, hash: &str) -> Option<PathBuf> {
    if top_name.ends_with(".exe") {
        let p = deps_dir.join(format!("{stem}-{hash}.exe"));
        if p.exists() {
            return Some(p);
        }
    }
    if let Some((base, ext)) = top_name.rsplit_once('.')
        && matches!(ext, "rlib" | "dylib" | "so" | "dll" | "a" | "lib")
    {
        let p = deps_dir.join(format!("{base}-{hash}.{ext}"));
        if p.exists() {
            return Some(p);
        }
    }
    let p = deps_dir.join(format!("{stem}-{hash}"));
    p.exists().then_some(p)
}

fn same_artifact_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if a.dev() == b.dev() && a.ino() == b.ino() {
            return true;
        }
    }
    a.len() == b.len() && a.modified().ok() == b.modified().ok()
}

fn relative_to_profile(path: &Path, profile_dir: &Path, canonical_profile: &Path) -> Option<PathBuf> {
    if let Ok(rel) = path.strip_prefix(profile_dir) {
        return Some(rel.to_path_buf());
    }
    if let Ok(rel) = path.strip_prefix(canonical_profile) {
        return Some(rel.to_path_buf());
    }
    let canonical_path = fs::canonicalize(path).ok()?;
    canonical_path.strip_prefix(canonical_profile).ok().map(Path::to_path_buf)
}

fn extract_pkg_name_from_id(pkg_id: &str) -> Option<String> {
    if let Some((_, tail)) = pkg_id.rsplit_once('#') {
        let name_part = tail.split('@').next().unwrap_or(tail);
        if !name_part.is_empty() {
            return Some(name_part.to_owned());
        }
    }
    pkg_id.split_whitespace().next().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// Extract the trailing 16-hex Cargo unit hash from a filename or directory name.
#[must_use]
pub fn extract_unit_hash(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    for (idx, &b) in bytes.iter().enumerate().rev() {
        if b != b'-' {
            continue;
        }
        let after = &name[idx + 1..];
        if after.len() < 16 {
            continue;
        }
        let cand = &after[..16];
        let boundary_ok = after.len() == 16 || after.as_bytes()[16] == b'.';
        if boundary_ok && cand.bytes().all(|ch| matches!(ch, b'0'..=b'9' | b'a'..=b'f')) {
            return Some(cand);
        }
    }
    None
}

fn prune_top_archives(target_dir: &Path, stats: &mut TrimStats) -> Result<(), String> {
    let entries = fs::read_dir(target_dir).map_err(|e| format!("read `{}`: {e}", target_dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry `{}`: {e}", target_dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if path.is_file() && name_str.ends_with(".tar.zst") {
            remove_entry(&path, stats)?;
        }
    }
    Ok(())
}

fn prune_named_unit_dir(dir: &Path, active_dirs: &BTreeSet<String>, active_hashes: &BTreeSet<String>, stats: &mut TrimStats) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|e| format!("read `{}`: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry `{}`: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let keep = active_dirs.contains(&name) || extract_unit_hash(&name).is_some_and(|h| active_hashes.contains(h));
        if !keep {
            remove_entry(&entry.path(), stats)?;
        }
    }
    Ok(())
}

fn prune_deps_dir(dir: &Path, active_hashes: &BTreeSet<String>, stats: &mut TrimStats) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|e| format!("read `{}`: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry `{}`: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let is_temp_cgu = name.ends_with(".rcgu.o");
        let keep = !is_temp_cgu && extract_unit_hash(&name).is_some_and(|h| active_hashes.contains(h));
        if !keep {
            remove_entry(&entry.path(), stats)?;
        }
    }
    Ok(())
}

fn prune_dir_children(dir: &Path, stats: &mut TrimStats) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|e| format!("read `{}`: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry `{}`: {e}", dir.display()))?;
        remove_entry(&entry.path(), stats)?;
    }
    Ok(())
}

fn prune_examples_dir(dir: &Path, active_files: &BTreeSet<String>, active_hashes: &BTreeSet<String>, stats: &mut TrimStats) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|e| format!("read `{}`: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry `{}`: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let is_temp_cgu = name.ends_with(".rcgu.o");
        let keep = !is_temp_cgu && (active_files.contains(&name) || extract_unit_hash(&name).is_some_and(|h| active_hashes.contains(h)));
        if !keep {
            remove_entry(&entry.path(), stats)?;
        }
    }
    Ok(())
}

fn prune_profile_top_files(profile_dir: &Path, active_top_files: &BTreeSet<String>, stats: &mut TrimStats) -> Result<(), String> {
    let entries = fs::read_dir(profile_dir).map_err(|e| format!("read `{}`: {e}", profile_dir.display()))?;
    let current_exe = std::env::current_exe().ok().and_then(|p| fs::canonicalize(p).ok());
    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry `{}`: {e}", profile_dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || matches!(name.as_str(), "build" | "deps" | "examples" | "incremental") {
            continue;
        }
        if active_top_files.contains(&name) {
            continue;
        }
        let path = entry.path();
        if let Some(ref exe) = current_exe
            && fs::canonicalize(&path).ok().as_ref() == Some(exe)
        {
            continue;
        }
        remove_entry(&path, stats)?;
    }
    Ok(())
}

fn remove_entry(path: &Path, stats: &mut TrimStats) -> Result<(), String> {
    let bytes = entry_size_bytes(path);
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("inspect `{}`: {err}", path.display())),
    };
    if meta.file_type().is_dir() {
        fs::remove_dir_all(path).map_err(|e| format!("remove dir `{}`: {e}", path.display()))?;
    } else {
        fs::remove_file(path).map_err(|e| format!("remove file `{}`: {e}", path.display()))?;
    }
    stats.removed_entries = stats.removed_entries.saturating_add(1);
    stats.removed_bytes = stats.removed_bytes.saturating_add(bytes);
    Ok(())
}

fn entry_size_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() || meta.file_type().is_file() {
        return meta.len();
    }
    if !meta.file_type().is_dir() {
        return 0;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries.flatten().map(|e| entry_size_bytes(&e.path())).sum()
}

fn write_base_state(root: &Path, target_dir: &Path, base_sha: &str) -> Result<(), String> {
    let sha_path = target_dir.join(BASE_SHA_FILE);
    curvebook_platform::fs::write_atomic(&sha_path, format!("{base_sha}\n").as_bytes())
        .map_err(|e| format!("write `{}`: {e}", sha_path.display()))?;

    if root.join(".git").exists() {
        let tree_hashes = collect_git_tree_hashes(root)?;
        let hashes_path = target_dir.join(TREE_HASHES_FILE);
        curvebook_platform::fs::write_atomic(&hashes_path, tree_hashes.as_bytes()).map_err(|e| format!("write `{}`: {e}", hashes_path.display()))?;
    }
    Ok(())
}

fn collect_git_tree_hashes(root: &Path) -> Result<String, String> {
    let raw = git_stdout_bytes(root, &["-c", "core.quotePath=false", "ls-files", "-s", "-z"])?;
    let mut out = String::new();
    for entry in raw.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        let line = std::str::from_utf8(entry).map_err(|e| format!("git ls-files -s utf-8: {e}"))?;
        if let Some((meta, path)) = line.split_once('\t') {
            let mut parts = meta.split_whitespace();
            let _mode = parts.next();
            if let Some(oid) = parts.next() {
                out.push_str(oid);
                out.push('\t');
                out.push_str(path);
                out.push('\n');
            }
        }
    }
    Ok(out)
}

fn read_cached_base_sha(target_dir: &Path) -> Result<Option<String>, String> {
    let sha_path = target_dir.join(BASE_SHA_FILE);
    let Some(bytes) =
        curvebook_platform::fs::read_regular_file_bytes_if_exists(&sha_path, 4096).map_err(|e| format!("read `{}`: {e}", sha_path.display()))?
    else {
        return Ok(None);
    };
    let text = String::from_utf8(bytes).map_err(|e| format!("decode `{}`: {e}", sha_path.display()))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    validate_sha_text(trimmed).map(Some)
}

/// Restore modification times of files in `root` unchanged since `base_sha` to a deterministic
/// baseline timestamp older than restored target fingerprints.
pub fn restore_mtimes(root: &Path, target_dir: &Path, base_sha: &str) -> Result<RestoreStats, String> {
    let clean_sha = validate_sha_text(base_sha)?;
    let Some(changed) = collect_changed_since_base(root, target_dir, &clean_sha)? else {
        println!(
            "ci-cache restore-mtimes: base commit `{}` is absent from shallow checkout and no tree hash manifest exists; skipping",
            short_sha(&clean_sha)
        );
        return Ok(RestoreStats::default());
    };

    let tracked = git_nul_list(root, &["-c", "core.quotePath=false", "ls-files", "-z"])?;
    let baseline = compute_baseline_timestamp(target_dir);
    let mut stats = RestoreStats::default();

    for rel in tracked {
        if changed.contains(&rel) {
            stats.skipped_modified = stats.skipped_modified.saturating_add(1);
            continue;
        }
        let full = root.join(&rel);
        let Ok(meta) = fs::symlink_metadata(&full) else {
            continue;
        };
        if !meta.file_type().is_file() {
            continue;
        }
        set_file_mtime(&full, baseline)?;
        stats.restored_files = stats.restored_files.saturating_add(1);
    }
    Ok(stats)
}

fn collect_changed_since_base(root: &Path, target_dir: &Path, base_sha: &str) -> Result<Option<BTreeSet<String>>, String> {
    let commit_spec = format!("{base_sha}^{{commit}}");
    if git_check_ok(root, &["rev-parse", "--verify", "--quiet", &commit_spec]) {
        let mut changed: BTreeSet<String> =
            git_nul_list(root, &["-c", "core.quotePath=false", "diff", "--name-only", "--no-renames", "-z", base_sha])?.into_iter().collect();
        for staged in git_nul_list(root, &["-c", "core.quotePath=false", "diff", "--cached", "--name-only", "--no-renames", "-z", base_sha])? {
            changed.insert(staged);
        }
        return Ok(Some(changed));
    }

    let hashes_path = target_dir.join(TREE_HASHES_FILE);
    let Some(bytes) = curvebook_platform::fs::read_regular_file_bytes_if_exists(&hashes_path, 32 * 1024 * 1024)
        .map_err(|e| format!("read `{}`: {e}", hashes_path.display()))?
    else {
        return Ok(None);
    };
    let manifest_text = String::from_utf8(bytes).map_err(|e| format!("decode `{}`: {e}", hashes_path.display()))?;
    let mut base_hashes: BTreeMap<String, String> = BTreeMap::new();
    for line in manifest_text.lines() {
        if let Some((oid, path)) = line.split_once('\t') {
            base_hashes.insert(path.to_owned(), oid.to_owned());
        }
    }

    let current_raw = collect_git_tree_hashes(root)?;
    let mut changed = BTreeSet::new();
    for line in current_raw.lines() {
        if let Some((oid, path)) = line.split_once('\t')
            && base_hashes.get(path).map(String::as_str) != Some(oid)
        {
            changed.insert(path.to_owned());
        }
    }
    for dirty in git_nul_list(root, &["-c", "core.quotePath=false", "diff", "--name-only", "--no-renames", "-z"])? {
        changed.insert(dirty);
    }
    Ok(Some(changed))
}

fn compute_baseline_timestamp(target_dir: &Path) -> SystemTime {
    let default_time = UNIX_EPOCH + Duration::from_secs(DEFAULT_BASELINE_EPOCH_SECS);
    let mut oldest: Option<SystemTime> = None;

    if let Ok(entries) = fs::read_dir(target_dir) {
        for entry in entries.flatten() {
            let fp_dir = entry.path().join(".fingerprint");
            if !fp_dir.is_dir() {
                continue;
            }
            if let Ok(fp_entries) = fs::read_dir(&fp_dir) {
                for unit in fp_entries.flatten().take(64) {
                    let ts_file = unit.path().join("invoked.timestamp");
                    if let Ok(meta) = fs::metadata(&ts_file)
                        && let Ok(mtime) = meta.modified()
                    {
                        oldest = Some(oldest.map_or(mtime, |cur| cur.min(mtime)));
                    }
                }
            }
        }
    }

    match oldest {
        Some(min_target) if min_target <= default_time => min_target.checked_sub(Duration::from_secs(60)).unwrap_or(UNIX_EPOCH),
        _ => default_time,
    }
}

fn set_file_mtime(path: &Path, mtime: SystemTime) -> Result<(), String> {
    let file = open_for_mtime(path).map_err(|e| format!("open `{}` for set_modified: {e}", path.display()))?;
    file.set_modified(mtime).map_err(|e| format!("set_modified `{}`: {e}", path.display()))
}

#[cfg(windows)]
fn open_for_mtime(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
    OpenOptions::new().access_mode(FILE_WRITE_ATTRIBUTES).open(path)
}

#[cfg(not(windows))]
fn open_for_mtime(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().write(true).open(path).or_else(|_| File::open(path))
}

fn git_head_sha(root: &Path) -> Result<String, String> {
    let out = git_stdout_bytes(root, &["rev-parse", "HEAD"])?;
    let sha = String::from_utf8(out).map_err(|e| format!("git rev-parse utf-8: {e}"))?;
    validate_sha_text(&sha)
}

fn git_check_ok(root: &Path, args: &[&str]) -> bool {
    Command::new("git").current_dir(root).args(args).output().is_ok_and(|o| o.status.success())
}

fn git_nul_list(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let raw = git_stdout_bytes(root, args)?;
    let mut paths = Vec::new();
    for entry in raw.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        let s = std::str::from_utf8(entry).map_err(|e| format!("git {} utf-8: {e}", args.join(" ")))?;
        paths.push(s.to_owned());
    }
    Ok(paths)
}

fn git_stdout_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("git").current_dir(root).args(args).output().map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

fn short_sha(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}

fn bytes_to_mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{
        BASE_SHA_FILE, DEFAULT_BASELINE_EPOCH_SECS, TREE_HASHES_FILE, extract_unit_hash, restore_mtimes, run, set_file_mtime, trim_target_dir,
    };

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(label: &str) -> Self {
            let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
            let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            let path = std::env::temp_dir().join(format!("cb-ci-cache-{label}-{}-{nanos}-{seq}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn run_git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git").current_dir(dir).args(args).output().unwrap();
        assert!(out.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn init_repo(dir: &Path) {
        run_git(dir, &["init"]);
        run_git(dir, &["config", "user.name", "Test"]);
        run_git(dir, &["config", "user.email", "test@example.com"]);
    }

    #[test]
    fn extract_unit_hash_parses_cargo_artifact_names() {
        assert_eq!(extract_unit_hash("libcurvebook_text-90d41afc8e4682d5.rlib"), Some("90d41afc8e4682d5"));
        assert_eq!(extract_unit_hash("audit_r2_s13-b6893210fe3f41e6"), Some("b6893210fe3f41e6"));
        assert_eq!(extract_unit_hash("r0.35-0123456789abcdef.exe"), Some("0123456789abcdef"));
        assert_eq!(extract_unit_hash("curvebook-ui-egui-14e01d86b1be1e8b"), Some("14e01d86b1be1e8b"));
        assert_eq!(extract_unit_hash("xtask"), None);
        assert_eq!(extract_unit_hash("curvebook-0.1.0"), None);
    }

    #[test]
    fn trim_retains_active_units_and_prunes_stale_artifacts_and_archives() {
        let fx = TempDirGuard::new("trim");
        let root = fx.path.join("repo");
        let target = root.join("target");
        let debug = target.join("debug");
        fs::create_dir_all(&root).unwrap();
        init_repo(&root);
        fs::write(root.join("lib.rs"), "pub fn ok() {}\n").unwrap();
        run_git(&root, &["add", "lib.rs"]);
        run_git(&root, &["commit", "-m", "base"]);
        let sha = run_git(&root, &["rev-parse", "HEAD"]);

        for rel in [
            "debug/.fingerprint/kept-1111111111111111/lib-kept",
            "debug/.fingerprint/test_pkg-2222222222222222/test-it",
            "debug/.fingerprint/build_pkg-3333333333333333/build-script-build",
            "debug/.fingerprint/build_pkg-4444444444444444/run-build-script",
            "debug/.fingerprint/app_pkg-5555555555555555/bin-appbin",
            "debug/.fingerprint/stale-aaaaaaaaaaaaaaaa/lib-stale",
            "debug/deps/libkept-1111111111111111.rlib",
            "debug/deps/libkept-1111111111111111.rmeta",
            "debug/deps/kept-1111111111111111.d",
            "debug/deps/kept-1111111111111111.kept.0000-cgu.0.rcgu.o",
            "debug/deps/it-2222222222222222",
            "debug/deps/it-2222222222222222.d",
            "debug/deps/appbin-5555555555555555",
            "debug/deps/libstale-aaaaaaaaaaaaaaaa.rlib",
            "debug/deps/stale_test-bbbbbbbbbbbbbbbb",
            "debug/build/build_pkg-3333333333333333/build-script-build",
            "debug/build/build_pkg-4444444444444444/out/gen.rs",
            "debug/build/stale_pkg-cccccccccccccccc/out/old.rs",
            "debug/incremental/session-old/dep-graph.bin",
            "debug/stale_bin",
            "ci-tests.tar.zst",
        ] {
            let p = target.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"artifact").unwrap();
        }
        // Top-level binary whose hash lives only in `deps/appbin-5555555555555555` + `.fingerprint/app_pkg-5555555555555555`.
        let top_bin = debug.join("appbin");
        fs::write(&top_bin, b"artifact").unwrap();
        let dep_bin = debug.join("deps/appbin-5555555555555555");
        let top_mtime = fs::metadata(&top_bin).unwrap().modified().unwrap();
        let _ = OpenOptions::new().write(true).open(&dep_bin).unwrap().set_modified(top_mtime);

        let msgs = [
            serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": "path+file:///repo#kept@0.1.0",
                "filenames": [
                    target.join("debug/deps/libkept-1111111111111111.rlib"),
                    target.join("debug/deps/libkept-1111111111111111.rmeta"),
                ],
                "executable": serde_json::Value::Null
            })
            .to_string(),
            serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": "path+file:///repo#test_pkg@0.1.0",
                "filenames": [target.join("debug/deps/it-2222222222222222")],
                "executable": target.join("debug/deps/it-2222222222222222")
            })
            .to_string(),
            serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": "path+file:///repo#build_pkg@0.1.0",
                "filenames": [target.join("debug/build/build_pkg-3333333333333333/build-script-build")],
                "executable": serde_json::Value::Null
            })
            .to_string(),
            serde_json::json!({
                "reason": "build-script-executed",
                "package_id": "path+file:///repo#build_pkg@0.1.0",
                "out_dir": target.join("debug/build/build_pkg-4444444444444444/out")
            })
            .to_string(),
            serde_json::json!({
                "reason": "compiler-artifact",
                "package_id": "path+file:///repo#app_pkg@0.1.0",
                "filenames": [target.join("debug/appbin")],
                "executable": target.join("debug/appbin")
            })
            .to_string(),
        ]
        .join("\n");

        // Empty message stream must fail closed rather than wiping target/.
        assert!(trim_target_dir(&root, &target, "debug", "", &sha).is_err());

        let stats = trim_target_dir(&root, &target, "debug", &msgs, &sha).unwrap();
        assert!(stats.removed_entries >= 6, "{stats:?}");

        for kept in [
            "debug/.fingerprint/kept-1111111111111111/lib-kept",
            "debug/.fingerprint/test_pkg-2222222222222222/test-it",
            "debug/.fingerprint/build_pkg-3333333333333333/build-script-build",
            "debug/.fingerprint/build_pkg-4444444444444444/run-build-script",
            "debug/.fingerprint/app_pkg-5555555555555555/bin-appbin",
            "debug/deps/libkept-1111111111111111.rlib",
            "debug/deps/libkept-1111111111111111.rmeta",
            "debug/deps/kept-1111111111111111.d",
            "debug/deps/it-2222222222222222",
            "debug/deps/it-2222222222222222.d",
            "debug/deps/appbin-5555555555555555",
            "debug/build/build_pkg-3333333333333333/build-script-build",
            "debug/build/build_pkg-4444444444444444/out/gen.rs",
            "debug/appbin",
            BASE_SHA_FILE,
            TREE_HASHES_FILE,
        ] {
            assert!(target.join(kept).exists(), "expected active artifact `{kept}` to remain");
        }
        assert_eq!(fs::read_to_string(target.join(BASE_SHA_FILE)).unwrap().trim(), sha);

        for pruned in [
            "debug/.fingerprint/stale-aaaaaaaaaaaaaaaa",
            "debug/deps/kept-1111111111111111.kept.0000-cgu.0.rcgu.o",
            "debug/deps/libstale-aaaaaaaaaaaaaaaa.rlib",
            "debug/deps/stale_test-bbbbbbbbbbbbbbbb",
            "debug/build/stale_pkg-cccccccccccccccc",
            "debug/incremental/session-old",
            "debug/stale_bin",
            "ci-tests.tar.zst",
        ] {
            assert!(!target.join(pruned).exists(), "expected stale entry `{pruned}` to be pruned");
        }
    }

    #[test]
    fn restore_mtimes_resets_unchanged_files_and_keeps_modified_and_shallow_fallback() {
        let fx = TempDirGuard::new("restore");
        let root = fx.path.join("repo");
        let target = root.join("target");
        fs::create_dir_all(&root).unwrap();
        init_repo(&root);

        fs::write(root.join("unchanged.rs"), "pub fn u() -> i32 { 1 }\n").unwrap();
        fs::write(root.join("changed_commit.rs"), "pub fn c() -> i32 { 1 }\n").unwrap();
        fs::write(root.join("changed_worktree.rs"), "pub fn w() -> i32 { 1 }\n").unwrap();
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "-m", "base"]);
        let base_sha = run_git(&root, &["rev-parse", "HEAD"]);

        // Write tree hashes at base commit so shallow fallback is also verified.
        fs::create_dir_all(target.join("debug/.fingerprint/u-1")).unwrap();
        super::write_base_state(&root, &target, &base_sha).unwrap();

        fs::write(root.join("changed_commit.rs"), "pub fn c() -> i32 { 2 }\n").unwrap();
        fs::write(root.join("added_commit.rs"), "pub fn a() -> i32 { 3 }\n").unwrap();
        run_git(&root, &["add", "changed_commit.rs", "added_commit.rs"]);
        run_git(&root, &["commit", "-m", "second"]);

        fs::write(root.join("changed_worktree.rs"), "pub fn w() -> i32 { 4 }\n").unwrap();
        fs::write(root.join("untracked.rs"), "pub fn x() -> i32 { 5 }\n").unwrap();

        // Simulate fresh checkout assigning newer timestamps to every file.
        let checkout_time = UNIX_EPOCH + Duration::from_secs(1_760_000_000);
        for name in ["unchanged.rs", "changed_commit.rs", "added_commit.rs", "changed_worktree.rs", "untracked.rs"] {
            set_file_mtime(&root.join(name), checkout_time).unwrap();
        }

        let stats = restore_mtimes(&root, &target, &base_sha).unwrap();
        assert_eq!(stats.restored_files, 1);
        assert_eq!(stats.skipped_modified, 3);

        let baseline = UNIX_EPOCH + Duration::from_secs(DEFAULT_BASELINE_EPOCH_SECS);
        assert_eq!(fs::metadata(root.join("unchanged.rs")).unwrap().modified().unwrap(), baseline);
        for newer in ["changed_commit.rs", "added_commit.rs", "changed_worktree.rs", "untracked.rs"] {
            assert_eq!(fs::metadata(root.join(newer)).unwrap().modified().unwrap(), checkout_time, "{newer}");
        }

        // Verify shallow fallback when base SHA is absent from git objects.
        set_file_mtime(&root.join("unchanged.rs"), checkout_time).unwrap();
        let missing_sha = "0123456789abcdef0123456789abcdef01234567";
        let fallback_stats = restore_mtimes(&root, &target, missing_sha).unwrap();
        assert_eq!(fallback_stats.restored_files, 1);
        assert_eq!(fs::metadata(root.join("unchanged.rs")).unwrap().modified().unwrap(), baseline);
        assert_eq!(fs::metadata(root.join("changed_commit.rs")).unwrap().modified().unwrap(), checkout_time);
    }

    #[test]
    fn restore_mtimes_prevents_cargo_recompilation_of_unchanged_crate() {
        let fx = TempDirGuard::new("cargo-fresh");
        let root = fx.path.join("ws");
        let target = fx.path.join("target");
        let cargo_home = fx.path.join("cargo-home");
        fs::create_dir_all(root.join("crate_a/src")).unwrap();
        fs::create_dir_all(root.join("crate_b/src")).unwrap();
        fs::create_dir_all(&cargo_home).unwrap();
        init_repo(&root);

        fs::write(root.join("Cargo.toml"), "[workspace]\nresolver = \"3\"\nmembers = [\"crate_a\", \"crate_b\"]\n").unwrap();
        fs::write(root.join("crate_a/Cargo.toml"), "[package]\nname = \"crate_a\"\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
        fs::write(root.join("crate_a/src/lib.rs"), "pub fn a() -> i32 { 1 }\n").unwrap();
        fs::write(root.join("crate_b/Cargo.toml"), "[package]\nname = \"crate_b\"\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
        fs::write(root.join("crate_b/src/lib.rs"), "pub fn b() -> i32 { 2 }\n").unwrap();
        run_git(&root, &["add", "."]);
        run_git(&root, &["commit", "-m", "initial"]);
        let base_sha = run_git(&root, &["rev-parse", "HEAD"]);

        let build1 = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_HOME", &cargo_home)
            .env("CARGO_INCREMENTAL", "0")
            .args(["build", "--workspace", "--message-format=json"])
            .output()
            .unwrap();
        assert!(build1.status.success(), "{}", String::from_utf8_lossy(&build1.stderr));
        let msgs1 = String::from_utf8(build1.stdout).unwrap();
        let msgs_path = fx.path.join("msgs.jsonl");
        fs::write(&msgs_path, &msgs1).unwrap();

        run(
            &root,
            &[
                String::from("trim"),
                String::from("--target-dir"),
                target.to_string_lossy().into_owned(),
                String::from("--messages"),
                msgs_path.to_string_lossy().into_owned(),
                String::from("--base-sha"),
                base_sha,
            ],
        )
        .unwrap();

        // Commit a change ONLY to crate_b, then assign fresh checkout mtimes to every file.
        fs::write(root.join("crate_b/src/lib.rs"), "pub fn b() -> i32 { 22 }\n").unwrap();
        run_git(&root, &["add", "crate_b/src/lib.rs"]);
        run_git(&root, &["commit", "-m", "update crate_b"]);

        let future_checkout = SystemTime::now() + Duration::from_secs(10);
        for rel in ["Cargo.toml", "crate_a/Cargo.toml", "crate_a/src/lib.rs", "crate_b/Cargo.toml", "crate_b/src/lib.rs"] {
            set_file_mtime(&root.join(rel), future_checkout).unwrap();
        }

        run(&root, &[String::from("restore-mtimes"), String::from("--target-dir"), target.to_string_lossy().into_owned()]).unwrap();

        let build2 = Command::new("cargo")
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", &target)
            .env("CARGO_HOME", &cargo_home)
            .env("CARGO_INCREMENTAL", "0")
            .args(["build", "--workspace", "--message-format=json"])
            .output()
            .unwrap();
        assert!(build2.status.success(), "{}", String::from_utf8_lossy(&build2.stderr));

        let mut fresh_by_name = std::collections::BTreeMap::new();
        for line in String::from_utf8(build2.stdout).unwrap().lines() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line)
                && val.get("reason").and_then(serde_json::Value::as_str) == Some("compiler-artifact")
                && let (Some(name), Some(fresh)) = (
                    val.get("target").and_then(|t| t.get("name")).and_then(serde_json::Value::as_str),
                    val.get("fresh").and_then(serde_json::Value::as_bool),
                )
            {
                fresh_by_name.insert(name.to_owned(), fresh);
            }
        }
        assert_eq!(fresh_by_name.get("crate_a"), Some(&true), "unchanged crate_a must stay fresh: {fresh_by_name:?}");
        assert_eq!(fresh_by_name.get("crate_b"), Some(&false), "modified crate_b must recompile: {fresh_by_name:?}");
    }
}
